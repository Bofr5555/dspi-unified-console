// Auxiliary Outputs (AN8's S13): the list, each with its switch; a row opens the
// output's page, which until AN12's S14 is the shared binding editor (S3) with the
// core's aux rows. Moved here from ControlSurfacesSettingsTab by WP AN9 (it replaces
// the rename dialog whose Remove cleared the output unasked).
package se.bofr.dspi.feature.settings.cs

import androidx.compose.material3.Switch
import androidx.compose.runtime.Composable
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import se.bofr.dspi.bridge.DspiBridge
import se.bofr.dspi.feature.settings.CsModel
import se.bofr.dspi.feature.settings.FormCaption
import se.bofr.dspi.feature.settings.SettingRow
import se.bofr.dspi.feature.settings.SettingsForm
import se.bofr.dspi.feature.settings.SettingsSaveCoordinator
import se.bofr.dspi.feature.settings.SettingsSection
import se.bofr.dspi.feature.settings.StatusLine
import se.bofr.dspi.ui.writeEnabled

@Composable
internal fun CsAuxPage(bridge: DspiBridge, coordinator: SettingsSaveCoordinator, cs: CsModel, connected: Boolean) {
    var editing by rememberSaveable { mutableStateOf<Int?>(null) }
    editing?.let { slot ->
        CsSlotEditor(bridge, coordinator, cs, slot, connected) { editing = null }
        return
    }
    val busy by coordinator.csBusy.collectAsState()
    val status by coordinator.csStatus.collectAsState()
    val summaries = csSummaries(bridge, cs)
    SettingsForm {
        if (status != null) item { StatusLine(status?.first, status?.second ?: false) }
        val outs = cs.bindings.withIndex().filter { CsModel.isAuxType(it.value.type) }
        item {
            SettingsSection("Auxiliary Outputs") {
                if (outs.isEmpty()) FormCaption("No Auxiliary Outputs Configured")
                outs.forEach { (slot, b) ->
                    val on = cs.auxState.getOrElse(slot) { 0 } != 0
                    SettingRow(
                        cs.auxName(slot),
                        summaries[slot] ?: "${CsModel.typeName(b.type)} · GPIO ${b.gpio0}",
                        Modifier.testTag("cs.slot.$slot"),
                        onClick = { editing = slot }
                    ) {
                        // Switching an output is instant and never writes to flash
                        Switch(
                            checked = on,
                            onCheckedChange = { coordinator.csOp(CsModel.auxStateOp(slot, it)) },
                            enabled = writeEnabled(!busy)
                        )
                    }
                }
            }
        }
        item {
            FormCaption(
                "An on/off output follows its switch. A dimmable output follows its switch and its level, so one button and one " +
                    "knob can " +
                    "share a lamp. Turn on \"Active-Low Output\" for the relay and opto-isolator boards that switch when the pin " +
                    "goes low.\n\n" +
                    "A GPIO is a 3.3 V pin good for a few milliamps. Anything real needs a MOSFET, a transistor with a flyback " +
                    "diode, or an " +
                    "opto-isolated relay module in between, and a dimmed load should have its own supply so its switching noise " +
                    "stays out of " +
                    "the DAC.\n\nSwitching an output is instant and never writes to flash. The pin, name and power-on behaviour " +
                    "are stored on " +
                    "the device alongside the controls and share their Save and Revert."
            )
        }
    }
}
