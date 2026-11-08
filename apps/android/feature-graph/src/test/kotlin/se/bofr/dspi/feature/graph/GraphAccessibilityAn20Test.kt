package se.bofr.dspi.feature.graph

import android.app.Application
import android.os.Looper
import android.view.View
import androidx.activity.ComponentActivity
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.material3.Surface
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.snapshots.Snapshot
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalView
import androidx.compose.ui.semantics.SemanticsActions
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.test.SemanticsMatcher
import androidx.compose.ui.test.SemanticsNodeInteraction
import androidx.compose.ui.test.assert
import androidx.compose.ui.test.assertIsSelected
import androidx.compose.ui.test.hasContentDescription
import androidx.compose.ui.test.hasStateDescription
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.performSemanticsAction
import androidx.compose.ui.unit.dp
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
import se.bofr.dspi.core.DspiConstants.FILTER_PEAKING
import se.bofr.dspi.fake.FakeDspiCore
import se.bofr.dspi.ui.DspiTheme
import se.bofr.dspi.ui.a11y.AtfChecks
import se.bofr.dspi.ui.settings.AppSettings
import se.bofr.dspi.ui.settings.InMemoryAppSettingsStore
import se.bofr.dspi.ui.settings.LocalAppSettings

/**
 * WP AN20: the response graph as TalkBack reads it. Each PEQ dot is a node
 * ("Band 1, Bell", its values as state) whose custom actions do what the
 * drag does in the arrow keys' steps; the chip's values type on a click and
 * step with actions; the resize strip has Taller / Shorter. ATF's checks on
 * the graph screen. V32 FakeDspiCore; native graphics (see AtfChecks).
 */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35], application = Application::class, qualifiers = "w400dp-h800dp-port-mdpi")
