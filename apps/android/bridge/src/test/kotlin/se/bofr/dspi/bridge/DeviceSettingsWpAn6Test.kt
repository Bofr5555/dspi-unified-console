package se.bofr.dspi.bridge

import java.io.File
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.advanceTimeBy
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import se.bofr.dspi.core.DspiConstants
import se.bofr.dspi.fake.DspStateJson
import se.bofr.dspi.fake.FakeDspiCore
import se.bofr.dspi.fake.demoDspState

/** WP AN6: device name, source names, latency, core voltage, and what read-only firmware shows. */
@OptIn(ExperimentalCoroutinesApi::class)
class DeviceSettingsWpAn6Test {
    private fun fixture(name: String) = FakeDspiCore(DspStateJson.parse(File("../fixtures/wire/$name.json").readText(), demoDspState()))

    @Test
    fun wpAn6NamesRoundTrip() = runTest {
        val core = FakeDspiCore()
        val b = newBridge(core)
        assertFalse(b.deviceSettings.names.value.deviceSupported)
        attach(b, core)
        val s = b.deviceSettings
        assertTrue(s.names.value.deviceSupported && s.names.value.sourcesSupported)

        assertTrue(s.setDeviceName("Bench"))
        assertEquals("Bench", s.names.value.device)
        assertEquals("Bench", core.getDeviceName())

        assertTrue(s.setSourceName(1, "Turntable"))
        assertEquals("Turntable", s.names.value.source(1))
        val option = b.inputSources.first { it.id == 1 }
        assertEquals("Turntable", option.name)
        assertEquals("S/PDIF", option.builtIn)

        assertTrue(s.setSourceName(1, ""))
        assertEquals("S/PDIF", b.inputSources.first { it.id == 1 }.name)
    }

    @Test
    fun wpAn6ARenameElsewhereArrivesWithoutAStateFlag() = runTest {
        val core = FakeDspiCore()
        val b = newBridge(core)
        attach(b, core)
        core.simulateRename(0xFF, "Elsewhere")
        core.simulateRename(0, "Desk")
        advanceTimeBy(1_000)
        runCurrent()
        assertEquals("Elsewhere", b.deviceSettings.names.value.device)
        assertEquals("Desk", b.inputSources.first { it.id == 0 }.name)
    }

    @Test
    fun wpAn6HiddenWhenAbsent() = runTest {
        val core = FakeDspiCore().apply {
            namesSupported = false
            latencySupported = false
            coreVregSupported = false
        }
        val b = newBridge(core)
        attach(b, core)
        val s = b.deviceSettings
        s.refreshLatency()
        runCurrent()
        assertFalse(s.names.value.deviceSupported || s.names.value.sourcesSupported)
        assertFalse(s.latency.value.supported)
        assertFalse(s.coreVoltage.value.supported)
        assertEquals(0, core.callCount("fetchCoreVreg"))
        assertEquals("USB", b.inputSources.first().name)
    }

    @Test
    fun wpAn6TheDeviceGoingAwayClearsThem() = runTest {
        val core = FakeDspiCore().apply { coreVregSupported = true }
        val b = newBridge(core)
        attach(b, core)
        val s = b.deviceSettings
        s.setDeviceName("Bench")
        s.refreshLatency()
        runCurrent()
        assertTrue(s.coreVoltage.value.supported && s.latency.value.supported)
        b.detachDevice(core.serial)
        runCurrent()
        assertEquals(DeviceNames(), s.names.value)
        assertEquals(OutputLatencyState(), s.latency.value)
        assertEquals(CoreVoltageState(), s.coreVoltage.value)
    }

    @Test
    fun wpAn6AnotherUnitLeavingKeepsTheRows() = runTest {
        val core = FakeDspiCore()
        val b = newBridge(core)
        attach(b, core)
        b.deviceSettings.refreshLatency()
        runCurrent()
        b.detachDevice("some-other-serial")
        runCurrent()
        assertTrue(b.connected)
        assertTrue(b.deviceSettings.latency.value.supported)
    }

    @Test
    fun wpAn6ThePickerRenames() = runTest {
        val core = FakeDspiCore()
        val b = newBridge(core)
        attach(b, core)
        val s = b.deviceSettings
        s.renameSource(2, "  Mixer ")
        runCurrent()
        assertEquals("Mixer", core.getInputSourceName(2))
        assertEquals(1, s.names.value.stamp(2))
        assertEquals(0, s.names.value.stamp(1))
        val before = core.callCount("setInputSourceName")
        s.renameSource(2, "Mixer")
        runCurrent()
        assertEquals(before, core.callCount("setInputSourceName"))
    }

