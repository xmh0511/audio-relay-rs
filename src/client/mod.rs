use anyhow::{Context, Result};
use futures_util::{SinkExt, StreamExt};
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio_tungstenite::connect_async;
use tungstenite::Message as WsMessage;

use crate::audio::capture::AudioCapture;
use crate::audio::playback::AudioPlayback;
use crate::protocol::{
    decode_audio_frame, encode_audio_frame, AudioFrame, Message, StreamMode, CHANNELS, SAMPLE_RATE,
};

type WsSplitSink = futures_util::stream::SplitSink<
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
    WsMessage,
>;

pub async fn run_client(server_host: &str, port: u16, as_mic: bool) -> Result<()> {
    let url = format!("ws://{}:{}", server_host, port);
    log::info!("Connecting to {}", url);

    let (ws_stream, _) = connect_async(&url)
        .await
        .context("Failed to connect to server")?;

    log::info!("Connected to server");

    let (ws_sender_raw, mut ws_receiver) = ws_stream.split();
    let ws_sender: Arc<Mutex<Option<WsSplitSink>>> = Arc::new(Mutex::new(Some(ws_sender_raw)));

    let client_id = uuid::Uuid::new_v4().to_string();
    let mode = if as_mic {
        StreamMode::Microphone
    } else {
        StreamMode::Speaker
    };

    let hello = Message::Hello {
        client_id: client_id.clone(),
        mode: mode.clone(),
        sample_rate: SAMPLE_RATE,
        channels: CHANNELS,
    };

    let hello_json = serde_json::to_string(&hello)?;
    {
        let mut guard = ws_sender.lock().await;
        if let Some(sender) = guard.as_mut() {
            sender.send(WsMessage::Text(hello_json)).await?;
        }
    }

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

    match mode {
        StreamMode::Microphone => {
            log::info!("Starting in microphone mode");
            let (tx, mut rx) = tokio::sync::mpsc::channel::<(u32, Vec<u8>)>(50);

            let _capture = AudioCapture::new(tx).context("Failed to initialize audio capture")?;

            let mut audio_seq: u64 = 0;

            let send_handle = tokio::spawn(async move {
                while let Some((_rate, data)) = rx.recv().await {
                    let frame = AudioFrame {
                        sequence: audio_seq,
                        timestamp: crate::protocol::timestamp_ms(),
                        sample_rate: crate::protocol::SAMPLE_RATE,
                        data,
                    };
                    audio_seq += 1;
                    let binary = encode_audio_frame(&frame);
                    let mut guard = ws_sender.lock().await;
                    if let Some(sender) = guard.as_mut() {
                        if sender.send(WsMessage::Binary(binary)).await.is_err() {
                            break;
                        }
                    }
                }
            });

            tokio::select! {
                _ = ws_receiver.next() => {}
                _ = &mut shutdown_rx => {
                    log::info!("Shutting down...");
                }
            }

            send_handle.abort();
        }
        StreamMode::Speaker => {
            log::info!("Starting in speaker mode");

            let (audio_tx, audio_rx) = crossbeam_channel::bounded::<Vec<u8>>(50);
            let _playback =
                AudioPlayback::new(audio_rx).context("Failed to initialize audio playback")?;

            let mut current_sample_rate = crate::protocol::SAMPLE_RATE;
            let recv_handle = tokio::spawn(async move {
                while let Some(msg) = ws_receiver.next().await {
                    match msg {
                        Ok(WsMessage::Binary(data)) => {
                            if let Some(frame) = decode_audio_frame(&data) {
                                if frame.sample_rate != current_sample_rate {
                                    log::info!(
                                        "Sample rate changed: {}Hz -> {}Hz",
                                        current_sample_rate,
                                        frame.sample_rate
                                    );
                                    current_sample_rate = frame.sample_rate;
                                }
                                let _ = audio_tx.send(frame.data);
                            }
                        }
                        Ok(WsMessage::Text(text)) => {
                            match Message::from_json_bytes(text.as_bytes()) {
                                Some(Message::Pong { .. }) => {}
                                _ => {}
                            }
                        }
                        Ok(WsMessage::Close(_)) => {
                            log::info!("Server closed connection");
                            break;
                        }
                        Err(e) => {
                            log::error!("WebSocket error: {}", e);
                            break;
                        }
                        _ => {}
                    }
                }
            });

            let ping_sender = ws_sender.clone();
            let ping_handle = tokio::spawn(async move {
                let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(5));
                loop {
                    interval.tick().await;
                    let ping = Message::Ping {
                        timestamp: crate::protocol::timestamp_ms(),
                    };
                    if let Ok(json) = serde_json::to_string(&ping) {
                        let mut guard = ping_sender.lock().await;
                        if let Some(sender) = guard.as_mut() {
                            if sender.send(WsMessage::Text(json)).await.is_err() {
                                break;
                            }
                        }
                    }
                }
            });

            tokio::select! {
                _ = recv_handle => {}
                _ = &mut shutdown_rx => {
                    log::info!("Shutting down...");
                }
            }

            ping_handle.abort();
        }
    }

    Ok(())
}
