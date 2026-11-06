# One Rust core: design and work packages

*Branch `core-merge`. Design 2026-11-06. Base: see section 4.*

This plan merges the repository's two Rust protocol cores into one:

- **Terminal crates** (`dspi-proto`, `dspi-transport`, `dspi-session`):
  nusb, constants generated from the firmware headers, GPL-3.0-or-later.
  They are the base.
- **Old desktop core** (`crates/dspi-core-linux`, now package
  `dspi-core-linux`, library `dspi_core`): rusb, hand-copied constants, the
  ~150-function C ABI the Qt app uses. It is retired into the base.

The owner's order of work:
1. Windows (done);
2. parity audit (`dspi-mobile/docs/FEATURE-PARITY.md`);
3. **this merge**;
4. Android's `dspi-ffi` onto the merged core;
5. the iOS app onto it, through UniFFI Swift on its own branch.

The design keeps steps 4 and 5 thin wrappers and leaves a web/wasm build
possible later.

Background reading:
- `dspi-mobile/docs/RUST-CORE-CONSOLIDATION.md` (the study);
- `dspi-mobile/docs/WIRE-FORMAT-COMPAT.md`;
- Bofr5555/DSPi `Documentation/Features/{usb_midi_control_spec,extension_channel_spec,control_interfaces_spec}.md`.

The work is split into work packages (WPs) for parallel agents. Each one
gets [`WP-BRIEF.md`](WP-BRIEF.md).

---

## 1. Design

### 1.1 Layout

```
crates/
  dspi-proto      pure protocol. No I/O, no threads.
                  generated constants (bofr-main headers), bulk codec (V32),
                  wire_compat (V16..V33 read-only, normalised to V32),
                  ext (extension-channel codecs), frame (UART frame, CRC16,
                  MIDI SysEx 7-bit packing and decoder)
  dspi-transport  the one Transport trait, TransportCaps, TransportError
                  usb    nusb; UsbTransport::from_fd on Linux/Android
                  midi   sans-IO MidiClient + blocking MidiTransport over an
                         app-supplied MidiPipe; midir ports behind feature `midir`
                  link   DSPi Link adapter (feature `link`)
                  fake   the stateful demo device
                  mock   scripted replies (awkward paths)
  dspi-link       DSPi Link client (WebSocket), unchanged
  dspi-session    state machine + protocol operations: probe, DeviceState (the
                  bulk image as mirror), notifications with origin, writes
                  (confirmed, and the fast path), compat reads, ext probe,
                  RTA engine, control surfaces (+ the editing model), presets,
                  filter files, AutoEQ, channel clipboard
  dspi-core       the embedder API: DspiCore, one object for a GUI or a
                  binding. Records (DspState, ...), typed DspiError, listener
                  traits, device manager, notify/RTA threads, JSON features,
                  firmware files
  dspi-capi       C ABI over dspi-core for the Qt app: the old names and
                  #[repr(C)] records, header include/dspi_core.h; library name
                  becomes dspi_core in WP Q3
  dspi-cmd, dspi-tui, apps/terminal   the terminal, a direct session consumer
  dspi-core-linux the old core; deleted by WP Q3
```

Later, on top of `dspi-core`:
- `dspi-ffi`: UniFFI, giving Kotlin for Android and Swift for iOS (the
  scaffold is WP F1);
- `dspi-wasm`: over proto and session, with an async driver (not in this
  plan).

### 1.2 The Transport trait (done in the base)

Terminal's shape is kept: `&mut self`; `control_in`, `control_in_upto` and
`control_out`; `max_transfer`; and `notifications()`, which returns a
separate `NotificationSource`. What the Link needs (from Android patch 0004)
comes as default methods, so every existing implementor still compiles:

| Method | Default (USB) | Used for |
|---|---|---|
| `caps() -> TransportCaps` | kind `Usb`, `chunked_bulk: true`, `remote: false` | when `chunked_bulk` is false, bulk goes single-shot as `0xA0`/`0xA1` (Link spec 8.2) |
| `session_id() -> Option<u16>` | `None` | echo attribution |
| `lock(reason, timeout)` / `unlock()` | no-op | the Link device lock around multi-step flows (spec 7.7) |
| `NotificationSource::read_attributed` | `read()` with origin `None` | Link supplies the writer's session id |

`TransportError` holds no backend types. It gained `Denied`, `Locked` and
`Remote` for non-USB links.

**Echo rule** (gate 1, B1; revised after wave 1).
`dspi_transport::is_own_write(kind, source, origin, session,
own_bulk_pending)` decides it; `Notification::is_own_echo(kind, session,
own_bulk_pending)` applies it to `PARAM_CHANGED` and `BULK_INVALIDATED`.
The firmware tags a change by the path it came in on, except bulk applies:

| Kind | Ours | Why |
|---|---|---|
| Usb, Fake | `HOST_SET`; `BULK_SET` only while `own_bulk_pending` | a device has one USB host for scalar writes |
| Midi | `PARAM_SRC_MIDI` (10); `BULK_SET` only while `own_bulk_pending` | MIDI writes notify as MIDI (notify.h:122, MIDI spec 4.2), so a `HOST_SET` seen over MIDI is another host's USB write |
| Link | origin == our session id and != 0, any source; `own_bulk_pending` ignored | the hub writes for every client, over USB or (a bridge) UART; only the origin tells clients apart (Link spec 8.3) |

**Bulk echoes.** The firmware tags *every* bulk apply `BULK_SET`, whatever
path it came in on (bulk_params.c:371/408, `notify_end_bulk`; confirmed on a
unit over MIDI), so the source alone cannot tell our `0xA1`/`0xA3` from
another host's. `dspi_session::notify::EchoFilter` (pure; the caller passes
the clock) tracks it:
- `bulk_write_started()` when our bulk write goes out;
- `bulk_write_finished(now, ok)` on its reply: an OK reply opens a window
  until `now + 1 s` (`EchoFilter::WINDOW`), a failure closes it;
- `classify(now, kind, source, origin, session)` claims the first
  `BULK_SET` while the write is in flight or inside the window, **once**,
  then it is consumed. Everything else is foreign: an unclaimed invalidation
  only costs a re-read.

`Connection::echo` holds one per connection (base). WP C2 (`apply_all_params`)
and any other bulk writer bracket their write with it; WP C7 classifies
with it; WP C1 resets it on reopen. Reference behaviour: the iOS console's
`MIDITransport` (`dspi-ios/patches/console-116/0005`). A firmware follow-up
(owner decision) could tag bulk applies by transport; the filter then only
matters for old firmware.

**Busy retries** (`dspi_transport::with_busy_retry(kind, ..)`, revised after
wave 1): a single `TimedOut { retries: 0 }` is retried on every link; a
`TimedOut` with `retries > 0` (the transport already ran its own busy
window: MIDI, Link) is returned at once; `Stalled` is retried only on Usb
and Fake (on MIDI and Link a stall is a definitive `ERROR`).

