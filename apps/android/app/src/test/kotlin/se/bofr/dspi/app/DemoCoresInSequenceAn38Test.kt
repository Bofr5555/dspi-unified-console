package se.bofr.dspi.app

import java.io.File
import java.util.concurrent.Executors
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.asCoroutineDispatcher
import kotlinx.coroutines.cancel
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withContext
import kotlinx.coroutines.withTimeoutOrNull
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Test

/**
 * WP AN38: leaving demo mode and entering it again (or switching devices) makes a new real core in
 * the same process; its analyser must wake its bridge as the first one did. Outside Robolectric, so
 * the bindings load in the test JVM's own classloader, as the app's on a phone. The suite's sandbox
 * tests run in this JVM too and share that copy (DspiRobolectricTestRunner).
 */
class DemoCoresInSequenceAn38Test {
    @Test
    fun an38EachNewDemoCoreWakesItsAnalyser() = runBlocking {
        val mainExecutor = Executors.newSingleThreadExecutor()
        val main = mainExecutor.asCoroutineDispatcher()
        val parent = CoroutineScope(SupervisorJob() + main)
        try {
            for (n in 1..2) {
                val session = startDemoSession(parent, "an38-$n", main = main)
                try {
                    val caps = withTimeoutOrNull(15_000) {
                        session.bridge.status.first { it.connected }
                        session.bridge.rta.caps.first { it?.supported == true }
                    }
                    assertNotNull("demo core $n: the analyser never woke the bridge", caps)
                } finally {
                    withContext(main) { session.close() }
                }
            }
        } finally {
            parent.cancel()
            main.close()
        }
    }

    /** A test on the stock runner gets a sandbox copy of the bindings again, and the fault back. */
    @Test
    fun an38EveryRobolectricTestHereSharesTheBindings() {
        val stock = "@RunWith(" + "RobolectricTestRunner::class)"
        val offenders = File("src/test/kotlin").walk().filter { it.extension == "kt" && stock in it.readText() }.map { it.name }.toList()
        assertEquals("use DspiRobolectricTestRunner", emptyList<String>(), offenders)
    }
}
