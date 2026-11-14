// -- WP P1AN2 -- the UI mode a screen is shown in (docs/UI-MODES.md): the
// app resolves it from the core's manifest (`mode_resolve`) and provides it
// to every screen; a screen tested on its own sees Advanced, today's screens.
package se.bofr.dspi.ui

import androidx.compose.runtime.compositionLocalOf

/** The mode ids of the core's manifest, and the locked-user state's. */
object UiModes {
    const val SIMPLE = "simple"
    const val ADVANCED = "advanced"
    const val INSTALLER = "installer"
    const val LOCKED_USER = "locked_user"
}

/** The mode the screens are shown in: one of [UiModes]. */
val LocalUiMode = compositionLocalOf { UiModes.ADVANCED }
