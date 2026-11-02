package anomaly.hex.android.service

import android.Manifest
import android.content.ClipData
import android.content.ClipDescription
import android.content.ClipboardManager
import android.content.Context
import android.content.pm.PackageManager
import android.os.Handler
import android.os.Looper
import android.os.PersistableBundle
import android.util.Log
import anomaly.hex.android.HexApp
import anomaly.hex.android.audio.AudioCapture
import anomaly.hex.android.audio.CaptureListener
import anomaly.hex.android.core.models.AudioLevels
import anomaly.hex.android.core.field.Eligibility
import anomaly.hex.android.core.overlay.DictationEffect
import anomaly.hex.android.core.overlay.DictationMachine
import anomaly.hex.android.core.overlay.DictationState
import anomaly.hex.android.core.overlay.ErrorKind
import anomaly.hex.android.core.overlay.GestureEvent
import anomaly.hex.android.core.overlay.ProcessingPhase
import anomaly.hex.android.core.overlay.RecordingMode
import anomaly.hex.android.core.overlay.ResultReason
import anomaly.hex.android.core.overlay.StopReason
import anomaly.hex.android.core.pipeline.CancelToken
import anomaly.hex.android.core.pipeline.CancelledException
import anomaly.hex.android.core.pipeline.CleanupPipeline
import anomaly.hex.android.core.pipeline.CleanupStage
import anomaly.hex.android.core.pipeline.Vocabulary
import anomaly.hex.android.data.HistoryEntry
import anomaly.hex.android.engine.EngineDeadlineException
import anomaly.hex.android.engine.EngineDiedException
import java.io.File
import java.nio.ByteBuffer
import java.nio.ByteOrder
import java.util.concurrent.Executors

sealed interface InsertOutcome {
    data object Inserted : InsertOutcome
    data object FocusMoved : InsertOutcome
    /** Sent to the field but not seen there. It may already be in; don't send it again by itself. */
    data object Unconfirmed : InsertOutcome
    data class Failed(val reason: String) : InsertOutcome
}

/**
 * Runs one dictation from gesture to inserted text. It owns the state machine,
 * the recording, and the worker thread. A capture is on disk from its first
 * second until it has safely turned into text or the person discards it.
 */
class DictationCoordinator(private val context: Context, private val host: HexAccessibilityService, private val render: (DictationState) -> Unit) : CaptureListener {
    private val app = HexApp.get(context)
    private val main = Handler(Looper.getMainLooper())
    private val worker = Executors.newSingleThreadExecutor { Thread(it, "hex-dictation") }
    private val machine = DictationMachine()
    private val captureDir = app.captureDir.apply { mkdirs() }

    private class Session(val id: String, val pcm: File, val createdMs: Long, val target: TextTarget?, val keepForRecovery: Boolean) {
        val cancel = CancelToken()
        var capture: AudioCapture? = null
        var mode: RecordingMode = RecordingMode.HandsFree
        var pendingHistoryId: Long? = null
        var historyId: Long? = null
        var audioName: String? = null
    }

    /** The one dictation the machine is about. Late callbacks from any other are ignored. */
    private var session: Session? = null
    private var pendingResult: String? = null
    private var foregroundStarting = false
    private var foregroundWanted = false

    /** Latest microphone peak for the bubble, read on the main thread. */
    var level = 0f
        private set
    var onLevelChanged: () -> Unit = {}
    val state get() = machine.state

    init {
        active = this
        worker.execute(::recoverCaptures)
    }

    private val settle = Runnable { apply(machine.onSettled()) }

    private fun apply(effects: List<DictationEffect>) {
        for (effect in effects) when (effect) {
            is DictationEffect.StartMicrophone -> start(effect.mode)
            is DictationEffect.StopMicrophone -> stop(effect.submit, effect.reason)
            DictationEffect.CancelProcessing -> cancelProcessing()
            DictationEffect.ClearResult -> pendingResult = null
            DictationEffect.DiscardFailed -> discardFailed()
        }
        val state = machine.state
        Log.i(TAG, "state ${state.javaClass.simpleName}")
        main.removeCallbacks(settle)
        when (state) {
            DictationState.Inserted -> main.postDelayed(settle, 900)
            is DictationState.Failed -> main.postDelayed(settle, 15_000)
            else -> Unit
        }
        render(state)
    }

