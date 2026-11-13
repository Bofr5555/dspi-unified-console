package se.bofr.dspi.app

import android.app.Application
import android.graphics.Bitmap
import android.graphics.Canvas
import android.os.Looper
import androidx.activity.ComponentActivity
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.snapshots.Snapshot
import androidx.compose.ui.test.hasAnyAncestor
import androidx.compose.ui.test.hasTestTag
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.performClick
import java.io.File
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import org.junit.After
import org.junit.Assume.assumeTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode
import se.bofr.dspi.ui.DspiTheme
import se.bofr.dspi.ui.settings.AppSettings
import se.bofr.dspi.ui.settings.InMemoryAppSettingsStore
import se.bofr.dspi.ui.settings.LocalAppSettings

/**
 * WP AN42: screenshots of the strip's Undo and Redo on the phone over the real
 * core's demo device, after an undo (its snackbar showing), for the report.
 * As UIM1's HomeShotsWpUim1Test: only with `-Pdspi.shotsDir=<dir>`; nothing is asserted.
 */
@RunWith(DspiRobolectricTestRunner::class)
@Config(sdk = [35], application = Application::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
class UndoBarShotsWpAn42Test {
    @get:Rule val rule = createAndroidComposeRule<ComponentActivity>()

    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
    private var session: DemoSession? = null
    private val dir = System.getProperty("dspi.shotsDir")

    @After
    fun stop() {
        rule.runOnUiThread { session?.close() }
        scope.cancel()
    }

    private fun pumpUntil(until: () -> Boolean) {
        val end = System.currentTimeMillis() + 15_000
        while (!until() && System.currentTimeMillis() < end) {
            shadowOf(Looper.getMainLooper()).idle()
            rule.runOnUiThread { Snapshot.sendApplyNotifications() }
            rule.mainClock.advanceTimeBy(50)
            Thread.sleep(10)
        }
    }

    private fun shoot(name: String, dark: Boolean) {
        assumeTrue("no -Pdspi.shotsDir", !dir.isNullOrEmpty())
        val bridge = startDemoSession(scope, "an42").also { session = it }.bridge
        val store = InMemoryAppSettingsStore(AppSettings())
        rule.mainClock.autoAdvance = false
        rule.setContent {
            CompositionLocalProvider(LocalAppSettings provides store) {
                DspiTheme(darkTheme = dark) {
                    AppScaffold(
                        bridge,
                        ConnectionStrip.Demo,
                        onExitDemo = {},
                        onRequestPermission = {},
                        devices = listOf(DeviceRow(DeviceChoice.Usb, "USB"), DeviceRow(DeviceChoice.Demo, "Demo", current = true))
                    )
                }
            }
        }
        pumpUntil { bridge.writesAllowed.value }
        // Two changes, one undone with the strip's button: both buttons on, the snackbar saying what moved
        rule.runOnUiThread { bridge.setUserMute(!bridge.userMute) }
        pumpUntil { bridge.undoState.value.canUndo }
        rule.runOnUiThread { bridge.setMasterVolume(-12f) }
        pumpUntil { bridge.masterVolumeDB == -12f }
        rule.onNode(hasTestTag(UNDO_BUTTON_TAG)).performClick()
        pumpUntil {
            rule.onAllNodes(hasText("Undone: Master volume") and hasAnyAncestor(hasTestTag(REFUSAL_NOTICE_TAG)), useUnmergedTree = true)
                .fetchSemanticsNodes().isNotEmpty()
        }
        rule.mainClock.advanceTimeBy(400) // the snackbar's entry animation
        shadowOf(Looper.getMainLooper()).idle()
        val out = File(dir!!).apply { mkdirs() }.resolve("$name.png")
        rule.runOnUiThread {
            val root = rule.activity.window.decorView
            val bitmap = Bitmap.createBitmap(root.width, root.height, Bitmap.Config.ARGB_8888)
            root.draw(Canvas(bitmap))
            out.outputStream().use { bitmap.compress(Bitmap.CompressFormat.PNG, 100, it) }
        }
    }

    @Test
    @Config(qualifiers = "w400dp-h860dp-port-xhdpi")
    fun an42PhoneUndoRedoDark() = shoot("undo-redo-phone-dark", dark = true)

    @Test
    @Config(qualifiers = "w400dp-h860dp-port-xhdpi")
    fun an42PhoneUndoRedoLight() = shoot("undo-redo-phone-light", dark = false)

    /** A tablet: the wide strip, its actions and Devices beside Undo and Redo. */
    @Test
    @Config(qualifiers = "w800dp-h1280dp-port-mdpi")
    fun an42TabletUndoRedoDark() = shoot("undo-redo-tablet-dark", dark = true)
}
