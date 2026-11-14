package se.bofr.dspi.feature.home.system

import kotlin.time.Duration.Companion.seconds
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.StandardTestDispatcher
import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.advanceUntilIdle
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import se.bofr.dspi.bridge.DspiBridgeImpl
import se.bofr.dspi.core.DspiConstants
import se.bofr.dspi.fake.FakeDspiCore

/**
 * WP LKUI1: the System screen's model over the bridge and the fake core's audio
 * link installation (FakeAlinkSystem: `uconsole --demo hub --alink` as the
 * core gives it): the view, recalls and the refused one, the draft guarded by its
 * revision, and what the roles and G1 leave on offer.
 */
@OptIn(ExperimentalCoroutinesApi::class)
class SystemScreenModelWpLkui1Test {
    private class Rig(scope: TestScope) {
        val core = FakeDspiCore()
        val d = StandardTestDispatcher(scope.testScheduler)
        val bridge = DspiBridgeImpl(core, scope.backgroundScope, ffi = d, main = d, clock = { scope.testScheduler.currentTime })

        // The test's own scope: advanceUntilIdle waits for the model's actions
        val model = SystemScreenModel(bridge, scope)
        val system get() = core.alinkSystem
    }

    private suspend fun TestScope.rig(onHub: Boolean = true, role: String = "admin"): Rig {
        val r = Rig(this)
        assertTrue(r.bridge.attachDevice(42, r.core.serial, DspiConstants.VENDOR_ID))
        advanceUntilIdle()
        r.system.onHub = onHub
        r.system.role = role
        r.model.refresh()
        advanceUntilIdle()
        return r
    }

    @Test
    fun wpLkui1OffAHubThereIsNoView() = runTest(timeout = 20.seconds) {
        val r = rig(onHub = false)
        val s = r.model.state.value
        assertNull(s.view)
        assertFalse(s.available)
        assertEquals("", s.error)
    }

    @Test
    fun wpLkui1TheViewShowsLockedUnitsWithTheirLinkDelay() = runTest(timeout = 20.seconds) {
        val r = rig()
        val v = r.model.state.value.view!!
        assertTrue(v.installed && v.mayEdit && v.mayRecall)
        val units = v.units.filter { it.kind == "unit" }
        assertEquals(4, units.size)
        assertTrue(units.all { it.stateWord == "Locked" && it.alinkDelayFrames != null && it.alinkDelayMin != null })
        assertEquals(listOf("Sender", "Fan-out"), v.units.take(2).map { it.role })
        assertTrue(v.summary, v.summary.startsWith("Audio link: 4 of 4 locked"))
        // -- fixup lk1 -- the demo hub's numbers: range 257..399 (4 ms), D 260, units at 300 until aligned
        assertEquals(4L, v.pending)
        assertEquals(260L, v.systemDelayFrames)
        assertEquals(listOf(260L, 259L, 259L, 260L), units.map { it.alinkDelayFrames })
        assertEquals(listOf(6, 4, -2, 4), units.map { it.hopOffset })
        assertTrue(units.all { it.alinkDelayMin == 257L && it.alinkDelayMax == 399L && it.unitAlinkDelayFrames == 300L })
        // Writing the audio link delays leaves none pending
        r.model.align()
        advanceUntilIdle()
        assertEquals("The audio link delays were written. Each unit re-anchored with a short silence.", r.model.state.value.actionMessage)
        assertEquals(0L, r.model.state.value.view!!.pending)
        assertEquals(
            listOf(260L, 259L, 259L, 260L),
            r.model.state.value.view!!.units.filter { it.kind == "unit" }.map { it.unitAlinkDelayFrames }
        )
    }

    @Test
    fun wpLkui1ARecallAndTheNextOneRefusedNamingTheHeldFrame() = runTest(timeout = 20.seconds) {
        val r = rig()
        r.model.recall("evening")
        advanceUntilIdle()
        val first = r.model.state.value
        assertTrue(first.lastRecall!!.taken)
        assertEquals("Evening", first.recalledScene)
        assertTrue(first.lastRecall!!.message.startsWith("Evening recalled through the sender"))
        val held = r.system.heldFrame!!
        r.model.recall("closed")
        advanceUntilIdle()
        val second = r.model.state.value
        assertFalse(second.lastRecall!!.taken)
        assertTrue(
            second.lastRecall!!.message,
            second.lastRecall!!.message.startsWith(
                "Closed was refused: a scene is already scheduled on the sender for audio link frame $held"
            )
        )
        assertEquals("the scene recalled is still Evening", "Evening", second.recalledScene)
        // -- fixup lk1 -- the sender's write is the scene's, and the mute is said to be unchanged
        assertEquals(listOf("Sender · Scene Closed: Refused"), second.lastRecall!!.writes)
        assertTrue(second.lastRecall!!.message.endsWith(" System mute unchanged."))
    }

