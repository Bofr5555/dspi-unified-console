package se.bofr.dspi.feature.graph

import android.app.Application
import android.os.Looper
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.darkColorScheme
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.snapshots.Snapshot
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.test.SemanticsMatcher
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onAllNodesWithTag
import java.io.File
import java.util.concurrent.Executors
import java.util.concurrent.TimeUnit
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.asCoroutineDispatcher
import kotlinx.coroutines.cancel
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withContext
import kotlinx.coroutines.withTimeout
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config
import se.bofr.dspi.bridge.DspiBridgeImpl
import se.bofr.dspi.bridge.InMemoryLinkStore
import se.bofr.dspi.core.DspiConstants
import se.bofr.dspi.core.LinkDevice
import se.bofr.dspi.ffi.FfiDspiCore
import se.bofr.dspi.ui.settings.AppSettings
import se.bofr.dspi.ui.settings.InMemoryAppSettingsStore
import se.bofr.dspi.ui.settings.LocalAppSettings

/**
 * WP CG30 (the AN39 follow-up, review c5b "c4j N2"): with this console's own
 * spectrum selection cleared, the response graph still draws the spectrum
 * another console drives (GraphView.swift:328 shows the overlay whenever the
 * graph shows the spectrum): the overlay registers as watching with no
 * channel, and the core keeps polling what it follows. Two real cores on
 * crates/dspi-link's fake hub (`--rta`) as a process, as :app's AN34 test: A
 * shows the graph on the main looper, B is headless on its own threads. Real
 * sockets, no USB.
 */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35], application = Application::class, qualifiers = "w400dp-h1200dp-port-mdpi")
class GraphFollowingEmptySelectionCg30Test {
    @get:Rule val rule = createComposeRule()

    private val hub: Process = ProcessBuilder(listOf(System.getProperty("dspi.fakehub"), "--bind", "127.0.0.1:0", "--rta"))
        .redirectError(ProcessBuilder.Redirect.to(File("/dev/null")))
        .start()
    private val port: Int = run {
        val banner = hub.inputStream.bufferedReader().readLine()
        Regex("""on 127\.0\.0\.1:(\d+) """).find(banner)?.groupValues?.get(1)?.toInt() ?: error("unexpected banner: $banner")
    }
    private val url = "/dspi/v1"

    // A: the console on screen, its bridge on the main looper like the app's
    private val aExecutor = Executors.newSingleThreadExecutor { r -> Thread(r, "cg30-a-ffi").apply { isDaemon = true } }
    private val aFfi = aExecutor.asCoroutineDispatcher()
    private val aCore = FfiDspiCore()
    private val aScope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
    private val a = DspiBridgeImpl(aCore, aScope, ffi = aFfi, main = Dispatchers.Main.immediate, linkStore = InMemoryLinkStore())

    // B: another console, headless on its own threads
    private val bMainExecutor = Executors.newSingleThreadExecutor { r -> Thread(r, "cg30-b-main").apply { isDaemon = true } }
    private val bMain = bMainExecutor.asCoroutineDispatcher()
    private val bExecutor = Executors.newSingleThreadExecutor { r -> Thread(r, "cg30-b-ffi").apply { isDaemon = true } }
    private val bFfi = bExecutor.asCoroutineDispatcher()
    private val bCore = FfiDspiCore()
    private val bScope = CoroutineScope(SupervisorJob() + bMain)
    private val b = DspiBridgeImpl(bCore, bScope, ffi = bFfi, main = bMain, linkStore = InMemoryLinkStore())

    @After
    fun tearDown() {
        rule.runOnUiThread { runCatching { a.close() } }
        aScope.cancel()
        aExecutor.execute {
            runCatching { aCore.linkClose() }
            runCatching { aCore.close() }
        }
        aExecutor.shutdown()
        aExecutor.awaitTermination(10, TimeUnit.SECONDS)
        runBlocking { withContext(bMain) { runCatching { b.close() } } }
        bScope.cancel()
        runCatching { bCore.linkClose() }
        bCore.close()
        bExecutor.shutdown()
        bMainExecutor.shutdown()
        hub.destroyForcibly().waitFor(5, TimeUnit.SECONDS)
    }

    private fun pump(what: String, until: () -> Boolean) {
        val end = System.currentTimeMillis() + 15_000
        while (!until()) {
            check(System.currentTimeMillis() < end) { "timed out: $what" }
            shadowOf(Looper.getMainLooper()).idle()
            rule.runOnUiThread { Snapshot.sendApplyNotifications() }
            rule.mainClock.advanceTimeBy(100)
            Thread.sleep(10)
        }
    }

