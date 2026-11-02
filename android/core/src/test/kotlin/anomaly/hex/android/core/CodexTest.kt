package anomaly.hex.android.core

import anomaly.hex.android.core.codex.CodexDeviceLogin
import anomaly.hex.android.core.codex.CodexEndpoints
import anomaly.hex.android.core.codex.CodexPolisher
import anomaly.hex.android.core.codex.CodexSession
import anomaly.hex.android.core.codex.CodexSignedOutException
import anomaly.hex.android.core.codex.CodexTokenStore
import anomaly.hex.android.core.codex.CodexTokens
import anomaly.hex.android.core.pipeline.CancelToken
import anomaly.hex.android.core.pipeline.CancelledException
import anomaly.hex.android.core.pipeline.GenerationException
import com.sun.net.httpserver.HttpExchange
import com.sun.net.httpserver.HttpServer
import java.net.InetSocketAddress
import java.util.Base64
import java.util.concurrent.CopyOnWriteArrayList
import java.util.concurrent.CountDownLatch
import java.util.concurrent.Executors
import java.util.concurrent.TimeUnit
import kotlin.concurrent.thread
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertThrows
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test

/** Runs the client over real sockets against a local server. All tokens here are made up. */
class CodexTest {
    private class Memory(var tokens: CodexTokens?) : CodexTokenStore {
        override fun load() = tokens
        override fun save(tokens: CodexTokens?) { this.tokens = tokens }
    }

    private data class Seen(val path: String, val authorization: String?, val account: String?, val body: String)

    private lateinit var server: HttpServer
    private lateinit var endpoints: CodexEndpoints
    private val seen = CopyOnWriteArrayList<Seen>()
    private var responses: (HttpExchange) -> Unit = { reply(it, 500, "") }
    private var tokenReply: (HttpExchange) -> Unit = { reply(it, 500, "") }

    private fun reply(exchange: HttpExchange, status: Int, body: String) {
        val bytes = body.toByteArray()
        exchange.sendResponseHeaders(status, if (bytes.isEmpty()) -1 else bytes.size.toLong())
        if (bytes.isNotEmpty()) exchange.responseBody.use { it.write(bytes) }
        exchange.close()
    }

    private fun record(exchange: HttpExchange) {
        seen += Seen(
            exchange.requestURI.path,
            exchange.requestHeaders.getFirst("Authorization"),
            exchange.requestHeaders.getFirst("chatgpt-account-id"),
            exchange.requestBody.bufferedReader().readText(),
        )
    }

    @Before
    fun start() {
        server = HttpServer.create(InetSocketAddress("127.0.0.1", 0), 0)
        server.executor = Executors.newCachedThreadPool()
        server.createContext("/api/responses") { record(it); responses(it) }
        server.createContext("/auth/oauth/token") { record(it); tokenReply(it) }
        server.start()
        val base = "http://127.0.0.1:${server.address.port}"
        endpoints = CodexEndpoints("$base/auth", "$base/api")
    }

    @After
    fun stop() = server.stop(0)

