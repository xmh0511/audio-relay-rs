package com.audiorelay.client

import android.app.Notification
import android.app.PendingIntent
import android.app.Service
import android.content.Intent
import android.media.AudioAttributes
import android.media.AudioFormat
import android.media.AudioTrack
import android.os.Build
import android.os.IBinder
import android.os.PowerManager
import android.util.Log
import androidx.core.app.NotificationCompat
import kotlinx.coroutines.*
import okhttp3.*
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicBoolean
import org.json.JSONArray
import org.json.JSONObject

class AudioRelayService : Service() {

    private var audioTrack: AudioTrack? = null
    private var webSocket: WebSocket? = null
    private var wakeLock: PowerManager.WakeLock? = null

    private val scope = CoroutineScope(Dispatchers.IO + SupervisorJob())
    private val isRunning = AtomicBoolean(false)

    var onStateChanged: ((ServiceState) -> Unit)? = null
    var onAudioLevel: ((Float) -> Unit)? = null

    private var currentState = ServiceState.DISCONNECTED

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onCreate() {
        super.onCreate()
        instance = this
        Log.d(TAG, "Service created")
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        when (intent?.action) {
            ACTION_START -> {
                val host = intent.getStringExtra(EXTRA_HOST) ?: "192.168.1.100"
                val port = intent.getIntExtra(EXTRA_PORT, 8080)
                startForeground(NOTIFICATION_ID, buildNotification("Connecting to $host:$port…"))
                connect(host, port)
            }
            ACTION_STOP -> {
                stop()
                stopForeground(STOP_FOREGROUND_REMOVE)
                stopSelf()
            }
        }
        return START_STICKY
    }

    fun connect(host: String, port: Int) {
        if (isRunning.get()) return
        isRunning.set(true)
        acquireWakeLock()

        val client = OkHttpClient.Builder()
            .connectTimeout(10, TimeUnit.SECONDS)
            .readTimeout(0, TimeUnit.SECONDS)
            .pingInterval(30, TimeUnit.SECONDS)
            .build()

        val request = Request.Builder()
            .url("ws://$host:$port")
            .build()

        updateState(ServiceState.CONNECTING)

        webSocket = client.newWebSocket(request, object : WebSocketListener() {
            override fun onOpen(webSocket: WebSocket, response: Response) {
                Log.d(TAG, "WebSocket connected")
                sendHello(webSocket)
                updateNotification("Connected, waiting for handshake…")
                updateState(ServiceState.CONNECTED)
            }

            override fun onMessage(webSocket: WebSocket, text: String) {
                try {
                    handleMessage(text)
                } catch (e: Exception) {
                    Log.e(TAG, "Error parsing message: ${e.message}", e)
                }
            }

            override fun onClosing(webSocket: WebSocket, code: Int, reason: String) {
                Log.d(TAG, "WebSocket closing: $code $reason")
                webSocket.close(1000, null)
                handleDisconnect()
            }

            override fun onClosed(webSocket: WebSocket, code: Int, reason: String) {
                Log.d(TAG, "WebSocket closed: $code $reason")
                handleDisconnect()
            }

            override fun onFailure(webSocket: WebSocket, t: Throwable, response: Response?) {
                Log.e(TAG, "WebSocket failure: ${t.message}", t)
                handleDisconnect()
            }
        })
    }

    private fun sendHello(ws: WebSocket) {
        val hello = JSONObject().apply {
            put("Hello", JSONObject().apply {
                put("client_id", Build.MODEL ?: "Android")
                put("mode", "Speaker")
                put("sample_rate", SAMPLE_RATE)
                put("channels", CHANNELS)
            })
        }
        Log.d(TAG, "Sending Hello: $hello")
        ws.send(hello.toString())
    }

    private fun handleMessage(text: String) {
        val json = JSONObject(text)

        when {
            json.has("HelloAck") -> {
                val ack = json.getJSONObject("HelloAck")
                val sessionId = ack.optString("session_id", "")
                val serverSampleRate = ack.optInt("sample_rate", SAMPLE_RATE)
                Log.d(TAG, "HelloAck: session=$sessionId, rate=$serverSampleRate")
                initAudioTrack(serverSampleRate)
                updateNotification("Streaming audio…")
                updateState(ServiceState.STREAMING)
            }
            json.has("AudioData") -> {
                val audioData = json.getJSONObject("AudioData")
                val data = audioData.getJSONArray("data")
                val sequence = audioData.optLong("sequence", 0)
                playAudio(data, sequence)
            }
            json.has("Pong") -> {
                Log.d(TAG, "Pong received")
            }
            json.has("AudioDataAck") -> {
                // Server acknowledged our audio data (if we were in mic mode)
            }
        }
    }

