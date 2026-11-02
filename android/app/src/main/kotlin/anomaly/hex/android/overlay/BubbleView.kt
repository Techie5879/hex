package anomaly.hex.android.overlay

import android.animation.ValueAnimator
import android.annotation.SuppressLint
import android.content.Context
import android.graphics.Canvas
import android.graphics.Color
import android.graphics.Paint
import android.graphics.Rect
import android.graphics.RectF
import android.os.Handler
import android.os.Looper
import android.os.SystemClock
import android.util.TypedValue
import android.view.Gravity
import android.view.HapticFeedbackConstants
import android.view.MotionEvent
import android.view.View
import android.view.accessibility.AccessibilityNodeInfo
import android.view.animation.LinearInterpolator
import android.widget.LinearLayout
import android.widget.TextView
import anomaly.hex.android.core.overlay.DictationState
import anomaly.hex.android.core.overlay.Edge
import anomaly.hex.android.core.overlay.GestureClassifier
import anomaly.hex.android.core.overlay.GestureEvent
import anomaly.hex.android.core.overlay.ProcessingPhase
import anomaly.hex.android.core.overlay.RecordingMode
import anomaly.hex.android.core.overlay.ResultReason
import kotlin.math.PI
import kotlin.math.cos
import kotlin.math.max
import kotlin.math.min
import kotlin.math.sin

/** Colours for the overlay: a dark plate that reads on light and dark apps alike. */
private object Ink {
    val plate = Color.rgb(0x17, 0x19, 0x1D)
    val plateRaised = Color.rgb(0x20, 0x23, 0x29)
    val stroke = Color.argb(46, 255, 255, 255)
    val text = Color.argb(240, 255, 255, 255)
    val muted = Color.argb(143, 255, 255, 255)
    val contrast = Color.argb(245, 255, 255, 255)
    val hot = Color.rgb(0xFF, 0x95, 0x00)
    val danger = Color.rgb(0xE5, 0x4D, 0x42)
    val success = Color.rgb(0x34, 0xC7, 0x59)
}

private fun Context.dp(value: Float) = TypedValue.applyDimension(TypedValue.COMPLEX_UNIT_DIP, value, resources.displayMetrics)

/**
 * The floating control. The handle sits at the screen edge in every state and keeps
 * its place under a finger; other controls appear beside it, towards the middle of
 * the screen. A window this size leaves everything else touchable.
 */
@SuppressLint("ViewConstructor", "ClickableViewAccessibility")
class BubbleView(context: Context, private val actions: Actions) : LinearLayout(context) {
    interface Actions {
        fun onGesture(event: GestureEvent)
        fun onDragMoved(rawDx: Float, rawDy: Float)
        fun onDragEnded()
        fun onCancel()
        fun onRetry()
        fun onInsertHere()
        fun onCopy()
        fun onTouched()
        fun onMoveToOtherEdge()
        fun onOpenApp()
    }

    private val handler2 = Handler(Looper.getMainLooper())
    private val gestures = GestureClassifier(
        holdMs = HOLD_MS,
        slop = context.dp(10f),
        cancelDistance = context.dp(90f),
    )
    val handle = HandleView(context)
    private val cancelButton = RoundButton(context, "Cancel dictation") { actions.onCancel() }
    private val scope = DotScope(context)
    private val label = TextView(context).apply {
        setTextColor(Ink.text)
        setTextSize(TypedValue.COMPLEX_UNIT_SP, 14f)
        gravity = Gravity.CENTER_VERTICAL
        maxLines = 3
    }
    private val pill = PillLayout(context)
    private val secondary = TextButton(context)
    private val primary = TextButton(context)

    private var edge = Edge.Right
    private var state: DictationState = DictationState.Idle
    private var downRawX = 0f
    private var downRawY = 0f
    private var recordingStart = 0L
    private val tickTimer = object : Runnable {
        override fun run() {
            if (state is DictationState.Recording) {
                val mode = (state as DictationState.Recording).mode
                if (mode == RecordingMode.HandsFree) scope.setClock(((SystemClock.elapsedRealtime() - recordingStart) / 1000).toInt())
                handler2.postDelayed(this, 500)
            }
        }
    }

