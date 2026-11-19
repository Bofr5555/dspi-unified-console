package se.bofr.dspi.bridge

import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.advanceTimeBy
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import se.bofr.dspi.core.AlinkInfo
import se.bofr.dspi.core.AlinkStatus
import se.bofr.dspi.core.DeviceKind
import se.bofr.dspi.core.DevicePage
import se.bofr.dspi.core.DeviceProtocol
import se.bofr.dspi.core.DspiException
import se.bofr.dspi.core.ExtEnvelope
import se.bofr.dspi.core.FirmwareGeneration
import se.bofr.dspi.core.FirmwareGuardBoard
import se.bofr.dspi.core.FirmwareGuardBoardSource
import se.bofr.dspi.core.FirmwareLineage
import se.bofr.dspi.core.FirmwareTarget
import se.bofr.dspi.core.FirmwareUpdateVerdict
import se.bofr.dspi.core.ProtocolGeneration
import se.bofr.dspi.fake.FakeDspiCore

/**
 * WP PGAND: the bridge reads `device_protocol()` once per connection on the core's
 * thread and publishes it with the connection (main); the alink sender gets no DSP
 * reads (no baseline, no analyser, no meter poll) and its page reads `alink_info`
 * and `fetch_alink_status`; the update check passes through [CoreTools].
 */
@OptIn(ExperimentalCoroutinesApi::class)
class DeviceProtocolWpPgandTest {
    private val paged = DeviceProtocol(
        generation = ProtocolGeneration.PAGED,
        protocolMajor = 1,
        protocolMinor = 0,
        pages = listOf(DevicePage(0x01, 1), DevicePage(0xE0, 1)),
        extEnvelope = ExtEnvelope.TUNNEL,
        boardId = 0,
        boardName = "DSPi",
        deviceKind = DeviceKind.DSP
    )
    private val sender = paged.copy(
        generation = ProtocolGeneration.LEGACY,
        protocolMajor = 0,
        pages = emptyList(),
        extEnvelope = ExtEnvelope.LEGACY,
        deviceKind = DeviceKind.ALINK_SENDER
    )

    private class ProtocolCore(var protocol: DeviceProtocol?) : FakeDspiCore() {
        var protocolReads = 0
        var targetReads = 0
        val alink = AlinkInfo("sender", 2, 1, 1, 2, 8, 8, 0xFF, 2, 0L, 0, 0, false, false, false)
        val status = AlinkStatus("locked", 2, 1, 1, 8, 0, 0xFF, 7, 0, 123L, 1L, 0L, 0L, 0L, 0L, 0L, 0f, 0f, 0)

        override fun deviceProtocol(): DeviceProtocol {
            protocolReads++
            return protocol ?: throw DspiException.Disconnected()
        }

        override fun alinkInfo() = alink

        override fun fetchAlinkStatus() = status

        override fun firmwareTarget(): FirmwareTarget {
            targetReads++
            return FirmwareTarget(
                FirmwareGuardBoard(1200, FirmwareGuardBoardSource.EXT_CORE_VREG),
                0,
                FirmwareLineage.BOFR,
                FirmwareGeneration.LEGACY,
                DeviceKind.DSP // -- WP IMGKIND --
            )
        }

        override fun firmwareUpdateCheck(image: List<Int>, target: FirmwareTarget, typedConfirmation: String): FirmwareUpdateVerdict =
            throw IllegalStateException("boom")
    }

    @Test
    fun wpPgandTheProtocolIsReadOncePerConnectionAndPublished() = runTest {
        val core = ProtocolCore(paged)
        val b = newBridge(core)
        assertNull(b.deviceProtocol.value)
        assertTrue(attach(b, core))
        assertEquals(paged, b.deviceProtocol.value)
        // One read with each connection the bridge sets up (each `fetchAll`), none in between
        fun opens() = core.calls.count { it == "fetchAll" }
        val first = opens()
        assertTrue(first >= 1)
        assertEquals(first, core.protocolReads)
        // Polling and state changes do not read it again
        b.setUiVisible(true)
        advanceTimeBy(2_000)
        runCurrent()
        assertEquals(first, core.protocolReads)
        b.detachDevice(core.serial)
        runCurrent()
        assertNull("gone with the connection", b.deviceProtocol.value)
        assertTrue(attach(b, core))
        assertEquals(paged, b.deviceProtocol.value)
        assertTrue("read again for the new connection", core.protocolReads > first)
        assertEquals(opens(), core.protocolReads)
    }

    @Test
    fun wpPgandAProtocolThatCannotBeReadIsNull() = runTest {
        val core = ProtocolCore(null)
        val b = newBridge(core)
        assertTrue(attach(b, core))
        assertTrue(b.connected)
        assertNull(b.deviceProtocol.value)
    }

    @Test
    fun wpPgandTheAlinkSenderGetsNoDspReads() = runTest {
        val core = ProtocolCore(sender)
        val b = newBridge(core)
        b.setUiVisible(true)
        assertTrue(attach(b, core))
        assertEquals(DeviceKind.ALINK_SENDER, b.deviceProtocol.value?.deviceKind)
        advanceTimeBy(2_000)
        runCurrent()
        for (dsp in listOf("captureBaseline", "rtaStart", "fetchStatus")) {
            assertFalse("$dsp on the sender", core.calls.any { it.startsWith(dsp) })
        }
        val view = b.fetchAlinkSender()
        assertEquals(core.alink, view.info)
        assertEquals(core.status, view.status)
        assertEquals("", view.error)
    }

    @Test
    fun wpPgandADspStillGetsItsReads() = runTest {
        val core = ProtocolCore(paged)
        val b = newBridge(core)
        b.setUiVisible(true)
        assertTrue(attach(b, core))
        advanceTimeBy(2_000)
        runCurrent()
        assertTrue(core.calls.any { it.startsWith("captureBaseline") })
        assertTrue(core.calls.any { it.startsWith("fetchStatus") })
    }

    @Test
    fun wpPgandTheUpdateCheckPassesThroughAndFailsClosed() = runTest {
        val core = ProtocolCore(paged)
        val b = newBridge(core)
        val target = b.coreTools.firmwareTarget()
        assertEquals(1, core.targetReads)
        assertEquals(FirmwareLineage.BOFR, target.lineage)
        val v = b.coreTools.firmwareUpdateCheck(byteArrayOf(1, 2), target, "")
        assertFalse(v.verdict.allowed)
        assertTrue(v.verdict.message, v.verdict.message.contains("boom"))
        // A core without the call: no board, so no image
        assertEquals(CoreTools.NO_TARGET, newBridge(FakeDspiCore()).coreTools.firmwareTarget())
        // -- review PG-D2 -- the write gate fails closed too when the core cannot be asked
        val gate = newBridge(FakeDspiCore()).coreTools.firmwareWriteCheck(byteArrayOf(1, 2), CoreTools.UNKNOWN_BOARD, "x")
        assertFalse(gate.verdict.allowed)
    }

    @Test
    fun wpPgandTestDoublesDefaultToNothingKnown() = runTest {
        val core = FakeDspiCore()
        val b = newBridge(core)
        val double = object : DspiBridge by b {
            override val deviceProtocol get() = super<DspiBridge>.deviceProtocol

            override suspend fun fetchAlinkSender() = super<DspiBridge>.fetchAlinkSender()
        }
        assertNull(double.deviceProtocol.value)
        assertEquals(AlinkSenderView(), double.fetchAlinkSender())
    }
}
