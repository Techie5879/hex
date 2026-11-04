package anomaly.hex.android.engine

import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.content.ServiceConnection
import android.os.DeadObjectException
import android.os.Handler
import android.os.IBinder
import android.os.Looper
import android.os.RemoteException
import anomaly.hex.android.core.pipeline.CancelToken
import anomaly.hex.android.core.pipeline.CancelledException
import anomaly.hex.android.core.pipeline.LocalCleaner
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit

class EngineDeadlineException : Exception("Took too long")
class EngineDiedException : Exception("The speech engine stopped unexpectedly")
class EngineException(message: String) : Exception(message)

/** The app's connection to the `:engine` process, released after an idle period. */
class EngineClient(private val context: Context, private val idleMinutes: () -> Int) {
    private val main = Handler(Looper.getMainLooper())
    private val lock = Object()
    private var engine: IHexEngine? = null
    private var bound = false
    private val release = Runnable { unbind() }

    private val connection = object : ServiceConnection {
        override fun onServiceConnected(name: ComponentName, service: IBinder) {
            synchronized(lock) {
                engine = IHexEngine.Stub.asInterface(service)
                lock.notifyAll()
            }
        }

        override fun onServiceDisconnected(name: ComponentName) {
            synchronized(lock) { engine = null }
        }

        override fun onBindingDied(name: ComponentName) {
            unbind()
        }
    }

    private fun connected(): IHexEngine {
        main.removeCallbacks(release)
        synchronized(lock) {
            if (!bound) {
                bound = context.bindService(Intent(context, EngineService::class.java), connection, Context.BIND_AUTO_CREATE)
                if (!bound) throw EngineException("Could not start the speech engine")
            }
            val deadline = System.currentTimeMillis() + 20_000
            while (engine == null) {
                val left = deadline - System.currentTimeMillis()
                if (left <= 0) throw EngineDiedException()
                lock.wait(left)
            }
            return engine!!
        }
    }

    private fun touch() {
        main.removeCallbacks(release)
        main.postDelayed(release, idleMinutes().coerceAtLeast(1) * 60_000L)
    }

    private fun unbind() {
        synchronized(lock) {
            if (!bound) return
            runCatching { engine?.release() }
            runCatching { context.unbindService(connection) }
            bound = false
            engine = null
        }
    }

    fun prewarm(modelPath: String, threads: Int) {
        Thread({ runCatching { connected().prewarm(modelPath, threads); touch() } }, "hex-prewarm").start()
    }

    private inline fun call(cancel: CancelToken, block: (IHexEngine) -> String): String {
        val service = connected()
        cancel.onCancel { runCatching { service.cancel() } }
        try {
            val reply = block(service)
            return when (reply.firstOrNull()) {
                'T' -> reply.substring(1)
                'C' -> throw CancelledException()
                'D' -> throw EngineDeadlineException()
                else -> throw EngineException(reply.drop(1).ifEmpty { "Engine failed" })
            }
        } catch (_: DeadObjectException) {
            unbind()
            throw EngineDiedException()
        } catch (_: RemoteException) {
            unbind()
            throw EngineDiedException()
        } finally {
            touch()
        }
    }

    fun transcribe(pcmPath: String, modelPath: String, language: String, prompt: String, threads: Int, deadlineMs: Long, cancel: CancelToken): String =
        call(cancel) { it.transcribe(pcmPath, modelPath, language, prompt, threads, deadlineMs) }

    /** Null when the detector can't say, so the level check alone decides. */
    fun hasSpeech(detectorPath: String, pcmPath: String): Boolean? = try {
        when (connected().hasSpeech(detectorPath, pcmPath)) {
            1 -> true
            0 -> false
            else -> null
        }
    } catch (_: RemoteException) {
        unbind()
        null
    } finally {
        touch()
    }

    fun cleaner(modelPath: String, threads: Int, deadlineMs: Long) = LocalCleaner { system, transcript, cancel ->
        call(cancel) { it.cleanup(modelPath, system, transcript, threads, deadlineMs) }
    }

    fun stats(): String? = runCatching { connected().stats() }.getOrNull()
}
