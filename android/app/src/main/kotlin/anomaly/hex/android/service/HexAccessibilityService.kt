package anomaly.hex.android.service

import android.accessibilityservice.AccessibilityService
import android.accessibilityservice.InputMethod
import android.app.KeyguardManager
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.content.res.Configuration
import android.graphics.Rect
import android.os.Build
import android.os.Handler
import android.os.Looper
import android.os.SystemClock
import android.provider.Settings
import android.text.InputType
import android.util.Log
import android.view.accessibility.AccessibilityEvent
import android.view.accessibility.AccessibilityNodeInfo
import android.view.accessibility.AccessibilityWindowInfo
import android.view.inputmethod.EditorInfo
import anomaly.hex.android.HexApp
import anomaly.hex.android.core.field.Eligibility
import anomaly.hex.android.core.field.FieldEligibility
import anomaly.hex.android.core.field.FieldFacts
import anomaly.hex.android.core.field.HideReason
import anomaly.hex.android.core.overlay.StopReason

/**
 * Sees which field has focus, shows the bubble for ordinary text fields, and
 * inserts finished text. It reads no field text except to place text in the
 * fallback path, and it keeps nothing from the screen.
 */
class HexAccessibilityService : AccessibilityService() {
    private val main = Handler(Looper.getMainLooper())
    private lateinit var overlay: OverlayController
    private lateinit var coordinator: DictationCoordinator
    private lateinit var inserter: TextInserter
    private var inputGeneration = 0
    private var selectionChanges = 0
    private var inputChangedAt = 0L
    private val refresh = Runnable { refreshFocus() }
    private val lateRefresh = Runnable { refreshFocus() }

    private val screenReceiver = object : BroadcastReceiver() {
        override fun onReceive(context: Context, intent: Intent) {
            if (intent.action == Intent.ACTION_SCREEN_OFF) coordinator.onExternalStop(StopReason.ScreenOff)
            scheduleRefresh()
        }
    }

    override fun onServiceConnected() {
        super.onServiceConnected()
        Log.i(TAG, "connected")
        overlay = OverlayController(this)
        coordinator = DictationCoordinator(this, this) { state -> overlay.render(state) }
        overlay.attach(coordinator)
        inserter = TextInserter(this, { inputMethod }, { selectionChanges })
        val filter = IntentFilter().apply {
            addAction(Intent.ACTION_SCREEN_OFF)
            addAction(Intent.ACTION_SCREEN_ON)
            addAction(Intent.ACTION_USER_PRESENT)
        }
        registerReceiver(screenReceiver, filter, Context.RECEIVER_NOT_EXPORTED)
        scheduleRefresh()
    }

    override fun onCreateInputMethod(): InputMethod = object : InputMethod(this) {
        override fun onStartInput(attribute: EditorInfo, restarting: Boolean) {
            super.onStartInput(attribute, restarting)
            if (!restarting) inputGeneration += 1
            refreshAfterInputChange()
        }

        override fun onFinishInput() {
            super.onFinishInput()
            refreshAfterInputChange()
        }

        override fun onUpdateSelection(oldSelStart: Int, oldSelEnd: Int, newSelStart: Int, newSelEnd: Int, candidatesStart: Int, candidatesEnd: Int) {
            super.onUpdateSelection(oldSelStart, oldSelEnd, newSelStart, newSelEnd, candidatesStart, candidatesEnd)
            selectionChanges += 1
        }
    }

    override fun onAccessibilityEvent(event: AccessibilityEvent) = scheduleRefresh()

    override fun onInterrupt() = Unit

    /**
     * A new editor can start before the app's accessibility tree says which field took focus,
     * and the app may send no event when it catches up, so look again a few times.
     */
    private fun refreshAfterInputChange() {
        inputChangedAt = SystemClock.uptimeMillis()
        scheduleRefresh()
        main.removeCallbacks(lateRefresh)
        for (delay in LATE_REFRESH_MS) main.postDelayed(lateRefresh, delay)
    }

    private fun scheduleRefresh() {
        main.removeCallbacks(refresh)
        main.postDelayed(refresh, REFRESH_DELAY_MS)
    }

    override fun onConfigurationChanged(newConfig: Configuration) {
        super.onConfigurationChanged(newConfig)
        if (::overlay.isInitialized) overlay.onConfigurationChanged()
        scheduleRefresh()
    }

    private fun refreshFocus() {
        if (!::coordinator.isInitialized) return
        val target = snapshot()
        val eligibility = target.eligibility
        val sensitive = eligibility is Eligibility.Hide && eligibility.reason in SENSITIVE
        coordinator.onEligibility(eligibility is Eligibility.Show, sensitive)
        overlay.setKeyboardTop(keyboardTop())
        overlay.setSearchField((eligibility as? Eligibility.Show)?.search == true)
        target.packageName?.let { HexFocus.note(it) }
    }

