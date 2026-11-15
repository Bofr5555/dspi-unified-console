package se.bofr.dspi.feature.presets.snapshots

import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import se.bofr.dspi.bridge.HubServer
import se.bofr.dspi.fake.FakeHubDspiCore
import se.bofr.dspi.feature.presets.attached
import se.bofr.dspi.feature.presets.press

/**
 * WP P2AN: the Snapshots screen's "On the hub" section on the fake's hub
 * model. Only a hub with `hub_snapshots` has the section (nothing asked
 * otherwise); "Save to the hub" files this console's snapshot there; "Copy
 * here" brings the hub's here; Recall of a hub's snapshot copies it here first
 * and recalls it as any other (the same way back); the hub's wins on a
 * differing id and this console's is marked as a local copy; another
 * console's change (`snapshot.changed`) shows without a reload; Delete from
 * the hub keeps this console's copy.
 */
@OptIn(ExperimentalCoroutinesApi::class)
class SnapshotsHubWpP2anTest {
    private fun FakeHubDspiCore.hubCalls() = calls.filter { it.startsWith("hubSnapshots") || it == "snapshotListMerged" }

    @Test
    fun wpP2anWithoutTheCapThereIsNoHubSectionAndNothingIsAsked() = runTest {
        val core = FakeHubDspiCore(hubCaps = setOf("events"))
        val b = attached(core)
        val m = SnapshotsModel(b, backgroundScope, folder = null)
        core.snapshotCapture("Warm", "")
        m.load()
        runCurrent()
        assertEquals(listOf("Warm"), m.items.value.map { it.name })
        assertEquals(emptyList<Any>(), m.hubItems.value)
        assertEquals(emptyMap<String, LocalMark>(), m.localMarks.value)
        assertEquals(emptyList<String>(), core.hubCalls())
    }

    @Test
    fun wpP2anSaveToTheHubCopyHereAndRecallFromTheHub() = runTest {
        val core = FakeHubDspiCore()
        val b = attached(core)
        runCurrent()
        val m = SnapshotsModel(b, backgroundScope, folder = null)
        val warm = core.snapshotCapture("Warm", "")
        val venue = core.hubPutRemotely("Venue default")
        m.load()
        runCurrent()
        assertEquals(listOf("Venue default"), m.hubItems.value.map { it.snapshot.name })
        assertEquals(LocalMark.HERE_ONLY, m.localMarks.value[warm.id])
        // Save to the hub: both sides now, listed on the hub as "also here"
        m.saveToHub(warm)
        runCurrent()
        assertEquals("Saved \"Warm\" to the hub.", m.result.value)
        assertEquals(LocalMark.ON_HUB_TOO, m.localMarks.value[warm.id])
        assertEquals(
            mapOf(warm.id to HubServer.WHERE_BOTH, venue.id to HubServer.WHERE_HUB),
            m.hubItems.value.associate { it.snapshot.id to it.location }
        )
        // Recall of a hub-only snapshot: copied here, then recalled as any other
        m.recallFromHub(m.hubItems.value.first { it.snapshot.id == venue.id })
        runCurrent()
        assertTrue(core.calls.toString(), core.calls.contains("hubSnapshotsCopyHere(${venue.id})"))
        assertTrue(core.calls.contains("snapshotRecall(${venue.id})"))
        assertEquals("Recalled \"Venue default\": 1 change.", m.result.value)
        assertEquals(setOf(warm.id, venue.id), m.items.value.map { it.id }.toSet())
        // Recall of one both sides hold: this console's copy, nothing fetched
        val copies = core.calls.count { it.startsWith("hubSnapshotsCopyHere") }
        m.recallFromHub(m.hubItems.value.first { it.snapshot.id == warm.id })
        runCurrent()
        assertEquals(copies, core.calls.count { it.startsWith("hubSnapshotsCopyHere") })
        assertTrue(core.calls.contains("snapshotRecall(${warm.id})"))
    }

    @Test
    fun wpP2anTheHubsWinsADifferingIdAndOtherConsolesChangesShow() = runTest {
        val core = FakeHubDspiCore()
        val b = attached(core)
        runCurrent()
        val m = SnapshotsModel(b, backgroundScope, folder = null)
        val mine = core.snapshotCapture("Mine", "")
        m.load()
        runCurrent()
        // Another console files a different snapshot under the same id: event only, no reload asked
        core.hubPutRemotely("Theirs", id = mine.id)
        runCurrent()
        assertEquals(listOf("Theirs"), m.hubItems.value.map { it.snapshot.name })
        assertEquals(LocalMark.LOCAL_COPY, m.localMarks.value[mine.id])
        assertEquals("Mine", m.items.value.single().name)
        assertEquals("Local copy: the hub holds a different one", markLine(LocalMark.LOCAL_COPY))
        // Saving it again is the hub's refusal, in its words; nothing overwritten
        m.saveToHub(mine)
        runCurrent()
        assertEquals("Hub: the hub holds a different snapshot with this id", m.host.alert.value?.informativeText)
        m.host.press("OK")
        assertEquals("Theirs", core.hub[mine.id]?.name)
    }

    @Test
    fun wpP2anRenameNoteAndDeleteOnTheHub() = runTest {
        val core = FakeHubDspiCore()
        val b = attached(core)
        runCurrent()
        val m = SnapshotsModel(b, backgroundScope, folder = null)
        val s = core.snapshotCapture("Warm", "")
        core.hubSnapshotsSave(s.id)
        m.load()
        runCurrent()
        m.askRenameOnHub(s)
        m.host.press("Rename", se.bofr.dspi.ui.alert.AlertResponse(text = "Warm (hall)"))
        runCurrent()
        assertEquals("Warm (hall)", m.hubItems.value.single().snapshot.name)
        assertEquals("Warm", m.items.value.single().name)
        m.askNoteOnHub(m.hubItems.value.single().snapshot)
        m.host.press("Save", se.bofr.dspi.ui.alert.AlertResponse(text = "Evening"))
        runCurrent()
        assertEquals("Evening", core.hub[s.id]?.note)
        m.askDeleteFromHub(m.hubItems.value.single().snapshot)
        m.host.press("Delete")
        runCurrent()
        assertEquals(emptyList<Any>(), m.hubItems.value)
        assertEquals("this console's copy stays", listOf("Warm"), m.items.value.map { it.name })
        // A Control session may not delete from the hub (Admin only): the hub's refusal
        core.role = "control"
        core.hubSnapshotsSave(s.id)
        m.refreshHub()
        runCurrent()
        m.askDeleteFromHub(m.hubItems.value.single().snapshot)
        m.host.press("Delete")
        runCurrent()
        assertEquals("Not allowed: This console's role on the hub does not allow that.", m.host.alert.value?.informativeText)
    }
}
