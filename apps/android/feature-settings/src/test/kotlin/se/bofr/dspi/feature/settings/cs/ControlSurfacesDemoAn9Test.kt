package se.bofr.dspi.feature.settings.cs

import android.app.Application
import android.os.Looper
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
import androidx.compose.ui.test.hasTestTag
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.onRoot
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollToNode
import androidx.compose.ui.test.performTextInput
import androidx.compose.ui.test.printToString
import java.util.concurrent.Executors
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.asCoroutineDispatcher
import kotlinx.coroutines.cancel
import kotlinx.coroutines.runBlocking
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotEquals
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

/**
 * WP AN9: the Control Surfaces pages on the real core's demo device (`open_demo`,
 * CG16's FakeDevice surfaces) through the real bridge and CG20's rules, at phone
 * size. The host build of libdspi_ffi over JNA, as :app's AN28 tests. No device.
 */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35], application = Application::class, qualifiers = "w360dp-h640dp-port-mdpi")
class ControlSurfacesDemoAn9Test {
    @get:Rule val rule = createComposeRule()

    private val ffiCore = FfiDspiCore()
    private val executor = Executors.newSingleThreadExecutor { r -> Thread(r, "an9-ffi").apply { isDaemon = true } }
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
    private lateinit var bridge: DspiBridgeImpl
    private var back: OnBackPressedDispatcher? = null

    /** The demo device found the usual way: listed, then opened on select (as :app's DemoCore). */
    private class Demo(private val core: DspiCoreApi) : DspiCoreApi by core {
        override fun scanDevices(): List<DeviceInfo> = listOf(DeviceInfo(SERIAL, 0))

        override fun selectDevice(serial: String) {
            if (serial != SERIAL) throw DspiException.Disconnected()
            if (!core.isConnected()) core.openDemo()
        }
    }

