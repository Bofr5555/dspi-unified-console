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
import androidx.compose.material3.Slider
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
import kotlin.math.roundToInt
import kotlinx.coroutines.launch
import se.bofr.dspi.bridge.DspiBridge
import se.bofr.dspi.bridge.HubServer
import se.bofr.dspi.core.AbSide
import se.bofr.dspi.core.GuardPhase
import se.bofr.dspi.core.MergedSnapshot
import se.bofr.dspi.core.SnapshotInfo
import se.bofr.dspi.feature.presets.DiffRowLine
import se.bofr.dspi.feature.presets.DocDiff
import se.bofr.dspi.feature.presets.PresetFileAccess
import se.bofr.dspi.ui.DspiIcons
import se.bofr.dspi.ui.alert.ConsoleAlertHost
import se.bofr.dspi.ui.collectAsStateOnMain
import se.bofr.dspi.ui.settings.AppSettings
import se.bofr.dspi.ui.settings.LocalAppSettings
import se.bofr.dspi.ui.writesAllowed

const val SNAPSHOTS_SCREEN_TAG = "snapshots-screen"
const val AB_TOGGLE_TAG = "ab-toggle"
const val AB_START_TAG = "ab-start"
const val CAPTURE_TAG = "snapshot-capture"

fun snapshotRowTag(id: String) = "snapshot-row:$id"

fun snapshotRecallTag(id: String) = "snapshot-recall:$id"

// -- WP P2AN --
const val HUB_SECTION_TAG = "snapshots-hub"

fun hubSnapshotRowTag(id: String) = "hub-snapshot-row:$id"

