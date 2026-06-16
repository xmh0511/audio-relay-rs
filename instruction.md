# Audio Relay 实现文档

## 架构概览

```
┌─────────────┐     WebSocket      ┌─────────────┐
│   Android   │ ◄════════════════► │  Rust Server │
│   Client    │   JSON + Binary    │              │
└─────────────┘                    └──────┬───────┘
                                          │ broadcast channel
                                   ┌──────▼───────┐
                                   │ AudioCapture  │
                                   │ (WASAPI/cpal) │
                                   └──────────────┘
```

数据流：音频设备 → AudioCapture → broadcast channel → WebSocket → Android AudioTrack

---

## 模块详解

### 1. protocol/mod.rs — 通信协议

#### 控制消息（JSON 文本帧）

所有控制消息通过 `Message` 枚举定义，使用 serde 序列化为 JSON。

```rust
pub enum Message {
    Hello { client_id, mode, sample_rate, channels },
    HelloAck { session_id, sample_rate, channels },
    Ping { timestamp },
    Pong { timestamp },
    LatencyReport { latency_ms },
    // ...
}
```

#### 音频帧（二进制帧）

音频数据使用自定义二进制格式，避免 JSON 编码 PCM 字节数组的 3x 膨胀：

```rust
pub struct AudioFrame {
    pub sequence: u64,      // 帧序号，用于丢帧检测
    pub timestamp: u64,     // 发送时间戳（毫秒）
    pub sample_rate: u32,   // 采样率（支持动态切换）
    pub data: Vec<u8>,      // PCM 16-bit LE 数据
}
```

编解码函数：
- `encode_audio_frame()` — 将 AudioFrame 编码为二进制字节
- `decode_audio_frame()` — 从二进制字节解码 AudioFrame

二进制帧格式（20 字节头 + PCM 数据）：
```
[0..8]   sequence   u64 LE
[8..16]  timestamp  u64 LE
[16..20] sample_rate u32 LE
[20..]   pcm_data   raw bytes
```

---

### 2. audio/capture.rs — 音频采集

#### 平台差异

| 平台 | API | 原始数据格式 |
|------|-----|-------------|
| Windows | WASAPI (wasapi crate) | 16-bit i16 LE 或 32-bit f32 LE |
| Linux/macOS | cpal | 32-bit f32 |

#### AudioCapture 结构

```rust
pub struct AudioCapture {
    stop_flag: Arc<AtomicBool>,         // 通知采集线程停止
    stopped_rx: Option<oneshot::Receiver<()>>,  // 等待线程结束
    _handle: Option<JoinHandle<()>>,    // 采集线程句柄
}
```

- `new()` — 在新 OS 线程中启动采集（非 tokio 线程，避免阻塞异步运行时）
- `stop()` — 设置 stop_flag，采集线程会在下次循环检查时退出
- `wait_stopped()` — 异步等待采集线程结束（通过 oneshot channel）

#### Windows 路径 (`capture_windows`)

WASAPI Loopback 捕获系统音频输出：

1. 获取默认 Render 设备（`get_default_device(&Direction::Render)`）
2. 获取 mix format（设备原生格式）
3. 初始化为 Loopback Capture（`Direction::Capture` + `loopback=true`）
4. 事件驱动读取（`wait_for_event`）

**16-bit 数据的字节序处理**：

WASAPI 返回的 PCM 数据是 **little-endian** 格式。这是由 Windows 平台决定的：

> Windows 只运行在 little-endian 架构上（x86、x64、ARM LE 模式），
> WASAPI 的 PCM 数据字节序与平台 native endian 一致。

