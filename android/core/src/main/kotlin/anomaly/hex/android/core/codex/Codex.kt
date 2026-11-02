package anomaly.hex.android.core.codex

import anomaly.hex.android.core.pipeline.CancelToken
import anomaly.hex.android.core.pipeline.CancelledException
import anomaly.hex.android.core.pipeline.GenerationException
import anomaly.hex.android.core.pipeline.Polisher
import java.io.IOException
import java.net.HttpURLConnection
import java.net.SocketTimeoutException
import java.net.URL
import java.net.URLEncoder
import java.util.Base64
import java.util.Timer
import java.util.concurrent.ExecutionException
import java.util.concurrent.Executors
import java.util.concurrent.Future
import java.util.concurrent.TimeUnit
import java.util.concurrent.TimeoutException
import kotlin.concurrent.timerTask
import kotlinx.serialization.Serializable
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonArray
import kotlinx.serialization.json.booleanOrNull
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.put
import kotlinx.serialization.json.putJsonObject

class CodexEndpoints(
    val auth: String = "https://auth.openai.com",
    val api: String = "https://chatgpt.com/backend-api/codex",
) {
    val approvalPage get() = "$auth/codex/device"
}

/** This phone's own ChatGPT login. It is never shared with another device's session. */
@Serializable
data class CodexTokens(val access: String, val refresh: String, val accountId: String)

interface CodexTokenStore {
    fun load(): CodexTokens?

    /** Throws IOException if the change didn't reach the disk. */
    fun save(tokens: CodexTokens?)
}

class CodexSignedOutException : Exception("Sign in with ChatGPT again")

/** This sign-in can't finish, and trying the same code again won't help. */
class CodexLoginFailedException(message: String) : IOException(message)

private const val CLIENT_ID = "app_EMoamEEZ73f0CkXaXp7hrann"
private const val USER_AGENT = "hex-android/0.1"
private const val AUTH_TIMEOUT_MS = 20_000
private const val REFRESH_MARGIN_MS = 5 * 60_000L
private const val WAIT_SLICE_MS = 50L

private fun send(url: String, contentType: String, body: String, timeoutMs: Int = AUTH_TIMEOUT_MS): Pair<Int, String> {
    val connection = (URL(url).openConnection() as HttpURLConnection).apply {
        requestMethod = "POST"
        doOutput = true
        connectTimeout = timeoutMs
        readTimeout = timeoutMs
        setRequestProperty("Content-Type", contentType)
        setRequestProperty("User-Agent", USER_AGENT)
    }
    try {
        connection.outputStream.use { it.write(body.toByteArray()) }
        val code = connection.responseCode
        val stream = if (code in 200..299) connection.inputStream else connection.errorStream
        return code to stream?.bufferedReader()?.use { it.readText() }.orEmpty()
    } finally {
        connection.disconnect()
    }
}

private fun form(vararg fields: Pair<String, String>) =
    fields.joinToString("&") { (key, value) -> "$key=${URLEncoder.encode(value, "UTF-8")}" }

private const val FORM = "application/x-www-form-urlencoded"

private fun jwtClaims(token: String): JsonObject? = runCatching {
    Json.parseToJsonElement(String(Base64.getUrlDecoder().decode(token.split('.')[1]))).jsonObject
}.getOrNull()

private fun JsonObject.string(key: String) = (this[key] as? JsonPrimitive)?.contentOrNull?.takeIf { it.isNotEmpty() }

private fun accountId(token: String?): String? =
    token?.let(::jwtClaims)?.get("https://api.openai.com/auth")?.let { it as? JsonObject }?.string("chatgpt_account_id")

/** Reads a token reply. A refresh may omit what didn't change, so [previous] fills the gaps. */
private fun parseTokens(body: String, previous: CodexTokens?): CodexTokens {
    val reply = runCatching { Json.parseToJsonElement(body).jsonObject }.getOrElse { throw IOException("Invalid token reply") }
    val access = reply.string("access_token") ?: throw IOException("Invalid token reply")
    return CodexTokens(
        access = access,
        refresh = reply.string("refresh_token") ?: previous?.refresh ?: throw IOException("Invalid token reply"),
        accountId = accountId(reply.string("id_token")) ?: accountId(access) ?: previous?.accountId
            ?: throw IOException("The sign-in has no ChatGPT account"),
    )
}