fun hubSnapshotRecallTag(id: String) = "hub-snapshot-recall:$id"

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
    // -- WP ANDFU -- the bridge's flows, written after its calls (the bar's poll, Keep, Revert): set on main
    val compare by model.compare.collectAsStateOnMain()
    val guard by model.guard.collectAsStateOnMain()
    val choice by model.choice.collectAsState()
    val safeRecall by model.safeRecall.collectAsState()
    val guardDefault by model.guardDefault.collectAsState()
    val diff by model.diff.collectAsState()
    // -- WP P2AN -- the hub's section (bridge flows: set on main)
    val hubCaps by model.hubCaps.collectAsStateOnMain()
    val hubItems by model.hubItems.collectAsState()
    val localMarks by model.localMarks.collectAsState()
    val hubLine by model.hubLine.collectAsState()
    // -- WP P1DCFIX -- the safe apply's timeout: the app's setting (default 30 s), adjustable in Advanced
    val settingsStore = LocalAppSettings.current
    val appSettings by settingsStore.settings.collectAsState()
    val timeoutS = AppSettings.safeApplyTimeout(appSettings.safeApplyTimeoutS)
    LaunchedEffect(model, timeoutS) { model.setTimeoutS(timeoutS) }
    val onTimeout: (Int) -> Unit = { s -> settingsStore.update { it.copy(safeApplyTimeoutS = AppSettings.safeApplyTimeout(s)) } }
    val connected = status.connected
    val advanced = mode == SnapshotsMode.ADVANCED
    val canWrite = connected && writesAllowed() && !busy
    val guardWaiting = guard.phase == GuardPhase.WAITING
    LaunchedEffect(model, connected) { if (connected) model.load() }
    LaunchedEffect(model, hubCaps.hubSnapshots) { if (connected && hubCaps.hubSnapshots) model.refreshHub() } // -- WP P2AN --
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
                    // -- WP P1DCFIX -- Simple: no rollback choice and no timeout (recall and A/B only)
                    if (guardDefault) {
                        Caption("Recalls over DSPi Link come with Keep and Revert.")
                    } else if (advanced) {
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
                    if (advanced) TimeoutRow(timeoutS, onChange = onTimeout)
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
                        mark = localMarks[s.id], // -- WP P2AN --
                        onSaveToHub = { model.saveToHub(s) }.takeIf { canWrite && localMarks[s.id] == LocalMark.HERE_ONLY },
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
                // -- WP P2AN -- "On the hub": only a hub that keeps snapshots shows it
                if (connected && hubCaps.hubSnapshots) {
                    item(key = HUB_SECTION_TAG) {
                        Text(
                            "On the hub",
                            style = MaterialTheme.typography.titleSmall,
                            modifier = Modifier.testTag(HUB_SECTION_TAG).padding(start = 16.dp, end = 16.dp, top = 12.dp, bottom = 4.dp)
                        )
                        when {
                            hubLine.isNotEmpty() -> Caption(hubLine)
                            hubItems.isEmpty() -> Caption("The hub keeps no snapshots of this device yet.")
                        }
                    }
                    items(hubItems, key = { "hub:" + it.snapshot.id }) { m ->
                        HubSnapshotRow(
                            m,
                            advanced = advanced,
                            canRecall = canWrite && !guardWaiting,
                            canWrite = canWrite,
                            canEdit = !busy,
                            onRecall = { model.recallFromHub(m) },
                            onCopyHere = { model.copyHere(m.snapshot) },
                            onRename = { model.askRenameOnHub(m.snapshot) },
                            onNote = { model.askNoteOnHub(m.snapshot) },
                            onDelete = { model.askDeleteFromHub(m.snapshot) }
                        )
                        HorizontalDivider(Modifier.padding(horizontal = 16.dp))
                    }
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

// -- WP P1DCFIX --
const val SAFE_APPLY_TIMEOUT_TAG = "safe-apply-timeout"

/** Advanced: how long a safe apply waits for Keep, 5–300 s in steps of 5. */
@Composable
private fun TimeoutRow(seconds: Int, onChange: (Int) -> Unit) {
    var dragged by remember(seconds) { mutableStateOf(seconds.toFloat()) }
    Column(Modifier.fillMaxWidth().padding(horizontal = 16.dp)) {
        Text("Rollback time: ${dragged.roundToInt()} s", style = MaterialTheme.typography.bodyMedium)
        Slider(
            value = dragged,
            onValueChange = { dragged = (it / 5f).roundToInt() * 5f },
            onValueChangeFinished = { onChange(dragged.roundToInt()) },
            valueRange = AppSettings.SAFE_APPLY_TIMEOUT_MIN_S.toFloat()..AppSettings.SAFE_APPLY_TIMEOUT_MAX_S.toFloat(),
            steps = (AppSettings.SAFE_APPLY_TIMEOUT_MAX_S - AppSettings.SAFE_APPLY_TIMEOUT_MIN_S) / 5 - 1,
            modifier = Modifier.testTag(SAFE_APPLY_TIMEOUT_TAG).semantics { contentDescription = "Rollback time" }
        )
    }
}

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

// -- WP P2AN --
/** One of this console's snapshots as it stands to the hub's ("" when it needs no word). */
internal fun markLine(mark: LocalMark?): String = when (mark) {
    LocalMark.ON_HUB_TOO -> "Also on the hub"
    LocalMark.LOCAL_COPY -> "Local copy: the hub holds a different one"
    LocalMark.HERE_ONLY, null -> ""
}

/** A row of "On the hub": Recall (this console's copy, or the hub's copied here first), and its menu. */
@Composable
private fun HubSnapshotRow(
    m: MergedSnapshot,
    advanced: Boolean,
    canRecall: Boolean,
    canWrite: Boolean,
    canEdit: Boolean,
    onRecall: () -> Unit,
    onCopyHere: () -> Unit,
    onRename: () -> Unit,
    onNote: () -> Unit,
    onDelete: () -> Unit
) {
    val s = m.snapshot
    var menu by remember { mutableStateOf(false) }
    Row(
        Modifier.fillMaxWidth().testTag(hubSnapshotRowTag(s.id)).padding(start = 16.dp, end = 4.dp, top = 6.dp, bottom = 6.dp),
        verticalAlignment = Alignment.CenterVertically
    ) {
        Column(Modifier.weight(1f)) {
            Text(s.name, style = MaterialTheme.typography.bodyLarge, maxLines = 1, overflow = TextOverflow.Ellipsis)
            Text(
                DateFormat.getDateTimeInstance(DateFormat.MEDIUM, DateFormat.SHORT).format(Date(s.savedMs)) +
                    if (m.location == HubServer.WHERE_BOTH) " · also here" else "",
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant
            )
            if (s.note.isNotEmpty()) {
                Text(s.note, style = MaterialTheme.typography.bodySmall, maxLines = 2, overflow = TextOverflow.Ellipsis)
            }
        }
        TextButton(onClick = onRecall, enabled = canRecall, modifier = Modifier.testTag(hubSnapshotRecallTag(s.id))) { Text("Recall") }
        Box {
            IconButton(onClick = {
                menu = true
            }, enabled = canEdit) { Icon(DspiIcons.more, contentDescription = "More for ${s.name} on the hub") }
            DropdownMenu(expanded = menu, onDismissRequest = { menu = false }) {
                val copy = if (m.location == HubServer.WHERE_BOTH) emptyList() else listOf("Copy here" to onCopyHere)
                val entries =
                    copy +
                        if (advanced && canWrite) {
                            listOf("Rename…" to onRename, "Note…" to onNote, "Delete from the hub…" to onDelete)
                        } else {
                            emptyList()
                        }
                if (entries.isEmpty()) {
                    DropdownMenuItem(text = { Text("Nothing to do here") }, enabled = false, onClick = {})
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
private fun SnapshotRow(
    s: SnapshotInfo,
    // -- WP P2AN --
    mark: LocalMark? = null,
    onSaveToHub: (() -> Unit)? = null,
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
            // -- WP P2AN --
            markLine(mark).takeIf { it.isNotEmpty() }?.let {
                Text(it, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
            }
        }
        TextButton(onClick = onRecall, enabled = canRecall, modifier = Modifier.testTag(snapshotRecallTag(s.id))) { Text("Recall") }
        Box {
            IconButton(onClick = { menu = true }, enabled = canEdit) { Icon(DspiIcons.more, contentDescription = "More for ${s.name}") }
            DropdownMenu(expanded = menu, onDismissRequest = { menu = false }) {
                val entries =
                    listOf("Use as A" to onUseAsA, "Use as B" to onUseAsB) +
                        listOfNotNull(onSaveToHub?.let { "Save to the hub" to it }) + // -- WP P2AN --
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
