package se.bofr.dspi.app

import android.app.Application
import androidx.compose.runtime.snapshots.Snapshot
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.hasClickAction
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.junit4.createComposeRule
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
import org.robolectric.annotation.Config

/**
 * -- WP UB12 -- (UXP B12) the demo strip offers "Reset demo" beside "Exit demo", over a real demo
 * bridge (the core's tuned example), and the button is the app's reset. A wide window: the strip's
 * actions are buttons there, not the overflow's.
 */
@RunWith(DspiRobolectricTestRunner::class)
@Config(sdk = [35], application = Application::class, qualifiers = "w800dp-h600dp-land-mdpi")
class DemoResetUb12Test {
    @get:Rule val rule = createComposeRule()

    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)

    private var session: DemoSession? = null

    @After
    fun stop() {
        rule.runOnUiThread { session?.close() }
        scope.cancel()
    }

    @Test
    fun theDemoStripResetsTheDemo() {
        val bridge = startDemoSession(scope, "test").also { session = it }.bridge
        var resets = 0
        var exits = 0
        rule.mainClock.autoAdvance = false
        rule.setContent {
            AppScaffold(bridge, ConnectionStrip.Demo, onExitDemo = { exits++ }, onRequestPermission = {}, onResetDemo = { resets++ })
        }
        rule.runOnUiThread { Snapshot.sendApplyNotifications() }
        rule.mainClock.advanceTimeBy(3_000)
        val m = hasText(RESET_DEMO) and hasClickAction()
        assertEquals("one Reset demo", 1, rule.onAllNodes(m).fetchSemanticsNodes().size)
        rule.onNode(m).assertIsDisplayed().performClick()
        rule.runOnUiThread { Snapshot.sendApplyNotifications() }
        rule.mainClock.advanceTimeBy(1_000)
        assertEquals(1, resets)
        assertEquals(0, exits)
    }
}
