use anyhow::Result;
use axum::{extract::State as AxumState, extract::Json, routing::{get, post}, Router};
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{broadcast, Mutex, RwLock};
use tokio_tungstenite::accept_async;
use tungstenite::Message as WsMessage;

use crate::audio::capture::AudioCapture;
use crate::protocol::{Message, StreamMode, CHANNELS, timestamp_ms};

pub static ACTUAL_SAMPLE_RATE: std::sync::atomic::AtomicU32 =
    std::sync::atomic::AtomicU32::new(44100);

pub type ClientInfo = Arc<RwLock<HashMap<String, ClientEntry>>>;

#[derive(Clone)]
pub struct ClientEntry {
    pub client_id: String,
    pub addr: SocketAddr,
    pub mode: StreamMode,
    pub latency_ms: f64,
    pub connected_at: u64,
    pub ws_sender: Arc<Mutex<Option<WsSplitSink>>>,
}

pub struct AppState {
    pub clients: ClientInfo,
    pub sample_rate: Arc<RwLock<u32>>,
    pub audio_capture: Arc<RwLock<Option<AudioCapture>>>,
    pub broadcast_tx: broadcast::Sender<Vec<u8>>,
    pub broadcast_handle: Arc<Mutex<Option<tokio::task::JoinHandle<()>>>>,
}

pub async fn run_server(host: &str, port: u16, web_port: u16) -> Result<()> {
    let addr = format!("{}:{}", host, port);
    let listener = TcpListener::bind(&addr).await?;
    log::info!("Server listening on {}", addr);

    let (broadcast_tx, _) = broadcast::channel::<Vec<u8>>(1000);

    let state = Arc::new(AppState {
        clients: Arc::new(RwLock::new(HashMap::new())),
        sample_rate: Arc::new(RwLock::new(
            ACTUAL_SAMPLE_RATE.load(std::sync::atomic::Ordering::Relaxed),
        )),
        audio_capture: Arc::new(RwLock::new(None)),
        broadcast_tx: broadcast_tx.clone(),
        broadcast_handle: Arc::new(Mutex::new(None)),
    });

    let web_state = state.clone();
    tokio::spawn(async move {
        start_web_server(web_port, web_state).await;
    });

    let mut shutdown_rx = {
        let (tx, rx) = tokio::sync::oneshot::channel();
        let mut shutdown_tx = Some(tx);
        ctrlc::set_handler(move || {
            if let Some(tx) = shutdown_tx.take() {
                let _ = tx.send(());
            }
        })?;
        rx
    };

    let server_broadcast = broadcast_tx.clone();
    let server_clients = state.clients.clone();
    let state_for_accept = state.clone();

    let server_handle = tokio::spawn(async move {
        loop {
            tokio::select! {
                accept = listener.accept() => {
                    match accept {
                        Ok((stream, addr)) => {
                            log::info!("New connection from {}", addr);
                            let broadcast = server_broadcast.clone();
                            let clients = server_clients.clone();
                            let state_clone = state_for_accept.clone();
                            tokio::spawn(handle_connection(stream, addr, broadcast, clients, state_clone));
                        }
                        Err(e) => {
                            log::error!("Accept error: {}", e);
                        }
                    }
                }
                _ = &mut shutdown_rx => {
                    log::info!("Shutdown signal received");
                    break;
                }
            }
        }
    });

    start_audio_capture(state.clone()).await;

    server_handle.await?;
    Ok(())
}

async fn start_audio_capture(state: Arc<AppState>) {
    let (audio_tx, mut audio_rx) = tokio::sync::mpsc::channel::<Vec<u8>>(200);
    let broadcast_tx = state.broadcast_tx.clone();

    match AudioCapture::new(audio_tx) {
        Ok(capture) => {
            *state.audio_capture.write().await = Some(capture);
            log::info!("System audio capture started");

            let handle = tokio::spawn(async move {
                while let Some(data) = audio_rx.recv().await {
                    let _ = broadcast_tx.send(data);
                }
            });
            *state.broadcast_handle.lock().await = Some(handle);
        }
        Err(e) => {
            log::error!("Failed to start audio capture: {}", e);
        }
    }
}

