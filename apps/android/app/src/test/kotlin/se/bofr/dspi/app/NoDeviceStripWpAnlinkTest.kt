package se.bofr.dspi.app

import android.app.Application
import android.graphics.Bitmap
import android.graphics.Canvas
import android.os.Looper
import androidx.activity.ComponentActivity
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.runtime.snapshots.Snapshot
import androidx.compose.ui.platform.ViewRootForTest
import androidx.compose.ui.test.SemanticsNodeInteraction
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.hasAnyAncestor
import androidx.compose.ui.test.hasTestTag
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.isDialog
import androidx.compose.ui.test.isPopup
import androidx.compose.ui.test.junit4.createAndroidComposeRule
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
import kotlinx.coroutines.flow.StateFlow
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode
import se.bofr.dspi.bridge.DspiBridgeImpl
import se.bofr.dspi.fake.FakeDspiCore
import se.bofr.dspi.link.LinkHub
import se.bofr.dspi.ui.DspiTheme
import se.bofr.dspi.ui.link.LocalNetworking
import se.bofr.dspi.ui.link.NetworkingController
import se.bofr.dspi.ui.link.NetworkingUiState
import se.bofr.dspi.ui.settings.AppSettings
import se.bofr.dspi.ui.settings.InMemoryAppSettingsStore
import se.bofr.dspi.ui.settings.LocalAppSettings
import se.bofr.dspi.usb.UsbSessionState

/**
 * WP ANLINK (owner 2026-11-21): with no DSPi on USB and DSPi Link off, the main page leads to
 * DSPi Link and the demo. The strip shows "Connect over DSPi Link" (Networking, as Settings ›
 * Networking, its switch untouched) and "Try the demo"; the device list always has "DSPi Link…".
 * With `-Pdspi.shotsDir=<dir>` the screenshots are written there too (as AN42's).
 */
