package se.bofr.dspi.app

import android.app.Application
import android.os.Looper
import androidx.activity.ComponentActivity
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.snapshots.Snapshot
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.test.SemanticsMatcher
import androidx.compose.ui.test.hasAnyAncestor
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.isEnabled
import androidx.compose.ui.test.isPopup
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.performClick
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.flow.launchIn
import kotlinx.coroutines.flow.onEach
import kotlinx.coroutines.launch
import org.junit.After
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config
import se.bofr.dspi.bridge.BridgeEvent
import se.bofr.dspi.bridge.DspiBridge
import se.bofr.dspi.feature.home.HomeScreen
import se.bofr.dspi.ui.DspiTheme
import se.bofr.dspi.ui.LocalUiMode
import se.bofr.dspi.ui.UiModes
import se.bofr.dspi.ui.settings.AppSettings
import se.bofr.dspi.ui.settings.InMemoryAppSettingsStore
import se.bofr.dspi.ui.settings.LocalAppSettings

/**
 * -- FIX016 -- (LOCKSRC, owner D4) the locked user may change the input source from Home, the remote: the
 * phone's lock Home offers it as a control (the picker, not words), and a choice goes to the core. Until the core
 * agent's LOCKSRC rule is in this tree the core's lock refuses it (its sentence); after it the source changes.
 * Either way the core was asked, never the page refusing on its own. Over the real core's demo device.
 */
@RunWith(DspiRobolectricTestRunner::class)
@Config(sdk = [35], application = Application::class, qualifiers = "w400dp-h2000dp-port-mdpi")
class LockedSourceFix016Test {
    @get:Rule val rule = createAndroidComposeRule<ComponentActivity>()

    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
    private var session: DemoSession? = null
    private val failures = mutableListOf<String>()

    @After
    fun stop() {
        rule.runOnUiThread { session?.close() }
        scope.cancel()
    }

    private fun pumpUntil(what: String, until: () -> Boolean) {
        val end = System.currentTimeMillis() + 20_000
        while (!until()) {
            if (System.currentTimeMillis() > end) throw AssertionError("timed out: $what")
            shadowOf(Looper.getMainLooper()).idle()
            rule.runOnUiThread { Snapshot.sendApplyNotifications() }
            rule.mainClock.advanceTimeBy(50)
            Thread.sleep(10)
        }
    }

    private val picker = SemanticsMatcher.expectValue(SemanticsProperties.Role, Role.DropdownList)

    @Test
    fun fix016TheLockedHomeOffersTheSourceAndAsksTheCore() {
        val bridge: DspiBridge = startDemoSession(scope, "fix016-source").also { session = it }.bridge
        bridge.events.onEach { if (it is BridgeEvent.CommandFailed) failures += "${it.command}: ${it.error}" }.launchIn(scope)
        rule.mainClock.autoAdvance = false
        val settings = InMemoryAppSettingsStore(AppSettings())
        rule.setContent {
            CompositionLocalProvider(LocalAppSettings provides settings, LocalUiMode provides UiModes.LOCKED_USER) {
                DspiTheme(darkTheme = true) { HomeScreen(bridge, {}, {}) }
            }
        }
        pumpUntil("writes") { bridge.writesAllowed.value }
        var done = false
        rule.runOnUiThread {
            scope.launch {
                bridge.installer.setPin("2468").getOrThrow()
                bridge.installer.cover(listOf("front")).getOrThrow()
                bridge.installer.lockNow().getOrThrow()
                done = true
            }
        }
        pumpUntil("locked") { done && bridge.installer.lock.value?.locked == true }
        val sources = bridge.inputSources
        assertTrue("the demo has a source to choose: $sources", sources.size > 1)
        val now = sources.first { it.id == bridge.inputSource }
        val other = sources.first { it.id != now.id }
        // The control: the picker, enabled, under the lock
        pumpUntil("the picker") {
            rule.onAllNodes(picker and hasText(now.name) and isEnabled(), useUnmergedTree = false).fetchSemanticsNodes().isNotEmpty()
        }
        rule.onNode(picker and hasText(now.name)).performClick()
        val item = hasText(other.name) and hasAnyAncestor(isPopup())
        pumpUntil("the list") { rule.onAllNodes(item).fetchSemanticsNodes().isNotEmpty() }
        rule.onNode(item).performClick()
        // The core was asked: the source changed, or the core's lock refused it in its own words
        pumpUntil("the core's answer") {
            bridge.inputSource == other.id || failures.any { "locked" in it.lowercase() }
        }
    }
}
