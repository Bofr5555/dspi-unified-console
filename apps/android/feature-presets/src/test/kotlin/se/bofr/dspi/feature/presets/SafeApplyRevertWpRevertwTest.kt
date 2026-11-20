package se.bofr.dspi.feature.presets

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import se.bofr.dspi.bridge.Snapshots
import se.bofr.dspi.core.GuardPhase
import se.bofr.dspi.core.GuardedApply
import se.bofr.dspi.feature.presets.snapshots.guardBarShown
import se.bofr.dspi.feature.presets.snapshots.guardLines

/**
 * WP REVERTW: a revert that was not fully successful (owner 2026-11-19) is
 * said in the core's `revert.*` notice: its title first, then what was not
 * put back and what to do; the bar stays until the user's OK.
 */
class SafeApplyRevertWpRevertwTest {
    private fun ended(phase: GuardPhase, notice: String) = GuardedApply(
        3, phase, "Room", false, 30_000, 0, "reverted",
        "The revert was not fully successful: some of the settings from before were not put back (see the notes).",
        emptyList(), listOf("Device name: not written (refused)"), notice
    )

    @Test
    fun wp_revertw_the_bar_leads_with_the_notice_and_stays() {
        for ((phase, key) in listOf(
            GuardPhase.REVERT_INCOMPLETE to "revert.incomplete",
            GuardPhase.REVERT_FAILED to "revert.failed",
            GuardPhase.NOT_WRITTEN to "revert.not_written",
            GuardPhase.LOST to "revert.unknown"
        )) {
            val g = ended(phase, key)
            val (first, second) = guardLines(g)
            assertEquals(Snapshots.REVERT_INCOMPLETE_TITLE, first)
            assertTrue(second, second.contains("Device name: not written"))
            assertTrue(second, second.endsWith(Snapshots.REVERT_INCOMPLETE_ACTION))
            // Shown until the user's OK, also for a guard this bar did not watch
            assertTrue(guardBarShown(g, watched = -1, endShown = null))
            assertFalse(guardBarShown(g, watched = 3, endShown = 3L to phase))
        }
        // A clean revert keeps its own words
        val clean = ended(GuardPhase.REVERTED, "").copy(message = "Reverted: the settings from before are back.", notes = emptyList())
        assertEquals("Reverted: the settings from before are back." to "", guardLines(clean))
    }
}
