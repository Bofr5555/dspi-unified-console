package se.bofr.dspi.link

import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import se.bofr.dspi.core.DspiException
import se.bofr.dspi.core.LinkDevice
import se.bofr.dspi.core.LinkHello
import se.bofr.dspi.core.LinkInfo
import se.bofr.dspi.core.LinkListener
import se.bofr.dspi.core.LinkPairLink
import se.bofr.dspi.core.LinkPhase
import se.bofr.dspi.core.LinkPhaseText
import se.bofr.dspi.core.LinkSecurity
import se.bofr.dspi.core.LinkStatus
import se.bofr.dspi.ffi.FfiDspiCore

/**
 * WP SECAN: the engine's half of PLAN-SEC 4.3 on Android: the credential the core gives
 * kept in the token store, the new-key notice once, pairing again offered for the key and
 * TLS failures only, and a `dspi://pair` link that is a hint, never a bypass. The core's
 * own pinning is SECCORE's (its fake hub); here the core behind [LinkCoreOps] is faked,
 * and the pairing link is the real core's parse.
 */
@OptIn(ExperimentalCoroutinesApi::class)
class LinkSecurityWpSecanTest {
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
        var status = status(LinkPhase.CHOOSE_DEVICE)
        var hello = hello()
        var security: LinkSecurity? = LinkSecurity(encrypted = false, local = false, hubKey = "", hubKeyNew = false, credential = "")
        var connectError: DspiException? = null
        val log = ArrayList<String>()
        val pins = ArrayList<String>()

        override suspend fun connect(
            url: String,
            clientName: String,
            appVersion: String,
            token: String,
            expectedHubId: String,
            tlsAdvertised: Boolean
        ): LinkHello {
            log += "connect($token, $expectedHubId)"
            connectError?.let { throw it }
            return hello
        }

        override suspend fun pair(pin: String): String {
            pins += pin
            return "paired"
        }

        override suspend fun devices() = listOf(LinkDevice(0, "A", "Bench", "1.1.6", 5, 1, "online", "usb", 0))

        override suspend fun status() = status

        override suspend fun security() = security

        override suspend fun fetchInfo(host: String, port: Int) = LinkInfo("x", "x", "console", "pin", 1, "/dspi/v1", emptyList())

        override suspend fun close() {}

        override suspend fun retryNow() {}

        override suspend fun setUrl(url: String) {}

        override suspend fun attach(handle: Long, serial: String): Boolean {
            status = status(LinkPhase.BOUND)
            return true
        }

        override suspend fun detach(serial: String) {}

        override suspend fun restartAnalyser() {}

        override fun wsUrl(host: String, port: Int, path: String) = "ws://$host:$port$path"

        override fun phaseText(phase: LinkPhase, pinSource: String) = LinkPhaseText(phase.name, phase.name, "text of $phase", null, null)

        override fun setListener(listener: LinkListener?) {
            events = listener
        }

        override fun setReadOnly(readOnly: Boolean) {}

        override fun setInterrupted(interrupted: Boolean) {}

