// The owner's rule for a lost DSPi Link connection (WP AN5): say so at once, on
// every screen, with the way out. The mirror stays on screen under it and the core
// refuses writes; nothing here waits or retries. Qt's `components/LinkBanner.qml`.
package se.bofr.dspi.ui.link

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp

/** Test tag of the banner. */
const val LINK_BANNER_TAG = "link-banner"

/** One button of the banner: [label] in the core's wording where it has one. */
data class LinkBannerAction(val label: String, val onClick: () -> Unit)

/**
 * The lost-connection banner. [text] is the core's wording (CG10 `reconnecting`),
 * [detail] what the client adds (the hub, the core's `close_reason`), [actions]
 * at least the way out (`link_close`).
 */
@Composable
fun LinkBanner(text: String, detail: String, actions: List<LinkBannerAction>, modifier: Modifier = Modifier) {
    val colors = MaterialTheme.colorScheme
    Row(
        modifier
            .testTag(LINK_BANNER_TAG)
            .fillMaxWidth()
            .background(colors.errorContainer)
            .padding(horizontal = 16.dp, vertical = 4.dp)
            // TalkBack reads it out when it appears, as a sighted user sees it at once
            .semantics { liveRegion = LiveRegionMode.Polite },
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.SpaceBetween
    ) {
        Column(Modifier.weight(1f)) {
            Text(text, style = MaterialTheme.typography.bodyMedium, color = colors.onErrorContainer)
            if (detail.isNotEmpty()) {
                Text(detail, style = MaterialTheme.typography.bodySmall, color = colors.onErrorContainer)
            }
        }
        Row {
            actions.forEach { a -> TextButton(onClick = a.onClick) { Text(a.label, color = colors.onErrorContainer) } }
        }
    }
}
