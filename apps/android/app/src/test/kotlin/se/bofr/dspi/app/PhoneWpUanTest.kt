package se.bofr.dspi.app

import android.app.Application
import android.graphics.Bitmap
import android.graphics.Canvas
import android.os.Looper
import androidx.activity.ComponentActivity
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.snapshots.Snapshot
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.semantics.getOrNull
import androidx.compose.ui.test.hasAnyAncestor
import androidx.compose.ui.test.hasClickAction
import androidx.compose.ui.test.hasContentDescription
import androidx.compose.ui.test.hasTestTag
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.performClick
import androidx.compose.ui.unit.dp
import java.io.File
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.launch
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode
import se.bofr.dspi.bridge.DspiBridge
import se.bofr.dspi.feature.home.HOME_MUTE_TAG
import se.bofr.dspi.ui.DspiTheme
import se.bofr.dspi.ui.settings.AppSettings
import se.bofr.dspi.ui.settings.InMemoryAppSettingsStore
import se.bofr.dspi.ui.settings.LocalAppSettings
import se.bofr.dspi.usb.UsbSessionState

/**
 * WP UAN (UXP A11, A12) on a phone, over the real core's demo device: a page the lock refuses
 * says so with Unlock… (it said nothing); Home has a mute beside the volume; the strip shows
 * Exit demo and Allow USB access as buttons, with the "Always open" sentence on its own line,
 * and no button on it is drawn under 40dp; unplug and re-plug have their notices. With
 * `-Pdspi.shotsDir=<dir>` the screens are also saved as screenshots.
 */
@RunWith(DspiRobolectricTestRunner::class)
@Config(sdk = [35], application = Application::class, qualifiers = "w400dp-h860dp-port-xhdpi")
@GraphicsMode(GraphicsMode.Mode.NATIVE)
class PhoneWpUanTest {
    @get:Rule val rule = createAndroidComposeRule<ComponentActivity>()

    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
    private var session: DemoSession? = null
    private val dir = System.getProperty("dspi.shotsDir")

    @After
    fun stop() {
        rule.runOnUiThread { session?.close() }
        scope.cancel()
    }

    private fun pumpUntil(what: String, until: () -> Boolean) {
        val end = System.currentTimeMillis() + 20_000
        while (!until()) {
            if (System.currentTimeMillis() > end) throw AssertionError("timed out: $what")
            shadowOf(Looper.getMainLooper()).idle()
            rule.runOnUiThread { Snapshot.sendApplyNotifications() }
            rule.mainClock.advanceTimeBy(50)
            Thread.sleep(10)
        }
    }

    private fun shown(tag: String) = rule.onAllNodes(hasTestTag(tag)).fetchSemanticsNodes().isNotEmpty()

    private fun textShown(text: String) =
        rule.onAllNodes(hasText(text, substring = true), useUnmergedTree = true).fetchSemanticsNodes().isNotEmpty()

    private fun noticeShows(text: String) =
        rule.onAllNodes(hasText(text) and hasAnyAncestor(hasTestTag(REFUSAL_NOTICE_TAG))).fetchSemanticsNodes().isNotEmpty()

    private fun shoot(name: String) {
        if (dir.isNullOrEmpty()) return
        rule.mainClock.advanceTimeBy(400)
        shadowOf(Looper.getMainLooper()).idle()
        val out = File(dir).apply { mkdirs() }.resolve("$name.png")
        rule.runOnUiThread {
            val root = rule.activity.window.decorView
            val bitmap = Bitmap.createBitmap(root.width, root.height, Bitmap.Config.ARGB_8888)
            root.draw(Canvas(bitmap))
            out.outputStream().use { bitmap.compress(Bitmap.CompressFormat.PNG, 100, it) }
        }
    }

    private fun demo(): DspiBridge {
        val bridge = startDemoSession(scope, "uan").also { session = it }.bridge
        rule.mainClock.autoAdvance = false
        return bridge
    }

    private val strip = mutableStateOf<ConnectionStrip>(ConnectionStrip.Demo)
    private val settings = InMemoryAppSettingsStore(AppSettings())

    private fun scaffold(bridge: DspiBridge, dark: Boolean = true) {
        rule.setContent {
            CompositionLocalProvider(LocalAppSettings provides settings) {
                DspiTheme(darkTheme = dark) { Scaffolded(bridge) }
            }
        }
    }

    @Composable
    private fun Scaffolded(bridge: DspiBridge) {
        AppScaffold(
            bridge,
            strip.value,
            onExitDemo = {},
            onRequestPermission = {},
            devices = listOf(DeviceRow(DeviceChoice.Usb, "USB"), DeviceRow(DeviceChoice.Demo, "Demo", current = true)),
            modeStore = InMemoryUiModeStore()
        )
    }

    @Test
    fun uanALockedUsersRefusedTapShowsTheLockNoticeWithUnlock() {
        val bridge = demo()
        scaffold(bridge)
        pumpUntil("writes") { bridge.writesAllowed.value }
        var done = false
        rule.runOnUiThread {
            scope.launch {
                bridge.installer.setPin("2468").getOrThrow()
                bridge.installer.cover(listOf("front")).getOrThrow()
                bridge.installer.lockNow().getOrThrow()
                done = true
            }
        }
        pumpUntil("locked") { done && bridge.installer.lock.value?.locked == true }
        pumpUntil("banner") { shown(LOCK_BANNER_TAG) }
        val line = "Locked by the installer: only volume, mute and presets can be changed."
        assertFalse("no notice before the tap", noticeShows(UNLOCK_ENTRY))
        // The graph's full-screen page is not a locked user's: the tap says so
        rule.onNode(hasContentDescription("Open the graph full screen")).performClick()
        pumpUntil("notice") { noticeShows(UNLOCK_ENTRY) }
        assertTrue(noticeShows(line))
        shoot("uan-locked-refused-tap-dark")
    }

