package anomaly.hex.android.core.pipeline

/** Port of the desktop's output guard: reject known-destructive output, keep the input. */
object CleanupGuard {
    private val NUMBERS = Regex("""\b\d+(?:[.,:]\d+)*\b""")
    private val CORRECTIONS = Regex("""(?i)\b(\d+(?:[.,:]\d+)*)\s*,?\s+no\s*,?\s+(\d+(?:[.,:]\d+)*)\b""")
    private val QUOTES = Regex("""\"([^\"\n]+)\"|“([^”\n]+)”""")
    private val TECHNICAL = Regex("""\b[A-Za-z]\w*_\w+\b|\b(?:https?://|www\.)\S+|/[A-Za-z0-9_.-]+(?:/[A-Za-z0-9_.-]+)*""")
    private val MARKERS = listOf(
        "<transcript>",
        "</transcript>",
        "Profile instructions:",
        "Foreground application:",
        "Browser host:",
        "Vocabulary (spell these exactly",
    )

    /** Returns why [output] must be rejected, or null when it passes. */
    fun rejection(input: String, output: String): String? {
        if (output.isBlank()) return "returned empty text"
        if (MARKERS.any { it in output && it !in input }) return "included prompt metadata"
        val quotes = QUOTES.findAll(input).toList()
        for (match in quotes) {
            val quote = match.groups[1]?.value ?: match.groups[2]!!.value
            if (quote !in output) return "changed quoted text"
        }
        val remaining = NUMBERS.findAll(output).map { it.value }.toSet()
        val corrected = CORRECTIONS.findAll(input).filter { c ->
            quotes.none { it.range.first <= c.range.first && it.range.last >= c.range.last } && c.groupValues[2] in remaining
        }.map { it.groupValues[1] }.toSet()
        if (NUMBERS.findAll(input).any { it.value !in remaining && it.value !in corrected }) return "removed a number"
        for (token in TECHNICAL.findAll(input)) {
            if (token.value.trimEnd('.', ',', ';', ')') !in output) return "changed a technical token"
        }
        return null
    }
}
