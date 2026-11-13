# Object names: QP1 (sidebar and global controls)

Section: `qp1-sidebar` (order 10), `smoke/SmokeSidebar.cpp`.

| Name | What |
|---|---|
| `Sidebar` | the sidebar; `selectedChannel` mirrors `SidebarController.selectedChannel` |
| `Sidebar.list` | its channel list |
| `Sidebar.channel.<wire channel>` | a channel row; `shownName` (kept from the base) |
| `Sidebar.pill.<wire channel>` | the row's descriptor pill, the curve's show/hide toggle; `checked` |
| `Sidebar.meter.<wire channel>` | the row's level meter; `shownLevel` |
| `Sidebar.rename.<wire channel>` | the row's rename field (Alt+click or the menu) |
| `Sidebar.rowMenu.<wire channel>` | the row's context menu (a QObject child of the row, not in the window's tree) |
| `Sidebar.menu.identify.<ch>`, `.rename.<ch>`, `.copy.<ch>`, `.paste.<ch>` | its rows |
| `Sidebar.cpu0`, `Sidebar.cpu1` | the cores' loads; `shownName` ("12%" or "–") |
| `QuickStrip` | the quick strip |
| `QuickStrip.matrix`, `.crossfeed`, `.loudness`, `.leveller`, `.psybass`, `.stats`, `.settings`, `.bypass` | its buttons; `checked` is the active look |
| `GlobalControls` | preset, source and volume |
| `GlobalControls.preset` | the preset popup; `shownName` (with "*" while unsaved) |
| `GlobalControls.presetSave` | its save button |
| `GlobalControls.presetMenu` | the slot menu (right-click on the popup) |
| `GlobalControls.copyTo.<slot>`, `GlobalControls.presetClear`, `GlobalControls.presetClearAll` | slot menu rows |
| `GlobalControls.source` | the source popup; `shownName` |
| `GlobalControls.sourceMenu` | its rename menu (right-click) |
| `GlobalControls.volumeMode` | the "User Volume ▾" / "Master Volume ▾" heading; `shownName` |
| `GlobalControls.volumeMode.user`, `.master` | its two menu rows |
| `GlobalControls.volumeSlider` | the volume slider (position 0..1 on the mode's taper) |
| `GlobalControls.volumeReadout` | the dB readout; `shownName` |
| `GlobalControls.unsavedQuestion` | the unsaved-changes question (Save / Discard / Cancel) |
| `GlobalControls.clearQuestion`, `GlobalControls.clearAllQuestion` | the clear questions |
| `GlobalControls.presetRename`, `GlobalControls.sourceRename` | the rename dialogs; `.field`, `.accept` |
| `GlobalControls.failure` | a failed preset operation's alert |
| `DeviceStatus.button` | the device button; `shownName` (kept from the base) |
| `DeviceStatus.text` | the line's text: the device's name, or the status notice |

Screenshots: `window-main-sidebar.png`, `window-main-sidebar-menu.png`, `window-main-unsaved-question.png`,
`window-main-preset-menu.png`, `window-main-master-volume.png`.
