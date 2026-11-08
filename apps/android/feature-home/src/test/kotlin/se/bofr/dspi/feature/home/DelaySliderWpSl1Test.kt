package se.bofr.dspi.feature.home

import android.app.Application
import android.os.Looper
import androidx.activity.ComponentActivity
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.material3.Surface
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.snapshots.Snapshot
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.SemanticsActions
import androidx.compose.ui.test.SemanticsMatcher
import androidx.compose.ui.test.assert
import androidx.compose.ui.test.hasContentDescription
import androidx.compose.ui.test.hasStateDescription
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.performSemanticsAction
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config
import se.bofr.dspi.bridge.DspiBridgeImpl
import se.bofr.dspi.fake.FakeDspiCore
import se.bofr.dspi.feature.home.components.delayStateText
import se.bofr.dspi.ui.DspiTheme
import se.bofr.dspi.ui.settings.AppSettings
import se.bofr.dspi.ui.settings.InMemoryAppSettingsStore
import se.bofr.dspi.ui.settings.LocalAppSettings

/**
 * WP SL1: the output page's DELAY slider (review c6q AN40-1) commits whole ms
 * as macOS does, while a fractional delay from elsewhere stays shown until the
 * user drags; TalkBack reads it at the field's precision (AN40-2).
 */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35], application = Application::class, qualifiers = "w400dp-h800dp-port-mdpi")
class DelaySliderWpSl1Test {
    @get:Rule val rule = createAndroidComposeRule<ComponentActivity>()

    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main)
    private val bridge = DspiBridgeImpl(FakeDspiCore.seeded(), scope, ffi = Dispatchers.Main, main = Dispatchers.Main)

    @After fun stop() = scope.cancel()

    private fun settle(until: () -> Boolean = { true }) {
        val end = System.currentTimeMillis() + 10_000
        var rounds = 0
        while (rounds < 20 || !until()) {
            check(System.currentTimeMillis() < end) { "timed out" }
            shadowOf(Looper.getMainLooper()).idle()
            rule.runOnUiThread { Snapshot.sendApplyNotifications() }
            rule.mainClock.advanceTimeBy(50)
            rounds++
        }
    }

    private fun showOutputPage() {
        rule.mainClock.autoAdvance = false
        bridge.scanDevices()
        settle { bridge.status.value.connected }
        rule.setContent {
            CompositionLocalProvider(LocalAppSettings provides InMemoryAppSettingsStore(AppSettings())) {
                DspiTheme(darkTheme = true) { Surface(Modifier.fillMaxSize()) { ChannelScreen(bridge, 2, {}, {}) } }
            }
        }
        settle()
    }

    private fun delaySlider() = rule.onNode(hasContentDescription("Delay") and SemanticsMatcher.keyIsDefined(SemanticsActions.SetProgress))

    @Test
    fun wpSl1StateTextHasTheFieldsPrecision() {
        assertEquals("0.25 ms", delayStateText(0.25f))
        assertEquals("0.5 ms", delayStateText(0.5f))
        assertEquals("12.0 ms", delayStateText(12f))
        assertEquals("12.35 ms", delayStateText(12.3456f))
    }

    @Test
    fun wpSl1AFractionalDelayStaysShownThenASlideCommitsWholeMs() {
        showOutputPage()
        // a typed (or file / other client) fraction is kept and shown as it is
        bridge.setOutputDelay(0, 0.25f)
        settle { bridge.outputDelayMS(0) == 0.25f }
        settle()
        delaySlider().assert(hasStateDescription("0.25 ms"))
        // TalkBack's slide is a drag (grab, value, release): whole ms
        delaySlider().performSemanticsAction(SemanticsActions.SetProgress) { it(3.4f) }
        settle { bridge.outputDelayMS(0) == 3f }
        settle()
        assertEquals(3f, bridge.outputDelayMS(0), 0f)
        delaySlider().assert(hasStateDescription("3.0 ms"))
    }
}
