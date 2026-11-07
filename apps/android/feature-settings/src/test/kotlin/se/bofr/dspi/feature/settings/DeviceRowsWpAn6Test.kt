package se.bofr.dspi.feature.settings

import java.io.File
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.StandardTestDispatcher
import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import se.bofr.dspi.bridge.DspiBridgeImpl
import se.bofr.dspi.bridge.OutputLatencyState
import se.bofr.dspi.core.CoreVregInfo
import se.bofr.dspi.core.DspiConstants
import se.bofr.dspi.core.OutputLatencyOption
import se.bofr.dspi.fake.FakeDspiCore

/** WP AN6: the staged rows on the demo core, through the save bar's Save and Revert. */
@OptIn(ExperimentalCoroutinesApi::class)
class DeviceRowsWpAn6Test {
    private class Rig(val core: FakeDspiCore, val bridge: DspiBridgeImpl, val c: SettingsSaveCoordinator)

    private suspend fun TestScope.rig(core: FakeDspiCore = FakeDspiCore()): Rig {
        val d = StandardTestDispatcher(testScheduler)
        val bridge = DspiBridgeImpl(core, backgroundScope, ffi = d, main = d, clock = { testScheduler.currentTime })
        bridge.attachDevice(42, core.serial, DspiConstants.VENDOR_ID)
        runCurrent()
        bridge.deviceSettings.refreshLatency()
        runCurrent()
        return Rig(core, bridge, SettingsSaveCoordinator(bridge, backgroundScope))
    }

    @Test
    fun wpAn6EveryRowStagesAndSaves() = runTest {
        val r = rig()
        val c = r.c
        assertFalse(c.hasPendingChanges())

        c.editDeviceName("Bench")
        c.editSourceName(1, "Turntable")
        c.editLatency(3)
        assertEquals("Bench", c.deviceNameText())
        assertEquals("Turntable", c.sourceNameText(1))
        assertEquals(3, c.latencyShown())
        assertTrue(c.hasPendingChanges())
        assertEquals(3, c.pendingSummary().size)
        assertEquals("nothing goes out before Save", 0, r.core.callCount("setDeviceName") + r.core.callCount("setOutputLatency"))

        val result = c.save(null, null)
        assertTrue(result!!.ok)
        assertEquals("Bench", r.core.getDeviceName())
        assertEquals("Turntable", r.core.getInputSourceName(1))
        assertEquals(3, r.bridge.deviceSettings.latency.value.requestedId)
        assertFalse(c.hasPendingChanges())
        assertEquals("Bench", c.deviceNameText())
    }

    @Test
    fun wpAn6RevertDropsTheDrafts() = runTest {
        val r = rig()
        r.c.editDeviceName("Bench")
        r.c.editSourceName(1, "Turntable")
        r.c.editLatency(3)
        r.c.revert(null)
        assertFalse(r.c.hasPendingChanges())
        assertEquals("", r.c.deviceNameText())
        assertEquals(5, r.c.latencyShown())
        assertEquals(0, r.core.callCount("setDeviceName"))
    }

    @Test
    fun wpAn6AnEditBackToTheStoredValueIsNotPending() = runTest {
        val r = rig()
        r.c.editDeviceName("Bench")
        r.c.editDeviceName("")
        r.c.editSourceName(0, "x")
        r.c.editSourceName(0, "")
        r.c.editLatency(3)
        r.c.editLatency(5)
        assertFalse(r.c.deviceDraftDirty())
    }

    @Test
    fun wpAn6AFailedSaveKeepsTheDraftAndSaysSo() = runTest {
        val r = rig()
        r.core.failNext("setDeviceName")
        r.c.editDeviceName("Bench")
        val result = r.c.save(null, null)
        assertEquals(listOf("The device did not take the name."), result!!.failures)
        assertEquals("Bench", r.c.pendingDeviceName())

        r.core.latencyRefusesChange = true
        r.c.editDeviceName("")
        r.c.editLatency(3)
        assertEquals(listOf("The device kept the previous latency."), r.c.save(null, null)!!.failures)
        assertEquals(3, r.c.pendingLatency())
    }

    @Test
    fun wpAn6ThePickersRenameSupersedesAnUnsavedEdit() = runTest {
        val r = rig()
        r.c.editSourceName(1, "Draft")
        r.bridge.deviceSettings.renameSource(1, "Picked")
        runCurrent()
        assertEquals("Picked", r.c.sourceNameText(1))
        assertNull(r.c.pendingSourceName(1))
        assertFalse(r.c.deviceDraftDirty())
    }

