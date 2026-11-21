package se.bofr.dspi.app

import android.app.Application
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.runtime.snapshots.Snapshot
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import org.junit.Assert.assertEquals
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.annotation.Config
import se.bofr.dspi.ui.DspiTheme
import se.bofr.dspi.ui.LocalUserVolumeShown
import se.bofr.dspi.ui.ReportUserVolumeShown
import se.bofr.dspi.ui.UserVolumeShown

/**
 * WP ANVOL: the volume buttons' indicator. It shows the level a press set for a moment, and
 * stays away while a User Volume slider is on screen (the slider moves instead).
 */
@RunWith(DspiRobolectricTestRunner::class)
@Config(sdk = [35], application = Application::class)
class VolumeKeyIndicatorWpAnvolTest {
    @get:Rule val rule = createComposeRule()

    @Test
    fun wpAnvolTheWords() {
        assertEquals("User Volume -23.0 dB", volumeKeyIndicatorText(VolumeKeyFeedback(-23f, false, 1)))
        assertEquals("User Volume 0.0 dB · Muted", volumeKeyIndicatorText(VolumeKeyFeedback(0f, true, 2)))
    }

    @Test
    fun wpAnvolShownForAMomentUnlessASliderIsOnScreen() {
        val shown = UserVolumeShown()
        var feedback by mutableStateOf<VolumeKeyFeedback?>(null)
        var slider by mutableStateOf(false)

        // a change from the test thread reaches the composition with its apply notification
        fun set(change: () -> Unit) = rule.runOnUiThread {
            change()
            Snapshot.sendApplyNotifications()
        }
        rule.mainClock.autoAdvance = false
        rule.setContent {
            DspiTheme {
                CompositionLocalProvider(LocalUserVolumeShown provides shown) {
                    if (slider) ReportUserVolumeShown()
                    val count by shown.count.collectAsState()
                    VolumeKeyIndicator(feedback, sliderShown = count > 0)
                }
            }
        }
        rule.mainClock.advanceTimeByFrame()
        rule.onNodeWithTag(VOLUME_KEY_INDICATOR_TAG).assertDoesNotExist()

        set { feedback = VolumeKeyFeedback(-23f, false, 1) }
        rule.mainClock.advanceTimeBy(100)
        rule.onNodeWithText("User Volume -23.0 dB").assertExists()
        rule.mainClock.advanceTimeBy(VOLUME_KEY_INDICATOR_MS)
        rule.onNodeWithTag(VOLUME_KEY_INDICATOR_TAG).assertDoesNotExist()

        // the Home slider is composed: it moves, no indicator
        set { slider = true }
        rule.mainClock.advanceTimeBy(100)
        assertEquals(1, shown.count.value)
        set { feedback = VolumeKeyFeedback(-22f, false, 2) }
        rule.mainClock.advanceTimeBy(100)
        rule.onNodeWithTag(VOLUME_KEY_INDICATOR_TAG).assertDoesNotExist()

        set { slider = false }
        rule.mainClock.advanceTimeBy(100)
        assertEquals(0, shown.count.value)
    }
}
