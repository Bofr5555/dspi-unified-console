# Limits profiles

PRO1 S1, built in WP P1LM. An installer narrows the ranges a system may be
driven through, per device, so the people who use it afterwards cannot push
it past what it was set up for. The core enforces it; each client adds a
Limits page in Installer mode (`docs/UI-MODES.md`).

**No profile, no change.** A device without a profile behaves as it always
has: nothing is checked, nothing is shown. The system runs fully without an
installer PIN when none is set.

**What a host-side limit cannot stop.** The DSPi's firmware does not know the
profile. A control surface, the UART, or a console that does not hold the
profile can still go past it (FW2, a firmware-side clamp, closes this later).
The Limits page says so in one line: the notice `limits.scope`.

## The profile

One per device, by serial. It is a document of the `.dspipreset` family
([document-format.md](document-format.md)) with `meta.kind = "limits"` and a
`limits` block:

```json
{
  "schemaVersion": 1,
  "meta": { "kind": "limits", "name": "Limits profile" },
  "limits": {
    "serial": "E6614103E7...",
    "outputs": [
      { "output": 2, "maxGainDb": 3.0, "highpassMinHz": 1500.0,
        "highpassMinSlopeDb": 24, "delayMinMs": 1.0, "delayMaxMs": 5.0,
        "limiterCeilingDb": -3.0 }
    ],
    "maxPeqBoostDb": 6.0,
    "masterVolumeMaxDb": -10.0,
    "lockedPages": ["outputs"],
    "note": "Tweeters on output 3",
    "contact": "installer@example.org"
  }
}
```

Every field may be left out; a field left out limits nothing.

| Field | Limit |
|---|---|
| `outputs[].maxGainDb` | The output's highest gain. Never above the registry's +20 dB (a profile only narrows). |
| `outputs[].highpassMinHz`, `highpassMinSlopeDb` | The output must keep an active high-pass (PEQ high-pass, or a crossover high-pass: LR, Butterworth, Bessel) with its corner at or above the lowest frequency and at least the slope (6 to 48 dB/oct; 6 when not given). |
| `outputs[].delayMinMs`, `delayMaxMs` | The output's delay range. |
| `outputs[].limiterCeilingDb` | The output's limiter stays on, its threshold at or below this ceiling (dBFS). |
| `maxPeqBoostDb` | The most any PEQ peaking or shelf band, on any channel, may boost. |
| `masterVolumeMaxDb` | The highest master volume. Muting is always allowed. |
| `lockedPages` | The pages an installer locks, by the mode manifest's page ids (P1LK reads it; nothing in P1LM). |
| `note`, `contact` | The installer's note and how to reach them. |

A profile is normalised when read: every number held to the registry's range,
NaN dropped, a delay range put the right way round, an output named twice
taken once (the last).

An exported document (`config_export`) carries the open device's profile in
`meta.limits`, so it travels with the tuning. Importing such a document does
not set the profile: applying it is an installer's step
(`limits_apply_profile` takes any document with `meta.limits`).

### The PIN is not in the profile

The installer PIN (P1LK) is kept beside the profile in a host-only file,
`limits.pin.json`: `{version, kdf: "pbkdf2-sha256", iterations, salt, hash}`,
the salt 16 random bytes from the OS with the serial appended, the hash
PBKDF2-HMAC-SHA256 (100 000 iterations for a new PIN; the file keeps its own
count). Neither the PIN nor its hash is ever written into a document: not an
export, a snapshot, a template, a limits document or a diagnostics bundle.

## Where it lives

`limits_set_dir(dir)` names the app's own data folder. Each device has a
subfolder named as the snapshot store and the event log name theirs (`s-<serial>`,
or `x-<hex>` for a serial with other characters):

| File | What |
|---|---|
| `limits.dspipreset` | the profile |
| `limits.good.dspipreset` | the last profile that was written cleanly: the fallback |
| `limits.pin.json` | the PIN's hash (P1LK) |
| `limits.recovery.json` | the recovery code's hash (P1LK, [installer-lock.md](installer-lock.md)) |
| `limits.tries.json` | the wrong-PIN count and refusal window (P1DKFIX) |

Name the folder before opening a device: with a device open whose installer PIN
is set, `limits_set_dir` needs the console unlocked (`pin_required`), and the
PINs and profiles in force are carried to the new folder (P1DKFIX,
[installer-lock.md](installer-lock.md)).

Every file is written atomically: a temporary file beside it, then a rename,
so a crash leaves the old file or the new one.

On the web there are no files: the page keeps the profile's document (browser
storage, download and upload) and hands it to `limitsApply` when a device
opens. A profile set with no folder named is kept for the core's lifetime.

A hub keeps its own copy per device (P1HL); local USB users of the same
machine keep theirs.

### A corrupt or missing profile (OD-P1k)

| Found | Then | `limits_status` |
|---|---|---|
| no profile, no good copy | the device runs as today | `state: "none"` |
| the profile reads cleanly | it is in force | `state: "ok"` |
| the profile is missing or unreadable, the good copy reads | the good copy is in force (fails closed) | `state: "fell_back"`, notice `limits.fell_back` |
| neither reads | no limits; the notice stands | `state: "unreadable"`, notice `limits.unreadable` |