    @Test
    fun wpAn6AnotherDeviceDropsTheDrafts() = runTest {
        val r = rig()
        r.c.noteSelectedDevice(r.core.serial)
        r.c.editDeviceName("Bench")
        r.c.noteSelectedDevice("other")
        assertFalse(r.c.deviceDraftDirty())
    }

    @Test
    fun wpAn6AbsentFeaturesAreNeverPending() = runTest {
        val core = FakeDspiCore().apply {
            namesSupported = false
            latencySupported = false
        }
        val r = rig(core)
        r.c.editDeviceName("Bench")
        r.c.editSourceName(1, "Turntable")
        r.c.editLatency(3)
        assertFalse(r.c.deviceDraftDirty())
        assertNull(r.c.save(null, null))
        assertFalse(latencyVisible(r.bridge.deviceSettings.latency.value))
    }

    @Test
    fun wpAn6ReadOnlyFirmwareSavesNothing() = runTest {
        val core = FakeDspiCore(se.bofr.dspi.fake.demoDspState().copy(formatVersion = 28, compat = DspiConstants.COMPAT_FIRMWARE_TOO_OLD))
        val r = rig(core)
        r.c.editDeviceName("Bench")
        r.c.save(null, null)
        assertEquals(0, core.callCount("setDeviceName"))
    }

    private fun option(id: Int, us: Int, available: Boolean = true, effective: Int = us) =
        OutputLatencyOption(id, false, available, us, 1000, effective)

    @Test
    fun wpAn6LatencyRows() {
        assertEquals("0.67 ms", formatLatencyMs(667))
        assertEquals("4 ms", formatLatencyMs(4000))
        val options = listOf(option(3, 2000), option(5, 4000, effective = 3660))
        assertEquals(listOf(3 to "2 ms", 5 to "4 ms (default)"), latencyChoices(options, 5))
        assertEquals(9 to "-", latencyChoices(options, 9).last())
        assertTrue(latencyCaption(options, 5).contains("so it runs as 3.66 ms."))
        assertFalse(latencyCaption(options, 3).contains("runs as"))
        assertFalse(latencyVisible(OutputLatencyState(supported = true)))
        assertTrue(latencyVisible(OutputLatencyState(supported = true, options = options)))
    }

    @Test
    fun wpAn6CoreVoltageRows() {
        assertEquals("1.20 V", vregLabel(0x0D, "RP2350"))
        assertEquals("0.80 V", vregLabel(0x02, "RP2040"))
        assertEquals("code 0x10", vregLabel(0x10, "RP2040"))
        assertEquals("1.35 V", vregLabel(0x10, "RP2350"))
        assertEquals("1 h 2 min", formatUptime(3_725))
        val idle = CoreVregInfo(0x0D, 1200, 0, 0x0C, false, false, 2, false, 0, 0, 0, 59)
        val rows = coreVoltageRows(idle, "RP2350").toMap()
        assertEquals("1.20 V", rows["Running at"])
        assertEquals("Build default", rows["Stored"])
        assertEquals("On", rows["Automatic raising"])
        assertEquals("2", rows["Hangs this session"])
        assertEquals("None", rows["Last failed trial"])
        assertEquals("59 s", rows["Uptime"])
        assertFalse("Trial voltage" in rows)
        val busy = idle.copy(
            trialActive = true,
            trialVreg = 0x0E,
            storedSel = 0x0D,
            autoRaised = true,
            failedTrialVreg = 0x0C,
            failedTrialUptimeS = 61
        )
        val hot = coreVoltageRows(busy, "RP2350").toMap()
        assertEquals("1.20 V (trial)", hot["Running at"])
        assertEquals("1.25 V", hot["Trial voltage"])
        assertEquals("1.20 V, raised by the device", hot["Stored"])
        assertEquals("1.15 V, hung after 1 min 1 s", hot["Last failed trial"])
    }

    @Test
    fun wpAn6TheTextNamesNoHostDevice() {
        val text = File("src/main/kotlin/se/bofr/dspi/feature/settings/DeviceRows.kt").readText()
        for (word in listOf("computer", "phone", "tablet", "laptop", "desktop", "PC ")) {
            assertFalse(word, text.contains(word, ignoreCase = true))
        }
    }
}
