package se.bofr.dspi.feature.settings

import android.app.Application
import android.os.Looper
import androidx.activity.ComponentActivity
import androidx.compose.runtime.snapshots.Snapshot
import androidx.compose.ui.test.hasTestTag
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import java.util.Collections
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
import se.bofr.dspi.bridge.DspiBridgeImpl
import se.bofr.dspi.fake.FakeHubDspiCore
import se.bofr.dspi.ui.settings.AppSettings
import se.bofr.dspi.ui.settings.InMemoryAppSettingsStore

/**
 * WP P2AN on screen (Robolectric, the bridge's own FFI thread): Settings ›
 * History shows this console's log, offers "On the hub" through a hub with the
 * `events` cap and shows the hub's records with who did it, a new one live;
 * without the cap there is no picker. Compose state written on main (ANDFU).
 */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35], application = Application::class, qualifiers = "w400dp-h860dp-port-xhdpi")
class EventsPageViewWpP2anTest {
    @get:Rule val rule = createAndroidComposeRule<ComponentActivity>()

    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
    private val store = InMemoryAppSettingsStore(AppSettings())

    @After fun stop() = scope.cancel()

    private fun pumpUntil(what: String, until: () -> Boolean) {
        val end = System.currentTimeMillis() + 20_000
        while (!until()) {
            if (System.currentTimeMillis() > end) throw AssertionError("timed out: $what")
            shadowOf(Looper.getMainLooper()).idle()
            rule.runOnUiThread { Snapshot.sendApplyNotifications() }
            rule.mainClock.advanceTimeBy(50)
            Thread.sleep(5)
        }
    }

    private fun textShown(t: String) =
        rule.onAllNodes(hasText(t, substring = true), useUnmergedTree = true).fetchSemanticsNodes().isNotEmpty()

    private fun open(core: FakeHubDspiCore): DspiBridgeImpl {
        val bridge = DspiBridgeImpl(core, scope)
        rule.mainClock.autoAdvance = false
        bridge.scanDevices()
        pumpUntil("connected") { bridge.status.value.connected }
        pumpUntil("caps") { bridge.hubServer.caps.value.onHub == core.onHub }
        return bridge
    }

    @Test
    fun wpP2anHistoryShowsThisConsolesLogAndTheHubsWithWhoDidIt() {
        val core = FakeHubDspiCore()
        core.localAddEvent("Output 1 gain -4 dB")
        core.hubAddEvent("Output 1 gain -6 dB", by = "Front of house")
        val offMain = Collections.synchronizedList(mutableListOf<String>())
        val observer = Snapshot.registerGlobalWriteObserver {
            if (Looper.myLooper() != Looper.getMainLooper()) offMain += Thread.currentThread().name
        }
        try {
            val bridge = open(core)
            rule.setContent { SettingsView(bridge, store, {}, {}, initialPage = SettingsCategory.EVENTS) }
            pumpUntil("local row") { textShown("Output 1 gain -4 dB") }
            assertTrue(rule.onAllNodes(hasTestTag(EVENTS_SOURCE_TAG)).fetchSemanticsNodes().isNotEmpty())
            rule.onNodeWithText("On the hub").performClick()
            pumpUntil("hub row") { textShown("Output 1 gain -6 dB") && textShown("Front of house") }
            pumpUntil("watching") { core.watching != null }
            core.hubAddEvent("Preset 2 recalled", by = "Schedule: evening")
            pumpUntil("live row") { textShown("Schedule: evening") }
        } finally {
            observer.dispose()
        }
        assertEquals("Compose state written off the main thread", emptyList<String>(), offMain.toList())
    }

    @Test
    fun wpP2anWithoutTheCapThereIsNoHubPicker() {
        val core = FakeHubDspiCore(hubCaps = emptySet())
        core.localAddEvent("Output 1 gain -4 dB")
        val bridge = open(core)
        rule.setContent { SettingsView(bridge, store, {}, {}, initialPage = SettingsCategory.EVENTS) }
        pumpUntil("local row") { textShown("Output 1 gain -4 dB") }
        assertTrue(rule.onAllNodes(hasTestTag(EVENTS_SOURCE_TAG)).fetchSemanticsNodes().isEmpty())
        assertTrue(!textShown("On the hub"))
    }
}
