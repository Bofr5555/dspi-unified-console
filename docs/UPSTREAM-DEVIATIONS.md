# Deliberate deviations from upstream (Apple app)

Behaviour of `apps/apple` that differs from Weeb Labs' Swift DSPi Console on purpose, one line each, with
the reason. [`UPSTREAM-TRACKING.md`](UPSTREAM-TRACKING.md) section 2 asks for a reason for any change outside
its four places; this is where it is written down. Android keeps its own list in
`apps/android/docs/UPSTREAM-DEVIATIONS.md`.

- **SWF5, fractional delays** (`Commands.swift` `setDelay`/`setOutputDelay`/`sendOutputDelayToDevice`,
  `DashboardView`, `Components.swift` `ChannelSettingsView`, `MatrixMixerView` `CompactDelayField`,
  `PresetSnapshot`): upstream rounds every channel and output delay to whole ms in the setters; here they keep
  the core's 0.0001 ms grid (`quantize_delay`), so presets keep their delays across clients: a 0.5 ms delay
  written by Qt, the web client or the device no longer becomes 1 ms on import or on the next write on Mac and
  iOS. The sliders and scroll steps still move in whole ms; a typed value is kept; values are shown to 0.0001 ms
  with trailing zeros trimmed.
- **BUG1, output labels from the routing** (owner decision 2026-11-09, option B; `Core/OutputLabels.swift`,
  `ContentView` sidebar, `Components.swift` `OutputRow`/`ChannelVisibilityPill`, `DashboardView`,
  `CrossfeedPairMapView`): upstream shows an output's stored name, which until the user renames it is the
  firmware's slot name ("I2S 1 L": the left half of DAC slot 1). On the owner's K340, where slot 1 drives the
  right speaker's woofer and tweeter, every "L" was a woofer and every "R" a tweeter. Here an output with a
  default name is labelled from the matrix and its filters by the core (`output_labels_for`, dspi-session
  `output_label.rs`): "Out 1 · R · Low" (output, channel L/R/L+R/an input's name/Mix/None, band
  Low/High/Band/Full); a name the user set is shown as it is; the slot name is the row's tooltip and is in
  the pill's VoiceOver label. The sidebar's name column widens from 80 to 112 pt while a derived label shows.
  The dashboard pairs output cards by the K340 crossfeed pair map when the device has one ("Woofers L" |
  "Woofers R"), else by slot as upstream. Every client of this project does the same.
