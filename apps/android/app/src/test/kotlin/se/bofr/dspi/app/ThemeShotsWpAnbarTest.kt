package se.bofr.dspi.app

import android.app.Application
import android.graphics.Bitmap
import android.graphics.Canvas
import android.os.Looper
import androidx.activity.ComponentActivity
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.Button
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.RadioButton
import androidx.compose.material3.Slider
import androidx.compose.material3.Snackbar
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.runtime.snapshots.Snapshot
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.unit.dp
import java.io.File
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import org.junit.After
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode
import se.bofr.dspi.bridge.DspiBridgeImpl
import se.bofr.dspi.fake.FakeDspiCore
import se.bofr.dspi.feature.analysis.AnalysisScreen
import se.bofr.dspi.feature.presets.PresetsScreen
import se.bofr.dspi.feature.settings.SettingsScreen
import se.bofr.dspi.ui.DspiTheme
import se.bofr.dspi.ui.PillButton
import se.bofr.dspi.ui.ToolChip
import se.bofr.dspi.ui.link.LinkBanner
import se.bofr.dspi.ui.link.LinkBannerAction
import se.bofr.dspi.ui.settings.AppSettings
import se.bofr.dspi.ui.settings.InMemoryAppSettingsStore
import se.bofr.dspi.ui.settings.LocalAppSettings

/**
 * WP ANBAR (owner 2026-11-21): the screens and controls the accent and the Material roles reach
 * (Settings' selected page, the Presets list, Analysis' selected page, pills, chips, Material
 * buttons, switch, radio, slider, snackbar, the lost-connection banner), light and dark. With
 * `-Pdspi.shotsDir=<dir>` each is written there as `theme-<screen>-<theme>.png`.
 */
@RunWith(DspiRobolectricTestRunner::class)
@Config(sdk = [35], application = Application::class, qualifiers = "w1280dp-h800dp-land-mdpi")
@GraphicsMode(GraphicsMode.Mode.NATIVE)
class ThemeShotsWpAnbarTest {
    @get:Rule val rule = createAndroidComposeRule<ComponentActivity>()

    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
    private val bridge = DspiBridgeImpl(FakeDspiCore.seeded(), scope, ffi = Dispatchers.Main, main = Dispatchers.Main)
    private val store = InMemoryAppSettingsStore(AppSettings())
    private val dir = System.getProperty("dspi.shotsDir")

    private val screens: List<Pair<String, @Composable () -> Unit>> = listOf(
        "controls" to { Controls() },
        "settings" to { SettingsScreen(bridge, onNavigate = {}, onBack = {}) },
        "presets" to { PresetsScreen(bridge, onNavigate = {}, onBack = {}) },
        "analysis" to { AnalysisScreen(bridge, onNavigate = {}, onBack = {}) }
    )
    private var screen by mutableStateOf(0)

    @After fun stop() = scope.cancel()

    private fun settle() {
        repeat(20) {
            shadowOf(Looper.getMainLooper()).idle()
            rule.runOnUiThread { Snapshot.sendApplyNotifications() }
            rule.mainClock.advanceTimeBy(50)
        }
    }

    @Composable
    private fun Controls() {
        Column(
            Modifier.fillMaxSize().background(MaterialTheme.colorScheme.background).padding(16.dp),
            verticalArrangement = Arrangement.spacedBy(12.dp)
        ) {
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                PillButton("Selected pill", {}, selected = true)
                PillButton("Pill", {})
                ToolChip("Chip on", on = true, enabled = true) {}
                ToolChip("Chip off", on = false, enabled = true) {}
            }
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                Button({}) { Text("Filled button") }
                TextButton({}) { Text("Text button") }
                Switch(checked = true, onCheckedChange = {})
                Switch(checked = false, onCheckedChange = {})
                RadioButton(selected = true, onClick = {})
            }
            Slider(value = 0.6f, onValueChange = {})
            Snackbar(action = { TextButton({}) { Text("Revert", color = MaterialTheme.colorScheme.inversePrimary) } }) {
                Text("A snackbar on inverseSurface")
            }
            LinkBanner("Connection lost. Reconnecting…", "Studio Mac", listOf(LinkBannerAction("Retry now") {}))
        }
    }

    private fun shootAll(dark: Boolean) {
        rule.mainClock.autoAdvance = false
        rule.setContent {
            CompositionLocalProvider(LocalAppSettings provides store) {
                DspiTheme(darkTheme = dark) { screens[screen].second() }
            }
        }
        for (i in screens.indices) {
            screen = i
            settle()
            if (dir.isNullOrEmpty()) continue
            val out = File(dir).apply { mkdirs() }.resolve("theme-${screens[i].first}-${if (dark) "dark" else "light"}.png")
            rule.runOnUiThread {
                val root = rule.activity.window.decorView
                val bitmap = Bitmap.createBitmap(root.width, root.height, Bitmap.Config.ARGB_8888).also { root.draw(Canvas(it)) }
                out.outputStream().use { bitmap.compress(Bitmap.CompressFormat.PNG, 100, it) }
            }
        }
    }

    @Test fun wpAnbarThemeShotsLight() = shootAll(dark = false)

    @Test fun wpAnbarThemeShotsDark() = shootAll(dark = true)
}
