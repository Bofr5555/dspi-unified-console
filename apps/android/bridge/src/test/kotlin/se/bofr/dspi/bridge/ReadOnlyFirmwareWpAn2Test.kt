package se.bofr.dspi.bridge

import java.io.File
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import se.bofr.dspi.core.DspiConstants
import se.bofr.dspi.fake.DspStateJson
import se.bofr.dspi.fake.FakeDspiCore
import se.bofr.dspi.fake.demoDspState

/**
 * WP AN2: the bridge on what the real core reports for each firmware
 * (`fixtures/wire/`, from `crates/dspi-core/tests/wp_an2_read_only_firmware.rs`).
 */
@OptIn(ExperimentalCoroutinesApi::class)
class ReadOnlyFirmwareWpAn2Test {
    private fun fixture(name: String) = FakeDspiCore(DspStateJson.parse(File("../fixtures/wire/$name.json").readText(), demoDspState()))

    @Test
    fun wpAn2OlderAndNewerFirmwareIsShownAndNeverWritten() = runTest {
        val cases =
            mapOf(
                "v17_1_1_5_beta3" to "1.1.5",
                "v26_1_1_5_beta6" to "1.1.5",
                "v28_1_1_5" to "1.1.5",
                "v28_1_1_6_early_beta" to "1.1.6 early beta",
                "v30_1_1_6_beta3" to "1.1.6-beta3",
                "v31_dev" to "1.1.6-beta3",
                "v33_mock" to "1.1.7",
                "v15_unmapped" to "1.1.4",
                "v34_unmapped" to "1.1.8"
            )
        for ((name, version) in cases) {
            val core = fixture(name)
            val b = newBridge(core)
            attach(b, core)
            assertTrue(name, b.connected)
            assertFalse(name, b.writesAllowed.value)
            assertEquals(name, version, b.firmwareVersion)
            assertEquals(name, core.getState().formatVersion, b.state.value.formatVersion)
            assertEquals(name, -20f, b.masterVolumeDB, 0.01f)

            val mark = core.calls.size
            b.setPreamp(-1f)
            b.setMasterVolume(-3f)
            b.setUserMute(true)
            b.setBypass(true)
            runCurrent()
            val written = core.calls.drop(mark).filter { it.startsWith("set") }
            assertEquals(name, emptyList<String>(), written)
        }
    }

    @Test
    fun wpAn2V32FirmwareIsWritable() = runTest {
        for (name in listOf("stock_final_v32", "k340_v32")) {
            val core = fixture(name)
            val b = newBridge(core)
            attach(b, core)
            assertEquals(name, DspiConstants.COMPAT_OK, b.compat)
            assertTrue(name, b.writesAllowed.value)
            assertEquals(name, "1.1.6", b.firmwareVersion)
            assertEquals(name, "", b.compatMessage)
            b.setMasterVolume(-3f)
            runCurrent()
            assertEquals(name, 1, core.callCount("setMasterVolume"))
        }
    }
}
