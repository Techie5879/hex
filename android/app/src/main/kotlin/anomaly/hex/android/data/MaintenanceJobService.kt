package anomaly.hex.android.data

import android.app.job.JobInfo
import android.app.job.JobParameters
import android.app.job.JobScheduler
import android.app.job.JobService
import android.content.ComponentName
import android.content.Context
import anomaly.hex.android.HexApp
import java.io.File
import kotlin.concurrent.thread

/** Applies retention and syncs on an interval, whether or not anything else is running. */
class MaintenanceJobService : JobService() {
    override fun onStartJob(params: JobParameters): Boolean {
        val app = HexApp.get(this)
        thread(name = "hex-maintenance") {
            val settings = app.settings.current
            app.history.prune(settings.historyRetention)
            app.archive.prune(settings.audioRetention)
            app.history.prunePending(settings.audioRetention).forEach { File(app.captureDir, it).delete() }
            app.sync.runBlocking()
            jobFinished(params, false)
        }
        return true
    }

    override fun onStopJob(params: JobParameters) = true

    companion object {
        private const val JOB_ID = 41
        private const val SYNC_INTERVAL_MS = 60 * 60 * 1000L

        fun schedule(context: Context) {
            val scheduler = context.getSystemService(JobScheduler::class.java)
            if (scheduler.getPendingJob(JOB_ID)?.intervalMillis == SYNC_INTERVAL_MS) return
            scheduler.schedule(
                JobInfo.Builder(JOB_ID, ComponentName(context, MaintenanceJobService::class.java))
                    .setPeriodic(SYNC_INTERVAL_MS)
                    .setRequiresBatteryNotLow(true)
                    .build(),
            )
        }
    }
}
