package anomaly.hex.android.service

import android.view.accessibility.AccessibilityNodeInfo
import anomaly.hex.android.core.field.Eligibility

/** The field a dictation is for. It holds an identity and facts, not the field's text. */
data class TextTarget(
    val packageName: String?,
    val node: AccessibilityNodeInfo?,
    val inputGeneration: Int,
    val eligibility: Eligibility,
    val noPersonalizedLearning: Boolean,
) {
    /** Whether [other] is the same field: same node when there is one, else the same editor session. */
    fun sameField(other: TextTarget): Boolean {
        if (packageName != other.packageName) return false
        return if (node != null && other.node != null) node == other.node else inputGeneration == other.inputGeneration
    }
}