    private fun initAudioTrack(serverSampleRate: Int) {
        audioTrack?.release()

        val channelConfig = if (CHANNELS == 2)
            AudioFormat.CHANNEL_OUT_STEREO else AudioFormat.CHANNEL_OUT_MONO

        val bufferSize = AudioTrack.getMinBufferSize(
            serverSampleRate, channelConfig, AudioFormat.ENCODING_PCM_16BIT
        ) * 4

        audioTrack = AudioTrack.Builder()
            .setAudioAttributes(
                AudioAttributes.Builder()
                    .setUsage(AudioAttributes.USAGE_MEDIA)
                    .setContentType(AudioAttributes.CONTENT_TYPE_MUSIC)
                    .build()
            )
            .setAudioFormat(
                AudioFormat.Builder()
                    .setSampleRate(serverSampleRate)
                    .setChannelMask(channelConfig)
                    .setEncoding(AudioFormat.ENCODING_PCM_16BIT)
                    .build()
            )
            .setBufferSizeInBytes(bufferSize)
            .setTransferMode(AudioTrack.MODE_STREAM)
            .build()

        audioTrack?.play()
        Log.d(TAG, "AudioTrack initialized: ${serverSampleRate}Hz, buffer=$bufferSize")
    }

    private fun playAudio(dataArray: JSONArray, sequence: Long) {
        try {
            val bytes = ByteArray(dataArray.length())
            for (i in 0 until dataArray.length()) {
                bytes[i] = dataArray.getInt(i).toByte()
            }

            audioTrack?.write(bytes, 0, bytes.size)

            var sum = 0L
            for (b in bytes) {
                sum += kotlin.math.abs(b.toInt())
            }
            val level = (sum.toFloat() / bytes.size / 128f).coerceIn(0f, 1f)
            onAudioLevel?.invoke(level)
        } catch (e: Exception) {
            Log.e(TAG, "Error playing audio: ${e.message}", e)
        }
    }

    fun sendPing() {
        val ping = JSONObject().apply {
            put("Ping", JSONObject().apply {
                put("timestamp", System.currentTimeMillis())
            })
        }
        webSocket?.send(ping.toString())
    }

    private fun handleDisconnect() {
        isRunning.set(false)
        audioTrack?.release()
        audioTrack = null
        releaseWakeLock()
        updateState(ServiceState.DISCONNECTED)
        updateNotification("Disconnected")
        stopForeground(STOP_FOREGROUND_REMOVE)
        stopSelf()
    }

    fun stop() {
        webSocket?.close(1000, "Client disconnect")
        webSocket = null
        isRunning.set(false)
        audioTrack?.release()
        audioTrack = null
        releaseWakeLock()
        updateState(ServiceState.DISCONNECTED)
        scope.cancel()
    }

    fun isConnected(): Boolean = isRunning.get()

    private fun acquireWakeLock() {
        val pm = getSystemService(POWER_SERVICE) as PowerManager
        wakeLock = pm.newWakeLock(
            PowerManager.PARTIAL_WAKE_LOCK,
            "AudioRelay::StreamingLock"
        ).apply {
            acquire(24 * 60 * 60 * 1000L) // 24 hours max
        }
    }

    private fun releaseWakeLock() {
        wakeLock?.let {
            if (it.isHeld) it.release()
        }
        wakeLock = null
    }

    private fun buildNotification(text: String): Notification {
        val pendingIntent = PendingIntent.getActivity(
            this, 0,
            Intent(this, MainActivity::class.java).apply {
                flags = Intent.FLAG_ACTIVITY_SINGLE_TOP
            },
            PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE
        )

        val stopIntent = PendingIntent.getService(
            this, 1,
            Intent(this, AudioRelayService::class.java).apply { action = ACTION_STOP },
            PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE
        )

        return NotificationCompat.Builder(this, AudioRelayApp.NOTIFICATION_CHANNEL_ID)
            .setContentTitle("Audio Relay")
            .setContentText(text)
            .setSmallIcon(android.R.drawable.ic_media_play)
            .setContentIntent(pendingIntent)
            .addAction(android.R.drawable.ic_media_pause, "Stop", stopIntent)
            .setOngoing(true)
            .setPriority(NotificationCompat.PRIORITY_LOW)
            .setCategory(NotificationCompat.CATEGORY_SERVICE)
            .build()
    }

    private fun updateNotification(text: String) {
        val manager = getSystemService(NOTIFICATION_SERVICE) as android.app.NotificationManager
        try {
            manager.notify(NOTIFICATION_ID, buildNotification(text))
        } catch (e: Exception) {
            Log.e(TAG, "Failed to update notification", e)
        }
    }

    private fun updateState(state: ServiceState) {
        currentState = state
        onStateChanged?.invoke(state)
    }

    override fun onDestroy() {
        instance = null
        stop()
        super.onDestroy()
    }

    enum class ServiceState {
        DISCONNECTED, CONNECTING, CONNECTED, STREAMING
    }

    companion object {
        private const val TAG = "AudioRelayService"
        private const val NOTIFICATION_ID = 1001
        const val ACTION_START = "com.audiorelay.START"
        const val ACTION_STOP = "com.audiorelay.STOP"
        private const val EXTRA_HOST = "host"
        private const val EXTRA_PORT = "port"
        private const val SAMPLE_RATE = 44100
        private const val CHANNELS = 1

        @Volatile
        var instance: AudioRelayService? = null
            private set
    }
}