/**
 * The stored login. Refresh tokens rotate, so a refresh runs to completion on its
 * own thread even when the polish that asked for it has given up: abandoning one
 * midway would lose the new token and end the session.
 */
class CodexSession(
    private val store: CodexTokenStore,
    private val endpoints: CodexEndpoints = CodexEndpoints(),
    private val now: () -> Long = System::currentTimeMillis,
    private val onChange: (signedIn: Boolean) -> Unit = {},
) {
    private val lock = Any()
    private var refreshing: Future<CodexTokens>? = null
    private val worker = Executors.newSingleThreadExecutor { task -> Thread(task, "hex-codex-refresh").apply { isDaemon = true } }

    val signedIn: Boolean get() = synchronized(lock) { store.load() != null }

    fun signIn(tokens: CodexTokens) = change(tokens)

    fun signOut() = change(null)

    private fun change(tokens: CodexTokens?) {
        synchronized(lock) { store.save(tokens) }
        onChange(tokens != null)
    }

    /** Valid tokens within [remainingMs]. [rejected] names an access token the server refused. */
    fun fresh(remainingMs: Long, rejected: String? = null, cancel: CancelToken? = null): CodexTokens {
        val pending = synchronized(lock) {
            val tokens = store.load() ?: throw CodexSignedOutException()
            if (tokens.access != rejected && !expiresSoon(tokens.access)) return tokens
            refreshing ?: worker.submit<CodexTokens> { refresh(tokens) }.also { refreshing = it }
        }
        return await(pending, remainingMs, cancel)
    }

    /** Stops waiting on a deadline or cancel. The refresh itself keeps going, and saves its result. */
    private fun await(pending: Future<CodexTokens>, remainingMs: Long, cancel: CancelToken?): CodexTokens {
        val giveUp = now() + remainingMs
        while (true) {
            if (cancel?.cancelled == true) throw CancelledException()
            val left = giveUp - now()
            if (left <= 0) throw GenerationException("Polish timed out")
            try {
                return pending.get(minOf(left, WAIT_SLICE_MS), TimeUnit.MILLISECONDS)
            } catch (_: TimeoutException) {
                // Look at the deadline and the cancel again.
            } catch (error: ExecutionException) {
                when (val cause = error.cause) {
                    is CodexSignedOutException -> throw cause
                    else -> throw GenerationException("Couldn't renew the ChatGPT sign-in")
                }
            }
        }
    }

    private fun expiresSoon(access: String): Boolean {
        val expiry = (jwtClaims(access)?.get("exp") as? JsonPrimitive)?.contentOrNull?.toLongOrNull() ?: return false
        return expiry * 1000 - now() < REFRESH_MARGIN_MS
    }

    private fun refresh(tokens: CodexTokens): CodexTokens {
        var signedOut = false
        try {
            if (tokens.refresh.isEmpty()) {
                signedOut = clearIfCurrent(tokens)
                throw CodexSignedOutException()
            }
            val (code, body) = send(
                "${endpoints.auth}/oauth/token",
                FORM,
                form("grant_type" to "refresh_token", "refresh_token" to tokens.refresh, "client_id" to CLIENT_ID),
            )
            synchronized(lock) {
                // Signed out, or signed in again, while this ran: that choice stands.
                val current = store.load()
                if (current?.refresh != tokens.refresh) return current ?: throw CodexSignedOutException()
                when (code) {
                    200 -> return parseTokens(body, tokens).also(store::save)
                    400, 401 -> {
                        store.save(null)
                        signedOut = true
                        throw CodexSignedOutException()
                    }
                    else -> throw IOException("Token server said $code")
                }
            }
        } finally {
            synchronized(lock) { refreshing = null }
            if (signedOut) onChange(false)
        }
    }

    private fun clearIfCurrent(tokens: CodexTokens): Boolean = synchronized(lock) {
        if (store.load() != tokens) return false
        store.save(null)
        true
    }
}

