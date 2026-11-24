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
 * BUG1: the K340's output labels in their phone forms (the core's, tests/fixtures/preset/
 * k340-bug1.dspipreset; review c86 F3: "R · High" beside the OUTn pill) fit the Home rows of a
 * 360 dp phone whole at font scale 1.0 and 1.3, the dashboard cards' headings too ("Woofers L" |
 * "R", review c86 F2), and so do the default demo's and longer routings' forms; the pills tell
 * TalkBack the label and the slot. Native graphics, so text is measured by the font; the fake core
 * with the core's labels, as SpectrumSegmentsAn32Test explains why.
 */
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@RunWith(DspiRobolectricTestRunner::class)
@Config(sdk = [35], application = Application::class, qualifiers = "w360dp-h2000dp-port-mdpi")
class OutputLabelsFitBug1Test {
    @get:Rule val rule = createComposeRule()

    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
    private var session: DemoSession? = null

    private fun label(o: Int, long: String, short: String, slot: String) = OutputLabel(o, long, short, slot, false, "", "")

    private val k340 =
        listOf(
            label(0, "Out 1 · R · Low", "R · Low", "I2S 1 L"),
            label(1, "Out 2 · R · High", "R · High", "I2S 1 R"),
            label(2, "Out 3 · L · Low", "L · Low", "I2S 2 L"),
            label(3, "Out 4 · L · High", "L · High", "I2S 2 R")
        )
    private val k340Cards =
        listOf(
            OutputCard(2, 0, "Woofers", "Woofers L", "Woofers R", "Woofers L", "R"),
            OutputCard(3, 1, "Tweeters", "Tweeters L", "Tweeters R", "Tweeters L", "R")
        )

    /** The default demo's forms, and the longest a routing gives (both inputs, another input, nothing).
     *  -- WP INTD -- (REVD W12) the core says no "· None" and no "· Full" any more: "Out 3", "R". */
    private val longer =
        listOf(
            label(0, "Out 1 · L+R · High", "L+R · High", "SPDIF 1 L"),
            label(1, "Out 2 · USB 3 · High", "USB 3 · High", "SPDIF 1 R"),
            label(2, "Out 3", "Out 3", "SPDIF 2 L"),
            label(3, "Out 4 · R", "R", "SPDIF 2 R")
        )
    private val longerCards =
        listOf(
            OutputCard(0, 1, "", "Out 1 · L+R · High", "Out 2 · USB 3 · High", "L+R · High", "USB 3 · High"),
            OutputCard(2, 3, "", "Out 3", "Out 4 · R", "Out 3", "R")
        )

    /** The demo fake with the core's labels and cards (outputs 1-4 on, as the demo has them). */
    private inner class LabelledCore(val labels: List<OutputLabel>, val cards: List<OutputCard>) : FakeDspiCore() {
        override fun outputLabels(): List<OutputLabel> = labels + super.outputLabels().drop(4)

        override fun outputCards(): List<OutputCard> = cards
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

    private fun showHome(scale: Float, core: FakeDspiCore, first: String) {
        val bridge = startDemoSession(scope, "bug1", core = core).also { session = it }.bridge
        rule.mainClock.autoAdvance = false
        var fontScale by mutableFloatStateOf(scale)
        rule.setContent {
            val d = LocalDensity.current
            CompositionLocalProvider(LocalDensity provides Density(d.density, fontScale), LocalDemoMode provides true) {
                HomeScreen(bridge, {}, {})
            }
        }
        val end = System.currentTimeMillis() + 15_000
        while (rule.onAllNodesWithText(first, useUnmergedTree = true).fetchSemanticsNodes().isEmpty()) {
            check(System.currentTimeMillis() < end) { "timed out: the output rows" }
            shadowOf(Looper.getMainLooper()).idle()
            settle()
            Thread.sleep(10)
        }
        settle()
    }

    /** Every text node showing [text] is whole: one line, nothing cut off. */
    private fun assertWhole(text: String, scale: Float) {
        val nodes = rule.onAllNodesWithText(text, useUnmergedTree = true).fetchSemanticsNodes()
            .filter { SemanticsActions.GetTextLayoutResult in it.config && it.boundsInRoot.width > 0f }
        assertTrue("\"$text\" at font scale $scale is shown", nodes.isNotEmpty())
        for (n in nodes) {
            val out = mutableListOf<TextLayoutResult>()
            n.config[SemanticsActions.GetTextLayoutResult].action!!.invoke(out)
            val t = out.single()
            val what = "\"$text\" at font scale $scale (${t.getLineRight(0) - t.getLineLeft(0)} of ${t.size.width} px)"
            assertFalse("$what: not cut", t.isLineEllipsized(0))
            assertEquals("$what: one line", 1, t.lineCount)
        }
    }

    private fun assertFits(scale: Float, labels: List<OutputLabel>, cards: List<OutputCard>) {
        showHome(scale, LabelledCore(labels, cards), labels.first().shortLabel)
        for (l in labels) assertWhole(l.shortLabel, scale)
        for (c in cards) {
            assertWhole(c.leftShortName, scale)
            assertWhole(c.rightShortName, scale)
        }
    }

    @Test
    fun bug1K340FitsA360DpPhoneAtFontScale10() {
        assertFits(1.0f, k340, k340Cards)
        // The pill's spoken name: descriptor, label without the dots, slot
        val pills = rule.onAllNodesWithText("OUT2", useUnmergedTree = false).fetchSemanticsNodes()
        val spoken = pills.flatMap { it.config.getOrElse(SemanticsProperties.ContentDescription) { emptyList() } }
        assertTrue("$spoken", "OUT2, Out 2, R, High, slot I2S 1 R" in spoken)
    }

    @Test
    fun bug1K340FitsA360DpPhoneAtFontScale13() = assertFits(1.3f, k340, k340Cards)

    @Test
    fun bug1LongerFormsFitA360DpPhoneAtFontScale10() = assertFits(1.0f, longer, longerCards)

    @Test
    fun bug1LongerFormsFitA360DpPhoneAtFontScale13() = assertFits(1.3f, longer, longerCards)
}
