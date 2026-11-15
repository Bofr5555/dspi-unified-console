// WP P1AN1: the safe apply's countdown bar (PRO1 W6, P1SA), with Keep and
// Revert, over every screen while a guarded apply waits. On USB the core runs
// the timer on the caller's poll (the web rule: no thread of its own), so the
// bar polls `apply_poll` while it is on screen (decision E6, WhileVisible);
// over DSPi Link the hub runs it, and a console that goes away is reverted
// there. After the end, the core's sentence stays for a few seconds.
// WP P1DCFIX: the countdown is the core's sentence (P1NT `guard.waiting`, F7);
// a lost guard's bar never hides by itself (F2): Revert stays offered while
// the core says Lost, and is only ever the user's tap.
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
import se.bofr.dspi.bridge.Snapshots
import se.bofr.dspi.core.GuardPhase
import se.bofr.dspi.core.GuardedApply
import se.bofr.dspi.ui.collectAsStateOnMain
import se.bofr.dspi.ui.onMainThread
import se.bofr.dspi.ui.rememberWhileVisible

const val SAFE_APPLY_BAR_TAG = "safe-apply-bar"
const val SAFE_APPLY_KEEP_TAG = "safe-apply-keep"
const val SAFE_APPLY_REVERT_TAG = "safe-apply-revert"

/** How often the bar drives the guard while it waits (the countdown's resolution). */
internal const val GUARD_POLL_MS = 250L

/** While the connection is lost: ask how it ended (the hub's answer after a reconnect). */
internal const val GUARD_LOST_POLL_MS = 1_000L

/** How long the end's sentence stays (a kept or reverted apply; never a lost one). */
internal const val GUARD_END_SHOWN_MS = 8_000L

/**
 * The bar's words for [g]: the first line and the second ("" none). While it
 * waits: what was applied, then the core's countdown sentence [words] (`{s}`
 * the seconds left).
 */
internal fun guardLines(
    g: GuardedApply,
    words: String = Snapshots.COUNTDOWN_WORDS,
    // -- WP P1AN2 -- the hub's backstop sentence, after the countdown, when the hub holds the guard
    backstop: String = Snapshots.HUB_BACKSTOP_WORDS
): Pair<String, String> = when (g.phase) {
    GuardPhase.WAITING ->
        "\"${g.label.ifEmpty { "The change" }}\"" to
            (Snapshots.countdownLine(words, g.remainingMs) + if (g.hub) " $backstop" else "")
    GuardPhase.LOST -> g.message.ifEmpty { "The connection was lost." } to ""
    else -> g.message to g.notes.joinToString(" ")
}

/**
 * The bar shows for [g]: while it waits; while it is lost (until the user's
 * OK, never by a timer: Revert stays offered); and the end of one this bar
 * watched until [endShown] or the user's OK.
 */
internal fun guardBarShown(g: GuardedApply, watched: Long, endShown: Pair<Long, GuardPhase>?): Boolean = when (g.phase) {
    GuardPhase.WAITING -> true
    GuardPhase.IDLE -> false
    GuardPhase.LOST -> endShown != (g.id to g.phase)
    else -> g.id == watched && endShown != (g.id to g.phase) && g.message.isNotEmpty()
}

/**
 * The bar, or nothing. Place it over the screens (it does not push them):
 * it shows while a guarded apply waits, and the end's sentence after one this
 * bar watched.
 */
@Composable
fun SafeApplyBar(bridge: DspiBridge, modifier: Modifier = Modifier) {
    val snaps = bridge.snapshots
    // -- WP ANDFU -- the poll below writes the guard after its bridge call: its collector writes on main
    val g by snaps.guard.collectAsStateOnMain()
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
    // -- WP P1DCFIX -- a lost guard is not an end: no timer hides it (F2)
    val ended = g.phase != GuardPhase.WAITING && g.phase != GuardPhase.IDLE && g.phase != GuardPhase.LOST && g.id == watched
    LaunchedEffect(g.id, g.phase, ended) {
        if (ended) {
            delay(GUARD_END_SHOWN_MS)
            endShown = g.id to g.phase
        }
    }
    var words by remember(bridge) { mutableStateOf(Snapshots.COUNTDOWN_WORDS) }
    var backstop by remember(bridge) { mutableStateOf(Snapshots.HUB_BACKSTOP_WORDS) } // -- WP P1AN2 --
    LaunchedEffect(bridge) {
        val w = snaps.countdownWords()
        val b = snaps.backstopWords()
        // -- WP ANDFU -- written after the bridge's calls (FLAKE2's pattern)
        onMainThread {
            words = w
            backstop = b
        }
    }
    if (!guardBarShown(g, watched, endShown)) return
    val (first, second) = guardLines(g, words, backstop)
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
