package anomaly.hex.android.core

import anomaly.hex.android.core.field.Eligibility
import anomaly.hex.android.core.field.FieldEligibility
import anomaly.hex.android.core.field.FieldFacts
import anomaly.hex.android.core.field.HideReason
import anomaly.hex.android.core.field.InputTypeBits as B
import anomaly.hex.android.core.insert.Insertion
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class FieldAndInsertionTest {
    private val text = FieldFacts("app", nodeEditable = true, editorActive = true, editorInputType = B.CLASS_TEXT)
    private fun hidden(f: FieldFacts, why: HideReason) = assertEquals(Eligibility.Hide(why), FieldEligibility.evaluate(f))

    @Test
    fun plainEditorsShow() {
        assertEquals(Eligibility.Show(multiline = false, search = false), FieldEligibility.evaluate(text))
        assertEquals(
            Eligibility.Show(multiline = true, search = false),
            FieldEligibility.evaluate(text.copy(editorInputType = B.CLASS_TEXT or B.TEXT_MULTI_LINE)),
        )
        // Chrome's contenteditable reports a text class, like any other web field.
        assertTrue(FieldEligibility.evaluate(text.copy(editorInputType = B.CLASS_TEXT or 0xa0)) is Eligibility.Show)
    }

    @Test
    fun everyPasswordSignalHides() {
        hidden(text.copy(nodePassword = true), HideReason.Password)
        hidden(text.copy(editorInputType = B.CLASS_TEXT or B.TEXT_PASSWORD), HideReason.Password)
        hidden(text.copy(editorInputType = B.CLASS_TEXT or B.TEXT_VISIBLE_PASSWORD), HideReason.Password)
        hidden(text.copy(editorInputType = B.CLASS_TEXT or B.TEXT_WEB_PASSWORD), HideReason.Password)
        hidden(text.copy(editorInputType = B.CLASS_NUMBER or B.NUMBER_PASSWORD), HideReason.Password)
        // The node and the editor may disagree: either one saying password wins.
        hidden(text.copy(nodeInputType = B.CLASS_TEXT or B.TEXT_WEB_PASSWORD), HideReason.Password)
        hidden(text.copy(nodeDataSensitive = true), HideReason.DataSensitive)
    }

    @Test
    fun nonTextAndLockedAndExcludedHide() {
        hidden(text.copy(editorInputType = B.CLASS_NUMBER), HideReason.NonText)
        hidden(text.copy(editorInputType = B.CLASS_PHONE), HideReason.NonText)
        hidden(text.copy(editorInputType = B.CLASS_DATETIME), HideReason.NonText)
        hidden(text.copy(keyguardLocked = true), HideReason.Locked)
        hidden(text.copy(excludedByUser = true), HideReason.ExcludedApp)
        hidden(text.copy(nodeEnabled = false), HideReason.Disabled)
        hidden(text.copy(packageName = null), HideReason.NoPackage)
    }

    @Test
    fun anyFieldHexWouldRefuseEndsARecordingAndLeavingAFieldDoesNot() {
        for (reason in listOf(HideReason.Password, HideReason.DataSensitive, HideReason.NonText, HideReason.PaymentField, HideReason.ExcludedApp, HideReason.Locked)) {
            assertTrue(reason.name, reason.endsCapture(editorSettled = false))
        }
        for (reason in listOf(HideReason.NoField, HideReason.NotEditable, HideReason.Disabled, HideReason.SystemSurface, HideReason.NoPackage)) {
            assertTrue(reason.name, !reason.endsCapture(editorSettled = true))
        }
        assertTrue(!HideReason.UnknownType.endsCapture(editorSettled = false))
        assertTrue(HideReason.UnknownType.endsCapture(editorSettled = true))
    }

    @Test
    fun aNewEditorWaitsForTheAccessibilityTreeUnlessTheFieldIsAlreadyKnown() {
        // Chrome connects its editor before its tree says which field took focus. That field's
        // id and placeholder may yet hide the button, so the editor alone can't show it yet.
        val unsettled = FieldFacts("app", nodeEditable = false, editorActive = true, editorSettled = false, editorInputType = B.CLASS_TEXT)
        hidden(unsettled, HideReason.NoField)
        assertTrue(FieldEligibility.evaluate(unsettled.copy(editorSettled = true)) is Eligibility.Show)
        assertTrue(FieldEligibility.evaluate(unsettled.copy(nodeEditable = true)) is Eligibility.Show)
    }

    @Test
    fun anEditableFieldWithNoKnownTypeHides() {
        hidden(text.copy(editorActive = false, editorInputType = null, nodeInputType = 0), HideReason.UnknownType)
        hidden(text.copy(editorInputType = 0, nodeInputType = 0), HideReason.UnknownType)
        // The accessibility node alone is enough when it names a text class.
        assertTrue(FieldEligibility.evaluate(text.copy(editorActive = false, editorInputType = null, nodeInputType = B.CLASS_TEXT)) is Eligibility.Show)
    }

    @Test
    fun failsClosedWithoutAnEditor() {
        hidden(FieldFacts("app", nodeEditable = false, editorActive = false), HideReason.NoField)
        // A page with no editable focus: Chrome's connection reports TYPE_NULL.
        hidden(FieldFacts("app", nodeEditable = false, editorActive = true, editorInputType = 0), HideReason.NoField)
        // An editor the accessibility tree hasn't caught up with still counts by its input type.
        assertTrue(FieldEligibility.evaluate(FieldFacts("app", nodeEditable = false, editorActive = true, editorInputType = B.CLASS_TEXT)) is Eligibility.Show)
    }

    @Test
    fun paymentIdentifiersHideButOrdinaryWordsDont() {
        for (id in listOf("cc-number", "card_number", "com.bank:id/cvv", "checkout-cvc", "cardNumber".lowercase())) {
            hidden(text.copy(viewId = id), HideReason.PaymentField)
        }
        hidden(text.copy(hint = "Credit card"), HideReason.PaymentField)
        for (id in listOf("cardigan", "notes", "scorecard", "message", "comment-body")) {
            assertTrue(id, FieldEligibility.evaluate(text.copy(viewId = id)) is Eligibility.Show)
        }
    }

    private fun ok(r: Insertion.Result) = (r as Insertion.Result.Ok).plan

    @Test
    fun replacesSelectionAndPlacesCaret() {
        val p = ok(Insertion.plan("hello cruel world", 6, 11, "kind", true, smartSpacing = false))
        assertEquals("hello kind world", p.text)
        assertEquals(10, p.cursor)
    }

    @Test
    fun reversedAndUnknownSelections() {
        assertEquals("hello kind world", ok(Insertion.plan("hello cruel world", 11, 6, "kind", true, smartSpacing = false)).text)
        assertEquals("start end", ok(Insertion.plan("start", -1, -1, "end", true)).text)
        assertEquals("abcd", ok(Insertion.plan("abc", 99, 99, "d", true, smartSpacing = false)).text)
    }

    @Test
    fun hintTextIsNotContent() {
        val p = ok(Insertion.plan("Dictate here", 0, 0, "Real", true, showingHint = true))
        assertEquals("Real", p.text)
        assertEquals(4, p.cursor)
    }

    @Test
    fun neverSplitsASurrogatePair() {
        val existing = "a😀b"
        // An offset landing inside the emoji moves to its start.
        val p = ok(Insertion.plan(existing, 2, 2, "x", true, smartSpacing = false))
        assertEquals("ax😀b", p.text)
        assertEquals("a😀b→", ok(Insertion.plan(existing, 4, 4, "→", true, smartSpacing = false)).text)
        assertEquals("क्षx", ok(Insertion.plan("क्ष", 3, 3, "x", true, smartSpacing = false)).text)
    }

    @Test
    fun singleLineFlattensLineBreaksAndMultilineKeepsThem() {
        assertEquals("one two three", ok(Insertion.plan("", 0, 0, "one\ntwo \r\n three", false)).text)
        assertEquals("one\ntwo", ok(Insertion.plan("", 0, 0, "one\ntwo", true)).text)
    }

    @Test
    fun tooLongIsRefusedNotTruncated() {
        assertEquals(Insertion.Result.TooLong, Insertion.plan("12345", 5, 5, "678", true, maxLength = 7, smartSpacing = false))
        assertTrue(Insertion.plan("12345", 0, 5, "678", true, maxLength = 7, smartSpacing = false) is Insertion.Result.Ok)
        assertEquals(Insertion.Result.Empty, Insertion.plan("x", 0, 0, "  \n ", true))
    }

    @Test
    fun smartSpacing() {
        assertEquals("Hello world", ok(Insertion.plan("Hello", 5, 5, "world", true)).text)
        assertEquals("Hello, world", ok(Insertion.plan("Hello,", 6, 6, "world", true)).text)
        assertEquals("Hello.", ok(Insertion.plan("Hello", 5, 5, ".", true)).text)
        assertEquals("(word", ok(Insertion.plan("(", 1, 1, "word", true)).text)
        assertEquals("word next", ok(Insertion.plan("next", 0, 0, "word", true)).text)
        assertEquals("word.", ok(Insertion.plan(".", 0, 0, "word", true)).text)
        assertEquals("line\nword", ok(Insertion.plan("line\n", 5, 5, "word", true)).text)
    }
}
