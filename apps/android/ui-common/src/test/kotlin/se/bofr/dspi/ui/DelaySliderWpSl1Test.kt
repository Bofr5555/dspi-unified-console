package se.bofr.dspi.ui

import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.advanceTimeBy
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Test

/** WP SL1: a delay slider sends and commits whole ms (macOS release/v1.1.6); typed values keep their fraction. */
@OptIn(ExperimentalCoroutinesApi::class)
class DelaySliderWpSl1Test {
    @Test
    fun wpSl1RoundsHalfAwayFromZeroWithoutNegativeZero() {
        assertEquals(0f, roundDelayToWholeMs(0.25f), 0f)
        assertEquals(1f, roundDelayToWholeMs(0.5f), 0f)
        assertEquals(2f, roundDelayToWholeMs(1.5f), 0f)
        assertEquals(12f, roundDelayToWholeMs(12.4999f), 0f)
        assertEquals(85f, roundDelayToWholeMs(85f), 0f)
        assertEquals(Float.POSITIVE_INFINITY, 1f / roundDelayToWholeMs(-0.2f), 0f) // no negative zero
    }

    @Test
    fun wpSl1DragFromAFractionalStartSendsAndCommitsWholeMs() = runTest {
        val live = mutableListOf<Float>()
        val commits = mutableListOf<Float>()
        val d = SliderDragState(backgroundScope, initial = 0.25f, intervalMs = 30, quantize = ::roundDelayToWholeMs)
        d.editing(true) { commits += it }
        // the thumb moves continuously
        d.change(0.4f) { live += it }
        assertEquals(0.4f, d.value, 0f)
        advanceTimeBy(31)
        runCurrent()
        d.change(1.7f) { live += it }
        advanceTimeBy(31)
        runCurrent()
        d.change(2.6f) { live += it }
        advanceTimeBy(31)
        runCurrent()
        d.change(3.3f) { live += it }
        d.editing(false) { commits += it }
        advanceTimeBy(100)
        runCurrent()
        assertEquals(listOf(0f, 2f, 3f), live)
        assertEquals(listOf(3f), commits)
        // the thumb snaps to the committed value on release
        assertEquals(3f, d.value, 0f)
    }

    @Test
    fun wpSl1AMoveThatRoundsToTheSameMsIsSentOnce() = runTest {
        val live = mutableListOf<Float>()
        val d = SliderDragState(backgroundScope, initial = 5f, intervalMs = 30, quantize = ::roundDelayToWholeMs)
        d.editing(true) {}
        d.change(5.2f) { live += it } // still 5 ms: not sent
        advanceTimeBy(31)
        runCurrent()
        d.change(5.6f) { live += it }
        advanceTimeBy(31)
        runCurrent()
        d.change(5.9f) { live += it }
        advanceTimeBy(31)
        runCurrent()
        assertEquals(listOf(6f), live)
    }

    @Test
    fun wpSl1TypedAndResetValuesAreKeptAsGiven() = runTest {
        val commits = mutableListOf<Float>()
        val d = SliderDragState(backgroundScope, initial = 3f, quantize = ::roundDelayToWholeMs)
        d.set(2.25f) { commits += it }
        assertEquals(2.25f, d.value, 0f)
        // a fractional value from the device is shown as it is until a drag
        d.sync(0.5f)
        assertEquals(0.5f, d.value, 0f)
        d.set(0f) { commits += it }
        assertEquals(listOf(2.25f, 0f), commits)
    }

    @Test
    fun wpSl1WithoutQuantizeTheGainSliderIsUnchanged() = runTest {
        val commits = mutableListOf<Float>()
        val d = SliderDragState(backgroundScope, initial = -3.25f)
        d.editing(true) {}
        d.change(-2.37f) {}
        d.editing(false) { commits += it }
        assertEquals(listOf(-2.37f), commits)
        assertEquals(-2.37f, d.value, 0f)
    }
}
