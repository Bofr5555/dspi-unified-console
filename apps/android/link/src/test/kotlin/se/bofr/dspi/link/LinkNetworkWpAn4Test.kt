package se.bofr.dspi.link

import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.launch
import kotlinx.coroutines.test.UnconfinedTestDispatcher
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/** WP AN4: the platform's callbacks and DNS-SD as edges, with a fake monitor and browser. */
@OptIn(ExperimentalCoroutinesApi::class)
class LinkNetworkWpAn4Test {
    private class Monitor {
        val log = ArrayList<String>()
        val edges = LinkNetworkEdges({ log += "bind($it)" }, { log += "available" }, { log += "lost" })
    }

    @Test
    fun wpAn4ANetworkIsOneEdgeHoweverOftenTheCallbackRepeatsIt() {
        val m = Monitor()
        m.edges.available(1)
        m.edges.available(1)
        m.edges.validated(1, true)
        m.edges.validated(1, true)
        assertEquals(listOf("available"), m.log)
        m.edges.lost(1)
        m.edges.lost(1)
        m.edges.lost(2)
        assertEquals(listOf("available", "lost"), m.log)
        m.edges.available(1)
        assertEquals("back: an edge again", listOf("available", "lost", "available"), m.log)
        m.edges.validated(9, true)
        assertEquals("an unknown network says nothing", 3, m.log.size)
    }

    @Test
    fun wpAn4NoBindingWithoutASessionOrWithValidatedWifi() {
        val m = Monitor()
        m.edges.available(1)
        m.edges.setSessionActive(false)
        assertEquals(listOf("available"), m.log)
        m.edges.validated(1, true)
        m.edges.setSessionActive(true)
        assertEquals("the default route goes there anyway", listOf("available"), m.log)
    }

    @Test
    fun wpAn4WifiWithoutInternetIsBoundForASessionBeforeItsDial() {
        val m = Monitor()
        m.edges.setSessionActive(true)
        m.edges.available(1)
        assertEquals("bound before the edge starts a dial", listOf("bind(1)", "available"), m.log)
        m.edges.validated(1, false)
        assertEquals(2, m.log.size)
        m.edges.validated(1, true)
        assertEquals("bind(null)", m.log.last())
        m.edges.validated(1, false)
        assertEquals("bind(1)", m.log.last())
        m.edges.setSessionActive(false)
        assertEquals("the session over: the process is free again", "bind(null)", m.log.last())
    }

    @Test
    fun wpAn4LosingTheBoundNetworkLiftsOrMovesTheBinding() {
        val m = Monitor()
        m.edges.setSessionActive(true)
        m.edges.available(1)
        m.edges.available(2)
        assertEquals("bind(2)", m.log[m.log.size - 2])
        m.edges.lost(2)
        assertEquals(listOf("bind(1)", "lost"), m.log.takeLast(2))
        m.edges.lost(1)
        assertEquals(listOf("bind(null)", "lost"), m.log.takeLast(2))
        m.edges.available(3)
        m.edges.reset()
        assertEquals("bind(null)", m.log.last())
    }

    private val hub = LinkHub("hub-1", "Hub", "console", "pin", 1, emptyList(), "10.0.0.2", 11915)

    @Test
    fun wpAn4ASightingIsAnEdgeNotAState() {
        val s = HubSightings()
        assertEquals("first seen", hub, s.next("hub-1", listOf(hub)))
        assertNull("still there", s.next("hub-1", listOf(hub)))
        assertNull(s.next("hub-1", listOf(hub.copy(deviceCount = 3))))
        val moved = hub.copy(host = "10.0.0.3")
        assertEquals("another address", moved, s.next("HUB-1", listOf(moved)))
        assertNull("gone", s.next("hub-1", emptyList()))
        assertEquals("back at the same address", moved, s.next("hub-1", listOf(moved)))
        assertNull("not the wanted one", s.next("hub-2", listOf(moved)))
        assertNull("no session", s.next(null, listOf(moved)))
    }

