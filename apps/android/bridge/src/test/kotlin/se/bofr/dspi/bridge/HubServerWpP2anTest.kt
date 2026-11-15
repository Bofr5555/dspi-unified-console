package se.bofr.dspi.bridge

import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.launch
import kotlinx.coroutines.test.StandardTestDispatcher
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import se.bofr.dspi.core.HubAlarm
import se.bofr.dspi.core.HubEventRecord
import se.bofr.dspi.fake.FakeHubDspiCore

/**
 * WP P2AN in the bridge, on the fake's hub model: the caps are read per
 * device; a hub without a family is asked nothing; the alarms are the open
 * device's and the hub's own, announced once, acknowledged through the hub;
 * Identify goes through the hub with the cap and to the device directly; the
 * hub's history is read and stays live while watched; `snapshot.changed`
 * reaches the screens; the hub's refusals come back in the core's words.
 */
@OptIn(ExperimentalCoroutinesApi::class)
class HubServerWpP2anTest {
    private fun FakeHubDspiCore.hubCallsSince(mark: Int) = calls.drop(mark).filter { it.startsWith("hub") || it == "snapshotListMerged" }

    @Test
    fun wpP2anCapsAreReadAndAHubWithoutTheFamiliesIsAskedNothing() = runTest {
        val core = FakeHubDspiCore(hubCaps = emptySet())
        val b = newBridge(core)
        attach(b, core)
        val h = b.hubServer
        runCurrent()
        assertTrue(h.caps.value.onHub)
        assertFalse(h.caps.value.alarms || h.caps.value.events || h.caps.value.hubSnapshots || h.caps.value.identify)
        assertFalse("no Identify on a hub without the cap", h.identifyOffered)
        val mark = core.calls.size
        h.refreshAlarms()
        h.watchEvents(true)
        runCurrent()
        assertTrue(h.identify(10).isFailure)
        assertEquals(emptyList<HubAlarm>(), h.alarms.value)
        assertEquals("nothing sent", emptyList<String>(), core.hubCallsSince(mark))
        assertFalse(h.mergedSnapshots().hubListed)
    }

    @Test
    fun wpP2anAlarmsAreTheDevicesAndTheHubsAnnouncedOnceAndAcknowledged() = runTest {
        val core = FakeHubDspiCore()
        val other = core.hubRaiseAlarm("device_offline").copy(id = "device:99", serial = "OTHER", subject = "OTHER")
        core.hubAlarmList[0] = other
        val mine = core.hubRaiseAlarm("device_restarted", severity = "critical")
        val b = newBridge(core)
        attach(b, core)
        val h = b.hubServer
        val seen = mutableListOf<HubAlarm>()
        backgroundScope.launch(StandardTestDispatcher(testScheduler)) { h.raised.collect { seen += it } }
        runCurrent()
        assertEquals("another device's alarm is not this console's", listOf(mine.id), h.alarms.value.map { it.id })
        assertEquals(listOf(mine.id), seen.map { it.id })
        // The hub raises one about no one device (the audio link): shown; raised again: not announced again
        val alink = core.hubRaiseAlarm("alink_lost", forDevice = false)
        runCurrent()
        assertEquals(listOf(alink.id, mine.id), h.alarms.value.map { it.id })
        core.listener()?.onHubServerEvent(hubEvent(alink.copy(count = 2)))
        runCurrent()
        assertEquals(listOf(mine.id, alink.id), seen.map { it.id })
        // Acknowledged here: a restart alarm is cleared by it and leaves the list
        val acked = h.acknowledge(mine.id).getOrThrow()
        runCurrent()
        assertEquals("cleared", acked.state)
        assertEquals(listOf(alink.id), h.alarms.value.map { it.id })
        assertEquals("Alarm device_restarted", h.alarmNotice(mine)?.title)
    }

