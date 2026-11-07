// The Control Surfaces page's stack (AN8's S1–S3): the list of controls with
// what each does, Add (a component type, applied at once with the core's defaults,
// then its editor), and one control's editor. The route survives rotation; the
// drafts live in CsDrafts (WP AN9).
package se.bofr.dspi.feature.settings.cs

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.layout.Box
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateMapOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import se.bofr.dspi.bridge.DspiBridge
import se.bofr.dspi.feature.settings.CsBinding
import se.bofr.dspi.feature.settings.CsModel
import se.bofr.dspi.feature.settings.FormCaption
import se.bofr.dspi.feature.settings.Pill
import se.bofr.dspi.feature.settings.SettingRow
import se.bofr.dspi.feature.settings.SettingsForm
import se.bofr.dspi.feature.settings.SettingsSaveCoordinator
import se.bofr.dspi.feature.settings.SettingsSection
import se.bofr.dspi.feature.settings.StatusLine
import se.bofr.dspi.ui.MacColors
import se.bofr.dspi.ui.json.Json
import se.bofr.dspi.ui.writeEnabled

/** Where the Control Surfaces stack is: the list, Add, or one slot's editor. */
internal sealed interface CsRoute {
    data object List : CsRoute

    data object Add : CsRoute

    data class Edit(val slot: Int) : CsRoute

    companion object {
        fun save(r: CsRoute): String = when (r) {
            List -> "list"
            Add -> "add"
            is Edit -> "edit/${r.slot}"
        }

        fun restore(s: String): CsRoute = when {
            s == "add" -> Add
            s.startsWith("edit/") -> s.removePrefix("edit/").toIntOrNull()?.let { Edit(it) } ?: List
            else -> List
        }
    }
}

/** A binding as a rules draft: the same JSON as `cs_snapshot`'s record. */
internal fun draftOf(b: CsBinding): String = Json.write(b.json())

/** A record's editor, or null when the slot is gone (removed elsewhere): then the stack falls back to [onGone]. */
@Composable
internal fun CsSlotEditor(
    bridge: DspiBridge,
    coordinator: SettingsSaveCoordinator,
    cs: CsModel,
    slot: Int,
    connected: Boolean,
    onClose: () -> Unit
) {
    val drafts = remember(bridge) { CsDrafts.of(bridge) }
    val b = cs.bindings.getOrNull(slot)
    if (b == null || !b.configured) {
        LaunchedEffect(slot) { onClose() }
        return
    }
    // -- WP AN11 -- a display opens its own page (S6): wiring, panel state, settings and pages
    if (b.type == CsModel.TYPE_DISPLAY) {
        CsDisplayStack(bridge, coordinator, cs, slot, connected, onClose)
        return
    }
    val busy by coordinator.csBusy.collectAsState()
    val editor = remember(drafts, slot) { drafts.editor(CsRules.KIND_BINDING, slot, draftOf(b), cs.slotName(slot)) }
    // The device's copy moved (an Apply re-read, a revert): an untouched draft follows it
    LaunchedEffect(editor, b, cs.slotName(slot)) { editor.sync(draftOf(b), cs.slotName(slot)) }
    CsBindingEditorPage(editor, connected, busy, onClose) {
        coordinator.csOp(CsModel.clearBindingOp(slot))
        drafts.drop(CsRules.KIND_BINDING, slot)
        onClose()
    }
}

@Composable
internal fun CsControlsPage(bridge: DspiBridge, coordinator: SettingsSaveCoordinator, cs: CsModel, connected: Boolean) {
    var routeText by rememberSaveable { mutableStateOf(CsRoute.save(CsRoute.List)) }
    val route = CsRoute.restore(routeText)
    val go: (CsRoute) -> Unit = { routeText = CsRoute.save(it) }
    when (route) {
        CsRoute.List -> CsControlsList(bridge, coordinator, cs, onAdd = { go(CsRoute.Add) }, onOpen = { go(CsRoute.Edit(it)) })
        CsRoute.Add -> CsAddPage(bridge, cs, onCancel = { go(CsRoute.List) }, onAdded = { go(CsRoute.Edit(it)) })
        is CsRoute.Edit -> CsSlotEditor(bridge, coordinator, cs, route.slot, connected) { go(CsRoute.List) }
    }
}

