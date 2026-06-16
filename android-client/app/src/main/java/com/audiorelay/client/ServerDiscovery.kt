package com.audiorelay.client

import android.util.Log
import androidx.compose.runtime.mutableStateListOf
import kotlinx.coroutines.*
import java.net.DatagramPacket
import java.net.DatagramSocket
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
            val socket = DatagramSocket(8082)
            socket.soTimeout = 3000
            val buffer = ByteArray(1024)

            Log.d("Discovery", "Listening on port 8082")

            while (isActive) {
                try {
                    val packet = DatagramPacket(buffer, buffer.size)
                    socket.receive(packet)
                    val data = String(packet.data, 0, packet.length)
                    val address = packet.address.hostAddress ?: continue

                    Log.d("Discovery", "Received from $address: $data")

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

                    updateList()
                } catch (e: java.net.SocketTimeoutException) {
                    cleanupOldServers()
                } catch (e: Exception) {
                    if (isActive) {
                        Log.e("Discovery", "Error: ${e.message}")
                    }
                }
            }
            socket.close()
        }
    }

    fun stopListening() {
        scope.cancel()
    }

    private fun cleanupOldServers() {
        val now = System.currentTimeMillis()
        val before = servers.size
        servers.entries.removeIf { now - it.value.lastSeen > 10000 }
        if (servers.size != before) {
            updateList()
        }
    }

    private fun updateList() {
        val newList = servers.values.sortedBy { it.address }
        discoveredList.clear()
        discoveredList.addAll(newList)
        Log.d("Discovery", "Server list updated: ${newList.size} servers")
    }
}
