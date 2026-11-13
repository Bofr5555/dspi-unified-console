package se.bofr.dspi.feature.home.system

import android.app.Application
import android.graphics.Bitmap
import android.graphics.Canvas
import android.os.Looper
import androidx.activity.ComponentActivity
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.material3.Surface
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.snapshots.Snapshot
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.assertIsEnabled
import androidx.compose.ui.test.assertIsNotEnabled
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import java.io.File
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.launch
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assume.assumeTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode
import se.bofr.dspi.bridge.DspiBridgeImpl
import se.bofr.dspi.core.DspiConstants
import se.bofr.dspi.fake.FakeDspiCore
import se.bofr.dspi.feature.home.HomeScreen
import se.bofr.dspi.ui.DspiRoutes
import se.bofr.dspi.ui.DspiTheme
import se.bofr.dspi.ui.settings.AppSettings
import se.bofr.dspi.ui.settings.InMemoryAppSettingsStore
import se.bofr.dspi.ui.settings.LocalAppSettings

/**
 * WP LKUI1: the System screen and Home's chip in Compose, over the fake core's
 * audio link installation: the units with their lock, a recall and the refused
 * one, the roles, and the chip only on a hub. With `-Pdspi.shotsDir=<dir>`, the
 * screen in dark and light as PNGs (as HomeShotsWpUim1Test). The page is tall
 * enough to show every section without scrolling (the clock stays paused).
 */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35], application = Application::class, qualifiers = "w400dp-h2000dp-port-mdpi")
