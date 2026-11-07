package se.bofr.dspi.link

import java.util.concurrent.CountDownLatch
import java.util.concurrent.Executors
import java.util.concurrent.TimeUnit
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.asCoroutineDispatcher
import kotlinx.coroutines.async
import kotlinx.coroutines.awaitAll
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import kotlinx.coroutines.withContext
import kotlinx.coroutines.withTimeout
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import se.bofr.dspi.bridge.DspiBridgeImpl
import se.bofr.dspi.bridge.InMemoryLinkStore
import se.bofr.dspi.core.DspiException
import se.bofr.dspi.core.LinkDevice
import se.bofr.dspi.core.LinkHello
import se.bofr.dspi.core.LinkInfo
import se.bofr.dspi.core.LinkListener
import se.bofr.dspi.core.LinkPhase
import se.bofr.dspi.core.LinkPhaseText
import se.bofr.dspi.core.LinkStatus
import se.bofr.dspi.fake.FakeDspiCore

/**
 * WP AN3: what the fake hub cannot time on demand: orderings and races, with
 * the core behind [LinkCoreOps] faked. The whole path is in [LinkEngineFakeHubTest].
 */
@OptIn(ExperimentalCoroutinesApi::class)
class LinkSessionEngineTest {
    private class Tokens : LinkTokenStore {
        val m = HashMap<String, Pair<String, String>>()
        val removed = ArrayList<String>()

        override fun token(hubId: String) = m[hubId]?.second

        override fun setToken(hubId: String, name: String, token: String) {
            m[hubId] = name to token
        }

        override fun remove(hubId: String) {
            removed += hubId
            m.remove(hubId)
        }

        override fun paired() = m.map { PairedHub(it.key, it.value.first) }
    }

    private class Ops : LinkCoreOps {
        var events: LinkListener? = null
        var listenerSets = 0
        var status = status(LinkPhase.CHOOSE_DEVICE)
        var devices = listOf(device("A"))
        var hello = hello()
        val log = ArrayList<String>()
        var connectGate: CompletableDeferred<Unit>? = null
        var pairGate: CompletableDeferred<String>? = null
        var closeGate: CompletableDeferred<Unit>? = null
        var connectError: DspiException? = null
        var open = true

        override suspend fun connect(url: String, clientName: String, appVersion: String, token: String, expectedHubId: String): LinkHello {
            log += "connect($token, $expectedHubId)"
            connectGate?.await()
            connectError?.let { throw it }
            return hello
        }

        override suspend fun pair(pin: String): String {
            log += "pair"
            return pairGate?.await() ?: "paired"
        }

        override suspend fun devices() = devices

        override suspend fun status() = status

        override suspend fun fetchInfo(host: String, port: Int) =
            LinkInfo("hub-from-info", "Info Hub", "console", "pin", 1, "/dspi/v1", emptyList())

        override suspend fun close() {
            closeGate?.await()
            log += "close"
        }

        override suspend fun retryNow() {
            log += "retryNow"
        }

        override suspend fun attach(handle: Long, serial: String): Boolean {
            log += "attach($serial)"
            status = status(LinkPhase.BOUND)
            return true
        }

        override suspend fun detach(serial: String) {
            log += "detach($serial)"
        }

        override suspend fun restartAnalyser() {
            log += "rtaStart"
        }

        override fun wsUrl(host: String, port: Int, path: String) = "ws://$host:$port$path"

        override fun phaseText(phase: LinkPhase, pinSource: String) = LinkPhaseText(phase.name, phase.name, "text of $phase", null, null)

        override fun setListener(listener: LinkListener?) {
            listenerSets++
            events = listener
        }

        override fun setReadOnly(readOnly: Boolean) {
            log += "readOnly($readOnly)"
        }

        override fun setInterrupted(interrupted: Boolean) {
            log += "interrupted($interrupted)"
        }

        override fun deviceOpen() = open
    }

    private class Stack(scope: TestScope) {
        val ops = Ops()
        val tokens = Tokens()
        val engine =
            LinkSessionEngine(ops, tokens, { "Pixel 9" }, "test", scope.backgroundScope, clock = { scope.testScheduler.currentTime })
    }

