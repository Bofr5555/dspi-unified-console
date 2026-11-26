package se.bofr.dspi.feature.graph

import android.app.Application
import android.os.Looper
import androidx.activity.ComponentActivity
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.material3.Surface
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.snapshots.Snapshot
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.SemanticsActions
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.semantics.getOrNull
import androidx.compose.ui.test.SemanticsMatcher
import androidx.compose.ui.test.click
import androidx.compose.ui.test.hasContentDescription
import androidx.compose.ui.test.hasTestTag
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performMouseInput
import androidx.compose.ui.test.performSemanticsAction
import androidx.compose.ui.test.performTouchInput
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode
import se.bofr.dspi.bridge.DspiBridgeImpl
import se.bofr.dspi.core.DspiConstants.FILTER_HIGHPASS
import se.bofr.dspi.core.DspiConstants.FILTER_HIGHPASS1
import se.bofr.dspi.core.DspiConstants.FILTER_LOWSHELF
import se.bofr.dspi.core.DspiConstants.FILTER_LOWSHELF1
import se.bofr.dspi.core.DspiConstants.FILTER_PEAKING
import se.bofr.dspi.fake.FakeDspiCore
import se.bofr.dspi.ui.DspiTheme
import se.bofr.dspi.ui.settings.AppSettings
import se.bofr.dspi.ui.settings.InMemoryAppSettingsStore
import se.bofr.dspi.ui.settings.LocalAppSettings

/**
 * WP UAN (UXP A12, B10) on a phone: a finger on a band opens the band sheet with 48dp rows
 * whose Lower/Raise step the band as TalkBack's actions do and whose Done lets it go; a
 * mouse keeps the floating chip; the legend's pills choose the edited channel, a long-press
 * shows or hides the curve; the value dialog's ± flips a gain's sign.
 */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35], application = Application::class, qualifiers = "w400dp-h800dp-port-mdpi")
@GraphicsMode(GraphicsMode.Mode.NATIVE)
class BandSheetWpUanTest {
    @get:Rule val rule = createAndroidComposeRule<ComponentActivity>()

    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main)
    private val bridge = DspiBridgeImpl(FakeDspiCore.seeded(), scope, ffi = Dispatchers.Main, main = Dispatchers.Main)
    private val store = InMemoryAppSettingsStore(AppSettings())

    // App id 2 = output 1
    private val out1 = 2

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

    private fun showEditor() {
        rule.mainClock.autoAdvance = false
        bridge.scanDevices()
        settle { bridge.status.value.connected }
        bridge.setFilter(out1, 0, FILTER_PEAKING, 2000f, 3f, 1f)
        settle()
        rule.setContent {
            CompositionLocalProvider(LocalAppSettings provides store) {
                DspiTheme(darkTheme = true) {
                    Surface {
                        Column { GraphView(bridge, Modifier.fillMaxWidth().height(300.dp), channels = setOf(out1), editable = true) }
                    }
                }
            }
        }
        settle()
    }

    /** Band 1's dot, in the graph's own coordinates. */
    private fun dot(): androidx.compose.ui.geometry.Offset {
        val handle = rule.onNode(
            hasContentDescription("Band 1,", substring = true) and SemanticsMatcher.keyIsDefined(SemanticsProperties.StateDescription)
        ).fetchSemanticsNode().boundsInRoot
        val graph = rule.onNodeWithContentDescription("Response graph").fetchSemanticsNode().boundsInRoot
        return handle.center - graph.topLeft
    }

    private fun sheetShown() = rule.onAllNodes(hasTestTag(PEQ_BAND_SHEET_TAG)).fetchSemanticsNodes().isNotEmpty()

    @Test
    fun uanAFingerOnABandOpensTheSheetWhoseRowsStepIt() {
        showEditor()
        val at = dot()
        rule.onNodeWithContentDescription("Response graph").performTouchInput { click(at) }
        settle { sheetShown() }
        // 48dp rows: each step button and the value buttons
        for (name in listOf("Lower Freq", "Raise Freq", "Lower Gain", "Raise Gain", "Lower Q", "Raise Q")) {
            val b = rule.onNodeWithContentDescription(name).fetchSemanticsNode().boundsInRoot
            assertTrue("$name ${b.height}", b.height >= 47.5f && b.width >= 47.5f)
        }
        rule.onNodeWithContentDescription("Raise Gain").performClick()
        settle { bridge.filterGain(out1, 0) == 3.5f }
        rule.onNodeWithContentDescription("Lower Gain").performClick()
        rule.onNodeWithContentDescription("Lower Gain").performClick()
        settle { bridge.filterGain(out1, 0) == 2.5f }
        assertTrue(rule.onAllNodes(hasContentDescription("Bypass band 1")).fetchSemanticsNodes().isNotEmpty())
        rule.onNode(
            SemanticsMatcher.expectValue(SemanticsProperties.Text, listOf(androidx.compose.ui.text.AnnotatedString("Done")))
        ).performClick()
        settle { !sheetShown() }
    }

    @Test
    fun uanAMouseKeepsTheFloatingChip() {
        showEditor()
        val at = dot()
        rule.onNodeWithContentDescription("Response graph").performMouseInput { click(at) }
        settle { rule.onAllNodes(hasContentDescription("Shape, Bell")).fetchSemanticsNodes().isNotEmpty() }
        assertFalse(sheetShown())
    }

    @Test
    fun uanTheLegendChoosesTheEditedChannelAndALongPressHidesTheCurve() {
        rule.mainClock.autoAdvance = false
        bridge.scanDevices()
        settle { bridge.status.value.connected }
        val chosen = mutableStateOf<Int?>(null)
        rule.setContent {
            DspiTheme(darkTheme = true) { GraphLegend(bridge, chosen = chosen.value) { chosen.value = it } }
        }
        settle()
        val pill = rule.onNodeWithTag(GRAPH_LEGEND_PILL_TAG + out1)
        assertEquals(false, pill.fetchSemanticsNode().config.getOrNull(SemanticsProperties.Selected))
        // TalkBack's tap and long-press (the actions the pill offers)
        pill.performSemanticsAction(SemanticsActions.OnClick)
        settle { chosen.value == out1 }
        assertEquals(
            true,
            rule.onNodeWithTag(GRAPH_LEGEND_PILL_TAG + out1).fetchSemanticsNode().config.getOrNull(SemanticsProperties.Selected)
        )
        val shown = bridge.channelVisible(out1)
        rule.onNodeWithTag(GRAPH_LEGEND_PILL_TAG + out1).performSemanticsAction(SemanticsActions.OnLongClick)
        settle { bridge.channelVisible(out1) != shown }
        assertEquals("a long-press chooses nothing", out1, chosen.value)
    }

    @Test
    fun uanTheValueDialogsSignButtonFlipsAGain() {
        assertEquals("-3", flipSign("3"))
        assertEquals("3.5", flipSign("-3.5"))
        assertEquals("-2", flipSign("+2"))
        assertEquals("", flipSign(" "))
    }

    @Test
    fun uanTheSheetsShapesAreTheBandsWithTheirOrders() {
        val shapes = peqSheetShapes(setOf(FILTER_PEAKING, FILTER_HIGHPASS, FILTER_HIGHPASS1, FILTER_LOWSHELF, FILTER_LOWSHELF1))
        assertEquals(
            listOf("Bell", "Low Shelf", "Low Shelf, 1st order", "Low Cut, 12 dB/oct", "Low Cut, 6 dB/oct"),
            shapes.map { it.third }
        )
        assertEquals(PeqShape.LOW_CUT to 1, shapes.last().let { it.first to it.second })
    }
}
