package se.bofr.dspi.app

import android.app.Application
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performTextInput
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
import se.bofr.dspi.core.LinkDevice
import se.bofr.dspi.core.LinkPhase
import se.bofr.dspi.link.LinkHub
import se.bofr.dspi.link.LinkSessionState
import se.bofr.dspi.ui.link.LINK_BANNER_TAG
import se.bofr.dspi.usb.UsbSessionState

/**
 * WP AN5: the Link UI for every [LinkPhase], on the real core's wording (CG10).
 * The strip and banner in AppScaffold over a demo bridge, and the PIN prompt.
 */
@RunWith(DspiRobolectricTestRunner::class)
@Config(sdk = [35], application = Application::class, qualifiers = "w400dp-h800dp-port-mdpi")
class LinkUiAn5Test {
    @get:Rule val rule = createComposeRule()

    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
    private var session: DemoSession? = null

    private val hub = LinkHub("6f1a", "Studio Mac", "console", "pin", 1, emptyList(), "192.168.1.20", 11915)
    private val dev = LinkDevice(0, "0123456789ABCDEF", "Living room", "1.1.6", 0, 1, "online", "usb", 0)
    private val words = LinkWording.CORE

    @After
    fun stop() {
        rule.runOnUiThread { session?.close() }
        scope.cancel()
    }

    /** AppScaffold showing [state]'s strip; the REMOTE actions it was asked for. */
    private fun show(state: LinkSessionState): MutableList<RemoteAction> {
        val bridge = startDemoSession(scope, "test").also { session = it }.bridge
        val actions = mutableListOf<RemoteAction>()
        // On Home, Compose does not go idle with the clock on auto, so drive the clock (as AN27's test)
        rule.mainClock.autoAdvance = false
        rule.setContent {
            AppScaffold(
                bridge,
                connectionStrip(AppMode.REMOTE, UsbSessionState.NoDevice, state),
                onExitDemo = {},
                onRequestPermission = {},
                onRemoteAction = { actions += it }
            )
        }
        rule.mainClock.advanceTimeBy(3_000)
        return actions
    }

    private fun text(phase: LinkPhase, pinSource: String = "") = words.of(phase, pinSource).text

    @Test
    fun wpAn5ReconnectingIsTheBannerWithTheWayOut() {
        val actions = show(LinkSessionState.Reconnecting(hub, "hub shutting down", 0, 2, text(LinkPhase.RECONNECTING)))
        rule.onNodeWithTag(LINK_BANNER_TAG).assertIsDisplayed()
        rule.onNodeWithText("Connection lost, reconnecting").assertIsDisplayed()
        rule.onNodeWithText("Studio Mac: hub shutting down").assertIsDisplayed()
        rule.onNodeWithTag(CONNECTION_STRIP_TAG).assertDoesNotExist()
        rule.onNodeWithText(words.of(LinkPhase.RECONNECTING, "").action!!).performClick()
        rule.onNodeWithText("Retry now").performClick()
        assertEquals(listOf(RemoteAction.DISCONNECT, RemoteAction.RETRY), actions)
    }

    @Test
    fun wpAn5NoDeviceSaysNoDspiFound() {
        show(LinkSessionState.WaitingForDevice(hub, ""))
        rule.onNodeWithText("Connected to hub, no DSPi found · Studio Mac").assertIsDisplayed()
        rule.onNodeWithTag(LINK_BANNER_TAG).assertDoesNotExist()
        // -- WP AN42 -- a phone-width strip keeps its actions behind the overflow
        rule.onNodeWithTag(STRIP_MENU_TAG).performClick()
        rule.mainClock.advanceTimeBy(3_000)
        rule.onNodeWithText("Disconnect").assertIsDisplayed()
    }

    @Test
    fun wpAn5Idle() {
        show(LinkSessionState.Idle)
        rule.onNodeWithText(text(LinkPhase.IDLE)).assertIsDisplayed()
    }

    @Test
    fun wpAn5ChooseDevice() {
        show(LinkSessionState.ChooseDevice(hub, listOf(dev, dev.copy(serial = "FEDCBA9876543210"))))
        rule.onNodeWithText("${text(LinkPhase.CHOOSE_DEVICE)} · Studio Mac").assertIsDisplayed()
    }

    @Test
    fun wpAn5Bound() {
        show(LinkSessionState.Connected(hub, dev, "admin"))
        rule.onNodeWithText("Living room via Studio Mac").assertIsDisplayed()
        rule.onNodeWithTag(LINK_BANNER_TAG).assertDoesNotExist()
    }

