package se.bofr.dspi.feature.settings.cs

import android.app.Application
import android.os.Looper
import android.view.View
import androidx.activity.OnBackPressedDispatcher
import androidx.activity.compose.LocalOnBackPressedDispatcherOwner
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.darkColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalView
import androidx.compose.ui.test.assertIsNotEnabled
import androidx.compose.ui.test.hasClickAction
import androidx.compose.ui.test.hasTestTag
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onRoot
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollToNode
import androidx.compose.ui.test.printToString
import java.time.Duration
import java.util.concurrent.Executors
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.asCoroutineDispatcher
import kotlinx.coroutines.cancel
import kotlinx.coroutines.runBlocking
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config
import se.bofr.dspi.bridge.DspiBridgeImpl
import se.bofr.dspi.core.DeviceInfo
import se.bofr.dspi.core.DspiCoreApi
import se.bofr.dspi.core.DspiException
import se.bofr.dspi.feature.settings.ControlSurfacesSettingsTab
import se.bofr.dspi.feature.settings.CsModel
import se.bofr.dspi.feature.settings.CsSection
import se.bofr.dspi.feature.settings.SettingsSaveCoordinator
import se.bofr.dspi.ffi.FfiDspiCore
import se.bofr.dspi.ui.json.Json
import se.bofr.dspi.ui.json.int
import se.bofr.dspi.ui.json.long

/**
 * WP AN10: the IR receiver's pages and the learn on the real core's demo device
 * (CG16's surfaces: a receiver on slot 3 with one learned NEC button), through the
 * real bridge and CG20's rules, at phone size. A press on the remote is the
 * core's `cs_demo_learn`. Real receivers and remotes are [HW] and not here.
 */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35], application = Application::class, qualifiers = "w360dp-h640dp-port-mdpi")
class ControlSurfacesIrDemoAn10Test {
    @get:Rule val rule = createComposeRule()

    private val ffiCore = FfiDspiCore()
    private val executor = Executors.newSingleThreadExecutor { r -> Thread(r, "an10-ffi").apply { isDaemon = true } }
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
    private lateinit var bridge: DspiBridgeImpl
    private var back: OnBackPressedDispatcher? = null
    private var view: View? = null

    private class Demo(private val core: DspiCoreApi) : DspiCoreApi by core {
        override fun scanDevices(): List<DeviceInfo> = listOf(DeviceInfo(SERIAL, 0))

        override fun selectDevice(serial: String) {
            if (serial != SERIAL) throw DspiException.Disconnected()
            if (!core.isConnected()) core.openDemo()
        }
    }

    @Before
    fun start() {
        bridge = DspiBridgeImpl(Demo(ffiCore), scope, ffi = executor.asCoroutineDispatcher(), appVersion = "an10")
        rule.mainClock.autoAdvance = false
        pump("connected") { bridge.status.value.connected }
        bridge.cs.refresh()
        pump("cs loaded") { CsModel.parse(bridge.cs.model.value).bindings.isNotEmpty() }
    }

    @After
    fun stop() {
        rule.runOnUiThread { runCatching { bridge.close() } }
        scope.cancel()
        executor.execute {
            runCatching { ffiCore.disconnect() }
            runCatching { ffiCore.close() }
        }
        executor.shutdown()
    }

    /** The main looper's clock moves only here (the learn's polls wait on it); the FFI thread runs in real time. */
    private fun pump(what: String, until: () -> Boolean) {
        val end = System.currentTimeMillis() + 20_000
        while (!until()) {
            if (System.currentTimeMillis() >= end) {
                runCatching { println(rule.onRoot(useUnmergedTree = true).printToString()) }
                error("timed out: $what")
            }
            shadowOf(Looper.getMainLooper()).idleFor(Duration.ofMillis(50))
            rule.mainClock.advanceTimeBy(50)
            Thread.sleep(5)
        }
    }

    private fun scrollTo(list: String, tag: String) {
        rule.mainClock.autoAdvance = true
        try {
            rule.onNodeWithTag(list).performScrollToNode(hasTestTag(tag))
        } finally {
            rule.mainClock.autoAdvance = false
        }
    }

    private fun model() = CsModel.parse(bridge.cs.model.value)

    private fun ir(sub: Int) = Json.parseObject(CsIr.records(bridge.cs.model.value)[sub])!!

    private fun exists(tag: String) = rule.onAllNodes(hasTestTag(tag)).fetchSemanticsNodes().isNotEmpty()

