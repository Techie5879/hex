package anomaly.hex.android.sync

import anomaly.hex.android.core.codex.CodexDeviceLogin
import anomaly.hex.android.core.codex.CodexEndpoints
import anomaly.hex.android.core.codex.CodexSession
import java.io.IOException
import kotlin.concurrent.thread
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow

/** Runs the code sign-in in the app, so it survives leaving for the browser and coming back. */
class ChatGptSignIn(private val session: CodexSession, val signedIn: StateFlow<Boolean>) {
    sealed interface State {
        data object Idle : State
        data object Starting : State
        data class Waiting(val code: String) : State
        data class Failed(val message: String) : State
    }

    val approvalPage = CodexEndpoints().approvalPage
    val state = MutableStateFlow<State>(State.Idle)

    @Volatile private var running: Thread? = null

    fun start() {
        cancel()
        state.value = State.Starting
        running = thread(name = "hex-chatgpt-sign-in", isDaemon = true) { run(Thread.currentThread()) }
    }

    fun cancel() {
        running?.interrupt()
        running = null
        state.value = State.Idle
    }

    fun signOut() {
        cancel()
        session.signOut()
    }

    private fun run(self: Thread) {
        val login = CodexDeviceLogin()
        try {
            val code = login.start()
            if (running !== self) return
            state.value = State.Waiting(code.userCode)
            val giveUp = System.currentTimeMillis() + LIMIT_MS
            while (System.currentTimeMillis() < giveUp) {
                Thread.sleep(code.intervalSeconds * 1000L)
                // A dropped connection while the person is in the browser isn't a failed sign-in.
                val tokens = runCatching { login.poll(code) }.getOrNull() ?: continue
                if (running !== self) return
                session.signIn(tokens)
                running = null
                state.value = State.Idle
                return
            }
            if (running === self) state.value = State.Failed("The code expired. Try again.")
        } catch (_: InterruptedException) {
            // Cancelled.
        } catch (_: IOException) {
            if (running === self) state.value = State.Failed("Couldn't reach ChatGPT. Try again.")
        }
    }

    private companion object {
        const val LIMIT_MS = 15 * 60_000L
    }
}
