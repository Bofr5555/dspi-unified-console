# Object names: LAY7 (both layouts)

Smoke sections (`smoke/SmokeLay7.cpp`): `lay7-contrast` (order 901, every text on Modern's pages against what it is drawn on, in both tones and
schemes) and `lay7-devices` (968, the layouts switched with three devices open, the quit question, menus and the palette that follow, a restored
window size). Both layouts are also in `qp19-gallery`, `qp18-access` and `qp18-pseudo`, and in the launches `demo-entry-modern/`,
`tray-hidden-modern/` and `hw-dry-modern/`.

- `SmokeDriver::item` and `object` find one that shows first, then any; `object` also finds the popups of a page's items.
- `SmokeDriver::switchLayout`, `modernTabs`, `goTo`, `fits`.
- `FilesHost.busyQuestion`: quitting while a firmware update or a calibration runs on a device.
