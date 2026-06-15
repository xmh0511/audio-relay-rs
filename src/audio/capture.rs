use anyhow::Result;
use std::collections::VecDeque;
use tokio::sync::mpsc;
use wasapi::*;

pub struct AudioCapture {
    _handle: std::thread::JoinHandle<()>,
    pub sample_rate: u32,
    pub channels: u16,
}

impl AudioCapture {
    pub fn new(tx: mpsc::Sender<Vec<u8>>) -> Result<Self> {
        let sample_rate = 44100u32;
        let channels = 1u16;

        let handle = std::thread::Builder::new()
            .name("wasapi-loopback".to_string())
            .spawn(move || {
                let _ = initialize_mta();

                let device = match get_default_device(&Direction::Render) {
                    Ok(d) => d,
                    Err(e) => {
                        log::error!("No render device: {:?}", e);
                        return;
                    }
                };

                log::info!("Output device: {:?}", device.get_friendlyname());

                let mut audio_client = match device.get_iaudioclient() {
                    Ok(c) => c,
                    Err(e) => {
                        log::error!("Failed to get audio client: {:?}", e);
                        return;
                    }
                };

                let mix_format = match audio_client.get_mixformat() {
                    Ok(f) => {
                        log::info!(
                            "Device mix format: {}Hz, {} ch, {} bit",
                            f.get_samplespersec(),
                            f.get_nchannels(),
                            f.get_bitspersample(),
                        );
                        f
                    }
                    Err(e) => {
                        log::error!("Failed to get mix format: {:?}", e);
                        return;
                    }
                };

                let (def_time, _min_time) = match audio_client.get_periods() {
                    Ok(p) => p,
                    Err(e) => {
                        log::error!("Failed to get periods: {:?}", e);
                        return;
                    }
                };

                if let Err(e) = audio_client.initialize_client(
                    &mix_format,
                    def_time,
                    &Direction::Capture,
                    &ShareMode::Shared,
                    true,
                ) {
                    log::error!("Failed to init loopback client: {:?}", e);
                    return;
                }

                let h_event = match audio_client.set_get_eventhandle() {
                    Ok(h) => h,
                    Err(e) => {
                        log::error!("Failed to set event: {:?}", e);
                        return;
                    }
                };

                let buffer_frame_count = match audio_client.get_bufferframecount() {
                    Ok(c) => c,
                    Err(e) => {
                        log::error!("Failed to get buffer size: {:?}", e);
                        return;
                    }
                };

                let capture_client = match audio_client.get_audiocaptureclient() {
                    Ok(c) => c,
                    Err(e) => {
                        log::error!("Failed to get capture client: {:?}", e);
                        return;
                    }
                };

                let blockalign = mix_format.get_blockalign() as usize;
                let bits_per_sample = mix_format.get_bitspersample() as usize;
                let bytes_per_sample = bits_per_sample / 8;
                let device_channels = mix_format.get_nchannels() as usize;
                let device_sample_rate = mix_format.get_samplespersec();

                log::info!(
                    "Loopback: {}Hz, {} ch, {}bit, blockalign={}, buffer={}",
                    device_sample_rate,
                    device_channels,
                    bits_per_sample,
                    blockalign,
                    buffer_frame_count
                );

                crate::server::ACTUAL_SAMPLE_RATE.store(
                    device_sample_rate,
                    std::sync::atomic::Ordering::Relaxed,
                );

                let mut sample_queue: VecDeque<u8> =
                    VecDeque::with_capacity(blockalign * buffer_frame_count as usize * 4);

                if let Err(e) = audio_client.start_stream() {
                    log::error!("Failed to start loopback: {:?}", e);
                    return;
                }

                log::info!("Loopback capture started, waiting for audio...");

                let mut total_bytes: u64 = 0;

                loop {
                    match capture_client.read_from_device_to_deque(&mut sample_queue) {
                        Ok(_) => {}
                        Err(_) => {}
                    }

                    let frame_size = blockalign;
                    let frames_available = sample_queue.len() / frame_size;

                    if frames_available > 0 {
                        let frames_to_send = frames_available.min(4096);
                        let bytes_to_send = frames_to_send * frame_size;
                        let mut raw_chunk = vec![0u8; bytes_to_send];
                        for byte in raw_chunk.iter_mut() {
                            if let Some(b) = sample_queue.pop_front() {
                                *byte = b;
                            }
                        }

                        let pcm_chunk = convert_to_i16_pcm(
                            &raw_chunk,
                            bits_per_sample,
                            device_channels,
                        );

                        total_bytes += pcm_chunk.len() as u64;
                        if total_bytes % (44100 * 2) < pcm_chunk.len() as u64 {
                            log::info!("Captured {:.1}s of audio", total_bytes as f64 / 44100.0 / 2.0);
                        }

                        if tx.try_send(pcm_chunk).is_err() {
                            log::warn!("Audio channel full, dropping frame");
                        }
                    }

                    h_event.wait_for_event(10000).ok();
                }
            })?;

        Ok(Self {
            _handle: handle,
            sample_rate,
            channels,
        })
    }
}

fn convert_to_i16_pcm(raw: &[u8], bits_per_sample: usize, channels: usize) -> Vec<u8> {
    match bits_per_sample {
        32 => {
            let float_count = raw.len() / 4;
            let floats = unsafe { std::slice::from_raw_parts(raw.as_ptr() as *const f32, float_count) };

            if channels == 2 {
                let mono_count = float_count / 2;
                let mut output = Vec::with_capacity(mono_count * 2);
                for i in 0..mono_count {
                    let left = floats[i * 2];
                    let right = floats[i * 2 + 1];
                    let mixed = (left + right) / 2.0;
                    let sample = (mixed.clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
                    output.extend_from_slice(&sample.to_le_bytes());
                }
                output
            } else {
                let mut output = Vec::with_capacity(float_count * 2);
                for &f in floats {
                    let sample = (f.clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
                    output.extend_from_slice(&sample.to_le_bytes());
                }
                output
            }
        }
        16 => {
            if channels == 2 {
                let sample_count = raw.len() / 2;
                let samples = unsafe {
                    std::slice::from_raw_parts(raw.as_ptr() as *const i16, sample_count)
                };
                let mono_count = sample_count / 2;
                let mut output = Vec::with_capacity(mono_count * 2);
                for i in 0..mono_count {
                    let left = samples[i * 2] as i32;
                    let right = samples[i * 2 + 1] as i32;
                    let mixed = ((left + right) / 2) as i16;
                    output.extend_from_slice(&mixed.to_le_bytes());
                }
                output
            } else {
                raw.to_vec()
            }
        }
        _ => {
            log::warn!("Unsupported bits_per_sample: {}, using raw", bits_per_sample);
            raw.to_vec()
        }
    }
}
