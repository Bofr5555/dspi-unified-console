# Object names: QP9 (Analysis)

Smoke sections (`smoke/SmokeAnalysis.cpp`): `qp9-siggen` (90), `qp9-monitor` (91), `qp9-stats` (92),
`qp9-spectrum` (93), `qp9-follow` (94, needs `DSPI_SMOKE_FAKEHUB`).

Tool windows, opened by their menu rows (`Registrar::Window`):

| Window | Menu row | Size (min) |
|---|---|---|
| `SiggenWindow` | `tools.signalGenerator` | 780×500 (740×500) |
| `SpectrumWindow` | `tools.spectrum` | 760×560 (620×420) |
| `StatsWindow` | `tools.stats` | 960×660 (900×480) |
| `MonitorWindow` | `tools.interruptMonitor` | 900×480 (720×320) |

A control whose text a check reads exposes it as `shownName`.

## Signal generator

| Name | What |
|---|---|
| `SiggenWindow.state` | the state pill (`shownName`: "Running", "Idle", ...) |
| `SiggenWindow.unsupported` | the notice on firmware without a generator |
| `SiggenWindow.signal.<id>` | a signal tile, ids 0..14 |
| `SiggenWindow.blurb` | the chosen signal's line under the tiles |
| `SiggenWindow.all`, `SiggenWindow.none` | All, None |
| `SiggenWindow.output.<n>` | an output chip (`shownName` ends in " ø" while inverted) |
| `SiggenWindow.levelField`, `SiggenWindow.level` | the level's field and slider |
| `SiggenWindow.isp` | the ISP test's two patterns |
| `SiggenWindow.param.<i>` | parameter `i`'s field (0..3) |
| `SiggenWindow.presetSmpte`, `SiggenWindow.presetCcif` | the tone pair's presets |
| `SiggenWindow.duration`, `SiggenWindow.repeat`, `SiggenWindow.gap` | the timing fields |
| `SiggenWindow.raw`, `SiggenWindow.decorrelate`, `SiggenWindow.walk` | the option switches |
| `SiggenWindow.title`, `SiggenWindow.detail` | the transport's two lines (`shownName`) |
| `SiggenWindow.start`, `SiggenWindow.stop`, `SiggenWindow.stopNow` | Start, the faded Stop, the hard stop |

## Spectrum analyser

| Name | What |
|---|---|
| `GraphHost.spectrum`, `GraphWindow.spectrum` | the overlay in the graph's RtaOverlay slot (main window, pop-out); `.surface` its `RtaSurface` (`drawnCount`) |
| `GraphHost.options.rta`, `GraphWindow.options.rta` | the analyser's part of the graph options (RtaOptions slot) |
| `<options>.unavailable` | the line without an analyser or without a device |
| `<options>.following`, `<options>.takeControl` | the following row and its button |
| `<options>.side`, `<options>.inputs`, `<options>.outputs` | the side picker |
| `<options>.channel.<n>` | a channel chip |
| `<options>.summary`, `<options>.clear` | "N channels" and Clear |
| `<options>.graph`, `<options>.bars` | the FFT Graph and RTA Bars switches |
| `SpectrumStrip.dashboard`, `SpectrumStrip.page` | the bar strip on the overview and on a channel page |
| `SpectrumStrip.<place>.bars.<i>` | a cell's `RtaBars` (`drawnBars`) |
| `SpectrumStrip.<place>.options`, `.popover`, `.columns.<n>`, `.openWindow` | the strip's gear, its popover, the layout tiles, Open in Window |
| `SpectrumStrip.<place>.resize` | the bottom edge that drags the height |
| `SpectrumWindow.unavailable`, `.hidden`, `.allHidden` | the window's three notices |
| `SpectrumWindow.page` | the page's title (`shownName`) |
| `SpectrumWindow.legend.<n>` | a legend entry (hides the channel in the window) |
| `SpectrumWindow.mode` | Curves / Bars / Both (the window's `mode`: 0, 1, 2) |
| `SpectrumWindow.curves`, `SpectrumWindow.bars` | the curve view (`RtaSurface`) and the bars' grid |
| `SpectrumWindow.status` | the status bar; `.running`, `.telemetry`, `.rejected`, `.following`, `.takeControl`, `.lowBands`, `.range` its parts |

## Stats for nerbs

| Name | What |
|---|---|
| `StatsWindow.platform`, `.firmware`, `.serial`, `.reconnects` | device rows (`shownName`) |
| `StatsWindow.clock`, `.coreVoltage`, `.sampleRate`, `.temperature` | system rows |
| `StatsWindow.usbRing`, `StatsWindow.bufferPool` | the audio output counters |
| `StatsWindow.ringHeading`, `StatsWindow.ring.<i>` | output ring health (hidden when the device does not report it) |
| `StatsWindow.starvation` | the starvation total (`shownName`) |
| `StatsWindow.reset` | Reset Watermarks |
| `StatsWindow.fill.<i>` | a buffer fill row (`StatsFillRow`) |
| `StatsWindow.spdifInput`, `.lg`, `.adat`, `.i2s` | the interface sections (shown when the device has them) |
| `StatsWindow.spdif.<i>` | an S/PDIF input row |
| `StatsWindow.clockDiagnostics` | clock diagnostics (only on firmware built with them) |
| `StatsWindow.usbErrors` | USB errors (shown once the device has counted one) |
| `StatsWindow.connection` | the footer's Connected / Disconnected |

## Interrupt Monitor

| Name | What |
|---|---|
| `MonitorWindow.pause`, `.clear`, `.copy` | Pause/Resume, Clear, Copy |
| `MonitorWindow.status` | "Inactive", "Paused", "Listening" (`shownName`) |
| `MonitorWindow.count` | "N events" (`shownName`) |
| `MonitorWindow.log` | the list of lines |

Screenshots: `window-siggen.png`, `window-monitor.png`, `window-stats.png`, `window-spectrum.png`,
`window-spectrum-following.png`, `window-main-spectrum.png`, `window-main-spectrum-options.png`.