**Threads.** The old core shared one libusb handle between threads. In the
new core the control path sits behind the `DspiCore` mutex, and
notifications come from the separate `NotificationSource`. The mutex also
serialises chunked bulk sessions, which any other request aborts on the
device.

**Thread and lock rules** (gate 1, S-f), binding on C1, C1b, C7, C9 and C15:
- never join a thread (notification reader, RTA worker, Link reader) while
  holding the `Inner` lock: take the object out under the lock, drop the
  guard, then drop or join it;
- stop and join the old notification reader before a new device is opened
  (`attach_probed` does this);
- call listeners with the `Inner` lock released (collect events under the
  lock, then call), through `Slot::call`; each of these WPs has a test that
  a listener calling back into `DspiCore` does not deadlock;
- listeners must not block (`callbacks.rs` says so);
- the notification queue is bounded (`Notifications::QUEUE`, 1024): an
  overflow drops packets and marks the next delivered one `lost`, which
  forces a bulk re-read.

**Opening a device** always ends in `DspiCore::attach_probed` (gate 1, B2;
in the base, owned by WP C1 from wave 2):
1. drop the old connection with the lock released;
2. probe, and build the `Session`;
3. read the bulk image into the mirror;
4. start the notification reader with the `NotifyListener` waker
   (`Notifications::start_with_waker`);
5. install the connection.

The base version is strict V32 and keeps the image in `bulk_raw` only. C1
switches it to `probe_any` (S2), `compat::read_bulk_any` (S2),
`state::decode_bulk` (C0) and `ExtState::probe` (S3).

**Writes are typed** (gate 1, S-h). `Inner::writable_conn` returns a
`Writable<'_>`, the only type with `set`, `set_as_read` and the session's
write path. A plain `Connection` offers reads only, so writing on a
read-only connection (§1.5) does not compile. Reviewers check that
`Session::send`/`write`/`write_band`, `control_out` and write-as-read
opcodes appear only on a `Writable`.

One named exception: `REQ_CLEAR_CLIPS` (clearing the meters' clip latch,
`clear_clips`) changes no setting and is allowed on a read-only
connection, so it may go through a plain `Connection` (`get`).
`REQ_RESET_BUFFER_STATS` (`reset_buffer_stats`, resets the watermarks only)
and `REQ_ENTER_BOOTLOADER` (`enter_bootloader`, so a too-old or too-new device
can still be updated, see 1.5) are allowed the same way. Any other exception
needs a line here.

### 1.3 Sans-IO

These parts are pure:
- all of `dspi-proto`;
- `DeviceState::apply`;
- notify decoding;
- the RTA engine's `accept_*`;
- the mirror codec `dspi_core::state`;
- `MidiClient` (request to bytes, bytes to events). The blocking
  `MidiTransport` drives it over a `MidiPipe`.

Session operations block on `&mut dyn Transport`. A wasm build needs an async
twin of the trait, or a request/reply split of those operations. Both build on
the pure parts. Neither is in this plan, but no WP may add I/O to a pure part.

### 1.4 The mirror

`DspiCore` keeps a `DspState` record (`Inner::state`, returned by
`get_state`) that holds the device's bulk image in `bulk_raw`.
- `state::decode_bulk` fills the record from the image.
- `state::encode_bulk` writes the record back into the image.

The contract is in `crates/dspi-core/src/state.rs` (WP C0). It is the old
core's proven model, now on the generated offsets.

How the mirror stays current:
- **Setters** take a `Writable`, send through the session's fast path
  (`Session::send`, WP S1) or a raw opcode where the registry has no path,
  then update the record field and `bulk_raw` (`encode_bulk`).
- **Notifications** patch `encode_bulk(state)` at the packet's offset and
  re-decode.
- **Our own echoes** (§1.2) are applied too, because the device is
  authoritative. They are not reported as external changes.

The Terminal keeps `dspi_session::DeviceState` (a `BulkPacket`). The same
notifications drive both.

Some values are not in the image: the preset directory and names, the Core 1
mode, DAC mute support, the control interfaces and subharm solo. They stay in
the record, and `fetch_all` reads them one by one.

### 1.5 Wire formats and the read-only mode

V32 is read and written. Other wire formats are read best-effort, for display
only (`dspi_proto::wire_compat`, in the base, from Android patch 0003):
- older images (V16..V31) are mapped onto the V32 layout;
- newer images keep their V32 prefix.

Such a connection is **read-only**:
- `Connection::writable()` is false;
- every setter returns `DspiError::ReadOnly`;
- `compat` is `COMPAT_FIRMWARE_TOO_OLD` or `COMPAT_FIRMWARE_TOO_NEW`;
- `bulk_valid` is false, so the image is never written back.

**Unmappable images** (decision after wave 2a review). When the device
identifies itself (`GET_PLATFORM` succeeds) but its image cannot be mapped
(pre-V16, an implausible newer image, or a legacy-VID unit whose bulk read
fails), `attach_probed` still installs the connection, so the app can show
the device, its status and offer a firmware update:
- `Connection::link` stays `Wire::Raw` (no `Session`), `wire` is `None`;
- the mirror comes from `state::apply_platform` only, `bulk_valid` false;
- `compat` is `COMPAT_FIRMWARE_TOO_OLD` or `COMPAT_FIRMWARE_TOO_NEW` from
  the version byte;
- `Connection::get` / `get_upto` keep working (`fetch_status`,
  `enter_bootloader`), every setter is `ReadOnly`, and anything needing the
  session returns `Disconnected`'s sibling error from `Connection::session`.

### 1.6 Extension channel

`REQ_EXTENSION` 0x00 carries the sub-command in `wValue`:
- `EXT_LIST` 0x0000;
- `EXT_CROSSFEED_PAIR_MAP` 0x0001;
- `EXT_CORE_VREG` 0x0002.

The codecs are in `dspi_proto::ext` and the I/O in `dspi_session::ext`.
`ExtState::probe` (WP S3) runs on connect (WP C1) and is kept on the
connection. A stall means "not supported". It is never an error and is never
retried.

### 1.7 Trust boundaries and the panic policy

