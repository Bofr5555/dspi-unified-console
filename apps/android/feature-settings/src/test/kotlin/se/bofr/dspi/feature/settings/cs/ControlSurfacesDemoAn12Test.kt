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
import androidx.compose.ui.test.assert
import androidx.compose.ui.test.hasTestTag
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.isEnabled
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onRoot
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollToNode
import androidx.compose.ui.test.performTextReplacement
import androidx.compose.ui.test.printToString
import java.util.concurrent.Executors
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.asCoroutineDispatcher
import kotlinx.coroutines.cancel
import kotlinx.coroutines.launch
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
import se.bofr.dspi.ui.json.Json
import se.bofr.dspi.ui.json.int
import se.bofr.dspi.ui.json.list
import se.bofr.dspi.ui.json.long
import se.bofr.dspi.ui.json.obj

/**
 * WP AN12: Channel Groups, Macros and Auxiliary Outputs on the real core's demo
 * device (`open_demo`, CG16's FakeDevice surfaces: group 0 "Front Pair", macro 0
 * "Night" with two steps, aux outputs in slots 10 and 11, a button in 12 on the
 * first) through the real bridge and CG20's rules, at phone size. As AN9's test:
 * the host build of libdspi_ffi over JNA. No device.
 */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35], application = Application::class, qualifiers = "w360dp-h640dp-port-mdpi")
class ControlSurfacesDemoAn12Test {
    @get:Rule val rule = createComposeRule()

    private val ffiCore = FfiDspiCore()
    private val executor = Executors.newSingleThreadExecutor { r -> Thread(r, "an12-ffi").apply { isDaemon = true } }
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
    private lateinit var bridge: DspiBridgeImpl
    private var back: OnBackPressedDispatcher? = null

    private class Demo(private val core: DspiCoreApi) : DspiCoreApi by core {
        override fun scanDevices(): List<DeviceInfo> = listOf(DeviceInfo(SERIAL, 0))

        override fun selectDevice(serial: String) {
            if (serial != SERIAL) throw DspiException.Disconnected()
            if (!core.isConnected()) core.openDemo()
        }
    }

