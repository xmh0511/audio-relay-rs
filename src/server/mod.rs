use anyhow::Result;
use axum::{extract::Json, extract::State as AxumState, routing::get, Router};
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
use crate::protocol::{
    decode_audio_frame, encode_audio_frame, timestamp_ms, AudioFrame, Message, StreamMode, CHANNELS,
};

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
    pub bytes_up: u64,
    pub bytes_down: u64,
    pub last_speed_calc: u64,
    pub bytes_up_snapshot: u64,
    pub bytes_down_snapshot: u64,
    pub speed_up: f64,
    pub speed_down: f64,
    #[allow(dead_code)]
    pub ws_sender: Arc<Mutex<Option<WsSplitSink>>>,
}

impl ClientEntry {
    pub fn new(
        client_id: String,
        addr: SocketAddr,
        mode: StreamMode,
        ws_sender: Arc<Mutex<Option<WsSplitSink>>>,
    ) -> Self {
        let now = crate::protocol::timestamp_ms();
        Self {
            client_id,
            addr,
            mode,
            latency_ms: 0.0,
            connected_at: now,
            bytes_up: 0,
            bytes_down: 0,
            last_speed_calc: now,
            bytes_up_snapshot: 0,
            bytes_down_snapshot: 0,
            speed_up: 0.0,
            speed_down: 0.0,
            ws_sender,
        }
    }

    pub fn update_speed(&mut self) {
        let now = crate::protocol::timestamp_ms();
        let elapsed = now.saturating_sub(self.last_speed_calc);
        if elapsed >= 1000 {
            self.speed_up =
                (self.bytes_up - self.bytes_up_snapshot) as f64 / elapsed as f64 * 1000.0;
            self.speed_down =
                (self.bytes_down - self.bytes_down_snapshot) as f64 / elapsed as f64 * 1000.0;
            self.bytes_up_snapshot = self.bytes_up;
            self.bytes_down_snapshot = self.bytes_down;
            self.last_speed_calc = now;
        }
    }
}

pub struct AudioCaptureState {
    pub capture: AudioCapture,
    pub broadcast_handle: tokio::task::JoinHandle<()>,
}

pub struct AppState {
    pub clients: ClientInfo,
    pub audio_capture: Arc<Mutex<Option<AudioCaptureState>>>,
    pub broadcast_tx: broadcast::Sender<(u32, Vec<u8>)>,
}

