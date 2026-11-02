package anomaly.hex.android.sync

import anomaly.hex.android.core.codex.CodexDeviceLogin
import anomaly.hex.android.core.codex.CodexEndpoints
import anomaly.hex.android.core.codex.CodexLoginFailedException
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

    // Only the newest attempt may change anything. Cancelling makes the running one stale.
    private val lock = Any()
    private var attempt = 0L
    private var worker: Thread? = null

    fun start() {
        synchronized(lock) {
            val mine = supersede()
            state.value = State.Starting
            worker = thread(name = "hex-chatgpt-sign-in", isDaemon = true) { run(mine) }
        }
    }

    fun cancel() {
        synchronized(lock) {
            supersede()
            state.value = State.Idle
        }
    }

    fun signOut() {
        synchronized(lock) {
            supersede()
            state.value = try {
                session.signOut()
                State.Idle
            } catch (_: IOException) {
                State.Failed("Couldn't remove the sign-in. Try again.")
            }
        }
    }

    private fun supersede(): Long {
        worker?.interrupt()
        worker = null
        return ++attempt
    }

    private inline fun ifCurrent(mine: Long, block: () -> Unit) = synchronized(lock) { if (attempt == mine) block() }

    private fun run(mine: Long) {
        val login = CodexDeviceLogin()
        try {
            val code = login.start()
            ifCurrent(mine) { state.value = State.Waiting(code.userCode) }
            val giveUp = System.currentTimeMillis() + LIMIT_MS
            while (System.currentTimeMillis() < giveUp) {
                Thread.sleep(code.intervalSeconds * 1000L)
                val tokens = try {
                    login.poll(code) ?: continue
                } catch (failed: CodexLoginFailedException) {
                    throw failed
                } catch (_: IOException) {
                    // A dropped connection while the person is in the browser isn't a failed sign-in.
                    continue
                }
                ifCurrent(mine) {
                    worker = null
                    state.value = try {
                        session.signIn(tokens)
                        State.Idle
                    } catch (_: IOException) {
                        State.Failed("Couldn't save the sign-in. Try again.")
                    }
                }
                return
            }
            ifCurrent(mine) { state.value = State.Failed("The code expired. Try again.") }
        } catch (_: InterruptedException) {
            // Cancelled.
        } catch (_: CodexLoginFailedException) {
            ifCurrent(mine) { state.value = State.Failed("ChatGPT refused the sign-in. Try again.") }
        } catch (_: IOException) {
            ifCurrent(mine) { state.value = State.Failed("Couldn't reach ChatGPT. Try again.") }
        }
    }

    private companion object {
        const val LIMIT_MS = 15 * 60_000L
    }
}
