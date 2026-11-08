# dspi-wasm

The web binding (WP WB3): `dspi-core` built for `wasm32-unknown-unknown`, run by
the web console's Worker (`apps/web/src/core/wasm.ts`) under JavaScript Promise
Integration. The crate docs (`src/lib.rs`) have the details.

- **One export for calls**, `dspi_call(session, json)`: `{"method", "params"}` in,
  `{"ok", "value"}` or `{"ok": false, "code", "error"}` out. Methods and shapes:
  `src/calls.rs`, and `apps/web/src/core/protocol.ts` for the page.
- **Sessions:** any number per module, each its own `DspiCore`.
- **One call at a time, in the module** (`src/guard.rs`): a call that enters
  while another is suspended is refused (`busy`) before it touches anything.
- **Panics** mark the module dead (the hook runs before the abort); every later
  call is refused (`dead`) and the page reports the session lost.
- **Imports:** `dspi.dspi_now_ms`, `dspi_unix_ms`, `dspi_sleep_ms` (the clock
  seam) and `dspi_random_fill` (dspi-link's getrandom backend), defined by the
  core's crates; WebUSB's `dspi_usb_in`, `dspi_usb_out` (Suspending) and
  `dspi_usb_error` (WB4a, `src/usb.rs`); the browser's WebSocket,
  `dspi_ws_open` and `dspi_ws_wait` (Suspending), `dspi_ws_error`,
  `dspi_ws_send`, `dspi_ws_close`, `dspi_ws_poll`, `dspi_ws_len`,
  `dspi_ws_take`, `dspi_ws_closed` (WB4b, `src/link.rs`;
  `apps/web/src/core/ws.ts`).
- **Link** (WB4b, `src/link.rs`): `dspi-link`'s client and the core's session
  and supervisor over the browser's WebSocket, caller-driven (no threads):
  `linkConnect {url, token, hubId}` (the hello), `linkPair {pin}` (the token),
  `linkDevices`, `open {"kind": "hub", "url", "deviceId"}` (a serial from the
  list or a fleet's), `linkStatus`, and `linkTick` (the Worker's turn: when to
  come back, and the status). `poll` and `meters` read what came over Link
  first; `close` leaves the hub.
- **USB** (WB4a, `src/usb.rs`): `open {"kind": "usb", "serial", "device",
  "vendorId"}` after the Worker claimed the device; `usbPush {"packets"}` hands
  in notification packets the Worker read (the transport has no notification
  source: one or the other, c6a W2-2); `usbGone` ends the connection and answers
  the core's words; `usbIds` the protocol's IDs. The test module adds a mock
  DSPi (`dspi_mock_*`, the core's `FakeDevice::demo`) for the WebUSB mock.
- **Writes** by registry path, indices and a typed value, as `Session::write`
  takes them (`src/write.rs`); the registry validates, the core's setter sends,
  the answer is read back from the device's image. Routed paths so far:
  `vol.user`, `vol.mute`, `vol.master`, `pre`, `bypass`, `out.gain`, `out.mute`,
  `loud.on`, `loud.ref`, `loud.intensity`; WB6's slices add the rest.

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

With WebUSB and Link (WB4b), 2026-11-08: **1 085 783 bytes, 362 302 gzip**
(`gzip -9`). Link adds 433 KB, 130 KB gzip: `dspi-link`, the core's Link
session, and `tungstenite` with its handshake and `rand`, which the web build
links although the browser's WebSocket replaces it (`dspi-link`'s
`TcpConnector`; gating it out of the wasm build is an OPT9 candidate).

With WebUSB (WB4a), 2026-11-08: **652 935 bytes, 232 678 gzip**.

Profile `wasm-web` (release, `opt-level = "s"`, thin LTO, stripped, no
`wasm-opt`), without Link, 2026-11-08: **637 682 bytes, 227 423 gzip** (`gzip -9`), 184 353 brotli (`-q 11`).
WB0's study measured all of the C ABI at 753 KB / 288 KB gzip; WB2's scenario
module, with Link and at `opt-level = 3`, 844 KB / 302 KB gzip.