@GraphicsMode(GraphicsMode.Mode.NATIVE)
class SystemScreenWpLkui1Test {
    @get:Rule val rule = createAndroidComposeRule<ComponentActivity>()

    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main)
    private val core = FakeDspiCore()
    private val bridge = DspiBridgeImpl(core, scope, ffi = Dispatchers.Main, main = Dispatchers.Main)
    private val model = SystemScreenModel(bridge, scope)
    private val dir = System.getProperty("dspi.shotsDir")

    @After fun stop() = scope.cancel()

    private fun settle(rounds: Int = 20) {
        repeat(rounds) {
            shadowOf(Looper.getMainLooper()).idle()
            rule.runOnUiThread { Snapshot.sendApplyNotifications() }
            rule.mainClock.advanceTimeBy(50)
        }
    }

    private fun open(role: String, dark: Boolean = true) {
        rule.mainClock.autoAdvance = false
        scope.launch { bridge.attachDevice(42, core.serial, DspiConstants.VENDOR_ID) }
        settle()
        core.audioLinkSystem.onHub = true
        core.audioLinkSystem.role = role
        scope.launch { model.refresh() }
        settle()
        rule.setContent { DspiTheme(darkTheme = dark) { Surface(Modifier.fillMaxSize()) { SystemView(model) {} } } }
        settle()
    }

    private fun refresh() {
        scope.launch { model.refresh() }
        settle()
    }

    @Test
    fun wpLkui1UnitsScenesAndARefusedRecall() {
        open("admin")
        rule.onNodeWithContentDescription("Status: Audio link: 4 of 4 locked, link delay to write on 2").assertIsDisplayed()
        rule.onNodeWithContentDescription("Main left, Receiver, Locked").assertIsDisplayed()
        rule.onNodeWithContentDescription("Main right, Receiver, Locked, link delay pending").assertIsDisplayed()
        rule.onNodeWithContentDescription("Recall Evening").assertIsEnabled().performClick()
        settle()
        rule.onNodeWithText("Evening recalled through the sender", substring = true).assertIsDisplayed()
        rule.onNodeWithContentDescription("Recall Closed; it sets the system mute").performClick()
        settle()
        rule.onNodeWithText("Closed was refused: a scene is already scheduled on the sender", substring = true).assertIsDisplayed()
        assertEquals("Evening", model.state.value.recalledScene)
        shoot("system-refused", dark = true)
    }

    @Test
    fun wpLkui1AViewerGetsNothingThatWrites() {
        open("viewer")
        rule.onNodeWithText("Role on the hub: Viewer").assertIsDisplayed()
        rule.onNodeWithContentDescription("Recall Evening").assertIsNotEnabled()
        rule.onNodeWithText("Recalling a scene needs the hub's Control role.").assertIsDisplayed()
        rule.onNodeWithContentDescription("Clock and links, folded").performClick()
        settle()
        rule.onNodeWithText("Changing the installation needs the hub's Admin role. This part is read only.").assertIsDisplayed()
        rule.onNodeWithContentDescription("System delay automatic").assertIsNotEnabled()
        rule.onNodeWithText("Save").assertIsNotEnabled()
        rule.onNodeWithText("Write link delays…").assertIsNotEnabled()
        shoot("system-viewer", dark = false)
    }

    @Test
    fun wpLkui1StaleSaveOffersReload() {
        open("admin")
        rule.onNodeWithContentDescription("Clock and links, folded").performClick()
        settle()
        rule.runOnUiThread { model.setUnitLatency("B", "150") }
        settle()
        rule.onNodeWithText("Save").assertIsEnabled()
        core.audioLinkSystem.changeElsewhere()
        refresh()
        rule.onNodeWithText(
            "The installation changed on the hub while you edited it",
            substring = true
        ).assertIsDisplayed()
        rule.onNodeWithText("Reload").assertIsEnabled()
        shoot("system-save-stale", dark = true)
        rule.onNodeWithText("Reload").performClick()
        settle()
        assertEquals(false, model.state.value.stale)
    }

    @Test
    fun wpLkui1TheChipShowsOnAHubOnlyAndOpensTheScreen() {
        rule.mainClock.autoAdvance = false
        val routes = mutableListOf<String>()
        rule.setContent {
            DspiTheme(darkTheme = true) {
                val s by model.state.collectAsState()
                SystemStatusChipView(s, onOpen = { routes += DspiRoutes.SYSTEM })
            }
        }
        settle()
        rule.onNodeWithText("System ›").assertDoesNotExist()
        core.audioLinkSystem.onHub = true
        refresh()
        rule.onNodeWithContentDescription(
            "Audio link: 4 of 4 locked, link delay to write on 2. Open System"
        ).assertIsDisplayed().performClick()
        assertEquals(listOf(DspiRoutes.SYSTEM), routes)
    }

    @Test
    fun wpLkui1ShotsInBothSchemes() {
        assumeTrue("no -Pdspi.shotsDir", !dir.isNullOrEmpty())
        open("admin", dark = false)
        shoot("system", dark = false)
    }

    /** Home on a hub with the installation: the chip under the health line. */
    @Test
    @Config(qualifiers = "w400dp-h860dp-port-xhdpi")
    fun wpLkui1HomeChipShot() {
        assumeTrue("no -Pdspi.shotsDir", !dir.isNullOrEmpty())
        rule.mainClock.autoAdvance = false
        scope.launch { bridge.attachDevice(42, core.serial, DspiConstants.VENDOR_ID) }
        settle()
        core.audioLinkSystem.onHub = true
        rule.setContent {
            CompositionLocalProvider(LocalAppSettings provides InMemoryAppSettingsStore(AppSettings())) {
                DspiTheme(darkTheme = true) { Surface(Modifier.fillMaxSize()) { HomeScreen(bridge, {}, {}) } }
            }
        }
        settle(40)
        rule.onNodeWithText("System ›").assertIsDisplayed()
        shoot("home-chip", dark = true)
    }

    private fun shoot(name: String, dark: Boolean) {
        if (dir.isNullOrEmpty()) return
        settle()
        val out = File(dir).apply { mkdirs() }.resolve("$name-${if (dark) "dark" else "light"}.png")
        rule.runOnUiThread {
            val root = rule.activity.window.decorView
            val bitmap = Bitmap.createBitmap(root.width, root.height, Bitmap.Config.ARGB_8888)
            root.draw(Canvas(bitmap))
            out.outputStream().use { bitmap.compress(Bitmap.CompressFormat.PNG, 100, it) }
        }
    }
}
