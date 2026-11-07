package se.bofr.dspi.ui

import androidx.compose.runtime.compositionLocalOf

/**
 * True in demo mode (WP AN28), so a screen does not tell the user to update firmware
 * that is not there. The app provides it around the demo stack's screens.
 */
val LocalDemoMode = compositionLocalOf { false }
