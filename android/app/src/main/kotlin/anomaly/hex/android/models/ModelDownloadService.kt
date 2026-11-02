package anomaly.hex.android.models

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.os.IBinder
import androidx.core.app.ServiceCompat
import anomaly.hex.android.HexApp
import anomaly.hex.android.R
import anomaly.hex.android.core.models.ModelCatalog
import anomaly.hex.android.ui.MainActivity
import kotlin.concurrent.thread

/** Keeps a model download alive while Hex is in the background. Started only from the app's screens. */
class ModelDownloadService : Service() {
    private val manager by lazy { getSystemService(NotificationManager::class.java) }

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        val id = intent?.getStringExtra(EXTRA_ID)
        val artifact = id?.let(ModelCatalog::byId)
        if (artifact == null) {
            stopSelf(startId)
            return START_NOT_STICKY
        }
        manager.createNotificationChannel(NotificationChannel(CHANNEL, "Model downloads", NotificationManager.IMPORTANCE_LOW))
        ServiceCompat.startForeground(this, NOTIFICATION, notification(artifact.name), ServiceInfo.FOREGROUND_SERVICE_TYPE_DATA_SYNC)
        thread(name = "hex-model-download") {
            HexApp.get(this).models.download(artifact)
            ServiceCompat.stopForeground(this, ServiceCompat.STOP_FOREGROUND_REMOVE)
            stopSelf(startId)
        }
        return START_NOT_STICKY
    }

    private fun notification(name: String): Notification = Notification.Builder(this, CHANNEL)
        .setSmallIcon(R.drawable.ic_stat_hex)
        .setContentTitle("Downloading $name")
        .setContentIntent(MainActivity.pendingIntent(this))
        .setOngoing(true)
        .build()

    companion object {
        private const val CHANNEL = "hex-models"
        private const val NOTIFICATION = 2
        private const val EXTRA_ID = "id"

        fun start(context: Context, id: String) {
            context.startForegroundService(Intent(context, ModelDownloadService::class.java).putExtra(EXTRA_ID, id))
        }
    }
}
