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
  seam) and `dspi_random_fill` (dspi-link's getrandom backend, once Link is in).
  Defined by the core's crates; this one adds none.
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

Profile `wasm-web` (release, `opt-level = "s"`, thin LTO, stripped, no
`wasm-opt`), without Link, 2026-11-08: **637 682 bytes, 227 423 gzip** (`gzip -9`), 184 353 brotli (`-q 11`).
WB0's study measured all of the C ABI at 753 KB / 288 KB gzip; WB2's scenario
module, with Link and at `opt-level = 3`, 844 KB / 302 KB gzip.
