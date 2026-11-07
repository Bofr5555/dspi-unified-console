package se.bofr.dspi.app

import android.app.Application
import android.os.Looper
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.darkColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.snapshots.Snapshot
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onAllNodesWithTag
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performTouchInput
import androidx.compose.ui.test.swipeDown
import androidx.compose.ui.unit.dp
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicInteger
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config
import se.bofr.dspi.bridge.DspiBridge
import se.bofr.dspi.core.DspiConstants
import se.bofr.dspi.core.DspiCoreApi
import se.bofr.dspi.core.RtaOwnership
import se.bofr.dspi.feature.analysis.RTA_BAR_STRIP_HANDLE_TAG
import se.bofr.dspi.feature.analysis.RTA_BAR_STRIP_OPTIONS_TAG
import se.bofr.dspi.feature.analysis.RTA_BAR_STRIP_TAG
import se.bofr.dspi.feature.analysis.RTA_FOLLOWING_TEXT
import se.bofr.dspi.feature.analysis.RTA_TAKE_CONTROL_TEXT
import se.bofr.dspi.feature.analysis.RtaChannelSelection
import se.bofr.dspi.feature.analysis.SpectrumAnalyserView
import se.bofr.dspi.feature.analysis.SpectrumBarStrip
import se.bofr.dspi.feature.analysis.rtaBarStripCellTag
import se.bofr.dspi.ffi.FfiDspiCore
import se.bofr.dspi.ui.settings.AppSettings
import se.bofr.dspi.ui.settings.InMemoryAppSettingsStore

/**
 * WP AN17: the analyser bar strip and the following state on the real core's
 * demo device (host build of libdspi_ffi over JNA), through the real bridge.
 */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35], application = Application::class, qualifiers = "w400dp-h1200dp-port-mdpi")
class SpectrumBarStripAn17Test {
    @get:Rule val rule = createComposeRule()

    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
    private val ffiCore = FfiDspiCore()
    private var session: DemoSession? = null
    private lateinit var bridge: DspiBridge

    /** Another console on the hub, as the core reports it (`rta_get_ownership`); the demo has no hub. */
    private val following = AtomicBoolean(false)
    private val takeControls = AtomicInteger(0)

    private inner class FollowableDemo(core: DspiCoreApi) : DspiCoreApi by core {
        override fun rtaGetOwnership(): RtaOwnership = if (following.get()) {
            RtaOwnership(following = true, origin = 7, tap = DspiConstants.RTA_TAP_INPUT, mask = 0b11, fftOrder = 10)
        } else {
            RtaOwnership(following = false, origin = 0, tap = 0, mask = 0, fftOrder = 0)
        }

        override fun rtaTakeControl() {
            takeControls.incrementAndGet()
            following.set(false)
        }
    }

    @After
    fun stop() {
        val s = session
        if (s != null) rule.runOnUiThread { s.close() } else ffiCore.close()
        scope.cancel()
    }

    private fun start() {
        val s = startDemoSession(scope, "an17", core = FollowableDemo(DemoCore(ffiCore)))
        session = s
        bridge = s.bridge
        rule.mainClock.autoAdvance = false
        settleUntil("connected") { bridge.status.value.connected }
        settleUntil("analyser caps") { bridge.rta.caps.value?.supported == true }
    }

    private fun show(content: @Composable () -> Unit) {
        rule.setContent { MaterialTheme(colorScheme = darkColorScheme()) { Surface(Modifier.fillMaxSize()) { content() } } }
        settle()
    }

    private fun settle() = repeat(20) {
        shadowOf(Looper.getMainLooper()).idle()
        rule.mainClock.advanceTimeBy(100)
        Thread.sleep(5)
    }

    /** The FFI thread and the analyser deliver in real time; the main looper is pumped here. */
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

    private fun shown(text: String) =
        rule.onAllNodesWithText(text, substring = true, useUnmergedTree = true).fetchSemanticsNodes().isNotEmpty()

    @Test
    fun an17TheStripDrawsTheDashboardChannelsWhenBarsAreOn() {
        start()
        val store = InMemoryAppSettingsStore(AppSettings(rtaDashboardShowBars = true, rtaDashboardSelectionKey = "out:0,1"))
        show { SpectrumBarStrip(bridge, store = store) }
        settleUntil("the strip") { tagged(RTA_BAR_STRIP_TAG) }
        assertTrue(tagged(rtaBarStripCellTag(DspiConstants.RTA_TAP_OUTPUT, 0)))
        assertTrue(tagged(rtaBarStripCellTag(DspiConstants.RTA_TAP_OUTPUT, 1)))
        // It subscribes with its channels, and the demo's analyser runs for it
        settleUntil("frames") { bridge.rta.snapshot.value?.running == true }
        val snap = bridge.rta.snapshot.value!!
        assertEquals(DspiConstants.RTA_TAP_OUTPUT, snap.tap)

        // Layout from the options: one column
        rule.onNodeWithTag(RTA_BAR_STRIP_OPTIONS_TAG, useUnmergedTree = true).performClick()
        settle()
        rule.onNodeWithText("1 column").performClick()
        settle()
        assertEquals(1, store.settings.value.rtaBarColumns)
    }