    init {
        orientation = HORIZONTAL
        gravity = Gravity.CENTER_VERTICAL
        clipChildren = false
        clipToPadding = false
        pill.addView(scope)
        pill.addView(label)
        pill.addView(secondary)
        pill.addView(primary)
        handle.setOnTouchListener { _, event -> onHandleTouch(event) }
        handle.setOnClickListener { actions.onGesture(GestureEvent.Tap) }
        handle.accessibilityDelegate = object : AccessibilityDelegate() {
            override fun onInitializeAccessibilityNodeInfo(host: View, info: AccessibilityNodeInfo) {
                super.onInitializeAccessibilityNodeInfo(host, info)
                info.addAction(AccessibilityNodeInfo.AccessibilityAction(ACTION_MOVE, "Move to the other edge"))
            }

            override fun performAccessibilityAction(host: View, action: Int, args: android.os.Bundle?): Boolean {
                if (action == ACTION_MOVE) {
                    actions.onMoveToOtherEdge()
                    return true
                }
                return super.performAccessibilityAction(host, action, args)
            }
        }
        rebuild()
        render(state)
    }

    fun setEdge(edge: Edge) {
        if (this.edge == edge) return
        this.edge = edge
        rebuild()
    }

    fun setAppearance(sizeDp: Int, opacity: Int, shrink: HandleView.Shrink) {
        handle.configure(context.dp(sizeDp.toFloat()), opacity / 100f, shrink)
    }

    fun setLevel(level: Float) {
        handle.level = level
        scope.push(level)
    }

    private fun onHandleTouch(event: MotionEvent): Boolean {
        val now = event.eventTime
        when (event.actionMasked) {
            MotionEvent.ACTION_DOWN -> {
                downRawX = event.rawX
                downRawY = event.rawY
                gestures.onDown(now, event.rawX, event.rawY)
                handler2.removeCallbacks(holdCheck)
                handler2.postDelayed(holdCheck, HOLD_MS)
                handle.pressedDown(true)
                actions.onTouched()
            }
            MotionEvent.ACTION_MOVE -> dispatch(gestures.onMove(now, event.rawX, event.rawY))
            MotionEvent.ACTION_UP -> {
                handler2.removeCallbacks(holdCheck)
                handle.pressedDown(false)
                dispatch(gestures.onUp(now))
            }
            MotionEvent.ACTION_CANCEL -> {
                handler2.removeCallbacks(holdCheck)
                handle.pressedDown(false)
                dispatch(gestures.onCancel())
            }
        }
        return true
    }

    private val holdCheck = Runnable { dispatch(gestures.onTimer(SystemClock.uptimeMillis())) }

    private fun dispatch(events: List<GestureEvent>) {
        for (event in events) when (event) {
            is GestureEvent.DragMoved -> actions.onDragMoved(event.dx, event.dy)
            GestureEvent.DragEnded -> actions.onDragEnded()
            GestureEvent.Tap -> handle.performClick()
            GestureEvent.HoldStarted -> {
                handle.performHapticFeedback(HapticFeedbackConstants.LONG_PRESS)
                actions.onGesture(event)
            }
            GestureEvent.HoldCancelled, GestureEvent.HoldReleased, is GestureEvent.HoldCancelArmed -> actions.onGesture(event)
        }
    }

    /** Keeps the row on screen: the message wraps rather than pushing its actions off the edge. */
    private fun fitLabel(actionsDp: Float) {
        val screen = context.getSystemService(android.view.WindowManager::class.java).currentWindowMetrics.bounds.width()
        label.maxWidth = (screen - context.dp(actionsDp)).toInt().coerceAtLeast(context.dp(80f).toInt())
    }

