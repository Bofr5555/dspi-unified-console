# The installer's PIN, the lock and the mode manifest

*WP P1LK (PRO1 S4 and U7). Owner decisions OD-P1c, OD-P1d, OD-P1e (2026-11-13). The design is
[`docs/UI-MODES.md`](../UI-MODES.md); the profile the lock lives in is [limits-profiles.md](limits-profiles.md).*

## The rule that comes first

With **no installer PIN set** for a device, nothing is locked: every mode is reachable, nothing asks for a PIN, and
every write passes as it does without this feature. A console never invents a lock.

## The PIN and the locked-user state

| State | When | Writes | Installer pages, profile, PIN |
|---|---|---|---|
| No PIN | the default | all (within the limits) | open to anyone |
| PIN set, unlocked | after `installer_pin_set` or `installer_unlock`, this session | all (within the limits) | open |
| PIN set, locked, nothing covered | `lockedPages` empty | all (within the limits) | need the PIN (`pin_required`) |
| **Locked user** | PIN set, `lockedPages` not empty, not unlocked | only `vol.master`, `vol.user`, `vol.mute`, `out.mute`, `preset.load`, within the limits | need the PIN |

- The locked-user state refuses every other write **before anything is sent**: `DspiError::InstallerLock { code:
  "locked" }`, notice key `installer_lock.locked`. The check sits where every write to the device starts, so a typed
  setter, `write_param`, an undo or a redo (refused as `locked`; an undo group is undone whole or not at all), an
  import, a snapshot recall, an A/B toggle, a flash save, a factory reset and the signal generator are all refused
  alike. A preset slot load passes, and the limits hold after it (it only narrows). A revert of a guarded apply passes
  too. **The analyser runs** while locked (its configuration push and STOP change no audio; decided in WP P1DKFIX),
  on the hub as well.
- What the lock covers is the profile's `lockedPages` (screen ids of the manifest); `installer_lock_cover(pages)`
  sets it. The write check does not depend on which pages: a covered lock allows only the writes above.
- **Unlocking lasts for the session** of this core, per device (a reconnect keeps it), until `installer_lock()`.
- **The lock is on this console, not on the DSPi** (OD-P1d). The firmware has no store for it: the same DSPi on
  another console, a control surface or the UART is open. The unlock dialog shows the notice
  `installer_lock.scope` in one line. It is a UI lock, not a security boundary; on a hub, the hub's roles are.
- **Repeated wrong PINs** (`wrong_pin`): 3 are free; each further one opens a refusal window of 30 s, doubling up to
  15 min. The window is the next allowed time on the core's clock seam, never a sleep: an attempt inside it is refused
  at once (`wait`, notice `installer_lock.wait`) and `installer_lock_state().retryAfterMs` says how long. A good PIN
  resets the count. With a limits folder named the count and the window are kept beside the PIN
  (`limits.tries.json`, the window's end on the wall clock, never trusted past 15 min), so restarting the app does not
  reset them; an unreadable count file leaves no free try. The hub uses the same rule.
- A PIN has 4 to 64 characters, with no space at either end. Checking one takes 0.1 to 1 s (PBKDF2): call the PIN
  calls off the UI thread.

## A forgotten PIN (OD-P1c)

1. **On a hub** an Admin clears it (P1HL).
2. **The recovery code**, shown once by `installer_pin_set` (`XXXXX-XXXXX-XXXXX-XXXXX`, 100 bits), for the hand-over
   note (notice `installer_lock.recovery`). `installer_unlock` takes it in place of the PIN (case, spaces and dashes
   do not matter); `lockState.recovered` is then true and the client should ask for a new PIN. Setting a new PIN makes
   a new code; the old one no longer opens it.
3. **The last resort:** delete the device's folder under the limits folder (`s-<serial>` or `x-<hex>`, see
   [limits-profiles.md](limits-profiles.md)). That removes the profile, its good copy, the PIN and the recovery code.

There is no "Remove lock" button: the locked user could press it too.

## The files

Beside the profile, in the device's folder, written atomically:

| File | What |
|---|---|
| `limits.pin.json` | the PIN's hash: `{version, kdf: "pbkdf2-sha256", iterations, salt, hash}` |
| `limits.recovery.json` | the recovery code's hash, the same shape |

| `limits.tries.json` | the wrong PINs since the last good one and the refusal window's end (ms since the epoch) |

**Naming another limits folder keeps the lock.** `limits_set_dir` is refused (`pin_required`) while the open device's
PIN is set and this console is not unlocked, so a locked user cannot name the lock away. Named by the installer, every
PIN and profile in force stays in force: kept in memory with no folder (""), written to the new folder where it holds
none for that device. Name the folder at start-up, before opening a device; with no device open the call is never
refused.

A PIN file that does not read counts as a PIN set (fails closed): the device stays locked, `lockState.detail` says
why, and only the recovery code or the folder's deletion opens it. With no limits folder named, the PIN is kept for
the core's lifetime. **On the web** no PIN is set (`installer_pin_set` is `unsupported`: no files and no random
source in the core); the hub keeps a managed device's lock.

## A device on a hub: one lock, the hub's

A hub (P1HL) keeps its own installer PIN per device and judges every Control client's writes against it over DSPi
Link (`docs/integrator/hub.md`). On a device reached through a hub, **the core defers to the hub's lock** so the user
sees one lock and one prompt:

- `installer_lock_refresh()` reads the hub's `limits.get` and `mode.pinned` for the bound device (a round trip: off
  the UI thread) and returns the lock state. Call it after binding a hub device, after a reconnect and when the hub
  sends `limits.changed` or `mode.changed`. With no hub session, or a hub without the `limits` cap, it asks nothing.
