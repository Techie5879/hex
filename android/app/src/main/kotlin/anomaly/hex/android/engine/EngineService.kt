package anomaly.hex.android.engine

import android.app.Service
import android.content.Intent
import android.os.IBinder
import anomaly.hex.android.nativeasr.AsrException
import anomaly.hex.android.nativeasr.NativeAsr
import anomaly.hex.android.nativellm.LlmException
import anomaly.hex.android.nativellm.NativeLlm
import java.io.File
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.put

/** Replies cross binder as a one-letter status and a body. Errors carry no user text. */
object EngineReply {
    fun text(value: String) = "T$value"
    fun cancelled() = "C"
    fun deadline() = "D"
    fun failed(message: String) = "F$message"
}

/**
 * Owns the loaded models in the `:engine` process, one inference at a time.
 * A native abort or out-of-memory kill here surfaces to the app as a dead
 * binder; the recording is already on disk, so the person can retry.
 */
class EngineService : Service() {
    private val lock = Any()
    private var asr: NativeAsr? = null
    private var llm: NativeLlm? = null
    @Volatile private var runningAsr: NativeAsr? = null
    @Volatile private var runningLlm: NativeLlm? = null
    @Volatile private var cancelRequested = false

    private fun speech(path: String, threads: Int): NativeAsr {
        asr?.let { if (it.modelPath == path) return it }
        asr?.close()
        asr = null
        return NativeAsr.open(path, threads).also { asr = it }
    }

    private fun cleaner(path: String, threads: Int): NativeLlm {
        llm?.let { if (it.modelPath == path) return it }
        llm?.close()
        llm = null
        return NativeLlm.open(path, threads).also { llm = it }
    }

    private fun failure(error: Exception): String = when (error.message) {
        "cancelled" -> EngineReply.cancelled()
        "deadline" -> EngineReply.deadline()
        else -> EngineReply.failed(error.message ?: "Engine failed")
    }

    private val binder = object : IHexEngine.Stub() {
        override fun prewarm(modelPath: String, threads: Int) {
            synchronized(lock) { runCatching { speech(modelPath, threads) } }
        }

        override fun hasSpeech(detectorPath: String, pcmPath: String): Int =
            synchronized(lock) { NativeAsr.hasSpeech(detectorPath, pcmPath) }

        override fun transcribe(pcmPath: String, modelPath: String, language: String, prompt: String, threads: Int, deadlineMs: Long): String =
            synchronized(lock) {
                cancelRequested = false
                try {
                    val engine = speech(modelPath, threads)
                    if (cancelRequested) throw AsrException("cancelled")
                    runningAsr = engine
                    EngineReply.text(engine.transcribe(pcmPath, language, prompt, deadlineMs))
                } catch (error: AsrException) {
                    failure(error)
                } finally {
                    runningAsr = null
                }
            }

        override fun cleanup(modelPath: String, system: String, transcript: String, threads: Int, deadlineMs: Long): String =
            synchronized(lock) {
                cancelRequested = false
                try {
                    val engine = cleaner(modelPath, threads)
                    if (cancelRequested) throw LlmException("cancelled")
                    runningLlm = engine
                    EngineReply.text(engine.clean(system, transcript, deadlineMs))
                } catch (error: LlmException) {
                    failure(error)
                } finally {
                    runningLlm = null
                }
            }

        override fun cancel() {
            cancelRequested = true
            runningAsr?.cancel()
            runningLlm?.cancel()
        }

        override fun release() {
            synchronized(lock) {
                asr?.close()
                asr = null
                llm?.close()
                llm = null
            }
        }

        override fun stats(): String {
            val status = runCatching { File("/proc/self/status").readLines() }.getOrDefault(emptyList())
            fun kb(name: String) = status.firstOrNull { it.startsWith(name) }?.filter(Char::isDigit)?.toLongOrNull() ?: 0L
            return buildJsonObject {
                put("peak_rss_kb", kb("VmHWM"))
                put("rss_kb", kb("VmRSS"))
                put("speech_loaded", asr != null)
                put("cleanup_loaded", llm != null)
            }.toString()
        }
    }

    override fun onBind(intent: Intent): IBinder = binder

    override fun onTrimMemory(level: Int) {
        super.onTrimMemory(level)
        // Under memory pressure the next dictation reloads instead of this process being killed.
        if (level >= TRIM_MEMORY_RUNNING_CRITICAL) binder.release()
    }

    override fun onDestroy() {
        binder.release()
        super.onDestroy()
    }
}
