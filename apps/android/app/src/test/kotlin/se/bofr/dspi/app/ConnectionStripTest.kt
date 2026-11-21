package se.bofr.dspi.app

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import se.bofr.dspi.core.DspiException
import se.bofr.dspi.core.LinkDevice
import se.bofr.dspi.core.LinkPhase
import se.bofr.dspi.link.LinkHub
import se.bofr.dspi.link.LinkSessionState
import se.bofr.dspi.usb.UsbIds
import se.bofr.dspi.usb.UsbSessionState

class ConnectionStripTest {
    @Test
    fun demoModeIsAlwaysLabelled() {
        for (s in listOf(UsbSessionState.NoDevice, UsbSessionState.Open("x", UsbIds.DSPI_VID))) {
            val strip = connectionStrip(AppMode.DEMO, s)
            assertEquals(ConnectionStrip.Demo, strip)
            assertTrue(strip.text.startsWith("Demo: a simulated DSPi") && strip.short == "Demo: a simulated DSPi")
        }
    }

    @Test
    fun deviceModeFollowsTheUsbSession() {
        fun of(s: UsbSessionState) = connectionStrip(AppMode.DEVICE, s)
        assertEquals(ConnectionStrip.NoDevice, of(UsbSessionState.NoDevice))
        assertTrue(of(UsbSessionState.PermissionRequired("")) is ConnectionStrip.NeedsPermission)
        assertTrue(of(UsbSessionState.PermissionDenied("")) is ConnectionStrip.NeedsPermission)
        assertTrue(of(UsbSessionState.PermissionPending("", dialogShown = false)) is ConnectionStrip.Info)
        assertTrue(of(UsbSessionState.Failed("x", "busy")).text.contains("busy"))
        assertEquals(ConnectionStrip.Usb, of(UsbSessionState.Open("x", UsbIds.DSPI_LEGACY_VID)))
    }

    private val hub = LinkHub("6f1a", "Studio Mac", "console", "pin", 1, emptyList(), "192.168.1.20", 11915)
    private val dev = LinkDevice(0, "0123456789ABCDEF", "Living room", "1.1.6", 0, 1, "online", "usb", 0)

    // -- WP AN7 --
    @Test
    fun an7UsbLinkAndDemoAreOneList() {
        val open = UsbSessionState.Open("x", UsbIds.DSPI_VID)
        val typed = LinkHub.manual("10.0.0.2", 11915)
        val rows = deviceList(AppMode.DEVICE, open, hubs = listOf(hub, typed), linkEnabled = true)
        assertEquals(
            listOf(DeviceChoice.Usb, DeviceChoice.Hub(hub), DeviceChoice.Hub(typed), DeviceChoice.Network, DeviceChoice.Demo),
            rows.map { it.choice }
        )
        assertEquals(listOf(true, false, false, false, false), rows.map { it.current })
        assertTrue("demo is offered with a DSPi open", rows.single { it.choice == DeviceChoice.Demo }.enabled)
        assertEquals("Studio Mac (network)", rows[1].label)
        // DSPi Link off: no hubs; nothing on USB: listed, disabled
        // -- WP ANLINK -- "DSPi Link…" (Networking) is listed with DSPi Link off too
        val off = deviceList(AppMode.DEMO, UsbSessionState.NoDevice, hubs = listOf(hub))
        assertEquals(listOf(DeviceChoice.Usb, DeviceChoice.Network, DeviceChoice.Demo), off.map { it.choice })
        assertFalse(off[0].enabled)
        assertTrue(off[1].enabled)
        assertTrue(off[2].current)
        // REMOTE marks its hub, by id or a typed hub by address
        val connected = LinkSessionState.Connected(hub.copy(id = "9c2e", host = "10.0.0.2"), dev, "control")
        val remote = deviceList(AppMode.REMOTE, open, connected, listOf(hub, typed), linkEnabled = true)
        assertEquals(listOf(false, false, true, false, false), remote.map { it.current })
        assertEquals(ConnectionStrip.Usb, connectionStrip(AppMode.DEVICE, open))
    }

    // -- WP ANLINK --
    @Test
    fun wpAnlink_noDeviceSaysBothWaysAndDspiLinkIsAlwaysListed() {
        val nd = connectionStrip(AppMode.DEVICE, UsbSessionState.NoDevice)
        assertEquals(ConnectionStrip.NoDevice, nd)
        assertEquals("No DSPi connected. Plug one in over USB, or connect over DSPi Link.", nd.text)
        assertEquals("No DSPi connected", nd.short)
        assertEquals("Connect over DSPi Link", NO_DEVICE_LINK_ACTION)
        // the owner's naming: "DSPi Link" whole, never a bare "link" for it
        for (t in listOf(nd.text, NO_DEVICE_LINK_ACTION)) {
            assertFalse(t, Regex("(?<!DSPi )\\b[Ll]ink\\b").containsMatchIn(t))
        }
        val open = UsbSessionState.Open("x", UsbIds.DSPI_VID)
        for (usb in listOf(UsbSessionState.NoDevice, open)) {
            for (mode in AppMode.entries) {
                val rows = deviceList(mode, usb, hubs = listOf(hub), linkEnabled = false)
                val link = rows.single { it.choice == DeviceChoice.Network }
                assertEquals("DSPi Link…", link.label)
                assertTrue(link.enabled && !link.current)
                assertTrue("hubs only while DSPi Link is on", rows.none { it.choice is DeviceChoice.Hub })
            }
        }
    }

