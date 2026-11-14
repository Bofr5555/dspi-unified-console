// The IR receiver's pages (AN8's S4, S5, D2; WP AN10). S4: the receiver's wiring
// (GPIO, sense) from the core's rows, with Revert and Apply, and its remote buttons
// with Add. S5: one remote button: its code, Learn, and the same rows as a control.
// D2: the learn, counting down the 10 s window with Cancel, the screen kept on.
// A learned code stays a draft until Apply; Remove asks first.
package se.bofr.dspi.feature.settings.cs

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.LazyListScope
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateMapOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalView
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.unit.dp
import se.bofr.dspi.bridge.DspiBridge
import se.bofr.dspi.feature.settings.CsModel
import se.bofr.dspi.feature.settings.FormCaption
import se.bofr.dspi.feature.settings.Pill
import se.bofr.dspi.feature.settings.RowDivider
import se.bofr.dspi.feature.settings.SettingRow
import se.bofr.dspi.feature.settings.SettingsSaveCoordinator
import se.bofr.dspi.feature.settings.SettingsSection
import se.bofr.dspi.feature.settings.StatusLine
import se.bofr.dspi.ui.MacColors
import se.bofr.dspi.ui.writeEnabled

private val DESTRUCTIVE = Color(0xFFFF453A)

/**
 * S4, for the receiver in [slot]: [editor] is its binding's draft (CsDrafts). The
 * remote buttons are device-wide; learning needs the receiver applied and active.
 */
@Composable
internal fun CsIrReceiverPage(
    bridge: DspiBridge,
    coordinator: SettingsSaveCoordinator,
    cs: CsModel,
    slot: Int,
    editor: CsEditor,
    connected: Boolean,
    busy: Boolean,
    onClose: () -> Unit,
    onRemove: () -> Unit
) {
    val store = remember(bridge) { CsIrStore.of(bridge) }
    val snapshot by bridge.cs.model.collectAsState()
    val records = remember(snapshot) { CsIr.records(snapshot) }
    LaunchedEffect(records) { store.sync(records) }
    var button by rememberSaveable { mutableStateOf<Int?>(null) }
    val receiverLive = cs.isSlotActive(slot)

    button?.let { sub ->
        val saved = records.getOrNull(sub)
        if (saved == null) {
            LaunchedEffect(sub) { button = null }
            return
        }
        val e = remember(store, sub) { store.editor(sub, saved) }
        CsRemoteButtonPage(store, coordinator, e, receiverLive, connected, onClose = { button = null })
        return
    }

    val summaries = irSummaries(bridge, records)
    var adding by remember { mutableStateOf(false) }
    var addError by remember { mutableStateOf<String?>(null) }
    val free = CsIr.firstFree(records, store.dirtySubs())
    CsWiringPage(editor, connected, busy, onClose, onRemove) { can ->
        item {
            SettingsSection("Remote Buttons") {
                FormCaption("Learn buttons on any remote and bind each to a function. ${records.count(CsIr::isLearned)}/${records.size}")
                if (!receiverLive) FormCaption(CsIr.NOT_LIVE, MacColors.orange)
                val shown = records.indices.filter { CsIr.isLearned(records[it]) || store.isDirty(it) }
                shown.forEach { sub ->
                    SettingRow(
                        summaries[sub] ?: "Remote Button",
                        CsIr.chip(records[sub]),
                        Modifier.testTag("cs.ir.$sub"),
                        onClick = { button = sub }
                    ) { if (store.isDirty(sub)) Pill("Edited", MacColors.orange) }
                }
                addError?.let { StatusLine(it, true) }
                Box(Modifier.testTag("cs.ir.add")) {
                    SettingRow(
                        "Add Remote Button",
                        if (free == null) "Every remote button is in use." else null,
                        enabled = writeEnabled(can && free != null && !adding)
                    ) {
                        OutlinedButton(
                            onClick = {
                                val sub = free ?: return@OutlinedButton
                                adding = true
                                addError = null
                                store.add(sub, records[sub]) { err ->
                                    adding = false
                                    if (err == null) button = sub else addError = err
                                }
                            },
                            enabled = writeEnabled(can && free != null && !adding)
                        ) { Text("Add") }
                    }
                }
            }
        }
    }
}

