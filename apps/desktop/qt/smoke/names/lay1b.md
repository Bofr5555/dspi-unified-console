# Object names: LAY1b (measure, analyse, library, system, devices and settings windows as panels)

Smoke section: `lay1b-panels` (order 885, `smoke/SmokeLay1b.cpp`).

Each window below is a `PanelWindow` that keeps the window's name (the type
`Registrar::Window` registered) and holds a panel named as its file. Every item
inside keeps the name it had in the window (see the owning package's file), so
a name is found the same way in the window and on a Modern page. A smoke test
that read a property of the old window reads it from the panel now: the
window's `content`.

| Action | Window (unchanged) | Panel | Names inside |
|---|---|---|---|
| `tools.measure` | `MeasureWindow` | `MeasurePanel` | `Measure.*` (p5qui.md) |
| `tools.signalGenerator` | `SiggenWindow` | `SiggenPanel` | `SiggenWindow.*` (qp9.md) |
| `tools.spectrum` | `SpectrumWindow` | `SpectrumPanel` | `SpectrumWindow.*` (qp9.md); `mode`, `visibleChannels`, `rendering` |
| `device.snapshots` | `SnapshotsWindow` | `SnapshotsPanel` | `SnapshotsWindow.*` (p1qt1.md, p2qt.md) |
| `edit.history` | `HistoryWindow` | `HistoryPanel` | `HistoryWindow.*` (qp18.md) |
| `autoeq.browse` | `AutoEqWindow` | `AutoEqPanel` | `AutoEqWindow.*` (qp15.md); `selectedId`, `applySelected()` |
| `device.events` | `EventsWindow` | `EventsPanel` | `Events.*` (p2qt.md) |
| `device.firmwareUpdate` | `FirmwareWindow` | `FirmwarePanel` | `FirmwareWindow.*` (qp16.md); `minimumHeight` |
| `help.whatsNew` | `WhatsNewWindow` | `WhatsNewPanel` | `WhatsNew.*` (qp17.md) |
| `edit.settings` | `SettingsWindow` | `SettingsPanel` | `Settings.*` (qp10.md); `query`, `rows`, `windowFont` |
