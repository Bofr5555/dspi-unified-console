package se.bofr.dspi.bridge

import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import se.bofr.dspi.core.ConfirmOp
import se.bofr.dspi.core.PresetStatus
import se.bofr.dspi.fake.FakeDspiCore

/**
 * WP P1AN2 in the bridge, on the fake: P1TK's tokens behind the bridge's own
 * (the requirement turned on, a core token minted for each confirmed gated
 * call, none for the ungated ones), the classified list unchanged, the limits
 * profile document, and a linked pair's preamp as the core's one-step pair
 * write. The real core's side (limits, lock, modes, groups) is the app's
 * InstallerWpP1an2Test on the demo device.
 */
@OptIn(ExperimentalCoroutinesApi::class)
class InstallerWpP1an2Test {
    @Test
    fun wpP1an2TheBridgeTurnsTheCoresTokensOnAndMintsOnePerConfirmedCall() = runTest {
        val core = FakeDspiCore()
        val b = newBridge(core)
        runCurrent()
        assertTrue("requirement on", core.confirmationRequired)
        attach(b, core)
        val ops = b.dangerousOps
        assertEquals(PresetStatus.OK, b.savePreset(ops.confirm(DangerousOp.SAVE_PRESET), 1))
        b.copyPreset(ops.confirm(DangerousOp.COPY_PRESET), 1, 2)
        b.deletePreset(ops.confirm(DangerousOp.DELETE_PRESET), 2)
        b.saveParams(ops.confirm(DangerousOp.SAVE_PARAMS))
        b.factoryReset(ops.confirm(DangerousOp.FACTORY_RESET))
        b.saveOutputConfig(ops.confirm(DangerousOp.SAVE_OUTPUT_CONFIG))
        b.configFiles.importConfiguration(ops.confirm(DangerousOp.CONFIG_IMPORT), "{}", volumes = true, hardware = false)
        b.saveMasterVolume(ops.confirm(DangerousOp.SAVE_MASTER_VOLUME)) // not gated in the core: no token
        runCurrent()
        assertEquals(
            listOf(
                ConfirmOp.SAVE_PRESET,
                ConfirmOp.COPY_PRESET,
                ConfirmOp.DELETE_PRESET,
                ConfirmOp.SAVE_PARAMS,
                ConfirmOp.FACTORY_RESET,
                ConfirmOp.SAVE_OUTPUT_CONFIG,
                ConfirmOp.CONFIG_IMPORT
            ),
            core.mintedTokens
        )
        // The calls reached the device as before (the fake runs the plain call for the confirmed one)
        assertTrue(core.calls.any { it.startsWith("savePreset(1") })
        assertTrue(core.calls.any { it.startsWith("configImport") })
    }

    @Test
    fun wpP1an2EveryGatedOpOfTheCoreHasItsTokenAndTheOthersNone() {
        val mapped = DangerousOp.entries.associateWith { it.confirmOp }
        assertNull(mapped[DangerousOp.SAVE_MASTER_VOLUME])
        assertNull(mapped[DangerousOp.CS_SAVE])
        assertNull(mapped[DangerousOp.HARDWARE_APPLY])
        // The core's ops a bridge call makes (VREG_SAVE: the calibration's own; FIRMWARE_FLASH: Android flashes by itself)
        assertEquals(
            ConfirmOp.entries.toSet() - ConfirmOp.VREG_SAVE - ConfirmOp.FIRMWARE_FLASH,
            mapped.values.filterNotNull().toSet()
        )
    }

    @Test
    fun wpP1an2TheLimitsDraftRoundTripsThroughItsDocument() {
        val d = LimitsDraft(
            outputs = listOf(OutputLimits(2, maxGainDb = 3f, highpassMinHz = 1500f, highpassMinSlopeDb = 24, limiterCeilingDb = -3f)),
            maxPeqBoostDb = 6f,
            masterVolumeMaxDb = -10f,
            lockedPages = listOf("front"),
            note = "Tweeters on output 3",
            contact = "installer@example.org"
        )
        val text = d.toDocument("SERIAL1")
        assertTrue(text, text.contains("\"kind\":\"limits\""))
        assertTrue(text, text.contains("\"serial\":\"SERIAL1\""))
        assertEquals(d, LimitsDraft.fromDocument(text))
        // An empty output row is dropped; a cleared field leaves the others
        val cleared = d.withOutput(OutputLimits(2)).withOutput(OutputLimits(0, maxGainDb = 0f))
        assertEquals(listOf(0), cleared.outputs.map { it.output })
        assertNull(LimitsDraft.fromDocument("{}"))
    }

    @Test
    fun wpP1an2ALinkedPairsPreampIsThePairWrite() = runTest {
        val core = FakeDspiCore()
        val b = newBridge(core)
        attach(b, core)
        b.setInputLinked(0, true, 0)
        runCurrent()
        val mark = core.calls.size
        b.setInputPreamp(0, -2f)
        runCurrent()
        // -- WP PAIRBAND -- the fake records the pair write as one call (one step), as the core's group
        val writes = core.calls.drop(mark).filter { it.startsWith("setInputPreamp") || it.startsWith("setInputPairPreamp") }
        assertEquals(listOf("setInputPairPreamp(0, -2.0)"), writes)
        assertEquals(-2f, b.inputPreampDB(0))
        assertEquals(-2f, b.inputPreampDB(1))
    }

    @Test
    fun wpP1an2TheStaleLineNeedsTheCoresFlag() {
        val s = BridgeStatus()
        assertEquals(false, s.stale)
        assertEquals(0L, s.ageMs)
    }
}
