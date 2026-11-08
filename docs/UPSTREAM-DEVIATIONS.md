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