| Boundary | Untrusted input | Rule |
|---|---|---|
| Device | every reply, the bulk image, notify packets, MIDI SysEx from the device | Check lengths before indexing. A short or odd reply is a typed error or "absent", never a panic. Nothing from the device sizes an allocation unchecked (`payload_length`, SysEx length). |
| DSPi Link peer (hub) | the JSON control plane, binary frames, `bulk_b64`, device lists | As for the device, plus time bounds (`dspi-link`'s timeouts). A hub cannot make the client block forever or allocate without bound. |
| App-supplied MIDI pipe | arbitrary bytes in any split, from Kotlin, Swift or midir; send failures | The decoder ignores foreign data. A malformed message fails the request in flight with an error. The decoder caps a message at 8192 bytes (the largest legal one is 7032) and drops a longer one (P1). `MidiPipe::send` returns a typed `PipeError`. |
| C ABI callers | null pointers, short buffers, bad UTF-8, wrong handles | Checked before use. Every export runs inside `dspi_capi::guard` (`catch_unwind`), which returns the documented failure value. |
| UniFFI callers (later) | values from Kotlin and Swift | UniFFI marshals them. Methods validate ranges (`InvalidArgument`). |
| Files | `.dspipreset`, filter files, `.uf2`, AutoEQ data | Parse errors are values. A UF2 is fully validated before any byte goes to a drive. |

**Panic policy.**
- No `unwrap`, no `expect`, no unchecked indexing and no unchecked
  arithmetic on device, network, file or caller data.
- `unwrap` is allowed only on an invariant the code itself establishes, with
  a comment saying why.
- Device-sourced indices are bounds-checked even in pure helpers
  (`PairMap::pair(p)` with `p >= 4` panics today: P1 fixes it).
- A transport never returns `Ok` with fewer bytes than `control_in` asked
  for; that is `ShortRead`. Bulk loops stop on zero progress (S2).
- Listener callbacks are wrapped (`Slot::call`).

### 1.8 Code-quality bar (reviewed per wave)

- **CI gates:** `ci/check.sh` passes. That means `cargo fmt`, clippy at the
  default lint set with `-D warnings`, the workspace tests, and the Windows
  GNU cross-build.
- **Strict lints** (gate 1, S-k) are on in the new code: crate-wide in
  `dspi-core` and `dspi-capi`, and module-wide in `fake.rs`, `midi.rs`,
  `link.rs`, `fast.rs`, `compat.rs` and `ext_state.rs`. They are
  `clippy::unwrap_used`, `expect_used`, `indexing_slicing` and
  `arithmetic_side_effects`. `clippy.toml` exempts test code from the first
  three; test modules carry `#[allow(clippy::arithmetic_side_effects)]`. New
  files in those crates inherit them; new modules elsewhere in the session
  or transport copy the attribute block.
- **missing_docs** is turned on in `dspi-core` and `dspi-capi` by WP F1,
  which documents every record field anyway. Until then the doc rule below
  is reviewed by hand.
- **Errors:** typed enums (`TransportError`, `WriteError`, `DspiError`,
  `FrameError`, ...), never strings or magic numbers. The one exception is
  the C ABI, which keeps the old conventions.
- **Docs:** every `pub` item in `dspi-core` and `dspi-capi` has a doc
  comment that states what it does, its units and its failure modes.
- **Tests:**
  - each WP adds tests, and the test module or test name carries the WP id
    (for example `mod tests_wp_c3`, `fn wp_c3_master_volume_round_trips`);
  - tests run without a device, against `FakeDevice` (WP T4),
    `MockTransport` or fixtures.
- **Skeleton markers:** each `todo!("WP xx")` and `#[ignore = "WP xx"]` is
  removed by the WP it names. After wave 4,
  `grep -rn 'todo!("WP\|ignore = "WP'` finds nothing.
- **Dependencies:** a new dependency needs a line in the WP report saying
  why. No unpinned git dependencies.

### 1.9 Fuzz and property tests

These tests use `proptest`, which runs on stable inside the normal test run.
**Every property test function is named `proptest_*`** (gate 1, S-c), so
`ci/check.sh --step proptest` can run them longer by name
(`cargo test -- proptest_`, 20 000 cases).
`cargo-fuzz` targets can come later under `fuzz/`. They need nightly, so they
will not be a gate.

| Target | Property | Lives in | WP |
|---|---|---|---|
| `Frame::decode` | never panics on any bytes; `decode(encode(f)) == f` | `crates/dspi-proto/tests/proptest_frame.rs` | P1 |
| `SysexDecoder::feed` | never panics on any bytes in any split; a valid message inside noise is found exactly once; a message over the 8192-byte cap is dropped, not buffered (P1 adds the cap to `frame.rs`) | same | P1 |
| `PairMap::pair`, `PairMap::parse`, `CoreVregStatus::decode` | never panic on any input (P1 adds the bound on `p`) | `crates/dspi-proto/tests/proptest_ext.rs` | P1 |
| `pack7` / `unpack7` | round trip; `unpack7` never panics | same | P1 |
| `wire_compat::normalize_bytes` / `normalize` | never panics on any bytes or length; for V16..V31 images, the header and the fields both layouts share agree after mapping | `crates/dspi-proto/tests/proptest_wire.rs` | P1 |
| `BulkPacket::decode`, `WireHeader::decode` | never panic | same | P1 |
| `dspi_session::notify::decode` | never panics; `DeviceState::apply` of any decoded packet never panics | `crates/dspi-session/tests/proptest_notify.rs` | P1 |
| `MidiClient::receive` | never panics; replies match their tags | `crates/dspi-transport/src/midi.rs` tests | T2 |
| `state::decode_bulk` / `encode_bulk` | `decode(encode(decode(x)))` equals `decode(x)` for random V32 images | `crates/dspi-core/src/state.rs` tests | C0 |
| `uf2_inspect` | never panics on any bytes (`proptest_uf2_inspect_never_panics`) | `crates/dspi-core/src/firmware.rs` tests | C11 |
| C ABI | null and short-buffer calls return the failure value | `crates/dspi-capi` tests | Q2a, Q2b |

### 1.10 Shared files

These files belong to the base. WPs change them only additively, and list
each change in their report; the reviewer merges them:
- `Cargo.toml` (the workspace's and each crate's);
- `Cargo.lock`, which is regenerated on merge;
- the `crates/*/src/lib.rs` module lists;
- `crates/dspi-core/src/{lib.rs,records.rs,consts.rs,error.rs,callbacks.rs}`.

New records and new `DspiError` variants count as additive. A WP that needs
to change `Inner` or `Connection` says so in its report rather than working
around the need.

**Section markers** (gate 1, S-a). These shared files have one
`// -- WP <id> --` line (`# -- WP <id> --` in TOML) per WP that may add to
them, in a fixed order. A WP appends only directly under its own marker, so
parallel additions merge cleanly:
- `crates/dspi-core/src/{records.rs,consts.rs}` (end of file), `error.rs`
  (inside `DspiError`), and the `pub use` block of
  `crates/dspi-core/src/lib.rs`;
- the `pub use` block of `crates/dspi-transport/src/lib.rs`;
- `[dependencies]` and `[dev-dependencies]` of `dspi-core`,
  `dspi-transport`, `dspi-session`, `dspi-proto` and `dspi-capi`.

The known dependencies are already in place: `proptest` (dev) in proto,
transport, session and core; `nusb` in core (C11); `cbindgen` (build) in
capi (Q1).

---

## 2. Work packages

**For every WP:**
- Branch `wp/<id>` from `core-merge`. The coordinator creates the worktree.
- Run `ci/check.sh --crate <crate>` for every crate touched. The reviewer
  runs the full `ci/check.sh` on the merged wave.
- Report in the format in `WP-BRIEF.md`.

The interfaces are in the skeleton; "Implements" points to them. A WP may add
private helpers and tests freely. It adds public items beyond its interface
only with a justification in its report.

Sizes are in agent-days.

### Wave 1 (depends on the base only)

**P1: Property tests for the codecs.** Size 0.5.
- **Owns:**
  - `crates/dspi-proto/tests/proptest_frame.rs`;
  - `crates/dspi-proto/tests/proptest_wire.rs`;
  - `crates/dspi-proto/tests/proptest_ext.rs`;
  - `crates/dspi-session/tests/proptest_notify.rs`.
- **May fix** panics it finds in `crates/dspi-proto/src/{frame.rs,wire_compat.rs,wire.rs,ext.rs}`
  and in `decode` in `crates/dspi-session/src/notify.rs`.
- **Must add** (gate 1, S-d): the 8192-byte message cap in `SysexDecoder`
  (a longer message is dropped whole; the decoder resynchronises on the
  next `F0`), and the bound on `PairMap::pair(p)` (`p >= 4` is the default
  pair or `None`; choose, document, test).
- **Implements:** the P1 rows of §1.9.
- **Done when:**
  - every target exists;
  - every panic found is fixed and has a regression test;
  - `ci/check.sh --crate dspi-proto --crate dspi-session` passes.
- **Parity:** none (hardening).

**S1: Session fast write.** Size 0.5.
- **Owns:** `crates/dspi-session/src/fast.rs`.
- **Implements:** `Session::send`, documented in the file. It goes through
  the registry the way `Session::write` does: `by_path`, `check_available`,
  `check_indices`, `kind.validate`, `build_wvalue`, `repr().encode`. EQ
  fields are written as whole bands.
- **Done when:**
  - a scalar costs one transfer (assert with `MockTransport::log_handle`);
  - an EQ scalar costs one read and one write;
  - write-as-read stores `last_write_status`;
  - its errors are the same as `write`'s.
- **Parity:** prerequisite of 3.0 "FFI".

**S2: Compat bulk read and a tolerant probe.** Size 1.
- **Owns:** `crates/dspi-session/src/compat.rs`,
  `crates/dspi-session/src/probe.rs`.
- **Implements:** `compat::read_bulk_any` and `compat::read_bulk_bytes`.
  The bulk read is chunked and sized by `wire_compat::image_length`, or
  single-shot when `!caps().chunked_bulk`.
- **Implements:** `probe::probe_any(t) -> Result<(Capabilities, WireRead)>`
  (skeleton in `probe.rs`, gate 1 S-m): what `probe` reads, but a firmware
  of another wire format is described instead of refused. The existing
  `probe` keeps its behaviour for the Terminal.
- **Sans-IO split** (gate 1, S-l): the chunk plan (offsets and lengths from
  the header) and the image assembly are pure functions, tested directly;
  the transport loop only drives them.
- **Robustness** (S-i): a chunk reply that adds no bytes ends the read with
  an error (no endless loop); never trust `payload_length` beyond
  `u16::MAX` or below the header.
- **Done when:** tests on `MockTransport` cover a V28 image, a V33 image
  (which reads past 6136 bytes), the single-shot path and a zero-progress
  chunk.
- **Parity:** 3.0, 3.2 and 3.3, "read-only decoder for older wire formats".

**S3: Extension probe in the session.** Size 0.5.
- **Owns:** `crates/dspi-session/src/ext_state.rs`,
  `crates/dspi-session/src/ext.rs`.
- **Implements:** `ExtState::probe` and `has`.
- **Sans-IO split** (S-l): deciding what to read from an `EXT_LIST` reply
  and building `ExtState` from replies are pure, tested without a transport.
- **Done when:**
  - stock firmware probes as an empty state;
  - a K340 gives the list, the pair map and the vreg status;
  - a stall is never an error, and a disconnect is.
- **Parity:** 3.0 and 3.3, "EXT_LIST probe on connect".

**S4: Move the control-surface editing model into the session.** Size 1.
- **Owns:**
  - `crates/dspi-session/src/cs_model.rs`;
  - `crates/dspi-tui/src/settings/cs_model.rs` (deleted or re-exported);
  - `crates/dspi-tui/src/settings/{mod.rs,surfaces.rs}` and
    `crates/dspi-tui/src/settings/surfaces/*.rs`, for imports only.
- **Moves** the part that does not depend on the TUI (data, validation, op
  application). TUI-only types (`Cx`, `SessionReply`) stay in the TUI behind
  an adapter. The tests move with the code.
- **Done when:** the TUI behaves as before (its tests pass with only path
  changes) and the session API is documented.
- **Parity:** 3.0, "TUI logic GUIs need: CS editing model".

**T1: USB from a file descriptor, and Android gating.** Size 0.5.
- **Owns:** `crates/dspi-transport/src/usb.rs`.
- **Implements:** `UsbTransport::from_fd`, which calls
  `nusb::Device::from_fd` and claims interface 2. It takes ownership of
  the `OwnedFd` it is given; the embedder (C1's `open_fd`) passes a
  `dup(2)` of the platform's descriptor, because Kotlin closes its own
  `UsbDeviceConnection` (gate 1, S-g).
- **Android:** `list_devices` and `open*` return an empty list or
  `NotFound` without calling `nusb::list_devices`, which does not exist
  there. The study's ~50-line diff for this was on the tmpfs scratch and is
  gone; RUST-CORE-CONSOLIDATION.md §5.2 describes it.
- **Done when:** `ci/check.sh --step android` gets past `dspi-transport`.
  It fails there today.
- **Parity:** prerequisite of step 4.

**T2: MIDI SysEx client and transport.** Size 1.
- **Owns:** `crates/dspi-transport/src/midi.rs`, except the `midir_port`
  module, whose signatures stay for T3.
- **Implements:** `MidiClient`, `MidiInput` and `MidiTransport`, following
  the docs in the file and spec section 5:
  - one request in flight, with tags 1..0x7E;
  - a `0x7F` reply, or an error with the request's tag, fails the request
    in flight;
  - `BUSY`, `BULK_LOCKED` and `CRC_ERROR` are retried;
  - `ERROR` maps to `Stalled`;
  - requests have timeouts;
  - notifications come through `NotificationSource`;
  - `MidiPipe::send` errors are `PipeError` (typed), the pipe is an
    `Arc<dyn MidiPipe>` (a binding can hand in a foreign object);
  - `caps().chunked_bulk` is false: the firmware serves the chunked
    opcodes over MIDI, but single-shot `0xA0`/`0xA1` are its recommended
    bulk commands there (spec 4.2), so bulk goes in one message.
- **Sans-IO split** (S-l): `MidiClient` stays pure (no clock, no thread);
  timeouts and retries live in `MidiTransport` only.
- **Done when:**
  - tests drive `MidiTransport` against an in-process fake device that
    speaks SysEx, built on `dspi_proto::frame`;
  - they cover GET, SET, bulk `0xA0`/`0xA1`, a notification between
    replies, a CRC error retried, and a timeout;
  - the proptest row for `MidiClient::receive` (§1.9) is covered.
- **Parity:** 3.0, "USB MIDI SysEx transport".

**T4: The stateful demo device.** Size 1.
- **Owns:** `crates/dspi-transport/src/fake.rs`.
- **Implements:** `FakeDevice::{new, demo}` and its `Transport` impl.
  - It bundles `tests/fixtures/link/device/{platform,bulk_v32}.bin` with
    `include_bytes!`.
  - It answers:
    - the platform, firmware-version and build-info reads;
    - bulk `0xA0`, and `0xA2` chunked (offset in `wValue`, per the bulk
      chunking spec);
    - bulk writes `0xA1` and `0xA3`;
    - `REQ_GET_STATUS`;
    - the preset directory and names.
  - Scalar writes that land at a bulk offset patch the image and queue a
    `PARAM_CHANGED` with source `HOST_SET`. Derive the offsets from
    `dspi_proto::generated` (`SECTIONS`, the `*_FIELDS` tables and the
    per-section layouts), never from hand-copied numbers (gate 1, S-i).
  - A read never returns `Ok` with fewer bytes than asked for, except
    through `control_in_upto` (S-i).
  - With `options.extensions` set, it answers `EXT_LIST`, the pair map and
    vreg.
  - It stalls on anything unknown.
- **Done when:**
  - the Terminal's `probe` and `read_bulk` succeed against
    `FakeDevice::demo()`;
  - a master-volume write reads back and notifies.
- **Parity:** 3.0 and 3.2, "stateful demo device".

**T5: DSPi Link transport.** Size 1.
- **Owns:** `crates/dspi-transport/src/link.rs`.
- **Implements:** `map_status` and `LinkTransport`.
- **Input:** `~/prog/dspi-mobile/dspi-android/rust/dspi-ffi/src/link.rs`.
- **Done when:** tests run against `dspi-link`'s fake hub (a
  dev-dependency with feature `fake-hub`; fixtures in `tests/fixtures/link`)
  and cover:
  - a bulk read, single-shot or from the snapshot;
  - a SET;
  - a notification with its origin;
  - the lock;
  - the status mapping.
- **Parity:** 3.0, 3.2 and 3.3, "`LinkClient` → `Transport`". mDNS browse is
  not in scope.

**C0: The mirror codec.** Size 1.
- **Owns:** `crates/dspi-core/src/state.rs`, and additions to
  `crates/dspi-core/src/consts.rs`.
- **Implements:** `default_state`, `apply_platform`, `decode_bulk` and
  `encode_bulk`.
- **Inputs:**
  - the old codec in `crates/dspi-core-linux/src/{protocol.rs,state.rs,types.rs}`;
  - the `dspi_proto::generated` offsets: `SECTIONS`, `wire::*` and the
    `*_FIELDS` tables.
- **Done when:**
  - the capture decodes to `tests/fixtures/device/dspstate_v32.json` field
    for field (camelCase names);
  - `default_state()` equals `dspstate_default.json`;
  - the round-trip property in §1.9 holds;
  - the two `#[ignore = "WP C0"]` tests in `lib.rs` are un-ignored.
- **Parity:** prerequisite of everything in `dspi-core`.

**C11: Firmware files and the bootloader drive.** Size 1.
- **Owns:** `crates/dspi-core/src/firmware.rs`.
- **Implements:** `uf2_inspect`, `bootloader_counts`, `bootloader_drives`
  and `flash_to_drive`.
- **Inputs:**
  - `apps/desktop/qt/src/FirmwareUpdater.cpp`, for drive discovery on
    Linux, macOS and Windows, and the sync rules;
  - the Android app's Kotlin UF2 validator (in `dspi-mobile/dspi-android`;
    search for `Uf2`).
