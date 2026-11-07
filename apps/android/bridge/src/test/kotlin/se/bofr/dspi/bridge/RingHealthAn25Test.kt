package se.bofr.dspi.bridge

import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.advanceTimeBy
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import se.bofr.dspi.core.FillBand
import se.bofr.dspi.core.OutputFill
import se.bofr.dspi.core.RingHealth
import se.bofr.dspi.core.StatsTick
import se.bofr.dspi.fake.FakeDspiCore

/** AN25: the stats controller's ring health and output fills. */
@OptIn(ExperimentalCoroutinesApi::class)
class RingHealthAn25Test {
    /** A device whose firmware can lose the status, as one without `REQ_OUTPUT_LATENCY` has none. */
    private class Firmware : FakeDspiCore() {
        @Volatile var hasRingHealth = true

        override fun fetchRingHealth(): RingHealth? {
            val health = super.fetchRingHealth()
            return if (hasRingHealth) health else null
        }
    }

    @Test
    fun an25RingHealthIsReadOnTheSlowCadenceAndFillsOnTheFastOne() = runTest {
        val core = FakeDspiCore()
        val b = newBridge(core)
        b.setUiVisible(false)
        attach(b, core)
        advanceTimeBy(5_000)
        assertEquals(0, core.callCount("fetchRingHealth"))
        assertEquals(0, core.callCount("fetchStatsTick"))

        b.stats.watching = true
        runCurrent()
        assertEquals(96, b.stats.ringHealth.value?.targetFrames)
        assertEquals(4, b.stats.outputFills.value.size)
        assertEquals(1, core.callCount("fetchRingHealth"))

        advanceTimeBy(4_100)
        assertEquals("every 2 s, on top of the first", 3, core.callCount("fetchRingHealth"))
        assertTrue(core.callCount("fetchStatsTick") >= 60)

        b.stats.watching = false
        val slow = core.callCount("fetchRingHealth")
        val fast = core.callCount("fetchStatsTick")
        advanceTimeBy(5_000)
        assertEquals(slow, core.callCount("fetchRingHealth"))
        assertEquals(fast, core.callCount("fetchStatsTick"))
    }

    @Test
    fun an25ATransferErrorKeepsTheLastRingHealth() = runTest {
        val core = FakeDspiCore()
        val b = newBridge(core)
        b.setUiVisible(false)
        attach(b, core)
        b.stats.watching = true
        runCurrent()
        val health = b.stats.ringHealth.value
        assertNotNull(health)

        core.failNext("fetchRingHealth", times = 2)
        val seen = core.callCount("fetchRingHealth")
        advanceTimeBy(2_100)
        assertEquals(seen + 1, core.callCount("fetchRingHealth"))
        assertEquals(health, b.stats.ringHealth.value)
    }

    @Test
    fun an25AFailedFillReadLeavesTheFillsWhileBuffersMoveOn() = runTest {
        val first = listOf(OutputFill(10.0, 1, 2, FillBand.GOOD))
        val core =
            object : FakeDspiCore() {
                @Volatile var fail = false

                override fun fetchStatsTick(): StatsTick = super.fetchStatsTick().copy(fills = if (fail) null else first)
            }
        val b = newBridge(core)
        b.setUiVisible(false)
        attach(b, core)
        b.stats.watching = true
        runCurrent()
        assertEquals(first, b.stats.outputFills.value)
        val before = b.stats.buffers.value?.sequence

        core.fail = true
        advanceTimeBy(300)
        assertEquals(first, b.stats.outputFills.value)
        assertTrue(b.stats.buffers.value?.sequence != before)
    }

    // -- WP CG21 --
    @Test
    fun cg21TheFastPollIsOneReadPerTick() = runTest {
        val core = FakeDspiCore()
        val b = newBridge(core)
        b.setUiVisible(false)
        attach(b, core)
        b.stats.watching = true
        runCurrent()
        val start = core.callCount("fetchStatsTick")
        advanceTimeBy(StatsControllerImpl.FAST_MS * 50 + 1)
        assertEquals(50, core.callCount("fetchStatsTick") - start)
        assertEquals(0, core.callCount("fetchBufferStats"))
        assertEquals(0, core.callCount("fetchOutputFills"))
        assertEquals(0, core.callCount("fetchOutputLatency"))
        assertEquals(4, b.stats.outputFills.value.size)
        assertNotNull(b.stats.buffers.value)
        b.stats.watching = false
    }

    @Test
    fun cg21AFailedTickKeepsBoth() = runTest {
        val core = FakeDspiCore()
        val b = newBridge(core)
        b.setUiVisible(false)
        attach(b, core)
        b.stats.watching = true
        runCurrent()
        val buffers = b.stats.buffers.value
        val fills = b.stats.outputFills.value
        assertNotNull(buffers)
        core.failNext("fetchStatsTick", times = 2)
        advanceTimeBy(StatsControllerImpl.FAST_MS + 1)
        assertEquals(buffers, b.stats.buffers.value)
        assertEquals(fills, b.stats.outputFills.value)
        b.stats.watching = false
    }

    @Test
    fun an25NullHidesAndADeviceChangeStartsOver() = runTest {
        val core = Firmware()
        val b = newBridge(core)
        b.setUiVisible(false)
        attach(b, core)
        b.stats.watching = true
        runCurrent()
        assertNotNull(b.stats.ringHealth.value)

        // Unlike an error, null replaces what was there
        core.hasRingHealth = false
        advanceTimeBy(2_100)
        assertNull(b.stats.ringHealth.value)

        core.hasRingHealth = true
        advanceTimeBy(2_100)
        assertNotNull(b.stats.ringHealth.value)

        b.detachDevice(core.serial)
        runCurrent()
        assertNull(b.stats.ringHealth.value)
        assertTrue(b.stats.outputFills.value.isEmpty())
        b.stats.watching = false
    }

    @Test
    fun an25AReadInFlightWhenTheDeviceGoesIsDropped() = runTest {
        val core = FakeDspiCore()
        val b = newBridge(core)
        b.setUiVisible(false)
        attach(b, core)
        b.stats.watching = true
        // The first polls are queued on the FFI dispatcher, not yet back
        b.detachDevice(core.serial)
        runCurrent()
        assertNull(b.stats.ringHealth.value)
        assertTrue(b.stats.outputFills.value.isEmpty())
        b.stats.watching = false
    }

    @Test
    fun an25ResetWatermarksRereadsTheRingWatermarks() = runTest {
        val core = FakeDspiCore()
        val b = newBridge(core)
        b.setUiVisible(false)
        attach(b, core)
        b.stats.watching = true
        runCurrent()
        val n = core.callCount("fetchRingHealth")
        b.stats.resetWatermarks()
        runCurrent()
        assertEquals(1, core.callCount("resetBufferStats"))
        assertEquals(n + 1, core.callCount("fetchRingHealth"))
        b.stats.watching = false
    }

    @Test
    fun an25DemoFillsSitOnTheRingScaleInsideTheTargetBand() {
        val core = FakeDspiCore()
        repeat(14) {
            for (f in core.fetchOutputFills()) {
                assertEquals(FillBand.GOOD, f.band)
                assertTrue("${f.fillPct}", f.fillPct >= f.minPct && f.fillPct <= f.maxPct)
            }
        }
        assertEquals(18.75, core.fetchRingHealth()!!.targetFrames * 100.0 / 512, 0.0)
    }
}
