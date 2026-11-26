package se.bofr.dspi.app

import android.app.Application
import android.os.Looper
import androidx.activity.ComponentActivity
import androidx.compose.foundation.layout.Box
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.snapshots.Snapshot
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.input.key.Key
import androidx.compose.ui.input.key.KeyEvent
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.test.hasTestTag
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.performClick
import java.io.File
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
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config
import se.bofr.dspi.bridge.DspiBridge
import se.bofr.dspi.core.GuardPhase
import se.bofr.dspi.feature.presets.snapshots.AB_START_TAG
import se.bofr.dspi.feature.presets.snapshots.CAPTURE_TAG
import se.bofr.dspi.feature.presets.snapshots.SAFE_APPLY_BAR_TAG
import se.bofr.dspi.feature.presets.snapshots.SAFE_APPLY_TIMEOUT_TAG
import se.bofr.dspi.feature.presets.snapshots.SafeApplyBar
import se.bofr.dspi.feature.presets.snapshots.SnapshotsMode
import se.bofr.dspi.feature.presets.snapshots.SnapshotsModel
import se.bofr.dspi.feature.presets.snapshots.SnapshotsView
import se.bofr.dspi.feature.presets.snapshots.snapshotRecallTag
import se.bofr.dspi.ui.DspiRoutes
import se.bofr.dspi.ui.DspiTheme
import se.bofr.dspi.ui.settings.AppSettings
import se.bofr.dspi.ui.settings.InMemoryAppSettingsStore
import se.bofr.dspi.ui.settings.LocalAppSettings

/**
 * WP P1DCFIX on the phone, over the real core's demo device: the A/B key is a
 * plain B (never while editing or under a dialog); Simple's Snapshots view is
 * recall and A/B only; Advanced sets the safe apply's timeout (default 30 s),
 * which a guarded recall takes; the countdown is the core's sentence.
 */
@RunWith(DspiRobolectricTestRunner::class)
@Config(sdk = [35], application = Application::class, qualifiers = "w400dp-h860dp-port-xhdpi")
class SnapshotsWpP1dcfixTest {
    @get:Rule val rule = createAndroidComposeRule<ComponentActivity>()

    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
    private var session: DemoSession? = null
    private val store = InMemoryAppSettingsStore(AppSettings())
    private val slot = mutableStateOf<@Composable () -> Unit>({})

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

    private fun demo(): DspiBridge {
        val bridge = startDemoSession(scope, "p1dcfix").also { session = it }.bridge
        rule.mainClock.autoAdvance = false
        rule.setContent { CompositionLocalProvider(LocalAppSettings provides store) { DspiTheme(darkTheme = false) { slot.value() } } }
        return bridge
    }

    private fun show(body: @Composable () -> Unit) {
        rule.runOnUiThread { slot.value = body }
    }

    /** A model on its own store with one snapshot, Out 1 at -4 dB; the device then at -10 dB. */
    private fun oneSnapshot(bridge: DspiBridge, name: String): SnapshotsModel {
        pumpUntil("writes") { bridge.writesAllowed.value }
        val dir = File("build/wp-p1dcfix/ui-$name").apply {
            deleteRecursively()
            mkdirs()
        }
        val m = SnapshotsModel(bridge, scope, dir.path)
        rule.runOnUiThread { m.load() }
        pumpUntil("loaded") { !m.busy.value }
        rule.runOnUiThread { bridge.setOutputGain(0, -4f) }
        pumpUntil("gain") { bridge.outputGainDB(0) == -4f }
        rule.runOnUiThread { m.capture("Warm", "") }
        pumpUntil("captured") { !m.busy.value && m.items.value.size == 1 }
        rule.runOnUiThread { bridge.setOutputGain(0, -10f) }
        pumpUntil("gain") { bridge.outputGainDB(0) == -10f }
        return m
    }

    @Test
    fun p1dcfixSimpleIsRecallAndAbOnly() {
        val bridge = demo()
        val m = oneSnapshot(bridge, "simple")
        show { SnapshotsView(m, SnapshotsMode.SIMPLE, onBack = {}) }
        pumpUntil("rows") { textShown("Warm") }
        rule.onNodeWithTag(snapshotRecallTag(m.items.value.single().id)).assertExists()
        rule.onNodeWithTag(AB_START_TAG).assertExists()
        rule.onNodeWithTag(CAPTURE_TAG).assertExists() // -- WP UAN -- (UXP A11) Capture in Simple: A/B needs it
        rule.onNodeWithTag(SAFE_APPLY_TIMEOUT_TAG).assertDoesNotExist()
        assertFalse("no rollback choice in Simple", textShown("Recall with rollback"))
        assertFalse(textShown("Import Snapshot File"))
        // Advanced has them, the timeout at the default 30 s
        show { SnapshotsView(m, SnapshotsMode.ADVANCED, onBack = {}) }
        pumpUntil("advanced") { shown(SAFE_APPLY_TIMEOUT_TAG) } // -- WP UAN -- Capture shows in Simple too
        rule.onNodeWithTag(SAFE_APPLY_TIMEOUT_TAG).assertExists()
        assertTrue(textShown("Rollback time: 30 s"))
        assertTrue(textShown("Recall with rollback"))
    }