    @Test
    fun an17TheStripIsHiddenWhenBarsAreOffOrNothingIsChosen() {
        start()
        val off = InMemoryAppSettingsStore(AppSettings(rtaDashboardShowBars = false, rtaDashboardSelectionKey = "out:0"))
        val none = InMemoryAppSettingsStore(AppSettings(rtaDashboardShowBars = true, rtaDashboardSelectionKey = "out:"))
        show {
            Column {
                SpectrumBarStrip(bridge, store = off)
                SpectrumBarStrip(bridge, store = none)
            }
        }
        settle()
        assertTrue(!tagged(RTA_BAR_STRIP_TAG))
    }

    @Test
    fun an17AChannelPageDrawsItsOwnChannelAndTheDragResizes() {
        start()
        val store = InMemoryAppSettingsStore(AppSettings(rtaChannelShowBars = true, rtaBarHeight = 96.0))
        val own = RtaChannelSelection.of(DspiConstants.RTA_TAP_OUTPUT, listOf(2))
        show { SpectrumBarStrip(bridge, Modifier.fillMaxWidth(), own = own, store = store) }
        settleUntil("the strip") { tagged(RTA_BAR_STRIP_TAG) }
        assertTrue(tagged(rtaBarStripCellTag(DspiConstants.RTA_TAP_OUTPUT, 2)))
        assertTrue(!tagged(rtaBarStripCellTag(DspiConstants.RTA_TAP_OUTPUT, 0)))
        // One channel and no analyser link: no options to offer
        assertTrue(!tagged(RTA_BAR_STRIP_OPTIONS_TAG))

        rule.onNodeWithTag(RTA_BAR_STRIP_HANDLE_TAG, useUnmergedTree = true).performTouchInput {
            swipeDown(
                startY = centerY,
                endY =
                centerY + 60f
            )
        }
        settle()
        val h = store.settings.value.rtaBarHeight
        assertTrue("dragged taller: $h", h > 96.0 && h <= 240.0)
    }

    @Test
    fun an17FollowingShowsTheDevicesChannelsUntilTakeControl() {
        start()
        val store =
            InMemoryAppSettingsStore(
                AppSettings(rtaDashboardShowBars = true, rtaDashboardShowGraph = false, rtaDashboardSelectionKey = "out:0")
            )
        show {
            Column(Modifier.fillMaxSize()) {
                SpectrumBarStrip(bridge, store = store)
                SpectrumAnalyserView(bridge, Modifier.height(700.dp), store = store)
            }
        }
        settleUntil("driving") {
            bridge.rta.ownership.value?.following == false &&
                tagged(rtaBarStripCellTag(DspiConstants.RTA_TAP_OUTPUT, 0))
        }
        assertTrue(!shown(RTA_FOLLOWING_TEXT))

        // Another console pushes inputs 1 and 2: both views draw those, and say so
        following.set(true)
        settleUntil("following") {
            bridge.rta.ownership.value?.following == true &&
                shown(RTA_FOLLOWING_TEXT) &&
                tagged(rtaBarStripCellTag(DspiConstants.RTA_TAP_INPUT, 0)) &&
                tagged(rtaBarStripCellTag(DspiConstants.RTA_TAP_INPUT, 1))
        }
        assertTrue(!tagged(rtaBarStripCellTag(DspiConstants.RTA_TAP_OUTPUT, 0)))
        // Following draws only: our own stored choice is untouched and nothing took control
        assertEquals("out:0", store.settings.value.rtaDashboardSelectionKey)
        assertEquals(0, takeControls.get())

        rule.onNodeWithText(RTA_TAKE_CONTROL_TEXT, useUnmergedTree = true).performClick()
        settleUntil("driving again") {
            bridge.rta.ownership.value?.following == false &&
                !shown(RTA_FOLLOWING_TEXT) &&
                tagged(rtaBarStripCellTag(DspiConstants.RTA_TAP_OUTPUT, 0))
        }
        assertEquals(1, takeControls.get())
    }

    @Test
    fun an17AChoiceMadeWhileFollowingTakesControl() {
        start()
        val store = InMemoryAppSettingsStore(AppSettings(rtaDashboardSelectionKey = "out:0"))
        following.set(true)
        show { SpectrumAnalyserView(bridge, Modifier.fillMaxSize(), store = store) }
        settleUntil("following") { shown(RTA_FOLLOWING_TEXT) }
        rule.onAllNodesWithText("Outputs")[0].performClick()
        settleUntil("taken back") { takeControls.get() == 1 && !shown(RTA_FOLLOWING_TEXT) }
    }
}