    fun render(state: DictationState) {
        val previous = this.state
        this.state = state
        scope.visibility = GONE
        label.visibility = GONE
        secondary.visibility = GONE
        primary.visibility = GONE
        pill.visibility = GONE
        cancelButton.visibility = GONE
        when (state) {
            DictationState.Hidden, DictationState.Idle -> {
                handle.mode = HandleView.Mode.Idle
                handle.contentDescription = "Hex dictation. Tap to dictate hands-free, or hold to talk."
            }
            is DictationState.Starting -> {
                handle.mode = HandleView.Mode.Opening
                handle.contentDescription = "Opening the microphone"
                if (state.mode == RecordingMode.HandsFree) cancelButton.visibility = VISIBLE
            }
            is DictationState.Recording -> {
                if (previous !is DictationState.Recording) {
                    recordingStart = SystemClock.elapsedRealtime()
                    handler2.removeCallbacks(tickTimer)
                    handler2.post(tickTimer)
                }
                when (state.mode) {
                    RecordingMode.Hold -> {
                        handle.mode = if (state.cancelArmed) HandleView.Mode.Armed else HandleView.Mode.Recording
                        handle.contentDescription = "Recording. Release to insert."
                    }
                    RecordingMode.HandsFree -> {
                        handle.mode = HandleView.Mode.Submit
                        handle.contentDescription = "Finish and insert"
                        cancelButton.visibility = VISIBLE
                        pill.visibility = VISIBLE
                        scope.visibility = VISIBLE
                    }
                }
            }
            is DictationState.Processing -> {
                handle.mode = HandleView.Mode.Working
                handle.contentDescription = "Working"
                pill.visibility = VISIBLE
                label.visibility = VISIBLE
                label.text = when (state.phase) {
                    ProcessingPhase.Transcribing -> "Transcribing…"
                    ProcessingPhase.CleaningUp -> "Cleaning up…"
                    ProcessingPhase.Polishing -> "Polishing…"
                    ProcessingPhase.Inserting -> "Inserting…"
                }
                cancelButton.visibility = VISIBLE
            }
            DictationState.Inserted -> {
                handle.mode = HandleView.Mode.Done
                handle.contentDescription = "Inserted"
            }
            is DictationState.ResultReady -> {
                handle.mode = HandleView.Mode.Idle
                handle.contentDescription = "Text ready"
                pill.visibility = VISIBLE
                label.visibility = VISIBLE
                label.text = when (state.reason) {
                    ResultReason.FocusMoved -> "Text ready"
                    ResultReason.Refused -> "Couldn't insert"
                    ResultReason.Unconfirmed -> "Check the field"
                }
                secondary.visibility = VISIBLE
                secondary.bind("Copy", Ink.text) { actions.onCopy() }
                primary.visibility = VISIBLE
                primary.bind("Insert", Ink.contrast) { actions.onInsertHere() }
                cancelButton.visibility = VISIBLE
            }
            is DictationState.Failed -> {
                handle.mode = HandleView.Mode.Error
                handle.contentDescription = state.message
                pill.visibility = VISIBLE
                label.visibility = VISIBLE
                label.text = state.message
                fitLabel(actionsDp = 56f + 44f + 100f + 44f)
                primary.visibility = VISIBLE
                if (state.canRetry) {
                    primary.bind("Retry", Ink.contrast) { actions.onRetry() }
                } else {
                    primary.bind("Open Hex", Ink.contrast) { actions.onOpenApp() }
                }
                cancelButton.visibility = VISIBLE
            }
        }
    }

    private fun rebuild() {
        removeAllViews()
        val gap = context.dp(6f).toInt()
        val children = listOf<View>(cancelButton, pill, handle).let { if (edge == Edge.Right) it else it.reversed() }
        for (child in children) {
            addView(child, LayoutParams(LayoutParams.WRAP_CONTENT, LayoutParams.WRAP_CONTENT).apply { setMargins(gap / 2, 0, gap / 2, 0) })
        }
    }

    override fun onDetachedFromWindow() {
        handler2.removeCallbacksAndMessages(null)
        super.onDetachedFromWindow()
    }

    companion object {
        private const val HOLD_MS = 220L
        private const val ACTION_MOVE = 0x7f0b0001
    }
}