    fun onEligibility(show: Boolean, sensitive: Boolean) = apply(machine.onEligibility(show, sensitive))

    fun onGesture(event: GestureEvent) {
        val effects = when (event) {
            GestureEvent.Tap -> machine.onTap()
            GestureEvent.HoldStarted -> machine.onHoldStarted()
            GestureEvent.HoldReleased -> machine.onHoldReleased()
            GestureEvent.HoldCancelled -> machine.onCancel()
            is GestureEvent.HoldCancelArmed -> machine.onHoldCancelArmed(event.armed)
            else -> emptyList()
        }
        apply(effects)
    }

    fun onSubmitTapped() = apply(machine.onSubmitTapped())
    fun onCancelTapped() = apply(machine.onCancel())
    fun onExternalStop(reason: StopReason) = apply(machine.onExternalStop(reason))
    fun onDismiss() = apply(machine.onSettled())

    // ---- starting ----

    private fun fail(kind: ErrorKind, message: String, canRetry: Boolean = false) = apply(machine.onFailure(kind, message, canRetry))

    private fun start(mode: RecordingMode) {
        if (context.checkSelfPermission(Manifest.permission.RECORD_AUDIO) != PackageManager.PERMISSION_GRANTED) {
            return fail(ErrorKind.MicrophoneDenied, "Microphone access is off. Open Hex to allow it.")
        }
        val settings = app.settings.current
        val model = settings.speechModel
        if (model == null || !app.models.isInstalled(model)) {
            return fail(ErrorKind.NoModel, "No speech model yet. Open Hex to get one.")
        }
        val now = System.currentTimeMillis()
        val id = "$now-${(1000..9999).random()}"
        val target = host.currentTarget()
        // The bubble may have been showing for a moment longer than the field deserved.
        if (target.eligibility !is Eligibility.Show) {
            apply(machine.onCancel())
            apply(machine.onEligibility(show = false, sensitive = true))
            return
        }
        // What a person chose not to keep can be retried right now, but never outlives this dictation.
        val keep = !target.noPersonalizedLearning && (settings.audioRetention.keepsNew || settings.historyRetention.keepsNew)
        val pcm = File(captureDir, if (keep) "$id.pcm" else "$id$EPHEMERAL_SUFFIX")
        val current = Session(id, pcm, now, target, keep).also { it.mode = mode }
        session = current
        app.engine.prewarm(app.models.file(model).path, settings.threads)
        startForeground()
    }

    private fun startForeground() {
        foregroundWanted = true
        if (foregroundStarting) return
        try {
            foregroundStarting = true
            CaptureForegroundService.start(context)
        } catch (error: RuntimeException) {
            foregroundStarting = false
            foregroundWanted = false
            session = null
            Log.e(TAG, "foreground start refused: ${error.javaClass.simpleName}")
            fail(ErrorKind.ServiceStart, "Android didn't let Hex start the microphone. Open Hex and try again.")
        }
    }

    /**
     * Android crashes an app that stops a foreground service before its startForeground ran.
     * A stop asked for in that window waits for the service to report in.
     */
    private fun stopForeground() {
        foregroundWanted = false
        if (!foregroundStarting) CaptureForegroundService.stop(context)
    }

    fun onForegroundReady(refusal: String?) = main.post {
        foregroundStarting = false
        if (!foregroundWanted) {
            if (refusal == null) CaptureForegroundService.stop(context)
            return@post
        }
        val current = session ?: return@post
        if (machine.state !is DictationState.Starting) return@post
        if (refusal != null) {
            foregroundWanted = false
            session = null
            fail(ErrorKind.ServiceStart, "Android blocked the microphone in the background. Open Hex and try again.")
            return@post
        }
        val silence = if (current.mode == RecordingMode.HandsFree) LONG_SILENCE_MS else 0
        current.capture = AudioCapture(context, current.pcm, this, silence).also { it.start() }
    }