    private val hub = LinkHub("hub-1", "Fake Hub", "console", "pin", 1, emptyList(), "10.0.2.2", 11915)

    @Test
    fun wpAn3TheKnownHubsIdAndTokenGoToTheDial() = runTest {
        val s = Stack(this)
        s.tokens.setToken("hub-1", "Fake Hub", "tok")
        s.engine.connect(hub)
        runCurrent()
        assertEquals("connect(tok, hub-1)", s.ops.log.first { it.startsWith("connect") })
        assertTrue(s.engine.state.value is LinkSessionState.Connected)
    }

    @Test
    fun wpAn3ATypedAddressKnownByItsInfoDocumentIsGuardedToo() = runTest {
        val s = Stack(this)
        s.tokens.setToken("hub-from-info", "Info Hub", "tok")
        s.engine.connect(LinkHub.manual("10.0.2.2", 11915))
        runCurrent()
        assertEquals("connect(tok, hub-from-info)", s.ops.log.first { it.startsWith("connect") })
    }

    @Test
    fun wpAn3TheListenerIsSetOnceAndNeverFromACallback() = runTest {
        val s = Stack(this)
        assertEquals(1, s.ops.listenerSets)
        s.engine.connect(hub)
        runCurrent()
        s.ops.status = status(LinkPhase.RECONNECTING)
        s.ops.events!!.onLinkStatusChanged()
        runCurrent()
        s.ops.status = status(LinkPhase.BOUND)
        s.ops.events!!.onLinkClosed(4003, "x")
        runCurrent()
        s.engine.disconnect()
        runCurrent()
        s.engine.connect(hub)
        runCurrent()
        s.engine.close()
        runCurrent()
        assertEquals(1, s.ops.listenerSets)
    }

    @Test
    fun wpAn3ReboundRestartsTheAnalyserOnceAndLiftsTheInterruption() = runTest {
        val s = Stack(this)
        s.engine.connect(hub)
        runCurrent()
        assertEquals("a first attach leaves the analyser to the bridge", 0, s.ops.log.count { it == "rtaStart" })
        s.ops.status = status(LinkPhase.RECONNECTING, reason = "gone", attempt = 3, inMs = 4_000)
        s.ops.events!!.onLinkStatusChanged()
        runCurrent()
        val r = s.engine.state.value as LinkSessionState.Reconnecting
        assertEquals(LinkSessionState.Reconnecting(hub, "gone", testScheduler.currentTime + 4_000, 3, "text of RECONNECTING"), r)
        assertEquals(1, s.ops.log.count { it == "interrupted(true)" })
        assertEquals("the app does not detach", 0, s.ops.log.count { it.startsWith("detach") })

        s.engine.retryNow()
        runCurrent()
        assertEquals(1, s.ops.log.count { it == "retryNow" })
        assertEquals("the app does not redial", 1, s.ops.log.count { it.startsWith("connect") })

        s.ops.status = status(LinkPhase.BOUND)
        s.ops.events!!.onLinkStatusChanged()
        runCurrent()
        assertTrue(s.engine.state.value is LinkSessionState.Connected)
        assertEquals(1, s.ops.log.count { it == "rtaStart" })
        assertEquals("interrupted(false)", s.ops.log.last { it.startsWith("interrupted") })

        s.ops.events!!.onLinkDevicesChanged()
        runCurrent()
        assertEquals(1, s.ops.log.count { it == "rtaStart" })
    }

    @Test
    fun wpAn3ABridgeThatLetGoIsBoundAgain() = runTest {
        val s = Stack(this)
        s.engine.connect(hub)
        runCurrent()
        s.ops.status = status(LinkPhase.RECONNECTING)
        s.ops.events!!.onLinkStatusChanged()
        runCurrent()
        s.ops.status = status(LinkPhase.BOUND)
        s.ops.open = false
        s.ops.events!!.onLinkStatusChanged()
        runCurrent()
        assertEquals(2, s.ops.log.count { it == "attach(A)" })
        assertTrue(s.engine.state.value is LinkSessionState.Connected)
    }