/** A dark rounded pill that holds a scope or a message and its actions. */
@SuppressLint("ViewConstructor")
private class PillLayout(context: Context) : LinearLayout(context) {
    private val paint = Paint(Paint.ANTI_ALIAS_FLAG).apply { color = Ink.plate }
    private val border = Paint(Paint.ANTI_ALIAS_FLAG).apply { color = Ink.stroke; style = Paint.Style.STROKE; strokeWidth = context.dp(1f) }
    private val rect = RectF()

    init {
        orientation = HORIZONTAL
        gravity = Gravity.CENTER_VERTICAL
        setWillNotDraw(false)
        val h = context.dp(14f).toInt()
        setPadding(h, context.dp(6f).toInt(), h, context.dp(6f).toInt())
        minimumHeight = context.dp(44f).toInt()
        elevation = context.dp(4f)
    }

    override fun onDraw(canvas: Canvas) {
        rect.set(0f, 0f, width.toFloat(), height.toFloat())
        val r = min(height / 2f, context.dp(22f))
        canvas.drawRoundRect(rect, r, r, paint)
        canvas.drawRoundRect(rect, r, r, border)
    }
}

/** Short text action inside the pill. */
@SuppressLint("AppCompatCustomView")
private class TextButton(context: Context) : TextView(context) {
    init {
        val h = context.dp(10f).toInt()
        setPadding(h, context.dp(10f).toInt(), h, context.dp(10f).toInt())
        setTextSize(TypedValue.COMPLEX_UNIT_SP, 14f)
        typeface = android.graphics.Typeface.DEFAULT_BOLD
        minimumHeight = context.dp(44f).toInt()
        maxLines = 1
    }

    fun bind(text: String, color: Int, action: () -> Unit) {
        this.text = text
        setTextColor(color)
        setOnClickListener { action() }
    }
}

/** Circle button: the cancel beside the pill. */
@SuppressLint("ViewConstructor")
private class RoundButton(context: Context, description: String, action: () -> Unit) : View(context) {
    private val plate = Paint(Paint.ANTI_ALIAS_FLAG).apply { color = Ink.plate }
    private val border = Paint(Paint.ANTI_ALIAS_FLAG).apply { color = Ink.stroke; style = Paint.Style.STROKE; strokeWidth = context.dp(1f) }
    private val mark = Paint(Paint.ANTI_ALIAS_FLAG).apply {
        color = Ink.text
        style = Paint.Style.STROKE
        strokeWidth = context.dp(2f)
        strokeCap = Paint.Cap.ROUND
    }

    init {
        contentDescription = description
        setOnClickListener { action() }
        elevation = context.dp(4f)
    }

    override fun onMeasure(widthMeasureSpec: Int, heightMeasureSpec: Int) {
        val s = context.dp(44f).toInt()
        setMeasuredDimension(s, s)
    }

    override fun onDraw(canvas: Canvas) {
        val c = width / 2f
        val r = c - context.dp(1f)
        canvas.drawCircle(c, c, r, plate)
        canvas.drawCircle(c, c, r, border)
        val a = context.dp(6f)
        canvas.drawLine(c - a, c - a, c + a, c + a, mark)
        canvas.drawLine(c + a, c - a, c - a, c + a, mark)
    }
}

/** Hex's Dot scope: a small plate of dots lit by the recent microphone level. */
@SuppressLint("ViewConstructor")
private class DotScope(context: Context) : View(context) {
    private val on = Paint(Paint.ANTI_ALIAS_FLAG).apply { color = Ink.contrast }
    private val off = Paint(Paint.ANTI_ALIAS_FLAG).apply { color = Ink.stroke }
    private val clockPaint = Paint(Paint.ANTI_ALIAS_FLAG).apply {
        color = Ink.muted
        textSize = context.dp(13f)
    }
    private val history = FloatArray(COLUMNS)
    private var clock = "0:00"

    fun push(level: Float) {
        System.arraycopy(history, 1, history, 0, COLUMNS - 1)
        history[COLUMNS - 1] = level
        invalidate()
    }

    fun setClock(seconds: Int) {
        clock = "%d:%02d".format(seconds / 60, seconds % 60)
        invalidate()
    }