/** The core's summary of each learned button ("Raise Volume", ...), refreshed with the snapshot. */
@Composable
private fun irSummaries(bridge: DspiBridge, records: List<String>): Map<Int, String> {
    val out = remember { mutableStateMapOf<Int, String>() }
    LaunchedEffect(records) {
        // -- WP FLAKE1 -- written on the main thread, as the controls list's (publishOnMain)
        val next = HashMap<Int, String>()
        records.forEachIndexed { sub, r ->
            if (!CsIr.isLearned(r)) return@forEachIndexed
            CsRules.parseDescription(bridge.cs.describe(CsIr.KIND, sub, r)).value?.let { d ->
                next[sub] = d.summary.ifEmpty { d.title }
            }
        }
        publishOnMain(out, next)
    }
    return out
}

/**
 * The receiver's own record, as AN9's binding page (name, the core's rows,
 * Remove, Revert and Apply), with [extra] items under the rows. [extra] gets
 * whether writes are possible now.
 */
@Composable
private fun CsWiringPage(
    editor: CsEditor,
    connected: Boolean,
    busy: Boolean,
    onClose: () -> Unit,
    onRemove: () -> Unit,
    extra: LazyListScope.(Boolean) -> Unit
) {
    val st by editor.state.collectAsState()
    var picking by rememberSaveable(editor.slot) { mutableStateOf<String?>(null) }
    var askDiscard by rememberSaveable { mutableStateOf(false) }
    var askRemove by rememberSaveable { mutableStateOf(false) }
    val desc = st.description
    val can = writeEnabled(connected && !busy && !st.working)

    BackHandler {
        when {
            picking != null -> picking = null
            st.dirty -> askDiscard = true
            else -> onClose()
        }
    }
    val picker = picking?.let { id -> desc?.rows?.firstOrNull { it.id == id } as? CsRow.Pick }
    if (picker != null) {
        CsPickerPage(picker) { v ->
            picking = null
            if (v != picker.value) editor.change(picker.id, v)
        }
        return
    }

    Column(Modifier.fillMaxSize()) {
        LazyColumn(Modifier.weight(1f).fillMaxWidth().testTag("cs.editor")) {
            item { PageHeader(st.name.ifEmpty { desc?.title ?: "Slot ${editor.slot + 1}" }, desc?.summary) }
            if (!connected) item { StatusLine(CsEditor.NOT_CONNECTED, true) }
            st.error?.let { item { Box(Modifier.testTag("cs.error")) { StatusLine(it, true) } } }
            item {
                SettingsSection(null) {
                    OutlinedTextField(
                        value = st.name,
                        onValueChange = editor::rename,
                        label = { Text("Name") },
                        placeholder = { Text(desc?.title ?: "") },
                        singleLine = true,
                        enabled = can,
                        modifier = Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 6.dp).testTag("cs.name")
                    )
                }
            }
            if (desc == null) {
                if (st.error == null) item { FormCaption("Reading…") }
            } else {
                item { DescribedRows(desc, can, onOpenPicker = { picking = it.id }) { id, v -> editor.change(id, v) } }
            }
            extra(can)
            item {
                TextButton(onClick = {
                    askRemove = true
                }, enabled = can, modifier = Modifier.padding(horizontal = 16.dp).testTag("cs.remove")) {
                    Text("Remove", color = if (can) DESTRUCTIVE else Color.Unspecified)
                }
            }
        }
        ApplyBar(st.working, can && st.dirty, can && st.dirty, onRevert = { editor.revert() }, onApply = { editor.apply() })
    }

    if (askDiscard) {
        DiscardDialog("The changes to this control have not been applied.", onKeep = { askDiscard = false }) {
            askDiscard = false
            editor.revert()
            onClose()
        }
    }
    if (askRemove) {
        RemoveDialog(
            "Remove ${st.name.ifEmpty { desc?.title ?: "Control" }}?",
            "The slot is freed on the device at once. Its remote buttons stay. Save keeps the change across a restart.",
            onCancel = { askRemove = false }
        ) {
            askRemove = false
            onRemove()
        }
    }
}

