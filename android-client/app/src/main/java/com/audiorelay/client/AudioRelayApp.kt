package com.audiorelay.client

import android.app.Application
import android.app.NotificationChannel
import android.app.NotificationManager
import android.os.Build

class AudioRelayApp : Application() {

    override fun onCreate() {
        super.onCreate()
        createNotificationChannel()
    }

    private fun createNotificationChannel() {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            val channel = NotificationChannel(
                NOTIFICATION_CHANNEL_ID,
                "Audio Relay Service",
                NotificationManager.IMPORTANCE_LOW
            ).apply {
                description = "Keeps audio streaming alive"
                setShowBadge(false)
            }
            val manager = getSystemService(NotificationManager::class.java)
            manager.createNotificationChannel(channel)
        }
    }

    companion object {
        const val NOTIFICATION_CHANNEL_ID = "audio_relay_service"
    }
}
