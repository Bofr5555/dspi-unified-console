// The picker page for a long pick (AN8's P1–P3): the function list in its
// families with a search field, plain lists, and the GPIO list with taken pins
// dimmed and their owners named. Choices come from the core's description (WP AN9).
package se.bofr.dspi.feature.settings.cs

import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.unit.dp
import se.bofr.dspi.feature.settings.FormCaption
import se.bofr.dspi.feature.settings.SettingRow
import se.bofr.dspi.ui.MacColors

internal fun optionTag(value: Int) = "cs.option.$value"

@Composable
internal fun CsPickerPage(row: CsRow.Pick, onPick: (Int) -> Unit) {
    var query by rememberSaveable(row.id) { mutableStateOf("") }
    val shown = CsRules.filter(row.options, query)
    LazyColumn(Modifier.fillMaxWidth().testTag("cs.picker")) {
        item {
            Text(row.title, style = MaterialTheme.typography.titleMedium, modifier = Modifier.padding(horizontal = 16.dp, vertical = 8.dp))
            if (row.detail.isNotEmpty()) FormCaption(row.detail)
        }
        if (row.options.size > CsRules.SEARCH_MIN) {
            item {
                OutlinedTextField(
                    value = query,
                    onValueChange = { query = it },
                    label = { Text("Search") },
                    singleLine = true,
                    modifier = Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 4.dp).testTag("cs.search")
                )
            }
        }
        if (shown.isEmpty()) item { FormCaption("Nothing matches \"$query\".") }
        CsRules.sections(shown).forEach { (section, options) ->
            if (section != null) {
                item {
                    Text(
                        section,
                        style = MaterialTheme.typography.titleSmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                        modifier = Modifier.padding(start = 20.dp, top = 12.dp, bottom = 4.dp)
                    )
                }
            }
            items(options, key = { "${section.orEmpty()}/${it.value}" }) { o ->
                SettingRow(
                    o.text,
                    o.owner,
                    Modifier.testTag(optionTag(o.value)),
                    enabled = !o.disabled,
                    detailColor = if (o.disabled) MacColors.orange else androidx.compose.ui.graphics.Color.Unspecified,
                    onClick = { onPick(o.value) }
                ) { if (o.value == row.value) Text("✓", color = MacColors.green) }
            }
        }
    }
}
