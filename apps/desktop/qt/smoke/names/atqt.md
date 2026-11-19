# WP ATQT: Tools > Measure… "Several units", and the S/PDIF units in Window > System

Sections (`smoke/SmokeMeasureUnits.cpp`): `atqt-units` (order 216) and
`atqt-spdif` (217), against `uconsole --demo hub --alink --with-measure-units`
(DSPI_SMOKE_UCONSOLE). Screenshots in `<out>/atqt/`: `units-checked`,
`units-settling`, `units-runs`, `units-applied`, `units-keep-question`,
`units-reverted-by-hub`, `system-spdif`.

| Name | What |
|---|---|
| `Measure.tab.units` | the "Several units" tab (not for a connection that may not play) |
| `MeasureUnits` | the tab |
| `MeasureUnits.unavailable` | "needs the DSPi hub" (`key`: `measure.system.needs_hub`) |
| `MeasureUnits.runBar`, `.phase`, `.stop` | the run's bar: the phase in words (the settle wait too) and Stop |
| `MeasureUnits.placement`, `.placementProblem` | where the microphone is, and why the choice stops every run (`key`: `measure_system_placement`) |
| `MeasureUnits.sim`, `.simOn` | (review AT-D2) the demo hub's simulated microphone: the choice (only where the hub offers it) and its label in the run's bar |
| `MeasureUnits.unit.<serial>`, `.choose.<serial>`, `.unitError.<serial>` | a unit of the hub, chosen or not, its connection's failure |
| `MeasureUnits.output.<serial>.<n>` | output n of a chosen unit (its role beside it) |
| `MeasureUnits.sweep`, `.passes`, `.anchor`, `.name` | the runs' settings |
| `MeasureUnits.check`, `.start`, `.blocker` | Check every unit, Measure the units, what keeps them disabled |
| `MeasureUnits.preflight.<key>.<serial>` | a line of the check (`ready` when nothing blocks) |
| `MeasureUnits.runs` | "needs N runs" (`key`: `measure_system_runs`) |
| `MeasureUnits.error`, `.said`, `.issue` | the last refusal (`key`: its notice), what was said, why a run stopped (`key`) |
| `MeasureUnits.result`, `.timing`, `.graph` | the result |
| `MeasureUnits.proposal`, `.proposalNotice`, `.guardNote` | the proposal, what stands in its way, the hub's guard in words |
| `MeasureUnits.sumCheck`, `.apply`, `.applyWhy` | the summed check, Apply (Admin only), why not (`key`) |
| `MeasureUnits.applied`, `.appliedText`, `.guard`, `.verify`, `.keep`, `.revert` | after the apply: where it stands, the guard's countdown, Verify, Keep, Revert (Admin) |
| `MeasureUnits.keepQuestion`, `.keepAnyway`, `.keepNotNow` | the second question after a verify that did not pass |
| `MeasureUnits.outcome` | kept, or reverted (the hub's words when its guard put it back) |
| `SystemWindow.spdif` | the S/PDIF units in "Clock and audio links" |
| `SystemWindow.spdifUnit.<id>` | an S/PDIF unit's row (`shownName`: name, role, state) |
| `SystemWindow.spdifSource.<id>` | a follower's source (Admin) |
| `SystemWindow.spdifSlot.<id>.<n>`, `.spdifD.<id>` | a source's time-base slot n (outputs 2n + 1 and 2n + 2) and its D (Admin) |
| `SystemWindow.spdifState.<id>`, `.spdifNoTimeBase.<id>` | its state, and "this S/PDIF source does not carry a DSPi time base" |
| `SystemWindow.spdifOff.<id>`, `.spdifAdd`, `.spdifAddUnit`, `.spdifAddAs`, `.spdifAddButton` | off S/PDIF, put a unit on S/PDIF (Admin) |
