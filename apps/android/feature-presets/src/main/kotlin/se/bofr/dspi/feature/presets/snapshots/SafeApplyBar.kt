// WP P1AN1: the safe apply's countdown bar (PRO1 W6, P1SA), with Keep and
// Revert, over every screen while a guarded apply waits. On USB the core runs
// the timer on the caller's poll (the web rule: no thread of its own), so the
// bar polls `apply_poll` while it is on screen (decision E6, WhileVisible);
// over DSPi Link the hub runs it, and a console that goes away is reverted
// there. After the end, the core's sentence stays for a few seconds.
package se.bofr.dspi.feature.presets.snapshots

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.Button
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableLongStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import se.bofr.dspi.bridge.DspiBridge
import se.bofr.dspi.core.GuardPhase
import se.bofr.dspi.core.GuardedApply
import se.bofr.dspi.ui.rememberWhileVisible

const val SAFE_APPLY_BAR_TAG = "safe-apply-bar"
const val SAFE_APPLY_KEEP_TAG = "safe-apply-keep"
const val SAFE_APPLY_REVERT_TAG = "safe-apply-revert"

/** How often the bar drives the guard while it waits (the countdown's resolution). */
internal const val GUARD_POLL_MS = 250L

/** While the connection is lost: ask how it ended (the hub's answer after a reconnect). */
internal const val GUARD_LOST_POLL_MS = 1_000L

/** How long the end's sentence stays. */
internal const val GUARD_END_SHOWN_MS = 8_000L

/** The bar's words for [g]: the first line and the second ("" none). */
internal fun guardLines(g: GuardedApply): Pair<String, String> = when (g.phase) {
    GuardPhase.WAITING -> {
        val what = g.label.ifEmpty { "the change" }
        val s = ((g.remainingMs + 999) / 1000).coerceAtLeast(0)
        "Applied \"$what\". Keep it?" to
            ("Reverts in $s s without an answer." + if (g.hub) " The hub puts it back if this console goes away." else "")
    }
    GuardPhase.LOST -> g.message.ifEmpty { "The connection was lost." } to ""
    else -> g.message to g.notes.joinToString(" ")
}

/**
 * The bar, or nothing. Place it over the screens (it does not push them):
 * it shows while a guarded apply waits, and the end's sentence after one this
 * bar watched.
 */
@Composable
fun SafeApplyBar(bridge: DspiBridge, modifier: Modifier = Modifier) {
    val snaps = bridge.snapshots
    val g by snaps.guard.collectAsState()
    val visible = rememberWhileVisible()
    val scope = rememberCoroutineScope()
    val polling = g.phase == GuardPhase.WAITING || g.phase == GuardPhase.LOST
    LaunchedEffect(bridge, polling, visible) {
        while (polling && visible) {
            val now = snaps.poll()
            delay(if (now.phase == GuardPhase.WAITING) GUARD_POLL_MS else GUARD_LOST_POLL_MS)
        }
    }
    // Only an apply this bar saw waiting gets its end shown (not an old one after a rotation or a new bridge)
    var watched by remember(bridge) { mutableLongStateOf(-1L) }
    var endShown by remember(bridge) { mutableStateOf<Pair<Long, GuardPhase>?>(null) }
    if (g.phase == GuardPhase.WAITING) watched = g.id
    val ended = g.phase != GuardPhase.WAITING && g.phase != GuardPhase.IDLE && g.id == watched
    LaunchedEffect(g.id, g.phase, ended) {
        if (ended) {
            delay(GUARD_END_SHOWN_MS)
            endShown = g.id to g.phase
        }
    }
    val show =
        g.phase == GuardPhase.WAITING || (ended && endShown != (g.id to g.phase) && (g.message.isNotEmpty() || g.phase == GuardPhase.LOST))
    if (!show) return
    val (first, second) = guardLines(g)
    Surface(
        modifier.fillMaxWidth().padding(8.dp).testTag(SAFE_APPLY_BAR_TAG),
        color = MaterialTheme.colorScheme.inverseSurface,
        contentColor = MaterialTheme.colorScheme.inverseOnSurface,
        shape = MaterialTheme.shapes.medium,
        shadowElevation = 6.dp
    ) {
        Row(
            Modifier.padding(start = 16.dp, end = 8.dp, top = 8.dp, bottom = 8.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(8.dp)
        ) {
            Column(Modifier.weight(1f).semantics { liveRegion = LiveRegionMode.Polite }) {
                Text(first, style = MaterialTheme.typography.bodyMedium)
                if (second.isNotEmpty()) Text(second, style = MaterialTheme.typography.bodySmall)
            }
            val canRevert = g.phase == GuardPhase.WAITING || (g.phase == GuardPhase.LOST && !g.hub)
            if (canRevert) {
                TextButton(
                    onClick = { scope.launch { snaps.revert() } },
                    modifier = Modifier.testTag(SAFE_APPLY_REVERT_TAG)
                ) { Text("Revert", color = MaterialTheme.colorScheme.inversePrimary) }
            }
            if (g.phase == GuardPhase.WAITING) {
                Button(onClick = { scope.launch { snaps.keep() } }, modifier = Modifier.testTag(SAFE_APPLY_KEEP_TAG)) { Text("Keep") }
            } else {
                TextButton(onClick = { endShown = g.id to g.phase }) { Text("OK", color = MaterialTheme.colorScheme.inversePrimary) }
            }
        }
    }
}