async fn restart_audio_capture(state: &AppState, rate: u32) -> Result<()> {
    let old = state.audio_capture.write().await.take();
    if let Some(mut c) = old {
        c.stop();
        c.wait_stopped().await;
    }

    if let Some(handle) = state.broadcast_handle.lock().await.take() {
        handle.abort();
    }

    ACTUAL_SAMPLE_RATE.store(rate, std::sync::atomic::Ordering::Relaxed);
    *state.sample_rate.write().await = rate;

    let (audio_tx, mut audio_rx) = tokio::sync::mpsc::channel::<Vec<u8>>(200);
    let broadcast_tx = state.broadcast_tx.clone();

    match AudioCapture::new(audio_tx) {
        Ok(capture) => {
            *state.audio_capture.write().await = Some(capture);
            log::info!("Audio capture restarted at {}Hz", rate);

            let handle = tokio::spawn(async move {
                while let Some(data) = audio_rx.recv().await {
                    let _ = broadcast_tx.send(data);
                }
            });
            *state.broadcast_handle.lock().await = Some(handle);
            Ok(())
        }
        Err(e) => {
            log::error!("Failed to restart audio capture: {}", e);
            Err(e)
        }
    }
}

type WsSplitSink = futures_util::stream::SplitSink<
    tokio_tungstenite::WebSocketStream<TcpStream>,
    WsMessage,
>;

async fn handle_connection(
    stream: TcpStream,
    addr: SocketAddr,
    broadcast_tx: broadcast::Sender<Vec<u8>>,
    clients: ClientInfo,
    state: Arc<AppState>,
) {
    let ws_stream = match accept_async(stream).await {
        Ok(ws) => ws,
        Err(e) => {
            log::error!("WebSocket handshake failed for {}: {}", addr, e);
            return;
        }
    };

    log::info!("WebSocket connection established with {}", addr);

    let (ws_sender_raw, mut ws_receiver) = ws_stream.split();
    let ws_sender: Arc<Mutex<Option<WsSplitSink>>> = Arc::new(Mutex::new(Some(ws_sender_raw)));

    let mut client_mode: Option<StreamMode> = None;
    let mut client_id: Option<String> = None;

    let heartbeat_sender = ws_sender.clone();
    let heartbeat_handle = tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(10));
        loop {
            interval.tick().await;
            let ping = Message::Ping {
                timestamp: timestamp_ms(),
            };
            if let Ok(json) = serde_json::to_string(&ping) {
                let mut guard = heartbeat_sender.lock().await;
                if let Some(sender) = guard.as_mut() {
                    if sender.send(WsMessage::Text(json)).await.is_err() {
                        break;
                    }
                }
            }
        }
    });

    while let Some(msg) = ws_receiver.next().await {
        match msg {
            Ok(WsMessage::Text(text)) => {
                match Message::from_json_bytes(text.as_bytes()) {
                    Some(Message::Hello {
                        client_id: id,
                        mode,
                        sample_rate,
                        channels,
                    }) => {
                        log::info!(
                            "Client {} connected, mode: {:?}, {}Hz, {}ch",
                            id, mode, sample_rate, channels
                        );

                        let actual_rate = ACTUAL_SAMPLE_RATE
                            .load(std::sync::atomic::Ordering::Relaxed);

                        let session_id = uuid::Uuid::new_v4().to_string();
                        let ack = Message::HelloAck {
                            session_id,
                            sample_rate: actual_rate,
                            channels: CHANNELS,
                        };

                        if let Ok(ack_bytes) = serde_json::to_string(&ack) {
                            let mut guard = ws_sender.lock().await;
                            if let Some(sender) = guard.as_mut() {
                                let _ = sender.send(WsMessage::Text(ack_bytes)).await;
                            }
                        }

                        clients.write().await.insert(
                            id.clone(),
                            ClientEntry {
                                client_id: id.clone(),
                                addr,
                                mode: mode.clone(),
                                latency_ms: 0.0,
                                connected_at: timestamp_ms(),
                                ws_sender: ws_sender.clone(),
                            },
                        );

                        client_id = Some(id.clone());
                        client_mode = Some(mode.clone());

                        match mode {
                            StreamMode::Speaker => {
                                let mut broadcast_rx = broadcast_tx.subscribe();
                                let speaker_sender = ws_sender.clone();
                                let client_id_for_task = id.clone();

                                tokio::spawn(async move {
                                    let mut seq: u64 = 0;
                                    loop {
                                        match broadcast_rx.recv().await {
                                            Ok(data) => {
                                                let rate = ACTUAL_SAMPLE_RATE
                                                    .load(std::sync::atomic::Ordering::Relaxed);
                                                let msg = Message::AudioData {
                                                    sequence: seq,
                                                    timestamp: timestamp_ms(),
                                                    sample_rate: rate,
                                                    data,
                                                };
                                                seq += 1;
                                                if let Ok(json) = serde_json::to_string(&msg) {
                                                    let mut guard =
                                                        speaker_sender.lock().await;
                                                    if let Some(sender) = guard.as_mut() {
                                                        if sender
                                                            .send(WsMessage::Text(json))
                                                            .await
                                                            .is_err()
                                                        {
                                                            log::info!(
                                                                "Speaker {} disconnected",
                                                                client_id_for_task
                                                            );
                                                            break;
                                                        }
                                                    }
                                                }
                                            }
                                            Err(broadcast::error::RecvError::Lagged(n)) => {
                                                log::debug!("Lagged {} messages", n);
                                            }
                                            Err(broadcast::error::RecvError::Closed) => break,
                                        }
                                    }
                                });

                                log::info!("Client {} set as speaker", id);
                            }
                            StreamMode::Microphone => {
                                log::info!("Client {} set as microphone", id);
                            }
                        }
                    }
                    Some(Message::AudioData { sequence, data, .. }) => {
                        if client_mode == Some(StreamMode::Microphone) {
                            let _ = broadcast_tx.send(data);

                            let ack = Message::AudioDataAck { sequence };
                            if let Ok(ack_bytes) = serde_json::to_string(&ack) {
                                let mut guard = ws_sender.lock().await;
                                if let Some(sender) = guard.as_mut() {
                                    let _ = sender.send(WsMessage::Text(ack_bytes)).await;
                                }
                            }
                        }
                    }
                    Some(Message::Pong { timestamp }) => {
                        let now = timestamp_ms();
                        let latency = (now as f64 - timestamp as f64) / 2.0;
                        if let Some(ref id) = client_id {
                            if let Some(entry) = clients.write().await.get_mut(id) {
                                entry.latency_ms = latency;
                            }
                        }
                    }
                    Some(Message::Ping { timestamp }) => {
                        let pong = Message::Pong { timestamp };
                        if let Ok(json) = serde_json::to_string(&pong) {
                            let mut guard = ws_sender.lock().await;
                            if let Some(sender) = guard.as_mut() {
                                let _ = sender.send(WsMessage::Text(json)).await;
                            }
                        }
                    }
                    Some(Message::LatencyReport { latency_ms }) => {
                        if let Some(ref id) = client_id {
                            if let Some(entry) = clients.write().await.get_mut(id) {
                                entry.latency_ms = latency_ms;
                            }
                        }
                    }
                    Some(_) => {}
                    None => {
                        log::warn!("Invalid message from {}", addr);
                    }
                }
            }
            Ok(WsMessage::Close(_)) => {
                log::info!("Client disconnected from {}", addr);
                break;
            }
            Err(e) => {
                log::error!("WebSocket error from {}: {}", addr, e);
                break;
            }
            _ => {}
        }
    }

    heartbeat_handle.abort();
    {
        let mut guard = ws_sender.lock().await;
        *guard = None;
    }
    if let Some(id) = &client_id {
        clients.write().await.remove(id);
        log::info!("Client {} removed", id);
    }
    log::info!("Connection closed for {}", addr);
}

