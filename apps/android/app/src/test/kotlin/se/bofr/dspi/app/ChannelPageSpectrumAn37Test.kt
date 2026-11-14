package se.bofr.dspi.app

import android.app.Application
import android.os.Looper
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.darkColorScheme
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.snapshots.Snapshot
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.hasAnyAncestor
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.isDialog
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onAllNodesWithContentDescription
import androidx.compose.ui.test.onAllNodesWithTag
import androidx.compose.ui.test.performClick
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config
import se.bofr.dspi.bridge.DspiBridge
import se.bofr.dspi.core.DspiConstants
import se.bofr.dspi.feature.analysis.rtaBarStripCellTag
import se.bofr.dspi.feature.graph.RtaChannelSelection
import se.bofr.dspi.feature.graph.graphSpectrumTag
import se.bofr.dspi.feature.home.ChannelScreen
import se.bofr.dspi.ffi.FfiDspiCore
import se.bofr.dspi.ui.settings.AppSettings
import se.bofr.dspi.ui.settings.InMemoryAppSettingsStore
import se.bofr.dspi.ui.settings.LocalAppSettings
import se.bofr.dspi.ui.settings.RtaSpectrumView

/**
 * WP AN37 (review c4j N1): a channel page's bar strip and the graph gear
 * share one page selection, as both read `vm.rtaSelection` on macOS, so after
 * a gear change the strip shows the gear's channels and the analyser keeps
 * sending frames for them. On the real core's demo device.
 */
@RunWith(DspiRobolectricTestRunner::class)
@Config(sdk = [35], application = Application::class, qualifiers = "w400dp-h1200dp-port-mdpi")
class ChannelPageSpectrumAn37Test {
    @get:Rule val rule = createComposeRule()

    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
    private val ffiCore = FfiDspiCore()
    private var session: DemoSession? = null
    private lateinit var bridge: DspiBridge

    // Channel pages with the FFT graph and the RTA bars on
    private val store =
        InMemoryAppSettingsStore(
            AppSettings().withRtaShows(RtaSpectrumView.GRAPH, false, true).withRtaShows(RtaSpectrumView.BARS, false, true)
        )

    @After
    fun stop() {
        val s = session
        if (s != null) rule.runOnUiThread { s.close() } else ffiCore.close()
        scope.cancel()
    }

    private fun start() {
        val s = startDemoSession(scope, "an37", core = DemoCore(ffiCore))
        session = s
        bridge = s.bridge
        rule.mainClock.autoAdvance = false
        settleUntil("connected") { bridge.status.value.connected }
        settleUntil("analyser caps") { bridge.rta.caps.value?.supported == true }
    }

    private fun settle() = repeat(20) {
        shadowOf(Looper.getMainLooper()).idle()
        rule.mainClock.advanceTimeBy(100)
        Thread.sleep(5)
    }

    private fun settleUntil(what: String, until: () -> Boolean) {
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

    private fun stripCell(tap: Int, ch: Int) = tagged(rtaBarStripCellTag(tap, ch))

    private fun overlay(tap: Int, vararg ch: Int) = tagged(graphSpectrumTag(RtaChannelSelection(tap, ch.toList())))

    /** A control inside the gear panel (a Dialog, its own root; the strip's labels carry the same names). */
    private fun inGear(text: String) =
        rule.onAllNodes(hasText(text) and hasAnyAncestor(isDialog()), useUnmergedTree = true).fetchSemanticsNodes().isNotEmpty()

    private fun clickInGear(text: String) {
        rule.onAllNodes(hasText(text) and hasAnyAncestor(isDialog()), useUnmergedTree = true)[0].performClick()
        settle()
    }

    /** Frames keep arriving for [tap]/[ch]: the snapshot analyses it and its version moves on, more than once. */
    private fun assertBarsMove(tap: Int, ch: Int) {
        val seen = mutableSetOf<Long>()
        settleUntil("frames for tap $tap ch $ch") {
            val snap = bridge.rta.snapshot.value
            if (snap != null && snap.running && snap.tap == tap && snap.frames.any { it.channel == ch }) seen += snap.version
            seen.size >= 3
        }
    }

    @Test
    fun an37TheStripFollowsTheGearAndItsBarsKeepMoving() {
        start()
        rule.setContent {
            CompositionLocalProvider(LocalAppSettings provides store) {
                MaterialTheme(colorScheme = darkColorScheme()) { Surface(Modifier.fillMaxSize()) { ChannelScreen(bridge, 0, {}, {}) } }
            }
        }
        val input = DspiConstants.RTA_TAP_INPUT
        val usb1 = bridge.rta.tapIndex(0)
        val usb2 = bridge.rta.tapIndex(1)
        settleUntil("the page starts on its own channel") { stripCell(input, usb1) && overlay(input, usb1) }

        // The review's scenario: in the gear, check USB 2 and uncheck USB 1
        rule.onAllNodesWithContentDescription(
            se.bofr.dspi.feature.graph.GRAPH_OPTIONS_DESCRIPTION,
            useUnmergedTree = true
        )[0].performClick()
        settleUntil("the gear panel") { inGear("SPECTRUM") }
        clickInGear(bridge.channelName(1))
        clickInGear(bridge.channelName(0))
        settleUntil("the strip follows the gear") { stripCell(input, usb2) && !stripCell(input, usb1) }
        assertTrue("the overlay draws the same channel", overlay(input, usb2))
        assertTrue("the gear shows one channel", inGear("1 channel"))
        assertBarsMove(input, usb2)

        // Switching side in the gear: the strip moves with it, and gets frames there
        clickInGear("Outputs")
        settleUntil("the other side starts empty") { !stripCell(input, usb2) }
        val outputs = bridge.rta.availableChannels(DspiConstants.RTA_TAP_OUTPUT)
        val out = outputs.first()
        // BUG1: the gear lists outputs by their labels
        clickInGear(bridge.channelLabel(bridge.rta.appChannel(DspiConstants.RTA_TAP_OUTPUT, out)))
        val output = DspiConstants.RTA_TAP_OUTPUT
        settleUntil("the strip on the output side") { stripCell(output, out) }
        assertTrue(overlay(output, out))
        assertFalse(stripCell(input, usb2))
        assertBarsMove(output, out)

        // Back to the inputs brings USB 2 back, in both
        clickInGear("Inputs")
        settleUntil("USB 2 again") { stripCell(input, usb2) && overlay(input, usb2) }
        assertFalse(stripCell(output, out))
        assertEquals(
            "the page's choice is not stored as the dashboard's",
            AppSettings().rtaDashboardSelectionKey,
            store.settings.value.rtaDashboardSelectionKey
        )
        assertBarsMove(input, usb2)
    }
}
