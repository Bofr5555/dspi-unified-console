package se.bofr.dspi.app

import android.app.Application
import androidx.compose.runtime.snapshots.Snapshot
import androidx.compose.ui.test.SemanticsNodeInteraction
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.hasClickAction
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.longClick
import androidx.compose.ui.test.onAllNodesWithContentDescription
import androidx.compose.ui.test.onAllNodesWithTag
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performTouchInput
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
import org.robolectric.annotation.Config
import se.bofr.dspi.feature.tools.ToolRoute
import se.bofr.dspi.feature.tools.ToolsScreen
import se.bofr.dspi.feature.tools.toolPageTag
import se.bofr.dspi.ui.DspiRoutes

/**
 * AN31: a visible way into Tools. The whole app (AppScaffold and its NavHost) over a real demo
 * bridge: the Tools menu button on Home reaches every tool with taps only, and a long-press on a
 * quick-strip icon opens that icon's own tool page (macOS right-click), not the list.
 * A tall window, so nothing needs scrolling: performScrollTo animates, and hangs on the paused clock.
 */
@RunWith(DspiRobolectricTestRunner::class)
@Config(sdk = [35], application = Application::class, qualifiers = "w400dp-h1600dp-port-mdpi")
class ToolsEntryAn31Test {
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
        // On Home, Compose does not go idle with the clock on auto (60 s timeout), so drive the clock
        rule.mainClock.autoAdvance = false
        rule.setContent { AppScaffold(bridge, ConnectionStrip.Demo, onExitDemo = {}, onRequestPermission = {}) }
        settle()
    }

    /**
     * Past a NavHost crossfade. A click's state write reaches composition through an apply
     * notification posted to the main looper, which the paused clock does not run; send it.
     */
    private fun settle() {
        rule.runOnUiThread { Snapshot.sendApplyNotifications() }
        rule.mainClock.advanceTimeBy(3_000)
    }

    private fun tap(node: SemanticsNodeInteraction) {
        node.assertIsDisplayed().performClick()
        settle()
    }

    // Lookups count first: a failed single-node lookup prints the whole tree, which takes minutes under Robolectric
    private fun text(text: String): SemanticsNodeInteraction {
        assertEquals("nodes with text \"$text\"", 1, rule.onAllNodesWithText(text, useUnmergedTree = true).fetchSemanticsNodes().size)
        return rule.onNodeWithText(text, useUnmergedTree = true)
    }

    /** The clickable row or button showing [text]. */
    private fun clickable(text: String): SemanticsNodeInteraction {
        val m = hasText(text) and hasClickAction()
        assertEquals("clickable nodes with text \"$text\"", 1, rule.onAllNodes(m).fetchSemanticsNodes().size)
        return rule.onNode(m)
    }

    private fun described(label: String): SemanticsNodeInteraction {
        val n = rule.onAllNodesWithContentDescription(label).fetchSemanticsNodes().size
        assertEquals("nodes described \"$label\"", 1, n)
        return rule.onNodeWithContentDescription(label)
    }

    private fun shown(tool: ToolRoute) = rule.onAllNodesWithTag(toolPageTag(tool), useUnmergedTree = true).fetchSemanticsNodes().size == 1

    private val toolsMenu get() = described("Tools menu")

    @Test
    fun tubeSubharmonicAndUpmixerAreReachableFromHomeWithTapsOnly() {
        showHome()
        toolsMenu.assertIsDisplayed()
        tap(toolsMenu)
        for (tool in listOf(ToolRoute.TUBE, ToolRoute.SUBHARM, ToolRoute.UPMIX)) {
            tap(clickable(tool.title))
            assertTrue("$tool page open", shown(tool))
            // The tool closes to the list
            tap(clickable("‹ Tools"))
            assertFalse("$tool page closed", shown(tool))
        }
    }

    @Test
    fun theMenuListsTheMacToolsMenu() {
        showHome()
        tap(toolsMenu)
        val items = listOf("Commit, Revert, Factory Reset", "Firmware Update", "Matrix Mixer") +
            ToolRoute.entries.map { it.title } +
            listOf("Signal Generator", "Spectrum Analyser", "Stats for nerbs", "Interrupt Monitor")
        items.forEach { text(it) }
        assertEquals(14, items.size)
    }

    @Test
    fun longPressOnCrossfeedOpensTheCrossfeedPageNotTheList() {
        showHome()
        described("Headphone Crossfeed").assertIsDisplayed().performTouchInput { longClick() }
        settle()
        assertTrue("crossfeed page open", shown(ToolRoute.CROSSFEED))
        assertEquals("no list", 0, rule.onAllNodesWithText("Matrix Mixer", useUnmergedTree = true).fetchSemanticsNodes().size)
        // Back returns Home, as closing the macOS window does
        tap(clickable("‹ Back"))
        assertFalse("crossfeed page closed", shown(ToolRoute.CROSSFEED))
        toolsMenu.assertIsDisplayed()
    }

    @Test
    fun everyStripLongPressNamesARealTool() {
        val ids = listOf("crossfeed", "loudness", "leveller", "psybass")
        assertEquals(
            listOf(ToolRoute.CROSSFEED, ToolRoute.LOUDNESS, ToolRoute.LEVELLER, ToolRoute.PSYBASS),
            ids.map { ToolRoute.of(DspiRoutes.tool(it)) }
        )
    }

    @Test
    fun anUnknownToolIdShowsTheList() {
        val bridge = startDemoSession(scope, "test").also { session = it }.bridge
        rule.mainClock.autoAdvance = false
        rule.setContent { ToolsScreen(bridge, onNavigate = {}, onBack = {}, open = "nope") }
        settle()
        text("Matrix Mixer")
    }
}
