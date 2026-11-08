package se.bofr.dspi.link

import java.io.BufferedWriter
import java.io.File
import java.util.concurrent.Executors
import java.util.concurrent.TimeUnit
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.asCoroutineDispatcher
import kotlinx.coroutines.cancel
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withContext
import kotlinx.coroutines.withTimeout
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import se.bofr.dspi.bridge.DspiBridgeImpl
import se.bofr.dspi.bridge.InMemoryLinkStore
import se.bofr.dspi.core.LinkPhase
import se.bofr.dspi.ffi.FfiDspiCore

/**
 * WP AN3: the engine over the real core, the real bridge and the fake hub of
 * crates/dspi-link as a process, with L1's faults on its stdin. Real sockets and
 * the core's real backoff, no USB, no phone.
 */
class LinkEngineFakeHubTest {
    private class Hub(vararg args: String) {
        val process: Process = ProcessBuilder(listOf(System.getProperty("dspi.fakehub"), "--bind", "127.0.0.1:0") + args)
            .redirectError(ProcessBuilder.Redirect.to(File("/dev/null")))
            .start()
        private val input: BufferedWriter = process.outputStream.bufferedWriter()
        val port: Int
        val id: String

        init {
            val banner = process.inputStream.bufferedReader().readLine()
            val m = Regex("""on 127\.0\.0\.1:(\d+) id (\S+)""").find(banner) ?: error("unexpected banner: $banner")
            port = m.groupValues[1].toInt()
            id = m.groupValues[2]
        }

        fun fault(line: String) {
            input.write(line + "\n")
            input.flush()
        }

        fun stop() {
            process.destroyForcibly().waitFor(5, TimeUnit.SECONDS)
        }
    }

    private class Tokens : LinkTokenStore {
        val m = HashMap<String, Pair<String, String>>()

        override fun token(hubId: String) = m[hubId]?.second

        override fun setToken(hubId: String, name: String, token: String) {
            m[hubId] = name to token
        }

        override fun remove(hubId: String) {
            m.remove(hubId)
        }

        override fun paired() = m.map { PairedHub(it.key, it.value.first) }
    }

    private class Service : LinkServiceControl {
        @Volatile var running = false

        override fun start(): Boolean {
            running = true
            return true
        }

        override fun stop() {
            running = false
        }
    }

    /** The one thread that plays the main thread: the engine and bridge are main-thread objects. */
    private val mainExecutor = Executors.newSingleThreadExecutor { r -> Thread(r, "test-main").apply { isDaemon = true } }
    private val main = mainExecutor.asCoroutineDispatcher()
    private val ffiExecutor = Executors.newSingleThreadExecutor { r -> Thread(r, "test-ffi").apply { isDaemon = true } }
    private val ffi = ffiExecutor.asCoroutineDispatcher()
    private val scope = CoroutineScope(SupervisorJob() + main)
    private val core = FfiDspiCore()
    private val bridge = DspiBridgeImpl(core, scope, ffi = ffi, main = main, linkStore = InMemoryLinkStore())
    private val tokens = Tokens()
    private val service = Service()
    private val engine = runBlocking(main) {
        bridge.setUiVisible(true) // the status poll runs, as with the app in front
        LinkSessionEngine(BridgeLinkCoreOps(core, bridge, ffi), tokens, { "Pixel 9" }, "test", scope, service)
    }
    private val hubs = ArrayList<Hub>()

    private fun hub(vararg args: String) = Hub(*args).also { hubs += it }

    private fun hubOf(h: Hub, id: String = h.id) = LinkHub(id, "Fake Hub", "console", "none", 1, emptyList(), "127.0.0.1", h.port)

    private fun manualOf(h: Hub) = LinkHub.manual("127.0.0.1", h.port)

    private suspend fun connect(hub: LinkHub) = withContext(main) { engine.connect(hub) }

    private suspend inline fun <reified S : LinkSessionState> await(ms: Long = 20_000): S =
        withTimeout(ms) { engine.state.first { it is S } } as S

    @After
    fun tearDown() {
        runBlocking { withContext(main) { engine.close() } }
        hubs.forEach { it.stop() }
        scope.cancel()
        core.close()
        ffi.close()
        main.close()
    }

