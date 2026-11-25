package se.bofr.dspi.feature.home

import org.junit.Assert.assertEquals
import org.junit.Test

// -- WP INTE -- owner's rule 2026-11-24: an output nothing is routed to is hidden in non-edit views
class OutputMetersUnusedTest {
    private val enabled = listOf(true, true, true, false, true)

    @Test
    fun metersSkipDisabledAndUnusedOutputs() {
        val outs = MatrixOutput.metered("RP2040", listOf(0, 0), enabled) { it == 2 }
        assertEquals(listOf("OUT1", "OUT2", "OUT5"), outs.map { it.descriptor })
    }

    @Test
    fun withNothingUnusedEveryEnabledOutputHasAMeter() {
        val outs = MatrixOutput.metered("RP2040", listOf(0, 0), enabled) { false }
        assertEquals(listOf(0, 1, 2, 4), outs.map { it.index })
    }
}
