package se.bofr.dspi.app

import java.util.concurrent.Executors
import kotlinx.coroutines.CoroutineDispatcher
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.asCoroutineDispatcher
import kotlinx.coroutines.cancel
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.launch
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withContext
import kotlinx.coroutines.withTimeout
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Test
import se.bofr.dspi.bridge.BridgeEvent
import se.bofr.dspi.bridge.DangerousOp
import se.bofr.dspi.bridge.DspiBridge
import se.bofr.dspi.bridge.HistoryStep
import se.bofr.dspi.bridge.LimitsDraft
import se.bofr.dspi.bridge.OutputLimits
import se.bofr.dspi.core.AutoEqFilter
import se.bofr.dspi.core.AutoEqProfile
import se.bofr.dspi.core.DspiConstants
import se.bofr.dspi.core.DspiException
import se.bofr.dspi.core.PresetStatus
import se.bofr.dspi.feature.presets.snapshots.SnapshotsMode
import se.bofr.dspi.ffi.FfiDspiCore
import se.bofr.dspi.ui.UiModes

/**
 * WP P1AN2 on the real core's demo device (AN28's stack, outside Robolectric
 * as P1AN1's tests): with no PIN nothing is asked and every mode opens; a
 * limits profile clamps and reports, and a write past it is refused with the
 * core's notice; a PIN, the lock's cover and Lock now give the locked-user
 * state (only volume, mute and preset pass), a wrong PIN is refused and the
 * right one unlocks; tokens are required and a confirmed save passes; Clear
 * All, direct routing and protect all outputs are one undo step each; the
 * AutoEQ preview writes nothing and the apply changes only its pair.
 */
class InstallerWpP1an2Test {
    private class Stack(val bridge: DspiBridge, val core: FfiDspiCore, val main: CoroutineDispatcher, val scope: CoroutineScope)

    private fun withDemo(block: suspend Stack.() -> Unit) = runBlocking {
        val executor = Executors.newSingleThreadExecutor()
        val main = executor.asCoroutineDispatcher()
        val parent = CoroutineScope(SupervisorJob() + main)
        val ffi = FfiDspiCore()
        val session = startDemoSession(parent, "p1an2", main = main, core = DemoCore(ffi))
        try {
            withTimeout(15_000) {
                session.bridge.status.first { it.connected }
                session.bridge.writesAllowed.first { it }
            }
            Stack(session.bridge, ffi, main, parent).block()
        } finally {
            withContext(main) { session.close() }
            parent.cancel()
            executor.shutdown()
        }
    }

    private suspend fun until(what: String, ms: Long = 15_000, cond: () -> Boolean) {
        val end = System.currentTimeMillis() + ms
        while (!cond()) {
            if (System.currentTimeMillis() > end) throw AssertionError("timed out: $what")
            delay(20)
        }
    }

    private suspend fun <T> Stack.onMain(block: suspend () -> T): T = withContext(main) { block() }

    /** The bridge's refusals from now on. */
    private suspend fun Stack.refusals(): MutableList<BridgeEvent.CommandFailed> {
        val out = java.util.Collections.synchronizedList(mutableListOf<BridgeEvent.CommandFailed>())
        scope.launch { bridge.events.collect { if (it is BridgeEvent.CommandFailed) out += it } }
        delay(50)
        return out
    }

    private suspend fun Stack.modes(): UiModeModel = onMain { UiModeModel(bridge, InMemoryUiModeStore(), scope) }.also { m ->
        until("resolved") { m.choice.value != null }
    }

    @Test
    fun p1an2NoPinAsksNothingAndEveryModeOpens() = withDemo {
        val m = modes()
        // -- WP UAN -- owner D9: a phone opens in Advanced until Simple means something there (was Simple)
        assertEquals("the app opens in Advanced", UiModes.ADVANCED, m.mode.value)
        val c = m.choice.value!!
        assertFalse(c.needsPin)
        assertTrue(c.openModes.containsAll(listOf(UiModes.SIMPLE, UiModes.ADVANCED, UiModes.INSTALLER)))
        assertFalse(bridge.installer.lock.value?.pinSet ?: false)
        onMain { m.openDialog() }
        until("dialog") { m.dialog.value != null }
        onMain { m.select(UiModes.INSTALLER) }
        assertFalse("no PIN field with no PIN set", m.needsPin(m.dialog.value!!))
        onMain { m.confirm("") }
        until("installer") { m.mode.value == UiModes.INSTALLER && m.dialog.value == null }
        // Every write passes as without the feature
        onMain { bridge.setOutputGain(0, -7f) }
        until("written") { bridge.outputGainDB(0) == -7f }
        assertEquals(SnapshotsMode.SIMPLE, snapshotsModeOf(UiModes.SIMPLE))
        assertEquals(SnapshotsMode.SIMPLE, snapshotsModeOf(UiModes.LOCKED_USER))
        assertEquals(SnapshotsMode.ADVANCED, snapshotsModeOf(UiModes.INSTALLER))
        onMain { m.close() }
    }

