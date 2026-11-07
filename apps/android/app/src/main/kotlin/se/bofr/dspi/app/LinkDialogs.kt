// DSPi Link prompts: the pairing prompt (macOS `PairingPrompt.swift`, made
// non-modal: a state of the engine), the device choice when a hub shares
// several boards, and the USB prompt of owner decision D9. The prompt's text is
// CG10's, by `pin_source` (WP AN5).
package se.bofr.dspi.app

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import se.bofr.dspi.core.LinkDevice
import se.bofr.dspi.core.LinkPhase
import se.bofr.dspi.feature.settings.NetworkingSettingsTab
import se.bofr.dspi.link.LinkSessionState
import se.bofr.dspi.link.PairingText
import se.bofr.dspi.ui.link.NetworkingController

/** The prompts the REMOTE session needs, shown over any screen. */
@Composable
fun RemoteDialogs(
    state: LinkSessionState,
    usbPrompt: Boolean,
    onPin: (String) -> Unit,
    onCancelPin: () -> Unit,
    onChoose: (LinkDevice) -> Unit,
    onCancelChoice: () -> Unit,
    onUsbAnswer: (useUsb: Boolean) -> Unit,
    wording: LinkWording = LinkWording.CORE
) {
    when (state) {
        is LinkSessionState.NeedsPin -> PinDialog(state, wording, onPin, onCancelPin)
        is LinkSessionState.ChooseDevice -> ChooseDeviceDialog(state, onChoose, onCancelChoice)
        else -> Unit
    }
    if (usbPrompt) {
        AlertDialog(
            onDismissRequest = { onUsbAnswer(false) },
            title = { Text("A DSPi is connected over USB") },
            text = {
                Text("${state.hub?.name ?: "A hub"}'s DSPi is open over the network. Switch to the DSPi on USB?")
            },
            confirmButton = { TextButton(onClick = { onUsbAnswer(true) }) { Text("Use USB") } },
            dismissButton = { TextButton(onClick = { onUsbAnswer(false) }) { Text("Stay on the Network") } }
        )
    }
}

// -- WP AN5 --
/**
 * What to try after the hub turned a PIN down, by `pin_source`. A window PIN may
 * also have expired or its window closed, and the hub does not say which
 * (`link_pair`'s `wrong_pin`).
 */
fun wrongPinHint(pinSource: String): String = if (pinSource == FIXED_PIN) {
    "Check the hub's PIN and try again."
} else {
    "Check the PIN shown on the hub. If the hub no longer shows it, get a new one there and try again."
}

/** Test tag of the PIN prompt's hint after a PIN was turned down. */
internal const val WRONG_PIN_HINT_TAG = "wrong-pin-hint"

@Composable
private fun PinDialog(s: LinkSessionState.NeedsPin, wording: LinkWording, onPin: (String) -> Unit, onCancel: () -> Unit) {
    var pin by remember(s.hub.id) { mutableStateOf("") }
    // An error after the user's own try is the hub's answer to that PIN; one without (revoked) is not
    var tried by remember(s.hub.id) { mutableStateOf(false) }
    AlertDialog(
        onDismissRequest = {},
        title = { Text(PairingText.title(s.hub)) },
        text = { PinPromptBody(s, wording, pin, { pin = it }, tried) },
        confirmButton = {
            TextButton(
                enabled = pin.isNotEmpty() && !s.busy,
                onClick = {
                    tried = true
                    onPin(pin)
                }
            ) { Text("Pair") }
        },
        dismissButton = { TextButton(onClick = onCancel) { Text("Cancel") } }
    )
}

/**
 * The PIN prompt's body: CG10's text by `pin_source`, the field, the hub's answer and,
 * when that answer is to the user's own [tried] PIN, what to try next. Apart from the
 * dialog so it is tested without a dialog window.
 */
@Composable
internal fun PinPromptBody(s: LinkSessionState.NeedsPin, wording: LinkWording, pin: String, onPinChange: (String) -> Unit, tried: Boolean) {
    Column {
        Text(wording.of(LinkPhase.NEEDS_PAIRING, s.pinSource).text)
        Spacer(Modifier.height(12.dp))
        OutlinedTextField(
            value = pin,
            onValueChange = { v -> onPinChange(v.filter { it.isDigit() }.take(6)) },
            placeholder = { Text("000000", fontFamily = FontFamily.Monospace) },
            textStyle = MaterialTheme.typography.titleLarge.copy(fontFamily = FontFamily.Monospace, textAlign = TextAlign.Center),
            keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.NumberPassword),
            singleLine = true,
            enabled = !s.busy,
            modifier = Modifier.fillMaxWidth()
        )
        if (s.busy) LinearProgressIndicator(Modifier.fillMaxWidth().padding(top = 8.dp))
        if (s.error.isNotEmpty()) {
            Text(s.error, color = MaterialTheme.colorScheme.error, style = MaterialTheme.typography.bodySmall)
            if (tried && !s.busy) {
                Text(
                    wrongPinHint(s.pinSource),
                    Modifier.testTag(WRONG_PIN_HINT_TAG),
                    color = MaterialTheme.colorScheme.error,
                    style = MaterialTheme.typography.bodySmall
                )
            }
        }
        Spacer(Modifier.height(8.dp))
        Text(
            "The PIN and the access token cross your network unencrypted. Pair only on a network you trust.",
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant
        )
    }
}

@Composable
private fun ChooseDeviceDialog(s: LinkSessionState.ChooseDevice, onChoose: (LinkDevice) -> Unit, onCancel: () -> Unit) {
    AlertDialog(
        onDismissRequest = {},
        title = { Text("Choose a DSPi on ${s.hub.name}") },
        text = {
            Column {
                s.devices.forEach { d ->
                    Text(
                        d.name.ifEmpty { "DSPi (${d.serial.takeLast(8)})" } + if (d.fw.isNotEmpty()) "  ·  ${d.fw}" else "",
                        Modifier.fillMaxWidth().clickable { onChoose(d) }.padding(vertical = 12.dp),
                        style = MaterialTheme.typography.bodyLarge
                    )
                }
            }
        },
        confirmButton = {},
        dismissButton = { TextButton(onClick = onCancel) { Text("Cancel") } }
    )
}

/** Settings › Networking as a full-screen dialog (from the connection strip). */
@Composable
fun NetworkingDialog(controller: NetworkingController, onDone: () -> Unit) {
    Dialog(onDismissRequest = onDone, properties = DialogProperties(usePlatformDefaultWidth = false)) {
        Surface(Modifier.fillMaxSize(), color = MaterialTheme.colorScheme.background) {
            Column {
                androidx.compose.foundation.layout.Row(Modifier.fillMaxWidth().padding(4.dp)) {
                    TextButton(onClick = onDone) { Text("‹ Back") }
                    Text("Networking", style = MaterialTheme.typography.titleMedium, modifier = Modifier.padding(top = 12.dp))
                }
                NetworkingSettingsTab(controller)
            }
        }
    }
}
