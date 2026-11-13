package se.bofr.dspi.app

import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.launch
import kotlinx.coroutines.test.StandardTestDispatcher
import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import se.bofr.dspi.bridge.DspiBridgeImpl
import se.bofr.dspi.bridge.HistoryStep
import se.bofr.dspi.bridge.InMemoryLinkStore
import se.bofr.dspi.core.DspiConstants
import se.bofr.dspi.core.DspiException
import se.bofr.dspi.core.HistoryEntry
import se.bofr.dspi.core.HistoryMove
import se.bofr.dspi.core.HistorySource
import se.bofr.dspi.core.ParamValue
import se.bofr.dspi.fake.FakeDspiCore

/**
 * WP AN42: the Undo / Redo view-model over the bridge and the fake's journal
 * (the buttons' state, G1 and the viewer role, the steps and their words), and
 * the snackbar's wording for every kind of step.
 */
@OptIn(ExperimentalCoroutinesApi::class)
class UndoRedoModelWpAn42Test {
    private fun TestScope.bridge(core: FakeDspiCore): DspiBridgeImpl {
        val d = StandardTestDispatcher(testScheduler)
        return DspiBridgeImpl(core, backgroundScope, ffi = d, main = d, linkStore = InMemoryLinkStore(), clock = {
            testScheduler.currentTime
        })
    }

    private fun TestScope.model(b: DspiBridgeImpl): Pair<UndoRedoModel, List<String>> {
        val m = UndoRedoModel(b, backgroundScope)
        val out = mutableListOf<String>()
        backgroundScope.launch(StandardTestDispatcher(testScheduler)) { m.messages.collect { out += it } }
        runCurrent()
        return m to out
    }

    @Test
    fun wpAn42TheButtonsFollowTheJournalAndEachStepIsWorded() = runTest {
        val core = FakeDspiCore()
        val b = bridge(core)
        val (m, said) = model(b)
        assertEquals(UndoBarState(), m.state.value)
        assertFalse("nothing to undo: nothing sent", m.undo())
        b.attachDevice(42, core.serial, DspiConstants.VENDOR_ID)
        runCurrent()
        assertEquals(UndoBarState(), m.state.value)

        b.setUserMute(true)
        runCurrent()
        assertEquals(UndoBarState(canUndo = true, canRedo = false), m.state.value)
        assertTrue(m.undo())
        runCurrent()
        assertFalse(b.userMute)
        assertEquals(UndoBarState(canUndo = false, canRedo = true), m.state.value)
        assertEquals(listOf("Undone: setUserMute"), said)

        assertTrue(m.redo())
        runCurrent()
        assertTrue(b.userMute)
        assertEquals(listOf("Undone: setUserMute", "Redone: setUserMute"), said)
        assertEquals(1, core.callCount("undo"))
        assertEquals(1, core.callCount("redo"))
    }

    @Test
    fun wpAn42ReadOnlyAndAViewerDisableBothAndSendNothing() = runTest {
        val core = FakeDspiCore()
        val b = bridge(core)
        val (m, said) = model(b)
        b.attachDevice(42, core.serial, DspiConstants.VENDOR_ID)
        runCurrent()
        b.setUserMute(true)
        runCurrent()
        b.undo()
        b.setUserMute(true)
        runCurrent()
        b.undo()
        runCurrent()
        assertEquals(UndoBarState(canUndo = false, canRedo = true), m.state.value)
        b.redo()
        runCurrent()
        assertEquals(UndoBarState(canUndo = true, canRedo = false), m.state.value)

        // DSPi Link viewer: both off, the keys pass, nothing reaches the core
        b.setRemoteReadOnly(true)
        runCurrent()
        assertEquals(UndoBarState(), m.state.value)
        val undos = core.callCount("undo")
        assertFalse(m.undo())
        assertFalse(m.redo())
        runCurrent()
        assertEquals(undos, core.callCount("undo"))
        b.setRemoteReadOnly(false)
        runCurrent()
        assertTrue(m.state.value.canUndo)

        // G1: an incompatible firmware
        core.simulateRemote { it.copy(compat = DspiConstants.COMPAT_FIRMWARE_TOO_NEW) }
        testScheduler.advanceTimeBy(100)
        runCurrent()
        assertFalse(b.writesAllowed.value)
        assertEquals(UndoBarState(), m.state.value)
        assertFalse(m.undo())
        assertTrue(said.isEmpty())
    }

    private fun entry(path: String, vararg ix: Int, undoable: Boolean = true) =
        HistoryEntry(path, ix.toList(), null, ParamValue.Bool(true), HistorySource.LOCAL, 0L, undoable, label = null)

    @Test
    fun wpAn42TheWordsForEachKindOfStep() {
        fun moved(e: HistoryEntry?, skipped: List<HistoryEntry> = emptyList(), note: String? = null) =
            HistoryStep.Moved(HistoryMove(e, skipped, note))
        assertEquals("Undone: Gain, channel 1, band 3", historyMessage(false, moved(entry("eq.gain", 0, 2))))
        assertEquals("Redone: Crosspoint, IN1 to OUT2", historyMessage(true, moved(entry("mix", 0, 1))))
        assertEquals("Undone: Output gain, OUT4", historyMessage(false, moved(entry("out.gain", 3))))
        assertEquals("Undone: Loudness", historyMessage(false, moved(entry("loud.ref"))))
        assertEquals("Nothing to undo", historyMessage(false, moved(null)))
        assertEquals("Nothing to redo", historyMessage(true, moved(null)))
        // A hazard stepped over (a preset load, an output switched on)
        assertEquals(
            "Undone: Volume\nStepped over a change that cannot be undone: Output on, OUT1",
            historyMessage(false, moved(entry("vol.user"), listOf(entry("out.enable", 0, undoable = false))))
        )
        assertEquals(
            "Nothing was undone\nStepped over changes that cannot be undone: Output on, OUT1; Preset",
            historyMessage(
                false,
                moved(null, listOf(entry("out.enable", 0, undoable = false), entry("preset.load", undoable = false)))
            )
        )
        // Another writer's change left alone: the core's note names them (skip-and-name)
        val note = "Volume was changed by DSPi Link session 2 since; it was left as they set it"
        assertEquals("Nothing was undone\n$note", historyMessage(false, moved(null, listOf(entry("vol.user")), note)))
        assertEquals(
            "Undone: Filter band, channel 1, band 1\n$note",
            historyMessage(false, moved(entry("eq.band", 0, 0), listOf(entry("vol.user")), note))
        )
        // Failures in the core's words; a refusal was said by the bridge already
        assertEquals("Could not undo: device not connected", historyMessage(false, HistoryStep.Failed(DspiException.Disconnected())))
        assertNull(historyMessage(true, HistoryStep.Refused))
        // An unknown path shows as itself
        assertEquals("Undone: ext.vreg", historyMessage(false, moved(entry("ext.vreg"))))
    }
}
