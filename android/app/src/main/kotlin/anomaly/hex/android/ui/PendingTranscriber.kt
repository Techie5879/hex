package anomaly.hex.android.ui

import anomaly.hex.android.HexApp
import anomaly.hex.android.core.pipeline.CancelToken
import anomaly.hex.android.core.pipeline.Vocabulary
import anomaly.hex.android.data.HistoryEntry
import java.io.File

/** Turns a recording saved by a failure or crash into text, from History. */
object PendingTranscriber {
    private fun file(app: HexApp, entry: HistoryEntry) = entry.pendingPcm?.let { File(app.captureDir, it) }

    fun run(app: HexApp, entry: HistoryEntry): String? {
        val pcm = file(app, entry) ?: return "The recording is gone."
        if (!pcm.isFile) return "The recording is gone."
        val settings = app.settings.current
        val model = settings.speechModel?.takeIf(app.models::isInstalled) ?: return "Get a speech model first."
        return try {
            val cancel = CancelToken()
            val terms = app.profile.current.liveTerms
            val raw = app.engine.transcribe(
                pcm.path,
                app.models.file(model).path,
                settings.language,
                Vocabulary.transcriptionPrompt(terms),
                settings.threads,
                0,
                cancel,
            )
            val processed = app.cleanupPipeline().process(raw, app.profile.policy, terms, entry.app, cancel)
            app.history.complete(entry.id, processed)
            pcm.delete()
            null
        } catch (error: Exception) {
            "Transcribing failed: ${error.message}"
        }
    }

    fun delete(app: HexApp, entry: HistoryEntry) {
        file(app, entry)?.delete()
        app.history.delete(entry.id)
    }
}