    @Test
    fun wpAn6NamesAreCleanedLikeTheDeviceStoresThem() {
        assertEquals("abc", DeviceNames.clean("aåb\nc"))
        assertEquals(31, DeviceNames.clean("x".repeat(40)).length)
    }

    @Test
    fun wpAn6LatencyWaitsForTheDevice() = runTest {
        val core = FakeDspiCore()
        val b = newBridge(core)
        attach(b, core)
        val s = b.deviceSettings
        s.refreshLatency()
        runCurrent()
        assertEquals(5, s.latency.value.requestedId)
        assertEquals(7, s.latency.value.options.size)

        core.latencyPendingReads = 3
        assertEquals(LatencyApply.APPLIED, s.setOutputLatency(3))
        assertEquals(3, s.latency.value.requestedId)
        assertEquals(1, core.callCount("setOutputLatency"))

        core.latencyRefusesChange = true
        assertEquals(LatencyApply.KEPT, s.setOutputLatency(4))
        assertEquals(3, s.latency.value.requestedId)

        core.latencyRefusesChange = false
        core.latencyPendingReads = 1_000
        assertEquals(LatencyApply.NOT_YET_APPLIED, s.setOutputLatency(6))
        assertEquals("never sent again", 3, core.callCount("setOutputLatency"))

        core.latencyPendingReads = 0
        core.failNext("setOutputLatency")
        assertEquals(LatencyApply.NOT_SENT, s.setOutputLatency(7))
    }

    @Test
    fun wpAn6AFailedReadKeepsTheLatency() = runTest {
        val core = FakeDspiCore()
        val b = newBridge(core)
        attach(b, core)
        val s = b.deviceSettings
        s.refreshLatency()
        runCurrent()
        core.failNext("fetchOutputLatency", times = 100)
        s.refreshLatency()
        runCurrent()
        assertTrue(s.latency.value.supported)
    }

    @Test
    fun wpAn6CoreVoltageIsReadAndRefreshed() = runTest {
        val core = FakeDspiCore().apply { coreVregSupported = true }
        val b = newBridge(core)
        attach(b, core)
        val s = b.deviceSettings
        runCurrent()
        assertTrue(s.coreVoltage.value.supported)
        assertEquals(1200, s.coreVoltage.value.status?.millivolts)
        val reads = core.callCount("fetchCoreVreg")
        s.refreshCoreVoltage()
        assertEquals(reads + 1, core.callCount("fetchCoreVreg"))
        core.failNext("fetchCoreVreg", times = 100)
        s.refreshCoreVoltage()
        assertNotNull("a failed read keeps the last status", s.coreVoltage.value.status)
    }

    @Test
    fun wpAn6TheDemoDeviceHasEveryRow() = runTest {
        val core = FakeDspiCore.demo()
        val b = newBridge(core)
        attach(b, core)
        runCurrent()
        assertTrue(b.deviceSettings.coreVoltage.value.supported)
        assertTrue(b.deviceSettings.names.value.deviceSupported)
        core.close()
    }

    // ── Read-only firmware ──

    @Test
    fun wpAn6ReadOnlyFirmwareRefusesRowWrites() = runTest {
        val core = fixture("v28_1_1_5")
        val b = newBridge(core)
        attach(b, core)
        val s = b.deviceSettings
        s.refreshLatency()
        runCurrent()
        assertTrue("a read-only device still shows them", s.names.value.deviceSupported && s.latency.value.supported)
        assertFalse(s.setDeviceName("x"))
        assertFalse(s.setSourceName(0, "x"))
        assertEquals(LatencyApply.NOT_SENT, s.setOutputLatency(3))
        s.renameSource(0, "x")
        runCurrent()
        assertEquals(0, core.callCount("setDeviceName") + core.callCount("setInputSourceName") + core.callCount("setOutputLatency"))
        assertEquals(0, b.guardedCore.trips)
    }

    private data class Expect(
        val name: String,
        val multi: Boolean,
        val spdif: Int,
        val adatOut: Boolean,
        val adatIn: Boolean,
        val limiter: Boolean,
        val upmix: Boolean
    )

