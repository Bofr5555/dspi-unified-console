// Port of DSPi-Console-b5:DSPi Console/Components.swift (MeterBarNSView,
// HorizontalMeterBar, ChannelMeterBar, CpuMeter, CpuSection) @ b5fdeac.
package se.bofr.dspi.ui

import androidx.compose.animation.core.Animatable
import androidx.compose.animation.core.LinearEasing
import androidx.compose.animation.core.tween
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.derivedStateOf
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.runtime.snapshotFlow
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.draw.clip
import androidx.compose.ui.geometry.CornerRadius
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.semantics.ProgressBarRangeInfo
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.progressBarRangeInfo
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp

/**
 * A level bar (macOS `MeterBarNSView`): a 60 ms linear glide between polls,
 * and a 3 pt red marker at the right end while the channel has clipped.
 *
 * [level] and [clipping] are read in the draw phase, so a 60 ms status poll
 * moves the bar without recomposing the row around it (macOS: the row does
 * not observe the meters). Pass lambdas that read a Compose `State`.
 *
 * With a [label] TalkBack reads the meter ("USB 1 level, -12 dB"): the level
 * in whole [MeterReadout] steps, so a 60 ms poll changes the semantics only
 * when the spoken value does.
 */
@Composable
fun HorizontalMeterBar(
    level: () -> Float,
    color: Color,
    modifier: Modifier = Modifier,
    isMuted: Boolean = false,
    clipping: () -> Boolean = { false },
    label: String? = null
) {
    val anim = remember { Animatable(0f) }
    // -- WP AN20 -- what TalkBack reads, recomputed only when the spoken value changes
    val spoken by remember { derivedStateOf { MeterReadout.of(level(), clipping()) } }
    val a11y = if (label == null) {
        Modifier
    } else {
        Modifier.semantics {
            contentDescription = label
            stateDescription = spoken.text + if (isMuted) ", muted" else ""
            progressBarRangeInfo = ProgressBarRangeInfo(spoken.fraction, 0f..1f)
        }
    }
    LaunchedEffect(Unit) {
        snapshotFlow { level().coerceIn(0f, 1f) }.collect { anim.animateTo(it, tween(60, easing = LinearEasing)) }
    }
    val red = DspiTheme.colors.red
    Canvas(modifier.height(6.dp).fillMaxWidth().alpha(if (isMuted) 0.4f else 1f).then(a11y)) {
        val clipZone = 3.dp.toPx()
        val meterWidth = (size.width - clipZone).coerceAtLeast(0f)
        val r = CornerRadius(2.dp.toPx())
        drawRoundRect(color, Offset.Zero, Size(anim.value * meterWidth, size.height), r)
        if (clipping()) drawRoundRect(red, Offset(meterWidth, 0f), Size(clipZone, size.height), r)
    }
}

/** A meter bar for a plain value the caller supplies. */
@Composable
fun HorizontalMeterBar(
    level: Float,
    color: Color,
    modifier: Modifier = Modifier,
    isMuted: Boolean = false,
    isClipping: Boolean = false,
    label: String? = null
) {
    HorizontalMeterBar({ level }, color, modifier, isMuted, { isClipping }, label)
}

/**
 * WP AN20: a meter's level (linear peak, 0..1) as TalkBack reads it, in
 * [STEP_DB] steps of dBFS, "Silent" below [FLOOR_DB], "Clipping" while the
 * channel clips. Pure (JVM-tested).
 */
data class MeterReadout(val text: String, val fraction: Float) {
    companion object {
        const val STEP_DB = 3
        const val FLOOR_DB = -60

        fun of(level: Float, clipping: Boolean): MeterReadout {
            val l = level.coerceIn(0f, 1f)
            if (clipping) return MeterReadout("Clipping", 1f)
            val db = if (l > 0f) 20.0 * kotlin.math.log10(l.toDouble()) else Double.NEGATIVE_INFINITY
            if (db < FLOOR_DB) return MeterReadout("Silent", 0f)
            val stepped = (kotlin.math.round(db / STEP_DB) * STEP_DB).toInt()
            return MeterReadout("$stepped dB", ((stepped - FLOOR_DB).toFloat() / -FLOOR_DB).coerceIn(0f, 1f))
        }
    }
}

/** `CpuMeter`: "C0:" + a 40 pt bar (red above 90 %) + the percentage. */
@Composable
fun CpuMeter(core: Int, load: Int, modifier: Modifier = Modifier) {
    val colors = DspiTheme.colors
    Row(
        // -- WP AN20 -- one node: "Core 0 load, 40%", not "C0:" and "40%"
        modifier.clearAndSetSemantics {
            contentDescription = "Core $core load"
            stateDescription = "$load%"
            progressBarRangeInfo = ProgressBarRangeInfo(load.coerceIn(0, 100).toFloat(), 0f..100f)
        },
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(4.dp)
    ) {
        Text("C$core:", fontSize = 11.sp, color = colors.secondaryLabel)
        Box(Modifier.size(40.dp, 6.dp).clip(RoundedCornerShape(2.dp)).background(Color.Gray.copy(alpha = 0.3f))) {
            Box(
                Modifier
                    .fillMaxHeight()
                    .size(width = (load.coerceIn(0, 100) * 0.4f).dp, height = 6.dp)
                    .clip(RoundedCornerShape(2.dp))
                    .background(if (load > 90) colors.red else colors.blue)
            )
        }
        Text("$load%", style = DspiType.mono9, color = colors.label)
    }
}

/** `CpuSection`: Core 0 on the left, Core 1 on the right. */
@Composable
fun CpuSection(cpu0: Int, cpu1: Int, modifier: Modifier = Modifier) {
    Row(modifier.fillMaxWidth().padding(vertical = 2.dp), horizontalArrangement = Arrangement.SpaceBetween) {
        CpuMeter(0, cpu0)
        CpuMeter(1, cpu1)
    }
}
