// The pop-out graph window (DSPi-Console-b5:DSPi Console/GraphView.swift
// BodePlotView(isPopOut: true) + GraphLegend, README "Pop-Out Graph Window")
// as a full-screen route: the graph fills the screen (landscape-friendly) and
// the legend toggles the curves underneath. With "Follow Channel Selection"
// (AppSettings.popoutGraphFollowsSelection, Graph Setup) the edited channel is
// the main window's selection (:ui-common AppState, macOS activeEqChannel);
// without it a channel menu picks the channel edited here
// (docs/UPSTREAM-DEVIATIONS.md GRAPH).
package se.bofr.dspi.feature.graph

import android.content.Context
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import se.bofr.dspi.bridge.ChannelMap
import se.bofr.dspi.bridge.DspiBridge
import se.bofr.dspi.ui.DspiRoutes
import se.bofr.dspi.ui.settings.LocalAppSettings
import se.bofr.dspi.ui.state.LocalAppState

/** The channels the graph can edit: the legend's pills (active inputs, enabled outputs). */
internal fun editableChannels(bridge: DspiBridge): List<Int> = (0 until ChannelMap.APP_CHANNEL_COUNT).filter { ch ->
    bridge.channelExists(ch) &&
        if (bridge.isInputChannel(ch)) {
            (0 until bridge.liveInputCount()).any { bridge.inputAppId(it) == ch }
        } else {
            val out = bridge.rta.tapIndex(ch)
            out >= 0 && bridge.outputEnabled(out)
        }
}

/**
 * Entry point of :feature-graph (route [DspiRoutes.GRAPH]).
 * The signature is the contract with :app — keep it.
 */
@Composable
fun GraphScreen(bridge: DspiBridge, onNavigate: (String) -> Unit, onBack: () -> Unit, modifier: Modifier = Modifier) {
    val stateRev by bridge.stateRevision.collectAsStateWithLifecycle()
    val settings by LocalAppSettings.current.settings.collectAsStateWithLifecycle()
    val selected by LocalAppState.current.activeEqChannel.collectAsStateWithLifecycle()
    val follows = settings.popoutGraphFollowsSelection
    var picked by rememberSaveable { mutableStateOf(-1) }
    var menuOpen by remember { mutableStateOf(false) }
    val liveInputs = bridge.observeLiveInputs()
    val choices = remember(stateRev, liveInputs) { editableChannels(bridge) }
    if (picked >= 0 && picked !in choices) picked = -1
    val edited = if (follows) selected?.takeIf { it in choices } ?: -1 else picked
    // Plain getters read here, keyed on the observed revision, and handed to the button and the
    // menu as values: they follow a rename by value, not by the identity of `choices`
    // (review-stalestate #7, audit N4).
    val labels =
        remember(stateRev, choices) {
            choices.associateWith { "${bridge.channelDescriptor(it)}  ${bridge.channelLabel(it)}" }
        }
    val editedDescriptor = remember(stateRev, edited) { if (edited >= 0) bridge.channelDescriptor(edited) else null }
    Column(modifier.fillMaxSize().background(Color(0xFF1A1A1C)).padding(8.dp)) {
        Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
            TextButton(onClick = onBack) { Text("‹ Back") }
            Text("Response Graph", fontWeight = FontWeight.Bold, color = Color.White, modifier = Modifier.weight(1f))
            if (follows) {
                Text(
                    if (editedDescriptor != null) "Following: $editedDescriptor" else "Following: overview",
                    fontSize = 13.sp,
                    color = Color.White.copy(alpha = 0.7f),
                    modifier = Modifier.padding(horizontal = 12.dp)
                )
            } else {
                Box {
                    TextButton(onClick = { menuOpen = true }) {
                        Text(if (editedDescriptor != null) "Edit: $editedDescriptor" else "Edit: none", fontSize = 13.sp)
                    }
                    DropdownMenu(expanded = menuOpen, onDismissRequest = { menuOpen = false }) {
                        DropdownMenuItem(text = { Text("None (view only)") }, onClick = {
                            picked = -1
                            menuOpen = false
                        })
                        for (ch in choices) {
                            DropdownMenuItem(
                                text = { Text(labels[ch].orEmpty()) },
                                onClick = {
                                    picked = ch
                                    if (!bridge.channelVisible(ch)) bridge.setChannelVisible(ch, true)
                                    menuOpen = false
                                }
                            )
                        }
                    }
                }
            }
        }
        // -- WP UAN -- (UXP A12, B10) nothing edited: say how to start; editing: the gestures, once
        if (edited < 0) {
            Text(
                GRAPH_PICK_HINT,
                fontSize = 13.sp,
                color = Color.White.copy(alpha = 0.75f),
                modifier = Modifier.padding(4.dp).testTag(GRAPH_PICK_HINT_TAG)
            )
        }
        BodePlotView(
            bridge = bridge,
            modifier = Modifier.weight(1f).fillMaxSize(),
            channels = if (edited >= 0) setOf(edited) else emptySet(),
            editable = edited >= 0,
            isPopOut = true
        )
        if (edited >= 0) GraphGestureHint()
        // -- WP UAN -- (UXP B10) a pill chooses the channel edited here; a long-press shows or hides its curve
        val appState = LocalAppState.current
        GraphLegend(bridge, Modifier.fillMaxWidth(), chosen = edited.takeIf { it >= 0 }) { ch ->
            if (follows) appState.updateSelection(ch) else picked = ch
            if (!bridge.channelVisible(ch)) bridge.setChannelVisible(ch, true)
        }
        if (edited >= 0) {
            Text(
                "Open channel page",
                fontSize = 12.sp,
                color = Color(0xFF7AA7FF),
                // -- WP AN20 -- read as a button
                modifier =
                Modifier.padding(top = 4.dp).clickable(role = Role.Button) {
                    onNavigate(DspiRoutes.channel(edited))
                }
            )
        }
    }
}

// -- WP UAN -- (UXP A12, B10)
/** What the graph page says while no channel is edited (it opened "Following: overview" in silence). */
const val GRAPH_PICK_HINT = "Tap a channel below to edit its filters."

/** Test tag of [GRAPH_PICK_HINT]. */
const val GRAPH_PICK_HINT_TAG = "graph-pick-hint"

/** The gestures, under the graph until the user says Got it (kept on this phone). */
const val GRAPH_GESTURE_HINT = "Drag a dot to move a band · double-tap to add one · pinch to change its width · long-press for more."

/** Test tag of [GRAPH_GESTURE_HINT]'s line. */
const val GRAPH_GESTURE_HINT_TAG = "graph-gesture-hint"

private const val HINT_PREFS = "dspi_hints"
private const val HINT_GESTURES = "graphGestures"

/** [GRAPH_GESTURE_HINT] with Got it, which hides it for good on this phone. */
@Composable
fun GraphGestureHint(modifier: Modifier = Modifier) {
    val context = LocalContext.current
    val prefs = remember(context) { context.getSharedPreferences(HINT_PREFS, Context.MODE_PRIVATE) }
    var shown by remember { mutableStateOf(!prefs.getBoolean(HINT_GESTURES, false)) }
    if (!shown) return
    Row(modifier.fillMaxWidth().testTag(GRAPH_GESTURE_HINT_TAG), verticalAlignment = Alignment.CenterVertically) {
        Text(GRAPH_GESTURE_HINT, fontSize = 12.sp, color = Color.White.copy(alpha = 0.75f), modifier = Modifier.weight(1f).padding(4.dp))
        TextButton(onClick = {
            prefs.edit().putBoolean(HINT_GESTURES, true).apply()
            shown = false
        }) { Text("Got it") }
    }
}
