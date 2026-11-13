// -- WP LKUI1 -- the System screen (route DspiRoutes.SYSTEM): the hub's audio link
// installation as the web console's System view shows it (SW3), touch-sized. Every
// element with its role and link state, the link delay with ms beside it, the link
// delay range, the hop offset and the phase error; the scenes; and Clock and links,
// folded until opened. The hub's role decides what may be changed: a Viewer gets
// nothing that writes, and G1's read-only state turns writing controls off too.
// Words: docs/GLOSSARY.md 3.1; firmware names only in the help line under a field.
package se.bofr.dspi.feature.home.system

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Button
import androidx.compose.material3.Checkbox
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.focus.onFocusChanged
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import se.bofr.dspi.bridge.DspiBridge
import se.bofr.dspi.core.LinkSystemUnit
import se.bofr.dspi.ui.DspiColors
import se.bofr.dspi.ui.DspiTheme
import se.bofr.dspi.ui.DspiType
import se.bofr.dspi.ui.HorizontalRule
import se.bofr.dspi.ui.dspiCard
import se.bofr.dspi.ui.rememberWhileVisible
import se.bofr.dspi.ui.writeEnabled

/** Entry point for [se.bofr.dspi.ui.DspiRoutes.SYSTEM]. */
@Composable
fun SystemScreen(bridge: DspiBridge, @Suppress("UNUSED_PARAMETER") onNavigate: (String) -> Unit, onBack: () -> Unit) {
    val scope = rememberCoroutineScope()
    val model = remember(bridge) { SystemScreenModel(bridge, scope) }
    // -- fixup lk1 -- only while shown (decision E6): nothing is asked while the app is stopped
    val visible = rememberWhileVisible()
    LaunchedEffect(model, visible) { if (visible) model.poll() }
    SystemView(model, onBack)
}

/** Health's colour: meaning only (docs/UI-MODES.md §7). */
internal fun healthColor(c: DspiColors, health: String): Color = when (health) {
    "ok" -> c.green
    "warning" -> c.orange
    "problem" -> c.red
    else -> c.gray
}

private fun stateColor(c: DspiColors, u: LinkSystemUnit): Color = when {
    !u.connected -> c.gray
    u.state == "holdover" -> c.red
    u.state == "searching" || u.state == "off" || u.state == "unknown" -> c.orange
    else -> c.green
}

private fun ms(v: Double?) = v?.let { "%.2f ms".format(it) }.orEmpty()

private fun roleTitle(r: String) = when (r) {
    "admin" -> "Admin"
    "control" -> "Control"
    "viewer" -> "Viewer"
    else -> r
}

@OptIn(ExperimentalLayoutApi::class)
@Composable
internal fun SystemView(model: SystemScreenModel, onBack: () -> Unit) {
    val c = DspiTheme.colors
    val s by model.state.collectAsStateWithLifecycle()
    val view = s.view
    Column(Modifier.fillMaxSize().background(c.mainContent)) {
        Row(Modifier.fillMaxWidth().padding(start = 4.dp, end = 16.dp, top = 4.dp), verticalAlignment = Alignment.CenterVertically) {
            TextButton(onClick = onBack) { Text("‹ Back") }
            Text("System", style = DspiType.headline, color = c.label, modifier = Modifier.semantics { heading() })
        }
        if (view == null) {
            Box(Modifier.fillMaxSize().padding(24.dp), contentAlignment = Alignment.Center) {
                Text(
                    s.error.ifEmpty { "Open a DSPi on a hub with an audio link to see its installation here." },
                    style = DspiType.body,
                    color = c.secondaryLabel
                )
            }
            return@Column
        }
        Column(
            Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(horizontal = 16.dp, vertical = 8.dp),
            verticalArrangement = Arrangement.spacedBy(12.dp)
        ) {
            // Status
            Row(
                Modifier.fillMaxWidth().semantics(mergeDescendants = true) { contentDescription = "Status: ${view.summary}" },
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(8.dp)
            ) {
                Box(Modifier.size(10.dp).background(healthColor(c, view.health), CircleShape))
                Text(view.summary, style = DspiType.headline, color = c.label, modifier = Modifier.weight(1f))
            }
            Text("Role on the hub: ${roleTitle(view.role)}", style = DspiType.caption, color = c.secondaryLabel)
            if (s.error.isNotEmpty()) Text(s.error, style = DspiType.caption, color = c.orange)
            for (a in view.alarms) {
                Text(
                    "${a.subject}: ${a.message}" + if (a.state == "acknowledged") " (acknowledged)" else "",
                    style = DspiType.caption,
                    color = if (a.severity == "critical") c.red else c.orange
                )
            }

            // Scenes
            if (view.scenes.isNotEmpty()) {
                SectionTitle("Scenes")
                val recallOn = writeEnabled(s.mayRecall && !s.busy)
                FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                    view.scenes.forEachIndexed { i, scene ->
                        val current = s.recalledScene == scene.name
                        val label = "${i + 1}  ${scene.name}"
                        val description = "Recall ${scene.name}" + when (scene.systemMute) {
                            true -> "; it sets the system mute"
                            false -> "; it clears the system mute"
                            null -> ""
                        }
                        if (current) {
                            Button(
                                onClick = { model.recall(scene.id) },
                                enabled = recallOn,
                                modifier = Modifier.heightIn(min = 48.dp).semantics { contentDescription = description }
                            ) { Text(label) }
                        } else {
                            OutlinedButton(
                                onClick = { model.recall(scene.id) },
                                enabled = recallOn,
                                modifier = Modifier.heightIn(min = 48.dp).semantics { contentDescription = description }
                            ) { Text(label) }
                        }
                    }
                }
                if (!s.mayRecall) Text(view.recallNote, style = DspiType.caption, color = c.secondaryLabel)
                s.lastRecall?.let { r ->
                    Text(r.message, style = DspiType.body, color = if (r.taken) c.label else c.red)
                    if (r.writes.isNotEmpty()) Text(r.writes.joinToString("   "), style = DspiType.caption2, color = c.secondaryLabel)
                }
            }

            // Units
            SectionTitle("Units")
            Column(Modifier.fillMaxWidth().dspiCard()) {
                view.units.forEachIndexed { i, u ->
                    if (i > 0) HorizontalRule(Modifier.fillMaxWidth())
                    UnitRow(u, c)
                }
            }
            for (issue in view.issues) Text(issue, style = DspiType.caption, color = c.orange)

            // Clock and links
            ClockAndLinks(model, s)
            Spacer(Modifier.heightIn(min = 24.dp))
        }
    }
}

