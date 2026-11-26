package se.bofr.dspi.feature.analysis

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

// -- WP INTE -- owner's rule 2026-11-24: an output nothing is routed to is hidden in non-edit views
class StatsUnusedPairsTest {
    @Test
    fun aPairIsHiddenOnlyWhenBothOutputsAreUnused() {
        val unused: (Int) -> Boolean = { it in setOf(2, 3, 4) }
        assertTrue(statsPairShown(0, unused))
        assertFalse(statsPairShown(1, unused))
        assertTrue(statsPairShown(2, unused)) // OUT5 unused, OUT6 still routed
        assertEquals(setOf(1), statsUnusedPairs(unused))
    }

    @Test
    fun nothingUnusedHidesNothing() {
        assertEquals(emptySet<Int>(), statsUnusedPairs { false })
    }
}
