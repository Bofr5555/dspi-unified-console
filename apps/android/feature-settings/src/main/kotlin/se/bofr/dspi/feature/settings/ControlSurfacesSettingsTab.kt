// Port of DSPi-Console-b5:DSPi Console/DSPi_ConsoleApp.swift
// `ControlSurfacesSettingsTab` (sections .controls / .groups / .macros / .aux),
// over the core's cs_snapshot model (CsController). This file only says which
// page is shown and when there is nothing to show; the pages are in `cs/`, one
// file set per record kind (WP AN9's split: AN10 IR, AN11 display, AN12 groups,
// macros and aux each add theirs there). Applied edits are live and leave the
// device dirty; the save bar's Save persists them (CS_SAVE) and Revert discards
// them (cs.revert, ungated D1).
package se.bofr.dspi.feature.settings

import androidx.compose.runtime.Composable
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import se.bofr.dspi.bridge.DspiBridge
import se.bofr.dspi.feature.settings.cs.CsAuxPage
import se.bofr.dspi.feature.settings.cs.CsControlsPage
import se.bofr.dspi.feature.settings.cs.CsGroupsPage
import se.bofr.dspi.feature.settings.cs.CsMacrosPage

/** Which Control page (macOS `ControlSurfacesSettingsTab.Section`). */
enum class CsSection { CONTROLS, GROUPS, MACROS, AUX }

@Composable
fun ControlSurfacesSettingsTab(section: CsSection, bridge: DspiBridge, coordinator: SettingsSaveCoordinator, cs: CsModel) {
    val connected = bridge.status.collectAsState().value.connected
    val loaded by bridge.cs.loaded.collectAsState()

    // A lost connection keeps the last model, so an open editor keeps its draft readable (never replayed)
    val message = when {
        !connected && !cs.supported -> "No Device Connected. Control surfaces live on the device."
        !connected -> null
        !loaded -> "Reading control-surface capabilities from the device..."
        !cs.supported -> "The connected firmware has no control surfaces."
        else -> null
    }
    if (message != null) {
        SettingsForm { item { SettingsSection(null) { FormCaption(message) } } }
        return
    }
    when (section) {
        CsSection.CONTROLS -> CsControlsPage(bridge, coordinator, cs, connected)
        // -- WP AN12 -- the groups and macros pages edit, so they need the bridge and the connection
        CsSection.GROUPS -> CsGroupsPage(bridge, coordinator, cs, connected)
        CsSection.MACROS -> CsMacrosPage(bridge, coordinator, cs, connected)
        CsSection.AUX -> CsAuxPage(bridge, coordinator, cs, connected)
    }
}
