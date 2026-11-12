# QP4: on-graph PEQ editor

Section `qp4-peq` (order 35); checks start with `qp4:`.

| Name | What |
|---|---|
| `PeqEditor` | the editor in the main window's graph (GraphController.PeqOverlay slot); `GraphWindow.peq` in the pop-out |
| `PeqEditor.overlay` | the PeqOverlay item: `editedChannel`, `selectedBands`, `chipBand`, `cardOpen`, `shapePage`, `ghostShown`, `animating`; `dotPoint(band)`, `settle()` (`GraphWindow.peq.overlay` in the pop-out) |
| `PeqEditor.frequencyReadout`, `PeqEditor.levelReadout` | the readouts on empty graph; `shownName` |
| `PeqEditor.bandMenu`, `.bandMenu.title`, `.bandMenu.bypass`, `.bandMenu.invert`, `.bandMenu.delete` | the band menu |
| `PeqEditor.slopeMenu`, `PeqEditor.slopeMenu.<order>` | its Slope / Order submenu (1 = 6 dB/oct or 1st order) |
| `PeqEditor.graphMenu`, `.graphMenu.selectAll`, `.graphMenu.deselect`, `.graphMenu.delete` | the graph menu |
| `PeqChip` | the band chip |
| `PeqChip.shape`, `PeqChip.bypass` | its shape button (opens the shape page) and power button |
| `PeqChip.row.<field>` | a value row, field 0 Freq, 1 Gain (or fp), 2 Width / Order / Slope; `shownName` is number and unit |
| `PeqChip.value.<field>`, `PeqChip.input.<field>` | the row's drag area and its text field while typing |
| `PeqChip.shapes.back`, `.shapes.name`, `.shapes.shape.<shape>`, `.shapes.order.<order>` | the shape page (shape 0 Bell, 1 Low Shelf, 2 Low Cut, 3 High Shelf, 4 High Cut, 5 Notch, 6 All Pass) |
| `PeqCard`, `PeqCard.back`, `PeqCard.name`, `PeqCard.shape.<shape>`, `PeqCard.order.<order>` | the Ctrl-click shape card |

Screenshots: `window-main-peq.png`, `window-main-peq-chip.png`, `window-main-peq-card.png`,
`window-main-peq-shapes.png`, `window-main-peq-menu.png`.