    @Before
    fun start() {
        bridge = DspiBridgeImpl(Demo(ffiCore), scope, ffi = executor.asCoroutineDispatcher(), appVersion = "an9")
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

    /** The bridge posts to the main looper, which nothing else pumps here; the FFI thread runs in real time. */
    private fun pump(what: String, until: () -> Boolean) {
        val end = System.currentTimeMillis() + 15_000
        while (!until()) {
            if (System.currentTimeMillis() >= end) {
                runCatching { println(rule.onRoot(useUnmergedTree = true).printToString()) }
                error("timed out: $what")
            }
            shadowOf(Looper.getMainLooper()).idle()
            rule.mainClock.advanceTimeBy(50)
            Thread.sleep(10)
        }
    }

    /** A lazy list's scroll waits for a frame, which the paused clock never gives: run the clock for it. */
    private fun scrollTo(list: String, tag: String) {
        rule.mainClock.autoAdvance = true
        try {
            rule.onNodeWithTag(list).performScrollToNode(hasTestTag(tag))
        } finally {
            rule.mainClock.autoAdvance = false
        }
    }

    private fun model() = CsModel.parse(bridge.cs.model.value)

    private fun exists(tag: String) = rule.onAllNodes(hasTestTag(tag)).fetchSemanticsNodes().isNotEmpty()

    private fun hasNodeText(text: String) = rule.onAllNodes(hasText(text, substring = true)).fetchSemanticsNodes().isNotEmpty()

    private var shown by mutableStateOf(true)

    private fun host(section: CsSection = CsSection.CONTROLS) {
        rule.setContent {
            back = LocalOnBackPressedDispatcherOwner.current?.onBackPressedDispatcher
            MaterialTheme(colorScheme = darkColorScheme()) {
                Surface(Modifier.fillMaxSize()) { if (shown) Tab(section) }
            }
        }
    }

    @Composable
    private fun Tab(section: CsSection) {
        val json by bridge.cs.model.collectAsState()
        val cs = remember(json) { CsModel.parse(json) }
        ControlSurfacesSettingsTab(section, bridge, SettingsSaveCoordinator.of(bridge), cs)
    }

    /** A configured control the editor can change the function of. */
    private fun controlSlot(): Int = model().bindings.indexOfFirst { it.type in CsModel.TYPE_BUTTON..CsModel.TYPE_ENCODER }
        .also { check(it >= 0) { "the demo has no control" } }

    private fun describe(slot: Int): CsDescription =
        runBlocking { CsRules.parseDescription(bridge.cs.describe(CsRules.KIND_BINDING, slot, draftOf(model().bindings[slot]))) }.value!!

    private fun openEditor(slot: Int) {
        pump("list") { exists("cs.slot.$slot") }
        rule.onNodeWithTag("cs.slot.$slot").performClick()
        pump("editor rows") { exists(rowTag("type")) }
    }

    @Test
    fun an9ListShowsTheCoresSummaries() {
        host()
        val m = model()
        val slots = m.bindings.indices.filter { m.bindings[it].configured && !CsModel.isAuxType(m.bindings[it].type) }
        assertTrue("the demo has controls", slots.isNotEmpty())
        for (slot in slots) {
            val d = describe(slot)
            val words = listOf(d.title, d.summary).filter { it.isNotEmpty() }.joinToString(" · ")
            pump("summary of slot $slot: $words") { hasNodeText(words) }
        }
        // An IR receiver or display reads as itself, not "Volume · Adjust" (AN8's bug list)
        for (slot in slots.filter { m.bindings[it].type == CsModel.TYPE_IR || m.bindings[it].type == CsModel.TYPE_DISPLAY }) {
            assertTrue(describe(slot).summary.contains("Volume · Adjust").not())
        }
    }

    @Test
    fun an9EditTheFunctionOnThePickerPageThenApply() {
        host()
        val slot = controlSlot()
        openEditor(slot)
        val noun = describe(slot).rows.first { it.id == "noun" } as CsRow.Pick
        val target = noun.options.first { !it.disabled && it.value != noun.value }
        rule.onNodeWithTag(rowTag("noun")).performClick()
        pump("picker") { exists("cs.picker") }
        scrollTo("cs.picker", optionTag(target.value))
        rule.onNodeWithTag(optionTag(target.value)).performClick()
        pump("draft shows ${target.text}") { exists("cs.editor") && hasNodeText(target.text) }
        // Nothing reaches the device before Apply
        assertEquals(noun.value, model().bindings[slot].noun)
        rule.onNodeWithTag("cs.apply").performClick()
        pump("applied") { model().bindings[slot].noun == target.value }
        assertTrue("live, not yet saved", bridge.cs.unsaved.value)
        pump("draft clean") { !CsDrafts.of(bridge).isDirty(CsRules.KIND_BINDING, slot) }
    }

    @Test
    fun an9AddAppliesAtOnceAndOpensTheEditor() {
        host()
        val free = firstFreeSlot(model()) ?: error("the demo has no free slot")
        pump("add") { exists("cs.add") }
        rule.onNodeWithText("Add").performClick()
        pump("types") { exists(optionTag(CsModel.TYPE_BUTTON)) }
        rule.onNodeWithTag(optionTag(CsModel.TYPE_BUTTON)).performClick()
        pump("added") { model().bindings[free].type == CsModel.TYPE_BUTTON }
        pump("its editor") { exists("cs.editor") && exists(rowTag("noun")) }
    }

    @Test
    fun an9ARefusedApplyKeepsTheDraftAndSaysWhy() {
        host()
        val slot = controlSlot()
        openEditor(slot)
        rule.onNodeWithTag("cs.name").performTextInput("Desk")
        pump("dirty") { CsDrafts.of(bridge).isDirty(CsRules.KIND_BINDING, slot) }
        // The demo held in a running revert: a write settles busy, as a real device can
        executor.submit { ffiCore.csDemoBusy(true) }.get()
        rule.onNodeWithTag("cs.apply").performClick()
        pump("refusal shown") { exists("cs.error") }
        assertNotEquals("Desk", model().slotName(slot))
        assertTrue("the draft is kept", CsDrafts.of(bridge).isDirty(CsRules.KIND_BINDING, slot))
        executor.submit { ffiCore.csDemoBusy(false) }.get()
    }

    @Test
    fun an9AnErrorReplyFromTheRulesIsShown() {
        host()
        val slot = controlSlot()
        openEditor(slot)
        val b = model().bindings[slot]
        val editor = CsDrafts.of(bridge).editor(CsRules.KIND_BINDING, slot, draftOf(b), model().slotName(slot))
        rule.runOnUiThread { editor.change("noun", 9999) }
        pump("error shown") { exists("cs.error") && hasNodeText("does not offer") }
        assertEquals(b, model().bindings[slot])
    }

    @Test
    fun an9BackWithAnEditedDraftAsksAndTheDraftOutlivesThePage() {
        host()
        val slot = controlSlot()
        openEditor(slot)
        rule.onNodeWithTag("cs.name").performTextInput("Desk")
        pump("dirty") { CsDrafts.of(bridge).isDirty(CsRules.KIND_BINDING, slot) }
        // A frame shows the typed name before the page goes (the paused clock gives none by itself)
        pump("name shown") { hasNodeText("Desk") }
        // Rotation recreates the page: the draft is in the app-scoped store
        shown = false
        pump("gone") { !exists("cs.editor") }
        shown = true
        pump("back on the list") { exists("cs.slot.$slot") }
        rule.onNodeWithTag("cs.slot.$slot").performClick()
        pump("draft kept") { hasNodeText("Desk") }
        rule.runOnUiThread { back!!.onBackPressed() }
        pump("asks") { hasNodeText("Discard Changes?") }
        rule.onNodeWithTag("cs.discard").performClick()
        pump("list again") { exists("cs.slot.$slot") && !exists("cs.editor") }
        assertTrue(!CsDrafts.of(bridge).isDirty(CsRules.KIND_BINDING, slot))
    }

    @Test
    fun an9RemoveAsksFirst() {
        host()
        val slot = controlSlot()
        openEditor(slot)
        scrollTo("cs.editor", "cs.remove")
        rule.onNodeWithTag("cs.remove").performClick()
        pump("asks") { hasNodeText("The slot is freed") }
        assertTrue("nothing removed yet", model().bindings[slot].configured)
        rule.onNodeWithTag("cs.remove.confirm").performClick()
        pump("removed") { !model().bindings[slot].configured }
    }

    @Test
    fun an9AuxRowsOpenTheirPageInsteadOfTheOldDialog() {
        host(CsSection.AUX)
        val m = model()
        val aux = m.bindings.indexOfFirst { CsModel.isAuxType(it.type) }
        assertTrue("the demo has an aux output", aux >= 0)
        pump("aux list") { exists("cs.slot.$aux") }
        rule.onNodeWithTag("cs.slot.$aux").performClick()
        pump("aux editor") { exists("cs.editor") && exists(rowTag("type")) }
    }

    private companion object {
        const val SERIAL = "DEMO0000DSPI0001"
    }
}
