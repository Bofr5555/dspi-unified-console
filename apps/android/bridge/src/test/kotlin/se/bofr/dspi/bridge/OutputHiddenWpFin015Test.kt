package se.bofr.dspi.bridge

import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.advanceTimeBy
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import se.bofr.dspi.core.DspiConstants
import se.bofr.dspi.core.OutputLabel
import se.bofr.dspi.fake.FakeDspiCore

/**
 * -- WP FIN015 -- (REVF R1) An unused output (nothing routed to it) the signal generator plays on is not
 * hidden: the generator writes after the matrix, so it is loud. The views that only show levels (the Home's
 * meters, the Spectrum chips, Stats) read the core's `OutputLabel.hidden` through [DspiBridge.outputHidden],
 * and a generator start or stop alone (NOTIFY_SIGGEN, no state change) brings the new labels.
 */
@OptIn(ExperimentalCoroutinesApi::class)
class OutputHiddenWpFin015Test {
    /** Output 2 has nothing routed to it; the generator plays on it while [playing]. */
    private class Core : FakeDspiCore() {
        @Volatile var playing = false

        override fun outputLabels(): List<OutputLabel> = super.outputLabels().map {
            if (it.output == 2) it.copy(label = "3 · None", unused = true, hidden = !playing) else it
        }
    }

    @Test
    fun anUnusedOutputTheGeneratorPlaysOnKeepsItsMeter() = runTest {
        val core = Core()
        val b = newBridge(core)
        b.setUiVisible(false)
        attach(b, core)
        advanceTimeBy(100)
        runCurrent()
        assertTrue(b.outputUnused(2))
        assertTrue("silent and unrouted: hidden", b.outputHidden(2))

        val rev = b.stateRevision.value
        core.playing = true
        core.simulateRemote(flags = DspiConstants.NOTIFY_SIGGEN)
        advanceTimeBy(40)
        runCurrent()
        assertTrue("still unused", b.outputUnused(2))
        assertFalse("but not hidden while the generator plays on it", b.outputHidden(2))
        assertTrue("the views recompose", b.stateRevision.value > rev)

        core.playing = false
        core.simulateRemote(flags = DspiConstants.NOTIFY_SIGGEN)
        advanceTimeBy(40)
        runCurrent()
        assertTrue("stopped: hidden again", b.outputHidden(2))
    }
}
