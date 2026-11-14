// -- WP AN41 --
package se.bofr.dspi.app

import androidx.compose.material3.SnackbarDuration
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import kotlinx.coroutines.flow.collectLatest
import kotlinx.coroutines.flow.mapNotNull
import se.bofr.dspi.bridge.BridgeEvent
import se.bofr.dspi.bridge.DspiBridge
import se.bofr.dspi.core.DspiException

/** Test tag of the notice that shows a refused write. */
internal const val REFUSAL_NOTICE_TAG = "refusal-notice"

/**
 * The core's words for a write it refused as an invalid argument (a delay past
 * the platform's line, an output Core 1 cannot take), or null for any other
 * event. Nothing was sent; the bridge has already republished the device's
 * state, so the control shows the old value again.
 */
fun refusalText(e: BridgeEvent): String? = ((e as? BridgeEvent.CommandFailed)?.error as? DspiException.InvalidArgument)?.msg

// -- WP P1AN2 --
/**
 * P1NT: the core's notice row for a write it refused for a named reason (a
 * limit, the installer's lock, a missing confirmation, the hub's image or
 * limits refusal, a role), "Title: reason"; null for anything else. "Another
 * console is applying changes" is the strip's own notice (macOS), not this.
 */
suspend fun coreRefusalText(bridge: DspiBridge, e: BridgeEvent): String? {
    val err = (e as? BridgeEvent.CommandFailed)?.error as? DspiException ?: return null
    if (err is DspiException.Locked) return null
    return bridge.installer.noticeText(err)
}

/**
 * Shows each [refusalText] (and [coreRefusalText]) of [bridge]'s events in a snackbar over the screen
 * (it takes no room in the layout, phones included); a newer refusal replaces
 * the one showing.
 */
@Composable
internal fun RefusalNotices(bridge: DspiBridge, host: SnackbarHostState) {
    LaunchedEffect(bridge) {
        bridge.events.mapNotNull { e -> refusalText(e) ?: coreRefusalText(bridge, e) }
            .collectLatest { host.showSnackbar(it, duration = SnackbarDuration.Short) }
    }
}

@Composable
internal fun RefusalNoticeHost(host: SnackbarHostState) = SnackbarHost(host, Modifier.testTag(REFUSAL_NOTICE_TAG))
