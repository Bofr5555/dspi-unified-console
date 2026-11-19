# WP P5QUI: Tools > Measure… (measurement, live view, auto-tune, documents)

Sections (`smoke/SmokeMeasure.cpp`): `p5qui-measure` (order 210), `p5qui-live`
(211), `p5qui-tune` (212), on the demo DSPi with its simulated microphone.
Screenshots in `<out>/p5qui/`: `results`, `live`, `tune-proposal`,
`tune-applied`, `tune-keep-question`.

| Name | What |
|---|---|
| `MeasureWindow` | Tools > Measure… (a `ToolWindow`, action `tools.measure`) |
| `Measure.stopBar`, `Measure.stop` | the Stop bar, shown while anything plays or listens, and its Stop |
| `Measure.error`, `Measure.said` | the last failure, the last thing said |
| `Measure.tabs`, `Measure.tab.<measure, results, live, tune, documents>` | the tabs |
| `MeasureSetup.mic`, `.calibration`, `.placement`, `.count`, `.coupler` | the microphone, its calibration, the placement |
| `MeasureSetup.output.<n>` | output n measured or not (its way, speaker and group beside it) |
| `MeasureSetup.sweep`, `.passes`, `.name` | the sweep length, the passes, the document's name |
| `MeasureSetup.preflight`, `.start`, `.next`, `.preflightLines` | Check, Measure, Next Position, the preflight in words |
| `MeasureSetup.phase`, `.progress`, `.issue`, `.timing` | the run's state in words, its progress, why it stopped, the timing verdict |
| `MeasureResults.name`, `.timing`, `.graph`, `.save`, `.saveHub`, `MeasureResults.output.<n>` | the results |
| `MeasureLive.start`, `.stop`, `.output`, `.mode`, `.hold`, `.state`, `.graph` | the live view |
| `MeasureLive.at`, `.cut`, `.fit`, `.undo`, `.proposal`, `.apply`, `.discard`, `.said` | the direct adjustments |
| `MeasureTune.stage`, `.error`, `.output.<n>`, `.propose`, `.writes`, `.withdrawn`, `.graph`, `.sumCheck`, `.sumCheckRun`, `.apply`, `.applyQuestion` | the proposal |
| `MeasureTune.guard`, `.verify`, `.verifyRun`, `.keep`, `.revert`, `.keepQuestion`, `.kept`, `.abStart`, `.abToggle` | the guard, Keep (and its second question after a verify that did not pass), Revert, A/B |
| `MeasureDocs.list`, `.reload`, `MeasureDocs.name.<i>`, `.open.<i>`, `.delete.<i>`, `.deleteQuestion` | the documents |
| `<graph>.canvas` | a `MeasureGraph`'s canvas; the graph's `drawnSeries` and `drawnKinds` say what it drew |
