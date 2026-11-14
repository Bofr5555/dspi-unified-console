package se.bofr.dspi.app

import android.app.Application
import android.graphics.Bitmap
import android.graphics.Canvas
import android.os.Looper
import androidx.activity.ComponentActivity
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.key
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.snapshots.Snapshot
import androidx.compose.ui.test.hasTestTag
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performImeAction
import androidx.compose.ui.test.performTextInput
import java.io.File
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
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode
import se.bofr.dspi.bridge.DspiBridge
import se.bofr.dspi.feature.settings.LIMIT_REPORT_TAG
import se.bofr.dspi.feature.settings.PIN_FIELD_TAG
import se.bofr.dspi.feature.settings.SettingsCategory
import se.bofr.dspi.feature.settings.SettingsView
import se.bofr.dspi.ui.DspiTheme
import se.bofr.dspi.ui.LocalUiMode
import se.bofr.dspi.ui.UiModes
import se.bofr.dspi.ui.settings.AppSettings
import se.bofr.dspi.ui.settings.InMemoryAppSettingsStore
import se.bofr.dspi.ui.settings.LocalAppSettings

/**
 * WP P1AN2 on the phone, over the real core's demo device: the strip's Mode
 * entry and its dialog with no PIN set (no PIN field, Installer opens at once),
 * the Installer page narrowing a limit below the current value and showing the
 * clamp report, the power-on page, and the locked-user state (the banner,
 * Unlock in place of Mode, no Snapshots entry, the PIN asked). With
 * `-Pdspi.shotsDir=<dir>` each step is also saved as a screenshot.
 */