/** S5: one remote button. Apply needs a learned code; Remove (of a learned one) asks first. */
@Composable
private fun CsRemoteButtonPage(
    store: CsIrStore,
    coordinator: SettingsSaveCoordinator,
    editor: CsIrEditor,
    receiverLive: Boolean,
    connected: Boolean,
    onClose: () -> Unit
) {
    val st by editor.state.collectAsState()
    val learn by store.learn.collectAsState()
    val busy by coordinator.csBusy.collectAsState()
    var picking by rememberSaveable(editor.sub) { mutableStateOf<String?>(null) }
    var askDiscard by rememberSaveable { mutableStateOf(false) }
    var askRemove by rememberSaveable { mutableStateOf(false) }
    val desc = st.description
    val can = writeEnabled(connected && !busy && !st.working)
    val savedLearned = CsIr.isLearned(st.saved)
    val leave = {
        // A fresh button never applied has nothing on the device to come back to
        if (!savedLearned) store.drop(editor.sub)
        onClose()
    }

    BackHandler {
        when {
            picking != null -> picking = null
            st.dirty -> askDiscard = true
            else -> leave()
        }
    }
    val picker = picking?.let { id -> desc?.rows?.firstOrNull { it.id == id } as? CsRow.Pick }
    if (picker != null) {
        CsPickerPage(picker) { v ->
            picking = null
            if (v != picker.value) editor.change(picker.id, v)
        }
        return
    }

    Column(Modifier.fillMaxSize()) {
        LazyColumn(Modifier.weight(1f).fillMaxWidth().testTag("cs.ir.editor")) {
            item { PageHeader(desc?.title ?: "Remote Button", desc?.summary) }
            if (!connected) item { StatusLine(CsEditor.NOT_CONNECTED, true) }
            st.error?.let { item { Box(Modifier.testTag("cs.error")) { StatusLine(it, true) } } }
            st.message?.let { (text, warn) -> item { Box(Modifier.testTag("cs.ir.message")) { StatusLine(text, warn) } } }
            item {
                SettingsSection(null) {
                    SettingRow("Code", CsIr.chip(st.draft), Modifier.testTag("cs.ir.code")) {
                        OutlinedButton(
                            onClick = { store.startLearn(editor) },
                            enabled = writeEnabled(can && receiverLive && learn == null),
                            modifier = Modifier.testTag("cs.ir.learn")
                        ) { Text(if (st.learned) "Re-learn" else "Learn Button") }
                    }
                    if (!receiverLive) FormCaption(CsIr.NOT_LIVE, MacColors.orange)
                }
            }
            if (desc == null) {
                if (st.error == null) item { FormCaption("Reading…") }
            } else {
                item { DescribedRows(desc, can, onOpenPicker = { picking = it.id }) { id, v -> editor.change(id, v) } }
            }
            if (savedLearned) {
                item {
                    TextButton(
                        onClick = { askRemove = true },
                        enabled = can,
                        modifier = Modifier.padding(horizontal = 16.dp).testTag("cs.remove")
                    ) { Text("Remove", color = if (can) DESTRUCTIVE else Color.Unspecified) }
                }
            }
            if (!st.learned) item { FormCaption(CsIr.LEARN_FIRST) }
        }
        ApplyBar(st.working, can && st.dirty, can && st.dirty && st.learned, onRevert = { editor.revert() }, onApply = { editor.apply() })
    }

    learn?.takeIf { it.sub == editor.sub }?.let { LearnDialog(it, onCancel = { store.cancelLearn() }) }
    if (askDiscard) {
        DiscardDialog("The changes to this remote button have not been applied.", onKeep = { askDiscard = false }) {
            askDiscard = false
            editor.revert()
            leave()
        }
    }
    if (askRemove) {
        RemoveDialog(
            "Remove This Remote Button?",
            "It is removed on the device at once. Save keeps the change across a restart.",
            onCancel = { askRemove = false }
        ) {
            askRemove = false
            coordinator.csOp(CsIr.clearOp(editor.sub))
            store.drop(editor.sub)
            onClose()
        }
    }
}

