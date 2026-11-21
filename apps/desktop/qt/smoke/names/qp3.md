# Object names: QP3 (response graph)

Section `qp3-graph` (order 30), in `smoke/SmokeGraph.cpp`.

| Name | What |
|---|---|
| `GraphHost` | the graph at the top of the right pane (replaces the base's `RightPane.graph` placeholder) |
| `GraphHost.closePopOut` | the header's button that closes the pop-out |
| `GraphHost.plot` | the main window's graph (`GraphPlot`); `optionsOpen` |
| `GraphHost.plot.grid`, `.curves`, `.labels` | its three `BodePlot` layers; `.curves` has `drawnChannels`, `drawnLines`, `drawnPhase`, `animating` |
| `GraphHost.plot.rtaSlot`, `GraphHost.plot.peqSlot` | the loaders of the RTA overlay (QP9) and the PEQ overlay (QP4) |
| `GraphHost.peqDock` | the loader of the PEQ band chip under the graph (QP4) |
| `GraphHost.resize` | the drag strip under the graph |
| `GraphHost.options` | the gear; `open` |
| `GraphHost.options.popover` | its popover; `setup` (the setup page shows) |
| `GraphHost.options.rtaSlot` | the loader of the analyser's options (QP9) |
| `GraphHost.options.setup`, `GraphHost.options.popOut` | the main page's Graph Setup and Pop Out Graph rows (`activated()`) |
| `GraphHost.options.<control>` | the setup page: `back`, `resetScale`, `minFreq`, `maxFreq`, `range`, `center`, `freqGrid`, `freqLabels`, `dbGrid`, `dbLabels`, `freqReadout`, `levelReadout`, `gridOpacity`, `lineWidth`, `glow`, `phase`, `unwrap`, `follow` (pop-out only) |
| `GraphWindow` | the pop-out "Filter Response" window |
| `GraphWindow.plot`, `GraphWindow.plot.*`, `GraphWindow.options*` | the same names in the pop-out |
| `GraphLegend` | the pop-out's legend |
| `GraphLegend.channel.<wire channel>` | a legend pill |

Screenshots: `window-main-graph.png`, `window-main-graph-band.png` (a +6 dB band
at 1 kHz on input 1), `window-graph-options.png`, `window-graph-setup.png`,
`window-graph.png` (the pop-out), `window-graph-setup-popout.png`.