- **Constraints:**
  - nusb enumeration is gated off on Android;
  - drive tests use a temp dir under `$TMPDIR` (on `~/.cache` in the
    container) that contains an `INFO_UF2.TXT`.
- **Done when** (gate 1, S-n):
  - full UF2 validation: both magics and the end magic in every block,
    `block_no < num_blocks`, the same `num_blocks` in every block, payload
    size ≤ 476, a known family id (RP2040 `0xe48bff56`, RP2350 ARM-S
    `0xe48bff59`, ARM-NS `0xe48bff5b`, RISC-V `0xe48bff5a`, absolute
    `0xe48bff57`), target addresses inside the XIP flash window;
  - drives are filtered by `INFO_UF2.TXT` (`Board-ID` / `Model` of an
    RP2040 or RP2350 bootloader), and `INFO_UF2.TXT` is checked again
    right before writing;
  - the image is written with a single `write_all` and `sync_all`, and
    refused up front when the drive's free space is short;
  - tests cover an RP2040 image, an RP2350 image, a truncated file, a wrong
    family, a block out of order, and garbage
    (`proptest_uf2_inspect_never_panics`).
- **Parity:** 3.0 and 3.3, "UF2 validation / flash writer".

**C12: Curves and stateless helpers.** Size 0.5. (Merged in wave 1.)
- **Owns:** `crates/dspi-core/src/curves.rs`, `crates/dspi-core/src/curves_ref.rs`
  (frozen reference data from the old core).