    private fun keyboardTop(): Int? = runCatching {
        windows.firstOrNull { it.type == AccessibilityWindowInfo.TYPE_INPUT_METHOD }?.let { window ->
            val bounds = Rect()
            window.getBoundsInScreen(bounds)
            bounds.top.takeIf { !bounds.isEmpty }
        }
    }.getOrNull()

    private fun snapshot(): TextTarget {
        val settings = HexApp.get(this).settings.current
        val method = inputMethod
        val started = method?.currentInputStarted == true
        val editor = if (started) method?.currentInputEditorInfo else null
        val root = runCatching { rootInActiveWindow }.getOrNull()
        // From the window's root: the service-wide search stops at Chrome's outer view, which hides
        // the web field's own password flag, id, and placeholder.
        val node: AccessibilityNodeInfo? = runCatching { root?.findFocus(AccessibilityNodeInfo.FOCUS_INPUT) ?: findFocus(AccessibilityNodeInfo.FOCUS_INPUT) }.getOrNull()
        val active = root?.packageName?.toString()
        val packageName = node?.packageName?.toString() ?: editor?.packageName
        val keyboard = runCatching { Settings.Secure.getString(contentResolver, Settings.Secure.DEFAULT_INPUT_METHOD) }.getOrNull()
            ?.substringBefore('/')
        val overShade = active != null && active != packageName && active == SYSTEM_UI
        val facts = FieldFacts(
            packageName = packageName,
            nodeEditable = node?.isEditable == true,
            nodeEnabled = node?.isEnabled != false,
            nodePassword = node?.isPassword == true,
            nodeDataSensitive = node != null && Build.VERSION.SDK_INT >= 34 && node.isAccessibilityDataSensitive,
            nodeInputType = node?.inputType ?: 0,
            editorInputType = editor?.inputType,
            editorActive = started && editor != null,
            editorSettled = SystemClock.uptimeMillis() - inputChangedAt >= EDITOR_ALONE_AFTER_MS,
            isSearchField = editor != null && (editor.imeOptions and EditorInfo.IME_MASK_ACTION) == EditorInfo.IME_ACTION_SEARCH,
            keyguardLocked = getSystemService(KeyguardManager::class.java).isKeyguardLocked,
            excludedByUser = packageName in settings.excludedPackages || packageName == keyboard || packageName == SYSTEM_UI || overShade,
            viewId = node?.viewIdResourceName,
            hint = node?.hintText?.toString(),
        )
        return TextTarget(
            packageName = packageName,
            node = node,
            inputGeneration = inputGeneration,
            eligibility = FieldEligibility.evaluate(facts),
            noPersonalizedLearning = editor != null && (editor.imeOptions and EditorInfo.IME_FLAG_NO_PERSONALIZED_LEARNING) != 0,
        ).also { Log.d(TAG, "eligibility ${it.eligibility} type=${editor?.inputType?.let { t -> t and InputType.TYPE_MASK_CLASS }}") }
    }

    // ---- for the coordinator ----

    fun currentTarget(): TextTarget = snapshot()

    fun appLabel(packageName: String?): String {
        if (packageName == null) return "unknown"
        return runCatching {
            packageManager.getApplicationLabel(packageManager.getApplicationInfo(packageName, 0)).toString()
        }.getOrDefault(packageName)
    }

    fun insert(text: String, target: TextTarget?, done: (InsertOutcome) -> Unit) {
        val current = snapshot()
        // Re-checked at the last moment: the field must still be the same and still be safe.
        if (current.eligibility !is Eligibility.Show) return done(InsertOutcome.FocusMoved)
        if (target != null && !target.sameField(current)) return done(InsertOutcome.FocusMoved)
        inserter.insert(text, current, HexApp.get(this).settings.current.smartSpacing, done)
    }

    override fun onUnbind(intent: Intent?): Boolean {
        if (::coordinator.isInitialized) coordinator.onServiceLost()
        return super.onUnbind(intent)
    }

    override fun onDestroy() {
        main.removeCallbacks(refresh)
        main.removeCallbacks(lateRefresh)
        if (::overlay.isInitialized) {
            overlay.destroy()
            runCatching { unregisterReceiver(screenReceiver) }
        }
        super.onDestroy()
    }

    companion object {
        private const val TAG = "HexA11y"
        private const val REFRESH_DELAY_MS = 40L
        private val LATE_REFRESH_MS = longArrayOf(120, 350, 800, 1500)
        private const val EDITOR_ALONE_AFTER_MS = 1400L
        private const val SYSTEM_UI = "com.android.systemui"
        private val SENSITIVE = setOf(HideReason.Password, HideReason.DataSensitive, HideReason.PaymentField, HideReason.Locked)
    }
}

/** Recently focused apps, in memory only, to offer in Excluded apps. Never persisted or synced. */
object HexFocus {
    private val recent = LinkedHashSet<String>()

    @Synchronized
    fun note(packageName: String) {
        recent.remove(packageName)
        recent.add(packageName)
        if (recent.size > 12) recent.remove(recent.first())
    }

    @Synchronized
    fun recent(): List<String> = recent.toList().reversed()
}