/** D2: the learn window counting down, with Cancel; the screen stays on while it listens. */
@Composable
private fun LearnDialog(learn: CsIrStore.Learn, onCancel: () -> Unit) {
    val view = LocalView.current
    DisposableEffect(view) {
        view.keepScreenOn = true
        onDispose { view.keepScreenOn = false }
    }
    val seconds = (learn.msLeft + 999) / 1000
    AlertDialog(
        onDismissRequest = onCancel,
        title = { Text("Learn Remote Button") },
        text = {
            Column(Modifier.testTag("cs.ir.learning"), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                Text(CsIr.LEARN_PROMPT)
                Text("${CsIr.LEARN_WAITING} $seconds s", color = MaterialTheme.colorScheme.onSurfaceVariant)
                LinearProgressIndicator(
                    progress = { (learn.msLeft.toFloat() / CsIr.LEARN_WINDOW_MS).coerceIn(0f, 1f) },
                    modifier = Modifier.fillMaxWidth()
                )
            }
        },
        confirmButton = { TextButton(onClick = onCancel, modifier = Modifier.testTag("cs.ir.learn.cancel")) { Text("Cancel") } }
    )
}

@Composable
private fun PageHeader(title: String, summary: String?) {
    Column(Modifier.padding(horizontal = 16.dp, vertical = 8.dp)) {
        Text(title, style = MaterialTheme.typography.titleLarge)
        summary?.takeIf { it.isNotEmpty() }?.let {
            Text(it, style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
        }
    }
}

@Composable
private fun DescribedRows(desc: CsDescription, can: Boolean, onOpenPicker: (CsRow.Pick) -> Unit, onChange: (String, Any) -> Unit) {
    SettingsSection(null) {
        desc.rows.forEachIndexed { i, row ->
            if (i > 0) RowDivider()
            CsRowView(row, can, onOpenPicker, onChange)
        }
    }
}

@Composable
private fun ApplyBar(working: Boolean, canRevert: Boolean, canApply: Boolean, onRevert: () -> Unit, onApply: () -> Unit) {
    HorizontalDivider()
    Row(Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 8.dp), horizontalArrangement = Arrangement.spacedBy(12.dp)) {
        if (working) Text("Applying…", Modifier.weight(1f).padding(top = 10.dp)) else Box(Modifier.weight(1f))
        OutlinedButton(onClick = onRevert, enabled = canRevert, modifier = Modifier.testTag("cs.revert")) {
            Text("Revert")
        }
        Button(onClick = onApply, enabled = canApply, modifier = Modifier.testTag("cs.apply")) { Text("Apply") }
    }
}

@Composable
private fun DiscardDialog(text: String, onKeep: () -> Unit, onDiscard: () -> Unit) {
    AlertDialog(
        onDismissRequest = onKeep,
        title = { Text("Discard Changes?") },
        text = { Text(text) },
        confirmButton = {
            TextButton(onClick = onDiscard, modifier = Modifier.testTag("cs.discard")) { Text("Discard", color = DESTRUCTIVE) }
        },
        dismissButton = { TextButton(onClick = onKeep) { Text("Keep Editing") } }
    )
}

@Composable
private fun RemoveDialog(title: String, text: String, onCancel: () -> Unit, onRemove: () -> Unit) {
    AlertDialog(
        onDismissRequest = onCancel,
        title = { Text(title) },
        text = { Text(text) },
        confirmButton = {
            TextButton(onClick = onRemove, modifier = Modifier.testTag("cs.remove.confirm")) { Text("Remove", color = DESTRUCTIVE) }
        },
        dismissButton = { TextButton(onClick = onCancel) { Text("Cancel") } }
    )
}
