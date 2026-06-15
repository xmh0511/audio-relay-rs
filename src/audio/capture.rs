use anyhow::{Context, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{Stream, StreamConfig};
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;



pub struct AudioCapture {
    _stream: Stream,
    sample_rate: u32,
    channels: u16,
}

impl AudioCapture {
    pub fn new(tx: mpsc::Sender<Vec<u8>>) -> Result<Self> {
        let host = cpal::default_host();
        let device = host
            .default_input_device()
            .context("No input device available")?;

        let supported = device
            .default_input_config()
            .context("No default input config")?;

        let sample_rate = supported.sample_rate().0;
        let channels = supported.channels();

        log::info!(
            "Capture device: {}, {}Hz, {} ch",
            device.name().unwrap_or_default(),
            sample_rate,
            channels
        );

        let config = StreamConfig {
            channels,
            sample_rate: cpal::SampleRate(sample_rate),
            buffer_size: cpal::BufferSize::Default,
        };

        let tx = Arc::new(Mutex::new(tx));

        let stream = device
            .build_input_stream(
                &config,
                move |data: &[f32], _: &cpal::InputCallbackInfo| {
                    let pcm_data = float_to_i16_bytes(data);
                    if let Ok(sender) = tx.lock() {
                        let _ = sender.try_send(pcm_data);
                    }
                },
                |err| {
                    log::error!("Capture error: {}", err);
                },
                None,
            )
            .context("Failed to build input stream")?;

        stream.play().context("Failed to start capture stream")?;

        Ok(Self {
            _stream: stream,
            sample_rate,
            channels,
        })
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    pub fn channels(&self) -> u16 {
        self.channels
    }
}

fn float_to_i16_bytes(samples: &[f32]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(samples.len() * 2);
    for &s in samples {
        let clamped = s.clamp(-1.0, 1.0);
        let val = (clamped * i16::MAX as f32) as i16;
        bytes.extend_from_slice(&val.to_le_bytes());
    }
    bytes
}
