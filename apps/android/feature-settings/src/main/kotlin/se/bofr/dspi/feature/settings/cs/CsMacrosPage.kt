// Macros (AN8's S10): the list, each with Run (Stop while it runs), Add Macro, and
// a macro's page (S11, S12 in CsMacroEditorPage). The list was moved here from
// ControlSurfacesSettingsTab by WP AN9; the pages, Add, Stop and the open draft are
// WP AN12's. A new macro is not applied until Apply (the Mac's addMacro).
package se.bofr.dspi.feature.settings.cs

import androidx.compose.foundation.layout.Box
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.testTag
import se.bofr.dspi.bridge.DspiBridge
import se.bofr.dspi.feature.settings.CsModel
import se.bofr.dspi.feature.settings.FormCaption
import se.bofr.dspi.feature.settings.SettingRow
import se.bofr.dspi.feature.settings.SettingsForm
import se.bofr.dspi.feature.settings.SettingsSaveCoordinator
import se.bofr.dspi.feature.settings.SettingsSection
import se.bofr.dspi.feature.settings.StatusLine
import se.bofr.dspi.ui.MacColors
import se.bofr.dspi.ui.writeEnabled

@Composable
internal fun CsMacrosPage(bridge: DspiBridge, coordinator: SettingsSaveCoordinator, cs: CsModel, connected: Boolean) {
    val drafts = remember(bridge) { CsRecordDrafts.of(bridge) }
    var editing by rememberSaveable { mutableStateOf<Int?>(null) }
    editing?.let { m ->
        CsMacroSlot(bridge, coordinator, drafts, cs, m, connected) { editing = null }
        return
    }
    val busy by coordinator.csBusy.collectAsState()
    val status by coordinator.csStatus.collectAsState()
    val model by bridge.cs.model.collectAsState()
    val shown = cs.macros.indices.filter { cs.macros[it].configured || drafts.isMacroDirty(it) }
    val free = cs.macros.indices.firstOrNull { !cs.macros[it].configured && !drafts.isMacroDirty(it) }
    SettingsForm {
        if (status != null) item { StatusLine(status?.first, status?.second ?: false) }
        item {
            SettingsSection("Macros") {
                if (shown.isEmpty()) FormCaption("No Macros Configured")
                shown.forEach { i ->
                    val m = cs.macros[i]
                    val running = cs.macroRunning == i
                    val pending = drafts.isMacroDirty(i)
                    SettingRow(
                        cs.macroName(i),
                        if (!m.configured) {
                            "Not applied"
                        } else {
                            "${m.steps.size} step${if (m.steps.size == 1) "" else "s"}" + (if (running) " · Running" else "") +
                                if (pending) " · Edited" else ""
                        },
                        Modifier.testTag("cs.macro.$i"),
                        detailColor = when {
                            running -> MacColors.green
                            pending -> MacColors.orange
                            else -> Color.Unspecified
                        },
                        onClick = { editing = i }
                    ) {
                        CsRunButton(coordinator, i, running, m.steps.isNotEmpty(), connected && !busy)
                    }
                }
            }
        }
        item {
            Box(Modifier.testTag("cs.macro.add")) {
                SettingRow(
                    "Add Macro",
                    if (free == null) "Every macro slot is in use." else null,
                    enabled = writeEnabled(connected && !busy && free != null)
                ) {
                    OutlinedButton(onClick = {
                        if (free != null) {
                            drafts.newMacro(free, CsRecords.maxSteps(model))
                            editing = free
                        }
                    }, enabled = writeEnabled(connected && !busy && free != null), modifier = Modifier.testTag("cs.macro.new")) {
                        Text("Add")
                    }
                }
            }
        }
        item {
            FormCaption(
                "A macro runs a short sequence of changes from a single press: select an input and load a preset, switch " +
                    "monitors, mute " +
                    "after a delay. Bind a button, switch, or remote key to \"Macro\" to fire one.\n\nEach step can wait before " +
                    "it runs, and can " +
                    "address a channel group. One macro runs at a time - firing another cancels the first at its current " +
                    "step.\n\nMacros are " +
                    "stored on the device alongside the controls and share their Save and Revert."
            )
        }
    }
}

/**
 * Run, or Stop while [running]: one tap (D1), never a flash write. It runs the
 * macro as stored on the device, so it needs [hasSteps] there.
 */
@Composable
internal fun CsRunButton(coordinator: SettingsSaveCoordinator, m: Int, running: Boolean, hasSteps: Boolean, enabled: Boolean) {
    OutlinedButton(
        onClick = { coordinator.csOp(if (running) CsRecords.macroCancelOp() else CsModel.macroFireOp(m)) },
        enabled = writeEnabled(enabled && (running || hasSteps)),
        modifier = Modifier.testTag("cs.macro.run.$m")
    ) { Text(if (running) "Stop" else "Run") }
}

/** Macro [m]'s page over its draft, kept in step with the device's copy. */
@Composable
private fun CsMacroSlot(
    bridge: DspiBridge,
    coordinator: SettingsSaveCoordinator,
    drafts: CsRecordDrafts,
    cs: CsModel,
    m: Int,
    connected: Boolean,
    onClose: () -> Unit
) {
    val model by bridge.cs.model.collectAsState()
    val saved = remember(model, m) { CsRecords.macro(model, m) }
    if (saved == null) {
        LaunchedEffect(m) { onClose() }
        return
    }
    val busy by coordinator.csBusy.collectAsState()
    val editor = remember(drafts, m) { drafts.macro(m, saved, CsRecords.maxSteps(model)) }
    LaunchedEffect(editor, saved) { editor.sync(saved) }
    CsMacroEditorPage(editor, coordinator, cs, connected, busy, onClose) {
        if (editor.state.value.stored) coordinator.csOp(CsRecords.macroClearOp(m))
        drafts.dropMacro(m)
        onClose()
    }
}
