package se.bofr.dspi.feature.matrix

import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.flow.launchIn
import kotlinx.coroutines.flow.onEach
import kotlinx.coroutines.test.StandardTestDispatcher
import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.advanceTimeBy
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import se.bofr.dspi.bridge.BridgeEvent
import se.bofr.dspi.bridge.DspiBridgeImpl
import se.bofr.dspi.core.DspiConstants
import se.bofr.dspi.core.DspiException
import se.bofr.dspi.fake.FakeDspiCore
import se.bofr.dspi.ui.CompactDrag
import se.bofr.dspi.ui.compactDelayDisplay
import se.bofr.dspi.ui.compactDelayEditText
import se.bofr.dspi.ui.parseCompactDelay

/** MatrixMixerState.from + MatrixActions against DspiBridgeImpl over the V32 FakeDspiCore. */
@OptIn(ExperimentalCoroutinesApi::class)
class MatrixActionsTest {
    private class Rig(val core: FakeDspiCore, val bridge: DspiBridgeImpl, val actions: MatrixActions)

    private suspend fun TestScope.rig(overview: Boolean = false): Rig {
        val core = FakeDspiCore.seeded()
        val d = StandardTestDispatcher(testScheduler)
        val bridge = DspiBridgeImpl(core, backgroundScope, ffi = d, main = d, clock = { testScheduler.currentTime })
        bridge.attachDevice(42, core.serial, DspiConstants.VENDOR_ID)
        runCurrent()
        val actions = MatrixActions(bridge, backgroundScope, isOverviewMode = { overview })
        core.calls.clear()
        return Rig(core, bridge, actions)
    }

    private fun Rig.calls(name: String) = core.calls.filter { it.startsWith("$name(") }

    @Test
    fun stateFromTheFixture() = runTest {
        val r = rig()
        val s = MatrixMixerState.from(r.bridge, activeInputChannels = 2, connected = true)
        assertEquals("RP2350", s.platformName)
        assertTrue(s.is8ch)
        assertEquals(9, s.visibleOutputs.size)
        assertEquals("OUT1", s.visibleOutputs[0].descriptor)
        assertEquals("OUT9", s.visibleOutputs[8].descriptor)
        assertEquals("SPDIF 1 L", s.visibleOutputs[0].name)
        assertEquals("PDM", s.visibleOutputs[8].name)
        assertEquals(8, s.pdmOutputIndex)
        // WP AN41: the core's line for the RP2350 (bridge.maxDelayMs), not the Mac's 85
        assertEquals(42f, s.maxDelay)
        // V32 fixture: stereo active, upmixer off -> 2 rows
        assertEquals(listOf("USB 1", "USB 2"), s.matrixInputs.map { it.name })
        assertEquals("#4A8FE3", s.matrixInputs[0].color)
        assertEquals(s.routes.size, s.matrixInputs.size)
        assertEquals(r.bridge.matrixRouting(0, 0), s.route(0, 0).isConnected)
        assertEquals(r.bridge.outputEnabled(0), s.output(0).enabled)
        // 8 active channels -> 8 rows with the 3/4, 5/6 pair dividers
        val s8 = MatrixMixerState.from(r.bridge, activeInputChannels = 8, connected = true)
        assertEquals(8, s8.matrixInputs.size)
        assertEquals(8, s8.inputTrimDB.size)
    }

    @Test
    fun upmixerAddsDerivedRows() = runTest {
        val r = rig()
        r.bridge.setUpmixParam(DspiConstants.UPMIX_PARAM_ENABLED, 1f)
        r.bridge.setUpmixParam(DspiConstants.UPMIX_PARAM_SURROUND_MODE, 1f)
        runCurrent()
        val s = MatrixMixerState.from(r.bridge, activeInputChannels = 2, connected = true)
        assertEquals(listOf("USB 1", "USB 2", "C", "Ls", "Rs"), s.matrixInputs.map { it.name })
        assertEquals("Upmix Centre", s.matrixInputs[2].fullName)
        r.bridge.setUpmixParam(DspiConstants.UPMIX_PARAM_SURROUND_MODE, 0f)
        runCurrent()
        assertEquals(3, MatrixMixerState.from(r.bridge, 2, true).matrixInputs.size)
    }

    @Test
    fun toggleRouteKeepsGainAndPolarity() = runTest {
        val r = rig()
        r.bridge.setMatrixRoute(1, 4, false, -3f, true)
        runCurrent()
        r.actions.toggleRoute(1, 4)
        runCurrent()
        assertEquals("setMatrixRoute(1, 4, true, -3.0, true)", r.calls("setMatrixRoute").last())
        assertTrue(r.bridge.matrixRouting(1, 4))
        r.actions.toggleInvert(1, 4)
        runCurrent()
        assertFalse(r.bridge.matrixInvert(1, 4))
    }