    @Test
    fun p1an2ALimitBelowTheValueClampsReportsAndRefusesWithItsNotice() = withDemo {
        onMain {
            bridge.setMasterVolume(-3f)
            bridge.setOutputGain(1, 0f)
        }
        until("set") { bridge.masterVolumeDB == -3f && bridge.outputGainDB(1) == 0f }
        val draft = LimitsDraft(masterVolumeMaxDb = -10f).withOutput(OutputLimits(1, maxGainDb = -6f))
        val report = onMain { bridge.installer.applyLimits(draft) }.getOrThrow()
        assertTrue("held rows: $report", report.held.size >= 2)
        assertTrue(report.summary, report.summary.isNotEmpty())
        until("clamped") { bridge.masterVolumeDB == -10f && bridge.outputGainDB(1) == -6f }
        assertEquals(draft.masterVolumeMaxDb, bridge.installer.profile.value?.masterVolumeMaxDb)
        assertEquals("ok", bridge.installer.limits.value?.state)
        // A write past it: refused before anything is sent, in the core's words
        val refused = refusals()
        onMain { bridge.setMasterVolume(-2f) }
        until("refused") { refused.any { it.error is DspiException.LimitExceeded } }
        val words = coreRefusalText(bridge, refused.first { it.error is DspiException.LimitExceeded })
        assertTrue(words, words!!.startsWith("Outside the limits"))
        assertEquals(-10f, bridge.masterVolumeDB)
        onMain { bridge.installer.clearLimits() }.getOrThrow()
        until("cleared") { bridge.installer.profile.value == null }
    }

    @Test
    fun p1an2PinLockAndUnlock() = withDemo {
        val m = modes()
        val code = onMain { bridge.installer.setPin("4321") }.getOrThrow()
        assertTrue(code, Regex("[A-Z0-9]{5}(-[A-Z0-9]{5}){3}").matches(code))
        assertTrue(bridge.installer.lock.value!!.pinSet && bridge.installer.lock.value!!.unlocked)
        onMain { bridge.installer.cover(listOf("front")) }.getOrThrow()
        onMain { bridge.installer.lockNow() }.getOrThrow()
        until("locked user") { m.mode.value == UiModes.LOCKED_USER }
        assertTrue(bridge.installer.lock.value!!.locked)
        // Only volume, mute and preset pass
        val refused = refusals()
        onMain { bridge.setOutputGain(0, -5f) }
        until("refused") { refused.any { it.error is DspiException.InstallerLock } }
        assertTrue(coreRefusalText(bridge, refused.first())!!.startsWith("Locked by the installer"))
        onMain { bridge.setMasterVolume(-12f) }
        until("volume passes") { bridge.masterVolumeDB == -12f }
        // The dialog is the unlock; a wrong PIN is refused in the core's words
        onMain { m.openDialog() }
        until("dialog") { m.dialog.value != null }
        assertTrue(m.dialog.value!!.unlockOnly)
        assertTrue("the PIN is asked", m.needsPin(m.dialog.value!!))
        assertTrue("the scope line", m.dialog.value!!.scope.contains("console"))
        onMain { m.confirm("0000") }
        until("wrong") { m.dialog.value?.error?.isNotEmpty() == true }
        assertTrue(m.dialog.value!!.error, m.dialog.value!!.error.startsWith("Wrong PIN"))
        onMain { m.confirm("4321") }
        until("unlocked") { m.mode.value != UiModes.LOCKED_USER && m.dialog.value == null }
        assertEquals(UiModes.ADVANCED, m.mode.value) // -- WP UAN -- the phone's default (owner D9)
        onMain { bridge.setOutputGain(0, -5f) }
        until("written") { bridge.outputGainDB(0) == -5f }
        onMain { bridge.installer.clearPin() }.getOrThrow()
        assertFalse(bridge.installer.lock.value!!.pinSet)
        onMain { m.close() }
    }

    @Test
    fun p1an2TokensAreRequiredAndAConfirmedSavePasses() = withDemo {
        // -- WP P1TZ -- required by every core: no switch, and a save without a token is refused
        val token = onMain { bridge.dangerousOps.confirm(DangerousOp.SAVE_PRESET) }
        assertEquals(PresetStatus.OK, onMain { bridge.savePreset(token, 3) })
        try {
            core.savePresetConfirmed(3, null)
            fail("a save without a token is refused")
        } catch (e: DspiException.ConfirmationRequired) {
            assertEquals("missing", e.code)
        }
    }

