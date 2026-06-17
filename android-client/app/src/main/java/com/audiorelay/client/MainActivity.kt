package com.audiorelay.client

import android.Manifest
import android.content.Intent
import android.content.pm.PackageManager
import android.net.Uri
import android.os.Build
import android.os.Bundle
import android.os.PowerManager
import android.provider.Settings
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
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.core.content.ContextCompat

class MainActivity : ComponentActivity() {

    private val permissionLauncher = registerForActivityResult(
        ActivityResultContracts.RequestMultiplePermissions()
    ) { permissions ->
        val allGranted = permissions.values.all { it }
        if (!allGranted) {
            Toast.makeText(this, "Some permissions denied", Toast.LENGTH_SHORT).show()
        }
        requestBatteryOptimization()
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        requestPermissions()

        setContent {
            AudioRelayTheme {
                AudioRelayScreen()
            }
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
        } else {
            requestBatteryOptimization()
        }
    }

    private fun requestBatteryOptimization() {
        val pm = getSystemService(POWER_SERVICE) as PowerManager
        if (!pm.isIgnoringBatteryOptimizations(packageName)) {
            try {
                val intent = Intent(Settings.ACTION_REQUEST_IGNORE_BATTERY_OPTIMIZATIONS).apply {
                    data = Uri.parse("package:$packageName")
                }
                startActivity(intent)
            } catch (e: Exception) {
                // Some devices don't support this intent
            }
        }
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun AudioRelayScreen() {
    var serverHost by remember { mutableStateOf("192.168.1.100") }
    var serverPort by remember { mutableStateOf("8080") }
    var isPlaying by remember { mutableStateOf(false) }
    var audioLevel by remember { mutableFloatStateOf(0f) }
    var latency by remember { mutableFloatStateOf(0f) }
    var avgLatency by remember { mutableFloatStateOf(0f) }
    val context = LocalContext.current

    val discovery = remember { ServerDiscovery() }

    DisposableEffect(Unit) {
        discovery.startListening()
        onDispose {
            discovery.stopListening()
        }
    }

    LaunchedEffect(Unit) {
        while (true) {
            val svc = AudioRelayService.instance
            if (svc != null) {
                svc.onAudioLevel = { audioLevel = it }
                isPlaying = svc.isConnected()
                latency = svc.getLatency()
                avgLatency = svc.getAvgLatency()
            } else {
                isPlaying = false
                latency = 0f
                avgLatency = 0f
            }
            kotlinx.coroutines.delay(500)
        }
    }

    Box(
        modifier = Modifier
            .fillMaxSize()
            .background(
                Brush.verticalGradient(
                    colors = listOf(Color(0xFF1A1A2E), Color(0xFF16213E), Color(0xFF0F3460))
                )
            )
    ) {
        Column(
            modifier = Modifier.fillMaxSize().padding(24.dp),
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

            Text("Audio Relay", fontSize = 32.sp, fontWeight = FontWeight.Bold, color = Color.White)

            Spacer(modifier = Modifier.height(4.dp))

            Text("v${context.packageManager.getPackageInfo(context.packageName, 0).versionName}",
                fontSize = 12.sp, color = Color.White.copy(alpha = 0.4f))

            Spacer(modifier = Modifier.height(8.dp))

            Text(stringResource(R.string.stream_pc_audio), fontSize = 14.sp, color = Color.White.copy(alpha = 0.6f))

            Spacer(modifier = Modifier.height(40.dp))

            AudioVisualizer(level = if (isPlaying) audioLevel else 0f, isConnected = isPlaying)

            Spacer(modifier = Modifier.height(40.dp))

            OutlinedTextField(
                value = serverHost,
                onValueChange = { serverHost = it },
                label = { Text("Server IP") },
                leadingIcon = { Icon(Icons.Default.Dns, contentDescription = null) },
                modifier = Modifier.fillMaxWidth(),
                enabled = !isPlaying,
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
                label = { Text(stringResource(R.string.port)) },
                leadingIcon = { Icon(Icons.Default.Lan, contentDescription = null) },
                modifier = Modifier.fillMaxWidth(),
                enabled = !isPlaying,
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

            if (discovery.discoveredList.isNotEmpty() && !isPlaying) {
                Spacer(modifier = Modifier.height(16.dp))
                Text(
                    text = stringResource(R.string.discovered_servers),
                    color = Color.White.copy(alpha = 0.5f),
                    fontSize = 12.sp
                )
                Spacer(modifier = Modifier.height(8.dp))
                discovery.discoveredList.toList().forEach { server ->
                    Surface(
                        onClick = {
                            serverHost = server.address
                            serverPort = server.wsPort.toString()
                            val intent = Intent(context, AudioRelayService::class.java).apply {
                                action = AudioRelayService.ACTION_START
                                putExtra("host", server.address)
                                putExtra("port", server.wsPort)
                            }
                            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
                                context.startForegroundService(intent)
                            } else {
                                context.startService(intent)
                            }
                            isPlaying = true
                            Toast.makeText(context, "Connecting to ${server.address}:${server.wsPort}", Toast.LENGTH_SHORT).show()
                        },
                        modifier = Modifier
                            .fillMaxWidth()
                            .padding(vertical = 4.dp),
                        color = Color(0xFF16213E),
                        shape = RoundedCornerShape(8.dp)
                    ) {
                        Row(
                            modifier = Modifier.padding(12.dp),
                            verticalAlignment = Alignment.CenterVertically
                        ) {
                            Icon(
                                Icons.Default.Computer,
                                contentDescription = null,
                                tint = Color(0xFF533483),
                                modifier = Modifier.size(20.dp)
                            )
                            Spacer(modifier = Modifier.width(12.dp))
                            Column {
                                Text(
                                    text = server.name,
                                    color = Color.White,
                                    fontSize = 14.sp
                                )
                                Text(
                                    text = "${server.address}:${server.wsPort}",
                                    color = Color.White.copy(alpha = 0.5f),
                                    fontSize = 12.sp
                                )
                            }
                        }
                    }
                }
            }

            Spacer(modifier = Modifier.height(32.dp))

            Button(
                onClick = {
                    if (isPlaying) {
                        val intent = Intent(context, AudioRelayService::class.java).apply {
                            action = AudioRelayService.ACTION_STOP
                        }
                        context.startService(intent)
                        isPlaying = false
                    } else {
                        val port = serverPort.toIntOrNull() ?: 8080
                        val intent = Intent(context, AudioRelayService::class.java).apply {
                            action = AudioRelayService.ACTION_START
                            putExtra("host", serverHost)
                            putExtra("port", port)
                        }
                        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
                            context.startForegroundService(intent)
                        } else {
                            context.startService(intent)
                        }
                        isPlaying = true
                        Toast.makeText(context, "Connecting to $serverHost:$port", Toast.LENGTH_SHORT).show()
                    }
                },
                modifier = Modifier.fillMaxWidth().height(56.dp),
                shape = RoundedCornerShape(16.dp),
                colors = ButtonDefaults.buttonColors(
                    containerColor = if (isPlaying) Color(0xFFE94560) else Color(0xFF533483)
                )
            ) {
                Icon(
                    imageVector = if (isPlaying) Icons.Default.Stop else Icons.Default.PlayArrow,
                    contentDescription = null,
                    modifier = Modifier.size(24.dp)
                )
                Spacer(modifier = Modifier.width(8.dp))
                Text(
                    text = if (isPlaying) stringResource(R.string.disconnect) else stringResource(R.string.connect),
                    fontSize = 18.sp,
                    fontWeight = FontWeight.SemiBold
                )
            }

            Spacer(modifier = Modifier.height(24.dp))

            Row(verticalAlignment = Alignment.CenterVertically) {
                Box(
                    modifier = Modifier
                        .size(10.dp)
                        .clip(CircleShape)
                        .background(if (isPlaying) Color(0xFF4CAF50) else Color(0xFFE94560))
                )
                Spacer(modifier = Modifier.width(8.dp))
                Text(
                    text = if (isPlaying) stringResource(R.string.streaming) else stringResource(R.string.disconnected),
                    color = Color.White.copy(alpha = 0.7f),
                    fontSize = 14.sp
                )
            }

            if (isPlaying) {
                Spacer(modifier = Modifier.height(12.dp))
                Row(
                    horizontalArrangement = Arrangement.spacedBy(24.dp),
                    modifier = Modifier.fillMaxWidth(),
                    verticalAlignment = Alignment.CenterVertically
                ) {
                    Column(horizontalAlignment = Alignment.CenterHorizontally) {
                        Text(
                            text = stringResource(R.string.latency),
                            color = Color.White.copy(alpha = 0.5f),
                            fontSize = 11.sp
                        )
                        Text(
                            text = "${String.format("%.0f", latency)}ms",
                            color = when {
                                latency < 50 -> Color(0xFF4CAF50)
                                latency < 150 -> Color(0xFFFF9800)
                                else -> Color(0xFFE94560)
                            },
                            fontSize = 16.sp,
                            fontWeight = FontWeight.Bold
                        )
                    }
                    Column(horizontalAlignment = Alignment.CenterHorizontally) {
                        Text(
                            text = stringResource(R.string.avg_latency),
                            color = Color.White.copy(alpha = 0.5f),
                            fontSize = 11.sp
                        )
                        Text(
                            text = "${String.format("%.0f", avgLatency)}ms",
                            color = when {
                                avgLatency < 50 -> Color(0xFF4CAF50)
                                avgLatency < 150 -> Color(0xFFFF9800)
                                else -> Color(0xFFE94560)
                            },
                            fontSize = 16.sp,
                            fontWeight = FontWeight.Bold
                        )
                    }
                }
            }
        }
    }
}

@Composable
fun AudioVisualizer(level: Float, isConnected: Boolean) {
    val infiniteTransition = rememberInfiniteTransition(label = "visualizer")
    val animatedPhase by infiniteTransition.animateFloat(
        initialValue = 0f,
        targetValue = 1f,
        animationSpec = infiniteRepeatable(
            animation = tween(800, easing = FastOutSlowInEasing),
            repeatMode = RepeatMode.Restart
        ),
        label = "phase"
    )

    Row(
        modifier = Modifier.fillMaxWidth().height(100.dp),
        horizontalArrangement = Arrangement.SpaceEvenly,
        verticalAlignment = Alignment.CenterVertically
    ) {
        repeat(16) { index ->
            val phaseOffset = index * 0.12f
            val wave = if (isConnected) {
                val base = level * (0.5f + 0.5f * kotlin.math.sin(
                    (animatedPhase * 2 * Math.PI + phaseOffset).toFloat()
                ))
                (base * 80f).coerceIn(4f, 90f)
            } else {
                4f
            }

            val barHeight = wave.dp
            val fraction = wave / 90f

            val barColor = when {
                !isConnected -> Color.White.copy(alpha = 0.08f)
                fraction > 0.7f -> Color(0xFFE94560)
                fraction > 0.4f -> Color(0xFF7B2D8E)
                fraction > 0.2f -> Color(0xFF533483)
                else -> Color(0xFF0F3460)
            }

            Box(
                modifier = Modifier
                    .width(6.dp)
                    .height(barHeight)
                    .clip(RoundedCornerShape(3.dp))
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
