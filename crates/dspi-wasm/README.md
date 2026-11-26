# dspi-wasm

The web binding of uConsole for DSPi: `dspi-core` built for `wasm32-unknown-unknown`, run by
the web console's Worker (`apps/web/src/core/wasm.ts`) under JavaScript Promise
Integration. The crate docs (`src/lib.rs`) have the details.

- **One export for calls**, `dspi_call(session, json)`: `{"method", "params"}` in,
  `{"ok", "value"}` or `{"ok": false, "code", "error"}` out. Methods and shapes:
  `src/calls.rs` and the modules it hands a method to (one per family: `system.rs`,
  `speaker_layout.rs`, `measure_*.rs`, `hub_server*.rs`, `pair_map.rs` and so on), and
  `apps/web/src/core/protocol.ts` for the page. The other exports are the module's
  plumbing: `dspi_alloc`, `dspi_free`, `dspi_session_new`, `dspi_session_free`,
  `dspi_reply_ptr`, `dspi_panic_ptr`, `dspi_panic_len`.
- **Notices**: a failure from the core also carries `noticeKey`;
  `noticeFor {key, detail}` gives the notice to show (`dspi_core::notice_text`),
  `noticeTexts` the whole table.
- **Sessions:** any number per module, each its own `DspiCore`.
- **One call at a time, in the module** (`src/guard.rs`): a call that enters
  while another is suspended is refused (`busy`) before it touches anything.
- **Panics** mark the module dead (the hook runs before the abort); every later
  call is refused (`dead`) and the page reports the session lost.
- **Imports:** `dspi.dspi_now_ms`, `dspi_unix_ms`, `dspi_sleep_ms` (the clock
  seam) and `dspi_random_fill` (dspi-link's getrandom backend), defined by the
  core's crates; WebUSB's `dspi_usb_in`, `dspi_usb_out` (Suspending) and
  `dspi_usb_error` (`src/usb.rs`); the browser's WebSocket,
  `dspi_ws_open` and `dspi_ws_wait` (Suspending), `dspi_ws_error`,
  `dspi_ws_send`, `dspi_ws_close`, `dspi_ws_poll`, `dspi_ws_len`,
  `dspi_ws_take`, `dspi_ws_closed` (`src/link.rs`;
  `apps/web/src/core/ws.ts`).
- **Link** (`src/link.rs`): `dspi-link`'s client and the core's session
  and supervisor over the browser's WebSocket, caller-driven (no threads):
  `linkConnect {url, token, hubId}` (the hello), `linkPair {pin}` (the token),
  `linkDevices`, `open {"kind": "hub", "url", "deviceId"}` (a serial from the
  list or a fleet's), `linkStatus`, and `linkTick` (the Worker's turn: when to
  come back, and the status). `poll` and `meters` read what came over Link
  first; `close` leaves the hub.
- **USB** (`src/usb.rs`): `open {"kind": "usb", "serial", "device",
  "vendorId"}` after the Worker claimed the device; `usbPush {"packets"}` hands
  in notification packets the Worker read (the transport has no notification
  source of its own); `usbGone` ends the connection and answers
  the core's words; `usbIds` the protocol's IDs. The test module adds a mock
  DSPi (`dspi_mock_*`, the core's `FakeDevice::demo`, with the `test-hooks`
  feature only) for the WebUSB mock.
- **Writes** by registry path, indices and a typed value, as `Session::write`
  takes them (`src/write.rs`); the registry validates, the core's setter sends,
  the answer is read back from the device's image. Every writable path is routed
  except those the core writes only through a flow of its own
  (`dspi_core::write_refusal`, refused in the core's words); `describe` says which
  (`routed`). A gated path (`preset.save`, `preset.delete`, `dev.save`,
  `dev.save.io`) takes a `confirmToken`.
- **Presets** (`src/presets.rs`): `presetChanges` (`{dirty, changes}`,
  the core's sentences), `presetCopy {source, dest}` (from the active preset
  only), `presetRevert`, `presetLoadNotice` and `factoryReset` (the page asks
  the user first). Save, load, delete, names and
  the startup slot are `write`s (`preset.*`). `open` of the demo or a USB DSPi
  ends with the core's `fetch_all`, as every other client's connect does: the
  preset directory and names and the other reads outside the image, and the
  baseline the unsaved-changes check compares with (a hub DSPi had it already).
- **The rest, by family**: undo and redo (`undo`, `redo`, `canUndo`, `history`,
  `beginGroup`/`endGroup`); meters, the event log and `diagnostics`; the analyser
  (`rta*`); snapshots, A/B (`abStartSaved`: the preset as saved against now) and
  the guarded apply behind a trial (`applyGuard*`, `applyConfirm`, `applyRevert`);
  Solo (`soloOutput`, `soloEnd`, `soloState`); files and documents (`config*`,
  `document*`, `filters*`, `template*`); AutoEQ (`autoeq*`); the limits profile
  (`limits*`); curves (`curves`, with the phase); outputs (`crossoverType`,
  `crosspoint`, `outputLimits`); effects and Crossfeed's pair map
  (`crossfeedPairMap`, `setCrossfeedPairMap`); measurement, auto-tune and several
  units (`measure*`); the speaker layout (`speakerLayout`, `speakerLayoutPropose`,
  `speakerLayoutSet`, refused under the installer's lock) and its detection
  (`detect*`); the hub's families (`hub*`, `installation*`, `systemVolume*`); the
  firmware update check (`firmwareTarget`, `firmwareUpdateCheck`; nothing is
  written).

## Build and test

```bash
crates/dspi-wasm/tools/build.sh <out-dir>   # cross container: dspi_wasm.wasm, dspi_wasm_test.wasm
ci/check.sh --crate dspi-wasm               # native: fmt, clippy, the calls and the guard
ci/check.sh --step web                      # builds it, then the page's unit tests (the
                                            # module under JSPI) and e2e on the core's demo
ci/check.sh --step wasm-core                # includes clippy of this crate for wasm
```

`dspi_wasm_test.wasm` is built with the `test-hooks` feature (`test.sleep`, a
call that suspends; `test.panic`) for the binding's own tests. The page never
loads it.

## Size

`tools/build.sh` prints each module's size, raw and `gzip -9`. Profile `wasm-web`:
release (thin LTO, one codegen unit, stripped) with `opt-level = "s"`, no `wasm-opt`.

In November 2026, with every page of the web console (measurement and the simulated
microphone, the hub's families, the speaker layout, effects): **about 6.9 MB, 1.9 MB
gzip**.

Earlier measurements, for the history (2026-11-08): without Link 637 682 bytes, 227 423
gzip; with WebUSB 652 935 bytes, 232 678 gzip; with WebUSB and Link 1 085 783 bytes,
362 302 gzip; with the first preset calls 1 125 650 bytes, 372 668 gzip. Link adds
`dspi-link`, the core's Link session, and `tungstenite` with its handshake and `rand`,
which the web build links although the browser's WebSocket replaces it (`dspi-link`'s
`TcpConnector`; gating it out of the wasm build is still open).
