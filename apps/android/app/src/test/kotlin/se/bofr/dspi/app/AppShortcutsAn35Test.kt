package se.bofr.dspi.app

import android.app.Application
import androidx.compose.runtime.snapshots.Snapshot
import androidx.compose.ui.input.key.Key
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.KeyInjectionScope
import androidx.compose.ui.test.isFocused
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onAllNodesWithTag
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.performKeyInput
import androidx.compose.ui.test.pressKey
import androidx.compose.ui.test.withKeyDown
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
import se.bofr.dspi.feature.tools.ToolRoute
import se.bofr.dspi.feature.tools.toolPageTag

/**
 * WP AN35: real key events reach AppScaffold's handler over the whole app on a
 * demo bridge: Ctrl and Meta both stand for Command, and nothing needs focusing first.
 */
@OptIn(ExperimentalTestApi::class)
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35], application = Application::class, qualifiers = "w400dp-h1600dp-port-mdpi")
class AppShortcutsAn35Test {
    @get:Rule val rule = createComposeRule()

    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)

    private var session: DemoSession? = null

    @After
    fun stop() {
        rule.runOnUiThread { session?.close() }
        scope.cancel()
    }

    private fun showHome() {
        val bridge = startDemoSession(scope, "an35").also { session = it }.bridge
        rule.mainClock.autoAdvance = false
        rule.setContent { AppScaffold(bridge, ConnectionStrip.Demo, onExitDemo = {}, onRequestPermission = {}) }
        settle()
    }

    private fun settle() {
        rule.runOnUiThread { Snapshot.sendApplyNotifications() }
        rule.mainClock.advanceTimeBy(3_000)
    }

    private fun keys(block: KeyInjectionScope.() -> Unit) {
        rule.onNode(isFocused()).performKeyInput(block)
        settle()
    }

    private fun shown(tool: ToolRoute) = rule.onAllNodesWithTag(toolPageTag(tool), useUnmergedTree = true).fetchSemanticsNodes().size == 1

    private fun texts(text: String) = rule.onAllNodesWithText(text, useUnmergedTree = true).fetchSemanticsNodes().size

    @Test
    fun an35ShortcutsOpenTheirPagesFromHome() {
        showHome()
        assertEquals("not on the matrix yet", 0, texts("Matrix Mixer"))
        keys { withKeyDown(Key.CtrlLeft) { withKeyDown(Key.ShiftLeft) { pressKey(Key.X) } } }
        assertTrue("Ctrl+Shift+X: crossfeed", shown(ToolRoute.CROSSFEED))
        keys { withKeyDown(Key.MetaLeft) { withKeyDown(Key.ShiftLeft) { pressKey(Key.L) } } }
        assertTrue("Meta+Shift+L: loudness", shown(ToolRoute.LOUDNESS))
        keys { withKeyDown(Key.CtrlLeft) { withKeyDown(Key.ShiftLeft) { pressKey(Key.M) } } }
        assertEquals("Ctrl+Shift+M: the matrix", 1, texts("Matrix Mixer"))
    }
}
