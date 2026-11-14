package se.bofr.dspi.app

import android.app.Application
import android.os.Looper
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.darkColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.runtime.snapshots.Snapshot
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onAllNodesWithContentDescription
import androidx.compose.ui.test.onAllNodesWithTag
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.performClick
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import org.junit.After
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config
import se.bofr.dspi.bridge.DspiBridge
import se.bofr.dspi.core.DspiConstants
import se.bofr.dspi.feature.analysis.RTA_BAR_STRIP_TAG
import se.bofr.dspi.feature.analysis.rtaBarStripCellTag
import se.bofr.dspi.feature.home.ChannelScreen
import se.bofr.dspi.feature.home.HomeScreen
import se.bofr.dspi.ffi.FfiDspiCore
import se.bofr.dspi.ui.settings.AppSettings
import se.bofr.dspi.ui.settings.InMemoryAppSettingsStore
import se.bofr.dspi.ui.settings.LocalAppSettings

/**
 * WP AN34: AN17's bar strip placed on Home and the channel page, switched on
 * with the graph gear's "RTA Bars", on the real core's demo device.
 */
@RunWith(DspiRobolectricTestRunner::class)
@Config(sdk = [35], application = Application::class, qualifiers = "w400dp-h1200dp-port-mdpi")
class BarStripPlacementAn34Test {
    @get:Rule val rule = createComposeRule()

    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
    private val ffiCore = FfiDspiCore()
    private var session: DemoSession? = null
    private lateinit var bridge: DspiBridge
    private val store = InMemoryAppSettingsStore(AppSettings())

    @After
    fun stop() {
        val s = session
        if (s != null) rule.runOnUiThread { s.close() } else ffiCore.close()
        scope.cancel()
    }

    private fun start() {
        val s = startDemoSession(scope, "an34", core = DemoCore(ffiCore))
        session = s
        bridge = s.bridge
        rule.mainClock.autoAdvance = false
        settleUntil("connected") { bridge.status.value.connected }
        settleUntil("analyser caps") { bridge.rta.caps.value?.supported == true }
    }

    /** One composition, so screens swap as they do under the NavHost. */
    private fun host(): (@Composable () -> Unit) -> Unit {
        var current by mutableStateOf<(@Composable () -> Unit)?>(null)
        rule.setContent {
            CompositionLocalProvider(LocalAppSettings provides store) {
                MaterialTheme(colorScheme = darkColorScheme()) { Surface(Modifier.fillMaxSize()) { current?.invoke() } }
            }
        }
        return { screen ->
            rule.runOnUiThread { current = null }
            settle()
            rule.runOnUiThread { current = screen }
            settle()
        }
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

    /** The graph's gear, then its "RTA Bars" switch (the gear panel is a Dialog: its own root). */
    private fun toggleRtaBars() {
        rule.onAllNodesWithContentDescription(
            se.bofr.dspi.feature.graph.GRAPH_OPTIONS_DESCRIPTION,
            useUnmergedTree = true
        )[0].performClick()
        settleUntil("the gear panel") {
            rule.onAllNodesWithText("RTA Bars", useUnmergedTree = true).fetchSemanticsNodes().isNotEmpty()
        }
        rule.onAllNodesWithText("RTA Bars", useUnmergedTree = true)[0].performClick()
        settle()
    }

    @Test
    fun an34TheStripShowsOnHomeAndTheChannelPageAfterTheToggle() {
        start()
        val show = host()

        // Home: off by default, on after the dashboard's toggle
        show { HomeScreen(bridge, {}, {}) }
        assertFalse("no strip before the toggle", tagged(RTA_BAR_STRIP_TAG))
        toggleRtaBars()
        assertTrue(store.settings.value.rtaDashboardShowBars)
        assertFalse("the dashboard's toggle only", store.settings.value.rtaChannelShowBars)
        settleUntil("the dashboard strip") { tagged(RTA_BAR_STRIP_TAG) }

        // Channel page (Out 1, app channel 2): its own toggle, its own channel
        show { ChannelScreen(bridge, 2, {}, {}) }
        assertFalse("the channel pages have their own setting", tagged(RTA_BAR_STRIP_TAG))
        toggleRtaBars()
        assertTrue(store.settings.value.rtaChannelShowBars)
        val out = bridge.rta.tapIndex(2)
        settleUntil("the channel page strip") {
            tagged(RTA_BAR_STRIP_TAG) && tagged(rtaBarStripCellTag(DspiConstants.RTA_TAP_OUTPUT, out))
        }

        // An input page draws its input
        show { ChannelScreen(bridge, 0, {}, {}) }
        settleUntil("the input page strip") { tagged(rtaBarStripCellTag(DspiConstants.RTA_TAP_INPUT, bridge.rta.tapIndex(0))) }
        assertFalse(tagged(rtaBarStripCellTag(DspiConstants.RTA_TAP_OUTPUT, out)))
    }
}
