package se.bofr.dspi.feature.presets.snapshots

import android.app.Application
import android.os.Looper
import androidx.activity.ComponentActivity
import androidx.compose.runtime.snapshots.Snapshot
import androidx.compose.ui.test.hasScrollAction
import androidx.compose.ui.test.hasTestTag
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollToNode
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

/**
 * WP P2AN on screen (Robolectric, the bridge's own FFI thread): through a hub
 * with `hub_snapshots` the Snapshots screen has "On the hub" with the hub's
 * rows and its Recall; a hub without the cap shows no such section; all
 * Compose state written on the main thread (ANDFU's rule).
 */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35], application = Application::class, qualifiers = "w400dp-h860dp-port-xhdpi")
class SnapshotsHubViewWpP2anTest {
    @get:Rule val rule = createAndroidComposeRule<ComponentActivity>()

    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)

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

    private fun shown(tag: String) = rule.onAllNodes(hasTestTag(tag)).fetchSemanticsNodes().isNotEmpty()

    private fun open(core: FakeHubDspiCore): SnapshotsModel {
        val bridge = DspiBridgeImpl(core, scope)
        rule.mainClock.autoAdvance = false
        bridge.scanDevices()
        pumpUntil("connected") { bridge.status.value.connected }
        pumpUntil("caps") { bridge.hubServer.caps.value.onHub == core.onHub }
        return SnapshotsModel(bridge, scope, folder = null)
    }

    @Test
    fun wpP2anTheHubSectionShowsTheHubsRowsAndRecallsOne() {
        val core = FakeHubDspiCore()
        core.snapshotCapture("Warm", "")
        val venue = core.hubPutRemotely("Venue default")
        val offMain = Collections.synchronizedList(mutableListOf<String>())
        val observer = Snapshot.registerGlobalWriteObserver {
            if (Looper.myLooper() != Looper.getMainLooper()) offMain += Thread.currentThread().name
        }
        try {
            val model = open(core)
            rule.setContent { SnapshotsView(model, SnapshotsMode.ADVANCED, onBack = {}) }
            pumpUntil("hub rows") { model.hubItems.value.isNotEmpty() }
            rule.onNodeWithTag(SNAPSHOTS_SCREEN_TAG).assertExists()
            pumpUntil("row") { scrollTo(hubSnapshotRowTag(venue.id)) && shown(hubSnapshotRowTag(venue.id)) }
            assertTrue(shown(HUB_SECTION_TAG) || scrollTo(HUB_SECTION_TAG))
            assertTrue(rule.onAllNodes(hasText("Venue default")).fetchSemanticsNodes().isNotEmpty())
            rule.onNodeWithTag(hubSnapshotRecallTag(venue.id)).performClick()
            pumpUntil("recalled") { synchronized(core.calls) { core.calls.contains("snapshotRecall(${venue.id})") } }
            pumpUntil("result") { model.result.value.startsWith("Recalled") }
        } finally {
            observer.dispose()
        }
        assertEquals("Compose state written off the main thread", emptyList<String>(), offMain.toList())
    }

    @Test
    fun wpP2anAHubWithoutTheCapShowsNoHubSection() {
        val core = FakeHubDspiCore(hubCaps = setOf("events"))
        core.snapshotCapture("Warm", "")
        val model = open(core)
        rule.setContent { SnapshotsView(model, SnapshotsMode.ADVANCED, onBack = {}) }
        pumpUntil("rows") { model.items.value.isNotEmpty() && !model.busy.value }
        rule.mainClock.advanceTimeBy(500)
        shadowOf(Looper.getMainLooper()).idle()
        assertTrue(!shown(HUB_SECTION_TAG))
        assertTrue(rule.onAllNodes(hasText("On the hub")).fetchSemanticsNodes().isEmpty())
    }

    /** Scroll the screen's list to the node tagged [tag]; false while it is not in the list yet. */
    private fun scrollTo(tag: String): Boolean = runCatching {
        rule.onNode(hasScrollAction()).performScrollToNode(hasTestTag(tag))
    }.isSuccess
}
