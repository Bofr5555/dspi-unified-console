package se.bofr.dspi.app

import android.app.Application
import androidx.compose.runtime.snapshots.Snapshot
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.assertIsNotEnabled
import androidx.compose.ui.test.hasAnyAncestor
import androidx.compose.ui.test.hasTestTag
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
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
import se.bofr.dspi.link.LinkHub
import se.bofr.dspi.usb.UsbIds
import se.bofr.dspi.usb.UsbSessionState

/**
 * WP AN7: the strip's "Devices" menu, with a DSPi open on USB: USB, a hub, DSPi
 * Link… and demo in one list; demo is one tap away; "DSPi Link…" opens the dialog.
 */
@RunWith(DspiRobolectricTestRunner::class)
@Config(sdk = [35], application = Application::class, qualifiers = "w400dp-h800dp-port-mdpi")
class DeviceListAn7UiTest {
    @get:Rule val rule = createComposeRule()

    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)

    private var session: DemoSession? = null

    @After
    fun stop() {
        rule.runOnUiThread { session?.close() }
        scope.cancel()
    }

    private val hub = LinkHub("6f1a", "Studio Mac", "console", "none", 1, emptyList(), "192.168.1.20", 11915)

    private fun settle() {
        rule.runOnUiThread { Snapshot.sendApplyNotifications() }
        rule.mainClock.advanceTimeBy(3_000)
    }

    @Test
    fun an7DemoIsInTheListWithADspiOpen() {
        val chosen = mutableListOf<DeviceChoice>()
        var network = 0
        val open = UsbSessionState.Open("x", UsbIds.DSPI_VID)
        val bridge = startDemoSession(scope, "test").also { session = it }.bridge
        rule.mainClock.autoAdvance = false
        rule.setContent {
            AppScaffold(
                bridge,
                connectionStrip(AppMode.DEVICE, open),
                onExitDemo = {},
                onRequestPermission = {},
                onConnectNetwork = { network++ },
                devices = deviceList(AppMode.DEVICE, open, hubs = listOf(hub), linkEnabled = true),
                onChooseDevice = { chosen += it }
            )
        }
        settle()
        val inStrip = hasAnyAncestor(hasTestTag(CONNECTION_STRIP_TAG))
        rule.onNode(hasText("DSPi on USB") and inStrip).assertIsDisplayed()

        rule.onNodeWithTag(STRIP_MENU_TAG).performClick() // -- WP AN42 -- the phone strip's overflow
        settle()
        rule.onNodeWithText("✓ DSPi on USB").assertIsDisplayed()
        rule.onNodeWithText("Studio Mac (network)").assertIsDisplayed()
        rule.onNodeWithText("DSPi Link…").assertIsDisplayed()
        rule.onNodeWithText("Demo (no device)").assertIsDisplayed().performClick()
        settle()
        assertEquals(listOf<DeviceChoice>(DeviceChoice.Demo), chosen)

        // the current one is no switch; DSPi Link… opens the dialog
        rule.onNodeWithTag(STRIP_MENU_TAG).performClick() // -- WP AN42 -- the phone strip's overflow
        settle()
        rule.onNodeWithText("✓ DSPi on USB").performClick()
        settle()
        rule.onNodeWithTag(STRIP_MENU_TAG).performClick() // -- WP AN42 -- the phone strip's overflow
        settle()
        rule.onNodeWithText("DSPi Link…").performClick()
        settle()
        assertEquals(listOf<DeviceChoice>(DeviceChoice.Demo), chosen)
        assertEquals(1, network)
    }

    @Test
    fun an7WithNothingOnUsbTheUsbRowIsDisabled() {
        val bridge = startDemoSession(scope, "test").also { session = it }.bridge
        rule.mainClock.autoAdvance = false
        rule.setContent {
            AppScaffold(
                bridge,
                connectionStrip(AppMode.DEVICE, UsbSessionState.NoDevice),
                onExitDemo = {},
                onRequestPermission = {},
                devices = deviceList(AppMode.DEVICE, UsbSessionState.NoDevice)
            )
        }
        settle()
        rule.onNodeWithTag(STRIP_MENU_TAG).performClick() // -- WP AN42 -- the phone strip's overflow
        settle()
        rule.onNodeWithText("✓ USB: no DSPi connected").assertIsNotEnabled()
        rule.onNodeWithText("Demo (no device)").assertIsDisplayed()
    }
}