参考文档：
- [WAVEFORMATEX (mmeapi.h)](https://learn.microsoft.com/en-us/windows/win32/api/mmeapi/ns-mmeapi-waveformatex)
  - `wBitsPerSample` 定义每样本位数
  - PCM 数据按平台原生字节序存储
- [WAVEFORMATEXTENSIBLE](https://learn.microsoft.com/en-us/windows/win32/api/mmreg/ns-mmreg-waveformatextensible)
  - 扩展格式，支持更多声道和位深

因此在 16-bit 路径中，可以直接 `unsafe` reinterpret `&[u8]` 为 `&[i16]`：

```rust
// Safety: WASAPI PCM data is little-endian (Windows only runs on LE architectures).
// i16 on x86/x64 is also LE, so the memory layout matches and reinterpret is valid.
let i16_slice = unsafe {
    std::slice::from_raw_parts(raw_chunk.as_ptr() as *const i16, bytes_to_send / 2)
};
```

**处理流程**：

| 设备格式 | 处理方式 |
|----------|----------|
| 16-bit mono | 直接 reinterpret 为 `&[i16]`，无需转换 |
| 16-bit stereo | reinterpret 为 `&[i16]`，在 i16 层面混音为 mono |
| 32-bit float | `convert_to_i16_pcm()` 转换（f32 → i16） |

#### Linux/macOS 路径 (`capture_cpal`)

cpal 回调提供 `&[f32]` 数据：

1. 通过 `sync_channel` 从回调线程传递到采集循环
2. `float_to_i16_bytes()` 将 f32 转换为 i16 LE 字节
3. 后续处理与 Windows 一致

---

### 3. audio/playback.rs — 音频播放

使用 cpal 播放 PCM 音频，通过 `crossbeam_channel` 从 WebSocket 接收线程传递数据到 cpal 回调：

```rust
pub fn new(rx: crossbeam_channel::Receiver<Vec<u8>>) -> Result<Self>
```

cpal 回调中使用 `rx.try_recv()`（无锁/低锁），避免在实时音频回调中获取 Mutex。

播放回调中的转换：
- `i16_bytes_to_float()` — 将 i16 LE 字节转换为 f32（cpal 要求 f32 输出）
- 如果数据不足，剩余部分填充 0.0（静音）

---

### 4. audio/resampler.rs — 采样率转换

使用 rubato 库的 `SincFixedIn` sinc 插值重采样器：

```rust
pub struct AudioResampler {
    resampler: SincFixedIn<f64>,
    input_buffer: Vec<Vec<f64>>,   // 每通道输入缓冲
    temp_channels: Vec<Vec<f64>>,  // 处理用临时缓冲（复用）
}
```

参数：
- `sinc_len: 256` — sinc 滤波器长度
- `f_cutoff: 0.95` — 截止频率
- `oversampling_factor: 16` — 过采样因子
- `chunk_size: 1024` — 每次处理的帧数

处理流程：
1. i16 输入归一化为 f64（除以 `i16::MAX`）
2. 累积到 `chunk_size` 后批量处理
3. 输出 clamp 到 `[-1.0, 1.0]` 后转回 i16

---

### 5. server/mod.rs — 服务端

#### 核心结构

```rust
pub struct AppState {
    pub clients: ClientInfo,                           // 已连接客户端
    pub audio_capture: Arc<Mutex<Option<AudioCaptureState>>>,  // 当前音频采集
    pub broadcast_tx: broadcast::Sender<(u32, Vec<u8>)>,       // 音频广播
}
```

#### 连接处理流程

1. WebSocket 握手（`accept_async`）
2. 接收 `Hello` 消息，回复 `HelloAck`
3. 根据客户端模式：
   - **Speaker**：订阅 broadcast channel，将音频帧编码为二进制发送
   - **Microphone**：接收二进制音频帧，解码后发送到 broadcast channel
4. 心跳：每 10 秒发送 `Ping`，接收 `Pong` 计算延迟

#### Web 管理 API

- `GET /` — 管理界面 HTML
- `GET /api/clients` — 已连接客户端列表
- `GET /api/sample-rate` — 当前采样率
- `POST /api/sample-rate` — 动态切换采样率（重启音频采集）

#### UDP 发现

服务端在 UDP 8082 端口响应发现请求，返回服务信息 JSON：
```json
{"name": "AudioRelay", "ws_port": 8080, "web_port": 8081}
```

---

### 6. client/mod.rs — 客户端（PC 测试用）

PC 端的测试客户端，用于调试服务端功能。

#### Microphone 模式
- 启动 AudioCapture 采集本地音频
- 将音频帧编码为二进制发送到服务端

#### Speaker 模式
- 接收服务端二进制音频帧
- 解码后通过 crossbeam-channel 传递给 AudioPlayback
- 每 5 秒发送 Ping 测量延迟

---

### 7. Android 客户端

#### AudioRelayService.kt

前台服务，核心功能：
- WebSocket 连接管理（OkHttp）
- 音频播放（AudioTrack，MODE_STREAM）
- 断线自动重连（3 秒延迟）
- Wake Lock 防止休眠

消息处理：
- `onMessage(text)` — 处理 JSON 控制消息（HelloAck, Ping/Pong）
- `onMessage(bytes)` — 处理二进制音频帧

二进制帧解析（与 Rust 端一致）：
```kotlin
val sequence = ByteBuffer.wrap(buffer, 0, 8).order(LITTLE_ENDIAN).long
val timestamp = ByteBuffer.wrap(buffer, 8, 8).order(LITTLE_ENDIAN).long
val sampleRate = ByteBuffer.wrap(buffer, 16, 4).order(LITTLE_ENDIAN).int
val pcmData = buffer.copyOfRange(20, buffer.size)
```

#### MainActivity.kt

Jetpack Compose UI：
- 服务器 IP/端口输入
- 一键连接/断开
- 实时音频电平可视化
- 延迟显示（当前/平均）
- UDP 自动发现服务器

---

## 关键设计决策

### 1. 音频采集使用 OS 线程而非 tokio

`AudioCapture::new()` 使用 `std::thread::Builder::new().spawn()` 而非 `tokio::spawn`，因为：
- WASAPI/cpal 的回调可能阻塞
- 避免阻塞 tokio 异步运行时
- 音频采集是 CPU 密集型任务

### 2. 二进制音频帧 vs JSON

选择二进制帧传输音频数据，因为：
- JSON 编码 PCM 字节数组会产生 ~3x 膨胀（每个字节变成 1-3 个 ASCII 字符）
- 解析 JSON 数组比直接 memcpy 慢得多
- 控制消息（Hello/Ping 等）频率低，JSON 可读性好

### 3. crossbeam-channel 替代 tokio mpsc

播放回调中使用 `crossbeam_channel::try_recv()` 而非 `tokio::sync::Mutex<mpsc::Receiver>`：
- cpal 回调是同步的，不能 `.await`
- crossbeam try_recv 是 lock-free 的，适合实时音频回调
- 避免在回调中获取 Mutex 导致的优先级反转

### 4. WASAPI Loopback 而非麦克风

Windows 端使用 WASAPI Loopback 捕获系统音频输出（而非麦克风输入），因为：
- 主要用途是将 PC 音频串流到手机
- Loopback 可以捕获任何应用播放的音频
- 不需要额外的虚拟音频设备

---

## 已知限制

1. **单客户端 Speaker**：broadcast channel 只支持一个 Speaker 客户端，多个 Speaker 会各自收到相同数据
2. **采样率切换**：动态切换采样率需要重启音频采集，可能有短暂中断
3. **UDP 发现端口硬编码**：8082 端口无法通过命令行配置
4. **无 TLS**：WebSocket 连接未加密，仅适合局域网使用