    @Test
    fun wpAn4BrowsingFollowsTheSessionNotTheAppBeingOpen() {
        val conn = LinkSessionState.Connecting(hub)
        val failed = LinkSessionState.Failed(hub, "x")
        val back = LinkSessionState.Reconnecting(hub, "", 0, 1, "lost")
        assertTrue(linkBrowseWanted(back, uiVisible = false))
        assertFalse("parked", linkBrowseWanted(back.copy(parked = true), uiVisible = false))
        assertTrue(linkBrowseWanted(failed, uiVisible = true))
        assertFalse(linkBrowseWanted(failed, uiVisible = false))
        assertTrue("stays on across the dial, or a failed dial would see the hub as new", linkBrowseWanted(conn, uiVisible = true))
        assertFalse(linkBrowseWanted(conn, uiVisible = false))
        assertFalse(linkBrowseWanted(LinkSessionState.Idle, uiVisible = true))
    }

    private class Browser : HubBrowser {
        override val hubs = MutableStateFlow<List<LinkHub>>(emptyList())
        override val error = MutableStateFlow<String?>(null)
        override val browsing = MutableStateFlow(true)
        var refreshes = 0

        override fun start() {}

        override fun stop() {}

        override fun refresh() {
            refreshes++
            hubs.value = emptyList()
        }
    }

    private class Target : LinkNetworkTarget {
        val log = ArrayList<String>()

        override fun networkAvailable() {
            log += "available"
        }

        override fun networkLost() {
            log += "lost"
        }

        override fun hubSeen(found: LinkHub) {
            log += "seen ${found.host}"
        }
    }

    @Test
    fun wpAn4TheFollowerHandsOnEdgesAndLooksAgainOnlyWhenItMatters() = runTest(UnconfinedTestDispatcher()) {
        val browser = Browser()
        val state = MutableStateFlow<LinkSessionState>(LinkSessionState.Connecting(hub))
        val target = Target()
        val f = LinkNetworkFollower(browser, state) { target }
        backgroundScope.launch { f.follow() }

        // Connecting and connected: the browser is left alone and a listing is no edge to act on twice
        browser.hubs.value = listOf(hub)
        assertEquals(listOf("seen 10.0.0.2"), target.log)
        f.networkAvailable()
        f.uiShown()
        assertEquals(0, browser.refreshes)

        state.value = LinkSessionState.Failed(hub, "x")
        assertEquals("the hub stays listed: no second dial", 2, target.log.size)
        f.networkAvailable()
        assertEquals(1, browser.refreshes)

        state.value = LinkSessionState.Connected(hub, se.bofr.dspi.core.LinkDevice(0, "S", "B", "", -1, -1, "online", "usb", 0), "admin")
        val refreshes = browser.refreshes
        browser.hubs.value = listOf(hub)
        state.value = LinkSessionState.Reconnecting(hub, "", 0, 1, "lost")
        assertEquals("a lost session looks afresh once", refreshes + 1, browser.refreshes)
        state.value = LinkSessionState.Reconnecting(hub, "", 0, 2, "lost")
        assertEquals("not for every attempt", refreshes + 1, browser.refreshes)

        val before = target.log.size
        browser.hubs.value = listOf(hub.copy(host = "10.0.0.3"))
        assertEquals("seen 10.0.0.3", target.log[before])
        f.networkLost()
        assertEquals("lost", target.log.last())
    }

    @Test
    fun wpAn4NoSessionNothingToTell() = runTest(UnconfinedTestDispatcher()) {
        val browser = Browser()
        val state = MutableStateFlow<LinkSessionState>(LinkSessionState.Idle)
        val f = LinkNetworkFollower(browser, state) { null }
        backgroundScope.launch { f.follow() }
        browser.hubs.value = listOf(hub)
        f.networkAvailable()
        f.networkLost()
        f.uiShown()
        assertEquals(0, browser.refreshes)
    }
}
