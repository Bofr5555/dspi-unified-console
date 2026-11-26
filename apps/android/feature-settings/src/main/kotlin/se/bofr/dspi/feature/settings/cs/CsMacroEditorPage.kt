// S11: one macro's page (name, the steps as the core's summaries with up, down and
// remove, Add Step, Run), and S12: one step's rows from the core's rules (kind
// "step"). A step's page edits the macro's draft; the macro's Apply sends it whole.
// Up and down rather than drag, as on the Mac (AN8 section 6, WP AN12).
package se.bofr.dspi.feature.settings.cs

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import se.bofr.dspi.feature.settings.CsModel
import se.bofr.dspi.feature.settings.FormCaption
import se.bofr.dspi.feature.settings.RowDivider
import se.bofr.dspi.feature.settings.SettingRow
import se.bofr.dspi.feature.settings.SettingsSaveCoordinator
import se.bofr.dspi.feature.settings.SettingsSection
import se.bofr.dspi.ui.writeEnabled

internal fun stepTag(i: Int) = "cs.step.$i"

@Composable
internal fun CsMacroEditorPage(
    editor: CsMacroEditor,
    coordinator: SettingsSaveCoordinator,
    cs: CsModel,
    connected: Boolean,
    busy: Boolean,
    onClose: () -> Unit,
    onRemove: () -> Unit
) {
    val st by editor.state.collectAsState()
    var step by rememberSaveable(editor.slot) { mutableStateOf<Int?>(null) }
    var askDiscard by rememberSaveable { mutableStateOf(false) }
    val can = writeEnabled(connected && !busy && !st.working)
    val placeholder = "Macro ${editor.slot + 1}"

    // A step that went away (removed, or the draft reverted) closes its page
    val open = step
    if (open != null) {
        if (open in st.draft.steps.indices) {
            CsStepPage(editor, open, st, connected, can) { step = null }
            return
        }
        LaunchedEffect(open) { step = null }
    }

    BackHandler {
        if (st.dirty) askDiscard = true else onClose()
    }

    val steps = st.draft.steps
    val full = steps.size >= editor.maxSteps
    val running = cs.macroRunning == editor.slot
    CsRecordFrame(
        title = st.draft.name.ifEmpty { placeholder },
        summary = if (running) "Running" else null,
        connected = connected,
        error = st.error,
        working = st.working,
        dirty = st.dirty,
        enabled = can,
        canApply = true,
        onRevert = { editor.revert() },
        onApply = { editor.apply() }
    ) {
        item { CsNameField(st.draft.name, placeholder, can, editor::rename) }
        item {
            SettingsSection("Steps") {
                if (steps.isEmpty()) FormCaption("No steps yet. A macro with no steps does nothing.")
                steps.indices.forEach { i ->
                    if (i > 0) RowDivider()
                    val d = st.steps.getOrNull(i)
                    SettingRow(
                        "${i + 1}. ${d?.summary?.ifEmpty { null } ?: d?.title ?: "Step"}",
                        null,
                        Modifier.testTag(stepTag(i)),
                        enabled = can,
                        onClick = { step = i }
                    ) {
                        Row(verticalAlignment = Alignment.CenterVertically) {
                            TextButton(
                                onClick = { editor.moveStep(i, -1) },
                                enabled = can && i > 0,
                                // -- WP UAN -- (UXP A12) named: TalkBack read the arrows
                                modifier = Modifier.testTag("${stepTag(i)}.up").semantics { contentDescription = "Move step ${i + 1} up" }
                            ) { Text("↑", Modifier.clearAndSetSemantics {}) }
                            TextButton(
                                onClick = { editor.moveStep(i, 1) },
                                enabled = can && i < steps.size - 1,
                                modifier = Modifier.testTag("${stepTag(i)}.down").semantics { contentDescription = "Move step ${i + 1} down" }
                            ) { Text("↓", Modifier.clearAndSetSemantics {}) }
                            TextButton(
                                onClick = { editor.removeStep(i) },
                                enabled = can,
                                modifier = Modifier.testTag("${stepTag(i)}.remove").semantics { contentDescription = "Remove step ${i + 1}" }
                            ) {
                                Text(
                                    "−",
                                    Modifier.clearAndSetSemantics {},
                                    color = if (can) CS_DESTRUCTIVE else androidx.compose.ui.graphics.Color.Unspecified
                                )
                            }
                        }
                    }
                }
                RowDivider()
                SettingRow(
                    "Add Step",
                    if (full) "A macro holds up to ${editor.maxSteps} steps." else null,
                    Modifier.testTag("cs.step.add"),
                    enabled = can && !full
                ) {
                    OutlinedButton(
                        onClick = { editor.addStep { step = it } },
                        enabled = can && !full,
                        modifier = Modifier.testTag("cs.step.new")
                    ) { Text("Add") }
                }
            }
        }
        item {
            SettingsSection(null) {
                SettingRow(
                    "Run",
                    if (st.dirty) "Runs the macro as applied on the device, not this draft." else "Runs the macro now.",
                    enabled = connected && !busy
                ) {
                    CsRunButton(coordinator, editor.slot, running, st.saved.steps.isNotEmpty(), connected && !busy)
                }
            }
        }
        item {
            CsRemoveButton(
                st.draft.name.ifEmpty { placeholder },
                if (st.stored) {
                    "The macro is cleared on the device at once. Save keeps the change across a restart."
                } else {
                    "The macro has not been applied; its draft is dropped."
                },
                can,
                onRemove
            )
        }
    }

    if (askDiscard) {
        CsDiscardDialog("macro", onKeep = { askDiscard = false }) {
            askDiscard = false
            editor.revert()
            onClose()
        }
    }
}

/** S12: step [i] of the macro's draft. Back returns to the macro; its edits stay in the draft. */
@Composable
private fun CsStepPage(editor: CsMacroEditor, i: Int, st: CsMacroEditor.State, connected: Boolean, enabled: Boolean, onDone: () -> Unit) {
    var picking by rememberSaveable(editor.slot, i) { mutableStateOf<String?>(null) }
    val desc = st.steps.getOrNull(i)
    BackHandler { if (picking != null) picking = null else onDone() }

    val picker = picking?.let { id -> desc?.rows?.firstOrNull { it.id == id } as? CsRow.Pick }
    if (picker != null) {
        CsPickerPage(picker) { v ->
            picking = null
            if (v != picker.value) editor.changeStep(i, picker.id, v)
        }
        return
    }

    CsRecordFrame(
        title = "Step ${i + 1}",
        summary = desc?.summary,
        connected = connected,
        error = st.error,
        working = st.working,
        dirty = st.dirty,
        enabled = enabled,
        canApply = true,
        onRevert = { editor.revert() },
        onApply = { editor.apply() }
    ) {
        if (desc == null) {
            if (st.error == null) item { FormCaption("Reading…") }
        } else {
            csRows(desc.rows, enabled, onOpenPicker = { picking = it.id }) { id, v -> editor.changeStep(i, id, v) }
        }
        item {
            Column(Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 8.dp)) {
                Text(
                    "Changes here are part of the macro's draft: Apply sends the whole macro.",
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant
                )
                TextButton(onClick = onDone, modifier = Modifier.testTag("cs.step.done")) { Text("Back to the Macro") }
            }
        }
    }
}