    @Test
    fun remoteModeIsAlwaysLabelledWithItsHub() {
        fun of(s: LinkSessionState, lock: Boolean = false) = connectionStrip(
            AppMode.REMOTE,
            UsbSessionState.Open("x", UsbIds.DSPI_VID),
            s,
            lockNotice = lock
        ) as ConnectionStrip.Remote
        assertEquals("Living room via Studio Mac", of(LinkSessionState.Connected(hub, dev, "control")).text)
        assertEquals("Living room via Studio Mac · view only", of(LinkSessionState.Connected(hub, dev, "viewer")).text)
        assertEquals(DspiException.LOCKED_MESSAGE, of(LinkSessionState.Connected(hub, dev, "control"), lock = true).text)
        assertTrue(of(LinkSessionState.Connecting(hub)).text.startsWith("Connecting to Studio Mac"))
        assertEquals("Enter the hub's PIN. · Studio Mac", of(LinkSessionState.NeedsPin(hub, "fixed")).text)
        val r = of(LinkSessionState.Reconnecting(hub, "hub shutting down", 0, 1, "Connection lost, reconnecting"))
        assertEquals("Connection lost, reconnecting", r.text)
        assertEquals("Studio Mac: hub shutting down", r.detail)
        assertTrue(r.lost)
        assertEquals(listOf(RemoteAction.RETRY, RemoteAction.DISCONNECT), r.actions)
        assertEquals(listOf(RemoteAction.CONNECT, RemoteAction.DISCONNECT), of(LinkSessionState.Failed(hub, "revoked")).actions)
        assertTrue(of(LinkSessionState.WaitingForDevice(hub, "0123456789ABCDEF")).text.contains("89ABCDEF"))
    }

    // -- WP AN29 --
    @Test
    fun an29PermissionStripsTellAboutAlwaysOpenWithoutDeviceNouns() {
        val required = connectionStrip(AppMode.DEVICE, UsbSessionState.PermissionRequired(""))
        val pending = connectionStrip(AppMode.DEVICE, UsbSessionState.PermissionPending("", dialogShown = true))
        for (strip in listOf(required, pending)) {
            assertTrue(strip.text, strip.text.contains("\"Always open\""))
            assertTrue(strip.text, strip.text.endsWith("Tick \"Always open\" in Android's dialog and it will not ask again."))
            for (noun in listOf("phone", "tablet", "computer")) assertFalse(strip.text, strip.text.lowercase().contains(noun))
        }
    }

    // -- WP AN5 --
    @Test
    fun wpAn5TheCoresWordingByPhase() {
        fun of(s: LinkSessionState) = remoteStrip(s)
        assertEquals("Not connected to a hub", of(LinkSessionState.Idle).text)
        assertEquals("Connected to hub, no DSPi found · Studio Mac", of(LinkSessionState.WaitingForDevice(hub, "")).text)
        assertEquals("Connected to hub, choose a DSPi · Studio Mac", of(LinkSessionState.ChooseDevice(hub, listOf(dev, dev))).text)
        assertTrue(of(LinkSessionState.NeedsPin(hub, "window")).text.startsWith("Enter the PIN shown on the hub."))
        assertTrue(of(LinkSessionState.NeedsPin(hub, "")).text.startsWith("Enter the PIN shown on the hub."))
        // Only a lost connection is the banner
        val states = listOf(
            LinkSessionState.Idle,
            LinkSessionState.Connecting(hub),
            LinkSessionState.NeedsPin(hub, "window"),
            LinkSessionState.ChooseDevice(hub, listOf(dev)),
            LinkSessionState.WaitingForDevice(hub, ""),
            LinkSessionState.Connected(hub, dev, "admin"),
            LinkSessionState.Failed(hub, "x")
        )
        states.forEach { assertFalse(it.toString(), of(it).lost) }
        assertEquals("Studio Mac", of(LinkSessionState.Reconnecting(hub, "", 0, 1, "lost")).detail)
    }

    @Test
    fun wpAn5TheWayOutIsCalledWhatTheCoreCallsIt() {
        assertEquals(LinkWording.CORE.of(LinkPhase.RECONNECTING, "").action, RemoteAction.DISCONNECT.label)
        assertEquals("reconnecting.disconnect", LinkWording.CORE.of(LinkPhase.RECONNECTING, "").actionKey)
    }

    @Test
    fun wpAn5WrongPinHintBySource() {
        assertEquals("Check the hub's PIN and try again.", wrongPinHint("fixed"))
        for (source in listOf("window", "")) assertTrue(wrongPinHint(source).contains("get a new one"))
        for (source in listOf("window", "fixed")) {
            for (noun in listOf("phone", "tablet", "computer")) assertFalse(wrongPinHint(source).lowercase().contains(noun))
        }
    }
}
