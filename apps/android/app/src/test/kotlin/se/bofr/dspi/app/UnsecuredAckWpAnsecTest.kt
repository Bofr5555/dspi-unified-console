package se.bofr.dspi.app

import java.lang.reflect.Proxy
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.UnconfinedTestDispatcher
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import se.bofr.dspi.bridge.DspiBridge
import se.bofr.dspi.core.LinkDevice
import se.bofr.dspi.core.Notice
import se.bofr.dspi.core.NoticeKind
import se.bofr.dspi.link.HubBrowser
import se.bofr.dspi.link.HubSecurity
import se.bofr.dspi.link.InMemoryKeyValue
import se.bofr.dspi.link.InMemoryLinkSettingsStore
import se.bofr.dspi.link.KeyValue
import se.bofr.dspi.link.LinkHub
import se.bofr.dspi.link.LinkSessionState
import se.bofr.dspi.link.LinkTokenStore
import se.bofr.dspi.link.PairedHub
import se.bofr.dspi.link.UnsecuredAckStore
import se.bofr.dspi.ui.link.AcknowledgedHubRow
import se.bofr.dspi.usb.DspiUsbDevice
import se.bofr.dspi.usb.UsbSessionState

/**
 * WP ANSEC (owner 2026-11-21): the user acknowledges an unsecured hub connection; the bar then
 * keeps its normal colours but still says "(unsecured)" with its info button. Remembered per hub
 * across restarts, dropped when the hub is secured or another hub answers at a typed address, and
 * forgotten in Settings › Networking.
 */
@OptIn(ExperimentalCoroutinesApi::class)
class UnsecuredAckWpAnsecTest {
    private val bridge = Proxy.newProxyInstance(DspiBridge::class.java.classLoader, arrayOf(DspiBridge::class.java)) { p, m, a ->
        when (m.name) {
            "hashCode" -> System.identityHashCode(p)
            "equals" -> p === a!![0]
            "getConnected" -> false
            else -> null
        }
    } as DspiBridge

    private class Browser : HubBrowser {
        override val hubs = MutableStateFlow<List<LinkHub>>(emptyList())
        override val error = MutableStateFlow<String?>(null)
        override val browsing = MutableStateFlow(false)

        override fun start() {}

        override fun stop() {}
    }

    private class Tokens : LinkTokenStore {
        override fun token(hubId: String): String? = null

        override fun setToken(hubId: String, name: String, token: String) {}

        override fun remove(hubId: String) {}

        override fun paired() = emptyList<PairedHub>()
    }

    private class Remote(override val bridge: DspiBridge) : RemoteSession {
        override val state = MutableStateFlow<LinkSessionState>(LinkSessionState.Idle)
        override val security = MutableStateFlow<HubSecurity?>(null)

        override fun connect(hub: LinkHub) {
            state.value = LinkSessionState.Connecting(hub)
        }

        override fun submitPin(pin: String) {}

        override fun cancelPin() {}

        override fun chooseDevice(device: LinkDevice) {}

        override fun retryNow() {}

        override fun disconnect() {}

        override fun close() {}
    }

    /** The core's two rows, so the strip is read without the native core. */
    private val words = UnsecuredWording { key ->
        Notice(
            key,
            NoticeKind.INFO,
            if (key ==
                UNSECURED_NOTICE_KEY
            ) {
                "unsecured"
            } else {
                "Info"
            },
            "",
            "",
            ""
        )
    }

    private val studio = LinkHub("6f1a", "Studio Mac", "console", "pin", 1, emptyList(), "192.168.1.20", 11915)
    private val dev = LinkDevice(0, "0123456789ABCDEF", "Living room", "1.1.6", 0, 1, "online", "usb", 0)
    private val plain = HubSecurity("6f1a", encrypted = false, hubKey = "")

    private fun TestScope.scope() = CoroutineScope(backgroundScope.coroutineContext + UnconfinedTestDispatcher(testScheduler))

    private fun TestScope.modes(remotes: MutableList<Remote>, kv: KeyValue): AppModeController = AppModeController(
        bridge,
        MutableStateFlow<UsbSessionState>(UsbSessionState.NoDevice),
        MutableStateFlow<List<DspiUsbDevice>>(emptyList()),
        scope(),
        demoFactory = { error("no demo") },
        forwardUiVisible = {},
        remoteFactory = { Remote(bridge).also { remotes += it } },
        unsecuredAcks = UnsecuredAckStore(kv)
    )

    private fun TestScope.networking(modes: AppModeController) =
        AppNetworkingController(InMemoryLinkSettingsStore(), Browser(), Tokens(), modes, scope(), "Pixel 9", parseAddress = { null })

    /** On [hub]'s DSPi with [security]; the remote session that is. */
    private fun AppModeController.onHub(remotes: List<Remote>, hub: LinkHub, security: HubSecurity): Remote {
        enterRemote(hub)
        val r = remotes.last()
        r.state.value = LinkSessionState.Connected(hub, dev, "control")
        r.security.value = security
        return r
    }

    private fun AppModeController.strip() = connectionStrip(
        AppMode.REMOTE,
        UsbSessionState.NoDevice,
        remoteState.value,
        security = remoteSecurity.value,
        unsecuredWording = words
    )
        as ConnectionStrip.Remote

