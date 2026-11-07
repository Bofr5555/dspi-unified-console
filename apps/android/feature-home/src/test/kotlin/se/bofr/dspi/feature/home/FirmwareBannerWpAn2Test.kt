package se.bofr.dspi.feature.home

import java.io.File
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.StandardTestDispatcher
import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Test
import se.bofr.dspi.bridge.DspiBridgeImpl
import se.bofr.dspi.core.DspiConstants
import se.bofr.dspi.fake.DspStateJson
import se.bofr.dspi.fake.FakeDspiCore
import se.bofr.dspi.fake.demoDspState

/**
 * WP AN2: the banner per `WIRE-FORMAT-COMPAT.md` 4.4, on what the real core
 * reports for each firmware (`fixtures/wire/`). It fires on the wire format
 * alone: the first update shows none on a K340 or on stock final firmware.
 */
@OptIn(ExperimentalCoroutinesApi::class)
class FirmwareBannerWpAn2Test {
    private val readOnly = "Until the firmware matches, this app is read-only: settings are shown but cannot be changed."
    private val unread =
        "This app cannot read that firmware's settings, so the values shown are defaults. Until the firmware matches, nothing can be changed."

    private suspend fun TestScope.banner(name: String): String? {
        val core = FakeDspiCore(DspStateJson.parse(File("../fixtures/wire/$name.json").readText(), demoDspState()))
        val d = StandardTestDispatcher(testScheduler)
        val b = DspiBridgeImpl(core, backgroundScope, ffi = d, main = d, clock = { testScheduler.currentTime })
        b.attachDevice(42, core.serial, DspiConstants.VENDOR_ID)
        runCurrent()
        return HomeModel.firmwareBannerMessage(b)
    }

    private fun older(version: String) = "This device runs firmware $version; DSPi Console expects 1.1.6-beta5. " +
        "Features that firmware does not have are hidden. $readOnly"

    @Test
    fun wpAn2NoBannerOnAK340OrStockFinalFirmware() = runTest {
        assertNull(banner("stock_final_v32"))
        assertNull(banner("k340_v32"))
    }

    @Test
    fun wpAn2OlderDecodedFirmware() = runTest {
        assertEquals(older("1.1.5"), banner("v17_1_1_5_beta3"))
        assertEquals(older("1.1.5"), banner("v26_1_1_5_beta6"))
        assertEquals(older("1.1.5"), banner("v28_1_1_5"))
        assertEquals(older("1.1.6 early beta"), banner("v28_1_1_6_early_beta"))
        assertEquals(older("1.1.6-beta3"), banner("v30_1_1_6_beta3"))
        assertEquals(older("1.1.6-beta3"), banner("v31_dev"))
    }

    @Test
    fun wpAn2NewerDecodedFirmware() = runTest {
        assertEquals(
            "This device runs firmware 1.1.7, which is newer than DSPi Console 1.1.6-beta5. Some of its features may not be shown. $readOnly",
            banner("v33_mock")
        )
    }

    @Test
    fun wpAn2AFormatItCannotMapDoesNotClaimSettingsAreShown() = runTest {
        assertEquals("This device runs firmware 1.1.4; DSPi Console expects 1.1.6-beta5. $unread", banner("v15_unmapped"))
        assertEquals(
            "This device runs firmware 1.1.8, which is newer than DSPi Console 1.1.6-beta5. $unread",
            banner("v34_unmapped")
        )
    }

    @Test
    fun wpAn2TheVersionAloneNeverFiresIt() {
        for (version in listOf("1.1.6", "1.1.6-beta4", "1.1.6-beta5", "1.1.7", "1.1.5", "")) {
            assertNull(version, HomeModel.firmwareBannerMessage(DspiConstants.COMPAT_OK, version, HomeModel.EXPECTED_FIRMWARE))
        }
    }

    @Test
    fun wpAn2SettingsReadFollowsTheImage() {
        val state = demoDspState()
        assertEquals(true, HomeModel.settingsRead(state.copy(bulkRaw = byteArrayOf(32))))
        assertEquals(false, HomeModel.settingsRead(state.copy(bulkRaw = byteArrayOf(0))))
        assertEquals(false, HomeModel.settingsRead(state.copy(bulkRaw = byteArrayOf())))
    }

    @Test
    fun wpAn2TextNamesNoHostDevice() = runTest {
        val cases = listOf("v28_1_1_5", "v33_mock", "v15_unmapped", "v34_unmapped")
        for (name in cases) {
            val text = banner(name)!!.lowercase()
            for (word in listOf("phone", "tablet", "computer", "android")) {
                assertFalse("$name: $word", word in text)
            }
        }
    }
}
