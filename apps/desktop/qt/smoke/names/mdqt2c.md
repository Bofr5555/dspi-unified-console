# WP MDQT2c: jobs over several open devices

Smoke sections: `mdqt2c-jobs` (order 966) and `mdqt2c-controllers` (967), `smoke/SmokeJobs.cpp`.

| Name | What |
|---|---|
| `Measure.runningText` | the Stop bar's words; with several devices open, "Running on <device>. ..." |
| `Measure.otherRun` | what runs on another open device, one bar each |
| `Measure.otherRun.show`, `Measure.otherRun.stop` | its Show and Stop buttons |
| `MeasureUnits.held` | the Several units tab shows another device's measurement, still running or in its guard |
