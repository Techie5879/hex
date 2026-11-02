package anomaly.hex.android.core.insert

/** Inserting dictated text at a known selection. All indices are UTF-16 offsets. */
object Insertion {
    data class Plan(val text: String, val cursor: Int, val inserted: String)

    sealed interface Result {
        data class Ok(val plan: Plan) : Result
        data object TooLong : Result
        data object Empty : Result
    }

    /**
     * The text a field should hold after replacing its selection. An unknown
     * caret (-1) appends. A selection never starts or ends inside a surrogate
     * pair. A field longer than its limit is refused, not truncated.
     */
    fun plan(
        existing: String,
        selectionStart: Int,
        selectionEnd: Int,
        dictated: String,
        multiline: Boolean,
        showingHint: Boolean = false,
        maxLength: Int = 0,
        smartSpacing: Boolean = true,
    ): Result {
        val current = if (showingHint) "" else existing
        var start = if (selectionStart < 0 || showingHint) current.length else minOf(selectionStart, current.length)
        var end = if (selectionEnd < 0 || showingHint) current.length else minOf(selectionEnd, current.length)
        if (start > end) start = end.also { end = start }
        start = aligned(current, start)
        end = aligned(current, end)
        val before = current.substring(0, start)
        val after = current.substring(end)
        val body = prepare(dictated, multiline, smartSpacing, before, after)
        if (body.isEmpty()) return Result.Empty
        val text = before + body + after
        if (maxLength > 0 && text.length > maxLength) return Result.TooLong
        return Result.Ok(Plan(text, before.length + body.length, body))
    }

    /** The exact text to commit through an input connection, given what's around the caret. */
    fun prepare(dictated: String, multiline: Boolean, smartSpacing: Boolean, before: String?, after: String?): String {
        var body = if (multiline) dictated.trim() else dictated.trim().replace(LINE_BREAKS, " ")
        if (body.isEmpty()) return body
        if (smartSpacing) {
            if (needsLeadingSpace(before, body)) body = " $body"
            if (needsTrailingSpace(after, body)) body = "$body "
        }
        return body
    }

    private fun needsLeadingSpace(before: String?, body: String): Boolean {
        val last = before?.lastOrNull() ?: return false
        val first = body.first()
        return !last.isWhitespace() && last !in OPENING && first !in NO_SPACE_BEFORE
    }

    private fun needsTrailingSpace(after: String?, body: String): Boolean {
        val next = after?.firstOrNull() ?: return false
        return !next.isWhitespace() && next !in NO_SPACE_BEFORE && !body.last().isWhitespace()
    }

    private fun aligned(text: String, index: Int): Int =
        if (index in 1 until text.length && text[index].isLowSurrogate() && text[index - 1].isHighSurrogate()) index - 1 else index

    private val LINE_BREAKS = Regex("\\s*[\\r\\n]+\\s*")
    private const val OPENING = "([{\"'“‘/-"
    private const val NO_SPACE_BEFORE = ".,;:!?)]}…”’"
}
