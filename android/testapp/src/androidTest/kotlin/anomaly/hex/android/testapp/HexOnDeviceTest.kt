package anomaly.hex.android.testapp

import android.app.Instrumentation
import android.app.UiAutomation
import android.os.SystemClock
import android.view.View
import android.widget.EditText
import androidx.test.core.app.ActivityScenario
import androidx.test.platform.app.InstrumentationRegistry
import org.junit.Assert.assertEquals
import org.junit.Before
import org.junit.FixMethodOrder
import org.junit.Test
import org.junit.runners.MethodSorters

/**
 * Hex (installed separately) against real editors, on a real system image. The
 * UiAutomation flag keeps Hex's accessibility service connected while this test runs.
 * Plain instrumentation suppresses other services, which would look like Hex is broken.
 */
@FixMethodOrder(MethodSorters.NAME_ASCENDING)
class HexOnDeviceTest {
    private val instrumentation: Instrumentation = InstrumentationRegistry.getInstrumentation()
    private val automation: UiAutomation = instrumentation.getUiAutomation(UiAutomation.FLAG_DONT_SUPPRESS_ACCESSIBILITY_SERVICES)

    private fun shell(command: String): String {
        val pipe = automation.executeShellCommand(command)
        return android.os.ParcelFileDescriptor.AutoCloseInputStream(pipe).bufferedReader().use { it.readText() }
    }

    @Before
    fun connectService() {
        shell("settings put secure enabled_accessibility_services $SERVICE")
        shell("settings put secure accessibility_enabled 1")
        shell("pm grant anomaly.hex.android android.permission.RECORD_AUDIO")
        shell("input keyevent KEYCODE_WAKEUP")
        shell("wm dismiss-keyguard")
        waitUntil("the service to connect") { "anomaly.hex.android" in shell("dumpsys accessibility") && "Bound services" in shell("dumpsys accessibility") }
    }

    private fun bubbleShown(): Boolean = "HexBubble" in shell("dumpsys window windows")

    private fun waitUntil(what: String, timeoutMs: Long = 60_000, condition: () -> Boolean) {
        val deadline = SystemClock.uptimeMillis() + timeoutMs
        while (SystemClock.uptimeMillis() < deadline) {
            if (condition()) return
            SystemClock.sleep(500)
        }
        throw AssertionError("Timed out waiting for $what")
    }

    private fun focus(activity: ActivityScenario<FieldsActivity>, pick: (FieldsActivity) -> View) {
        activity.onActivity { pick(it).requestFocus() }
    }

    private fun expectBubble(activity: ActivityScenario<FieldsActivity>, name: String, shown: Boolean, pick: (FieldsActivity) -> View) {
        focus(activity, pick)
        if (shown) {
            waitUntil("the button on $name") { bubbleShown() }
        } else {
            waitUntil("the button to leave $name") { !bubbleShown() }
        }
    }

    @Test
    fun a_buttonFollowsEligibleFieldsAndNeverShowsOnSensitiveOnes() {
        ActivityScenario.launch(FieldsActivity::class.java).use { activity ->
            expectBubble(activity, "a multiline field", true) { it.multi }
            expectBubble(activity, "a password field", false) { it.password }
            expectBubble(activity, "a single-line field", true) { it.single }
            expectBubble(activity, "a visible password", false) { it.visiblePassword }
            expectBubble(activity, "the multiline field again", true) { it.multi }
            expectBubble(activity, "a PIN", false) { it.pin }
            expectBubble(activity, "the single-line field again", true) { it.single }
            expectBubble(activity, "a number field", false) { it.number }
            expectBubble(activity, "the multiline field once more", true) { it.multi }
            expectBubble(activity, "a card-number id", false) { it.cardNumber }
        }
    }

    private fun insert(text: String) {
        // executeShellCommand has no shell to handle quoting, so text travels encoded.
        val encoded = android.util.Base64.encodeToString(text.toByteArray(), android.util.Base64.NO_WRAP)
        shell("am broadcast -a anomaly.hex.android.debug.INSERT -n anomaly.hex.android/.debug.DebugReceiver --es textBase64 $encoded")
    }

    private fun EditText.valueOnMain(activity: ActivityScenario<FieldsActivity>): String {
        var value = ""
        activity.onActivity { value = text.toString() }
        return value
    }

    @Test
    fun b_insertionReplacesTheSelectionInNativeFields() {
        ActivityScenario.launch(FieldsActivity::class.java).use { activity ->
            lateinit var field: EditText
            activity.onActivity {
                field = it.multi
                it.multi.requestFocus()
                it.multi.setText("hello cruel world")
                it.multi.setSelection(6, 11)
            }
            waitUntil("the button") { bubbleShown() }
            insert("kind")
            waitUntil("the insertion") { field.valueOnMain(activity) == "hello kind world" }
            activity.onActivity { assertEquals(10, it.multi.selectionStart) }
        }
    }

    @Test
    fun c_insertionFlattensLinesInSingleLineFieldsAndHandlesUnicode() {
        ActivityScenario.launch(FieldsActivity::class.java).use { activity ->
            lateinit var field: EditText
            activity.onActivity {
                field = it.single
                it.single.requestFocus()
                it.single.setText("emoji 😀")
                it.single.setSelection(it.single.text.length)
            }
            waitUntil("the button") { bubbleShown() }
            insert("next 日本語")
            waitUntil("the insertion") { field.valueOnMain(activity) == "emoji 😀 next 日本語" }
        }
    }

    @Test
    fun d_composeFieldsTakeText() {
        ActivityScenario.launch(FieldsActivity::class.java).use { activity ->
            val metrics = instrumentation.targetContext.resources.displayMetrics
            lateinit var compose: View
            activity.onActivity { compose = (it.multi.parent as android.view.ViewGroup).getChildAt(8) }
            val location = IntArray(2)
            activity.onActivity {
                compose.getLocationOnScreen(location)
            }
            shell("input tap ${metrics.widthPixels / 2} ${location[1] + compose.height / 2}")
            waitUntil("the button on a Compose field") { bubbleShown() }
            insert("from compose")
            var value = ""
            waitUntil("the insertion") {
                activity.onActivity { value = it.composeText }
                value == "from compose"
            }
        }
    }

    @Test
    fun e_lockedDevicesShowNoButton() {
        ActivityScenario.launch(FieldsActivity::class.java).use { activity ->
            focus(activity) { it.multi }
            waitUntil("the button") { bubbleShown() }
            shell("locksettings set-pin 1234")
            try {
                shell("input keyevent KEYCODE_SLEEP")
                SystemClock.sleep(3000)
                shell("input keyevent KEYCODE_WAKEUP")
                waitUntil("the button to leave on the lock screen") { !bubbleShown() }
            } finally {
                shell("locksettings clear --old 1234")
                shell("input keyevent KEYCODE_WAKEUP")
                shell("wm dismiss-keyguard")
            }
        }
    }

    private companion object {
        const val SERVICE = "anomaly.hex.android/anomaly.hex.android.service.HexAccessibilityService"
    }
}