    private fun jwt(account: String? = "acct", expiresAt: Long = 4_000_000_000L): String {
        val auth = account?.let { """"https://api.openai.com/auth":{"chatgpt_account_id":"$it"},""" }.orEmpty()
        val encode = { text: String -> Base64.getUrlEncoder().withoutPadding().encodeToString(text.toByteArray()) }
        return "${encode("{}")}.${encode("""{$auth"exp":$expiresAt}""")}.sig"
    }

    private val completed = """
        event: response.output_text.delta
        data: {"type":"response.output_text.delta","delta":"Clean "}

        data: {"type":"response.output_text.delta","delta":"text."}

        data: {"type":"response.completed","response":{}}

    """.trimIndent()

    private val good = CodexTokens(jwt(), "refresh-1", "acct")

    private fun polisher(store: Memory, changed: (Boolean) -> Unit = {}) =
        CodexPolisher(CodexSession(store, endpoints, onChange = changed), endpoints)

    private fun polish(p: CodexPolisher, deadlineMs: Long = 8000, cancel: CancelToken = CancelToken()) =
        p.polish("PROMPT", "gpt-6-luna", "none", deadlineMs, cancel)

    @Test
    fun sendsOnePromptWithReasoningOffAndTheAccountHeader() {
        responses = { reply(it, 200, completed) }
        assertEquals("Clean text.", polish(polisher(Memory(good))))
        val request = seen.single()
        assertEquals("Bearer ${good.access}", request.authorization)
        assertEquals("acct", request.account)
        assertTrue(request.body.contains(""""model":"gpt-6-luna""""))
        assertTrue(request.body.contains(""""reasoning":{"effort":"none"}"""))
        assertTrue(request.body.contains(""""store":false"""))
        assertTrue(request.body.contains("PROMPT"))
    }

    @Test
    fun aRejectedTokenRenewsOnceSavesTheRotatedOneAndRetries() {
        val renewed = jwt(expiresAt = 4_000_000_001L)
        responses = { if (it.requestHeaders.getFirst("Authorization") == "Bearer $renewed") reply(it, 200, completed) else reply(it, 401, "{}") }
        tokenReply = { reply(it, 200, """{"access_token":"$renewed","refresh_token":"refresh-2"}""") }
        val store = Memory(good)
        assertEquals("Clean text.", polish(polisher(store)))
        assertEquals(CodexTokens(renewed, "refresh-2", "acct"), store.tokens)
        assertTrue(seen.single { it.path.endsWith("/oauth/token") }.body.contains("refresh_token=refresh-1"))
    }

    @Test
    fun aTokenAboutToExpireRenewsBeforeSending() {
        val renewed = jwt(expiresAt = 4_000_000_001L)
        responses = { reply(it, 200, completed) }
        tokenReply = { reply(it, 200, """{"access_token":"$renewed"}""") }
        val store = Memory(good.copy(access = jwt(expiresAt = System.currentTimeMillis() / 1000 + 60)))
        assertEquals("Clean text.", polish(polisher(store)))
        assertEquals(listOf("/auth/oauth/token", "/api/responses"), seen.map { it.path })
        assertEquals("refresh-1", store.tokens?.refresh)
    }

    @Test
    fun stillRejectedAfterRenewingFailsWithoutAThirdRequest() {
        responses = { reply(it, 401, "{}") }
        tokenReply = { reply(it, 200, """{"access_token":"${jwt()}","refresh_token":"refresh-2"}""") }
        assertThrows(CodexSignedOutException::class.java) { polish(polisher(Memory(good))) }
        assertEquals(2, seen.count { it.path.endsWith("/responses") })
    }

    @Test
    fun aRefusedRefreshTokenSignsOutAndSaysSo() {
        for (status in listOf(400, 401)) {
            seen.clear()
            var changed: Boolean? = null
            responses = { reply(it, 401, "{}") }
            tokenReply = { reply(it, status, """{"error":"invalid_grant"}""") }
            val store = Memory(good)
            assertThrows(CodexSignedOutException::class.java) { polish(polisher(store) { changed = it }) }
            assertNull(store.tokens)
            assertEquals(false, changed)
        }
    }

    @Test
    fun aTokenServerOutageKeepsTheLogin() {
        responses = { reply(it, 401, "{}") }
        tokenReply = { reply(it, 503, "") }
        val store = Memory(good)
        assertThrows(GenerationException::class.java) { polish(polisher(store)) }
        assertEquals(good, store.tokens)
    }

    @Test
    fun aRefreshOutlivesThePolishThatGaveUp() {
        val release = CountDownLatch(1)
        val renewed = jwt(expiresAt = 4_000_000_001L)
        responses = { reply(it, 401, "{}") }
        tokenReply = {
            release.await(5, TimeUnit.SECONDS)
            reply(it, 200, """{"access_token":"$renewed","refresh_token":"refresh-2"}""")
        }
        val store = Memory(good)
        assertThrows(GenerationException::class.java) { polish(polisher(store), deadlineMs = 300) }
        release.countDown()
        val until = System.currentTimeMillis() + 3000
        while (store.tokens?.refresh != "refresh-2" && System.currentTimeMillis() < until) Thread.sleep(10)
        assertEquals("refresh-2", store.tokens?.refresh)
    }

    @Test
    fun signingOutDuringARefreshStaysSignedOut() {
        val entered = CountDownLatch(1)
        val release = CountDownLatch(1)
        responses = { reply(it, 401, "{}") }
        tokenReply = {
            entered.countDown()
            release.await(5, TimeUnit.SECONDS)
            reply(it, 200, """{"access_token":"${jwt()}","refresh_token":"refresh-2"}""")
        }
        val store = Memory(good)
        val session = CodexSession(store, endpoints)
        val attempt = thread { runCatching { session.fresh(4000, rejected = good.access) } }
        assertTrue(entered.await(3, TimeUnit.SECONDS))
        session.signOut()
        release.countDown()
        attempt.join(4000)
        assertNull(store.tokens)
    }

    @Test
    fun concurrentRenewalsSpendTheRefreshTokenOnce() {
        val renewed = jwt(expiresAt = 4_000_000_001L)
        tokenReply = {
            Thread.sleep(100)
            reply(it, 200, """{"access_token":"$renewed","refresh_token":"refresh-2"}""")
        }
        val session = CodexSession(Memory(good), endpoints)
        val results = CopyOnWriteArrayList<String>()
        List(4) { thread { results += session.fresh(3000, rejected = good.access).access } }.forEach { it.join(4000) }
        assertEquals(listOf(renewed, renewed, renewed, renewed), results.toList())
        assertEquals(1, seen.count { it.path.endsWith("/oauth/token") })
    }

    @Test
    fun aUsageLimitKeepsTheLoginAndNeverRetries() {
        responses = { reply(it, 429, """{"error":{"type":"usage_limit_reached","message":"x"}}""") }
        val store = Memory(good)
        assertThrows(GenerationException::class.java) { polish(polisher(store)) }
        assertEquals(1, seen.size)
        assertEquals(good, store.tokens)
    }

    @Test
    fun theWordsInAnErrorChangeNothing() {
        responses = { reply(it, 400, """{"error":{"type":"x","message":"unauthorized quota token expired, sign in"}}""") }
        val store = Memory(good)
        assertThrows(GenerationException::class.java) { polish(polisher(store)) }
        assertEquals(1, seen.size)
        assertEquals(good, store.tokens)
    }

    @Test
    fun theDeadlineCoversASlowReply() {
        val release = CountDownLatch(1)
        responses = { release.await(5, TimeUnit.SECONDS); runCatching { reply(it, 200, completed) } }
        val started = System.currentTimeMillis()
        val failure = assertThrows(GenerationException::class.java) { polish(polisher(Memory(good)), deadlineMs = 300) }
        release.countDown()
        assertTrue(failure.message!!.contains("timed out"))
        assertTrue(System.currentTimeMillis() - started < 2000)
    }

    @Test
    fun theDeadlineCoversAReplyThatKeepsTrickling() {
        val stop = CountDownLatch(1)
        responses = { exchange ->
            exchange.sendResponseHeaders(200, 0)
            runCatching {
                while (!stop.await(50, TimeUnit.MILLISECONDS)) {
                    exchange.responseBody.write("data: {\"type\":\"response.output_text.delta\",\"delta\":\"x\"}\n\n".toByteArray())
                    exchange.responseBody.flush()
                }
            }
            exchange.close()
        }
        val started = System.currentTimeMillis()
        assertThrows(GenerationException::class.java) { polish(polisher(Memory(good)), deadlineMs = 300) }
        stop.countDown()
        assertTrue(System.currentTimeMillis() - started < 2000)
    }

    @Test
    fun cancellingStopsTheRequest() {
        val release = CountDownLatch(1)
        val arrived = CountDownLatch(1)
        responses = { arrived.countDown(); release.await(5, TimeUnit.SECONDS); runCatching { reply(it, 200, completed) } }
        val cancel = CancelToken()
        thread { arrived.await(3, TimeUnit.SECONDS); cancel.cancel() }
        assertThrows(CancelledException::class.java) { polish(polisher(Memory(good)), cancel = cancel) }
        release.countDown()
    }

    @Test
    fun alreadyCancelledSendsNothing() {
        assertThrows(CancelledException::class.java) { polish(polisher(Memory(good)), cancel = CancelToken().apply { cancel() }) }
        assertEquals(0, seen.size)
    }

    @Test
    fun aCutOffOrFailedReplyIsNeverUsed() {
        val delta = """data: {"type":"response.output_text.delta","delta":"half"}"""
        for (events in listOf(delta, "$delta\ndata: {\"type\":\"response.failed\"}", "$delta\ndata: {\"type\":\"response.incomplete\"}", "$delta\ndata: {\"type\":\"error\"}")) {
            assertThrows(events, GenerationException::class.java) { CodexPolisher.readEvents(events.lineSequence()) }
        }
    }

    @Test
    fun signedOutSendsNothing() {
        assertThrows(CodexSignedOutException::class.java) { polish(polisher(Memory(null))) }
        assertEquals(0, seen.size)
    }

    @Test
    fun deviceLoginWaitsThenExchangesTheApprovedCode() {
        var approved = false
        server.createContext("/auth/api/accounts/deviceauth/usercode") {
            record(it)
            reply(it, 200, """{"device_auth_id":"dev-1","user_code":"ABCD-EFGH","interval":"7"}""")
        }
        server.createContext("/auth/api/accounts/deviceauth/token") {
            record(it)
            if (approved) reply(it, 200, """{"authorization_code":"auth-1","code_challenge":"c","code_verifier":"verify-1"}""") else reply(it, 403, """{"error":{"code":"pending"}}""")
        }
        tokenReply = { reply(it, 200, """{"id_token":"${jwt("acct-9")}","access_token":"${jwt(null)}","refresh_token":"refresh-9"}""") }
        val login = CodexDeviceLogin(endpoints)
        val code = login.start()
        assertEquals(CodexDeviceLogin.Code("dev-1", "ABCD-EFGH", 7), code)
        assertNull(login.poll(code))
        approved = true
        val tokens = login.poll(code)
        assertEquals("refresh-9", tokens?.refresh)
        assertEquals("acct-9", tokens?.accountId)
        val exchange = seen.last { it.path.endsWith("/oauth/token") }.body
        assertTrue(exchange, exchange.contains("grant_type=authorization_code"))
        assertTrue(exchange, exchange.contains("code=auth-1"))
        assertTrue(exchange, exchange.contains("code_verifier=verify-1"))
        assertFalse(seen.first { it.path.endsWith("/usercode") }.body.isEmpty())
    }
}
