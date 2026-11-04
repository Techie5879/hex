package anomaly.hex.android.sync

import anomaly.hex.android.core.sync.ProfileTransport
import anomaly.hex.android.core.sync.RemoteDocument
import anomaly.hex.android.core.sync.SyncConflictException
import java.io.IOException
import java.net.HttpURLConnection
import java.net.URL

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
