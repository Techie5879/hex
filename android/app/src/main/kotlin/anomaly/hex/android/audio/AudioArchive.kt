package anomaly.hex.android.audio

import android.content.Context
import android.media.MediaCodec
import android.media.MediaFormat
import android.media.MediaMuxer
import android.util.Log
import anomaly.hex.android.core.retention.Retention
import java.io.File
import java.io.FileInputStream
import java.util.concurrent.Executors

/**
 * Local compressed copies of dictations, independent of text history. A capture
 * is staged as raw PCM before anything else runs, then encoded to Ogg Opus in the
 * background, as on desktop. A failed encode leaves the staged PCM to retry.
 * Nothing here ever leaves the device. Off stops new saves and keeps existing clips.
 */
class AudioArchive(context: Context) {
    val root = File(context.noBackupFilesDir, "dictation-audio").apply { mkdirs() }
    private val pending = File(root, "pending").apply { mkdirs() }
    private val worker = Executors.newSingleThreadExecutor { Thread(it, "hex-audio-archive") }

    /** Queues [pcm] (left in place) for encoding. Returns the clip's final name. */
    fun retain(pcm: File, id: String, retention: Retention): String? {
        if (!retention.keepsNew || pcm.length() == 0L) return null
        val staged = File(pending, "$id.pcm")
        pcm.copyTo(staged, overwrite = true)
        worker.execute { encodePending() }
        return "$id.ogg"
    }

    /** Clips that a crash or failed encode left behind. */
    fun resume() = worker.execute { encodePending() }

    private fun encodePending() {
        for (file in pending.listFiles { f -> f.extension == "pcm" }.orEmpty()) {
            val output = File(root, "${file.nameWithoutExtension}.ogg")
            val partial = File(root, "${file.nameWithoutExtension}.ogg.partial")
            try {
                encodeOggOpus(file, partial)
                if (partial.renameTo(output)) file.delete()
            } catch (error: Exception) {
                partial.delete()
                Log.e(TAG, "Encode failed; staged audio retained: ${error.javaClass.simpleName}")
            }
        }
    }

    fun clip(name: String) = File(root, name).takeIf { it.isFile && it.parentFile == root }

    fun delete(name: String) {
        clip(name)?.delete()
    }

    fun totalBytes() = root.walkTopDown().filter { it.isFile }.sumOf { it.length() }

    fun clipCount() = root.listFiles { f -> f.extension == "ogg" }.orEmpty().size

    fun deleteAll() {
        root.listFiles().orEmpty().filter { it.isFile }.forEach { it.delete() }
        pending.listFiles().orEmpty().forEach { it.delete() }
    }

    /**
     * Applies a dated retention to everything held: clips, and anything a failed encode
     * left staged. Files are dated by their name's first field, then their mtime.
     */
    fun prune(retention: Retention, nowMs: Long = System.currentTimeMillis()): Int {
        if (retention.maxAgeMs == null) return 0
        var removed = 0
        for (file in root.walkTopDown().filter { it.isFile }) {
            val created = file.name.substringBefore('-').toLongOrNull() ?: file.lastModified()
            if (retention.isExpired(created, nowMs) && file.delete()) removed += 1
        }
        return removed
    }

    private fun encodeOggOpus(pcm: File, output: File) {
        val format = MediaFormat.createAudioFormat(MediaFormat.MIMETYPE_AUDIO_OPUS, SAMPLE_RATE, 1).apply {
            setInteger(MediaFormat.KEY_BIT_RATE, 24_000)
            setInteger(MediaFormat.KEY_MAX_INPUT_SIZE, 16 * 1024)
        }
        val codec = MediaCodec.createEncoderByType(MediaFormat.MIMETYPE_AUDIO_OPUS)
        val muxer = MediaMuxer(output.path, MediaMuxer.OutputFormat.MUXER_OUTPUT_OGG)
        var track = -1
        var muxing = false
        try {
            codec.configure(format, null, null, MediaCodec.CONFIGURE_FLAG_ENCODE)
            codec.start()
            FileInputStream(pcm).use { input ->
                val chunk = ByteArray(8 * 1024)
                val info = MediaCodec.BufferInfo()
                var written = 0L
                var inputDone = false
                var outputDone = false
                while (!outputDone) {
                    if (!inputDone) {
                        val index = codec.dequeueInputBuffer(10_000)
                        if (index >= 0) {
                            val buffer = codec.getInputBuffer(index)!!
                            val read = input.read(chunk, 0, minOf(chunk.size, buffer.remaining()))
                            val presentationUs = written * 1_000_000 / (SAMPLE_RATE * 2)
                            if (read <= 0) {
                                codec.queueInputBuffer(index, 0, 0, presentationUs, MediaCodec.BUFFER_FLAG_END_OF_STREAM)
                                inputDone = true
                            } else {
                                buffer.put(chunk, 0, read)
                                codec.queueInputBuffer(index, 0, read, presentationUs, 0)
                                written += read
                            }
                        }
                    }
                    val out = codec.dequeueOutputBuffer(info, 10_000)
                    when {
                        out == MediaCodec.INFO_OUTPUT_FORMAT_CHANGED -> {
                            track = muxer.addTrack(codec.outputFormat)
                            muxer.start()
                            muxing = true
                        }
                        out >= 0 -> {
                            val buffer = codec.getOutputBuffer(out)!!
                            val config = info.flags and MediaCodec.BUFFER_FLAG_CODEC_CONFIG != 0
                            if (muxing && !config && info.size > 0) muxer.writeSampleData(track, buffer, info)
                            codec.releaseOutputBuffer(out, false)
                            if (info.flags and MediaCodec.BUFFER_FLAG_END_OF_STREAM != 0) outputDone = true
                        }
                    }
                }
            }
        } finally {
            runCatching { codec.stop() }
            codec.release()
            if (muxing) runCatching { muxer.stop() }
            muxer.release()
        }
    }

    companion object {
        private const val TAG = "HexAudio"
        const val SAMPLE_RATE = 16_000
    }
}
