// Port of DSPi Console/Onboarding/WhatsNewView.swift (ext-channel-116) on the
// core's release notes (CG10). A page instead of a window; "Done" goes back.
package se.bofr.dspi.feature.onboarding

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material3.Button
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import se.bofr.dspi.ui.DspiRoutes

/** Test tag of one release's block, plus its version. */
const val WHATS_NEW_RELEASE_TAG = "whats-new-release-"

/** Entry point for [DspiRoutes.WHATS_NEW]: every release's notes, [releases] newest first. */
@Composable
fun WhatsNewScreen(releases: List<ReleaseNotes>, onBack: () -> Unit, modifier: Modifier = Modifier) {
    Surface(modifier.fillMaxSize(), color = MaterialTheme.colorScheme.background) {
        Column(Modifier.fillMaxSize()) {
            PageHeader("What's New", onBack)
            LazyColumn(
                Modifier.weight(1f).fillMaxWidth(),
                contentPadding = androidx.compose.foundation.layout.PaddingValues(20.dp),
                verticalArrangement = Arrangement.spacedBy(24.dp)
            ) {
                items(releases, key = { it.version }) { release ->
                    Column(Modifier.testTag(WHATS_NEW_RELEASE_TAG + release.version), verticalArrangement = Arrangement.spacedBy(10.dp)) {
                        Row(verticalAlignment = Alignment.Top) {
                            Text(release.headline, style = MaterialTheme.typography.titleMedium, modifier = Modifier.weight(1f))
                            Spacer(Modifier.width(8.dp))
                            Text(
                                release.version,
                                style = MaterialTheme.typography.labelSmall,
                                color = MaterialTheme.colorScheme.onSurfaceVariant
                            )
                        }
                        release.items.forEach { item ->
                            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                                Text("•", color = MaterialTheme.colorScheme.onSurfaceVariant)
                                Text(item, style = MaterialTheme.typography.bodyMedium)
                            }
                        }
                    }
                }
                if (releases.isEmpty()) {
                    item { Text("No release notes are available in this build.", color = MaterialTheme.colorScheme.onSurfaceVariant) }
                }
            }
            HorizontalDivider()
            Row(Modifier.fillMaxWidth().padding(16.dp), horizontalArrangement = Arrangement.End) {
                Button(onClick = onBack) { Text("Done") }
            }
        }
    }
}

/** The "‹ Back" bar the app's pages use. */
@Composable
internal fun PageHeader(title: String, onBack: () -> Unit) {
    Row(
        Modifier.fillMaxWidth().background(MaterialTheme.colorScheme.surfaceContainer).padding(horizontal = 4.dp, vertical = 4.dp),
        verticalAlignment = Alignment.CenterVertically
    ) {
        TextButton(onClick = onBack) { Text("‹ Back") }
        Spacer(Modifier.width(4.dp))
        Text(title, style = MaterialTheme.typography.titleMedium, fontWeight = FontWeight.Normal)
    }
}