    override fun onMeasure(widthMeasureSpec: Int, heightMeasureSpec: Int) {
        setMeasuredDimension(context.dp(130f).toInt(), context.dp(32f).toInt())
    }

    override fun onDraw(canvas: Canvas) {
        val pitch = context.dp(5f)
        val radius = context.dp(1.5f)
        val mid = height / 2f
        for (column in 0 until COLUMNS) {
            val lit = (history[column] * 6f * ROWS).toInt().coerceIn(0, ROWS)
            for (row in 0 until ROWS) {
                val y = mid + (row - ROWS / 2) * pitch
                val distance = kotlin.math.abs(row - ROWS / 2)
                canvas.drawCircle(column * pitch + radius, y, radius, if (distance * 2 < lit) on else off)
            }
        }
        canvas.drawText(clock, width - clockPaint.measureText(clock), mid + clockPaint.textSize / 3, clockPaint)
    }

    private companion object {
        const val COLUMNS = 19
        const val ROWS = 5
    }
}

/** The bubble itself. Its touch target stays at least 48 dp whatever the visual size. */
class HandleView(context: Context) : View(context) {
    enum class Mode { Idle, Opening, Recording, Armed, Submit, Working, Done, Error }
    enum class Shrink { None, Small, Dot }

    var mode = Mode.Idle
        set(value) {
            if (field == value) return
            field = value
            animation()
            invalidate()
        }
    var level = 0f
        set(value) {
            field = value
            if (mode == Mode.Recording) invalidate()
        }
    private var visualSize = context.dp(44f)
    private var idleAlpha = 0.9f
    private var shrink = Shrink.None
    private var pressed = false
    private val plate = Paint(Paint.ANTI_ALIAS_FLAG)
    private val border = Paint(Paint.ANTI_ALIAS_FLAG).apply { color = Ink.stroke; style = Paint.Style.STROKE; strokeWidth = context.dp(1f) }
    private val ink = Paint(Paint.ANTI_ALIAS_FLAG).apply {
        style = Paint.Style.STROKE
        strokeCap = Paint.Cap.ROUND
        strokeJoin = Paint.Join.ROUND
    }
    private val rect = RectF()
    private var spin = 0f
    private var spinner: ValueAnimator? = null

    fun configure(size: Float, opacity: Float, shrink: Shrink) {
        visualSize = size
        idleAlpha = opacity
        this.shrink = shrink
        requestLayout()
        invalidate()
    }

    fun pressedDown(down: Boolean) {
        pressed = down
        invalidate()
    }

    private fun animation() {
        val working = mode == Mode.Working || mode == Mode.Opening
        if (working && spinner == null) {
            spinner = ValueAnimator.ofFloat(0f, 360f).apply {
                duration = 900
                repeatCount = ValueAnimator.INFINITE
                interpolator = LinearInterpolator()
                addUpdateListener {
                    spin = it.animatedValue as Float
                    invalidate()
                }
                start()
            }
        } else if (!working) {
            spinner?.cancel()
            spinner = null
        }
    }

    override fun onMeasure(widthMeasureSpec: Int, heightMeasureSpec: Int) {
        val size = max(visualSize + context.dp(4f), context.dp(48f)).toInt()
        setMeasuredDimension(size, size)
    }

    override fun onLayout(changed: Boolean, left: Int, top: Int, right: Int, bottom: Int) {
        super.onLayout(changed, left, top, right, bottom)
        // At the screen edge, gesture navigation would otherwise take a hold or drag as Back.
        systemGestureExclusionRects = listOf(Rect(0, 0, width, height))
    }

    private fun effectiveSize(): Float = when {
        mode != Mode.Idle || pressed -> visualSize
        shrink == Shrink.Small -> visualSize * 0.68f
        shrink == Shrink.Dot -> context.dp(12f)
        else -> visualSize
    }

