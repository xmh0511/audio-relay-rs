use anyhow::{Context, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{Stream, StreamConfig};

pub struct AudioPlayback {
    _stream: Stream,
}

impl AudioPlayback {
    pub fn new(rx: crossbeam_channel::Receiver<Vec<u8>>) -> Result<Self> {
        let host = cpal::default_host();
        let device = host
            .default_output_device()
            .context("No output device available")?;

        let supported = device
            .default_output_config()
            .context("No default output config")?;

        let sample_rate = supported.sample_rate().0;
        let channels = supported.channels();

        log::info!(
            "Playback device: {}, {}Hz, {} ch",
            device.name().unwrap_or_default(),
            sample_rate,
            channels
        );

        let config = StreamConfig {
            channels,
            sample_rate: cpal::SampleRate(sample_rate),
            buffer_size: cpal::BufferSize::Default,
        };

        let stream = device
            .build_output_stream(
                &config,
                move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                    match rx.try_recv() {
                        Ok(pcm_bytes) => {
                            let samples = i16_bytes_to_float(&pcm_bytes);
                            let len = data.len().min(samples.len());
                            data[..len].copy_from_slice(&samples[..len]);
                            if len < data.len() {
                                data[len..].fill(0.0);
                            }
                        }
                        Err(_) => {
                            data.fill(0.0);
                        }
                    }
                },
                |err| {
                    log::error!("Playback error: {}", err);
                },
                None,
            )
            .context("Failed to build output stream")?;

        stream.play().context("Failed to start playback stream")?;

        Ok(Self { _stream: stream })
    }
}

fn i16_bytes_to_float(bytes: &[u8]) -> Vec<f32> {
    let mut samples = Vec::with_capacity(bytes.len() / 2);
    for chunk in bytes.chunks_exact(2) {
        let val = i16::from_le_bytes([chunk[0], chunk[1]]);
        samples.push(val as f32 / i16::MAX as f32);
    }
    samples
}
