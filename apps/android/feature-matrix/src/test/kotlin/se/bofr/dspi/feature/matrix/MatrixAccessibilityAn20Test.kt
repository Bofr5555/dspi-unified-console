package se.bofr.dspi.feature.matrix

import android.app.Application
import android.view.View
import androidx.activity.ComponentActivity
import androidx.compose.material3.Surface
import androidx.compose.ui.platform.LocalView
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.SemanticsActions
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.test.SemanticsMatcher
import androidx.compose.ui.test.assert
import androidx.compose.ui.test.assertIsOff
import androidx.compose.ui.test.assertIsOn
import androidx.compose.ui.test.hasContentDescription
import androidx.compose.ui.test.hasStateDescription
import androidx.compose.ui.test.isToggleable
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.performClick
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
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode
import se.bofr.dspi.bridge.DspiBridgeImpl
import se.bofr.dspi.fake.FakeDspiCore
import se.bofr.dspi.ui.DspiTheme
import se.bofr.dspi.ui.a11y.AtfChecks

/**
 * WP AN20: the matrix as TalkBack reads it (each crosspoint names its input
 * and output and says whether it routes; INV, enable and mute are switches
 * with their output's name) and ATF's checks on both layouts, over the V32
 * FakeDspiCore. Native graphics: see AtfChecks.
 */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35], application = Application::class, qualifiers = "w400dp-h800dp-port-mdpi")
@GraphicsMode(GraphicsMode.Mode.NATIVE)
class MatrixAccessibilityAn20Test {
    @get:Rule val rule = createAndroidComposeRule<ComponentActivity>()

    // The bridge on the main looper, as in the app; the fake answers at once
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main)
    private val bridge = DspiBridgeImpl(FakeDspiCore.seeded(), scope, ffi = Dispatchers.Main, main = Dispatchers.Main)
    private lateinit var view: View

    @After fun stop() = scope.cancel()

    /** The real screen: the layout follows the window's width (the grid from 600 dp). */
    private fun show() {
        bridge.scanDevices()
        rule.setContent {
            view = LocalView.current
            DspiTheme(darkTheme = true) { Surface { MatrixMixerView(bridge, {}, {}) } }
        }
        rule.waitUntil(5_000) { bridge.status.value.connected }
        rule.waitForIdle()
    }

    private fun firstRoute(): Pair<MatrixInput, MatrixOutput> {
        val s = MatrixMixerState.from(bridge, activeInputChannels = 2, connected = true)
        return s.matrixInputs.first() to s.visibleOutputs.first()
    }

    @Test
    fun an20CrosspointsNameTheirRouteAndState() {
        show()
        val (row, out) = firstRoute()
        val label = routeLabel(row, out)
        val dot = rule.onNode(hasContentDescription(label) and isToggleable())
        dot.assert(SemanticsMatcher.expectValue(SemanticsProperties.Role, Role.Switch))
        val routed = bridge.matrixRouting(row.index, out.index)
        dot.assert(hasStateDescription(if (routed) "Connected" else "Not connected"))
        if (routed) {
            rule.onNode(hasContentDescription("Invert phase, $label") and isToggleable()).assertExists()
            rule.onNode(hasContentDescription("Gain, $label")).assert(SemanticsMatcher.keyIsDefined(SemanticsActions.OnClick))
        }
        dot.performClick()
        rule.waitForIdle()
        assertEquals("the switch routes", !routed, bridge.matrixRouting(row.index, out.index))
        dot.assert(hasStateDescription(if (routed) "Not connected" else "Connected"))
    }

    @Test
    fun an20OutputSwitchesSayTheirOutputAndState() {
        show()
        val out = firstRoute().second
        val mute = rule.onNode(hasContentDescription("Mute ${out.name}") and isToggleable())
        if (bridge.outputMuted(out.index)) mute.assertIsOn() else mute.assertIsOff()
        rule.onNode(hasContentDescription("Enable ${out.name}", substring = true) and isToggleable()).assertExists()
    }

    @Test
    fun an20AtfPassesOnThePhonePages() {
        show()
        assertTrue("ATF sees the matrix", AtfChecks.nodeCount(view) > 20)
        AtfChecks.assertPass(view, "matrix, phone pages")
    }

    @Test
    @Config(qualifiers = "w1000dp-h1400dp-land-mdpi")
    fun an20AtfPassesOnTheGrid() {
        show()
        val (row, out) = firstRoute()
        rule.onNode(hasContentDescription(routeLabel(row, out)) and isToggleable()).assertExists()
        assertTrue("ATF sees the grid", AtfChecks.nodeCount(view) > 20)
        AtfChecks.assertPass(view, "matrix grid")
    }
}
