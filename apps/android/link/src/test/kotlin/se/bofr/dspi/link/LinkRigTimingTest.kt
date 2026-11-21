package se.bofr.dspi.link

import java.lang.reflect.InvocationHandler
import java.lang.reflect.InvocationTargetException
import java.lang.reflect.Proxy
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.Executors
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.asCoroutineDispatcher
import kotlinx.coroutines.async
import kotlinx.coroutines.cancel
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.combine
import kotlinx.coroutines.launch
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withContext
import kotlinx.coroutines.withTimeoutOrNull
import org.junit.Assume.assumeTrue
import org.junit.Test
import se.bofr.dspi.bridge.DspiBridgeImpl
import se.bofr.dspi.bridge.InMemoryLinkStore
import se.bofr.dspi.core.DspiCoreApi
import se.bofr.dspi.core.SystemStatus
import se.bofr.dspi.ffi.FfiDspiCore

/**
 * WP LKFAST: the Android path from a tap on a hub to Home and its first moving
 * meter, timed against the emulated ESP32 bridge (tools/esp-rig, `android.sh`).
 * The stack is the app's REMOTE stack as `startRemoteSession` builds it: the
 * real core (FfiDspiCore), DspiBridgeImpl on its own serial FFI thread (with
 * the controllers' device-changed hooks), LinkSessionEngine over
 * BridgeLinkCoreOps, one thread as the main thread. Every core call is timed
 * through a proxy, so the breakdown shows which call the time went to.
 *
 * "Home ready" is what Home waits for (`observeDeviceReady`: the bridge's
 * status connected and the state's image valid), as published on the main
 * thread; the Compose frame after it is not included. "Meter live" is the
 * first meter read (`fetchStatus`, the bridge's 60 ms poll) that returns live
 * data (not stale) after Home is ready: the next poll publishes it. The demo
 * device's levels can stay constant, so "moving" means a live frame, not a
 * changed value. With DSPI_RIG_AFTER_MS the run goes on after that and
 * reports how the meters fared while the start-up reads ran in the
 * background (the longest meter age read, the polls that read stale data)
 * and when the last of those reads (the control surfaces) ended. With
 * DSPI_RIG_TOUCH_MS a user action comes that long after Home: how long it
 * waits for the bridge's FFI thread (where every device call queues) and for
 * the core lock (`isConnected` takes it) while those reads run.
 *
 * DSPI_RIG_LINK_DIR names the core's DSPi Link memory folder (`link_set_dir`), kept
 * between runs as between app starts.
 *
 * Skipped unless DSPI_RIG_HOST is set; one run per JVM (the rig script
 * interleaves runs with the reference client).
 */
class LinkRigTimingTest {
    private val host = System.getenv("DSPI_RIG_HOST").orEmpty()
    private val port = System.getenv("DSPI_RIG_PORT")?.toIntOrNull() ?: LINK_DEFAULT_PORT
    private val limitMs = System.getenv("DSPI_RIG_LIMIT_MS")?.toLongOrNull() ?: 60_000L
    private val afterMs = System.getenv("DSPI_RIG_AFTER_MS")?.toLongOrNull() ?: 0L
    private val label = System.getenv("DSPI_RIG_LABEL").orEmpty()
    private val verbose = System.getenv("DSPI_RIG_VERBOSE") == "1"
    private val touchMs = System.getenv("DSPI_RIG_TOUCH_MS")?.toLongOrNull() ?: -1L

    private class Call(val name: String, val thread: String, val start: Long, var end: Long = -1, var note: String = "")

    private var t0 = 0L
    private fun now() = (System.nanoTime() - t0) / 1_000_000

    private val calls = java.util.Collections.synchronizedList(ArrayList<Call>())
    private val counts = ConcurrentHashMap<String, Int>()
    private val marks = java.util.Collections.synchronizedMap(LinkedHashMap<String, Long>())

    @Volatile private var homeReady = false

    // After the first live meter on Home: meter polls, those that read stale data, the oldest age read
    @Volatile private var polls = 0

    @Volatile private var stalePolls = 0

    @Volatile private var maxAgeMs = 0L

    private fun mark(name: String) {
        if (t0 != 0L) marks.putIfAbsent(name, now())
    }

    /** Polling calls are kept only for their first few, so the timeline stays readable. */
    private val sampled = setOf("fetchStatus", "linkStatus", "isConnected", "linkDevices", "linkSecurity", "getState")

    private fun timed(real: DspiCoreApi): DspiCoreApi = Proxy.newProxyInstance(
        DspiCoreApi::class.java.classLoader,
        arrayOf(DspiCoreApi::class.java),
        InvocationHandler { _, m, args ->
            val n = counts.merge(m.name, 1, Int::plus) ?: 1
            val keep = t0 != 0L && (m.name !in sampled || n <= 3)
            val c = Call(m.name, Thread.currentThread().name, if (t0 != 0L) now() else -1)
            if (keep) calls += c
            try {
                val r = m.invoke(real, *(args ?: emptyArray()))
                if (m.name == "fetchStatus" && r is SystemStatus) {
                    if (!r.stale) mark("core: first live meter (fetchStatus not stale)")
                    if (homeReady && !r.stale) mark(METER)
                    if (METER in marks) {
                        polls++
                        if (r.stale) stalePolls++
                        if (r.ageMs > maxAgeMs) maxAgeMs = r.ageMs
                    }
                }
                r
            } catch (e: InvocationTargetException) {
                c.note = e.targetException.javaClass.simpleName
                throw e.targetException
            } finally {
                if (keep) c.end = now()
            }
        }
    ) as DspiCoreApi