    @Test
    fun wpAn5ClosedAndRefusedOfferToConnectAgain() {
        // The engine words an ended session from the core's table (AN3)
        for (phase in listOf(LinkPhase.CLOSED, LinkPhase.REFUSED)) {
            val strip = remoteStrip(LinkSessionState.Failed(hub, text(phase)))
            assertEquals(text(phase), strip.text)
            assertEquals(listOf(RemoteAction.CONNECT, RemoteAction.DISCONNECT), strip.actions)
        }
        val actions = show(LinkSessionState.Failed(hub, text(LinkPhase.REFUSED)))
        rule.onNodeWithText(text(LinkPhase.REFUSED)).assertIsDisplayed()
        rule.onNodeWithTag(STRIP_MENU_TAG).performClick() // -- WP AN42 --
        rule.mainClock.advanceTimeBy(3_000)
        rule.onNodeWithText("Connect").performClick()
        assertEquals(listOf(RemoteAction.CONNECT), actions)
    }

    // The PIN prompt (NEEDS_PAIRING), by pin_source. Its body, as a Compose dialog's
    // window never lets Robolectric go idle

    private class Prompt(initial: LinkSessionState.NeedsPin) {
        var state by mutableStateOf(initial)
        var pin by mutableStateOf("")
        var tried by mutableStateOf(false)
    }

    private fun showPrompt(initial: LinkSessionState.NeedsPin): Prompt {
        val p = Prompt(initial)
        rule.setContent { PinPromptBody(p.state, words, p.pin, { p.pin = it }, p.tried) }
        return p
    }

    @Test
    fun wpAn5WindowPinWrongPinSaysGetANewOne() {
        val asked = LinkSessionState.NeedsPin(hub, "window")
        val p = showPrompt(asked)
        rule.onNodeWithText(text(LinkPhase.NEEDS_PAIRING, "window")).assertIsDisplayed()
        rule.onNodeWithText("000000").performTextInput("12a3456")
        assertEquals("only digits", "123456", p.pin)
        // Pair tapped: the hub is asked
        rule.runOnIdle {
            p.tried = true
            p.state = asked.copy(busy = true)
        }
        rule.onNodeWithTag(WRONG_PIN_HINT_TAG).assertDoesNotExist()
        rule.runOnIdle { p.state = asked.copy(error = "Pairing failed: the hub did not accept that code.", errorCode = "wrong_pin") }
        rule.onNodeWithTag(WRONG_PIN_HINT_TAG).assertIsDisplayed()
        rule.onNodeWithText(wrongPinHint("window")).assertIsDisplayed()
    }

    @Test
    fun wpAn5FixedPinWrongPinSaysCheckIt() {
        val asked = LinkSessionState.NeedsPin(hub, "fixed")
        val p = showPrompt(asked)
        rule.onNodeWithText("Enter the hub's PIN.").assertIsDisplayed()
        rule.runOnIdle {
            p.tried = true
            p.state = asked.copy(error = "Pairing failed: the hub did not accept that code.", errorCode = "wrong_pin")
        }
        rule.onNodeWithText("Check the hub's PIN and try again.").assertIsDisplayed()
    }

    @Test
    fun wpAn5ARevokedTokenIsNotAWrongPin() {
        showPrompt(LinkSessionState.NeedsPin(hub, "window", error = "Access to Studio Mac was revoked. Pair again to reconnect."))
        rule.onNodeWithText("Access to Studio Mac was revoked. Pair again to reconnect.").assertIsDisplayed()
        rule.onNodeWithTag(WRONG_PIN_HINT_TAG).assertDoesNotExist()
    }

    // -- WP AN36 --

    @Test
    fun wpAn36TooManyAttemptsIsNotAWrongPin() {
        val asked = LinkSessionState.NeedsPin(hub, "window")
        val p = showPrompt(asked)
        val wait = "Pairing failed: too many attempts. Wait a minute and try again."
        rule.runOnIdle {
            p.tried = true
            p.state = asked.copy(error = wait, errorCode = "rate_limited")
        }
        rule.onNodeWithText(wait).assertIsDisplayed()
        rule.onNodeWithTag(WRONG_PIN_HINT_TAG).assertDoesNotExist()
        // The next try is turned down for its PIN: now the hint shows
        rule.runOnIdle { p.state = asked.copy(error = "Pairing failed: the hub did not accept that code.", errorCode = "wrong_pin") }
        rule.onNodeWithTag(WRONG_PIN_HINT_TAG).assertIsDisplayed()
    }
}
