# WP P2QT: the hub's snapshots, Events, alarm notices, Identify

Section (`smoke/SmokeHubPages.cpp`): `p2qt-hub` (order 162; the hub half
against `uconsole --demo hub --open`, DSPI_SMOKE_UCONSOLE). Screenshots, each
dark and light, in `<out>/p2qt/`: `events-local`, `snapshots-hub`,
`events-hub`, `alarm-banner`.

| Name | What |
|---|---|
| `Menu.device.events`, `Menu.device.identify` | Device > Events… and Device > Identify |
| `EventsWindow` | Device > Events… (a `ToolWindow`) |
| `Events.source`, `Events.source.local`, `Events.source.hub` | "On This Console" / "On the Hub" (only on a hub with the `events` cap) |
| `Events.reload` | Refresh |
| `Events.problem` | why the hub's history could not be read |
| `Events.list`, `Events.row.<n>`, `Events.by.<n>` | the records (newest first), a row, and who caused it |
| `SnapshotsWindow.onHub.<n>`, `SnapshotsWindow.localCopy.<n>` | a local row's "On the hub" and "Local copy" pills |
| `SnapshotsWindow.saveToHub` | Save to the Hub (the selected local snapshot) |
| `SnapshotsWindow.hubProblem` | why the hub's list is missing |
| `SnapshotsWindow.hubList`, `SnapshotsWindow.hubRow.<n>` | "On the Hub" and a row (`hubSelectedId` on the window) |
| `SnapshotsWindow.hubRecall`, `.hubCopy`, `.hubDelete`, `.hubDeleteQuestion` | Recall from the Hub, Copy Here, Delete from the Hub… and its question |
| `NoticeBanner.acknowledge` | a hub alarm's Acknowledge |
