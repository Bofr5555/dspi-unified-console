# WP P1QT1: Snapshots, A/B, import preview, safe apply, offline files

Sections (`smoke/SmokeSnapshots.cpp`): `p1qt1-snapshots` (order 155),
`p1qt1-import` (156), `p1qt1-offline` (157). Screenshots, each dark and light,
in `<out>/p1qt1/`: `snapshots-ab`, `snapshots-guarded`, `safe-apply-bar`,
`import-preview`, `offline-file`, `template-picker`.

| Name | What |
|---|---|
| `SnapshotsWindow` | Device > Snapshots… (a `ToolWindow`; `selectedId`) |
| `SnapshotsWindow.captureName`, `.captureNote`, `.capture` | the new snapshot's name, note and Capture |
| `SnapshotsWindow.list`, `SnapshotsWindow.row.<n>` | the list (newest first) and a row |
| `SnapshotsWindow.name`, `SnapshotsWindow.note` | the selected snapshot's name and note (rename, note on Enter or leaving the field) |
| `SnapshotsWindow.recall`, `.compare`, `.export`, `.import`, `.delete` | the buttons under the list |
| `SnapshotsWindow.deleteQuestion` | Delete's destructive question |
| `SnapshotsWindow.rollback` | "Recall with rollback" (fixed on over DSPi Link to a hub with the guard) |
| `SnapshotsWindow.abA`, `.abB`, `.abStart` | the A/B pickers and Start |
| `SnapshotsWindow.abSideA`, `.abSideB`, `.abToggle`, `.abKeepA`, `.abKeepB` | the running compare's buttons |
| `SnapshotsWindow.abDiff` | what differs from A to B |
| `SafeApplyBanner` | the countdown bar of a guarded apply (a `Banner`) |
| `SafeApplyBanner.countdown`, `.keep`, `.revert` | the time left, Keep and Revert |
| `ImportConfig.<block>` | a block's box in the import dialog: `eq`, `crossover`, `routing`, `outputs`, `limiters`, `names`, `effects`, `volumes`, `hardware` (in the dialog's `contentItem`) |
| `ImportConfig.summary`, `ImportConfig.rows` | how many values change, and the list |
| `ImportConfig.rollback` | "Apply with rollback" |
| `TemplatePicker` | File > New from Template… (`templates`, `chosen`) |
| `TemplatePicker.list`, `.open`, `.cancel` | its list and buttons |
| `FileBanner` | the open file's banner |
| `FileBanner.save`, `FileBanner.saveTemplate` | Save File, Save as Template… |
| `DeviceStatus.file` | the file icon on the device button ("File: <name>") |
