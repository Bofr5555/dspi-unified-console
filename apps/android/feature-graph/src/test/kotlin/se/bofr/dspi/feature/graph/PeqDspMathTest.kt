package se.bofr.dspi.feature.graph

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import se.bofr.dspi.core.FilterParams

/** The graph's band response against values that hold for any correct design. */
class PeqDspMathTest {
    private fun band(type: Int, freq: Float = 1000f, q: Float = 0.707f, gain: Float = 0f, qp: Float = 0.707f, bypass: Boolean = false) =
        FilterParams(type, bypass, freq, q, gain, qp)

    private fun at(freq: Float, vararg bands: FilterParams) = PeqDspMath.responseAt(freq, bands.toList()).toDouble()

    @Test
    fun clean1_peq_shapes_meet_their_corner_values() {
        assertEquals(6.0, at(1000f, band(1, q = 2f, gain = 6f)), 1e-3)
        assertEquals(0.0, at(20f, band(1, q = 2f, gain = 6f)), 0.05)
        assertEquals(-3.01, at(1000f, band(4)), 0.02) // Q 0.707 low-pass
        assertEquals(-3.01, at(1000f, band(5)), 0.02)
        assertEquals(-3.01, at(1000f, band(12)), 0.02) // first-order
        assertEquals(-3.01, at(1000f, band(13)), 0.02)
        assertEquals(6.0, at(20f, band(2, gain = 6f)), 0.05) // low shelf
        assertEquals(6.0, at(20000f, band(3, gain = 6f)), 0.3) // high shelf
        assertTrue(at(1000f, band(6, q = 5f)) < -60.0) // notch
        assertEquals(0.0, at(300f, band(7)), 1e-6) // all-passes
        assertEquals(0.0, at(300f, band(8)), 1e-6)
    }

    @Test
    fun clean1_bypassed_flat_and_neutral_bands_add_nothing() {
        assertEquals(0.0, at(1000f, band(1, gain = 12f, bypass = true)), 0.0)
        assertEquals(0.0, at(1000f, band(0, gain = 12f)), 0.0)
        // A Linkwitz Transform whose target is the driver itself.
        assertEquals(0.0, at(50f, band(11, freq = 40f, q = 0.9f, gain = 40f, qp = 0.9f)), 1e-6)
        // fp 0: flat rather than a DC boost.
        assertEquals(0.0, at(50f, band(11, freq = 40f, gain = 0f)), 0.0)
    }

    @Test
    fun clean1_crossovers_meet_their_corner_values() {
        assertEquals(-6.02, at(1000f, band(34)), 0.02) // LR4 LP
        assertEquals(-6.02, at(1000f, band(35)), 0.02) // LR4 HP
        assertEquals(-3.01, at(1000f, band(40)), 0.02) // BW1 LP
        assertEquals(-3.01, at(1000f, band(45)), 0.02) // BW3 HP
        assertEquals(-3.01, at(1000f, band(46)), 0.02) // BW4 LP
        assertEquals(-3.01, at(1000f, band(58)), 0.05) // Bessel 4 LP, magnitude-normalised
        assertEquals(-3.01, at(1000f, band(59)), 0.05)
        // Pass band at unity, stop band falling at the order's slope.
        assertEquals(0.0, at(20f, band(34)), 0.01)
        assertEquals(0.0, at(20000f, band(35)), 0.05)
        assertEquals(-48.0, at(4000f, band(46)), 1.0) // 4th order, two octaves up
        // LR LP + HP sum flat in power at fc (each -6 dB in amplitude, in phase).
        assertEquals(at(1000f, band(36)), at(1000f, band(37)), 1e-6)
    }
}
