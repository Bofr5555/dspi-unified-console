package se.bofr.dspi.bridge

import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import se.bofr.dspi.core.DspiConstants
import se.bofr.dspi.fake.FakeDspiCore

/**
 * WP PAIRBANDF (review pairband F1, F2) over the fake's undo groups (F3): Bypass All
 * on a linked pair is the pair call per band in one "Bypass All" step; Link Inputs
 * with a kept side copies in one "Link Inputs" step; a paste is one "Paste Parameters" step (the core's, CLIPAN).
 */
@OptIn(ExperimentalCoroutinesApi::class)
class GroupsWpPairbandfTest {
    @Test
    fun pairbandfBypassAllOnALinkedPairIsOneStep() = runTest {
        val core = FakeDspiCore()
        val b = newBridge(core)
        attach(b, core)
        b.setInputLinked(0, true, keepCh = 0)
        runCurrent()
        for (band in 0 until 3) b.setFilter(0, band, DspiConstants.FILTER_PEAKING, 400f + 300f * band, 2f, 1f)
        runCurrent()
        core.calls.clear()
        b.setAllBandsBypass(0, true, false)
        runCurrent()
        assertEquals(3, core.calls.count { it.startsWith("setInputPairBandBypass") })
        assertEquals("no one-sided bypass", 0, core.calls.count { it.startsWith("setBandBypass(") })
        assertTrue((0 until 3).all { b.filterBypass(0, it) && b.filterBypass(1, it) })
        assertEquals("Bypass All", core.history(1).first().label)
        b.undo()
        assertTrue("one undo enables every band of both sides", (0 until 3).none { b.filterBypass(0, it) || b.filterBypass(1, it) })
    }

    @Test
    fun pairbandfLinkInputsWithAKeptSideIsOneStep() = runTest {
        val core = FakeDspiCore()
        val b = newBridge(core)
        attach(b, core)
        b.setInputLinked(0, false, -1)
        runCurrent()
        for (band in 0 until 3) b.setFilter(0, band, DspiConstants.FILTER_PEAKING, 400f + 300f * band, 2f, 1f)
        b.setInputPreamp(0, -3f)
        runCurrent()
        val before1 = b.channelBands(1)!!.peq
        val pre1 = b.inputPreampDB(1)
        b.setInputLinked(0, true, keepCh = 0)
        runCurrent()
        assertTrue(b.inputPairMatches(0))
        assertEquals("Link Inputs", core.history(1).first().label)
        b.undo()
        assertEquals("one undo puts the copied side back", before1, b.channelBands(1)!!.peq)
        assertEquals(pre1, b.inputPreampDB(1))
        assertFalse(b.inputPairMatches(0))
    }

    @Test
    fun pairbandfAPasteIsOneStepOnTheFake() = runTest {
        val core = FakeDspiCore()
        val b = newBridge(core)
        attach(b, core)
        b.setInputLinked(0, true, keepCh = 0)
        runCurrent()
        val out = 2
        for (band in 0 until 3) b.setFilter(out, band, DspiConstants.FILTER_PEAKING, 400f + 300f * band, 2f, 1f)
        runCurrent()
        val before = b.channelBands(0)!!.peq to b.channelBands(1)!!.peq
        b.copyChannel(out)
        b.pasteChannel(0)
        runCurrent()
        assertEquals("Paste Parameters", core.history(1).first().label)
        b.undo()
        assertEquals(before, b.channelBands(0)!!.peq to b.channelBands(1)!!.peq)
    }
}