    private fun tagged(tag: String) = rule.onAllNodesWithTag(tag, useUnmergedTree = true).fetchSemanticsNodes().isNotEmpty()

    private val overlay = SemanticsMatcher("a graph spectrum") {
        it.config.getOrElseNullable(SemanticsProperties.TestTag) { null }?.startsWith("graphSpectrum:") == true
    }

    private fun anyOverlay() = rule.onAllNodes(overlay, useUnmergedTree = true).fetchSemanticsNodes().isNotEmpty()

    private fun connectA() {
        val device = aExecutor.submit<LinkDevice> {
            aCore.linkConnect(aCore.linkWsUrl("127.0.0.1", port, url), "cg30-a", "test", "", "")
            aCore.linkDevices().first()
        }.get(10, TimeUnit.SECONDS)
        var attached: Boolean? = null
        aScope.launch { attached = a.attachRemote(device.handle, device.serial) }
        pump("A attached") { attached != null }
        assertTrue(attached == true)
        pump("A's analyser caps") { a.status.value.connected && a.rta.caps.value?.supported == true }
    }

    private fun connectB() = runBlocking {
        val device = withContext(bFfi) {
            bCore.linkConnect(bCore.linkWsUrl("127.0.0.1", port, url), "cg30-b", "test", "", "")
            bCore.linkDevices().first()
        }
        assertTrue(withContext(bMain) { b.attachRemote(device.handle, device.serial) })
        withTimeout(10_000) { while (withContext(bMain) { b.rta.caps.value?.supported } != true) delay(20) }
    }

    private fun bFollowing() = runBlocking { withContext(bMain) { b.rta.ownership.value?.following } }

    @Test
    fun cg30AClearedOwnSelectionStillDrawsAFollowedSpectrum() {
        connectA()
        connectB()
        rule.mainClock.autoAdvance = false
        val store = InMemoryAppSettingsStore(AppSettings(rtaDashboardShowGraph = true, rtaDashboardSelectionKey = "out:0"))
        rule.setContent {
            CompositionLocalProvider(LocalAppSettings provides store) {
                MaterialTheme(colorScheme = darkColorScheme()) { Surface(Modifier.fillMaxSize()) { GraphView(a, Modifier.fillMaxWidth()) } }
            }
        }
        // A chooses output 1 and drives
        val ours = graphSpectrumTag(RtaChannelSelection(DspiConstants.RTA_TAP_OUTPUT, listOf(0)))
        pump("A drives") { tagged(ours) && a.rta.snapshot.value?.count == 1 && a.rta.ownership.value?.following != true }

        // A clears its choice while driving (no take-control: nothing is followed): nothing to draw
        // ("out:" is a deliberate choice of nothing, RtaChannelSelection.storageKey)
        store.update { it.copy(rtaDashboardSelectionKey = "out:") }
        pump("A cleared") { !anyOverlay() && a.rta.snapshot.value?.count == 0 }

        // B pushes inputs 1 and 2: A follows and its graph draws them, from frames the core still polls
        val theirs = graphSpectrumTag(RtaChannelSelection(DspiConstants.RTA_TAP_INPUT, listOf(0, 1)))
        runBlocking { withContext(bMain) { b.rta.update(1, DspiConstants.RTA_TAP_INPUT, 0b11, false) } }
        pump("A follows B and polls") {
            val s = a.rta.snapshot.value
            a.rta.ownership.value?.following == true && tagged(theirs) && s?.tap == DspiConstants.RTA_TAP_INPUT && s.count == 2
        }

        // Drawing only: a while later B still drives and A's empty choice is untouched
        val end = System.currentTimeMillis() + 1_500
        pump("time passes") { System.currentTimeMillis() > end }
        assertEquals(false, bFollowing())
        assertEquals(true, a.rta.ownership.value?.following)
        assertTrue(tagged(theirs))
        assertEquals(2, a.rta.snapshot.value?.count)
        assertEquals("out:", store.settings.value.rtaDashboardSelectionKey)

        // B picks another channel: A's graph follows it
        val next = graphSpectrumTag(RtaChannelSelection(DspiConstants.RTA_TAP_INPUT, listOf(1)))
        runBlocking { withContext(bMain) { b.rta.update(1, DspiConstants.RTA_TAP_INPUT, 0b10, false) } }
        pump("A follows B's new choice") { tagged(next) && a.rta.snapshot.value?.count == 1 }
    }
}
