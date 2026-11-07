package se.bofr.dspi.feature.graph

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertSame
import org.junit.Assert.assertTrue
import org.junit.Test
import se.bofr.dspi.core.DspiConstants
import se.bofr.dspi.core.RtaCapsInfo
import se.bofr.dspi.core.RtaChannelFrame
import se.bofr.dspi.core.RtaSnapshot
import se.bofr.dspi.fake.FakeDspiCore

/**
 * AN30: the response graph's spectrum overlay draws the selected channel's
 * frame. The core packs the analysed channels' frames into
 * `frames[0 until count]`, so the frame is found by its channel, not by index,
 * and the bands the core marks unpopulated are left out of the curve.
 */
class AN30OverlayChannelTest {
    private val centres = FakeDspiCore.RTA_BANDS
    private val nBands = centres.size
    private val scale = RtaScale(-90.0, 6.0)
    private val height = 300f

    private val caps =
        RtaCapsInfo(
            supported = true,
            inputChannels = 2,
            outputChannels = 9,
            orderMin = 8,
            orderMax = 10,
            orderDefault = 10,
            bassBands = FakeDspiCore.RTA_BASS_BANDS,
            maxBands = DspiConstants.RTA_MAX_BANDS,
            dynamicRangeDb = 100,
            bassDynamicRangeDb = 90,
            centres = List(DspiConstants.RTA_MAX_BANDS) { b -> centres.getOrElse(b) { 0 } }
        )

    /** Channel ch's flat level, distinct per channel. */
    private fun level(ch: Int) = -20.0 - 10 * ch

    /**
     * A snapshot laid out as the core's rta.rs `build` does: the selected
     * channels' frames packed in channel order, the other slots default frames
     * at `RTA_FLOOR_DB`, unpopulated bands at the floor too.
     */
    private fun packed(channels: List<Int>, fftOrder: Int = 10, bins: Boolean = false): RtaSnapshot {
        val populated = FakeDspiCore.populatedBands(nBands, 0, 48_000, fftOrder)
        val floor = DspiConstants.RTA_FLOOR_DB
        val frames =
            List(DspiConstants.RTA_MAX_CHANNELS) { i ->
                val ch = channels.getOrNull(i)
                if (ch == null) {
                    RtaChannelFrame(0, 0, true, List(DspiConstants.RTA_MAX_BANDS) { floor }, List(DspiConstants.RTA_MAX_BANDS) { floor })
                } else {
                    val avg =
                        List(DspiConstants.RTA_MAX_BANDS) { b ->
                            if (b < nBands && (populated shr b) and 1L == 1L) level(ch).toFloat() else floor
                        }
                    RtaChannelFrame(ch, nBands, false, avg, avg.map { if (it == floor) floor else it + 4f })
                }
            }
        val withBins = bins && channels.size == 1
        return RtaSnapshot(
            version = 1,
            telemetryVersion = 1,
            tap = DspiConstants.RTA_TAP_OUTPUT,
            count = channels.size,
            frames = frames,
            populated = populated,
            binsCount = if (withBins) 256 else 0,
            binsChannel = if (withBins) channels[0] else 0,
            binsSampleRate = 48_000,
            binsDb = List(DspiConstants.RTA_MAX_BINS) { if (withBins) -40f else floor },
            running = true,
            liveCount = channels.size,
            liveMask = channels.fold(0) { m, c -> m or (1 shl c) },
            framesPerS = 40,
            busyUsPerS = 42_000,
            lastFrameUs = 900,
            bassBusyUsPerS = 8_000,
            sampleRate = 48_000,
            firstBand = 0,
            fftOrder = fftOrder,
            configRejected = false,
            refreshIntervalS = channels.size / 40f
        )
    }

    private fun cache(): SpectrumRenderCache {
        val cache = SpectrumRenderCache()
        cache.builder.configure(20.0, 20000.0, 600.0, height.toDouble(), scale)
        cache.reshape(cache.builder.columns, 600f, height)
        return cache
    }

    private fun build(cache: SpectrumRenderCache, snap: RtaSnapshot, ch: Int, wantsBins: Boolean): ChannelTrace {
        val t = cache.trace(ch)
        buildTrace(cache, t, caps, snap, ch, wantsBins, 0.0, 0.0, true, height)
        return t
    }

    /** The band x positions a trace drawn from [snap] must have (computed into scratch arrays). */
    private fun expectedBandX(cache: SpectrumRenderCache, snap: RtaSnapshot): FloatArray {
        val xs = FloatArray(DspiConstants.RTA_MAX_BANDS + 1)
        val n = cache.builder.bandPoints(caps.centres, DoubleArray(nBands), nBands, nBands, 0, snap.populated, xs, FloatArray(xs.size))
        return xs.copyOf(n)
    }

