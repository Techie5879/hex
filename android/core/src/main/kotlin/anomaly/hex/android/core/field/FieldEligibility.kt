package anomaly.hex.android.core.field

/** Android `InputType` values, copied so this logic stays testable off-device. */
object InputTypeBits {
    const val MASK_CLASS = 0x0000000f
    const val MASK_VARIATION = 0x00000ff0
    const val CLASS_TEXT = 1
    const val CLASS_NUMBER = 2
    const val CLASS_PHONE = 3
    const val CLASS_DATETIME = 4
    const val TEXT_MULTI_LINE = 0x00020000
    const val TEXT_PASSWORD = 0x80
    const val TEXT_VISIBLE_PASSWORD = 0x90
    const val TEXT_WEB_PASSWORD = 0xe0
    const val NUMBER_PASSWORD = 0x10
}

/** Everything the system tells us about the focused field. Not its text. */
data class FieldFacts(
    val packageName: String?,
    val nodeEditable: Boolean,
    val nodeEnabled: Boolean = true,
    val nodePassword: Boolean = false,
    val nodeDataSensitive: Boolean = false,
    val nodeInputType: Int = 0,
    val editorInputType: Int? = null,
    val editorActive: Boolean = false,
    /** The app's accessibility tree has had time to name the field, so the editor alone can count. */
    val editorSettled: Boolean = true,
    val isSearchField: Boolean = false,
    val keyguardLocked: Boolean = false,
    val excludedByUser: Boolean = false,
    /** The keyboard's own field, the notification shade, and similar: never a dictation target. */
    val systemSurface: Boolean = false,
    /** The field's resource or HTML id, and its hint. Used only as an extra signal to hide. */
    val viewId: String? = null,
    val hint: String? = null,
)

enum class HideReason {
    NoField, NotEditable, Disabled, Password, DataSensitive, NonText, UnknownType, PaymentField, ExcludedApp, SystemSurface, Locked, NoPackage;

    /**
     * Whether focus landing here must end a recording. Leaving a text field for nothing, or
     * pulling down the shade, needn't. A field Hex wouldn't dictate into must, or speech meant for
     * it could be captured with the button gone. A field whose type hasn't arrived yet may simply
     * be a moment behind, so it counts once the editor has settled.
     */
    fun endsCapture(editorSettled: Boolean) = when (this) {
        NoField, NotEditable, Disabled, SystemSurface, NoPackage -> false
        UnknownType -> editorSettled
        Password, DataSensitive, NonText, PaymentField, ExcludedApp, Locked -> true
    }
}

sealed interface Eligibility {
    data class Show(val multiline: Boolean, val search: Boolean) : Eligibility
    data class Hide(val reason: HideReason) : Eligibility
}

/**
 * Decides whether the bubble may show for the focused field, failing closed.
 * The system's password, data-sensitive, and input-type signals on both the
 * accessibility node and the editor decide, and one of them must positively say
 * the field takes text. Android doesn't expose HTML autocomplete to accessibility
 * services, and payment forms often use plain text fields, so identifiers are a
 * further reason to hide, never a reason to show. This can't catch every payment
 * field: docs/architecture.md states the limit, and a person can exclude any app.
 */
object FieldEligibility {
    fun evaluate(facts: FieldFacts): Eligibility {
        fun hide(reason: HideReason) = Eligibility.Hide(reason)
        if (facts.keyguardLocked) return hide(HideReason.Locked)
        if (facts.packageName.isNullOrEmpty()) return hide(HideReason.NoPackage)
        if (facts.excludedByUser) return hide(HideReason.ExcludedApp)
        if (facts.systemSurface) return hide(HideReason.SystemSurface)
        // Chrome keeps an input connection with no input type while nothing editable has focus.
        val editorIsField = facts.editorActive && facts.editorSettled && (facts.editorInputType ?: 0) != 0
        if (!facts.nodeEditable && !editorIsField) return hide(HideReason.NoField)
        if (facts.nodePassword) return hide(HideReason.Password)
        if (facts.nodeDataSensitive) return hide(HideReason.DataSensitive)
        if (!facts.nodeEnabled) return hide(HideReason.Disabled)
        val types = listOfNotNull(facts.editorInputType, facts.nodeInputType.takeIf { it != 0 })
        for (type in types) {
            if (isPasswordType(type)) return hide(HideReason.Password)
            when (type and InputTypeBits.MASK_CLASS) {
                InputTypeBits.CLASS_NUMBER, InputTypeBits.CLASS_PHONE, InputTypeBits.CLASS_DATETIME ->
                    return hide(HideReason.NonText)
            }
        }
        // A field's safety has to be established: with no input type from either source, it can't be.
        if (types.none { it and InputTypeBits.MASK_CLASS == InputTypeBits.CLASS_TEXT }) return hide(HideReason.UnknownType)
        if (looksLikePaymentField(facts.viewId) || looksLikePaymentField(facts.hint)) return hide(HideReason.PaymentField)
        val multiline = types.any { it and InputTypeBits.TEXT_MULTI_LINE != 0 }
        return Eligibility.Show(multiline, facts.isSearchField)
    }

    fun isPasswordType(type: Int): Boolean {
        val variation = type and InputTypeBits.MASK_VARIATION
        return when (type and InputTypeBits.MASK_CLASS) {
            InputTypeBits.CLASS_TEXT -> variation == InputTypeBits.TEXT_PASSWORD ||
                variation == InputTypeBits.TEXT_VISIBLE_PASSWORD ||
                variation == InputTypeBits.TEXT_WEB_PASSWORD
            InputTypeBits.CLASS_NUMBER -> variation == InputTypeBits.NUMBER_PASSWORD
            else -> false
        }
    }

    private val PAYMENT_TOKENS = setOf(
        "cvv", "cvc", "csc", "cvn", "ccnum", "ccnumber", "cccsc", "ccexp", "cardnumber", "cardnum", "cardcvc",
        "cardcvv", "creditcard", "debitcard", "securitycode", "socialsecurity", "ssn", "iban", "routingnumber",
        "accountnumber",
    )
    private val SEPARATORS = Regex("[^a-z0-9]+")

    /** Whole tokens and adjacent pairs only: "cardigan" and "scorecard" stay visible. */
    private fun looksLikePaymentField(text: String?): Boolean {
        if (text.isNullOrBlank()) return false
        val tokens = text.lowercase().split(SEPARATORS).filter(String::isNotEmpty)
        return tokens.any { it in PAYMENT_TOKENS } ||
            tokens.zipWithNext { a, b -> a + b }.any { it in PAYMENT_TOKENS }
    }
}