@RunWith(DspiRobolectricTestRunner::class)
@Config(sdk = [35], application = Application::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
class ModeUiWpP1an2Test {
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

    private fun textShown(text: String) = rule.onAllNodes(hasText(text, substring = true)).fetchSemanticsNodes().isNotEmpty()

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
        val bridge = startDemoSession(scope, "p1an2").also { session = it }.bridge
        rule.mainClock.autoAdvance = false
        return bridge
    }

    private val slot = mutableStateOf<@Composable () -> Unit>({})
    private val settings = InMemoryAppSettingsStore(AppSettings())

    private fun content(dark: Boolean, body: @Composable () -> Unit) {
        slot.value = body
        rule.setContent { CompositionLocalProvider(LocalAppSettings provides settings) { DspiTheme(darkTheme = dark) { slot.value() } } }
    }

    private fun show(body: @Composable () -> Unit) {
        rule.runOnUiThread { slot.value = body }
    }

    private fun scaffold(bridge: DspiBridge, store: UiModeStore) {
        show {
            AppScaffold(
                bridge,
                ConnectionStrip.Demo,
                onExitDemo = {},
                onRequestPermission = {},
                devices = listOf(DeviceRow(DeviceChoice.Usb, "USB"), DeviceRow(DeviceChoice.Demo, "Demo", current = true)),
                modeStore = store
            )
        }
    }

    private fun settingsPage(bridge: DspiBridge, mode: String, page: SettingsCategory) = show {
        // A page of its own (SettingsView keeps the open page across a recomposition)
        key(page, mode) {
            CompositionLocalProvider(LocalUiMode provides mode) { SettingsView(bridge, settings, {}, {}, initialPage = page) }
        }
    }

    @Test
    @Config(qualifiers = "w400dp-h860dp-port-xhdpi")
    fun p1an2WithNoPinTheModeDialogAsksNothingAndInstallerOpens() {
        val bridge = demo()
        val store = InMemoryUiModeStore()
        content(dark = true) {}
        scaffold(bridge, store)
        pumpUntil("writes") { bridge.writesAllowed.value }
        rule.onNodeWithTag(STRIP_MENU_TAG).performClick()
        pumpUntil("menu") { textShown(MODE_ENTRY) }
        assertTrue("Snapshots is offered", textShown(SNAPSHOTS_ENTRY))
        rule.onNode(hasText(MODE_ENTRY)).performClick()
        pumpUntil("dialog") { shown(MODE_DIALOG_TAG) }
        assertTrue(textShown("Simple") && textShown("Advanced") && textShown("Installer"))
        assertTrue(textShown("Remember for this device") && textShown("Use for every device on this console"))
        assertFalse("no PIN field with no PIN set", shown(MODE_PIN_TAG))
        rule.onNode(hasText("Installer")).performClick()
        rule.onNodeWithTag(MODE_OK_TAG).performClick()
        pumpUntil("closed") { !shown(MODE_DIALOG_TAG) }
        pumpUntil("kept") { store.chosen(bridge.selectedSerial) == UiModes.INSTALLER }
        // The Installer mode's pages appear in Settings
        settingsPage(bridge, UiModes.INSTALLER, SettingsCategory.INSTALLER)
        pumpUntil("page") { textShown("No installer PIN is set") }
        assertTrue(textShown("Set PIN"))
        shoot("installer-page-no-pin-dark")
    }

    @Test
    @Config(qualifiers = "w400dp-h860dp-port-xhdpi")
    fun p1an2TheInstallerPageNarrowsALimitBelowTheValueAndShowsTheReport() {
        val bridge = demo()
        content(dark = false) {}
        pumpUntil("writes") { bridge.writesAllowed.value }
        rule.runOnUiThread { bridge.setOutputGain(0, 0f) }
        pumpUntil("gain") { bridge.outputGainDB(0) == 0f }
        settingsPage(bridge, UiModes.INSTALLER, SettingsCategory.INSTALLER)
        pumpUntil("page") { shown("limit-Gain, at most (dB)") }
        rule.onNodeWithTag("limit-Gain, at most (dB)").performTextInput("-6")
        rule.onNodeWithTag("limit-Gain, at most (dB)").performImeAction()
        pumpUntil("clamped") { bridge.outputGainDB(0) == -6f }
        pumpUntil("report") {
            rule.onAllNodes(hasTestTag(LIMIT_REPORT_TAG)).fetchSemanticsNodes().isNotEmpty() && textShown("-6")
        }
        assertEquals("ok", bridge.installer.limits.value?.state)
        assertEquals(-6f, bridge.installer.profile.value?.output(0)?.maxGainDb)
        shoot("installer-limit-report-light")
        settingsPage(bridge, UiModes.INSTALLER, SettingsCategory.POWER_ON)
        pumpUntil("power-on") { textShown("Test power-on") }
        assertTrue(textShown("S/PDIF and ADAT inputs stay muted"))
        shoot("power-on-page-light")
        rule.runOnUiThread { scope.launch { bridge.installer.clearLimits() } }
        pumpUntil("cleared") { bridge.installer.profile.value == null }
    }

    @Test
    @Config(qualifiers = "w400dp-h860dp-port-xhdpi")
    fun p1an2TheLockedUserStateShowsTheBannerAndAsksForThePin() {
        val bridge = demo()
        content(dark = true) {}
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
        scaffold(bridge, InMemoryUiModeStore())
        pumpUntil("banner") { shown(LOCK_BANNER_TAG) }
        // -- WP P1EFIX -- the line is the core's installer_lock.banner row, no glyph
        val line = "Locked by the installer: only volume, mute and presets can be changed."
        pumpUntil("banner line") { textShown(line) }
        assertFalse(textShown("\uD83D\uDD12"))
        rule.onNodeWithTag(STRIP_MENU_TAG).performClick()
        pumpUntil("menu") { textShown(UNLOCK_ENTRY) }
        assertFalse("no Snapshots while locked", textShown(SNAPSHOTS_ENTRY))
        assertFalse(textShown(MODE_ENTRY))
        shoot("locked-banner-menu-dark")
        // The unlock: the dialog asks for the PIN, with the scope line. Its model only: under Robolectric a dialog
        // holding a text field never goes idle (the no-PIN dialog, with no field, is drawn in the test above)
        val model = UiModeModel(bridge, InMemoryUiModeStore(), scope)
        pumpUntil("resolved") { model.mode.value == UiModes.LOCKED_USER }
        rule.runOnUiThread { model.openDialog() }
        pumpUntil("unlock") { model.dialog.value != null }
        assertTrue(model.dialog.value!!.unlockOnly)
        assertTrue("the PIN is asked", model.needsPin(model.dialog.value!!))
        assertTrue(model.dialog.value!!.scope.startsWith("This lock is on this console"))
        rule.runOnUiThread { model.closeDialog() }
        // Settings in the locked-user state: About and the installer's note only
        settingsPage(bridge, UiModes.LOCKED_USER, SettingsCategory.INSTALLER)
        pumpUntil("note page") { textShown("locked by the installer") }
        assertFalse(shown(PIN_FIELD_TAG))
        shoot("locked-settings-dark")
    }
}
