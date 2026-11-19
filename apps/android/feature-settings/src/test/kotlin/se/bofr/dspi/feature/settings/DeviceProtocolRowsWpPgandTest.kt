package se.bofr.dspi.feature.settings

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import se.bofr.dspi.core.DeviceKind
import se.bofr.dspi.core.DevicePage
import se.bofr.dspi.core.DeviceProtocol
import se.bofr.dspi.core.ExtEnvelope
import se.bofr.dspi.core.ProtocolGeneration
import se.bofr.dspi.ui.DspiRoutes

/** WP PGAND: the device-info row's words, per generation, and the alink sender's pages and routes. */
class DeviceProtocolRowsWpPgandTest {
    private val legacy = DeviceProtocol(ProtocolGeneration.LEGACY, 0, 0, emptyList(), ExtEnvelope.LEGACY, 0, "DSPi", DeviceKind.DSP)

    @Test
    fun wpPgandALegacyDeviceSaysPageZeroAndTheLegacyChannel() {
        assertEquals(
            listOf(
                "Board" to "DSPi",
                "Protocol generation" to "1.1.6 (legacy)",
                "Pages" to "None (page 0 only)",
                "Extension channel" to "Legacy request 0x00"
            ),
            deviceProtocolRows(legacy)
        )
        assertEquals("None (stock firmware)", extEnvelopeLabel(ExtEnvelope.NO_CHANNEL))
    }

    @Test
    fun wpPgandAPagedDeviceListsItsVersionAndPages() {
        val g2b = legacy.copy(
            generation = ProtocolGeneration.PAGED,
            protocolMajor = 1,
            protocolMinor = 3,
            pages = listOf(DevicePage(0x01, 1), DevicePage(0x05, 2), DevicePage(0xE0, 1)),
            extEnvelope = ExtEnvelope.TUNNEL
        )
        val rows = deviceProtocolRows(g2b).toMap()
        assertEquals("1.1.7 (paged)", rows["Protocol generation"])
        assertEquals("1.3", rows["Protocol version"])
        assertEquals("0x01 r1, 0x05 r2, 0xE0 r1", rows["Pages"])
        assertEquals("Page 0xE0 tunnel", rows["Extension channel"])
        // A paged device whose transport gave no page list (a relay that passes no discovery on) says so
        assertEquals("Not listed on this connection", pagesLabel(g2b.copy(pages = emptyList())))
    }

    @Test
    fun wpPgandTheBoardAndTheSenderAreNamed() {
        val zero = legacy.copy(boardId = 1, boardName = "DSPi Zero")
        assertEquals("DSPi Zero", deviceProtocolRows(zero).toMap()["Board"])
        assertEquals("Unknown board", deviceProtocolRows(legacy.copy(boardName = "")).toMap()["Board"])
        val sender = legacy.copy(deviceKind = DeviceKind.ALINK_SENDER)
        assertEquals("Audio link sender", deviceProtocolRows(sender).toMap()["Device"])
        assertFalse("a DSP has no Device row", deviceProtocolRows(legacy).toMap().containsKey("Device"))
        assertEquals("Not reported", generationLabel(ProtocolGeneration.UNKNOWN))
    }

    @Test
    fun wpPgandTheSenderOpensNoDspPage() {
        val f = SettingsAvailability(connected = true, alinkSender = true, networkingAvailable = true)
        assertEquals(
            setOf(SettingsCategory.GENERAL, SettingsCategory.NETWORKING),
            SettingsCategory.entries.filter { it.isAvailable(f) }.toSet()
        )
        assertFalse(SettingsCategory.NETWORKING.isAvailable(f.copy(networkingAvailable = false)))
        for (r in listOf(DspiRoutes.HOME, DspiRoutes.SETTINGS, DspiRoutes.FIRMWARE, DspiRoutes.GETTING_STARTED, DspiRoutes.WHATS_NEW)) {
            assertTrue(r, allowedForAlinkSender(r))
        }
        for (r in listOf(
            DspiRoutes.GRAPH,
            DspiRoutes.MATRIX,
            DspiRoutes.TOOLS,
            DspiRoutes.PRESETS,
            DspiRoutes.CHANNEL,
            DspiRoutes.ANALYSIS
        )) {
            assertFalse(r, allowedForAlinkSender(r))
        }
    }
}
