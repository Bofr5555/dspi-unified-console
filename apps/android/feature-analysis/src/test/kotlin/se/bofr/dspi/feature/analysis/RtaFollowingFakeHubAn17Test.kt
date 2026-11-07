package se.bofr.dspi.feature.analysis

import java.io.File
import java.util.concurrent.Executors
import java.util.concurrent.TimeUnit
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.asCoroutineDispatcher
import kotlinx.coroutines.cancel
import kotlinx.coroutines.delay
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withContext
import kotlinx.coroutines.withTimeout
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import se.bofr.dspi.bridge.DspiBridgeImpl
import se.bofr.dspi.bridge.InMemoryLinkStore
import se.bofr.dspi.core.DspiConstants
import se.bofr.dspi.core.RtaOwnership
import se.bofr.dspi.ffi.FfiDspiCore

/**
 * WP AN17: following another console and Take control, over the real core and
 * bridge, with two consoles on crates/dspi-link's fake hub (`--rta`) as a
 * process. Real sockets, no USB.
 */
class RtaFollowingFakeHubAn17Test {
    private val hub: Process = ProcessBuilder(listOf(System.getProperty("dspi.fakehub"), "--bind", "127.0.0.1:0", "--rta"))
        .redirectError(ProcessBuilder.Redirect.to(File("/dev/null")))
        .start()
    private val port: Int = run {
        val banner = hub.inputStream.bufferedReader().readLine()
        Regex("""on 127\.0\.0\.1:(\d+) """).find(banner)?.groupValues?.get(1)?.toInt() ?: error("unexpected banner: $banner")
    }

    /** One console: its own core, bridge and threads, as two apps would have. */
    private inner class Console(val name: String) {
        private val mainExecutor = Executors.newSingleThreadExecutor { r -> Thread(r, "$name-main").apply { isDaemon = true } }
        val main = mainExecutor.asCoroutineDispatcher()
        private val ffiExecutor = Executors.newSingleThreadExecutor { r -> Thread(r, "$name-ffi").apply { isDaemon = true } }
        val ffi = ffiExecutor.asCoroutineDispatcher()
        val scope = CoroutineScope(SupervisorJob() + main)
        val core = FfiDspiCore()
        val bridge = DspiBridgeImpl(core, scope, ffi = ffi, main = main, linkStore = InMemoryLinkStore())

        suspend fun connect() {
            val device = withContext(ffi) {
                core.linkConnect(core.linkWsUrl("127.0.0.1", port, "/dspi/v1"), name, "test", "", "")
                core.linkDevices().first()
            }
            assertTrue(withContext(main) { bridge.attachRemote(device.handle, device.serial) })
            withTimeout(10_000) { while (withContext(main) { bridge.rta.caps.value?.supported } != true) delay(20) }
        }

        suspend fun watch(tap: Int, mask: Int) = withContext(main) { bridge.rta.update(1, tap, mask, false) }

        suspend fun ownership(): RtaOwnership? = withContext(main) { bridge.rta.ownership.value }

        suspend fun awaitOwnership(what: String, until: (RtaOwnership?) -> Boolean): RtaOwnership? = withTimeout(10_000) {
            var o = ownership()
            while (!until(o)) {
                delay(20)
                o = ownership()
            }
            o
        }.also { assertTrue(what, until(it)) }

        fun close() {
            runBlocking { withContext(main) { bridge.close() } }
            scope.cancel()
            runCatching { core.linkClose() }
            core.close()
            ffi.close()
            main.close()
        }
    }

    private val consoles = ArrayList<Console>()

    private fun console(name: String) = Console(name).also { consoles += it }

    @After
    fun tearDown() {
        consoles.forEach { it.close() }
        hub.destroyForcibly().waitFor(5, TimeUnit.SECONDS)
    }

    @Test
    fun an17AnotherConsolesConfigIsFollowedAndTakeControlWinsItBack() = runBlocking<Unit> {
        val a = console("an17-a")
        val b = console("an17-b")
        a.connect()
        b.connect()
        val ownA = RtaChannelSelection.of(DspiConstants.RTA_TAP_OUTPUT, listOf(0))
        val ownB = RtaChannelSelection.of(DspiConstants.RTA_TAP_INPUT, listOf(0, 1))

        // A drives: its own configuration is on the device, so it does not follow
        a.watch(ownA.tap, ownA.mask)
        withTimeout(10_000) { while (withContext(a.main) { a.bridge.rta.snapshot.value?.running } != true) delay(20) }
        a.awaitOwnership("A drives") { it != null && !it.following }

        // B pushes its own: A follows B's tap and channels and draws them; B drives
        b.watch(ownB.tap, ownB.mask)
        val followed = a.awaitOwnership("A follows B") { it?.following == true && it.tap == ownB.tap }
        assertEquals(ownB, RtaChannelSelection.followed(followed))
        assertEquals(ownB, RtaChannelSelection.drawn(ownA, RtaChannelSelection.followed(followed)))
        assertFalse("our own config does not make us follow", b.ownership()?.following == true)

        // Take control: A drives again with its own request, and now B follows A
        withContext(a.main) { a.bridge.rta.takeControl() }
        a.awaitOwnership("A drives again") { it != null && !it.following }
        val bFollows = b.awaitOwnership("B follows A") { it?.following == true && it.tap == ownA.tap }
        assertEquals(ownA, RtaChannelSelection.followed(bFollows))
        assertEquals(ownA, RtaChannelSelection.drawn(ownA, RtaChannelSelection.followed(a.ownership())))
    }
}
