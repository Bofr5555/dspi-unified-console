// -- WP P1AN2 --
// Settings > At Power-On (PRO1 S5, the Installer mode): everything the DSPi
// does when it starts, on one page: the startup preset, how the master volume
// and the hardware configuration are kept, the external mute's timing, the
// auxiliary outputs' states, the S/PDIF and ADAT inputs' mute until lock, and
// "Test power-on", which loads the preset the DSPi starts with. Changes apply
// at once (no Save bar; PLAN-CLIENTS 5.9): each is the bridge's write.
package se.bofr.dspi.feature.settings

import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import kotlinx.coroutines.launch
import se.bofr.dspi.bridge.DspiBridge
import se.bofr.dspi.core.DspState
import se.bofr.dspi.core.PresetStatus
import se.bofr.dspi.feature.settings.GlobalSettingsDraft.Companion.MASTER_VOLUME_MODE_INDEPENDENT
import se.bofr.dspi.feature.settings.GlobalSettingsDraft.Companion.MASTER_VOLUME_MODE_WITH_PRESET
import se.bofr.dspi.feature.settings.GlobalSettingsDraft.Companion.OUTPUT_CONFIG_MODE_INDEPENDENT
import se.bofr.dspi.feature.settings.GlobalSettingsDraft.Companion.OUTPUT_CONFIG_MODE_WITH_PRESET
import se.bofr.dspi.ui.alert.AlertButton
import se.bofr.dspi.ui.alert.AlertButtonRole
import se.bofr.dspi.ui.alert.AlertHost
import se.bofr.dspi.ui.alert.AlertStyle
import se.bofr.dspi.ui.alert.ConsoleAlert
import se.bofr.dspi.ui.alert.ConsoleAlertHost
import se.bofr.dspi.ui.writeEnabled

/** The power-on page's actions, apart from Compose. Main thread. */
object PowerOn {
    /** The slot the DSPi loads when it starts: the default with "Specified Default", the active one with "Last Used". */
    fun startupSlot(s: DspState): Int =
        if (s.presetStartupMode == GlobalSettingsDraft.STARTUP_SPECIFIED) s.presetDefaultSlot else s.activePresetSlot

    /** The line under the inputs (the firmware mutes a digital input until its receiver locks). */
    const val DIGITAL_INPUTS_LINE = "S/PDIF and ADAT inputs stay muted until they lock to a signal, so a start with no source is silent."
}

