// WP P2AN: Settings › History, the open device's events: this console's own
// log, and "On the hub" where the hub keeps a shared history (PLAN-P2 M1),
// with who did it. Read-only: nothing here writes. Logic in EventsModel.
package se.bofr.dspi.feature.settings

import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.items
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import java.text.DateFormat
import java.util.Date
import se.bofr.dspi.bridge.DspiBridge
import se.bofr.dspi.ui.Caption
import se.bofr.dspi.ui.SegmentedPicker
import se.bofr.dspi.ui.collectAsStateOnMain

const val EVENTS_PAGE_TAG = "settings-events"
const val EVENTS_SOURCE_TAG = "events-source"

fun eventRowTag(id: Long) = "event-row:$id"

/** The source picker's words. */
internal fun sourceLabel(s: EventsSource): String = when (s) {
    EventsSource.CONSOLE -> "This console"
    EventsSource.HUB -> "On the hub"
}

@Composable
fun EventsSettingsTab(bridge: DspiBridge) {
    val scope = rememberCoroutineScope()
    val model = remember(bridge) { EventsModel(bridge, scope) }
    DisposableEffect(model) {
        model.open()
        onDispose { model.close() }
    }
    EventsPage(model, bridge)
}

@Composable
internal fun EventsPage(model: EventsModel, bridge: DspiBridge) {
    val caps by bridge.hubServer.caps.collectAsStateOnMain()
    val source by model.source.collectAsStateOnMain()
    val rows by model.rows.collectAsStateOnMain()
    val line by model.line.collectAsStateOnMain()
    val status by bridge.status.collectAsStateOnMain()
    SettingsForm {
        item {
            Column(Modifier.fillMaxWidth().testTag(EVENTS_PAGE_TAG).padding(horizontal = 16.dp, vertical = 8.dp)) {
                Row(verticalAlignment = Alignment.CenterVertically) {
                    if (caps.events) {
                        SegmentedPicker(
                            EventsSource.entries.toList(),
                            source,
                            ::sourceLabel,
                            model::choose,
                            Modifier.weight(1f).testTag(EVENTS_SOURCE_TAG)
                        )
                    } else {
                        Text(sourceLabel(EventsSource.CONSOLE), Modifier.weight(1f), style = MaterialTheme.typography.titleSmall)
                    }
                    TextButton(onClick = model::refresh) { Text("Refresh") }
                }
                when {
                    !status.connected -> Caption("No device connected.")
                    line.isNotEmpty() -> Caption(line, color = MaterialTheme.colorScheme.error)
                    rows.isEmpty() -> Caption(
                        if (source ==
                            EventsSource.HUB
                        ) {
                            "The hub has no history of this device yet."
                        } else {
                            "Nothing has happened on this device yet."
                        }
                    )
                }
            }
        }
        items(rows, key = { "${source.name}:${it.id}" }) { r ->
            EventLine(r)
            HorizontalDivider(Modifier.padding(horizontal = 16.dp))
        }
    }
}

@Composable
private fun EventLine(r: EventRow) {
    Column(Modifier.fillMaxWidth().testTag(eventRowTag(r.id)).padding(horizontal = 16.dp, vertical = 6.dp)) {
        Text(r.text, style = MaterialTheme.typography.bodyMedium, maxLines = 3, overflow = TextOverflow.Ellipsis)
        Text(
            DateFormat.getDateTimeInstance(DateFormat.MEDIUM, DateFormat.MEDIUM).format(Date(r.timeMs)) +
                if (r.by.isNotEmpty()) " · ${r.by}" else "",
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            maxLines = 1,
            overflow = TextOverflow.Ellipsis
        )
    }
}
