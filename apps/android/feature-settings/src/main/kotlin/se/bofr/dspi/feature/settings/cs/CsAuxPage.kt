// Auxiliary Outputs (AN8's S13): the list, each with its switch and (dimmable)
// level, and Add Output; a row opens the output's page (S14): its live switch and
// level on top, then the shared binding editor (S3) with the core's aux rows
// (wiring, timing, power-on, driven by). The list was moved here from
// ControlSurfacesSettingsTab by WP AN9; the level, Add and S14 are WP AN12's.
package se.bofr.dspi.feature.settings.cs

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Slider
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.unit.dp
import kotlin.math.roundToInt
import se.bofr.dspi.bridge.DspiBridge
import se.bofr.dspi.feature.settings.CsBinding
import se.bofr.dspi.feature.settings.CsModel
import se.bofr.dspi.feature.settings.FormCaption
import se.bofr.dspi.feature.settings.RowDivider
import se.bofr.dspi.feature.settings.SettingRow
import se.bofr.dspi.feature.settings.SettingsForm
import se.bofr.dspi.feature.settings.SettingsSaveCoordinator
import se.bofr.dspi.feature.settings.SettingsSection
import se.bofr.dspi.feature.settings.StatusLine
import se.bofr.dspi.ui.writeEnabled

/** Where the aux stack is: the list, Add Output, or one output's page (survives rotation). */
private const val AUX_LIST = -1
private const val AUX_ADD = -2

@Composable
internal fun CsAuxPage(bridge: DspiBridge, coordinator: SettingsSaveCoordinator, cs: CsModel, connected: Boolean) {
    var route by rememberSaveable { mutableStateOf(AUX_LIST) }
    when (route) {
        AUX_LIST -> CsAuxList(bridge, coordinator, cs, connected, onAdd = { route = AUX_ADD }) { route = it }
        AUX_ADD -> CsAuxAddPage(bridge, cs, onCancel = { route = AUX_LIST }) { route = it }
        else -> CsAuxEditor(bridge, coordinator, cs, route, connected) { route = AUX_LIST }
    }
}

@Composable
private fun CsAuxList(
    bridge: DspiBridge,
    coordinator: SettingsSaveCoordinator,
    cs: CsModel,
    connected: Boolean,
    onAdd: () -> Unit,
    onOpen: (Int) -> Unit
) {
    val busy by coordinator.csBusy.collectAsState()
    val status by coordinator.csStatus.collectAsState()
    val summaries = csSummaries(bridge, cs)
    val free = firstFreeSlot(cs)
    SettingsForm {
        if (status != null) item { StatusLine(status?.first, status?.second ?: false) }
        val outs = cs.bindings.withIndex().filter { CsModel.isAuxType(it.value.type) }
        item {
            SettingsSection("Auxiliary Outputs") {
                if (outs.isEmpty()) FormCaption("No Auxiliary Outputs Configured")
                outs.forEachIndexed { n, (slot, b) ->
                    if (n > 0) RowDivider()
                    val on = cs.auxState.getOrElse(slot) { 0 } != 0
                    SettingRow(
                        cs.auxName(slot),
                        summaries[slot] ?: "${CsModel.typeName(b.type)} · GPIO ${b.gpio0}",
                        Modifier.testTag("cs.slot.$slot"),
                        onClick = { onOpen(slot) }
                    ) {
                        // Switching an output is instant and never writes to flash
                        Switch(
                            checked = on,
                            onCheckedChange = { coordinator.csOp(CsModel.auxStateOp(slot, it)) },
                            enabled = writeEnabled(connected && !busy && cs.isSlotActive(slot)),
                            modifier = Modifier.testTag("cs.aux.switch.$slot")
                        )
                    }
                    if (b.type == CsModel.TYPE_AUX_PWM) CsAuxLevel(bridge, coordinator, cs, slot, connected && !busy)
                }
            }
        }
        item {
            Box(Modifier.testTag("cs.aux.add")) {
                SettingRow(
                    "Add Output",
                    if (free == null) "Every slot is in use." else null,
                    enabled = writeEnabled(connected && !busy && free != null)
                ) {
                    OutlinedButton(
                        onClick = onAdd,
                        enabled = writeEnabled(connected && !busy && free != null),
                        modifier = Modifier.testTag("cs.aux.new")
                    ) { Text("Add") }
                }
            }
        }
        item {
            FormCaption(
                "An on/off output follows its switch. A dimmable output follows its switch and its level, so one button and one " +
                    "knob can " +
                    "share a lamp. Turn on \"Active-Low Output\" for the relay and opto-isolator boards that switch when the pin " +
                    "goes low.\n\n" +
                    "A GPIO is a 3.3 V pin good for a few milliamps. Anything real needs a MOSFET, a transistor with a flyback " +
                    "diode, or an " +
                    "opto-isolated relay module in between, and a dimmed load should have its own supply so its switching noise " +
                    "stays out of " +
                    "the DAC.\n\nSwitching an output is instant and never writes to flash. The pin, name and power-on behaviour " +
                    "are stored on " +
                    "the device alongside the controls and share their Save and Revert."
            )
        }
    }
}

