package se.bofr.dspi.app

import android.app.Application
import android.os.Looper
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.darkColorScheme
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.runtime.snapshots.Snapshot
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.isEnabled
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onAllNodesWithTag
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onLast
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performTextInput
import java.time.Duration
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
import se.bofr.dspi.bridge.DspiBridge
import se.bofr.dspi.core.VregCalPhase
import se.bofr.dspi.core.VregDemoEvent
import se.bofr.dspi.fake.FakeDspiCore
import se.bofr.dspi.feature.settings.AdvancedSettingsTab
import se.bofr.dspi.feature.settings.SettingsCategory
import se.bofr.dspi.ffi.FfiDspiCore
import se.bofr.dspi.ui.settings.InMemoryAppSettingsStore

/**
 * WP AN23: Advanced's Core Voltage page on the real core's demo device (host
 * build of libdspi_ffi over JNA), with the demo's simulated hangs, reboots and
 * clock. The demo is a K340 build: default 1.20 V, so the flow offers 1.20 and
 * 1.25 V, and 1.25 V needs `OVERVOLT`.
 */
@RunWith(DspiRobolectricTestRunner::class)
@Config(sdk = [35], application = Application::class, qualifiers = "w400dp-h2400dp-port-mdpi")
class CoreVoltageAn23Test {
    @get:Rule val rule = createComposeRule()

    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
    private val ffiCore = FfiDspiCore()
    private var session: DemoSession? = null
    private lateinit var bridge: DspiBridge
    private var shown by mutableStateOf(true)

    @After
    fun stop() {
        session.let { if (it != null) rule.runOnUiThread { it.close() } else ffiCore.close() }
        scope.cancel()
    }

    /** The bridge's timers post to the main looper, which only moves here; the FFI thread runs in real time. */
    private fun pump(what: String, until: () -> Boolean) {
        val end = System.currentTimeMillis() + 20_000
        while (!until()) {
            check(System.currentTimeMillis() < end) { "timed out: $what" }
            shadowOf(Looper.getMainLooper()).idleFor(Duration.ofMillis(250))
            rule.runOnUiThread { Snapshot.sendApplyNotifications() }
            rule.mainClock.advanceTimeBy(100)
            Thread.sleep(10)
        }
    }

    private fun onScreen(text: String) =
        rule.onAllNodesWithText(text, substring = true, useUnmergedTree = true).fetchSemanticsNodes().isNotEmpty()

    private fun phase() = bridge.coreVoltageCalibration.view.value?.phase

    private fun startOn(core: se.bofr.dspi.core.DspiCoreApi, canWrite: Boolean = true) {
        session = startDemoSession(scope, "an23", core = core)
        bridge = session!!.bridge
        rule.mainClock.autoAdvance = false
        pump("connected") { bridge.status.value.connected }
        pump("core voltage probed") { bridge.deviceSettings.coreVoltage.value.supported || core !is DemoCore }
        val store = InMemoryAppSettingsStore()
        rule.setContent {
            MaterialTheme(colorScheme = darkColorScheme()) {
                Surface(Modifier.fillMaxSize()) { if (shown) AdvancedSettingsTab(bridge, store, canWrite) }
            }
        }
        pump("Advanced") { onScreen("Channel Names") }
    }

    private fun startDemo() = startOn(DemoCore(ffiCore))

    private fun click(text: String) {
        rule.onNodeWithText(text, useUnmergedTree = true).performClick()
        rule.runOnUiThread { Snapshot.sendApplyNotifications() }
    }

    private fun openPage() {
        click("Open")
        pump("the flow open") { phase() == VregCalPhase.REVIEW }
        pump("the warning") { onScreen("Start Trial") }
    }

    /** Menu [index] of those reading [menu] (the calibration's is first, Manual's second). */
    private fun pick(menu: String, item: String, index: Int = 0) {
        rule.onAllNodesWithText(menu, useUnmergedTree = true)[index].performClick()
        rule.runOnUiThread { Snapshot.sendApplyNotifications() }
        pump("menu $item") { rule.onAllNodesWithText(item).fetchSemanticsNodes().isNotEmpty() }
        rule.onAllNodesWithText(item).onLast().performClick()
        rule.runOnUiThread { Snapshot.sendApplyNotifications() }
    }

