# Audio Relay

在 PC 和手机之间实时串流音频的工具，使用 Rust 编写服务端，Kotlin 编写 Android 客户端。

## 功能

- **PC → 手机**：将 PC 音频实时传输到手机播放（Speaker 模式）
- **手机 → PC**：将手机麦克风音频传输到 PC 播放（Microphone 模式）
- **前台服务**：Android 端使用 Foreground Service + Wake Lock，熄屏或切换后台后持续运行
- **跨平台服务端**：支持 Windows / Linux / macOS
- **Web 管理界面**：通过 HTTP API 查看客户端状态、实时调整采样率（无需重启采集）

## 快速开始

### 1. 启动服务端（PC）

```bash
# 编译
cargo build --release

# 启动，默认监听 0.0.0.0:8080
cargo run --release -- server

# 自定义端口
cargo run --release -- server -p 9090
```

### 2. 安装 Android 客户端

编译 Android 客户端：

```bash
cd android-client
./gradlew assembleRelease
# 输出: app/build/outputs/apk/release/app-release.apk
```

> 首次安装需允许「通知权限」和「忽略电池优化」。

### 3. 连接

1. 确保手机和 PC 在同一局域网
2. 打开 Android App，输入 PC 的 IP 地址和端口
3. 点击 **Connect**
4. 开始播放 PC 端的音乐/视频，手机会实时播放音频

## 命令行参数

### Server

```
audio-relay-rs server [OPTIONS]

Options:
  -h, --host <HOST>      监听地址 [default: 0.0.0.0]
  -p, --port <PORT>      WebSocket 端口 [default: 8080]
      --web-port <PORT>  Web 管理端口 [default: 8081]
```

### Client（测试用，PC 模拟客户端）

```
audio-relay-rs client [OPTIONS]

Options:
  -s, --server <SERVER>  服务端地址
  -p, --port <PORT>      服务端端口 [default: 8080]
  -m, --mode <MODE>      模式: speaker / mic [default: speaker]
```

## 项目结构

```
audio-relay-rs/
├── Cargo.toml
├── README.md                    # 项目说明
├── instruction.md               # 实现细节与设计文档
├── src/
│   ├── main.rs                  # CLI 入口
│   ├── protocol/mod.rs          # 通信协议（JSON 控制帧 + 二进制音频帧）
│   ├── audio/
│   │   ├── capture.rs           # 音频采集（WASAPI/cpal，设备原生采样率输出）
│   │   ├── playback.rs          # 音频播放（cpal + crossbeam-channel）
│   │   └── resampler.rs         # 采样率转换（rubato SincFixedIn + 可复用缓冲区）
│   ├── server/
│   │   ├── mod.rs               # WebSocket 服务端 + Web API + Relay Task
│   │   └── web/index.html       # Web 管理界面
│   └── client/mod.rs            # WebSocket 客户端（PC 测试用）
├── android-client/              # Android 客户端（Kotlin）
│   └── app/src/main/java/com/audiorelay/client/
│       ├── AudioRelayService.kt # 前台服务 + WebSocket + AudioTrack
│       └── MainActivity.kt      # Jetpack Compose UI
└── .github/workflows/
    └── build.yml                # CI/CD 自动构建
```

## 技术栈

| 组件 | 技术 |
|------|------|
| 服务端 | Rust + tokio + tungstenite + cpal |
| Windows 音频采集 | WASAPI (wasapi crate) |
| 跨平台音频采集/播放 | cpal |
| 重采样 | rubato (SincFixedIn) |
| 播放回调通道 | crossbeam-channel |
| Android 客户端 | Kotlin + Jetpack Compose + OkHttp + AudioTrack |
| 通信协议 | WebSocket（JSON 控制帧 + 二进制音频帧） |
| 音频格式 | PCM 16-bit, Mono（采样率动态可调） |

## 协议说明

### 控制消息（JSON 文本帧）

```json
// 客户端 → 服务端：握手
{"Hello": {"client_id": "device_name", "mode": "Speaker", "sample_rate": 44100, "channels": 1}}

// 服务端 → 客户端：握手确认
{"HelloAck": {"session_id": "uuid", "sample_rate": 44100, "channels": 1}}

// 心跳
{"Ping": {"timestamp": 1700000000000}}
{"Pong": {"timestamp": 1700000000000}}
```

### 音频数据（二进制帧）

音频数据使用 WebSocket 二进制帧传输，避免 JSON 编码开销：

```
Offset  Size   Field
0       8      sequence (u64 LE)      帧序号
8       8      timestamp (u64 LE)     时间戳（毫秒）
16      4      sample_rate (u32 LE)   采样率
20      N      pcm_data (raw bytes)   PCM 16-bit LE 数据
```

## 构建

### 服务端

```bash
# 需要 Rust 1.70+
cargo build --release
# 输出: target/release/audio-relay-rs
```

### Android 客户端（需要 Android Studio / SDK）

```bash
cd android-client
./gradlew assembleRelease
# 输出: app/build/outputs/apk/release/app-release.apk
```

### 自动构建

推送到 GitHub 后，Actions 会自动构建所有平台的产物。

## 常见问题

**Q: 手机收不到声音？**
- 确认 PC 防火墙允许 8080 端口
- 确认手机和 PC 在同一局域网
- 检查 PC 端是否有音频正在播放

**Q: 音频有延迟？**
- 局域网延迟通常 < 50ms
- 可通过 Web 管理界面（http://localhost:8081）实时调整采样率，无需重启

**Q: Android 后台被杀死？**
- 确保 App 有通知权限
- 在电池优化中将 Audio Relay 设为「不受限制」
- 前台服务会显示常驻通知，这是正常行为
