package se.bofr.dspi.feature.analysis

import org.junit.Assert.assertEquals
import org.junit.Test
import se.bofr.dspi.core.DspiConstants.RTA_TAP_INPUT
import se.bofr.dspi.core.DspiConstants.RTA_TAP_OUTPUT

// -- WP INTE -- owner's rule 2026-11-24: an output nothing is routed to is hidden in non-edit views
class SpectrumChipsUnusedTest {
    private val unused: (Int) -> Boolean = { it == 2 || it == 3 }

    @Test
    fun unusedOutputsHaveNoChip() {
        assertEquals(listOf(0, 1), spectrumChipChannels(RTA_TAP_OUTPUT, listOf(0, 1, 2, 3), listOf(0), unused))
    }

    @Test
    fun anAnalysedUnusedOutputKeepsItsChipSoItCanBeDropped() {
        assertEquals(listOf(0, 1, 3), spectrumChipChannels(RTA_TAP_OUTPUT, listOf(0, 1, 2, 3), listOf(0, 3), unused))
    }

    @Test
    fun inputsAreNeverFilteredByTheOutputRule() {
        assertEquals(listOf(0, 1, 2, 3), spectrumChipChannels(RTA_TAP_INPUT, listOf(0, 1, 2, 3), listOf(1), unused))
    }

    @Test
    fun theStateDefaultIsAvailablePlusSelected() {
        val s =
            SpectrumUiState(
                connected = true,
                caps = null,
                selection = RtaChannelSelection(RTA_TAP_OUTPUT, listOf(5)),
                available = listOf(0, 2),
                mode = RtaDisplayMode.BARS,
                prefs = RtaPreferences()
            )
        assertEquals(listOf(0, 2, 5), s.chips)
    }
}