- When the hub holds a PIN for the device, `LockState.heldBy` is `hub`: `pinSet`, `locked`, `unlocked`, `covers`,
  `wrongTries` and `retryAfterMs` are the hub's for this session (an Admin is always unlocked), `scopeNoticeKey` is
  `installer_lock.hub`, and `hubPinnedMode` is the mode the hub pins for this role (also in `mode_pinned()`).
  `installer_unlock(pin)` sends the PIN to the hub (`wrong_pin`, `wait` as above, with the hub's window) and
  `installer_lock()` locks it there. This console's own PIN for that device asks nothing and refuses nothing meanwhile:
  the hub refuses over Link, and its refusal reaches the client as `InstallerLock { code: "locked" }`.
- Otherwise `heldBy` is `console` (this console's PIN) or "" (none). A console PIN still applies to the device when it
  is attached over USB, and on a hub that holds no PIN for it.

The hub's other named refusals keep their notice too: a limit is `LimitExceeded { code: <the limit> }`, an image a
Control client may not write is `DspiError::ImageRefused` (`image_refused`), a write the hub could not judge because
it could not read the device is `DspiError::LimitsUnknown` (`limits_unknown`).

## Modes

`mode_manifest()` is `assets/modes/modes.json` as a typed record: the modes (Simple, Advanced, Installer) with their
default platform classes (`phone` Simple; `desktop` and `web` Advanced), the locked-user state (its screens, its
Settings and its writes), the screens in order (`layer: "pro"` for a Pro screen a client shows once built) and the
operations per mode. A client renders what it is given.

`mode_resolve(platform, chosen)` decides the mode a client shows, in this order:

1. the locked-user state: `locked_user`, `reason: "locked"`, no switch;
2. a mode the hub pins (`mode_set_pinned`, from P1HL's `mode.pinned`): `reason: "pinned"`, no switch;
3. the user's choice (`chosen`, kept per device by the client): `reason: "chosen"`;
4. the platform class's default: `reason: "default"`.

When the mode asked for is the Installer, a PIN is set and the console is not unlocked, `needsPin` is true and the
mode is the default meanwhile; with no PIN set `needsPin` is never true. `openModes` lists the modes the switch offers
without a prompt; `screens`, `firstScreen` and `operations` are the chosen mode's for that platform class. A mode
never writes and never grants a permission. Qt keeps its Window > Mode… entry hidden and reaches the Installer pages
from Settings > Installer (OD-P1e).

## The calls

| Rust / UniFFI | C | Web (`call`) |
|---|---|---|
| `mode_manifest() -> ModeManifest` (free function) | `dspi_mode_manifest` | `modeManifest` |
| `mode_resolve(platform, chosen) -> ModeChoice` | `dspi_mode_resolve` | `modeResolve {platform, chosen?}` |
| `mode_set_pinned(mode)` ("" clears), `mode_pinned()` | `dspi_mode_set_pinned` | `modeSetPinned {mode}`, `modePinned` |
| `installer_lock_state() -> LockState` | `dspi_installer_lock_state` | `installerLockState` |
| `installer_unlock(pin) -> LockState` | `dspi_installer_unlock` | `installerUnlock {pin}` |
| `installer_lock() -> LockState` | `dspi_installer_lock` | `installerLock` |
| `installer_pin_set(pin) -> InstallerPin {recoveryCode, state}` | `dspi_installer_pin_set` | `installerPinSet` (unsupported) |
| `installer_pin_clear() -> LockState` | `dspi_installer_pin_clear` | `installerPinClear` |
| `installer_lock_cover(pages) -> LockState` | `dspi_installer_lock_cover` (a JSON array) | `installerLockCover {pages}` |
| `installer_lock_refresh() -> LockState` | `dspi_installer_lock_refresh` | `installerLockRefresh` |

`LockState`: `serial, pinSet, unlocked, locked, installerOpen, covers, allowedWrites, wrongTries, retryAfterMs,
recovered, note, contact, scopeNoticeKey, detail, heldBy, hubPinnedMode`. Errors: `DspiError::InstallerLock { code, msg }` with `code`
`locked`, `pin_required`, `wrong_pin`, `wait` or `no_pin`; C and the web reply `"code": "installer_lock"` and the
notice key `installer_lock.<code>`.

## From a script: `uconsole lock` (WP P1TM)

| Command | Core call |
|---|---|
| `uconsole lock [status] [--json]` | `installer_lock_state()` (`--json`: the `LockState`) |
| `uconsole lock unlock` | `installer_unlock(pin)`: checks the PIN or the recovery code |
| `uconsole lock set-pin` | `installer_pin_set(pin)`; prints the recovery code once, with `installer_lock.recovery` |
| `uconsole lock clear-pin` | `installer_pin_clear()` |

One command is one session of the core, so an unlock lasts for that command:
`limits set`, `limits clear`, `lock set-pin` and `lock clear-pin` ask for the PIN
themselves when one is set (no PIN set: nothing asks). The PIN comes from the
terminal, `DSPI_INSTALLER_PIN`, or `--installer-pin -` (a line of standard
input); a new PIN from two prompts, `DSPI_NEW_INSTALLER_PIN` or `--new-pin -`
(the next line). Exit codes: 0 done, 1 no PIN to be had, 3 a wrong PIN or the
refusal window (`retryAfterMs`). The Terminal's own folder is
`<local data dir>/uconsole/limits`, so a PIN set there locks that console only.

```sh
printf '%s\n%s\n' "$OLD" "$NEW" | uconsole lock set-pin --installer-pin - --new-pin -
DSPI_INSTALLER_PIN="$OLD" uconsole lock clear-pin
```
