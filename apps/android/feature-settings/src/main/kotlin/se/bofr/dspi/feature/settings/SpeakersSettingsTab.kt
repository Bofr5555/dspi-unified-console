// -- WP UAN -- (UXP B8) Settings › Speakers and the Getting Started step: the rows of SpeakersModel.
package se.bofr.dspi.feature.settings

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Button
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import se.bofr.dspi.bridge.DspiBridge
import se.bofr.dspi.core.MeasureRole
import se.bofr.dspi.ui.collectAsStateOnMain
import se.bofr.dspi.ui.writeEnabled

const val SPEAKERS_PAGE_TAG = "settings-speakers"
const val SPEAKERS_CONFIRM_TAG = "speakers-confirm"
const val SPEAKERS_SAME_TAG = "speakers-same-right"

fun speakerRoleTag(output: Int) = "speaker-role:$output"

fun speakerSideTag(output: Int) = "speaker-side:$output"

/** Settings › Speakers: the page of its own, scrolling. */
@Composable
fun SpeakersSettingsTab(bridge: DspiBridge) {
    val scope = rememberCoroutineScope()
    val model = remember(bridge) { SpeakersModel(bridge, scope) }
    Column(Modifier.fillMaxWidth().verticalScroll(rememberScrollState()).padding(16.dp).testTag(SPEAKERS_PAGE_TAG)) {
        Text(SpeakerWords.TITLE, style = MaterialTheme.typography.titleLarge, modifier = Modifier.semantics { heading() })
        SpeakersEditor(bridge, model)
    }
}

/** The pairs of outputs "Same for the right side" fills: the dashboard's cards of two (left, right). */
fun speakerPairs(bridge: DspiBridge): List<Pair<Int, Int>> = bridge.outputCards().mapNotNull { c -> c.right?.let { c.left to it } }

/**
 * The rows and their actions, for the Settings page and the Getting Started step ([onConfirmed]
 * after a Confirm). Reads the device's layout when it opens and when the device changes.
 */
@Composable
fun SpeakersEditor(bridge: DspiBridge, model: SpeakersModel, onConfirmed: () -> Unit = {}) {
    val status by bridge.status.collectAsStateOnMain()
    val generation by bridge.deviceGeneration.collectAsStateOnMain()
    LaunchedEffect(model, status.connected, generation) { model.load() }
    val s by model.state.collectAsStateOnMain()
    val canWrite = writeEnabled(s.connected) && !s.busy
    Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
        Text(SpeakerWords.INTRO, style = MaterialTheme.typography.bodyMedium)
        Text(SpeakerWords.SAFETY, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
        if (!s.connected) {
            Text(SpeakerWords.NO_DEVICE, style = MaterialTheme.typography.bodyLarge)
            return@Column
        }
        Text(
            when {
                s.confirmed && !s.changed -> SpeakerWords.CONFIRMED
                s.detected && !s.changed -> SpeakerWords.DETECTED
                else -> SpeakerWords.UNCONFIRMED
            },
            style = MaterialTheme.typography.bodyMedium,
            fontWeight = if (!s.confirmed || s.changed) FontWeight.SemiBold else null
        )
        HorizontalDivider()
        val limits by bridge.installer.profile.collectAsStateOnMain()
        for (row in s.rows) {
            SpeakerRowView(row, canWrite, speakerFloorWords(row.role, limits?.output(row.output)), model)
            HorizontalDivider()
        }
        val pairs = remember(s.rows, generation) { speakerPairs(bridge) }
        if (pairs.isNotEmpty()) {
            TextButton(
                onClick = { model.sameForRight(pairs) },
                enabled = canWrite && sameForRight(s.rows, pairs) != s.rows,
                modifier = Modifier.heightIn(min = 48.dp).testTag(SPEAKERS_SAME_TAG)
            ) { Text("Same for the right side") }
        }
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.CenterVertically) {
            TextButton(onClick = model::useSuggestion, enabled = canWrite, modifier = Modifier.heightIn(min = 48.dp)) {
                Text("Use the Suggestion")
            }
            Button(
                onClick = { model.confirm(onConfirmed) },
                enabled = canWrite && (s.changed || !s.confirmed),
                modifier = Modifier.heightIn(min = 48.dp).testTag(SPEAKERS_CONFIRM_TAG)
            ) { Text("Confirm") }
        }
        if (s.error.isNotEmpty()) Text(s.error, color = MaterialTheme.colorScheme.error, style = MaterialTheme.typography.bodyMedium)
        Text(SpeakerWords.MEASURE, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
    }
}

/** "Out 1 · L" over Role ▾ and Side ▾ (48dp each), the high-pass a tweeter or mid gets under them. */
@Composable
private fun SpeakerRowView(row: SpeakerRow, enabled: Boolean, floor: String?, model: SpeakersModel) {
    Column(Modifier.fillMaxWidth().padding(vertical = 4.dp)) {
        Text(row.label, style = MaterialTheme.typography.bodyLarge)
        Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            Chooser(
                "Role",
                SpeakerWords.role(row.role),
                SpeakerWords.roles.map { SpeakerWords.role(it) to it },
                enabled,
                Modifier.testTag(speakerRoleTag(row.output)),
                "Out ${row.output + 1} role"
            ) { r: MeasureRole? -> model.setRole(row.output, r) }
            if (row.role != null) {
                val sides = (SpeakerWords.sides + row.side).distinct()
                Chooser(
                    "Side",
                    row.side,
                    sides.map { it to it },
                    enabled,
                    Modifier.testTag(speakerSideTag(row.output)),
                    "Out ${row.output + 1} side"
                ) { side: String -> model.setSide(row.output, side) }
            }
        }
        floor?.let { Text(it, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant) }
    }
}

@Composable
private fun <T> Chooser(
    title: String,
    shown: String,
    options: List<Pair<String, T>>,
    enabled: Boolean,
    modifier: Modifier,
    name: String,
    onPick: (T) -> Unit
) {
    var open by remember { mutableStateOf(false) }
    Box {
        TextButton(
            onClick = { open = true },
            enabled = enabled,
            modifier = modifier.heightIn(min = 48.dp).semantics { contentDescription = "$name, $shown" }
        ) { Text("$title: $shown ▾") }
        DropdownMenu(expanded = open, onDismissRequest = { open = false }) {
            for ((words, value) in options) {
                DropdownMenuItem(text = { Text(if (words == shown) "✓ $words" else words) }, onClick = {
                    open = false
                    onPick(value)
                })
            }
        }
    }
}
