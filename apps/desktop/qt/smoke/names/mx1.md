# Object names: MX1 (the matrix mixer redrawn)

Smoke section: `mx1-matrix` (order 897, `smoke/SmokeMx1.cpp`). Its screenshots are `mx1-page.png`, `mx1-page-crosspoint.png`,
`mx1-page-output.png`, `mx1-page-input.png`, `mx1-page-dense.png`, `mx1-window.png` and `mx1-window-dense.png`.

Kept from QP6 / LAY5a: `MatrixPanel`, `Matrix.table`, `Matrix.scroll`, `Matrix.header.<output>` (`shownName`) with `.select`, `.menu`,
`.rename`; `Matrix.row.<row>` (`shownName`) with `.select`, `.trim`; `Matrix.cell.<row>.<output>` with `.connect` (the tile's button),
`.gain` (the number in the tile, a field while typed in), `.invert` (the ø), `.touched`; `Matrix.enable.<output>` and
`Matrix.mute.<output>` (now in the column header); `Matrix.gain.<output>`, `Matrix.delay.<output>` (the footer); `Matrix.columnMenu`,
`Matrix.menu.*`, `Matrix.enableQuestion`; `RoutingInspector` (`.sheet`, `.none`, `.equaliser`), `RoutingTools`, `.inspector`.

New:
- `Matrix.grid` (the keyboard's grid, a FocusScope), `Matrix.corner`, `Matrix.tail.<row>` (`text`), `Matrix.fedBy.<output>`
  (`sources`), `Matrix.pairMark.<even row>` (Link L/R pairs), `Matrix.empty` (no device, window only).
- `Matrix.cellMenu` with `.connect`, `.gain`, `.reset`, `.invert`, `.identify`.
- The tools (`MatrixTools`), `<prefix>.link`, `.direct`, `.clear`, `.dense`, `.clearQuestion`: prefix `Matrix` in the window
  (`Matrix.tools`), `RoutingTools` on the page (`RoutingTools.matrix`). `Matrix.direct` and `Matrix.clear` keep their names.
- The inspector: `RoutingInspector.summary`; a crosspoint: `.title`, `.crosspoint`, `.connected`, `.gainSlider`, `.gain`, `.invert`,
  `.mirror`, `.alsoFeeds` (`.alsoFeeds.chips.chip.<k>`), `.outputLine`; an output: `.output`, `.enabled`, `.outputGain`,
  `.outputDelay`, `.mute`, `.polarity`, `.fedBy` (`.fedBy.chips.chip.<row>`); an input: `.input`, `.trim`.
