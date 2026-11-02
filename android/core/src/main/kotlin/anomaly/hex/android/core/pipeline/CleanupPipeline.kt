package anomaly.hex.android.core.pipeline

import anomaly.hex.android.core.profile.CleanupPolicy
import anomaly.hex.android.core.profile.VocabEntry

/** A local cleanup model. Returns text, or throws to keep the input. */
fun interface LocalCleaner {
    fun clean(system: String, transcript: String, cancel: CancelToken): String
}

fun interface Polisher {
    fun polish(prompt: String, model: String, effort: String?, deadlineMs: Long, cancel: CancelToken): String
}

enum class CleanupStage { LocalCleanup, Polish }

/** What a dictation became, and which steps ran or failed. */
data class ProcessedText(
    val raw: String,
    val text: String,
    val stagesRun: List<String>,
    val failures: List<String>,
) {
    val stageNames get() = stagesRun.joinToString(" + ")
    val failureNotes get() = failures.joinToString("; ")
}

/**
 * The desktop's order: local cleanup, then the size gate on its output, then
 * polish. A failed or empty step keeps its input. The gate is inclusive on
 * either positive threshold, and counts whitespace-separated words and Unicode
 * characters. Both zero means polish every enabled dictation.
 */
class CleanupPipeline(
    private val localCleaner: LocalCleaner?,
    private val polisher: Polisher?,
) {
    fun process(
        raw: String,
        policy: CleanupPolicy,
        vocabulary: List<VocabEntry>,
        appName: String,
        cancel: CancelToken,
        onStage: (CleanupStage) -> Unit = {},
    ): ProcessedText {
        val input = raw.trim()
        var text = input
        val stages = mutableListOf<String>()
        val failures = mutableListOf<String>()
        if (text.isEmpty()) return ProcessedText(raw, text, stages, failures)
        val willPolish = policy.polishEnabled && polisher != null

        checkCancelled(cancel)
        if (localCleaner != null && text.codePointCount(0, text.length) <= LOCAL_INPUT_LIMIT) {
            onStage(CleanupStage.LocalCleanup)
            stages += "local"
            val attempt = runStage { localCleaner.clean(localSystem(policy.localPrompt, vocabulary), text, cancel) }
            val rejection = attempt.getOrNull()?.let { CleanupGuard.rejection(text, it) }
            when {
                attempt.isFailure -> failures += "Local cleanup: ${attempt.exceptionOrNull()?.message ?: "failed"}"
                rejection != null -> failures += "Local cleanup: $rejection"
                else -> text = attempt.getOrThrow().trim()
            }
        } else {
            val cleaned = FillerWords.apply(text).trim()
            if (cleaned != text && cleaned.isNotEmpty()) {
                text = cleaned
                stages += "filler"
            }
        }

        checkCancelled(cancel)
        if (willPolish && meetsThreshold(text, policy.minWords, policy.minChars)) {
            // Desktop saves `provider/model`. The phone has one provider, so only the model counts.
            val model = policy.model.substringAfterLast('/').trim()
            if (model.isEmpty()) {
                failures += "Polish: invalid model"
            } else {
                onStage(CleanupStage.Polish)
                stages += "polish"
                val attempt = runStage {
                    polisher.polish(
                        polishPrompt(policy.polishPrompt, text, vocabulary, appName),
                        model,
                        policy.variant?.takeIf(String::isNotBlank),
                        policy.deadlineSeconds * 1000L,
                        cancel,
                    )
                }
                val polished = attempt.getOrNull()?.trim().orEmpty()
                when {
                    attempt.isFailure -> failures += "Polish: ${attempt.exceptionOrNull()?.message ?: "failed"}"
                    polished.isEmpty() -> failures += "Polish: returned empty text"
                    else -> text = polished
                }
            }
        }
        checkCancelled(cancel)
        return ProcessedText(raw, text, stages, failures)
    }

    private inline fun runStage(block: () -> String): Result<String> = try {
        Result.success(block())
    } catch (cancelled: CancelledException) {
        throw cancelled
    } catch (error: Exception) {
        Result.failure(error)
    }

    private fun checkCancelled(cancel: CancelToken) {
        if (cancel.cancelled) throw CancelledException()
    }

    companion object {
        const val LOCAL_INPUT_LIMIT = 3000
        private const val MIN_ASR_DEADLINE_MS = 20_000L

        /** How long speech may take: bounded, and relative to the clip's length. */
        fun asrDeadlineMs(samples: Long, factor: Int) = maxOf(MIN_ASR_DEADLINE_MS, samples / 16 * factor)
        private const val SYSTEM =
            "You transform dictated speech into replacement text. Return only the text that should be pasted. " +
                "Do not add an explanation, label, alternative, or Markdown fence."

        fun localSystem(instructions: String, vocabulary: List<VocabEntry>): String {
            val body = instructions.trim().ifEmpty { CleanupPolicy.DEFAULT_LOCAL_PROMPT }
            return "$SYSTEM\n\n$body\n\n${Vocabulary.cleanupPrompt(vocabulary, Vocabulary.LOCAL_BUDGET)}"
        }

        fun polishPrompt(instructions: String, transcript: String, vocabulary: List<VocabEntry>, appName: String): String =
            "$SYSTEM\n\nProfile instructions:\n$instructions\n\n${Vocabulary.cleanupPrompt(vocabulary)}" +
                "Foreground application: $appName\nBrowser host: none\n\n" +
                "Raw transcript:\n<transcript>\n$transcript\n</transcript>"

        fun meetsThreshold(text: String, minWords: Int, minChars: Int): Boolean =
            (minWords == 0 && minChars == 0) ||
                (minWords > 0 && text.split(WHITESPACE).count { it.isNotEmpty() } >= minWords) ||
                (minChars > 0 && text.codePointCount(0, text.length) >= minChars)

        private val WHITESPACE = Regex("\\s+")
    }
}
