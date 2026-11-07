// S3: one binding's page (button, switch, pot, encoder, LED; the IR receiver's
// and display's wiring and aux outputs until their own pages). Rows come from the
// core's rules; the bar has Revert and Apply. Apply is live, the save bar's Save
// writes flash; a refused Apply keeps the draft and says why (AN8 section 2, WP AN9).
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

private val DESTRUCTIVE = Color(0xFFFF453A)

/**
 * [connected] false keeps the draft readable with every control off (never
 * replayed); [busy] is another Control Surfaces operation in flight.
 */
@Composable
internal fun CsBindingEditorPage(editor: CsEditor, connected: Boolean, busy: Boolean, onClose: () -> Unit, onRemove: () -> Unit) {
    val st by editor.state.collectAsState()
    var picking by rememberSaveable(editor.slot) { mutableStateOf<String?>(null) }
    var askDiscard by rememberSaveable { mutableStateOf(false) }
    var askRemove by rememberSaveable { mutableStateOf(false) }
    val desc = st.description
    val can = writeEnabled(connected && !busy && !st.working)

    // System back: the picker first, then an edited draft asks (SettingsView's own handler would close the whole page)
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
            item {
                Column(Modifier.padding(horizontal = 16.dp, vertical = 8.dp)) {
                    Text(
                        st.name.ifEmpty { desc?.title ?: "Slot ${editor.slot + 1}" },
                        style = MaterialTheme.typography.titleLarge
                    )
                    desc?.summary?.takeIf { it.isNotEmpty() }?.let {
                        Text(it, style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
                    }
                }
            }
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
                item {
                    SettingsSection(null) {
                        desc.rows.forEachIndexed { i, row ->
                            if (i > 0) RowDivider()
                            CsRowView(row, can, onOpenPicker = { picking = it.id }) { id, v -> editor.change(id, v) }
                        }
                    }
                }
            }
            item {
                TextButton(
                    onClick = { askRemove = true },
                    enabled = can,
                    modifier = Modifier.padding(horizontal = 16.dp).testTag("cs.remove")
                ) { Text("Remove", color = if (can) DESTRUCTIVE else Color.Unspecified) }
            }
        }
        HorizontalDivider()
        Row(Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 8.dp), horizontalArrangement = Arrangement.spacedBy(12.dp)) {
            if (st.working) Text("Applying…", Modifier.weight(1f).padding(top = 10.dp)) else Box(Modifier.weight(1f))
            OutlinedButton(onClick = { editor.revert() }, enabled = can && st.dirty, modifier = Modifier.testTag("cs.revert")) {
                Text("Revert")
            }
            Button(onClick = { editor.apply() }, enabled = can && st.dirty, modifier = Modifier.testTag("cs.apply")) { Text("Apply") }
        }
    }

    if (askDiscard) {
        AlertDialog(
            onDismissRequest = { askDiscard = false },
            title = { Text("Discard Changes?") },
            text = { Text("The changes to this control have not been applied.") },
            confirmButton = {
                TextButton(onClick = {
                    askDiscard = false
                    editor.revert()
                    onClose()
                }, modifier = Modifier.testTag("cs.discard")) { Text("Discard", color = DESTRUCTIVE) }
            },
            dismissButton = { TextButton(onClick = { askDiscard = false }) { Text("Keep Editing") } }
        )
    }
    if (askRemove) {
        AlertDialog(
            onDismissRequest = { askRemove = false },
            title = { Text("Remove ${st.name.ifEmpty { desc?.title ?: "Control" }}?") },
            text = { Text("The slot is freed on the device at once. Save keeps the change across a restart.") },
            confirmButton = {
                TextButton(onClick = {
                    askRemove = false
                    onRemove()
                }, modifier = Modifier.testTag("cs.remove.confirm")) { Text("Remove", color = DESTRUCTIVE) }
            },
            dismissButton = { TextButton(onClick = { askRemove = false }) { Text("Cancel") } }
        )
    }
}
