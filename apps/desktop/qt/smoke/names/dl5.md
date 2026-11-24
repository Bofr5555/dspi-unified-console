# WP DL5: Modern's shell in the design language

- `TopBar.preset`: the preset control (a button now, not a combo box): `shownName` (the active slot's title, with its
  "*"), `popup` (its `PresetPopover`). Up and Down load the next slot; Enter or Space open the popover on the active
  slot's row; a right-click opens the active slot's menu. `TopBar.presetMenu` is gone.
- `PresetPopover`: `fromKeyboard`; its `contentItem` is the list `PresetPopover.list` (rows by `itemAtIndex(slot)`);
  rows `PresetPopover.slot.<n>` (ListRow) with `.recall`, `.save`, `.copy` (an empty slot) and `.menu`;
  `PresetPopover.startup` the line under them.
- `DemoBanner.edge` (and `<banner>.edge` for every Banner in the status strip): the strip's dress (the 3 px edge, the
  cover over the banner's line). `AiWorkBanner.edge`: the AI strip's edge.
- `AiWorkBanner.details`: Modern's button to the AI popover (grants and Revoke); `AiWorkBanner.revokeGrant` is
  Classic's only.
- `NavRail.<section>.mark`: the current item's accent mark.
- `NotificationDrawer.empty`: the drawer's EmptyState.
- `DeviceSwitcher.hubs`: the row to Devices › Hubs. The switcher's rows keep `DevicesPopup.open.<id>`,
  `DevicesPopup.close.<id>`, `DevicesPopup.usb.<serial>`, `DevicesPopup.demo` and `DevicesPopup.openHeading`, now
  ListRows (`selected` is the open one; `clicked(int)`); it has no `DevicesPopup.done`.

Section `dl5-shell` (order 988, main run, smoke/SmokeDl5.cpp, up to 150 s): the preset popover (rows, keyboard,
Copy Here, Recall, a row's menu), the strip's dress, the rail's mark, the drawer's empty state, the switcher (no Done,
list rows, the hubs' row), and the AI strip with a fake agent's grant (its line after the job, Revoke in the AI
popover). Screenshots `dl5-*.png`.
