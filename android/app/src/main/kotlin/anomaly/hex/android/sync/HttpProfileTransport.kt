package anomaly.hex.android.sync

import anomaly.hex.android.core.pipeline.CancelToken
import anomaly.hex.android.core.pipeline.CancelledException
import anomaly.hex.android.core.pipeline.GenerationException
import anomaly.hex.android.core.pipeline.GenerationTransport
import anomaly.hex.android.core.pipeline.ModelRef
import anomaly.hex.android.core.pipeline.Polisher
import anomaly.hex.android.core.pipeline.QuotaFallbackGenerator
import anomaly.hex.android.core.sync.ProfileTransport
import anomaly.hex.android.core.sync.RemoteDocument
import anomaly.hex.android.core.sync.SyncConflictException
import java.io.IOException
import java.net.HttpURLConnection
import java.net.SocketTimeoutException
import java.net.URL
import java.util.Timer
import kotlin.concurrent.timerTask

/** Rejected by the server: wrong token, or a URL that isn't this document. */
class SyncAuthException : IOException("The server refused the token")

/**
 * GET and PUT one document with ETag and If-Match: the Hex companion, or any
 * WebDAV server. Basic authentication is used when the token holds `user:password`.
 */
class HttpProfileTransport(private val url: String, private val token: String?) : ProfileTransport {
    private fun open(method: String): HttpURLConnection = (URL(url).openConnection() as HttpURLConnection).apply {
        requestMethod = method
        connectTimeout = 10_000
        readTimeout = 15_000
        setRequestProperty("Accept", "application/json")
        authorization(token)?.let { setRequestProperty("Authorization", it) }
    }

    override fun fetch(): RemoteDocument {
        val connection = open("GET")
        try {
            when (val code = connection.responseCode) {
                200 -> return RemoteDocument(connection.inputStream.bufferedReader().use { it.readText() }, connection.getHeaderField("ETag"))
                404 -> return RemoteDocument(null, null)
                401, 403 -> throw SyncAuthException()
                else -> throw IOException("Sync server said $code")
            }
        } finally {
            connection.disconnect()
        }
    }

    override fun store(text: String, ifMatch: String?): String? {
        val connection = open("PUT")
        try {
            connection.doOutput = true
            connection.setRequestProperty("Content-Type", "application/json")
            if (ifMatch != null) connection.setRequestProperty("If-Match", ifMatch) else connection.setRequestProperty("If-None-Match", "*")
            connection.outputStream.use { it.write(text.toByteArray()) }
            when (val code = connection.responseCode) {
                in 200..299 -> return connection.getHeaderField("ETag")
                412 -> throw SyncConflictException()
                401, 403 -> throw SyncAuthException()
                else -> throw IOException("Sync server said $code")
            }
        } finally {
            connection.disconnect()
        }
    }

    companion object {
        fun authorization(token: String?): String? = when {
            token.isNullOrEmpty() -> null
            ':' in token -> "Basic " + java.util.Base64.getEncoder().encodeToString(token.toByteArray())
            else -> "Bearer $token"
        }
    }
}

/** Sends a polish request to the companion, which holds the provider credentials. */
class CompanionTransport(private val url: String, private val token: String?) : GenerationTransport {
    override fun post(body: String, remainingMs: Long, cancel: CancelToken): String {
        val connection = (URL(url).openConnection() as HttpURLConnection).apply {
            requestMethod = "POST"
            doOutput = true
            connectTimeout = remainingMs.coerceIn(1, 3000).toInt()
            readTimeout = remainingMs.coerceAtLeast(1).toInt()
            setRequestProperty("Content-Type", "application/json")
            setRequestProperty("X-Hex-Deadline-Ms", remainingMs.toString())
            HttpProfileTransport.authorization(token)?.let { setRequestProperty("Authorization", it) }
        }
        cancel.onCancel { connection.disconnect() }
        // Socket timeouts bound each read, not the whole reply: this bounds the whole request.
        var timedOut = false
        val watchdog = Timer("hex-polish-deadline", true).apply {
            schedule(timerTask { timedOut = true; connection.disconnect() }, remainingMs.coerceAtLeast(1))
        }
        try {
            connection.outputStream.use { it.write(body.toByteArray()) }
            val code = connection.responseCode
            val stream = if (code in 200..299) connection.inputStream else connection.errorStream
            val reply = stream?.bufferedReader()?.use { it.readText() } ?: throw GenerationException("Companion returned nothing")
            // A provider's own 401 arrives as a JSON envelope; the companion's is plain text.
            if ((code == 401 || code == 403) && !reply.trimStart().startsWith("{")) throw GenerationException("Companion refused the token")
            return reply
        } catch (_: SocketTimeoutException) {
            throw GenerationException("Polish timed out")
        } catch (_: IOException) {
            if (cancel.cancelled) throw CancelledException()
            throw GenerationException(if (timedOut) "Polish timed out" else "Polish could not reach the companion")
        } finally {
            watchdog.cancel()
            connection.disconnect()
        }
    }
}

class CompanionPolisher(url: String, token: String?) : Polisher {
    private val generator = QuotaFallbackGenerator(CompanionTransport(url, token))
    override fun polish(prompt: String, model: ModelRef, fallbackProvider: String?, deadlineMs: Long, cancel: CancelToken): String =
        generator.generate(prompt, model, fallbackProvider, deadlineMs, cancel)
}
