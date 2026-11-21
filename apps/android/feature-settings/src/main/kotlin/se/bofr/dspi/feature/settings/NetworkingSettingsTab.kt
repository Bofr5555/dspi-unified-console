// Port of DSPi-Console release/v1.1.6 `NetworkingSettingsView.swift`, the
// client half only ("Other Hubs": Find hubs automatically, the hub rows with
// their Open/Private badge, Add by Address…). The hub half (This Hub, Paired
// Clients, sharing this phone's DSPi) is not ported: the Android app is a
// Link client only (owner decisions D1/D5). Android additions: the opt-in
// switch (D2), the client name (D7), paired hubs with Forget, and the
// trusted-network notice.
package se.bofr.dspi.feature.settings

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import se.bofr.dspi.ui.MacColors
import se.bofr.dspi.ui.WritesExempt
import se.bofr.dspi.ui.link.NetworkHubRow
import se.bofr.dspi.ui.link.NetworkingController
import se.bofr.dspi.ui.link.PairedHubRow

/** macOS LinkDiscovery.defaultPort. */
private const val DEFAULT_PORT = 11915

/** Settings › Networking (the DSPi Link client). App preferences only: usable without a device. */
@Composable
fun NetworkingSettingsTab(controller: NetworkingController) = WritesExempt {
    val s by controller.state.collectAsState()
    DisposableEffect(controller) {
        controller.setVisible(true)
        onDispose { controller.setVisible(false) }
    }
    var adding by remember { mutableStateOf(false) }
    SettingsForm {
        item {
            SettingsSection(
                "DSPi Link",
                footer = DSPI_LINK_FOOTER // -- WP ANSEC -- encrypted to this project's hubs since SECAN
            ) {
                FormSwitchRow("Control DSPi devices over the network", null, s.enabled) { controller.setEnabled(it) }
                // -- WP LKFAST --
                if (s.enabled) {
                    FormSwitchRow(RECONNECT_AT_START, null, s.reconnectAtStart) { controller.setReconnectAtStart(it) }
                }
                if (s.status.isNotEmpty()) {
                    SettingRow("Connection", s.status) {
                        if (s.connected) TextButton(onClick = { controller.disconnect() }) { Text("Disconnect") }
                    }
                }
            }
        }
        if (!s.enabled) return@SettingsForm
        item {
            SettingsSection("Local", footer = "Hubs list this name among their clients.") {
                var draft by remember(s.clientName) { mutableStateOf(s.clientName) }
                OutlinedTextField(
                    value = draft,
                    onValueChange = {
                        draft = it.take(64)
                        controller.setClientName(draft.trim())
                    },
                    label = { Text("Name") },
                    placeholder = { Text(s.defaultClientName) },
                    singleLine = true,
                    modifier = Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 8.dp)
                )
            }
        }
        item {
            SettingsSection("Other Hubs", footer = "Tap a hub to control its DSPi from here.") {
                FormSwitchRow("Find hubs automatically", null, s.findAutomatically) { controller.setFindAutomatically(it) }
                if (s.findAutomatically && s.browseError != null) {
                    FormCaption(s.browseError.orEmpty(), MacColors.orange)
                }
                s.hubs.forEach { hub -> HubRow(hub, controller) }
                if (s.hubs.isEmpty()) {
                    if (s.findAutomatically) {
                        Row(Modifier.padding(horizontal = 16.dp, vertical = 10.dp), verticalAlignment = Alignment.CenterVertically) {
                            CircularProgressIndicator(Modifier.width(16.dp), strokeWidth = 2.dp)
                            Spacer(Modifier.width(8.dp))
                            Text("Looking for hubs…", color = MaterialTheme.colorScheme.onSurfaceVariant)
                        }
                    } else {
                        FormCaption("No hubs added.")
                    }
                }
                Row(Modifier.fillMaxWidth().padding(horizontal = 8.dp), horizontalArrangement = Arrangement.End) {
                    TextButton(onClick = { adding = true }) { Text("Add by Address…") }
                }
            }
        }
        if (s.paired.isNotEmpty()) {
            item {
                // -- WP PINTLS -- Forget is the only way back to an unencrypted pairing (owner decision 2026-11-17)
                SettingsSection("Paired Hubs", footer = PAIRED_HUBS_FOOTER) {
                    s.paired.forEach { p ->
                        // -- WP SECAN -- the key it is pinned to, or that it is not
                        SettingRow(p.name.ifEmpty { p.id }, detail = pairedHubDetail(p)) {
                            TextButton(onClick = { controller.forget(p.id) }) { Text("Forget") }
                        }
                    }
                }
            }
        }
        // -- WP ANSEC -- the hubs whose unsecured connection is not highlighted, and the way back
        if (s.acknowledged.isNotEmpty()) {
            item {
                SettingsSection(ACKNOWLEDGED_HUBS_TITLE, footer = ACKNOWLEDGED_HUBS_FOOTER) {
                    s.acknowledged.forEach { a ->
                        SettingRow(a.name.ifEmpty { a.id }, detail = a.detail.ifEmpty { null }) {
                            TextButton(onClick = { controller.forgetAcknowledgement(a.id) }) { Text("Forget") }
                        }
                    }
                    if (s.acknowledged.size > 1) {
                        Row(Modifier.fillMaxWidth().padding(horizontal = 8.dp), horizontalArrangement = Arrangement.End) {
                            TextButton(onClick = { controller.forgetAcknowledgements() }) { Text(FORGET_ACKNOWLEDGEMENTS) }
                        }
                    }
                }
            }
        }
    }
    if (adding) AddHubDialog(controller, onDismiss = { adding = false })
}

