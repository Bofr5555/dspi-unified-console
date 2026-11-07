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

/**
 * WP AN11: the display's pages (S6, S7) on the real core's demo device (CG16's
 * FakeDevice surfaces: a graphic display with two pages, cycling) through the
 * real bridge and CG20's `page` and `display` records, at phone size. Same
 * harness as ControlSurfacesDemoAn9Test. No device.
 */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35], application = Application::class, qualifiers = "w360dp-h640dp-port-mdpi")
class ControlSurfacesDisplayAn11Test {
    @get:Rule val rule = createComposeRule()

    private val ffiCore = FfiDspiCore()
    private val executor = Executors.newSingleThreadExecutor { r -> Thread(r, "an11-ffi").apply { isDaemon = true } }
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
        bridge = DspiBridgeImpl(Demo(ffiCore), scope, ffi = executor.asCoroutineDispatcher(), appVersion = "an11")
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

    private fun display() = CsDisplay.parse(bridge.cs.model.value)

    private fun displaySlot(): Int = model().bindings.indexOfFirst { it.type == CsModel.TYPE_DISPLAY }
        .also { check(it >= 0) { "the demo has no display" } }

    private fun describePage(i: Int): CsDescription =
        runBlocking { CsRules.parseDescription(bridge.cs.describe(CsDisplay.KIND_PAGE, i, display().pages[i])) }.value!!

    private fun pageFlags(i: Int): Int = Json.parseObject(display().pages[i])?.int("flags") ?: 0

    private fun openDisplay() {
        host()
        val slot = displaySlot()
        pump("list") { exists("cs.slot.$slot") }
        rule.onNodeWithTag("cs.slot.$slot").performClick()
        pump("display page") { exists("cs.display") }
    }

    private fun openPage(i: Int) {
        openDisplay()
        pump("page row $i") { exists("cs.display.page.$i") }
        rule.onNodeWithTag("cs.display.page.$i").performClick()
        pump("page editor") { exists("cs.editor") && exists(rowTag("noun")) }
    }

    @Test
    fun an11DisplayShowsItsStateAndPagesAndPollsThePanel() {
        val before = display()
        assertEquals(listOf(0, 1), before.activePages)
        openDisplay()
        for (i in before.activePages) {
            val words = "${i + 1}. ${describePage(i).summary}"
            pump("page $i reads $words") { hasNodeText(words) }
        }
        pump("panel state") { exists("cs.display.state") && hasNodeText("Running") }
        // The panel cycles one page per status read on the demo: the page reads its state while open
        val seen = mutableSetOf<Int>()
        pump("status read again") {
            seen += display().status.page
            seen.size >= 2
        }
        pump("on-screen marker") { hasNodeText("On Screen") }
    }

    @Test
    fun an11WiringOpensTheBindingsRows() {
        openDisplay()
        pump("wiring row") { exists("cs.display.wiring") }
        rule.onNodeWithTag("cs.display.wiring").performClick()
        pump("wiring rows") { exists("cs.editor") && exists(rowTag("model")) && exists(rowTag("sda")) }
    }

    @Test
    fun an11EditAPageThenApply() {
        assertTrue("the demo's first page is large", pageFlags(0) and LARGE != 0)
        openPage(0)
        pump("large row") { exists(rowTag("large")) }
        rule.onNodeWithTag(rowTag("large")).performClick()
        pump("dirty") { CsDrafts.of(bridge).isDirty(CsDisplay.KIND_PAGE, 0) }
        // Nothing reaches the device before Apply
        assertTrue(pageFlags(0) and LARGE != 0)
        rule.onNodeWithTag("cs.apply").performClick()
        pump("applied") { pageFlags(0) and LARGE == 0 }
        pump("live, not yet saved") { bridge.cs.unsaved.value }
        pump("draft clean") { !CsDrafts.of(bridge).isDirty(CsDisplay.KIND_PAGE, 0) }
    }

