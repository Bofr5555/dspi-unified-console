package se.bofr.dspi.ffi

import java.nio.file.Files
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertThrows
import org.junit.Assert.assertTrue
import org.junit.Test
import se.bofr.dspi.core.AutoEqFetch
import se.bofr.dspi.core.DspiConstants
import se.bofr.dspi.core.DspiException
import se.bofr.dspi.core.FilterParams
import se.bofr.dspi.core.FirmwareGuardBoardSource
import se.bofr.dspi.core.LinkListener
import se.bofr.dspi.core.LinkPhase
import se.bofr.dspi.core.NotifyListener

/**
 * WP AN1: the generated adapter against the real library, built for the host
 * (ffi/build.gradle.kts): every call crosses JNA and comes back through the
 * mapping, on the core's demo device. No USB, no phone.
 */
class FfiLiveDemoTest {
    private fun demo() = FfiDspiCore().apply { openDemo() }

    @Test
    fun wpAn1DemoDeviceThroughTheAdapter() {
        demo().use { core ->
            assertTrue(core.isConnected())
            val state = core.getState()
            assertEquals(DspiConstants.COMPAT_OK, state.compat)
            assertEquals(DspiConstants.BULK_PARAMS_SIZE, state.bulkRaw.size)

            core.setPreamp(-3f)
            assertEquals(-3f, core.getState().preampDb, 0.01f)

            val band = FilterParams(filterType = DspiConstants.FILTER_PEAKING, bypass = false, freq = 1000f, q = 1.5f, gain = 2f, qp = 0f)
            core.setFilter(0, 1, band)
            assertEquals(1000f, core.getState().filters[0][1].freq, 0.01f)

            val status = core.fetchStatus()
            assertFalse(status.stale)
            assertTrue(status.ageMs >= 0)
        }
    }

    @Test
    fun wpAn1LibraryLogsThroughItsOwnNamespace() {
        // Loading the adapter called logging.initLogging(); a second call changes nothing.
        FfiDspiCore().use { se.bofr.dspi.ffi.uniffi.logging.initLogging() }
    }

    @Test
    fun wpAn1ErrorsComeBackTyped() {
        FfiDspiCore().use { core ->
            assertThrows(DspiException.Disconnected::class.java) { core.setPreamp(0f) }
            assertThrows(DspiException.InvalidArgument::class.java) { core.linkParseAddress("") }
        }
        demo().use { core ->
            assertThrows(DspiException.InvalidArgument::class.java) { core.setFilter(99, 0, FilterParams(1, false, 1000f, 1f, 0f, 0f)) }
        }
    }

    @Test
    fun wpAn1EnumsAndFreeFunctions() {
        FfiDspiCore().use { core ->
            assertEquals(LinkPhase.IDLE, core.linkStatus().phase)
            assertEquals(FirmwareGuardBoardSource.BOOTLOADER, core.firmwareGuardBoardInBootloader().source)
            assertEquals("ws://hub:11915/dspi/v1", core.linkWsUrl("hub", 11915, ""))
            assertEquals(11915, core.linkParseAddress("hub:11915").port)
            assertFalse(core.uf2Inspect(listOf(1, 2, 3)).valid)
        }
    }

    @Test
    fun wpAn1ListenersReachTheCore() {
        FfiDspiCore().use { core ->
            core.setNotifyListener(NotifyListener { })
            core.setNotifyListener(null)
            core.setLinkListener(object : LinkListener {
                override fun onLinkDevicesChanged() {}

                override fun onLinkClosed(code: Int, reason: String) {}
            })
            core.setLinkListener(null)
        }
    }

    @Test
    fun wpAn1ForeignFetchErrorCrossesBothWays() {
        val dir = Files.createTempDirectory("wp-an1").toFile()
        try {
            FfiDspiCore().use { core ->
                val db = dir.resolve("autoeq.db").path
                assertFalse(core.autoeqDatabaseInfo(db).hasUserDatabase)
                val failing = AutoEqFetch { throw DspiException.Transfer("no network") }
                val e = assertThrows(DspiException.AutoEq::class.java) { core.autoeqUpdate(db, "https://example.invalid/db", failing) }
                assertEquals("network", e.code)
            }
        } finally {
            dir.deleteRecursively()
        }
    }
}