    override fun onStarted() = apply(machine.onMicrophoneStarted())

    override fun onLevel(peak: Float) {
        level = peak
        onLevelChanged()
    }

    override fun onSilenced(silenced: Boolean) {
        if (silenced && machine.microphoneOn) {
            Log.w(TAG, "capture silenced by the system")
            apply(machine.onExternalStop(StopReason.ServiceLost))
        }
    }

    override fun onFailed(permission: Boolean) {
        val current = session
        if (current?.capture != null && current.pcm.length() > 0) {
            // Keep what was already said.
            apply(machine.onExternalStop(StopReason.ServiceLost))
        } else {
            stopForeground()
            session = null
            fail(
                if (permission) ErrorKind.MicrophoneDenied else ErrorKind.MicrophoneBusy,
                if (permission) "Microphone access is off. Open Hex to allow it." else "Couldn't open the microphone. Another app may be using it.",
            )
        }
    }

    override fun onLongSilence() {
        Log.i(TAG, "hands-free capture stopped after long silence")
        apply(machine.onExternalStop(StopReason.Silence))
    }

    // ---- stopping ----

    private fun stop(submit: Boolean, reason: StopReason) {
        val current = session ?: return stopForeground()
        val tail = if (submit && reason == StopReason.Submit && current.mode == RecordingMode.Hold) POST_RELEASE_TAIL_MS else 0
        if (!submit) {
            session = null
            stopForeground()
        }
        worker.execute {
            current.capture?.stop(tail)
            if (submit) process(current, retry = false) else current.pcm.delete()
        }
    }

    /** Cancel takes effect at once. The worker then notices and leaves no trace. */
    private fun cancelProcessing() {
        session?.let { current ->
            current.cancel.cancel()
            session = null
            stopForeground()
            worker.execute { discard(current) }
        }
    }

    /** Cancelled: nothing of this dictation stays. Runs on the worker, so after any step in progress. */
    private fun discard(session: Session) {
        session.pcm.delete()
        session.pendingHistoryId?.let(app.history::delete)
        session.historyId?.let(app.history::delete)
        session.audioName?.let(app.archive::discard)
    }

    /** A dismissed failure: what may be recovered stays in History, and the rest goes now. */
    private fun discardFailed() {
        val current = session ?: return
        session = null
        if (!current.keepForRecovery) worker.execute { current.pcm.delete() }
    }

    /** Retries the last failed dictation from its saved audio. */
    fun onRetry() {
        val current = session ?: return onDismiss()
        apply(machine.onRetry())
        startForeground()
        worker.execute { process(current, retry = true) }
    }

    /** Runs on the main thread only while [session] is still the one the machine is about. */
    private fun forCurrent(session: Session, block: () -> Unit) {
        main.post { if (this.session === session && !session.cancel.cancelled) block() }
    }

    private fun phase(session: Session, phase: ProcessingPhase, note: String) = forCurrent(session) {
        apply(machine.onProcessing(phase))
        CaptureForegroundService.update(context, note)
    }

