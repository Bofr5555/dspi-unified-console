# Object names: QP8 (Effects B)

Smoke section: `qp8-tools` (order 72, `smoke/SmokeTools.cpp`).

Tool windows, opened by their menu rows (`Registrar::Window`):

| Window | Menu row | Size (min) |
|---|---|---|
| `SubharmonicWindow` | `tools.subharmonic` | 780×620 (740×612) |
| `TubeWindow` | `tools.tube` | 780×664 (740×400) |
| `UpmixerWindow` | `tools.upmixer` | 400×720 (400×400, at most 400×1200) |

The limiter's settings are a popover on the output card (`OutputCard.qml`), opened by a
right-click on `OutputCard.limiter` (ChannelController's `limiterSettingsRequested`).

Shared parts (`<W>` is the window's name):

| Name | What |
|---|---|
| `<W>.header` | the header (`ToolHeader`); `<W>.header.switch` is the effect's on/off switch |
| `<W>.unsupported` | the note shown instead of the body on firmware without the effect |
| `<W>.<param>` | a parameter row (`ToolParameterRow`): signals `live(v)`, `commit(v)`; `liveValue` while held; `<W>.<param>.value` its field, `<W>.<param>.slider` its slider, `<W>.<param>.off` the "Off" over the field at a floor or ceiling that switches the stage off |
| `<W>.<choice>.<i>` | segment `i` of a segmented choice (`ToolSegments`), clicked like a button |

Subharmonic synthesizer:

| Name | What |
|---|---|
| `SubharmonicWindow.body` | the two columns |
| `SubharmonicWindow.solo` | Solo (wire V30); `checked` in `Accessible.checked`; closing the window switches it off |
| `SubharmonicWindow.startingPoints`, `.startingPoint.<0..3>` | Apply preset and its rows (Subwoofer feed, Club / large PA, Thin recordings, Cinema LFE) |
| `SubharmonicWindow.graph` | the band graph (`SubharmonicGraph`) |
| `SubharmonicWindow.headroom`, `.headroom.value` | the headroom cost and its figure ("+3.2 dB", "none") |
| `SubharmonicWindow.low`, `.high`, `.top` | the three band levels (`.top` with V30) |
| `SubharmonicWindow.mode.<0..2>`, `.mode.summary` | selectivity: All material, Percussive, Sustained; the line under it |
| `SubharmonicWindow.depth`, `.hold` | shown while the mode is not All material |
| `SubharmonicWindow.ceiling`, `.boost` | the sub ceiling (V30) and the 70 Hz bell |
| `SubharmonicWindow.outputs.presets` | the outputs' Presets menu: `.outputs.sub`, `.outputs.all`, `.outputs.none` |
| `SubharmonicWindow.output.<n>`, `.meter.<n>` | an output's chip and its sub meter |
| `SubharmonicWindow.linkPairs` | Link output pairs (V30) |

Tube modeller:

| Name | What |
|---|---|
| `TubeWindow.mode.<0..1>` | Basic, Advanced (the window's `advanced`, remembered under `tools/tube`) |
| `TubeWindow.basic`, `.advanced` | the two bodies |
| `TubeWindow.showcase`, `.showcase.name`, `.showcase.caption` | Basic's tube on show |
| `TubeWindow.tube`, `.heater`, `.bloom` | the drawing's three layers (`TubeIllustration`, `family`); the heater's `opacity` follows the switch, the bloom's the outputs' level |
| `TubeWindow.type.<1..16>` | Basic's shelf chips |
| `TubeWindow.basicDrive`, `.basicMix` | Basic's two rows |
| `TubeWindow.basicOutput.<n>`, `.basicOutput.presets`, `.all`, `.noSub`, `.none` | Basic's outputs |
| `TubeWindow.startingPoints`, `.startingPoint.<0..4>` | Advanced's Apply preset |
| `TubeWindow.graph` | the transfer curve (`TubeTransferGraph`) |
| `TubeWindow.harmonics.second.value`, `.harmonics.third.value` | the harmonics at full scale |
| `TubeWindow.typePicker`, `.typeCaption` | the tube picker and its line |
| `TubeWindow.drive`, `.mix`, `.trim`, `.bias`, `.asym`, `.hardness`, `.sag` | Advanced's rows |
| `TubeWindow.characterFrom` | "from 12AX7 / ECC83" or "custom" |
| `TubeWindow.rectifier.<0..3>`, `.rectifier.summary` | the rectifier and its line |
| `TubeWindow.xfmr`, `.damping`, `.lift`, `.resonance` | the output stage's switch, rows and lift line |
| `TubeWindow.output.<n>`, `.output.presets`, `.all`, `.noSub`, `.none` | Advanced's outputs |

Stereo upmixer:

| Name | What |
|---|---|
| `UpmixerWindow.body` | the scrolling column |
| `UpmixerWindow.status`, `.status.text` | the status and its line |
| `UpmixerWindow.correlation`, `.centerGain`, `.lsGain`, `.rsGain` | the gauges (`.bar` each) |
| `UpmixerWindow.centre.<0..2>` | Off, Sinner, Logician (wire 2, 0, 1) |
| `UpmixerWindow.surround.<0..2>` | Off, Sinner, Logician (wire 0, 1, 2) |
| `UpmixerWindow.centreSection`, `.surroundSection` | gone while their engine is off |
| `UpmixerWindow.strength`, `.width`, `.presence`, `.threshold`, `.attack`, `.release`, `.detector` | the centre's rows (the last four with Logician only) |
| `UpmixerWindow.delay`, `.hpf`, `.lpf`, `.decorrelation` | the surround's rows |

Limiter settings (popover):

| Name | What |
|---|---|
| `LimiterSettings` | the popover (`opened`) |
| `LimiterSettings.output` | the output's name |
| `LimiterSettings.enabled` | the switch |
| `LimiterSettings.settings` | threshold, release and link group; disabled and at 0.4 opacity while off |
| `LimiterSettings.threshold`, `.release` | the rows |
| `LimiterSettings.group.<0..4>`, `.groupSummary` | the link group (Off, 1..4) and who shares it |
| `LimiterSettings.copyToAll` | Copy to all outputs |
| `LimiterSettings.allOutputs` | the All outputs menu: `.linkPairs`, `.unlinkAll`, `.protectAll` (Protect all outputs), `.allOff` |
