package se.bofr.dspi.bridge

import java.util.concurrent.CountDownLatch
import java.util.concurrent.Executors
import java.util.concurrent.TimeUnit
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.asCoroutineDispatcher
import kotlinx.coroutines.cancel
import kotlinx.coroutines.launch
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import se.bofr.dspi.core.SiggenCaps
import se.bofr.dspi.fake.FakeDspiCore

/**
 * WP LKFAST: over DSPi Link the start-up reads of what Home does not show
 * never hold up a user's call: they run on their own lane (they wait for the
 * core's background probe), and the control surfaces wait for their page.
 */
class LinkStartupWpLkfastTest {
    /** A fake on a Link session whose generator read is held (it waits for the background probe). */
    private class HeldStartup : FakeDspiCore() {
        val release = CountDownLatch(1)

        @Volatile var asked = false

        override fun siggenFetchCaps(): SiggenCaps {
            asked = true
            release.await(10, TimeUnit.SECONDS)
            return super.siggenFetchCaps()
        }
    }

    private fun waitFor(what: String, f: () -> Boolean) {
        val until = System.nanoTime() + 5_000_000_000L
        while (!f()) {
            assertTrue("timed out waiting for $what", System.nanoTime() < until)
            Thread.sleep(10)
        }
    }

    private fun withBridge(core: FakeDspiCore, body: (DspiBridgeImpl) -> Unit) {
        val ffiExec = Executors.newSingleThreadExecutor()
        val mainExec = Executors.newSingleThreadExecutor()
        val scope = CoroutineScope(SupervisorJob() + mainExec.asCoroutineDispatcher())
        val b = DspiBridgeImpl(core, scope, ffi = ffiExec.asCoroutineDispatcher(), main = mainExec.asCoroutineDispatcher())
        try {
            body(b)
        } finally {
            b.close()
            scope.cancel()
            ffiExec.shutdownNow()
            mainExec.shutdownNow()
        }
    }

    @Test
    fun wpLkfastAUserCallOverLinkDoesNotWaitForTheStartupReads() {
        val core = HeldStartup().apply {
            closeFd()
            linkConnect("ws://10.0.2.2:11915/dspi/v1", "Pixel", "test", "", "")
        }
        withBridge(core) { b ->
            b.scope.launch { b.attachRemote(0, core.serial) }
            waitFor("the connection") { b.status.value.connected }
            waitFor("the held generator read") { core.asked }
            // A user's call on the FFI thread goes through while the start-up read is held
            runBlocking { withTimeout(2_000) { b.onFfi { core.setMasterVolume(-20f) } } }
            assertEquals(1, core.callCount("setMasterVolume"))
            assertEquals("the control surfaces wait for their page", 0, core.callCount("csFetch"))
            core.release.countDown()
            // Settings opening without them reads them (SettingsScreen)
            b.cs.refresh()
            waitFor("the control surfaces on demand") { core.callCount("csFetch") == 1 }
        }
    }

    @Test
    fun wpLkfastOnUsbTheStartupReadsStayOnTheFfiThread() {
        val core = FakeDspiCore()
        withBridge(core) { b ->
            b.selectDevice(core.serial)
            waitFor("the connection") { b.status.value.connected }
            waitFor("the control surfaces at the open, as before") { core.callCount("csFetch") >= 1 }
            val thread = runBlocking { b.onStartup { Thread.currentThread() } }
            val ffiThread = runBlocking { b.onFfi { Thread.currentThread() } }
            assertEquals(ffiThread, thread)
        }
    }
}