    private fun process(session: Session, retry: Boolean) {
        val settings = app.settings.current
        try {
            val model = settings.speechModel?.takeIf(app.models::isInstalled) ?: throw MissingModel()
            val samples = (session.pcm.length() / 2).toInt()
            if (samples < AudioLevels.MIN_DICTATION_SAMPLES) {
                session.pcm.delete()
                return finish(session) { apply(machine.onFailure(ErrorKind.TooQuiet, "That was too short to transcribe.", false)) }
            }
            if (isSilent(session.pcm)) {
                session.pcm.delete()
                return finish(session) { apply(machine.onFailure(ErrorKind.TooQuiet, "Too quiet. Check your mic.", false)) }
            }
            // Models invent text for silence, so nothing without speech reaches one, or the field.
            if (app.engine.hasSpeech(app.models.speechDetector().path, session.pcm.path) == false) {
                session.pcm.delete()
                return finish(session) { apply(machine.onFailure(ErrorKind.TooQuiet, "No speech heard. Check your mic.", false)) }
            }
            val noLearning = session.target?.noPersonalizedLearning == true
            if (!retry && !noLearning) session.audioName = app.archive.retain(session.pcm, session.id, settings.audioRetention)
            if (session.cancel.cancelled) throw CancelledException()

            phase(session, ProcessingPhase.Transcribing, "Transcribing")
            val profile = app.profile.current
            val terms = profile.liveTerms
            val deadline = CleanupPipeline.asrDeadlineMs(samples.toLong(), settings.asrDeadlineFactor)
            val prompt = Vocabulary.transcriptionPrompt(terms)
            val raw = app.engine.transcribe(
                session.pcm.path,
                app.models.file(model).path,
                settings.language,
                prompt,
                settings.threads,
                deadline,
                session.cancel,
            )
            if (raw.isBlank() || Vocabulary.isPromptEcho(raw, prompt)) {
                session.pcm.delete()
                return finish(session) { apply(machine.onFailure(ErrorKind.TooQuiet, "No speech heard. Check your mic.", false)) }
            }

            val target = session.target
            val processed = app.cleanupPipeline().process(raw, app.profile.policy, terms, host.appLabel(target?.packageName), session.cancel) { stage ->
                when (stage) {
                    CleanupStage.LocalCleanup -> phase(session, ProcessingPhase.CleaningUp, "Cleaning up")
                    CleanupStage.Polish -> phase(session, ProcessingPhase.Polishing, "Polishing")
                }
            }
            Log.i(TAG, "processed stages=${processed.stagesRun} failures=${processed.failures.size}")

            if (session.cancel.cancelled) throw CancelledException()
            record(session, settings.historyRetention.keepsNew && !noLearning, target, processed)
            session.pcm.delete()
            forCurrent(session) {
                apply(machine.onProcessing(ProcessingPhase.Inserting))
                host.insert(processed.text, target) { outcome ->
                    forCurrent(session) {
                        this.session = null
                        stopForeground()
                        val reason = when (outcome) {
                            InsertOutcome.Inserted -> null
                            InsertOutcome.FocusMoved -> ResultReason.FocusMoved
                            InsertOutcome.Unconfirmed -> ResultReason.Unconfirmed
                            is InsertOutcome.Failed -> ResultReason.Refused.also { Log.w(TAG, "insert failed: ${outcome.reason}") }
                        }
                        if (reason == null) {
                            apply(machine.onInserted())
                        } else {
                            pendingResult = processed.text
                            apply(machine.onResultReady(processed.text, reason))
                        }
                    }
                }
            }
        } catch (_: CancelledException) {
            // cancelProcessing queues the discard, which runs once this returns.
        } catch (_: EngineDeadlineException) {
            failWithAudio(session, ErrorKind.Timeout, "Transcribing took too long. Try a smaller model.")
        } catch (_: EngineDiedException) {
            failWithAudio(session, ErrorKind.Engine, "The speech engine stopped. Your recording is safe.")
        } catch (_: MissingModel) {
            failWithAudio(session, ErrorKind.NoModel, "The speech model is missing. Your recording is safe.")
        } catch (error: Exception) {
            Log.e(TAG, "dictation failed: ${error.javaClass.simpleName}")
            failWithAudio(session, ErrorKind.Unknown, "Transcribing failed. Your recording is safe.")
        }
    }

    private class MissingModel : Exception()

    private fun finish(session: Session, then: () -> Unit) = forCurrent(session) {
        this.session = null
        stopForeground()
        then()
    }

