package se.bofr.dspi.feature.settings

import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.StandardTestDispatcher
import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import se.bofr.dspi.bridge.DspiBridgeImpl
import se.bofr.dspi.core.DspiConstants
import se.bofr.dspi.fake.FakeHubDspiCore

/**
 * WP P2AN: Settings › History on the fake's hub model. This console's own log
 * is shown by default, newest first; "On the hub" is offered only by a hub
 * with the `events` cap and reads the hub's history with who did it (`by`);
 * while the page is open the hub's new records arrive (`event.watch`), and
 * closing it stops the watch; the page is in the System group.
 */
@OptIn(ExperimentalCoroutinesApi::class)
class EventsPageWpP2anTest {
    private suspend fun TestScope.page(core: FakeHubDspiCore): EventsModel {
        val d = StandardTestDispatcher(testScheduler)
        val bridge = DspiBridgeImpl(core, backgroundScope, ffi = d, main = d, clock = { testScheduler.currentTime })
        bridge.attachDevice(42, core.serial, DspiConstants.VENDOR_ID)
        runCurrent()
        bridge.hubServer
        runCurrent()
        return EventsModel(bridge, backgroundScope)
    }

    @Test
    fun wpP2anThisConsolesLogFirstThenTheHubsWithWhoDidIt() = runTest {
        val core = FakeHubDspiCore()
        core.localAddEvent("Output 1 gain -4 dB")
        core.localAddEvent("Preset 1 recalled", kind = "recall")
        core.hubAddEvent("Output 1 gain -6 dB", by = "Front of house")
        val m = page(core)
        m.open()
        runCurrent()
        assertEquals(EventsSource.CONSOLE, m.source.value)
        assertEquals(listOf("Preset 1 recalled", "Output 1 gain -4 dB"), m.rows.value.map { it.text })
        assertEquals(EventRow.THIS_CONSOLE, m.rows.value.first().by)
        assertTrue(m.hubOffered)
        m.choose(EventsSource.HUB)
        runCurrent()
        assertEquals(listOf("Output 1 gain -6 dB" to "Front of house"), m.rows.value.map { it.text to it.by })
        assertEquals("watching while open", core.serial, core.watching)
        core.hubAddEvent("Preset 2 recalled", by = "Schedule: evening")
        runCurrent()
        assertEquals(listOf("Schedule: evening", "Front of house"), m.rows.value.map { it.by })
        m.close()
        runCurrent()
        assertEquals(null, core.watching)
        assertEquals("On the hub", sourceLabel(EventsSource.HUB))
    }

    @Test
    fun wpP2anAHubWithoutTheEventsCapOffersOnlyThisConsolesLog() = runTest {
        val core = FakeHubDspiCore(hubCaps = setOf("hub_snapshots"))
        core.localAddEvent("Output 1 gain -4 dB")
        val m = page(core)
        m.open()
        runCurrent()
        assertFalse(m.hubOffered)
        m.choose(EventsSource.HUB)
        runCurrent()
        assertEquals(EventsSource.CONSOLE, m.source.value)
        assertEquals(listOf("Output 1 gain -4 dB"), m.rows.value.map { it.text })
        assertTrue("nothing asked of the hub", core.calls.none { it.startsWith("hubEvents") })
    }

    @Test
    fun wpP2anHistoryIsASystemPageShownWithADevice() {
        assertTrue(SettingsCategory.EVENTS in SettingsCategory.groups.first { it.first == "System" }.second)
        assertTrue(SettingsCategory.EVENTS.isAvailable(SettingsAvailability(connected = true)))
        assertFalse(SettingsCategory.EVENTS.isAvailable(SettingsAvailability(connected = false)))
        assertFalse(SettingsCategory.EVENTS.isAvailable(SettingsAvailability(connected = true, lockedUser = true)))
    }
}