- **Also provides** `quantize_gain` and `quantize_delay` (public in
  `curves.rs`), the old core's exact behaviour without clamping: C3/C4
  clamp a gain **before** quantizing and a delay **after**, as the old core
  did.
- **Implements:** every function in that file.
- **Inputs:** the old `dsp_math.rs`, and `dspi_proto::dsp`. Prefer
  `dspi_proto::dsp`, and keep the 201-point grid.
- **Done when:** values match the old core's to within 1e-6 dB on its test
  bands, with its tests ported.
- **Parity:** none (the C ABI needs it).

**Q1: The C ABI's records and header.** Size 1.
- **Owns:**
  - `crates/dspi-capi/src/types.rs`;
  - `crates/dspi-capi/build.rs`;
  - `crates/dspi-capi/cbindgen.toml`;
  - `crates/dspi-capi/include/`;
  - additions to `crates/dspi-capi/Cargo.toml` (cbindgen as a
    build-dependency).
- **Implements:**
  - the `#[repr(C)]` structs of the old ABI, with identical layout:
    `DspState`, `FilterParams`, `SystemStatus`, ... and the `notify`,
    `rta`, `siggen` and `stats` records;
  - `From<&dspi_core::records::X>` conversions;
  - cbindgen writing `include/dspi_core.h`, keeping the old `build.rs`'s
    workaround for cross builds.
- **Done when** (gate 1, B3):
  - a test asserts `size_of`, `align_of` and `offset_of` against the old
    crate's types for every shared struct;
  - a test diffs the struct and constant definitions of the generated
    `include/dspi_core.h` textually against
    `crates/dspi-core-linux/include/dspi_core.h`: equal apart from
    additions (optional on top: a C `_Static_assert(offsetof(...))` file
    compiled with `cc`);
  - note for Q2a: `dspi_get_state` returns a pointer to a `DspState`
    (12,812 bytes) cached per handle, refreshed on each call.
- **Parity:** 3.0 "FFI", C side.

### Wave 2a (needs wave 1 merged)

Wave 2 is split (gate 1, B2) so the connect path lands before the WPs that
build on it: 2a is C1, C2, C13, T3 and S5; 2b is everything else.

Every `dspi-core` WP:
- tests against `FakeDevice` (T4), attached with `DspiCore::attach`
  (bare) or, from wave 2b, opened with `open_transport` / `open_demo`;