async fn start_web_server(port: u16, state: Arc<AppState>) {
    let app = Router::new()
        .route("/", get(index_page))
        .route("/api/clients", get(get_clients))
        .route("/api/sample-rate", get(get_sample_rate).post(set_sample_rate))
        .with_state(state);

    let addr = format!("0.0.0.0:{}", port);
    log::info!("Management UI at http://{}", addr);

    let listener = tokio::net::TcpListener::bind(&addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();
}

async fn index_page() -> axum::response::Html<&'static str> {
    axum::response::Html(include_str!("web/index.html"))
}

async fn get_clients(AxumState(state): AxumState<Arc<AppState>>) -> Json<Value> {
    let clients = state.clients.read().await;
    let list: Vec<Value> = clients
        .values()
        .map(|c| {
            json!({
                "client_id": c.client_id,
                "addr": c.addr.to_string(),
                "mode": format!("{:?}", c.mode),
                "latency_ms": (c.latency_ms * 100.0).round() / 100.0,
                "connected_at": c.connected_at,
            })
        })
        .collect();
    Json(json!({ "clients": list }))
}

async fn get_sample_rate(AxumState(_state): AxumState<Arc<AppState>>) -> Json<Value> {
    let rate = ACTUAL_SAMPLE_RATE.load(std::sync::atomic::Ordering::Relaxed);
    Json(json!({ "sample_rate": rate }))
}

async fn set_sample_rate(
    AxumState(state): AxumState<Arc<AppState>>,
    Json(payload): Json<Value>,
) -> Json<Value> {
    let Some(rate) = payload.get("sample_rate").and_then(|v| v.as_u64()) else {
        return Json(json!({ "ok": false, "error": "missing sample_rate" }));
    };

    let rate = rate as u32;
    log::info!("Sample rate changed to {}Hz, restarting capture...", rate);

    if let Err(e) = restart_audio_capture(&state, rate).await {
        return Json(json!({ "ok": false, "error": format!("{}", e) }));
    }

    Json(json!({ "ok": true, "sample_rate": rate }))
}
