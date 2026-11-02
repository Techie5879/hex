package anomaly.hex.android.audio

import android.annotation.SuppressLint
import android.content.Context
import android.media.AudioFormat
import android.media.AudioManager
import android.media.AudioRecord
import android.media.AudioRecordingConfiguration
import android.media.MediaRecorder
import android.os.Handler
import android.os.Looper
import android.os.SystemClock
import anomaly.hex.android.core.models.AudioLevels
import java.io.BufferedOutputStream
import java.io.File
import java.io.FileOutputStream
import java.nio.ByteBuffer
import java.nio.ByteOrder
import java.util.concurrent.atomic.AtomicBoolean

interface CaptureListener {
    fun onStarted()
    /** Called about 30 times a second, on the main thread. Peak in 0..1. */
    fun onLevel(peak: Float)
    fun onSilenced(silenced: Boolean)
    /** The recorder failed mid-way. Audio delivered so far stays in the file. */
    fun onFailed(permission: Boolean)
    /** Called after this much continuous silence in hands-free mode. */
    fun onLongSilence()
}

/**
 * Records 16 kHz mono PCM straight to a file, synced each second, so a crash
 * loses at most a second. The microphone opens only in [start] and closes as
 * soon as capture ends: the buffered tail is kept up to the moment of stop.
 */
class AudioCapture(
    private val context: Context,
    private val file: File,
    private val listener: CaptureListener,
    private val longSilenceMs: Long,
) {
    private val main = Handler(Looper.getMainLooper())
    private val running = AtomicBoolean(false)
    private var thread: Thread? = null
    private var record: AudioRecord? = null
    private var tailUntil = 0L
    private val callback = object : AudioManager.AudioRecordingCallback() {
        override fun onRecordingConfigChanged(configs: MutableList<AudioRecordingConfiguration>) {
            val session = record?.audioSessionId ?: return
            val mine = configs.firstOrNull { it.clientAudioSessionId == session } ?: return
            main.post { listener.onSilenced(mine.isClientSilenced) }
        }
    }

    @SuppressLint("MissingPermission")
    fun start() {
        val minimum = AudioRecord.getMinBufferSize(RATE, AudioFormat.CHANNEL_IN_MONO, AudioFormat.ENCODING_PCM_16BIT)
        val recorder = try {
            AudioRecord(
                MediaRecorder.AudioSource.VOICE_RECOGNITION,
                RATE,
                AudioFormat.CHANNEL_IN_MONO,
                AudioFormat.ENCODING_PCM_16BIT,
                maxOf(minimum, RATE),
            )
        } catch (_: SecurityException) {
            main.post { listener.onFailed(permission = true) }
            return
        }
        if (recorder.state != AudioRecord.STATE_INITIALIZED) {
            recorder.release()
            main.post { listener.onFailed(permission = false) }
            return
        }
        record = recorder
        running.set(true)
        thread = Thread({ loop(recorder) }, "hex-capture").apply {
            priority = Thread.MAX_PRIORITY
            start()
        }
    }

    private fun loop(recorder: AudioRecord) {
        var failed = false
        try {
            recorder.registerAudioRecordingCallback({ it.run() }, callback)
            recorder.startRecording()
            if (recorder.recordingState != AudioRecord.RECORDSTATE_RECORDING) {
                main.post { listener.onFailed(permission = false) }
                return
            }
            main.post { listener.onStarted() }
            FileOutputStream(file).use { raw ->
                BufferedOutputStream(raw, 64 * 1024).use { out ->
                    val shorts = ShortArray(RATE / 30)
                    val bytes = ByteBuffer.allocate(shorts.size * 2).order(ByteOrder.LITTLE_ENDIAN)
                    var lastSync = SystemClock.elapsedRealtime()
                    var lastVoice = lastSync
                    while (running.get() || SystemClock.elapsedRealtime() < tailUntil) {
                        val read = recorder.read(shorts, 0, shorts.size)
                        if (read < 0) {
                            failed = true
                            break
                        }
                        if (read == 0) continue
                        bytes.clear()
                        bytes.asShortBuffer().put(shorts, 0, read)
                        out.write(bytes.array(), 0, read * 2)
                        val peak = AudioLevels.peak(shorts, read)
                        main.post { listener.onLevel(peak) }
                        val now = SystemClock.elapsedRealtime()
                        if (AudioLevels.rmsDbfs(shorts, 0, read) >= AudioLevels.SILENCE_THRESHOLD_DBFS) lastVoice = now
                        if (longSilenceMs > 0 && now - lastVoice >= longSilenceMs) {
                            lastVoice = now
                            main.post { listener.onLongSilence() }
                        }
                        if (now - lastSync >= 1000) {
                            out.flush()
                            raw.fd.sync()
                            lastSync = now
                        }
                    }
                    out.flush()
                    raw.fd.sync()
                }
            }
        } catch (_: Exception) {
            failed = true
        } finally {
            runCatching { recorder.stop() }
            runCatching { recorder.unregisterAudioRecordingCallback(callback) }
            recorder.release()
        }
        if (failed && running.get()) main.post { listener.onFailed(permission = false) }
    }

    /** Stops and waits for the file to be complete, keeping [tailMs] more audio. */
    fun stop(tailMs: Long) {
        tailUntil = SystemClock.elapsedRealtime() + tailMs
        running.set(false)
        thread?.join(tailMs + 2000)
    }

    private companion object {
        const val RATE = AudioArchive.SAMPLE_RATE
    }
}