        override fun deviceOpen() = true
    }

    private class Stack(scope: TestScope) {
        val ops = Ops()
        val tokens = Tokens()
        val engine = LinkSessionEngine(ops, tokens, { "Pixel 9" }, "test", scope.backgroundScope)

        fun connects() = ops.log.filter { it.startsWith("connect") }
    }

    private val hub = LinkHub("hub-1", "Fake Hub", "console", "pin", 1, emptyList(), "10.0.2.2", 11915)

    /** The real core: its pairing-link parse and its credential reader. */
    private val core = FfiDspiCore()

    @After
    fun close() = core.close()

    private val key = "A".repeat(43)
    private val hid = "6f1a2b3c-4d5e-4f60-8a9b-0c1d2e3f4a5b"

    private fun link(pin: String = "482913"): LinkPairLink = core.linkParsePairLink("dspi://pair?h=$hid&a=10.0.2.2:11915&k=$key&p=$pin")

    @Test
    fun wpSecanAnUpgradedCredentialIsSavedWhenItChanges() = runTest {
        val s = Stack(this)
        s.tokens.setToken("hub-1", "Fake Hub", "v1-token")
        val pinned = "v2.djEtdG9rZW4.$key.t"
        s.ops.security =
            LinkSecurity(encrypted = true, local = false, hubKey = "SHA256:AAAA-AAAA-AAAA-AAAA", hubKeyNew = true, credential = pinned)
        s.engine.connect(hub)
        runCurrent()
        assertEquals("the stored v1 token went out", "connect(v1-token, hub-1)", s.connects().single())
        assertEquals("the v2 credential replaced it", pinned, s.tokens.token("hub-1"))
        assertEquals("Fake Hub", s.tokens.m.getValue("hub-1").first)
        assertTrue(s.engine.state.value is LinkSessionState.Connected)
        // The same credential again is not written again
        s.tokens.m["hub-1"] = "Renamed" to pinned
        s.ops.events!!.onLinkStatusChanged()
        runCurrent()
        assertEquals("Renamed", s.tokens.m.getValue("hub-1").first)
    }

    @Test
    fun wpSecanTheSecurityStatesReachTheUi() = runTest {
        val s = Stack(this)
        s.engine.connect(hub)
        runCurrent()
        assertEquals(HubSecurity("hub-1", encrypted = false, hubKey = ""), s.engine.security.value)
        assertNull("no credential, nothing stored", s.tokens.token("hub-1"))
        s.ops.security =
            LinkSecurity(encrypted = true, local = false, hubKey = "SHA256:K7Q2-M9XD-4TNB-0ZRA", hubKeyNew = false, credential = "")
        s.ops.events!!.onLinkStatusChanged()
        runCurrent()
        assertEquals(HubSecurity("hub-1", encrypted = true, hubKey = "SHA256:K7Q2-M9XD-4TNB-0ZRA"), s.engine.security.value)
        // -- WP SECFIX -- the core's `local` reaches the UI: not unsecured
        s.ops.security = LinkSecurity(encrypted = false, local = true, hubKey = "", hubKeyNew = false, credential = "")
        s.ops.events!!.onLinkStatusChanged()
        runCurrent()
        assertEquals(HubSecurity("hub-1", encrypted = false, hubKey = "", local = true), s.engine.security.value)
        assertEquals(false, s.engine.security.value!!.unsecured)
        s.engine.disconnect()
        runCurrent()
        assertNull(s.engine.security.value)
    }

    @Test
    fun wpSecanTheNewKeyNoticeComesOnce() = runTest {
        val s = Stack(this)
        s.ops.security =
            LinkSecurity(encrypted = true, local = false, hubKey = "SHA256:K7Q2-M9XD-4TNB-0ZRA", hubKeyNew = true, credential = "")
        s.engine.connect(hub)
        runCurrent()
        assertTrue(s.engine.security.value!!.keyNotice)
        s.engine.ackKeyNotice()
        assertFalse(s.engine.security.value!!.keyNotice)
        // The core still says new for this session: not shown again, nor after a redial
        s.ops.events!!.onLinkStatusChanged()
        runCurrent()
        assertFalse(s.engine.security.value!!.keyNotice)
        s.engine.connect(hub)
        runCurrent()
        assertFalse(s.engine.security.value!!.keyNotice)
        // Unencrypted never has one
        s.ops.security = LinkSecurity(encrypted = false, local = false, hubKey = "", hubKeyNew = true, credential = "")
        s.engine.connect(hub.copy(id = "hub-2"))
        runCurrent()
        assertFalse(s.engine.security.value!!.keyNotice)
    }

    @Test
    fun wpSecanAChangedKeyOffersPairingAgainAndNothingElse() = runTest {
        val s = Stack(this)
        s.tokens.setToken("hub-1", "Fake Hub", "v2.dG9r.$key.t")
        for (code in REPAIR_CODES) {
            s.ops.connectError = DspiException.Link(code, "the core's words for $code")
            s.engine.connect(hub)
            runCurrent()
            val failed = s.engine.state.value as LinkSessionState.Failed
            assertEquals(code, failed.code)
            assertFalse(failed.fromPairLink)
        }
        // A failure that is not about keys or TLS offers nothing: rePair does nothing
        s.ops.connectError = DspiException.Link("io", "unreachable")
        s.engine.connect(hub)
        runCurrent()
        val before = s.connects().size
        s.engine.rePair()
        runCurrent()
        assertEquals(before, s.connects().size)
        assertEquals("v2.dG9r.$key.t", s.tokens.token("hub-1"))
        // hub_key_changed: the pinned credential goes, and the hub is dialled without it (it asks for a PIN)
        s.ops.connectError = DspiException.Link("hub_key_changed", "this hub's key changed; pair again to trust the new key")
        s.engine.connect(hub)
        runCurrent()
        s.ops.connectError = null
        s.ops.status = status(LinkPhase.NEEDS_PAIRING)
        s.ops.hello = hello().copy(needsPairing = true)
        s.ops.security =
            LinkSecurity(encrypted = true, local = false, hubKey = "SHA256:NEW0-NEW0-NEW0-NEW0", hubKeyNew = false, credential = "")
        s.engine.rePair()
        runCurrent()
        assertEquals(listOf("hub-1"), s.tokens.removed)
        assertEquals("connect(, hub-1)", s.connects().last())
        val prompt = s.engine.state.value as LinkSessionState.NeedsPin
        assertEquals("the PIN prompt shows the key the hub presented", "SHA256:NEW0-NEW0-NEW0-NEW0", prompt.hubKey)
        assertEquals("", prompt.pin)
    }

    @Test
    fun wpSecanAPairingLinkWithAWrongKeyIsRefusedBeforeAnyPin() = runTest {
        val s = Stack(this)
        s.tokens.setToken(hid, "Studio", "v2.b2xk.$key.t")
        val l = link()
        // The core checks k against the hub's TLS key and refuses before the PIN
        s.ops.connectError = DspiException.Link("hub_key_unverified", "this hub's key does not match the one it should have")
        s.engine.connectByLink(l)
        runCurrent()
        assertEquals("the link's pinned credential, never the stored token", "connect(${l.credential}, $hid)", s.connects().single())
        assertEquals("pinned to k", l.hubKey, core.linkCredentialHubKey(l.credential))
        val failed = s.engine.state.value as LinkSessionState.Failed
        assertEquals("hub_key_unverified", failed.code)
        assertTrue(failed.fromPairLink)
        assertTrue("no PIN went out", s.ops.pins.isEmpty())
        // Offered: connect as usual, and the stored (good) pairing stays
        s.ops.connectError = null
        s.engine.rePair()
        runCurrent()
        assertEquals("v2.b2xk.$key.t", s.tokens.token(hid))
        assertEquals("connect(v2.b2xk.$key.t, $hid)", s.connects().last())
        assertTrue(s.ops.pins.isEmpty())
    }

    @Test
    fun wpSecanAPairingLinkFillsThePromptAndPairsOnlyOnTheTap() = runTest {
        val s = Stack(this)
        val l = link("482913")
        s.ops.hello = hello().copy(hubId = hid, hubName = "Studio Mac", needsPairing = true)
        s.ops.status = status(LinkPhase.NEEDS_PAIRING)
        // As the core gives it: no token yet, so no credential to store
        s.ops.security = LinkSecurity(encrypted = true, local = false, hubKey = l.hubKey, hubKeyNew = false, credential = "")
        s.engine.connectByLink(l)
        runCurrent()
        val prompt = s.engine.state.value as LinkSessionState.NeedsPin
        assertEquals("Studio Mac", prompt.hub.name)
        assertEquals(l.hubKey, prompt.hubKey)
        assertEquals("482913", prompt.pin)
        assertTrue("nothing paired without the tap", s.ops.pins.isEmpty())
        assertNull("a credential without a token is not stored", s.tokens.token(hid))
        s.engine.submitPin(prompt.pin)
        runCurrent()
        assertEquals(listOf("482913"), s.ops.pins)
        assertEquals("paired", s.tokens.token(hid))
    }

    @Test
    fun wpSecanTheTokenStoreKeepsAV2Credential() {
        val kv = object : KeyValue {
            val m = HashMap<String, String>()

            override fun get(key: String) = m[key]

            override fun put(key: String, value: String?) {
                if (value == null) m.remove(key) else m[key] = value
            }

            override fun keys() = m.keys.toSet()
        }
        val cipher = object : TokenCipher {
            override fun encrypt(plain: ByteArray) = byteArrayOf(9) + plain.map { (it.toInt() xor 0x33).toByte() }

            override fun decrypt(sealed: ByteArray) =
                if (sealed.firstOrNull() != 9.toByte()) null else sealed.drop(1).map { (it.toInt() xor 0x33).toByte() }.toByteArray()
        }
        val store = SealedLinkTokenStore(kv, cipher)
        val v2 = "v2.dG9rZW4tYnl0ZXM.$key.t.${"Q".repeat(42)}A"
        store.setToken("6F1A", "Studio Mac", v2)
        assertEquals(v2, store.token("6f1a"))
        assertEquals("the pin survives the store", core.linkCredentialHubKey(v2), core.linkCredentialHubKey(store.token("6f1a")!!))
        assertTrue(core.linkCredentialHubKey(v2).startsWith("SHA256:"))
        assertTrue("sealed", kv.m.values.none { "dG9rZW4" in it })
        assertEquals(listOf(PairedHub("6f1a", "Studio Mac")), store.paired())
    }

    @Test
    fun wpSecanBadges() {
        val browsed = hub
        assertEquals(HubBadge.VERIFIED, hubBadge(browsed, pinned = true))
        assertEquals(HubBadge.ENCRYPTED, hubBadge(browsed.copy(tlsAdvertised = true), pinned = false))
        assertEquals(HubBadge.UNENCRYPTED, hubBadge(browsed, pinned = false))
        assertEquals(HubBadge.UNKNOWN, hubBadge(LinkHub.manual("10.0.0.2", 11915), pinned = false))
        assertEquals(HubBadge.VERIFIED, hubBadge(LinkHub.manual("10.0.0.2", 11915), pinned = true))
    }

    private companion object {
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

        fun status(phase: LinkPhase) = LinkStatus(
            connected = true,
            ready = true,
            session = 1,
            role = "control",
            denied = emptyList(),
            closed = false,
            closeCode = -1,
            closeReason = "",
            phase = phase,
            reconnectAttempt = 0,
            reconnectInMs = 0,
            pinSource = "window",
            tokenRefused = false
        )
    }
}
