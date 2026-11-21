// WP ANVOL: the volume buttons' brief on-screen level, in place of Android's volume panel,
// when no User Volume slider is on screen to move instead.
package se.bofr.dspi.app

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.systemBars
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.delay
import se.bofr.dspi.ui.DspiTheme
import se.bofr.dspi.ui.DspiType
import se.bofr.dspi.ui.UserVolumeTaper

internal const val VOLUME_KEY_INDICATOR_TAG = "volumeKeyIndicator"

/** How long the indicator stays after the last press, ms. */
internal const val VOLUME_KEY_INDICATOR_MS = 1_500L

/** The indicator's words: "User Volume -23.0 dB", "· Muted" while muted. */
internal fun volumeKeyIndicatorText(f: VolumeKeyFeedback): String =
    "User Volume " + UserVolumeTaper.displayString(f.db) + if (f.muted) " · Muted" else ""

/**
 * Shows each new [feedback] for [VOLUME_KEY_INDICATOR_MS] at the top of the screen, unless
 * [sliderShown] (the Home slider moves instead). Takes no touches.
 */
@Composable
internal fun VolumeKeyIndicator(feedback: VolumeKeyFeedback?, sliderShown: Boolean) {
    var visible by remember { mutableStateOf<VolumeKeyFeedback?>(null) }
    LaunchedEffect(feedback) {
        if (feedback == null) return@LaunchedEffect
        visible = feedback
        delay(VOLUME_KEY_INDICATOR_MS)
        visible = null
    }
    val f = visible ?: return
    if (sliderShown) return
    val c = DspiTheme.colors
    Box(Modifier.fillMaxSize().windowInsetsPadding(WindowInsets.systemBars), contentAlignment = Alignment.TopCenter) {
        Text(
            volumeKeyIndicatorText(f),
            style = DspiType.bodyMono,
            color = c.label,
            modifier = Modifier
                .padding(top = 12.dp)
                .background(c.controlBackground, RoundedCornerShape(16.dp))
                .padding(horizontal = 16.dp, vertical = 8.dp)
                .semantics { liveRegion = LiveRegionMode.Polite }
                .testTag(VOLUME_KEY_INDICATOR_TAG)
        )
    }
}
