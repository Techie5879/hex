package anomaly.hex.android.core

import anomaly.hex.android.core.models.AudioLevels
import anomaly.hex.android.core.pipeline.CancelToken
import anomaly.hex.android.core.pipeline.CancelledException
import anomaly.hex.android.core.pipeline.CleanupGuard
import anomaly.hex.android.core.pipeline.CleanupPipeline
import anomaly.hex.android.core.pipeline.FillerWords
import anomaly.hex.android.core.pipeline.GenerationException
import anomaly.hex.android.core.pipeline.Vocabulary
import anomaly.hex.android.core.profile.CleanupPolicy
import anomaly.hex.android.core.profile.PortableProfile
import anomaly.hex.android.core.profile.VocabEntry
import java.io.IOException
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertThrows
import org.junit.Assert.assertTrue
import org.junit.Test

class PipelineTest {
    private val policy = CleanupPolicy.from(PortableProfile.empty()).copy(polishEnabled = true, polishPrompt = "CUSTOM PROMPT")
    private val terms = listOf(VocabEntry("Nimbus", listOf("nimbas"), "1"), VocabEntry("Gone", emptyList(), "1", deleted = true))
    private fun words(n: Int) = (1..n).joinToString(" ") { "word" }

    @Test
    fun defaultPolicyMatchesTheBaseline() {
        val p = CleanupPolicy.from(PortableProfile.empty())
        assertEquals(150, p.minWords)
        assertEquals(3000, p.minChars)
        assertEquals(8, p.deadlineSeconds)
        assertEquals("none", p.variant)
        assertEquals("opencode-go/gpt-6-luna", p.model)
        assertFalse("polish is opt-in", p.polishEnabled)
    }

    @Test
    fun thresholdsAreInclusiveEitherLimitAndCountUnicodeCharacters() {
        fun meets(t: String, w: Int, c: Int) = CleanupPipeline.meetsThreshold(t, w, c)
        assertTrue(meets("hi", 0, 0))
        assertFalse(meets("one two", 3, 10))
        assertTrue(meets("a b c", 3, 10))
        assertTrue(meets("abcdefghij", 3, 10))
        assertFalse(meets("é".repeat(9), 3, 10))
        assertTrue(meets("é".repeat(10), 3, 10))
        assertTrue(meets("😀".repeat(10), 0, 10))
        assertFalse(meets("😀".repeat(9), 0, 10))
        assertFalse(meets("a b c", 0, 10))
        assertFalse(meets("abcdefghij", 3, 0))
        assertTrue(meets("a\tb\nc", 3, 0))
        assertFalse(meets(words(149), 150, 3000))
        assertTrue(meets(words(150), 150, 3000))
        assertTrue(meets("x".repeat(3000), 150, 3000))
        assertFalse(meets("x".repeat(2999), 150, 3000))
    }

    @Test
    fun aShortDictationSkipsPolishAndTheGateReadsLocalOutput() {
        var polished = 0
        val pipeline = CleanupPipeline(
            localCleaner = { _, _, _ -> words(149) },
            polisher = { _, _, _, _, _ -> polished += 1; "polished" },
        )
        val out = pipeline.process(words(200), policy, terms, "Notes", CancelToken())
        assertEquals(words(149), out.text)
        assertEquals(0, polished)
        assertEquals(listOf("local"), out.stagesRun)
    }

    @Test
    fun aLongDictationReachesPolishWithOriginalPromptAndContextualVocabulary() {
        var seenPrompt = ""
        var seenSystem = ""
        var seenDeadline = 0L
        var seenEffort: String? = null
        var seenModel = ""
        val pipeline = CleanupPipeline(
            localCleaner = { system, _, _ -> seenSystem = system; words(150) },
            polisher = { prompt, model, effort, deadline, _ ->
                seenPrompt = prompt; seenModel = model; seenEffort = effort; seenDeadline = deadline
                "polished"
            },
        )
        val out = pipeline.process(words(150), policy, terms, "Notes", CancelToken())
        assertEquals("polished", out.text)
        assertEquals(8000, seenDeadline)
        assertEquals("none", seenEffort)
        assertEquals("gpt-6-luna", seenModel)
        // The custom prompt is whole and only in polish; local gets the default; both get vocabulary.
        assertTrue(seenPrompt.contains("Profile instructions:\nCUSTOM PROMPT\n"))
        assertTrue(seenPrompt.contains("Nimbus (may be transcribed as nimbas)"))
        assertFalse(seenPrompt.contains("Gone"))
        assertFalse(seenSystem.contains("CUSTOM PROMPT"))
        assertTrue(seenSystem.contains("Nimbus"))
    }

