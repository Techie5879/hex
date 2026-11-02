package anomaly.hex.android.models

import android.content.Context
import anomaly.hex.android.core.models.ModelArtifact
import anomaly.hex.android.core.models.ModelCatalog
import java.io.File
import java.io.FileOutputStream
import java.io.IOException
import java.net.HttpURLConnection
import java.net.URL
import java.security.MessageDigest
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow

sealed interface DownloadState {
    data object Idle : DownloadState
    data class Running(val modelId: String, val done: Long, val total: Long) : DownloadState
    data class Verifying(val modelId: String) : DownloadState
    data class Failed(val modelId: String, val reason: String) : DownloadState
}

class ModelCancelled : Exception()

/** Models live outside the APK and backups, verified against a pinned digest before use. */
class ModelStore(private val context: Context) {
    val directory = File(context.noBackupFilesDir, "models").apply { mkdirs() }
    private val states = MutableStateFlow<DownloadState>(DownloadState.Idle)
    val state: StateFlow<DownloadState> = states
    private val installedRevision = MutableStateFlow(0)
    val revision: StateFlow<Int> = installedRevision
    @Volatile private var cancelRequested = false

    fun file(id: String) = File(directory, "$id.gguf")
    private fun part(id: String) = File(directory, "$id.gguf.part")

    fun isInstalled(id: String) = file(id).isFile
    fun partialBytes(id: String) = part(id).takeIf { it.isFile }?.length() ?: 0L
    fun installedSpeech() = ModelCatalog.SPEECH.filter { isInstalled(it.id) }

    /** The speech detector ships in the APK; the engine needs it as a file. */
    fun speechDetector(): File {
        val target = File(directory, DETECTOR)
        if (!target.isFile) {
            val staged = File(directory, "$DETECTOR.tmp")
            context.assets.open(DETECTOR).use { input -> staged.outputStream().use { input.copyTo(it) } }
            staged.renameTo(target)
        }
        return target
    }

    fun remove(id: String) {
        file(id).delete()
        part(id).delete()
        installedRevision.value += 1
    }

    fun cancel() {
        cancelRequested = true
    }

    /** Resumes a partial file with a Range request, checks its size and SHA-256, then activates it. */
    fun download(artifact: ModelArtifact) {
        cancelRequested = false
        val part = part(artifact.id)
        try {
            val free = directory.usableSpace
            val needed = artifact.sizeBytes - partialBytes(artifact.id)
            if (free < needed + RESERVE_BYTES) throw IOException("Not enough storage: needs ${needed / MIB} MB free")
            fetch(artifact, part)
            states.value = DownloadState.Verifying(artifact.id)
            val digest = sha256(part)
            if (digest != artifact.sha256) {
                part.delete()
                throw IOException("Checksum mismatch. The download was discarded.")
            }
            if (!part.renameTo(file(artifact.id))) throw IOException("Could not save the model")
            states.value = DownloadState.Idle
            installedRevision.value += 1
        } catch (_: ModelCancelled) {
            states.value = DownloadState.Idle
        } catch (error: IOException) {
            states.value = DownloadState.Failed(artifact.id, error.message ?: "Download failed")
        }
    }

    private fun fetch(artifact: ModelArtifact, part: File) {
        var url = URL(artifact.url)
        var redirects = 0
        while (true) {
            val offset = part.takeIf { it.isFile }?.length() ?: 0L
            if (offset == artifact.sizeBytes) return
            val connection = (url.openConnection() as HttpURLConnection).apply {
                connectTimeout = 15_000
                readTimeout = 30_000
                instanceFollowRedirects = false
                if (offset > 0) setRequestProperty("Range", "bytes=$offset-")
            }
            try {
                val code = connection.responseCode
                if (code in 300..399) {
                    val location = connection.getHeaderField("Location") ?: throw IOException("Bad redirect")
                    if (++redirects > 8) throw IOException("Too many redirects")
                    url = URL(url, location)
                    if (url.protocol != "https") throw IOException("Refused a non-HTTPS download")
                    continue
                }
                val resuming = code == HttpURLConnection.HTTP_PARTIAL
                when {
                    code == 416 -> {
                        part.delete()
                        continue
                    }
                    code != HttpURLConnection.HTTP_OK && !resuming -> throw IOException("Server said $code")
                }
                FileOutputStream(part, resuming).use { out ->
                    val buffer = ByteArray(256 * 1024)
                    var done = if (resuming) offset else 0L
                    connection.inputStream.use { input ->
                        while (true) {
                            if (cancelRequested) throw ModelCancelled()
                            val read = input.read(buffer)
                            if (read < 0) break
                            out.write(buffer, 0, read)
                            done += read
                            states.value = DownloadState.Running(artifact.id, done, artifact.sizeBytes)
                        }
                    }
                }
                if (part.length() != artifact.sizeBytes) throw IOException("The download ended early. Try again to resume.")
                return
            } finally {
                connection.disconnect()
            }
        }
    }

    private fun sha256(file: File): String {
        val digest = MessageDigest.getInstance("SHA-256")
        file.inputStream().use { input ->
            val buffer = ByteArray(1024 * 1024)
            while (true) {
                val read = input.read(buffer)
                if (read < 0) break
                digest.update(buffer, 0, read)
            }
        }
        return digest.digest().joinToString("") { "%02x".format(it) }
    }

    private companion object {
        const val DETECTOR = "silero-vad-v6.2-F32.gguf"
        const val MIB = 1024 * 1024
        const val RESERVE_BYTES = 200L * MIB
    }
}