    private fun typeWord() {
        rule.onNodeWithTag("coreVoltage.word").performTextInput("OVERVOLT")
        rule.runOnUiThread { Snapshot.sendApplyNotifications() }
    }

    /** The soak on the device's clock, then the next reads. */
    private fun advanceSoak(seconds: Long) = ffiCore.coreVregDemoEvent(VregDemoEvent.ADVANCE_UPTIME, seconds)

    @Test
    fun an23PassNeedsTheWordAndSavesOnlyWhenAsked() {
        startDemo()
        openPage()
        assertTrue("the datasheet warning", onScreen("1.21 V"))
        pick("1.20 V ▾", "1.25 V")
        pump("the word field") { onScreen("Type OVERVOLT") }

        // Without the word nothing goes out
        click("Start Trial")
        pump("the refusal") { onScreen("OVERVOLT") && rule.onAllNodesWithTag("coreVoltage.error").fetchSemanticsNodes().isNotEmpty() }
        assertFalse("no trial without the word", ffiCore.fetchCoreVreg().trialActive)

        typeWord()
        click("Start Trial")
        pump("soaking") { phase() == VregCalPhase.SOAKING }
        pump("abort offered") { onScreen("Abort") }
        assertEquals(0x0E, ffiCore.fetchCoreVreg().trialVreg)
        assertEquals("a trial is not a save", 0, ffiCore.fetchCoreVreg().storedSel)

        advanceSoak(30 * 60)
        pump("passed") { phase() == VregCalPhase.PASSED }
        pump("save offered") { onScreen("Save 1.25 V") }
        assertEquals("a pass is not a save", 0, ffiCore.fetchCoreVreg().storedSel)
        assertTrue("saving above 1.20 V asks again", bridge.coreVoltageCalibration.view.value!!.commitNeedsWord)
        typeWord()
        click("Save 1.25 V")
        pump("committed") { phase() == VregCalPhase.COMMITTED }
        val after = ffiCore.fetchCoreVreg()
        assertEquals(0x0E, after.storedSel)
        assertFalse(after.trialActive)
    }

    @Test
    fun an23HangFailsTheTrialAndOffersTheNextStepUpToTheCap() {
        startDemo()
        openPage()
        click("Start Trial") // 1.20 V, the build default: no word
        pump("soaking") { phase() == VregCalPhase.SOAKING }

        ffiCore.coreVregDemoEvent(VregDemoEvent.HANG, 0)
        pump("failed") { phase() == VregCalPhase.FAILED }
        val failed = bridge.coreVoltageCalibration.view.value!!
        assertEquals(0x0D, failed.failedVreg)
        assertEquals("the next step up", 0x0E, failed.nextVreg)
        assertFalse(failed.hardwareProblem)
        assertEquals("back on the stored voltage", 0, ffiCore.fetchCoreVreg().storedSel)
        pump("1.25 V preselected") { onScreen("1.25 V ▾") && onScreen("Type OVERVOLT") }

        typeWord()
        click("Start Trial")
        pump("soaking at 1.25 V") { phase() == VregCalPhase.SOAKING }
        ffiCore.coreVregDemoEvent(VregDemoEvent.HANG, 0)
        pump("failed at the cap") { phase() == VregCalPhase.FAILED }
        val atCap = bridge.coreVoltageCalibration.view.value!!
        assertTrue("nothing above 1.25 V", atCap.hardwareProblem)
        assertEquals(0, atCap.nextVreg)
        assertTrue("1.30 V is never offered", atCap.choices.none { it.code > 0x0E })
        assertEquals("nothing was saved", 0, ffiCore.fetchCoreVreg().storedSel)
    }

    @Test
    fun an23AbortEndsTheTrial() {
        startDemo()
        openPage()
        click("Start Trial")
        pump("soaking") { phase() == VregCalPhase.SOAKING && onScreen("Abort") }
        click("Abort")
        pump("ended") { phase() == VregCalPhase.ENDED }
        assertFalse(ffiCore.fetchCoreVreg().trialActive)
    }