pub async fn run_server(host: &str, port: u16, web_port: u16) -> Result<()> {
    let detected_rate = crate::audio::capture::detect_sample_rate();
    ACTUAL_SAMPLE_RATE.store(detected_rate, std::sync::atomic::Ordering::Relaxed);

    let addr = format!("{}:{}", host, port);
    let listener = TcpListener::bind(&addr).await?;
    log::info!("Server listening on {}", addr);

    start_udp_broadcast(host, port, web_port);

    let (broadcast_tx, _) = broadcast::channel::<(u32, Vec<u8>)>(1000);

    let state = Arc::new(AppState {
        clients: Arc::new(RwLock::new(HashMap::new())),
        audio_capture: Arc::new(Mutex::new(None)),
        broadcast_tx: broadcast_tx.clone(),
    });

    let web_state = state.clone();
    tokio::spawn(async move {
        if let Err(e) = start_web_server(web_port, web_state).await {
            log::error!("Web server error: {}", e);
        }
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

    replace_audio_capture(&state, detected_rate).await.ok();

    server_handle.await?;
    Ok(())
}

async fn replace_audio_capture(state: &AppState, target_rate: u32) -> Result<()> {
    let mut guard = state.audio_capture.lock().await;

    if let Some(mut old) = guard.take() {
        old.broadcast_handle.abort();
        old.capture.stop();
        old.capture.wait_stopped().await;
    }

    ACTUAL_SAMPLE_RATE.store(target_rate, std::sync::atomic::Ordering::Relaxed);

    let (audio_tx, mut audio_rx) = tokio::sync::mpsc::channel::<(u32, Vec<u8>)>(200);
    let broadcast_tx = state.broadcast_tx.clone();

    match AudioCapture::new(audio_tx, target_rate) {
        Ok(capture) => {
            let handle = tokio::spawn(async move {
                while let Some(item) = audio_rx.recv().await {
                    let _ = broadcast_tx.send(item);
                }
            });

            *guard = Some(AudioCaptureState {
                capture,
                broadcast_handle: handle,
            });
            log::info!("Audio capture started at {}Hz", target_rate);
            Ok(())
        }
        Err(e) => {
            log::error!("Failed to start audio capture: {}", e);
            Err(e)
        }
    }
}

fn start_udp_broadcast(_host: &str, port: u16, web_port: u16) {
    let service_info = serde_json::json!({
        "name": "AudioRelay",
        "ws_port": port,
        "web_port": web_port,
    })
    .to_string();

    std::thread::spawn(move || {
        let socket = match std::net::UdpSocket::bind("0.0.0.0:8082") {
            Ok(s) => s,
            Err(e) => {
                log::error!("Failed to bind UDP socket on 8082: {}", e);
                return;
            }
        };
        socket.set_broadcast(true).expect("Failed to set broadcast");

        log::info!("UDP discovery listener on port 8082");

        let mut buf = [0u8; 1024];
        loop {
            match socket.recv_from(&mut buf) {
                Ok((len, src)) => {
                    let msg = String::from_utf8_lossy(&buf[..len]);
                    log::debug!("Discovery request from {}: {}", src, msg);
                    let _ = socket.send_to(service_info.as_bytes(), src);
                }
                Err(e) => {
                    log::debug!("UDP recv error: {}", e);
                }
            }
        }
    });
}

type WsSplitSink =
    futures_util::stream::SplitSink<tokio_tungstenite::WebSocketStream<TcpStream>, WsMessage>;

async fn handle_connection(
    stream: TcpStream,
    addr: SocketAddr,
    broadcast_tx: broadcast::Sender<(u32, Vec<u8>)>,
    clients: ClientInfo,
    _state: Arc<AppState>,
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
                let msg_len = text.len() as u64;
                if let Some(ref id) = client_id {
                    if let Some(entry) = clients.write().await.get_mut(id) {
                        entry.bytes_up += msg_len;
                    }
                }
                match Message::from_json_bytes(text.as_bytes()) {
                    Some(Message::Hello {
                        client_id: id,
                        mode,
                        sample_rate,
                        channels,
                    }) => {
                        log::info!(
                            "Client {} connected, mode: {:?}, {}Hz, {}ch",
                            id,
                            mode,
                            sample_rate,
                            channels
                        );

                        let actual_rate =
                            ACTUAL_SAMPLE_RATE.load(std::sync::atomic::Ordering::Relaxed);

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
                            ClientEntry::new(id.clone(), addr, mode.clone(), ws_sender.clone()),
                        );

                        client_id = Some(id.clone());
                        client_mode = Some(mode.clone());

                        match mode {
                            StreamMode::Speaker => {
                                let mut broadcast_rx = broadcast_tx.subscribe();
                                let speaker_sender = ws_sender.clone();
                                let client_id_for_task = id.clone();
                                let clients_clone = clients.clone();

                                tokio::spawn(async move {
                                    let mut seq: u64 = 0;
                                    loop {
                                        match broadcast_rx.recv().await {
                                            Ok((rate, data)) => {
                                                let frame = AudioFrame {
                                                    sequence: seq,
                                                    timestamp: timestamp_ms(),
                                                    sample_rate: rate,
                                                    data,
                                                };
                                                seq += 1;
                                                let binary = encode_audio_frame(&frame);
                                                let bytes_len = binary.len() as u64;
                                                let mut guard = speaker_sender.lock().await;
                                                if let Some(sender) = guard.as_mut() {
                                                    if sender
                                                        .send(WsMessage::Binary(binary))
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
                                                if let Some(entry) = clients_clone
                                                    .write()
                                                    .await
                                                    .get_mut(&client_id_for_task)
                                                {
                                                    entry.bytes_down += bytes_len;
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
                    Some(Message::AudioData {
                        data, sample_rate, ..
                    }) => {
                        if client_mode == Some(StreamMode::Microphone) {
                            let _ = broadcast_tx.send((sample_rate, data));
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
            Ok(WsMessage::Binary(data)) => {
                let data_len = data.len() as u64;
                if let Some(ref id) = client_id {
                    if let Some(entry) = clients.write().await.get_mut(id) {
                        entry.bytes_up += data_len;
                    }
                }
                if client_mode == Some(StreamMode::Microphone) {
                    if let Some(frame) = decode_audio_frame(&data) {
                        let _ = broadcast_tx.send((frame.sample_rate, frame.data));
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

async fn start_web_server(port: u16, state: Arc<AppState>) -> Result<()> {
    let app = Router::new()
        .route("/", get(index_page))
        .route("/api/clients", get(get_clients))
        .route(
            "/api/sample-rate",
            get(get_sample_rate).post(set_sample_rate),
        )
        .with_state(state);

    let addr = format!("0.0.0.0:{}", port);
    log::info!("Management UI at http://{}", addr);

    let listener = tokio::net::TcpListener::bind(&addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

async fn index_page() -> axum::response::Html<&'static str> {
    axum::response::Html(include_str!("web/index.html"))
}

async fn get_clients(AxumState(state): AxumState<Arc<AppState>>) -> Json<Value> {
    let mut clients = state.clients.write().await;
    let list: Vec<Value> = clients
        .values_mut()
        .map(|c| {
            c.update_speed();
            json!({
                "client_id": c.client_id,
                "addr": c.addr.to_string(),
                "mode": format!("{:?}", c.mode),
                "latency_ms": (c.latency_ms * 100.0).round() / 100.0,
                "connected_at": c.connected_at,
                "bytes_up": c.bytes_up,
                "bytes_down": c.bytes_down,
                "speed_up": (c.speed_up * 10.0).round() / 10.0,
                "speed_down": (c.speed_down * 10.0).round() / 10.0,
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

    if let Err(e) = replace_audio_capture(&state, rate).await {
        return Json(json!({ "ok": false, "error": format!("{}", e) }));
    }

    Json(json!({ "ok": true, "sample_rate": rate }))
}
