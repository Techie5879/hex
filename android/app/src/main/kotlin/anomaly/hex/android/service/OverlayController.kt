package anomaly.hex.android.service

import android.animation.ValueAnimator
import android.content.Context
import android.content.Intent
import android.graphics.PixelFormat
import android.os.Handler
import android.os.Looper
import android.view.Gravity
import android.view.WindowInsets
import android.view.WindowManager
import android.view.animation.DecelerateInterpolator
import anomaly.hex.android.HexApp
import anomaly.hex.android.core.overlay.BubblePosition
import anomaly.hex.android.core.overlay.DictationState
import anomaly.hex.android.core.overlay.Edge
import anomaly.hex.android.core.overlay.GestureEvent
import anomaly.hex.android.data.DeviceSettings
import anomaly.hex.android.overlay.BubbleView
import anomaly.hex.android.overlay.HandleView
import anomaly.hex.android.ui.MainActivity
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.flow.collectLatest
import kotlinx.coroutines.launch

/**
 * One accessibility overlay window, just big enough for the bubble. It can't take
 * focus, so the keyboard and the field under it stay exactly as they were.
 */
class OverlayController(private val context: Context) {
    private val windows = context.getSystemService(WindowManager::class.java)
    private val app = HexApp.get(context)
    private val main = Handler(Looper.getMainLooper())
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
    private lateinit var coordinator: DictationCoordinator
    private lateinit var bubble: BubbleView
    private val params = WindowManager.LayoutParams(
        WindowManager.LayoutParams.WRAP_CONTENT,
        WindowManager.LayoutParams.WRAP_CONTENT,
        WindowManager.LayoutParams.TYPE_ACCESSIBILITY_OVERLAY,
        WindowManager.LayoutParams.FLAG_NOT_FOCUSABLE or
            WindowManager.LayoutParams.FLAG_LAYOUT_NO_LIMITS or
            WindowManager.LayoutParams.FLAG_NOT_TOUCH_MODAL,
        PixelFormat.TRANSLUCENT,
    ).apply {
        gravity = Gravity.TOP or Gravity.END
        title = "HexBubble"
    }
    private var attached = false
    private var keyboardTop: Int? = null
    private var searchField = false
    private var edge = Edge.Right
    private var yFraction = BubblePosition.DEFAULT.yFraction
    private var dragStartX = 0
    private var dragStartY = 0
    private var dragging = false
    private var snap: ValueAnimator? = null
    private var idleShrunk = false
    private val shrinkTimer = Runnable {
        idleShrunk = true
        applyAppearance(app.settings.current)
    }

    fun attach(coordinator: DictationCoordinator) {
        this.coordinator = coordinator
        bubble = BubbleView(context, Actions())
        coordinator.onLevelChanged = { bubble.setLevel(coordinator.level) }
        val start = app.settings.current
        edge = start.edge
        yFraction = start.yFraction
        scope.launch {
            app.settings.flow.collectLatest { settings ->
                edge = settings.edge
                yFraction = settings.yFraction
                applyAppearance(settings)
                if (attached && !dragging) place()
            }
        }
    }

