// Fixup lk1 (L1), review lk1's probe kept: Home's audio-link chip polls the core only
// while the activity is started (decision E6, ui-common WhileVisible.kt).
package se.bofr.dspi.feature.home.system

import android.app.Application
import android.os.Looper
import androidx.activity.ComponentActivity
import androidx.compose.runtime.snapshots.Snapshot
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.lifecycle.Lifecycle
import java.time.Duration
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.launch
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
import se.bofr.dspi.core.DspiConstants
import se.bofr.dspi.fake.FakeDspiCore
import se.bofr.dspi.ui.DspiTheme

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35], application = Application::class, qualifiers = "w400dp-h2000dp-port-mdpi")
class SystemChipVisibilityWpLkui1Test {
    @get:Rule val rule = createAndroidComposeRule<ComponentActivity>()

    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main)
    private val core = FakeDspiCore()
    private val bridge = DspiBridgeImpl(core, scope, ffi = Dispatchers.Main, main = Dispatchers.Main)

    @After fun stop() = scope.cancel()

    private fun settle(rounds: Int = 20) {
        repeat(rounds) {
            shadowOf(Looper.getMainLooper()).idle()
            rule.runOnUiThread { Snapshot.sendApplyNotifications() }
            rule.mainClock.advanceTimeBy(50)
        }
    }

    private fun seconds(n: Int) {
        repeat(n * 2) {
            shadowOf(Looper.getMainLooper()).idleFor(Duration.ofMillis(500))
            rule.mainClock.advanceTimeBy(500)
        }
    }

    private fun views() = core.calls.count { it.startsWith("linkSystemView") }

    @Test
    fun wpLkui1TheChipStopsPollingWhileTheActivityIsStopped() {
        rule.mainClock.autoAdvance = false
        scope.launch { bridge.attachDevice(42, core.serial, DspiConstants.VENDOR_ID) }
        settle()
        core.audioLinkSystem.onHub = true
        rule.setContent { DspiTheme(darkTheme = true) { SystemStatusChip(bridge, onNavigate = {}) } }
        settle()
        seconds(10)
        val shown = views()
        assertTrue("the chip polls while shown ($shown)", shown >= 3)
        rule.activityRule.scenario.moveToState(Lifecycle.State.CREATED)
        settle()
        val atStop = views()
        seconds(30)
        val afterBackground = views()
        assertEquals("the chip must not poll the core while the activity is stopped", atStop, afterBackground)
        // Shown again: it polls again
        rule.activityRule.scenario.moveToState(Lifecycle.State.RESUMED)
        settle()
        seconds(4)
        assertTrue("the chip polls again once shown", views() > afterBackground)
    }
}
