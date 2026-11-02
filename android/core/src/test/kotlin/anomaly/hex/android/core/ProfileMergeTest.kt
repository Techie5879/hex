package anomaly.hex.android.core

import anomaly.hex.android.core.profile.PortableProfile
import anomaly.hex.android.core.profile.ProfileEditor
import anomaly.hex.android.core.profile.ProfileException
import anomaly.hex.android.core.profile.ProfileKeys
import anomaly.hex.android.core.profile.ProfileMerge
import anomaly.hex.android.core.profile.StampClock
import anomaly.hex.android.core.profile.termKey
import java.io.File
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertThrows
import org.junit.Assert.assertTrue
import org.junit.Test

class ProfileMergeTest {
    private val fixtures = generateSequence(File("").absoluteFile) { it.parentFile }
        .map { File(it, "tests/fixtures/profile-sync") }.first { it.isDirectory }

    @Test
    fun sharedVectors() {
        val cases = Json.parseToJsonElement(File(fixtures, "vectors.json").readText()).jsonObject.getValue("cases").jsonArray
        assertTrue(cases.size >= 10)
        for (case in cases) {
            val c = case.jsonObject
            val p = { key: String -> PortableProfile.decode(c.getValue(key).toString()) }
            val name = c.getValue("name").jsonPrimitive.content
            val expected = p("expected").encode()
            assertEquals(name, expected, ProfileMerge.merge(p("local"), p("remote"), p("ancestor")).encode())
            // Either device must reach the same document.
            assertEquals("$name (swapped)", expected, ProfileMerge.merge(p("remote"), p("local"), p("ancestor")).encode())
        }
    }

    @Test
    fun sharedTermKeys() {
        val terms = Json.parseToJsonElement(File(fixtures, "term_keys.json").readText()).jsonObject.getValue("terms").jsonArray
        for (pair in terms) {
            val (raw, key) = pair.jsonArray.map { it.jsonPrimitive.content }
            assertEquals(key, termKey(raw))
        }
    }

    @Test
    fun rejectsNewerVersionsWithoutTouchingAnything() {
        val e = assertThrows(ProfileException::class.java) {
            PortableProfile.decode("""{"schema":"hex.profile","version":2,"vocabulary":{}}""")
        }
        assertEquals(ProfileException.Kind.TooNew, e.kind)
        assertThrows(ProfileException::class.java) { PortableProfile.decode("""{"schema":"other","version":1}""") }
        assertThrows(ProfileException::class.java) { PortableProfile.decode("not json") }
    }

    @Test
    fun editsAreStampedTombstonedAndValidated() {
        var now = 1000L
        val editor = ProfileEditor(StampClock("phone") { now })
        var p = editor.putTerm(PortableProfile.empty(), "  Zebra Quartz ", listOf("zebra quarts", ""))
        assertEquals("Zebra Quartz", p.liveTerms.single().term)
        assertEquals(listOf("zebra quarts"), p.liveTerms.single().soundsLike)
        val firstStamp = p.liveTerms.single().stamp
        // The same edit changes nothing, so an idle sync uploads nothing.
        assertEquals(p, editor.putTerm(p, "Zebra Quartz", listOf("zebra quarts")))
        now = 500 // A clock running backwards must still produce a later stamp.
        p = editor.removeTerm(p, "zebra quartz")
        assertTrue(p.liveTerms.isEmpty())
        assertTrue(p.vocabulary.getValue("zebra quartz").stamp > firstStamp)
        p = editor.putTerm(p, "ZEBRA QUARTZ")
        assertEquals("ZEBRA QUARTZ", p.liveTerms.single().term)
        assertThrows(ProfileException::class.java) { editor.putTerm(p, "x".repeat(65)) }
        assertThrows(ProfileException::class.java) { editor.putTerm(p, "   ") }
    }

    @Test
    fun dictionaryIsBoundedButReplacingAnExistingTermIsAllowed() {
        val editor = ProfileEditor(StampClock("phone") { 1000L })
        var p = PortableProfile.empty()
        repeat(500) { p = editor.putTerm(p, "term $it") }
        assertThrows(ProfileException::class.java) { editor.putTerm(p, "one more") }
        p = editor.putTerm(p, "TERM 7")
        assertEquals(500, p.liveTerms.size)
    }

    @Test
    fun resolvingAConflictKeepsTheChosenValueAndSyncsAsResolved() {
        val editor = ProfileEditor(StampClock("phone") { 5000L })
        val key = ProfileKeys.POLISH_PROMPT
        val ancestor = editor.putSetting(PortableProfile.empty(), key, JsonPrimitive("old"))
        val local = editor.putSetting(ancestor, key, JsonPrimitive("phone"))
        val remote = ProfileEditor(StampClock("mac") { 6000L }).putSetting(ancestor, key, JsonPrimitive("mac"))
        val merged = ProfileMerge.merge(local, remote, ancestor)
        assertEquals(JsonPrimitive("mac"), merged.settings.getValue(key).value)
        val conflict = merged.openConflicts.single()
        assertEquals(JsonPrimitive("phone"), conflict.value)

        val resolved = editor.resolve(merged, conflict.id, useConflictValue = true)
        assertEquals(JsonPrimitive("phone"), resolved.settings.getValue(key).value)
        assertTrue(resolved.openConflicts.isEmpty())
        // A device that still shows it open learns it was resolved, and a third merge stays settled.
        val elsewhere = ProfileMerge.merge(merged, resolved, merged)
        assertTrue(elsewhere.openConflicts.isEmpty())
        assertEquals(JsonPrimitive("phone"), elsewhere.settings.getValue(key).value)
        assertFalse(ProfileMerge.merge(elsewhere, resolved, elsewhere).encode() != resolved.encode())
    }

    @Test
    fun encodingIsCanonical() {
        val text = """{"version":1,"settings":{"b":{"stamp":"2","value":1},"a":{"value":2,"stamp":"1"}},"schema":"hex.profile"}"""
        val encoded = PortableProfile.decode(text).encode()
        assertEquals(encoded, PortableProfile.decode(encoded).encode())
        assertTrue(encoded.indexOf("\"a\"") < encoded.indexOf("\"b\""))
    }
}
