# QPI2: wave-2 integration

Checks start with `qpi2:`; they live in the sections they extend (`qp4-peq`,
`qp6-matrix`, `qp14-*`).

| Name | What |
|---|---|
| `PeqBar` | the band bar docked under the main window's graph (GraphController.PeqDock slot, `PeqBandBar.qml`); `GraphWindow.peqBar` under the pop-out's graph |
| `PeqBar.placeholder` | its line while no band is selected |
| `PeqBar.channelTag`, `PeqBar.channel`, `PeqBar.band` | the edited channel's colour tag and name, "Band N" |
| `PeqBar.shape` | the shape button: glyph and code; opens the shape chooser |
| `PeqBar.shapesPopup`, `PeqBar.shapes.back`, `.shapes.name`, `.shapes.shape.<shape>`, `.shapes.order.<order>` | the shape chooser (shape 0 Bell, 1 Low Shelf, 2 Low Cut, 3 High Shelf, 4 High Cut, 5 Notch, 6 All Pass) |
| `PeqBar.input.<field>` | the value fields: 0 Freq, 1 Gain, 2 Q (typed, or scrolled) |
| `PeqBar.fixed.<field>` | a value the shape fixes ("6 dB/oct", "1st"), as text |
| `PeqBar.bypass`, `PeqBar.invert`, `PeqBar.delete` | the band's buttons; they act on the selection, as the band menu does |
| `FilesHost.deviceQuestion` | Commit Parameters, Revert to Saved and Factory Reset's question (FilesController.deviceQuestion) |

Screenshots: `window-main-peq-bar.png`, `window-main-peq-bar-empty.png`, and the pop-out's
`window-graph-peq.png` now with its bar.