    /** Audit T2: connect then INV before the first write is published keeps both. */
    @Test
    fun connectThenInvertInOneFrameKeepsBoth() = runTest {
        val r = rig()
        r.bridge.setMatrixRoute(1, 4, false, -3f, false)
        runCurrent()
        r.actions.toggleRoute(1, 4)
        r.actions.toggleInvert(1, 4)
        runCurrent()
        assertTrue(r.bridge.matrixRouting(1, 4))
        assertTrue(r.bridge.matrixInvert(1, 4))
        assertEquals(-3f, r.bridge.matrixGain(1, 4), 1e-6f)
    }

    /** review-stalestate #2: a gain commit right after a connect keeps the connection. */
    @Test
    fun aGainCommitRightAfterConnectKeepsTheConnection() = runTest {
        val r = rig()
        r.bridge.setMatrixRoute(1, 4, false, 0f, false)
        runCurrent()
        r.actions.toggleRoute(1, 4)
        r.actions.matrixGainCommit(1, 4, -2f) // same frame: the composed route still says disconnected
        runCurrent()
        assertTrue(r.bridge.matrixRouting(1, 4))
        assertEquals(-2f, r.bridge.matrixGain(1, 4), 1e-6f)
    }

    @Test
    fun matrixGainDragIsThrottledThenCommittedOnce() = runTest {
        val r = rig()
        check(r.bridge.matrixRouting(0, 0) && !r.bridge.matrixInvert(0, 0))
        val rev0 = r.bridge.stateRevision.value
        r.actions.matrixGainLive(0, 0, -1f) // goes out at once
        r.actions.matrixGainLive(0, 0, -2f)
        r.actions.matrixGainLive(0, 0, -3f) // latest wins
        runCurrent()
        advanceTimeBy(MatrixActions.THROTTLE_MS + 1)
        runCurrent()
        assertEquals(
            listOf("setMatrixRoute(0, 0, true, -1.0, false)", "setMatrixRoute(0, 0, true, -3.0, false)"),
            r.calls("setMatrixRoute")
        )
        // sendOnly: the device follows, no stateChanged during the drag
        assertEquals(rev0, r.bridge.stateRevision.value)
        advanceTimeBy(100) // the throttle goes idle
        runCurrent()
        r.core.calls.clear()
        r.actions.matrixGainLive(0, 0, -4f)
        r.actions.matrixGainLive(0, 0, -5f) // pending, dropped by the commit
        r.actions.matrixGainCommit(0, 0, -6f)
        runCurrent()
        advanceTimeBy(100)
        runCurrent()
        assertEquals(
            listOf("setMatrixRoute(0, 0, true, -4.0, false)", "setMatrixRoute(0, 0, true, -6.0, false)"),
            r.calls("setMatrixRoute")
        )
        assertEquals(-6f, r.bridge.matrixGain(0, 0))
        assertTrue(r.bridge.stateRevision.value > rev0) // the commit emits
    }

    @Test
    fun outputGainRoundedDelayNot() = runTest {
        val r = rig()
        r.actions.outputGainLive(2, -3.04f)
        r.actions.outputGainCommit(2, -3.26f)
        r.actions.outputDelayLive(2, 9.6f)
        r.actions.outputDelayCommit(2, 12.4f)
        runCurrent()
        assertEquals(listOf("setOutputGain(2, -3.0)", "setOutputGain(2, -3.3)"), r.calls("setOutputGain"))
        // WP AN40: the delay goes out as given; the core quantises to 0.0001 ms
        assertEquals(listOf("setOutputDelay(2, 9.6)", "setOutputDelay(2, 12.4)"), r.calls("setOutputDelay"))
        assertEquals(-3.3f, r.bridge.outputGainDB(2), 1e-6f)
        assertEquals(12.4f, r.bridge.outputDelayMS(2), 0f)
    }

    /** WP AN40: 0.5 and 1.5 ms typed into the delay fields apply exactly (no whole-ms rounding). */
    @Test
    fun wpAn40HalfMillisecondDelaysApplyExactly() = runTest {
        val r = rig()
        r.actions.outputDelayCommit(0, parseCompactDelayOrFail("0.5"))
        r.actions.outputDelayCommit(1, parseCompactDelayOrFail("1,5 ms"))
        runCurrent()
        assertEquals(listOf("setOutputDelay(0, 0.5)", "setOutputDelay(1, 1.5)"), r.calls("setOutputDelay"))
        assertEquals(0.5f, r.bridge.outputDelayMS(0), 0f)
        assertEquals(1.5f, r.bridge.outputDelayMS(1), 0f)
        val s = MatrixMixerState.from(r.bridge, activeInputChannels = 2, connected = true)
        assertEquals("0.5 ms", compactDelayDisplay(s.output(0).delayMS))
        assertEquals("1.5 ms", compactDelayDisplay(s.output(1).delayMS))
    }

