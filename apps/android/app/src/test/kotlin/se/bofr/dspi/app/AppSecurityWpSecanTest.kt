package se.bofr.dspi.app

import java.lang.reflect.Proxy
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.UnconfinedTestDispatcher
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test
import se.bofr.dspi.bridge.DspiBridge
import se.bofr.dspi.core.LinkDevice
import se.bofr.dspi.core.LinkPairLink
import se.bofr.dspi.link.HubBrowser
import se.bofr.dspi.link.HubSecurity
import se.bofr.dspi.link.InMemoryLinkSettingsStore
import se.bofr.dspi.link.LinkHub
import se.bofr.dspi.link.LinkSessionState
import se.bofr.dspi.link.LinkTokenStore
import se.bofr.dspi.link.PairedHub
import se.bofr.dspi.usb.DspiUsbDevice
import se.bofr.dspi.usb.UsbSessionState

/**
 * WP SECAN: the app's plumbing of the hub security: the hub list's badges and the paired
 * hubs' keys (Settings > Networking), the session's security reaching the mode controller,
 * and a confirmed pairing link dialled as one.
 */
@OptIn(ExperimentalCoroutinesApi::class)
class AppSecurityWpSecanTest {
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
        val m = HashMap<String, String>()

        override fun token(hubId: String) = m[hubId]

        override fun setToken(hubId: String, name: String, token: String) {
            m[hubId] = token
        }

        override fun remove(hubId: String) {
            m.remove(hubId)
        }

        override fun paired() = m.keys.sorted().map { PairedHub(it, "Hub $it") }
    }

    private class Remote(override val bridge: DspiBridge) : RemoteSession {
        override val state = MutableStateFlow<LinkSessionState>(LinkSessionState.Idle)
        override val security = MutableStateFlow<HubSecurity?>(null)
        val links = mutableListOf<LinkPairLink>()

        override fun connect(hub: LinkHub) {
            state.value = LinkSessionState.Connecting(hub)
        }

        override fun connectByLink(link: LinkPairLink) {
            links += link
            state.value =
                LinkSessionState.Connecting(LinkHub(link.hubId, link.host, "unknown", "pin", 0, emptyList(), link.host, link.port))
        }

        override fun submitPin(pin: String) {}

        override fun cancelPin() {}

        override fun chooseDevice(device: LinkDevice) {}

        override fun retryNow() {}

        override fun disconnect() {}

        override fun close() {}
    }

    private fun TestScope.modes(remotes: MutableList<Remote>): AppModeController = AppModeController(
        bridge,
        MutableStateFlow<UsbSessionState>(UsbSessionState.NoDevice),
        MutableStateFlow<List<DspiUsbDevice>>(emptyList()),
        CoroutineScope(backgroundScope.coroutineContext + UnconfinedTestDispatcher(testScheduler)),
        demoFactory = { error("no demo") },
        forwardUiVisible = {},
        remoteFactory = { Remote(bridge).also { remotes += it } }
    )

    @Test
    fun wpSecanTheHubListSaysWhichHubsAreVerifiedEncryptedOrNot() = runTest {
        val browser = Browser()
        val tokens = Tokens()
        val modes = modes(mutableListOf())
        val c = AppNetworkingController(
            InMemoryLinkSettingsStore(),
            browser,
            tokens,
            modes,
            CoroutineScope(backgroundScope.coroutineContext + UnconfinedTestDispatcher(testScheduler)),
            "Pixel 9",
            parseAddress = { null },
            credentialKey = { if (it.startsWith("v2.")) "SHA256:K7Q2-M9XD-4TNB-0ZRA" else "" }
        )
        fun hub(id: String, tls: Boolean = false) =
            LinkHub(id, "Hub $id", "console", "pin", 1, emptyList(), "10.0.0.$id", 11915, tlsAdvertised = tls)
        tokens.setToken("1", "Hub 1", "v2.dG9r.key.t")
        tokens.setToken("3", "Hub 3", "bare-v1-token")
        browser.hubs.value = listOf(hub("1", tls = true), hub("2", tls = true), hub("3"))
        c.setVisible(true)
        assertEquals(listOf("Verified", "Encrypted", "Unencrypted"), c.state.value.hubs.map { it.badge })
        assertEquals(listOf("SHA256:K7Q2-M9XD-4TNB-0ZRA", ""), c.state.value.paired.map { it.hubKey })
    }

    @Test
    fun wpSecanAPairingLinkIsDialledAndTheSecurityFollows() = runTest {
        val remotes = mutableListOf<Remote>()
        val modes = modes(remotes)
        val link = LinkPairLink("6f1a", "192.168.1.20", 11915, "ws://192.168.1.20:11915/dspi/v1", "SHA256:K7Q2", "v2..x.t", "482913")
        modes.pairByLink(link)
        assertEquals(AppMode.REMOTE, modes.mode.value)
        assertEquals(listOf(link), remotes.single().links)
        assertEquals("6f1a", modes.remoteState.value.hub?.id)
        val plain = HubSecurity("6f1a", encrypted = false, hubKey = "")
        remotes.single().security.value = plain
        assertEquals(plain, modes.remoteSecurity.value)
        modes.exitRemote()
        assertNull(modes.remoteSecurity.value)
    }
}
