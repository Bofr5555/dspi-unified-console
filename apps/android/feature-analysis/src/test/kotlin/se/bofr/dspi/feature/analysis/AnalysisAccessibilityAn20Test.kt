package se.bofr.dspi.feature.analysis

import android.app.Application
import androidx.activity.ComponentActivity
import androidx.compose.ui.test.junit4.createAndroidComposeRule
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
import org.robolectric.annotation.GraphicsMode
import se.bofr.dspi.bridge.DspiBridgeImpl
import se.bofr.dspi.fake.FakeDspiCore
import se.bofr.dspi.ui.a11y.ScreenHarness
import se.bofr.dspi.ui.settings.AppSettings
import se.bofr.dspi.ui.settings.InMemoryAppSettingsStore

/** WP AN20: ATF's checks on the analysis pages, over the V32 FakeDspiCore. Native graphics: see AtfChecks. */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35], application = Application::class, qualifiers = "w400dp-h800dp-port-mdpi")
@GraphicsMode(GraphicsMode.Mode.NATIVE)
class AnalysisAccessibilityAn20Test {
    @get:Rule val rule = createAndroidComposeRule<ComponentActivity>()

    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main)
    private val bridge = DspiBridgeImpl(FakeDspiCore.seeded(), scope, ffi = Dispatchers.Main, main = Dispatchers.Main)
    private val store = InMemoryAppSettingsStore(AppSettings())
    private val screen = ScreenHarness(rule, store)

    @After fun stop() = scope.cancel()

    private fun connect() {
        rule.mainClock.autoAdvance = false
        bridge.scanDevices()
        screen.settle { bridge.status.value.connected }
    }

    @Test
    fun an20AtfPassesOnEveryAnalysisPage() {
        connect()
        for (page in AnalysisRoute.entries) {
            screen.show { AnalysisScreen(bridge, {}, {}, open = page.route.removePrefix("analysis/")) }
            screen.assertAtfPasses(page.title, minNodes = 5)
        }
    }
}
