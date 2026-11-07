// Channel Groups (AN8's S8): the list, Add Group, and a group's page (S9, in
// CsGroupEditorPage). The list was moved here from ControlSurfacesSettingsTab by
// WP AN9; the page, Add and the open draft are WP AN12's. A new group is not
// applied until it has a member (the Mac's addGroup): the firmware refuses one
// with none.
package se.bofr.dspi.feature.settings.cs

import androidx.compose.foundation.layout.Box
import androidx.compose.material3.OutlinedButton
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
import se.bofr.dspi.bridge.DspiBridge
import se.bofr.dspi.feature.settings.CsModel
import se.bofr.dspi.feature.settings.FormCaption
import se.bofr.dspi.feature.settings.SettingRow
import se.bofr.dspi.feature.settings.SettingsForm
import se.bofr.dspi.feature.settings.SettingsSaveCoordinator
import se.bofr.dspi.feature.settings.SettingsSection
import se.bofr.dspi.feature.settings.StatusLine
import se.bofr.dspi.ui.MacColors
import se.bofr.dspi.ui.writeEnabled

@Composable
internal fun CsGroupsPage(bridge: DspiBridge, coordinator: SettingsSaveCoordinator, cs: CsModel, connected: Boolean) {
    val drafts = remember(bridge) { CsRecordDrafts.of(bridge) }
    var editing by rememberSaveable { mutableStateOf<Int?>(null) }
    editing?.let { g ->
        CsGroupSlot(bridge, coordinator, drafts, g, connected) { editing = null }
        return
    }
    val busy by coordinator.csBusy.collectAsState()
    val status by coordinator.csStatus.collectAsState()
    // A group with an unapplied draft (a new one included) stays on the list until it is applied or dropped
    val shown = cs.groups.indices.filter { cs.groups[it].configured || drafts.isGroupDirty(it) }
    val free = cs.groups.indices.firstOrNull { !cs.groups[it].configured && !drafts.isGroupDirty(it) }
    SettingsForm {
        if (status != null) item { StatusLine(status?.first, status?.second ?: false) }
        item {
            SettingsSection("Channel Groups") {
                if (shown.isEmpty()) FormCaption("No Channel Groups Configured")
                shown.forEach { g ->
                    val grp = cs.groups[g]
                    val kind = when (grp.targetKind) {
                        1 -> "Inputs"
                        2 -> "Outputs"
                        else -> "All Channels"
                    }
                    val users = cs.bindings.count { it.configured && (it.flags and 0x20) != 0 && it.target == g }
                    val pending = drafts.isGroupDirty(g)
                    SettingRow(
                        cs.groupName(g),
                        if (!grp.configured) {
                            "Not applied"
                        } else {
                            "$kind · ${java.lang.Long.bitCount(
                                grp.memberMask
                            )} channels · Used by $users control${if (users == 1) "" else "s"}" + if (pending) " · Edited" else ""
                        },
                        Modifier.testTag("cs.group.$g"),
                        detailColor = if (pending) MacColors.orange else androidx.compose.ui.graphics.Color.Unspecified,
                        onClick = { editing = g }
                    )
                }
            }
        }
        item {
            Box(Modifier.testTag("cs.group.add")) {
                SettingRow(
                    "Add Group",
                    if (free == null) "Every group slot is in use." else null,
                    enabled = writeEnabled(connected && !busy && free != null)
                ) {
                    OutlinedButton(onClick = {
                        if (free != null) {
                            drafts.newGroup(free)
                            editing = free
                        }
                    }, enabled = writeEnabled(connected && !busy && free != null), modifier = Modifier.testTag("cs.group.new")) {
                        Text("Add")
                    }
                }
            }
        }
        item {
            FormCaption(
                "A group is a named set of channels one control can drive as a unit: mute a zone, trim a stereo pair, light an " +
                    "LED when any " +
                    "member clips. Turn on \"Address a Group\" in a control, or in a macro step, to use one.\n\nRelative moves " +
                    "(an encoder, a " +
                    "button) step every member from its own value, so the balance between them survives. A knob moves the " +
                    "group's average and " +
                    "keeps those offsets unless \"Match Members Exactly\" is on.\n\nGroups are stored on the device alongside " +
                    "the controls and " +
                    "share their Save and Revert."
            )
        }
    }
}

/** Group [g]'s page over its draft, kept in step with the device's copy. */
@Composable
private fun CsGroupSlot(
    bridge: DspiBridge,
    coordinator: SettingsSaveCoordinator,
    drafts: CsRecordDrafts,
    g: Int,
    connected: Boolean,
    onClose: () -> Unit
) {
    val model by bridge.cs.model.collectAsState()
    val saved = remember(model, g) { CsRecords.group(model, g) }
    if (saved == null) {
        LaunchedEffect(g) { onClose() }
        return
    }
    val busy by coordinator.csBusy.collectAsState()
    val editor = remember(drafts, g) { drafts.group(g, saved.first, saved.second) }
    LaunchedEffect(editor, saved) { editor.sync(saved.first, saved.second) }
    CsGroupEditorPage(editor, connected, busy, onClose) {
        if (editor.state.value.stored) coordinator.csOp(CsRecords.groupOp(g, CsRecords.GROUP_CLEARED, "") ?: return@CsGroupEditorPage)
        drafts.dropGroup(g)
        onClose()
    }
}
