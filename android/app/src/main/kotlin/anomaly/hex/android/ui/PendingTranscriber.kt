package anomaly.hex.android.ui

import anomaly.hex.android.HexApp
import anomaly.hex.android.core.models.AudioLevels
import anomaly.hex.android.core.pipeline.CancelToken
import anomaly.hex.android.core.pipeline.CleanupPipeline
import anomaly.hex.android.core.pipeline.Vocabulary
import anomaly.hex.android.data.HistoryEntry
import java.io.File

/** Turns a recording saved by a failure or crash into text, under the same rules as a dictation. */
object PendingTranscriber {
    sealed interface Outcome {
        /** Saved in History. */
        data object Saved : Outcome
        /** Text history is off, so this is the only copy: show it, don't keep it. */
        data class TextOnly(val text: String) : Outcome
        data class Failed(val reason: String) : Outcome
    }

    private fun file(app: HexApp, entry: HistoryEntry) = entry.pendingPcm?.let { File(app.captureDir, it) }

    fun run(app: HexApp, entry: HistoryEntry): Outcome {
        val pcm = file(app, entry)?.takeIf { it.isFile } ?: return Outcome.Failed("The recording is gone.")
        val settings = app.settings.current
        val model = settings.speechModel?.takeIf(app.models::isInstalled) ?: return Outcome.Failed("Get a speech model first.")
        return try {
            val cancel = CancelToken()
            val terms = app.profile.current.liveTerms
            if (pcm.length() / 2 < AudioLevels.MIN_DICTATION_SAMPLES) return Outcome.Failed("This recording is too short to transcribe.")
            if (pcm.inputStream().buffered().use(AudioLevels::isSilent)) return Outcome.Failed("This recording is too quiet to transcribe.")
            if (app.engine.hasSpeech(app.models.speechDetector().path, pcm.path) == false) return Outcome.Failed("No speech heard in this recording.")
            val prompt = Vocabulary.transcriptionPrompt(terms)
            val raw = app.engine.transcribe(
                pcm.path,
                app.models.file(model).path,
                settings.language,
                prompt,
                settings.threads,
                CleanupPipeline.asrDeadlineMs(pcm.length() / 2, settings.asrDeadlineFactor),
                cancel,
            )
            if (raw.isBlank() || Vocabulary.isPromptEcho(raw, prompt)) return Outcome.Failed("No speech heard in this recording.")
            val processed = app.cleanupPipeline().process(raw, app.profile.policy, terms, entry.app, cancel)
            // Published before the only other copy goes. Off for either setting means that one isn't kept.
            val audioName = app.archive.retain(pcm, pcm.nameWithoutExtension, settings.audioRetention)
            if (settings.historyRetention.keepsNew) {
                app.history.complete(entry.id, processed, audioName)
                pcm.delete()
                Outcome.Saved
            } else {
                app.history.delete(entry.id)
                pcm.delete()
                Outcome.TextOnly(processed.text)
            }
        } catch (error: Exception) {
            Outcome.Failed("Transcribing failed. The recording is still here.")
        }
    }

    fun delete(app: HexApp, entry: HistoryEntry) {
        file(app, entry)?.delete()
        app.history.delete(entry.id)
    }
}