    @Test
    fun wpAnsecAcknowledgingKeepsTheWordsButNotTheWarning() = runTest {
        val kv = InMemoryKeyValue()
        val remotes = mutableListOf<Remote>()
        val modes = modes(remotes, kv)
        val net = networking(modes)
        modes.onHub(remotes, studio, plain)
        modes.strip().let {
            assertTrue(it.unsecured)
            assertTrue("highlighted before the acknowledgement", it.warning)
            assertEquals("Living room via Studio Mac (unsecured)", it.text)
        }
        net.acknowledgeUnsecured()
        assertTrue(modes.remoteSecurity.value!!.acknowledged)
        modes.strip().let {
            assertTrue("still says it", it.unsecured)
            assertTrue(it.acknowledged)
            assertFalse("no longer highlighted", it.warning)
            assertEquals("Living room via Studio Mac (unsecured)", it.text)
            assertEquals(" (unsecured)", it.suffix)
        }
        assertEquals(listOf(AcknowledgedHubRow("6f1a", "Studio Mac")), net.state.value.acknowledged)
        // The info dialog's undo
        net.highlightUnsecuredAgain()
        assertTrue(modes.strip().warning)
        assertTrue(net.state.value.acknowledged.isEmpty())
    }

    @Test
    fun wpAnsecAnEncryptedOrLocalConnectionIsNeverAcknowledged() = runTest {
        val remotes = mutableListOf<Remote>()
        val modes = modes(remotes, InMemoryKeyValue())
        modes.onHub(remotes, studio, plain.copy(encrypted = true, hubKey = "SHA256:K7Q2"))
        modes.acknowledgeUnsecured()
        assertTrue(modes.unsecuredAcks.acks.value.isEmpty())
        assertFalse(modes.strip().unsecured)
        // Connecting (no live connection yet) offers nothing either
        modes.exitRemote()
        modes.enterRemote(studio)
        modes.acknowledgeUnsecured()
        assertTrue(modes.unsecuredAcks.acks.value.isEmpty())
    }

    @Test
    fun wpAnsecItIsRememberedAcrossRestartsAndDroppedWhenTheHubIsSecured() = runTest {
        val kv = InMemoryKeyValue()
        val first = mutableListOf<Remote>()
        modes(first, kv).apply {
            onHub(first, studio, plain)
            acknowledgeUnsecured()
            exitRemote()
        }
        // A restart: a new controller over the same storage
        val remotes = mutableListOf<Remote>()
        val modes = modes(remotes, kv)
        val r = modes.onHub(remotes, studio, plain)
        assertTrue(modes.strip().acknowledged)
        assertFalse(modes.strip().warning)
        // The hub is secured (paired with TLS, its key pinned): the acknowledgement goes
        r.security.value = plain.copy(encrypted = true, hubKey = "SHA256:K7Q2", keyNotice = true)
        assertFalse(modes.strip().unsecured)
        assertTrue(modes.unsecuredAcks.acks.value.isEmpty())
        // ... so unsecured again later is a change, and highlighted again
        r.security.value = plain
        assertTrue(modes.strip().warning)
        assertFalse(UnsecuredAckStore(kv).acknowledged(studio))
    }

    @Test
    fun wpAnsecAnotherHubAtATypedAddressIsHighlighted() = runTest {
        val kv = InMemoryKeyValue()
        val remotes = mutableListOf<Remote>()
        val modes = modes(remotes, kv)
        val bridgeHub = LinkHub.manual("192.168.1.30", 11915).copy(id = "9c2e", name = "Bridge")
        modes.onHub(remotes, bridgeHub, plain.copy(hubId = "9c2e"))
        modes.acknowledgeUnsecured()
        assertFalse(modes.strip().warning)
        modes.exitRemote()
        // Another hub answers at that address now
        val other = LinkHub.manual("192.168.1.30", 11915).copy(id = "d00d", name = "Impostor")
        modes.onHub(remotes, other, plain.copy(hubId = "d00d"))
        assertTrue(modes.strip().warning)
        assertTrue(modes.unsecuredAcks.acks.value.isEmpty())
    }

    @Test
    fun wpAnsecSettingsNetworkingForgetsOneOrAll() = runTest {
        val kv = InMemoryKeyValue()
        val remotes = mutableListOf<Remote>()
        val modes = modes(remotes, kv)
        val net = networking(modes)
        val bridgeHub = LinkHub.manual("192.168.1.30", 11915).copy(id = "9c2e", name = "Bridge")
        modes.onHub(remotes, bridgeHub, plain.copy(hubId = "9c2e"))
        net.acknowledgeUnsecured()
        modes.onHub(remotes, studio, plain)
        net.acknowledgeUnsecured()
        assertEquals(
            listOf(AcknowledgedHubRow("9c2e", "Bridge", "192.168.1.30:11915"), AcknowledgedHubRow("6f1a", "Studio Mac")),
            net.state.value.acknowledged
        )
        net.forgetAcknowledgement("6f1a")
        assertTrue("the current hub is highlighted again", modes.strip().warning)
        assertEquals(listOf("9c2e"), net.state.value.acknowledged.map { it.id })
        net.forgetAcknowledgements()
        assertTrue(net.state.value.acknowledged.isEmpty())
        assertTrue(UnsecuredAckStore(kv).acks.value.isEmpty())
    }
}
