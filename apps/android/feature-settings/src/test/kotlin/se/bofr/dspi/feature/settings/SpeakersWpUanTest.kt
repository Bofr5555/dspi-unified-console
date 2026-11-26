package se.bofr.dspi.feature.settings

import java.io.File
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import se.bofr.dspi.bridge.OutputLimits
import se.bofr.dspi.core.MeasureRole
import se.bofr.dspi.core.RoleSource
import se.bofr.dspi.core.SpeakerLayout
import se.bofr.dspi.core.SpeakerLayoutOutput

/**
 * WP UAN (UXP B8): the Speakers page's rows: every output, None for one the layout leaves out;
 * the layout they make (one group per speaker, the user's rows); Same for the right side; the
 * high-pass a tweeter or mid gets, in words, as the core's tweeter rule has it (the floors are
 * read from crates/dspi-core, so the two cannot drift apart).
 */
class SpeakersWpUanTest {
    private fun layout(vararg rows: SpeakerLayoutOutput) =
        SpeakerLayout("S1", rows.toList(), confirmedByUser = false, updatedMs = 0, notes = emptyList(), detected = null, stated = null)

    @Test
    fun uanRowsCoverEveryOutputAndLeftOutIsNone() {
        val l =
            layout(
                SpeakerLayoutOutput(0, MeasureRole.LOW, "L", 0, RoleSource.SUGGESTED),
                SpeakerLayoutOutput(2, MeasureRole.HIGH, "", 0, RoleSource.SUGGESTED)
            )
        val rows = speakerRows(l, 4) { "Out ${it + 1}" }
        assertEquals(listOf(MeasureRole.LOW, null, MeasureRole.HIGH, null), rows.map { it.role })
        assertEquals("a blank speaker takes the slot's side", listOf("L", "R", "L", "R"), rows.map { it.side })
        assertEquals(4, speakerRows(null, 4) { "" }.size)
    }

    @Test
    fun uanTheLayoutKeepsTheRolesGroupedBySpeaker() {
        val rows = listOf(
            SpeakerRow(0, "Out 1", MeasureRole.LOW, "L"),
            SpeakerRow(1, "Out 2", MeasureRole.LOW, "R"),
            SpeakerRow(2, "Out 3", MeasureRole.HIGH, "L"),
            SpeakerRow(3, "Out 4", null, "R")
        )
        val l = layoutOf(rows, "S1")
        assertEquals(listOf(0, 1, 2), l.outputs.map { it.output })
        assertEquals("one group per speaker", listOf(0, 1, 0), l.outputs.map { it.group })
        assertTrue(l.confirmedByUser && l.outputs.all { it.source == RoleSource.USER })
    }

    @Test
    fun uanSameForTheRightSide() {
        val rows = listOf(
            SpeakerRow(0, "Out 1", MeasureRole.HIGH, "L"),
            SpeakerRow(1, "Out 2", null, "R"),
            SpeakerRow(2, "Out 3", MeasureRole.SUB, "C"),
            SpeakerRow(3, "Out 4", null, "C")
        )
        val out = sameForRight(rows, listOf(0 to 1, 2 to 3))
        assertEquals(SpeakerRow(1, "Out 2", MeasureRole.HIGH, "R"), out[1])
        assertEquals("a centre pair keeps its own side", SpeakerRow(3, "Out 4", MeasureRole.SUB, "C"), out[3])
        assertEquals(rows[0], out[0])
    }

    @Test
    fun uanTheFloorIsTheCoresTweeterRule() {
        assertEquals("Its high-pass: at least 1000 Hz, 12 dB/octave", speakerFloorWords(MeasureRole.HIGH, null))
        assertEquals("Its high-pass: at least 80 Hz, 12 dB/octave", speakerFloorWords(MeasureRole.MID, null))
        // A profile may raise a tweeter's, never lower it
        assertEquals(
            "Its high-pass: at least 1500 Hz, 24 dB/octave",
            speakerFloorWords(MeasureRole.HIGH, OutputLimits(0, highpassMinHz = 1500f, highpassMinSlopeDb = 24))
        )
        assertEquals(
            "Its high-pass: at least 1000 Hz, 12 dB/octave",
            speakerFloorWords(MeasureRole.HIGH, OutputLimits(0, highpassMinHz = 500f, highpassMinSlopeDb = 6))
        )
        assertEquals(
            "Its high-pass: at least 300 Hz, 12 dB/octave",
            speakerFloorWords(MeasureRole.MID, OutputLimits(0, highpassMinHz = 300f))
        )
        for (r in listOf(MeasureRole.LOW, MeasureRole.SUB, MeasureRole.FULL, null)) assertNull(speakerFloorWords(r, null))
        // The core's numbers
        val rs = generateSequence(File("").absoluteFile) { it.parentFile }
            .map { File(it, "crates/dspi-core/src/measure/preflight.rs") }.first { it.isFile }.readText()
        fun num(name: String) = Regex("const $name: \\w+ = ([0-9_.]+);").find(rs)!!.groupValues[1].replace("_", "").toFloat()
        assertEquals(TWEETER_MIN_HZ, num("HIGH_MIN_HZ"))
        assertEquals(MID_MIN_HZ, num("MID_MIN_HZ"))
        assertEquals(MIN_SLOPE_DB.toFloat(), num("MIN_SLOPE_DB"))
    }
}
