// Port of DSPi-Console-b5:DSPi Console/SpectrumAnalyserView.swift @ b5fdeac
// (SpectrumBarStrip, BarStripOptionsPanel, rtaBarStripPlot's cell layout): the
// third-octave bars above the dashboard's cards or a channel page's filters,
// one card with a cell per channel. Drawn with the analyser page's
// RtaBandsCanvas instead of the Metal bars (WP AN17).
// Deviations: no hover on a touch screen, so the options button always shows;
// the strip lays out at most two columns below 600 dp.
package se.bofr.dspi.feature.analysis

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.gestures.Orientation
import androidx.compose.foundation.gestures.draggable
import androidx.compose.foundation.gestures.rememberDraggableState
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.LongState
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import kotlin.math.min
import se.bofr.dspi.bridge.DspiBridge
import se.bofr.dspi.ui.ChannelPalette
import se.bofr.dspi.ui.WhileVisible
import se.bofr.dspi.ui.WritesExempt
import se.bofr.dspi.ui.settings.AppSettingsStore
import se.bofr.dspi.ui.settings.LocalAppSettings
import se.bofr.dspi.ui.settings.RtaSpectrumView

/** Test tag of the bar strip's card. */
const val RTA_BAR_STRIP_TAG = "rtaBarStrip"

/** Test tag of the bar strip's options button. */
const val RTA_BAR_STRIP_OPTIONS_TAG = "rtaBarStripOptions"

/** Test tag of the bar strip's resize handle. */
const val RTA_BAR_STRIP_HANDLE_TAG = "rtaBarStripHandle"

/** Test tag of one bar strip cell: analyser [channel] at [tap]. */
fun rtaBarStripCellTag(tap: Int, channel: Int): String = "rtaBarStripCell:$tap:$channel"

/**
 * Third-octave bars for the selected channels, when the page has bars
 * switched on (Swift `SpectrumBarStrip`). [own] is the page's selection: null
 * is the dashboard's stored one, a channel page passes its own channel. While
 * another console drives the analyser the strip draws the device's channels,
 * but keeps subscribing with [own], so following stays following. [onOpenAnalyser]
 * adds "Open Analyser" to the options.
 */
@Composable
fun SpectrumBarStrip(
    bridge: DspiBridge,
    modifier: Modifier = Modifier,
    own: RtaChannelSelection? = null,
    store: AppSettingsStore = LocalAppSettings.current,
    onOpenAnalyser: (() -> Unit)? = null
) {
    val rta = bridge.rta
    val bridgeStatus by bridge.status.collectAsState()
    val revision = bridge.stateRevision.collectAsState().value
    val caps by rta.caps.collectAsState()
    val ownership by rta.ownership.collectAsState()
    val settings by store.settings.collectAsState()
    val prefs = remember(settings) { settings.rtaPreferences() }

    val onDashboard = own == null
    val selection = own ?: RtaChannelSelection.resolve(RtaChannelSelection.fromStorageKey(settings.rtaDashboardSelectionKey)) {
        rta.availableChannels(it)
    }
    val drawn = RtaChannelSelection.drawn(selection, RtaChannelSelection.followed(ownership))
    val shown = bridgeStatus.connected &&
        caps?.supported == true &&
        !selection.isEmpty &&
        !drawn.isEmpty &&
        settings.rtaShows(RtaSpectrumView.BARS, onDashboard)

    // Our own request, not the drawn one, so following stays following
    val subscriptionId = remember { rtaSubscriptionIds.incrementAndGet() }
    var visible by remember { mutableStateOf(false) }
    WhileVisible(key = rta, onShow = { visible = true }, onHide = { visible = false })
    val watching = visible && shown
    LaunchedEffect(watching, selection.tap, selection.mask) {
        if (watching) rta.update(subscriptionId, selection.tap, selection.mask, false) else rta.release(subscriptionId)
    }
    DisposableEffect(rta) { onDispose { rta.release(subscriptionId) } }
    if (!shown) return

    val (render, tick) = rememberRtaFrames(rta, watching, drawn, false, prefs)
    val name = remember(revision) { { tap: Int, ch: Int -> bridge.channelLabel(rta.appChannel(tap, ch)).ifEmpty { "Ch ${ch + 1}" } } }
    val color =
        remember(revision) {
            { tap: Int, ch: Int ->
                val app = rta.appChannel(tap, ch)
                ChannelPalette.fromHex(bridge.channelColor(app), ChannelPalette.color(app, bridge.numOutputChannels))
            }
        }
    // Layout and height are view preferences: usable read-only (G1)
    WritesExempt {
        SpectrumBarStripContent(
            drawn = drawn,
            render = render,
            tick = tick,
            prefs = prefs,
            barHeight = settings.rtaBarHeight,
            channelName = name,
            channelColor = color,
            onColumns = { n -> store.update { it.copy(rtaBarColumns = n.coerceIn(1, 4)) } },
            onBarHeight = { h -> store.update { it.copy(rtaBarHeight = h) } },
            onOpenAnalyser = onOpenAnalyser,
            modifier = modifier
        )
    }
}

