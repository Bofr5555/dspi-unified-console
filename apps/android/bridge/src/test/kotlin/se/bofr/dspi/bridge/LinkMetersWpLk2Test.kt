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

/** WP LK2: over DSPi Link the meters start at once and never wait behind the start-up reads (WP LK4: from the open on). */
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
            // -- WP LKFAST -- the control surfaces wait for their page (Settings reads them when it opens)
            waitFor("the generator after the meters") { core.callCount("siggenFetchCaps") >= 1 }
            assertEquals("no control-surface read at the open", 0, core.callCount("csFetch"))
        } finally {
            core.release.countDown()
            b.close()
            scope.cancel()
            ffiExec.shutdownNow()
            mainExec.shutdownNow()
        }
    }

    /** A fake whose linkSelectDevice is held after the image (the probe's feature reads on a slow link). */
    private class SlowSelect : FakeDspiCore() {
        val release = CountDownLatch(1)

        @Volatile var selectReturned = false

        override fun linkSelectDevice(handle: Long, serial: String) {
            super.linkSelectDevice(handle, serial)
            release.await(10, TimeUnit.SECONDS)
            selectReturned = true
        }
    }

    @Test
    fun wpLk4TheMeterPollRunsWhileLinkSelectDeviceBlocks() {
        val core = SlowSelect().linked()
        val ffiExec = Executors.newSingleThreadExecutor()
        val mainExec = Executors.newSingleThreadExecutor()
        val ffi = ffiExec.asCoroutineDispatcher()
        val main = mainExec.asCoroutineDispatcher()
        val scope = CoroutineScope(SupervisorJob() + main)
        val b = DspiBridgeImpl(core, scope, ffi = ffi, main = main)
        try {
            scope.launch { b.attachRemote(0, core.serial) }
            waitFor("the select to start") { core.callCount("linkSelectDevice") == 1 }
            waitFor("meter reads while the select is held") { core.callCount("fetchStatus") >= 3 }
            assertTrue("the select has not returned", !core.selectReturned)
            assertEquals("no start-up read yet", 0, core.callCount("fetchAll"))
            core.release.countDown()
            waitFor("the connection") { b.status.value.connected }
            assertEquals(1, core.callCount("fetchAll"))
        } finally {
            core.release.countDown()
            b.close()
            scope.cancel()
            ffiExec.shutdownNow()
            mainExec.shutdownNow()
        }
    }

    /** A fake whose analyser start is held (a probe read on a slow link). */
    private class SlowRta : FakeDspiCore() {
        val release = CountDownLatch(1)

        override fun rtaStart() {
            release.await(10, TimeUnit.SECONDS)
            super.rtaStart()
        }
    }

    @Test
    fun wpLk4OverLinkTheDeviceShowsBeforeTheAnalyserStarts() {
        val core = SlowRta().linked()
        val ffiExec = Executors.newSingleThreadExecutor()
        val mainExec = Executors.newSingleThreadExecutor()
        val ffi = ffiExec.asCoroutineDispatcher()
        val main = mainExec.asCoroutineDispatcher()
        val scope = CoroutineScope(SupervisorJob() + main)
        val b = DspiBridgeImpl(core, scope, ffi = ffi, main = main)
        try {
            scope.launch { b.attachRemote(0, core.serial) }
            waitFor("the connection") { b.status.value.connected }
            assertEquals("the analyser start still held", 0, core.callCount("fetchCrossfeedPairMap"))
            core.release.countDown()
            waitFor("the pair-map probe after it") { core.callCount("fetchCrossfeedPairMap") >= 1 }
        } finally {
            core.release.countDown()
            b.close()
            scope.cancel()
            ffiExec.shutdownNow()
            mainExec.shutdownNow()
        }
    }

    @Test
    fun wpLk4AFailedSelectStopsThePoll() = runTest {
        val core = FakeDspiCore().apply { closeFd() } // nothing open, no Link session: the select fails
        val b = newBridge(core)
        assertTrue(!b.attachRemote(0, core.serial))
        runCurrent()
        val polled = core.callCount("fetchStatus")
        advanceTimeBy(DspiBridgeImpl.STATUS_POLL_MS * 5)
        runCurrent()
        assertEquals("no meter poll after a failed open", polled, core.callCount("fetchStatus"))
        b.close()
    }

    @Test
    fun wpLk2OverLinkThePagesReadsWaitForTheFirstMeter() = runTest {
        val core = FakeDspiCore().linked()
        val b = newBridge(core)
        assertTrue(b.attachRemote(0, core.serial))
        runCurrent()
        assertTrue(b.connected)
        assertEquals(0, core.callCount("siggenFetchCaps"))
        advanceTimeBy(DspiBridgeImpl.STATUS_POLL_MS * 3)
        runCurrent()
        // -- WP LKFAST -- the generator on its own lane (a thread), the control surfaces at their page
        waitFor("the generator read") { core.callCount("siggenFetchCaps") == 1 }
        assertEquals(0, core.callCount("csFetch"))
        val calls = core.calls.toList()
        assertTrue(calls.indexOfFirst { it == "fetchStatus" } < calls.indexOfFirst { it == "siggenFetchCaps" })
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
        // -- WP LKFAST -- the generator at once (on its own lane), the control surfaces at their page
        waitFor("the generator read") { core.callCount("siggenFetchCaps") == 1 }
        assertEquals(0, core.callCount("csFetch"))
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
