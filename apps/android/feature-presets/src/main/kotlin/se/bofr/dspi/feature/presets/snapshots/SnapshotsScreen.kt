// WP P1AN1: the Snapshots screen (route `snapshots`), as Qt's P1QT1 window on
// a phone: the A/B card first, then the list. Simple shows it read-mostly
// (recall and A/B); Advanced also captures, renames, notes, deletes, imports
// and exports (SAF). Logic in SnapshotsModel, held by a ViewModel (E9).
package se.bofr.dspi.feature.presets.snapshots

import android.content.Context
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material3.Button
import androidx.compose.material3.Card
import androidx.compose.material3.Checkbox
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import androidx.lifecycle.viewmodel.compose.viewModel
import androidx.lifecycle.viewmodel.initializer
import androidx.lifecycle.viewmodel.viewModelFactory
import java.io.File
import java.text.DateFormat
import java.util.Date
import kotlinx.coroutines.launch
import se.bofr.dspi.bridge.DspiBridge
import se.bofr.dspi.core.AbSide
import se.bofr.dspi.core.GuardPhase
import se.bofr.dspi.core.SnapshotInfo
import se.bofr.dspi.feature.presets.DiffRowLine
import se.bofr.dspi.feature.presets.DocDiff
import se.bofr.dspi.feature.presets.PresetFileAccess
import se.bofr.dspi.ui.DspiIcons
import se.bofr.dspi.ui.alert.ConsoleAlertHost
import se.bofr.dspi.ui.writesAllowed

const val SNAPSHOTS_SCREEN_TAG = "snapshots-screen"
const val AB_TOGGLE_TAG = "ab-toggle"
const val AB_START_TAG = "ab-start"
const val CAPTURE_TAG = "snapshot-capture"

fun snapshotRowTag(id: String) = "snapshot-row:$id"

fun snapshotRecallTag(id: String) = "snapshot-recall:$id"

/** The snapshot store's folder: the app's own files, one subfolder per serial (the core's). */
fun snapshotsFolder(context: Context): String = File(context.filesDir, "snapshots").path

class SnapshotsViewModel(bridge: DspiBridge, folder: String?, private val files: PresetFileAccess?) : ViewModel() {
    val model = SnapshotsModel(bridge, viewModelScope, folder)

    /** Advanced: "Export…" for [s] through the save picker. */
    fun export(context: Context, s: SnapshotInfo) {
        val f = files ?: return
        val picked = f.startCreate(context, "${s.name.ifEmpty { "Snapshot" }}.dspipreset")
        viewModelScope.launch {
            val uri = f.await(picked, model.host) ?: return@launch
            val text = model.exportText(s) ?: return@launch
            f.write(uri, text, model.host)
        }
    }

    /** Advanced: "Import Snapshot File…" through the open picker. */
    fun import(context: Context) {
        val f = files ?: return
        val picked = f.startOpen(context)
        viewModelScope.launch {
            val uri = f.await(picked, model.host) ?: return@launch
            val text = f.read(uri, model.host) ?: return@launch
            model.importText(text)
        }
    }

    companion object {
        fun factory(bridge: DspiBridge, folder: String?, files: PresetFileAccess?) = viewModelFactory {
            initializer { SnapshotsViewModel(bridge, folder, files) }
        }
    }
}

/** Entry of route [se.bofr.dspi.ui.DspiRoutes.SNAPSHOTS]. [mode]: P1AN2 passes the device's mode; Advanced until then. */
@Composable
fun SnapshotsScreen(
    bridge: DspiBridge,
    onNavigate: (String) -> Unit,
    onBack: () -> Unit,
    modifier: Modifier = Modifier,
    mode: SnapshotsMode = SnapshotsMode.ADVANCED
) {
    val context = LocalContext.current
    val vm: SnapshotsViewModel =
        viewModel(factory = SnapshotsViewModel.factory(bridge, snapshotsFolder(context), PresetFileAccess(context.applicationContext)))
    SnapshotsView(
        vm.model,
        mode,
        onBack,
        onExport = { vm.export(context, it) },
        onImport = { vm.import(context) },
        modifier = modifier
    )
}