    @Test
    fun wpAn3AnOpenHubConnectsAndAttachesItsDevice() = runBlocking<Unit> {
        val h = hub()
        connect(hubOf(h))
        val st = await<LinkSessionState.Connected>()
        assertTrue(bridge.connected)
        assertEquals(st.device.serial, bridge.selectedSerial)
        assertTrue(bridge.writesAllowed.value)
        assertTrue(service.running)

        withContext(main) { engine.disconnect() }
        assertEquals(LinkSessionState.Idle, engine.state.value)
        withTimeout(5_000) { while (bridge.connected) delay(10) }
        assertFalse(service.running)
        assertEquals(LinkPhase.IDLE, core.linkStatus().phase)
    }

    @Test
    fun wpAn3AWrongPinAsksAgainThenTheTokenIsRemembered() = runBlocking<Unit> {
        val h = hub("--auth", "fixed:123456")
        connect(hubOf(h))
        assertEquals("fixed", await<LinkSessionState.NeedsPin>().pinSource)
        withContext(main) { engine.submitPin("000000") }
        val again = withTimeout(10_000) {
            engine.state.first { it is LinkSessionState.NeedsPin && it.error.isNotEmpty() }
        } as LinkSessionState.NeedsPin
        assertEquals("Pairing failed: the hub did not accept that code.", again.error)
        assertEquals("wrong_pin", again.errorCode)
        assertFalse(again.busy)
        assertNull(tokens.token(h.id))

        withContext(main) { engine.submitPin("123456") }
        await<LinkSessionState.Connected>()
        assertTrue(tokens.token(h.id)!!.isNotEmpty())

        // Next time the token goes along and no prompt appears
        withContext(main) { engine.disconnect() }
        connect(hubOf(h))
        await<LinkSessionState.Connected>()
    }

    @Test
    fun wpAn3ATypedAddressPairsAndFindsItsTokenByTheHubsId() = runBlocking<Unit> {
        val h = hub("--auth", "fixed:123456")
        connect(manualOf(h))
        val ask = await<LinkSessionState.NeedsPin>()
        assertEquals(h.id, ask.hub.id)
        withContext(main) { engine.submitPin("123456") }
        await<LinkSessionState.Connected>()
        assertTrue(tokens.token(h.id) != null)

        withContext(main) { engine.disconnect() }
        connect(manualOf(h))
        assertEquals(h.id, await<LinkSessionState.Connected>().hub.id)
    }

    @Test
    fun wpAn3ALostConnectionKeepsTheMirrorAndTheCoreBringsItBack() = runBlocking<Unit> {
        val h = hub("--rta")
        connect(hubOf(h))
        await<LinkSessionState.Connected>()
        withTimeout(10_000) { while (!core.rtaGetCaps().supported) delay(20) }
        val preamp = bridge.preampDB
        val serial = bridge.selectedSerial
        val revision = bridge.stateRevision.value

        // 503: no redial gets through until the hub says so
        h.fault("refuse 503")
        h.fault("drop")
        val lost = await<LinkSessionState.Reconnecting>()
        assertEquals("Connection lost, reconnecting", lost.text)
        assertEquals("the mirror stays readable", preamp, bridge.preampDB)
        assertEquals(serial, bridge.selectedSerial)
        assertTrue(bridge.connected)
        assertFalse("writes are refused, not queued", bridge.writesAllowed.value)
        assertTrue("the service stays for the reconnect", service.running)
        assertEquals("the core retries, not the app", LinkPhase.RECONNECTING, core.linkStatus().phase)

        h.fault("refuse ok")
        withContext(main) { engine.retryNow() }
        await<LinkSessionState.Connected>(30_000)
        assertEquals(serial, bridge.selectedSerial)
        withTimeout(5_000) { while (!bridge.writesAllowed.value) delay(10) }
        withTimeout(5_000) { while (bridge.stateRevision.value == revision) delay(10) } // NOTIFY_REFRESHED reached the UI
        // The analyser's worker ended with the old connection: only a new rta_start brings its caps back
        withTimeout(10_000) { while (!core.rtaGetCaps().supported) delay(20) }
    }

