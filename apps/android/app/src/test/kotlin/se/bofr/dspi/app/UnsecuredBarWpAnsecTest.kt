package se.bofr.dspi.app

import android.app.Application
import android.graphics.Bitmap
import android.graphics.Canvas
import android.os.Looper
import androidx.activity.ComponentActivity
import androidx.compose.material3.ColorScheme
import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.runtime.snapshots.Snapshot
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.luminance
import androidx.compose.ui.graphics.toArgb
import androidx.compose.ui.platform.ViewRootForTest
import androidx.compose.ui.test.SemanticsNodeInteraction
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.hasAnyAncestor
import androidx.compose.ui.test.hasTestTag
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.isDialog
import androidx.compose.ui.test.isPopup
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.unit.dp
import java.io.File
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.flow.MutableStateFlow
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotEquals
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode
import se.bofr.dspi.bridge.DspiBridgeImpl
import se.bofr.dspi.core.LinkDevice
import se.bofr.dspi.fake.FakeDspiCore
import se.bofr.dspi.feature.settings.NetworkingSettingsTab
import se.bofr.dspi.link.HubSecurity
import se.bofr.dspi.link.LinkHub
import se.bofr.dspi.link.LinkSessionState
import se.bofr.dspi.ui.DspiBarColors
import se.bofr.dspi.ui.DspiTheme
import se.bofr.dspi.ui.link.AcknowledgedHubRow
import se.bofr.dspi.ui.link.LocalNetworking
import se.bofr.dspi.ui.link.NetworkHubRow
import se.bofr.dspi.ui.link.NetworkingController
import se.bofr.dspi.ui.link.NetworkingUiState
import se.bofr.dspi.ui.link.PairedHubRow
import se.bofr.dspi.ui.settings.AppSettings
import se.bofr.dspi.ui.settings.InMemoryAppSettingsStore
import se.bofr.dspi.ui.settings.LocalAppSettings
import se.bofr.dspi.usb.UsbSessionState

/**
 * WP ANSEC (owner 2026-11-21): the unsecured bar is amber, not red, with AA contrast in both
 * themes; the info dialog (and a phone's overflow) acknowledges it, after which the bar has the
 * normal colours but still says "(unsecured)" and keeps its info button; Settings › Networking
 * lists the acknowledged hubs. With `-Pdspi.shotsDir=<dir>` the screenshots are written there.
 */
