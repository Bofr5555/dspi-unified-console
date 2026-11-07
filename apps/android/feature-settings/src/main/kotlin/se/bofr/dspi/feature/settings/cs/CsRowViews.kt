// The rows of a Control Surfaces record page, drawn from the core's description
// (CsRules): a pick opens a short menu or the picker page (P1–P3), a number opens
// number entry (D1), a toggle switches, a note is words. AN8's screen map, WP AN9.
package se.bofr.dspi.feature.settings.cs

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.input.KeyboardType
import se.bofr.dspi.feature.settings.FormCaption
import se.bofr.dspi.feature.settings.FormSwitchRow
import se.bofr.dspi.feature.settings.SettingRow
import se.bofr.dspi.ui.MacColors
import se.bofr.dspi.ui.writeEnabled

internal fun rowTag(id: String) = "cs.row.$id"

/** One described row. [onOpenPicker] pushes the picker page for a long pick. */
@Composable
internal fun CsRowView(row: CsRow, enabled: Boolean, onOpenPicker: (CsRow.Pick) -> Unit, onChange: (String, Any) -> Unit) {
    val on = writeEnabled(enabled)
    when (row) {
        is CsRow.Pick -> when (CsRules.pickStyle(row)) {
            CsPickStyle.PAGE -> SettingRow(
                row.title,
                row.detail,
                Modifier.testTag(rowTag(row.id)),
                enabled = on,
                onClick = { onOpenPicker(row) }
            ) { Text("${row.text} ›") }
            CsPickStyle.MENU -> CsMenuRow(row, on) { onChange(row.id, it) }
        }
        is CsRow.Number -> CsNumberRow(row, on) { onChange(row.id, it) }
        is CsRow.Toggle -> Box(Modifier.testTag(rowTag(row.id))) {
            FormSwitchRow(row.title, row.detail, row.on, on && (row.enabled || row.on)) { onChange(row.id, it) }
        }
        is CsRow.Note -> Column(Modifier.testTag(rowTag(row.id))) {
            if (row.title.isNotEmpty()) FormCaption(row.title)
            FormCaption(row.detail, if (row.warning) MacColors.orange else androidx.compose.ui.graphics.Color.Unspecified)
        }
    }
}

/** A pick of at most [CsRules.MENU_MAX] plain choices: the value and its menu. */
@Composable
private fun CsMenuRow(row: CsRow.Pick, enabled: Boolean, onPick: (Int) -> Unit) {
    var open by remember { mutableStateOf(false) }
    SettingRow(row.title, row.detail, Modifier.testTag(rowTag(row.id)), enabled = enabled, onClick = { open = true }) {
        Box {
            TextButton(onClick = { open = true }, enabled = enabled) { Text("${row.text} ▾") }
            DropdownMenu(expanded = open, onDismissRequest = { open = false }) {
                row.options.forEach { o ->
                    DropdownMenuItem(text = { Text(o.text) }, enabled = !o.disabled, onClick = {
                        open = false
                        if (o.value != row.value) onPick(o.value)
                    })
                }
            }
        }
    }
}

/** A number: its value, and number entry (D1) with the row's range and unit. */
@Composable
private fun CsNumberRow(row: CsRow.Number, enabled: Boolean, onValue: (Double) -> Unit) {
    var entering by remember { mutableStateOf(false) }
    SettingRow(row.title, row.detail, Modifier.testTag(rowTag(row.id)), enabled = enabled, onClick = { entering = true }) {
        TextButton(onClick = { entering = true }, enabled = enabled) { Text(CsRules.formatNumber(row)) }
    }
    if (entering) {
        CsNumberEntry(row, onDismiss = { entering = false }) {
            entering = false
            onValue(it)
        }
    }
}

/** D1: typed entry, refused outside the range with the range in words. */
@Composable
internal fun CsNumberEntry(row: CsRow.Number, onDismiss: () -> Unit, onValue: (Double) -> Unit) {
    var text by remember(row.id) { mutableStateOf(CsRules.formatNumber(row).removeSuffix(row.unit).trim()) }
    val parsed = CsRules.parseNumber(row, text)
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text(row.title) },
        text = {
            OutlinedTextField(
                value = text,
                onValueChange = { text = it },
                label = { Text(row.unit.ifEmpty { "Value" }) },
                supportingText = { Text(parsed.error ?: CsRules.rangeText(row)) },
                isError = !parsed.ok,
                singleLine = true,
                keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Decimal),
                modifier = Modifier.testTag("cs.number")
            )
        },
        confirmButton = { TextButton(onClick = { parsed.value?.let(onValue) }, enabled = parsed.ok) { Text("Set") } },
        dismissButton = { TextButton(onClick = onDismiss) { Text("Cancel") } }
    )
}