    /**
     * WP AN40: fractional delays set elsewhere (the device, another client,
     * Link) reach the matrix as they are and survive unrelated writes,
     * including an untouched pass through the delay editor.
     */
    @Test
    fun wpAn40FractionalDelayFromTheDeviceSurvivesUnrelatedWrites() = runTest {
        val r = rig()
        r.core.simulateRemote { st ->
            st.copy(outputDelayMs = st.outputDelayMs.mapIndexed { i, v -> mapOf(2 to 0.25f, 3 to 1.5f)[i] ?: v })
        }
        advanceTimeBy(100)
        runCurrent()
        assertEquals(0.25f, r.bridge.outputDelayMS(2), 0f)
        val before = MatrixMixerState.from(r.bridge, activeInputChannels = 2, connected = true)
        assertEquals("0.25 ms", compactDelayDisplay(before.output(2).delayMS))

        r.core.calls.clear()
        r.actions.outputGainCommit(2, -4f)
        r.actions.setOutputMute(2, true)
        r.actions.outputDelayCommit(4, 3f)
        r.actions.matrixGainCommit(0, 0, -6f)
        // OK on the editor without typing: the text it starts with parses back to the same value
        r.actions.outputDelayCommit(3, parseCompactDelayOrFail(compactDelayEditText(before.output(3).delayMS)))
        runCurrent()
        assertEquals(listOf("setOutputDelay(4, 3.0)", "setOutputDelay(3, 1.5)"), r.calls("setOutputDelay"))
        assertEquals(0.25f, r.bridge.outputDelayMS(2), 0f)
        assertEquals(1.5f, r.bridge.outputDelayMS(3), 0f)
        assertEquals(0.25f, r.core.getState().outputDelayMs[2], 0f)
        assertEquals(1.5f, r.core.getState().outputDelayMs[3], 0f)
    }

    private fun parseCompactDelayOrFail(text: String) = checkNotNull(parseCompactDelay(text, 42f))

    @Test
    fun inputTrimLiveIsSendOnlyAndCommits() = runTest {
        val r = rig()
        r.actions.inputTrimLive(4, -2f)
        r.actions.inputTrimCommit(4, -2.5f)
        runCurrent()
        assertEquals(listOf("setInputPreamp(4, -2.0)", "setInputPreamp(4, -2.5)"), r.calls("setInputPreamp"))
        assertEquals(-2.5f, r.bridge.inputPreampDB(4), 0f)
    }

    @Test
    fun enableOutsideOverviewModeLeavesGraphVisibilityAlone() = runTest {
        val r = rig(overview = false)
        val before = r.bridge.channelVisible(4)
        r.actions.setOutputEnable(2, !r.bridge.outputEnabled(2))
        runCurrent()
        assertEquals(before, r.bridge.channelVisible(4))
    }

    @Test
    fun enableInOverviewModeFollowsOnlyWhenTheDeviceTookIt() = runTest {
        val r = rig(overview = true)
        r.actions.setOutputEnable(2, true)
        runCurrent()
        assertTrue(r.bridge.outputEnabled(2))
        assertTrue(r.bridge.channelVisible(4))
        // PDM on while OUT3 runs on Core 1: the device refuses, visibility stays
        r.bridge.setChannelVisible(10, false)
        r.actions.setOutputEnable(8, true)
        runCurrent()
        assertFalse(r.bridge.outputEnabled(8))
        assertFalse(r.bridge.channelVisible(10))
    }

    @Test
    fun pdmConflictAlertAndSwitchToPdm() = runTest {
        val r = rig(overview = true)
        r.actions.setOutputEnable(2, true)
        runCurrent()
        val s = MatrixMixerState.from(r.bridge, 2, true)
        r.actions.requestOutputEnable(s, 8)
        runCurrent()
        assertEquals(PendingConflict(8, true, PDM_REASON), r.actions.pendingConflict.value)
        r.core.calls.clear()
        r.actions.resolveConflict(r.actions.pendingConflict.value!!, s.eqWorkerRange, s.pdmOutputIndex)
        assertEquals(null, r.actions.pendingConflict.value)
        runCurrent()
        // macOS switchToPDM: every EQ-worker output off, then PDM on
        assertEquals((2..7).map { "setOutputEnable($it, false)" } + "setOutputEnable(8, true)", r.calls("setOutputEnable"))
        assertTrue(r.bridge.outputEnabled(8))
        assertFalse(r.bridge.outputEnabled(2))
        assertTrue(r.bridge.outputEnabled(0))
        assertFalse(r.bridge.channelVisible(4))
        assertTrue(r.bridge.channelVisible(10))
    }

