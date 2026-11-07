// Channel Groups (AN8's S8): the list. Moved here from ControlSurfacesSettingsTab
// by WP AN9 so that AN12 owns its own file; the group editor (S9) is AN12's.
package se.bofr.dspi.feature.settings.cs

import androidx.compose.runtime.Composable
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import se.bofr.dspi.feature.settings.CsModel
import se.bofr.dspi.feature.settings.FormCaption
import se.bofr.dspi.feature.settings.SettingRow
import se.bofr.dspi.feature.settings.SettingsForm
import se.bofr.dspi.feature.settings.SettingsSaveCoordinator
import se.bofr.dspi.feature.settings.SettingsSection
import se.bofr.dspi.feature.settings.StatusLine

@Composable
internal fun CsGroupsPage(coordinator: SettingsSaveCoordinator, cs: CsModel) {
    val status by coordinator.csStatus.collectAsState()
    SettingsForm {
        if (status != null) item { StatusLine(status?.first, status?.second ?: false) }
        val groups = cs.groups.withIndex().filter { it.value.configured }
        item {
            SettingsSection("Channel Groups") {
                if (groups.isEmpty()) FormCaption("No Channel Groups Configured")
                groups.forEach { (g, grp) ->
                    val kind = when (grp.targetKind) {
                        1 -> "Inputs"
                        2 -> "Outputs"
                        else -> "All Channels"
                    }
                    val users = cs.bindings.count { it.configured && (it.flags and 0x20) != 0 && it.target == g }
                    SettingRow(
                        cs.groupName(g),
                        "$kind · ${java.lang.Long.bitCount(
                            grp.memberMask
                        )} channels · Used by $users control${if (users == 1) "" else "s"}"
                    )
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
