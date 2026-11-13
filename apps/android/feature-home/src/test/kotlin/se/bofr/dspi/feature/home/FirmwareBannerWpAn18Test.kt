package se.bofr.dspi.feature.home

import java.io.File
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.StandardTestDispatcher
import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test
import se.bofr.dspi.bridge.DspiBridgeImpl
import se.bofr.dspi.core.DspState
import se.bofr.dspi.core.DspiConstants
import se.bofr.dspi.fake.DspStateJson
import se.bofr.dspi.fake.FakeDspiCore
import se.bofr.dspi.fake.demoDspState

/**
 * WP AN18: on this app's wire format the banner compares firmware versions as
 * the Mac does (`DSPViewModel.firmwareMatch`, M41), against the bundled v1.1.6.
 * A build stamp after a matching version (the K340's `bofr-main` build) is the
 * same version. The stamp strings are fixtures: bofr-main does not stamp its
 * images yet (HANDOVER section 5).
 */
@OptIn(ExperimentalCoroutinesApi::class)
class FirmwareBannerWpAn18Test {
    private fun fixture(name: String): DspState = DspStateJson.parse(File("../fixtures/wire/$name.json").readText(), demoDspState())

    private suspend fun TestScope.bridgeFor(state: DspState): DspiBridgeImpl {
        val core = FakeDspiCore(state)
        val d = StandardTestDispatcher(testScheduler)
        val b = DspiBridgeImpl(core, backgroundScope, ffi = d, main = d, clock = { testScheduler.currentTime })
        b.attachDevice(42, core.serial, DspiConstants.VENDOR_ID)
        runCurrent()
        return b
    }

    @Test
    fun wpAn18AnotherVersionOnTheSameWireFormatShowsTheBanner() = runTest {
        val k340 = fixture("k340_v32")
        val beta = bridgeFor(k340.copy(fwBeta = 5))
        assertEquals(DspiConstants.COMPAT_OK, beta.compat)
        assertEquals("This device runs firmware 1.1.6-beta5; uConsole for DSPi expects 1.1.6.", HomeModel.firmwareBannerMessage(beta))
        assertEquals("Update...", HomeModel.firmwareBannerButton(beta.compat, beta.firmwareVersion))

        val newer = bridgeFor(k340.copy(fwPatch = 7))
        assertEquals(DspiConstants.COMPAT_OK, newer.compat)
        assertEquals(
            "This device runs firmware 1.1.7, which is newer than uConsole for DSPi 1.1.6. Some of its features may not be shown.",
            HomeModel.firmwareBannerMessage(newer)
        )
        assertEquals("Details...", HomeModel.firmwareBannerButton(newer.compat, newer.firmwareVersion))
    }

    @Test
    fun wpAn18NoBannerForAMatchingVersion() = runTest {
        assertNull(HomeModel.firmwareBannerMessage(bridgeFor(fixture("k340_v32"))))
        assertNull(HomeModel.firmwareBannerMessage(bridgeFor(fixture("stock_final_v32"))))
    }

    @Test
    fun wpAn18NoBannerForAMatchingVersionWithABuildStamp() {
        for (stamped in listOf("1.1.6-16-g557bce7", "1.1.6+bofr.04122d3", "1.1.6-dirty", "1.1.6 (bofr-main 04122d3)")) {
            assertEquals(stamped, HomeModel.FirmwareOrder.MATCH, HomeModel.firmwareMatch(stamped, HomeModel.EXPECTED_FIRMWARE))
            assertNull(stamped, HomeModel.firmwareBannerMessage(DspiConstants.COMPAT_OK, stamped, HomeModel.EXPECTED_FIRMWARE))
            assertEquals(stamped, "Update...", HomeModel.firmwareBannerButton(DspiConstants.COMPAT_OK, stamped))
        }
        // A stamp does not hide a different version.
        assertEquals(HomeModel.FirmwareOrder.OLDER, HomeModel.firmwareMatch("1.1.6-beta5-3-gdeadbee", "1.1.6"))
        assertEquals(HomeModel.FirmwareOrder.NEWER, HomeModel.firmwareMatch("1.1.7-2-gabc1234", "1.1.6"))
    }

    @Test
    fun wpAn18OrderFollowsTheMac() {
        assertEquals(HomeModel.FirmwareOrder.OLDER, HomeModel.firmwareMatch("1.1.6 early beta", "1.1.6-beta1"))
        assertEquals(HomeModel.FirmwareOrder.OLDER, HomeModel.firmwareMatch("1.1.7-beta3", "1.1.7"))
        assertEquals(HomeModel.FirmwareOrder.NEWER, HomeModel.firmwareMatch("1.2", "1.1.16"))
        assertEquals(HomeModel.FirmwareOrder.MATCH, HomeModel.firmwareMatch("1.2", "1.2.0"))
        assertNull(HomeModel.firmwareMatch("", "1.1.6"))
        assertNull(HomeModel.firmwareMatch("unknown", "1.1.6"))
    }

    @Test
    fun wpAn18TheDemoDeviceGetsNoVersionBanner() = runTest {
        val b = bridgeFor(fixture("k340_v32").copy(fwBeta = 4))
        assertNull(HomeModel.firmwareBannerMessage(b, demo = true))
        // Its wire format still counts.
        val old = bridgeFor(fixture("v28_1_1_5"))
        assertEquals(HomeModel.firmwareBannerMessage(old), HomeModel.firmwareBannerMessage(old, demo = true))
    }

    @Test
    fun wpAn18ExpectedIsTheBundledImage() {
        val names = File("../feature-firmware/src/main/assets/firmware").list().orEmpty().filter { it.endsWith(".uf2") }.sorted()
        assertEquals(
            listOf("DSPi-RP2040-v${HomeModel.EXPECTED_FIRMWARE}.uf2", "DSPi-RP2350-v${HomeModel.EXPECTED_FIRMWARE}.uf2"),
            names
        )
    }
}
