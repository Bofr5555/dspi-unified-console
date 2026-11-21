package se.bofr.dspi.app

import android.app.Application
import android.content.Intent
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.annotation.Config
import se.bofr.dspi.core.DspiException
import se.bofr.dspi.core.LinkDevice
import se.bofr.dspi.core.LinkEncryption
import se.bofr.dspi.core.LinkPairLink
import se.bofr.dspi.core.NoticeKind
import se.bofr.dspi.link.HubSecurity
import se.bofr.dspi.link.LinkHub
import se.bofr.dspi.link.LinkSessionState
import se.bofr.dspi.link.REPAIR_CODES
import se.bofr.dspi.usb.UsbSessionState

/**
 * WP SECAN: the owner's rule (2026-11-15) on the real core's words: an unencrypted hub
 * connection gives the bar " (unsecured)" and an info button; an encrypted one does not.
 * -- WP ANSEC -- (owner 2026-11-21) the bar is amber, not red, until acknowledged
 * ([UnsecuredBarWpAnsecTest]). The PIN prompt's key, the pair-again offer, and the pairing link's reading.
 */
@RunWith(DspiRobolectricTestRunner::class)
@Config(sdk = [35], application = Application::class, qualifiers = "w400dp-h800dp-port-mdpi")
class LinkSecurityUiWpSecanTest {
    @get:Rule val rule = createComposeRule()

    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
    private var session: DemoSession? = null

    private val hub = LinkHub("6f1a", "Studio Mac", "console", "pin", 1, emptyList(), "192.168.1.20", 11915)
    private val dev = LinkDevice(0, "0123456789ABCDEF", "Living room", "1.1.6", 0, 1, "online", "usb", 0)
    private val connected = LinkSessionState.Connected(hub, dev, "control")
    private val plain = HubSecurity("6f1a", encrypted = false, hubKey = "")
    private val tls = HubSecurity("6f1a", encrypted = true, hubKey = "SHA256:K7Q2-M9XD-4TNB-0ZRA")
    private val words = UnsecuredWording.CORE

    @After
    fun stop() {
        rule.runOnUiThread { session?.close() }
        scope.cancel()
    }

    private fun show(strip: ConnectionStrip) {
        val bridge = startDemoSession(scope, "test").also { session = it }.bridge
        rule.mainClock.autoAdvance = false
        rule.setContent { AppScaffold(bridge, strip, onExitDemo = {}, onRequestPermission = {}) }
        rule.mainClock.advanceTimeBy(3_000)
    }

    /** The dialog's window comes and goes on the driven clock: advance it until [done] (at most 10 s). */
    private fun pumpUntil(done: () -> Boolean) {
        repeat(100) {
            if (done()) return
            rule.mainClock.advanceTimeBy(100)
            rule.waitForIdle()
        }
        assertTrue("timed out", done())
    }

    @Test
    fun wpSecanTheSharedWordsAreTheCoresRows() {
        assertEquals(" (unsecured)", words.suffix())
        val info = words.info()
        assertEquals(UNSECURED_INFO_NOTICE_KEY, info.key)
        assertEquals(NoticeKind.INFO, info.kind)
        assertTrue(info.reason, info.reason.contains("trusted local network") && info.reason.contains("VPN"))
        assertTrue(info.reason, info.reason.contains("pairing PIN"))
        assertTrue(info.action, info.action.contains("TLS") && info.action.contains("compatibility mode"))
    }

    @Test
    fun wpSecanAnUnencryptedHubMakesTheBarSaySo() {
        val strip = connectionStrip(AppMode.REMOTE, UsbSessionState.NoDevice, connected, security = plain) as ConnectionStrip.Remote
        assertTrue(strip.unsecured)
        assertTrue("-- WP ANSEC -- in the warning colours until acknowledged", strip.warning)
        assertFalse(strip.acknowledged)
        assertEquals("Living room via Studio Mac (unsecured)", strip.text)
        assertEquals(" (unsecured)", strip.suffix)
        // Every state with a live hub connection, not a lost or failed one
        for (s in listOf(
            LinkSessionState.NeedsPin(hub, "window"),
            LinkSessionState.ChooseDevice(hub, listOf(dev)),
            LinkSessionState.WaitingForDevice(hub, "")
        )) {
            assertTrue(s.toString(), remoteStrip(s).secured(s, plain).text.endsWith(" (unsecured)"))
        }
        for (s in listOf(
            LinkSessionState.Connecting(hub),
            LinkSessionState.Failed(hub, "gone"),
            LinkSessionState.Reconnecting(hub, "", 0, 1, "Connection lost, reconnecting")
        )) {
            assertFalse(s.toString(), remoteStrip(s).secured(s, plain).unsecured)
        }
        // Another hub's security, or none known yet, says nothing
        assertFalse(remoteStrip(connected).secured(connected, plain.copy(hubId = "other")).unsecured)
        assertFalse(remoteStrip(connected).secured(connected, null).unsecured)
    }

