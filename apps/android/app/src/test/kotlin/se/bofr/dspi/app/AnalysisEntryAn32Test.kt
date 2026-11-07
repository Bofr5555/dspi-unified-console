package se.bofr.dspi.app

import android.app.Application
import androidx.compose.runtime.snapshots.Snapshot
import androidx.compose.ui.test.SemanticsMatcher
import androidx.compose.ui.test.SemanticsNodeInteraction
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.hasClickAction
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.isSelected
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onAllNodesWithContentDescription
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.performClick
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import se.bofr.dspi.feature.analysis.AnalysisRoute

/**
 * AN32: each analysis entry of the Tools menu opens its own page of the analysis screen, not the
 * first one. The whole app (AppScaffold and its NavHost) over a real demo bridge, taps only.
 * A tall window, so nothing needs scrolling: performScrollTo animates, and hangs on the paused clock
 * (as ToolsEntryAn31Test).
 */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35], application = Application::class, qualifiers = "w400dp-h800dp-port-mdpi")
class AnalysisEntryAn32Test {
    @get:Rule val rule = createComposeRule()

    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)

    private var session: DemoSession? = null

    @After
    fun stop() {
        rule.runOnUiThread { session?.close() }
        scope.cancel()
    }

    private fun showHome() {
        val bridge = startDemoSession(scope, "test").also { session = it }.bridge
        rule.mainClock.autoAdvance = false
        rule.setContent { AppScaffold(bridge, ConnectionStrip.Demo, onExitDemo = {}, onRequestPermission = {}) }
        settle()
    }

    /** Past a NavHost crossfade; a click's state write needs its apply notification sent on the paused clock. */
    private fun settle() {
        rule.runOnUiThread { Snapshot.sendApplyNotifications() }
        rule.mainClock.advanceTimeBy(3_000)
    }

    private fun tap(node: SemanticsNodeInteraction) {
        node.assertIsDisplayed().performClick()
        settle()
    }

    // Lookups count first: a failed single-node lookup prints the whole tree, which takes minutes under Robolectric
    private fun clickable(text: String): SemanticsNodeInteraction {
        val m = hasText(text) and hasClickAction()
        assertEquals("clickable nodes with text \"$text\"", 1, rule.onAllNodes(m).fetchSemanticsNodes().size)
        return rule.onNode(m)
    }

    private fun described(label: String): SemanticsNodeInteraction {
        assertEquals("nodes described \"$label\"", 1, rule.onAllNodesWithContentDescription(label).fetchSemanticsNodes().size)
        return rule.onNodeWithContentDescription(label)
    }

    /** The analysis pages whose tab is selected. */
    private fun selectedPages(): List<AnalysisRoute> = AnalysisRoute.entries.filter { r ->
        val m: SemanticsMatcher = hasText(r.title) and isSelected()
        rule.onAllNodes(m).fetchSemanticsNodes().isNotEmpty()
    }

    @Test
    @Config(qualifiers = "w400dp-h1600dp-port-mdpi")
    fun an32EachAnalysisToolOpensItsOwnPage() {
        showHome()
        tap(described("Tools menu"))
        val entries = listOf(
            "Signal Generator" to AnalysisRoute.TEST_SIGNALS,
            "Spectrum Analyser" to AnalysisRoute.SPECTRUM_ANALYSER,
            "Stats for nerbs" to AnalysisRoute.STATS,
            "Interrupt Monitor" to AnalysisRoute.INTERRUPT_MONITOR
        )
        for ((item, page) in entries) {
            tap(clickable(item))
            assertEquals("$item opens its page", listOf(page), selectedPages())
            // Back returns to the Tools list
            tap(clickable("‹ Back"))
            assertEquals("$item page closed", emptyList<AnalysisRoute>(), selectedPages())
        }
    }

    @Test
    @Config(qualifiers = "w400dp-h1600dp-port-mdpi")
    fun an32TheStatsIconOnHomeOpensTheStatsPage() {
        showHome()
        tap(described("Stats for Nerbs"))
        assertEquals(listOf(AnalysisRoute.STATS), selectedPages())
    }
}
