// -- WP PGAND -- The device-info row (PLAN-PG section 1.9, PGAND): what the open
// connection speaks, from the core's `device_protocol()` as the bridge read it once
// for this connection ([se.bofr.dspi.bridge.DspiBridge.deviceProtocol]). The words
// for the values are the Terminal's (`uconsole info`, Settings > About), so the
// clients say the same. No device I/O here: the page only shows what was read.
// The alink sender's page (AlinkSenderScreen.kt) shows the same rows.
package se.bofr.dspi.feature.settings

import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import java.util.Locale
import se.bofr.dspi.core.DeviceKind
import se.bofr.dspi.core.DeviceProtocol
import se.bofr.dspi.core.ExtEnvelope
import se.bofr.dspi.core.ProtocolGeneration

/** Test tag of the device-info section. */
const val DEVICE_INFO_TAG = "device-info"

/** What the alink sender page and About say about a sender. */
const val ALINK_SENDER_NOTE =
    "This is the audio link sender. It has no DSP: only its device information, its audio link and firmware updates are shown."

/** The generation in words. */
fun generationLabel(g: ProtocolGeneration): String = when (g) {
    ProtocolGeneration.LEGACY -> "1.1.6 (legacy)"
    ProtocolGeneration.PAGED -> "1.1.7 (paged)"
    ProtocolGeneration.UNKNOWN -> "Not reported"
}

/** The extension envelope in words. */
fun extEnvelopeLabel(e: ExtEnvelope): String = when (e) {
    ExtEnvelope.NO_CHANNEL -> "None (stock firmware)"
    ExtEnvelope.LEGACY -> "Legacy request 0x00"
    ExtEnvelope.TUNNEL -> "Page 0xE0 tunnel"
}

/** "0x01 r1, 0x05 r2": the pages with their revisions; why none are listed otherwise. */
fun pagesLabel(p: DeviceProtocol): String = when {
    p.pages.isNotEmpty() -> p.pages.joinToString(", ") { String.format(Locale.ROOT, "0x%02X r%d", it.page, it.revision) }
    p.generation == ProtocolGeneration.PAGED -> "Not listed on this connection"
    else -> "None (page 0 only)"
}

/** The rows, in order: label to value. */
fun deviceProtocolRows(p: DeviceProtocol): List<Pair<String, String>> = buildList {
    add("Board" to p.boardName.ifBlank { "Unknown board" })
    if (p.deviceKind == DeviceKind.ALINK_SENDER) add("Device" to "Audio link sender")
    add("Protocol generation" to generationLabel(p.generation))
    if (p.generation == ProtocolGeneration.PAGED) add("Protocol version" to "${p.protocolMajor}.${p.protocolMinor}")
    add("Pages" to pagesLabel(p))
    add("Extension channel" to extEnvelopeLabel(p.extEnvelope))
}

/** The device-info section: the rows for [protocol], or that nothing is connected. */
@Composable
fun DeviceInfoSection(protocol: DeviceProtocol?, connected: Boolean) {
    SettingsSection("Device", Modifier.testTag(DEVICE_INFO_TAG)) {
        when {
            protocol != null -> {
                deviceProtocolRows(protocol).forEach { (label, value) -> SettingRow(label, value) }
                if (protocol.deviceKind == DeviceKind.ALINK_SENDER) FormCaption(ALINK_SENDER_NOTE)
            }
            connected -> SettingRow("Protocol", "Not available on this connection")
            else -> SettingRow("No device connected")
        }
    }
}
