package anomaly.hex.android.core.profile

import java.text.Normalizer
import java.util.Locale
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonNull
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.boolean
import kotlinx.serialization.json.booleanOrNull
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive

/**
 * The portable, versioned profile shared between Hex on Android and desktop:
 * dictionary, cleanup prompts, and cleanup policy. Nothing else belongs here:
 * no credentials, audio, history, or device settings. The merge rules are
 * mirrored by `src/profile_sync` in the desktop crate, and both run the vectors
 * in `tests/fixtures/profile-sync`.
 */
const val PROFILE_SCHEMA = "hex.profile"
const val PROFILE_VERSION = 1
const val MAX_PROFILE_BYTES = 1024 * 1024
const val MAX_LIVE_TERMS = 500
const val MAX_TERM_CHARS = 64
const val MAX_SOUNDS_LIKE = 16
const val MAX_PROMPT_CHARS = 64 * 1024

class ProfileException(message: String, val kind: Kind = Kind.Invalid) : Exception(message) {
    enum class Kind { Invalid, TooNew, TooLarge }
}

data class VocabEntry(
    val term: String,
    val soundsLike: List<String> = emptyList(),
    val stamp: String,
    val deleted: Boolean = false,
    val extra: JsonObject = EMPTY,
) {
    val isLive get() = !deleted
}

data class Register(
    val value: JsonElement,
    val stamp: String,
    val extra: JsonObject = EMPTY,
)

/** The losing side of two concurrent edits, kept until someone chooses. */
data class Conflict(
    val key: String,
    val value: JsonElement,
    val stamp: String,
    val winnerStamp: String,
    val resolvedStamp: String? = null,
    val extra: JsonObject = EMPTY,
) {
    val id get() = "$key#$stamp"
    val isOpen get() = resolvedStamp == null
}

