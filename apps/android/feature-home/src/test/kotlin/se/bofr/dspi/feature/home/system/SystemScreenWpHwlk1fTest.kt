package se.bofr.dspi.feature.home.system

import android.app.Application
import android.os.Looper
import androidx.activity.ComponentActivity
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.material3.Surface
import androidx.compose.runtime.snapshots.Snapshot
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.launch
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode
import se.bofr.dspi.bridge.DspiBridgeImpl
import se.bofr.dspi.core.DspiConstants
import se.bofr.dspi.fake.FakeDspiCore
import se.bofr.dspi.ui.DspiTheme

/**
 * WP HWLK1F: a unit whose firmware has no audio link (hw-lk1's K340, the demo hub's
 * `--with-plain-unit`), as the core gives it: the System screen says "No audio link
 * (firmware)" for it, never "Unknown" or a Receiver, the summary leaves it out of the
 * locked count, a recall says it was applied at once there, and the alignment says it
 * was skipped and why.
 */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35], application = Application::class, qualifiers = "w400dp-h2000dp-port-mdpi")
@GraphicsMode(GraphicsMode.Mode.NATIVE)
class SystemScreenWpHwlk1fTest {
    @get:Rule val rule = createAndroidComposeRule<ComponentActivity>()

    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main)
    private val core = FakeDspiCore()
    private val bridge = DspiBridgeImpl(core, scope, ffi = Dispatchers.Main, main = Dispatchers.Main)
    private val model = SystemScreenModel(bridge, scope)

    @After fun stop() = scope.cancel()

    private fun settle(rounds: Int = 20) {
        repeat(rounds) {
            shadowOf(Looper.getMainLooper()).idle()
            rule.runOnUiThread { Snapshot.sendApplyNotifications() }
            rule.mainClock.advanceTimeBy(50)
        }
    }

    @Test
    fun wpHwlk1fAUnitWithoutTheAudioLinkIsSaidSo() {
        rule.mainClock.autoAdvance = false
        scope.launch { bridge.attachDevice(42, core.serial, DspiConstants.VENDOR_ID) }
        settle()
        core.audioLinkSystem.onHub = true
        core.audioLinkSystem.plainUnit = true
        scope.launch { model.refresh() }
        settle()
        rule.setContent { DspiTheme(darkTheme = true) { Surface(Modifier.fillMaxSize()) { SystemView(model) {} } } }
        settle()
        rule.onNodeWithContentDescription(
            "Status: Audio link: 4 of 4 locked, 1 without the audio link (firmware), link delay to write on 4"
        ).assertIsDisplayed()
        rule.onNodeWithContentDescription("Plain unit, No audio link, No audio link (firmware)").assertIsDisplayed()
        val p = model.state.value.view!!.units.first { it.id == "P" }
        assertEquals("no_alink", p.state)
        assertFalse(p.pending)

        rule.onNodeWithContentDescription("Recall Evening").performClick()
        settle()
        rule.onNodeWithText(
            "Evening recalled: applied now on Plain unit (no audio link); at link frame",
            substring = true
        ).assertIsDisplayed()
        val recall = model.state.value.lastRecall!!
        assertTrue(recall.taken)
        assertFalse("never on time on every unit with P", recall.message.contains("on time on every unit"))

        rule.onNodeWithContentDescription("Clock and links, folded").performClick()
        settle()
        rule.onNodeWithText("Align link delays…").performClick()
        settle()
        rule.onNodeWithText("Align").performClick()
        settle()
        assertTrue(
            model.state.value.actionMessage,
            model.state.value.actionMessage.endsWith("skipped on Plain unit (no audio link (firmware)).")
        )
    }
}
