// -- WP P1AN2 --
// PRO1 T6 on Android: before Apply, the AutoEQ browser shows which channel pair
// the profile goes to (any pair, not only inputs 1 and 2) and the core's
// preview (`autoeq_preview`: the response curve, the suggested trim on outputs,
// what a limits profile would hold), and Apply goes through `autoeq_apply`
// (a snapshot first, one undo step, outputs' gain kept). Nothing is written by
// the preview.
package se.bofr.dspi.feature.presets.autoeq

import androidx.compose.foundation.Canvas
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Path
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.unit.dp
import se.bofr.dspi.bridge.DspiBridge
import se.bofr.dspi.bridge.Installer
import se.bofr.dspi.core.AutoEqFilter
import se.bofr.dspi.core.AutoEqPreview
import se.bofr.dspi.core.AutoEqProfile

/** Test tags. */
internal const val AUTOEQ_PAIR_TAG = "autoeq-pair"
internal const val AUTOEQ_PREVIEW_TAG = "autoeq-preview"

/** A channel pair a profile can go to: wire channels [left] and [right]. */
data class AutoEqPair(val left: Int, val right: Int, val label: String)

/** The pairs of a device with [numIn] inputs and [numOut] outputs: inputs first (1–2 the default), then outputs. */
fun autoEqPairs(numIn: Int, numOut: Int): List<AutoEqPair> =
    (0 until numIn / 2).map { p -> AutoEqPair(2 * p, 2 * p + 1, "IN ${2 * p + 1}–${2 * p + 2}") } +
        (0 until numOut / 2).map { p -> AutoEqPair(numIn + 2 * p, numIn + 2 * p + 1, "OUT ${2 * p + 1}–${2 * p + 2}") }

/** The database entry as the core's profile. */
fun profileOf(entry: HeadphoneEntry): AutoEqProfile = AutoEqProfile(
    id = entry.id,
    name = entry.displayName,
    preampDb = entry.preamp.toFloat(),
    filters = entry.filters.map { AutoEqFilter(it.type, it.freq.toFloat(), it.q.toFloat(), it.gain.toFloat()) }
)

/** The preview's words: the suggested trim on outputs, the values a limits profile holds, the notes. */
fun previewLines(p: AutoEqPreview): List<String> = buildList {
    if (p.suggestedTrimDb != 0f && p.channels.any { it.isOutput }) {
        add("Outputs keep their gain; suggested trim ${"%+.1f".format(p.suggestedTrimDb)} dB.")
    }
    if (p.held.isNotEmpty()) add("Held to the limits: " + p.held.joinToString(", "))
    if (p.unsupported.isNotEmpty()) add("Not applied: " + p.unsupported.joinToString(", "))
    addAll(p.notes)
}

/** Whether this bridge applies AutoEQ through the core (a test double without one keeps the old path). */
fun coreAutoEq(bridge: DspiBridge): Boolean = bridge.installer !== Installer.Unavailable

/** The pair choice and the preview of [entry] on it. */
@Composable
internal fun AutoEQPreviewPanel(
    bridge: DspiBridge,
    entry: HeadphoneEntry,
    pairs: List<AutoEqPair>,
    pair: AutoEqPair,
    onPair: (AutoEqPair) -> Unit
) {
    var preview by remember(entry.id, pair) { mutableStateOf<AutoEqPreview?>(null) }
    var error by remember(entry.id, pair) { mutableStateOf("") }
    LaunchedEffect(entry.id, pair) {
        bridge.installer.autoeqPreview(profileOf(entry), pair.left, pair.right)
            .onSuccess { preview = it }
            .onFailure { error = it.message.orEmpty() }
    }
    Column(Modifier.fillMaxWidth().testTag(AUTOEQ_PREVIEW_TAG), verticalArrangement = Arrangement.spacedBy(2.dp)) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Text("Apply to", style = MaterialTheme.typography.bodySmall)
            var open by remember { mutableStateOf(false) }
            Box {
                TextButton(onClick = { open = true }, Modifier.testTag(AUTOEQ_PAIR_TAG)) { Text("${pair.label} ▾") }
                DropdownMenu(expanded = open, onDismissRequest = { open = false }) {
                    pairs.forEach { p ->
                        DropdownMenuItem(text = { Text(p.label) }, onClick = {
                            open = false
                            onPair(p)
                        })
                    }
                }
            }
        }
        preview?.let { p ->
            p.channels.firstOrNull()?.curve?.takeIf { it.size > 1 }?.let { PreviewCurve(it) }
            previewLines(p).forEach { Text(it, style = MaterialTheme.typography.bodySmall) }
        }
        if (error.isNotEmpty()) Text(error, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.error)
    }
}

/** The preview's response curve (dB on the core's log frequency grid), ±15 dB, with the 0 dB line. */
@Composable
private fun PreviewCurve(curve: List<Double>) {
    val line = MaterialTheme.colorScheme.primary
    val axis = MaterialTheme.colorScheme.outlineVariant
    Canvas(Modifier.fillMaxWidth().height(64.dp)) {
        val range = 15.0
        fun y(db: Double) = (size.height / 2f) - (db.coerceIn(-range, range) / range * size.height / 2f).toFloat()
        drawLine(axis, Offset(0f, size.height / 2f), Offset(size.width, size.height / 2f))
        val path = Path()
        curve.forEachIndexed { i, db ->
            val x = size.width * i / (curve.size - 1)
            if (i == 0) path.moveTo(x, y(db)) else path.lineTo(x, y(db))
        }
        drawPath(path, line, style = Stroke(width = 2.dp.toPx()))
    }
}
