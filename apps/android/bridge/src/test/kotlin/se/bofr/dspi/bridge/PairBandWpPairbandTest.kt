package se.bofr.dspi.bridge

import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import se.bofr.dspi.core.DspiConstants
import se.bofr.dspi.core.FilterParams
import se.bofr.dspi.fake.FakeDspiCore

/**
 * WP PAIRBAND: a linked input pair's band edits (the band list, the bypass,
 * a graph drag's live ticks and its commit) go to the core as its one-step
 * pair write, never as two `setFilter`s; an unlinked input stays one
 * `setFilter`. One undo puts both sides back.
 */
@OptIn(ExperimentalCoroutinesApi::class)
class PairBandWpPairbandTest {
    private fun peq(freq: Float, gain: Float) = FilterParams(DspiConstants.FILTER_PEAKING, false, freq, 1f, gain, 0.707f)

    @Test
    fun pairbandALinkedPairsBandIsTheCoresPairWrite() = runTest {
        val core = FakeDspiCore()
        val b = newBridge(core)
        attach(b, core)
        b.setInputLinked(0, false, -1)
        runCurrent()
        core.calls.clear()
        b.setFilter(0, 1, DspiConstants.FILTER_PEAKING, 900f, 2f, 1f)
        runCurrent()
        assertEquals("unlinked: one side", 1, core.calls.count { it.startsWith("setFilter(") })
        assertEquals(0, core.calls.count { it.startsWith("setInputPairFilter") })

        b.setInputLinked(0, true, keepCh = 0)
        runCurrent()
        val before = b.channelBands(0)!!.peq to b.channelBands(1)!!.peq
        core.calls.clear()
        b.setFilter(1, 2, DspiConstants.FILTER_PEAKING, 2000f, -3f, 2f)
        b.setBandBypass(0, 2, true)
        b.setDragging(true)
        b.sendBandLive(0, 3, peq(500f, 1f))
        b.sendBandLive(0, 3, peq(600f, 2f))
        b.commitBands(0, listOf(3), listOf(peq(700f, 3f)))
        b.setDragging(false)
        runCurrent()
        assertEquals(
            "no one-sided write while linked",
            0,
            core.calls.count {
                it.startsWith("setFilter(") || it.startsWith("setBandBypass(")
            }
        )
        assertEquals(4, core.calls.count { it.startsWith("setInputPairFilter") })
        assertEquals(1, core.calls.count { it.startsWith("setInputPairBandBypass") })
        assertTrue(core.calls.first { it.startsWith("setInputPairFilter") }.startsWith("setInputPairFilter(1, 2,"))
        for (ch in 0..1) {
            assertEquals(2000f, b.filterFreq(ch, 2))
            assertTrue(b.filterBypass(ch, 2))
            assertEquals(700f, b.filterFreq(ch, 3))
        }
        // Three steps (the edit, the bypass, the drag): three undos, both sides each time
        b.undo()
        assertEquals(before.first[3], b.channelBands(0)!!.peq[3])
        assertEquals(before.second[3], b.channelBands(1)!!.peq[3])
        b.undo()
        assertFalse(b.filterBypass(0, 2) || b.filterBypass(1, 2))
        b.undo()
        assertEquals(before.first, b.channelBands(0)!!.peq)
        assertEquals(before.second, b.channelBands(1)!!.peq)
    }
}