Nothing here panics or refuses writes for want of a profile: refusing until an
installer restores it risks a hard lockout.

### A forgotten PIN (OD-P1c)

The documented last resort: delete the device's folder. That removes the
profile, its good copy and the PIN together (and the recovery code; before it,
the recovery code shown when the PIN was set, or a hub's Admin:
[installer-lock.md](installer-lock.md)).

## Enforcement

| What | With a profile |
|---|---|
| A direct write (a typed setter, `write_param`, an undo, a redo) past a limit | Refused before anything is sent: `DspiError::LimitExceeded { code, msg }`, notice key `limit_exceeded`; `code` names the limit (`out_gain`, `eq_boost`, `highpass`, `delay`, `limiter`, `master_volume`). C and the web: `"code": "limit_exceeded"`. |
| A bulk image: an import, a snapshot recall, an A/B toggle, a template or document applied | Held to the limits before it is sent, and reported in the import's words: `Held to the limits profile: out.gain[2] 10 → 3 dB, eq.gain[0,1] 12 → 6 dB` (an import's `lines`, a recall's `notes`). |
| The profile applied (`limits_apply_profile`), the device outside a narrowed limit | Clamped at once: ordinary writes, journalled as one undo group labelled "Limits profile" (a limiter switched on stands apart, being audible), returned as `held` rows and kept in `limits_status().lastHeld`. |
| A preset slot loaded (OD-P1j) | The firmware loads the slot's stored values, which the host never held; the core reads them back (as every load does), clamps what is outside and reports it in `limits_status().lastHeld`. |
| A revert of a guarded apply (P1SA, P1HA) | Never refused or held: it restores a state captured before, past any limit set since. |

A high-pass is judged on the output's whole set of bands: a write is refused
when the output kept the high-pass the profile asks for and would no longer
(the corner lowered, the slope made shallower, the band bypassed or changed to
another type). Holding adds one where an output has none: an active high-pass
steep enough is raised to the corner; else a crossover high-pass (one there,
or a flat crossover slot) is made a Linkwitz-Riley high-pass at least as steep,
at the corner. An output with no free crossover slot is named, not changed.

A limiter is judged the same way: a write may not take an output's limiter
outside its ceiling, nor further outside (switched off, its threshold raised);
one that brings it closer passes, so it can be put right a step at a time.

## The calls

| Rust / UniFFI | C | Web (`call`) |
|---|---|---|
| `limits_set_dir(dir)` | `dspi_limits_set_dir` | (none: no files) |
| `limits_apply_profile(document) -> LimitsApply {lines, held, notes}` | `dspi_limits_apply_profile` | `limitsApply {text}` |
| `limits_profile_document() -> String` ("" with none) | `dspi_limits_profile_document` | `limitsProfile -> {text}` |
| `limits_clear()` | `dspi_limits_clear` | `limitsClear` |
| `limits_status() -> LimitsStatus {active, serial, state, noticeKey, detail, lastHeld}` | `dspi_limits_status` | `limitsStatus` |

The notices (`notice_for_key`): `limit_exceeded`, `limits.fell_back`,
`limits.unreadable`, `limits.scope`.

### From a script: `uconsole limits` (WP P1TM)

| Command | Core call |
|---|---|
| `uconsole limits show [--json]` | `limits_status()`, `limits_profile_document()` (`--json`: `{status, profile}`) |
| `uconsole limits set FILE` | `limits_apply_profile(text)`; prints the rows held at once |
| `uconsole limits clear` | `limits_clear()` |

The Terminal keeps profiles in `<local data dir>/uconsole/limits` (on Linux
`~/.local/share/uconsole/limits`). With an installer PIN set, `set` and `clear`
need it: on a terminal they ask; in a script, `DSPI_INSTALLER_PIN=... uconsole
limits set FILE`, or `--installer-pin -` with the PIN as a line of standard input.
A wrong PIN exits 3, a missing one 1. A plain `uconsole import` (the session path)
is held to the same profile (`preset_file::apply_limited`). Not yet: the
Terminal's single writes (`uconsole set`, scripts, its interface) write through
`dspi-session` and are not checked against the profile or the lock.

```sh
uconsole limits set k340-site.limits.dspipreset
uconsole limits show --json | jq .status.active
```

The shared code is `dspi_session::limits` (the profile, its document, the
plain-value checks, the files and the PIN's hash), used by the core and the
hub; the Terminal's own import path is `preset_file::apply_limited`.

## What the existing safeguards do

- The hub's roles: a `viewer` cannot change anything, a `control` client
  cannot change the device configuration ([hub.md](hub.md)).
- The core-voltage rule: no client on the network raises the core voltage
  above 1.20 V.
- Read-only connections: firmware of another wire format is shown, never
  written.
- Each parameter's own range and hazard in the registry
  (`crates/dspi-proto/src/registry.rs`).
