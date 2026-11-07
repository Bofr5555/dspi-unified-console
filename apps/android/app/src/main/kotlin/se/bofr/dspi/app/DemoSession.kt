package se.bofr.dspi.app

import java.util.concurrent.Executors
import kotlinx.coroutines.CoroutineDispatcher
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.asCoroutineDispatcher
import kotlinx.coroutines.cancel
import kotlinx.coroutines.job
import se.bofr.dspi.bridge.DspiBridge
import se.bofr.dspi.bridge.DspiBridgeImpl
import se.bofr.dspi.bridge.InMemoryLinkStore
import se.bofr.dspi.core.DspiCoreApi
import se.bofr.dspi.ffi.FfiDspiCore

/**
 * The demo stack (WP AN28): the real core on its demo device ([DemoCore]) behind
 * its own [DspiBridgeImpl] with a child scope of [parent], its own FFI thread and
 * in-memory links (demo links are not persisted). No USB is involved, and the core
 * only ever sees the simulated device; the bridge selects it at once.
 * [DemoSession.close] closes the bridge (timers, controllers, core listeners) and
 * cancels the scope; the core is closed last on the FFI thread, after any call
 * still running there, because closing it waits for its reader threads.
 */
fun startDemoSession(
    parent: CoroutineScope,
    appVersion: String,
    main: CoroutineDispatcher = Dispatchers.Main.immediate,
    core: DspiCoreApi = DemoCore(FfiDspiCore()),
    log: (String) -> Unit = {}
): DemoSession {
    val executor = Executors.newSingleThreadExecutor { r -> Thread(r, "dspi-demo-ffi").apply { isDaemon = true } }
    val ffi = executor.asCoroutineDispatcher()
    val scope = CoroutineScope(SupervisorJob(parent.coroutineContext.job) + main)
    val demoBridge = DspiBridgeImpl(core, scope, ffi = ffi, main = main, linkStore = InMemoryLinkStore(), appVersion = appVersion)
    log("demo mode on")
    return object : DemoSession {
        override val bridge: DspiBridge = demoBridge

        override fun close() {
            if (executor.isShutdown) return
            // review A2: timers, the four controllers and the core's listeners first
            runCatching { demoBridge.close() }
            scope.cancel()
            executor.execute {
                runCatching { core.disconnect() }
                runCatching { core.close() }
            }
            executor.shutdown()
            log("demo mode off")
        }
    }
}
