// Port of DSPi-Console-b5:DSPi Console/DSPi_ConsoleApp.swift `AdvancedSettingsTab`.
// The "Onboarding (Developer)" section is not ported (onboarding is Phase 6).
// WP AN23: the one way into Core Voltage (CoreVoltageScreen.kt).
package se.bofr.dspi.feature.settings

import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import se.bofr.dspi.bridge.DspiBridge
import se.bofr.dspi.core.VregCalPhase
import se.bofr.dspi.ui.settings.AppSettingsStore
import se.bofr.dspi.ui.writesAllowed

@Composable
fun AdvancedSettingsTab(bridge: DspiBridge, appSettings: AppSettingsStore, canWrite: Boolean = writesAllowed()) {
    val s by appSettings.settings.collectAsState()
    val connected = bridge.status.collectAsState().value.connected
    val voltage by bridge.deviceSettings.coreVoltage.collectAsState()
    val calibration by bridge.coreVoltageCalibration.view.collectAsState()
    var coreVoltageOpen by rememberSaveable { mutableStateOf(false) }
    if (coreVoltageOpen) {
        CoreVoltageScreen(bridge, canWrite) { coreVoltageOpen = false }
        return
    }
    SettingsForm {
        // -- WP ANVOL -- an app preference: usable while read-only
        item {
            SettingsSection("Volume Buttons") {
                FormSwitchRow(
                    "Volume Buttons Control User Volume",
                    "While this app is open with a DSPi, the volume buttons change the DSPi's User Volume in 1 dB steps " +
                        "instead of this device's own volume.",
                    s.volumeKeysControlUserVolume
                ) { v ->
                    appSettings.update { it.copy(volumeKeysControlUserVolume = v) }
                }
            }
        }
        item {
            SettingsSection("Channel Names") {
                SettingRow("Reset all channel names to factory defaults.", null, enabled = connected) {
                    OutlinedButton(onClick = { bridge.resetChannelNames() }, enabled = connected && canWrite) { Text("Reset") }
                }
            }
        }
        // CG11: only with EXT_CORE_VREG listed and a writable connection
        if (connected && canWrite && voltage.supported) {
            item {
                SettingsSection(CORE_VOLTAGE_TITLE) {
                    val trial = voltage.status?.trialActive == true
                    val detail = calibration?.takeIf { it.phase != VregCalPhase.IDLE }?.message
                        ?: if (trial) "A trial voltage is running. Open to end or keep it." else "Calibrate or set the core voltage."
                    SettingRow(CORE_VOLTAGE_TITLE, detail, onClick = { coreVoltageOpen = true }) {
                        OutlinedButton(onClick = { coreVoltageOpen = true }) { Text("Open") }
                    }
                }
            }
        }
        item {
            SettingsSection("Diagnostics") {
                FormSwitchRow("Show Debug Information", "Display additional diagnostic data in the UI", s.showDebugInfo) { v ->
                    appSettings.update { it.copy(showDebugInfo = v) }
                }
            }
        }
    }
}