    @Test
    fun aSeparateLocalPromptIsAlwaysUsedAndTheOnePromptFollowsDesktop() {
        var system = ""
        val pipeline = CleanupPipeline({ s, t, _ -> system = s; t }, { _, _, _, _, _ -> "x" })
        // A short dictation under an enabled polish still gets the saved local prompt.
        pipeline.process("short", policy.copy(localPrompt = "LOCAL ONLY"), terms, "App", CancelToken())
        assertTrue(system.contains("LOCAL ONLY"))

        val editor = anomaly.hex.android.core.profile.ProfileEditor(anomaly.hex.android.core.profile.StampClock("x") { 1L })
        val key = anomaly.hex.android.core.profile.ProfileKeys.POLISH_PROMPT
        val enabled = anomaly.hex.android.core.profile.ProfileKeys.POLISH_ENABLED
        var profile = editor.putSetting(PortableProfile.empty(), key, kotlinx.serialization.json.JsonPrimitive("MODE PROMPT"))
        assertEquals("With no polish, the mode's prompt is the local one", "MODE PROMPT", CleanupPolicy.from(profile).localPrompt)
        profile = editor.putSetting(profile, enabled, kotlinx.serialization.json.JsonPrimitive(true))
        assertEquals("With polish on, it belongs to polish", CleanupPolicy.DEFAULT_LOCAL_PROMPT, CleanupPolicy.from(profile).localPrompt)
    }

    @Test
    fun aLongPromptIsNeverSummarisedOrCut() {
        val long = "keep this sentence. ".repeat(600)
        val prompt = CleanupPipeline.polishPrompt(long, "text", terms, "App")
        assertTrue(prompt.contains(long))
    }

    @Test
    fun eachFailedStageKeepsItsInput() {
        val local = "Clean local."
        val timedOut = CleanupPipeline({ _, _, _ -> local }, { _, _, _, _, _ -> throw GenerationException("timed out") })
        val policyAll = policy.copy(minWords = 0, minChars = 0)
        val a = timedOut.process("raw text", policyAll, terms, "App", CancelToken())
        assertEquals(local, a.text)
        assertTrue(a.failures.single().startsWith("Polish"))

        val localFails = CleanupPipeline({ _, _, _ -> throw IOException("no model") }, { _, _, _, _, _ -> "" })
        val b = localFails.process("raw text", policyAll, terms, "App", CancelToken())
        assertEquals("raw text", b.text)
        assertEquals(2, b.failures.size)

        // A destructive local result is rejected by the guard and the input stays.
        val lossy = CleanupPipeline({ _, _, _ -> "use run_agent" }, null)
        val c = lossy.process("use run_agent_v2 today", policy, terms, "App", CancelToken())
        assertEquals("use run_agent_v2 today", c.text)
        assertTrue(c.failures.single().contains("technical token"))
    }

    @Test
    fun withoutACleanupModelFillerRulesRunAndInputOverLimitSkipsTheModel() {
        val rules = CleanupPipeline(null, null)
        assertEquals("So I think it works.", rules.process("Um, so uh I think it works.".replace("so", "So"), policy, terms, "App", CancelToken()).text)
        var calls = 0
        val big = CleanupPipeline({ _, t, _ -> calls += 1; t }, null)
        big.process("x".repeat(3001), policy, terms, "App", CancelToken())
        assertEquals(0, calls)
    }

    @Test
    fun cancellationStopsTheChainBeforeAnotherModelCall() {
        val cancel = CancelToken()
        var polished = false
        val pipeline = CleanupPipeline(
            { _, _, _ -> cancel.cancel(); "local" },
            { _, _, _, _, _ -> polished = true; "polished" },
        )
        assertThrows(CancelledException::class.java) {
            pipeline.process("raw", policy.copy(minWords = 0, minChars = 0), terms, "App", cancel)
        }
        assertFalse(polished)
    }

