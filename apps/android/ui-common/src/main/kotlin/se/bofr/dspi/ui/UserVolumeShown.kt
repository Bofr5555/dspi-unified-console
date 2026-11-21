// WP ANVOL: whether a User Volume slider is on screen, so the volume buttons' indicator
// (app's VolumeKeyIndicator) shows only when no slider moves in its place.
package se.bofr.dspi.ui

import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.staticCompositionLocalOf
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow

/** How many User Volume sliders are composed. Main thread only. */
class UserVolumeShown {
    private val state = MutableStateFlow(0)

    /** The number of User Volume sliders composed; above 0 while one is on screen. */
    val count: StateFlow<Int> = state.asStateFlow()

    fun add() {
        state.value += 1
    }

    fun remove() {
        state.value = (state.value - 1).coerceAtLeast(0)
    }
}

/** The app's one [UserVolumeShown]; without a provider (previews, tests) a shared one. */
val LocalUserVolumeShown = staticCompositionLocalOf { UserVolumeShown() }

/** Counts the calling User Volume slider as shown while it is composed. */
@Composable
fun ReportUserVolumeShown() {
    val shown = LocalUserVolumeShown.current
    DisposableEffect(shown) {
        shown.add()
        onDispose { shown.remove() }
    }
}