@RunWith(DspiRobolectricTestRunner::class)
@Config(sdk = [35], application = Application::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
class NoDeviceStripWpAnlinkTest {
    @get:Rule val rule = createAndroidComposeRule<ComponentActivity>()

    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)

    // The USB bridge with nothing on the bus: the fake core disconnected (Home's "Not connected")
    private val core = FakeDspiCore.seeded().also { it.disconnect() }
    private val bridge = DspiBridgeImpl(core, scope, ffi = Dispatchers.Main, main = Dispatchers.Main)
    private val store = InMemoryAppSettingsStore(AppSettings())
    private val networking = RecordingNetworking()
    private val chosen = mutableListOf<DeviceChoice>()
    private val dir = System.getProperty("dspi.shotsDir")

    private val hub = LinkHub("6f1a", "Studio Mac", "console", "pin", 1, emptyList(), "192.168.1.20", 11915)

    @After fun stop() = scope.cancel()

    private fun settle() {
        repeat(20) {
            shadowOf(Looper.getMainLooper()).idle()
            rule.runOnUiThread { Snapshot.sendApplyNotifications() }
            rule.mainClock.advanceTimeBy(50)
        }
    }

    /** The app as MainActivity wires it with no DSPi: DEVICE mode, DSPi Link off, Networking as the overlay. */
    private fun show(dark: Boolean = true) {
        rule.mainClock.autoAdvance = false
        rule.setContent {
            var networkingOpen by remember { mutableStateOf(false) }
            CompositionLocalProvider(LocalAppSettings provides store, LocalNetworking provides networking) {
                DspiTheme(darkTheme = dark) {
                    AppScaffold(
                        bridge,
                        connectionStrip(AppMode.DEVICE, UsbSessionState.NoDevice),
                        onExitDemo = {},
                        onRequestPermission = {},
                        onConnectNetwork = { networkingOpen = true },
                        devices = deviceList(AppMode.DEVICE, UsbSessionState.NoDevice, hubs = listOf(hub)),
                        onChooseDevice = { chosen += it },
                        overlay = { if (networkingOpen) NetworkingDialog(networking) { networkingOpen = false } }
                    )
                }
            }
        }
        settle()
    }

    private fun shoot(name: String) {
        if (dir.isNullOrEmpty()) return
        val out = File(dir).apply { mkdirs() }.resolve("$name.png")
        rule.runOnUiThread {
            val root = rule.activity.window.decorView
            val bitmap = Bitmap.createBitmap(root.width, root.height, Bitmap.Config.ARGB_8888)
            root.draw(Canvas(bitmap))
            out.outputStream().use { bitmap.compress(Bitmap.CompressFormat.PNG, 100, it) }
        }
    }

    /**
     * A popup or dialog is a window of its own, which the decor view's drawing leaves out: it is
     * drawn over the screen where its window sits (captureToImage waits for an idle Home, which never comes).
     */
    private fun shootWindow(name: String, node: SemanticsNodeInteraction) {
        if (dir.isNullOrEmpty()) return
        val window = (node.fetchSemanticsNode().root as ViewRootForTest).view.rootView
        val out = File(dir).apply { mkdirs() }.resolve("$name.png")
        rule.runOnUiThread {
            val root = rule.activity.window.decorView
            val bitmap = Bitmap.createBitmap(root.width, root.height, Bitmap.Config.ARGB_8888)
            val canvas = Canvas(bitmap)
            root.draw(canvas)
            val at = IntArray(2).also(window::getLocationOnScreen)
            canvas.translate(at[0].toFloat(), at[1].toFloat())
            window.draw(canvas)
            out.outputStream().use { bitmap.compress(Bitmap.CompressFormat.PNG, 100, it) }
        }
    }

    private val inStrip = hasAnyAncestor(hasTestTag(CONNECTION_STRIP_TAG))

    private fun assertActionsShown() {
        rule.onNode(hasText(NO_DEVICE_LINK_ACTION) and inStrip).assertIsDisplayed()
        rule.onNode(hasText(NO_DEVICE_DEMO_ACTION) and inStrip).assertIsDisplayed()
        // Material's touch target on both (the button is drawn 40dp high, its touch area is 48dp)
        for (tag in listOf(NO_DEVICE_LINK_TAG, NO_DEVICE_DEMO_TAG)) {
            val touch = rule.onNodeWithTag(tag).fetchSemanticsNode().touchBoundsInRoot
            val minPx = with(rule.density) { 48.dp.toPx() } - 1f
            assertTrue("$tag touch target ${touch.height}px", touch.height >= minPx && touch.width >= minPx)
        }
    }

    /** The Link action opens Networking (the same page as Settings › Networking) and leaves DSPi Link off. */
    private fun assertLinkOpensNetworking(shot: String? = null) {
        rule.onNodeWithTag(NO_DEVICE_LINK_TAG).performClick()
        settle()
        rule.onNodeWithText("Networking").assertIsDisplayed()
        shot?.let { shootWindow(it, rule.onNode(isDialog())) }
        assertEquals("choosing it never turns DSPi Link on", emptyList<Boolean>(), networking.enabledCalls)
        assertEquals("a dialog, not a device switch", emptyList<DeviceChoice>(), chosen)
        rule.onNodeWithText("‹ Back").performClick()
        settle()
    }

    @Test
    @Config(qualifiers = "w400dp-h860dp-port-xhdpi")
    fun wpAnlink_phonePortraitStripLeadsToDspiLinkAndDemo() {
        show()
        rule.onNode(hasText("No DSPi connected") and inStrip).assertIsDisplayed()
        assertActionsShown()
        shoot("nodevice-phone-portrait-dark")
        assertLinkOpensNetworking("networking-from-strip-phone")
        rule.onNodeWithTag(NO_DEVICE_DEMO_TAG).performClick()
        settle()
        assertEquals(listOf<DeviceChoice>(DeviceChoice.Demo), chosen)
    }

    @Test
    @Config(qualifiers = "w400dp-h860dp-port-xhdpi")
    fun wpAnlink_phonePortraitLight() {
        show(dark = false)
        assertActionsShown()
        shoot("nodevice-phone-portrait-light")
    }

    /** A phone on its side: the wide strip, the actions inline beside the whole text. */
    @Test
    @Config(qualifiers = "w860dp-h400dp-land-xhdpi")
    fun wpAnlink_phoneLandscapeStripLeadsToDspiLinkAndDemo() {
        show()
        rule.onNode(hasText(ConnectionStrip.NoDevice.text) and inStrip).assertIsDisplayed()
        assertActionsShown()
        shoot("nodevice-phone-landscape-dark")
        assertLinkOpensNetworking()
    }

    @Test
    @Config(qualifiers = "w800dp-h1280dp-port-mdpi")
    fun wpAnlink_tabletStrip() {
        show()
        assertActionsShown()
        shoot("nodevice-tablet-dark")
    }

    /** The overflow's device list with DSPi Link off: "DSPi Link…" is there, no hub, USB disabled. */
    @Test
    @Config(qualifiers = "w400dp-h860dp-port-xhdpi")
    fun wpAnlink_deviceListOffersDspiLinkWhileItIsOff() {
        show()
        rule.onNodeWithTag(STRIP_MENU_TAG).performClick()
        settle()
        rule.onNodeWithText("✓ USB: no DSPi connected").assertIsDisplayed()
        rule.onNodeWithText("Demo (no device)").assertIsDisplayed()
        assertTrue(rule.onAllNodes(hasText("Studio Mac (network)")).fetchSemanticsNodes().isEmpty())
        shootWindow("device-list-phone-link-off", rule.onNode(isPopup()))
        rule.onNodeWithText("DSPi Link…").assertIsDisplayed().performClick()
        settle()
        rule.onNodeWithText("Networking").assertIsDisplayed()
        assertEquals(emptyList<Boolean>(), networking.enabledCalls)
        assertEquals(emptyList<DeviceChoice>(), chosen)
    }

    /** Networking as the app's controller presents it with DSPi Link off; records the switch. */
    private class RecordingNetworking : NetworkingController {
        val enabledCalls = mutableListOf<Boolean>()
        override val state: StateFlow<NetworkingUiState> = MutableStateFlow(NetworkingUiState(defaultClientName = "Test"))

        override fun setEnabled(enabled: Boolean) {
            enabledCalls += enabled
        }

        override fun setFindAutomatically(on: Boolean) = Unit

        override fun setClientName(name: String) = Unit

        override fun connect(hubId: String) = Unit

        override fun disconnect() = Unit

        override fun isValidAddress(text: String) = false

        override fun addManual(text: String) = false

        override fun removeManual(hubId: String) = Unit

        override fun forget(hubId: String) = Unit

        override fun setVisible(visible: Boolean) = Unit
    }
}