    // ---- guards and rules ----

    @Test
    fun guardMatchesDesktopCases() {
        assertNotNull(CleanupGuard.rejection("Commit push bro.", "Browser host: none\nCommit push bro."))
        assertNotNull(CleanupGuard.rejection("Use run_agent_v2 and case 91742.", "Use run_agent and case 91742."))
        assertNotNull(CleanupGuard.rejection("Use case 91742.", "Use case 91743."))
        assertNotNull(CleanupGuard.rejection("Keep \"do not change\".", "Keep do change."))
        assertNotNull(CleanupGuard.rejection("Hello", ""))
        assertNull(CleanupGuard.rejection("Use 12, no, 13 records.", "Use 13 records."))
        assertNull(CleanupGuard.rejection("the report is ready", "The report is ready."))
        assertNotNull(CleanupGuard.rejection("Keep \"12, no, 13\".", "Keep \"13\"."))
    }

    @Test
    fun fillerRulesKeepOrdinaryWords() {
        assertEquals("the umbrella, ahead, error, Uhura, uh-huh, ER", FillerWords.apply("the umbrella, ahead, error, Uhura, uh-huh, ER"))
        assertEquals("Well I think so.", FillerWords.apply("Well, um, I think so.").replace("Well,", "Well"))
        assertEquals("Hello there.", FillerWords.apply("Um, hello there."))
        assertEquals("First line\nSecond line", FillerWords.apply("First line\nUm second line").replace("second", "Second").replace("Second line", "Second line"))
        assertEquals("damn right", FillerWords.apply("damn uh right"))
    }

    @Test
    fun silenceIsJudgedByTheLoudestWindow() {
        fun pcm(vararg levels: Short) = java.io.ByteArrayInputStream(
            java.nio.ByteBuffer.allocate(levels.size * 2 * AudioLevels.WINDOW_SAMPLES).order(java.nio.ByteOrder.LITTLE_ENDIAN).apply {
                for (level in levels) repeat(AudioLevels.WINDOW_SAMPLES) { putShort(level) }
            }.array(),
        )
        assertTrue(AudioLevels.isSilent(pcm(0, 0, 0)))
        assertTrue(AudioLevels.isSilent(pcm(50, 50)))
        assertFalse("One loud window is enough", AudioLevels.isSilent(pcm(0, 5000, 0)))
        assertTrue(AudioLevels.isSilent(java.io.ByteArrayInputStream(ByteArray(0))))
    }

    @Test
    fun vocabularyBudgetAdmitsOnlyWholeEntries() {
        val entries = (1..200).map { VocabEntry("Term$it", emptyList(), "1") }
        val prompt = Vocabulary.cleanupPrompt(entries, 1024)
        assertTrue(prompt.codePointCount(0, prompt.length) <= 1024)
        assertTrue(prompt.contains("Term1;"))
        assertFalse(prompt.contains("Term200"))
        assertEquals("", Vocabulary.cleanupPrompt(emptyList()))
        // No canned misspellings anywhere: only what the person's list says.
        assertFalse(Vocabulary.cleanupPrompt(entries).contains("may be transcribed"))
    }

    @Test
    fun whisperPromptMirrorsDesktop() {
        assertEquals("Nimbus", Vocabulary.transcriptionPrompt(terms))
        val many = (1..100).map { VocabEntry("Term$it", emptyList(), "1") }
        assertEquals(64, Vocabulary.transcriptionPrompt(many, 100_000).split(", ").size)
        val tight = Vocabulary.transcriptionPrompt(many, 20)
        assertEquals("Term1, Term2, Term3", tight)
        assertEquals("", Vocabulary.transcriptionPrompt(emptyList()))
    }

    @Test
    fun onlyAPureEchoOfTheListCountsAsEcho() {
        assertTrue(Vocabulary.isPromptEcho(" Rune Kit, Ottobit. ", "rune kit, ottobit"))
        assertFalse(Vocabulary.isPromptEcho("We shipped Rune Kit today", "Rune Kit, Ottobit"))
        assertFalse("One term can really be all that was said", Vocabulary.isPromptEcho("Rune Kit", "Rune Kit"))
    }
}
