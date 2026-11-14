package se.bofr.dspi.feature.home.system

import android.app.Application
import android.os.Looper
import androidx.activity.ComponentActivity
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.material3.Surface
import androidx.compose.runtime.snapshots.Snapshot
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.SemanticsActions
import androidx.compose.ui.test.assertCountEquals
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performSemanticsAction
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.launch
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode
import se.bofr.dspi.bridge.DspiBridgeImpl
import se.bofr.dspi.core.DspiConstants
import se.bofr.dspi.fake.FakeDspiCore
import se.bofr.dspi.ui.DspiTheme

/**
 * WP AUSB1: the demo hub's USB-direct units (Rear L/R, Height), as the core gives them:
 * the System screen shows their state, USB-direct delay and range and the statistics
 * line (host and VCXO ppm, phase error, gaps, missed and late samples), the summary
 * counts them apart, a recall says the bus frame they took it at, and aligning writes
 * their mode and delay.
 */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35], application = Application::class, qualifiers = "w400dp-h2400dp-port-mdpi")
@GraphicsMode(GraphicsMode.Mode.NATIVE)
class SystemScreenWpAusb1Test {
    @get:Rule val rule = createAndroidComposeRule<ComponentActivity>()

    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main)
    private val core = FakeDspiCore()
    private val bridge = DspiBridgeImpl(core, scope, ffi = Dispatchers.Main, main = Dispatchers.Main)
    private val model = SystemScreenModel(bridge, scope)

    @After fun stop() = scope.cancel()

    private fun settle(rounds: Int = 20) {
        repeat(rounds) {
            shadowOf(Looper.getMainLooper()).idle()
            rule.runOnUiThread { Snapshot.sendApplyNotifications() }
            rule.mainClock.advanceTimeBy(50)
        }
    }

    @Test
    fun wpAusb1UsbDirectUnitsAreShownWithTheirStatistics() {
        rule.mainClock.autoAdvance = false
        scope.launch { bridge.attachDevice(42, core.serial, DspiConstants.VENDOR_ID) }
        settle()
        core.alinkSystem.onHub = true
        core.alinkSystem.usbDirect = true
        scope.launch { model.refresh() }
        settle()
        val view = model.state.value.view!!
        val rear = view.units.first { it.id == "R" }
        assertEquals("usb_direct", rear.transport)
        assertEquals("phase_locked", rear.state)
        assertTrue(rear.pending)
        assertEquals(2L, view.usbUnitCount)
        rule.setContent { DspiTheme(darkTheme = true) { Surface(Modifier.fillMaxSize()) { SystemView(model) {} } } }
        settle()
        rule.onNodeWithContentDescription(
            "Status: Audio link: 4 of 4 locked, USB-direct 2 of 2 locked, audio link delay to write on 6"
        ).assertIsDisplayed()
        rule.onNodeWithContentDescription("Rear L/R, USB-direct, Phase-locked (USB), audio link delay pending").assertIsDisplayed()
        // Both units: the same delay against the system delay, each its statistics line
        rule.onAllNodesWithText("USB-direct delay 266 samples (5.54 ms), range 170–399, phase lock").assertCountEquals(2)
        rule.onAllNodesWithText("host -11.5 ppm · VCXO -11.5 ppm", substring = true).assertCountEquals(2)
        rule.onNodeWithText("stream 144 samples early", substring = true).assertIsDisplayed()

        rule.onNodeWithContentDescription("Recall Evening").performClick()
        settle()
        rule.onNodeWithText("Evening recalled: at USB frame", substring = true).assertIsDisplayed()
        assertTrue(model.state.value.lastRecall!!.message.contains("on Rear L/R, Height; at audio link frame"))

        rule.onNodeWithContentDescription("Clock and audio links, folded").performClick()
        settle()
        rule.onNodeWithText("Align audio link delays…").performSemanticsAction(SemanticsActions.OnClick)
        settle()
        rule.onNodeWithText("Align").performClick()
        settle()
        assertFalse(model.state.value.view!!.units.first { it.id == "R" }.pending)
        assertTrue(model.state.value.actionFailed.not())
    }
}
