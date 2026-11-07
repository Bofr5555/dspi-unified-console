package se.bofr.dspi.app

import android.app.Application
import android.os.Looper
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.setValue
import androidx.compose.runtime.snapshots.Snapshot
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.semantics.SemanticsActions
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.text.TextLayoutResult
import androidx.compose.ui.unit.Density
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
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode
import se.bofr.dspi.fake.FakeDspiCore
import se.bofr.dspi.feature.analysis.AnalysisScreen
import se.bofr.dspi.ui.LocalDemoMode

/**
 * AN32: every segment of the Spectrum Analyser's two pickers shows its whole text ("Curves" showed
 * as "Curve" on a phone) on a 360 dp wide screen at font scale 1.0 to 1.3, and in the
 * side-by-side layout from 600 dp. The demo bridge on the fake core; the analyser page opened directly.
 */
// Native graphics: the legacy shadows measure text by character count, not by the font. The fake
// core, not the real one: the FFI library loads once per JVM, in the other tests' (legacy) sandbox.
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@RunWith(DspiRobolectricTestRunner::class)
@Config(sdk = [35], application = Application::class, qualifiers = "w360dp-h740dp-port-mdpi")
class SpectrumSegmentsAn32Test {
    @get:Rule val rule = createComposeRule()

    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)

    private var session: DemoSession? = null

    @After
    fun stop() {
        rule.runOnUiThread { session?.close() }
        scope.cancel()
    }

    private val segments = listOf("Inputs", "Outputs", "Curves", "Bars", "Both")

    private fun settle() {
        rule.runOnUiThread { Snapshot.sendApplyNotifications() }
        rule.mainClock.advanceTimeBy(100)
    }

    private fun layoutOf(text: String): TextLayoutResult {
        val nodes = rule.onAllNodesWithText(text, useUnmergedTree = true).fetchSemanticsNodes()
            .filter { SemanticsActions.GetTextLayoutResult in it.config }
        assertEquals("text nodes \"$text\"", 1, nodes.size)
        val out = mutableListOf<TextLayoutResult>()
        nodes[0].config[SemanticsActions.GetTextLayoutResult].action!!.invoke(out)
        return out.single()
    }

    private fun assertEverySegmentFitsAtScales(vararg scales: Float) {
        val bridge = startDemoSession(scope, "an32", core = FakeDspiCore.demo()).also { session = it }.bridge
        rule.mainClock.autoAdvance = false
        var fontScale by mutableFloatStateOf(scales.first())
        rule.setContent {
            val d = LocalDensity.current
            CompositionLocalProvider(LocalDensity provides Density(d.density, fontScale), LocalDemoMode provides true) {
                AnalysisScreen(bridge, {}, {}, open = "spectrum")
            }
        }
        // The pickers show once the analyser's caps arrive from the FFI thread, in real time
        val end = System.currentTimeMillis() + 15_000
        while (rule.onAllNodesWithText("Curves", useUnmergedTree = true).fetchSemanticsNodes().isEmpty()) {
            check(System.currentTimeMillis() < end) { "timed out: the analyser's pickers" }
            shadowOf(Looper.getMainLooper()).idle()
            settle()
            Thread.sleep(10)
        }
        for (scale in scales) {
            rule.runOnUiThread { fontScale = scale }
            settle()
            for (s in segments) {
                val l = layoutOf(s)
                // A word too wide for its segment breaks inside the word, and maxLines = 1 hides the rest ("Curve")
                val what = "\"$s\" at font scale $scale (line 0 ends at ${l.getLineEnd(0, true)}, " +
                    "${l.getLineRight(0) - l.getLineLeft(0)} of ${l.size.width} px)"
                assertEquals("$what: the whole text on the first line", s.length, l.getLineEnd(0, true))
                assertFalse("$what: no hidden line", l.didOverflowHeight)
                assertTrue("$what: the line fits", l.getLineRight(0) - l.getLineLeft(0) <= l.size.width + 0.5f)
            }
        }
    }

    @Test
    fun an32EverySegmentFitsA360DpPhoneAtFontScale1To13() = assertEverySegmentFitsAtScales(1.0f, 1.15f, 1.3f)

    @Test
    @Config(qualifiers = "w600dp-h960dp-port-mdpi")
    fun an32EverySegmentFitsSideBySideFrom600Dp() = assertEverySegmentFitsAtScales(1.0f, 1.3f)
}
