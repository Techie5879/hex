package anomaly.hex.android.sync

import android.os.Handler
import android.os.Looper
import android.util.Log
import anomaly.hex.android.core.profile.ProfileException
import anomaly.hex.android.core.sync.ProfileSync
import anomaly.hex.android.core.sync.SyncGaveUpException
import anomaly.hex.android.data.AppSettings
import anomaly.hex.android.data.ProfileRepository
import anomaly.hex.android.data.SecureStore
import java.io.IOException
import java.net.URI
import java.util.concurrent.Executors
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow

/** Runs sync off the main thread. One at a time, and only with sync turned on. */
class SyncRunner(
    private val settings: AppSettings,
    private val profile: ProfileRepository,
    private val secure: SecureStore,
) {
    private val worker = Executors.newSingleThreadExecutor { Thread(it, "hex-sync") }
    private val main = Handler(Looper.getMainLooper())
    private val debounced = Runnable { runNow() }
    private val runningState = MutableStateFlow(false)
    val running: StateFlow<Boolean> = runningState

    /** After an edit: wait for a pause, so typing a word doesn't upload each letter. */
    fun schedule() {
        main.removeCallbacks(debounced)
        main.postDelayed(debounced, 4000)
    }

    fun runNow() {
        val config = settings.current
        if (!config.syncEnabled || config.syncUrl.isBlank()) return
        worker.execute { runBlocking() }
    }

    /** Returns null on success, or a short reason safe to show. Called from background jobs too. */
    @Synchronized
    fun runBlocking(): String? {
        val config = settings.current
        if (!config.syncEnabled || config.syncUrl.isBlank()) return null
        runningState.value = true
        val error = try {
            validateUrl(config.syncUrl)
            ProfileSync(HttpProfileTransport(config.syncUrl, secure.token(SecureStore.SYNC_TOKEN)), profile).sync()
            null
        } catch (error: SyncAuthException) {
            error.message
        } catch (error: ProfileException) {
            when (error.kind) {
                ProfileException.Kind.TooNew -> "The server has a newer profile format. Update Hex."
                else -> "The server's profile can't be read. Local data is unchanged."
            }
        } catch (_: SyncGaveUpException) {
            "Another device kept changing the profile. Will try again."
        } catch (_: IOException) {
            "Can't reach the sync server. Edits are saved here and will sync later."
        } catch (error: IllegalArgumentException) {
            error.message
        }
        Log.i(TAG, if (error == null) "sync ok" else "sync failed")
        settings.update {
            it.copy(lastSyncError = error, lastSyncMs = if (error == null) System.currentTimeMillis() else it.lastSyncMs)
        }
        runningState.value = false
        return error
    }

    companion object {
        private const val TAG = "HexSync"

        fun validateUrl(url: String) {
            val uri = runCatching { URI(url) }.getOrNull()
            val host = uri?.host ?: throw IllegalArgumentException("That doesn't look like a URL.")
            if (uri.scheme != "https" && host != "10.0.2.2") throw IllegalArgumentException("Sync needs an HTTPS URL.")
        }
    }
}
