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
        val urls = ArrayList<String>()
        var connectGate: CompletableDeferred<Unit>? = null
        var pairGate: CompletableDeferred<String>? = null
        var closeGate: CompletableDeferred<Unit>? = null
        var connectError: DspiException? = null
        var open = true

        val tlsAdvertised = ArrayList<Boolean>() // SEC-B

        override suspend fun connect(
            url: String,
            clientName: String,
            appVersion: String,
            token: String,
            expectedHubId: String,
            tlsAdvertised: Boolean
        ): LinkHello {
            log += "connect($token, $expectedHubId)"
            urls += url
            this.tlsAdvertised += tlsAdvertised
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

        override suspend fun setUrl(url: String) {
            log += "setUrl($url)"
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

        // -- WP PINTLS -- these tests keep v1 tokens only: nothing to keep after a refusal
        override fun credentialUnpaired(credential: String) = ""

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

    private class Service : LinkServiceControl {
        var running = false
        var refuse = false
        var starts = 0
        var stops = 0

        override fun start(): Boolean {
            starts++
            if (!refuse) running = true
            return !refuse
        }

        override fun stop() {
            stops++
            running = false
        }
    }

    private class Stack(scope: TestScope) {
        val ops = Ops()
        val tokens = Tokens()
        val service = Service()
        private val clock = { scope.testScheduler.currentTime }
        val engine = LinkSessionEngine(ops, tokens, { "Pixel 9" }, "test", scope.backgroundScope, service, clock = clock)

        fun connects() = ops.log.count { it.startsWith("connect") }

        fun phase(p: LinkPhase, attempt: Long = 0) {
            ops.status = status(p, attempt = attempt)
            ops.events!!.onLinkStatusChanged()
        }
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

    /** SEC-B: the browsed record's `tls=1` reaches the dial, so the core never falls back to plaintext. */
    @Test
    fun secbAnAdvertisedTlsHubIsDialledAsSuch() = runTest {
        val s = Stack(this)
        s.engine.connect(hub.copy(tlsAdvertised = true))
        runCurrent()
        assertEquals(listOf(true), s.ops.tlsAdvertised)
        assertTrue(s.engine.state.value is LinkSessionState.Connected)
    }

    @Test
    fun secbAnUpstreamHubOrATypedAddressIsNot() = runTest {
        val s = Stack(this)
        s.engine.connect(hub)
        runCurrent()
        s.engine.connect(LinkHub.manual("10.0.2.2", 11915))
        runCurrent()
        assertTrue(s.ops.tlsAdvertised.isNotEmpty())
        assertTrue(s.ops.tlsAdvertised.none { it })
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
        assertEquals("timeout", (s.engine.state.value as LinkSessionState.NeedsPin).errorCode)
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
                    ops.connect("ws://h", "n", "v", "", "", false)
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

    // -- WP AN4 --

    private val moved = hub.copy(host = "10.0.2.9", port = 11916)
    private val tenMinutes = LINK_RECONNECT_BOUND_FOR_TEST

    private fun TestScope.failedFirstDial(): Stack {
        val s = Stack(this)
        s.ops.connectError = DspiException.Link("io", "no route")
        s.engine.connect(hub)
        runCurrent()
        assertTrue(s.engine.state.value is LinkSessionState.Failed)
        assertEquals(1, s.connects())
        return s
    }

    @Test
    fun wpAn4AFailedFirstDialIsRepeatedOncePerNetworkEdgeAndNeverOnATimer() = runTest {
        val s = failedFirstDial()
        testScheduler.advanceTimeBy(24 * 3_600_000L)
        runCurrent()
        assertEquals("no timer", 1, s.connects())

        s.engine.networkAvailable()
        runCurrent()
        assertEquals(2, s.connects())
        assertTrue(s.engine.state.value is LinkSessionState.Failed)
        testScheduler.advanceTimeBy(3_600_000L)
        runCurrent()
        assertEquals("a failed redial starts nothing", 2, s.connects())

        s.ops.connectError = null
        s.engine.networkAvailable()
        runCurrent()
        assertEquals(3, s.connects())
        assertTrue(s.engine.state.value is LinkSessionState.Connected)
    }

    @Test
    fun wpAn4ARedialKeepsTheHubsGuardAndTheWantedBoard() = runTest {
        val s = Stack(this)
        s.tokens.setToken("hub-1", "Fake Hub", "tok")
        s.ops.connectError = DspiException.Link("io", "no route")
        s.engine.connect(hub, "B")
        runCurrent()
        s.ops.connectError = null
        s.ops.devices = listOf(device("A"), device("B"))
        s.engine.networkAvailable()
        runCurrent()
        assertEquals("connect(tok, hub-1)", s.ops.log.last { it.startsWith("connect") })
        assertEquals("B", s.engine.attachedSerial)
    }

    @Test
    fun wpAn4ANetworkEdgeDuringAFirstDialIsNotLost() = runTest {
        val s = Stack(this)
        val gate = CompletableDeferred<Unit>()
        s.ops.connectGate = gate
        s.ops.connectError = DspiException.Link("io", "no route")
        s.engine.connect(hub)
        runCurrent()
        s.engine.networkAvailable()
        s.engine.networkAvailable()
        runCurrent()
        assertEquals("no second dial next to the first", 1, s.connects())
        gate.complete(Unit)
        runCurrent()
        assertEquals("however many edges came, one more dial", 2, s.connects())
        assertTrue(s.engine.state.value is LinkSessionState.Failed)
        runCurrent()
        assertEquals(2, s.connects())
    }

    @Test
    fun wpAn4ADialThatWorkedIsNotRepeatedForAnEdgeDuringIt() = runTest {
        val s = Stack(this)
        val gate = CompletableDeferred<Unit>()
        s.ops.connectGate = gate
        s.engine.connect(hub)
        runCurrent()
        s.engine.networkAvailable()
        gate.complete(Unit)
        runCurrent()
        assertEquals(1, s.connects())
        assertTrue(s.engine.state.value is LinkSessionState.Connected)
    }

    @Test
    fun wpAn4ANetworkEdgeWhileReconnectingOnlyCutsTheBackoffShort() = runTest {
        val s = Stack(this)
        s.engine.connect(hub)
        runCurrent()
        s.engine.networkAvailable()
        s.engine.networkLost()
        runCurrent()
        assertEquals("nothing to do on a live session", 0, s.ops.log.count { it == "retryNow" })

        s.phase(LinkPhase.RECONNECTING)
        runCurrent()
        s.engine.networkAvailable()
        runCurrent()
        assertEquals(1, s.ops.log.count { it == "retryNow" })
        s.engine.networkLost()
        runCurrent()
        assertEquals(2, s.ops.log.count { it == "retryNow" })
        assertEquals("the core reconnects, the app does not dial", 1, s.connects())
    }

    @Test
    fun wpAn4NothingRedialsAHubTheUserLeftOrAPromptWaitingForThem() = runTest {
        val s = failedFirstDial()
        s.engine.disconnect()
        runCurrent()
        s.engine.networkAvailable()
        s.engine.hubSeen(moved)
        runCurrent()
        assertEquals(1, s.connects())
        assertEquals(LinkSessionState.Idle, s.engine.state.value)

        s.ops.connectError = null
        s.engine.connect(hub)
        runCurrent()
        s.phase(LinkPhase.NEEDS_PAIRING)
        runCurrent()
        s.engine.networkAvailable()
        s.engine.hubSeen(moved)
        runCurrent()
        assertEquals(2, s.connects())
        assertTrue(s.engine.state.value is LinkSessionState.NeedsPin)
    }

    @Test
    fun wpAn4AMovedHubGoesToTheCoreWhileReconnectingWithoutADial() = runTest {
        val s = Stack(this)
        s.engine.connect(hub)
        runCurrent()
        s.phase(LinkPhase.RECONNECTING)
        runCurrent()
        s.engine.hubSeen(moved)
        runCurrent()
        assertEquals(listOf("setUrl(ws://10.0.2.9:11916/dspi/v1)"), s.ops.log.filter { it.startsWith("setUrl") })
        assertEquals(1, s.connects())
        assertEquals("10.0.2.9", s.engine.state.value.hub?.host)

        // Its state follows the new address from now on, and the address sticks
        s.phase(LinkPhase.RECONNECTING, attempt = 2)
        runCurrent()
        assertEquals(11916, s.engine.state.value.hub?.port)
        s.engine.hubSeen(moved)
        runCurrent()
        assertEquals("no change, no new address", 1, s.ops.log.count { it.startsWith("setUrl") })
        assertEquals("announced again: try now", 1, s.ops.log.count { it == "retryNow" })
    }

    @Test
    fun wpAn4AnotherHubsAnnouncementChangesNothing() = runTest {
        val s = Stack(this)
        s.engine.connect(hub)
        runCurrent()
        s.phase(LinkPhase.RECONNECTING)
        runCurrent()
        s.engine.hubSeen(moved.copy(id = "hub-2"))
        runCurrent()
        assertEquals(0, s.ops.log.count { it.startsWith("setUrl") || it == "retryNow" })
        assertEquals(hub.host, s.engine.state.value.hub?.host)
    }

    @Test
    fun wpAn4AFailedDialGoesOnceToTheNewAddress() = runTest {
        val s = failedFirstDial()
        s.engine.hubSeen(moved)
        runCurrent()
        assertEquals(2, s.connects())
        assertEquals("ws://10.0.2.9:11916/dspi/v1", s.ops.urls.last())
        assertEquals("hub-1", s.engine.state.value.hub?.id)
    }

    @Test
    fun wpAn4AMoveSeenDuringAFirstDialIsUsedIfThatDialFails() = runTest {
        val s = Stack(this)
        val gate = CompletableDeferred<Unit>()
        s.ops.connectGate = gate
        s.ops.connectError = DspiException.Link("io", "no route")
        s.engine.connect(hub)
        runCurrent()
        s.engine.hubSeen(hub) // listed where we dial: no news
        s.engine.hubSeen(moved)
        runCurrent()
        assertEquals(1, s.connects())
        gate.complete(Unit)
        runCurrent()
        assertEquals(2, s.connects())
        assertEquals("ws://10.0.2.9:11916/dspi/v1", s.ops.urls.last())
    }

    @Test
    fun wpAn4AListedHubDoesNotRepeatADialThatJustFailed() = runTest {
        val s = Stack(this)
        s.ops.connectError = DspiException.Link("io", "no route")
        s.engine.connect(hub)
        s.engine.hubSeen(hub)
        runCurrent()
        assertEquals(1, s.connects())
        assertTrue(s.engine.state.value is LinkSessionState.Failed)
    }

    @Test
    fun wpAn4TheServiceIsLetGoAfterTenMinutesOutOfSightAndTheSessionStays() = runTest {
        val s = Stack(this)
        s.engine.connect(hub)
        runCurrent()
        assertTrue(s.service.running)
        s.engine.setUiVisible(false)
        s.phase(LinkPhase.RECONNECTING)
        runCurrent()
        // A new attempt does not start the time again
        testScheduler.advanceTimeBy(tenMinutes - 1)
        s.phase(LinkPhase.RECONNECTING, attempt = 9)
        runCurrent()
        assertTrue(s.service.running)
        assertFalse((s.engine.state.value as LinkSessionState.Reconnecting).parked)

        testScheduler.advanceTimeBy(2)
        runCurrent()
        assertFalse(s.service.running)
        assertTrue((s.engine.state.value as LinkSessionState.Reconnecting).parked)
        assertEquals("the session is not closed", 0, s.ops.log.count { it == "close" })
        assertEquals("nor the device let go", 0, s.ops.log.count { it.startsWith("detach") })

        // A status while parked keeps it parked and starts no new timer
        s.phase(LinkPhase.RECONNECTING, attempt = 10)
        runCurrent()
        assertTrue((s.engine.state.value as LinkSessionState.Reconnecting).parked)

        // Back in front
        s.engine.setUiVisible(true)
        runCurrent()
        assertTrue(s.service.running)
        assertFalse((s.engine.state.value as LinkSessionState.Reconnecting).parked)
    }

    @Test
    fun wpAn4NoBoundInFrontOrOnceTheHubIsBack() = runTest {
        val s = Stack(this)
        s.engine.connect(hub)
        runCurrent()
        s.phase(LinkPhase.RECONNECTING)
        runCurrent()
        testScheduler.advanceTimeBy(tenMinutes * 3)
        runCurrent()
        assertTrue("in front the service stays", s.service.running)

        s.engine.setUiVisible(false)
        testScheduler.advanceTimeBy(tenMinutes - 1)
        s.phase(LinkPhase.BOUND)
        runCurrent()
        testScheduler.advanceTimeBy(tenMinutes * 3)
        runCurrent()
        assertTrue("connected again: the timer is off", s.service.running)
        assertTrue(s.engine.state.value is LinkSessionState.Connected)
    }

    @Test
    fun wpAn4AHubBackWhileParkedConnectsAndAskedForTheServiceAgain() = runTest {
        val s = Stack(this)
        s.engine.connect(hub)
        runCurrent()
        s.engine.setUiVisible(false)
        s.phase(LinkPhase.RECONNECTING)
        runCurrent()
        testScheduler.advanceTimeBy(tenMinutes + 1)
        runCurrent()
        assertFalse(s.service.running)

        // F10: Android refuses a foreground service from the background
        s.service.refuse = true
        s.phase(LinkPhase.BOUND)
        runCurrent()
        assertTrue(s.engine.state.value is LinkSessionState.Connected)
        assertFalse(s.service.running)
        val stops = s.service.stops
        assertTrue("a refused start lets go of the shared service's hold", stops >= 2)

        s.service.refuse = false
        s.engine.setUiVisible(true)
        runCurrent()
        assertTrue("the next look tries again", s.service.running)
    }

    @Test
    fun wpAn4ABoardFlapOnABridgeIsNoReconnect() = runTest {
        val s = Stack(this)
        s.engine.connect(hub)
        runCurrent()
        s.phase(LinkPhase.NO_DEVICE)
        runCurrent()
        assertTrue(s.engine.state.value is LinkSessionState.WaitingForDevice)
        assertEquals("F10: the app does not detach for a flap", 0, s.ops.log.count { it.startsWith("detach") })
        assertTrue(s.service.running)
        s.engine.networkAvailable()
        s.phase(LinkPhase.BOUND)
        runCurrent()
        assertTrue(s.engine.state.value is LinkSessionState.Connected)
        assertEquals(1, s.connects())
        assertEquals(1, s.ops.log.count { it == "rtaStart" })
    }

    private fun unusedBridge(core: FakeDspiCore, ffi: kotlinx.coroutines.CoroutineDispatcher) =
        DspiBridgeImpl(core, kotlinx.coroutines.CoroutineScope(ffi), ffi = ffi, main = ffi, linkStore = InMemoryLinkStore())

    private companion object {
        const val LINK_RECONNECT_BOUND_FOR_TEST = 10 * 60 * 1000L

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
