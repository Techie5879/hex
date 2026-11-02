package anomaly.hex.android.core.pipeline

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

/** A failed polish. The pipeline keeps the text it already has. */
class GenerationException(message: String) : Exception(message)