@Composable
private fun HubRow(hub: NetworkHubRow, controller: NetworkingController) {
    SettingRow(
        title = hub.name,
        detail = hub.detail + if (hub.active) " · connected" else "",
        onClick = { if (!hub.active) controller.connect(hub.id) }
    ) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            if (hub.open) Pill("Open", MacColors.green) else Pill("Private", MacColors.blue)
            // -- WP SECAN -- encrypted or not, in words
            if (hub.badge.isNotEmpty()) Pill(hub.badge, if (hub.badge == UNENCRYPTED_BADGE) MacColors.red else MacColors.green)
            if (hub.manual) {
                TextButton(onClick = { controller.removeManual(hub.id) }) { Text("Remove") }
            }
        }
    }
}

/** macOS `addHubPopover`. */
@Composable
private fun AddHubDialog(controller: NetworkingController, onDismiss: () -> Unit) {
    var draft by remember { mutableStateOf("") }
    val valid = controller.isValidAddress(draft)
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text("Add a Hub by Address") },
        text = {
            Column {
                OutlinedTextField(
                    value = draft,
                    onValueChange = { draft = it },
                    placeholder = { Text("192.168.1.20 or studio.local") },
                    singleLine = true
                )
                Spacer(Modifier.width(4.dp))
                Text(
                    "Hostname or IP address. Add :port if the hub doesn't use $DEFAULT_PORT.",
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant
                )
            }
        },
        confirmButton = {
            TextButton(enabled = valid, onClick = {
                if (controller.addManual(draft)) onDismiss()
            }) { Text("Add") }
        },
        dismissButton = { TextButton(onClick = onDismiss) { Text("Cancel") } }
    )
}

// -- WP ANSEC --
/**
 * The DSPi Link section's footer. Since SECAN the connection to a uConsole for DSPi hub is TLS with
 * the hub's key pinned at pairing; Weeb Labs' DSPi Console and the DSPi bridge have no TLS.
 */
// -- WP LKFAST -- (owner 2026-11-21) by the hub, never "all unencrypted"
internal const val DSPI_LINK_FOOTER =
    "Control a DSPi on a hub (uConsole for DSPi, DSPi Console or a DSPi bridge) from here. " +
        "Whether a hub's DSPi Link connection is encrypted depends on the hub: each hub below says so, and so does the bar " +
        "while you are connected. To reach a hub from elsewhere, use a VPN such as Tailscale."

// -- WP LKFAST --
internal const val RECONNECT_AT_START = "Reconnect to the last hub at start"

internal const val ACKNOWLEDGED_HUBS_TITLE = "Acknowledged Unsecured Hubs"

internal const val ACKNOWLEDGED_HUBS_FOOTER =
    "You chose not to highlight these hubs' unencrypted DSPi Link connections. The bar still says \"(unsecured)\". " +
        "Forget highlights a hub again; so does its connection being encrypted once, or another hub answering at its address."

internal const val FORGET_ACKNOWLEDGEMENTS = "Forget All"

/** -- WP SECAN -- the hub list's badge for a hub without encryption (red). */
private const val UNENCRYPTED_BADGE = "Unencrypted"

// -- WP PINTLS -- (owner decision 2026-11-17) Pair Again keeps a hub that had a key encrypted-only;
// Forget removes everything, the only way back to an unencrypted pairing.
internal const val PAIRED_HUBS_FOOTER =
    "Pairing again with a hub that had a key keeps it encrypted-only. Forgetting a hub deletes its pairing and its key; " +
        "it asks for a PIN again next time, and a hub without encryption can then be paired unencrypted."

/** A paired hub's line: the key it is pinned to, encrypted-only until paired again, or unencrypted. */
internal fun pairedHubDetail(p: PairedHubRow): String = when {
    p.hubKey.isNotEmpty() -> "Key ${p.hubKey}"
    p.tlsOnly -> "Encrypted only; pair again"
    else -> "Unencrypted"
}
