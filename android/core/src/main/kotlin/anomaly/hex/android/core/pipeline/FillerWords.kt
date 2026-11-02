package anomaly.hex.android.core.pipeline

/**
 * Port of the desktop's `dictation::filler_words`: hesitations leave with no model.
 * Whole words only, lowercase or first-letter capitalised, so "umbrella", "ER",
 * and "uh-huh" stay. A filler's comma or ellipsis goes with it, a sentence-starting
 * capital passes to the next word, and line breaks stay.
 */
object FillerWords {
    private val FILLERS = setOf("um", "umm", "uh", "uhh", "erm", "er", "ah")
    private val CLOSING = setOf('.', ',', ';', ':', '!', '?', ')', ']', '}', '”', '’', '…')

    fun apply(transcript: String): String {
        val cps = transcript.codePoints().toArray()
        val out = StringBuilder(transcript.length)
        var removed = false
        var capitalize = false
        var index = 0
        while (index < cps.size) {
            val end = fillerAt(cps, index)
            if (end != null) {
                trimEnd(out, emptySet())
                if (!removed) capitalize = Character.isUpperCase(cps[index]) && sentenceStart(out)
                index = afterFiller(cps, end)
                removed = true
                continue
            }
            val c = cps[index]
            index += 1
            if (removed) {
                if (isSpace(c)) continue
                if (c == '\n'.code) {
                    removed = false
                    capitalize = false
                    out.appendCodePoint(c)
                    continue
                }
                if ((c == '.'.code || c == '!'.code || c == '?'.code || c == ','.code) && sentenceStart(out)) continue
                removed = false
                val closing = (c <= 0xFFFF && c.toChar() in CLOSING) ||
                    (c == '"'.code && (index >= cps.size || !Character.isLetterOrDigit(cps[index])))
                if (closing) {
                    trimEnd(out, setOf(','))
                    capitalize = false
                } else {
                    if (needsSpace(out)) out.append(' ')
                    if (capitalize) {
                        capitalize = false
                        out.append(String(Character.toChars(c)).uppercase())
                        continue
                    }
                }
            }
            out.appendCodePoint(c)
        }
        if (removed) trimEnd(out, setOf(','))
        return out.toString()
    }

    private fun fillerAt(cps: IntArray, start: Int): Int? {
        if (start > 0 && isWord(cps[start - 1])) return null
        var end = start
        while (end < cps.size && Character.isLetter(cps[end])) end += 1
        if (end == start || (end < cps.size && (isWord(cps[end]) || cps[end] == '-'.code || cps[end] == '\''.code || cps[end] == '’'.code))) return null
        val word = String(cps, start, end - start)
        val rest = word.substring(1)
        if (rest.any(Char::isUpperCase)) return null
        return if (word.lowercase() in FILLERS) end else null
    }

    private fun afterFiller(cps: IntArray, end: Int): Int {
        var index = end
        while (index < cps.size) {
            val c = cps[index]
            when {
                c == ','.code -> index += 1
                c == '…'.code -> index += 1
                c == '.'.code && index + 1 < cps.size && cps[index + 1] == '.'.code -> {
                    while (index < cps.size && cps[index] == '.'.code) index += 1
                }
                else -> return index
            }
        }
        return index
    }

    private fun isWord(c: Int) = Character.isLetterOrDigit(c) || c == '_'.code
    private fun isSpace(c: Int) = c == ' '.code || c == '\t'.code

    private fun trimEnd(out: StringBuilder, also: Set<Char>) {
        while (out.isNotEmpty() && (out.last() == ' ' || out.last() == '\t' || out.last() in also)) out.setLength(out.length - 1)
    }

    private fun sentenceStart(out: StringBuilder): Boolean {
        val tail = out.trimEnd { it == ' ' || it == '\t' }
        return tail.isEmpty() || tail.last() == '\n' || tail.last() in setOf('.', '!', '?')
    }

    private fun needsSpace(out: StringBuilder) = out.isNotEmpty() && !out.last().isWhitespace()
}
