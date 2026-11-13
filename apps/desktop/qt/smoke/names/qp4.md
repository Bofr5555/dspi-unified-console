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
| (the band chip) | replaced by QPI2's band bar under the graph, `PeqBar` (`names/qpi2.md`) |
| `PeqCard`, `PeqCard.back`, `PeqCard.name`, `PeqCard.shape.<shape>`, `PeqCard.order.<order>` | the Ctrl-click shape card |

Screenshots: `window-main-peq.png`, `window-main-peq-card.png`,
`window-main-peq-shapes.png`, `window-main-peq-menu.png`.