    @Before
    fun start() {
        bridge = DspiBridgeImpl(Demo(ffiCore), scope, ffi = executor.asCoroutineDispatcher(), appVersion = "an12")
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

    private fun scrollTo(list: String, tag: String) {
        rule.mainClock.autoAdvance = true
        try {
            rule.onNodeWithTag(list).performScrollToNode(hasTestTag(tag))
        } finally {
            rule.mainClock.autoAdvance = false
        }
    }

    private fun model() = CsModel.parse(bridge.cs.model.value)

    private fun running(json: String): Long? = Json.parseObject(json)?.obj("ext")?.long("macroRunning")

    private fun raw() = Json.parseObject(bridge.cs.model.value)!!

    /** The device's macro [m] steps' nouns, in order. */
    private fun macroNouns(m: Int): List<Int> = model().macros[m].steps.map { it.noun }

    private fun exists(tag: String) = rule.onAllNodes(hasTestTag(tag)).fetchSemanticsNodes().isNotEmpty()

    private fun hasNodeText(text: String) = rule.onAllNodes(hasText(text, substring = true)).fetchSemanticsNodes().isNotEmpty()

    private var shown by mutableStateOf(true)

    private fun host(section: CsSection) {
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

    private fun open(tag: String, until: String) {
        pump("list: $tag") { exists(tag) }
        rule.onNodeWithTag(tag).performClick()
        pump("page: $until") { exists(until) }
    }

    private fun click(tag: String) {
        scrollTo("cs.editor", tag)
        rule.onNodeWithTag(tag).performClick()
    }

    // ── Channel Groups (S8, S9) ──

    @Test
    fun an12GroupMemberToggleAndRenameApplyLive() {
        host(CsSection.GROUPS)
        val before = model().groups[0]
        open("cs.group.0", rowTag("kind"))
        assertTrue(hasNodeText("Front Pair"))
        rule.onNodeWithTag("cs.name").performTextReplacement("Mains")
        // Output 3 joins: the core's member row, nothing sent before Apply
        click(rowTag("member.2"))
        pump("draft has the member") { CsRecordDrafts.of(bridge).isGroupDirty(0) }
        assertEquals(before, model().groups[0])
        rule.onNodeWithTag("cs.apply").performClick()
        pump("applied") {
            check(!exists("cs.error")) { CsRecordDrafts.of(bridge).openGroup(0)?.state?.value.toString() }
            model().groups[0].memberMask == before.memberMask or 0b100
        }
        assertEquals("Mains", model().groups[0].name)
        assertTrue("live, not yet saved", bridge.cs.unsaved.value)
        pump("draft clean") { !CsRecordDrafts.of(bridge).isGroupDirty(0) }
    }

    @Test
    fun an12AddGroupHoldsApplyUntilItHasAMember() {
        host(CsSection.GROUPS)
        pump("add") { exists("cs.group.new") }
        rule.onNodeWithTag("cs.group.new").performClick()
        pump("new group page") { exists(rowTag("kind")) && exists(rowTag("noMembers")) }
        rule.onNodeWithTag("cs.apply").assert(isEnabled().not())
        click(rowTag("member.0"))
        pump("a member: Apply allowed") { !exists(rowTag("noMembers")) }
        rule.onNodeWithTag("cs.apply").performClick()
        pump("applied") { model().groups[1].configured }
        assertEquals("Group 2", model().groups[1].name)
        assertEquals(1L, model().groups[1].memberMask)
    }

    @Test
    fun an12RemoveGroupAsksThenClearsIt() {
        host(CsSection.GROUPS)
        open("cs.group.0", rowTag("kind"))
        click("cs.remove")
        pump("asks") { hasNodeText("cleared on the device") }
        assertTrue("nothing removed yet", model().groups[0].configured)
        rule.onNodeWithTag("cs.remove.confirm").performClick()
        pump("removed") { !model().groups[0].configured }
        pump("list again") { exists("cs.group.add") && !exists("cs.editor") }
    }

    @Test
    fun an12ARefusedGroupApplyKeepsTheDraftAndSaysWhy() {
        host(CsSection.GROUPS)
        open("cs.group.0", rowTag("kind"))
        click(rowTag("member.3"))
        pump("dirty") { CsRecordDrafts.of(bridge).isGroupDirty(0) }
        executor.submit { ffiCore.csDemoBusy(true) }.get()
        rule.onNodeWithTag("cs.apply").performClick()
        pump("refusal shown") { exists("cs.error") }
        assertEquals(0b0011L, model().groups[0].memberMask)
        assertTrue("the draft is kept", CsRecordDrafts.of(bridge).isGroupDirty(0))
        executor.submit { ffiCore.csDemoBusy(false) }.get()
    }

    @Test
    fun an12AnErrorReplyFromTheGroupRulesIsShown() {
        host(CsSection.GROUPS)
        open("cs.group.0", rowTag("kind"))
        val editor = CsRecordDrafts.of(bridge).openGroup(0)!!
        rule.runOnUiThread { editor.change("kind", 99) }
        pump("error shown") { exists("cs.error") }
        assertEquals(0b0011L, model().groups[0].memberMask)
    }

    @Test
    fun an12GroupDraftOutlivesThePageAndBackAsks() {
        host(CsSection.GROUPS)
        open("cs.group.0", rowTag("kind"))
        click(rowTag("member.4"))
        pump("dirty") { CsRecordDrafts.of(bridge).isGroupDirty(0) }
        shown = false
        pump("gone") { !exists("cs.editor") }
        shown = true
        pump("list marks the edit") { exists("cs.group.0") && hasNodeText("Edited") }
        open("cs.group.0", rowTag("kind"))
        assertTrue(CsRecordDrafts.of(bridge).isGroupDirty(0))
        rule.runOnUiThread { back!!.onBackPressed() }
        pump("asks") { hasNodeText("Discard Changes?") }
        rule.onNodeWithTag("cs.discard").performClick()
        pump("list again") { exists("cs.group.0") && !exists("cs.editor") }
        assertTrue(!CsRecordDrafts.of(bridge).isGroupDirty(0))
    }

    // ── Macros (S10, S11, S12) ──

    @Test
    fun an12MacroStepsReorderAndApplyKeepsEveryField() {
        host(CsSection.MACROS)
        val before = macroNouns(0)
        assertEquals(2, before.size)
        open("cs.macro.0", stepTag(1))
        // The core's summaries, not words of ours
        val first = runBlocking {
            CsRules.parseDescription(bridge.cs.describe(CsRecords.KIND_STEP, 0, CsRecords.macro(bridge.cs.model.value, 0)!!.steps[0]))
        }.value!!
        pump("summary") { hasNodeText(first.summary) }
        rule.onNodeWithTag("${stepTag(1)}.up").performClick()
        pump("dirty") { CsRecordDrafts.of(bridge).isMacroDirty(0) }
        assertEquals("nothing sent before Apply", before, macroNouns(0))
        val delayBefore = raw().list("macros")!!.let { (it[0] as Map<*, *>)["steps"] as List<*> }.map { (it as Map<*, *>)["preDelay"] }
        rule.onNodeWithTag("cs.apply").performClick()
        pump("applied") { macroNouns(0) == before.reversed() }
        val delayAfter = raw().list("macros")!!.let { (it[0] as Map<*, *>)["steps"] as List<*> }.map { (it as Map<*, *>)["preDelay"] }
        assertEquals("the delays moved with their steps", delayBefore.reversed(), delayAfter)
        pump("draft clean") { !CsRecordDrafts.of(bridge).isMacroDirty(0) }
    }

    @Test
    fun an12AddStepOpensItsPageAndItsRowsComeFromTheCore() {
        host(CsSection.MACROS)
        open("cs.macro.0", "cs.step.add")
        click("cs.step.new")
        pump("step page") { exists(rowTag("preDelay")) && hasNodeText("Step 3") }
        // A delay typed in natural units (seconds), refused by nothing: the core takes it
        val editor = CsRecordDrafts.of(bridge).openMacro(0)!!
        rule.runOnUiThread { editor.changeStep(2, "preDelay", 1.5) }
        pump("delay in the draft") { (Json.parseObject(editor.state.value.draft.steps[2])?.int("preDelay") ?: 0) > 0 }
        rule.onNodeWithTag("cs.step.done").performClick()
        pump("macro page") { exists(stepTag(2)) }
        rule.onNodeWithTag("cs.apply").performClick()
        pump("three steps") { model().macros[0].steps.size == 3 }
        assertNotEquals(0, model().macros[0].steps[2].preDelay)
    }

    @Test
    fun an12RemoveAStepThenTheMacro() {
        host(CsSection.MACROS)
        open("cs.macro.0", stepTag(1))
        rule.onNodeWithTag("${stepTag(0)}.remove").performClick()
        pump("one step left") { !exists(stepTag(1)) }
        rule.onNodeWithTag("cs.apply").performClick()
        pump("applied") { model().macros[0].steps.size == 1 }
        click("cs.remove")
        pump("asks") { hasNodeText("cleared on the device") }
        rule.onNodeWithTag("cs.remove.confirm").performClick()
        pump("removed") { !model().macros[0].configured }
    }

    @Test
    fun an12AddMacroIsADraftUntilApply() {
        host(CsSection.MACROS)
        pump("add") { exists("cs.macro.new") }
        rule.onNodeWithTag("cs.macro.new").performClick()
        pump("new macro page") { exists("cs.step.add") && hasNodeText("Macro 2") }
        assertTrue("not on the device yet", !model().macros[1].configured)
        click("cs.step.new")
        pump("step page") { exists(rowTag("preDelay")) }
        rule.onNodeWithTag("cs.step.done").performClick()
        pump("macro page") { exists(stepTag(0)) }
        rule.onNodeWithTag("cs.apply").performClick()
        pump("applied") { model().macros[1].configured && model().macros[1].steps.size == 1 }
        assertEquals("Macro 2", model().macros[1].name)
    }

    @Test
    fun an12RunFromTheListFiresTheStoredMacro() {
        host(CsSection.MACROS)
        pump("list") { exists("cs.macro.run.0") }
        // Every model the bridge publishes is seen: the run may end within a few status reads
        var sawRunning = false
        val watch = scope.launch { bridge.cs.model.collect { if (running(it) == 0L) sawRunning = true } }
        rule.onNodeWithTag("cs.macro.run.0").performClick()
        pump("ran") { sawRunning }
        watch.cancel()
        // The bridge follows the run on the main looper's clock, which only idleFor moves
        pump("it ends") {
            shadowOf(Looper.getMainLooper()).idleFor(java.time.Duration.ofMillis(200))
            running(bridge.cs.model.value) == 255L
        }
    }

    // ── Auxiliary Outputs (S13, S14) ──

    @Test
    fun an12AuxSwitchAndLevelAreLive() {
        host(CsSection.AUX)
        pump("aux list") { exists("cs.aux.switch.10") && exists("cs.aux.level.11") }
        val was = model().auxState[10] != 0
        rule.onNodeWithTag("cs.aux.switch.10").performClick()
        pump("switched") { (model().auxState[10] != 0) != was }
        assertTrue("never a flash write", !bridge.cs.unsaved.value)
        // The level's op, as the slider's release sends it
        SettingsSaveCoordinator.of(bridge).csOp(CsRecords.auxLevelOp(11, 25.0))
        pump("level") { CsRecords.auxPercent(model().auxLevel[11]).let { it in 24.5..25.5 } }
        pump("shown") { hasNodeText("Level 25 %") }
    }

    @Test
    fun an12AuxPageShowsLiveRowsAndWhatDrivesIt() {
        host(CsSection.AUX)
        open("cs.slot.10", rowTag("drivenBy"))
        assertTrue("live switch on top", exists("cs.aux.live") && exists("cs.aux.switch.10"))
        // The button in slot 12 toggles this output: the core names it
        scrollTo("cs.editor", rowTag("driver.12"))
        assertTrue(exists(rowTag("driver.12")))
    }

    @Test
    fun an12AddOutputAppliesAtOnceAndOpensItsPage() {
        host(CsSection.AUX)
        val free = firstFreeSlot(model()) ?: error("the demo has no free slot")
        pump("add") { exists("cs.aux.new") }
        rule.onNodeWithTag("cs.aux.new").performClick()
        pump("aux types") { exists(optionTag(CsModel.TYPE_AUX_PWM)) }
        assertTrue("only aux types", !exists(optionTag(CsModel.TYPE_BUTTON)))
        rule.onNodeWithTag(optionTag(CsModel.TYPE_AUX_PWM)).performClick()
        pump("added") { model().bindings[free].type == CsModel.TYPE_AUX_PWM }
        pump("its page") { exists("cs.aux.live") && exists(rowTag("startLevel")) }
    }

    // ── The pure parts ──

    @Test
    fun an12SnapshotRecordsKeepEveryStepField() {
        val model = bridge.cs.model.value
        val m = CsRecords.macro(model, 0)!!
        assertEquals("Night", m.name)
        assertEquals(2, m.steps.size)
        // flags and step survive (CsModel drops them): the op sends what the device has
        val step = Json.parseObject(m.steps[0])!!
        assertTrue(step.containsKey("flags") && step.containsKey("step"))
        val op = Json.parseObject(CsRecords.macroOp(0, m)!!)!!
        assertEquals("macro", op["op"])
        assertEquals(CsRecords.maxSteps(model), raw().int("maxSteps"))
        val (g, name) = CsRecords.group(model, 0)!!
        assertEquals("Front Pair", name)
        assertEquals(3L, Json.parseObject(g)!!.long("memberMask"))
        assertTrue(CsRecords.sameRecord("""{"memberMask":3,"name":"","targetKind":2}""", g))
        assertEquals(listOf(2, 1, 3), CsRecords.moved(listOf(1, 2, 3), 1, -1))
        assertEquals(listOf(1, 2, 3), CsRecords.moved(listOf(1, 2, 3), 2, 1))
        assertEquals(null, CsRecords.group(model, 99))
    }

    private companion object {
        const val SERIAL = "DEMO0000DSPI0001"
    }
}
