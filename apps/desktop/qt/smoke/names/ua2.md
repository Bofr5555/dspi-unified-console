# Object names: UA2 (A2, QT-MEASUREBAR: Measure and Auto-tune's action bar)

Smoke section: `ua2-measurebar` (order 995), `smoke/SmokeUa2.cpp`.

| Name | What |
|---|---|
| `Measure.actionBar` | the action bar above Measure's tabs (`MeasureActionBar.qml`): the run, a proposal (Auto-tune tab), the trial of an applied tune |
| `Measure.runningText` | its line (was the Stop bar's): what runs, "Proposed: …", "Trying the auto-tune · goes back by itself in 3:02" |
| `Measure.actionStep` | its second line: the step under way, the verify's verdict |
| `Measure.progress` | its progress (a run's, the summed check's, the verify's) |
| `Measure.stopBar` | its Stop part (Stop, Next Position): shown while something plays or listens |
| `Measure.stop` | Stop (moved into the bar) |
| `MeasureSetup.next` | Next Position (moved from the Measure tab into the bar) |
| `MeasureTune.forget`, `MeasureTune.apply`, `MeasureTune.sumCheckRun` | a proposal's actions (moved into the bar) |
| `MeasureTune.applyQuestion` | Apply…'s question (now the bar's) |
| `MeasureTune.verifyRun`, `MeasureTune.revert`, `MeasureTune.keep` | the trial's actions (moved into the bar) |
| `Measure.refusal.<i>` | a refused Measure's problem `i` (`MeasureRefusalRow.qml`); `.notice` its notice row (Modern), `.notice.action` its fix |
| `MeasureTune.page` | the Auto-tune tab's ScrollView |
| `MeasureTune.details` | the proposal's folded details (held to, withdrawn, notes) |
| `MeasureTune.stageLine` | Modern: the stage in words in the Applied section |