@Composable
private fun SectionTitle(text: String) {
    Text(
        text.uppercase(),
        style = DspiType.sectionLabel,
        color = DspiTheme.colors.secondaryLabel,
        modifier = Modifier.semantics {
            heading()
        }
    )
}

@Composable
private fun UnitRow(u: LinkSystemUnit, c: DspiColors) {
    val state = if (u.pending) "${u.stateWord}, link delay pending" else u.stateWord
    Column(
        Modifier.fillMaxWidth().padding(12.dp).semantics(mergeDescendants = true) {
            contentDescription = listOf(u.name, u.role, state).joinToString(", ")
        },
        verticalArrangement = Arrangement.spacedBy(4.dp)
    ) {
        Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            Box(Modifier.size(8.dp).background(stateColor(c, u), CircleShape))
            Text(
                u.name,
                style = DspiType.body,
                color = c.label,
                modifier = Modifier.weight(1f),
                maxLines = 1,
                overflow = TextOverflow.Ellipsis
            )
            Text(u.role, style = DspiType.caption, color = c.secondaryLabel)
        }
        Text(
            listOf(u.room, state).filter { it.isNotEmpty() }.joinToString(" · "),
            style = DspiType.caption,
            color = if (u.pending) c.orange else c.secondaryLabel
        )
        if (u.kind == "unit") {
            val delay = u.linkDelayFrames?.let { "Link delay $it frames (${ms(u.linkDelayMs)})" } ?: "Link delay –"
            val range = if (u.linkDelayMin != null && u.linkDelayMax != null) "range ${u.linkDelayMin}–${u.linkDelayMax}" else null
            Text(listOfNotNull(delay, range).joinToString(", "), style = DspiType.caption, color = c.label)
            val extra = listOfNotNull(
                u.hopOffset?.let { "Hop offset $it" },
                u.phaseErrorFrames?.let { "phase error %.2f frames".format(it) }
            )
            if (extra.isNotEmpty()) Text(extra.joinToString(", "), style = DspiType.caption, color = c.secondaryLabel)
        } else if (u.hopDelayFrames != null) {
            Text("+${u.hopDelayFrames} frames", style = DspiType.caption, color = c.secondaryLabel)
        }
    }
}