/** The screen for [model]; previewable and Robolectric-tested without SAF. */
@Composable
fun SnapshotsView(
    model: SnapshotsModel,
    mode: SnapshotsMode,
    onBack: () -> Unit,
    onExport: (SnapshotInfo) -> Unit = {},
    onImport: () -> Unit = {},
    modifier: Modifier = Modifier
) {
    val bridge = model.bridge
    val status by bridge.status.collectAsState()
    val items by model.items.collectAsState()
    val busy by model.busy.collectAsState()
    val result by model.result.collectAsState()
    val compare by model.compare.collectAsState()
    val guard by model.guard.collectAsState()
    val choice by model.choice.collectAsState()
    val safeRecall by model.safeRecall.collectAsState()
    val guardDefault by model.guardDefault.collectAsState()
    val diff by model.diff.collectAsState()
    val connected = status.connected
    val advanced = mode == SnapshotsMode.ADVANCED
    val canWrite = connected && writesAllowed() && !busy
    val guardWaiting = guard.phase == GuardPhase.WAITING
    LaunchedEffect(model, connected) { if (connected) model.load() }
    Surface(modifier.fillMaxSize().testTag(SNAPSHOTS_SCREEN_TAG)) {
        Column(Modifier.fillMaxSize()) {
            Row(Modifier.fillMaxWidth().padding(horizontal = 4.dp), verticalAlignment = Alignment.CenterVertically) {
                TextButton(onClick = onBack) { Text("‹ Back") }
                Text("Snapshots", style = MaterialTheme.typography.titleLarge, modifier = Modifier.weight(1f))
                if (advanced) {
                    TextButton(onClick = model::askCapture, enabled = connected && !busy, modifier = Modifier.testTag(CAPTURE_TAG)) {
                        Text("Capture…")
                    }
                }
            }
            LazyColumn(Modifier.fillMaxSize()) {
                item {
                    AbCard(
                        compareActive = compare.active,
                        side = compare.side,
                        aName = compare.aName,
                        bName = compare.bName,
                        items = items,
                        choice = choice,
                        enabled = canWrite && !guardWaiting,
                        guardWaiting = guardWaiting,
                        onChooseA = model::chooseA,
                        onChooseB = model::chooseB,
                        onStart = model::startCompare,
                        onToggle = model::toggle,
                        onDiff = model::showDiff,
                        onKeep = model::keep
                    )
                }
                item {
                    if (guardDefault) {
                        Caption("Recalls over DSPi Link come with Keep and Revert.")
                    } else {
                        Row(
                            Modifier.fillMaxWidth().clickable(role = Role.Checkbox) {
                                model.setSafeRecall(!safeRecall)
                            }.padding(horizontal = 8.dp),
                            verticalAlignment = Alignment.CenterVertically
                        ) {
                            Checkbox(checked = safeRecall, onCheckedChange = null, modifier = Modifier.padding(8.dp))
                            Text("Recall with rollback", modifier = Modifier.weight(1f))
                        }
                    }
                    if (result.isNotEmpty()) Caption(result, Modifier.testTag(SNAPSHOT_RESULT_TAG))
                    HorizontalDivider()
                }
                when {
                    !connected -> item { Caption("No device connected.") }
                    items.isEmpty() -> item {
                        Caption(
                            "No snapshots of this device yet." +
                                if (advanced) " Capture one to come back to these settings later." else ""
                        )
                    }
                }
                items(items, key = { it.id }) { s ->
                    SnapshotRow(
                        s,
                        advanced = advanced,
                        canRecall = canWrite && !guardWaiting,
                        canEdit = !busy,
                        onRecall = { model.recall(s) },
                        onUseAsA = { model.chooseA(s.id) },
                        onUseAsB = { model.chooseB(s.id) },
                        onRename = { model.askRename(s) },
                        onNote = { model.askNote(s) },
                        onExport = { onExport(s) },
                        onDelete = { model.askDelete(s) }
                    )
                    HorizontalDivider(Modifier.padding(horizontal = 16.dp))
                }
                if (advanced) {
                    item {
                        TextButton(onClick = onImport, enabled = connected && !busy, modifier = Modifier.padding(8.dp)) {
                            Text("Import Snapshot File…")
                        }
                    }
                }
            }
        }
    }
    diff?.let { d -> AbDiffDialog(d, compare.aName, compare.bName, model::hideDiff) }
    ConsoleAlertHost(model.host)
}

