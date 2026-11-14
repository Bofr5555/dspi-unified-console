// WP P1AN1: the import preview (PRO1 W5, W8, W6) as Qt's P1QT1 import dialog:
// the core's diff of the file against the device, a choice per block, and
// "Apply with rollback". Phone first: a full-screen sheet, one scrolling list.
package se.bofr.dspi.feature.presets

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material3.Checkbox
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import se.bofr.dspi.ui.writesAllowed

/** Test tag of the preview's sheet. */
const val IMPORT_PREVIEW_TAG = "import-preview"

/** Test tag of its Import button. */
const val IMPORT_PREVIEW_APPLY_TAG = "import-preview-apply"

/** "Apply with rollback": what it does, in one line. */
const val ROLLBACK_EXPLAINED = "After the import, Keep or Revert. Without an answer the settings from before come back."

/** The open preview of [files], as a full-screen dialog; nothing while none is open. */
@Composable
fun ImportPreviewHost(files: FileMenuActions, preview: ImportPreview?) {
    val p = preview ?: return
    Dialog(onDismissRequest = files::cancelPreview, properties = DialogProperties(usePlatformDefaultWidth = false)) {
        Surface(Modifier.fillMaxSize()) {
            ImportPreviewSheet(
                p,
                onBlock = files::setBlock,
                onGuarded = files::setGuarded,
                onCancel = files::cancelPreview,
                onApply = files::applyPreview
            )
        }
    }
}

/** The preview's content; previewable. */
@Composable
fun ImportPreviewSheet(
    p: ImportPreview,
    onBlock: (ImportBlock, Boolean) -> Unit,
    onGuarded: (Boolean) -> Unit,
    onCancel: () -> Unit,
    onApply: () -> Unit,
    modifier: Modifier = Modifier
) {
    val counts = p.diff.countByBlock
    val rows = p.chosenRows
    val canWrite = writesAllowed()
    Column(modifier.fillMaxSize().testTag(IMPORT_PREVIEW_TAG)) {
        Row(Modifier.fillMaxWidth().padding(horizontal = 4.dp), verticalAlignment = Alignment.CenterVertically) {
            TextButton(onClick = onCancel) { Text("Cancel") }
            Text(
                "Import",
                style = MaterialTheme.typography.titleLarge,
                modifier = Modifier.weight(1f)
            )
            TextButton(
                onClick = onApply,
                enabled = p.selected.isNotEmpty() && canWrite,
                modifier = Modifier.testTag(IMPORT_PREVIEW_APPLY_TAG)
            ) { Text(if (p.guarded) "Apply with rollback" else "Import") }
        }
        HorizontalDivider()
        LazyColumn(Modifier.fillMaxSize(), verticalArrangement = Arrangement.spacedBy(2.dp)) {
            item {
                Column(Modifier.padding(horizontal = 16.dp, vertical = 8.dp), verticalArrangement = Arrangement.spacedBy(4.dp)) {
                    if (p.name.isNotEmpty()) Text(p.name, style = MaterialTheme.typography.titleMedium)
                    if (p.intro.isNotEmpty()) Text(p.intro, style = MaterialTheme.typography.bodyMedium)
                    Text(
                        when {
                            !p.diff.ok -> p.diff.error
                            p.diff.rows.isEmpty() -> "Nothing in this file differs from the device."
                            else -> "${changes(p.diff.rows.size)} from the device; ${changes(rows.size)} chosen."
                        },
                        style = MaterialTheme.typography.bodyMedium,
                        color = MaterialTheme.colorScheme.onSurfaceVariant
                    )
                }
            }
            item { Heading("Bring in") }
            items(ImportBlock.entries.toList(), key = { it.key }) { b ->
                val n = counts[b.key] ?: 0
                CheckRow(
                    title = b.title,
                    detail = if (p.diff.ok) (if (n == 0) "no changes" else changes(n)) else "",
                    checked = b in p.selected,
                    onChange = { onBlock(b, it) }
                )
            }
            item {
                Heading("Way back")
                CheckRow(
                    title = "Apply with rollback",
                    detail = if (p.guardDefault) "on by default over DSPi Link" else "",
                    checked = p.guarded,
                    onChange = onGuarded
                )
                Text(
                    ROLLBACK_EXPLAINED,
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    modifier = Modifier.padding(horizontal = 16.dp)
                )
            }
            if (rows.isNotEmpty()) {
                item { Heading("What changes") }
                items(rows) { r -> DiffRowLine(r) }
            }
            if (p.diff.notes.isNotEmpty()) {
                item { Heading("Notes") }
                items(p.diff.notes) { n ->
                    Text(n, style = MaterialTheme.typography.bodySmall, modifier = Modifier.padding(horizontal = 16.dp, vertical = 2.dp))
                }
            }
        }
    }
}

internal fun changes(n: Int) = if (n == 1) "1 change" else "$n changes"

@Composable
private fun Heading(text: String) {
    Text(
        text,
        style = MaterialTheme.typography.labelLarge,
        fontWeight = FontWeight.SemiBold,
        color = MaterialTheme.colorScheme.primary,
        modifier = Modifier.padding(start = 16.dp, end = 16.dp, top = 12.dp, bottom = 2.dp)
    )
}

@Composable
private fun CheckRow(title: String, detail: String, checked: Boolean, onChange: (Boolean) -> Unit) {
    Row(
        Modifier.fillMaxWidth().clickable(role = Role.Checkbox) { onChange(!checked) }.padding(horizontal = 8.dp),
        verticalAlignment = Alignment.CenterVertically
    ) {
        Checkbox(checked = checked, onCheckedChange = null, modifier = Modifier.padding(8.dp))
        Text(title, modifier = Modifier.weight(1f))
        if (detail.isNotEmpty()) {
            Text(
                detail,
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                modifier = Modifier.padding(end = 8.dp)
            )
        }
    }
}

/** One changed value: "Section · item" over "before → after". */
@Composable
fun DiffRowLine(r: DiffRow) {
    Column(Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 3.dp)) {
        Text(listOf(r.section, r.item).filter { it.isNotEmpty() }.joinToString(" · "), style = MaterialTheme.typography.bodyMedium)
        Text(
            "${r.before.ifEmpty { "–" }} → ${r.after.ifEmpty { "–" }}",
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant
        )
    }
}