@Composable
internal fun SpectrumBarStripContent(
    drawn: RtaChannelSelection,
    render: RtaRenderState,
    tick: LongState,
    prefs: RtaPreferences,
    barHeight: Double,
    channelName: (tap: Int, ch: Int) -> String,
    channelColor: (tap: Int, ch: Int) -> Color,
    onColumns: (Int) -> Unit,
    onBarHeight: (Double) -> Unit,
    onOpenAnalyser: (() -> Unit)?,
    modifier: Modifier = Modifier
) {
    val count = drawn.channels.size
    val single = count == 1
    val scale = remember(prefs) { RtaScale(prefs.floorDb, prefs.ceilingDb) }
    val colors = remember(drawn, channelColor) { drawn.channels.map { channelColor(drawn.tap, it) } }
    // The dragged height, held locally until the drag ends: one store write per drag
    var dragging by remember { mutableStateOf<Double?>(null) }
    val height = dragging ?: barHeight
    val shape = RoundedCornerShape(10.dp)
    val edge = if (single) colors[0].copy(alpha = 0.3f) else MaterialTheme.colorScheme.outline.copy(alpha = 0.2f)
    BoxWithConstraints(
        modifier
            .fillMaxWidth()
            .testTag(RTA_BAR_STRIP_TAG)
            .clip(shape)
            .background(MaterialTheme.colorScheme.surfaceVariant.copy(alpha = 0.6f))
            .border(1.dp, edge, shape)
    ) {
        val maxColumns = if (maxWidth < 600.dp) 2 else 4
        val columns = rtaBarColumns(count, min(prefs.barColumns, maxColumns))
        val rows = drawn.channels.indices.chunked(columns)
        val cellHeight = rtaBarStripCellHeight(height, single).dp
        Column(Modifier.fillMaxWidth().padding(10.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
            for ((r, row) in rows.withIndex()) {
                Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                    for ((i, slot) in row.withIndex()) {
                        val ch = drawn.channels[slot]
                        Column(
                            Modifier.weight(1f).testTag(rtaBarStripCellTag(drawn.tap, ch)),
                            verticalArrangement = Arrangement.spacedBy(2.dp)
                        ) {
                            Row(
                                Modifier.height(18.dp),
                                verticalAlignment = Alignment.CenterVertically,
                                horizontalArrangement = Arrangement.spacedBy(5.dp)
                            ) {
                                Dot(colors[slot], 5.dp)
                                Text(
                                    channelName(drawn.tap, ch),
                                    fontSize = 10.sp,
                                    fontWeight = FontWeight.SemiBold,
                                    color = secondaryText(),
                                    maxLines = 1,
                                    overflow = TextOverflow.Ellipsis,
                                    modifier = Modifier.weight(1f)
                                )
                                // The options sit at the end of the top-right cell's name row: no extra height
                                if (r == 0 && i == row.lastIndex) {
                                    BarStripOptions(single, maxColumns, prefs.barColumns, onColumns, onOpenAnalyser)
                                }
                            }
                            RtaBandsCanvas(
                                render,
                                slot,
                                colors[slot],
                                scale,
                                prefs.showPeakHold,
                                tick,
                                Modifier.fillMaxWidth().height(cellHeight)
                            )
                        }
                    }
                    repeat(columns - row.size) { Spacer(Modifier.weight(1f)) }
                }
            }
        }
        // Drag strip in the card's bottom padding, like the one under the response graph
        // The state's lambda is kept across recompositions: it reads the current values
        val density = LocalDensity.current.density
        val drag by rememberUpdatedState { delta: Float ->
            val start = dragging ?: barHeight
            dragging = rtaBarStripDraggedHeight(start, delta / density, rows.size, single)
        }
        val dragState = rememberDraggableState { delta -> drag(delta) }
        Box(
            Modifier
                .align(Alignment.BottomCenter)
                .fillMaxWidth()
                .height(10.dp)
                .testTag(RTA_BAR_STRIP_HANDLE_TAG)
                .semantics { contentDescription = "Resize the spectrum bars" }
                .draggable(
                    dragState,
                    Orientation.Vertical,
                    onDragStopped = {
                        dragging?.let(onBarHeight)
                        dragging = null
                    }
                )
        )
    }
}

/** Swift `BarStripOptionsPanel`: the column layout (not for one channel) and the analyser page. */
@Composable
private fun BarStripOptions(single: Boolean, maxColumns: Int, chosen: Int, onColumns: (Int) -> Unit, onOpenAnalyser: (() -> Unit)?) {
    if (single && onOpenAnalyser == null) return
    var open by remember { mutableStateOf(false) }
    Box {
        Text(
            "⚙",
            fontSize = 12.sp,
            color = if (open) MaterialTheme.colorScheme.onSurface else secondaryText(),
            modifier = Modifier
                .size(18.dp)
                .clip(RoundedCornerShape(4.dp))
                .clickable { open = !open }
                .testTag(RTA_BAR_STRIP_OPTIONS_TAG)
                .semantics { contentDescription = "Spectrum strip options" }
        )
        DropdownMenu(expanded = open, onDismissRequest = { open = false }) {
            if (!single) {
                Text(
                    "LAYOUT",
                    fontSize = 10.sp,
                    fontWeight = FontWeight.Bold,
                    color = secondaryText(),
                    modifier = Modifier.padding(horizontal = 12.dp, vertical = 6.dp)
                )
                for (n in 1..maxColumns) {
                    val on = min(chosen, maxColumns) == n
                    DropdownMenuItem(
                        text = {
                            Text(
                                if (n == 1) "1 column" else "Up to $n columns",
                                fontWeight = if (on) FontWeight.SemiBold else FontWeight.Normal,
                                color = if (on) MaterialTheme.colorScheme.primary else MaterialTheme.colorScheme.onSurface
                            )
                        },
                        onClick = {
                            onColumns(n)
                            open = false
                        }
                    )
                }
                if (onOpenAnalyser != null) HorizontalDivider()
            }
            if (onOpenAnalyser != null) {
                DropdownMenuItem(
                    text = { Text("Open Analyser") },
                    onClick = {
                        open = false
                        onOpenAnalyser()
                    }
                )
            }
        }
    }
}
