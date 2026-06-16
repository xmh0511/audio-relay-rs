use serde::{Deserialize, Serialize};

pub const SAMPLE_RATE: u32 = 44100;
pub const CHANNELS: u16 = 1;

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
    AudioDataAck {
        sequence: u64,
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
    pub fn to_json_bytes(&self) -> Vec<u8> {
        serde_json::to_vec(self).expect("Failed to serialize message")
    }

    pub fn from_json_bytes(bytes: &[u8]) -> Option<Self> {
        serde_json::from_slice(bytes).ok()
    }
}

pub fn timestamp_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