/**
 * A dimmable output's live level: sent while dragged (not waited for), then
 * committed on release, which re-reads the model. Never a flash write.
 */
@Composable
private fun CsAuxLevel(bridge: DspiBridge, coordinator: SettingsSaveCoordinator, cs: CsModel, slot: Int, enabled: Boolean) {
    val live = CsRecords.auxPercent(cs.auxLevel.getOrElse(slot) { 0 })
    var dragging by remember(slot) { mutableStateOf<Float?>(null) }
    val shown = dragging ?: live.toFloat()
    Column(Modifier.fillMaxWidth().padding(horizontal = 16.dp).padding(bottom = 6.dp)) {
        Text("Level ${shown.roundToInt()} %", Modifier.testTag("cs.aux.levelText.$slot"))
        Slider(
            value = shown,
            onValueChange = { v ->
                if (dragging?.roundToInt() != v.roundToInt()) bridge.cs.sendOnly(CsRecords.auxLevelOp(slot, v.roundToInt().toDouble()))
                dragging = v
            },
            onValueChangeFinished = {
                dragging?.let { coordinator.csOp(CsRecords.auxLevelOp(slot, it.roundToInt().toDouble())) }
                dragging = null
            },
            valueRange = 0f..100f,
            enabled = writeEnabled(enabled && cs.isSlotActive(slot)),
            modifier = Modifier.testTag("cs.aux.level.$slot")
        )
    }
}

/** Add Output: the aux types the core offers for the first free slot, applied at once with its defaults. */
@Composable
private fun CsAuxAddPage(bridge: DspiBridge, cs: CsModel, onCancel: () -> Unit, onAdded: (Int) -> Unit) {
    val drafts = remember(bridge) { CsDrafts.of(bridge) }
    val slot = firstFreeSlot(cs)
    var types by remember { mutableStateOf<List<CsOption>?>(null) }
    var error by remember { mutableStateOf<String?>(null) }
    var adding by remember { mutableStateOf(false) }
    BackHandler(onBack = onCancel)
    LaunchedEffect(slot) {
        if (slot == null) return@LaunchedEffect
        // An aux output's Component row lists the aux types (a slot keeps to its page's types)
        val probe = draftOf(CsBinding(type = CsModel.TYPE_AUX_OUT))
        val r = CsRules.parseDescription(bridge.cs.describe(CsRules.KIND_BINDING, slot, probe))
        val row = r.value?.rows?.firstOrNull { it.id == "type" } as? CsRow.Pick
        types = row?.options?.filter { CsModel.isAuxType(it.value) && !it.disabled }
        error = r.error
    }
    SettingsForm {
        if (error != null) item { Box(Modifier.testTag("cs.error")) { StatusLine(error, true) } }
        item {
            SettingsSection("Add Output") {
                when {
                    slot == null -> FormCaption("Every slot is in use. Remove a control or an output first.")
                    types == null -> FormCaption("Reading…")
                    else -> types.orEmpty().forEach { t ->
                        SettingRow(t.text, modifier = Modifier.testTag(optionTag(t.value)), enabled = writeEnabled(!adding), onClick = {
                            adding = true
                            error = null
                            drafts.add(slot, t.value) { e ->
                                adding = false
                                if (e == null) onAdded(slot) else error = e
                            }
                        })
                    }
                }
            }
        }
        item { FormCaption("The new output is applied at once on a free pin and its page opens. Save keeps it across a restart.") }
    }
}

/** S14: the live switch and level on top, then the binding editor with the core's aux rows. */
@Composable
private fun CsAuxEditor(
    bridge: DspiBridge,
    coordinator: SettingsSaveCoordinator,
    cs: CsModel,
    slot: Int,
    connected: Boolean,
    onClose: () -> Unit
) {
    val b = cs.bindings.getOrNull(slot)
    val busy by coordinator.csBusy.collectAsState()
    Column {
        if (b != null && CsModel.isAuxType(b.type)) {
            val live = connected && !busy && cs.isSlotActive(slot)
            Column(Modifier.testTag("cs.aux.live")) {
                SettingRow(
                    "Output",
                    if (cs.isSlotActive(slot)) {
                        "Switches the pin now. Instant, and never written to flash."
                    } else {
                        "Apply the output first; the switch works once it is running."
                    }
                ) {
                    Switch(
                        checked = cs.auxState.getOrElse(slot) { 0 } != 0,
                        onCheckedChange = { coordinator.csOp(CsModel.auxStateOp(slot, it)) },
                        enabled = writeEnabled(live),
                        modifier = Modifier.testTag("cs.aux.switch.$slot")
                    )
                }
                if (b.type == CsModel.TYPE_AUX_PWM) CsAuxLevel(bridge, coordinator, cs, slot, connected && !busy)
            }
        }
        CsSlotEditor(bridge, coordinator, cs, slot, connected, onClose)
    }
}
