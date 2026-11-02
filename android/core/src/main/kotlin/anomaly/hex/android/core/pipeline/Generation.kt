package anomaly.hex.android.core.pipeline

import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.put
import kotlinx.serialization.json.putJsonObject

/** A cancellation flag checked between steps and by long-running work. */
class CancelToken {
    @Volatile var cancelled: Boolean = false
        private set
    private val listeners = mutableListOf<() -> Unit>()

    fun cancel() {
        val toRun = synchronized(listeners) {
            cancelled = true
            listeners.toList().also { listeners.clear() }
        }
        toRun.forEach { it() }
    }

    fun onCancel(listener: () -> Unit) {
        val runNow = synchronized(listeners) {
            if (cancelled) true else { listeners += listener; false }
        }
        if (runNow) listener()
    }
}

class CancelledException : Exception("Cancelled")

data class ModelRef(val provider: String, val id: String, val variant: String?) {
    companion object {
        /** `provider/model`, as saved in desktop settings. */
        fun parse(key: String, variant: String?): ModelRef? {
            val slash = key.indexOf('/')
            if (slash <= 0 || slash == key.lastIndex) return null
            return ModelRef(key.substring(0, slash), key.substring(slash + 1), variant?.takeIf { it.isNotBlank() })
        }
    }
}

/** A provider's reply, parsed the way the desktop parses OpenCode's envelope. */
sealed interface GenerationReply {
    data class Success(val text: String) : GenerationReply
    data class Failure(val tag: String, val message: String, val provider: String?, val reason: String?) : GenerationReply {
        fun quotaLimited(model: ModelRef) =
            tag == "ProviderRequestError" && reason == "rate_limit" && provider == model.provider
    }

    companion object {
        fun parse(body: String): GenerationReply {
            val root = runCatching { Json.parseToJsonElement(body).jsonObject }
                .getOrElse { throw GenerationException("Invalid generation response") }
            val text = (root["data"] as? JsonObject)?.get("text")?.let { it as? JsonPrimitive }?.takeIf { it.isString }?.content
            if (text != null) return Success(text)
            return Failure(
                tag = (root["_tag"] as? JsonPrimitive)?.contentOrNull.orEmpty(),
                message = (root["message"] as? JsonPrimitive)?.contentOrNull.orEmpty(),
                provider = (root["providerID"] as? JsonPrimitive)?.contentOrNull,
                reason = (root["reason"] as? JsonPrimitive)?.contentOrNull,
            )
        }
    }
}

/** Transport or deadline failure. Never a reason to try another provider. */
class GenerationException(message: String) : Exception(message)

fun interface GenerationTransport {
    /** POST [body] with at most [remainingMs]; returns the response body or throws. */
    fun post(body: String, remainingMs: Long, cancel: CancelToken): String
}

/**
 * Same-model provider fallback, only on OpenCode's typed quota rejection.
 * One alternate request, sharing the original deadline and cancellation.
 * Messages are never inspected; timeouts, network errors, authentication, and
 * untyped errors all fail so the caller keeps the previous stage's text.
 */
class QuotaFallbackGenerator(
    private val transport: GenerationTransport,
    private val now: () -> Long = System::currentTimeMillis,
) {
    fun generate(prompt: String, model: ModelRef, fallbackProvider: String?, deadlineMs: Long, cancel: CancelToken): String {
        val deadline = now() + deadlineMs
        when (val first = request(prompt, model, deadline, cancel)) {
            is GenerationReply.Success -> return first.text
            is GenerationReply.Failure -> {
                val fallback = fallbackProvider?.trim().orEmpty()
                if (!first.quotaLimited(model) || fallback.isEmpty() || fallback == model.provider) {
                    throw GenerationException("Polish failed: ${first.tag.ifEmpty { "error" }}")
                }
                when (val second = request(prompt, model.copy(provider = fallback), deadline, cancel)) {
                    is GenerationReply.Success -> return second.text
                    is GenerationReply.Failure -> throw GenerationException("Polish fallback $fallback failed: ${second.tag.ifEmpty { "error" }}")
                }
            }
        }
    }

    private fun request(prompt: String, model: ModelRef, deadline: Long, cancel: CancelToken): GenerationReply {
        if (cancel.cancelled) throw CancelledException()
        val remaining = deadline - now()
        if (remaining <= 0) throw GenerationException("Polish deadline passed")
        val body = transport.post(requestBody(prompt, model), remaining, cancel)
        if (now() >= deadline) throw GenerationException("Polish deadline passed")
        return GenerationReply.parse(body)
    }

    companion object {
        fun requestBody(prompt: String, model: ModelRef): String = buildJsonObject {
            put("prompt", prompt)
            putJsonObject("model") {
                put("providerID", model.provider)
                put("id", model.id)
                model.variant?.let { put("variant", it) }
            }
        }.toString()
    }
}