    @Test
    fun p1dcfixAGuardedRecallTakesTheSettingsTimeoutAndTheCoresSentence() {
        val bridge = demo()
        val m = oneSnapshot(bridge, "timeout")
        rule.runOnUiThread { store.update { it.copy(safeApplyTimeoutS = 45) } }
        show {
            Box {
                SnapshotsView(m, SnapshotsMode.ADVANCED, onBack = {})
                SafeApplyBar(bridge, Modifier.align(Alignment.BottomCenter))
            }
        }
        pumpUntil("rows") { textShown("Warm") && textShown("Rollback time: 45 s") }
        // -- WP FLAKE2 -- the view's own load (LaunchedEffect: model.load()) holds the model busy and
        // Recall off; the rows are already there from before, so a click while it ran went nowhere
        val recall = snapshotRecallTag(m.items.value.single().id)
        pumpUntil("recall on") {
            !m.busy.value &&
                rule.onAllNodes(hasTestTag(recall)).fetchSemanticsNodes().singleOrNull()
                    ?.config?.contains(SemanticsProperties.Disabled) == false
        }
        rule.runOnUiThread { m.setSafeRecall(true) }
        rule.onNodeWithTag(recall).performClick()
        pumpUntil("bar") { shown(SAFE_APPLY_BAR_TAG) && bridge.snapshots.guard.value.phase == GuardPhase.WAITING }
        assertEquals(45_000L, bridge.snapshots.guard.value.timeoutMs)
        assertTrue(textShown("Keep these settings? They go back in "))
        assertTrue(textShown("\"Warm\""))
        // Out of range never reaches the core
        assertEquals(5, AppSettings.safeApplyTimeout(1))
        assertEquals(300, AppSettings.safeApplyTimeout(9_999))
    }

    private class Host : ShortcutHost {
        var toggles = 0
        override val currentRoute: String? = DspiRoutes.HOME
        override val selectedChannel: Int? = null
        override val writesAllowed = true
        override val generatorRunning = false
        override val generatorCanStart = false

        override fun canPaste() = false

        override fun navigate(route: String) {}

        override fun copyChannel(ch: Int) {}

        override fun pasteChannel(ch: Int) {}

        override fun startGenerator() {}

        override fun stopGenerator() {}

        override fun abToggle(): Boolean {
            toggles++
            return true
        }
    }

    private fun down(code: Int, meta: Int = 0) = KeyEvent(android.view.KeyEvent(0, 0, android.view.KeyEvent.ACTION_DOWN, code, 0, meta))

    @Test
    fun p1dcfixTheAbKeyIsAPlainBNeverWhileEditingOrUnderADialog() {
        assertEquals(ConsoleShortcut.AbToggle, ConsoleKeyMap.shortcutOf(Key.B, command = false, shift = false, alt = false))
        assertNull("Shift+B stays with the view", ConsoleKeyMap.shortcutOf(Key.B, command = false, shift = true, alt = false))
        val bypass = ConsoleKeyMap.shortcutOf(Key.B, command = true, shift = false, alt = false)
        assertEquals("Command+B is still bypass", ConsoleShortcut.Bypass, bypass)
        assertNull("Command+T is no longer the toggle", ConsoleKeyMap.shortcutOf(Key.T, command = true, shift = false, alt = false))
        val host = Host()
        var editing = false
        var dialog = false
        val keys = ShortcutKeys().apply {
            this.host = host
            this.editing = { editing }
            dialogOpen = { dialog }
        }
        val b = android.view.KeyEvent.KEYCODE_B
        assertTrue(keys.onKey(down(b)))
        assertEquals(1, host.toggles)
        editing = true
        assertFalse("a text field takes the B", keys.onKey(down(b)))
        editing = false
        dialog = true
        assertFalse("a dialog is open", keys.onKey(down(b)))
        dialog = false
        // Ctrl+B is the bypass, not the toggle
        keys.onKey(down(b, android.view.KeyEvent.META_CTRL_ON or android.view.KeyEvent.META_CTRL_LEFT_ON))
        assertEquals(1, host.toggles)
    }
}