    private fun muteState() = rule.onNodeWithTag(HOME_MUTE_TAG).fetchSemanticsNode().config.getOrNull(SemanticsProperties.StateDescription)

    @Test
    fun uanHomeHasAMuteBesideTheVolume() {
        val bridge = demo()
        scaffold(bridge)
        pumpUntil("writes") { bridge.writesAllowed.value }
        pumpUntil("mute") { shown(HOME_MUTE_TAG) }
        val node = rule.onNodeWithTag(HOME_MUTE_TAG).fetchSemanticsNode()
        assertEquals("Not muted", muteState())
        assertTrue("48dp to touch", node.size.height >= with(rule.density) { 48.dp.toPx() } - 1f)
        rule.onNodeWithTag(HOME_MUTE_TAG).performClick()
        pumpUntil("muted") { bridge.userMute }
        pumpUntil("said") { muteState() == "Muted" }
        shoot("uan-home-muted-dark")
        rule.onNodeWithTag(HOME_MUTE_TAG).performClick()
        pumpUntil("unmuted") { !bridge.userMute }
    }

    /** Every button on the bar is drawn (laid out) at least 40dp each way. */
    private fun assertStripButtonsAtLeast40dp(name: String) {
        val min = with(rule.density) { 40.dp.toPx() } - 1f
        val buttons = rule.onAllNodes(hasClickAction() and hasAnyAncestor(hasTestTag(CONNECTION_STRIP_TAG))).fetchSemanticsNodes()
        assertTrue("$name: buttons", buttons.isNotEmpty())
        for (b in buttons) assertTrue("$name ${b.config} ${b.size}", b.size.height >= min && b.size.width >= min)
    }

    private fun onBar(text: String) = rule.onAllNodes(hasText(text) and hasClickAction() and hasAnyAncestor(hasTestTag(CONNECTION_STRIP_TAG)))
        .fetchSemanticsNodes().isNotEmpty()

    @Test
    fun uanAPhonesStripShowsItsWayOutAndUsbAccessAsButtons() {
        val bridge = demo()
        scaffold(bridge, dark = false)
        pumpUntil("writes") { bridge.writesAllowed.value }
        pumpUntil("exit demo") { onBar("Exit demo") }
        assertFalse(shown(STRIP_HINT_TAG))
        assertStripButtonsAtLeast40dp("demo")
        shoot("uan-strip-demo-light")
        // The menu no longer repeats it
        rule.onNodeWithTag(STRIP_MENU_TAG).performClick()
        pumpUntil("menu") { textShown(MODE_ENTRY) }
        assertEquals(1, rule.onAllNodes(hasText("Exit demo"), useUnmergedTree = true).fetchSemanticsNodes().size)
        rule.runOnUiThread { strip.value = connectionStrip(AppMode.DEVICE, UsbSessionState.PermissionRequired("")) }
        pumpUntil("allow") { onBar("Allow USB access") }
        pumpUntil("hint") { shown(STRIP_HINT_TAG) && textShown(USB_ALWAYS_OPEN_HINT) }
        assertStripButtonsAtLeast40dp("permission")
        shoot("uan-strip-usb-access-light")
    }

    @Test
    fun uanUnplugAndReplugHaveTheirNotices() {
        val bridge = demo()
        scaffold(bridge)
        rule.runOnUiThread { strip.value = ConnectionStrip.Usb }
        pumpUntil("writes") { bridge.writesAllowed.value }
        assertFalse(textShown(USB_LOST_NOTICE))
        rule.runOnUiThread { strip.value = ConnectionStrip.NoDevice }
        pumpUntil("lost") { textShown(USB_LOST_NOTICE) }
        shoot("uan-usb-lost-dark")
        rule.runOnUiThread { strip.value = ConnectionStrip.Usb }
        pumpUntil("back") { textShown(USB_BACK_NOTICE) && !textShown(USB_LOST_NOTICE) }
    }

    @Test
    fun uanTheLossNoticesFollowTheStrip() {
        val n = UsbLossNotices()
        assertNull("the first open says nothing", n.next(ConnectionStrip.Usb))
        assertEquals(USB_LOST_NOTICE, n.next(ConnectionStrip.NoDevice))
        assertNull(n.next(ConnectionStrip.NoDevice))
        assertEquals(USB_BACK_NOTICE, n.next(ConnectionStrip.Usb))
        assertNull("said once", n.next(ConnectionStrip.Usb))
        // Leaving for the demo is no loss, and forgets one
        assertNull(n.next(ConnectionStrip.Demo))
        assertNull(n.next(ConnectionStrip.Usb))
        assertEquals(USB_LOST_NOTICE, n.next(ConnectionStrip.NeedsPermission("x")))
        assertNull(n.next(ConnectionStrip.Demo))
        assertNull("the demo forgot the loss", n.next(ConnectionStrip.Usb))
    }
}
