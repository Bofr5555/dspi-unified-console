# WP DL8: Equaliser polish (Modern design language)

Checked in section `lay4-equaliser` (smoke/SmokeLay4.cpp, `dl8Part`, `dl8CrossoverPart`):

- `EqualiserTools.files`: the "Files ▾" button in the Equaliser's tab row; its menu `EqualiserTools.files.menu`, rows
  `EqualiserTools.files.menu.file.importFilters` and `EqualiserTools.files.menu.file.exportFilters`.
  `EqualiserTools.autoeq.browse` (LAY4's) now shows only while an input is selected; `EqualiserTools.file.importFilters`
  and `EqualiserTools.file.exportFilters` are gone.
- `EqualiserCrossover.plot` is now a GraphFrame: `.plot.plot` (its plot area), `.plot.labels` (the axis labels);
  `EqualiserCrossover.grid` (BodePlot's grid layer).
- `EqualiserCrossover.legend`: the help line under the table; `EqualiserCrossover.bandsHeading`: the section label
  over `EqualiserCrossover.bands`, both shown only for a family other than Linkwitz-Riley or a second high or low pass.
- `EqualiserCrossover.unsupported`: the EmptyState without crossovers.
