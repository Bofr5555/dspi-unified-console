package se.bofr.dspi.feature.home

import android.app.Application
import android.os.Looper
import android.view.View
import androidx.activity.ComponentActivity
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.material3.Surface
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.snapshots.Snapshot
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalView
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.SemanticsActions
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.test.SemanticsMatcher
import androidx.compose.ui.test.assert
import androidx.compose.ui.test.getUnclippedBoundsInRoot
import androidx.compose.ui.test.hasContentDescription
import androidx.compose.ui.test.hasStateDescription
import androidx.compose.ui.test.isToggleable
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.performSemanticsAction
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
import se.bofr.dspi.ui.a11y.AtfChecks
import se.bofr.dspi.ui.settings.AppSettings
import se.bofr.dspi.ui.settings.InMemoryAppSettingsStore
import se.bofr.dspi.ui.settings.LocalAppSettings

/**
 * WP AN20: Home and the channel pages as TalkBack reads them (meters with
 * their level, the effect icons as switches, named sliders and band
 * checkboxes) and ATF's checks on each, over the V32 FakeDspiCore. Native
 * graphics: see AtfChecks.
 */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35], application = Application::class, qualifiers = "w400dp-h800dp-port-mdpi")
@GraphicsMode(GraphicsMode.Mode.NATIVE)
class HomeAccessibilityAn20Test {
    @get:Rule val rule = createAndroidComposeRule<ComponentActivity>()

    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main)
    private val bridge = DspiBridgeImpl(FakeDspiCore.seeded(), scope, ffi = Dispatchers.Main, main = Dispatchers.Main)
    private val store = InMemoryAppSettingsStore(AppSettings())
    private lateinit var view: View

    @After fun stop() = scope.cancel()

    /** Home never idles with the clock on auto (as :app's AN28 tests): run frames by hand. */
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

    private fun show(screen: @Composable () -> Unit) {
        rule.mainClock.autoAdvance = false
        bridge.scanDevices()
        settle { bridge.status.value.connected }
        rule.setContent {
            view = LocalView.current
            CompositionLocalProvider(LocalAppSettings provides store) {
                DspiTheme(darkTheme = true) { Surface(Modifier.fillMaxSize()) { screen() } }
            }
        }
        settle()
    }

    @Test
    fun an20HomeReadsMetersAndEffectSwitches() {
        show { HomeScreen(bridge, {}, {}) }
        val meters = rule.onAllNodes(hasContentDescription("Level", substring = true)).fetchSemanticsNodes()
        assertTrue("channel rows carry their meter", meters.isNotEmpty())
        val bypass = rule.onNode(hasContentDescription("Bypass Master EQ") and isToggleable())
        bypass.assert(SemanticsMatcher.expectValue(SemanticsProperties.Role, Role.Switch))
        val before = bridge.bypass
        // TalkBack's double-tap: the click action
        bypass.performSemanticsAction(SemanticsActions.OnClick)
        settle { bridge.bypass != before }
        rule.onNode(hasContentDescription("Headphone Crossfeed") and isToggleable())
            .assert(SemanticsMatcher.keyIsDefined(SemanticsActions.OnLongClick))
        rule.onNode(hasContentDescription("Settings")).assert(SemanticsMatcher.expectValue(SemanticsProperties.Role, Role.Button))
    }

    @Test
    fun an20AtfPassesOnHome() {
        show { HomeScreen(bridge, {}, {}) }
        assertTrue("ATF sees Home", AtfChecks.nodeCount(view) > 30)
        AtfChecks.assertPass(view, "home")
    }

    /** Review c6c AN20-2: the bottom icon row fits a 360 dp phone (8 x 44 dp in 352 dp), as on main. */
    @Test
    @Config(qualifiers = "w360dp-h740dp-port-mdpi")
    fun an20HomeIconRowFitsA360DpPhone() {
        show { HomeScreen(bridge, {}, {}) }
        val icons = listOf(
            "Matrix Mixer",
            "Headphone Crossfeed",
            "Loudness Compensation",
            "Volume Leveller",
            "Psychoacoustic Bass",
            "Stats for Nerbs",
            "Settings",
            "Bypass Master EQ"
        ).map {
            rule.onNode(hasContentDescription(it) and SemanticsMatcher.keyIsDefined(SemanticsActions.OnClick)).getUnclippedBoundsInRoot()
        }
        icons.forEach { assertEquals(44f, (it.right - it.left).value, 0.5f) }
        assertTrue("the Bypass icon ends inside the screen: ${icons.last()}", icons.last().right.value <= 360f)
        icons.zipWithNext().forEach { (a, b) -> assertTrue("icons do not overlap", a.right <= b.left) }
    }

    @Test
    fun an20InputPageNamesItsPreampAndBands() {
        show { ChannelScreen(bridge, 0, {}, {}) }
        rule.onNode(hasContentDescription("Preamp") and SemanticsMatcher.keyIsDefined(SemanticsActions.SetProgress))
            .assert(SemanticsMatcher.keyIsDefined(SemanticsProperties.StateDescription))
        val bands = rule.onAllNodes(hasContentDescription("Band ", substring = true) and isToggleable()).fetchSemanticsNodes()
        assertTrue("band checkboxes", bands.isNotEmpty())
        assertTrue(bands.all { it.config[SemanticsProperties.Role] == Role.Checkbox })
    }

    @Test
    fun an20AtfPassesOnTheInputPage() {
        show { ChannelScreen(bridge, 0, {}, {}) }
        assertTrue("ATF sees the page", AtfChecks.nodeCount(view) > 30)
        AtfChecks.assertPass(view, "input channel page")
    }

    @Test
    fun an20AtfPassesOnTheOutputPage() {
        show { ChannelScreen(bridge, 2, {}, {}) }
        rule.onNode(hasContentDescription("Gain") and SemanticsMatcher.keyIsDefined(SemanticsActions.SetProgress))
            .assert(hasStateDescription("%.1f dB".format(java.util.Locale.ROOT, bridge.outputGainDB(0))))
        assertTrue("ATF sees the page", AtfChecks.nodeCount(view) > 30)
        AtfChecks.assertPass(view, "output channel page")
    }
}
