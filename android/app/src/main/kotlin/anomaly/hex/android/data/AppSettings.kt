package anomaly.hex.android.data

import android.content.Context
import android.content.SharedPreferences
import anomaly.hex.android.core.overlay.BubblePosition
import anomaly.hex.android.core.overlay.Edge
import anomaly.hex.android.core.retention.Retention
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.serialization.Serializable
import kotlinx.serialization.encodeToString
import kotlinx.serialization.json.Json

/** Settings for this phone alone. Portable ones live in the profile. */
@Serializable
data class DeviceSettings(
    val setupDone: Boolean = false,
    val disclosureAccepted: Boolean = false,
    val bubbleSizeDp: Int = DEFAULT_SIZE_DP,
    val bubbleOpacity: Int = DEFAULT_OPACITY,
    val shrinkWhenIdle: Boolean = false,
    val shrinkToDot: Boolean = false,
    val shrinkInSearch: Boolean = false,
    val edge: Edge = BubblePosition.DEFAULT.edge,
    val yFraction: Float = BubblePosition.DEFAULT.yFraction,
    val audioRetention: Retention = Retention.Forever,
    val historyRetention: Retention = Retention.Forever,
    val smartSpacing: Boolean = true,
    val speechModel: String? = null,
    val cleanupModel: String? = null,
    val language: String = "en",
    val threads: Int = 4,
    val unloadIdleMinutes: Int = 5,
    val localCleanupDeadlineSeconds: Int = 8,
    val excludedPackages: Set<String> = emptySet(),
    val syncUrl: String = "",
    val syncEnabled: Boolean = false,
    val lastSyncMs: Long = 0,
    val lastSyncError: String? = null,
    val asrDeadlineFactor: Int = 8,
) {
    companion object {
        const val DEFAULT_SIZE_DP = 44
        const val DEFAULT_OPACITY = 90
    }
}

class AppSettings(context: Context) {
    private val prefs: SharedPreferences = context.getSharedPreferences("hex-device", Context.MODE_PRIVATE)
    private val state = MutableStateFlow(read())
    val flow: StateFlow<DeviceSettings> = state.asStateFlow()
    val current get() = state.value

    private fun read(): DeviceSettings =
        prefs.getString(KEY, null)?.let { saved -> runCatching { JSON.decodeFromString<DeviceSettings>(saved) }.getOrNull() } ?: DeviceSettings()

    fun update(change: (DeviceSettings) -> DeviceSettings) {
        synchronized(this) {
            val next = change(state.value)
            prefs.edit().putString(KEY, JSON.encodeToString(next)).apply()
            state.value = next
        }
    }

    private companion object {
        const val KEY = "settings"
        val JSON = Json { ignoreUnknownKeys = true }
    }
}
