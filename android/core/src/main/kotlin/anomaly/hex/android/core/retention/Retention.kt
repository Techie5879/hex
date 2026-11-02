package anomaly.hex.android.core.retention

import kotlinx.serialization.Serializable

/** Audio and history use this separately. Off stops new saves and keeps what's there. */
@Serializable
enum class Retention(val wire: String, val maxAgeMs: Long?, val keepsNew: Boolean) {
    Off("off", null, false),
    Day("day", 86_400_000L, true),
    Week("week", 7 * 86_400_000L, true),
    Month("month", 30 * 86_400_000L, true),
    Forever("forever", null, true);

    fun isExpired(createdMs: Long, nowMs: Long): Boolean {
        val age = maxAgeMs ?: return false
        return nowMs - createdMs >= age
    }

    companion object {
        fun fromWire(value: String?, fallback: Retention = Forever) =
            entries.firstOrNull { it.wire == value } ?: fallback
    }
}