    @Test
    fun wpAn6CapsFollowTheWireFormatOnReadOnlyFirmware() = runTest {
        val cases = listOf(
            Expect("v17_1_1_5_beta3", multi = false, spdif = 1, adatOut = true, adatIn = false, limiter = false, upmix = false),
            Expect("v26_1_1_5_beta6", multi = true, spdif = 3, adatOut = true, adatIn = true, limiter = false, upmix = true),
            Expect("v28_1_1_5", multi = true, spdif = 4, adatOut = true, adatIn = true, limiter = false, upmix = true),
            Expect("v31_dev", multi = true, spdif = 4, adatOut = true, adatIn = true, limiter = false, upmix = true),
            Expect("stock_final_v32", multi = true, spdif = 4, adatOut = true, adatIn = true, limiter = true, upmix = true)
        )
        for (c in cases) {
            val core = fixture(c.name)
            val b = newBridge(core)
            attach(b, core)
            val caps = b.hardwareCaps
            assertTrue(c.name, caps.inputSourceSupported && caps.lgSoundSyncSupported && caps.i2sInputSupported)
            assertEquals(c.name, c.multi, caps.multiSpdifSupported)
            assertEquals(c.name, c.spdif, caps.spdifInputCount)
            assertEquals(c.name, c.multi, caps.i2sClockModeSupported)
            assertEquals(c.name, c.multi, caps.i2sClockPinModeSupported)
            assertEquals(c.name, c.adatOut, caps.adatOutSupported)
            assertEquals(c.name, c.adatIn, caps.adatInputSupported)
            assertEquals(c.name, c.limiter, caps.limiterSupported)
            assertEquals(c.name, c.upmix, b.upmixSupported)
        }
    }

    @Test
    fun wpAn6UpmixIsV25FromTheFirmwareAndRp2350Only() = runTest {
        fun at(version: Int, platform: Int = 1): FakeDspiCore {
            val state = demoDspState().copy(formatVersion = version, platformId = platform, compat = DspiConstants.COMPAT_FIRMWARE_TOO_OLD)
            return FakeDspiCore(state)
        }
        for ((core, expected) in listOf(at(24) to false, at(25) to true, at(26) to true, at(25, platform = 0) to false)) {
            val b = newBridge(core)
            attach(b, core)
            assertEquals(core.getState().formatVersion.toString(), expected, b.upmixSupported)
        }
    }

    @Test
    fun wpAn6AnImageTheCoreCannotMapShowsNoOptions() = runTest {
        val core = fixture("v15_unmapped")
        val b = newBridge(core)
        attach(b, core)
        val caps = b.hardwareCaps
        assertFalse(caps.inputSourceSupported || caps.i2sInputSupported || caps.multiSpdifSupported || caps.limiterSupported)
    }

    @Test
    fun wpAn6MetersKeepRunningOnReadOnlyFirmware() = runTest {
        val core = fixture("v28_1_1_5")
        val b = newBridge(core)
        attach(b, core)
        val polls = core.callCount("fetchStatus")
        advanceTimeBy(1_000)
        runCurrent()
        assertTrue("the status poll reads", core.callCount("fetchStatus") > polls)

        core.simulateClip(0)
        advanceTimeBy(200)
        runCurrent()
        assertTrue("the clip is latched", b.status.value.clipFlags != 0L)
        assertTrue("and cleared on the device, which the core allows", core.callCount("clearClips") > 0)
        assertEquals("not a write the guard had to stop", 0, b.guardedCore.trips)
    }

    @Test
    fun wpAn6MetersKeepRunningForAViewerAndPauseOnADroppedLink() = runTest {
        val core = FakeDspiCore()
        val b = newBridge(core)
        attach(b, core)
        b.setRemoteReadOnly(true)
        var polls = core.callCount("fetchStatus")
        advanceTimeBy(500)
        runCurrent()
        assertTrue(core.callCount("fetchStatus") > polls)

        b.setRemoteInterrupted(true)
        runCurrent()
        polls = core.callCount("fetchStatus")
        advanceTimeBy(500)
        runCurrent()
        assertEquals(polls, core.callCount("fetchStatus"))
        assertTrue("the mirror stays readable", b.hardwareCaps.inputSourceSupported)
    }

    @Test
    fun wpAn6CompatMessageMatchesQt() = runTest {
        val old = fixture("v28_1_1_5")
        val b = newBridge(old)
        attach(b, old)
        assertEquals(
            "This DSPi runs firmware 1.1.5, which is too old for this Console. Its settings are shown read-only; " +
                "features it lacks are hidden. Update it to firmware 1.1.6.",
            b.compatMessage
        )
        val new = fixture("v33_mock")
        val b2 = newBridge(new)
        attach(b2, new)
        assertEquals(
            "This DSPi runs firmware 1.1.7, which is newer than this Console understands. Its settings are shown read-only, " +
                "and some of its features may not be shown. Update the Console.",
            b2.compatMessage
        )
    }

    @Test
    fun wpAn6NoDeviceNoCaps() = runTest {
        val core = FakeDspiCore()
        val b = newBridge(core)
        assertEquals(HardwareCaps(), b.hardwareCaps)
        assertFalse(b.upmixSupported)
        assertNull(b.deviceSettings.coreVoltage.value.status)
    }
}