    override fun onDraw(canvas: Canvas) {
        val cx = width / 2f
        val cy = height / 2f
        val size = effectiveSize()
        val half = size / 2f
        rect.set(cx - half, cy - half, cx + half, cy + half)
        val corner = if (shrink == Shrink.Dot && mode == Mode.Idle && !pressed) half else size * 0.3f
        val alpha = if (mode == Mode.Idle && !pressed) idleAlpha else 1f
        plate.color = if (mode == Mode.Armed) Ink.danger else Ink.plate
        plate.alpha = (Color.alpha(plate.color) * alpha).toInt()
        canvas.drawRoundRect(rect, corner, corner, plate)
        border.alpha = (46 * alpha).toInt()
        canvas.drawRoundRect(rect, corner, corner, border)
        ink.alpha = (255 * alpha).toInt()
        if (shrink == Shrink.Dot && mode == Mode.Idle && !pressed) return
        val unit = size / 44f
        ink.strokeWidth = context.dp(2.2f) * unit.coerceAtLeast(0.7f)
        when (mode) {
            Mode.Idle -> {
                ink.color = withAlpha(Ink.text, alpha)
                hexagon(canvas, cx, cy, size * 0.26f)
            }
            Mode.Recording -> {
                ink.color = Ink.hot
                val ringHalf = half + context.dp(1f)
                rect.set(cx - ringHalf, cy - ringHalf, cx + ringHalf, cy + ringHalf)
                ink.strokeWidth = context.dp(2f)
                canvas.drawRoundRect(rect, corner, corner, ink)
                bars(canvas, cx, cy, size)
            }
            Mode.Armed -> {
                ink.color = Color.WHITE
                val a = size * 0.18f
                canvas.drawLine(cx - a, cy - a, cx + a, cy + a, ink)
                canvas.drawLine(cx + a, cy - a, cx - a, cy + a, ink)
            }
            Mode.Submit -> {
                ink.color = Ink.contrast
                check(canvas, cx, cy, size)
            }
            Mode.Opening, Mode.Working -> {
                ink.color = Ink.text
                val r = size * 0.24f
                rect.set(cx - r, cy - r, cx + r, cy + r)
                canvas.drawArc(rect, spin, 270f, false, ink)
            }
            Mode.Done -> {
                ink.color = Ink.success
                check(canvas, cx, cy, size)
            }
            Mode.Error -> {
                ink.color = Ink.hot
                val a = size * 0.18f
                canvas.drawLine(cx, cy - a, cx, cy + a * 0.3f, ink)
                canvas.drawLine(cx, cy + a, cx, cy + a, ink)
            }
        }
    }

    private fun withAlpha(color: Int, factor: Float) =
        Color.argb((Color.alpha(color) * factor).toInt(), Color.red(color), Color.green(color), Color.blue(color))

    private fun hexagon(canvas: Canvas, cx: Float, cy: Float, r: Float) {
        var px = 0f
        var py = 0f
        for (i in 0..6) {
            val angle = (PI / 3 * i - PI / 2).toFloat()
            val x = cx + r * cos(angle)
            val y = cy + r * sin(angle)
            if (i > 0) canvas.drawLine(px, py, x, y, ink)
            px = x
            py = y
        }
        canvas.drawPoint(cx, cy, ink)
    }

    private fun bars(canvas: Canvas, cx: Float, cy: Float, size: Float) {
        val pitch = size * 0.14f
        val shape = floatArrayOf(0.45f, 0.8f, 1f, 0.8f, 0.45f)
        val level = (level * 5f).coerceIn(0.12f, 1f)
        for (i in shape.indices) {
            val h = size * 0.34f * shape[i] * level
            val x = cx + (i - 2) * pitch
            canvas.drawLine(x, cy - h, x, cy + h, ink)
        }
    }

    private fun check(canvas: Canvas, cx: Float, cy: Float, size: Float) {
        val a = size * 0.2f
        canvas.drawLine(cx - a, cy, cx - a * 0.2f, cy + a * 0.75f, ink)
        canvas.drawLine(cx - a * 0.2f, cy + a * 0.75f, cx + a, cy - a * 0.7f, ink)
    }

    override fun onDetachedFromWindow() {
        spinner?.cancel()
        spinner = null
        super.onDetachedFromWindow()
    }
}
