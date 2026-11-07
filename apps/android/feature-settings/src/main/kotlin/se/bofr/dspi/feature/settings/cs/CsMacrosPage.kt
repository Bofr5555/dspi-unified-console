// Macros (AN8's S10): the list, each with Run. Moved here from
// ControlSurfacesSettingsTab by WP AN9 so that AN12 owns its own file; the macro
// and step editors (S11, S12) are AN12's.
package se.bofr.dspi.feature.settings.cs

import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import se.bofr.dspi.feature.settings.CsModel
import se.bofr.dspi.feature.settings.FormCaption
import se.bofr.dspi.feature.settings.SettingRow
import se.bofr.dspi.feature.settings.SettingsForm
import se.bofr.dspi.feature.settings.SettingsSaveCoordinator
import se.bofr.dspi.feature.settings.SettingsSection
import se.bofr.dspi.feature.settings.StatusLine
import se.bofr.dspi.ui.writeEnabled

@Composable
internal fun CsMacrosPage(coordinator: SettingsSaveCoordinator, cs: CsModel) {
    val busy by coordinator.csBusy.collectAsState()
    val status by coordinator.csStatus.collectAsState()
    SettingsForm {
        if (status != null) item { StatusLine(status?.first, status?.second ?: false) }
        val macros = cs.macros.withIndex().filter { it.value.configured }
        item {
            SettingsSection("Macros") {
                if (macros.isEmpty()) FormCaption("No Macros Configured")
                macros.forEach { (i, m) ->
                    val running = cs.macroRunning == i
                    SettingRow(
                        cs.macroName(i),
                        "${m.steps.size} step${if (m.steps.size == 1) "" else "s"}" + if (running) " · Running" else ""
                    ) {
                        // Firing a macro is one click (D1), never a flash write
                        OutlinedButton(onClick = { coordinator.csOp(CsModel.macroFireOp(i)) }, enabled = writeEnabled(!busy)) {
                            Text("Run")
                        }
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
