package se.bofr.dspi.feature.analysis

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import se.bofr.dspi.core.AdatOutStatus
import se.bofr.dspi.core.FillBand
import se.bofr.dspi.core.InputLockStatus
import se.bofr.dspi.core.LgStatus
import se.bofr.dspi.core.OutputFill
import se.bofr.dspi.core.RingHealth
import se.bofr.dspi.core.SpdifBufferStats
import se.bofr.dspi.core.SpdifRxStatus

class StatsModelTest {
    private fun ring(lowest: Long?) = RingHealth(96, 2000, 48, 0, 0, lowest, 0, 0, 0)

    @Test
    fun an25RingHealthTextAsTheReference() {
        assertEquals("96 frames (2 ms)", ringTargetText(ring(1)))
        assertEquals("64 frames (1.33 ms)", ringTargetText(ring(1).copy(targetFrames = 64, targetUs = 1333)))
        assertEquals("72 frames", ringLowestFillText(ring(72)))
        assertEquals("-", ringLowestFillText(ring(null)))
        assertEquals("9999 µs", ringGapText(9_999))
        assertEquals("10.0 ms", ringGapText(10_000))
        assertEquals("125.5 ms", ringGapText(125_500))
    }

    @Test
    fun an25AnOutputReadsTheCoresFillAndBandOverTheFixedOnes() {
        val slot = SpdifBufferStats(2, 2, 4, 12, 9, 16)
        // 12% is yellow on the fixed bands; the core's band around the target wins
        assertEquals(FillHealth.YELLOW, BufferFillThresholds.SPDIF.health(12))
        val r = outputFillReading(slot, OutputFill(18.75, 14, 23, FillBand.GOOD))
        assertEquals(OutputFillReading(18.75, 14, 23, FillHealth.GREEN), r)
        assertEquals(19, r.wholePct)
        assertEquals(FillHealth.YELLOW, FillBand.WARN.health())
        assertEquals(FillHealth.RED, FillBand.BAD.health())
        // Before the first read: the slot on its fixed bands
        assertEquals(OutputFillReading(12.0, 9, 16, FillHealth.YELLOW), outputFillReading(slot, null))
        assertEquals(0, OutputFillReading(Double.NaN, 0, 0, FillHealth.RED).wholePct)
    }

    @Test
    fun an25TracesTakeTheRingScaleWhenThereIsOne() {
        val h = BufferFillHistory()
        h.append(testBufferStats(3), List(4) { OutputFill(18.75, 14, 23, FillBand.GOOD) })
        assertEquals(18.75f, h.value(0, 0), 0f)
        // Firmware's fixed scale can pass 100; the trace stops at the top
        h.append(testBufferStats(3), List(4) { OutputFill(120.0, 14, 23, FillBand.BAD) })
        assertEquals(100f, h.value(0, 1), 0f)
        h.append(testBufferStats(3))
        assertEquals(50f, h.value(0, 2), 0f)
    }

    @Test
    fun systemInformationRows() {
        assertEquals("150.0 MHz", SystemInfoFormat.clock(150_000_000))
        assertEquals("1.10 V", SystemInfoFormat.coreVoltage(1100))
        assertEquals("48.0 kHz", SystemInfoFormat.sampleRate(48_000))
        assertEquals("31.2 °C", SystemInfoFormat.temperature(3125))
        assertEquals("-5.0 °C", SystemInfoFormat.temperature(0xFFFF_FE0CL)) // sign-extended
        assertEquals("-5.0 °C", SystemInfoFormat.temperature(0xFE0CL)) // i16 bits only
        assertEquals("-", SystemInfoFormat.clock(null))
    }

    @Test
    fun elapsedAsMac() {
        assertEquals("0m 00s", statsFormatElapsed(0))
        assertEquals("1m 05s", statsFormatElapsed(65))
        assertEquals("1h 02m 03s", statsFormatElapsed(3723))
        assertEquals("0m 00s", statsFormatElapsed(-4))
    }

    @Test
    fun thresholds() {
        assertEquals(FillHealth.RED, BufferFillThresholds.SPDIF.health(0))
        assertEquals(FillHealth.RED, BufferFillThresholds.SPDIF.health(100))
        assertEquals(FillHealth.YELLOW, BufferFillThresholds.SPDIF.health(24))
        assertEquals(FillHealth.GREEN, BufferFillThresholds.SPDIF.health(50))
        assertEquals(FillHealth.YELLOW, BufferFillThresholds.PDM_DMA.health(4))
        assertEquals(FillHealth.GREEN, BufferFillThresholds.PDM_DMA.health(20))
        assertEquals(FillHealth.RED, BufferFillThresholds.PDM_DMA.health(51))
        assertEquals(FillHealth.GREEN, BufferFillThresholds.PDM_RING.health(20))
        assertEquals(FillHealth.YELLOW, BufferFillThresholds.PDM_RING.health(21))
    }

