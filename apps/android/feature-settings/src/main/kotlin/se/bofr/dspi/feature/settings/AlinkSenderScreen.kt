// -- WP PGAND -- The alink sender's page (PLAN-PG, PG-D part 1 review carry-over;
// `DeviceProtocol::device_kind`, PGFIX): the sender has no DSP, so the app shows
// this page in place of Home and no DSP screen: its device info, its audio link
// block (`alink_info`, `fetch_alink_status`, read on the core's thread when the page
// opens and on Refresh) and the firmware update. Every DSP call would answer
// Unsupported anyway; nothing here writes.
package se.bofr.dspi.feature.settings

import androidx.compose.foundation.layout.Box
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import java.util.Locale
import se.bofr.dspi.bridge.AlinkSenderView
import se.bofr.dspi.bridge.DspiBridge
import se.bofr.dspi.core.AlinkInfo
import se.bofr.dspi.core.AlinkStatus
import se.bofr.dspi.ui.DspiRoutes
import se.bofr.dspi.ui.WritesExempt

/** Test tag of the sender page. */
const val ALINK_SENDER_TAG = "alink-sender"

/** The routes the app opens for the alink sender: no DSP screen. */
val ALINK_SENDER_ROUTES = setOf(
    DspiRoutes.HOME,
    DspiRoutes.SETTINGS,
    DspiRoutes.FIRMWARE,
    DspiRoutes.GETTING_STARTED,
    DspiRoutes.WHATS_NEW
)

/** Whether [route] is opened while the alink sender is connected. */
fun allowedForAlinkSender(route: String): Boolean = route.substringBefore('/').substringBefore('?') in ALINK_SENDER_ROUTES

/** The audio link's identity rows. */
fun alinkInfoRows(i: AlinkInfo): List<Pair<String, String>> = listOf(
    "Role" to i.role,
    "Audio link protocol" to "${i.protocolVersion}",
    "Audio link outputs" to "${i.maxAlinks}",
    "Channels at most" to "${i.maxChannels}"
)

/** The audio link's live rows. */
fun alinkStatusRows(s: AlinkStatus): List<Pair<String, String>> = listOf(
    "State" to s.state,
    "Line rate" to if (s.alinkClass in 1..3) "L${s.alinkClass}" else "Unknown",
    "Sample rate" to when (s.rateCode) {
        0 -> "44.1 kHz"
        1 -> "48 kHz"
        2 -> "96 kHz"
        else -> "Unknown"
    },
    "Channels" to "${s.channels}",
    "Frame" to "${s.frame}",
    "Locked / lost" to "${s.lockCount} / ${s.lossCount}",
    "Check errors" to String.format(Locale.ROOT, "%d header, %d audio, %d superframe", s.crc8Errors, s.crc16Errors, s.superframeErrors)
)

/** The sender's page, over [bridge]; [onNavigate] opens the update and Settings. */
@Composable
fun AlinkSenderScreen(bridge: DspiBridge, onNavigate: (String) -> Unit) {
    val protocol by bridge.deviceProtocol.collectAsState()
    val connected = bridge.status.collectAsState().value.connected
    var reads by remember { mutableIntStateOf(0) }
    var link by remember { mutableStateOf<AlinkSenderView?>(null) }
    // Read on the core's thread (the bridge's FFI dispatcher); the state is set back here, on main
    LaunchedEffect(protocol, reads) { link = if (connected) bridge.fetchAlinkSender() else null }
    WritesExempt {
        Box(Modifier.testTag(ALINK_SENDER_TAG)) {
            SettingsForm {
                item {
                    SettingsSection("Audio link sender") { FormCaption(ALINK_SENDER_NOTE) }
                }
                item { DeviceInfoSection(protocol, connected) }
                item { AlinkBlock(link, onRefresh = { reads++ }) }
                item {
                    SettingsSection(null) {
                        SettingRow("Firmware Update", "Install firmware on the sender", onClick = { onNavigate(DspiRoutes.FIRMWARE) })
                        SettingRow("Settings", "This app's settings", onClick = { onNavigate(DspiRoutes.SETTINGS) })
                    }
                }
            }
        }
    }
}

@Composable
private fun AlinkBlock(link: AlinkSenderView?, onRefresh: () -> Unit) {
    SettingsSection("Audio link") {
        when {
            link == null -> SettingRow("Reading...")
            else -> {
                link.info?.let { info -> alinkInfoRows(info).forEach { (l, v) -> SettingRow(l, v) } }
                    ?: SettingRow("Audio link", "Not reported by this firmware")
                link.status?.let { s -> alinkStatusRows(s).forEach { (l, v) -> SettingRow(l, v) } }
                if (link.status == null && link.error.isNotEmpty()) {
                    SettingRow("Status", "Could not be read: ${link.error}", detailColor = MaterialTheme.colorScheme.error)
                }
            }
        }
        TextButton(onClick = onRefresh) { Text("Refresh") }
    }
}
