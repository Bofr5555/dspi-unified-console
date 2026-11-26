// The macOS system colours (dark appearance) the ported views name directly
// (`.green`, `.orange`, `.red`, …), one copy for every feature (decision E6;
// replaces TOOLS' ToolColors, ANALYSIS' MacColors, MATRIX's Matrix* and
// SETTINGS' WarningOrange/OkGreen). Same values as DspiColors.Dark.
package se.bofr.dspi.ui

import androidx.compose.runtime.Composable
import androidx.compose.runtime.ReadOnlyComposable
import androidx.compose.ui.graphics.Color

/** NSColor system colours, dark appearance. */
object MacColors {
    val red = Color(0xFFFF453A)
    val orange = Color(0xFFFF9F0A)
    val yellow = Color(0xFFFFD60A)
    val green = Color(0xFF32D74B)
    val blue = Color(0xFF0A84FF)
    val purple = Color(0xFFBF5AF2)
    val pink = Color(0xFFFF375F)
    val gray = Color(0xFF8E8E93)

    // -- WP UAN -- (UXP A12) the design language's text values: orange text on the light theme's
    // #ECECEC was about 1.7:1; these are its `warning` (#875200) and `dangerText` (#b42318) there
    /** [orange] for text and small marks: the light theme's darker warning. */
    val warningText: Color
        @Composable @ReadOnlyComposable
        get() = if (DspiTheme.colors.isDark) orange else WARNING_TEXT_LIGHT

    /** [red] for text and small marks: the light theme's darker danger text. */
    val dangerText: Color
        @Composable @ReadOnlyComposable
        get() = if (DspiTheme.colors.isDark) red else DANGER_TEXT_LIGHT

    /** DESIGN-LANGUAGE `Theme.warning`, light. */
    val WARNING_TEXT_LIGHT = Color(0xFF875200)

    /** DESIGN-LANGUAGE `Theme.dangerText`, light. */
    val DANGER_TEXT_LIGHT = Color(0xFFB42318)
}
