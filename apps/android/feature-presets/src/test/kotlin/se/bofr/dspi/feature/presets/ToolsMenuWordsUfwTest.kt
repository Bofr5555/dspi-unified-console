package se.bofr.dspi.feature.presets

import android.app.Application
import androidx.activity.ComponentActivity
import androidx.compose.ui.test.assertCountEquals
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onAllNodesWithText
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import org.junit.After
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import se.bofr.dspi.bridge.DspiBridgeImpl
import se.bofr.dspi.fake.FakeDspiCore
import se.bofr.dspi.ui.a11y.ScreenHarness
import se.bofr.dspi.ui.settings.AppSettings
import se.bofr.dspi.ui.settings.InMemoryAppSettingsStore

/** WP UFW (UXP A10, D1): the Tools menu says Save to Device, the desktop's and the web's word for it. */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35], application = Application::class, qualifiers = "w400dp-h800dp-port-mdpi")
class ToolsMenuWordsUfwTest {
    @get:Rule val rule = createAndroidComposeRule<ComponentActivity>()

    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main)
    private val bridge = DspiBridgeImpl(FakeDspiCore.seeded(), scope, ffi = Dispatchers.Main, main = Dispatchers.Main)
    private val screen = ScreenHarness(rule, InMemoryAppSettingsStore(AppSettings()))

    @After fun stop() = scope.cancel()

    @Test
    fun ufwTheToolsMenuSaysSaveToDevice() {
        rule.mainClock.autoAdvance = false
        bridge.scanDevices()
        screen.settle { bridge.status.value.connected }
        screen.show { PresetsScreen(bridge, {}, {}) }
        rule.onAllNodesWithText("Save to Device...").assertCountEquals(1)
        rule.onAllNodesWithText("Commit Parameters...").assertCountEquals(0)
    }
}
