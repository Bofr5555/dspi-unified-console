package se.bofr.dspi.app

import android.app.Application
import androidx.compose.runtime.snapshots.Snapshot
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.test.SemanticsMatcher
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.getBoundsInRoot
import androidx.compose.ui.test.hasAnyAncestor
import androidx.compose.ui.test.hasTestTag
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.unit.dp
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
import org.robolectric.annotation.Config

/**
 * WP AN42: below 600 dp the strip is one line: the short text (the whole of
 * it as the tooltip and for TalkBack), Undo, Redo and an overflow holding the
 * strip's actions and the device list. A tablet keeps the wide strip.
 */
@RunWith(DspiRobolectricTestRunner::class)
@Config(sdk = [35], application = Application::class)
class StripCompactWpAn42Test {
    @get:Rule val rule = createComposeRule()

    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
    private var session: DemoSession? = null
    private val rows =
        listOf(
            DeviceRow(DeviceChoice.Usb, "USB: no DSPi connected", enabled = false),
            DeviceRow(DeviceChoice.Demo, "Demo (no device)", current = true)
        )
    private var exits = 0

    @After
    fun stop() {
        rule.runOnUiThread { session?.close() }
        scope.cancel()
    }

    private fun show(strip: ConnectionStrip) {
        val bridge = startDemoSession(scope, "an42").also { session = it }.bridge
        rule.mainClock.autoAdvance = false
        rule.setContent { AppScaffold(bridge, strip, onExitDemo = { exits++ }, onRequestPermission = {}, devices = rows) }
        rule.runOnUiThread { Snapshot.sendApplyNotifications() }
        rule.mainClock.advanceTimeBy(2_000)
    }

    private val inStrip = hasAnyAncestor(hasTestTag(CONNECTION_STRIP_TAG))

    @Test
    @Config(qualifiers = "w400dp-h800dp-port-mdpi")
    fun wpAn42APhoneStripIsOneLineWithAnOverflow() {
        show(ConnectionStrip.Demo)
        val strip = rule.onNodeWithTag(CONNECTION_STRIP_TAG).getBoundsInRoot()
        assertTrue("one line: ${strip.bottom - strip.top}", strip.bottom - strip.top <= 56.dp)
        rule.onNode(hasText("Demo: a simulated DSPi") and inStrip).assertIsDisplayed()
        // the whole text for TalkBack
        rule.onNode(
            SemanticsMatcher.expectValue(SemanticsProperties.ContentDescription, listOf(ConnectionStrip.Demo.text)),
            useUnmergedTree = true
        )
            .assertExists()
        assertEquals(0, rule.onAllNodes(hasText("Exit demo")).fetchSemanticsNodes().size)
        rule.onNodeWithTag(DEVICE_LIST_TAG).assertDoesNotExist()
        rule.onNodeWithTag(UNDO_BUTTON_TAG).assertIsDisplayed()
        rule.onNodeWithTag(REDO_BUTTON_TAG).assertIsDisplayed()

        rule.onNodeWithTag(STRIP_MENU_TAG).performClick()
        rule.runOnUiThread { Snapshot.sendApplyNotifications() }
        rule.mainClock.advanceTimeBy(3_000)
        rule.onNodeWithText("✓ Demo (no device)").assertIsDisplayed()
        rule.onNodeWithText("Exit demo").assertIsDisplayed().performClick()
        assertEquals(1, exits)
    }

    @Test
    @Config(qualifiers = "w400dp-h800dp-port-mdpi")
    fun wpAn42TheUsbTextsAreShortOnAPhoneToo() {
        show(connectionStrip(AppMode.DEVICE, se.bofr.dspi.usb.UsbSessionState.NoDevice))
        rule.onNode(hasText("No DSPi connected") and inStrip).assertIsDisplayed()
        val strip = rule.onNodeWithTag(CONNECTION_STRIP_TAG).getBoundsInRoot()
        assertTrue(strip.bottom - strip.top <= 56.dp)
    }

    @Test
    @Config(qualifiers = "w800dp-h1280dp-port-mdpi")
    fun wpAn42ATabletKeepsTheWideStrip() {
        show(ConnectionStrip.Demo)
        rule.onNode(hasText(ConnectionStrip.Demo.text) and inStrip).assertIsDisplayed()
        rule.onNode(hasText("Exit demo") and inStrip).assertIsDisplayed()
        rule.onNodeWithTag(DEVICE_LIST_TAG).assertIsDisplayed()
        rule.onNodeWithTag(STRIP_MENU_TAG).assertDoesNotExist()
        rule.onNodeWithTag(UNDO_BUTTON_TAG).assertIsDisplayed()
    }
}