    private fun hasNodeText(text: String) = rule.onAllNodes(hasText(text, substring = true)).fetchSemanticsNodes().isNotEmpty()

    /** A press on the remote while the receiver listens: false when nothing is armed. */
    private fun press(protocol: Int, code: Long): Boolean = executor.submit<Boolean> { ffiCore.csDemoLearn(protocol, code) }.get()

    private fun learnState(): Int = executor.submit<Int> { Json.parseObject(ffiCore.csApply(CsIr.READ))?.int("state") ?: -1 }.get()

    private var shown by mutableStateOf(true)

    private fun host() {
        rule.setContent {
            back = LocalOnBackPressedDispatcherOwner.current?.onBackPressedDispatcher
            view = LocalView.current
            MaterialTheme(colorScheme = darkColorScheme()) {
                Surface(Modifier.fillMaxSize()) { if (shown) Tab() }
            }
        }
    }

    @Composable
    private fun Tab() {
        val json by bridge.cs.model.collectAsState()
        val cs = remember(json) { CsModel.parse(json) }
        ControlSurfacesSettingsTab(CsSection.CONTROLS, bridge, SettingsSaveCoordinator.of(bridge), cs)
    }

    private fun receiverSlot(): Int = model().bindings.indexOfFirst { it.type == CsModel.TYPE_IR }
        .also { check(it >= 0) { "the demo has no IR receiver" } }

    /** S4: the receiver's page, scrolled to its remote buttons. */
    private fun openReceiver() {
        val slot = receiverSlot()
        assertTrue("the demo's receiver is active", model().isSlotActive(slot))
        pump("list") { exists("cs.slot.$slot") }
        rule.onNodeWithTag("cs.slot.$slot").performClick()
        pump("receiver rows") { exists(rowTag("gpio0")) }
        scrollTo("cs.editor", "cs.ir.add")
    }

    private fun openButton(sub: Int) {
        openReceiver()
        scrollTo("cs.editor", "cs.ir.$sub")
        rule.onNodeWithTag("cs.ir.$sub").performClick()
        pump("button page") { exists("cs.ir.editor") && exists(rowTag("noun")) }
    }

    private fun learn(protocol: Int, code: Long) {
        rule.onNodeWithTag("cs.ir.learn").performClick()
        pump("D2 shown") { exists("cs.ir.learning") }
        pump("the remote is heard") { press(protocol, code) }
        pump("D2 closed") { !exists("cs.ir.learning") }
    }

    @Test
    fun an10TheReceiverPageListsItsRemoteButtonsWithTheCoresSummary() {
        host()
        openReceiver()
        pump("the learned button") { exists("cs.ir.0") }
        assertTrue(hasNodeText("NEC 0x20DF40BF"))
        val summary = CsRules.parseDescription(
            runBlocking {
                bridge.cs.describe(CsIr.KIND, 0, CsIr.records(bridge.cs.model.value)[0])
            }
        ).value!!.summary
        assertTrue(summary.isNotEmpty())
        pump("summary: $summary") { hasNodeText(summary) }
        // The receiver's own rows are the core's: its GPIO and its sense
        assertTrue(exists(rowTag("invert")))
    }

    @Test
    fun an10ReLearnPutsTheCodeInTheDraftAndApplySendsIt() {
        host()
        openButton(0)
        learn(2, 0x1234)
        pump("learned message") { hasNodeText("Learned a RC5 code. Apply to keep it.") }
        assertTrue(hasNodeText("RC5 0x00001234"))
        // Nothing reaches the device before Apply
        assertEquals(0x20DF40BFL, ir(0).long("code"))
        assertTrue(CsIrStore.of(bridge).isDirty(0))
        rule.onNodeWithTag("cs.apply").performClick()
        pump("applied") { ir(0).long("code") == 0x1234L && ir(0).int("protocol") == 2 }
        assertTrue("live, not yet saved", bridge.cs.unsaved.value)
        pump("draft clean") { !CsIrStore.of(bridge).isDirty(0) }
    }