/** Signing in with a code the person approves in a browser. No password reaches Hex. */
class CodexDeviceLogin(private val endpoints: CodexEndpoints = CodexEndpoints()) {
    data class Code(val id: String, val userCode: String, val intervalSeconds: Int)

    fun start(): Code {
        val (status, body) = send(
            "${endpoints.auth}/api/accounts/deviceauth/usercode",
            "application/json",
            buildJsonObject { put("client_id", CLIENT_ID) }.toString(),
        )
        if (status != 200) throw IOException("Sign-in server said $status")
        val reply = runCatching { Json.parseToJsonElement(body).jsonObject }.getOrElse { throw IOException("Invalid sign-in reply") }
        return Code(
            id = reply.string("device_auth_id") ?: throw IOException("Invalid sign-in reply"),
            userCode = reply.string("user_code") ?: throw IOException("Invalid sign-in reply"),
            intervalSeconds = (reply.string("interval")?.toIntOrNull() ?: 5).coerceIn(1, 30),
        )
    }

    /**
     * The login once approved, or null while the code is waiting for approval. Throws
     * [CodexLoginFailedException] when it can't succeed, and plain IOException for an outage.
     */
    fun poll(code: Code): CodexTokens? {
        val (status, body) = send(
            "${endpoints.auth}/api/accounts/deviceauth/token",
            "application/json",
            buildJsonObject {
                put("device_auth_id", code.id)
                put("user_code", code.userCode)
            }.toString(),
        )
        when (status) {
            200 -> Unit
            403, 404 -> return null
            in 500..599 -> throw IOException("Sign-in server said $status")
            else -> throw CodexLoginFailedException("Sign-in server said $status")
        }
        val reply = runCatching { Json.parseToJsonElement(body).jsonObject }.getOrElse { throw CodexLoginFailedException("Invalid sign-in reply") }
        val (exchange, tokens) = send(
            "${endpoints.auth}/oauth/token",
            FORM,
            form(
                "grant_type" to "authorization_code",
                "code" to (reply.string("authorization_code") ?: throw CodexLoginFailedException("Invalid sign-in reply")),
                "redirect_uri" to "${endpoints.auth}/deviceauth/callback",
                "client_id" to CLIENT_ID,
                "code_verifier" to (reply.string("code_verifier") ?: throw CodexLoginFailedException("Invalid sign-in reply")),
            ),
        )
        // The approved code works once, so a failed exchange can't be retried.
        if (exchange != 200) throw CodexLoginFailedException("Token server said $exchange")
        return try {
            parseTokens(tokens, null)
        } catch (error: IOException) {
            throw CodexLoginFailedException(error.message.orEmpty())
        }
    }
}

/**
 * Sends one polish request straight to Codex with this phone's login. A 401 renews
 * the login and retries once, inside the same deadline. Anything else fails, so the
 * pipeline keeps the local text; statuses decide that, never the words in an error.
 */