- writes the mirror with C0's codec;
- writes only through `Inner::writable_conn` (a `Writable`, §1.2).

Facts from wave 1 that the setters (C2, C3, C4, C6) must respect:
- `DspState.spdif_inputs_enabled` bit 0 (S/PDIF input 1) always stays set
  when a setter writes the mask;
- a Linkwitz Transform's `qp` is clamped to 0.1..20 and a NaN is refused
  (`InvalidArgument`), as C0's codec expects;
- gains are clamped before `curves::quantize_gain`, delays after
  `curves::quantize_delay` (C12), as the old core did;
- a master-volume write is answered first by the legacy v1 volume packet,
  then by the `PARAM_CHANGED`, on the firmware and on `FakeDevice`;
  notification handling must expect both.

`FakeDevice` after wave 1 (T4, T4b): `FakeOptions` has `extensions`,
`ack_unknown_sets` (ACK unknown OUT requests as the firmware does on USB)
and `max_transfer`; `FakeRemote` has `poke`, `push`, `set_status`,
`load_image`, `image` and `disconnect` for tests. The demo answers every
`probe_features` read except the signal generator and the analyser (T4b;
its review may add small T4 fix-ups in parallel with wave 2a; they touch
`fake.rs` only).

**C1: Devices and transports.** Size 1.
- **Owns:** `crates/dspi-core/src/device.rs`, and `attach_probed` in
  `crates/dspi-core/src/lib.rs` (only that function).
- **Implements:**
  - discovery through nusb: VID `2E8B` (and the legacy VID), PID `FEAA`,
    and the count of DSPis it cannot open;
  - `select_device`, `open_fd` (dup the fd, §T1), `open_transport`,
    `open_demo`, `open_midi` (returns `Arc<MidiInputHandle>`) and
    `disconnect`;
  - `preamp_max_db`.
- **Connect sequence:** `attach_probed` (§1.2), switched to
  `probe_any_with_image` (S2; one image read serves probe and mirror),
  setting `wire` from its read mode, `state::decode_bulk` (C0) and
  `ExtState::probe` (S3, called inside `attach_probed`). Un-ignore
  `wp_c1_attach_probed_opens_the_demo_device`.
- **After wave 1** (review notes):
  - take a transport's `notifications()` **exactly once** (Link and MIDI
    allow one reader) and start the reader before anything else in the
    sequence can fail, so a failure cannot leave an orphaned subscription;
  - `open_fd`: refuse `fd < 0`, then
    `BorrowedFd::borrow_raw(fd).try_clone_to_owned()` (a `dup`; Kotlin keeps
    and closes its own), then `UsbTransport::from_fd`;
  - never surface `Notifications::is_disconnected()` after an intentional
    drop or re-attach (the old reader reports the drop as a disconnect;
    `FakeDevice` does too);
  - reset `Connection::echo` with the connection;
  - set `Connection::fresh_image` when the mirror was decoded from this
    connect's image (C2's `fetch_all` then skips re-reading platform and
    image, but still does the per-item reads);
  - unmappable images: install the connection read-only with `Wire::Raw`
    (§1.5, "Unmappable images");
  - close and displace paths stop the RTA worker with
    `rta::take_worker(&mut inner)`, then drop the guard, then
    `worker.stop()` (base hook; C9 fills it). The same pattern applies to
    the Link reader (C15).
- **Thread rules:** §1.2, with a test that reopening joins the old reader.
- **Deps:** T1, T2, T4, S2, S3, C0.
- **Parity:** 3.0 "FFI".

**C2: Connect, fetch, state.** Size 1.
- **Owns:** `crates/dspi-core/src/connect.rs`.
- **Implements:** `fetch_all`, `refresh_params`, `apply_all_params`,
  `fetch_status`, `get_state`, `max_delay_ms` and `clear_clips`.
  - `fetch_all` reads through `compat::read_bulk_any`. It sets `compat`,
    `wire_read` and the read-only mode (§1.5), then reads the per-item
    values.
  - `apply_all_params` refuses unless the connection is writable, and
    brackets its bulk write with `Connection::echo`
    (`bulk_write_started` / `bulk_write_finished(Instant::now(), ok)`).
  - Over Link, bulk reads are single-shot.
- **Deps:** C0, S2, T4.
- **Parity:** 3.2 "read-only mode for older firmware".

Also in wave 2a, described further down: **C13** (configuration and filter
files; it needs only C0), **T3** (midir ports) and **S5** (channel
clipboard).

### Wave 2b (needs wave 2a merged)

Notes for every wave-2b brief (after the wave-2a review):
- **Link device lock.** Multi-request flows take the transport's lock:
  `conn.transport().lock(reason, Duration::from_secs(60))` ... `unlock()`,
  as the Android console does: bulk restores, preset save/copy/import (C5),
  multi-step hardware flows (C6). A lock refusal is `DspiError::Locked`.
- **Refusals on the fake.** With `FakeOptions::ack_unknown_sets` a refused
  or unknown SET looks like success; tests assert a refusal by reading the
  value back, not by the SET's result.
- **C1b:** never call midir's `list_ports` faster than about once a
  second; hotplug diffs use an enumeration-only scan (no opening).
- **Desktop MIDI open** is `midir_port::list_ports()` + `midir_port::open()`
  + `DspiCore::open_transport` (no special API).
- **C2** consumes `Connection::fresh_image`; **C7** classifies echoes at
  `Notification::received`.

**C1b: Hotplug and reconnection.** Size 0.5.
- **Owns:** `crates/dspi-core/src/hotplug.rs`.
- **Implements:** `set_hotplug_callback`, `poll_hotplug`: arrivals and
  departures against the last scan, reconnect the selected serial when it
  reappears (through C1's `select_device`), close it when it departs.
- **Thread rules:** §1.2; listeners are called after the lock is dropped,
  with a test that a listener calling `scan_devices` does not deadlock.
- **Deps:** C1.
- **Parity:** 3.3 "auto-reconnect" (core side).

**C3: DSP parameters.** Size 1.
- **Owns:** `crates/dspi-core/src/params.rs`.
- **Implements:** its 37 methods. Each one:
  1. checks `writable_conn`;
  2. sends through S1's fast path;
  3. updates the record and `bulk_raw`.
- **Input:** the old core's `commands.rs`, for semantics (units, clamps,
  return codes).
- **Deps:** C0, S1, T4.

**C4: Effects parameters.** Size 0.5.
- **Owns:** `crates/dspi-core/src/params_fx.rs`.
- **Implements:** its 12 methods, under the same rules as C3.
- **Deps:** C0, S1, T4.

**C5: Presets.** Size 1.
- **Owns:** `crates/dspi-core/src/presets.rs`.
- **Implements:**
  - save, load, copy, delete and rename a preset;
  - the directory, startup mode, save and load params, factory reset;
  - the baseline and diff (`preset_dirty`, `preset_changes`).
- Load and factory reset run under the Link device lock.
- **Inputs:** the old `presetdiff.rs`; the session's `PresetSnapshot::diff`.
- **Deps:** C0, T4.

