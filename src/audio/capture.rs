use anyhow::Result;
use std::sync::Arc;
use tokio::sync::{mpsc, oneshot};

pub fn detect_sample_rate() -> u32 {
    #[cfg(target_os = "windows")]
    {
        detect_sample_rate_windows()
    }
    #[cfg(not(target_os = "windows"))]
    {
        detect_sample_rate_cpal()
    }
}

#[cfg(target_os = "windows")]
fn detect_sample_rate_windows() -> u32 {
    use wasapi::*;

    let _ = initialize_mta();

    let device = match get_default_device(&Direction::Render) {
        Ok(d) => d,
        Err(e) => {
            log::warn!("Failed to get default device for rate detection: {:?}", e);
            return 44100;
        }
    };

    let audio_client = match device.get_iaudioclient() {
        Ok(c) => c,
        Err(e) => {
            log::warn!("Failed to get audio client for rate detection: {:?}", e);
            return 44100;
        }
    };

    let mix_format = match audio_client.get_mixformat() {
        Ok(f) => f,
        Err(e) => {
            log::warn!("Failed to get mix format: {:?}", e);
            return 44100;
        }
    };

    let rate = mix_format.get_samplespersec();
    log::info!("Detected device sample rate: {}Hz", rate);
    rate
}

#[cfg(not(target_os = "windows"))]
fn detect_sample_rate_cpal() -> u32 {
    use cpal::traits::{DeviceTrait, HostTrait};

    let host = cpal::default_host();

    let device = match host.default_output_device() {
        Some(d) => d,
        None => {
            log::warn!("No output device for rate detection");
            return 44100;
        }
    };

    let config = match device.default_output_config() {
        Ok(c) => c,
        Err(e) => {
            log::warn!("Failed to get default output config: {}", e);
            return 44100;
        }
    };

    let rate = config.sample_rate().0;
    log::info!("Detected device sample rate: {}Hz", rate);
    rate
}

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
    pub fn new(tx: mpsc::Sender<(u32, Vec<u8>)>, target_rate: u32) -> Result<Self> {
        let stop_flag = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let flag = stop_flag.clone();
        let (stopped_tx, stopped_rx) = oneshot::channel::<()>();

        let handle = std::thread::Builder::new()
            .name("audio-capture".to_string())
            .spawn(move || {
                #[cfg(target_os = "windows")]
                capture_windows(tx, flag, stopped_tx, target_rate);
                #[cfg(not(target_os = "windows"))]
                capture_cpal(tx, flag, stopped_tx, target_rate);
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
    tx: mpsc::Sender<(u32, Vec<u8>)>,
    stop_flag: Arc<std::sync::atomic::AtomicBool>,
    stopped_tx: oneshot::Sender<()>,
    target_rate: u32,
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

    let mut resampler = if device_sample_rate != target_rate {
        log::info!("Resampling {}Hz -> {}Hz", device_sample_rate, target_rate);
        match crate::audio::resampler::AudioResampler::new(
            device_sample_rate,
            target_rate,
            device_channels,
        ) {
            Ok(r) => Some(r),
            Err(e) => {
                log::error!("Failed to create resampler: {}, sending at device rate", e);
                None
            }
        }
    } else {
        None
    };

    let mut sample_queue: VecDeque<u8> =
        VecDeque::with_capacity(blockalign * buffer_frame_count as usize * 4);

    if let Err(e) = audio_client.start_stream() {
        log::error!("Failed to start loopback: {:?}", e);
        return;
    }

    log::info!("Loopback capture started");

    let mut raw_chunk = vec![0u8; 4096 * blockalign];
    let mut pcm_output = Vec::with_capacity(4096 * blockalign);

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
            raw_chunk.truncate(bytes_to_send);
            raw_chunk.resize(bytes_to_send, 0);
            for byte in raw_chunk.iter_mut().take(bytes_to_send) {
                if let Some(b) = sample_queue.pop_front() {
                    *byte = b;
                }
            }

            let send_data;
            let send_rate;

            if bits_per_sample == 16 && device_channels == 1 {
                // Safety: WASAPI PCM data is little-endian (WAV/RIFF spec: "Data is stored
                // in little-endian byte order", see https://en.wikipedia.org/wiki/WAV).
                // Windows only runs on LE architectures, so i16 layout matches.
                let i16_slice = unsafe {
                    std::slice::from_raw_parts(raw_chunk.as_ptr() as *const i16, bytes_to_send / 2)
                };

                if let Some(ref mut resampler) = resampler {
                    match resampler.resample(i16_slice) {
                        Ok(resampled) => {
                            let mut bytes = Vec::with_capacity(resampled.len() * 2);
                            for s in resampled {
                                bytes.extend_from_slice(&s.to_le_bytes());
                            }
                            send_data = bytes;
                            send_rate = target_rate;
                        }
                        Err(e) => {
                            log::warn!("Resample error: {}", e);
                            send_data = raw_chunk[..bytes_to_send].to_vec();
                            send_rate = device_sample_rate;
                        }
                    }
                } else {
                    send_data = raw_chunk[..bytes_to_send].to_vec();
                    send_rate = device_sample_rate;
                }
            } else if bits_per_sample == 16 && device_channels == 2 {
                let sample_count = bytes_to_send / 2;
                // Safety: same as above — WAV/RIFF LE spec, matches i16 layout on Windows.
                let i16_slice = unsafe {
                    std::slice::from_raw_parts(raw_chunk.as_ptr() as *const i16, sample_count)
                };
                let mono_count = sample_count / 2;
                let mut mono_i16 = Vec::with_capacity(mono_count);
                for i in 0..mono_count {
                    let left = i16_slice[i * 2] as i32;
                    let right = i16_slice[i * 2 + 1] as i32;
                    mono_i16.push(((left + right) / 2) as i16);
                }

                if let Some(ref mut resampler) = resampler {
                    match resampler.resample(&mono_i16) {
                        Ok(resampled) => {
                            let mut bytes = Vec::with_capacity(resampled.len() * 2);
                            for s in resampled {
                                bytes.extend_from_slice(&s.to_le_bytes());
                            }
                            send_data = bytes;
                            send_rate = target_rate;
                        }
                        Err(e) => {
                            log::warn!("Resample error: {}", e);
                            let mut bytes = Vec::with_capacity(mono_i16.len() * 2);
                            for s in &mono_i16 {
                                bytes.extend_from_slice(&s.to_le_bytes());
                            }
                            send_data = bytes;
                            send_rate = device_sample_rate;
                        }
                    }
                } else {
                    let mut bytes = Vec::with_capacity(mono_i16.len() * 2);
                    for s in &mono_i16 {
                        bytes.extend_from_slice(&s.to_le_bytes());
                    }
                    send_data = bytes;
                    send_rate = device_sample_rate;
                }
            } else {
                pcm_output.clear();
                convert_to_i16_pcm(
                    &raw_chunk[..bytes_to_send],
                    bits_per_sample,
                    device_channels,
                    &mut pcm_output,
                );

                if let Some(ref mut resampler) = resampler {
                    let i16_data: Vec<i16> = pcm_output
                        .chunks_exact(2)
                        .map(|c| i16::from_le_bytes([c[0], c[1]]))
                        .collect();
                    match resampler.resample(&i16_data) {
                        Ok(resampled) => {
                            let mut bytes = Vec::with_capacity(resampled.len() * 2);
                            for s in resampled {
                                bytes.extend_from_slice(&s.to_le_bytes());
                            }
                            send_data = bytes;
                            send_rate = target_rate;
                        }
                        Err(e) => {
                            log::warn!("Resample error: {}", e);
                            send_data = pcm_output.clone();
                            send_rate = device_sample_rate;
                        }
                    }
                } else {
                    send_data = pcm_output.clone();
                    send_rate = device_sample_rate;
                }
            }

            if tx.try_send((send_rate, send_data)).is_err() {
                log::warn!("Audio channel full, dropping frame");
            }
        }

        h_event.wait_for_event(10000).ok();
    }

    log::info!("Loopback capture stopped");
    notifier.done();
}

