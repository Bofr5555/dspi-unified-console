# Object names: QP6 (matrix mixer)

Section `qp6-matrix` (order 60, `smoke/SmokeMatrix.cpp`). The window opens from
Tools > Matrix Mixer... (`tools.matrix`). `<i>` is a source row (an input, or an
upmix row), `<o>` an output, both from 0.

| Name | What |
|---|---|
| `MatrixWindow` | the window |
| `Matrix.scroll`, `Matrix.table` | the scroll view and the table inside it |
| `Matrix.header.<o>` | an output's column header; `shownName` is its label |
| `Matrix.header.<o>.menu` | the header's chevron (opens the column menu) |
| `Matrix.header.<o>.rename` | the rename field that replaces the label |
| `Matrix.columnMenu` | the column menu |
| `Matrix.menu.identify`, `Matrix.menu.rename`, `Matrix.menu.copy`, `Matrix.menu.paste` | its rows |
| `Matrix.routing` | the ROUTING band |
| `Matrix.direct`, `Matrix.clear` | Direct 1:1 and Clear (8-channel layout only) |
| `Matrix.row.<i>` | a source row's label; `shownName` is its name |
| `Matrix.row.<i>.trim` | the input's trim field (8-channel layout only) |
| `Matrix.cell.<i>.<o>` | a crosspoint (MatrixCell) |
| `Matrix.cell.<i>.<o>.connect`, `.gain`, `.invert` | its connect dot, gain field and INV |
| `Matrix.enableRow` | the ENABLE row |
| `Matrix.enable.<o>`, `Matrix.gain.<o>`, `Matrix.delay.<o>`, `Matrix.mute.<o>` | the OUTPUT rows' controls |
| `Matrix.enableQuestion` | the PDM / Core 1 question before an enable |

Screenshots: `window-matrix.png`, `window-matrix-cleared.png`, `window-matrix-readonly.png`.