@Composable
private fun ClockAndLinks(model: SystemScreenModel, s: SystemUiState) {
    val c = DspiTheme.colors
    val view = s.view ?: return
    var open by rememberSaveable { mutableStateOf(false) }
    Row(
        Modifier.fillMaxWidth().heightIn(min = 48.dp).clickable { open = !open }
            .semantics { contentDescription = if (open) "Clock and links, open" else "Clock and links, folded" },
        verticalAlignment = Alignment.CenterVertically
    ) {
        Text("CLOCK AND LINKS", style = DspiType.sectionLabel, color = c.secondaryLabel, modifier = Modifier.weight(1f))
        Text(if (open) "Hide" else "Show", style = DspiType.caption, color = c.accent)
    }
    if (!open) return
    val editOn = writeEnabled(s.mayEdit && !s.busy)
    if (!s.mayEdit) Text("${view.editNote} This part is read only.", style = DspiType.caption, color = c.secondaryLabel)

    Row(verticalAlignment = Alignment.CenterVertically) {
        Checkbox(
            checked = s.draft.delayAuto,
            onCheckedChange = { model.setSystemDelay(it, view.systemDelayFrames) },
            enabled = editOn,
            modifier = Modifier.semantics { contentDescription = "System delay automatic" }
        )
        Text("System delay automatic", style = DspiType.body, color = c.label, modifier = Modifier.weight(1f))
    }
    DraftField(
        label = "System delay, frames",
        help = "In use: ${view.systemDelayFrames} frames (${ms(view.systemDelayMs)})",
        value = s.draft.delayFrames.toString(),
        enabled = editOn && !s.draft.delayAuto
    ) { t -> t.trim().toLongOrNull()?.let { model.setSystemDelay(false, it) } ?: false }
    for (room in view.rooms) {
        DraftField(
            label = "Lip sync, ${room.name}, µs",
            help = "Empty: the model's own",
            value = s.roomLipSyncText(room.id),
            enabled = editOn
        ) { t -> model.setRoomLipSync(room.id, t) }
    }
    for (u in view.units.filter { it.kind == "unit" }) {
        DraftField(
            label = "Measured latency, ${u.name}, µs",
            help = "Empty: the output latency counts (${u.latencyUs} µs)",
            value = s.unitLatencyText(u.id),
            enabled = editOn
        ) { t -> model.setUnitLatency(u.id, t) }
    }
    if (s.stale) {
        Text(
            "The installation changed on the hub while you edited it. Save is withdrawn: reload it and make your changes again.",
            style = DspiType.body,
            color = c.orange
        )
    }
    FlowRowButtons(model, s, editOn)
    if (s.actionMessage.isNotEmpty()) {
        Text(s.actionMessage, style = DspiType.caption, color = if (s.actionFailed) c.red else c.secondaryLabel)
    }
}

@OptIn(ExperimentalLayoutApi::class)
@Composable
private fun FlowRowButtons(model: SystemScreenModel, s: SystemUiState, editOn: Boolean) {
    var askAlign by remember { mutableStateOf(false) }
    FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
        if (!s.stale) {
            Button(onClick = {
                model.save()
            }, enabled = editOn && s.draft.dirty, modifier = Modifier.heightIn(min = 48.dp)) { Text("Save") }
        }
        val reload = if (s.stale) "Reload" else "Revert"
        if (s.stale) {
            Button(onClick = { model.reload() }, enabled = !s.busy, modifier = Modifier.heightIn(min = 48.dp)) { Text(reload) }
        } else {
            OutlinedButton(onClick = { model.reload() }, enabled = s.draft.dirty && !s.busy, modifier = Modifier.heightIn(min = 48.dp)) {
                Text(reload)
            }
        }
        OutlinedButton(
            onClick = { askAlign = true },
            enabled = editOn && !s.draft.dirty,
            modifier = Modifier.heightIn(min = 48.dp)
        ) { Text("Align link delays…") }
    }
    if (askAlign) {
        androidx.compose.material3.AlertDialog(
            onDismissRequest = { askAlign = false },
            title = { Text("Align link delays?") },
            text = { Text("Every unit restarts its outputs in step with a short silence (2 to 6 ms).") },
            confirmButton = {
                TextButton(onClick = {
                    askAlign = false
                    model.align()
                }) { Text("Align") }
            },
            dismissButton = { TextButton(onClick = { askAlign = false }) { Text("Cancel") } }
        )
    }
}

/** A number field that keeps what is typed until it is committed (Done, or focus leaves). */
@Composable
private fun DraftField(label: String, help: String, value: String, enabled: Boolean, commit: (String) -> Boolean) {
    val c = DspiTheme.colors
    var text by remember(value) { mutableStateOf(value) }
    var focused by remember { mutableStateOf(false) }
    val done = {
        if (text != value && !commit(text)) text = value
    }
    Column(Modifier.fillMaxWidth()) {
        OutlinedTextField(
            value = text,
            onValueChange = { text = it },
            label = { Text(label) },
            enabled = enabled,
            singleLine = true,
            keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Number, imeAction = ImeAction.Done),
            keyboardActions = KeyboardActions(onDone = { done() }),
            modifier = Modifier.fillMaxWidth().onFocusChanged {
                if (focused && !it.isFocused) done()
                focused = it.isFocused
            }
        )
        Text(help, style = DspiType.caption2, color = c.secondaryLabel, modifier = Modifier.padding(start = 4.dp, top = 2.dp))
    }
}
