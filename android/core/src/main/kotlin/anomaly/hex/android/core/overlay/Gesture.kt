package anomaly.hex.android.core.overlay

import kotlin.math.abs
import kotlinx.serialization.Serializable
import kotlin.math.hypot

@Serializable
enum class Edge { Left, Right }

sealed interface GestureEvent {
    data object Tap : GestureEvent
    data object HoldStarted : GestureEvent
    data object HoldReleased : GestureEvent
    data object HoldCancelled : GestureEvent
    data class HoldCancelArmed(val armed: Boolean) : GestureEvent
    data class DragMoved(val dx: Float, val dy: Float) : GestureEvent
    data object DragEnded : GestureEvent
}

/**
 * Classifies one touch stream on the bubble, so a drag can never start the
 * microphone. Moving past [slop] before [holdMs] drags. Staying still past
 * [holdMs] starts a hold, which submits on release and cancels if released
 * after sliding [cancelDistance] away. A short still touch is a tap. Nothing
 * reaches the microphone until one of these is decided.
 */
class GestureClassifier(
    private val holdMs: Long = 220,
    private val slop: Float = 24f,
    private val cancelDistance: Float = 180f,
) {
    private enum class Phase { Idle, Pressed, Dragging, Holding }
    private var phase = Phase.Idle
    private var downAt = 0L
    private var downX = 0f
    private var downY = 0f
    private var armed = false

    val active get() = phase != Phase.Idle

    fun onDown(time: Long, x: Float, y: Float): List<GestureEvent> {
        phase = Phase.Pressed
        downAt = time
        downX = x
        downY = y
        armed = false
        return emptyList()
    }

    fun onTimer(time: Long): List<GestureEvent> {
        if (phase == Phase.Pressed && time - downAt >= holdMs) {
            phase = Phase.Holding
            return listOf(GestureEvent.HoldStarted)
        }
        return emptyList()
    }

    fun onMove(time: Long, x: Float, y: Float): List<GestureEvent> = when (phase) {
        Phase.Idle -> emptyList()
        Phase.Pressed -> {
            val moved = hypot(x - downX, y - downY) > slop
            when {
                moved -> {
                    phase = Phase.Dragging
                    listOf(GestureEvent.DragMoved(x - downX, y - downY))
                }
                time - downAt >= holdMs -> onTimer(time)
                else -> emptyList()
            }
        }
        Phase.Dragging -> listOf(GestureEvent.DragMoved(x - downX, y - downY))
        Phase.Holding -> {
            val now = abs(x - downX) > cancelDistance || abs(y - downY) > cancelDistance
            if (now != armed) {
                armed = now
                listOf(GestureEvent.HoldCancelArmed(now))
            } else {
                emptyList()
            }
        }
    }

    fun onUp(time: Long): List<GestureEvent> {
        val previous = phase
        phase = Phase.Idle
        return when (previous) {
            Phase.Idle -> emptyList()
            Phase.Pressed -> if (time - downAt >= holdMs) {
                listOf(GestureEvent.HoldStarted, GestureEvent.HoldReleased)
            } else {
                listOf(GestureEvent.Tap)
            }
            Phase.Dragging -> listOf(GestureEvent.DragEnded)
            Phase.Holding -> listOf(if (armed) GestureEvent.HoldCancelled else GestureEvent.HoldReleased)
        }
    }

    /** The system took the touch away. Holding fingers no longer mean anything: cancel, never submit. */
    fun onCancel(): List<GestureEvent> {
        val previous = phase
        phase = Phase.Idle
        return when (previous) {
            Phase.Holding -> listOf(GestureEvent.HoldCancelled)
            Phase.Dragging -> listOf(GestureEvent.DragEnded)
            Phase.Pressed, Phase.Idle -> emptyList()
        }
    }
}

/** Where the bubble sits: an edge and a fraction of screen height, clamped above the keyboard. */
data class BubblePosition(val edge: Edge, val yFraction: Float) {
    /** The window's top in pixels, kept inside the screen and above a visible keyboard. */
    fun top(screenHeight: Int, bubbleHeight: Int, topInset: Int, bottomLimit: Int, margin: Int): Int {
        val highest = topInset + margin
        val lowest = (bottomLimit - bubbleHeight - margin).coerceAtLeast(highest)
        return ((screenHeight * yFraction).toInt() - bubbleHeight / 2).coerceIn(highest, lowest)
    }

    companion object {
        val DEFAULT = BubblePosition(Edge.Right, 0.4f)
        fun nearestEdge(centerX: Float, screenWidth: Int) = if (centerX < screenWidth / 2f) Edge.Left else Edge.Right
    }
}