    @Test
    fun wpAn3DisconnectStopsTheReconnecting() = runBlocking<Unit> {
        val h = hub()
        connect(hubOf(h))
        await<LinkSessionState.Connected>()
        h.fault("refuse 503")
        h.fault("drop")
        await<LinkSessionState.Reconnecting>()

        withContext(main) { engine.disconnect() }
        assertEquals(LinkSessionState.Idle, engine.state.value)
        withTimeout(5_000) { while (bridge.connected) delay(10) }
        assertFalse(service.running)
        h.fault("refuse ok")
        delay(1_500)
        assertEquals("nothing came back", LinkSessionState.Idle, engine.state.value)
        assertEquals(LinkPhase.IDLE, core.linkStatus().phase)
    }

    @Test
    fun wpAn3ARevokedTokenIsDeletedAndPairingAgainBringsTheDeviceBack() = runBlocking<Unit> {
        val h = hub("--auth", "fixed:123456")
        connect(hubOf(h))
        await<LinkSessionState.NeedsPin>()
        withContext(main) { engine.submitPin("123456") }
        await<LinkSessionState.Connected>()
        assertTrue(tokens.token(h.id) != null)

        h.fault("revoke")
        val ask = await<LinkSessionState.NeedsPin>()
        assertTrue(ask.error, ask.error.contains("revoked"))
        assertNull(tokens.token(h.id))
        assertFalse(service.running)
        assertFalse(bridge.writesAllowed.value)

        withContext(main) { engine.submitPin("123456") }
        await<LinkSessionState.Connected>(30_000)
        assertTrue(tokens.token(h.id) != null)
        assertTrue(service.running)
        withTimeout(5_000) { while (!bridge.writesAllowed.value) delay(10) }
    }

    @Test
    fun wpAn3RateLimitedKeepsTheToken() = runBlocking<Unit> {
        val h = hub("--auth", "fixed:123456")
        tokens.setToken(h.id, "Fake Hub", "still-good")
        h.fault("token rate_limited")
        connect(hubOf(h))
        val failed = await<LinkSessionState.Failed>()
        assertTrue(failed.reason, failed.reason.isNotEmpty())
        assertEquals("still-good", tokens.token(h.id))
    }

    @Test
    fun wpAn3ARateLimitedRedialKeepsTheTokenToo() = runBlocking<Unit> {
        val h = hub("--auth", "fixed:123456")
        connect(hubOf(h))
        await<LinkSessionState.NeedsPin>()
        withContext(main) { engine.submitPin("123456") }
        await<LinkSessionState.Connected>()
        val token = tokens.token(h.id)

        h.fault("token rate_limited")
        h.fault("drop")
        await<LinkSessionState.Reconnecting>()
        withTimeout(10_000) { while (core.linkStatus().reconnectAttempt == 0L && core.linkStatus().reconnectInMs < 1_000) delay(20) }
        assertEquals(token, tokens.token(h.id))
    }

    // -- WP AN4 --

    @Test
    fun wpAn4AFirstDialThatFailedConnectsOnTheNextNetworkEdge() = runBlocking<Unit> {
        val h = hub()
        h.fault("refuse 503")
        connect(hubOf(h))
        await<LinkSessionState.Failed>()
        h.fault("refuse ok")
        withContext(main) { engine.networkAvailable() }
        await<LinkSessionState.Connected>()
        assertTrue(bridge.connected)
    }

    @Test
    fun wpAn4AHubThatMovedIsFollowedThroughTheCoresSessionKeepingTheMirror() = runBlocking<Unit> {
        val old = hub()
        connect(hubOf(old))
        val first = await<LinkSessionState.Connected>()
        val serial = bridge.selectedSerial
        val second = hub() // same hub id as the fake's default, at another port
        assertEquals(old.id, second.id)
        old.stop()
        await<LinkSessionState.Reconnecting>()
        assertFalse(bridge.writesAllowed.value)

        withContext(main) { engine.hubSeen(hubOf(second)) }
        val back = await<LinkSessionState.Connected>(30_000)
        assertEquals(second.port, back.hub.port)
        assertEquals(first.device.serial, back.device.serial)
        assertEquals(serial, bridge.selectedSerial)
        withTimeout(5_000) { while (!bridge.writesAllowed.value) delay(10) }
    }

