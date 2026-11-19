# Object names: PGQT (the protocol generations: device info, raw by id, the sender's page)

Section: `pgqt-generations` (order 215, `smoke/SmokeGenerations.cpp`).

| Name | What |
|---|---|
| `About.device` | Settings > About's "Connected Device" group |
| `About.raw` | its Raw Request… button |
| `DeviceInfoRows` | the device's protocol rows (About, the sender's page) |
| `DeviceInfoRows.<key>` (`.value`) | one row: `board`, `generation`, `protocol`, `pages`, `extensions` |
| `DeviceInfoRows.note.<n>`, `DeviceInfoRows.none` | a note under the rows; the line shown before a record is read |
| `RawRequestDialog` | the raw request dialog |
| `RawRequestDialog.get`, `.set`, `.id`, `.value`, `.last`, `.result`, `.send`, `.close` | its controls (`.last`: the length of a GET or the payload of a SET) |
| `MainWindow.sender` | the loader of the alink sender's page, in place of the DSP screens |
| `SenderPage`, `SenderPage.alink`, `SenderPage.alinkProblem`, `SenderPage.refresh`, `SenderPage.update` | the sender's page |
| `FirmwareWindow.imageNote`, `FirmwareWindow.generationChange`, `FirmwareWindow.guardGeneration` | the update window: no image for this board, and the generation change (in the window and in the question) |
