package se.bofr.dspi.bridge

import java.util.concurrent.CountDownLatch
import java.util.concurrent.Executors
import java.util.concurrent.TimeUnit
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.asCoroutineDispatcher
import kotlinx.coroutines.cancel
import kotlinx.coroutines.launch
import kotlinx.coroutines.test.advanceTimeBy
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import se.bofr.dspi.core.DspiConstants
import se.bofr.dspi.fake.FakeDspiCore

/** WP LK2: over DSPi Link the meters start at once and never wait behind the start-up reads. */
@OptIn(ExperimentalCoroutinesApi::class)
class LinkMetersWpLk2Test {
    /** A fake on a Link session whose fetchAll blocks until [release] (a slow link's start-up read). */
    private class SlowStart : FakeDspiCore() {
        val release = CountDownLatch(1)

        override fun fetchAll() {
            release.await(10, TimeUnit.SECONDS)
            super.fetchAll()
        }
    }

    private fun <T : FakeDspiCore> T.linked(): T = apply {
        closeFd()
        linkConnect("ws://10.0.2.2:11915/dspi/v1", "Pixel", "test", "", "")
    }

    private fun waitFor(what: String, f: () -> Boolean) {
        val until = System.nanoTime() + 5_000_000_000L
        while (!f()) {
            assertTrue("timed out waiting for $what", System.nanoTime() < until)
            Thread.sleep(10)
        }
    }

    @Test
    fun wpLk2TheMeterPollRunsWhileFetchAllBlocks() {
        val core = SlowStart().linked()
        val ffiExec = Executors.newSingleThreadExecutor()
        val mainExec = Executors.newSingleThreadExecutor()
        val ffi = ffiExec.asCoroutineDispatcher()
        val main = mainExec.asCoroutineDispatcher()
        val scope = CoroutineScope(SupervisorJob() + main)
        val b = DspiBridgeImpl(core, scope, ffi = ffi, main = main)
        try {
            assertTrue("a separate meter thread", b.meters !== b.ffi)
            scope.launch { b.attachRemote(0, core.serial) }
            waitFor("fetchAll to start") { core.callCount("linkSelectDevice") == 1 }
            waitFor("meter reads while fetchAll is held") { core.callCount("fetchStatus") >= 3 }
            assertEquals("fetchAll has not returned", 0, core.callCount("fetchAll"))
            assertTrue("the meters are shown", b.status.value.peaks.any { it > 0f })
            assertTrue("the connection is not shown before the read", !b.status.value.connected)
            core.release.countDown()
            waitFor("the connection") { b.status.value.connected }
            waitFor("the control surfaces after the meters") { core.callCount("csFetch") >= 1 }
        } finally {
            core.release.countDown()
            b.close()
            scope.cancel()
            ffiExec.shutdownNow()
            mainExec.shutdownNow()
        }
    }

    @Test
    fun wpLk2OverLinkThePagesReadsWaitForTheFirstMeter() = runTest {
        val core = FakeDspiCore().linked()
        val b = newBridge(core)
        assertTrue(b.attachRemote(0, core.serial))
        runCurrent()
        assertTrue(b.connected)
        assertEquals("no control-surface read before a meter", 0, core.callCount("csFetch"))
        assertEquals(0, core.callCount("siggenFetchCaps"))
        advanceTimeBy(DspiBridgeImpl.STATUS_POLL_MS * 3)
        runCurrent()
        assertEquals(1, core.callCount("csFetch"))
        assertEquals(1, core.callCount("siggenFetchCaps"))
        val calls = core.calls.toList()
        assertTrue(calls.indexOfFirst { it == "fetchStatus" } < calls.indexOfFirst { it == "csFetch" })
        b.close()
    }

    @Test
    fun wpLk2OverLinkWithNoUiThePagesReadAtOnce() = runTest {
        val core = FakeDspiCore().linked()
        val b = newBridge(core)
        b.setUiVisible(false)
        assertTrue(b.attachRemote(0, core.serial))
        runCurrent()
        assertEquals(0, core.callCount("fetchStatus"))
        assertEquals(1, core.callCount("csFetch"))
        b.close()
    }

    @Test
    fun wpLk2OverUsbTheOrderIsUnchanged() = runTest {
        val core = FakeDspiCore()
        val b = newBridge(core)
        assertTrue(b.attachDevice(42, core.serial, DspiConstants.VENDOR_ID))
        runCurrent()
        assertEquals("the hooks at once", 1, core.callCount("csFetch"))
        assertEquals("no meter before the device is read", 0, core.callCount("fetchStatus"))
        advanceTimeBy(DspiBridgeImpl.STATUS_POLL_MS * 2)
        runCurrent()
        assertTrue(core.callCount("fetchStatus") >= 1)
        assertTrue("one dispatcher for everything", b.meters === b.ffi)
        b.close()
    }
}