    @Test
    fun wpP2anAViewersAcknowledgeIsRefusedInTheCoresWords() = runTest {
        val core = FakeHubDspiCore().apply { role = "viewer" }
        val a = core.hubRaiseAlarm("temperature_high")
        val b = newBridge(core)
        attach(b, core)
        runCurrent()
        val r = b.hubServer.acknowledge(a.id)
        assertTrue(r.isFailure)
        assertEquals("Not allowed: This console's role on the hub does not allow that.", r.exceptionOrNull()?.message)
        assertEquals("raised", core.hubAlarmList.first().state)
    }

    @Test
    fun wpP2anIdentifyGoesThroughTheHubOrToTheDeviceDirectly() = runTest {
        val viaHub = FakeHubDspiCore()
        val b1 = newBridge(viaHub)
        attach(b1, viaHub)
        runCurrent()
        assertTrue(b1.hubServer.identifyOffered)
        assertEquals(10, b1.hubServer.identify(10).getOrThrow()?.remainingS)
        assertEquals(listOf(10 to null), viaHub.identified)
        val direct = FakeHubDspiCore(onHub = false)
        val b2 = newBridge(direct)
        attach(b2, direct)
        runCurrent()
        assertTrue("connected directly: the device's own EXT_IDENTIFY", b2.hubServer.identifyOffered)
        assertTrue(b2.hubServer.identify(0).isSuccess)
        assertEquals(listOf(0 to null), direct.identified)
        // Not while the bridge refuses writes: nothing sent
        b2.setRemoteReadOnly(true)
        runCurrent()
        assertTrue(b2.hubServer.identify(10).isFailure)
        assertEquals(1, direct.identified.size)
    }

    @Test
    fun wpP2anTheHubsHistoryIsReadAndStaysLiveWhileWatched() = runTest {
        val core = FakeHubDspiCore()
        core.hubAddEvent("Output 1 gain -6 dB", by = "Front of house")
        core.localAddEvent("Output 1 gain -4 dB")
        val b = newBridge(core)
        attach(b, core)
        val h = b.hubServer
        runCurrent()
        assertEquals(listOf("Output 1 gain -4 dB"), h.localEvents().map { it.text })
        val page = h.hubEvents().getOrThrow()
        assertEquals(listOf("Front of house"), page.records.map { it.by })
        val added = mutableListOf<HubEventRecord>()
        backgroundScope.launch(StandardTestDispatcher(testScheduler)) { h.eventAdded.collect { added += it } }
        h.watchEvents(true)
        runCurrent()
        assertEquals(core.serial, core.watching)
        core.hubAddEvent("Preset 2 recalled", by = "Schedule: evening")
        runCurrent()
        assertEquals(listOf("Schedule: evening"), added.map { it.by })
        h.watchEvents(false)
        runCurrent()
        assertEquals(null, core.watching)
    }

    @Test
    fun wpP2anSnapshotChangesReachTheScreensAndTheMergeIsTheCores() = runTest {
        val core = FakeHubDspiCore()
        val b = newBridge(core)
        attach(b, core)
        val h = b.hubServer
        val changed = mutableListOf<String>()
        backgroundScope.launch(StandardTestDispatcher(testScheduler)) { h.snapshotsChanged.collect { changed += it } }
        runCurrent()
        val mine = core.snapshotCapture("Warm", "")
        assertTrue(h.saveToHub(mine.id).getOrThrow().created)
        val theirs = core.hubPutRemotely("Venue default")
        runCurrent()
        assertEquals(listOf(mine.id, theirs.id), changed)
        val m = h.mergedSnapshots()
        assertTrue(m.hubListed)
        assertEquals(
            mapOf(mine.id to HubServer.WHERE_BOTH, theirs.id to HubServer.WHERE_HUB),
            m.entries.associate {
                it.snapshot.id to
                    it.location
            }
        )
        val copy = h.copyHere(theirs.id).getOrThrow()
        assertEquals(theirs.id, copy.id)
        assertTrue(h.mergedSnapshots().entries.all { it.location == HubServer.WHERE_BOTH })
    }

    private fun FakeHubDspiCore.listener() = serverListener

    private fun hubEvent(a: HubAlarm) = se.bofr.dspi.core.HubServerEvent("alarm.changed", a.serial, null, null, "", "", a, null)
}