    @Test
    fun historyKeepsTheNewest256AndMarksAbsentBuffers() {
        val h = BufferFillHistory()
        assertTrue(h.value(0, 0).isNaN())
        repeat(300) { h.append(testBufferStats(it)) }
        assertEquals(300, h.total)
        assertTrue(h.value(0, 10).isNaN()) // aged out
        assertEquals(testBufferStats(299).spdif[0].fillPct.toFloat(), h.value(0, 299), 0f)
        h.append(testBufferStats(0).copy(numSpdif = 1, pdmActive = false))
        assertTrue(h.value(1, 300).isNaN())
        assertTrue(h.value(BufferFillHistory.PDM_DMA_SERIES, 300).isNaN())
        assertEquals(50f, h.value(BufferFillHistory.PDM_DMA_SERIES, 299), 0f)
        h.clear()
        assertEquals(0, h.total)
    }

    @Test
    fun starvationDeltaAndEventTimes() {
        val t = StarvationTracker()
        t.update(5, 1000)
        assertEquals(0, t.delta) // first read sets the baseline
        assertNull(t.lastEventMs)
        t.update(7, 3000)
        assertEquals(2, t.delta)
        assertEquals(3000L, t.lastEventMs)
        assertNull(t.previousEventMs)
        t.update(7, 5000)
        assertEquals(0, t.delta)
        t.update(8, 9000)
        assertEquals(9000L, t.lastEventMs)
        assertEquals(3000L, t.previousEventMs)
        // 32-bit wrap
        t.update(0xFFFF_FFFFL, 10_000)
        t.update(1, 11_000)
        assertEquals(2, t.delta)
        t.reset()
        assertEquals(0, t.total)
        assertNull(t.lastEventMs)
    }

    @Test
    fun spdifInputRowsAsMac() {
        val locked = SpdifRxStatus(2, 4, 3, 1, 44_100, 7, 52, 2, 3, 1)
        assertEquals("Locked", spdifRxStateString(2))
        assertEquals(StateDot.ORANGE, spdifRxStateDot(3))
        assertEquals(StateDot.GRAY, spdifRxStateDot(0))
        assertEquals("S/PDIF 2", spdifRxSourceString(locked.inputSource))
        assertEquals("S/PDIF 1", spdifRxSourceString(1))
        assertEquals("I2S", spdifRxSourceString(2))
        assertEquals("USB", spdifRxSourceString(0))
        assertEquals("44.1 kHz", spdifRxSampleRateString(locked))
        assertEquals("-", spdifRxSampleRateString(locked.copy(state = 1)))
        assertEquals("Waiting Stable", spdifRxLibStateString(1))
        assertEquals("Unknown (9)", spdifRxLibStateString(9))
    }

    @Test
    fun rowsAndInterfaceStrings() {
        assertEquals(2, starvationInstanceCount(0))
        assertEquals(4, starvationInstanceCount(4))
        assertEquals("Out 3/4", starvationInstanceLabel(1))
        assertEquals("Out 5/6 (I2S)", bufferSlotTitle(2, 1))
        assertEquals("Out 1/2 (SPDIF)", bufferSlotTitle(0, 0))
        assertEquals("44–56%", watermarkText(44, 56))

        assertEquals("Disabled", adatStateString(AdatOutStatus(false, false, 20, true, 0, 0)))
        assertEquals("Streaming", adatStateString(AdatOutStatus(true, true, 20, true, 0, 0)))
        assertEquals("Suspended (rate above 48 kHz)", adatStateString(AdatOutStatus(true, false, 20, false, 0, 0)))
        assertEquals(StateDot.YELLOW, adatStateDot(AdatOutStatus(true, false, 20, true, 0, 0)))

        val locked = InputLockStatus(3, 1, 48_000, 47_998, true, 1, 0)
        assertEquals("Locked", i2sSlaveStateString(3))
        assertEquals(StateDot.ORANGE, i2sSlaveStateDot(2))
        assertEquals("48.0 kHz", i2sDetectedRateString(locked))
        assertEquals("-", i2sDetectedRateString(locked.copy(state = 1)))
        assertEquals("-", i2sMeasuredRateString(locked.copy(measuredHz = 0)))

        assertEquals("-", lgVolumeString(LgStatus(true, false, 0xFF, false)))
        assertEquals("23 / 100", lgVolumeString(LgStatus(true, true, 23, false)))
    }
}
