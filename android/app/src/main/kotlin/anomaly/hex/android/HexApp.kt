package anomaly.hex.android

import android.app.Application
import android.content.Context
import anomaly.hex.android.audio.AudioArchive
import anomaly.hex.android.core.pipeline.CleanupPipeline
import anomaly.hex.android.data.AppSettings
import anomaly.hex.android.data.HistoryStore
import anomaly.hex.android.data.MaintenanceJobService
import anomaly.hex.android.data.ProfileRepository
import anomaly.hex.android.data.SecureStore
import anomaly.hex.android.engine.EngineClient
import anomaly.hex.android.models.ModelStore
import anomaly.hex.android.core.codex.CodexPolisher
import anomaly.hex.android.core.codex.CodexSession
import anomaly.hex.android.data.SecureCodexTokens
import anomaly.hex.android.sync.ChatGptSignIn
import kotlinx.coroutines.flow.MutableStateFlow
import anomaly.hex.android.sync.SyncRunner
import java.io.File

/** The app's owners. Created in the main process only; the engine process runs none of this. */
class HexApp : Application() {
    lateinit var settings: AppSettings private set
    lateinit var profile: ProfileRepository private set
    lateinit var history: HistoryStore private set
    lateinit var archive: AudioArchive private set
    lateinit var models: ModelStore private set
    lateinit var secure: SecureStore private set
    lateinit var engine: EngineClient private set
    lateinit var sync: SyncRunner private set
    lateinit var codex: CodexSession private set
    lateinit var chatGpt: ChatGptSignIn private set

    val captureDir get() = File(noBackupFilesDir, "capture")

    fun cleanupPipeline(): CleanupPipeline {
        val current = settings.current
        return CleanupPipeline(
            localCleaner = current.cleanupModel?.takeIf(models::isInstalled)?.let {
                engine.cleaner(models.file(it).path, current.threads, current.localCleanupDeadlineSeconds * 1000L)
            },
            polisher = if (codex.signedIn) CodexPolisher(codex) else null,
        )
    }

    override fun onCreate() {
        super.onCreate()
        if (getProcessName().endsWith(":engine")) return
        settings = AppSettings(this)
        profile = ProfileRepository(this)
        history = HistoryStore(this)
        archive = AudioArchive(this)
        models = ModelStore(this)
        secure = SecureStore(this)
        val signedIn = MutableStateFlow(false)
        codex = CodexSession(SecureCodexTokens(secure), onChange = { signedIn.value = it })
        signedIn.value = codex.signedIn
        chatGpt = ChatGptSignIn(codex, signedIn)
        engine = EngineClient(this) { settings.current.unloadIdleMinutes }
        sync = SyncRunner(settings, profile, secure)
        profile.onEdited = { sync.schedule() }
        archive.resume()
        MaintenanceJobService.schedule(this)
    }

    companion object {
        fun get(context: Context) = context.applicationContext as HexApp
    }
}