    // -- WP SECFIX -- one rule with the web, Qt and the Terminal: not encrypted but local
    // (the core's `local`: the hub is this device's loopback) is not unsecured
    @Test
    fun wpSecfixALocalHubKeepsTheBarAsItWas() {
        val local = HubSecurity("6f1a", encrypted = false, hubKey = "", local = true)
        assertFalse(local.unsecured)
        assertTrue(plain.unsecured)
        assertFalse(tls.unsecured)
        val strip = connectionStrip(AppMode.REMOTE, UsbSessionState.NoDevice, connected, security = local) as ConnectionStrip.Remote
        assertFalse(strip.unsecured)
        assertEquals("Living room via Studio Mac", strip.text)
        assertEquals(remoteStrip(connected), strip)
    }

    @Test
    fun wpSecanAnEncryptedHubKeepsTheBarAsItWas() {
        val strip = connectionStrip(AppMode.REMOTE, UsbSessionState.NoDevice, connected, security = tls) as ConnectionStrip.Remote
        assertFalse(strip.unsecured)
        assertEquals("Living room via Studio Mac", strip.text)
        assertEquals(remoteStrip(connected), strip)
    }

    @Test
    fun wpSecanTheUnsecuredBarHasItsInfoButton() {
        show(connectionStrip(AppMode.REMOTE, UsbSessionState.NoDevice, connected, security = plain))
        // A phone's one line: the word is never ellipsised away, and TalkBack reads the whole text once
        rule.onNodeWithTag(UNSECURED_SUFFIX_TAG).assertIsDisplayed()
        rule.onNodeWithContentDescription("Living room via Studio Mac (unsecured)").assertIsDisplayed()
        rule.onNodeWithTag(UNSECURED_INFO_DIALOG_TAG).assertDoesNotExist()
        rule.onNodeWithContentDescription(UNSECURED_INFO_LABEL).assertIsDisplayed().performClick()
        pumpUntil { rule.onAllNodesWithText(words.info().reason).fetchSemanticsNodes().isNotEmpty() }
        rule.onNodeWithText(words.info().title).assertIsDisplayed()
        rule.onNodeWithText(words.info().reason).assertIsDisplayed()
        rule.onNodeWithText("OK").performClick()
        pumpUntil { rule.onAllNodesWithText(words.info().reason).fetchSemanticsNodes().isEmpty() }
    }

    @Test
    fun wpSecanAnEncryptedBarHasNoInfoButton() {
        show(connectionStrip(AppMode.REMOTE, UsbSessionState.NoDevice, connected, security = tls))
        rule.onNodeWithText("Living room via Studio Mac").assertIsDisplayed()
        rule.onNodeWithTag(UNSECURED_INFO_TAG).assertDoesNotExist()
        rule.onNodeWithTag(UNSECURED_SUFFIX_TAG).assertDoesNotExist()
    }

    @Test
    fun wpSecanThePinPromptShowsTheHubKey() {
        rule.setContent { PinPromptBody(LinkSessionState.NeedsPin(hub, "window", hubKey = tls.hubKey), LinkWording.CORE, "", {}, false) }
        rule.onNodeWithTag(PIN_HUB_KEY_TAG).assertIsDisplayed()
        rule.onNodeWithText("Hub key: ${tls.hubKey}").assertIsDisplayed()
        rule.onNodeWithText(PIN_HUB_KEY_HINT).assertIsDisplayed()
    }

    @Test
    fun wpSecanAnUnencryptedPinPromptWarns() {
        // -- WP LKFAST -- only when the session is known not to be encrypted
        val asked = LinkSessionState.NeedsPin(hub, "window", encryption = LinkEncryption.OFF)
        rule.setContent { PinPromptBody(asked, LinkWording.CORE, "", {}, false) }
        rule.onNodeWithTag(PIN_HUB_KEY_TAG).assertDoesNotExist()
        rule.onNodeWithText(PIN_UNENCRYPTED_LINE).assertIsDisplayed()
    }

    // -- WP LKFAST --
    @Test
    fun wpLkfastAPinPromptWhoseEncryptionIsNotKnownSaysNothingFalse() {
        rule.setContent { PinPromptBody(LinkSessionState.NeedsPin(hub, "window"), LinkWording.CORE, "", {}, false) }
        rule.onNodeWithText(PIN_UNENCRYPTED_LINE).assertDoesNotExist()
        rule.onNodeWithText("Whether the DSPi Link connection to this hub is encrypted shows once it answers.").assertIsDisplayed()
    }

