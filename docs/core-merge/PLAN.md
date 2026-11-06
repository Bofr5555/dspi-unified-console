# One Rust core: the merge

*Branch `core-merge`, started 2026-11-06.* This merges the repository's two
Rust protocol cores into one:

- **Terminal crates** (`dspi-proto`, `dspi-transport`, `dspi-session`):
  nusb, constants generated from the firmware headers, GPL-3.0-or-later.
  They are the base.
- **Desktop core** (`crates/dspi-core-linux`, package `dspi-core`): rusb,
  hand-copied constants, a ~150-function C ABI used by the Qt app. It is
  retired into the base.

Background: `dspi-mobile/docs/RUST-CORE-CONSOLIDATION.md` (the study).

The owner's order of work:
1. Windows (done);
2. parity audit;
3. **this merge**;
4. Android's `dspi-ffi` onto the merged core;
5. the iOS app onto it, through UniFFI Swift.

The design aims to make 4 and 5 thin wrappers, and to keep a later
web/wasm build possible.

## Layout

```
crates/
  dspi-proto      pure protocol. No I/O, no threads.
                  generated constants (bofr-main headers), bulk codec (V32),
                  wire_compat (V16..V33 read-only, normalised to V32),
                  ext (extension channel codecs), frame (UART frame,
                  CRC16, MIDI SysEx 7-bit packing and the decoder)
  dspi-transport  the one Transport trait, TransportCaps, TransportError
                  backends: usb (nusb; from_fd on Android/Linux),
                  midi (sans-IO SysEx client + blocking driver over a pipe;
                  midir ports behind feature `midir`), link (DSPi Link adapter,
                  feature `link`), mock (scripted), fake (demo device)
  dspi-link       DSPi Link client (WebSocket), unchanged
  dspi-session    state machine + protocol operations: probe, DeviceState
                  (the bulk image is the mirror), notifications with origin
                  attribution, writes (confirmed, and a fast path), RTA engine,
                  control surfaces, presets, filter files, AutoEQ
  dspi-core       the embedder API: one object for a GUI or a binding.
                  Device manager (scan, select, fd, transport, hotplug),
                  notify/RTA threads, the console operations as plain Rust
                  records (DspState, ...), JSON features
  dspi-capi       C ABI over dspi-core for the Qt app: the old function names
                  and repr(C) records, header include/dspi_core.h, library
                  name dspi_core
  dspi-cmd, dspi-tui, apps/terminal   unchanged consumers of the session
```

Later on top of `dspi-core`:
- `dspi-ffi`, UniFFI: Kotlin for Android, Swift for iOS;
- `dspi-wasm`, over proto and session with an async driver.

## The Transport trait

Terminal's shape is kept: `&mut self`, `control_in` / `control_in_upto` /
`control_out`, `max_transfer`, and `notifications()` returning a separate
`NotificationSource`. Android patch 0004's Link needs are added as
default methods, so existing implementors are unchanged:

| Method | Default (USB) | Used for |
|---|---|---|
| `caps() -> TransportCaps` | kind Usb, `chunked_bulk: true` | bulk reads and writes use single-shot 0xA0/0xA1 when false (Link spec 8.2) |
| `session_id() -> Option<u16>` | `None` | echo attribution (below) |
| `lock(reason, timeout)` / `unlock()` | no-op `Ok` | Link device lock around multi-step flows (spec 7.7) |
| `NotificationSource::read_attributed` | `(read(), origin None)` | Link and MIDI supply the origin |

Errors stay one `TransportError`, with no backend types inside. Link
statuses map onto it:

| Link status | `TransportError` |
|---|---|
| ERROR, BLOCKED | `Stalled` (the RTA bands-all fallback depends on this) |
| TIMEOUT | `TimedOut` |
| NO_DEVICE | `Disconnected` |
| DENIED | `Denied` |
| LOCKED | `Locked` |
| others | `Remote` |

