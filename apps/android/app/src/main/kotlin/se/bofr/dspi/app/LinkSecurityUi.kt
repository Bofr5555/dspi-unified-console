// -- WP SECAN -- The hub connection's security in the UI (PLAN-SEC 4.3, owner rule
// 2026-11-15): the info button beside "(unsecured)", the new-key notice, the
// pair-again dialog of REPAIR_CODES, and the confirmation of a `dspi://pair` link.
// -- WP ANSEC -- (owner 2026-11-21) the warning colours of the unsecured bar and the
// user's acknowledgement of it.
package se.bofr.dspi.app

import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.size
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.LocalMinimumInteractiveComponentSize
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.Immutable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.unit.dp
import se.bofr.dspi.core.LinkPairLink
import se.bofr.dspi.link.FORGET_HUB_CODE
import se.bofr.dspi.link.HubSecurity
import se.bofr.dspi.link.LinkSessionState
import se.bofr.dspi.link.REPAIR_CODES
import se.bofr.dspi.ui.DspiIcons

/** Test tag of the info button beside "(unsecured)". */
internal const val UNSECURED_INFO_TAG = "unsecured-info"

/** Test tag of the narrow bar's " (unsecured)" (read once, with the bar's text). */
internal const val UNSECURED_SUFFIX_TAG = "unsecured-suffix"

/** Test tag of the explanation it opens. */
internal const val UNSECURED_INFO_DIALOG_TAG = "unsecured-info-dialog"

/** Its accessible name. */
internal const val UNSECURED_INFO_LABEL = "Why unsecured?"

// -- WP ANSEC --
/** The info dialog's acknowledgement: the bar is no longer highlighted for this hub. */
const val UNSECURED_ACK_BUTTON = "I understand, don't highlight this hub"

/** The same on a phone's strip overflow. */
const val UNSECURED_ACK_MENU = "Don't highlight this hub"

/** The info dialog's undo once acknowledged (Settings › Networking has it too). */
const val UNSECURED_UNACK_BUTTON = "Highlight this hub again"

/** What the info dialog offers: acknowledge (not yet acknowledged) or undo it; null = neither (no Networking). */
internal data class UnsecuredAckActions(val onAcknowledge: (() -> Unit)? = null, val onHighlightAgain: (() -> Unit)? = null)

/**
 * The unsecured bar's warning colours before the user acknowledges it: amber, not the lost
 * connection's red ([se.bofr.dspi.ui.link.LinkBanner]); [content] is the text and icon on
 * [container], at least WCAG AA (4.5:1) in both themes.
 */
@Immutable
internal data class StripWarningColors(val container: Color, val content: Color) {
    companion object {
        val Light = StripWarningColors(container = Color(0xFFFFE08C), content = Color(0xFF3D2F00))
        val Dark = StripWarningColors(container = Color(0xFF574400), content = Color(0xFFFFE08C))

        fun of(dark: Boolean) = if (dark) Dark else Light
    }
}

/** The small info button beside "(unsecured)"; opens the core's calm explanation. */
@Composable
internal fun UnsecuredInfoButton(
    tint: Color,
    wording: UnsecuredWording = UnsecuredWording.CORE,
    // -- WP ANSEC --
    ack: UnsecuredAckActions = UnsecuredAckActions()
) {
    var open by remember { mutableStateOf(false) }
    IconButton(onClick = { open = true }, modifier = Modifier.size(32.dp).testTag(UNSECURED_INFO_TAG)) {
        Icon(DspiIcons.info, contentDescription = UNSECURED_INFO_LABEL, tint = tint, modifier = Modifier.size(18.dp))
    }
    // -- WP ANBAR -- the bar draws its buttons thin; the dialog's keep Material's 48dp
    if (open) {
        CompositionLocalProvider(LocalMinimumInteractiveComponentSize provides 48.dp) {
            UnsecuredInfoDialog(wording, ack) { open = false }
        }
    }
}

@Composable
internal fun UnsecuredInfoDialog(
    wording: UnsecuredWording = UnsecuredWording.CORE,
    // -- WP ANSEC --
    ack: UnsecuredAckActions = UnsecuredAckActions(),
    onDismiss: () -> Unit
) {
    val n = wording.info()
    // -- WP ANSEC -- the acknowledgement, or its undo, beside OK
    val other = ack.onAcknowledge?.let { UNSECURED_ACK_BUTTON to it } ?: ack.onHighlightAgain?.let { UNSECURED_UNACK_BUTTON to it }
    AlertDialog(
        onDismissRequest = onDismiss,
        modifier = Modifier.testTag(UNSECURED_INFO_DIALOG_TAG),
        title = { Text(n.title) },
        text = {
            Column {
                Text(n.reason)
                Spacer(Modifier.height(8.dp))
                Text(n.action, style = MaterialTheme.typography.bodySmall)
            }
        },
        confirmButton = { TextButton(onClick = onDismiss) { Text("OK") } },
        dismissButton = other?.let { (label, act) ->
            {
                TextButton(onClick = {
                    act()
                    onDismiss()
                }) { Text(label) }
            }
        }
    )
}

/** The failed dial offers pairing again: one of [REPAIR_CODES]. */
fun offersRePair(s: LinkSessionState): Boolean = s is LinkSessionState.Failed && s.code in REPAIR_CODES

