package com.audiorelay.client

import android.Manifest
import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.content.ServiceConnection
import android.content.pm.PackageManager
import android.os.Build
import android.os.Bundle
import android.os.IBinder
import android.widget.Toast
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.animation.core.*
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalLifecycleOwner
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.core.content.ContextCompat
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleEventObserver

class MainActivity : ComponentActivity() {

    private var boundService: AudioRelayService? = null
    private val isBound = mutableStateOf(false)
    private val serviceRef = mutableStateOf<AudioRelayService?>(null)

    private val connection = object : ServiceConnection {
        override fun onServiceConnected(name: ComponentName?, binder: IBinder?) {
            val localBinder = binder as AudioRelayService.LocalBinder
            boundService = localBinder.getService()
            serviceRef.value = boundService
            isBound.value = true
        }

        override fun onServiceDisconnected(name: ComponentName?) {
            boundService = null
            serviceRef.value = null
            isBound.value = false
        }
    }

    private val permissionLauncher = registerForActivityResult(
        ActivityResultContracts.RequestMultiplePermissions()
    ) { permissions ->
        val allGranted = permissions.values.all { it }
        if (!allGranted) {
            Toast.makeText(this, "Some permissions denied, notifications may not work", Toast.LENGTH_SHORT).show()
        }
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        requestPermissions()

        setContent {
            AudioRelayTheme {
                AudioRelayScreen(
                    service = serviceRef.value,
                    isBound = isBound.value
                )
            }
        }
    }

    override fun onStart() {
        super.onStart()
        Intent(this, AudioRelayService::class.java).also { intent ->
            bindService(intent, connection, Context.BIND_AUTO_CREATE)
        }
    }

    override fun onStop() {
        super.onStop()
        if (isBound.value) {
            unbindService(connection)
            isBound.value = false
        }
    }

    private fun requestPermissions() {
        val permissions = mutableListOf<String>()
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            if (ContextCompat.checkSelfPermission(this, Manifest.permission.POST_NOTIFICATIONS)
                != PackageManager.PERMISSION_GRANTED
            ) {
                permissions.add(Manifest.permission.POST_NOTIFICATIONS)
            }
        }
        if (permissions.isNotEmpty()) {
            permissionLauncher.launch(permissions.toTypedArray())
        }
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun AudioRelayScreen(
    service: AudioRelayService?,
    isBound: Boolean
) {
    var serverHost by remember { mutableStateOf("192.168.1.100") }
    var serverPort by remember { mutableStateOf("8080") }
    var isConnected by remember { mutableStateOf(false) }
    var audioLevel by remember { mutableFloatStateOf(0f) }
    val context = LocalContext.current

    LaunchedEffect(service, isBound) {
        if (isBound && service != null) {
            service.onStateChanged = { state ->
                isConnected = state == AudioRelayService.ServiceState.STREAMING
            }
            service.onAudioLevel = { level ->
                audioLevel = level
            }
            isConnected = service.isConnected()
        }
    }

    Box(
        modifier = Modifier
            .fillMaxSize()
            .background(
                Brush.verticalGradient(
                    colors = listOf(
                        Color(0xFF1A1A2E),
                        Color(0xFF16213E),
                        Color(0xFF0F3460)
                    )
                )
            )
    ) {
        Column(
            modifier = Modifier
                .fillMaxSize()
                .padding(24.dp),
            horizontalAlignment = Alignment.CenterHorizontally,
            verticalArrangement = Arrangement.Center
        ) {
            Icon(
                imageVector = Icons.Default.VolumeUp,
                contentDescription = null,
                modifier = Modifier.size(48.dp),
                tint = Color(0xFF533483)
            )

            Spacer(modifier = Modifier.height(16.dp))

            Text(
                text = "Audio Relay",
                fontSize = 32.sp,
                fontWeight = FontWeight.Bold,
                color = Color.White
            )

            Spacer(modifier = Modifier.height(8.dp))

            Text(
                text = "Stream PC audio to your phone",
                fontSize = 14.sp,
                color = Color.White.copy(alpha = 0.6f)
            )

            Spacer(modifier = Modifier.height(40.dp))

            AudioVisualizer(
                level = if (isConnected) audioLevel else 0f,
                isConnected = isConnected
            )

            Spacer(modifier = Modifier.height(40.dp))

            OutlinedTextField(
                value = serverHost,
                onValueChange = { serverHost = it },
                label = { Text("Server IP") },
                leadingIcon = { Icon(Icons.Default.Dns, contentDescription = null) },
                modifier = Modifier.fillMaxWidth(),
                enabled = !isConnected,
                colors = OutlinedTextFieldDefaults.colors(
                    focusedTextColor = Color.White,
                    unfocusedTextColor = Color.White,
                    focusedBorderColor = Color(0xFF533483),
                    unfocusedBorderColor = Color.White.copy(alpha = 0.3f),
                    focusedLabelColor = Color(0xFF533483),
                    unfocusedLabelColor = Color.White.copy(alpha = 0.5f),
                    cursorColor = Color(0xFF533483)
                ),
                singleLine = true
            )

            Spacer(modifier = Modifier.height(12.dp))

            OutlinedTextField(
                value = serverPort,
                onValueChange = { serverPort = it },
                label = { Text("Port") },
                leadingIcon = { Icon(Icons.Default.Lan, contentDescription = null) },
                modifier = Modifier.fillMaxWidth(),
                enabled = !isConnected,
                colors = OutlinedTextFieldDefaults.colors(
                    focusedTextColor = Color.White,
                    unfocusedTextColor = Color.White,
                    focusedBorderColor = Color(0xFF533483),
                    unfocusedBorderColor = Color.White.copy(alpha = 0.3f),
                    focusedLabelColor = Color(0xFF533483),
                    unfocusedLabelColor = Color.White.copy(alpha = 0.5f),
                    cursorColor = Color(0xFF533483)
                ),
                singleLine = true
            )

            Spacer(modifier = Modifier.height(32.dp))

            Button(
                onClick = {
                    if (!isBound || service == null) {
                        Toast.makeText(
                            context,
                            "Service not bound, retrying…",
                            Toast.LENGTH_SHORT
                        ).show()
                        return@Button
                    }

                    if (isConnected) {
                        service.stop()
                        isConnected = false
                    } else {
                        val port = serverPort.toIntOrNull() ?: 8080
                        val intent = Intent(
                            context,
                            AudioRelayService::class.java
                        ).apply {
                            action = "com.audiorelay.START"
                            putExtra("host", serverHost)
                            putExtra("port", port)
                        }
                        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
                            context.startForegroundService(intent)
                        } else {
                            context.startService(intent)
                        }
                        Toast.makeText(
                            context,
                            "Connecting to $serverHost:$port",
                            Toast.LENGTH_SHORT
                        ).show()
                    }
                },
                modifier = Modifier
                    .fillMaxWidth()
                    .height(56.dp),
                shape = RoundedCornerShape(16.dp),
                colors = ButtonDefaults.buttonColors(
                    containerColor = if (isConnected) Color(0xFFE94560) else Color(0xFF533483)
                )
            ) {
                Icon(
                    imageVector = if (isConnected) Icons.Default.Stop else Icons.Default.PlayArrow,
                    contentDescription = null,
                    modifier = Modifier.size(24.dp)
                )
                Spacer(modifier = Modifier.width(8.dp))
                Text(
                    text = if (isConnected) "Disconnect" else "Connect",
                    fontSize = 18.sp,
                    fontWeight = FontWeight.SemiBold
                )
            }

            Spacer(modifier = Modifier.height(24.dp))

            Row(
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.Center
            ) {
                Box(
                    modifier = Modifier
                        .size(10.dp)
                        .clip(CircleShape)
                        .background(if (isConnected) Color(0xFF4CAF50) else Color(0xFFE94560))
                )
                Spacer(modifier = Modifier.width(8.dp))
                Text(
                    text = if (isConnected) "Streaming" else "Disconnected",
                    color = Color.White.copy(alpha = 0.7f),
                    fontSize = 14.sp
                )
            }
        }
    }
}

