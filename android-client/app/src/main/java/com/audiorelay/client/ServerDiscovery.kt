package com.audiorelay.client

import android.util.Log
import androidx.compose.runtime.mutableStateListOf
import kotlinx.coroutines.*
import java.net.DatagramPacket
import java.net.DatagramSocket
import java.net.InetAddress
import org.json.JSONObject

data class DiscoveredServer(
    val address: String,
    val wsPort: Int,
    val webPort: Int,
    val name: String = "AudioRelay",
    var lastSeen: Long = System.currentTimeMillis()
)

class ServerDiscovery {
    private val scope = CoroutineScope(Dispatchers.IO + SupervisorJob())
    private val servers = mutableMapOf<String, DiscoveredServer>()
    val discoveredList = mutableStateListOf<DiscoveredServer>()

    fun startListening() {
        scope.launch {
            val socket = DatagramSocket()
            socket.soTimeout = 2000
            socket.broadcast = true

            try {
                while (isActive) {
                    try {
                        discover(socket)
                    } catch (e: Exception) {
                        if (isActive) {
                            Log.e("Discovery", "Error: ${e.message}")
                        }
                    }
                    delay(3000)
                }
            } finally {
                socket.close()
            }
        }
    }

    private suspend fun discover(socket: DatagramSocket) {
        withContext(Dispatchers.IO) {
            val msg = "DISCOVER_AUDIO_RELAY".toByteArray()
            val broadcast = InetAddress.getByName("255.255.255.255")
            val sendPacket = DatagramPacket(msg, msg.size, broadcast, 8082)
            socket.send(sendPacket)

            val buffer = ByteArray(1024)
            val recvPacket = DatagramPacket(buffer, buffer.size)
            socket.receive(recvPacket)

            val data = String(recvPacket.data, 0, recvPacket.length)
            val address = recvPacket.address.hostAddress ?: return@withContext

            Log.d("Discovery", "Response from $address: $data")

            val json = JSONObject(data)
            val wsPort = json.optInt("ws_port", 8080)
            val webPort = json.optInt("web_port", 8081)
            val name = json.optString("name", "AudioRelay")

            val key = "$address:$wsPort"
            servers[key] = DiscoveredServer(
                address = address,
                wsPort = wsPort,
                webPort = webPort,
                name = name,
                lastSeen = System.currentTimeMillis()
            )

            withContext(Dispatchers.Main) {
                updateList()
            }
        }
    }

    fun stopListening() {
        scope.cancel()
    }

    private fun updateList() {
        val now = System.currentTimeMillis()
        servers.entries.removeIf { now - it.value.lastSeen > 15000 }
        val newList = servers.values.sortedBy { it.address }
        discoveredList.clear()
        discoveredList.addAll(newList)
        Log.d("Discovery", "Server list: ${newList.size} servers")
    }
}
