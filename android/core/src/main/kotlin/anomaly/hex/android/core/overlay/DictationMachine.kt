package anomaly.hex.android.core.overlay

enum class RecordingMode { HandsFree, Hold }
enum class ProcessingPhase { Transcribing, CleaningUp, Polishing, Inserting }
enum class ErrorKind { MicrophoneDenied, MicrophoneBusy, ServiceStart, NoModel, Engine, TooQuiet, Timeout, Unknown }

/** Why finished text wasn't simply placed. An uncertain commit may already be in the field. */
enum class ResultReason { FocusMoved, Refused, Unconfirmed }

/** What the bubble and notification show. There is one of these at a time. */
sealed interface DictationState {
    data object Hidden : DictationState
    data object Idle : DictationState
    data class Starting(val mode: RecordingMode) : DictationState
    data class Recording(val mode: RecordingMode, val cancelArmed: Boolean = false) : DictationState
    data class Processing(val phase: ProcessingPhase) : DictationState
    data object Inserted : DictationState
    /** Final text exists but could not be placed where it was dictated for. */
    data class ResultReady(val text: String, val reason: ResultReason) : DictationState
    data class Failed(val kind: ErrorKind, val message: String, val canRetry: Boolean) : DictationState
}

enum class StopReason { Submit, Cancel, FocusSensitive, ScreenOff, ServiceLost, Silence }

sealed interface DictationEffect {
    data class StartMicrophone(val mode: RecordingMode) : DictationEffect
    data class StopMicrophone(val submit: Boolean, val reason: StopReason) : DictationEffect
    data object CancelProcessing : DictationEffect
    data object ClearResult : DictationEffect
    data object DiscardFailed : DictationEffect
}

/**
 * The one state machine behind the bubble. It runs no I/O: it maps gestures,
 * focus, and worker reports to a state plus effects, so every path that can
 * reach the microphone is visible and testable here.
 *  - Only an explicit tap or hold on a shown bubble starts capture.
 *  - A field that turns sensitive during capture ends capture at once.
 *  - The bubble can't be hidden while the microphone runs.
 */
class DictationMachine {
    var state: DictationState = DictationState.Hidden
        private set
    private var eligible = false

    val microphoneOn get() = state is DictationState.Starting || state is DictationState.Recording

    private fun go(next: DictationState, vararg effects: DictationEffect): List<DictationEffect> {
        state = next
        return effects.toList()
    }

    private fun resting() = if (eligible) DictationState.Idle else DictationState.Hidden

    fun onEligibility(show: Boolean, sensitive: Boolean): List<DictationEffect> {
        eligible = show
        return when (val s = state) {
            is DictationState.Starting, is DictationState.Recording -> {
                // Moving to another ordinary field keeps recording; a sensitive one stops it.
                if (sensitive) go(DictationState.Processing(ProcessingPhase.Transcribing), DictationEffect.StopMicrophone(true, StopReason.FocusSensitive))
                else emptyList()
            }
            DictationState.Hidden, DictationState.Idle -> go(resting())
            is DictationState.Failed -> if (!show && sensitive) go(DictationState.Hidden, DictationEffect.DiscardFailed) else emptyList()
            DictationState.Inserted -> if (!show && sensitive) go(DictationState.Hidden) else emptyList()
            is DictationState.ResultReady -> if (show && s.reason == ResultReason.Refused) go(s.copy(reason = ResultReason.FocusMoved)) else emptyList()
            is DictationState.Processing -> emptyList()
        }
    }

    fun onTap(): List<DictationEffect> = when (val s = state) {
        DictationState.Idle -> go(DictationState.Starting(RecordingMode.HandsFree), DictationEffect.StartMicrophone(RecordingMode.HandsFree))
        is DictationState.Recording -> if (s.mode == RecordingMode.HandsFree) submit() else emptyList()
        is DictationState.Failed -> go(resting(), DictationEffect.DiscardFailed)
        DictationState.Inserted -> go(resting())
        else -> emptyList()
    }

    fun onHoldStarted(): List<DictationEffect> = when (state) {
        DictationState.Idle -> go(DictationState.Starting(RecordingMode.Hold), DictationEffect.StartMicrophone(RecordingMode.Hold))
        else -> emptyList()
    }

    fun onHoldReleased(): List<DictationEffect> = when (val s = state) {
        is DictationState.Starting -> if (s.mode == RecordingMode.Hold) cancel() else emptyList()
        is DictationState.Recording -> if (s.mode == RecordingMode.Hold) submit() else emptyList()
        else -> emptyList()
    }

    fun onHoldCancelArmed(armed: Boolean): List<DictationEffect> {
        val s = state
        if (s is DictationState.Recording && s.mode == RecordingMode.Hold) go(s.copy(cancelArmed = armed))
        return emptyList()
    }

    fun onMicrophoneStarted(): List<DictationEffect> = when (val s = state) {
        is DictationState.Starting -> go(DictationState.Recording(s.mode))
        else -> emptyList()
    }

    fun onSubmitTapped(): List<DictationEffect> = if (state is DictationState.Recording) submit() else emptyList()

    fun onCancel(): List<DictationEffect> = when (state) {
        is DictationState.Starting, is DictationState.Recording -> cancel()
        is DictationState.Processing -> go(resting(), DictationEffect.CancelProcessing)
        is DictationState.ResultReady -> go(resting(), DictationEffect.ClearResult)
        is DictationState.Failed -> go(resting(), DictationEffect.DiscardFailed)
        DictationState.Inserted -> go(resting())
        else -> emptyList()
    }

    private fun submit() = go(DictationState.Processing(ProcessingPhase.Transcribing), DictationEffect.StopMicrophone(true, StopReason.Submit))

    private fun cancel() = go(resting(), DictationEffect.StopMicrophone(false, StopReason.Cancel))

    /** Stops capture from outside the bubble, keeping what was said. */
    fun onExternalStop(reason: StopReason): List<DictationEffect> =
        if (microphoneOn) go(DictationState.Processing(ProcessingPhase.Transcribing), DictationEffect.StopMicrophone(true, reason)) else emptyList()

    fun onProcessing(phase: ProcessingPhase): List<DictationEffect> =
        if (state is DictationState.Processing) go(DictationState.Processing(phase)) else emptyList()

    fun onInserted(): List<DictationEffect> = if (state is DictationState.Processing) go(DictationState.Inserted) else emptyList()

    fun onResultReady(text: String, reason: ResultReason): List<DictationEffect> =
        if (state is DictationState.Processing) go(DictationState.ResultReady(text, reason)) else emptyList()

    fun onFailure(kind: ErrorKind, message: String, canRetry: Boolean): List<DictationEffect> {
        val effects = if (microphoneOn) arrayOf<DictationEffect>(DictationEffect.StopMicrophone(false, StopReason.ServiceLost)) else emptyArray()
        return go(DictationState.Failed(kind, message, canRetry), *effects)
    }

    fun onRetry(): List<DictationEffect> =
        if (state is DictationState.Failed) go(DictationState.Processing(ProcessingPhase.Transcribing)) else emptyList()

    /** The inserted or error pill timed out, or a result was copied. */
    fun onSettled(): List<DictationEffect> = when (state) {
        DictationState.Inserted -> go(resting())
        is DictationState.Failed -> go(resting(), DictationEffect.DiscardFailed)
        is DictationState.ResultReady -> go(resting(), DictationEffect.ClearResult)
        else -> emptyList()
    }
}