    @Test
    fun an10AddLearnsAFreshButtonAndApplyNeedsTheCode() {
        host()
        openReceiver()
        val free = CsIr.firstFree(CsIr.records(bridge.cs.model.value))!!
        rule.onNode(hasText("Add") and hasClickAction()).performClick()
        pump("fresh button page") { exists("cs.ir.editor") && exists(rowTag("noun")) }
        assertTrue(hasNodeText(CsIr.NOT_LEARNED))
        rule.onNodeWithTag("cs.apply").assertIsNotEnabled()
        assertFalse(CsIr.isLearned(CsIr.records(bridge.cs.model.value)[free]))
        learn(1, 0x00FF00FF)
        pump("learned") { hasNodeText("NEC 0x00FF00FF") }
        rule.onNodeWithTag("cs.apply").performClick()
        pump("applied") { ir(free).long("code") == 0x00FF00FFL && ir(free).int("protocol") == 1 }
        // Back on the receiver page, the new button is listed
        rule.runOnUiThread { back!!.onBackPressed() }
        pump("listed") { exists("cs.editor") }
        scrollTo("cs.editor", "cs.ir.$free")
    }

    @Test
    fun an10D2KeepsTheScreenOnAndCancelDisarms() {
        host()
        openButton(0)
        rule.onNodeWithTag("cs.ir.learn").performClick()
        pump("armed") { exists("cs.ir.learning") && learnState() == CsIr.LEARN_ARMED }
        assertTrue("screen kept on while listening", view!!.keepScreenOn)
        assertTrue(hasNodeText(CsIr.LEARN_PROMPT))
        rule.onNodeWithTag("cs.ir.learn.cancel").performClick()
        pump("cancelled") { !exists("cs.ir.learning") && hasNodeText(CsIr.LEARN_CANCELLED) }
        pump("disarmed") { learnState() != CsIr.LEARN_ARMED }
        assertFalse("a later press is not taken", press(1, 7))
        assertFalse(view!!.keepScreenOn)
        assertFalse(CsIrStore.of(bridge).isDirty(0))
        pump("not busy") { !SettingsSaveCoordinator.of(bridge).csBusy.value }
    }

    @Test
    fun an10NoPressInTheWindowTimesOutAndDisarms() {
        host()
        openButton(0)
        rule.onNodeWithTag("cs.ir.learn").performClick()
        pump("armed") { exists("cs.ir.learning") }
        // The demo models no window: the page gives up a little past 10 s and disarms
        pump("timed out") { !exists("cs.ir.learning") && hasNodeText(CsIr.LEARN_TIMED_OUT) }
        pump("disarmed") { learnState() != CsIr.LEARN_ARMED }
        assertFalse(press(1, 7))
        assertFalse(CsIrStore.of(bridge).isDirty(0))
    }

    @Test
    fun an10ARefusedLearnSaysWhy() {
        host()
        openButton(0)
        // The demo held in a running revert refuses a learn, as a busy device does
        executor.submit { ffiCore.csDemoBusy(true) }.get()
        rule.onNodeWithTag("cs.ir.learn").performClick()
        pump("refusal shown") { !exists("cs.ir.learning") && exists("cs.ir.message") }
        assertTrue(hasNodeText(bridge.cs.statusMessage(0x1B)))
        executor.submit { ffiCore.csDemoBusy(false) }.get()
    }

    @Test
    fun an10RemoveAsksFirst() {
        host()
        openButton(0)
        scrollTo("cs.ir.editor", "cs.remove")
        rule.onNodeWithTag("cs.remove").performClick()
        pump("asks") { hasNodeText("Remove This Remote Button?") }
        assertTrue("nothing removed yet", CsIr.isLearned(CsIr.records(bridge.cs.model.value)[0]))
        rule.onNodeWithTag("cs.remove.confirm").performClick()
        pump("removed") { !CsIr.isLearned(CsIr.records(bridge.cs.model.value)[0]) }
        pump("receiver page") { exists("cs.editor") }
    }

    @Test
    fun an10ALearnedDraftOutlivesThePageAndBackAsks() {
        host()
        openButton(0)
        learn(3, 0xABCD)
        pump("learned") { hasNodeText("RC6 0x0000ABCD") }
        shown = false
        pump("gone") { !exists("cs.ir.editor") }
        shown = true
        openButton(0)
        pump("draft kept") { hasNodeText("RC6 0x0000ABCD") }
        rule.runOnUiThread { back!!.onBackPressed() }
        pump("asks") { hasNodeText("Discard Changes?") }
        rule.onNodeWithTag("cs.discard").performClick()
        pump("receiver page") { exists("cs.editor") && !exists("cs.ir.editor") }
        assertFalse(CsIrStore.of(bridge).isDirty(0))
        assertEquals(0x20DF40BFL, ir(0).long("code"))
    }

    companion object {
        const val SERIAL = "DEMO-AN10"
    }
}