data class PortableProfile(
    val vocabulary: Map<String, VocabEntry> = emptyMap(),
    val settings: Map<String, Register> = emptyMap(),
    val conflicts: Map<String, Conflict> = emptyMap(),
    val extra: JsonObject = EMPTY,
) {
    val liveTerms: List<VocabEntry>
        get() = vocabulary.values.filter { it.isLive }.sortedBy { it.term.lowercase(Locale.ROOT) }
    val openConflicts: List<Conflict>
        get() = conflicts.values.filter { it.isOpen }.sortedBy { it.id }

    fun encode(): String = JSON.encodeToString(JsonElement.serializer(), sorted(toJson()))

    private fun toJson() = JsonObject(
        extra + mapOf(
            "schema" to JsonPrimitive(PROFILE_SCHEMA),
            "version" to JsonPrimitive(PROFILE_VERSION),
            "vocabulary" to JsonObject(vocabulary.mapValues { (_, e) ->
                JsonObject(
                    e.extra + buildMap {
                        put("term", JsonPrimitive(e.term))
                        if (e.soundsLike.isNotEmpty()) {
                            put("sounds_like", JsonArray(e.soundsLike.map(::JsonPrimitive)))
                        }
                        put("stamp", JsonPrimitive(e.stamp))
                        if (e.deleted) put("deleted", JsonPrimitive(true))
                    },
                )
            }),
            "settings" to JsonObject(settings.mapValues { (_, r) ->
                JsonObject(r.extra + mapOf("value" to r.value, "stamp" to JsonPrimitive(r.stamp)))
            }),
            "conflicts" to JsonObject(conflicts.mapValues { (_, c) ->
                JsonObject(
                    c.extra + buildMap {
                        put("key", JsonPrimitive(c.key))
                        put("value", c.value)
                        put("stamp", JsonPrimitive(c.stamp))
                        put("winner_stamp", JsonPrimitive(c.winnerStamp))
                        c.resolvedStamp?.let { put("resolved_stamp", JsonPrimitive(it)) }
                    },
                )
            }),
        ),
    )

    fun validate() {
        val live = vocabulary.values.count { it.isLive }
        if (live > MAX_LIVE_TERMS) throw ProfileException("Too many dictionary terms: $live")
        for ((key, entry) in vocabulary) {
            if (key != termKey(entry.term)) throw ProfileException("Dictionary key does not match its term")
            if (entry.term.isBlank() || entry.term.codePointCount(0, entry.term.length) > MAX_TERM_CHARS) {
                throw ProfileException("Dictionary term has an invalid length")
            }
            if (entry.soundsLike.size > MAX_SOUNDS_LIKE) throw ProfileException("Too many alternative spellings")
        }
        for ((key, register) in settings) {
            val text = (register.value as? JsonPrimitive)?.takeIf { it.isString }?.content ?: continue
            if (text.length > MAX_PROMPT_CHARS) throw ProfileException("Setting $key is too long")
        }
        if (encode().toByteArray().size > MAX_PROFILE_BYTES) {
            throw ProfileException("Profile is larger than 1 MiB", ProfileException.Kind.TooLarge)
        }
    }

    companion object {
        private val EMPTY_PROFILE = PortableProfile()
        fun empty() = EMPTY_PROFILE

        fun decode(text: String): PortableProfile {
            if (text.toByteArray().size > MAX_PROFILE_BYTES) {
                throw ProfileException("Profile is larger than 1 MiB", ProfileException.Kind.TooLarge)
            }
            val root = runCatching { JSON.parseToJsonElement(text).jsonObject }
                .getOrElse { throw ProfileException("Profile is not JSON: ${it.message}") }
            if ((root["schema"] as? JsonPrimitive)?.content != PROFILE_SCHEMA) {
                throw ProfileException("Not a Hex profile")
            }
            val version = (root["version"] as? JsonPrimitive)?.content?.toIntOrNull()
                ?: throw ProfileException("Profile has no version")
            if (version > PROFILE_VERSION) {
                throw ProfileException("Profile version $version needs a newer Hex", ProfileException.Kind.TooNew)
            }
            return runCatching {
                PortableProfile(
                    vocabulary = root["vocabulary"]?.jsonObject.orEmpty().mapValues { (_, v) -> entry(v.jsonObject) },
                    settings = root["settings"]?.jsonObject.orEmpty().mapValues { (_, v) -> register(v.jsonObject) },
                    conflicts = root["conflicts"]?.jsonObject.orEmpty().mapValues { (_, v) -> conflict(v.jsonObject) },
                    extra = JsonObject(root - TOP_LEVEL),
                )
            }.getOrElse {
                if (it is ProfileException) throw it
                throw ProfileException("Profile is malformed: ${it.message}")
            }.also(PortableProfile::validate)
        }

        private val TOP_LEVEL = setOf("schema", "version", "vocabulary", "settings", "conflicts")

        private fun entry(o: JsonObject) = VocabEntry(
            term = o.getValue("term").jsonPrimitive.content,
            soundsLike = o["sounds_like"]?.jsonArray?.map { it.jsonPrimitive.content }.orEmpty(),
            stamp = o.getValue("stamp").jsonPrimitive.content,
            deleted = o["deleted"]?.jsonPrimitive?.booleanOrNull ?: false,
            extra = JsonObject(o - setOf("term", "sounds_like", "stamp", "deleted")),
        )

        private fun register(o: JsonObject) = Register(
            value = o["value"] ?: JsonNull,
            stamp = o.getValue("stamp").jsonPrimitive.content,
            extra = JsonObject(o - setOf("value", "stamp")),
        )

        private fun conflict(o: JsonObject) = Conflict(
            key = o.getValue("key").jsonPrimitive.content,
            value = o["value"] ?: JsonNull,
            stamp = o.getValue("stamp").jsonPrimitive.content,
            winnerStamp = o.getValue("winner_stamp").jsonPrimitive.content,
            resolvedStamp = o["resolved_stamp"]?.jsonPrimitive?.contentOrNull,
            extra = JsonObject(o - setOf("key", "value", "stamp", "winner_stamp", "resolved_stamp")),
        )
    }
}

private val EMPTY = JsonObject(emptyMap())
private val JSON = Json { prettyPrint = false; encodeDefaults = true; explicitNulls = true }

private fun sorted(element: JsonElement): JsonElement = when (element) {
    is JsonObject -> JsonObject(element.toSortedMap().mapValues { (_, v) -> sorted(v) })
    is JsonArray -> JsonArray(element.map(::sorted))
    else -> element
}

/** A term's identity: Unicode-normalised, case-folded, and space-collapsed. */
fun termKey(term: String): String =
    Normalizer.normalize(term, Normalizer.Form.NFC)
        .trim()
        .split(Regex("\\s+"))
        .joinToString(" ")
        .lowercase(Locale.ROOT)

