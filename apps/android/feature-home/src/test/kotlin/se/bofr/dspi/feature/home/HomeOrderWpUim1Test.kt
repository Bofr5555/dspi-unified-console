package se.bofr.dspi.feature.home

import android.app.Application
import android.os.Looper
import androidx.activity.ComponentActivity
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.material3.Surface
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.snapshots.Snapshot
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.SemanticsActions
import androidx.compose.ui.test.SemanticsMatcher
import androidx.compose.ui.test.getUnclippedBoundsInRoot
import androidx.compose.ui.test.hasContentDescription
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.unit.Dp
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
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
 * WP UIM1: the phone's Home puts volume first, then preset and source, the
 * output meters and the health line, then the graph, the channel list and the
 * tool icons (docs/UI-MODES.md). The tablet's split view keeps the macOS
 * sidebar order. Over the V32 FakeDspiCore, the page tall enough to lay out
 * every section.
 */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35], application = Application::class, qualifiers = "w400dp-h2600dp-port-mdpi")
@GraphicsMode(GraphicsMode.Mode.NATIVE)
class HomeOrderWpUim1Test {
    @get:Rule val rule = createAndroidComposeRule<ComponentActivity>()

    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main)
    private val bridge = DspiBridgeImpl(FakeDspiCore.seeded(), scope, ffi = Dispatchers.Main, main = Dispatchers.Main)
    private val store = InMemoryAppSettingsStore(AppSettings())

    @After fun stop() = scope.cancel()

    private fun settle(until: () -> Boolean = { true }) {
        val end = System.currentTimeMillis() + 10_000
        var rounds = 0
        while (rounds < 20 || !until()) {
            check(System.currentTimeMillis() < end) { "timed out" }
            shadowOf(Looper.getMainLooper()).idle()
            rule.runOnUiThread { Snapshot.sendApplyNotifications() }
            rule.mainClock.advanceTimeBy(50)
            rounds++
        }
    }

    private fun showHome() {
        rule.mainClock.autoAdvance = false
        bridge.scanDevices()
        settle { bridge.status.value.connected }
        rule.setContent {
            CompositionLocalProvider(LocalAppSettings provides store) {
                DspiTheme(darkTheme = true) { Surface(Modifier.fillMaxSize()) { HomeScreen(bridge, {}, {}) } }
            }
        }
        settle()
    }

    private val volume = hasContentDescription("User Volume") and SemanticsMatcher.keyIsDefined(SemanticsActions.SetProgress)
    private val preset = hasContentDescription("Preset actions")
    private val source = hasText("Source")
    private val out1Meter = hasContentDescription("OUT1 level")
    private val health = hasContentDescription("Health: ", substring = true)
    private val graphTitle = hasText("Filter Response")
    private val inputs = hasText("INPUTS")
    private val matrixIcon = hasContentDescription("Matrix Mixer")
    private val toolsMenu = hasContentDescription(TOOLS_MENU_LABEL)

    private fun top(m: SemanticsMatcher): Dp = rule.onNode(m, useUnmergedTree = true).getUnclippedBoundsInRoot().top

    private fun assertOrder(vararg sections: Pair<String, SemanticsMatcher>) {
        sections.toList().zipWithNext().forEach { (a, b) ->
            val ta = top(a.second)
            val tb = top(b.second)
            assertTrue("${a.first} ($ta) comes before ${b.first} ($tb)", ta < tb)
        }
    }

    @Test
    fun uim1PhoneHomePutsVolumeFirst() {
        showHome()
        assertOrder(
            "Tools menu" to toolsMenu,
            "volume" to volume,
            "preset" to preset,
            "source" to source,
            "output meters" to out1Meter,
            "health line" to health,
            "graph" to graphTitle,
            "channel list" to inputs,
            "tool icons" to matrixIcon
        )
    }

    @Test
    fun uim1PhoneHealthLineSaysAllIsWellOnTheDemo() {
        showHome()
        rule.onNode(hasContentDescription("Health: All outputs OK")).assertExists()
        // One meter per enabled output in the front, beside the channel list's own
        val s = bridge.state.value
        val enabled = MatrixOutput.visible(bridge.platformName, s.outputTypes).count { s.outputEnabled.getOrElse(it.index) { false } }
        assertTrue("the demo has outputs on", enabled > 0)
        assertEquals(enabled, rule.onAllNodes(hasContentDescription(" level", substring = true)).fetchSemanticsNodes().size)
    }

    /** The split view keeps the macOS sidebar: channels, the icons, then preset, source and volume; no phone front. */
    @Test
    @Config(qualifiers = "w1000dp-h800dp-land-mdpi")
    fun uim1TabletKeepsTheSidebarOrder() {
        showHome()
        assertOrder("channel list" to inputs, "tool icons" to matrixIcon, "preset" to preset, "volume" to volume)
        rule.onNode(health).assertDoesNotExist()
        rule.onNode(out1Meter).assertDoesNotExist()
    }
}
