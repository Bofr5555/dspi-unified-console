package se.bofr.dspi.app

import android.app.Application
import android.graphics.Bitmap
import android.graphics.Canvas
import android.os.Looper
import androidx.activity.ComponentActivity
import androidx.compose.foundation.layout.Box
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.snapshots.Snapshot
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.test.assertIsNotEnabled
import androidx.compose.ui.test.hasAnyAncestor
import androidx.compose.ui.test.hasContentDescription
import androidx.compose.ui.test.hasTestTag
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
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
import se.bofr.dspi.core.GuardPhase
import se.bofr.dspi.feature.presets.FileMenuActions
import se.bofr.dspi.feature.presets.IMPORT_PREVIEW_TAG
import se.bofr.dspi.feature.presets.ImportPreviewSheet
import se.bofr.dspi.feature.presets.snapshots.AB_TOGGLE_TAG
import se.bofr.dspi.feature.presets.snapshots.CAPTURE_TAG
import se.bofr.dspi.feature.presets.snapshots.SAFE_APPLY_BAR_TAG
import se.bofr.dspi.feature.presets.snapshots.SAFE_APPLY_REVERT_TAG
import se.bofr.dspi.feature.presets.snapshots.SNAPSHOTS_SCREEN_TAG
import se.bofr.dspi.feature.presets.snapshots.SafeApplyBar
import se.bofr.dspi.feature.presets.snapshots.SnapshotsMode
import se.bofr.dspi.feature.presets.snapshots.SnapshotsModel
import se.bofr.dspi.feature.presets.snapshots.SnapshotsView
import se.bofr.dspi.feature.presets.snapshots.snapshotRecallTag
import se.bofr.dspi.ui.DspiTheme
import se.bofr.dspi.ui.alert.AlertHost
import se.bofr.dspi.ui.settings.AppSettings
import se.bofr.dspi.ui.settings.InMemoryAppSettingsStore
import se.bofr.dspi.ui.settings.LocalAppSettings

/**
 * WP P1AN1 on the phone, over the real core's demo device: the app bar's
 * Snapshots entry, the screen in Advanced and Simple, the A/B toggle off while
 * a safe apply waits, the countdown bar's Revert, the import preview and the
 * strip of a document opened offline. With `-Pdspi.shotsDir=<dir>` each step is
 * also saved as a screenshot (as AN42's shots).
 */
