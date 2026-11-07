package se.bofr.dspi.feature.graph

import androidx.compose.ui.geometry.Size
import kotlin.math.ceil
import kotlin.math.log10
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/** AN32: the response graph's axis labels never overlap (the "-25" / "20" corner of phone test 1). */
class GraphAxisLabelsAn32Test {
    private val freqs = doubleArrayOf(20.0, 50.0, 100.0, 200.0, 500.0, 1000.0, 2000.0, 5000.0, 10000.0, 20000.0)
    private val freqTexts = listOf("20", "50", "100", "200", "500", "1k", "2k", "5k", "10k", "20k")

    /** A 9 sp label at [fontPx]: about 0.6 em per character, 1.2 em tall. */
    private fun size(text: String, fontPx: Float) = Size(text.length * 0.6f * fontPx, 1.2f * fontPx)

    private fun input(w: Float, h: Float, fontPx: Float, dTop: Double = 25.0, dBot: Double = -25.0, phase: Boolean = true): AxisLabelInput {
        val freq = freqs.indices.map { i ->
            AxisLabel(size(freqTexts[i], fontPx), ((log10(freqs[i]) - log10(20.0)) / (log10(20000.0) - log10(20.0)) * w).toFloat())
        }
        val step = dbGridStep(dTop - dBot)
        val db = buildList {
            var d = ceil(dBot / step) * step
            while (d <= dTop) {
                add(AxisLabel(size(dbLabel(d), fontPx), (h - (d - dBot) / (dTop - dBot) * h).toFloat()))
                d += step
            }
        }
        val ph = if (!phase) {
            emptyList()
        } else {
            listOf(180f, 90f, 0f, -90f, -180f).map { p ->
                AxisLabel(
                    size("+180°", fontPx),
                    h - (p + 180f) / 360f * h
                )
            }
        }
        return AxisLabelInput(freq, db, ph)
    }

    private fun layout(w: Float, h: Float, fontPx: Float, i: AxisLabelInput = input(w, h, fontPx)) =
        layoutAxisLabels(i, w, h, pad = fontPx * 4 / 9, bottom = fontPx * 2 / 9, gap = fontPx * 2 / 9)

    @Test
    fun an32ThePhoneGraphKeeps20AndLeavesOutTheCornerDbLabel() {
        // Phone test 1: a 1080 px phone at 420 dpi (9 sp = 23.6 px), graph about 990 x 650 px
        val l = layout(990f, 650f, 23.6f)
        assertNotNull("the 20 Hz label stays", l.freq[0])
        assertNull("-25 would sit on the 20 Hz label", l.db[0])
        assertNotNull("-20 stays", l.db[1])
        assertNotNull("+25 stays", l.db.last())
        assertTrue("every frequency label is drawn", l.freq.all { it != null })
        assertNoOverlap(l.drawn)
    }

    @Test
    fun an32NoTwoLabelsOverlapAtAnySize() {
        for (fontPx in listOf(9f, 18f, 23.6f, 31f, 47f)) {
            for (w in listOf(40f, 120f, 300f, 600f, 990f, 1600f, 2600f)) {
                for (h in listOf(20f, 80f, 200f, 400f, 650f, 1200f)) {
                    for ((top, bot) in listOf(25.0 to -25.0, 6.0 to -6.0, 30.0 to -60.0)) {
                        val l = layout(w, h, fontPx, input(w, h, fontPx, top, bot))
                        assertNoOverlap(l.drawn, "w=$w h=$h font=$fontPx dB=$bot..$top")
                        for (r in l.drawn) {
                            if (r.width <= w && r.height <= h) {
                                assertTrue("inside the canvas: $r in $w x $h", r.left >= 0f && r.top >= 0f && r.right <= w && r.bottom <= h)
                            }
                        }
                    }
                }
            }
        }
    }

    @Test
    fun an32WithoutFrequencyLabelsEveryDbLabelIsDrawn() {
        val i = input(990f, 650f, 23.6f, phase = false).copy(freq = emptyList())
        val l = layout(990f, 650f, 23.6f, i)
        assertEquals(11, l.db.size)
        assertTrue(l.db.all { it != null })
    }

    private fun assertNoOverlap(rects: List<androidx.compose.ui.geometry.Rect>, what: String = "") {
        for (a in rects.indices) {
            for (b in a + 1 until rects.size) {
                assertFalse("$what: ${rects[a]} overlaps ${rects[b]}", rects[a].overlaps(rects[b]))
            }
        }
    }
}