const val SNAPSHOT_RESULT_TAG = "snapshot-result"

@Composable
private fun Caption(text: String, modifier: Modifier = Modifier) {
    Text(
        text,
        style = MaterialTheme.typography.bodySmall,
        color = MaterialTheme.colorScheme.onSurfaceVariant,
        modifier = modifier.padding(horizontal = 16.dp, vertical = 6.dp)
    )
}

/** A/B: choose A (now, or a snapshot) and B, then toggle between them; Differences; Keep A or B to end. */
@Composable
private fun AbCard(
    compareActive: Boolean,
    side: AbSide?,
    aName: String,
    bName: String,
    items: List<SnapshotInfo>,
    choice: AbChoice,
    enabled: Boolean,
    guardWaiting: Boolean,
    onChooseA: (String) -> Unit,
    onChooseB: (String) -> Unit,
    onStart: () -> Unit,
    onToggle: () -> Unit,
    onDiff: () -> Unit,
    onKeep: (AbSide) -> Unit
) {
    Card(Modifier.fillMaxWidth().padding(12.dp)) {
        Column(Modifier.padding(12.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
            Text("A/B compare", style = MaterialTheme.typography.titleSmall)
            if (compareActive) {
                Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    Column(Modifier.weight(1f)) {
                        AbLine("A", aName, side == AbSide.A)
                        AbLine("B", bName, side == AbSide.B)
                    }
                    Button(
                        onClick = onToggle,
                        enabled = enabled,
                        modifier = Modifier.testTag(AB_TOGGLE_TAG).semantics {
                            contentDescription = "Switch to ${if (side == AbSide.A) "B" else "A"}"
                        }
                    ) { Text(if (side == AbSide.A) "A → B" else "B → A") }
                }
                Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    OutlinedButton(onClick = onDiff) { Text("Differences") }
                    TextButton(onClick = { onKeep(AbSide.A) }, enabled = enabled) { Text("Keep A") }
                    TextButton(onClick = { onKeep(AbSide.B) }, enabled = enabled) { Text("Keep B") }
                }
            } else {
                Chooser("A", choice.aId, items, allowNow = true, onChoose = onChooseA)
                Chooser("B", choice.bId, items, allowNow = false, onChoose = onChooseB)
                Button(
                    onClick = onStart,
                    enabled = enabled && choice.bId.isNotEmpty() && choice.aId != choice.bId,
                    modifier = Modifier.testTag(AB_START_TAG)
                ) { Text("Compare") }
            }
            if (guardWaiting) {
                Text(
                    SnapshotsModel.FINISH_SAFE_APPLY,
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.error
                )
            }
        }
    }
}

@Composable
private fun AbLine(label: String, name: String, current: Boolean) {
    Text(
        "$label: ${name.ifEmpty { "–" }}" + if (current) "  (on now)" else "",
        style = if (current) MaterialTheme.typography.bodyLarge else MaterialTheme.typography.bodyMedium,
        color = if (current) MaterialTheme.colorScheme.primary else MaterialTheme.colorScheme.onSurface,
        maxLines = 1,
        overflow = TextOverflow.Ellipsis
    )
}