@RunWith(DspiRobolectricTestRunner::class)
@Config(sdk = [35], application = Application::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
class SnapshotsUiWpP1an1Test {
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
        val bridge = startDemoSession(scope, "p1an1").also { session = it }.bridge
        rule.mainClock.autoAdvance = false
        return bridge
    }

    private val slot = mutableStateOf<@Composable () -> Unit>({})

    /** The test's one content (a rule takes one): the theme over [slot], which [show] replaces. */
    private fun content(dark: Boolean, body: @Composable () -> Unit) {
        val store = InMemoryAppSettingsStore(AppSettings())
        slot.value = body
        rule.setContent { CompositionLocalProvider(LocalAppSettings provides store) { DspiTheme(darkTheme = dark) { slot.value() } } }
    }

    private fun show(body: @Composable () -> Unit) {
        rule.runOnUiThread { slot.value = body }
    }

    private fun folder(name: String) = File("build/wp-p1an1/ui-$name").apply {
        deleteRecursively()
        mkdirs()
    }.path

    // -- WP P1AN2 -- one store for the test (a new one per composition would make a new mode model each time)
    private val advanced = InMemoryUiModeStore().apply { save("", "advanced", everyDevice = true) }

    private fun scaffold(bridge: DspiBridge, strip: ConnectionStrip, dark: Boolean) = content(dark) {
        AppScaffold(
            bridge,
            strip,
            onExitDemo = {},
            onRequestPermission = {},
            devices = listOf(DeviceRow(DeviceChoice.Usb, "USB"), DeviceRow(DeviceChoice.Demo, "Demo", current = true)),
            // -- WP P1AN2 -- the screen follows the device's mode now (the phone opens in Simple): Advanced here
            modeStore = advanced
        )
    }

    /** A model on its own store with two snapshots, Out 1 at -4 and -10 dB. */
    private fun twoSnapshots(bridge: DspiBridge, name: String): SnapshotsModel {
        pumpUntil("writes") { bridge.writesAllowed.value }
        val m = SnapshotsModel(bridge, scope, folder(name))
        rule.runOnUiThread { m.load() }
        pumpUntil("loaded") { !m.busy.value }
        for ((label, db) in listOf("Warm" to -4f, "Bright" to -10f)) {
            rule.runOnUiThread { bridge.setOutputGain(0, db) }
            pumpUntil("gain") { bridge.outputGainDB(0) == db }
            val n = m.items.value.size
            rule.runOnUiThread { m.capture(label, if (label == "Warm") "Living room, evening" else "") }
            pumpUntil("captured") { !m.busy.value && m.items.value.size == n + 1 }
        }
        return m
    }

    @Test
    @Config(qualifiers = "w400dp-h860dp-port-xhdpi")
    fun p1an1TheAppBarOpensSnapshots() {
        val bridge = demo()
        scaffold(bridge, ConnectionStrip.Demo, dark = true)
        pumpUntil("writes") { bridge.writesAllowed.value }
        rule.onNodeWithTag(STRIP_MENU_TAG).performClick()
        pumpUntil("menu") { textShown(SNAPSHOTS_ENTRY) }
        rule.onNode(hasText(SNAPSHOTS_ENTRY)).performClick()
        pumpUntil("screen") { shown(SNAPSHOTS_SCREEN_TAG) }
        rule.onNodeWithTag(CAPTURE_TAG).assertExists()
        rule.mainClock.advanceTimeBy(1_500) // the navigation's crossfade
        shoot("snapshots-phone-entry-dark")
    }

    @Test
    @Config(qualifiers = "w400dp-h860dp-port-xhdpi")
    fun p1an1AdvancedEditsSimpleRecallsAndCompares() {
        val bridge = demo()
        content(dark = false) {}
        val m = twoSnapshots(bridge, "modes")
        show { SnapshotsView(m, SnapshotsMode.ADVANCED, onBack = {}) }
        pumpUntil("rows") { textShown("Bright") && textShown("Warm") }
        rule.onNodeWithTag(CAPTURE_TAG).assertExists()
        assertTrue(textShown("Import Snapshot File"))
        shoot("snapshots-phone-advanced-light")
        // A/B between the two: Compare, then the toggle
        rule.runOnUiThread {
            m.chooseA(m.items.value.first { it.name == "Warm" }.id)
            m.chooseB(m.items.value.first { it.name == "Bright" }.id)
            m.startCompare()
        }
        pumpUntil("compare") { !m.busy.value && bridge.snapshots.compare.value.active }
        pumpUntil("toggle shown") { shown(AB_TOGGLE_TAG) }
        rule.onNodeWithTag(AB_TOGGLE_TAG).performClick()
        pumpUntil("on B") { !m.busy.value && bridge.outputGainDB(0) == -10f }
        shoot("snapshots-phone-ab-light")
        bridge.snapshots.compare.value.let { assertTrue(it.active) }
    }

    @Test
    @Config(qualifiers = "w400dp-h860dp-port-xhdpi")
    fun p1an1SimpleShowsNoEditing() {
        val bridge = demo()
        content(dark = true) {}
        val m = twoSnapshots(bridge, "simple")
        show { SnapshotsView(m, SnapshotsMode.SIMPLE, onBack = {}) }
        pumpUntil("rows") { textShown("Bright") }
        rule.onNodeWithTag(CAPTURE_TAG).assertExists() // -- WP UAN -- (UXP A11) Capture in Simple: A/B needs it
        assertFalse(textShown("Import Snapshot File"))
        rule.onNodeWithTag(snapshotRecallTag(m.items.value.first().id)).assertExists()
        rule.onNode(hasContentDescription("More for Bright")).performClick()
        pumpUntil("menu") { textShown("Use as A") }
        assertFalse("no editing in Simple", textShown("Rename"))
        assertFalse(textShown("Delete"))
        shoot("snapshots-phone-simple-dark")
    }

    /** The bar over every screen: a recall with rollback waits; A/B is off; Revert brings the value back. */
    @Test
    @Config(qualifiers = "w400dp-h860dp-port-xhdpi")
    fun p1an1TheSafeApplyBarRevertsAndAbWaitsForIt() {
        val bridge = demo()
        content(dark = true) {}
        val m = twoSnapshots(bridge, "bar")
        rule.runOnUiThread {
            m.chooseA("")
            m.chooseB(m.items.value.first { it.name == "Warm" }.id)
            m.startCompare()
        }
        pumpUntil("compare") { !m.busy.value && bridge.snapshots.compare.value.active }
        show {
            Box {
                SnapshotsView(m, SnapshotsMode.ADVANCED, onBack = {})
                SafeApplyBar(bridge, Modifier.align(Alignment.BottomCenter))
            }
        }
        pumpUntil("rows") { textShown("Bright") }
        // -- WP FLAKE2 -- as SnapshotsWpP1dcfixTest: the view's own load holds Recall off for a while
        val recall = snapshotRecallTag(m.items.value.first { it.name == "Warm" }.id)
        pumpUntil("recall on") {
            !m.busy.value &&
                rule.onAllNodes(hasTestTag(recall)).fetchSemanticsNodes().singleOrNull()
                    ?.config?.contains(SemanticsProperties.Disabled) == false
        }
        // The device is on A (-10 dB); recall Warm (-4 dB) with rollback
        rule.runOnUiThread { m.setSafeRecall(true) }
        rule.onNodeWithTag(recall).performClick()
        pumpUntil("bar") { shown(SAFE_APPLY_BAR_TAG) && bridge.snapshots.guard.value.phase == GuardPhase.WAITING }
        assertEquals(-4f, bridge.outputGainDB(0))
        assertTrue(textShown("Keep these settings?")) // -- WP P1DCFIX -- the core's countdown sentence (F7)
        assertTrue("A/B waits for the safe apply", textShown(SnapshotsModel.FINISH_SAFE_APPLY))
        rule.onNodeWithTag(AB_TOGGLE_TAG).assertIsNotEnabled()
        shoot("safe-apply-bar-phone-dark")
        rule.onNodeWithTag(SAFE_APPLY_REVERT_TAG).performClick()
        pumpUntil("reverted") { bridge.snapshots.guard.value.phase == GuardPhase.REVERTED && bridge.outputGainDB(0) == -10f }
        pumpUntil("compare ended") { !bridge.snapshots.compare.value.active }
        pumpUntil("end message") { shown(SAFE_APPLY_BAR_TAG) && bridge.snapshots.guard.value.message.isNotEmpty() }
        shoot("safe-apply-bar-reverted-phone-dark")
    }

    @Test
    @Config(qualifiers = "w400dp-h860dp-port-xhdpi")
    fun p1an1TheImportPreviewOnAPhone() {
        val bridge = demo()
        content(dark = false) {}
        pumpUntil("writes") { bridge.writesAllowed.value }
        val files = FileMenuActions(bridge, scope, AlertHost())
        var doc: String? = null
        rule.runOnUiThread { scope.launch { doc = files.configurationDocument("Room.dspipreset") } }
        pumpUntil("doc") { doc != null }
        rule.runOnUiThread {
            bridge.setOutputGain(0, -9f)
            bridge.setOutputGain(1, -9f)
        }
        pumpUntil("gain") { bridge.outputGainDB(1) == -9f }
        rule.runOnUiThread { files.importConfiguration(doc!!) }
        pumpUntil("preview") { files.preview.value != null }
        val p = files.preview.value!!
        assertTrue(p.diff.error, p.diff.rows.size >= 2)
        show { ImportPreviewSheet(p, { _, _ -> }, {}, {}, {}) }
        pumpUntil("sheet") { shown(IMPORT_PREVIEW_TAG) && textShown("What changes") }
        rule.onNodeWithText("Apply with rollback").assertExists()
        shoot("import-preview-phone-light")
    }

    @Test
    @Config(qualifiers = "w400dp-h860dp-port-xhdpi")
    fun p1an1ADocumentShowsFileInTheStrip() {
        val bridge = demo()
        scaffold(bridge, ConnectionStrip.Document("Living room.dspipreset"), dark = false)
        pumpUntil("strip") {
            rule.onAllNodes(hasText("File: Living room.dspipreset") and hasAnyAncestor(hasTestTag(CONNECTION_STRIP_TAG)))
                .fetchSemanticsNodes().isNotEmpty()
        }
        rule.onNodeWithTag(STRIP_MENU_TAG).performClick()
        pumpUntil("close file") { textShown("Close file") }
        shoot("document-strip-phone-light")
    }
}
