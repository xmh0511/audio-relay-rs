use anyhow::{Context, Result};
use futures_util::{SinkExt, StreamExt};
use tokio::sync::mpsc;
use tokio_tungstenite::connect_async;
use tungstenite::Message as WsMessage;

use crate::audio::capture::AudioCapture;
use crate::audio::playback::AudioPlayback;
use crate::protocol::{Message, StreamMode, CHANNELS, SAMPLE_RATE};

pub async fn run_client(server_host: &str, port: u16, as_mic: bool) -> Result<()> {
    let url = format!("ws://{}:{}", server_host, port);
    log::info!("Connecting to {}", url);

    let (ws_stream, _) = connect_async(&url)
        .await
        .context("Failed to connect to server")?;

    log::info!("Connected to server");

    let (mut ws_sender, mut ws_receiver) = ws_stream.split();

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
    ws_sender.send(WsMessage::Text(hello_json)).await?;

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
            let (tx, mut rx) = mpsc::channel::<(u32, Vec<u8>)>(50);

            let _capture = AudioCapture::new(tx, crate::protocol::SAMPLE_RATE)
                .context("Failed to initialize audio capture")?;

            let mut audio_seq: u64 = 0;

            let send_handle = tokio::spawn(async move {
                while let Some((_rate, data)) = rx.recv().await {
                    let msg = Message::AudioData {
                        sequence: audio_seq,
                        timestamp: crate::protocol::timestamp_ms(),
                        sample_rate: crate::protocol::SAMPLE_RATE,
                        data,
                    };
                    audio_seq += 1;
                    if let Ok(json) = serde_json::to_string(&msg) {
                        if ws_sender.send(WsMessage::Text(json)).await.is_err() {
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

            let (audio_tx, audio_rx) = mpsc::channel::<Vec<u8>>(50);
            let _playback =
                AudioPlayback::new(audio_rx).context("Failed to initialize audio playback")?;

            let audio_tx = audio_tx;

            let mut current_sample_rate = crate::protocol::SAMPLE_RATE;
            let recv_handle = tokio::spawn(async move {
                while let Some(msg) = ws_receiver.next().await {
                    match msg {
                        Ok(WsMessage::Text(text)) => {
                            match Message::from_json_bytes(text.as_bytes()) {
                                Some(Message::AudioData {
                                    data, sample_rate, ..
                                }) => {
                                    if sample_rate != current_sample_rate {
                                        log::info!(
                                            "Sample rate changed: {}Hz -> {}Hz",
                                            current_sample_rate,
                                            sample_rate
                                        );
                                        current_sample_rate = sample_rate;
                                    }
                                    let _ = audio_tx.send(data).await;
                                }
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

            let ping_handle = tokio::spawn(async move {
                let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(5));
                loop {
                    interval.tick().await;
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
