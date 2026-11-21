// Shared rows and controls of the ported windows (decision E6: one copy for
// every feature). From :feature-tools ToolComponents.kt (tool windows of
// DSPi-Console-b5 @ b5fdeac: `.help` tooltips, captions, section headers,
// switch rows, segmented pickers, the borderless pull-down menu, the tool
// window header and channel chips), which the SETTINGS, ANALYSIS and GRAPH
// copies were folded into.
package se.bofr.dspi.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.selection.toggleable
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.PlainTooltip
import androidx.compose.material3.SegmentedButton
import androidx.compose.material3.SegmentedButtonDefaults
import androidx.compose.material3.SingleChoiceSegmentedButtonRow
import androidx.compose.material3.Slider
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TooltipBox
import androidx.compose.material3.TooltipDefaults
import androidx.compose.material3.rememberTooltipState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.semantics.CustomAccessibilityAction
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.customActions
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.TextUnit
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import kotlinx.coroutines.launch

/** The 9-pt secondary explanation under a control. */
@Composable
fun Caption(text: String, modifier: Modifier = Modifier, color: Color = Color.Unspecified) {
    Text(
        text,
        modifier,
        fontSize = 12.sp,
        color = if (color == Color.Unspecified) MaterialTheme.colorScheme.onSurfaceVariant else color,
        lineHeight = 16.sp
    )
}

/**
 * The macOS `.help(...)` tooltip, on touch: long-press the [content] to read it.
 * Nothing is wrapped when [help] is empty.
 *
 * WP AN20: TalkBack reads the help as a custom action ("Help: …", which also
 * shows the tooltip) rather than through the tooltip's long-click node, which
 * is only as large as the caption it wraps; the long-press stays for touch.
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun HelpTip(help: String, modifier: Modifier = Modifier, content: @Composable () -> Unit) {
    if (help.isEmpty()) {
        Box(modifier) { content() }
        return
    }
    val state = rememberTooltipState(isPersistent = true)
    val scope = rememberCoroutineScope()
    TooltipBox(
        positionProvider = TooltipDefaults.rememberPlainTooltipPositionProvider(),
        tooltip = { PlainTooltip { Text(help) } },
        state = state,
        modifier =
        modifier
            .pointerInput(state) { detectTapGestures(onLongPress = { scope.launch { state.show() } }) }
            .semantics(mergeDescendants = true) {
                customActions =
                    listOf(
                        CustomAccessibilityAction("Help: $help") {
                            scope.launch { state.show() }
                            true
                        }
                    )
            },
        enableUserInput = false
    ) { content() }
}

/** A section heading with a "Presets" / "Apply preset" menu on the right. */
@Composable
fun SectionHeader(title: String, modifier: Modifier = Modifier, trailing: @Composable () -> Unit = {}) {
    Row(modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
        SectionLabel(title)
        Spacer(Modifier.weight(1f))
        trailing()
    }
}

/** The borderless pull-down menu (macOS `Menu` with `.borderlessButton`: "Presets", "Apply preset", "All outputs"). */
@Composable
fun PullDownMenu(label: String, enabled: Boolean, items: List<Pair<String, () -> Unit>>, dividerBefore: Set<Int> = emptySet()) {
    @Suppress("NAME_SHADOWING")
    val enabled = writeEnabled(enabled)
    var open by remember { mutableStateOf(false) }
    Box {
        TextButton(onClick = { open = true }, enabled = enabled) { Text(label, fontSize = 14.sp) }
        DropdownMenu(expanded = open, onDismissRequest = { open = false }) {
            items.forEachIndexed { i, (title, action) ->
                if (i in dividerBefore) HorizontalDivider()
                DropdownMenuItem(text = { Text(title) }, onClick = {
                    open = false
                    action()
                })
            }
        }
    }
}

/**
 * A labelled switch row (macOS `Toggle` in a form / `optionRow`: "Interaural
 * Time Delay", "Lookahead", Settings and analyser options): title, optional
 * [subtitle], the switch; a tap anywhere on the row toggles it, as a click on
 * a macOS toggle's label does. [help] is the long-press tooltip.
 */
@Composable
fun SwitchRow(
    title: String,
    subtitle: String?,
    checked: Boolean,
    enabled: Boolean = true,
    modifier: Modifier = Modifier,
    help: String = "",
    onChange: (Boolean) -> Unit
) {
    @Suppress("NAME_SHADOWING")
    val enabled = writeEnabled(enabled)
    HelpTip(help, modifier.fillMaxWidth()) {
        Row(
            Modifier.fillMaxWidth().toggleable(value = checked, enabled = enabled, role = Role.Switch, onValueChange = onChange),
            verticalAlignment = Alignment.CenterVertically
        ) {
            Column(Modifier.weight(1f).padding(end = 12.dp)) {
                Text(title, fontSize = 15.sp, fontWeight = FontWeight.Medium, color = if (enabled) Color.Unspecified else Color.Gray)
                if (!subtitle.isNullOrEmpty()) Caption(subtitle)
            }
            Switch(checked = checked, onCheckedChange = null, enabled = enabled)
        }
    }
}

/** `Picker(.segmented)` over [options], each shown as [label]. */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun <T> SegmentedPicker(
    options: List<T>,
    selected: T,
    label: (T) -> String,
    onSelect: (T) -> Unit,
    modifier: Modifier = Modifier,
    enabled: Boolean = true,
    fontSize: TextUnit = 13.sp
) {
    @Suppress("NAME_SHADOWING")
    val enabled = writeEnabled(enabled)
    SingleChoiceSegmentedButtonRow(modifier.fillMaxWidth()) {
        options.forEachIndexed { i, o ->
            SegmentedButton(
                selected = o == selected,
                onClick = { if (o != selected) onSelect(o) },
                shape = SegmentedButtonDefaults.itemShape(i, options.size),
                enabled = enabled,
                icon = {},
                label = { Text(label(o), maxLines = 1, fontSize = fontSize) }
            )
        }
    }
}

