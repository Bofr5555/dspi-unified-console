package se.bofr.dspi.ui

import androidx.compose.material3.ColorScheme
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.compositeOver
import androidx.compose.ui.graphics.luminance
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * WP ANBAR (owner 2026-11-21): no Material colour role is left at its baseline value (its
 * purples, lavenders and pinks), and the accent and every content role are WCAG AA (4.5:1) on
 * what they are drawn on, light and dark. The ratios are printed (the test's system-out).
 */
class ThemeWpAnbarTest {
    private fun contrast(fg: Color, bg: Color): Double {
        val opaque = bg.compositeOver(Color.Black) // the surfaces here are opaque; a translucent one would be over black
        val (hi, lo) = listOf(fg.compositeOver(opaque).luminance().toDouble(), opaque.luminance().toDouble()).sortedDescending()
        return (hi + 0.05) / (lo + 0.05)
    }

    /** Every colour role of [s] by name: its getters returning a packed Color (a long). */
    private fun roles(s: ColorScheme): Map<String, Long> = ColorScheme::class.java.methods
        .filter { it.name.startsWith("get") && it.parameterCount == 0 && it.returnType == java.lang.Long.TYPE }
        .associate { it.name.removePrefix("get").substringBefore('-') to it.invoke(s) as Long }

    private val ratios = StringBuilder()

    private fun assertAa(name: String, fg: Color, bg: Color) {
        val r = contrast(fg, bg)
        ratios.append("$name ${"%.2f".format(r)}\n")
        assertTrue("$name: $r", r >= 4.5)
    }

    @Test
    fun wpAnbarNoRoleIsMaterialsBaseline() {
        for ((theme, ours, baseline) in listOf(
            Triple("light", dspiColorScheme(false), lightColorScheme()),
            Triple("dark", dspiColorScheme(true), darkColorScheme())
        )) {
            val mine = roles(ours)
            val base = roles(baseline)
            assertTrue("$theme: the roles were found (${mine.keys})", mine.size >= 30 && mine.keys == base.keys)
            // Pure black and white are nobody's palette: Material's onPrimary, scrim, … may stay equal
            val plain = setOf(Color.White, Color.Black)
            val left = mine.filter { (k, v) -> v == base[k] && Color(v.toULong()) !in plain }.keys
            assertTrue("$theme: left at Material's baseline: $left", left.isEmpty())
        }
    }

    @Test
    fun wpAnbarTheAccentAndContentRolesAreAa() {
        for (dark in listOf(false, true)) {
            val t = if (dark) "dark" else "light"
            val s = dspiColorScheme(dark)
            val c = if (dark) DspiColors.Dark else DspiColors.Light
            // The accent as text (buttons, links, selected rows) on what it is drawn on
            val surfaces = mapOf(
                "background" to s.background,
                "surface" to s.surface,
                "surfaceVariant" to s.surfaceVariant,
                "sidebar" to c.sidebar,
                "surfaceContainerLowest" to s.surfaceContainerLowest,
                "surfaceContainerLow" to s.surfaceContainerLow,
                "surfaceContainer" to s.surfaceContainer,
                "surfaceContainerHigh" to s.surfaceContainerHigh, // dialogs
                "secondaryContainer" to s.secondaryContainer // the active preset row
            )
            assertTrue("$t: accent is primary", c.accent == s.primary)
            for ((n, bg) in surfaces) assertAa("$t accent on $n", s.primary, bg)
            // A selected pill or tile: accent text on accentTint over the page
            for ((n, bg) in surfaces.filterKeys { it in setOf("background", "surface", "surfaceVariant", "sidebar") }) {
                assertAa("$t accent on accentTint over $n", s.primary, c.accentTint.compositeOver(bg))
            }
            // Content roles on their containers
            assertAa("$t onPrimary on primary", s.onPrimary, s.primary)
            assertAa("$t onSecondary on secondary", s.onSecondary, s.secondary)
            assertAa("$t onTertiary on tertiary", s.onTertiary, s.tertiary)
            assertAa("$t onPrimaryContainer on primaryContainer", s.onPrimaryContainer, s.primaryContainer)
            assertAa("$t onSecondaryContainer on secondaryContainer", s.onSecondaryContainer, s.secondaryContainer)
            assertAa("$t onSurface on secondaryContainer", s.onSurface, s.secondaryContainer)
            assertAa("$t onTertiaryContainer on tertiaryContainer", s.onTertiaryContainer, s.tertiaryContainer)
            assertAa("$t onError on error", s.onError, s.error)
            assertAa("$t onErrorContainer on errorContainer", s.onErrorContainer, s.errorContainer)
            assertAa("$t inverseOnSurface on inverseSurface", s.inverseOnSurface, s.inverseSurface)
            assertAa("$t inversePrimary on inverseSurface", s.inversePrimary, s.inverseSurface)
            assertAa("$t onBackground on background", s.onBackground, s.background)
            for (n in listOf("surface", "surfaceVariant", "surfaceContainer", "surfaceContainerHigh", "surfaceContainerHighest")) {
                assertAa("$t onSurface on $n", s.onSurface, surfaces[n] ?: s.surfaceContainerHighest)
            }
        }
        println("WP ANBAR theme contrast\n$ratios")
    }
}
