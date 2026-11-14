package se.bofr.dspi.feature.home.system

import android.app.Application
import android.os.Looper
import androidx.activity.ComponentActivity
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.material3.Surface
import androidx.compose.runtime.snapshots.Snapshot
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.SemanticsActions
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.junit4.createAndroidComposeRule
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
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode
import se.bofr.dspi.bridge.DspiBridgeImpl
import se.bofr.dspi.core.AlinkUnitUsbMode
import se.bofr.dspi.core.DspiConstants
import se.bofr.dspi.fake.FakeDspiCore
import se.bofr.dspi.ui.DspiTheme

/**
 * WP AUSB2: a USB-direct unit that can't lock (Rear L/R, its host out of range) is
 * shown as the core gives it, with the hub's alarm; in Clock and audio links an
 * Admin sets each USB-direct unit's mode and what it does when it can't lock, and
 * "Keep USB-direct units aligned", saved as one revision-guarded change.
 */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35], application = Application::class, qualifiers = "w400dp-h2400dp-port-mdpi")
@GraphicsMode(GraphicsMode.Mode.NATIVE)
class SystemScreenWpAusb2Test {
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

    private fun open() {
        rule.mainClock.autoAdvance = false
        scope.launch { bridge.attachDevice(42, core.serial, DspiConstants.VENDOR_ID) }
        settle()
        core.alinkSystem.onHub = true
        core.alinkSystem.usbDirect = true
        scope.launch { model.refresh() }
        settle()
    }

    @Test
    fun wpAusb2ALockErrorIsShownWithItsAlarm() {
        open()
        core.alinkSystem.rearLockError = true
        scope.launch { model.refresh() }
        settle()
        rule.setContent { DspiTheme(darkTheme = true) { Surface(Modifier.fillMaxSize()) { SystemView(model) {} } } }
        settle()
        rule.onNodeWithContentDescription("Rear L/R, USB-direct, Can't lock (USB): muted, audio link delay pending").assertIsDisplayed()
        rule.onNodeWithText("Can't lock: the host's rate is out of range").assertIsDisplayed()
        rule.onNodeWithText("Rear L/R: USB-direct unit Rear L/R can't lock — muted (the host's rate is out of range)").assertIsDisplayed()
        rule.onNodeWithContentDescription(
            "Status: Audio link: 4 of 4 locked, USB-direct 1 of 2 locked, audio link delay to write on 6"
        ).assertIsDisplayed()
    }

    @Test
    fun wpAusb2TheInstallerSetsTheModeTheLockErrorActionAndKeepAligned() {
        open()
        rule.setContent { DspiTheme(darkTheme = true) { Surface(Modifier.fillMaxSize()) { SystemView(model) {} } } }
        settle()
        rule.onNodeWithContentDescription("Clock and audio links, folded").performClick()
        settle()
        rule.onNodeWithContentDescription("When it can't lock, Rear L/R: Mute, chosen").assertExists()
        rule.onNodeWithContentDescription("When it can't lock, Rear L/R: Play unaligned")
            .performSemanticsAction(SemanticsActions.OnClick)
        rule.onNodeWithContentDescription("Mode, Height: Rate lock")
            .performSemanticsAction(SemanticsActions.OnClick)
        rule.onNodeWithContentDescription("Keep USB-direct units aligned")
            .performSemanticsAction(SemanticsActions.OnClick)
        settle()
        val edits = model.state.value.draft.edits()
        assertEquals(
            setOf(AlinkUnitUsbMode("R", "phase_lock", "play_unaligned"), AlinkUnitUsbMode("H", "rate_lock", null)),
            edits.usbModes.toSet()
        )
        assertEquals(true, edits.keepUsbAligned)
        rule.onNodeWithText("Save").performSemanticsAction(SemanticsActions.OnClick)
        settle()
        val view = model.state.value.view!!
        assertTrue(view.keepUsbAligned)
        assertEquals("play_unaligned", view.units.first { it.id == "R" }.usbOnLockErrorWanted)
        assertEquals("rate_lock", view.units.first { it.id == "H" }.usbModeWanted)
        assertEquals("Saved on the hub.", model.state.value.actionMessage)
    }

    @Test
    fun wpAusb2AViewerCannotChangeThem() {
        core.alinkSystem.role = "viewer"
        open()
        assertTrue(!model.setOnLockError("R", "play_unaligned"))
        assertTrue(!model.setKeepUsbAligned(true))
        assertTrue(!model.state.value.draft.dirty)
    }
}
