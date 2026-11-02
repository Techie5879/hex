package anomaly.hex.android.debug

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.util.Log
import anomaly.hex.android.HexApp
import anomaly.hex.android.service.DictationCoordinator

/**
 * Test hooks for emulator runs. Absent from release builds, because this source set
 * only compiles into debug. It cannot weaken a privacy gate: it can set options a
 * person could set, or ask the real inserter to place text, which still re-checks the field.
 */
class DebugReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        val app = HexApp.get(context)
        when (intent.action) {
            "anomaly.hex.android.debug.CONFIG" -> {
                app.settings.update { settings ->
                    settings.copy(
                        setupDone = intent.getBooleanExtra("setupDone", settings.setupDone),
                        speechModel = intent.getStringExtra("speechModel") ?: settings.speechModel,
                        cleanupModel = intent.getStringExtra("cleanupModel") ?: settings.cleanupModel,
                        asrDeadlineFactor = intent.getIntExtra("asrDeadlineFactor", settings.asrDeadlineFactor),
                        threads = intent.getIntExtra("threads", settings.threads),
                        localCleanupDeadlineSeconds = intent.getIntExtra("localCleanupDeadlineSeconds", settings.localCleanupDeadlineSeconds),
                        syncUrl = intent.getStringExtra("syncUrl") ?: settings.syncUrl,
                        syncEnabled = intent.getBooleanExtra("syncEnabled", settings.syncEnabled),
                    )
                }
                intent.getStringExtra("token")?.let { app.secure.putToken("sync", it) }
                intent.getStringExtra("codexAccess")?.let {
                    app.codex.signIn(anomaly.hex.android.core.codex.CodexTokens(it, "", intent.getStringExtra("codexAccount").orEmpty()))
                }
                if (intent.getBooleanExtra("codexSignOut", false)) app.codex.signOut()
                intent.getStringExtra("addTerm")?.let { app.profile.putTerm(it) }
                intent.getStringExtra("removeTerm")?.let { app.profile.removeTerm(it) }
                intent.getStringExtra("settingKey")?.let { key ->
                    val raw = intent.getStringExtra("settingJson") ?: return@let
                    app.profile.putSetting(key, kotlinx.serialization.json.Json.parseToJsonElement(raw))
                }
                intent.getStringExtra("download")?.let { anomaly.hex.android.models.ModelDownloadService.start(context, it) }
                if (intent.getBooleanExtra("pauseDownload", false)) app.models.cancel()
                if (intent.getBooleanExtra("syncNow", false)) app.sync.runNow()
                if (intent.getBooleanExtra("dismissResult", false)) DictationCoordinator.active?.onCancelTapped()
                Log.i(TAG, "config applied")
            }
            "anomaly.hex.android.debug.INSERT" -> {
                val text = intent.getStringExtra("textBase64")?.let { String(android.util.Base64.decode(it, android.util.Base64.DEFAULT)) } ?: intent.getStringExtra("text").orEmpty()
                val coordinator = DictationCoordinator.active
                if (coordinator == null) Log.w(TAG, "insert: no service") else coordinator.debugInsert(text) { Log.i(TAG, "insert outcome: $it") }
            }
        }
    }

    private companion object {
        const val TAG = "HexDebug"
    }
}
