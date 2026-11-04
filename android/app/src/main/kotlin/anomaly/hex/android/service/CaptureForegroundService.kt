package anomaly.hex.android.service

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.os.IBinder
import android.util.Log
import androidx.core.app.ServiceCompat
import anomaly.hex.android.R
import anomaly.hex.android.ui.MainActivity

/**
 * Shows the system that Hex is recording, and keeps the process important while
 * a dictation is in progress. It exists only for the length of one dictation.
 */
class CaptureForegroundService : Service() {
    override fun onBind(intent: Intent?): IBinder? = null

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        val coordinator = DictationCoordinator.active
        when (intent?.action) {
            ACTION_STOP -> coordinator?.onSubmitTapped()
            ACTION_CANCEL -> coordinator?.onCancelTapped()
            ACTION_UPDATE -> getSystemService(NotificationManager::class.java).notify(
                NOTIFICATION,
                notification(intent.getStringExtra(EXTRA_TEXT) ?: "Working…", recording = false),
            )
            else -> {
                try {
                    getSystemService(NotificationManager::class.java)
                        .createNotificationChannel(NotificationChannel(CHANNEL, "Dictation", NotificationManager.IMPORTANCE_LOW))
                    ServiceCompat.startForeground(
                        this,
                        NOTIFICATION,
                        notification("Listening", recording = true),
                        ServiceInfo.FOREGROUND_SERVICE_TYPE_MICROPHONE,
                    )
                    coordinator?.onForegroundReady(null)
                } catch (error: RuntimeException) {
                    // The OS refused while-in-use microphone access. Say so instead of recording nothing.
                    Log.e(TAG, "startForeground refused: ${error.javaClass.simpleName}")
                    coordinator?.onForegroundReady(error.javaClass.simpleName)
                    stopSelf(startId)
                }
            }
        }
        return START_NOT_STICKY
    }

    private fun notification(text: String, recording: Boolean): Notification {
        val builder = Notification.Builder(this, CHANNEL)
            .setSmallIcon(R.drawable.ic_stat_hex)
            .setContentTitle("Hex")
            .setContentText(text)
            .setOngoing(true)
            .setContentIntent(MainActivity.pendingIntent(this))
            .setForegroundServiceBehavior(Notification.FOREGROUND_SERVICE_IMMEDIATE)
        if (recording) {
            builder.addAction(Notification.Action.Builder(null, "Finish", action(ACTION_STOP)).build())
            builder.addAction(Notification.Action.Builder(null, "Cancel", action(ACTION_CANCEL)).build())
        }
        return builder.build()
    }

    private fun action(name: String) = PendingIntent.getService(
        this,
        name.hashCode(),
        Intent(this, CaptureForegroundService::class.java).setAction(name),
        PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
    )

    companion object {
        private const val TAG = "HexCapture"
        private const val CHANNEL = "hex-dictation"
        private const val NOTIFICATION = 1
        private const val ACTION_STOP = "stop"
        private const val ACTION_CANCEL = "cancel"
        private const val ACTION_UPDATE = "update"
        private const val EXTRA_TEXT = "text"

        fun start(context: Context) {
            context.startForegroundService(Intent(context, CaptureForegroundService::class.java))
        }

        fun update(context: Context, text: String) {
            runCatching {
                context.startService(Intent(context, CaptureForegroundService::class.java).setAction(ACTION_UPDATE).putExtra(EXTRA_TEXT, text))
            }
        }

        fun stop(context: Context) {
            context.stopService(Intent(context, CaptureForegroundService::class.java))
        }
    }
}
