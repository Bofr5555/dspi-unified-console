// -- WP P1AN2 --
// The Mode dialog (UI-MODES section 5: Simple, Advanced, Installer, the PIN
// field only when a PIN is set, "Remember for this device" or "Use for every
// device on this console") and the locked-user state's banner (section 2: a
// lock badge, one line saying what is locked, the installer's contact).
package se.bofr.dspi.app

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.selection.selectable
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.RadioButton
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import se.bofr.dspi.core.LockState

/** Test tags. */
internal const val MODE_DIALOG_TAG = "mode-dialog"
internal const val MODE_PIN_TAG = "mode-pin"
internal const val MODE_OK_TAG = "mode-ok"
internal const val LOCK_BANNER_TAG = "lock-banner"

/** The strip's entry to the dialog, and the unlock's while locked. */
internal const val MODE_ENTRY = "Mode…"
internal const val UNLOCK_ENTRY = "Unlock…"

// -- WP P1EFIX -- (review p1e-a F2) the banner's line is the core's row, the same in every client
/** The core's notice row for the locked-user banner; its reason is the line. */
internal const val LOCK_BANNER_NOTICE_KEY = "installer_lock.banner"

/** The row's title, shown until the core's row is read (a test double without the table). */
internal const val LOCKED_FALLBACK = "Locked by the installer"

@Composable
internal fun ModeDialogHost(model: UiModeModel) {
    val d by model.dialog.collectAsStateWithLifecycle()
    val state = d ?: return
    var pin by remember(state.unlockOnly) { mutableStateOf("") }
    val askPin = model.needsPin(state)
    AlertDialog(
        modifier = Modifier.testTag(MODE_DIALOG_TAG),
        onDismissRequest = { if (!state.busy) model.closeDialog() },
        title = { Text(if (state.unlockOnly) "Installer Unlock" else "Mode") },
        text = {
            Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
                if (!state.unlockOnly) {
                    state.modes.forEach { m ->
                        Row(
                            Modifier
                                .fillMaxWidth()
                                .selectable(selected = state.selected == m.id, role = Role.RadioButton) { model.select(m.id) },
                            verticalAlignment = Alignment.CenterVertically
                        ) {
                            RadioButton(selected = state.selected == m.id, onClick = null)
                            Text(m.label, Modifier.padding(start = 8.dp))
                        }
                    }
                    Row(
                        Modifier.fillMaxWidth().selectable(selected = !state.everyDevice, role = Role.RadioButton) {
                            model.setEveryDevice(false)
                        },
                        verticalAlignment = Alignment.CenterVertically
                    ) {
                        RadioButton(selected = !state.everyDevice, onClick = null)
                        Text("Remember for this device", Modifier.padding(start = 8.dp), style = MaterialTheme.typography.bodySmall)
                    }
                    Row(
                        Modifier.fillMaxWidth().selectable(selected = state.everyDevice, role = Role.RadioButton) {
                            model.setEveryDevice(true)
                        },
                        verticalAlignment = Alignment.CenterVertically
                    ) {
                        RadioButton(selected = state.everyDevice, onClick = null)
                        Text(
                            "Use for every device on this console",
                            Modifier.padding(start = 8.dp),
                            style = MaterialTheme.typography.bodySmall
                        )
                    }
                }
                if (askPin) {
                    OutlinedTextField(
                        value = pin,
                        onValueChange = { pin = it },
                        label = { Text("Installer PIN") },
                        singleLine = true,
                        visualTransformation = PasswordVisualTransformation(),
                        keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Password),
                        modifier = Modifier.fillMaxWidth().testTag(MODE_PIN_TAG)
                    )
                    // OD-P1d: the lock is on this console (or the hub), the DSPi itself is open; one line
                    if (state.scope.isNotEmpty()) {
                        Text(state.scope, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
                    }
                }
                if (state.error.isNotEmpty()) {
                    Text(
                        state.error,
                        color = MaterialTheme.colorScheme.error,
                        style = MaterialTheme.typography.bodySmall
                    )
                }
            }
        },
        confirmButton = {
            TextButton(
                onClick = { model.confirm(pin) },
                enabled = !state.busy && (!askPin || pin.isNotEmpty()),
                modifier = Modifier.testTag(MODE_OK_TAG)
            ) { Text(if (state.unlockOnly) "Unlock" else "OK") }
        },
        dismissButton = { TextButton(onClick = model::closeDialog, enabled = !state.busy) { Text("Cancel") } }
    )
}

/** The locked-user state's banner under the strip: what is locked, the installer's note and contact, Unlock. */
@Composable
internal fun LockedBanner(lock: LockState?, line: String, onUnlock: () -> Unit) {
    val colors = MaterialTheme.colorScheme
    Row(
        Modifier.fillMaxWidth().background(colors.secondaryContainer).padding(start = 16.dp, end = 4.dp, top = 2.dp, bottom = 2.dp)
            .testTag(LOCK_BANNER_TAG),
        verticalAlignment = Alignment.CenterVertically
    ) {
        Column(Modifier.weight(1f)) {
            Text(line, style = MaterialTheme.typography.bodySmall, color = colors.onSecondaryContainer)
            val more = listOfNotNull(lock?.note?.ifBlank { null }, lock?.contact?.ifBlank { null }?.let { "Contact: $it" })
            if (more.isNotEmpty()) {
                Text(more.joinToString(" · "), style = MaterialTheme.typography.bodySmall, color = colors.onSecondaryContainer)
            }
        }
        TextButton(onClick = onUnlock) { Text("Unlock") }
    }
}
