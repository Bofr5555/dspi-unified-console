# OSC, MIDI and MQTT (placeholder)

Planned: control of DSPi parameters from show-control and automation systems
over OSC, MIDI and MQTT, through the hub (PRO1 integration items). Not built yet;
nothing on this page is a promise.

## What exists today

- **USB MIDI as a transport.** Firmware with USB MIDI control carries the vendor
  protocol over MIDI SysEx; clients connect with `--midi` (Terminal), CoreMIDI
  (iOS) or `MidiManager` (Android). This is the device's own protocol tunnelled
  through MIDI, not a mapping of MIDI notes or controllers to parameters.
- **Control surfaces in the firmware** (`crates/dspi-core/src/cs.rs`): knobs,
  buttons, IR remotes and the like bound to parameters on the device itself.
  See [firmware-control.md](firmware-control.md).

## To decide and write

- OSC address space, from the parameter registry; UDP port; replies and
  subscriptions.
- MIDI CC and note mapping, learn mode, value scaling.
- MQTT topics, retained state, the broker connection and its credentials.
- Which role a mapped control acts with on the hub, and rate limits.