**C6: Hardware configuration.** Size 1.
- **Owns:** `crates/dspi-core/src/hardware.rs`.
- **Implements:** its 35 methods. Status bytes map to `Refused`. The Core 1
  conflict check uses the session's `core1_conflict`.
- **Deps:** C0, S1, T4.

**C7: Notifications and the monitor.** Size 1.
- **Owns:** `crates/dspi-core/src/notify.rs`,
  `crates/dspi-core/src/monitor.rs`.
- **Implements:**
  - `set_notify_listener`;
  - `process_notifications`:
    - returns the `NotifyResult` flags the old core returned;
    - applies the echo rule (§1.2, §1.4);
    - forces a bulk re-read on a sequence gap;
    - on a read-only connection, re-reads instead of patching (patch
      0003);
    - takes the drained packets out under the lock and calls nothing
      while holding it (§1.2 thread rules);
    - classifies echoes with `Connection::echo.classify_notification(
      caps.kind, session_id, &note)`, which uses the packet's
      `Notification::received` stamp, not the drain time (§1.2 bulk
      echoes);
  - `monitor_read` and `monitor_clear`.
- **Input:** the old `notify.rs` and `monitor.rs`.
- **Deps:** C0, T4.

**C8: Extension channel, names, diagnostics.** Size 0.5.
- **Owns:** `crates/dspi-core/src/ext.rs`, `crates/dspi-core/src/info.rs`.
- **Implements:**
  - the pair map, `ext_status` and the core vreg;
  - the input-source and device names, output latency and clock
    diagnostics, through the registry paths.
- **Deps:** S3, C0, T4.
- **Parity:** 3.2 "extension channel" (core side); "input-source names,
  device name, output latency, clock diagnostics".

**C9: Spectrum analyser.** Size 1.
- **Owns:** `crates/dspi-core/src/rta.rs`.
- **Implements:** `rta_*` on the session's `RtaEngine`:
  - a worker thread takes the core lock for each poll and wakes the
    `RtaListener`;
  - on `Stalled` (Link), it falls back from bands-all;
  - the records match the old ones;
  - §1.2 thread rules: the worker is stopped and joined with the lock
    released (on `disconnect` and on reopen), the listener is called after
    the guard is dropped, with a test.
- **Deps:** T4, C1.

**C10: Signal generator and statistics.** Size 0.5.
- **Owns:** `crates/dspi-core/src/stats.rs`.
- **Implements:** `siggen_*`, the statistics, and the S/PDIF RX status.
- **Deps:** T4.

**C13: Configuration and filter files.** Size 1.
- **Owns:** `crates/dspi-core/src/files.rs`.
- **Implements:** `config_export`, `config_import`, `config_inspect`,
  `filters_export`, `filters_inspect` and `filters_import`.
  - They keep the old core's JSON shapes, which the Qt app parses.
  - Use the session's `preset_file` and `filterfile` where their output is
    identical.
- **Tests:** against `tests/fixtures/*Filters.txt`, plus the old core's
  tests.
- **Deps:** C0.

**C14: Control surfaces for the editor.** Size 1.
- **Owns:** `crates/dspi-core/src/cs.rs`.
- **Implements:** `cs_fetch`, `cs_snapshot`, `cs_apply` and `cs_pin_uses`,
  on the session's CS model (S4).
  - The JSON ops are the old core's `cs.rs` ops, which the Qt
    `CsController` sends.
- **Deps:** S4, T4.

**T3: midir ports.** Size 0.5.
- **Owns:** the `midir_port` module of `crates/dspi-transport/src/midi.rs`,
  and nothing else in that file.
- **Implements:** `list_ports` and `open`, over ALSA, WinMM and CoreMIDI,
  matching ports named "DSPi Control"; an input and an output port are
  paired by midir id.
- **Done when:**
  - it builds on Linux and Windows GNU with `--features midir`;
  - its tests run without a device, using a fake at the pipe level.
- **Deps:** T2.

**S5: Move the channel clipboard into the session.** Size 0.5.
- **Owns:**
  - `crates/dspi-session/src/clipboard.rs`;
  - `crates/dspi-tui/src/screens/clipboard.rs`;
  - the import lines in `crates/dspi-tui/src/screens/{mod.rs,matrix.rs}`
    and `crates/dspi-tui/src/live.rs`.
- **Deps:** S4, because both touch the same TUI tree.
- **Parity:** 3.0 "channel clipboard".

### Wave 3

**C15: DSPi Link in the core.** Size 1.
- **Handles:** Link device handles are one byte on the wire; the API takes
  `u32` (binding-friendly) and refuses values over 255 with
  `InvalidArgument`.
- **Threads:** §1.2 rules apply to the Link reader and the `LinkListener`.
- **Owns:** `crates/dspi-core/src/link.rs`.
- **Implements:**
  - `link_connect`, `link_pair`, `link_devices`, `link_status`,
    `link_close` and `set_link_listener`;
  - `link_select_device`, which opens a `LinkTransport` through C1's
    `open_transport`;
  - the module's free functions.
- **Input:** the link part of the Android `dspi-ffi/src/core.rs`.
- **Deps:** T5, C1.

**Q2a: C ABI, part 1.** Size 1.
- **Owns:** `crates/dspi-capi/src/fns_device.rs`.
- **Implements:** the C functions for device, connect, params, params_fx,
  presets and curves. Each one:
  - runs in `guard`;
  - keeps the old name, signature and failure value (see
    `crates/dspi-core-linux/src/lib.rs`).
- **Deps:** Q1, C0–C5, C12.
- **Header:** as Q2b, do not edit `cbindgen.toml`'s `[export] include`.
- **Check:** from this wave, `ci/check.sh --step qt-capi` builds the Qt app
  against `dspi_capi` (CMake `DSPI_CORE_LIB`, already in the base); it
  passes once Q2a and Q2b are both merged (the reviewer runs it on the
  merged wave).

**Q2b: C ABI, part 2.** Size 1.
- **Owns:** `crates/dspi-capi/src/fns_misc.rs`.
- **Implements:** the C functions for hardware, notify, monitor, RTA,
  siggen/stats, files and CS. It also adds new functions for the extension
  channel, names, diagnostics, firmware and the demo device; these are
  additions to the header.