**Echo attribution** (0004's rule): a notification whose source is
`HOST_SET` is our own echo when:
- the transport has no session id (USB: every host write is ours, as
  before);
- or its origin equals our session id (Link). Origin 0 is never ours.

`dspi-session` stores the origin on each `Notification` and decides this
in one function.

**Threads.** The desktop core shared one libusb handle between the GUI
thread, the notify listener and the RTA worker. The merged core gets the
same effect without `Sync` on the trait:
- the control path is one `Mutex<Box<dyn Transport>>` in `dspi-core`; it
  also serialises chunked bulk sessions, which any other request aborts on
  the device;
- notifications come from the separate `NotificationSource`.

## Sans-IO

What is pure today, and stays pure:
- `dspi-proto`, all of it;
- `DeviceState::apply(notification)`;
- notify decoding;
- the RTA engine's `accept_*`;
- the MIDI client: `MidiClient::request(..) -> bytes` and
  `MidiClient::receive(bytes) -> events`. It never touches a port. The
  blocking `MidiTransport` drives it over a `MidiPipe` that the app
  supplies (Kotlin `MidiManager`, Swift CoreMIDI) or that midir supplies
  (ALSA, WinMM, CoreMIDI).

The session's operations are written against `&mut dyn Transport` and block.
An async driver for wasm means either an async twin of the trait or a
request/reply split of those operations. Both build on the pure parts
above and are not part of this merge.

## The state mirror

The device's bulk image (`BulkPacket`) is the single source of truth:
- connect reads it;
- notifications patch it at their offsets;
- writes patch it once sent.

`dspi-core`'s `DspState` (the record the GUIs read) is a view decoded from
the image, plus the few values the image does not carry (preset directory,
Core 1 mode, DAC mute status, control interfaces). It is checked against
`dspstate_v32.json` (the Android fixture: the old core's decode of a real
capture).

Writes from a GUI happen at slider rate, so `dspi-core` uses the session's
fast path: encode, send, patch the mirror. It skips the Terminal CLI's
confirming read-back and undo journal.

## Wire formats

V32 is read and written. V16..V31 and newer images are read best-effort
for display (`wire_compat`, ported from Android patch 0003, design in
`dspi-mobile/docs/WIRE-FORMAT-COMPAT.md`). Such a state is never written
back, and the compatibility verdict stays TOO_OLD or TOO_NEW.

## Extension channel

`REQ_EXTENSION` 0x00 with the sub-command in `wValue`:
- `EXT_LIST` 0x0000;
- `EXT_CROSSFEED_PAIR_MAP` 0x0001;
- `EXT_CORE_VREG` 0x0002.

Spec: `DSPi-bofr/Documentation/Features/extension_channel_spec.md`. A stall
means "not supported", never an error worth retrying.

## Parity rows covered (FEATURE-PARITY.md, section 3)

The core-side rows of `dspi-mobile/docs/FEATURE-PARITY.md` (2026-11-06)
and where each one lands:

| Parity row | Lands in | Milestone |
|---|---|---|
| 3.0 FFI over `dspi-session` | `dspi-core` (embedder API) + `dspi-capi`; UniFFI later on `dspi-core` | 2–3 |
| 3.0 TUI logic GUIs need: control-surface editing model (`cs_model.rs`), channel clipboard | moved from `dspi-tui` to `dspi-session` | 2 |
| 3.0 / 3.2 / 3.3 Read-only decoder for older wire formats | `dspi-proto::wire_compat`; display-only state in session and core | 1 (codec), 2 (state) |
| 3.0 / 3.3 EXT_LIST probe on connect; pair map and core voltage in the device model | `dspi-proto::ext`; probe and `DeviceState` fields in session; C ABI accessors | 1 (codec), 2 (probe/state), 3 (C ABI) |
| 3.0 / 3.2 / 3.3 `LinkClient` as a Transport | `dspi-transport::link` (feature `link`); mDNS browse is not in scope | 1 |
| 3.0 USB MIDI SysEx transport | `dspi-proto::frame` + `dspi-transport::midi` (pipe; midir on desktop) | 1 |
| 3.0 / 3.2 Stateful demo device | `dspi-transport::fake`: writes patch its image, it notifies, and it answers the extension channel | 1 |
| 3.0 / 3.3 UF2 validation and flash writer | `dspi-core::firmware`: UF2 checks, bootloader-drive copy, bootloader entry | 3 |
| 3.2 Input-source and device names (0x54–0x57), output latency, clock diagnostics (0x03, 0x82) | already in session; exposed through `dspi-core` and new C ABI functions | 3 |
| 3.0 AutoEQ fetch injectable | not in scope (UI side, AutoEQ) unless trivial | — |

## Status

Milestones and their results are recorded at the end of this file as they
land.
