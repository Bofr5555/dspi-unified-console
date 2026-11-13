package se.bofr.dspi.bridge

import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import se.bofr.dspi.core.DspiException
import se.bofr.dspi.fake.FakeDspiCore

/**
 * WP AN42: undo and redo through the bridge over the fake's journal: the
 * availability follows every published state, a step publishes the state it
 * left, a drag between `setDragging(true)` and `(false)` is one step, the mark
 * runs in call order with the setters, and the history ends with the device.
 */
@OptIn(ExperimentalCoroutinesApi::class)
class UndoRedoWpAn42Test {
    @Test
    fun wpAn42AvailabilityFollowsTheJournalAndAStepPublishesTheState() = runTest {
        val core = FakeDspiCore()
        val b = newBridge(core)
        attach(b, core)
        assertEquals(UndoAvailability(), b.undoState.value)
        val before = b.userMute
        b.setUserMute(!before)
        runCurrent()
        assertEquals(UndoAvailability(canUndo = true, canRedo = false), b.undoState.value)

        val undone = b.undo()
        assertTrue(undone is HistoryStep.Moved && undone.move.entry != null)
        assertEquals("the state is published at once", before, b.userMute)
        assertEquals(UndoAvailability(canUndo = false, canRedo = true), b.undoState.value)

        b.redo()
        assertEquals(!before, b.userMute)
        assertEquals(UndoAvailability(canUndo = true, canRedo = false), b.undoState.value)
    }

    @Test
    fun wpAn42NothingToUndoIsAMoveWithNoEntry() = runTest {
        val core = FakeDspiCore()
        val b = newBridge(core)
        attach(b, core)
        val r = b.undo()
        assertTrue(r is HistoryStep.Moved)
        assertNull((r as HistoryStep.Moved).move.entry)
    }

    @Test
    fun wpAn42ACoreErrorComesBackAsFailedNotAsAnEvent() = runTest {
        val core = FakeDspiCore()
        val b = newBridge(core)
        attach(b, core)
        b.setUserMute(true)
        runCurrent()
        val events = collectEvents(b)
        core.failNext("undo") { DspiException.Locked() }
        val r = b.undo()
        assertTrue(r is HistoryStep.Failed && r.error is DspiException.Locked)
        runCurrent()
        assertTrue(events.filterIsInstance<BridgeEvent.CommandFailed>().isEmpty())
        assertTrue("the change stays to be undone", b.undoState.value.canUndo)
    }

    @Test
    fun wpAn42ADragIsOneStepAndItsCommitJoinsIt() = runTest {
        val core = FakeDspiCore()
        val b = newBridge(core)
        attach(b, core)
        val start = b.preampDB
        b.setDragging(true)
        b.sendPreampToDevice(-1f)
        b.sendPreampToDevice(-2f)
        b.sendPreampToDevice(-3f)
        b.setPreamp(-3.5f) // the release's commit, before the mark's end
        b.setDragging(false)
        runCurrent()
        val order = core.calls.filter { it.startsWith("setPreamp") || it.startsWith("setDragging") }
        assertEquals("in call order", "setDragging(false)", order.last())
        assertEquals("setDragging(true)", order.first())
        assertEquals(-3.5f, b.preampDB)
        b.undo()
        assertEquals("one undo puts the drag's start back", start, b.preampDB)
        assertFalse(b.undoState.value.canUndo)
        // A write after the drag is a step of its own
        b.redo()
        b.setPreamp(-6f)
        runCurrent()
        b.undo()
        assertEquals(-3.5f, b.preampDB)
    }

    @Test
    fun wpAn42TheHistoryEndsWithTheDevice() = runTest {
        val core = FakeDspiCore()
        val b = newBridge(core)
        attach(b, core)
        b.setUserMute(true)
        runCurrent()
        assertTrue(b.undoState.value.canUndo)
        b.detachDevice(core.serial)
        runCurrent()
        assertEquals(UndoAvailability(), b.undoState.value)
        val events = collectEvents(b)
        assertEquals(HistoryStep.Refused, b.undo())
        runCurrent()
        assertEquals(WritesNotAllowedException.NOT_CONNECTED, events.filterIsInstance<BridgeEvent.CommandFailed>().single().error.message)
    }
}
