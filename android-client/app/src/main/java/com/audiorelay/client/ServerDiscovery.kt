package com.audiorelay.client

import android.util.Log
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
    var onServersUpdated: ((List<DiscoveredServer>) -> Unit)? = null

    fun startListening() {
        scope.launch {
            val socket = DatagramSocket(8082)
            socket.soTimeout = 5000
            val buffer = ByteArray(1024)

            Log.d("Discovery", "Listening for server broadcasts on port 8082")

            while (isActive) {
                try {
                    val packet = DatagramPacket(buffer, buffer.size)
                    socket.receive(packet)
                    val data = String(packet.data, 0, packet.length)
                    val address = packet.address.hostAddress ?: continue

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

                    Log.d("Discovery", "Found server: $address:$wsPort")
                    withContext(Dispatchers.Main) {
                        onServersUpdated?.invoke(getServers())
                    }
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
        servers.entries.removeIf { now - it.value.lastSeen > 15000 }
        if (servers.size != before) {
            Log.d("Discovery", "Cleaned up ${before - servers.size} old servers")
        }
    }

    fun getServers(): List<DiscoveredServer> {
        cleanupOldServers()
        return servers.values.sortedBy { it.address }
    }
}
