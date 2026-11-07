// S9: one channel group's page (name, channel type, member toggles, used by), and
// the frame the AN12 record pages share: a title, the refusal, a list of rows, and
// a bar with Revert and Apply. Rows come from the core's rules (kind "group"); Apply
// is live, the save bar's Save writes flash; a refused Apply keeps the draft and
// says why (AN8 section 2, WP AN12).
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
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.unit.dp
import se.bofr.dspi.feature.settings.FormCaption
import se.bofr.dspi.feature.settings.RowDivider
import se.bofr.dspi.feature.settings.SettingsSection
import se.bofr.dspi.feature.settings.StatusLine
import se.bofr.dspi.ui.writeEnabled

internal val CS_DESTRUCTIVE = Color(0xFFFF453A)

/**
 * The shared frame: [title] and [summary] on top, the not-connected line, then
 * [content]; below, [error] and the bar with Revert and Apply. [canApply] is
 * Apply's own condition beyond [dirty] and [enabled].
 */
@Composable
internal fun CsRecordFrame(
    title: String,
    summary: String?,
    connected: Boolean,
    error: String?,
    working: Boolean,
    dirty: Boolean,
    enabled: Boolean,
    canApply: Boolean,
    onRevert: () -> Unit,
    onApply: () -> Unit,
    content: LazyListScope.() -> Unit
) {
    Column(Modifier.fillMaxSize()) {
        LazyColumn(Modifier.weight(1f).fillMaxWidth().testTag("cs.editor")) {
            item {
                Column(Modifier.padding(horizontal = 16.dp, vertical = 8.dp)) {
                    Text(title, style = MaterialTheme.typography.titleLarge)
                    summary?.takeIf { it.isNotEmpty() }?.let {
                        Text(it, style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
                    }
                }
            }
            if (!connected) item { StatusLine(CsEditor.NOT_CONNECTED, true) }
            content()
        }
        HorizontalDivider()
        // Above the bar, not in the list: a refused Apply is seen wherever the page is scrolled
        error?.let { Box(Modifier.testTag("cs.error")) { StatusLine(it, true) } }
        Row(Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 8.dp), horizontalArrangement = Arrangement.spacedBy(12.dp)) {
            if (working) Text("Applying…", Modifier.weight(1f).padding(top = 10.dp)) else Box(Modifier.weight(1f))
            OutlinedButton(onClick = onRevert, enabled = enabled && dirty, modifier = Modifier.testTag("cs.revert")) { Text("Revert") }
            Button(onClick = onApply, enabled = enabled && dirty && canApply, modifier = Modifier.testTag("cs.apply")) { Text("Apply") }
        }
    }
}

/** The name field of a record page. */
@Composable
internal fun CsNameField(name: String, placeholder: String, enabled: Boolean, onChange: (String) -> Unit) {
    SettingsSection(null) {
        OutlinedTextField(
            value = name,
            onValueChange = onChange,
            label = { Text("Name") },
            placeholder = { Text(placeholder) },
            singleLine = true,
            enabled = enabled,
            modifier = Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 6.dp).testTag("cs.name")
        )
    }
}

/** Remove, which asks first ([what] names the record, [detail] says what happens). */
@Composable
internal fun CsRemoveButton(what: String, detail: String, enabled: Boolean, onRemove: () -> Unit) {
    var ask by rememberSaveable { mutableStateOf(false) }
    TextButton(onClick = { ask = true }, enabled = enabled, modifier = Modifier.padding(horizontal = 16.dp).testTag("cs.remove")) {
        Text("Remove", color = if (enabled) CS_DESTRUCTIVE else Color.Unspecified)
    }
    if (ask) {
        AlertDialog(
            onDismissRequest = { ask = false },
            title = { Text("Remove $what?") },
            text = { Text(detail) },
            confirmButton = {
                TextButton(onClick = {
                    ask = false
                    onRemove()
                }, modifier = Modifier.testTag("cs.remove.confirm")) { Text("Remove", color = CS_DESTRUCTIVE) }
            },
            dismissButton = { TextButton(onClick = { ask = false }) { Text("Cancel") } }
        )
    }
}

/** Back with an edited draft asks: Discard drops it ([onDiscard]), Keep Editing stays. */
@Composable
internal fun CsDiscardDialog(what: String, onKeep: () -> Unit, onDiscard: () -> Unit) {
    AlertDialog(
        onDismissRequest = onKeep,
        title = { Text("Discard Changes?") },
        text = { Text("The changes to this $what have not been applied.") },
        confirmButton = {
            TextButton(onClick = onDiscard, modifier = Modifier.testTag("cs.discard")) { Text("Discard", color = CS_DESTRUCTIVE) }
        },
        dismissButton = { TextButton(onClick = onKeep) { Text("Keep Editing") } }
    )
}

/** [rows] as the core described them, one card; a long pick pushes the picker page. */
internal fun LazyListScope.csRows(
    rows: List<CsRow>,
    enabled: Boolean,
    onOpenPicker: (CsRow.Pick) -> Unit,
    onChange: (String, Any) -> Unit
) {
    item {
        SettingsSection(null) {
            rows.forEachIndexed { i, row ->
                if (i > 0) RowDivider()
                CsRowView(row, enabled, onOpenPicker, onChange)
            }
        }
    }
}

/**
 * [connected] false keeps the draft readable with every control off (never
 * replayed); [busy] is another Control Surfaces operation in flight.
 */
@Composable
internal fun CsGroupEditorPage(editor: CsGroupEditor, connected: Boolean, busy: Boolean, onClose: () -> Unit, onRemove: () -> Unit) {
    val st by editor.state.collectAsState()
    var picking by rememberSaveable(editor.slot) { mutableStateOf<String?>(null) }
    var askDiscard by rememberSaveable { mutableStateOf(false) }
    val desc = st.description
    val can = writeEnabled(connected && !busy && !st.working)
    val placeholder = "Group ${editor.slot + 1}"

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

    CsRecordFrame(
        title = st.name.ifEmpty { placeholder },
        summary = desc?.summary,
        connected = connected,
        error = st.error,
        working = st.working,
        dirty = st.dirty,
        enabled = can,
        canApply = st.appliable,
        onRevert = { editor.revert() },
        onApply = { editor.apply() }
    ) {
        item { CsNameField(st.name, placeholder, can, editor::rename) }
        if (desc == null) {
            if (st.error == null) item { FormCaption("Reading…") }
        } else {
            csRows(desc.rows, can, onOpenPicker = { picking = it.id }) { id, v -> editor.change(id, v) }
        }
        item {
            CsRemoveButton(
                st.name.ifEmpty { placeholder },
                if (st.stored) {
                    "The group is cleared on the device at once, and controls that address it stop. Save keeps the change across a restart."
                } else {
                    "The group has not been applied; its draft is dropped."
                },
                can,
                onRemove
            )
        }
    }

    if (askDiscard) {
        CsDiscardDialog("group", onKeep = { askDiscard = false }) {
            askDiscard = false
            editor.revert()
            onClose()
        }
    }
}
