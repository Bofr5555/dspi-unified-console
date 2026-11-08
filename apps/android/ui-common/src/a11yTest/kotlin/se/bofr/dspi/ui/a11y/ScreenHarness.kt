package se.bofr.dspi.ui.a11y

import android.os.Looper
import android.view.View
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.material3.Surface
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.runtime.snapshots.Snapshot
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalView
import androidx.compose.ui.test.junit4.ComposeContentTestRule
import org.robolectric.Shadows.shadowOf
import se.bofr.dspi.ui.DspiTheme
import se.bofr.dspi.ui.settings.AppSettingsStore
import se.bofr.dspi.ui.settings.LocalAppSettings

/**
 * WP AN20: one screen in the app's dark theme with the clock paused, for
 * AtfChecks. Screens with live meters and pollers never idle with the clock
 * on auto (as :app's AN28 tests): [settle] runs frames by hand.
 */
class ScreenHarness(private val rule: ComposeContentTestRule, private val store: AppSettingsStore) {
    lateinit var view: View
        private set

    fun settle(until: () -> Boolean = { true }) {
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

    private var current by mutableStateOf<(@Composable () -> Unit)?>(null)
    private var hosting = false

    /** Shows [screen] in place of the last one: one composition, as screens swap under the NavHost. */
    fun show(screen: @Composable () -> Unit) {
        rule.mainClock.autoAdvance = false
        if (!hosting) {
            hosting = true
            rule.setContent {
                view = LocalView.current
                CompositionLocalProvider(LocalAppSettings provides store) {
                    DspiTheme(darkTheme = true) { Surface(Modifier.fillMaxSize()) { current?.invoke() } }
                }
            }
        }
        rule.runOnUiThread { current = null }
        settle()
        rule.runOnUiThread { current = screen }
        settle()
    }

    /** ATF over what is on screen now, after a check that ATF saw more than [minNodes] nodes. */
    fun assertAtfPasses(what: String, minNodes: Int = 10) {
        settle()
        val n = AtfChecks.nodeCount(view)
        check(n > minNodes) { "$what: ATF saw only $n nodes" }
        AtfChecks.assertPass(view, what)
    }
}