    @Test
    fun an11SettingsApply() {
        assertEquals(1, Json.parseObject(display().config)?.int("mode"))
        openDisplay()
        pump("settings row") { exists("cs.display.settings") }
        rule.onNodeWithTag("cs.display.settings").performClick()
        pump("settings rows") { exists(rowTag("mode")) && exists(rowTag("dwell")) }
        rule.onNodeWithTag(rowTag("mode")).performClick()
        pump("menu") { hasNodeText("Cycle All") }
        rule.onNodeWithText("Cycle All").performClick()
        pump("dirty") { CsDrafts.of(bridge).isDirty(CsDisplay.KIND_DISPLAY, 0) }
        assertEquals(1, Json.parseObject(display().config)?.int("mode"))
        rule.onNodeWithTag("cs.apply").performClick()
        pump("applied") { Json.parseObject(display().config)?.int("mode") == 2 }
        pump("live, not yet saved") { bridge.cs.unsaved.value }
    }

    @Test
    fun an11AddAppliesAtOnceAndOpensThePage() {
        val free = display().firstFreePage ?: error("the demo has no free page")
        openDisplay()
        pump("add") { exists("cs.display.add") }
        rule.onNodeWithText("Add").performClick()
        assertTrue("saved to begin with", !bridge.cs.unsaved.value)
        pump("added") { display().isActive(free) }
        pump("live, not yet saved") { bridge.cs.unsaved.value }
        pump("its editor") { exists("cs.editor") && exists(rowTag("noun")) && hasNodeText("Page ${free + 1}") }
    }

    @Test
    fun an11RemovePageAsksFirst() {
        openPage(1)
        scrollTo("cs.editor", "cs.remove")
        rule.onNodeWithTag("cs.remove").performClick()
        pump("asks") { hasNodeText("The page leaves the panel") }
        assertTrue("nothing removed yet", display().isActive(1))
        rule.onNodeWithTag("cs.remove.confirm").performClick()
        pump("removed") { !display().isActive(1) }
        pump("live, not yet saved") { bridge.cs.unsaved.value }
        pump("back on the display page") { exists("cs.display") && !exists("cs.display.page.1") }
    }

    @Test
    fun an11AnErrorReplyAndARefusalAreShown() {
        openPage(0)
        val editor = CsDrafts.of(bridge).editor(CsDisplay.KIND_PAGE, 0, display().pages[0], "")
        // The rules' `error`: a function no page offers
        rule.runOnUiThread { editor.change("noun", 9999) }
        pump("error shown") { exists("cs.error") && hasNodeText("does not offer") }
        // The device's refusal: a write settles busy; the draft is kept
        pump("large row") { exists(rowTag("large")) }
        rule.onNodeWithTag(rowTag("large")).performClick()
        pump("dirty") { CsDrafts.of(bridge).isDirty(CsDisplay.KIND_PAGE, 0) && !exists("cs.error") }
        executor.submit { ffiCore.csDemoBusy(true) }.get()
        try {
            rule.onNodeWithTag("cs.apply").performClick()
            pump("refusal shown") { exists("cs.error") }
        } finally {
            executor.submit { ffiCore.csDemoBusy(false) }.get()
        }
        assertTrue("not applied", pageFlags(0) and LARGE != 0)
        assertTrue("the draft is kept", CsDrafts.of(bridge).isDirty(CsDisplay.KIND_PAGE, 0))
    }

    @Test
    fun an11BackWithAnEditedPageAsks() {
        openPage(0)
        pump("large row") { exists(rowTag("large")) }
        rule.onNodeWithTag(rowTag("large")).performClick()
        pump("dirty") { CsDrafts.of(bridge).isDirty(CsDisplay.KIND_PAGE, 0) }
        rule.runOnUiThread { back!!.onBackPressed() }
        pump("asks") { hasNodeText("Discard Changes?") }
        rule.onNodeWithTag("cs.discard").performClick()
        pump("display page again") { exists("cs.display") }
        assertTrue(!CsDrafts.of(bridge).isDirty(CsDisplay.KIND_PAGE, 0))
    }

    private companion object {
        const val SERIAL = "DEMO0000DSPI0001"

        /** `CsDisplayPage.flags`: a large value. */
        const val LARGE = 0x04
    }
}