/**
 * A hybrid logical clock: lexically ordered `millis-counter-device`. Local edits
 * always stamp later than anything this device has seen, so skew between devices
 * can't make a person's newest edit lose to an older one they already merged.
 */
class StampClock(private val device: String, private val now: () -> Long = System::currentTimeMillis) {
    private var last: String? = null

    init {
        require(device.isNotBlank() && '-' !in device) { "device id must not contain '-'" }
    }

    @Synchronized
    fun next(profile: PortableProfile = PortableProfile.empty()): String {
        val highest = (profile.vocabulary.values.map { it.stamp } + profile.settings.values.map { it.stamp } + listOfNotNull(last))
            .maxOrNull()
        val (seenMs, seenCounter) = highest?.let(::parse) ?: (0L to 0)
        val ms = maxOf(now(), seenMs)
        val counter = if (ms == seenMs) seenCounter + 1 else 0
        return format(ms, counter).also { last = it }
    }

    private fun parse(stamp: String): Pair<Long, Int> {
        val parts = stamp.split('-', limit = 3)
        return (parts.getOrNull(0)?.toLongOrNull() ?: 0L) to (parts.getOrNull(1)?.toIntOrNull() ?: 0)
    }

    private fun format(ms: Long, counter: Int) =
        ms.toString().padStart(13, '0') + "-" + counter.toString().padStart(4, '0') + "-" + device
}

/** Edits a profile the way a person's action does: stamped, validated, tombstoned. */
class ProfileEditor(private val clock: StampClock) {
    fun putTerm(profile: PortableProfile, term: String, soundsLike: List<String> = emptyList()): PortableProfile {
        val clean = term.trim()
        if (clean.isEmpty()) throw ProfileException("A term can't be empty")
        if (clean.codePointCount(0, clean.length) > MAX_TERM_CHARS) throw ProfileException("Terms are $MAX_TERM_CHARS characters at most")
        val key = termKey(clean)
        val spellings = soundsLike.map(String::trim).filter(String::isNotEmpty).distinct().take(MAX_SOUNDS_LIKE)
        val existing = profile.vocabulary[key]
        if (existing != null && existing.isLive && existing.term == clean && existing.soundsLike == spellings) return profile
        if ((existing == null || existing.deleted) && profile.vocabulary.values.count { it.isLive } >= MAX_LIVE_TERMS) {
            throw ProfileException("The dictionary holds $MAX_LIVE_TERMS terms at most")
        }
        val entry = VocabEntry(clean, spellings, clock.next(profile), extra = existing?.extra ?: EMPTY)
        return profile.copy(vocabulary = profile.vocabulary + (key to entry))
    }

    fun removeTerm(profile: PortableProfile, term: String): PortableProfile {
        val key = termKey(term)
        val existing = profile.vocabulary[key] ?: return profile
        if (existing.deleted) return profile
        return profile.copy(
            vocabulary = profile.vocabulary + (key to existing.copy(soundsLike = emptyList(), deleted = true, stamp = clock.next(profile))),
        )
    }

    fun putSetting(profile: PortableProfile, key: String, value: JsonElement): PortableProfile {
        val existing = profile.settings[key]
        if (existing?.value == value) return profile
        val register = Register(value, clock.next(profile), existing?.extra ?: EMPTY)
        return profile.copy(settings = profile.settings + (key to register))
    }

    /** Settle a conflict by keeping either the current value or the conflict's. */
    fun resolve(profile: PortableProfile, conflictId: String, useConflictValue: Boolean): PortableProfile {
        val conflict = profile.conflicts[conflictId] ?: return profile
        val base = if (useConflictValue) putSetting(profile, conflict.key, conflict.value) else profile
        val stamp = clock.next(base)
        return base.copy(conflicts = base.conflicts + (conflictId to conflict.copy(value = JsonNull, resolvedStamp = stamp)))
    }
}

/** A setting's value as JSON, or null if missing or a different type. */
fun Register?.stringValue(): String? = (this?.value as? JsonPrimitive)?.takeIf { it.isString }?.content
fun Register?.intValue(): Int? = (this?.value as? JsonPrimitive)?.content?.toIntOrNull()
fun Register?.boolValue(): Boolean? = (this?.value as? JsonPrimitive)?.let { runCatching { it.boolean }.getOrNull() }