    @Test
    fun wpAn3ASecondListedBoardAsksThenTheChoiceAttaches() = runTest {
        val s = Stack(this)
        s.ops.devices = listOf(device("A"), device("B"))
        s.engine.connect(hub)
        runCurrent()
        val ask = s.engine.state.value as LinkSessionState.ChooseDevice
        s.engine.chooseDevice(ask.devices[1])
        runCurrent()
        assertTrue(s.ops.log.contains("attach(B)"))
        assertEquals("B", s.engine.attachedSerial)
    }

    @Test
    fun wpAn3ATokenIssuedAfterThePairTimedOutIsStored() = runTest {
        val s = Stack(this)
        s.ops.status = status(LinkPhase.NEEDS_PAIRING)
        s.engine.connect(hub)
        runCurrent()
        s.ops.pairGate = CompletableDeferred<String>().apply { completeExceptionally(DspiException.Link("timeout", "no answer")) }
        s.engine.submitPin("1")
        runCurrent()
        assertEquals("Pairing failed: no answer", (s.engine.state.value as LinkSessionState.NeedsPin).error)
        s.ops.events!!.onLinkTokenIssued("hub-1", "late")
        s.ops.status = status(LinkPhase.CHOOSE_DEVICE)
        s.ops.events!!.onLinkStatusChanged()
        runCurrent()
        assertEquals("late", s.tokens.token("hub-1"))
        assertTrue(s.engine.state.value is LinkSessionState.Connected)
    }

    @Test
    fun wpAn3TokensGoAwayOnlyOnARefusal() = runTest {
        val s = Stack(this)
        s.tokens.setToken("hub-1", "Fake Hub", "tok")
        s.engine.connect(hub)
        runCurrent()
        for (phase in listOf(LinkPhase.RECONNECTING, LinkPhase.NO_DEVICE, LinkPhase.NEEDS_PAIRING, LinkPhase.CLOSED, LinkPhase.REFUSED)) {
            s.ops.status = status(phase)
            s.ops.events!!.onLinkStatusChanged()
            runCurrent()
        }
        assertEquals("tok", s.tokens.token("hub-1"))
        assertTrue(s.tokens.removed.isEmpty())
    }

    @Test
    fun wpAn3ALateStatusDoesNotDeleteTheTokenAPairingJustStored() = runTest {
        val s = Stack(this)
        s.tokens.setToken("hub-1", "Fake Hub", "bad")
        s.ops.status = status(LinkPhase.NEEDS_PAIRING, refused = true)
        s.engine.connect(hub)
        runCurrent()
        assertNull(s.tokens.token("hub-1"))
        s.engine.submitPin("1")
        runCurrent()
        assertEquals("paired", s.tokens.token("hub-1"))
        s.ops.events!!.onLinkStatusChanged() // the status still says refused
        runCurrent()
        assertEquals("paired", s.tokens.token("hub-1"))
    }

    @Test
    fun wpAn3ADialOvertakenByADisconnectClosesItsSession() = runTest {
        val s = Stack(this)
        val gate = CompletableDeferred<Unit>()
        s.ops.connectGate = gate
        s.engine.connect(hub)
        runCurrent()
        s.engine.disconnect()
        runCurrent()
        val closesBefore = s.ops.log.count { it == "close" }
        gate.complete(Unit)
        runCurrent()
        assertEquals("the late session is closed", closesBefore + 1, s.ops.log.count { it == "close" })
        assertEquals(LinkSessionState.Idle, s.engine.state.value)
        assertFalse(s.ops.log.any { it.startsWith("attach") })
    }

    @Test
    fun wpAn3ANewConnectWaitsForTheOldDialInsteadOfRacingIt() = runTest {
        val s = Stack(this)
        val gate = CompletableDeferred<Unit>()
        s.ops.connectGate = gate
        s.engine.connect(hub)
        runCurrent()
        s.engine.connect(hub.copy(id = "hub-2"))
        runCurrent()
        assertEquals(1, s.ops.log.count { it.startsWith("connect") })
        gate.complete(Unit)
        runCurrent()
        val order = s.ops.log.filter { it.startsWith("connect") || it == "close" }
        assertEquals(listOf("connect(, hub-1)", "close", "connect(, hub-2)"), order)
        assertEquals("hub-2", s.engine.state.value.hub?.id)
    }