@GraphicsMode(GraphicsMode.Mode.NATIVE)
class GraphAccessibilityAn20Test {
    @get:Rule val rule = createAndroidComposeRule<ComponentActivity>()

    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main)
    private val bridge = DspiBridgeImpl(FakeDspiCore.seeded(), scope, ffi = Dispatchers.Main, main = Dispatchers.Main)
    private val store = InMemoryAppSettingsStore(AppSettings())
    private lateinit var view: View

    // App id 2 = output 1
    private val out1 = 2

    @After fun stop() = scope.cancel()

    /** The graph never idles with the clock on auto (as :app's AN28 tests): run frames by hand. */
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

    private fun connect() {
        rule.mainClock.autoAdvance = false
        bridge.scanDevices()
        settle { bridge.status.value.connected }
    }

    private fun showEditor() {
        connect()
        // Two known bands on OUT1, low to high (the handles follow frequency, not band number)
        bridge.setFilter(out1, 0, FILTER_PEAKING, 2000f, 3f, 1f)
        bridge.setFilter(out1, 1, FILTER_PEAKING, 200f, -2f, 0.7f)
        settle()
        rule.setContent {
            view = LocalView.current
            CompositionLocalProvider(LocalAppSettings provides store) {
                DspiTheme(darkTheme = true) {
                    Surface {
                        Column {
                            GraphView(bridge, Modifier.fillMaxWidth().height(300.dp), channels = setOf(out1), editable = true)
                            GraphResizeHandle()
                        }
                    }
                }
            }
        }
        settle()
    }

    private fun handle(band: Int): SemanticsNodeInteraction = rule.onNode(
        hasContentDescription("Band ${band + 1},", substring = true) and SemanticsMatcher.keyIsDefined(SemanticsActions.CustomActions)
    )

    private fun SemanticsNodeInteraction.custom(label: String) {
        val actions = fetchSemanticsNode().config[SemanticsActions.CustomActions]
        val action = actions.firstOrNull { it.label == label } ?: error("no \"$label\" in ${actions.map { it.label }}")
        rule.runOnUiThread { assertTrue(action.action()) }
        settle()
    }

    @Test
    fun an20EveryDotIsANamedNodeInFrequencyOrder() {
        showEditor()
        val first = handle(0)
        first.assert(hasContentDescription("Band 1, Bell"))
            .assert(hasStateDescription("2.000 kHz, +3.00 dB, Q 1.000"))
            .assert(SemanticsMatcher.keyIsDefined(SemanticsActions.OnClick))
        val labels = first.fetchSemanticsNode().config[SemanticsActions.CustomActions].map { it.label }
        assertEquals(
            listOf("Increase frequency", "Decrease frequency", "Increase gain", "Decrease gain", "Increase Q", "Decrease Q", "Bypass band"),
            labels
        )
        // Traversal follows frequency: band 2 (200 Hz) before band 1 (2 kHz)
        val order = { b: Int -> handle(b).fetchSemanticsNode().config[SemanticsProperties.TraversalIndex] }
        assertTrue("200 Hz first", order(1) < order(0))
        // Each handle is a full 48 dp target
        val bounds = first.fetchSemanticsNode().boundsInRoot
        assertEquals(48f, bounds.width, 0.5f)
        assertEquals(48f, bounds.height, 0.5f)
    }

    @Test
    fun an20HandleActionsAdjustTheBandAsTheDragWould() {
        showEditor()
        handle(0).custom("Increase gain")
        handle(0).assert(hasStateDescription("2.000 kHz, +3.50 dB, Q 1.000"))
        handle(0).custom("Decrease frequency")
        // 1/12 octave down from 2 kHz
        handle(0).assert(hasStateDescription("1.888 kHz, +3.50 dB, Q 1.000"))
        handle(0).custom("Bypass band")
        handle(0).assert(hasStateDescription("1.888 kHz, +3.50 dB, Q 1.000, bypassed"))
        // The editor commits to the device after its delay (0.45 s)
        settle { bridge.filterGain(out1, 0) == 3.5f && bridge.filterBypass(out1, 0) }
        assertEquals(1888f, bridge.filterFreq(out1, 0), 1f)
    }

    @Test
    fun an20SelectingAHandleOpensTheChipWithTypableValues() {
        showEditor()
        handle(0).performSemanticsAction(SemanticsActions.OnClick)
        settle()
        handle(0).assertIsSelected()
        val freq = rule.onNode(hasContentDescription("Freq") and SemanticsMatcher.keyIsDefined(SemanticsActions.OnClick))
        freq.assert(hasStateDescription("2.00 kHz"))
        freq.custom("Increase")
        rule.onNode(hasContentDescription("Freq")).assert(hasStateDescription("2.11 kHz"))
        rule.onNodeWithContentDescription("Bypass band 1").assert(SemanticsMatcher.keyIsDefined(SemanticsProperties.ToggleableState))
        rule.onNodeWithContentDescription("Shape, Bell").assert(SemanticsMatcher.keyIsDefined(SemanticsActions.OnClick))
    }

    @Test
    fun an20ResizeStripHasTallerAndShorter() {
        showEditor()
        val strip = rule.onNodeWithContentDescription("Graph height")
        strip.assert(hasStateDescription("250 points"))
        strip.custom("Taller")
        assertEquals(275.0, store.settings.value.graphHeight, 0.0)
        strip.custom("Shorter")
        strip.custom("Shorter")
        assertEquals(225.0, store.settings.value.graphHeight, 0.0)
        strip.assert(hasStateDescription("225 points"))
    }

    @Test
    fun an20AtfPassesOnTheGraphScreen() {
        connect()
        rule.setContent {
            view = LocalView.current
            CompositionLocalProvider(LocalAppSettings provides store) {
                DspiTheme(darkTheme = true) { GraphScreen(bridge, onNavigate = {}, onBack = {}) }
            }
        }
        settle()
        assertTrue("ATF sees the graph", AtfChecks.nodeCount(view) > 10)
        AtfChecks.assertPass(view, "graph screen")
    }

    @Test
    fun an20AtfPassesOnTheEditableGraph() {
        showEditor()
        assertTrue(
            "the handles are there",
            rule.onAllNodes(hasContentDescription("Band ", substring = true)).fetchSemanticsNodes().size >= 2
        )
        AtfChecks.assertPass(view, "editable graph")
        // The chip keeps the macOS chip's metrics (15 dp rows); everything else on it passes, and every
        // value it edits is also on the band's handle (custom actions, 48 dp)
        handle(0).performSemanticsAction(SemanticsActions.OnClick)
        settle()
        val chip = AtfChecks.results(view)
        println("AN20 chip: $chip")
        assertTrue("only touch-target sizes: $chip", chip.isNotEmpty() && chip.all { it.startsWith("TouchTargetSizeCheck") })
    }
}