- **Deps:** Q1, C6–C11, C13, C14.
- **Header:** Q2a and Q2b must not edit `crates/dspi-capi/cbindgen.toml`'s
  `[export] include` list (Q1's); a function or type the header needs is
  exported by its own `#[unsafe(no_mangle)] pub extern "C"` / `pub`
  declaration.

**R1: The terminal on every transport.** Size 1.
- **Owns:** `apps/terminal/src/{main.rs,doctor.rs}`, and the connect code
  in `crates/dspi-tui/src/shell/`. List the files touched in the report.
- **Implements:**
  - `--demo` (FakeDevice), `--midi [PORT]` (midir) and
    `--link HOST[:PORT]`;
  - the ext probe shown on the overview line;
  - the echo rule;
  - auto-reconnect after a replug.
- **Deps:** T2, T3, T4, T5, S3.
- **Parity:** 3.3 "probe on connect" (probe side), "auto-reconnect", "Link
  client" (core side).

**F1: Binding readiness (UniFFI).** Size 1.
- **Owns:**
  - a `uniffi` feature in `crates/dspi-core/Cargo.toml`;
  - `cfg_attr(feature = "uniffi", ...)` derives in `records.rs`, `error.rs`
    and `callbacks.rs`, and the exports on `DspiCore`;
  - a new crate `crates/dspi-ffi`: a cdylib with the UniFFI scaffolding, on
    UniFFI 0.29.x as Android pins, with `init_logging()` (Android logcat,
    stderr elsewhere);
  - `missing_docs` on in `dspi-core` and `dspi-capi`, with every record
    field documented;
  - mobile size (gate 1 nit): `dspi-core` depends on `dspi-session` with
    `default-features = false`, and `dspi-session`'s dependency on
    `dspi-cmd` (undo rendering) and the bundled AutoEQ blob become optional
    features, so the mobile libraries carry neither. The Terminal keeps them
    on. This touches `crates/dspi-session/Cargo.toml` and its `undo.rs`
    gating.
- **Test:** runs `uniffi-bindgen` for Kotlin and Swift in the container,
  and compiles the output where it can (Kotlin with `kotlinc`, if the image
  has it; otherwise generation only).
- **Deps:** all of C0–C15.
- **Parity:** prerequisite of steps 4 and 5.

### Wave 4

**Q3: Switch the Qt app and delete the old core.** Size 1.
- **Owns:**
  - `apps/desktop/qt/CMakeLists.txt`: the defaults become
    `DSPI_CORE_DIR=crates/dspi-capi` and `DSPI_CORE_LIB=dspi_capi`. The
    library keeps the name `dspi_capi` (gate 1, S-j: renaming it to
    `dspi_core` collides in `target/`); the header stays `dspi_core.h`;
  - `apps/desktop/qt/src/*.{cpp,h}`, only where the ABI changed;
  - the build scripts under `apps/desktop/packaging/**`: `-p dspi-capi`,
    and the library file names (`dspi_capi.dll`, `libdspi_capi.a`, the
    macOS `lipo` line, `deploy-mingw.sh`);
  - `ci/check.sh` (the qt step and the exclusions);
  - `crates/dspi-capi`: delete Q1's temporary
    `wp_q1_snapshot_matches_the_old_crate` test and the dev-dependency it
    needs (the old header then lives only in
    `tests/data/dspi_core_old.h`);
  - the workspace `Cargo.toml` members;
  - deleting `crates/dspi-core-linux`.
- **Done when:**
  - the Qt 6 build runs offscreen on the demo device (an app flag or an
    environment variable; the WP picks one and reports it);
  - Qt 5 builds;
  - `build-windows.sh` (Windows GNU) builds;
  - the `windows-msvc` step builds `dspi_capi.dll`;
  - the macOS VM check runs if the coordinator schedules it (one heavy job
    at a time; GUI through `launchctl asuser`).
- **Deps:** Q2a, Q2b, F1, R1.

**D1: Docs.** Size 0.5.
- **Owns:** `README.md`, `docs/PORTABILITY.md`, and section 4 of this file.
- **Implements:** the layout, builds, crates and licences after the merge,
  and the updated parity table.
- **Deps:** Q3.

### Later work (after wave 4)

**P2: Shared protocol tables in `dspi-proto`.** Size 0.5.
- A public `LIVE_BANDS` constant (the 10 bands the consoles show of the
  firmware's 12), replacing the hand constants in `FakeDevice` and the core.
- `dspi_proto::xover` made f64-exact, with its Bessel table public, so C12
  deletes its duplicate (`curves.rs`) and computes crossovers through proto.
- **Done when:** C12's parity tests still hold (mag within 1e-6 dB).

**Firmware (owner decision, Bofr5555/DSPi):** `bulk_params_apply(.., source)`
so `BULK_INVALIDATED` carries `PARAM_SRC_MIDI`/`HOST_SET`/`UART`; the echo
filter then only serves older firmware.

### Wave summary

| Wave | WPs (run in parallel) |
|---|---|
| 1 | P1, S1, S2, S3, S4, T1, T2, T4, T5, C0, C11, C12, Q1 |
| 2a | C1, C2, C13, T3, S5 |
| 2b | C1b, C3, C4, C5, C6, C7, C8, C9, C10, C14 |
| 3 | C15, Q2a, Q2b, R1, F1 |
| 4 | Q3, D1 |

Within a wave, no two WPs own the same file. Shared files (§1.10) get
additive edits only.

### Review gates

1. **Before wave 1:** a review of the plan, and an adversarial review of the
   design and the base.
2. **After each wave:**
   - the full `ci/check.sh` on the merged result, plus `--step android`,
     `--step wasm` and `--step windows-msvc` from wave 2 on;
   - a code-quality review against §1.8;
   - an adversarial review of the new code against §1.7.

   Fix-ups go back to the WP's branch.
3. **Before `core-merge` goes to `main`:** a final full pass with
   `ci/check.sh --all`.

---

## 3. Parity rows covered (FEATURE-PARITY.md, section 3)

| Parity row | WP |
|---|---|
| 3.0 FFI over `dspi-session` | C0–C15 (API), Q1/Q2a/Q2b (C), F1 (UniFFI) |
| 3.0 TUI logic GUIs need: CS editing model, channel clipboard | S4, S5. The bootloader watch is C11 and curve sampling is C12. |
| 3.0 / 3.2 / 3.3 Read-only decoder for older wire formats | base (`wire_compat`), S2, C2, C7 |
| 3.0 / 3.3 EXT_LIST probe on connect, ext state in the device model | base (codecs), S3, C1, C8, R1 |
| 3.0 / 3.2 / 3.3 `LinkClient` as a Transport | T5, C15, R1. mDNS browse is not in scope. |
| 3.0 USB MIDI SysEx transport | base (`frame`), T2, T3 |
| 3.0 / 3.2 Stateful demo device | T4, C1 (`open_demo`), R1 (`--demo`) |
| 3.0 / 3.3 UF2 validation and flash writer | C11; bootloader entry is C6 |
| 3.2 Input-source and device names, output latency, clock diagnostics | C8, Q2b |
| 3.3 Auto-reconnect | C1 (core), R1 (terminal) |
| 3.0 AutoEQ fetch injectable | not in scope |

---

## 4. Status

- **Base** (gate 1): see the commit history up to `4f3e391`.
- **Wave 1** merged at `8b92066`: P1, S1, S2, S3, S4, T1, T2, T4, T5, C0,
  C11, C12, Q1, each reviewed and fixed; full CI green (1569 tests).
- **Base after wave 1** (this commit and the two before it): bulk-echo
  filter and the revised echo rule, kind-aware `with_busy_retry`,
  `Reply::Error` in the mock, MIDI using `Frame::check_len`,
  `Connection::echo`, record docs, plan notes for wave 2.
