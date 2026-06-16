use serde::{Deserialize, Serialize};

pub const SAMPLE_RATE: u32 = 44100;
pub const CHANNELS: u16 = 1;

const AUDIO_FRAME_HEADER_SIZE: usize = 20;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Message {
    Hello {
        client_id: String,
        mode: StreamMode,
        sample_rate: u32,
        channels: u16,
    },
    HelloAck {
        session_id: String,
        sample_rate: u32,
        channels: u16,
    },
    AudioData {
        sequence: u64,
        timestamp: u64,
        sample_rate: u32,
        data: Vec<u8>,
    },
    Ping {
        timestamp: u64,
    },
    Pong {
        timestamp: u64,
    },
    LatencyReport {
        latency_ms: f64,
    },
    StreamStart {
        direction: StreamDirection,
    },
    StreamStop,
    Error {
        code: u16,
        message: String,
    },
}

#[derive(Debug, Clone)]
pub struct AudioFrame {
    pub sequence: u64,
    pub timestamp: u64,
    pub sample_rate: u32,
    pub data: Vec<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum StreamMode {
    Speaker,
    Microphone,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum StreamDirection {
    PhoneToPc,
    PcToPhone,
}

impl Message {
    pub fn from_json_bytes(bytes: &[u8]) -> Option<Self> {
        serde_json::from_slice(bytes).ok()
    }
}

pub fn encode_audio_frame(frame: &AudioFrame) -> Vec<u8> {
    let mut buf = Vec::with_capacity(AUDIO_FRAME_HEADER_SIZE + frame.data.len());
    buf.extend_from_slice(&frame.sequence.to_le_bytes());
    buf.extend_from_slice(&frame.timestamp.to_le_bytes());
    buf.extend_from_slice(&frame.sample_rate.to_le_bytes());
    buf.extend_from_slice(&frame.data);
    buf
}

pub fn decode_audio_frame(bytes: &[u8]) -> Option<AudioFrame> {
    if bytes.len() < AUDIO_FRAME_HEADER_SIZE {
        return None;
    }
    let sequence = u64::from_le_bytes(bytes[0..8].try_into().ok()?);
    let timestamp = u64::from_le_bytes(bytes[8..16].try_into().ok()?);
    let sample_rate = u32::from_le_bytes(bytes[16..20].try_into().ok()?);
    let data = bytes[AUDIO_FRAME_HEADER_SIZE..].to_vec();
    Some(AudioFrame {
        sequence,
        timestamp,
        sample_rate,
        data,
    })
}

pub fn timestamp_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
