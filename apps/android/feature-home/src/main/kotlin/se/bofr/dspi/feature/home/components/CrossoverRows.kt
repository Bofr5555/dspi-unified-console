// -- WP UAN -- (UXP B10) An output's crossover in two rows on its page, "High-pass  LR24 ▾  2500 Hz"
// and "Low-pass  Off ▾", above the band list: a pair of LR24s was about 20 taps under ten PEQ
// bands. The rows write the XO bands the XO tab lists (the first of each way, else a free one),
// and "Same for the right side" gives the output's partner the same crossover in one undo step.
package se.bofr.dspi.feature.home.components

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
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
import androidx.compose.ui.unit.dp
import se.bofr.dspi.ui.BandFields
import se.bofr.dspi.ui.CrossoverFamily
import se.bofr.dspi.ui.DspiTheme
import se.bofr.dspi.ui.DspiType
import se.bofr.dspi.ui.FilterType
import se.bofr.dspi.ui.ValueField
import se.bofr.dspi.ui.dspiCard
import se.bofr.dspi.ui.writeEnabled

/** Test tags: the card, a way's type button and frequency (+ "hp" / "lp"), and Same for the right side. */
const val XO_ROWS_TAG = "xo-rows"
const val XO_TYPE_TAG = "xo-type-"
const val XO_SAME_TAG = "xo-same"

/** One way of the crossover: the XO band that holds it (null: none yet) and its type (FLAT: Off). */
data class XoWay(val band: Int?, val type: FilterType, val freq: Float)

/** A crossover type's short words: "LR24", "BW6", "BES12"; "Off" for FLAT. */
fun xoShortName(type: FilterType): String {
    val family = type.crossoverFamily ?: return "Off"
    return family.shortName + (6 * type.crossoverOrder)
}

/** Every crossover type of one way ([lowPass]), Off first, in family and order (the rows' menu). */
fun xoChoices(lowPass: Boolean): List<FilterType> =
    listOf(FilterType.FLAT) + CrossoverFamily.entries.flatMap { f -> f.availableOrders.mapNotNull { o -> f.filterType(o, lowPass) } }

/** The high-pass ([lowPass] false) or low-pass way in [xo]: the first band of that way, else Off with no band. */
fun xoWay(xo: List<BandFields>, lowPass: Boolean): XoWay {
    val i = xo.indexOfFirst { it.type.crossoverFamily != null && it.type.crossoverIsLowPass == lowPass }
    return if (i < 0) XoWay(null, FilterType.FLAT, DEFAULT_XO_HZ) else XoWay(i, xo[i].type, xo[i].freq)
}

/**
 * The band a way writes [type] to: its own, else the first Off band the other way does not hold;
 * null when every band is taken (the XO tab then has the rest).
 */
fun xoBandFor(xo: List<BandFields>, way: XoWay, other: XoWay): Int? =
    way.band ?: xo.indices.firstOrNull { it != other.band && xo[it].type == FilterType.FLAT }

/** A new crossover's corner when the band had none. */
const val DEFAULT_XO_HZ = 2500f

/**
 * The crossover card of output [ch] (app id): its high-pass and low-pass rows, each a type menu
 * and a frequency, and, for the left output of a stereo pair ([partnerName] non-null), the way to
 * give the partner the same crossover ([onSameForPartner]).
 */
@Composable
fun CrossoverRows(
    xo: List<BandFields>,
    connected: Boolean,
    onSet: (band: Int, type: FilterType, freq: Float) -> Unit,
    modifier: Modifier = Modifier,
    partnerName: String? = null,
    onSameForPartner: () -> Unit = {}
) {
    val hp = xoWay(xo, lowPass = false)
    val lp = xoWay(xo, lowPass = true)
    Column(modifier.fillMaxWidth().dspiCard().padding(horizontal = 12.dp, vertical = 4.dp).testTag(XO_ROWS_TAG)) {
        Text(
            "CROSSOVER",
            Modifier.padding(top = 6.dp).semantics { heading() },
            style = DspiType.captionBold,
            color = DspiTheme.colors.secondaryLabel
        )
        WayRow("High-pass", "hp", hp, lowPass = false, connected) { t, f -> xoBandFor(xo, hp, lp)?.let { onSet(it, t, f) } }
        WayRow("Low-pass", "lp", lp, lowPass = true, connected) { t, f -> xoBandFor(xo, lp, hp)?.let { onSet(it, t, f) } }
        partnerName?.let { name ->
            TextButton(
                onClick = onSameForPartner,
                enabled = writeEnabled(connected),
                modifier = Modifier.heightIn(min = 48.dp).testTag(XO_SAME_TAG)
            ) { Text(name) }
        }
    }
}

@Composable
private fun WayRow(title: String, tag: String, way: XoWay, lowPass: Boolean, connected: Boolean, onSet: (FilterType, Float) -> Unit) {
    var open by remember { mutableStateOf(false) }
    val enabled = writeEnabled(connected)
    Row(
        Modifier.fillMaxWidth().heightIn(min = 48.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(8.dp)
    ) {
        Text(title, Modifier.weight(1f))
        Box {
            TextButton(
                onClick = { open = true },
                enabled = enabled,
                modifier = Modifier.heightIn(min = 48.dp).testTag(XO_TYPE_TAG + tag).semantics {
                    contentDescription = "$title, ${xoShortName(way.type)}"
                }
            ) { Text(xoShortName(way.type) + " ▾") }
            DropdownMenu(expanded = open, onDismissRequest = { open = false }) {
                for (t in xoChoices(lowPass)) {
                    DropdownMenuItem(
                        text = { Text(if (t == way.type) "✓ ${xoShortName(t)}" else xoShortName(t)) },
                        onClick = {
                            open = false
                            if (t != way.type) onSet(t, way.freq)
                        }
                    )
                }
            }
        }
        if (way.type == FilterType.FLAT) {
            Spacer(Modifier.width(XO_FREQ_WIDTH))
        } else {
            ValueField(
                label = "Hz",
                value = way.freq,
                width = XO_FREQ_WIDTH - 24.dp,
                labelWidth = 24.dp,
                enabled = connected,
                range = BandFields.MIN_FREQ..20_000f,
                accessibilityLabel = "$title frequency"
            ) { onSet(way.type, it) }
        }
    }
}

private val XO_FREQ_WIDTH = 104.dp