/** `Picker(.segmented)` with Int tags: [labels] for [values] in display order. */
@Composable
fun SegmentedPicker(
    labels: List<String>,
    values: List<Int>,
    selected: Int,
    enabled: Boolean,
    onSelect: (Int) -> Unit,
    modifier: Modifier = Modifier
) = SegmentedPicker(values, selected, { labels[values.indexOf(it)] }, onSelect, modifier, enabled)

/**
 * A label (with the live value) above a slider, for app preferences (macOS
 * "Line Width: 2.0 pt" + Slider in Settings › Graphing and Graph Setup).
 * Not for device values: those go through [ParameterRow] / [CustomSlider].
 * Not gated by [LocalWritesAllowed] (preferences stay editable read-only).
 */
@Composable
fun SliderRow(
    label: String,
    value: Float,
    range: ClosedFloatingPointRange<Float>,
    modifier: Modifier = Modifier,
    steps: Int = 0,
    enabled: Boolean = true,
    detail: String? = null,
    onChange: (Float) -> Unit
) {
    Column(modifier.fillMaxWidth().alpha(if (enabled) 1f else 0.45f)) {
        Text(label, style = MaterialTheme.typography.bodyLarge)
        Slider(value = value, onValueChange = onChange, valueRange = range, steps = steps, enabled = enabled)
        if (!detail.isNullOrEmpty()) Caption(detail)
    }
}

/**
 * The tool window header: icon, title, subtitle, extras and the on/off switch
 * at the top right ("the same switch as the sidebar button").
 */
@Composable
fun ToolHeader(
    title: String,
    subtitle: String,
    isOn: Boolean,
    switchEnabled: Boolean,
    onToggle: (Boolean) -> Unit,
    modifier: Modifier = Modifier,
    icon: (@Composable () -> Unit)? = null,
    extras: @Composable () -> Unit = {}
) {
    @Suppress("NAME_SHADOWING")
    val switchEnabled = writeEnabled(switchEnabled)
    Row(modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 12.dp), verticalAlignment = Alignment.CenterVertically) {
        if (icon != null) {
            Box(Modifier.size(28.dp), contentAlignment = Alignment.Center) { icon() }
            Spacer(Modifier.width(12.dp))
        }
        Column(Modifier.weight(1f)) {
            Text(title, fontSize = 17.sp, fontWeight = FontWeight.SemiBold)
            Text(subtitle, fontSize = 12.sp, color = MaterialTheme.colorScheme.onSurfaceVariant)
        }
        extras()
        Spacer(Modifier.width(8.dp))
        Switch(checked = isOn, onCheckedChange = onToggle, enabled = switchEnabled)
    }
}

/**
 * A row of numbered chips: filled accent = on, grey = off; tap toggles, a
 * long-press shows the channel's name (`.help(outputName(out))`). [below]
 * draws under each chip (the Subharmonic Synthesizer's sub meter).
 */
@Composable
fun ChannelChips(
    count: Int,
    isOn: (Int) -> Boolean,
    names: List<String>,
    enabled: Boolean,
    onToggle: (Int) -> Unit,
    modifier: Modifier = Modifier,
    label: (Int) -> String = { "${it + 1}" },
    below: (@Composable (Int, Boolean) -> Unit)? = null
) {
    Row(modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(6.dp)) {
        for (i in 0 until count) {
            val on = isOn(i)
            val name = names.getOrElse(i) { "" }
            Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(3.dp)) {
                HelpTip(name) {
                    // -- WP AN20 -- the chip is its own TalkBack node, so the help's action never reaches it:
                    // the channel's name goes in the chip's description ("1, USB 1 L")
                    ToolChip(
                        label(i),
                        on,
                        enabled,
                        Modifier.fillMaxWidth(),
                        description = if (name.isEmpty()) null else "${label(i)}, $name"
                    ) { onToggle(i) }
                }
                below?.invoke(i, on)
            }
        }
    }
}

/** One chip (channel chips, tube shelf). [description] replaces [text] for TalkBack when set (WP AN20). */
@Composable
fun ToolChip(text: String, on: Boolean, enabled: Boolean, modifier: Modifier = Modifier, description: String? = null, onClick: () -> Unit) {
    @Suppress("NAME_SHADOWING")
    val enabled = writeEnabled(enabled)
    val shape = RoundedCornerShape(6.dp)
    val bg = if (on) MaterialTheme.colorScheme.primary else MaterialTheme.colorScheme.onSurfaceVariant.copy(alpha = 0.12f)
    Box(
        modifier
            .heightIn(min = 36.dp)
            .alpha(if (enabled) 1f else 0.5f)
            .background(bg, shape)
            .border(1.dp, if (on) Color.Transparent else MaterialTheme.colorScheme.onSurface.copy(alpha = 0.08f), shape)
            // -- WP AN20 -- TalkBack: "1, switch, on"
            .toggleable(value = on, enabled = enabled, role = Role.Switch, onValueChange = { onClick() })
            .then(if (description != null) Modifier.semantics { contentDescription = description } else Modifier),
        contentAlignment = Alignment.Center
    ) {
        Text(
            text,
            fontSize = 14.sp,
            fontWeight = FontWeight.SemiBold,
            maxLines = 1,
            // -- WP ANBAR -- onPrimary: white is not AA on the dark theme's lighter accent
            color = if (on) MaterialTheme.colorScheme.onPrimary else MaterialTheme.colorScheme.onSurface.copy(alpha = 0.6f)
        )
    }
}