    @Test
    fun switchFromPdm() = runTest {
        val r = rig()
        r.actions.setOutputEnable(8, true)
        runCurrent()
        assertTrue(r.bridge.outputEnabled(8))
        val s = MatrixMixerState.from(r.bridge, 2, true)
        r.actions.requestOutputEnable(s, 4)
        runCurrent()
        assertEquals(PendingConflict(4, false, OUT5_REASON), r.actions.pendingConflict.value)
        r.core.calls.clear()
        r.actions.resolveConflict(PendingConflict(4, false), s.eqWorkerRange, s.pdmOutputIndex)
        runCurrent()
        assertEquals(listOf("setOutputEnable(8, false)", "setOutputEnable(4, true)"), r.calls("setOutputEnable"))
        assertTrue(r.bridge.outputEnabled(4))
        assertFalse(r.bridge.outputEnabled(8))
    }

    @Test
    fun dismissAndRenameStateLiveInTheController() = runTest {
        val r = rig()
        val s = MatrixMixerState.from(r.bridge, 2, true)
        r.actions.requestOutputEnable(s, 0) // enabled -> disable at once, no alert
        assertEquals(null, r.actions.pendingConflict.value)
        r.actions.startRename(3)
        assertEquals(3, r.actions.renamingOutput.value)
        r.actions.cancelRename()
        assertEquals(null, r.actions.renamingOutput.value)
    }

    @Test
    fun directAndClearRouting() = runTest {
        val r = rig()
        r.actions.applyDirectRouting()
        runCurrent()
        assertTrue(r.bridge.matrixRouting(0, 0))
        assertTrue(r.bridge.matrixRouting(1, 1))
        assertFalse(r.bridge.matrixRouting(0, 1))
        r.actions.clearAllRoutes()
        runCurrent()
        for (i in 0 until 8) for (o in 0 until 9) assertFalse(r.bridge.matrixRouting(i, o))
    }

    @Test
    fun renameTrimsAndIgnoresEmpty() = runTest {
        val r = rig()
        r.actions.startRename(0)
        r.actions.commitRename(0, "   ")
        assertEquals(null, r.actions.renamingOutput.value)
        r.actions.commitRename(0, "  Left Tweeter ")
        runCurrent()
        assertEquals(1, r.calls("setChannelName").size)
        assertEquals("Left Tweeter", r.bridge.channelName(2))
    }

    @Test
    fun copyPasteParameters() = runTest {
        val r = rig()
        assertFalse(r.actions.canPaste())
        r.actions.copyChannelParams(0)
        assertTrue(r.actions.canPaste())
        r.actions.pasteChannelParams(1)
        runCurrent()
    }

    // ═══ WP AN41: delay line and Core 1 from the core ═══

    private fun TestScope.events(r: Rig): List<BridgeEvent> {
        val seen = mutableListOf<BridgeEvent>()
        r.bridge.events.onEach { seen += it }.launchIn(backgroundScope)
        runCurrent()
        return seen
    }

    /** The demo board becomes an RP2040 (a remote change), so the core's line is 21 ms. */
    private fun TestScope.becomeRp2040(r: Rig) {
        r.core.simulateRemote { it.copy(platformId = 0) }
        advanceTimeBy(100)
        runCurrent()
    }

    @Test
    fun wpAn41TheFieldOffersTheCoresLine() = runTest {
        val r = rig()
        assertEquals(42f, r.bridge.maxDelayMs)
        val s = MatrixMixerState.from(r.bridge, 2, true)
        assertEquals(r.bridge.maxDelayMs, s.maxDelay)
        // a drag stops at the line on whole ms (SL1), typing past it gives the line
        assertEquals(42f, CompactDrag.delay(40f, 400f, s.maxDelay), 0f)
        assertEquals(42f, parseCompactDelay("85", s.maxDelay)!!, 0f)
        // typed values below the line stay exact
        assertEquals(41.25f, parseCompactDelay("41.25", s.maxDelay)!!, 0f)
        becomeRp2040(r)
        assertEquals(21f, MatrixMixerState.from(r.bridge, 2, true).maxDelay)
    }