    @Test
    fun wpLkui1AControlRoleRefusedAnAdminOnlySceneSeesTheHubsWords() = runTest(timeout = 20.seconds) {
        val r = rig(role = "control")
        val s = r.model.state.value
        assertFalse(s.mayEdit)
        assertTrue(s.mayRecall)
        assertFalse("no draft for Control", r.model.setUnitLatency("B", "150"))
        r.model.recall("click")
        advanceUntilIdle()
        val line = r.model.state.value.lastRecall!!
        assertFalse(line.taken)
        assertEquals("Test click was refused: Op click on unit:A needs a role this session does not have", line.message)
    }

    @Test
    fun wpLkui1AViewerIsOfferedNothingThatWrites() = runTest(timeout = 20.seconds) {
        val r = rig(role = "viewer")
        val s = r.model.state.value
        assertFalse(s.mayEdit || s.mayRecall)
        assertEquals("Recalling a scene needs the hub's Control role.", s.view!!.recallNote)
        val before = r.core.calls.size
        r.model.recall("evening")
        r.model.align()
        assertFalse(r.model.setSystemDelay(false, 200))
        r.model.save()
        advanceUntilIdle()
        assertTrue(
            "nothing sent",
            r.core.calls.drop(before).none {
                it.startsWith("alinkSystemRecall") ||
                    it.startsWith("alinkSystemAlign") ||
                    it.startsWith("alinkSystemSave")
            }
        )
        assertNull(r.model.state.value.lastRecall)
    }

    @Test
    fun wpLkui1AnEditMadeStaleWithdrawsSaveAndReloadDropsIt() = runTest(timeout = 20.seconds) {
        val r = rig()
        assertTrue(r.model.setUnitLatency("B", "150"))
        assertFalse("not a number", r.model.setUnitLatency("B", "1.5"))
        var s = r.model.state.value
        assertTrue(s.draft.dirty && !s.stale)
        assertEquals("150", s.unitLatencyText("B"))
        // Another installer saves meanwhile: the next view sees the revision move
        r.system.changeElsewhere()
        r.model.refresh()
        advanceUntilIdle()
        s = r.model.state.value
        assertTrue(s.stale)
        val before = r.core.calls.size
        r.model.save()
        advanceUntilIdle()
        assertTrue("Save withdrawn", r.core.calls.drop(before).none { it.startsWith("alinkSystemSave") })
        r.model.reload()
        advanceUntilIdle()
        s = r.model.state.value
        assertFalse(s.draft.dirty || s.stale)
        assertEquals("", s.unitLatencyText("B"))
    }

    @Test
    fun wpLkui1ASaveSentStaleIsAConflictAndOneOnTimeIsTaken() = runTest(timeout = 20.seconds) {
        val r = rig()
        r.model.setUnitLatency("B", "150")
        r.system.changeElsewhere()
        r.model.save() // before any view saw the change
        advanceUntilIdle()
        var s = r.model.state.value
        assertTrue(s.stale && s.actionFailed)
        assertTrue(s.actionMessage, s.actionMessage.contains("Reload"))
        r.model.reload()
        advanceUntilIdle()
        val revision = r.model.state.value.view!!.revision
        r.model.setUnitLatency("B", "170")
        r.model.setRoomLipSync("bar", "500")
        r.model.setSystemDelay(false, 200)
        r.model.save()
        advanceUntilIdle()
        s = r.model.state.value
        assertFalse(s.actionFailed)
        assertEquals("Saved on the hub.", s.actionMessage)
        assertEquals(revision + 1, s.view!!.revision)
        assertEquals("170", s.unitLatencyText("B"))
        assertEquals("500", s.roomLipSyncText("bar"))
        assertEquals(200L, s.view!!.systemDelayFrames)
        assertFalse(s.view!!.systemDelayAuto)
    }

    @Test
    fun wpLkui1WritesAreRefusedWhileTheBridgeAllowsNone() = runTest(timeout = 20.seconds) {
        val r = rig()
        r.bridge.setRemoteReadOnly(true)
        advanceUntilIdle()
        r.model.recall("evening")
        advanceUntilIdle()
        val line = r.model.state.value.lastRecall!!
        assertFalse(line.taken)
        assertTrue("G1 stops it before the core", r.core.calls.none { it.startsWith("alinkSystemRecall") })
    }

    @Test
    fun wpLkui1MicrosecondsAreParsedAsTyped() {
        assertEquals(SystemScreenModel.Holder(null), SystemScreenModel.parseUs(" "))
        assertEquals(SystemScreenModel.Holder(-250), SystemScreenModel.parseUs("-250"))
        assertNull(SystemScreenModel.parseUs("12 ms"))
        assertNull(SystemScreenModel.parseUs("123456789"))
    }
}