    @Test
    fun wpAn3ADisconnectClosesTheSessionBeforeDetaching() = runTest {
        val s = Stack(this)
        s.engine.connect(hub)
        runCurrent()
        s.engine.disconnect()
        runCurrent()
        val tail = s.ops.log.filter { it == "close" || it.startsWith("detach") }
        assertEquals(listOf("close", "detach(A)"), tail)
    }

    @Test
    fun wpAn3AConnectRightAfterADisconnectWaitsForTheClose() = runTest {
        val s = Stack(this)
        s.engine.connect(hub)
        runCurrent()
        val gate = CompletableDeferred<Unit>()
        s.ops.closeGate = gate
        s.engine.disconnect()
        s.ops.status = status(LinkPhase.CHOOSE_DEVICE) // what the new session will say
        s.engine.connect(hub)
        runCurrent()
        assertEquals(1, s.ops.log.count { it.startsWith("connect") })
        gate.complete(Unit)
        runCurrent()
        assertEquals(
            listOf("connect(, hub-1)", "close", "connect(, hub-1)"),
            s.ops.log.filter {
                it.startsWith("connect") || it == "close"
            }
        )
        assertTrue(s.engine.state.value is LinkSessionState.Connected)
    }

    /** The dial must not sit on the bridge's FFI queue: real threads, the real ops, a dial held open. */
    @Test
    fun wpAn3ADialDoesNotBlockTheBridgesQueue() = runBlocking {
        val dialing = CountDownLatch(1)
        val release = CountDownLatch(1)
        val core = object : FakeDspiCore() {
            override fun linkConnect(url: String, clientName: String, appVersion: String, token: String, expectedHubId: String): LinkHello {
                dialing.countDown()
                release.await(20, TimeUnit.SECONDS)
                return super.linkConnect(url, clientName, appVersion, token, expectedHubId)
            }
        }
        val ffiPool = Executors.newSingleThreadExecutor()
        val ffi = ffiPool.asCoroutineDispatcher()
        val networkPool = Executors.newCachedThreadPool()
        val network = networkPool.asCoroutineDispatcher()
        try {
            val ops = BridgeLinkCoreOps(core, unusedBridge(core, ffi), ffi, network)
            val results = listOf(
                async(network) {
                    ops.connect("ws://h", "n", "v", "", "")
                    "dial"
                },
                async(network) {
                    assertTrue(dialing.await(10, TimeUnit.SECONDS))
                    // The FFI queue answers while the dial is held
                    withTimeout(5_000) { withContext(ffi) { "ffi free" } }.also { release.countDown() }
                }
            ).awaitAll()
            assertEquals(listOf("dial", "ffi free"), results)
        } finally {
            release.countDown()
            ffi.close()
            network.close()
        }
    }

    private fun unusedBridge(core: FakeDspiCore, ffi: kotlinx.coroutines.CoroutineDispatcher) =
        DspiBridgeImpl(core, kotlinx.coroutines.CoroutineScope(ffi), ffi = ffi, main = ffi, linkStore = InMemoryLinkStore())

    private companion object {
        fun device(serial: String) = LinkDevice(0, serial, "Bench", "1.1.6", 5, 1, "online", "usb", 0)

        fun hello() = LinkHello(
            hubId = "",
            hubName = "",
            hubKind = "",
            hubVersion = "",
            protoMinor = 0,
            auth = "",
            pinSource = "",
            needsPairing = false,
            caps = emptyList(),
            maxPayload = 0,
            maxInflight = 0,
            tokenRefused = false
        )

        fun status(phase: LinkPhase, reason: String = "", attempt: Long = 0, inMs: Long = 0, refused: Boolean = false) = LinkStatus(
            connected = true,
            ready = true,
            session = 1,
            role = "control",
            denied = emptyList(),
            closed = false,
            closeCode = -1,
            closeReason = reason,
            phase = phase,
            reconnectAttempt = attempt,
            reconnectInMs = inMs,
            pinSource = "window",
            tokenRefused = refused
        )
    }
}
