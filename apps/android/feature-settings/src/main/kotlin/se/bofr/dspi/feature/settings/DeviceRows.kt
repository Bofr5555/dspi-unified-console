// WP AN6. macOS: DSPi_ConsoleApp.swift GlobalSettingsTab (Device Name, Latency),
// `InputSourceNameRow`, CoreVoltageStatusView.swift. Wording as Qt where macOS names the host.
package se.bofr.dspi.feature.settings

import androidx.compose.foundation.layout.width
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import java.util.Locale
import kotlinx.coroutines.launch
import se.bofr.dspi.bridge.CoreVoltageState
import se.bofr.dspi.bridge.DspiBridge
import se.bofr.dspi.bridge.OutputLatencyState
import se.bofr.dspi.core.CoreVregInfo
import se.bofr.dspi.core.OutputLatencyOption
import se.bofr.dspi.ui.writeEnabled

private const val LATENCY_DEFAULT_ID = 5
private const val VREG_FOOTER =
    "At its 307.2 MHz clock the device raises its stored voltage one step by itself after two watchdog hangs in one power session, up to 1.25 V."

/** "0.67 ms", "4 ms": whole milliseconds without decimals (macOS `formatLatencyMs`). */
internal fun formatLatencyMs(us: Int): String =
    if (us % 1000 == 0) "${us / 1000} ms" else String.format(Locale.ROOT, "%.2f ms", us / 1000.0)

/** The section is shown (macOS: supported and the table is not empty). */
internal fun latencyVisible(s: OutputLatencyState): Boolean = s.supported && s.options.isNotEmpty()

/** The picker's rows; a stored id the table does not list shows as "-". */
internal fun latencyChoices(options: List<OutputLatencyOption>, shown: Int): List<Pair<Int, String>> =
    options.map { it.id to formatLatencyMs(it.latencyUs) + if (it.id == LATENCY_DEFAULT_ID) " (default)" else "" } +
        if (options.none { it.id == shown }) listOf(shown to "-") else emptyList()

/** The trade-off and, when the rate caps the selected option, what it actually runs as. */
internal fun latencyCaption(options: List<OutputLatencyOption>, shown: Int): String {
    val option = options.firstOrNull { it.id == shown }
    val capped = if (option != null && option.available && option.effectiveUs != option.latencyUs) {
        " At the current sample rate the output buffer is smaller than this, so it runs as ${formatLatencyMs(option.effectiveUs)}."
    } else {
        ""
    }
    return "Lower values reduce the delay from input to output. Higher values may benefit sources exhibiting high jitter in the event of " +
        "audio stutters.$capped Values less than 2 ms have no effect upon USB Audio."
}

/** macOS `VregVoltage.label`: "1.20 V", or the raw code when the chip has no such voltage. */
internal fun vregLabel(code: Int, platform: String): String {
    val high = listOf(1.35, 1.40, 1.50, 1.60, 1.65, 1.70, 1.80, 1.90, 2.00, 2.35, 2.50, 2.65, 2.80, 3.00, 3.15, 3.30)
    val volts = when (code) {
        in 0x00..0x0F -> if (platform == "RP2040" && code < 0x06) 0.80 else 0.55 + 0.05 * code
        in 0x10..0x1F -> if (platform == "RP2040") null else high[code - 0x10]
        else -> null
    }
    return volts?.let { String.format(Locale.ROOT, "%.2f V", it) } ?: String.format(Locale.ROOT, "code 0x%02X", code)
}

/** macOS `formatUptime`. */
internal fun formatUptime(seconds: Long): String {
    val d = seconds / 86_400
    val h = (seconds % 86_400) / 3_600
    val m = (seconds % 3_600) / 60
    val s = seconds % 60
    return when {
        d > 0 -> "$d d $h h"
        h > 0 -> "$h h $m min"
        m > 0 -> "$m min $s s"
        else -> "$s s"
    }
}

/** The read-only status rows (macOS `CoreVoltageSection.rows`). */
internal fun coreVoltageRows(s: CoreVregInfo, platform: String): List<Pair<String, String>> {
    fun volts(code: Int) = vregLabel(code, platform)
    return buildList {
        add("Running at" to volts(s.liveVreg) + if (s.trialActive) " (trial)" else "")
        if (s.trialActive && s.trialVreg != s.liveVreg) add("Trial voltage" to volts(s.trialVreg))
        add("Stored" to if (s.storedSel == 0) "Build default" else volts(s.storedSel) + if (s.autoRaised) ", raised by the device" else "")
        add("Build default" to volts(s.defaultVreg))
        add("Automatic raising" to if (s.autoDisabled) "Off" else "On")
        add("Hangs this session" to "${s.hangCount}")
        add(
            "Last failed trial" to
                if (s.failedTrialVreg != 0) "${volts(s.failedTrialVreg)}, hung after ${formatUptime(s.failedTrialUptimeS)}" else "None"
        )
        add("Uptime" to formatUptime(s.uptimeS))
    }
}