/** Why pairing again is offered, by the core's code; nothing was sent to the hub in any of them. */
fun rePairText(s: LinkSessionState.Failed): String {
    val hub = s.hub.name
    return when (s.code) {
        "hub_key_changed" ->
            "$hub presented another key than the one it had when this app paired with it, so nothing was sent to it. " +
                "If the hub was set up again or its key was reset, pair again and check that the key matches the one the hub shows."
        "hub_key_unverified" -> if (s.fromPairLink) {
            "$hub did not present the key in the pairing link, so no PIN was sent. The link may be old or not from this hub. " +
                "Connect to the hub as usual instead."
        } else {
            "$hub did not present the key it was expected to have, so nothing was stored. Pair again and check the key the hub shows."
        }
        "tls_refused" ->
            "$hub was set up with encryption, or says it supports it, but refused an encrypted connection. Nothing was sent unencrypted. " +
                "Check the hub's encryption setting, then pair again. Pairing again uses an encrypted connection only."
        // -- WP PINTLS -- the one offer is Forget Hub, and the words say what it gives up
        FORGET_HUB_CODE ->
            "$hub was paired with encryption before, so this app connects to it only encrypted, and it refused encryption. " +
                "Nothing was sent, not even the PIN. Turn TLS on in the hub's settings if you can. Forget Hub deletes its pairing " +
                "and key; it can then be paired without encryption, and on this network the PIN and settings could be read or changed."
        else -> "$hub accepts only encrypted connections. Pair again to set one up."
    }
}

/** -- WP PINTLS -- The dialog's button: Forget Hub, Connect (a pairing link's) or Pair Again. */
fun rePairLabel(s: LinkSessionState.Failed): String = when {
    s.code == FORGET_HUB_CODE -> "Forget Hub"
    s.fromPairLink && s.code == "hub_key_unverified" -> "Connect"
    else -> "Pair Again"
}

/** Test tag of the pair-again dialog. */
internal const val REPAIR_DIALOG_TAG = "repair-dialog"

@Composable
internal fun RePairDialog(s: LinkSessionState.Failed, onRePair: () -> Unit, onDismiss: () -> Unit) {
    val label = rePairLabel(s) // -- WP PINTLS --
    AlertDialog(
        onDismissRequest = onDismiss,
        modifier = Modifier.testTag(REPAIR_DIALOG_TAG),
        title = { Text("Could not connect securely") },
        text = { Text(rePairText(s)) },
        confirmButton = { TextButton(onClick = onRePair) { Text(label) } },
        dismissButton = { TextButton(onClick = onDismiss) { Text("Cancel") } }
    )
}

/** "Encrypted connection set up; hub key …", once per hub key. */
@Composable
internal fun HubKeyNoticeDialog(security: HubSecurity, hubName: String, onDismiss: () -> Unit) {
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text("Encrypted connection set up") },
        text = {
            Column {
                Text("The connection to $hubName is now encrypted. Hub key:")
                Spacer(Modifier.height(4.dp))
                Text(security.hubKey, fontFamily = FontFamily.Monospace)
                Spacer(Modifier.height(8.dp))
                Text(
                    "From now on this app connects to the hub only if it presents this key.",
                    style = MaterialTheme.typography.bodySmall
                )
            }
        },
        confirmButton = { TextButton(onClick = onDismiss) { Text("OK") } }
    )
}

/** Test tag of the pairing link's confirmation. */
internal const val PAIR_LINK_DIALOG_TAG = "pair-link-dialog"

/**
 * A `dspi://pair` link arrived (a scanned QR code, from any app): a hint, never a bypass.
 * Nothing is dialled before this tap, and the key is checked against the hub's TLS key
 * before the PIN prompt; the prompt itself needs another tap.
 */
fun pairLinkText(link: LinkPairLink, linkEnabled: Boolean): String = buildString {
    append("Pair with the hub at ")
    append(if (':' in link.host) "[${link.host}]:${link.port}" else "${link.host}:${link.port}")
    append("? This app connects only if the hub presents this key:")
    if (!linkEnabled) append("\n\nDSPi Link is turned on for this.")
}

@Composable
internal fun PairLinkDialog(link: LinkPairLink, linkEnabled: Boolean, onContinue: () -> Unit, onDismiss: () -> Unit) {
    AlertDialog(
        onDismissRequest = onDismiss,
        modifier = Modifier.testTag(PAIR_LINK_DIALOG_TAG),
        title = { Text("Pairing link") },
        text = {
            Column {
                Text(pairLinkText(link, linkEnabled))
                Spacer(Modifier.height(4.dp))
                Text(link.hubKey, fontFamily = FontFamily.Monospace)
            }
        },
        confirmButton = { TextButton(onClick = onContinue) { Text("Continue") } },
        dismissButton = { TextButton(onClick = onDismiss) { Text("Cancel") } }
    )
}

/** A `dspi://` link the core could not read. */
@Composable
internal fun BadPairLinkDialog(reason: String, onDismiss: () -> Unit) {
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text("Not a pairing link") },
        text = { Text("This link cannot be used to pair: $reason") },
        confirmButton = { TextButton(onClick = onDismiss) { Text("OK") } }
    )
}