    @Test
    fun wpLkfastTapToHomeAndMovingMeters() = runBlocking<Unit> {
        assumeTrue("DSPI_RIG_HOST not set: no rig", host.isNotEmpty())
        val mainExecutor = Executors.newSingleThreadExecutor { r -> Thread(r, "main").apply { isDaemon = true } }
        val main = mainExecutor.asCoroutineDispatcher()
        val ffiExecutor = Executors.newSingleThreadExecutor { r -> Thread(r, "dspi-link-ffi").apply { isDaemon = true } }
        val ffi = ffiExecutor.asCoroutineDispatcher()
        val scope = CoroutineScope(SupervisorJob() + main)
        val real = FfiDspiCore()
        // -- WP LKFAST -- the core's hub memory, kept between runs as between app starts
        System.getenv("DSPI_RIG_LINK_DIR")?.takeIf { it.isNotEmpty() }?.let { real.linkSetDir(it) }
        val core = timed(real)
        // The hub as DNS-SD would list it: its id from its info document (before the tap, not timed)
        val info = (1..5).firstNotNullOfOrNull { runCatching { real.linkFetchInfo(host, port) }.getOrNull() }
            ?: real.linkFetchInfo(host, port)
        val hub = LinkHub(info.hubId, info.hubName, info.hubKind, info.auth, 1, emptyList(), host, port, info.wsPath)
        val bridge = DspiBridgeImpl(core, scope, ffi = ffi, main = main, linkStore = InMemoryLinkStore(), appVersion = "rig")
        val tokens = object : LinkTokenStore {
            val m = HashMap<String, Pair<String, String>>()
            override fun token(hubId: String) = m[hubId]?.second
            override fun setToken(hubId: String, name: String, token: String) {
                m[hubId] = name to token
            }
            override fun remove(hubId: String) {
                m.remove(hubId)
            }
            override fun paired() = m.map { PairedHub(it.key, it.value.first) }
        }
        val engine = withContext(main) {
            bridge.setUiVisible(true)
            LinkSessionEngine(BridgeLinkCoreOps(core, bridge, ffi, Dispatchers.IO), tokens, { "rig" }, "rig", scope)
        }
        val watch = scope.launch {
            launch {
                engine.state.collect { s -> mark("engine: ${s::class.simpleName}") }
            }
            // Home observes both (observeDeviceReady): the status goes connected before the state is published
            combine(bridge.status, bridge.state) { st, state -> st.connected && state.bulkValid }.collect { ready ->
                if (ready && !homeReady) {
                    homeReady = true
                    mark("HOME READY (connected + image)")
                }
            }
        }
        t0 = System.nanoTime()
        withContext(main) { engine.connect(hub) }
        val done = withTimeoutOrNull(limitMs) {
            while (METER !in marks) delay(5)
        } != null
        var touch = ""
        if (done && touchMs >= 0) {
            delay(touchMs)
            val asked = System.nanoTime()
            val lockWait = scope.async(Dispatchers.IO) {
                real.isConnected()
                (System.nanoTime() - asked) / 1_000_000
            }
            withContext(ffi) {}
            val ffiWait = (System.nanoTime() - asked) / 1_000_000
            touch = " ffi_wait_ms=$ffiWait lock_wait_ms=${lockWait.await()}"
        }
        if (afterMs > 0) delay(afterMs)
        val m = HashMap(marks)
        val home = m["HOME READY (connected + image)"]
        val meter = m[METER]
        println(
            "LKFAST $label home=${home?.let { "%.3f".format(it / 1000.0) } ?: "never"} " +
                "meter=${meter?.let { "%.3f".format(it / 1000.0) } ?: "never"} ok=$done" +
                if (afterMs > 0) {
                    val cs = synchronized(calls) { calls.lastOrNull { it.name == "csFetch" }?.end ?: -1 }
                    " polls=$polls stale=$stalePolls max_age_ms=$maxAgeMs cs_done=${if (cs < 0) "no" else "%.3f".format(cs / 1000.0)}"
                } else {
                    ""
                } + touch
        )
        if (verbose) {
            m.entries.sortedBy { it.value }.forEach { (k, v) -> println("LKFAST mark %8d ms  %s".format(v, k)) }
            synchronized(calls) {
                calls.sortedBy { it.start }.forEach { c ->
                    val end = if (c.end < 0) "open" else c.end.toString()
                    println("LKFAST call %8d..%-8s %-14s %s %s".format(c.start, end, c.thread, c.name, c.note))
                }
            }
            println("LKFAST counts ${counts.entries.sortedBy { it.key }.joinToString(" ") { "${it.key}=${it.value}" }}")
        }
        watch.cancel()
        withContext(main) { engine.close() }
        scope.cancel()
        real.close()
        ffi.close()
        main.close()
    }

    private companion object {
        const val METER = "METER LIVE on Home"
    }
}