/** Where a source physically comes in, so similar sources can be told apart. */
internal fun sourceDetail(source: Int, device: OutputConfigSnapshot): String = when (source) {
    0 -> "USB Audio"
    2 -> "External I2S device"
    3 -> "Lightpipe on GPIO ${device.adatInputPin}"
    else -> {
        val index = if (source == 1) 0 else source - 3
        device.spdifRxPins.getOrNull(index)?.let { "Receiver on GPIO $it" } ?: "S/PDIF receiver"
    }
}

/** One staged name: typing only edits the draft, the save bar writes it. */
@Composable
private fun DraftNameRow(
    title: String,
    detail: String,
    text: String,
    placeholder: String,
    edited: Boolean,
    clearHelp: String,
    enabled: Boolean,
    onEdit: (String) -> Unit
) {
    SettingRow(title, detail, enabled = enabled) {
        OutlinedTextField(
            value = text,
            onValueChange = onEdit,
            enabled = enabled,
            singleLine = true,
            modifier = Modifier.width(190.dp),
            label = if (edited) ({ Text("Unsaved") }) else null,
            placeholder = if (placeholder.isEmpty()) null else ({ Text(placeholder) }),
            trailingIcon = if (text.isEmpty() || !enabled) {
                null
            } else {
                {
                    TextButton(onClick = { onEdit("") }, Modifier.semantics { contentDescription = clearHelp }) { Text("✕") }
                }
            }
        )
    }
}

/** Global Parameters: hidden on firmware without device names. */
@Composable
internal fun DeviceNameSection(bridge: DspiBridge, coordinator: SettingsSaveCoordinator, connected: Boolean) {
    val names by bridge.deviceSettings.names.collectAsState()
    if (!connected || !names.deviceSupported) return
    SettingsSection(
        "Device Name",
        footer = "Stored on the device, so it keeps this name wherever it is connected. Shown on its front panel and by DSPi Link hubs."
    ) {
        DraftNameRow(
            "Name",
            bridge.selectedSerial.let { if (it.isEmpty()) "" else "Serial $it" },
            coordinator.deviceNameText(),
            "",
            coordinator.pendingDeviceName() != null,
            "Remove the name",
            writeEnabled(connected),
            coordinator::editDeviceName
        )
    }
}

/** Global Parameters: hidden on firmware without `REQ_OUTPUT_LATENCY`. */
@Composable
internal fun LatencySection(bridge: DspiBridge, coordinator: SettingsSaveCoordinator, connected: Boolean) {
    val latency by bridge.deviceSettings.latency.collectAsState()
    if (!connected || !latencyVisible(latency)) return
    val shown = coordinator.latencyShown()
    SettingsSection("Latency") {
        ChoiceRow("Target", null, latencyChoices(latency.options, shown), shown, enabled = connected, onSelect = coordinator::editLatency)
        FormCaption(latencyCaption(latency.options, shown))
    }
}

/** Inputs: hidden on firmware without source names. */
@Composable
internal fun SourceNamesSection(
    bridge: DspiBridge,
    device: OutputConfigSnapshot,
    coordinator: SettingsSaveCoordinator,
    connected: Boolean
) {
    val names by bridge.deviceSettings.names.collectAsState()
    if (!connected || !names.sourcesSupported) return
    SettingsSection(
        "Source Names",
        footer =
        "Stored on the device and shown in the sidebar and on its front panel. An empty name uses the built-in one. Held until you " +
            "tap Save."
    ) {
        bridge.inputSources.forEach { source ->
            DraftNameRow(
                source.builtIn,
                sourceDetail(source.id, device),
                coordinator.sourceNameText(source.id),
                source.builtIn,
                coordinator.pendingSourceName(source.id) != null,
                "Use the built-in name",
                writeEnabled(connected)
            ) { coordinator.editSourceName(source.id, it) }
        }
    }
}

/** Overview: read-only, and only on firmware that lists `EXT_CORE_VREG`. */
@Composable
internal fun CoreVoltageSection(bridge: DspiBridge, connected: Boolean) {
    val voltage: CoreVoltageState by bridge.deviceSettings.coreVoltage.collectAsState()
    if (!connected || !voltage.supported) return
    val scope = rememberCoroutineScope()
    LaunchedEffect(bridge) { bridge.deviceSettings.refreshCoreVoltage() }
    SettingsSection("Core Voltage", footer = VREG_FOOTER) {
        val status = voltage.status
        if (status == null) {
            FormCaption("Reading…")
        } else {
            coreVoltageRows(status, bridge.platformName).forEach { (title, value) ->
                SettingRow(title) {
                    Text(value, style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
                }
            }
        }
        SettingRow("Read the status again") {
            // A read: stays usable on read-only firmware
            TextButton(onClick = { scope.launch { bridge.deviceSettings.refreshCoreVoltage() } }) { Text("Refresh") }
        }
    }
}
