package anomaly.hex.android.core

import anomaly.hex.android.core.overlay.BubblePosition
import anomaly.hex.android.core.overlay.DictationEffect
import anomaly.hex.android.core.overlay.DictationMachine
import anomaly.hex.android.core.overlay.DictationState
import anomaly.hex.android.core.overlay.Edge
import anomaly.hex.android.core.overlay.ErrorKind
import anomaly.hex.android.core.overlay.GestureClassifier
import anomaly.hex.android.core.overlay.GestureEvent
import anomaly.hex.android.core.overlay.ProcessingPhase
import anomaly.hex.android.core.overlay.RecordingMode
import anomaly.hex.android.core.overlay.ResultReason
import anomaly.hex.android.core.overlay.StopReason
import anomaly.hex.android.core.retention.Retention
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class OverlayTest {
    private val g = GestureClassifier(holdMs = 200, slop = 20f, cancelDistance = 100f)

    @Test
    fun aQuickTouchIsATap() {
        g.onDown(0, 10f, 10f)
        assertEquals(listOf(GestureEvent.Tap), g.onUp(100))
    }

    @Test
    fun jitterInsideTheSlopIsStillATap() {
        g.onDown(0, 10f, 10f)
        assertEquals(emptyList<GestureEvent>(), g.onMove(50, 20f, 20f))
        assertEquals(listOf(GestureEvent.Tap), g.onUp(100))
    }

    @Test
    fun aHoldStartsOnlyAfterTheThresholdAndSubmitsOnRelease() {
        g.onDown(0, 10f, 10f)
        assertEquals(emptyList<GestureEvent>(), g.onTimer(150))
        assertEquals(listOf(GestureEvent.HoldStarted), g.onTimer(200))
        assertEquals(listOf(GestureEvent.HoldReleased), g.onUp(900))
    }

    @Test
    fun aDragNeverBecomesAHold() {
        g.onDown(0, 10f, 10f)
        assertEquals(listOf(GestureEvent.DragMoved(70f, 0f)), g.onMove(50, 80f, 10f))
        assertEquals(emptyList<GestureEvent>(), g.onTimer(500))
        assertEquals(listOf(GestureEvent.DragMoved(70f, 90f)), g.onMove(600, 80f, 100f))
        assertEquals(listOf(GestureEvent.DragEnded), g.onUp(700))
    }

    @Test
    fun slidingAwayArmsCancelAndReturningDisarms() {
        g.onDown(0, 0f, 0f)
        g.onTimer(200)
        assertEquals(listOf(GestureEvent.HoldCancelArmed(true)), g.onMove(300, 150f, 0f))
        assertEquals(emptyList<GestureEvent>(), g.onMove(350, 160f, 0f))
        assertEquals(listOf(GestureEvent.HoldCancelArmed(false)), g.onMove(400, 20f, 0f))
        assertEquals(listOf(GestureEvent.HoldCancelArmed(true)), g.onMove(450, 0f, -150f))
        assertEquals(listOf(GestureEvent.HoldCancelled), g.onUp(500))
    }

    @Test
    fun aSystemCancelNeverSubmits() {
        g.onDown(0, 0f, 0f)
        g.onTimer(200)
        assertEquals(listOf(GestureEvent.HoldCancelled), g.onCancel())
        g.onDown(0, 0f, 0f)
        assertEquals(emptyList<GestureEvent>(), g.onCancel())
    }

    private fun idle() = DictationMachine().apply { onEligibility(show = true, sensitive = false) }

    @Test
    fun nothingStartsTheMicrophoneUntilAnExplicitGesture() {
        val m = DictationMachine()
        assertEquals(emptyList<DictationEffect>(), m.onTap())
        assertEquals(emptyList<DictationEffect>(), m.onHoldStarted())
        m.onEligibility(show = true, sensitive = false)
        assertEquals(DictationState.Idle, m.state)
        assertFalse(m.microphoneOn)
    }

    @Test
    fun tapRecordsHandsFreeAndTapAgainSubmits() {
        val m = idle()
        assertEquals(listOf(DictationEffect.StartMicrophone(RecordingMode.HandsFree)), m.onTap())
        m.onMicrophoneStarted()
        assertEquals(DictationState.Recording(RecordingMode.HandsFree), m.state)
        assertEquals(listOf(DictationEffect.StopMicrophone(true, StopReason.Submit)), m.onTap())
        assertEquals(DictationState.Processing(ProcessingPhase.Transcribing), m.state)
    }

    @Test
    fun holdSubmitsOnReleaseAndAQuickReleaseBeforeTheMicOpensCancels() {
        val m = idle()
        m.onHoldStarted()
        assertEquals(listOf(DictationEffect.StopMicrophone(false, StopReason.Cancel)), m.onHoldReleased())
        assertEquals(DictationState.Idle, m.state)
        m.onHoldStarted()
        m.onMicrophoneStarted()
        assertEquals(listOf(DictationEffect.StopMicrophone(true, StopReason.Submit)), m.onHoldReleased())
    }

    @Test
    fun cancelFromEveryRecordingState() {
        val m = idle()
        m.onTap()
        assertEquals(listOf(DictationEffect.StopMicrophone(false, StopReason.Cancel)), m.onCancel())
        m.onTap()
        m.onMicrophoneStarted()
        assertEquals(listOf(DictationEffect.StopMicrophone(false, StopReason.Cancel)), m.onCancel())
        assertFalse(m.microphoneOn)
        m.onTap()
        m.onMicrophoneStarted()
        m.onTap()
        assertEquals(listOf(DictationEffect.CancelProcessing), m.onCancel())
        assertEquals(DictationState.Idle, m.state)
    }

    @Test
    fun aSensitiveFieldEndsCaptureAndAnOrdinaryFieldDoesNot() {
        val m = idle()
        m.onTap()
        m.onMicrophoneStarted()
        assertEquals(emptyList<DictationEffect>(), m.onEligibility(show = true, sensitive = false))
        assertTrue(m.microphoneOn)
        // An ordinary loss of focus doesn't drop speech, and the bubble stays because the mic is on.
        assertEquals(emptyList<DictationEffect>(), m.onEligibility(show = false, sensitive = false))
        assertTrue(m.state is DictationState.Recording)
        assertEquals(listOf(DictationEffect.StopMicrophone(true, StopReason.FocusSensitive)), m.onEligibility(show = false, sensitive = true))
        assertFalse(m.microphoneOn)
    }

    @Test
    fun errorsHideOverSensitiveFieldsAndAreDismissable() {
        val m = idle()
        m.onFailure(ErrorKind.Engine, "boom", canRetry = true)
        m.onEligibility(show = true, sensitive = false)
        assertTrue(m.state is DictationState.Failed)
        m.onEligibility(show = false, sensitive = true)
        assertEquals(DictationState.Hidden, m.state)
        val n = idle()
        n.onFailure(ErrorKind.MicrophoneDenied, "x", canRetry = false)
        n.onCancel()
        assertEquals(DictationState.Idle, n.state)
    }

    @Test
    fun aResultReadyIsKeptAcrossFocusChangesUntilActedOn() {
        val m = idle()
        m.onTap()
        m.onMicrophoneStarted()
        m.onTap()
        m.onResultReady("hello", ResultReason.Refused)
        m.onEligibility(show = false, sensitive = true)
        assertEquals(DictationState.ResultReady("hello", ResultReason.Refused), m.state)
        m.onEligibility(show = true, sensitive = false)
        assertEquals(DictationState.ResultReady("hello", ResultReason.FocusMoved), m.state)
        assertEquals(listOf(DictationEffect.ClearResult), m.onCancel())
    }

    @Test
    fun aLostMicrophoneStopsCapture() {
        val m = idle()
        m.onTap()
        m.onMicrophoneStarted()
        assertEquals(listOf(DictationEffect.StopMicrophone(false, StopReason.ServiceLost)), m.onFailure(ErrorKind.MicrophoneBusy, "x", false))
        assertFalse(m.microphoneOn)
    }

    @Test
    fun screenOffKeepsWhatWasSaid() {
        val m = idle()
        m.onTap()
        m.onMicrophoneStarted()
        assertEquals(listOf(DictationEffect.StopMicrophone(true, StopReason.ScreenOff)), m.onExternalStop(StopReason.ScreenOff))
    }

    @Test
    fun bubbleClampsInsideTheScreenAndAboveTheKeyboard() {
        val p = BubblePosition(Edge.Right, 0.9f)
        assertEquals(2000 - 100 - 8, p.top(screenHeight = 2400, bubbleHeight = 100, topInset = 60, bottomLimit = 2000, margin = 8))
        assertEquals(68, BubblePosition(Edge.Left, 0f).top(2400, 100, 60, 2400, 8))
        // A keyboard covering nearly everything still leaves the bubble on screen.
        assertEquals(68, p.top(2400, 100, 60, 100, 8))
        assertEquals(Edge.Left, BubblePosition.nearestEdge(100f, 1080))
        assertEquals(Edge.Right, BubblePosition.nearestEdge(800f, 1080))
    }

    @Test
    fun retentionPolicies() {
        val day = 86_400_000L
        assertFalse(Retention.Forever.isExpired(0, 100 * 365 * day))
        assertTrue(Retention.Forever.keepsNew)
        assertFalse(Retention.Off.keepsNew)
        assertFalse("Off keeps what exists", Retention.Off.isExpired(0, 100 * 365 * day))
        assertTrue(Retention.Day.isExpired(0, day))
        assertFalse(Retention.Day.isExpired(0, day - 1))
        assertTrue(Retention.Week.isExpired(0, 7 * day))
        assertTrue(Retention.Month.isExpired(0, 30 * day))
        assertEquals(Retention.Forever, Retention.fromWire("forever", Retention.Off))
        assertEquals(Retention.Off, Retention.fromWire("nonsense", Retention.Off))
    }
}
