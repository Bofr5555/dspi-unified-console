package se.bofr.dspi.app

import android.app.Application
import android.os.Looper
import androidx.activity.ComponentActivity
import androidx.compose.runtime.snapshots.Snapshot
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.input.key.Key
import androidx.compose.ui.semantics.SemanticsActions
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.SemanticsMatcher
import androidx.compose.ui.test.assertIsEnabled
import androidx.compose.ui.test.assertIsNotEnabled
import androidx.compose.ui.test.hasAnyAncestor
import androidx.compose.ui.test.hasContentDescription
import androidx.compose.ui.test.hasTestTag
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.isFocused
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onFirst
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performKeyInput
import androidx.compose.ui.test.performTouchInput
import androidx.compose.ui.test.pressKey
import androidx.compose.ui.test.withKeyDown
import androidx.lifecycle.Lifecycle
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
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config
import se.bofr.dspi.bridge.DspiBridge

/**
 * WP AN42: Undo and Redo in the strip at the top of the app, over the real
 * core's demo device: off with nothing to step, on after a change, a press
 * steps and says what it did in the snackbar, Ctrl+Z / Ctrl+Shift+Z do the
 * same, a viewer gets both off; and a slider drag's mark ends when the app
 * stops mid-drag.
 */
@OptIn(ExperimentalTestApi::class)
@RunWith(DspiRobolectricTestRunner::class)
@Config(sdk = [35], application = Application::class, qualifiers = "w400dp-h1600dp-port-mdpi")
class UndoBarWpAn42Test {
    @get:Rule val rule = createAndroidComposeRule<ComponentActivity>()

    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)

    private var session: DemoSession? = null

    /** Every drag mark the app sent to the bridge. */
    private val marks = mutableListOf<Boolean>()

    @After
    fun stop() {
        rule.runOnUiThread { session?.close() }
        scope.cancel()
    }

    private fun show(): DspiBridge {
        val real = startDemoSession(scope, "an42").also { session = it }.bridge
        val bridge =
            object : DspiBridge by real {
                override fun setDragging(dragging: Boolean) {
                    marks += dragging
                    real.setDragging(dragging)
                }
            }
        rule.mainClock.autoAdvance = false
        rule.setContent { AppScaffold(bridge, ConnectionStrip.Demo, onExitDemo = {}, onRequestPermission = {}) }
        rule.mainClock.advanceTimeBy(3_000)
        pumpUntil("connected") { real.status.value.connected && real.writesAllowed.value }
        return real
    }

    /** The bridge posts to the main looper and the core's thread runs in real time (as AN41). */
    private fun pumpUntil(what: String, until: () -> Boolean) {
        val end = System.currentTimeMillis() + 15_000
        while (!until()) {
            check(System.currentTimeMillis() < end) { "timed out: $what" }
            shadowOf(Looper.getMainLooper()).idle()
            rule.runOnUiThread { Snapshot.sendApplyNotifications() }
            rule.mainClock.advanceTimeBy(50)
            Thread.sleep(10)
        }
    }

    private fun said(text: String): Boolean {
        val notice = hasText(text) and hasAnyAncestor(hasTestTag(REFUSAL_NOTICE_TAG))
        return rule.onAllNodes(notice, useUnmergedTree = true).fetchSemanticsNodes().isNotEmpty()
    }

    private fun enabled(tag: String): Boolean = runCatching { rule.onNodeWithTag(tag).assertIsEnabled() }.isSuccess

    @Test
    fun wpAn42TheButtonsStepAndSayWhatTheyDid() {
        val bridge = show()
        rule.onNodeWithTag(UNDO_BUTTON_TAG).assertIsNotEnabled()
        rule.onNodeWithTag(REDO_BUTTON_TAG).assertIsNotEnabled()
        val before = bridge.userMute
        rule.runOnUiThread { bridge.setUserMute(!before) }
        pumpUntil("undo on") { enabled(UNDO_BUTTON_TAG) }
        rule.onNodeWithTag(REDO_BUTTON_TAG).assertIsNotEnabled()

        rule.onNodeWithTag(UNDO_BUTTON_TAG).performClick()
        pumpUntil("undone, and said") { bridge.userMute == before && said("Undone: Mute") }
        pumpUntil("redo on") { enabled(REDO_BUTTON_TAG) && !enabled(UNDO_BUTTON_TAG) }

        rule.onNodeWithTag(REDO_BUTTON_TAG).performClick()
        pumpUntil("redone, and said") { bridge.userMute == !before && said("Redone: Mute") }
    }

    @Test
    fun wpAn42CtrlZAndCtrlShiftZStepToo() {
        val bridge = show()
        val before = bridge.userMute
        rule.runOnUiThread { bridge.setUserMute(!before) }
        pumpUntil("undo on") { enabled(UNDO_BUTTON_TAG) }
        rule.onNode(isFocused()).performKeyInput { withKeyDown(Key.CtrlLeft) { pressKey(Key.Z) } }
        pumpUntil("Ctrl+Z undid") { bridge.userMute == before && enabled(REDO_BUTTON_TAG) }
        rule.onNode(isFocused()).performKeyInput { withKeyDown(Key.CtrlLeft) { withKeyDown(Key.ShiftLeft) { pressKey(Key.Z) } } }
        pumpUntil("Ctrl+Shift+Z redid") { bridge.userMute == !before }
    }

    @Test
    fun wpAn42AViewerGetsBothOffAndTheKeysDoNothing() {
        val bridge = show()
        val before = bridge.userMute
        rule.runOnUiThread { bridge.setUserMute(!before) }
        pumpUntil("undo on") { enabled(UNDO_BUTTON_TAG) }
        rule.runOnUiThread { bridge.setRemoteReadOnly(true) }
        pumpUntil("undo off for a viewer") { !enabled(UNDO_BUTTON_TAG) }
        rule.onNodeWithTag(REDO_BUTTON_TAG).assertIsNotEnabled()
        rule.onNode(isFocused()).performKeyInput { withKeyDown(Key.CtrlLeft) { pressKey(Key.Z) } }
        rule.mainClock.advanceTimeBy(500)
        shadowOf(Looper.getMainLooper()).idle()
        assertEquals("nothing undone", !before, bridge.userMute)
        rule.runOnUiThread { bridge.setRemoteReadOnly(false) }
        pumpUntil("undo back") { enabled(UNDO_BUTTON_TAG) }
    }

    @Test
    fun wpAn42ADragHeldWhenTheAppStopsIsEnded() {
        show()
        // Home's volume slider (User or Master, as the volume mode has it)
        val slider =
            rule.onAllNodes(
                hasContentDescription("Volume", substring = true) and SemanticsMatcher.keyIsDefined(SemanticsActions.SetProgress),
                useUnmergedTree = true
            ).onFirst()
        slider.performTouchInput {
            down(center)
            repeat(6) { moveBy(Offset(-15f, 0f)) }
        }
        rule.mainClock.advanceTimeBy(100)
        assertEquals("the grab marks the drag", listOf(true), marks)
        rule.activityRule.scenario.moveToState(Lifecycle.State.CREATED)
        shadowOf(Looper.getMainLooper()).idle()
        assertEquals("stopping ends it", listOf(true, false), marks)
        assertTrue(marks.last() == false)
    }
}
