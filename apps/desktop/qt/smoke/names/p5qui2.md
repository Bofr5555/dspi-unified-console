# WP P5QUI2: Tools > Measure… level with the web (review P5-D D2)

Sections (`smoke/SmokeMeasure.cpp`): `p5qui2-races` (order 213: F5, a Stop or a
closed window withdraws a start on its way), `p5qui2-passive` (214: the
passive microphone RTA in the client read-only mode, a sine through
AudioCapture's seam), `p5qui2-viewer` (215: F6, a fake hub granting Viewer).
Results' smoothing and target are checked in `p5qui-measure`, F7 (Revert
withdraws a delayed "Keep Anyway") in `p5qui-tune`. Screenshots in
`<out>/p5qui/`: `passive`, `viewer`.

| Name | What |
|---|---|
| `Measure.tab.passive` | the passive RTA's tab, shown only for a connection that may not play |
| `MeasureResults.smoothing`, `.target`, `.curvesNote` | the results' smoothing, the levelled target on or off, the core's smoothing in words |
| `MeasurePassive.start`, `.stop`, `.mic`, `.averaging`, `.hold`, `.clearHold` | the passive RTA's controls |
| `MeasurePassive.state`, `.blocker`, `.graph`, `.compare` | frames averaged, why it may not start, the spectrum (and held copy), the compare |
