package se.bofr.dspi.ffi

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import se.bofr.dspi.core.DspiConstants
import se.bofr.dspi.core.DspiException
import se.bofr.dspi.core.FilterParams
import se.bofr.dspi.core.LinkPhase
import se.bofr.dspi.ffi.uniffi.DspiException as FfiException

/** Pure mapping checks (no native library is loaded on the JVM). */
class FfiMappingTest {
    @Test
    fun filterParamsRoundTrip() {
        val p = FilterParams(filterType = 11, bypass = true, freq = 40f, q = 0.5f, gain = 20f, qp = 0.9f)
        assertEquals(p, p.toFfi().toDomain())
    }

    @Test
    fun unsignedBytesSurvive() {
        val ffi = se.bofr.dspi.ffi.uniffi.PinUse(pin = 200u, slot = 255u)
        val d = ffi.toDomain()
        assertEquals(200, d.pin)
        assertEquals(255, d.slot)
    }

    @Test
    fun wpAn1EveryErrorVariantMapsAndBack() {
        assertTrue(FfiException.ReadOnly().toDomain() is DspiException.ReadOnly)
        assertTrue(FfiException.Disconnected().toDomain() is DspiException.Disconnected)
        assertEquals("gone", (FfiException.Io("gone").toDomain() as DspiException.Io).msg)
        assertEquals(255, (FfiException.Refused(255u).toDomain() as DspiException.Refused).status)
        assertEquals("no", (FfiException.FirmwareGuardBlocked("no").toDomain() as DspiException.FirmwareGuardBlocked).msg)
        val auto = FfiException.AutoEq("rate_limited", "wait", 4_000_000_000u).toDomain() as DspiException.AutoEq
        assertEquals("rate_limited", auto.code)
        assertEquals(4_000_000_000L, auto.retryAfterS)
        assertEquals(null, (FfiException.AutoEq("network", "x", null).toDomain() as DspiException.AutoEq).retryAfterS)
        val back = DspiException.AutoEq("rate_limited", "wait", 60).toFfi() as FfiException.AutoEq
        assertEquals(60u, back.retryAfterS)
        assertTrue(DspiException.ReadOnly().toFfi() is FfiException.ReadOnly)
    }

    @Test
    fun wpAn1LinkPhaseMapsEveryVariant() {
        for (phase in LinkPhase.entries) assertEquals(phase, phase.toFfi().toDomain())
        assertEquals(se.bofr.dspi.ffi.uniffi.LinkPhase.NEEDS_PAIRING, LinkPhase.NEEDS_PAIRING.toFfi())
    }

    @Test
    fun wpAn1SystemStatusCarriesStaleAndAge() {
        val ffi = se.bofr.dspi.ffi.uniffi.SystemStatus(
            peaks = emptyList(),
            cpu0 = 1u,
            cpu1 = 2u,
            clipFlags = 3u,
            activeInputChannels = 2u,
            numChannels = 4u,
            stale = true,
            ageMs = 4_000_000_000u,
            // -- WP P0M --
            peakHold = listOf(0.5f),
            peakMax = listOf(0.75f),
            clipHold = 5u
        )
        val d = ffi.toDomain()
        assertTrue(d.stale)
        assertEquals(4_000_000_000L, d.ageMs)
        assertEquals(ffi, d.toFfi())
    }

    @Test
    fun wpAn1NotifyStaleConstant() {
        assertEquals(1 shl 10, DspiConstants.NOTIFY_STALE)
        assertFalse(DspiConstants.NOTIFY_STALE and DspiConstants.NOTIFY_DISCONNECTED != 0)
    }

    /** Kept out of the contract (MIDI is the iOS transport), but in the bindings Android compiles. */
    @Test
    fun wpAn1MidiHandleStaysInTheBindings() {
        val closePort: (se.bofr.dspi.ffi.uniffi.MidiInputHandle) -> Unit = { it.closePort() }
        assertTrue(closePort is Function1<*, *>)
    }
}
