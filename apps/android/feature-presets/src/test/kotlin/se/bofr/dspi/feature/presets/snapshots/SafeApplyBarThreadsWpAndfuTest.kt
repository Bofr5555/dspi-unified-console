package se.bofr.dspi.feature.presets.snapshots

import android.app.Application
import android.os.Looper
import androidx.activity.ComponentActivity
import androidx.compose.foundation.layout.Column
import androidx.compose.runtime.snapshots.Snapshot
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import java.util.Collections
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
import se.bofr.dspi.core.GuardPhase
import se.bofr.dspi.core.GuardedApply
import se.bofr.dspi.core.Notice
import se.bofr.dspi.core.NoticeKind
import se.bofr.dspi.fake.FakeDspiCore
import se.bofr.dspi.feature.presets.presetFake

/**
 * WP ANDFU (FLAKE2 follow-up): the safe apply bar and the Snapshots screen write
 * their Compose state on the main thread only, though the bridge's calls run on
 * its real FFI thread and the effects resume wherever Compose's test dispatcher
 * leaves them. A global write observer sees every snapshot write made outside a
 * composition, on the writing thread; none may come from another thread.
 */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35], application = Application::class)
class SafeApplyBarThreadsWpAndfuTest {
    @get:Rule val rule = createAndroidComposeRule<ComponentActivity>()

    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)

    @After fun stop() = scope.cancel()

    /** A guard that waits (its time left new on every poll), a countdown and a backstop sentence of the core's. */
    private class Fake : FakeDspiCore(presetFake().getState()) {
        @Volatile private var left = 30_000L

        override fun applyStatus() = waiting()

        override fun applyPoll(): GuardedApply {
            calls += "applyPoll"
            left -= 250
            return waiting()
        }

        override fun noticeForKey(key: String, detail: String): Notice =
            Notice(key, NoticeKind.INFO, "Keep these?", "Back in {s} s.", "Press Keep.", detail)

        private fun waiting() = GuardedApply(7, GuardPhase.WAITING, "Room", true, 30_000, left, "", "", emptyList(), emptyList(), "")
    }

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

    @Test
    fun andfuTheBarAndTheScreenWriteTheirStateOnTheMainThreadOnly() {
        val core = Fake()
        // The bridge's own FFI thread, as on the phone
        val bridge = DspiBridgeImpl(core, scope)
        rule.mainClock.autoAdvance = false
        bridge.scanDevices()
        pumpUntil("connected") { bridge.status.value.connected }
        val offMain = Collections.synchronizedList(mutableListOf<String>())
        val observer =
            Snapshot.registerGlobalWriteObserver {
                if (Looper.myLooper() != Looper.getMainLooper()) offMain += Thread.currentThread().name
            }
        try {
            val model = SnapshotsModel(bridge, scope, folder = null)
            rule.setContent {
                Column {
                    SafeApplyBar(bridge)
                    SnapshotsView(model, SnapshotsMode.ADVANCED, onBack = {})
                }
            }
            pumpUntil("the core's sentence") {
                rule.onAllNodes(hasText("Keep these?", substring = true)).fetchSemanticsNodes().isNotEmpty()
            }
            pumpUntil("polls") { synchronized(core.calls) { core.calls.count { it == "applyPoll" } } >= 5 }
            rule.mainClock.advanceTimeBy(1_000)
            shadowOf(Looper.getMainLooper()).idle()
        } finally {
            observer.dispose()
        }
        assertEquals("Compose state written off the main thread", emptyList<String>(), offMain.toList())
    }
}