    @Test
    fun an23TheSoakGoesOnWithThePageAway() {
        startDemo()
        openPage()
        click("Start Trial")
        pump("soaking") { phase() == VregCalPhase.SOAKING }
        // Back to Advanced, then the whole page leaves the screen (screen off, app in the background)
        click("‹ Advanced")
        pump("Advanced again") { onScreen("Channel Names") }
        rule.runOnUiThread { shown = false }
        pump("gone") { !onScreen("Channel Names") }
        advanceSoak(30 * 60)
        assertTrue("the trial ran on without the page", ffiCore.fetchCoreVreg().trialActive)

        rule.runOnUiThread { shown = true }
        pump("Advanced") { onScreen("Channel Names") }
        openPageAt(VregCalPhase.PASSED)
        pump("save offered") { onScreen("Save 1.20 V") }
    }

    private fun openPageAt(phase: VregCalPhase) {
        click("Open")
        pump("$phase") { phase() == phase }
    }

    /**
     * A trial this process did not start: what a new process finds after the
     * old one died mid-soak. The flow is closed, the device has `trial_active`;
     * the page reads it and offers end or keep, and never saves by itself.
     */
    @Test
    fun an23ATrialLeftRunningIsFoundAndCanBeKeptOrEnded() {
        startDemo()
        ffiCore.coreVregManualTrial(0x0D, null)
        assertTrue(ffiCore.fetchCoreVreg().trialActive)
        rule.runOnUiThread { scope.launch { bridge.deviceSettings.refreshCoreVoltage() } }
        pump("the row says so") { onScreen("A trial voltage is running") }

        openPageAt(VregCalPhase.TRIAL_FOUND)
        pump("end or keep") { onScreen("Keep and Soak") && onScreen("End Trial") }
        click("Keep and Soak")
        pump("soaking") { phase() == VregCalPhase.SOAKING }
        advanceSoak(30 * 60)
        pump("passed") { phase() == VregCalPhase.PASSED }
        assertEquals("kept, not saved", 0, ffiCore.fetchCoreVreg().storedSel)
        click("End Trial")
        pump("ended") { phase() == VregCalPhase.ENDED }
        assertFalse(ffiCore.fetchCoreVreg().trialActive)
        assertEquals(0, ffiCore.fetchCoreVreg().storedSel)
    }

    @Test
    fun an23ManualSaveNeedsTheWordAndResetGoesBackToTheDefault() {
        startDemo()
        openPage()
        pick("1.20 V ▾", "1.25 V", index = 1)
        pump("the word field") { onScreen("Type OVERVOLT") }
        click("Save")
        pump("the refusal") { rule.onAllNodesWithTag("coreVoltage.error").fetchSemanticsNodes().isNotEmpty() }
        assertEquals("nothing saved without the word", 0, ffiCore.fetchCoreVreg().storedSel)

        typeWord()
        click("Save")
        pump("saved") { ffiCore.fetchCoreVreg().storedSel == 0x0E }
        pump("read back") { phase() == VregCalPhase.COMMITTED }
        pump("reset offered") { rule.onAllNodes(hasText("Reset") and isEnabled()).fetchSemanticsNodes().isNotEmpty() }
        click("Reset")
        pump("back on the default") { ffiCore.fetchCoreVreg().storedSel == 0 }
    }

    @Test
    fun an23HiddenWithoutTheFeatureOrWrites() {
        // Kotlin fake without EXT_CORE_VREG: no row
        startOn(FakeDspiCore())
        assertFalse(onScreen("Core Voltage"))
        // No Settings page of its own: reachable only from Advanced
        assertTrue(SettingsCategory.entries.none { it.title.contains("Voltage") })
    }

    @Test
    fun an23HiddenOnAReadOnlyConnection() {
        startOn(DemoCore(ffiCore), canWrite = false)
        assertTrue(bridge.deviceSettings.coreVoltage.value.supported)
        assertFalse(onScreen("Core Voltage"))
    }
}
