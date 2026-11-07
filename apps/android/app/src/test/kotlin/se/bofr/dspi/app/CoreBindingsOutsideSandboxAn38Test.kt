package se.bofr.dspi.app

import android.app.Application
import android.os.Looper
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import org.junit.Assert.assertNotSame
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config
import se.bofr.dspi.core.DspiCoreApi
import se.bofr.dspi.ffi.FfiDspiCore
import se.bofr.dspi.ffi.uniffi.FfiConverterTypeRtaListener

/**
 * WP AN38: in the sandbox the real core's Kotlin side is the test JVM's own copy, so the Rust library's one
 * callback vtable per listener interface reaches the same handle maps whichever test called the core
 * first ([DspiRobolectricTestRunner]). Sorts before [DemoCoresInSequenceAn38Test], which reaches the
 * core outside the sandbox: on the stock runner the analyser tests after both lost their wake.
 */
@RunWith(DspiRobolectricTestRunner::class)
@Config(sdk = [35], application = Application::class)
class CoreBindingsOutsideSandboxAn38Test {
    @Test
    fun an38TheSandboxUsesTheTestJvmsBindings() {
        val sandbox = javaClass.classLoader
        for (shared in listOf(FfiConverterTypeRtaListener::class.java, FfiDspiCore::class.java, DspiCoreApi::class.java)) {
            val loader = shared.classLoader
            assertNotSame(shared.name, sandbox, loader)
            assertTrue("${shared.name} comes from the sandbox's parent", generateSequence(sandbox) { it.parent }.any { it === loader })
        }
    }

    @Test
    fun an38TheDemoAnalyserWakesInTheSandbox() {
        val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
        val session = startDemoSession(scope, "an38")
        try {
            val end = System.currentTimeMillis() + 15_000
            while (session.bridge.rta.caps.value?.supported != true) {
                check(System.currentTimeMillis() < end) { "timed out: analyser caps" }
                shadowOf(Looper.getMainLooper()).idle()
                Thread.sleep(10)
            }
        } finally {
            session.close()
            scope.cancel()
        }
    }
}