    private fun record(session: Session, keepText: Boolean, target: TextTarget?, processed: anomaly.hex.android.core.pipeline.ProcessedText) {
        val pending = session.pendingHistoryId
        when {
            keepText && pending != null -> app.history.complete(pending, processed)
            keepText -> session.historyId = app.history.add(
                HistoryEntry(0, session.createdMs, host.appLabel(target?.packageName), processed.raw, processed.text, processed.stageNames, processed.failureNotes, session.audioName),
            )
            pending != null -> app.history.delete(pending)
        }
    }

    /**
     * Keeps the audio for Retry, and in History too unless the person chose to keep neither
     * or the field asked not to be learned from. Those are deleted once the failure is dismissed.
     */
    private fun failWithAudio(session: Session, kind: ErrorKind, message: String) {
        if (session.keepForRecovery && session.pendingHistoryId == null) {
            session.pendingHistoryId = runCatching { addPending(session.pcm, session.createdMs, host.appLabel(session.target?.packageName)) }.getOrNull()
        }
        forCurrent(session) {
            stopForeground()
            apply(machine.onFailure(kind, message, canRetry = true))
        }
    }

    private fun addPending(pcm: File, createdMs: Long, app: String = "Hex"): Long? {
        if (this.app.history.list(limit = 50).any { it.pendingPcm == pcm.name }) return null
        return this.app.history.add(HistoryEntry(0, createdMs, app, "", "", "", "Not transcribed yet", null, pendingPcm = pcm.name))
    }

    /** After a crash or kill: any capture left on disk appears in History, ready to transcribe. */
    private fun recoverCaptures() {
        for (file in captureDir.listFiles { f -> f.extension == "pcm" }.orEmpty()) {
            // Not to be kept, or nothing in it. Audio from a dictation still running isn't here yet:
            // this runs before the first capture of a new process.
            if (file.length() == 0L || file.name.endsWith(EPHEMERAL_SUFFIX)) {
                file.delete()
                continue
            }
            val created = file.name.substringBefore('-').toLongOrNull() ?: file.lastModified()
            runCatching { addPending(file, created) }
        }
    }

    private fun isSilent(pcm: File): Boolean {
        val bytes = ByteArray(AudioLevels.WINDOW_SAMPLES * 2)
        val shorts = ShortArray(AudioLevels.WINDOW_SAMPLES)
        pcm.inputStream().buffered().use { input ->
            while (true) {
                val read = input.readNBytes(bytes, 0, bytes.size)
                if (read <= 0) return true
                ByteBuffer.wrap(bytes, 0, read).order(ByteOrder.LITTLE_ENDIAN).asShortBuffer().get(shorts, 0, read / 2)
                if (AudioLevels.rmsDbfs(shorts, 0, read / 2) >= AudioLevels.SILENCE_THRESHOLD_DBFS) return false
            }
        }
    }

    // ---- result actions ----

    fun onInsertHere() {
        val text = pendingResult ?: return onDismiss()
        host.insert(text, null) { outcome ->
            if (outcome == InsertOutcome.Inserted) {
                pendingResult = null
                apply(machine.onSettled())
            }
        }
    }

    fun onCopyResult() {
        val text = pendingResult ?: return onDismiss()
        val clip = ClipData.newPlainText("Hex dictation", text).apply {
            description.extras = PersistableBundle().apply { putBoolean(ClipDescription.EXTRA_IS_SENSITIVE, true) }
        }
        context.getSystemService(ClipboardManager::class.java).setPrimaryClip(clip)
        pendingResult = null
        apply(machine.onSettled())
    }

    /** For the debug receiver: the real inserter, against whatever has focus now. */
    fun debugInsert(text: String, done: (InsertOutcome) -> Unit) = host.insert(text, null, done)

    fun onServiceLost() {
        apply(machine.onExternalStop(StopReason.ServiceLost))
        if (active === this) active = null
    }

    companion object {
        private const val TAG = "HexDictation"
        private const val POST_RELEASE_TAIL_MS = 200L
        private const val EPHEMERAL_SUFFIX = ".ephemeral.pcm"
        private const val LONG_SILENCE_MS = 2 * 60 * 1000L

        @Volatile var active: DictationCoordinator? = null
            private set
    }
}
