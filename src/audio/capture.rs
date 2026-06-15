use anyhow::Result;
use std::sync::Arc;
use tokio::sync::{mpsc, oneshot};

struct StopNotifier(Option<oneshot::Sender<()>>);

impl StopNotifier {
    fn new(tx: oneshot::Sender<()>) -> Self {
        Self(Some(tx))
    }

    fn done(mut self) {
        if let Some(tx) = self.0.take() {
            let _ = tx.send(());
        }
    }
}

impl Drop for StopNotifier {
    fn drop(&mut self) {
        if let Some(tx) = self.0.take() {
            let _ = tx.send(());
        }
    }
}

pub struct AudioCapture {
    stop_flag: Arc<std::sync::atomic::AtomicBool>,
    stopped_rx: Option<oneshot::Receiver<()>>,
    _handle: Option<std::thread::JoinHandle<()>>,
}

impl AudioCapture {
    pub fn new(tx: mpsc::Sender<Vec<u8>>) -> Result<Self> {
        let stop_flag = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let flag = stop_flag.clone();
        let (stopped_tx, stopped_rx) = oneshot::channel::<()>();

        let handle = std::thread::Builder::new()
            .name("audio-capture".to_string())
            .spawn(move || {
                #[cfg(target_os = "windows")]
                capture_windows(tx, flag, stopped_tx);
                #[cfg(not(target_os = "windows"))]
                capture_cpal(tx, flag, stopped_tx);
            })?;

        Ok(Self {
            stop_flag,
            stopped_rx: Some(stopped_rx),
            _handle: Some(handle),
        })
    }

    pub fn stop(&self) {
        self.stop_flag
            .store(true, std::sync::atomic::Ordering::Relaxed);
    }

    pub async fn wait_stopped(&mut self) {
        if let Some(rx) = self.stopped_rx.take() {
            let _ = rx.await;
        }
    }
}

#[cfg(target_os = "windows")]
fn capture_windows(
    tx: mpsc::Sender<Vec<u8>>,
    stop_flag: Arc<std::sync::atomic::AtomicBool>,
    stopped_tx: oneshot::Sender<()>,
) {
    use std::collections::VecDeque;
    use wasapi::*;

    let notifier = StopNotifier::new(stopped_tx);

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

    log::info!("Loopback capture started");

    loop {
        if stop_flag.load(std::sync::atomic::Ordering::Relaxed) {
            log::info!("Loopback capture stopping");
            let _ = audio_client.stop_stream();
            break;
        }

        let _ = capture_client.read_from_device_to_deque(&mut sample_queue);

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

            let pcm_chunk = convert_to_i16_pcm(&raw_chunk, bits_per_sample, device_channels);

            if tx.try_send(pcm_chunk).is_err() {
                log::warn!("Audio channel full, dropping frame");
            }
        }

        h_event.wait_for_event(10000).ok();
    }

    log::info!("Loopback capture stopped");
    notifier.done();
}

#[cfg(target_os = "windows")]
fn convert_to_i16_pcm(raw: &[u8], bits_per_sample: usize, channels: usize) -> Vec<u8> {
    match bits_per_sample {
        32 => {
            let float_count = raw.len() / 4;
            let floats =
                unsafe { std::slice::from_raw_parts(raw.as_ptr() as *const f32, float_count) };

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
        _ => raw.to_vec(),
    }
}

#[cfg(not(target_os = "windows"))]
fn capture_cpal(
    tx: mpsc::Sender<Vec<u8>>,
    stop_flag: Arc<std::sync::atomic::AtomicBool>,
    stopped_tx: oneshot::Sender<()>,
) {
    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

    let notifier = StopNotifier::new(stopped_tx);

    let host = cpal::default_host();

    let device = match host.default_input_device() {
        Some(d) => d,
        None => {
            log::error!("No input device available");
            return;
        }
    };

    log::info!("Capture device: {}", device.name().unwrap_or_default());

    let supported = match device.default_input_config() {
        Ok(c) => c,
        Err(e) => {
            log::error!("No default input config: {}", e);
            return;
        }
    };

    let sample_rate = supported.sample_rate().0;
    let channels = supported.channels();

    crate::server::ACTUAL_SAMPLE_RATE.store(
        sample_rate,
        std::sync::atomic::Ordering::Relaxed,
    );

    let config = cpal::StreamConfig {
        channels,
        sample_rate: cpal::SampleRate(sample_rate),
        buffer_size: cpal::BufferSize::Default,
    };

    let stream = match device.build_input_stream(
        &config,
        move |data: &[f32], _: &cpal::InputCallbackInfo| {
            let pcm_data = float_to_i16_bytes(data);
            if tx.try_send(pcm_data).is_err() {
                log::warn!("Audio channel full, dropping frame");
            }
        },
        |err| {
            log::error!("Capture error: {}", err);
        },
        None,
    ) {
        Ok(s) => s,
        Err(e) => {
            log::error!("Failed to build input stream: {}", e);
            return;
        }
    };

    if stream.play().is_err() {
        log::error!("Failed to start capture stream");
        return;
    }

    log::info!("Audio capture started: {}Hz, {} ch", sample_rate, channels);

    while !stop_flag.load(std::sync::atomic::Ordering::Relaxed) {
        std::thread::sleep(std::time::Duration::from_millis(100));
    }

    log::info!("Capture stopping");
    drop(stream);
    log::info!("Capture stopped");
    notifier.done();
}

#[cfg(not(target_os = "windows"))]
fn float_to_i16_bytes(samples: &[f32]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(samples.len() * 2);
    for &s in samples {
        let clamped = s.clamp(-1.0, 1.0);
        let val = (clamped * i16::MAX as f32) as i16;
        bytes.extend_from_slice(&val.to_le_bytes());
    }
    bytes
}
