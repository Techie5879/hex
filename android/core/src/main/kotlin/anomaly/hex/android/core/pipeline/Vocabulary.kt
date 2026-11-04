package anomaly.hex.android.core.pipeline

import anomaly.hex.android.core.profile.VocabEntry

/** Same spelling context as the desktop's `vocabulary::cleanup_prompt`. */
object Vocabulary {
    private const val PREFIX = "Vocabulary (spell these exactly when the speaker means them): "
    private const val RULES =
        "\nCorrect a likely phonetic or spelling error to a vocabulary term only when the surrounding sentence supports that name. " +
            "Listed alternative spellings are examples, not an exhaustive list. Do not replace ordinary words merely because they resemble a term. " +
            "Preserve quoted text, code, numbers, and all unrelated wording.\n\n"
    const val LOCAL_BUDGET = 1024

    fun cleanupPrompt(entries: List<VocabEntry>, maxChars: Int = Int.MAX_VALUE): String {
        var remaining = maxChars.toLong() - PREFIX.codePointCount(0, PREFIX.length) - RULES.codePointCount(0, RULES.length)
        val terms = StringBuilder()
        for (entry in entries.filter { it.isLive }) {
            val term = if (entry.soundsLike.isEmpty()) entry.term
            else "${entry.term} (may be transcribed as ${entry.soundsLike.joinToString(", ")})"
            val separator = if (terms.isEmpty()) "" else "; "
            val length = separator.length + term.codePointCount(0, term.length)
            if (length > remaining) continue
            terms.append(separator).append(term)
            remaining -= length
        }
        return if (terms.isEmpty()) "" else "$PREFIX$terms$RULES"
    }

    private const val MAX_PROMPT_TERMS = 64
    const val WHISPER_PROMPT_CHARS = 500

    /** The context prompt a Whisper run reads: first terms in order, comma-separated, whole terms only. */
    fun transcriptionPrompt(entries: List<VocabEntry>, maxChars: Int = WHISPER_PROMPT_CHARS): String {
        val prompt = StringBuilder()
        for (entry in entries.filter { it.isLive }.take(MAX_PROMPT_TERMS)) {
            val separator = if (prompt.isEmpty()) "" else ", "
            if (prompt.codePointCount(0, prompt.length) + separator.length + entry.term.codePointCount(0, entry.term.length) > maxChars) break
            prompt.append(separator).append(entry.term)
        }
        return prompt.toString()
    }

    /**
     * Whisper can write its prompt back during silence. A transcript that is only the list
     * of two or more terms was never said, and a real sentence mentioning them is kept.
     */
    fun isPromptEcho(transcript: String, prompt: String): Boolean {
        val terms = prompt.split(',', ';', '\n').count { it.isNotBlank() }
        if (terms < 2) return false
        return normalize(transcript) == normalize(prompt)
    }

    private fun normalize(text: String) = text.lowercase().filter { it.isLetterOrDigit() || it.isWhitespace() }.split(Regex("\\s+")).filter(String::isNotEmpty).joinToString(" ")
}
