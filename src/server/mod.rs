use anyhow::Result;
use futures_util::{SinkExt, StreamExt};
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{broadcast, Mutex};
use tokio_tungstenite::accept_async;
use tungstenite::Message as WsMessage;

use crate::protocol::{Message, StreamMode, SAMPLE_RATE, CHANNELS};

pub async fn run_server(host: &str, port: u16) -> Result<()> {
    let addr = format!("{}:{}", host, port);
    let listener = TcpListener::bind(&addr).await?;
    log::info!("Server listening on {}", addr);

    let (broadcast_tx, _) = broadcast::channel::<Vec<u8>>(100);

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

    let server_handle = tokio::spawn(async move {
        loop {
            tokio::select! {
                accept = listener.accept() => {
                    match accept {
                        Ok((stream, addr)) => {
                            log::info!("New connection from {}", addr);
                            let broadcast = server_broadcast.clone();
                            tokio::spawn(handle_connection(stream, addr, broadcast));
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

    server_handle.await?;
    Ok(())
}

async fn handle_connection(
    stream: TcpStream,
    addr: SocketAddr,
    broadcast_tx: broadcast::Sender<Vec<u8>>,
) {
    let ws_stream = match accept_async(stream).await {
        Ok(ws) => ws,
        Err(e) => {
            log::error!("WebSocket handshake failed for {}: {}", addr, e);
            return;
        }
    };

    log::info!("WebSocket connection established with {}", addr);

    let (ws_sender, mut ws_receiver) = ws_stream.split();
    let ws_sender = Arc::new(Mutex::new(ws_sender));

    let mut client_mode: Option<StreamMode> = None;

    while let Some(msg) = ws_receiver.next().await {
        match msg {
            Ok(WsMessage::Text(text)) => {
                match Message::from_json_bytes(text.as_bytes()) {
                    Some(Message::Hello {
                        client_id,
                        mode,
                        sample_rate,
                        channels,
                    }) => {
                        log::info!(
                            "Client {} connected, mode: {:?}, {}Hz, {}ch",
                            client_id,
                            mode,
                            sample_rate,
                            channels
                        );

                        let session_id = uuid::Uuid::new_v4().to_string();
                        let ack = Message::HelloAck {
                            session_id: session_id.clone(),
                            sample_rate: SAMPLE_RATE,
                            channels: CHANNELS,
                        };

                        if let Ok(ack_bytes) = serde_json::to_string(&ack) {
                            let mut sender = ws_sender.lock().await;
                            let _ = sender.send(WsMessage::Text(ack_bytes)).await;
                        }

                        match mode {
                            StreamMode::Speaker => {
                                let mut broadcast_rx = broadcast_tx.subscribe();
                                client_mode = Some(StreamMode::Speaker);
                                let speaker_sender = ws_sender.clone();

                                tokio::spawn(async move {
                                    let mut seq: u64 = 0;
                                    let mut sender = speaker_sender.lock().await;
                                    loop {
                                        match broadcast_rx.recv().await {
                                            Ok(data) => {
                                                let msg = Message::AudioData {
                                                    sequence: seq,
                                                    data,
                                                };
                                                seq += 1;
                                                if let Ok(json) = serde_json::to_string(&msg) {
                                                    if sender
                                                        .send(WsMessage::Text(json))
                                                        .await
                                                        .is_err()
                                                    {
                                                        break;
                                                    }
                                                }
                                            }
                                            Err(broadcast::error::RecvError::Lagged(n)) => {
                                                log::warn!("Speaker client lagged, skipped {} messages", n);
                                            }
                                            Err(broadcast::error::RecvError::Closed) => {
                                                break;
                                            }
                                        }
                                    }
                                });

                                log::info!("Client {} set as speaker", client_id);
                            }
                            StreamMode::Microphone => {
                                client_mode = Some(StreamMode::Microphone);
                                log::info!("Client {} set as microphone", client_id);
                            }
                        }
                    }
                    Some(Message::AudioData { sequence, data }) => {
                        if client_mode == Some(StreamMode::Microphone) {
                            log::debug!(
                                "Received audio, seq: {}, {} bytes",
                                sequence,
                                data.len()
                            );

                            let _ = broadcast_tx.send(data);

                            let ack = Message::AudioDataAck { sequence };
                            if let Ok(ack_bytes) = serde_json::to_string(&ack) {
                                let mut sender = ws_sender.lock().await;
                                let _ = sender.send(WsMessage::Text(ack_bytes)).await;
                            }
                        }
                    }
                    Some(Message::Ping { timestamp }) => {
                        let pong = Message::Pong { timestamp };
                        if let Ok(json) = serde_json::to_string(&pong) {
                            let mut sender = ws_sender.lock().await;
                            let _ = sender.send(WsMessage::Text(json)).await;
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

    log::info!("Connection closed for {}", addr);
}