class CodexPolisher(
    private val session: CodexSession,
    private val endpoints: CodexEndpoints = CodexEndpoints(),
    private val now: () -> Long = System::currentTimeMillis,
) : Polisher {
    private sealed interface Reply {
        data class Text(val text: String) : Reply
        data object Unauthorized : Reply
    }

    override fun polish(prompt: String, model: String, effort: String?, deadlineMs: Long, cancel: CancelToken): String {
        val deadline = now() + deadlineMs
        val body = requestBody(prompt, model, effort)
        var tokens = session.fresh(remaining(deadline, cancel), cancel = cancel)
        var reply = request(body, tokens, remaining(deadline, cancel), cancel)
        if (reply is Reply.Unauthorized) {
            tokens = session.fresh(remaining(deadline, cancel), rejected = tokens.access, cancel = cancel)
            reply = request(body, tokens, remaining(deadline, cancel), cancel)
        }
        if (reply !is Reply.Text) throw CodexSignedOutException()
        if (now() >= deadline) throw GenerationException("Polish timed out")
        return reply.text
    }

    private fun remaining(deadline: Long, cancel: CancelToken): Long {
        if (cancel.cancelled) throw CancelledException()
        val left = deadline - now()
        if (left <= 0) throw GenerationException("Polish timed out")
        return left
    }

    private fun request(body: String, tokens: CodexTokens, remainingMs: Long, cancel: CancelToken): Reply {
        val connection = (URL("${endpoints.api}/responses").openConnection() as HttpURLConnection).apply {
            requestMethod = "POST"
            doOutput = true
            connectTimeout = remainingMs.coerceIn(1, 3000).toInt()
            readTimeout = remainingMs.coerceAtLeast(1).toInt()
            setRequestProperty("Content-Type", "application/json")
            setRequestProperty("Accept", "text/event-stream")
            setRequestProperty("Authorization", "Bearer ${tokens.access}")
            setRequestProperty("chatgpt-account-id", tokens.accountId)
            setRequestProperty("originator", "hex_android")
            setRequestProperty("User-Agent", USER_AGENT)
        }
        cancel.onCancel { connection.disconnect() }
        // Socket timeouts bound each read, not the whole reply: this bounds the whole request.
        val timedOut = java.util.concurrent.atomic.AtomicBoolean(false)
        val watchdog = Timer("hex-polish-deadline", true).apply {
            schedule(timerTask { timedOut.set(true); connection.disconnect() }, remainingMs.coerceAtLeast(1))
        }
        try {
            connection.outputStream.use { it.write(body.toByteArray()) }
            return when (val code = connection.responseCode) {
                200 -> Reply.Text(connection.inputStream.bufferedReader().use { readEvents(it.lineSequence()) })
                401 -> Reply.Unauthorized
                429 -> throw GenerationException("ChatGPT usage or rate limit reached")
                else -> throw GenerationException("Polish failed: $code")
            }
        } catch (_: SocketTimeoutException) {
            throw GenerationException("Polish timed out")
        } catch (_: IOException) {
            if (cancel.cancelled) throw CancelledException()
            throw GenerationException(if (timedOut.get()) "Polish timed out" else "Polish could not reach ChatGPT")
        } finally {
            watchdog.cancel()
            connection.disconnect()
        }
    }

    companion object {
        fun requestBody(prompt: String, model: String, effort: String?): String = buildJsonObject {
            put("model", model)
            put("store", false)
            put("stream", true)
            effort?.let { putJsonObject("reasoning") { put("effort", it) } }
            put(
                "input",
                buildJsonArray {
                    add(
                        buildJsonObject {
                            put("role", "user")
                            put(
                                "content",
                                buildJsonArray {
                                    add(
                                        buildJsonObject {
                                            put("type", "input_text")
                                            put("text", prompt)
                                        },
                                    )
                                },
                            )
                        },
                    )
                },
            )
        }.toString()

        /**
         * The finished answer, only once the server says the turn ended. Commentary and
         * partial answers are other messages in the same reply, and are never text to paste.
         */
        fun readEvents(lines: Sequence<String>): String {
            val answer = StringBuilder()
            for (line in lines) {
                if (!line.startsWith("data:")) continue
                val event = runCatching { Json.parseToJsonElement(line.substring(5).trim()).jsonObject }.getOrNull() ?: continue
                when (val type = event.string("type")) {
                    "response.output_item.done" -> (event["item"] as? JsonObject)?.let { answer.append(finalText(it)) }
                    "response.completed" -> {
                        val endTurn = ((event["response"] as? JsonObject)?.get("end_turn") as? JsonPrimitive)?.booleanOrNull
                        if (endTurn == false) throw GenerationException("Polish reply ended early")
                        if (answer.isEmpty()) throw GenerationException("Polish returned no answer")
                        return answer.toString()
                    }
                    "response.failed", "response.incomplete", "error" -> throw GenerationException("Polish failed: $type")
                }
            }
            throw GenerationException("Polish reply ended early")
        }

        private fun finalText(item: JsonObject): String {
            val final = item.string("type") == "message" &&
                item.string("status").let { it == null || it == "completed" } &&
                item.string("phase").let { it == null || it == "final_answer" }
            if (!final) return ""
            return (item["content"] as? JsonArray).orEmpty().mapNotNull { it as? JsonObject }
                .filter { it.string("type") == "output_text" }
                .joinToString("") { it.string("text").orEmpty() }
        }
    }
}