    @Test
    fun wpAn41ARefusedDelayCarriesTheCoresWordsAndTheFieldReverts() = runTest {
        val r = rig()
        r.actions.outputDelayCommit(2, 12.5f)
        runCurrent()
        val seen = events(r)
        r.core.calls.clear()
        // a value past the line (a field still holding an older board's limit): the core refuses it
        r.actions.outputDelayCommit(2, 50f)
        runCurrent()
        val failed = seen.filterIsInstance<BridgeEvent.CommandFailed>().single()
        assertEquals("setOutputDelay", failed.command)
        val e = failed.error as DspiException.InvalidArgument
        assertEquals("out.delay: 50 ms is out of range; this device (RP2350) holds at most 42 ms", e.msg)
        // nothing changed: the field shows the device's value again
        assertEquals(12.5f, r.bridge.outputDelayMS(2), 0f)
        assertEquals(12.5f, MatrixMixerState.from(r.bridge, 2, true).output(2).delayMS, 0f)
        assertEquals(12.5f, r.core.getState().outputDelayMs[2], 0f)
        assertEquals(listOf("setOutputDelay(2, 50.0)"), r.calls("setOutputDelay"))
    }

    @Test
    fun wpAn41PasteTakesThisDevicesLine() = runTest {
        val r = rig()
        r.actions.outputDelayCommit(0, 40f)
        r.actions.setOutputMute(0, true)
        runCurrent()
        r.actions.copyChannelParams(0)
        becomeRp2040(r)
        val seen = events(r)
        r.core.calls.clear()
        r.actions.pasteChannelParams(1)
        runCurrent()
        assertEquals(emptyList<BridgeEvent>(), seen.filterIsInstance<BridgeEvent.CommandFailed>())
        assertEquals(listOf("setOutputDelay(1, 21.0)"), r.calls("setOutputDelay"))
        assertEquals(21f, r.bridge.outputDelayMS(1), 0f)
        // the rest of the paste is not cut short
        assertTrue(r.bridge.outputMuted(1))
        // a clip within the line is pasted exactly
        r.actions.outputDelayCommit(0, 12.25f)
        runCurrent()
        r.actions.copyChannelParams(0)
        r.actions.pasteChannelParams(1)
        runCurrent()
        assertEquals(12.25f, r.bridge.outputDelayMS(1), 0f)
    }

    @Test
    fun wpAn41EnableAsksTheCoreNotTheLayout() = runTest {
        val r = rig()
        r.actions.setOutputEnable(2, true)
        runCurrent()
        // a state read before OUT3-8 went off: the layout rule still sees a conflict
        val stale = MatrixMixerState.from(r.bridge, 2, true)
        assertTrue(stale.output(8).wouldConflict)
        for (o in 2..7) r.bridge.setOutputEnable(o, false)
        runCurrent()
        r.core.calls.clear()
        r.actions.requestOutputEnable(stale, 8)
        runCurrent()
        // the core says no conflict: enabled at once, no alert
        assertEquals(listOf("core1ConflictReason(8)"), r.calls("core1ConflictReason"))
        assertEquals(null, r.actions.pendingConflict.value)
        assertTrue(r.bridge.outputEnabled(8))
    }

    @Test
    fun wpAn41WithoutAnAnswerTheLayoutRuleStillAsks() = runTest {
        val r = rig()
        r.actions.setOutputEnable(2, true)
        runCurrent()
        val s = MatrixMixerState.from(r.bridge, 2, true)
        r.core.failNext("core1ConflictReason") { DspiException.Disconnected() }
        r.core.calls.clear()
        r.actions.requestOutputEnable(s, 8)
        runCurrent()
        assertEquals(PendingConflict(8, true, null), r.actions.pendingConflict.value)
        assertEquals(emptyList<String>(), r.calls("setOutputEnable"))
    }

    @Test
    fun directRoutingShowsTheOutputsItEnablesOnlyInOverview() = runTest {
        for (overview in listOf(false, true)) {
            val r = rig(overview)
            r.bridge.setOutputEnable(1, false)
            runCurrent()
            r.bridge.setChannelVisible(3, false)
            r.actions.applyDirectRouting()
            runCurrent()
            assertTrue(r.bridge.outputEnabled(1))
            assertEquals("overview=$overview", overview, r.bridge.channelVisible(3))
        }
    }

    companion object {
        /** The core's words (write_param.rs core1_conflict_reason) for the demo's 9 outputs. */
        const val PDM_REASON =
            "Output 9 is the PDM output, which shares Core 1 with outputs 3-8, and one of them is enabled: disable outputs 3-8 first"
        const val OUT5_REASON = "Output 5 shares Core 1 with the PDM output (output 9), which is enabled: disable the PDM output first"
    }
}
