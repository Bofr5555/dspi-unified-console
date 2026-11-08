package se.bofr.dspi.app

import android.app.Application
import androidx.compose.ui.test.getBoundsInRoot
import androidx.compose.ui.test.hasAnyAncestor
import androidx.compose.ui.test.hasTestTag
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithTag
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.annotation.Config
import se.bofr.dspi.bridge.BridgeEvent
import se.bofr.dspi.bridge.DspiBridge
import se.bofr.dspi.core.DspiException

/**
 * WP AN41: a write the core refuses (a delay past the platform's line) is shown
 * in the core's words over the screen, the value stays the device's, and the
 * layout does not move. AppScaffold over the real core's demo device, phone size.
 */
@RunWith(DspiRobolectricTestRunner::class)
@Config(sdk = [35], application = Application::class, qualifiers = "w400dp-h800dp-port-mdpi")
class RefusalNoticeAn41Test {
    @get:Rule val rule = createComposeRule()

    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)

    private var session: DemoSession? = null

    @After
    fun stop() {
        rule.runOnUiThread { session?.close() }
        scope.cancel()
    }

    private fun show(): DspiBridge {
        val bridge = startDemoSession(scope, "test").also { session = it }.bridge
        rule.mainClock.autoAdvance = false
        rule.setContent { AppScaffold(bridge, ConnectionStrip.Demo, onExitDemo = {}, onRequestPermission = {}) }
        rule.mainClock.advanceTimeBy(3_000)
        return bridge
    }

    private fun shown(text: String): Boolean {
        val notice = hasText(text) and hasAnyAncestor(hasTestTag(REFUSAL_NOTICE_TAG))
        return rule.onAllNodes(notice, useUnmergedTree = true).fetchSemanticsNodes().isNotEmpty()
    }

    /** The bridge's FFI work runs on its own thread: step the clock until [done] or give up. */
    private fun pumpUntil(done: () -> Boolean): Boolean {
        repeat(200) {
            rule.mainClock.advanceTimeBy(50)
            if (done()) return true
            Thread.sleep(10)
        }
        return false
    }

    @Test
    fun wpAn41ADelayPastTheLineShowsTheCoresWordsAndTheValueStays() {
        val bridge = show()
        val max = bridge.maxDelayMs
        val before = bridge.outputDelayMS(0)
        val strip = rule.onNodeWithTag(CONNECTION_STRIP_TAG, useUnmergedTree = true).getBoundsInRoot()
        val words = "out.delay: 50 ms is out of range; this device (${bridge.platformName}) holds at most ${max.toInt()} ms"
        assertTrue(max < 50f)
        rule.runOnUiThread { bridge.setOutputDelay(0, 50f) }
        assertTrue("the core's words are shown", pumpUntil { shown(words) })
        assertEquals(before, bridge.outputDelayMS(0))
        // an overlay: the strip (and so everything under it) has not moved
        assertEquals(strip, rule.onNodeWithTag(CONNECTION_STRIP_TAG, useUnmergedTree = true).getBoundsInRoot())
        // gone again after a while
        rule.mainClock.advanceTimeBy(10_000)
        assertTrue(!shown(words))
    }

    @Test
    fun wpAn41OnlyInvalidArgumentsAreNotices() {
        val refused = BridgeEvent.CommandFailed("setOutputDelay", DspiException.InvalidArgument("out.delay: too long"))
        assertEquals("out.delay: too long", refusalText(refused))
        assertNull(refusalText(BridgeEvent.CommandFailed("setOutputDelay", DspiException.Locked())))
        assertNull(refusalText(BridgeEvent.PreviewChanged))
    }
}