/** S1: every configured control except the aux outputs (their own page), with the core's summary. */
@Composable
private fun CsControlsList(
    bridge: DspiBridge,
    coordinator: SettingsSaveCoordinator,
    cs: CsModel,
    onAdd: () -> Unit,
    onOpen: (Int) -> Unit
) {
    val busy by coordinator.csBusy.collectAsState()
    val status by coordinator.csStatus.collectAsState()
    val slots = cs.bindings.withIndex().filter { it.value.configured && !CsModel.isAuxType(it.value.type) }
    val summaries = csSummaries(bridge, cs)
    val free = firstFreeSlot(cs)
    SettingsForm {
        if (status != null) item { StatusLine(status?.first, status?.second ?: false) }
        item {
            SettingsSection("Controls") {
                if (slots.isEmpty()) FormCaption("No Controls Configured")
                slots.forEach { (slot, b) ->
                    val name = cs.slotName(slot).ifEmpty { CsModel.typeName(b.type) }
                    SettingRow(
                        name,
                        summaries[slot] ?: "${CsModel.typeName(b.type)} · ${gpioText(b)}",
                        Modifier.testTag("cs.slot.$slot"),
                        onClick = { onOpen(slot) }
                    ) { SlotPill(cs, slot) }
                }
            }
        }
        item {
            Box(Modifier.testTag("cs.add")) {
                SettingRow(
                    "Add Control",
                    if (free ==
                        null
                    ) {
                        "Every slot is in use."
                    } else {
                        null
                    },
                    enabled = writeEnabled(!busy && free != null)
                ) {
                    OutlinedButton(onClick = onAdd, enabled = writeEnabled(!busy && free != null)) { Text("Add") }
                }
            }
        }
        item {
            FormCaption(
                "Buttons and switches wire between the GPIO and GND (internal pull-up); pots use an ADC pin (GPIO 26, 27, or 28) " +
                    "between 3V3 and GND; encoders use two GPIOs with the common wired to GND. LEDs drive active-high by default. " +
                    "An IR receiver module's OUT pin connects to any GPIO, and its remote buttons are learned by pressing them at " +
                    "the device.\n\nThis wiring is a board-level setting: it is stored on the device, survives preset changes, and " +
                    "survives a factory reset. Applied changes work at once; Save keeps them across a restart." +
                    if (cs.capsVersion != 0) "\n\nControl-surface capability version ${cs.capsVersion}." else ""
            )
        }
    }
}

/** S2: the component types the core offers for the first free slot. */
@Composable
private fun CsAddPage(bridge: DspiBridge, cs: CsModel, onCancel: () -> Unit, onAdded: (Int) -> Unit) {
    val drafts = remember(bridge) { CsDrafts.of(bridge) }
    val slot = firstFreeSlot(cs)
    var types by remember { mutableStateOf<List<CsOption>?>(null) }
    var error by remember { mutableStateOf<String?>(null) }
    var adding by remember { mutableStateOf(false) }
    BackHandler(onBack = onCancel)
    LaunchedEffect(slot) {
        if (slot == null) return@LaunchedEffect
        // An empty slot's Component row lists what may go in it (IR and display once each)
        val r = CsRules.parseDescription(bridge.cs.describe(CsRules.KIND_BINDING, slot, draftOf(CsBinding())))
        val row = r.value?.rows?.firstOrNull { it.id == "type" } as? CsRow.Pick
        types = row?.options?.filter { it.value != CsModel.TYPE_NONE && !it.disabled }
        error = r.error
    }
    SettingsForm {
        if (error != null) item { Box(Modifier.testTag("cs.error")) { StatusLine(error, true) } }
        item {
            SettingsSection("Add Control") {
                when {
                    slot == null -> FormCaption("Every slot is in use. Remove a control first.")
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
        item {
            FormCaption("The new control is applied at once on free pins, and its page opens. Save keeps it across a restart.")
        }
    }
}

/** The core's one-line summary per configured slot ("Adjusts Volume", ...), refreshed with the model. */
@Composable
internal fun csSummaries(bridge: DspiBridge, cs: CsModel): Map<Int, String> {
    val out = remember { mutableStateMapOf<Int, String>() }
    LaunchedEffect(cs) {
        out.clear()
        cs.bindings.forEachIndexed { slot, b ->
            if (!b.configured) return@forEachIndexed
            CsRules.parseDescription(bridge.cs.describe(CsRules.KIND_BINDING, slot, draftOf(b))).value?.let { d ->
                out[slot] = listOf(d.title, d.summary).filter { it.isNotEmpty() }.joinToString(" · ")
            }
        }
    }
    return out
}

internal fun firstFreeSlot(cs: CsModel): Int? = cs.bindings.indexOfFirst { !it.configured }.takeIf { it >= 0 }

private fun gpioText(b: CsBinding): String = if (b.gpio1 != 0xFF && b.gpio1 != b.gpio0) "GPIO ${b.gpio0}/${b.gpio1}" else "GPIO ${b.gpio0}"

@Composable
private fun SlotPill(cs: CsModel, slot: Int) {
    if (cs.isSlotActive(slot)) Pill("Active", MacColors.green) else Pill("Inactive", MacColors.orange)
}
