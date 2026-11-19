package se.bofr.dspi.feature.home.system

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test
import se.bofr.dspi.fake.FakeDspiCore

/**
 * Review AT-D2: since WP ATQT the core gives a unit fed by S/PDIF with transport
 * "spdif" and the S/PDIF time base's state (it is no longer an audio link row in
 * "Unknown, pending"). The System screen names it as such and never shows a unit
 * that cannot lock as healthy.
 */
class SystemScreenReviewAtd2Test {
    private val unit = FakeDspiCore().alinkSystem.also { it.onHub = true }.view().units.first { it.kind == "unit" }

    private fun spdif(state: String, word: String, connected: Boolean = true) =
        unit.copy(transport = "spdif", state = state, stateWord = word, pending = false, connected = connected)

    @Test
    fun reviewAtd2AnSpdifFedUnitIsNeverGreenUnlessLocked() {
        assertEquals("ok", spdifHealth(spdif("locked", "Locked")))
        assertEquals("problem", spdifHealth(spdif("lock_error", "Lock error")))
        assertEquals("problem", spdifHealth(spdif("holdover", "Holdover")))
        for (s in listOf("acquiring", "anchoring", "none", "not_read", "no_spdif", "something_new")) {
            assertEquals(s, "warning", spdifHealth(spdif(s, s)))
        }
        assertEquals("", spdifHealth(spdif("offline", "Not connected", connected = false)))
        // Any other unit keeps the audio link's and USB-direct's own colours
        assertNull(spdifHealth(unit))
        assertNull(spdifHealth(unit.copy(transport = "usb_direct")))
    }

    @Test
    fun reviewAtd2ItsRowSaysSpdifAndItsState() {
        assertEquals("S/PDIF: Lock error", unitStateText(spdif("lock_error", "Lock error")))
        assertEquals("S/PDIF: No S/PDIF time base", unitStateText(spdif("none", "No S/PDIF time base")))
        assertEquals("Locked", unitStateText(unit.copy(stateWord = "Locked", pending = false)))
        assertEquals("Locked, audio link delay pending", unitStateText(unit.copy(stateWord = "Locked", pending = true)))
    }
}
