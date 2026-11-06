# USB MIDI control channel test vectors

`midi_vectors.json` (JSON Lines) is copied unchanged from Bofr5555/DSPi
`bofr-main` `af001ec`, `firmware/tests/midi_vectors.json` (generated there by
`tools/midi_ctrl/gen_vectors.py`; the firmware's host tests read the same
file). SHA-256 `8e98a75396af33d878d44326fc4f2f1644c97c7eb6e6c8b58c8737a47538b723`. GPL-3.0, as the firmware.

Each line: `name`, `dir` (`h2d` host to device, `d2h` device to host),
`mfr` (hex manufacturer ID, 1 or 3 bytes), `tag`, `frame` (hex, type byte
through CRC) and `sysex` (hex, the whole message). Read by
`crates/dspi-proto/tests/midi_vectors.rs`.
