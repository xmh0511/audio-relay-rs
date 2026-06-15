# Audio Relay Android Client

A cross-platform Android client for streaming PC audio to your phone.

## Features

- **Foreground Service**: Prevents Android from killing the app when screen off or in background
- **Wake Lock**: Keeps CPU alive during audio streaming
- **Real-time Audio**: Uses AudioTrack for low-latency PCM audio playback
- **WebSocket Protocol**: Compatible with the Rust server (`audio-relay-rs`)
- **Material Design 3**: Modern UI with audio visualizer

## Requirements

- Android Studio Hedgehog (2023.1.1) or later
- JDK 17
- Android SDK 34
- Kotlin 1.9.21

## Build

1. Open `android-client/` folder in Android Studio
2. Sync Gradle
3. Build → Build APK (or Run on device)

Or via command line:
```bash
cd android-client
./gradlew assembleDebug
```

APK output: `app/build/outputs/apk/debug/app-debug.apk`

## Usage

1. Start the Rust server on your PC:
   ```bash
   cargo run -- server -p 8080
   ```

2. Install and open the Android app

3. Enter your PC's IP address and port (default: 8080)

4. Tap **Connect**

5. The app runs as a foreground service — you can switch apps or lock the screen

## Architecture

```
android-client/
├── app/src/main/java/com/audiorelay/client/
│   ├── AudioRelayApp.kt      # Application, notification channel
│   ├── AudioRelayService.kt   # Foreground Service, WebSocket, AudioTrack
│   └── MainActivity.kt        # Compose UI
├── app/build.gradle.kts
└── settings.gradle.kts
```

## Protocol

Compatible with the Rust server's JSON WebSocket protocol:

- **Hello**: `{"Hello": {"client_id": "...", "mode": "Speaker", "sample_rate": 44100, "channels": 1}}`
- **HelloAck**: `{"HelloAck": {"session_id": "...", "sample_rate": 44100, "channels": 1}}`
- **AudioData**: `{"AudioData": {"sequence": 0, "data": [byte array]}}`
- **Ping/Pong**: Heartbeat messages
