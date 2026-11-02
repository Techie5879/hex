package anomaly.hex.android.data

import android.content.Context
import android.util.Log
import anomaly.hex.android.core.profile.CleanupPolicy
import anomaly.hex.android.core.profile.PortableProfile
import anomaly.hex.android.core.profile.ProfileEditor
import anomaly.hex.android.core.profile.ProfileException
import anomaly.hex.android.core.profile.StampClock
import anomaly.hex.android.core.sync.ProfileStore
import java.io.File
import java.util.UUID
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.serialization.json.JsonElement

/** The local copy of the portable profile. Sync merges into this; offline edits land here. */
class ProfileRepository(context: Context) : ProfileStore {
    private val directory = File(context.noBackupFilesDir, "profile")
    private val localFile = File(directory, "profile.json")
    private val ancestorFile = File(directory, "ancestor.json")
    private val deviceId: String = context.getSharedPreferences("hex-device", Context.MODE_PRIVATE).let { prefs ->
        prefs.getString("deviceId", null) ?: UUID.randomUUID().toString().replace("-", "").take(12).also {
            prefs.edit().putString("deviceId", it).apply()
        }
    }
    private val clock = StampClock(deviceId)
    private val editor = ProfileEditor(clock)
    private val state = MutableStateFlow(load(localFile))
    val flow: StateFlow<PortableProfile> = state.asStateFlow()
    val current get() = state.value
    val policy get() = CleanupPolicy.from(current)

    /** Called after each local edit. Set once at startup. */
    var onEdited: () -> Unit = {}

    private fun load(file: File): PortableProfile {
        if (!file.isFile) return PortableProfile.empty()
        return try {
            PortableProfile.decode(file.readText())
        } catch (error: ProfileException) {
            // Keep the unreadable original beside it rather than losing it on the next save.
            Log.e(TAG, "Profile unreadable: ${error.kind}")
            file.copyTo(File(file.parentFile, "${file.name}.unreadable-${System.currentTimeMillis()}"), overwrite = true)
            PortableProfile.empty()
        }
    }

    @Synchronized
    private fun edit(change: (ProfileEditor, PortableProfile) -> PortableProfile) {
        val next = change(editor, state.value)
        if (next == state.value) return
        saveLocal(next)
        onEdited()
    }

    fun putTerm(term: String, soundsLike: List<String> = emptyList()) = edit { e, p -> e.putTerm(p, term, soundsLike) }
    fun removeTerm(term: String) = edit { e, p -> e.removeTerm(p, term) }
    fun putSetting(key: String, value: JsonElement) = edit { e, p -> e.putSetting(p, key, value) }
    fun resolve(conflictId: String, useConflictValue: Boolean) = edit { e, p -> e.resolve(p, conflictId, useConflictValue) }

    @Synchronized
    override fun updateLocal(change: (PortableProfile) -> PortableProfile): PortableProfile {
        val next = change(state.value)
        if (next != state.value) saveLocal(next)
        return next
    }

    private fun saveLocal(profile: PortableProfile) {
        AtomicFiles.write(localFile, profile.encode().toByteArray())
        state.value = profile
    }

    override fun ancestor(): PortableProfile = load(ancestorFile)

    override fun saveAncestor(profile: PortableProfile) {
        AtomicFiles.write(ancestorFile, profile.encode().toByteArray())
    }

    private companion object {
        const val TAG = "HexProfile"
    }
}