    @Test
    fun wpAn4AnAddressAnotherHubAnswersAtDoesNotEndTheReconnect() = runBlocking<Unit> {
        val h = hub()
        connect(hubOf(h))
        await<LinkSessionState.Connected>()
        val real = hub()
        val impostor = hub("--bridge") // another hub id than the one announced for it below
        h.stop()
        await<LinkSessionState.Reconnecting>()
        withContext(main) { engine.hubSeen(hubOf(impostor, id = h.id)) }
        delay(1_500) // the core has tried it, and it is not this hub
        assertTrue(engine.state.value is LinkSessionState.Reconnecting)
        assertEquals(LinkPhase.RECONNECTING, core.linkStatus().phase)

        withContext(main) { engine.hubSeen(hubOf(real)) }
        assertEquals(real.port, await<LinkSessionState.Connected>(30_000).hub.port)
    }

    @Test
    fun wpAn3ARefusedTokenIsDeletedAndThePromptAppears() = runBlocking<Unit> {
        val h = hub("--auth", "fixed:123456")
        tokens.setToken(h.id, "Fake Hub", "bad-token")
        connect(hubOf(h))
        await<LinkSessionState.NeedsPin>()
        assertNull(tokens.token(h.id))
    }

    @Test
    fun wpAn3AnotherHubAtTheAddressKeepsTheTokenAndFails() = runBlocking<Unit> {
        val h = hub()
        val other = "11111111-2222-4333-8444-555555555555"
        tokens.setToken(other, "Studio Mac", "theirs")
        connect(hubOf(h, id = other))
        val failed = await<LinkSessionState.Failed>()
        assertEquals("Another hub answers at this address.", failed.reason)
        assertEquals("theirs", tokens.token(other))
        assertEquals(LinkPhase.IDLE, core.linkStatus().phase)
    }

    // WP LK4: the core's first dial retries for 60 s before it fails (2 s here:
    // DSPI_LINK_FIRST_DIAL_MS in build.gradle.kts); the app adds no retry of its own
    @Test
    fun wpAn3AFailedFirstDialIsNotRetriedByTheApp() = runBlocking<Unit> {
        val h = hub()
        val port = h.port
        h.stop()
        connect(LinkHub("6f1a2b3c-4d5e-4f60-8a9b-0c1d2e3f4a5b", "Gone", "console", "none", 1, emptyList(), "127.0.0.1", port))
        val failed = await<LinkSessionState.Failed>()
        delay(2_000)
        assertEquals("still the same failure", failed, engine.state.value)
        assertEquals(LinkPhase.IDLE, core.linkStatus().phase)
    }

    @Test
    fun wpAn3ABoardThatLeavesAndComesBackKeepsTheMirror() = runBlocking<Unit> {
        val h = hub()
        connect(hubOf(h))
        val st = await<LinkSessionState.Connected>()
        h.fault("board off")
        assertEquals(st.device.serial, await<LinkSessionState.WaitingForDevice>().serial)
        assertTrue(bridge.connected)
        assertFalse(bridge.writesAllowed.value)
        h.fault("board on")
        await<LinkSessionState.Connected>(30_000)
        withTimeout(5_000) { while (!bridge.writesAllowed.value) delay(10) }
    }

    @Test
    fun wpAn3NoBoardWaitsThenBindsWhenItArrives() = runBlocking<Unit> {
        val h = hub("--no-board")
        connect(hubOf(h))
        assertEquals("", await<LinkSessionState.WaitingForDevice>().serial)
        assertFalse(bridge.connected)
        h.fault("board on")
        await<LinkSessionState.Connected>()
        assertTrue(bridge.connected)
    }

    @Test
    fun wpAn3AProtocolErrorEndsTheSession() = runBlocking<Unit> {
        val h = hub()
        connect(hubOf(h))
        await<LinkSessionState.Connected>()
        h.fault("close 4000")
        assertEquals("The hub reported a protocol error.", await<LinkSessionState.Failed>().reason)
        assertFalse(bridge.connected)
        assertFalse(service.running)
    }

    @Test
    fun wpAn3AViewerSessionIsReadOnly() = runBlocking<Unit> {
        val h = hub("--auth", "fixed:123456", "--role", "viewer")
        connect(hubOf(h))
        await<LinkSessionState.NeedsPin>()
        withContext(main) { engine.submitPin("123456") }
        assertEquals("viewer", await<LinkSessionState.Connected>().role)
        assertTrue(bridge.connected)
        assertFalse(bridge.writesAllowed.value)
    }
}
