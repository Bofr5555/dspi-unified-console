// Where the response graph's axis labels go (AN32). The macOS graph anchors
// the bottom dB label on the bottom edge and the 20 Hz label on the left edge,
// so the canvas clips both halfway. Here every label is
// kept whole inside the canvas, so the bottom-left corner would hold two
// labels ("-25" over "20"); instead a label that would touch one already
// placed is left out. Frequency labels go first (they name the x axis along
// its whole edge), then the dB labels, then the phase labels on the right.
package se.bofr.dspi.feature.graph

import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Rect
import androidx.compose.ui.geometry.Size
import kotlin.math.max

/** One label to place: its measured size and its anchor on the axis (x for a frequency, y for dB and phase). */
internal data class AxisLabel(val size: Size, val at: Float)

/** The placed labels' rectangles, in input order; null where a label is left out to avoid an overlap. */
internal data class AxisLabelLayout(val freq: List<Rect?>, val db: List<Rect?>, val phase: List<Rect?>) {
    val drawn: List<Rect> get() = (freq + db + phase).filterNotNull()
}

internal data class AxisLabelInput(val freq: List<AxisLabel>, val db: List<AxisLabel>, val phase: List<AxisLabel>)

/**
 * Lays the labels out in a [width] x [height] canvas: frequency labels centred on their x and
 * [bottom] above the bottom edge, dB labels [pad] from the left and phase labels [pad] from the
 * right, centred on their y; each kept inside the canvas. A label closer than [gap] to one
 * already placed is left out, so no two drawn labels overlap at any canvas size or font scale.
 */
internal fun layoutAxisLabels(input: AxisLabelInput, width: Float, height: Float, pad: Float, bottom: Float, gap: Float): AxisLabelLayout {
    val placed = mutableListOf<Rect>()

    fun place(r: Rect): Rect? {
        val grown = r.inflate(gap / 2f)
        if (placed.any { it.inflate(gap / 2f).overlaps(grown) }) return null
        placed += r
        return r
    }

    fun clampX(x: Float, w: Float) = x.coerceIn(0f, max(width - w, 0f))

    fun clampY(y: Float, h: Float) = y.coerceIn(0f, max(height - h, 0f))
    val freq = input.freq.map { l ->
        val s = l.size
        place(Rect(Offset(clampX(l.at - s.width / 2f, s.width), clampY(height - bottom - s.height, s.height)), s))
    }
    val db = input.db.map { l ->
        val s = l.size
        place(Rect(Offset(clampX(pad, s.width), clampY(l.at - s.height / 2f, s.height)), s))
    }
    val phase = input.phase.map { l ->
        val s = l.size
        place(Rect(Offset(clampX(width - pad - s.width, s.width), clampY(l.at - s.height / 2f, s.height)), s))
    }
    return AxisLabelLayout(freq, db, phase)
}
