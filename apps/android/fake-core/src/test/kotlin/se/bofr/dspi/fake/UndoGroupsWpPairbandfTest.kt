package se.bofr.dspi.fake

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Test

/** WP PAIRBANDF (review pairband F3): the fake's undo groups are one labelled step, as the core's. */
class UndoGroupsWpPairbandfTest {
    @Test
    fun pairbandfAGroupIsOneLabelledStepAndGroupsNest() {
        val core = FakeDspiCore()
        val start = core.getState().inputPreampDb
        core.beginGroup("Outer")
        core.setInputPreamp(0, -3f)
        core.beginGroup("Inner")
        core.setInputPreamp(1, -4f)
        core.endGroup()
        core.setInputPreamp(2, -5f)
        core.endGroup()
        core.endGroup() // a stray end does nothing
        assertEquals("Outer", core.history(1).first().label)
        // After the group a write is a step of its own, unlabelled.
        core.setMasterVolume(-20f)
        assertNull(core.history(1).first().label)
        core.undo()
        val m = core.undo()
        assertEquals("Outer", m.entry?.label)
        assertEquals(start, core.getState().inputPreampDb)
        assertFalse("one undo took the whole group back", core.canUndo())
        core.redo()
        val pre = core.getState().inputPreampDb
        assertEquals(listOf(-3f, -4f, -5f), pre.take(3))
    }

    @Test
    fun pairbandfAnEmptyGroupRecordsNothing() {
        val core = FakeDspiCore()
        core.beginGroup("Nothing")
        core.endGroup()
        assertFalse(core.canUndo())
    }
}