@Composable
private fun Chooser(label: String, id: String, items: List<SnapshotInfo>, allowNow: Boolean, onChoose: (String) -> Unit) {
    var open by remember { mutableStateOf(false) }
    val current = items.firstOrNull { it.id == id }?.name ?: if (allowNow) "The device now" else "Choose a snapshot"
    Box {
        Row(
            Modifier.fillMaxWidth().clickable { open = true }.padding(vertical = 4.dp),
            verticalAlignment = Alignment.CenterVertically
        ) {
            Text("$label:", style = MaterialTheme.typography.labelLarge, modifier = Modifier.padding(end = 8.dp))
            Text(current, modifier = Modifier.weight(1f), maxLines = 1, overflow = TextOverflow.Ellipsis)
            Text("▾")
        }
        DropdownMenu(expanded = open, onDismissRequest = { open = false }) {
            if (allowNow) {
                DropdownMenuItem(text = { Text("The device now") }, onClick = {
                    open = false
                    onChoose("")
                })
            }
            items.forEach { s ->
                DropdownMenuItem(text = { Text(s.name) }, onClick = {
                    open = false
                    onChoose(s.id)
                })
            }
        }
    }
}

@Composable
private fun SnapshotRow(
    s: SnapshotInfo,
    advanced: Boolean,
    canRecall: Boolean,
    canEdit: Boolean,
    onRecall: () -> Unit,
    onUseAsA: () -> Unit,
    onUseAsB: () -> Unit,
    onRename: () -> Unit,
    onNote: () -> Unit,
    onExport: () -> Unit,
    onDelete: () -> Unit
) {
    var menu by remember { mutableStateOf(false) }
    Row(
        Modifier.fillMaxWidth().testTag(snapshotRowTag(s.id)).padding(start = 16.dp, end = 4.dp, top = 6.dp, bottom = 6.dp),
        verticalAlignment = Alignment.CenterVertically
    ) {
        Column(Modifier.weight(1f)) {
            Text(s.name, style = MaterialTheme.typography.bodyLarge, maxLines = 1, overflow = TextOverflow.Ellipsis)
            Text(
                DateFormat.getDateTimeInstance(DateFormat.MEDIUM, DateFormat.SHORT).format(Date(s.savedMs)),
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant
            )
            if (s.note.isNotEmpty()) {
                Text(s.note, style = MaterialTheme.typography.bodySmall, maxLines = 2, overflow = TextOverflow.Ellipsis)
            }
        }
        TextButton(onClick = onRecall, enabled = canRecall, modifier = Modifier.testTag(snapshotRecallTag(s.id))) { Text("Recall") }
        Box {
            IconButton(onClick = { menu = true }, enabled = canEdit) { Icon(DspiIcons.more, contentDescription = "More for ${s.name}") }
            DropdownMenu(expanded = menu, onDismissRequest = { menu = false }) {
                val entries =
                    listOf("Use as A" to onUseAsA, "Use as B" to onUseAsB) +
                        if (advanced) {
                            listOf(
                                "Rename…" to onRename,
                                "Note…" to onNote,
                                "Export…" to onExport,
                                "Delete…" to onDelete
                            )
                        } else {
                            emptyList()
                        }
                entries.forEach { (title, act) ->
                    DropdownMenuItem(text = { Text(title) }, onClick = {
                        menu = false
                        act()
                    })
                }
            }
        }
    }
}

@Composable
private fun AbDiffDialog(d: DocDiff, aName: String, bName: String, onClose: () -> Unit) {
    Dialog(onDismissRequest = onClose, properties = DialogProperties(usePlatformDefaultWidth = false)) {
        Surface(Modifier.fillMaxSize()) {
            Column(Modifier.fillMaxSize()) {
                Row(Modifier.fillMaxWidth().padding(horizontal = 4.dp), verticalAlignment = Alignment.CenterVertically) {
                    TextButton(onClick = onClose) { Text("Done") }
                    Text("A → B", style = MaterialTheme.typography.titleLarge)
                }
                Caption("A: $aName   B: $bName")
                LazyColumn(Modifier.fillMaxSize()) {
                    when {
                        !d.ok -> item { Caption(d.error) }
                        d.rows.isEmpty() -> item { Caption("A and B hold the same settings.") }
                    }
                    items(d.rows) { DiffRowLine(it) }
                }
            }
        }
    }
}