@RunWith(DspiRobolectricTestRunner::class)
@Config(sdk = [35], application = Application::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
class UnsecuredBarWpAnsecTest {
    @get:Rule val rule = createAndroidComposeRule<ComponentActivity>()

    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
    private val bridge = DspiBridgeImpl(FakeDspiCore.seeded(), scope, ffi = Dispatchers.Main, main = Dispatchers.Main)
    private val store = InMemoryAppSettingsStore(AppSettings())
    private val networking = RecordingNetworking()
    private val dir = System.getProperty("dspi.shotsDir")

    private val hub = LinkHub("6f1a", "Studio Mac", "console", "pin", 1, emptyList(), "192.168.1.20", 11915)
    private val dev = LinkDevice(0, "0123456789ABCDEF", "Living room", "1.1.6", 0, 1, "online", "usb", 0)
    private val connected = LinkSessionState.Connected(hub, dev, "control")
    private val plain = HubSecurity("6f1a", encrypted = false, hubKey = "")
    private val words = UnsecuredWording.CORE

    private var security by mutableStateOf(plain)

    @After fun stop() = scope.cancel()

    private fun settle() {
        repeat(20) {
            shadowOf(Looper.getMainLooper()).idle()
            rule.runOnUiThread { Snapshot.sendApplyNotifications() }
            rule.mainClock.advanceTimeBy(50)
        }
    }

    private fun show(dark: Boolean) {
        rule.mainClock.autoAdvance = false
        rule.setContent {
            CompositionLocalProvider(LocalAppSettings provides store, LocalNetworking provides networking) {
                DspiTheme(darkTheme = dark) {
                    AppScaffold(
                        bridge,
                        connectionStrip(AppMode.REMOTE, UsbSessionState.NoDevice, connected, security = security),
                        onExitDemo = {},
                        onRequestPermission = {}
                    )
                }
            }
        }
        settle()
    }

    private fun shoot(name: String) {
        if (dir.isNullOrEmpty()) return
        val out = File(dir).apply { mkdirs() }.resolve("$name.png")
        rule.runOnUiThread { out.outputStream().use { screen().compress(Bitmap.CompressFormat.PNG, 100, it) } }
    }

    private fun screen(): Bitmap {
        val root = rule.activity.window.decorView
        return Bitmap.createBitmap(root.width, root.height, Bitmap.Config.ARGB_8888).also { root.draw(Canvas(it)) }
    }

    /** A dialog or popup is a window of its own: drawn over the screen where it sits (as ANLINK's). */
    private fun shootWindow(name: String, node: SemanticsNodeInteraction) {
        if (dir.isNullOrEmpty()) return
        val window = (node.fetchSemanticsNode().root as ViewRootForTest).view.rootView
        val out = File(dir).apply { mkdirs() }.resolve("$name.png")
        rule.runOnUiThread {
            val bitmap = screen()
            val canvas = Canvas(bitmap)
            val at = IntArray(2).also(window::getLocationOnScreen)
            canvas.translate(at[0].toFloat(), at[1].toFloat())
            window.draw(canvas)
            out.outputStream().use { bitmap.compress(Bitmap.CompressFormat.PNG, 100, it) }
        }
    }

    /** The strip's fill as drawn: a pixel in its start padding. */
    private fun stripFill(): Int {
        val node = rule.onNodeWithTag(CONNECTION_STRIP_TAG).fetchSemanticsNode()
        val at = node.positionInWindow
        var px = 0
        rule.runOnUiThread { px = screen().getPixel(at.x.toInt() + 3, (at.y + node.size.height / 2f).toInt()) }
        return px
    }

    private val inStrip = hasAnyAncestor(hasTestTag(CONNECTION_STRIP_TAG))

    /** A phone's one line keeps " (unsecured)" as its own text; a wide bar has it in the whole text. */
    private fun assertSaysUnsecured(phone: Boolean) {
        if (phone) {
            // Its whole width (" (unsecured)" is ~80dp at bodyMedium): the long name is ellipsised instead
            val suffix = rule.onNodeWithTag(UNSECURED_SUFFIX_TAG).assertIsDisplayed().fetchSemanticsNode().boundsInWindow
            val name = rule.onNodeWithContentDescription("Living room via Studio Mac (unsecured)").fetchSemanticsNode().boundsInWindow
            assertTrue("suffix $suffix", suffix.width >= with(rule.density) { 70.dp.toPx() })
            assertTrue("name $name before suffix $suffix", name.right <= suffix.left + 0.5f)
        } else {
            rule.onNode(hasText("Living room via Studio Mac (unsecured)") and inStrip).assertIsDisplayed()
        }
    }

    /** Before and after the acknowledgement, through the info dialog; [prefix] names the shots. */
    private fun acknowledgeThroughTheDialog(dark: Boolean, prefix: String) {
        show(dark)
        val warning = StripWarningColors.of(dark)
        assertEquals("amber before the acknowledgement", warning.container.toArgb(), stripFill())
        assertSaysUnsecured(prefix.startsWith("phone"))
        shoot("$prefix-unsecured")
        rule.onNodeWithTag(UNSECURED_INFO_TAG).performClick()
        settle()
        rule.onNodeWithText(words.info().reason).assertIsDisplayed()
        shootWindow("$prefix-unsecured-info", rule.onNode(isDialog()))
        rule.onNodeWithText(UNSECURED_ACK_BUTTON).performClick()
        settle()
        assertEquals(1, networking.acks)
        // The app's store answers with the acknowledged security
        security = plain.copy(acknowledged = true)
        settle()
        // -- WP ANBAR -- the acknowledged neutral, not Material's tertiary
        assertEquals("the acknowledged colours after it", DspiBarColors.of(dark).acknowledged.container.toArgb(), stripFill())
        // Never colour alone: the words and the info button stay
        assertSaysUnsecured(prefix.startsWith("phone"))
        rule.onNodeWithTag(UNSECURED_INFO_TAG).assertIsDisplayed()
        shoot("$prefix-acknowledged")
        rule.onNodeWithTag(UNSECURED_INFO_TAG).performClick()
        settle()
        rule.onNodeWithText(UNSECURED_ACK_BUTTON).assertDoesNotExist()
        rule.onNodeWithText(UNSECURED_UNACK_BUTTON).performClick()
        settle()
        assertEquals(1, networking.highlights)
    }

    @Test
    @Config(qualifiers = "w400dp-h860dp-port-xhdpi")
    fun wpAnsecPhoneLight() = acknowledgeThroughTheDialog(dark = false, "phone-light")

    @Test
    @Config(qualifiers = "w400dp-h860dp-port-xhdpi")
    fun wpAnsecPhoneDark() = acknowledgeThroughTheDialog(dark = true, "phone-dark")

    @Test
    @Config(qualifiers = "w1280dp-h800dp-land-mdpi")
    fun wpAnsecTabletLight() = acknowledgeThroughTheDialog(dark = false, "tablet-light")

    @Test
    @Config(qualifiers = "w1280dp-h800dp-land-mdpi")
    fun wpAnsecTabletDark() = acknowledgeThroughTheDialog(dark = true, "tablet-dark")

    /** A phone's overflow offers it too, and only before the acknowledgement. */
    @Test
    @Config(qualifiers = "w400dp-h860dp-port-xhdpi")
    fun wpAnsecPhoneOverflowAcknowledges() {
        show(dark = false)
        rule.onNodeWithTag(STRIP_MENU_TAG).performClick()
        settle()
        shootWindow("phone-light-overflow", rule.onNode(isPopup()))
        rule.onNodeWithText(UNSECURED_ACK_MENU).performClick()
        settle()
        assertEquals(1, networking.acks)
        security = plain.copy(acknowledged = true)
        settle()
        rule.onNodeWithTag(STRIP_MENU_TAG).performClick()
        settle()
        rule.onNodeWithText(UNSECURED_ACK_MENU).assertDoesNotExist()
    }

    private fun contrast(a: Color, b: Color): Double {
        val (hi, lo) = listOf(a.luminance().toDouble(), b.luminance().toDouble()).sortedDescending()
        return (hi + 0.05) / (lo + 0.05)
    }

    /** WCAG AA (4.5:1, text; the icon is the same colour) for the amber bar and the acknowledged one, both themes. */
    @Test
    fun wpAnsecTheColoursMeetContrastInBothThemes() {
        var light: ColorScheme? = null
        var dark: ColorScheme? = null
        rule.setContent {
            DspiTheme(darkTheme = false) { light = MaterialTheme.colorScheme }
            DspiTheme(darkTheme = true) { dark = MaterialTheme.colorScheme }
        }
        rule.waitForIdle()
        for ((name, w, s) in listOf(Triple("light", StripWarningColors.Light, light!!), Triple("dark", StripWarningColors.Dark, dark!!))) {
            assertTrue("$name warning ${contrast(w.content, w.container)}", contrast(w.content, w.container) >= 4.5)
            val ack = DspiBarColors.of(name == "dark").acknowledged // -- WP ANBAR --
            assertTrue("$name acknowledged", contrast(ack.content, ack.container) >= 4.5)
            // A warning, not the lost connection's red (its banner keeps errorContainer)
            assertNotEquals(name, s.errorContainer, w.container)
            assertNotEquals(name, ack.container, w.container)
            val r = w.container.red
            val g = w.container.green
            val b = w.container.blue
            assertTrue("$name amber: red over green over blue", r > g && g > b)
        }
    }

    /** Settings › Networking: the corrected DSPi Link footer and the acknowledged hubs with Forget. */
    private fun networkingPage(dark: Boolean, name: String) {
        networking.state.value = NetworkingUiState(
            enabled = true,
            defaultClientName = "Pixel 9",
            hubs = listOf(
                NetworkHubRow(
                    "6f1a",
                    "Studio Mac",
                    "192.168.1.20, 1 device",
                    "console",
                    open = false,
                    manual = false,
                    active = true,
                    badge = "Unencrypted"
                ),
                NetworkHubRow(
                    "a11c",
                    "Living room hub",
                    "192.168.1.21, 2 devices",
                    "console",
                    open = false,
                    manual = false,
                    active = false,
                    badge = "Verified"
                )
            ),
            paired = listOf(PairedHubRow("a11c", "Living room hub", "SHA256:K7Q2-M9XD-4TNB-0ZRA")),
            status = "Living room via Studio Mac",
            connected = true,
            acknowledged = listOf(AcknowledgedHubRow("6f1a", "Studio Mac"), AcknowledgedHubRow("9c2e", "Bridge", "192.168.1.30:11915"))
        )
        rule.mainClock.autoAdvance = false
        rule.setContent { DspiTheme(darkTheme = dark) { NetworkingSettingsTab(networking) } }
        settle()
        rule.onNodeWithText("encrypted depends on the hub", substring = true).assertExists()
        shoot(name)
    }

    @Test
    @Config(qualifiers = "w400dp-h1400dp-port-xhdpi")
    fun wpAnsecNetworkingPageLight() {
        networkingPage(dark = false, "networking-light")
        rule.onNodeWithText("Forget All").performClick()
        assertEquals(1, networking.forgotAll)
    }

    @Test
    @Config(qualifiers = "w400dp-h1400dp-port-xhdpi")
    fun wpAnsecNetworkingPageDark() = networkingPage(dark = true, "networking-dark")

    private class RecordingNetworking : NetworkingController {
        var acks = 0
        var highlights = 0
        var forgotAll = 0
        override val state = MutableStateFlow(NetworkingUiState(defaultClientName = "Test"))

        override fun setEnabled(enabled: Boolean) = Unit

        override fun setFindAutomatically(on: Boolean) = Unit

        override fun setClientName(name: String) = Unit

        override fun connect(hubId: String) = Unit

        override fun disconnect() = Unit

        override fun isValidAddress(text: String) = false

        override fun addManual(text: String) = false

        override fun removeManual(hubId: String) = Unit

        override fun forget(hubId: String) = Unit

        override fun setVisible(visible: Boolean) = Unit

        override fun acknowledgeUnsecured() {
            acks++
        }

        override fun highlightUnsecuredAgain() {
            highlights++
        }

        override fun forgetAcknowledgements() {
            forgotAll++
        }
    }
}
