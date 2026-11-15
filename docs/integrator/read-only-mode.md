# The client read-only mode

A client asks the core to refuse every write, for watching a unit that must not
change and for testing against one (WP ROMODE). Reference:
`crates/dspi-core/src/read_only.rs`.

**It is a client-side diagnostic, not a security boundary.** The client turns
it on and can turn it off; another client of the same device, or of the same
hub, is not bound by it. What may change a device is decided by its firmware,
a hub's roles, limits and lock (DSPi Link), and the installer's lock. Use those
to protect an installation, never this flag.

## Turning it on

| From | Call |
|---|---|
| Rust, Kotlin, Swift | `set_client_read_only(on)` / `setClientReadOnly(on:)`; `client_read_only()`, `client_read_only_refusals()` |
| C | `dspi_set_client_read_only(core, on)`, `dspi_client_read_only(core)`, `dspi_client_read_only_refusals(core)` |
| Web binding | `setClientReadOnly {on}`, `clientReadOnly` → `{on, refusals}` |
| Terminal | `uconsole --read-only ...` (any command, any target) |
| Swift console | `-ReadOnly YES` for every device through the core; `-MIDIReadOnly YES` for a DSPi on USB MIDI (the core's MIDI link; the Swift engine's own filter only under `-CoreTransport NO`) |
| Web console | `?readonly` (and a fleet's view access): the page's sessions turn it on |

It belongs to the core, not to a connection: it stays across a disconnect and
a reconnect, for every device the core opens, until it is turned off.

Turning it **on** is refused while a guarded apply (a safe apply) waits for Keep
or Revert, since its way back would be refused too and leave the device on the
trial: `InvalidArgument` with the `read_only.guard_pending` row
(`notice_for_error`; C returns false, the web call fails with that `noticeKey`).
Keep or revert first. Turning it off always works.

## What it refuses

Everything that writes, before anything is sent: every typed setter and
`write_param`, the gated `*_confirmed` calls (refused before the token is
checked, so the token is not spent), imports, snapshot recalls, A/B, undo and
redo, the guarded apply, the whole-image write, control surfaces, the core
voltage, identify, the signal generator, the audio link's setters, a firmware
flash and the bootloader. On the raw path every `raw_set`, and every `raw_get`
but the registry's pure reads (`dspi_proto::registry::is_pure_read`: the
firmware's `GET` table and the extension channel's IN side), so even the
housekeeping a read-only connection may still send raw (clear clips, the stats
resets) is refused there. Over DSPi Link, a hub request goes out only when it
reads (`HUB_READS`: `limits.get`, `mode.pinned`, `apply.status`, `system.get`,
`system.status`, `system.alarms`).

A refusal is `DspiError::ReadOnly` (C and web code `read_only`) with the notice
key `read_only.client`: `DspiCore::notice_for_error`, and the C ABI's and the
web binding's failure replies, give that row while the mode is on. It counts in
`client_read_only_refusals`, not in `read_only_refusals` (a read-only
*connection*'s, older or newer firmware).

## What goes on

Reads, notifications (another console's change reaches the mirror), the meters,
the typed analyser (its configuration push and STOP change no setting), and the
§1.2 housekeeping calls `clear_clips` and `reset_buffer_stats`. Local files are
not the device's: snapshot capture and exports work.
