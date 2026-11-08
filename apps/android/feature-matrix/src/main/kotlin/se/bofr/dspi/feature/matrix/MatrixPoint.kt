// Port of DSPi-Console (macOS) `DSPi Console/MatrixMixerView.swift` (MatrixPoint)
// @ v1.1.6-beta5, plus the power / speaker glyphs it uses (SF Symbols "power",
// "speaker.wave.2.fill", "speaker.slash.fill"), drawn locally because the
// build has no icon library. Consolidation note: the glyphs can move to
// :ui-common once HOME provides icons.
package se.bofr.dspi.feature.matrix

import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.animation.core.spring
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.selection.toggleable
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.minimumInteractiveComponentSize
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import se.bofr.dspi.ui.CompactGainField
import se.bofr.dspi.ui.MacColors
import se.bofr.dspi.ui.writeEnabled

/**
 * macOS `MatrixPoint`: gain field above the dot when connected, the connect
 * toggle (filled dot / empty ring), and INV below when connected.
 * [outlineColor] overrides the disconnected ring (PDM conflict: orange).
 * [label] names the crosspoint for TalkBack ("USB 1 L to OUT1"). Each of
 * the three controls has a 48 dp row (WP AN20).
 */
@Composable
fun MatrixPoint(
    isConnected: Boolean,
    gain: Float,
    isInverted: Boolean,
    inputColor: Color,
    onToggleConnected: () -> Unit,
    onGainLive: (Float) -> Unit,
    onGainCommit: (Float) -> Unit,
    onToggleInverted: () -> Unit,
    modifier: Modifier = Modifier,
    outlineColor: Color? = null,
    enabled: Boolean = true,
    label: String = ""
) {
    Column(modifier = modifier, horizontalAlignment = Alignment.CenterHorizontally) {
        if (isConnected) {
            CompactGainField(
                gain,
                onGainLive,
                onGainCommit,
                enabled = enabled,
                title = "Gain, $label",
                quantized = false
            )
        } else {
            Spacer(Modifier.height(MatrixTarget))
        }
        MatrixConnectToggle(isConnected, inputColor, outlineColor, enabled, label, onToggleConnected)
        if (isConnected) {
            MatrixInvertToggle(isInverted, enabled, onToggleInverted, label = label)
        } else {
            Spacer(Modifier.height(MatrixTarget))
        }
    }
}

/** WP AN20: the smallest touch target a matrix control gets. */
internal val MatrixTarget = 48.dp

/** The crosspoint circle; a 48 dp touch target around macOS's 28 pt one. TalkBack: "[label], switch, Connected". */
@Composable
fun MatrixConnectToggle(
    isConnected: Boolean,
    inputColor: Color,
    outlineColor: Color?,
    enabled: Boolean,
    label: String = "",
    onToggle: () -> Unit
) {
    val dot by animateFloatAsState(if (isConnected) 1f else 0f, spring(dampingRatio = 0.7f, stiffness = 600f), label = "dot")
    val ring = outlineColor?.copy(alpha = 0.45f) ?: MaterialTheme.colorScheme.onSurface.copy(alpha = 0.25f)
    Box(
        modifier =
        Modifier
            .size(MatrixTarget)
            .toggleable(
                value = isConnected,
                enabled = writeEnabled(enabled),
                role = Role.Switch,
                interactionSource = remember { MutableInteractionSource() },
                indication = null,
                onValueChange = { onToggle() }
            )
            // -- WP AN20 -- which crosspoint, and whether it routes (was "Connect"/"Disconnect" alone)
            .semantics {
                contentDescription = label.ifEmpty { "Route" }
                stateDescription = if (isConnected) "Connected" else "Not connected"
            },
        contentAlignment = Alignment.Center
    ) {
        Canvas(Modifier.size(22.dp)) {
            if (dot < 1f) {
                drawCircle(ring, radius = 9.dp.toPx() * (1f - dot * 0.2f), style = Stroke(1.5.dp.toPx()))
            }
            if (dot > 0f) drawCircle(inputColor, radius = 8.dp.toPx() * dot)
        }
    }
}

/** INV: TalkBack reads "Invert phase, [label], switch, on/off" (WP AN20). */
@Composable
fun MatrixInvertToggle(isInverted: Boolean, enabled: Boolean, onToggle: () -> Unit, modifier: Modifier = Modifier, label: String = "") {
    Box(
        modifier =
        modifier
            // -- WP AN20 -- a 48 dp target around the label
            .minimumInteractiveComponentSize()
            .toggleable(value = isInverted, enabled = writeEnabled(enabled), role = Role.Switch, onValueChange = { onToggle() })
            .semantics { contentDescription = listOf("Invert phase", label).filter { it.isNotEmpty() }.joinToString(", ") }
            .padding(horizontal = 10.dp, vertical = 3.dp),
        contentAlignment = Alignment.Center
    ) {
        Text(
            "INV",
            fontSize = 10.sp,
            fontWeight = if (isInverted) FontWeight.Bold else FontWeight.Medium,
            color = if (isInverted) MacColors.orange else MaterialTheme.colorScheme.onSurface.copy(alpha = 0.35f)
        )
    }
}

/** Small filled circle in the row's colour (phone list). */
@Composable
internal fun ColorDot(color: Color, modifier: Modifier = Modifier) {
    Canvas(modifier.size(8.dp)) { drawCircle(color) }
}