#[cfg(target_os = "windows")]
fn convert_to_i16_pcm(raw: &[u8], bits_per_sample: usize, channels: usize, output: &mut Vec<u8>) {
    match bits_per_sample {
        32 => {
            let float_count = raw.len() / 4;
            let floats =
                unsafe { std::slice::from_raw_parts(raw.as_ptr() as *const f32, float_count) };

            if channels == 2 {
                let mono_count = float_count / 2;
                output.reserve(mono_count * 2 - output.len());
                for i in 0..mono_count {
                    let left = floats[i * 2];
                    let right = floats[i * 2 + 1];
                    let mixed = (left + right) / 2.0;
                    let sample = (mixed.clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
                    output.extend_from_slice(&sample.to_le_bytes());
                }
            } else {
                output.reserve(float_count * 2 - output.len());
                for &f in floats {
                    let sample = (f.clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
                    output.extend_from_slice(&sample.to_le_bytes());
                }
            }
        }
        16 => {
            if channels == 2 {
                let sample_count = raw.len() / 2;
                let samples =
                    unsafe { std::slice::from_raw_parts(raw.as_ptr() as *const i16, sample_count) };
                let mono_count = sample_count / 2;
                output.reserve(mono_count * 2 - output.len());
                for i in 0..mono_count {
                    let left = samples[i * 2] as i32;
                    let right = samples[i * 2 + 1] as i32;
                    let mixed = ((left + right) / 2) as i16;
                    output.extend_from_slice(&mixed.to_le_bytes());
                }
            } else {
                let sample_count = raw.len() / 2;
                let samples =
                    unsafe { std::slice::from_raw_parts(raw.as_ptr() as *const i16, sample_count) };
                output.reserve(sample_count * 2 - output.len());
                for &s in samples {
                    output.extend_from_slice(&s.to_le_bytes());
                }
            }
        }
        _ => {
            output.extend_from_slice(raw);
        }
    }
}

#[cfg(not(target_os = "windows"))]
fn capture_cpal(
    tx: mpsc::Sender<(u32, Vec<u8>)>,
    stop_flag: Arc<std::sync::atomic::AtomicBool>,
    stopped_tx: oneshot::Sender<()>,
    target_rate: u32,
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

    let device_rate = supported.sample_rate().0;
    let channels = supported.channels() as usize;

    let mut resampler = if device_rate != target_rate {
        log::info!("Resampling {}Hz -> {}Hz", device_rate, target_rate);
        match crate::audio::resampler::AudioResampler::new(device_rate, target_rate, channels) {
            Ok(r) => Some(r),
            Err(e) => {
                log::error!("Failed to create resampler: {}, sending at device rate", e);
                None
            }
        }
    } else {
        None
    };

    let (raw_tx, mut raw_rx) = std::sync::mpsc::sync_channel::<Vec<f32>>(10);

    let config = cpal::StreamConfig {
        channels: supported.channels(),
        sample_rate: cpal::SampleRate(device_rate),
        buffer_size: cpal::BufferSize::Default,
    };

    let stream = match device.build_input_stream(
        &config,
        move |data: &[f32], _: &cpal::InputCallbackInfo| {
            let _ = raw_tx.try_send(data.to_vec());
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

    log::info!("Audio capture started: {}Hz, {} ch", device_rate, channels);

    let mut pcm_output = Vec::with_capacity(4096 * channels * 2);

    while !stop_flag.load(std::sync::atomic::Ordering::Relaxed) {
        match raw_rx.recv_timeout(std::time::Duration::from_millis(100)) {
            Ok(float_data) => {
                pcm_output.clear();
                float_to_i16_bytes(&float_data, &mut pcm_output);
                let (send_rate, send_data) = if let Some(ref mut resampler) = resampler {
                    let i16_data: Vec<i16> = pcm_output
                        .chunks_exact(2)
                        .map(|c| i16::from_le_bytes([c[0], c[1]]))
                        .collect();
                    match resampler.resample(&i16_data) {
                        Ok(resampled) => {
                            let mut bytes = Vec::with_capacity(resampled.len() * 2);
                            for s in resampled {
                                bytes.extend_from_slice(&s.to_le_bytes());
                            }
                            (target_rate, bytes)
                        }
                        Err(e) => {
                            log::warn!("Resample error: {}", e);
                            (device_rate, pcm_output.clone())
                        }
                    }
                } else {
                    (device_rate, pcm_output.clone())
                };
                if tx.try_send((send_rate, send_data)).is_err() {
                    log::warn!("Audio channel full, dropping frame");
                }
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }

    log::info!("Capture stopped");
    drop(stream);
    notifier.done();
}

#[cfg(not(target_os = "windows"))]
fn float_to_i16_bytes(samples: &[f32], output: &mut Vec<u8>) {
    output.reserve(samples.len() * 2 - output.len());
    for &s in samples {
        let clamped = s.clamp(-1.0, 1.0);
        let val = (clamped * i16::MAX as f32) as i16;
        output.extend_from_slice(&val.to_le_bytes());
    }
}
