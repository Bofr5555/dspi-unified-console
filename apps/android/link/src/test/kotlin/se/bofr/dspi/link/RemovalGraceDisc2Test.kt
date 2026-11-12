package se.bofr.dspi.link

import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.StandardTestDispatcher
import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.advanceTimeBy
import kotlinx.coroutines.test.runCurrent
import org.junit.Assert.assertEquals
import org.junit.Test

/** WP DISC2: a lost hub stays listed for the grace, on virtual time. */
@OptIn(ExperimentalCoroutinesApi::class)
class RemovalGraceDisc2Test {
    private val scope = TestScope(StandardTestDispatcher())
    private val removed = ArrayList<String>()
    private val grace = RemovalGrace(scope, 3_000L) { removed += it }

    @Test
    fun disc2ALostHubLeavesWhenTheGraceIsOver() {
        grace.lost("Studio")
        scope.advanceTimeBy(2_999)
        scope.runCurrent()
        assertEquals(emptyList<String>(), removed)
        scope.advanceTimeBy(2)
        scope.runCurrent()
        assertEquals(listOf("Studio"), removed)
        assertEquals(0, grace.pending)
    }

    @Test
    fun disc2AHubFoundAgainInTimeStays() {
        grace.lost("Studio")
        scope.advanceTimeBy(800)
        grace.back("Studio")
        scope.advanceTimeBy(10_000)
        scope.runCurrent()
        assertEquals(emptyList<String>(), removed)
    }

    @Test
    fun disc2ASecondGoodbyeDoesNotPutTheEndOff() {
        grace.lost("Studio")
        scope.advanceTimeBy(2_000)
        grace.lost("Studio")
        scope.advanceTimeBy(1_001)
        scope.runCurrent()
        assertEquals(listOf("Studio"), removed)
    }

    @Test
    fun disc2ClearForgetsPendingRemovals() {
        grace.lost("A")
        grace.lost("B")
        grace.clear()
        scope.advanceTimeBy(10_000)
        scope.runCurrent()
        assertEquals(emptyList<String>(), removed)
    }

    @Test
    fun disc2TheGraceIsTheCoresThreeSeconds() {
        assertEquals(3_000L, RemovalGrace.REMOVAL_GRACE_MS)
    }
}