@Composable
fun AudioVisualizer(level: Float, isConnected: Boolean) {
    val infiniteTransition = rememberInfiniteTransition(label = "visualizer")
    val animatedLevel by infiniteTransition.animateFloat(
        initialValue = 0.3f,
        targetValue = 1f,
        animationSpec = infiniteRepeatable(
            animation = tween(300, easing = FastOutSlowInEasing),
            repeatMode = RepeatMode.Reverse
        ),
        label = "level"
    )

    val displayLevel = if (isConnected) (level * animatedLevel).coerceIn(0.05f, 1f) else 0f

    Row(
        modifier = Modifier
            .fillMaxWidth()
            .height(80.dp),
        horizontalArrangement = Arrangement.SpaceEvenly,
        verticalAlignment = Alignment.CenterVertically
    ) {
        repeat(12) { index ->
            val baseHeight = when {
                !isConnected -> 4.dp
                else -> {
                    val offset = (index % 4) * 0.15f
                    ((displayLevel + offset) * 60f).dp.coerceIn(4.dp, 80.dp)
                }
            }
            val barColor = when {
                !isConnected -> Color.White.copy(alpha = 0.1f)
                baseHeight > 60.dp -> Color(0xFFE94560)
                baseHeight > 30.dp -> Color(0xFF533483)
                else -> Color(0xFF0F3460)
            }

            Box(
                modifier = Modifier
                    .width(8.dp)
                    .height(baseHeight)
                    .clip(RoundedCornerShape(4.dp))
                    .background(barColor)
            )
        }
    }
}

@Composable
fun AudioRelayTheme(content: @Composable () -> Unit) {
    MaterialTheme(
        colorScheme = darkColorScheme(
            primary = Color(0xFF533483),
            secondary = Color(0xFFE94560),
            background = Color(0xFF1A1A2E),
            surface = Color(0xFF16213E),
            onPrimary = Color.White,
            onSecondary = Color.White,
            onBackground = Color.White,
            onSurface = Color.White
        ),
        content = content
    )
}