    /** The trace's own band and peak points (written by buildTrace) all carry channel ch's levels. */
    private fun assertBandsAre(cache: SpectrumRenderCache, t: ChannelTrace, snap: RtaSnapshot, ch: Int) {
        val xs = expectedBandX(cache, snap)
        assertTrue("channel $ch has band points", xs.size > 1)
        val b = cache.builder
        for (i in xs.indices) {
            assertEquals("channel $ch band point $i x", xs[i], t.bandX[i], 1e-3f)
            assertEquals("channel $ch band point $i", b.y(level(ch)).toFloat(), t.bandY[i], 1e-3f)
            assertEquals("channel $ch peak point $i x", xs[i], t.peakX[i], 1e-3f)
            assertEquals("channel $ch peak point $i", b.y(level(ch) + 4).toFloat(), t.peakY[i], 1e-3f)
        }
    }

    @Test
    fun an30FrameIsFoundByChannelNotByIndex() {
        val snap = packed(listOf(2))
        assertSame(snap.frames[0], rtaOverlayFrame(snap, 2))
        assertNull("channel 0 is not analysed: no frame", rtaOverlayFrame(snap, 0))
        assertNull("frames past count are never read", rtaOverlayFrame(packed(emptyList()), 0))
        val pair = packed(listOf(1, 3))
        assertEquals(1, rtaOverlayFrame(pair, 1)?.channel)
        assertEquals(3, rtaOverlayFrame(pair, 3)?.channel)
    }

    @Test
    fun an30SingleChannelTwoDrawsItsOwnBassBelow200Hz() {
        val snap = packed(listOf(2), bins = true)
        val cache = cache()
        val t = build(cache, snap, 2, wantsBins = true)
        assertTrue(t.hasCurve)
        assertTrue(t.hasPeak)
        val b = cache.builder
        val want = b.y(level(2)).toFloat()
        val floorY = b.y(scale.floorDB).toFloat()
        assertEquals(height, floorY, 1e-3f)
        // Below the top of the bass bank (200 Hz) the curve is channel 2's bands, not the floor.
        for (hz in listOf(25.0, 50.0, 100.0, 150.0, 190.0)) {
            val y = cache.columnY[b.x(hz).toInt()]
            assertEquals("column at $hz Hz", want, y, 0.5f)
            assertTrue("column at $hz Hz is off the floor", y < floorY - 50f)
        }
        // The band points themselves carry channel 2's levels.
        assertBandsAre(cache, t, snap, 2)
    }

    @Test
    fun an30SelectionOneAndThreeDrawsTwoCurvesFromTheRightFrames() {
        val snap = packed(listOf(1, 3))
        val cache = cache()
        val t1 = build(cache, snap, 1, wantsBins = false)
        val t3 = build(cache, snap, 3, wantsBins = false)
        assertTrue(t1.hasCurve)
        assertTrue(t3.hasCurve)
        assertBandsAre(cache, t1, snap, 1)
        assertBandsAre(cache, t3, snap, 3)
        assertTrue("two distinct curves", t1.bandY[0] != t3.bandY[0])
    }

    @Test
    fun an30UnpopulatedBandsAreLeftOutAtOrder8And48kHz() {
        val snap = packed(listOf(0), fftOrder = 8)
        val gaps = listOf(251, 316, 501).map { centres.indexOf(it) }
        for (g in gaps) {
            assertTrue("band ${centres[g]} Hz is in the table", g >= 0)
            assertEquals("band ${centres[g]} Hz has no bin at order 8", 0L, (snap.populated shr g) and 1L)
        }
        val cache = cache()
        val t = build(cache, snap, 0, wantsBins = false)
        assertTrue(t.hasCurve)
        assertTrue(t.hasPeak)
        val b = cache.builder
        val gapX = gaps.map { b.x(centres[it].toDouble()).toFloat() }
        val visible = (0 until nBands).count { (snap.populated shr it) and 1L == 1L && centres[it] in 20..20000 }
        val n = expectedBandX(cache, snap).size
        assertTrue("the populated bands and at most one either side", n in visible..visible + 2)
        for (x in gapX) {
            for (i in 0 until n) {
                assertFalse("no band point at x=$x", kotlin.math.abs(t.bandX[i] - x) < 1e-3f)
                assertFalse("no peak point at x=$x", kotlin.math.abs(t.peakX[i] - x) < 1e-3f)
            }
        }
        // No point sits on the floor: every drawn band carries a reading.
        assertBandsAre(cache, t, snap, 0)
    }
}
