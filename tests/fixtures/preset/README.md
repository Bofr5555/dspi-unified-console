# Whole-device documents

Shared `.dspipreset` documents for round-trip tests across clients (WP TM10). Both
describe the capture in `../link/device/` (RP2350, firmware 1.1.6 beta 5, wire V32),
which is also the demo device.

| File | Written by |
|---|---|
| `qt-core-v32.dspipreset` | `dspi_core`'s `config_export`, the code Qt runs, with no edits. A test in `apps/terminal/src/transfer.rs` compares it to a fresh export; `DSPI_TM10_REGEN=1` rewrites it. |
| `mac-swift-v32.dspipreset` | **Not written by a Mac.** `gen_mac_doc.py`, which follows `PresetDocument.capture(from:)` and `PresetDocumentFile.encode` of the Swift reference (`ext-channel-116` at `48cf7c5`) line by line over `../device/dspstate_v32.json`, with a tuning on top so that applying it has to move something, and the K340's crossfeed pair map. Replace it with a real export when one is available. |
