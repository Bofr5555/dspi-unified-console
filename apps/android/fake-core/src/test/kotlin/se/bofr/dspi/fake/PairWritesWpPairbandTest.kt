package se.bofr.dspi.fake

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Test
import se.bofr.dspi.core.DspiConstants
import se.bofr.dspi.core.FilterParams

/** WP PAIRBAND: the fake's pair writes are one undo step each, as the core's groups. */
class PairWritesWpPairbandTest {
    @Test
    fun pairbandThePairPreampIsOneStep() {
        val core = FakeDspiCore()
        val start = core.getState().inputPreampDb
        core.setInputPairPreamp(1, -4.5f)
        val pre = core.getState().inputPreampDb
        assertEquals(-4.5f to -4.5f, pre[0] to pre[1])
        assertEquals(1, core.history(10).size)
        core.undo()
        assertEquals(start, core.getState().inputPreampDb)
        assertFalse("one undo took both back", core.canUndo())
    }

    @Test
    fun pairbandThePairBandAndBypassAreOneStepEach() {
        val core = FakeDspiCore()
        val start = core.getState().filters
        core.setInputPairFilter(0, 2, FilterParams(DspiConstants.FILTER_PEAKING, false, 1200f, 1f, 3f, 0.707f))
        core.setInputPairBandBypass(1, 2, true)
        val f = core.getState().filters
        assertEquals(f[0][2], f[1][2])
        assertEquals(true, f[0][2].bypass)
        core.undo()
        assertFalse(core.getState().filters[1][2].bypass)
        core.undo()
        assertEquals(start, core.getState().filters)
        assertFalse(core.canUndo())
    }
}
