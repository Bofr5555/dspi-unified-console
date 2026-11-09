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
import androidx.compose.ui.semantics.SemanticsProperties
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
import se.bofr.dspi.core.OutputCard
import se.bofr.dspi.core.OutputLabel
import se.bofr.dspi.fake.FakeDspiCore
import se.bofr.dspi.feature.home.HomeScreen
import se.bofr.dspi.ui.LocalDemoMode

/**
 * BUG1: the K340's output labels ("Out 2 · R · High", the core's, tests/fixtures/preset/
 * k340-bug1.dspipreset) fit the Home rows of a 360 dp phone whole, at font scale 1.0, and the
 * pills tell TalkBack the label and the slot. Native graphics, so text is measured by the font;
 * the fake core with the core's K340 labels, as SpectrumSegmentsAn32Test explains why.
 */
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@RunWith(DspiRobolectricTestRunner::class)
@Config(sdk = [35], application = Application::class, qualifiers = "w360dp-h740dp-port-mdpi")
class OutputLabelsFitBug1Test {
    @get:Rule val rule = createComposeRule()

    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
    private var session: DemoSession? = null

    private val k340 =
        listOf(
            OutputLabel(0, "Out 1 · R · Low", "I2S 1 L", false, "R", "Low"),
            OutputLabel(1, "Out 2 · R · High", "I2S 1 R", false, "R", "High"),
            OutputLabel(2, "Out 3 · L · Low", "I2S 2 L", false, "L", "Low"),
            OutputLabel(3, "Out 4 · L · High", "I2S 2 R", false, "L", "High")
        )

    /** The demo fake with the core's labels for the K340 (outputs 1-4 on, as the demo has them). */
    private inner class K340Core : FakeDspiCore() {
        override fun outputLabels(): List<OutputLabel> = k340 + super.outputLabels().drop(4)

        override fun outputCards(): List<OutputCard> =
            listOf(OutputCard(2, 0, "Woofers", "Woofers L", "Woofers R"), OutputCard(3, 1, "Tweeters", "Tweeters L", "Tweeters R"))
    }

    @After
    fun stop() {
        rule.runOnUiThread { session?.close() }
        scope.cancel()
    }

    private fun settle() {
        rule.runOnUiThread { Snapshot.sendApplyNotifications() }
        rule.mainClock.advanceTimeBy(100)
    }

    private fun layoutOf(text: String): TextLayoutResult {
        val nodes = rule.onAllNodesWithText(text, useUnmergedTree = true).fetchSemanticsNodes()
            .filter { SemanticsActions.GetTextLayoutResult in it.config }
        assertTrue("a text node \"$text\"", nodes.isNotEmpty())
        val out = mutableListOf<TextLayoutResult>()
        nodes[0].config[SemanticsActions.GetTextLayoutResult].action!!.invoke(out)
        return out.single()
    }

    private fun showHome(scale: Float) {
        val bridge = startDemoSession(scope, "bug1", core = K340Core()).also { session = it }.bridge
        rule.mainClock.autoAdvance = false
        var fontScale by mutableFloatStateOf(scale)
        rule.setContent {
            val d = LocalDensity.current
            CompositionLocalProvider(LocalDensity provides Density(d.density, fontScale), LocalDemoMode provides true) {
                HomeScreen(bridge, {}, {})
            }
        }
        val end = System.currentTimeMillis() + 15_000
        while (rule.onAllNodesWithText("Out 2 · R · High", useUnmergedTree = true).fetchSemanticsNodes().isEmpty()) {
            check(System.currentTimeMillis() < end) { "timed out: the output rows" }
            shadowOf(Looper.getMainLooper()).idle()
            settle()
            Thread.sleep(10)
        }
        settle()
    }

    @Test
    fun bug1K340LabelsFitA360DpPhone() {
        showHome(1.0f)
        for (l in k340) {
            val t = layoutOf(l.label)
            val what = "\"${l.label}\" (${t.getLineRight(0) - t.getLineLeft(0)} of ${t.size.width} px)"
            assertFalse("$what: not cut", t.isLineEllipsized(0))
            assertEquals("$what: one line", 1, t.lineCount)
        }
        // The pill's spoken name: descriptor, label without the dots, slot
        val pills = rule.onAllNodesWithText("OUT2", useUnmergedTree = false).fetchSemanticsNodes()
        val spoken = pills.flatMap { it.config.getOrElse(SemanticsProperties.ContentDescription) { emptyList() } }
        assertTrue("$spoken", "OUT2, Out 2, R, High, slot I2S 1 R" in spoken)
        // The dashboard pairs by the core's cards
        assertTrue(rule.onAllNodesWithText("Woofers L", useUnmergedTree = true).fetchSemanticsNodes().isNotEmpty())
        assertTrue(rule.onAllNodesWithText("Tweeters R", useUnmergedTree = true).fetchSemanticsNodes().isNotEmpty())
    }
}
