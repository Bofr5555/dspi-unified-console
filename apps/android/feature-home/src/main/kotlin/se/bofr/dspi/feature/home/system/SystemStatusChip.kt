// -- WP LKUI1 -- Home's status chip for the hub's audio link: one line (the dot's
// colour is its meaning), shown only on a hub with an installation; a tap opens
// the System screen. The front page's (Simple's) view of the installation.
package se.bofr.dspi.feature.home.system

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.role
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import se.bofr.dspi.bridge.DspiBridge
import se.bofr.dspi.ui.DspiRoutes
import se.bofr.dspi.ui.DspiTheme
import se.bofr.dspi.ui.DspiType

@Composable
fun SystemStatusChip(bridge: DspiBridge, onNavigate: (String) -> Unit, modifier: Modifier = Modifier) {
    val scope = rememberCoroutineScope()
    val model = remember(bridge) { SystemScreenModel(bridge, scope) }
    LaunchedEffect(model) { model.poll() }
    val s by model.state.collectAsStateWithLifecycle()
    SystemStatusChipView(s, onOpen = { onNavigate(DspiRoutes.SYSTEM) }, modifier)
}

@Composable
internal fun SystemStatusChipView(s: SystemUiState, onOpen: () -> Unit, modifier: Modifier = Modifier) {
    val view = s.view ?: return
    if (!view.installed) return
    val c = DspiTheme.colors
    val line = view.summary + (if (s.recalledScene.isNotEmpty()) " · Scene: ${s.recalledScene}" else "") +
        (if (view.systemMute == true) " · System mute on" else "") +
        (if (view.alarms.isNotEmpty()) " · ${view.alarms.size} alarm" + (if (view.alarms.size == 1) "" else "s") else "")
    Row(
        modifier
            .fillMaxWidth()
            .heightIn(min = 40.dp)
            .background(c.card, RoundedCornerShape(20.dp))
            .clickable(onClick = onOpen)
            .semantics(mergeDescendants = true) {
                role = Role.Button
                contentDescription = "$line. Open System"
            }
            .padding(horizontal = 12.dp, vertical = 8.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(8.dp)
    ) {
        Box(Modifier.size(8.dp).background(healthColor(c, view.health), CircleShape))
        Text(
            line,
            style = DspiType.caption,
            color = c.label,
            maxLines = 1,
            overflow = TextOverflow.Ellipsis,
            modifier = Modifier.weight(1f)
        )
        Text("System ›", style = DspiType.caption, color = c.accent)
    }
}
