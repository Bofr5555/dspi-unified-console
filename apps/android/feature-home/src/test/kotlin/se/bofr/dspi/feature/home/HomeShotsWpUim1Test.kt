package se.bofr.dspi.feature.home

import android.app.Application
import android.graphics.Bitmap
import android.graphics.Canvas
import android.os.Looper
import androidx.activity.ComponentActivity
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.material3.Surface
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.snapshots.Snapshot
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import java.io.File
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import org.junit.After
import org.junit.Assume.assumeTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode
import se.bofr.dspi.bridge.DspiBridgeImpl
import se.bofr.dspi.fake.FakeDspiCore
import se.bofr.dspi.ui.DspiTheme
import se.bofr.dspi.ui.settings.AppSettings
import se.bofr.dspi.ui.settings.InMemoryAppSettingsStore
import se.bofr.dspi.ui.settings.LocalAppSettings

/**
 * WP UIM1: screenshots of Home on the phone over the fake core, for the
 * reports. Runs only when Gradle is given `-Pdspi.shotsDir=<dir>` (the
 * module's build.gradle.kts hands it on); nothing is asserted.
 */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35], application = Application::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
class HomeShotsWpUim1Test {
    @get:Rule val rule = createAndroidComposeRule<ComponentActivity>()

    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main)
    private val bridge = DspiBridgeImpl(FakeDspiCore.seeded(animate = true), scope, ffi = Dispatchers.Main, main = Dispatchers.Main)
    private val store = InMemoryAppSettingsStore(AppSettings())
    private val dir = System.getProperty("dspi.shotsDir")

    @After fun stop() = scope.cancel()

    private fun settle(rounds: Int = 40) {
        repeat(rounds) {
            shadowOf(Looper.getMainLooper()).idle()
            rule.runOnUiThread { Snapshot.sendApplyNotifications() }
            rule.mainClock.advanceTimeBy(50)
        }
    }

    private fun shoot(name: String, dark: Boolean) {
        assumeTrue("no -Pdspi.shotsDir", !dir.isNullOrEmpty())
        rule.mainClock.autoAdvance = false
        bridge.scanDevices()
        settle()
        rule.setContent {
            CompositionLocalProvider(LocalAppSettings provides store) {
                DspiTheme(darkTheme = dark) { Surface(Modifier.fillMaxSize()) { HomeScreen(bridge, {}, {}) } }
            }
        }
        settle()
        val out = File(dir!!).apply { mkdirs() }.resolve("$name.png")
        // Drawn by hand: captureToImage waits for an idle Home, which moving meters never give
        rule.runOnUiThread {
            val root = rule.activity.window.decorView
            val bitmap = Bitmap.createBitmap(root.width, root.height, Bitmap.Config.ARGB_8888)
            root.draw(Canvas(bitmap))
            out.outputStream().use { bitmap.compress(Bitmap.CompressFormat.PNG, 100, it) }
        }
    }

    /** What a phone shows without scrolling. */
    @Test
    @Config(qualifiers = "w400dp-h860dp-port-xhdpi")
    fun uim1PhoneHomeFirstScreen() = shoot("home-phone-first-screen", dark = true)

    /** The whole page, scrolled out (tall enough for every section). */
    @Test
    @Config(qualifiers = "w400dp-h2600dp-port-mdpi")
    fun uim1PhoneHomeWholePage() = shoot("home-phone-whole-page", dark = true)

    @Test
    @Config(qualifiers = "w400dp-h860dp-port-xhdpi")
    fun uim1PhoneHomeFirstScreenLight() = shoot("home-phone-first-screen-light", dark = false)
}