@Composable
internal fun PowerOnSettingsTab(bridge: DspiBridge, onOpen: (SettingsCategory) -> Unit) {
    val state by bridge.state.collectAsState()
    val connected = bridge.status.collectAsState().value.connected
    val scope = rememberCoroutineScope()
    val alerts = remember(bridge) { AlertHost() }
    ConsoleAlertHost(alerts)
    var tested by remember { mutableStateOf<Pair<String, Boolean>?>(null) }
    fun slotLabel(slot: Int): String {
        val occupied = (state.presetOccupied shr slot) and 1 == 1
        val name = if (occupied) state.presetNames.getOrElse(slot) { "" }.trim().ifEmpty { "Preset ${slot + 1}" } else "Empty"
        return "${slot + 1}: $name"
    }
    SettingsForm {
        item {
            SettingsSection("Startup Preset", footer = "Changes apply at once.") {
                ChoiceRow(
                    "Mode",
                    null,
                    listOf(
                        GlobalSettingsDraft.STARTUP_SPECIFIED to "Specified Default",
                        GlobalSettingsDraft.STARTUP_LAST_ACTIVE to "Last Used"
                    ),
                    state.presetStartupMode
                ) { v -> bridge.setPresetStartup(v, state.presetDefaultSlot) }
                if (state.presetStartupMode == GlobalSettingsDraft.STARTUP_SPECIFIED) {
                    ChoiceRow("Default Preset", null, (0 until 10).map { it to slotLabel(it) }, state.presetDefaultSlot) { v ->
                        bridge.setPresetStartup(state.presetStartupMode, v)
                    }
                }
                val slot = PowerOn.startupSlot(state)
                SettingRow("Test power-on", "Loads preset ${slotLabel(slot)}, the one the DSPi starts with.") {
                    OutlinedButton(
                        onClick = {
                            alerts.show(
                                ConsoleAlert(
                                    "Test Power-On",
                                    "This loads preset ${slot + 1} as the DSPi does when it starts. Changes not saved to a preset are lost.",
                                    AlertStyle.WARNING,
                                    listOf(
                                        AlertButton("Load", AlertButtonRole.DEFAULT) {
                                            scope.launch {
                                                val st = bridge.loadPreset(slot)
                                                tested = if (st == PresetStatus.OK) {
                                                    "Preset ${slot + 1} loaded, as at power-on." to false
                                                } else {
                                                    "The preset did not load (${st.failureText})." to true
                                                }
                                            }
                                        },
                                        AlertButton("Cancel", AlertButtonRole.CANCEL)
                                    )
                                )
                            )
                        },
                        enabled = writeEnabled(connected)
                    ) { Text("Test") }
                }
                StatusLine(tested?.first, tested?.second ?: false)
            }
        }
        item {
            SettingsSection("Master Volume") {
                ChoiceRow(
                    "Mode",
                    if (state.masterVolumeMode ==
                        MASTER_VOLUME_MODE_INDEPENDENT
                    ) {
                        "Kept apart from presets, applied at power-on"
                    } else {
                        "Part of each preset"
                    },
                    listOf(MASTER_VOLUME_MODE_INDEPENDENT to "Independent", MASTER_VOLUME_MODE_WITH_PRESET to "With Preset"),
                    state.masterVolumeMode
                ) { bridge.setMasterVolumeMode(it) }
            }
        }
        item {
            SettingsSection("Hardware Configuration") {
                ChoiceRow(
                    "Mode",
                    if (state.outputConfigMode ==
                        OUTPUT_CONFIG_MODE_INDEPENDENT
                    ) {
                        "Kept apart from presets, applied at power-on"
                    } else {
                        "Part of each preset"
                    },
                    listOf(OUTPUT_CONFIG_MODE_INDEPENDENT to "Independent", OUTPUT_CONFIG_MODE_WITH_PRESET to "With Preset"),
                    state.outputConfigMode
                ) { bridge.setOutputConfigMode(it) }
            }
        }
        if (state.dacMuteSupported) {
            item {
                val m = DacHwMuteConfig.from(state)
                SettingsSection(
                    "External Mute Timing",
                    footer = if (m.enabled) {
                        "The pin and its polarity are on the Global Parameters page."
                    } else {
                        "Automatic mute is off (Global Parameters)."
                    }
                ) {
                    if (m.enabled) {
                        ChoiceRow(
                            "Hold Time",
                            "Mute-attack wait before clock-stop",
                            msOptions(listOf(5, 10, 20, 50, 100), m.holdMs),
                            m.holdMs
                        ) {
                            bridge.setDacMute(m.enabled, m.activeLow, m.pin, it, m.releaseMs)
                        }
                        ChoiceRow(
                            "Release Time",
                            "Wait after un-mute before audio resumes",
                            msOptions(listOf(0, 5, 10, 20, 50, 100), m.releaseMs),
                            m.releaseMs
                        ) { bridge.setDacMute(m.enabled, m.activeLow, m.pin, m.holdMs, it) }
                    }
                }
            }
        }
        item {
            SettingsSection("Inputs and Auxiliary Outputs") {
                FormCaption(PowerOn.DIGITAL_INPUTS_LINE)
                SettingRow("Auxiliary outputs", "Their states are saved with the control surfaces and come back at power-on.", onClick = {
                    onOpen(SettingsCategory.AUX_OUTPUTS)
                }) { Text("›") }
            }
        }
    }
}
