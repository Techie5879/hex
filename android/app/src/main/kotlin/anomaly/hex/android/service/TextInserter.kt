package anomaly.hex.android.service

import android.accessibilityservice.AccessibilityService
import android.accessibilityservice.InputMethod
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.util.Log
import android.view.accessibility.AccessibilityNodeInfo
import android.view.inputmethod.SurroundingText
import anomaly.hex.android.core.field.Eligibility
import anomaly.hex.android.core.insert.Insertion
import kotlin.concurrent.thread

/**
 * Puts finished text at the caret, one way only. With an input connection, it commits
 * like a keyboard: replacing the selection, finishing composing text, keeping rich
 * formatting and undo, in native, Compose, and Web fields. Without one, a plain field
 * gets ACTION_SET_TEXT. Whatever can't be confirmed goes back to the caller to offer.
 */
class TextInserter(
    private val service: AccessibilityService,
    private val inputMethod: () -> InputMethod?,
    private val selectionChanges: () -> Int,
) {
    private val main = Handler(Looper.getMainLooper())

    fun insert(text: String, target: TextTarget, smartSpacing: Boolean, done: (InsertOutcome) -> Unit) {
        val show = target.eligibility as? Eligibility.Show ?: return done(InsertOutcome.FocusMoved)
        val multiline = show.multiline || target.node?.isMultiLine == true
        val method = inputMethod()
        val connection = method?.currentInputConnection
        if (connection == null || !method.currentInputStarted) return setText(text, target, multiline, smartSpacing, done)
        // The connection's calls block on the target app, so they stay off the main thread.
        thread(name = "hex-insert") {
            val around = runCatching { connection.getSurroundingText(2, 2, 0) }.getOrNull()
            val body = Insertion.prepare(text, multiline, smartSpacing, around?.beforeSelection(), around?.afterSelection())
            if (body.isEmpty()) {
                main.post { done(InsertOutcome.Failed("empty")) }
                return@thread
            }
            val changesBefore = selectionChanges()
            connection.commitText(body, 1, null)
            val confirmed = confirm(connection, body, changesBefore)
            // An editor may change what it's given, or show it late. Inserting again could
            // duplicate the text, so an unconfirmed commit goes back to the person.
            main.post { done(if (confirmed) InsertOutcome.Inserted else InsertOutcome.Unconfirmed) }
        }
    }

    private fun confirm(connection: InputMethod.AccessibilityInputConnection, body: String, changesBefore: Int): Boolean {
        val tail = body.takeLast(VERIFY_CHARS)
        repeat(VERIFY_ATTEMPTS) {
            Thread.sleep(VERIFY_DELAY_MS)
            val around = runCatching { connection.getSurroundingText(VERIFY_CHARS + 2, 0, 0) }.getOrNull()
            if (around?.beforeSelection()?.endsWith(tail) == true) return true
            // A field that won't show its text still confirms by moving its caret.
            if (around == null && selectionChanges() != changesBefore) return true
        }
        return false
    }

    private fun SurroundingText.beforeSelection() = text.subSequence(0, selectionStart.coerceIn(0, text.length)).toString()
    private fun SurroundingText.afterSelection() = text.subSequence(selectionEnd.coerceIn(0, text.length), text.length).toString()

    private fun setText(text: String, target: TextTarget, multiline: Boolean, smartSpacing: Boolean, done: (InsertOutcome) -> Unit) {
        val node = target.node
        if (node == null || !node.refresh() || node.isPassword || !node.isEditable || !node.isFocused) return done(InsertOutcome.Failed("no field"))
        if (node.actionList.none { it.id == AccessibilityNodeInfo.ACTION_SET_TEXT }) return done(InsertOutcome.Failed("no set text"))
        // Replacing a rich editor's whole content would flatten its formatting.
        val role = node.extras.getCharSequence("AccessibilityNodeInfo.chromeRole")?.toString()
        if (role != null && role !in PLAIN_WEB_ROLES) return done(InsertOutcome.Failed("rich field"))

        val plan = Insertion.plan(
            existing = node.text?.toString().orEmpty(),
            selectionStart = node.textSelectionStart,
            selectionEnd = node.textSelectionEnd,
            dictated = text,
            multiline = multiline,
            showingHint = node.isShowingHintText,
            maxLength = node.maxTextLength.coerceAtLeast(0),
            smartSpacing = smartSpacing,
        )
        when (plan) {
            Insertion.Result.Empty -> done(InsertOutcome.Failed("empty"))
            Insertion.Result.TooLong -> done(InsertOutcome.Failed("too long"))
            is Insertion.Result.Ok -> {
                val args = Bundle().apply {
                    putCharSequence(AccessibilityNodeInfo.ACTION_ARGUMENT_SET_TEXT_CHARSEQUENCE, plan.plan.text)
                }
                if (!node.performAction(AccessibilityNodeInfo.ACTION_SET_TEXT, args)) return done(InsertOutcome.Failed("set text refused"))
                node.performAction(
                    AccessibilityNodeInfo.ACTION_SET_SELECTION,
                    Bundle().apply {
                        putInt(AccessibilityNodeInfo.ACTION_ARGUMENT_SELECTION_START_INT, plan.plan.cursor)
                        putInt(AccessibilityNodeInfo.ACTION_ARGUMENT_SELECTION_END_INT, plan.plan.cursor)
                    },
                )
                main.postDelayed({
                    val held = node.refresh() && node.text?.toString() == plan.plan.text
                    Log.i(TAG, "set text confirmed=$held")
                    done(if (held) InsertOutcome.Inserted else InsertOutcome.Unconfirmed)
                }, VERIFY_DELAY_MS)
            }
        }
    }

    private companion object {
        const val TAG = "HexInsert"
        const val VERIFY_ATTEMPTS = 5
        const val VERIFY_DELAY_MS = 120L
        const val VERIFY_CHARS = 16
        val PLAIN_WEB_ROLES = setOf("textField", "textFieldWithComboBox", "searchBox")
    }
}
