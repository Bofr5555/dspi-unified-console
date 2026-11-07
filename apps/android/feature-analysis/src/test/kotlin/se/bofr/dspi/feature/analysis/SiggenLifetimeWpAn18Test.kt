package se.bofr.dspi.feature.analysis

import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.StandardTestDispatcher
import kotlinx.coroutines.test.advanceTimeBy
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import se.bofr.dspi.bridge.DspiBridgeImpl
import se.bofr.dspi.core.DspiConstants
import se.bofr.dspi.fake.FakeDspiCore

/** WP AN18: the generator keeps running when its page leaves the screen, as on macOS. */
@OptIn(ExperimentalCoroutinesApi::class)
class SiggenLifetimeWpAn18Test {
    @Test
    fun wpAn18LeavingThePageDoesNotStopTheGenerator() = runTest {
        val core = FakeDspiCore()
        val d = StandardTestDispatcher(testScheduler)
        val b = DspiBridgeImpl(core, backgroundScope, ffi = d, main = d, clock = { testScheduler.currentTime })
        assertTrue(b.attachDevice(42, core.serial, DspiConstants.VENDOR_ID))
        runCurrent()
        SiggenLifetime.shown(b.siggen)
        runCurrent()
        assertTrue(b.siggen.start())
        runCurrent()
        assertTrue(b.siggen.status.value.isRunning)
        val controls = core.callCount("siggenControl")

        SiggenLifetime.hidden(b.siggen)
        advanceTimeBy(10_000)
        runCurrent()
        assertEquals("nothing is sent on leaving", controls, core.callCount("siggenControl"))
        val polls = core.callCount("siggenGetStatus")
        advanceTimeBy(5_000)
        runCurrent()
        assertEquals("no status poll off-screen", polls, core.callCount("siggenGetStatus"))

        SiggenLifetime.shown(b.siggen)
        advanceTimeBy(1_000)
        runCurrent()
        assertTrue("back on screen it is still running", b.siggen.status.value.isRunning)
        assertEquals(controls, core.callCount("siggenControl"))
        SiggenLifetime.hidden(b.siggen)
    }
}
