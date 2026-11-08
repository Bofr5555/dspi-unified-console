package se.bofr.dspi.feature.home

import org.junit.Assert.assertEquals
import org.junit.Test

/** WP AN40: the Dashboard shows a fractional delay instead of rounding it to whole ms. */
class DelayTextWpAn40Test {
    @Test
    fun wpAn40DashboardDelayKeepsItsFraction() {
        assertEquals("Delay: 0ms", dashboardDelayText(0f))
        assertEquals("Delay: 12ms", dashboardDelayText(12f))
        assertEquals("Delay: 0.5ms", dashboardDelayText(0.5f))
        assertEquals("Delay: 1.5ms", dashboardDelayText(1.5f))
        assertEquals("Delay: 0.25ms", dashboardDelayText(0.25f))
        assertEquals("Delay: 12.35ms", dashboardDelayText(12.3456f))
        assertEquals("Delay: 12ms", dashboardDelayText(12.004f))
    }
}
