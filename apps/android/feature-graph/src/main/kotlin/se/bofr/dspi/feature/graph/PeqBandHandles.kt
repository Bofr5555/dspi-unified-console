// WP AN20: the graph's band dots for TalkBack and switch access. The dots
// are drawn on one Canvas and dragged through PeqGraphEditor's pointer
// layer, which accessibility services cannot reach; each dot gets a node
// here, with the arrow keys' steps as custom actions.
package se.bofr.dspi.feature.graph

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.size
import androidx.compose.runtime.Composable
import androidx.compose.runtime.key
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.CustomAccessibilityAction
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.customActions
import androidx.compose.ui.semantics.isTraversalGroup
import androidx.compose.ui.semantics.onClick
import androidx.compose.ui.semantics.selected
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.ui.semantics.traversalIndex
import androidx.compose.ui.unit.dp
import se.bofr.dspi.core.FilterParams

/** A handle's side (dp): the 48 dp touch target. */
internal const val PEQ_HANDLE_SIZE = 48.0

/** "Band 3, Bell" (the Linkwitz Transform has no graph shape). */
internal fun peqBandName(band: Int, p: FilterParams): String =
    "Band ${band + 1}, " + (PeqShape.of(p.filterType)?.first?.title ?: "Linkwitz Transform")

/** "1.000 kHz, +3.00 dB, Q 0.707", then ", bypassed" when it is. */
internal fun peqBandState(p: FilterParams): String {
    val parts = mutableListOf(PeqValueText.frequency(p.freq.toDouble()))
    if (FilterTypes.hasGain(p.filterType)) parts += PeqValueText.gain(p.gain.toDouble())
    if (FilterTypes.hasQ(p.filterType)) parts += "Q " + PeqValueText.q(p.q.toDouble())
    if (p.bypass) parts += "bypassed"
    return parts.joinToString(", ")
}

/** The custom actions a band's handle offers: frequency, gain and Q steps where the band has them, and bypass. */
internal fun peqBandActions(editor: PeqGraphEditor, band: PeqA11yBand, bypassSupported: Boolean): List<CustomAccessibilityAction> {
    val type = band.params.filterType
    val actions = mutableListOf<CustomAccessibilityAction>()
    fun step(label: String, field: PeqHudField, up: Boolean) {
        actions += CustomAccessibilityAction(label) { editor.accessibilityAdjust(band.band, field, up) }
    }
    step("Increase frequency", PeqHudField.FREQ, up = true)
    step("Decrease frequency", PeqHudField.FREQ, up = false)
    if (FilterTypes.hasGain(type)) {
        step("Increase gain", PeqHudField.GAIN, up = true)
        step("Decrease gain", PeqHudField.GAIN, up = false)
    }
    if (FilterTypes.hasQ(type)) {
        step("Increase Q", PeqHudField.Q, up = true)
        step("Decrease Q", PeqHudField.Q, up = false)
    }
    if (bypassSupported) {
        actions += CustomAccessibilityAction(if (band.params.bypass) "Enable band" else "Bypass band") {
            editor.accessibilityToggleBypass(band.band)
        }
    }
    return actions
}

/**
 * One node per band dot over the graph, in frequency order, kept inside the
 * graph so each keeps its full 48 dp. They take no pointer input: touches
 * still reach the editor under them.
 */
@Composable
internal fun PeqBandHandles(editor: PeqGraphEditor, bands: List<PeqA11yBand>, bypassSupported: Boolean) {
    BoxWithConstraints(Modifier.fillMaxSize().semantics { isTraversalGroup = true }) {
        val w = maxWidth.value.toDouble()
        val h = maxHeight.value.toDouble()
        bands.forEachIndexed { order, b ->
            key(b.band) {
                val x = (b.point.x - PEQ_HANDLE_SIZE / 2).coerceIn(0.0, (w - PEQ_HANDLE_SIZE).coerceAtLeast(0.0))
                val y = (b.point.y - PEQ_HANDLE_SIZE / 2).coerceIn(0.0, (h - PEQ_HANDLE_SIZE).coerceAtLeast(0.0))
                Box(
                    Modifier
                        .offset(x.dp, y.dp)
                        .size(PEQ_HANDLE_SIZE.dp)
                        .semantics {
                            contentDescription = peqBandName(b.band, b.params)
                            stateDescription = peqBandState(b.params)
                            selected = b.selected
                            traversalIndex = order.toFloat()
                            onClick(label = "Select") { editor.accessibilitySelect(b.band) }
                            customActions = peqBandActions(editor, b, bypassSupported)
                        }
                )
            }
        }
    }
}