    @Test
    fun wpLkfastAPinPromptToThisDeviceSaysSo() {
        val asked = LinkSessionState.NeedsPin(hub, "window", encryption = LinkEncryption.LOCAL)
        rule.setContent { PinPromptBody(asked, LinkWording.CORE, "", {}, false) }
        rule.onNodeWithText(PIN_UNENCRYPTED_LINE).assertDoesNotExist()
        rule.onNodeWithTag(PIN_ENCRYPTION_TAG).assertIsDisplayed()
    }

    @Test
    fun wpSecanPairingAgainIsOfferedForKeyAndTlsFailuresOnly() {
        for (code in REPAIR_CODES) {
            val f = LinkSessionState.Failed(hub, "x", code)
            assertTrue(code, offersRePair(f))
            assertTrue(code, rePairText(f).startsWith("Studio Mac "))
        }
        for (code in listOf("", "io", "timeout", "wrong_hub", "handshake_refused", "hub_busy")) {
            assertFalse(code, offersRePair(LinkSessionState.Failed(hub, "x", code)))
        }
        assertFalse(offersRePair(connected))
        val badLink = LinkSessionState.Failed(hub, "x", "hub_key_unverified", fromPairLink = true)
        assertTrue(rePairText(badLink).contains("pairing link"))
    }

    @Test
    fun wpSecanThePairAgainDialogAsks() {
        var asked = 0
        val f = LinkSessionState.Failed(hub, "x", "hub_key_changed")
        rule.setContent { RePairDialog(f, onRePair = { asked++ }, onDismiss = {}) }
        rule.onNodeWithText(rePairText(f)).assertIsDisplayed()
        rule.onNodeWithText("Pair Again").performClick()
        assertEquals(1, asked)
    }

    // -- WP PINTLS -- (owner decision 2026-11-17) after tls_required_pinned the only offer is
    // Forget Hub, and its words say it is the way back to an unencrypted pairing
    @Test
    fun wpPintlsAPinnedHubRefusingTlsOffersForgetHub() {
        var asked = 0
        val f = LinkSessionState.Failed(hub, "x", "tls_required_pinned")
        assertTrue(offersRePair(f))
        assertEquals("Forget Hub", rePairLabel(f))
        assertEquals("Pair Again", rePairLabel(LinkSessionState.Failed(hub, "x", "hub_key_changed")))
        val text = rePairText(f)
        assertTrue(text, text.contains("not even the PIN") && text.contains("without encryption"))
        assertTrue(rePairText(LinkSessionState.Failed(hub, "x", "tls_refused")).contains("encrypted connection only"))
        rule.setContent { RePairDialog(f, onRePair = { asked++ }, onDismiss = {}) }
        rule.onNodeWithText(text).assertIsDisplayed()
        rule.onNodeWithText("Pair Again").assertDoesNotExist()
        rule.onNodeWithText("Forget Hub").performClick()
        assertEquals(1, asked)
    }

    @Test
    fun wpSecanAPairingLinkIsReadByTheCoreAndOnlyOffered() {
        val link = LinkPairLink("6f1a", "192.168.1.20", 11915, "ws://192.168.1.20:11915/dspi/v1", tls.hubKey, "v2..x.t", "482913")
        var seen = ""
        val ok = readPairLink(Intent.ACTION_VIEW, "dspi://pair?h=6f1a", {
            seen = it
            link
        })!!
        assertEquals(link, ok.getOrNull())
        assertEquals("dspi://pair?h=6f1a", seen)
        assertNull("not a VIEW", readPairLink(Intent.ACTION_MAIN, "dspi://pair?h=6f1a") { link })
        assertNull("not ours", readPairLink(Intent.ACTION_VIEW, "https://example.com") { link })
        val bad = readPairLink(Intent.ACTION_VIEW, "dspi://pair?h=x") {
            throw DspiException.InvalidArgument("the pairing link has no key")
        }!!
        assertEquals("the pairing link has no key", bad.exceptionOrNull()!!.message)
        // The confirmation names the address and asks; nothing is dialled before it
        assertTrue(pairLinkText(link, linkEnabled = true).startsWith("Pair with the hub at 192.168.1.20:11915?"))
        assertTrue(pairLinkText(link, linkEnabled = false).contains("DSPi Link is turned on"))
        assertTrue(pairLinkText(link.copy(host = "fe80::1"), true).contains("[fe80::1]:11915"))
    }
}