    @Test
    fun p1an2ClearAllDirectRoutingAndProtectAllAreOneUndoEach() = withDemo {
        // Clear All: three bands back with one undo
        onMain { for (b in 0 until 3) bridge.setFilter(0, b, DspiConstants.FILTER_PEAKING, 500f + 500f * b, 3f, 1f) }
        until("bands") { (0 until 3).all { bridge.filterType(0, it) == DspiConstants.FILTER_PEAKING } }
        onMain { bridge.clearPeq(0) }
        until("cleared") { (0 until 3).all { bridge.filterType(0, it) == DspiConstants.FILTER_FLAT } }
        onMain { bridge.undo() }
        until("back") { (0 until 3).all { bridge.filterType(0, it) == DspiConstants.FILTER_PEAKING } }
        // Direct routing: one undo puts the matrix back
        onMain { bridge.setMatrixRoute(0, 1, true, -3f, false) }
        until("route") { bridge.matrixRouting(0, 1) }
        onMain { bridge.directRouting() }
        until("direct") { bridge.matrixRouting(0, 0) && !bridge.matrixRouting(0, 1) }
        onMain { bridge.undo() }
        until("undone") { bridge.matrixRouting(0, 1) && bridge.matrixGain(0, 1) == -3f }
        // Protect all outputs: every limiter on and the pairs linked; one undo, labelled, puts the groups and
        // thresholds back (the limiters stay on: the core never undoes switching one, P1PR)
        onMain { bridge.disableAllLimiters() }
        until("off") { (0 until bridge.numOutputChannels).none { bridge.limiterEnabled(it) } }
        onMain { bridge.setOutputGain(0, 0f) }
        until("gain") { bridge.outputGainDB(0) == 0f }
        assertTrue("S2 warning", 0 in onMain { bridge.installer.outputsWithoutLimiter() })
        val before = (0 until bridge.numOutputChannels).map { bridge.limiterLinkGroup(it) to bridge.limiterThreshold(it) }
        onMain { bridge.installer.protectAllOutputs() }.getOrThrow()
        until("on") { (0 until bridge.numOutputChannels).all { bridge.limiterEnabled(it) && bridge.limiterThreshold(it) == -1f } }
        assertTrue(onMain { bridge.installer.outputsWithoutLimiter() }.isEmpty())
        val step = onMain { bridge.undo() }
        assertEquals("Protect all outputs", (step as HistoryStep.Moved).move.entry?.label)
        // -- WP P1EFIX -- the core's note, shown in the snackbar: the limiters stay on
        assertTrue(historyMessage(false, step)!!.contains("The limiters stay on"))
        assertTrue((0 until bridge.numOutputChannels).all { bridge.limiterEnabled(it) })
        until("one undo") {
            (0 until bridge.numOutputChannels).map { bridge.limiterLinkGroup(it) to bridge.limiterThreshold(it) } == before
        }
    }

    @Test
    fun p1an2TheAutoEqPreviewWritesNothingAndApplyChangesOnlyItsPair() = withDemo {
        val profile = AutoEqProfile(
            "t",
            "Test Phones",
            -4f,
            listOf(AutoEqFilter("peaking", 1000f, 1.2f, 3f), AutoEqFilter("highShelf", 8000f, 0.7f, -2f))
        )
        val nIn = bridge.numInputChannels
        val outL = nIn + 2 // OUT 3-4
        val outApp = 2 + 2 // app id of OUT3
        val gainBefore = bridge.outputGainDB(2)
        val outBefore = bridge.filterType(outApp, 0) to bridge.filterFreq(outApp, 0)
        val inBefore = bridge.filterType(0, 0) to bridge.filterFreq(0, 0)
        val preview = onMain { bridge.installer.autoeqPreview(profile, outL, outL + 1) }.getOrThrow()
        assertEquals(2, preview.channels.size)
        assertTrue(preview.channels.first().curve.size > 1)
        assertNotNull(preview.suggestedTrimDb)
        assertEquals("nothing written", outBefore, bridge.filterType(outApp, 0) to bridge.filterFreq(outApp, 0))
        onMain { bridge.installer.autoeqApply(profile, outL, outL + 1) }.getOrThrow()
        until("applied") { bridge.filterType(outApp, 0) == DspiConstants.FILTER_PEAKING && bridge.filterFreq(outApp, 0) == 1000f }
        assertEquals("outputs keep their gain", gainBefore, bridge.outputGainDB(2))
        assertEquals("inputs untouched", inBefore, bridge.filterType(0, 0) to bridge.filterFreq(0, 0))
        onMain { bridge.undo() }
        until("one undo") { (bridge.filterType(outApp, 0) to bridge.filterFreq(outApp, 0)) == outBefore }
    }

    // -- WP P1EFIX -- the limits editor's ranges are the core's one table; the banner and backstop rows
    @Test
    fun p1efixRangesBannerAndBackstopAreTheCoresRows() = withDemo {
        val r = onMain { bridge.installer.fieldRanges() }
        assertEquals(8, r.size)
        assertEquals(20f, r["maxGainDb"]?.max)
        assertEquals(-128f, r["masterVolumeMaxDb"]?.min)
        assertEquals("ms", r["delayMaxMs"]?.unit)
        val banner = onMain { bridge.installer.notice(LOCK_BANNER_NOTICE_KEY) }
        assertEquals("Locked by the installer: only volume, mute and presets can be changed.", banner?.reason)
        // review p1e-a F1: the backstop is the row's reason alone, as the web and Qt
        assertEquals("The hub puts them back too if this console goes away.", onMain { bridge.snapshots.backstopWords() })
    }
}
