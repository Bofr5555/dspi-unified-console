# Object names: UFW (UXP A8, A10, C8: finding things, one word for one thing)

Smoke section: `ufw-words` (order 992), `smoke/SmokeUfw.cpp`; checks in `lay3b-menus`, `lay6-palette` and
`qp17-onboarding` are marked `-- WP UFW --`.

- `CommandPalette.row.help:<id>`: a "Help: …" entry (`connect`, `questions`, `undo`, `refused`, `keep`, `usb`,
  `words`); its entry's `kind` is `help` and its `arg` the manual's https address with the section's anchor.
- `AppMenu.help.manual`, `AppMenu.help.shortcuts`: Help › User Manual and Keyboard Shortcuts (the latter once UXP
  A5's dialog handles `help.shortcuts`).
- `DeviceTabs.menu.<id>.device.factoryReset`: Factory Reset in a device tab's menu (MenuTable's place
  `device.reset`, under a line); Save ▾ (`TopBar.save.menu`) has none.
- `WhatsNew.part.app`, `WhatsNew.part.firmware`: the headings of the app's own notes (the app's name) and of the
  firmware's ("DSPi firmware"), shown when both parts are there.
- `WhatsNew.action.<page>`: a button under the app's notes that opens Settings › `<page>` (`appearance`, `ai`).
