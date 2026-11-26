# Object names: UA3 (save state per device, UXP A3)

Smoke section: `ua3-savestate` (order 899), `smoke/SmokeUa3.cpp`. Screenshots `ua3-<what>-dark.png` and `ua3-<what>-light.png`.

- `<device tabs>.dirty.<device id>`: the unsaved dot after a tab's name (`<device tabs>` is `DeviceTabs` in the top bar,
  `DeviceRow` for the row of several, `DeviceRail` in the rail). A tab's `dirty`, `unsaved` (count), `unsavedText` and
  `elided` properties; its `detail` starts with "N unsaved" while the device has unsaved changes.
- `<device tabs>.more.<device id>`: the active tab's ▾, which opens `<device tabs>.menu.<device id>`.
- `TopBar.save.menu.saveAll`: Save All, a lead row of Save ▾ (with two or more devices open); its question
  `TopBar.saveAllQuestion`. `TopBar.save`'s `countToolTip`.
- `TopBar.ab` shows only while a compare runs; its `toolTip` property, and `TopBar.ab.tip`.
- `DevicesSection.device.factoryReset`: Factory Reset on Devices › This Computer.
