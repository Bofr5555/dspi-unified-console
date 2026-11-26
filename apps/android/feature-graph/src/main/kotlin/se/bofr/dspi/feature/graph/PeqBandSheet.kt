// -- WP UAN -- (UXP A12) The band editor on a phone's touch screen: a sheet at the bottom of
// the window with 48dp rows (shape, Freq, Gain, Width, Bypass) in place of the floating chip,
// whose 15-18dp rows open under the finger. The chip stays for a mouse and on a tablet.
package se.bofr.dspi.feature.graph

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.IntOffset
import androidx.compose.ui.unit.IntRect
import androidx.compose.ui.unit.IntSize
import androidx.compose.ui.unit.LayoutDirection
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.Popup
import androidx.compose.ui.window.PopupPositionProvider
import androidx.compose.ui.window.PopupProperties

/** Test tag of the touch band sheet. */
const val PEQ_BAND_SHEET_TAG = "peq-band-sheet"

/** Below this window width a touch opens the sheet; wider (a tablet) keeps the floating chip. */
const val PEQ_SHEET_BELOW_DP = 600

/** The sheet's step buttons, as TalkBack says them: "Lower Freq", "Raise Gain". */
internal fun peqStepLabel(row: PeqHudRow, up: Boolean) = (if (up) "Raise " else "Lower ") + row.label

/**
 * The shapes the band can take, in the chip's order, each with the order it is picked at and its
 * words ("Low Cut, 12 dB/oct"); only types the band offers ([available]).
 */
internal fun peqSheetShapes(available: Set<Int>): List<Triple<PeqShape, Int, String>> = PeqShape.entries.flatMap { s ->
    listOf(2, 1).mapNotNull { order ->
        s.type(order)?.takeIf { it in available }?.let {
            val both = s.type(1)?.let { t1 -> t1 in available } == true && s.type(2)?.let { t2 -> t2 in available } == true
            val cut = s == PeqShape.LOW_CUT || s == PeqShape.HIGH_CUT
            val words = when {
                cut -> "${s.title}, ${6 * order} dB/oct"
                both && order == 1 -> "${s.title}, 1st order"
                else -> s.title
            }
            Triple(s, order, words)
        }
    }
}

/** At the bottom of the window, across it: a sheet that leaves the graph above it free to touch. */
private object BottomOfWindow : PopupPositionProvider {
    override fun calculatePosition(
        anchorBounds: IntRect,
        windowSize: IntSize,
        layoutDirection: LayoutDirection,
        popupContentSize: IntSize
    ) = IntOffset((windowSize.width - popupContentSize.width) / 2, windowSize.height - popupContentSize.height)
}

/**
 * The band of [state] in rows a finger can use: its shape (a menu), each value with Lower and
 * Raise (one step each, as TalkBack's actions) and the value itself (the typing dialog,
 * [onEditField]), Bypass, and Done (the band let go, as Escape). Not modal: the graph stays live.
 */
@Composable
fun PeqBandSheet(editor: PeqGraphEditor, state: PeqHudState, onEditField: (PeqHudField) -> Unit) {
    Popup(popupPositionProvider = BottomOfWindow, properties = PopupProperties(focusable = false, clippingEnabled = false)) {
        Surface(
            Modifier.fillMaxWidth().widthIn(max = 560.dp).testTag(PEQ_BAND_SHEET_TAG),
            shape = RoundedCornerShape(topStart = 16.dp, topEnd = 16.dp),
            color = MaterialTheme.colorScheme.surfaceContainerHigh,
            shadowElevation = 8.dp
        ) {
            Column(Modifier.navigationBarsPadding().padding(horizontal = 12.dp, vertical = 4.dp)) {
                SheetHeader(editor, state)
                for (row in hudRows(state.params)) SheetValueRow(editor, state, row, onEditField)
                if (state.bypassSupported) {
                    Row(
                        Modifier.fillMaxWidth().heightIn(min = 48.dp),
                        verticalAlignment = Alignment.CenterVertically
                    ) {
                        Text("Bypass", Modifier.weight(1f), style = MaterialTheme.typography.bodyLarge)
                        Switch(
                            checked = state.params.bypass,
                            onCheckedChange = { editor.hudBypass() },
                            modifier = Modifier.semantics { contentDescription = "Bypass band ${state.band + 1}" }
                        )
                    }
                }
            }
        }
    }
}

@Composable
private fun SheetHeader(editor: PeqGraphEditor, state: PeqHudState) {
    var shapes by remember { mutableStateOf(false) }
    val own = PeqShape.of(state.params.filterType)
    Row(Modifier.fillMaxWidth().heightIn(min = 48.dp), verticalAlignment = Alignment.CenterVertically) {
        Box(Modifier.size(10.dp).background(PeqBandPalette.color(state.band).toColor(1f), CircleShape))
        Text(
            "Band ${state.band + 1}",
            Modifier.padding(start = 8.dp).semantics { heading() },
            style = MaterialTheme.typography.titleMedium
        )
        Spacer(Modifier.weight(1f))
        Box {
            TextButton(
                onClick = { shapes = true },
                enabled = own != null,
                modifier = Modifier.heightIn(min = 48.dp).semantics {
                    contentDescription =
                        "Shape, " + (own?.first?.title ?: "Linkwitz Transform")
                }
            ) { Text((own?.first?.title ?: "Linkwitz Transform") + if (own != null) " ▾" else "") }
            DropdownMenu(expanded = shapes, onDismissRequest = { shapes = false }) {
                for ((shape, order, words) in peqSheetShapes(state.availableTypes)) {
                    DropdownMenuItem(
                        text = { Text(if (own == shape to order) "✓ $words" else words) },
                        onClick = {
                            shapes = false
                            editor.hudPick(shape, order)
                        }
                    )
                }
            }
        }
        TextButton(onClick = { editor.keyDown(PeqKey.ESCAPE, PeqMods.NONE) }, modifier = Modifier.heightIn(min = 48.dp)) { Text("Done") }
    }
}

@Composable
private fun SheetValueRow(editor: PeqGraphEditor, state: PeqHudState, row: PeqHudRow, onEditField: (PeqHudField) -> Unit) {
    Row(
        Modifier.fillMaxWidth().heightIn(min = 48.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(4.dp)
    ) {
        Text(row.label, Modifier.weight(1f), style = MaterialTheme.typography.bodyLarge)
        if (!row.adjustable) {
            Text("${row.number} ${row.unit}".trim(), style = MaterialTheme.typography.bodyLarge)
            return@Row
        }
        StepButton("−", peqStepLabel(row, up = false)) { editor.accessibilityAdjust(state.band, row.field, up = false) }
        TextButton(
            onClick = { onEditField(row.field) },
            modifier = Modifier.heightIn(min = 48.dp).widthIn(min = 112.dp)
                .semantics { contentDescription = "${row.label} ${row.number} ${row.unit}, edit" }
        ) { Text("${row.number} ${row.unit}".trim(), style = MaterialTheme.typography.bodyLarge) }
        StepButton("+", peqStepLabel(row, up = true)) { editor.accessibilityAdjust(state.band, row.field, up = true) }
    }
}

@Composable
private fun StepButton(glyph: String, name: String, onClick: () -> Unit) {
    TextButton(
        onClick = onClick,
        modifier = Modifier.size(48.dp).semantics { contentDescription = name },
        shape = CircleShape,
        contentPadding = androidx.compose.foundation.layout.PaddingValues(0.dp)
    ) { Text(glyph, style = MaterialTheme.typography.titleLarge) }
}