    private inner class Actions : BubbleView.Actions {
        override fun onGesture(event: GestureEvent) {
            wake()
            coordinator.onGesture(event)
        }

        override fun onDragMoved(rawDx: Float, rawDy: Float) {
            if (!dragging) {
                dragging = true
                snap?.cancel()
                dragStartX = params.x
                dragStartY = params.y
            }
            // With the window anchored to the right edge, its x grows to the left.
            params.x = if (edge == Edge.Right) dragStartX - rawDx.toInt() else dragStartX + rawDx.toInt()
            params.y = dragStartY + rawDy.toInt()
            if (attached) windows.updateViewLayout(bubble, params)
        }

        override fun onDragEnded() {
            dragging = false
            val width = screenWidth()
            val fromLeft = if (edge == Edge.Right) width - params.x - bubble.width else params.x
            val center = fromLeft + bubble.width / 2f
            val next = BubblePosition.nearestEdge(center, width)
            val fraction = ((params.y + bubble.height / 2f) / screenHeight()).coerceIn(0f, 1f)
            app.settings.update { it.copy(edge = next, yFraction = fraction) }
            edge = next
            yFraction = fraction
            place(animated = true)
        }

        override fun onCancel() = coordinator.onCancelTapped()
        override fun onRetry() = coordinator.onRetry()
        override fun onInsertHere() = coordinator.onInsertHere()
        override fun onCopy() = coordinator.onCopyResult()
        override fun onTouched() = wake()

        override fun onMoveToOtherEdge() {
            val next = if (edge == Edge.Right) Edge.Left else Edge.Right
            app.settings.update { it.copy(edge = next) }
        }

        override fun onOpenApp() {
            context.startActivity(Intent(context, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
            coordinator.onDismiss()
        }
    }

    fun render(state: DictationState) {
        val visible = state !is DictationState.Hidden
        bubble.render(state)
        if (visible && !attached) {
            place(measureOnly = true)
            runCatching { windows.addView(bubble, params) }.onSuccess { attached = true }
        } else if (!visible && attached) {
            snap?.cancel()
            runCatching { windows.removeView(bubble) }
            attached = false
        }
        if (state is DictationState.Idle) scheduleShrink() else wake()
        if (attached && !dragging) main.post { if (attached) place() }
    }

    private fun wake() {
        main.removeCallbacks(shrinkTimer)
        if (idleShrunk) {
            idleShrunk = false
            applyAppearance(app.settings.current)
        }
    }

    private fun scheduleShrink() {
        main.removeCallbacks(shrinkTimer)
        val settings = app.settings.current
        if (settings.shrinkWhenIdle || settings.shrinkToDot) main.postDelayed(shrinkTimer, SHRINK_AFTER_MS)
    }

    private fun applyAppearance(settings: DeviceSettings) {
        if (!::bubble.isInitialized) return
        val shrink = when {
            !idleShrunk && !(searchField && settings.shrinkInSearch) -> HandleView.Shrink.None
            settings.shrinkToDot -> HandleView.Shrink.Dot
            else -> HandleView.Shrink.Small
        }
        bubble.setAppearance(settings.bubbleSizeDp, settings.bubbleOpacity, shrink)
    }

    fun setKeyboardTop(top: Int?) {
        if (keyboardTop == top) return
        keyboardTop = top
        if (attached && !dragging) place()
    }

    fun setSearchField(search: Boolean) {
        if (searchField == search) return
        searchField = search
        applyAppearance(app.settings.current)
    }

    fun onConfigurationChanged() {
        if (attached && !dragging) place()
    }

    private fun screenWidth() = windows.currentWindowMetrics.bounds.width()
    private fun screenHeight() = windows.currentWindowMetrics.bounds.height()

    private fun place(animated: Boolean = false, measureOnly: Boolean = false) {
        bubble.setEdge(edge)
        val metrics = windows.currentWindowMetrics
        val bars = metrics.windowInsets.getInsetsIgnoringVisibility(WindowInsets.Type.systemBars() or WindowInsets.Type.displayCutout())
        val height = metrics.bounds.height()
        val bubbleHeight = bubble.height.takeIf { it > 0 } ?: (context.resources.displayMetrics.density * 56).toInt()
        val margin = (context.resources.displayMetrics.density * 8).toInt()
        val bottom = keyboardTop ?: (height - bars.bottom)
        val top = BubblePosition(edge, yFraction).top(height, bubbleHeight, bars.top, bottom, margin)
        params.gravity = Gravity.TOP or (if (edge == Edge.Right) Gravity.END else Gravity.START)
        val fromX = params.x
        val fromY = params.y
        val edgeInset = if (edge == Edge.Right) bars.right else bars.left
        if (measureOnly) {
            params.x = edgeInset
            params.y = top
            return
        }
        if (!attached) return
        snap?.cancel()
        if (!animated) {
            params.x = edgeInset
            params.y = top
            windows.updateViewLayout(bubble, params)
            return
        }
        snap = ValueAnimator.ofFloat(0f, 1f).apply {
            duration = 180
            interpolator = DecelerateInterpolator()
            addUpdateListener {
                if (!attached) return@addUpdateListener
                val t = it.animatedValue as Float
                params.x = (fromX + (edgeInset - fromX) * t).toInt()
                params.y = (fromY + (top - fromY) * t).toInt()
                windows.updateViewLayout(bubble, params)
            }
            start()
        }
    }

    fun destroy() {
        scope.cancel()
        main.removeCallbacksAndMessages(null)
        if (attached) runCatching { windows.removeView(bubble) }
        attached = false
    }

    private companion object {
        const val SHRINK_AFTER_MS = 4000L
    }
}
