package se.bofr.dspi.app

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
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config
import se.bofr.dspi.bridge.DspiBridgeImpl
import se.bofr.dspi.bridge.InMemoryLinkStore
import se.bofr.dspi.core.DspiConstants
import se.bofr.dspi.core.LinkDevice
import se.bofr.dspi.feature.graph.GraphView
import se.bofr.dspi.feature.graph.RtaChannelSelection
import se.bofr.dspi.feature.graph.graphSpectrumTag
import se.bofr.dspi.ffi.FfiDspiCore
import se.bofr.dspi.ui.settings.AppSettings
import se.bofr.dspi.ui.settings.InMemoryAppSettingsStore
import se.bofr.dspi.ui.settings.LocalAppSettings

/**
 * WP AN34: the response graph's spectrum overlay follows another console
 * without taking control. Two real cores on crates/dspi-link's fake hub
 * (`--rta`) as a process: A shows the graph on the main looper, B is headless
 * on its own threads. Real sockets, no USB.
 */
@RunWith(DspiRobolectricTestRunner::class)
@Config(sdk = [35], application = Application::class, qualifiers = "w400dp-h1200dp-port-mdpi")
class GraphFollowingFakeHubAn34Test {
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
    private val aExecutor = Executors.newSingleThreadExecutor { r -> Thread(r, "an34-a-ffi").apply { isDaemon = true } }
    private val aFfi = aExecutor.asCoroutineDispatcher()
    private val aCore = FfiDspiCore()
    private val aScope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
    private val a = DspiBridgeImpl(aCore, aScope, ffi = aFfi, main = Dispatchers.Main.immediate, linkStore = InMemoryLinkStore())

    // B: another console, headless on its own threads
    private val bMainExecutor = Executors.newSingleThreadExecutor { r -> Thread(r, "an34-b-main").apply { isDaemon = true } }
    private val bMain = bMainExecutor.asCoroutineDispatcher()
    private val bExecutor = Executors.newSingleThreadExecutor { r -> Thread(r, "an34-b-ffi").apply { isDaemon = true } }
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

    private fun connectA() {
        val device = aExecutor.submit<LinkDevice> {
            aCore.linkConnect(aCore.linkWsUrl("127.0.0.1", port, url), "an34-a", "test", "", "")
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
            bCore.linkConnect(bCore.linkWsUrl("127.0.0.1", port, url), "an34-b", "test", "", "")
            bCore.linkDevices().first()
        }
        assertTrue(withContext(bMain) { b.attachRemote(device.handle, device.serial) })
        withTimeout(10_000) { while (withContext(bMain) { b.rta.caps.value?.supported } != true) delay(20) }
    }

    private fun bFollowing() = runBlocking { withContext(bMain) { b.rta.ownership.value?.following } }

    // -- WP FLAKE2 -- B's event log records a notification when B's core drains it
    private fun bDrainedAsEvent() = runBlocking {
        withContext(bFfi) {
            bCore.eventLogRead("", 0, 500).entries.any { "RtaConfig" in it.text && "tap=out mask=0x0001" in it.text }
        }
    }

    @Test
    fun an34TheGraphDrawsAFollowedSelectionWithoutTakingControl() {
        connectA()
        connectB()
        rule.mainClock.autoAdvance = false
        val store = InMemoryAppSettingsStore(AppSettings(rtaDashboardShowGraph = true, rtaDashboardSelectionKey = "out:0"))
        rule.setContent {
            CompositionLocalProvider(LocalAppSettings provides store) {
                MaterialTheme(colorScheme = darkColorScheme()) { Surface(Modifier.fillMaxSize()) { GraphView(a, Modifier.fillMaxWidth()) } }
            }
        }
        val own = graphSpectrumTag(RtaChannelSelection(DspiConstants.RTA_TAP_OUTPUT, listOf(0)))
        val theirs = graphSpectrumTag(RtaChannelSelection(DspiConstants.RTA_TAP_INPUT, listOf(0, 1)))

        // A's graph drives the analyser with its dashboard selection, and the device holds it
        pump("A drives") {
            a.rta.ownership.value?.following == false &&
                tagged(own) &&
                a.rta.snapshot.value?.let { it.tap == DspiConstants.RTA_TAP_OUTPUT && it.count > 0 } == true
        }
        // -- WP FLAKE2 -- and B has drained the device's event for A's push. A choice is judged
        // against every event drained after it (rta_sharing: an event after our choice makes us
        // follow), so B's choice made while A's event still waited in B's queue made B follow A
        // (under load B's batch came 3 ms after B's choice), and A never followed B.
        val drainEnd = System.currentTimeMillis() + 15_000
        while (!bDrainedAsEvent()) {
            check(System.currentTimeMillis() < drainEnd) { "timed out: B drains A's analyser event" }
            Thread.sleep(20)
        }

        // B pushes inputs 1 and 2: A's graph draws those
        runBlocking { withContext(bMain) { b.rta.update(1, DspiConstants.RTA_TAP_INPUT, 0b11, false) } }
        pump("A follows B") { a.rta.ownership.value?.following == true && tagged(theirs) }
        assertFalse(tagged(own))

        // Drawing only: a while later B still drives, A still follows, A's choice is untouched
        val end = System.currentTimeMillis() + 1_500
        pump("time passes") { System.currentTimeMillis() > end }
        assertEquals(false, bFollowing())
        assertEquals(true, a.rta.ownership.value?.following)
        assertTrue(tagged(theirs))
        assertEquals("out:0", store.settings.value.rtaDashboardSelectionKey)

        // Take control (the analyser page's button): A's own selection is drawn again, and B follows A
        a.rta.takeControl()
        pump("A drives again") { a.rta.ownership.value?.following == false && tagged(own) }
        val bEnd = System.currentTimeMillis() + 10_000
        while (bFollowing() != true) {
            check(System.currentTimeMillis() < bEnd) { "timed out: B follows A" }
            Thread.sleep(20)
        }
    }
}
