# PRO phase 3: fleet in the web console

*WP P3PLAN, 2026-11-16, written on `main` at `77b9d397`. The spec is PRO1's report (`work/pro1/REPORT.md` section 2
rows F3 to F8 and U1; section 4 "Phase 3"; section 7 question 7), `PLAN-P2.md` section 3 ("Phase 3 outline") and the
owner's decisions OD-P2a (freeze REST v1 after phase 3) and OD-P2b (unattended flashing: yes, under conditions). Phase 2
(`PLAN-P2.md`, P2-A to P2-F less P2SW) and the security waves SEC-A to SEC-D (`PLAN-SEC.md`) are merged; this plan is
written against that code, cited `file:line`. Plan only: no code, no edits to `PLAN-P2.md` or `PLAN-SEC.md`.*

**Goal.** From any browser on the LAN, served by the hub itself, see and manage many DSPis at once: name them in
groups, set a volume or load a preset on a group with per-device offsets, apply a shared snapshot to a group with a way
back per device, see every device's health on one board with the alarm rules' thresholds, and update the firmware of the
hub's own USB devices with nobody at the rack, one device at a time, exported first and verified after. **Hub-first and
core-first, as phase 2:** each family is one more `HubService` in `crates/dspi-hub`, every write is a principal's judged
by role, limits and lock, the web reads it through the core's typed calls, and the fleet screens open the existing
session view for a device rather than becoming a second console. Qt, Android and Swift get nothing in this phase (the
multi-device view is the web console; owner, `PLAN-P2.md:28`). At the end REST v1 is frozen.

**Owner decisions this phase builds on:**
- **OD-P2b (2026-11-15): the hub may flash with nobody at the rack.** Admin only, one device at a time, the
  configuration exported first, verify after reboot (firmware version, `EXT_LIST`, and on a K340 the stored 1.20 V
  core setting read back), an alarm if the device does not come back, refuse when more than one boot-mode device is
  present; a **PICOBOOT writer in the core** (option 3), not a mass-storage copy (`PLAN-P2.md:531`).
- **The K340 is flashed only at the fixed 1.20 V core setting, never with a VREG or overvolt change.** The hub never
  writes `EXT_CORE_VREG` (`PLAN-P2.md:27`, `hub-api.md:185`); the rollout reads it back and refuses any image the firmware
  guard would ask a word or a typed phrase for (section 1.1).
- **OD-P2a:** REST under `/api/v1/` is "v1, experimental" until phase 3's fleet needs are in, then frozen: additive only
  within v1 (`PLAN-P2.md:530`).
- **Three roles only** (Admin, Control, Viewer); Admin is given on the hub's machine. A group has no role of its own
  (OD-P3a).
- **PLAN-SEC applies** to every new message: TLS 1.3 with pinning on the DSPi Link port (SECSRV), API keys and the
  `Origin` refusal on REST, no secrets in logs, events or the diagnostics bundle (section 3).
- **The owner's compatibility policy (2026-11-15):** nothing is released but Android (closed test), so no compat shims
  between our own components; what must keep working is our clients against an upstream Weeb Labs hub and USB to the
  hardware; our hub against upstream clients is best effort. No pull requests to Weeb Labs (OD-P2d).
- **Process:** parallel Opus WPs in waves of disjoint owned files, one Fable review per batch, nits folded into later
  WPs; toolchains in containers, no tmpfs, no devices in a WP (the K340 stays with the coordinator); Swift work on the
  Mac VM is slow, so Apple WPs are few and separate: this phase has none.
- **Naming:** "DSPi Link" or "dlink" for the network link, "alink" in code for the audio link, never bare "link";
  Snapshot is a host whole-device state, Scene the alink system scene; **System** is the alink system view that exists
  (`apps/web/src/screens/system/SystemScreen.tsx:1-7`), so U1's dense per-device table is the **Overview** (OD-P3j).

**What exists today (checked in the code on `77b9d397`):**

| Phase-3 item | State | Builds on |
|---|---|---|
| Service seam | `trait HubService` (`crates/dspi-hub/src/service.rs:101`), `ServiceConfig` (`:144`, one field per family, off by default), `ServiceSetup` (`:198`: the bus, clock, data folder), the registration list under `// -- WP <ID> --` markers (`:216-234`), `open_principal` (`:515`) | every phase-3 hub family is one more module and one line |
| Action executor | One `DspiCore` per (principal, device) over a child session, opened lazily, closed after 60 s idle, at most 32 open, `busy` past that (`crates/dspi-hub/src/api/exec.rs:1-20`, `:37-41`) | F4, F5, F6: every fan-out and the rollout's device writes go through it |
| Hub-side recall with a guard | `snapshot.recall {serial, id or name, guard?}`: `apply_begin(session, handle, timeout)` as the caller, revert on failure (`api/recall.rs:49-66`); guard timeouts 1 s to 600 s, default 30 s (`apply_guard.rs:71-75`, `:410`, `:517`, `:558`) | F5: one guard per device |
| Multi-target result | `results: [{serial, ok, code?, msg?}]` in request order (`docs/integrator/hub-api.md:97-117`); `group:<id>` reserved, answered `unsupported` (`crates/dspi-hub/src/schedule.rs:520-523`) | F3, F4, F5, F6 |
| Inventory | `Record` with `tags` (`inventory.rs:112`, `:150`), 1024 devices (`:59`), `inventory.list/set/forget`, `inventory.changed` | F3's members are inventory serials |
| Health poll and facts | `Facts {identify, reset_info, fw}` on attach, `Health {cpu, clip_mask, temp_c, uptime_s, hang_count}` every 2 s (`bus.rs:98-125`; `health.rs:1-36`); `health.get` record (`hub-api.md:457-490`); reads as `INTERNAL_SESSION_ID`, never a write | F7's row; F6's verify after reboot |
| Device alarms | `AlarmBook` with 8 codes (`device_alarms.rs:84`), severities (`:323`), rules in `alarms.json`, `alarm.list/ack/rules.*`, `Alarm` on the bus | F7's thresholds; F6's `rollout_failed` |
| History | `history.rs` from the bus; `EventKind` has `Firmware`, `Schedule`, `Access`, `EventsDropped` (`crates/dspi-session/src/events.rs:78-124`), field `by` | F6's record; F7's "last change and by whom" |
| REST mirror | `/api/v1` (`rest.rs:60`), `route` (`:788`), the endpoint table (`hub-api.md:990-1058`), 64 KiB body cap, `Origin` refused, keys `plaintext_ok` | the phase-3 endpoints and the freeze |
| Core calls | `hub_server.rs:898-1360` (P2CC1), `hub_server/admin.rs:1089-1520` (P2CC2), `hub_server/manage.rs:66-191` (P2CC3); events through `HubServerListener` / `hub_server_events` | P3CC adds a fourth section |
| Web hub pages | `HubBackend` (`apps/web/src/fleet/hub.ts:1-16`), the device table `HubRow` (`screens/hub/model.ts:11-31`), alarm board, stats, banner (`screens/hub/HubScreen.tsx`), the feed polling every 5 s (`screens/hub/feed.ts:1-12`, `:31`), hub settings (`screens/hub-settings/`), the hub history (`screens/events/HubHistory.tsx`), e2e `test/e2e/hub_p2wb1.spec.ts`, `hub_p2wb3.spec.ts`, `hubsettings_p2wb2.spec.ts`; screens registered in `apps/web/src/ui/screens.tsx:105-117` under markers | F7, F8 and U1 add files beside them |
| Firmware in the core | `uf2_inspect` (`crates/dspi-core/src/firmware.rs:585`; blocks `:138`), `flash_confirmed` to a mounted drive with the guard and a confirmation token (`:632`), `bootloader_counts` over nusb for `2E8A:0003`/`2E8A:000F` (`:84-88`, `:655`), `flash_to_drive` (`:930`); the firmware guard's rule table (`firmware_guard.rs:13-24`), `firmware_guard_board` (`:225`), `firmware_guard_check` (`:246`), `flash_to_drive_guarded` (`:276`); `verify_bundled` against the manifest (`firmware_manifest.rs:126`, SECFWV); `enter_bootloader_confirmed(token)` (`hardware.rs:1099`) | F6: the writer is the one missing piece |
| The hub's USB | `DeviceBus` scans `USB_PID` only (`crates/dspi-hub/src/usb.rs:22-33`, `:57-58`); `vid` is 0 for a device that is not on USB (`:14-15`); nusb behind feature `usb` (`Cargo.toml:18`) | F6 sees boot-mode boards through the core, not the bus |
| `fw.install` | `unsupported` (`server.rs:3377`), `FwData` frames `UNSUPPORTED` (`:2982`), `fw_install` not in `hello.caps` (`:3838`); `REQ_ENTER_BOOTLOADER` is Admin-only over DSPi Link (`policy.rs:347`) | stays as it is (OD-P3f) |
| TLS per connection | `serve`/`answer` carry `encrypted` (`server.rs:1897-1902`, `:1974-1981`); `Session` (`session.rs:83`) does not record it yet; API keys have `plaintext_ok` (`auth.rs:111-113`); tests run against a TLS hub with `test-tls-mode` (`crates/dspi-hub/Cargo.toml:30-36`) | F6's `tls_only` messages (OD-P3n) |
| PICOBOOT protocol | The SDK header `boot/picoboot.h` (`~/prog/DSPi/firmware/pico-sdk/src/common/boot_picoboot_headers/include/boot/picoboot.h`): magic `:26`, `PICOBOOT_IF_RESET` `:33`, commands `:48-59` (`EXCLUSIVE_ACCESS`, `REBOOT`, `FLASH_ERASE`, `READ`, `WRITE`, `EXIT_XIP`, `REBOOT2`, `GET_INFO`), exclusivity modes `:120-122`; picotool is built on the owner's host as a reference (`dspi-firmware-env`) | P3PB |
| Terminal | `uconsole flash` to a drive (`apps/terminal/src/flash.rs:85`), `uconsole hub` flags and `--no-<service>` switches (`hub.rs:650-672`), `uconsole api <type> [json]` (P2TM) | P3TM |
| Swift | SW-M3 is merged (the Mac app hosts the Rust hub); **P2SW and SECSW are not** (no Swift phase-2 views, no Swift TLS UI yet) | nothing in phase 3 waits on them |

**Corrections to PRO1 and to PLAN-P2 section 3 found while checking:**
- PRO1 F6 writes with a mass-storage copy; OD-P2b chose the PICOBOOT writer. The drive path (`flash_to_drive`) stays for
  the clients' own flashing; the hub never mounts anything.
- PRO1 F3 gives a group "a role". Roles are per client (three only, owner); a group has a name, a note and members
  (OD-P3a).
- PRO1 U1 calls it the "System" view. The web's System screen is the alink system (rooms, zones, topology, signal
  chain; `SystemScreen.tsx:1-7`) and the glossary's "System" is the alink system. U1 is the **Overview** screen (OD-P3j).
- PLAN-P2 section 3 has inventory records gain `groups`. One owner of the data is simpler: `group.list` is the source,
  `inventory.list` is unchanged, the web joins (OD-P3b).
- PLAN-P2 section 3 has `FleetBackend` gain `inventory`, `health`, `alarms`, `groups` methods. P2WB1 built it the other
  way: the hub's families reach the page through the session's core (`screens/hub/feed.ts:1-12`), and `FleetBackend`
  gives only installations, access and credentials (`apps/web/src/fleet/types.ts:59-104`). Phase 3 keeps that: no client
  speaks the hub's JSON; `FleetBackend` is not widened (OD-P3k).
- PRO1 F7 lists "link quality". The alink's lock and status are on the System screen already; the health board shows
  what the health poll has and gains the stream counters (OD-P3m), not the alink.
- PLAN-P2 section 3 has bulk apply "one at a time or in parallel by a setting". Sequential only (OD-P3d).

**Not in this phase:** several hubs under one page and a fleet server (phase 4, `PLAN-P2.md:509`; every answer already
carries `hub`); OSC and the MIDI control map (PRO1 phase 6); fleet or admin pages in Qt, Android or Swift (never: owner);
the Overview in Qt, Android and Swift (a later batch, "web first" per PRO1); the spec's `fw.install` with `FwData` frames
(OD-P3f); a rollback slot (FW10 is not feasible on the RP2 boot ROM); FW6 (temperature in the status packet); standing
parameter links between devices (OD-P3c); P2SW and SECSW (Swift's phase-2 views: a Swift batch of their own).

---

## 1. Goal and scope, item by item

"Message" means a JSON control-plane message on `/dspi/v1`; each new family is additive to DSPi Link 1.2, announced in
`hello.caps`, answers carry `"v": 1`, unknown types still answer `unknown_type`, and **no new error code is added**: the
phase-3 families answer with the codes of `hub-api.md` section 1.6 (`denied`, `bad_request`, `unsupported`, `not_found`,
`conflict`, `rate_limited`, `offline`, `locked`, `busy`, `limit_exceeded`, `installer_lock`, `image_refused`,
`confirmation_required`, `tls_required`). Every multi-device answer is the `results` shape of section 1.3.

| Item | What the hub does | Builds on | Clients |
|---|---|---|---|
| **F3 groups** | Named sets of inventory serials in `<data>/groups.json`: `group.list` (every role), `group.set {id?, name, note?, members}` (Admin; a new group without `id`, otherwise replaced whole), `group.delete {id}` (Admin), event `group.changed {v, hub, id, group}`. Ids are the hub's (`g1`, `g2`, never reused); 64 groups, 64 members each, name 1 to 48 characters, note 256; a member must be an inventory serial (`not_found` otherwise); a forgotten device leaves its groups with a `group.changed`; a device may be in many groups. Schedule targets `group:<id>` resolve to the members at run time (`schedule.rs:520-523` lifts its refusal); an unknown group is one `results` row `{serial: "group:<id>", ok: false, code: "not_found"}`; the expansion is bounded as the rule's rows are (at most 256 rows a run) | `inventory.rs` (serials, `inventory.changed`), `schedule.rs`, the service seam | Web groups editor (P3WGR); core calls (P3CC) |
| **F4 linked parameters** | One-shot fan-out, never a standing link (OD-P3c): `group.param.set {group or targets, path, idx?, value, offsets?: {serial: delta}}`, `group.preset.load {group or targets, slot or name, map?: {serial: slot or name}}`, `group.param.get {group or targets, path, idx?}` (every role), each a `results` row per member in group order, done one member after another through the executor as the caller's child session, so each member is judged by role, limits and lock on its own (a Control client's write past a limit is that row's `limit_exceeded`, nothing sent to it, the rest proceed). `offsets` only on paths of dB kind; `value + delta` is clamped to the registry range and the clamped value answered in the row. A partial failure is reported, never retried later. One group write at a time per session (`busy` otherwise) | `api/exec.rs`, `api/params.rs`, `api/presets.rs`, `write_param`'s guards, `policy.rs`, `limits.rs` | Web group action bar (P3WGR); `uconsole api`; REST (P3RS) |
| **F5 bulk apply** | `group.apply {group or targets, snapshot (id or name) or document (text), guard?, timeout_ms?}` (Control and Admin): per member, `recall.rs`'s path (`snapshot_recall_document` through the executor's core as the caller's child session, so P1HL's rule holds per device: `image_refused` for Control on a limited device, held after the fact for Admin); with `guard: true` a P1HA guard **per device**, begun as the caller immediately before that device's write, all owned by the caller's session; the answer carries `run`, the rows and `confirm_by` (the earliest guard deadline); `group.apply.confirm {run}` and `group.apply.cancel {run}` confirm or cancel every guard of the run (`apply_confirm`/`apply_cancel` per handle); an unconfirmed run reverts device by device at each guard's deadline or when the session closes, as today. Sequential only (OD-P3d); a document from another device is applied as the core applies it (wire compatibility and output count decide the row). Groups are capped at 64, so a guarded run on a big group needs `timeout_ms` (up to the guard's 600 s); the answer says so | `api/recall.rs`, `apply_guard.rs`, the hub snapshot store (`snapshots.rs`) | Web bulk apply (P3WGR); REST (P3RS) |
| **F6 firmware rollout** | Section 1.1. Our own family under cap `rollout` (OD-P3f): `fw.image.put {name, seq, data, last}` (Admin; chunked base64 upload, 48 KiB a chunk, 8 MiB an image, 4 images in `<data>/firmware/`, one upload per session, 2 min to finish), `fw.image.list` (every role: name, size, SHA-256, `firmware_version`, `platform`, `stamp_mv`, `verified`), `fw.image.delete` (Admin), `fw.rollout.start {serial or group, image}` (Admin), `fw.rollout.status {id?}` (every role), `fw.rollout.cancel {id}` (Admin; takes effect before the bootloader entry of the current device, never during a flash), event `fw.rollout.changed {v, hub, rollout}`. Records in `<data>/rollout.json` (64 kept). `fw.install` stays `unsupported`. **Every `fw.*` message is refused over plaintext (`tls_required`), in every TLS mode including upstream compatibility** (OD-P3n) | P3PB's writer in the core, `firmware_guard.rs`, `firmware_manifest.rs`, `api/exec.rs`, the hub snapshot store, `health.rs` facts, `device_alarms.rs`, `history.rs` | Web rollout screen (P3WRO); `uconsole hub firmware add` (P3TM); REST read-only (P3RS) |
| **F7 health board** | `health.get` records gain `counters` (the stream's underrun and starvation totals, one more single read every 30 s inside the poll budget), `clip_per_min` (0xE0 events in the last minute where the firmware has them; absent on latch-only firmware), `last_change {at, by, text}` (the newest history record of the device, from the bus). Thresholds are P2AL's rules, read by the page from `alarm.rules.get`; nothing is duplicated on the hub | `health.rs`, `bus.rs` (`Health` fields), `history.rs`'s `Event` on the bus, `device_alarms.rs` rules | Web health board (P3WHB) |
| **F8 fleet screens** | None on the hub: the hub side of F8 is F3 to F7. The web hub screen gains tabs or sections for Groups (edit members, Admin), a group action bar (volume, mute, preset with offsets; bulk apply with guard and one confirm), the health board, a fleet history (the hub history per device, merged and bounded) and the rollout; "open in the console" exists (P2WB3). `FleetBackend` unchanged (OD-P3k); every id in the page's models is (hub id, serial) | P2WB1/P2WB3's `screens/hub/`, `screens/events/HubHistory.tsx`, the core calls | Web (P3WHB, P3WGR, P3WRO) |
| **U1 Overview** | None on the hub. A web screen "Overview" for the session's device: every output in one table (routing sources, gain, delay, mute, limiter threshold and gain reduction, clip latch) and every channel's meter with peak hold, inputs and outputs, dense at desktop width and a card list at 375 px; gain and mute editable inline through the session's existing writes (the same guards, limits and undo as the Outputs screen). Web only in this phase (OD-P3j) | `screens/outputs/model.ts`, `screens/main/live.ts` (meters), the registry's labels and units | Web (P3OV) |
| **REST v1 and the freeze** | Within v1 and additive: `GET /api/v1/groups`, `GET /api/v1/groups/{id}`, `PUT /api/v1/groups/{id}/params/{path}?idx=` (`group.param.set` with `offsets` in the body), `POST /api/v1/groups/{id}/presets/{n}/load`, `POST /api/v1/groups/{id}/apply` (`group.apply`; a guarded run belongs to the key's session, confirmed and cancelled by `/api/v1/runs/{run}/confirm|cancel` within `rest_idle`), `GET /api/v1/rollouts`, `GET /api/v1/rollouts/{id}`, `GET /api/v1/firmware`. **No rollout start and no image upload over REST** (OD-P3l): a flash is an Admin's console act, and the 64 KiB body cap stays. After P3-C `hub-api.md` is marked **v1 frozen**: additive only; a breaking change is `/api/v2/` and new message types | `rest.rs`, `hub-api.md` section 14 | Integrators |

**What the hub never does in phase 3:** write `EXT_CORE_VREG` (it reads it back); type `OVERVOLT` or the guard's phrase
for anyone (an image the guard does not clear outright is refused); flash more than one device at a time, or any device
not on its own USB; retry a flash, a fan-out row or an apply; start a flash over plaintext; send `REQ_CLEAR_CLIPS`; keep
a standing link between devices; add a fourth write of its own (the rollout's device writes are the Admin's child
session's, OD-P3g: **the hub's own writes stay the closed list of three**, `hub-api.md:178-180`).

### 1.1 The rollout, step by step (F6)

One device, started by an Admin with `fw.rollout.start {serial, image}`; a group is the same, one member after another,
stopped at the first failure (OD-P3h). Each step is recorded in the rollout record and, at the end, in the device's
history as a `firmware` record (`events.rs:93-94`) with `by` the Admin's client name.

1. **Refuse early, before anything is touched.** The device must be online on this hub's own USB (`vid != 0`,
   `usb.rs:14-15`; a demo, MIDI or alink-attached device is `unsupported`), not `unresponsive`, not under another
   session's lock (`locked`), with no pending guarded apply (`conflict`), and master-muted unless the request carries
   `"while_live": true` (OD-P3r). No other rollout may be running (`busy`). **No board may be in BOOTSEL already**
   (`bootloader_counts` must be 0/0, `firmware.rs:655`; `conflict` otherwise). The image must be in the hub's store,
   `uf2_inspect` valid, of the device's platform, and `verified` by the manifest unless the hub runs with
   `--allow-unverified-firmware` (OD-P3e).
2. **Read the board and judge the image.** Through the executor's core for (this Admin, the device):
   `firmware_guard_board` (`firmware_guard.rs:225`: `required_mv` from `EXT_CORE_VREG`, else the status block) and
   `firmware_guard_check(image, board, "")` (`:246`) with **no typed word and no typed phrase**. Anything but "clear" is
   refused with the guard's own message: an image stamped above the board's voltage or above 1.20 V (the `OVERVOLT`
   row), an unstamped or lower image on an at-risk board (the typed-confirmation rows), an unknown board. So a K340 at
   1.20 V takes exactly a 1.20 V image, and a stock-firmware board (reporting 1.15 V) takes a 1.20 V image, as the
   guard's table says (`firmware_guard.rs:13-24`). The board record is kept for step 6: the board cannot be asked in
   BOOTSEL.
3. **Export first.** `snapshot_capture_document` (`crates/dspi-core/src/snapshot.rs:433`) through the same core, put in
   the hub's snapshot store as "Before firmware <version>, <date>" (OD-P3o; `conflict` and no further step when the
   store is full), and the device's current `Facts` (firmware, `EXT_LIST`, board) and `EXT_CORE_VREG` (live, stored,
   default) kept in the record.
4. **Enter the bootloader as the Admin's child session.** `enter_bootloader_confirmed(token)` (`hardware.rs:1099`) on
   the executor's core, the token minted and spent inside as `preset.save` does with `"confirm": true` (`api/params.rs`,
   P1TK's discipline): judged by role (`0xF0` is Admin-only, `policy.rs:347`), the installer lock and the device lock
   like any client's write; **not** one of the hub's own writes. The device leaves the bus; the registry keeps its handle
   for the 60 s grace. A `cancel` from here on is `conflict`.
5. **Find the board.** Scan for `2E8A:0003`/`2E8A:000F` every 500 ms for up to 15 s. **Exactly one** must appear
   (OD-P3i); none, or more than one, ends the rollout failed (the device may be sitting in BOOTSEL: the alarm says so).
   On RP2350 the boot ROM's USB serial is the chip's unique id, which is the DSPi's serial, and it must match; on RP2040
   the boot ROM's serial is fixed and uniqueness is the only check.
6. **Flash through the core's writer (P3PB).** `EXCLUSIVE_AND_EJECT` first (so the kernel's mass-storage probe stops,
   the race `dspi-firmware-env` recorded with picotool), `EXIT_XIP`, erase the sectors the image covers, write the pages,
   read them back and compare, `REBOOT` (RP2350: `REBOOT2` to a normal boot). The writer takes the guard's board record
   from step 2 and the image, and refuses anything the guard would not clear, again (a second line, as the clients'
   flash paths have).
7. **Wait for the device.** Online again within 60 s (the registry's `Online` for the serial, inside the grace) or the
   rollout fails. Then **verify** on the fresh attachment: `Facts.fw` equals the image's `firmware_version`; `EXT_LIST`
   is not shorter than before (the extension channel did not vanish); and when the board record had `required_mv`
   1200, `EXT_CORE_VREG` read back has **stored and live 1200** (the K340 condition, OD-P2b). The device's settings are
   compared with the export (the hub's bulk image, `crates/dspi-hub/src/snapshot.rs:1-10`, against the document's):
   `settings: unchanged | changed | unknown` in the record.
8. **Report.** The record ends `ok` or `failed {step, text}`; `fw.rollout.changed` goes out; the history gets its
   `firmware` record; a failure raises the alarm `rollout_failed` (critical) with the step, cleared when a later rollout
   of that device ends `ok` or the device returns with the expected firmware; a group rollout stops. **Nothing is
   recalled by the hub itself**: the web offers "Recall the export" (`snapshot.recall` of the step-3 snapshot, judged as
   any recall) when `settings` is `changed` (OD-P3h).

What one flash costs: steps 2 to 4 a few hundred milliseconds on the device's queue; the BOOTSEL wait 5 to 15 s; the
write about 2 s for a 630 KB UF2; the reboot and attach 2 to 5 s; the verify one attach's reads. About 30 s a device, and
the rest of the hub serves meanwhile (the device is offline to every client for that time, as any reboot).

---

## 2. Work packages

**Shared rules for every P3 WP.** `PLAN-CLIENTS.md` section 1, `WP-BRIEF.md`, the wave1 brief, and **every hub rule of
`PLAN-P2.md` section 2**: one module per family (a new file, its `mod` line, one `ServiceConfig` field and one line in
`service.rs`'s list under `// -- WP <ID> --`); pure state and thin glue with the clock passed in; every write judged as a
principal (the session-id rule; never `LOCAL_SESSION_ID`, `INTERNAL_SESSION_ID` or `GUARD_SESSION_ID`); the hub's own
writes a closed list of three; bounded everything with the corrupt-file rule (`<name>.bad-<time>`); store files 0600 and
atomic; each new data file in the diagnostics bundle under its own marked block in `crates/dspi-core/src/diag.rs:156-163`
(one WP per wave edits it: P3GR in A, P3RO in B); docs one disjoint section per WP, marked built. Added for phase 3:
- **No new error code** and **no new hub-own write**. A WP that thinks it needs either says so in its report instead.
- **Fan-outs are sequential and bounded:** members in group order, one at a time, through the executor; a run answers at
  most 256 `results` rows (as a schedule run does) and `bad_request` past it.
- **Tests run in containers with no USB.** The PICOBOOT writer is tested against an in-process fake boot pipe and byte
  vectors; the rollout service against the demo device with a fake writer and a fake BOOTSEL scan injected through
  `ServiceSetup`. The one real flash (P3HIL) is the coordinator's, on the K340, with the owner's go.
- **TLS tests:** a WP that adds a `tls_only` message runs its suite with `DSPI_HUB_TEST_TLS=on` and `=compat` too
  (`crates/dspi-hub/Cargo.toml:30-36`) and shows the plaintext refusal.
- **Web:** unit tests (`npm run test:unit`) and Playwright e2e in the container (`ci/check.sh --step web`), Chromium and
  Firefox, 375 px with no sideways scroll; user text from the core's notice table and the registry's labels, no
  host-device nouns; a hub feature shows only when `hub_caps()` has it.
- Binding wiring (P3HB, P3CC) is shared and additive: run `ci/drift.sh write`, regenerate the ABI anchor and the Android
  contract, classify new methods in `WriteGuardedCore`. The Apple generated bindings are regenerated at merge by the
  coordinator (as `77b9d397` did), and `ci/macvm.sh macos` runs once after P3-C: no Swift WP.

Sizes are this project's units (about one hour of agent time with review); estimates.

### Core and hub WPs

**P3PB: The PICOBOOT writer in the core.** [DONE 2026-11-16] Size 3. Wave P3-A. **Owns:** new `crates/dspi-core/src/picoboot.rs`
(and `picoboot/tests_wp_p3pb.rs`), behind the core's `usb` feature (nusb is already there, `crates/dspi-core/Cargo.toml:59`);
in `firmware.rs`, one marked block exposing the image's pages (`pub(crate) fn uf2_pages(&Uf2Info, &[u8]) -> Vec<Page>`,
over the existing `Block` decode at `:138`, the E10 block left out as `uf2_inspect` does). **Implements:** OD-P2b's
option (3): `trait BootPipe { bulk_out, bulk_in, control_in, control_out }` with a nusb implementation that claims **only
the PICOBOOT vendor interface** (class 0xFF; never the mass-storage one) of a `2E8A:0003` or `2E8A:000F` device;
`boot_devices() -> Vec<BootDevice {platform, serial?, bus_location}>` (the RP2350 serial is the board's unique id, the
RP2040's is the ROM's fixed string and reported as `None`); the 32-byte command framing of `picoboot.h:26`, `:48-59`
(`PICOBOOT_MAGIC`, token, `cmd_id`, `cmd_size`, `transfer_length`, args), the status read (`GET_COMMAND_STATUS`) and
`PICOBOOT_IF_RESET` (`:33`) recovery; `picoboot_flash(image, target: BootDevice, board: FirmwareGuardBoard, token) ->
Result<FlashReport>`: `EXCLUSIVE_AND_EJECT` (`:120-122`), `EXIT_XIP`, `FLASH_ERASE` of the 4 KiB sectors the pages cover
(never outside the XIP window the platform has, `firmware.rs:59-64`), `WRITE` by sector, `READ` back and compare, then
`REBOOT` (RP2040) or `REBOOT2` normal (RP2350); the firmware guard (`firmware_guard_check(image, board, "")` must be
clear) and the confirmation token (`ConfirmOp::FirmwareFlash`, as `flash_confirmed`) in front of it; typed errors, no
panic on any device answer, a transfer deadline of 5 s and a whole-flash deadline of 60 s. Exported to the bindings as
`picoboot_devices` and `picoboot_flash_confirmed` (the Terminal's `uconsole flash --picoboot` can use it later; not in
this WP). **Done when:** tests with a fake `BootPipe`: the exact command bytes for each step against vectors taken from
the header's layout; a page plan from a real-shaped UF2 (the `firmware.rs` test images) erases whole sectors only and
never an address outside the window; a mismatching read-back fails before `REBOOT`; a stalled pipe hits the deadline and
the error names the step; an `EXCLUSIVE_ACCESS` refusal is reported, not retried; an image the guard does not clear
sends nothing; a missing or wrong token is `ConfirmationRequired` with nothing sent; `proptest_*` over mutated images
never panics; `cargo deny check` unchanged (no new crate). **Deps:** none. **Clients:** P3RO.

**P3GR: Groups.** [DONE 2026-11-16] Size 1.5. Wave P3-A. **Owns:** new `crates/dspi-hub/src/groups.rs` (`GroupBook`, the service,
`<data>/groups.json`), the `group:<id>` resolution in `schedule.rs` under a marker (`:520-523`, through a `GroupResolver`
trait defined in `groups.rs` and handed over `ServiceSetup`), the `DeviceForgotten` handling, the `groups.json` block in
`diag.rs` (the one `diag.rs` edit of P3-A). **Implements:** F3 as in section 1: `group.list`, `group.set`, `group.delete`,
`group.changed`; bounds (64 groups, 64 members, names 1 to 48, notes 256, members inventory serials); the corrupt-file
rule; schedule targets `group:<id>` expanded at run time, an unknown group one `not_found` row, the run's rows capped at
256 with the last row saying so (`bad_request`, "truncated"); `inventory.list` unchanged (OD-P3b). **Done when:** tests
through the real server on the demo device: Admin sets and deletes, Control and Viewer refused, Viewer lists; a member
that is not in the inventory is `not_found`; a forgotten device leaves its groups and `group.changed` says so; a schedule
rule on `group:g1` runs on every member (a `results` row each) and on an unknown group reports one row; the 65th group
and the 65th member are `conflict`/`bad_request`; a torn file reads what it can; the bundle carries `groups.json`.
**Deps:** none (P2IN, P2SC merged). **Clients:** P3GP, P3BA, P3RO (group rollouts), P3WGR, P3CC.

**P3HB: Health for the board.** [DONE 2026-11-16] Size 1.5. Wave P3-A. **Owns:** `crates/dspi-hub/src/health.rs` (P2HP's, merged),
the `Health` fields in `bus.rs` under a marker (`:109-125`), the `HubHealth` record in `crates/dspi-core/src/hub_server.rs`
under a marker (its fields are additive; the wasm binding's TypeScript record in `apps/web/src/core/protocol.ts:1257`
too), binding wiring. **Implements:** F7's hub half: `counters {underruns, starvation}` from one more single read every
30 s on the device's queue (the status counters the diagnostics bundle already reads, `device/stats.json`), inside the
poll budget and behind the lock like the other reads; `clip_per_min` counted from 0xE0 `Notification` bus messages
(`decode_ext_event`), absent on latch-only firmware; `last_change {at, by, text}` from the newest history `Event` on the
bus (coalesced records count once); all three in `health.get` and in `Health` for the alarms. Nothing is written; no
`REQ_CLEAR_CLIPS` (the existing grep-level test still holds). **Done when:** tests on the demo device: counters step
when the fake steps them, a latch-only device has no `clip_per_min`, a device with 0xE0 counts 10 events in a minute and
forgets them after, `last_change` names the writing client, the transfer count per minute rises by exactly the one read;
`hub_health()` carries the fields; `ci/drift.sh` clean. **Deps:** none (P2HP, P2EV merged). **Clients:** P3WHB.

**P3GP: Linked parameters.** [DONE 2026-11-16] Size 2. Wave P3-B. **Owns:** new `crates/dspi-hub/src/api/group.rs`; one marked
registration line in `api/mod.rs::services` (`:65`). **Implements:** F4 as in section 1 through the executor
(`exec.rs`) as the caller's child session per member: `group.param.set` with `offsets` (dB-kind paths only; the clamp
reported), `group.preset.load` with `map`, `group.param.get`; `group` resolved through P3GR's `GroupResolver`, or an
explicit `targets` list (16 at most); every row judged on its own (role, `write_refusal`, limits, lock, the voltage rule
as any write); one run at a time per session; never a raw opcode. **Done when:** tests through the real server with two
demo devices: a volume set with an offset lands on both, each clamped to its own range and the row says the value sent;
a Viewer is refused; a Control child session's row past a limits profile is `limit_exceeded` and that device gets
nothing while the other does; an offline member is `offline` and nothing waits for it; an offset on a non-dB path is
`bad_request`; a preset map loads slot 3 here and 5 there; a `group.param.get` returns both values; a second run from the
same session while one runs is `busy`; the history shows two attributed writes with the client's name. **Deps:** P3GR
(P3-A), P2PA (merged). **Clients:** P3WGR, P3RS, P3CC.

**P3BA: Bulk apply.** [DONE 2026-11-16] Size 2. Wave P3-B. **Owns:** new `crates/dspi-hub/src/api/bulk.rs`; one marked
registration line in `api/mod.rs::services`. **Implements:** F5 as in section 1: `group.apply` (snapshot by id or name
from the hub store, or a document), sequential, `recall.rs`'s path per member as the caller's child session; with
`guard` one P1HA guard per device begun immediately before its write, all the caller's; `group.apply.confirm` and
`group.apply.cancel` by `run` over every guard; `confirm_by` in the answer; rows for a device whose core refuses the
document (`image_refused`, wire compatibility, output count) and for `conflict` on a device with a foreign pending guard
(`no_foreign_guard`). **Done when:** tests through the real server with two demo devices: an apply without a guard
restores both; with a guard, both revert at their deadlines when nobody confirms, and one `group.apply.confirm` keeps
both; a cancel reverts both now; a Control caller on a limited device gets `image_refused` for that row while the other
applies; the session closing reverts what is pending; a run past 256 rows is `bad_request`; the history records a
`recall` per device with the caller's name. **Deps:** P3GR (P3-A), P2PA, P2SS, P1HA (merged). **Clients:** P3WGR, P3RS,
P3CC.

**P3RO: The rollout service.** [DONE 2026-11-16] Size 4. Wave P3-B. **Owns:** new `crates/dspi-hub/src/rollout.rs` (and `rollout/`:
`images.rs`, `steps.rs`, tests), the folder `<data>/firmware/` and the file `<data>/rollout.json`; a `rollout` feature in
`crates/dspi-hub/Cargo.toml` (`= ["api", "usb", "dspi-core/usb"]`, under a marker; the only `Cargo.toml` edit of P3-B);
marked blocks in `bus.rs` (`RolloutEnded {serial, ok, step, text}`), `device_alarms.rs` (the code `rollout_failed`,
critical, raised from `RolloutEnded`, cleared as section 1.1 step 8 says), `session.rs` (`Session::encrypted()`, set by
one marked line where `upgrade` opens the session, `server.rs:1957`; the only `server.rs` edit of P3-B), `diag.rs`
(`hub/rollout.json`; never the images); `ServiceConfig::rollout` (off by default) and `allow_unverified_firmware` (false).
**Implements:** section 1.1 and F6's messages in section 1; the image store (bounds, SHA-256, `uf2_inspect`,
`verify_bundled` label, the chunked upload with its timeout); the guard with no typist (step 2); the export (step 3); the
bootloader entry as the Admin's child session (step 4, through `exec.rs`); the BOOTSEL scan and the one-board rule
(step 5) through a `BootScan` seam over P3PB's `boot_devices`; the flash through a `Writer` seam over `picoboot_flash`
(step 6); the wait, the verify incl. the K340 1.20 V read-back and the settings comparison (step 7); the record, the
history `firmware` record, the alarm and the event (step 8); group rollouts one member at a time, stopped at the first
failure; `cancel` before step 4 only; **`tls_only` for every `fw.*` message** (OD-P3n: `tls_required` over plaintext in
`on`, `off` and compat modes alike; a hub with `--tls off` has no encrypted connection and therefore no rollout). The
hub's own writes stay three: a grep-level test that `rollout.rs` never submits as `INTERNAL_SESSION_ID`. **Done when:**
tests through the real server on the demo device with a fake writer and fake scan injected through `ServiceSetup`, the
test clock, and `DSPI_HUB_TEST_TLS=on`: a full rollout succeeds and the record has every step, the export is in the
snapshot store, the history record names the Admin; refused early for Control and Viewer, over plaintext
(`tls_required`), for an offline device, a device not on USB (`vid` 0), a locked device, a pending guard, an unmuted
device without `while_live`, a board already in BOOTSEL, an unverified image without the flag, an image the guard would
ask a word or a phrase for (stamped 1250 on a 1200 board; unstamped on a 1200 board; any image on an unknown board) with
the guard's message and nothing sent; the bootloader entry goes out as a child session in the remote range (the
session-id test), is refused by an installer lock, and the record's `by` is the Admin; two boards in BOOTSEL after entry
end the rollout failed with `rollout_failed` raised and no write; a device that does not return in 60 s raises the alarm;
a device that returns with another version fails the verify; a 1200 board whose read-back is 1150 fails the verify; a
changed bulk image answers `settings: changed` and no recall is sent; a group rollout stops at the first failure and
says which members were not tried; `cancel` before entry stops the queue, after it `conflict`; the upload's caps and
timeout; `rollout.json` in the bundle, the images not; the hub's tests pass in `off`, `on` and `compat`. **Deps:** P3PB,
P3GR (P3-A); P2PA, P2SS, P2HP, P2AL, P2EV (merged). **Clients:** P3WRO, P3TM, P3RS (read-only), P3CC.

**P3CC: Core calls for groups, bulk apply, rollout.** [DONE 2026-11-16: `d74646ed` into review/p3c, main `462fcde1`] Size 2.5. Wave P3-C. **Owns:** new
`crates/dspi-core/src/hub_server/fleet.rs` (a fourth section, as `admin.rs` and `manage.rs`), its `pub mod` line and
cap constants in `hub_server.rs` under a marker, the notice rows `alarm.rollout_failed` and the rollout step texts in
`assets/notices/notices.json` (shared wording, no host-device nouns), the TypeScript records in
`apps/web/src/core/protocol.ts` under a marker, binding wiring (C, UniFFI, wasm; `WriteGuardedCore` classification).
**Implements:** `hub_groups()`, `hub_group_set`, `hub_group_delete`, `hub_group_param_set(targets, path, idx, value,
offsets)`, `hub_group_param_get`, `hub_group_preset_load`, `hub_group_apply(..., guard, timeout_ms)`,
`hub_group_apply_confirm/cancel(run)`, `hub_firmware_images()`, `hub_firmware_put(name, bytes)` (chunking inside, the
bytes never logged), `hub_firmware_delete`, `hub_rollout_start(target, image, while_live)`, `hub_rollouts()`,
`hub_rollout_cancel`; the events `group.changed` and `fw.rollout.changed` through the existing `HubServerListener` and
`hub_server_events`; every call `Unsupported` without the cap and sends nothing; the alarm notice test (P2CC2's: a code
without a row fails) passes with `rollout_failed`. **Done when:** tests through the real hub in-process as P2CC1's: each
call with and without the cap; the same text through C, UniFFI and wasm; a rollout driven end to end from the core with
the fake writer; `ci/drift.sh write` clean, the Android contract regenerated. **Deps:** P3GR, P3GP, P3BA, P3RO.
**Clients:** P3WGR, P3WRO.

**P3RS: REST for the fleet.** [DONE 2026-11-16: `ba5549be` into review/p3c, main `462fcde1`] Size 1.5. Wave P3-C. **Owns:** `crates/dspi-hub/src/rest.rs` (P2RS's, merged: the
new routes in `route`, `:788`, and their translation), the REST rows of `hub-api.md` section 14.2. **Implements:** the
endpoints of section 1's REST row onto the existing messages, each judged as the key's principal; a guarded `apply` run
belongs to the key's session (`/api/v1/runs/{run}/confirm|cancel` within `rest_idle`, as the single-device guard today);
`GET /api/v1/rollouts`, `/rollouts/{id}` and `/firmware` read-only; **no `POST` that starts a rollout or uploads an
image** (`405` with `Allow`); the body cap unchanged. **Done when:** conformance tests: each endpoint, a Viewer key's
`PUT` `403`, a guarded group apply confirmed by the same key and reverted when another key tries, a rollout `POST` `405`,
every existing P2RS and SECSRV REST test unchanged, in `on` and `require`. **[Reworded by WP P3FRZ (review p3c): "unchanged"
cannot hold as written, since a `require` hub refuses plaintext REST clients by design (`426`). Read it as: every existing
P2RS and SECSRV REST test passes unchanged in the modes its clients were written for (P2RS's plaintext suite in `off`
and `compat`, the TLS suites in every mode), and P3RS's own tests pass in `on` and `require`, over TLS, with plaintext
answered as `hub-api.md` section 14.2 says (`426` in `require`, only a key minted `plaintext_ok` in `on`, the firmware
reads `426` in every mode). The p3c review ran it so.]** **Deps:** P3GP, P3BA, P3RO (P3-B). **Clients:**
integrators.

### Web WPs

Each builds on the core's typed calls only (the Worker through the wasm binding, as `screens/hub/feed.ts` does); no
client talks JSON to the hub. `HubScreen.tsx` takes **one marked line per WP, added at merge** (PLAN-P2 review S7's
rule), so the three hub WPs own disjoint new files.

**P3OV: Web: the Overview.** [DONE 2026-11-16] Size 2.5. Wave P3-A. **Owns:** new `apps/web/src/screens/overview/` (`OverviewScreen.tsx`,
`model.ts`, `OutputRow.tsx`, `Meters.tsx`), one marked line in `apps/web/src/ui/screens.tsx:111` (`SCREENS`), new
`test/unit/overview_p3ov.test.ts`, `test/e2e/overview_p3ov.spec.ts`. **Implements:** U1 as in section 1 (OD-P3j): every
output's routing sources, gain, delay, mute, limiter threshold and gain reduction, clip latch in one table; every input
and output meter with peak hold from the session's existing meter feed (`screens/main/live.ts`); gain and mute edited
inline through the session's writes (the Outputs screen's model, `screens/outputs/model.ts`: same guards, limits, undo
and read-only mode); labels and units from the registry; dense at desktop width, a card per output at 375 px; shown in
every mode. **Done when:** unit tests on the model (rows from the mirror, hold decay); e2e on the demo device in Chromium
and Firefox: 8 outputs in one table without sideways scroll at 1280 px, cards at 375 px, an inline gain edit lands and
shows in the Outputs screen, a foreign change (another session) updates the row, meters move. **Deps:** none. **Clients:**
web.

**P3WHB: Web: the health board.** [DONE 2026-11-16] Size 2. Wave P3-B. **Owns:** new `apps/web/src/screens/hub/HealthBoard.tsx`
and `health-model.ts`, the feed's new fields under a marker in `screens/hub/feed.ts`, `test/unit/hub_p3whb.test.ts`,
`test/e2e/hub_p3whb.spec.ts`. **Implements:** F7's board: one row per online device (CPU, temperature, clip per minute or
"clipped" on latch-only firmware, counters, uptime, hang count, restart, last change and by whom, raised alarms), offline
devices listed below; thresholds coloured from `hubAlarmRulesGet` (P2CC2's call; nothing hard-coded); sort by any
column, filter by group tag text; 375 px as cards; acknowledge from the row. **Done when:** unit tests on the model;
e2e against the in-process hub with the demo device: a threshold from the rules colours a cell, a raised alarm shows and
acknowledges, counters shown when the hub gives them and the column hidden when not, no sideways scroll at 375 px.
**Deps:** P3HB (P3-A), P2CC1/P2CC2 (merged). **Clients:** web.

**P3WGR: Web: groups, linked parameters, bulk apply, fleet history.** [TODO] Size 3. Wave P3-D. **Owns:** new
`apps/web/src/screens/hub/Groups.tsx`, `GroupActions.tsx`, `BulkApply.tsx`, `FleetHistory.tsx`, `groups-model.ts`,
`test/unit/hub_p3wgr.test.ts`, `test/e2e/hub_p3wgr.spec.ts`. **Implements:** F8's group half: a groups editor (Admin:
name, note, members picked from the inventory; others read), a group action bar (volume with per-member offsets shown
as the clamped value each will get, mute, preset with a per-member map), the results shown per device and kept until
dismissed (partial failure is visible, never hidden); bulk apply (pick a hub snapshot, guard on or off, timeout; a
confirm bar with the countdown to `confirm_by`; confirm or cancel); a fleet history: the hub history of the group's
devices merged by time (`hub_events_read` per device, 50 rows each, bounded) with `by`. **Done when:** e2e as P2WB1's:
a group made, a volume with an offset set on two demo devices and both rows shown, one offline member shown as such, a
guarded bulk apply confirmed and another cancelled with both reverting, the fleet history shows the writes with the
client's name, Control sees the editor read-only, 375 px. **Deps:** P3CC (P3-C). **Clients:** web.

**P3WRO: Web: the firmware rollout.** [TODO] Size 2.5. Wave P3-D. **Owns:** new `apps/web/src/screens/hub/Rollout.tsx`,
`rollout-model.ts`, `test/unit/hub_p3wro.test.ts`, `test/e2e/hub_p3wro.spec.ts`. **Implements:** F8's rollout half,
Admin only: the image list with version, platform, 1.20 V stamp and `verified` / "not verified" (the core's SECFWV
label), a file picker that uploads through `hub_firmware_put` (progress; 8 MiB refused before upload); start on a device
or a group with a dialog that lists the steps in words and says what the device will do (offline for about 30 s, a
reboot), with "Mute first" when the device is unmuted and the `while_live` acceptance spelled out; live progress per
step from `fw.rollout.changed`; the result with the verify lines (version, extension channel, "core voltage 1.20 V read
back" on a K340, settings unchanged or changed); "Recall the export" when settings changed (one click,
`hub_snapshots_recall`); cancel before the bootloader entry; the `rollout_failed` alarm in the banner with its step.
Shown only with the `rollout` cap and the Admin role. **Done when:** e2e against the in-process hub with the fake
writer: upload, start, every step shown, the result lines, a failed verify shows the alarm and the recall button, cancel
before entry works and after it says so, Control does not see the screen, over plaintext (`DSPI_HUB_TEST_TLS=compat` with
an `http://` page) the start is refused with the TLS notice. **Deps:** P3CC (P3-C). **Clients:** web.

### Terminal and docs

**P3DOC: The contract, drafted.** [DONE 2026-11-16] Size 0.75. Wave P3-A. **Owns:** in `docs/integrator/hub-api.md`: new sections
19 Groups, 20 Group writes and bulk apply, 21 Firmware rollout, the F7 additions to section 5, the REST rows of 14.2, the
"Every message" rows of section 18 (each marked draft until its WP marks it built); `docs/GLOSSARY.md` rows (Group,
Bulk apply, Rollout, Overview; System restated as the alink system); `docs/integrator/hub.md` "The store" rows
(`groups.json`, `rollout.json`, `firmware/`); `docs/integrator/README.md` row text. **Implements:** every request and
answer shape of section 1, roles, events, the `results` rows per family, the rollout's steps as the client sees them,
the `tls_only` rule, the no-new-error-code rule stated. **Done when:** every message type of section 1 has a row; the
links resolve; the no-host-noun check passes on user-facing strings. **Deps:** none.

**P3TM: Terminal.** [DONE 2026-11-16: `f2c8ec41` into review/p3c, main `462fcde1`] Size 1. Wave P3-C. **Owns:** `apps/terminal/src/hub.rs` (flags `--rollout`,
`--allow-unverified-firmware`; prompt commands `groups`, `rollout <serial|group:id> <image>`, `firmware add FILE`,
`firmware list`), the I7 examples section 16 of `hub-api.md`. **Implements:** `firmware add` puts a file into the hub's
image store from the hub's machine (the same checks as the upload; the only path that needs no console), the prompt
commands over the services, `uconsole api` already generic. **Done when:** command tests against an in-process hub with
the demo device and the fake writer: a group set and listed, an image added and listed with its label, a rollout started
from the prompt and its result printed, exit codes for `denied` and `tls_required`. **Deps:** P3GR, P3RO (P3-A, P3-B).

**P3FRZ: The freeze.** [TODO] Size 0.75. Wave P3-D. **Owns:** `hub-api.md`'s header and section 1.2 ("v1 frozen
<date>: additive only; a breaking change is `/api/v2/` and new message types"), the built marks of sections 19 to 21
once P3RS has merged, `docs/integrator/security.md` (one subsection: the rollout, what it does, TLS only, verified
images, the guard with no typist, what the hub never does), `docs/integrator/hub.md` (the `--rollout` and
`--allow-unverified-firmware` switches with the same large-warning pattern as `--upstream-compat`), the "built by
PLAN-P3" marks on `PLAN-P2.md` section 3 (marked, not deleted, as SECDOC did), `docs/integrator/README.md`. **Done
when:** links resolve; the curl and Python examples of section 16 run against a test hub in a container with the new
endpoints. **Deps:** P3RS (P3-C); P3WGR and P3WRO merged for the user-facing text.

**P3HIL (the coordinator, not an agent WP).** Size 0.5. After P3-C. One real rollout of the K340 PCB unit
(`BB66A44F2AC1E7B6`) through `uconsole hub --rollout` on the hub's machine, with **the image the unit already runs**
(the reviewed `bofr-main` image at 1.20 V, same SHA-256), backed up first as `k340-flash-standing-go` requires (config
export plus two identical 4 MB flash reads), so nothing changes but the path; the verify lines, the settings comparison
and the alarm on a simulated non-return (unplug during the wait) are recorded in `work/p3hil/REPORT.md`. **Owner action
required:** a go for this flash (the standing go covers alink USB-direct test images only; section 6).

### Firmware-dependent items (`bofr-main`, extension opcode 0x00)

Nothing in phase 3 needs new firmware. The rollout reads what exists (`EXT_LIST`, `EXT_IDENTIFY`, `EXT_RESET_INFO`,
`EXT_CORE_VREG` IN) and sends `REQ_ENTER_BOOTLOADER` over USB, which FWSEC1 leaves as it is (it gates MIDI, UART and
I2C only, `PLAN-SEC.md:697-704`).

| Item | Firmware (bofr-main) | WP | On stock firmware |
|---|---|---|---|
| Verify after reboot: version, `EXT_LIST`, core voltage | `EXT_LIST` 0x0000, `EXT_CORE_VREG` 0x0002 | P3RO | version only; a stock board reports 1.15 V from the status block and takes a 1.20 V image (`firmware_guard.rs:13-24`); an unstamped (stock) image on a 1.20 V board is refused: never a downgrade of a K340 unattended |
| Clip per minute on the board | 0xE0 behind `EXT_NOTIFY_FEATURES` bit 1 | P3HB | "clipped since cleared" only |
| Restart reason after a flash | `EXT_RESET_INFO` 0x0005 | P3RO (recorded) | the detach and attach only |

Optional and not planned: FW6 (temperature in the status packet) would save P3HB one read; not worth a wire change.

### Waves

No two WPs in one wave own the same file, and no WP depends on another WP of its own wave. Registration lines
(`service.rs`, `api/mod.rs`), binding wiring, `HubScreen.tsx`'s one line per web WP and `SCREENS` are additive under
markers and regenerated or added at merge. `diag.rs` takes one marked block per wave (P3GR in A, P3RO in B). The
`bus.rs` edits are in different waves (P3HB fields in A, P3RO's variant in B).

| Wave | WPs (parallel) | Size | Needs |
|---|---|---|---|
| P3-A | P3PB 3, P3GR 1.5, P3HB 1.5, P3DOC 0.75, P3OV 2.5 | 9.25 | `main` at `77b9d397` or later. No other branch touching `crates/dspi-hub/src/{health,schedule,bus}.rs`, `crates/dspi-core/src/{firmware,hub_server}.rs` or `apps/web/src/ui/screens.tsx` is open; the coordinator checks |
| P3-B | P3GP 2, P3BA 2, P3RO 4, P3WHB 2 | 10 | P3-A |
| P3-C | P3CC 2.5, P3RS 1.5, P3TM 1 | 5 | P3-B. Then P3HIL (coordinator, 0.5, with the owner's go) and one `ci/macvm.sh macos` compile of the regenerated bindings |
| P3-D | P3WGR 3, P3WRO 2.5, P3FRZ 0.75 | 6.25 | P3-C |
| | **Total: 15 WPs** | **30.5** (+0.5 P3HIL) | core 5.5 (P3PB, P3CC), hub 12.5 (P3GR, P3HB, P3GP, P3BA, P3RO, P3RS), web 10 (P3OV, P3WHB, P3WGR, P3WRO), Terminal 1, docs 1.5 |

Soft dependencies inside a wave are on P3-A's seams, not on another WP of the wave: P3GP and P3BA take `group` through
P3GR's `GroupResolver`; P3RO takes the writer and the scan through seams injected in tests (the real ones are P3PB's,
merged in A). PRO1 estimated phase 3 at about 25 units; this plan is larger by the rollout's safety steps and its TLS
rule (P3RO 4 against PRO1's "4 (+3)" for the writer, which is P3PB), the REST additions and the freeze, and the
Overview sized for a dense web table rather than "1 per client". Each wave ends with one Fable batch review; the full
gate runs after P3-C (the hub and core complete) and after P3-D. P3-B is the widest; if fewer agents run, P3RO goes
first (it gates P3CC, P3TM, P3RS and P3WRO).

---

## 3. The security bar and the compatibility policy

**PLAN-SEC applies unchanged.** Every phase-3 message is on `/dspi/v1`, so it is served over SECSRV's TLS with the key
pinned at pairing; every REST endpoint is on the same listener with API keys, `Origin` refused and `plaintext_ok` as
before; the web pages keep the hub-served https path and the `uconsole web` pinning proxy. On top of that:
- **A flash command never travels in the clear.** `fw.*` is `tls_only` in every mode (OD-P3n): on an `on` or
  upstream-compatibility hub a plaintext session gets `tls_required`; a `--tls off` hub has no rollout. The web's
  rollout screen shows the TLS notice on an `http://` page.
- **Roles.** Admin: `group.set/delete`, `fw.*`, `inventory.*` writes as today. Control: `group.param.set`,
  `group.preset.load`, `group.apply`, each member judged by P1HL's limits and the lock (so a Control client's image on a
  limited device is `image_refused`, nothing sent). Viewer: every `*.list`, `*.get`, `*.status`. A group has no role
  (OD-P3a); Admin is still given only on the hub's machine.
- **The firmware image is checked three times before a byte goes out:** `uf2_inspect` (structure, family, window),
  `verify_bundled` against the manifest (required by default, OD-P3e; "not verified" is a labelled Admin choice on the
  hub's machine), and the firmware guard with no typist, once in the service and once in the writer. The guard's
  table is unchanged: the hub only refuses where a person would be asked.
- **The hub's own writes stay three.** The rollout's `REQ_ENTER_BOOTLOADER` is the Admin's child session's, judged by
  role, the installer lock and the device lock; the bootloader is reached over USB only; the writer talks to the boot ROM,
  not to any DSPi.
- **Bounded:** images 8 MiB, 4 kept, 48 KiB chunks, one upload per session with a 2 min deadline; 64 groups of 64;
  256 `results` rows a run; 64 rollout records; every data file 0600 and atomic with the corrupt-file rule.
- **No secrets, no image bytes in logs, events, `Debug` or the bundle:** `groups.json` and `rollout.json` go into the
  diagnostics bundle (they hold names, serials, hashes and step texts, no secret); the images never do. A test per WP
  greps its log output for base64 runs.
- **Never:** `EXT_CORE_VREG` written (read back only); `REQ_CLEAR_CLIPS`; a retry of a flash, a row or an apply; a
  second device while one is in BOOTSEL; an automatic recall after a flash (the export is offered, not applied).
- **What a stolen Admin token could do, and what stops it:** start a rollout of an image the hub already holds, over
  TLS only, with a verified image only (by default), on a muted device or with `while_live` spelled out, recorded in
  the history with the client's name, and alarmed on failure. Admin tokens idle for 180 days expire (OD-SECg) and are
  given only on the hub's machine.

**The compatibility policy (owner, 2026-11-15).**
- **No shims between our components.** The web reads every hub family through the core's typed calls, so a hub without
  a cap simply shows no screen (`hub_caps()`), and a core without the call fails the build, not the run. `groups.json`,
  `rollout.json` and `firmware/` are new files with no migration; the inventory and schedule files are read as they are,
  and a schedule rule with a `group:` target made before phase 3 simply starts working.
- **Our clients against an upstream Weeb Labs hub keep working:** every phase-3 call answers `Unsupported` without the
  cap and sends nothing (P2CC1's rule, tested per call in P3CC); the fleet screens, the health board's new columns and
  the rollout screen do not show; the Overview is the session's own and needs no hub.
- **USB to the hardware keeps working:** the clients' own flash path (`flash_confirmed` to a drive, `uconsole flash`) is
  untouched; the writer is new code behind the same guard and token; the K340 standing rules (1.20 V, backup first,
  settings verified identical) are what P3HIL does by hand once and what the rollout does every time.
- **Our hub against upstream clients, best effort:** the new types are unknown to them and ignored; `fw.install` stays
  `unsupported` and `fw_install` out of `caps`, so an upstream console never offers a hub flash; `device.list` and every
  `/dspi/` path are unchanged.

---

## 4. Decisions

Every decision below carries a firm recommendation; following the owner's rule of 2026-11-15 they are taken as
recommended unless the owner objects. Section 6 lists what remains for the owner (one action, no question).

| # | Decision | Options | Recommendation | Why |
|---|---|---|---|---|
| **OD-P3a** | **The group model** (PRO1 F3 gives a group "a role") | (1) name, note, members; no role, no nesting, a device in many groups; (2) a role per group (a fourth access mechanism); (3) nested groups | **(1)** | Three roles only is the owner's rule, and the roles are per client; a per-group role would be a second access system to keep sound. Nesting buys nothing at 64 devices and complicates every expansion bound |
| **OD-P3b** | **Where membership is read** (PLAN-P2 section 3 has inventory records gain `groups`) | (1) `group.list` is the source, `inventory.list` unchanged, the web joins; (2) `inventory.list` records gain `groups` | **(1)** | One owner of the data; no cross-service read on `inventory.list`'s hot path; a forgotten device is handled in one place (`DeviceForgotten` on the bus) |
| **OD-P3c** | **Linked parameters: one-shot or standing** | (1) one-shot fan-out with offsets and maps, the rows reported; (2) standing links (a change on one member is copied to the others) | **(1)** | A standing link is a hub-side write loop that fights every other writer (a knob, a schedule, another console) and would be a fourth kind of hub-own write. PRO1's wording is a fan-out |
| **OD-P3d** | **Bulk apply's guards** (PLAN-P2 section 3: "one at a time or in parallel by a setting") | (1) sequential, one guard per device owned by the caller, one confirm or cancel for the run; (2) parallel applies; (3) one shared guard | **(1)** | The executor is one core per (principal, device) and a device's queue is the pacing anyway; parallel buys seconds and costs a second scheduler. One guard per device keeps P1HA's semantics (the revert is per device, to its own captured state); a shared guard would revert a device whose apply never ran |
| **OD-P3e** | **Where a rollout's image comes from, and how it is trusted** | (1) chunked upload over DSPi Link by an Admin plus `uconsole hub firmware add` on the hub's machine, `verify_bundled` **required** by default with `--allow-unverified-firmware` to lift it; (2) only images bundled in the hub binary; (3) any image, labelled | **(1)** | The Admin is at a browser on the LAN, not at the hub machine; the manifest check is the one integrity control the core already has (SECFWV), so a venue hub flashes only what the release carried, while the owner's bench lifts it with one switch that is shown as a warning like `--upstream-compat`. (2) makes every firmware change a hub release |
| **OD-P3f** | **The wire family for the rollout** | (1) our own `fw.image.*`/`fw.rollout.*` under cap `rollout`; `fw.install` stays `unsupported`; (2) the spec's `fw.install` with `FwData` binary frames | **(1)** | The spec's shape streams an image per command from a client into a hub for a single device and no hub implements it (`HUB.md:343`); ours needs a store, a queue, a verify and a record, and an upstream console gains nothing from a hub flash it cannot verify. Best effort toward upstream clients is the policy |
| **OD-P3g** | **Whose write is the bootloader entry** | (1) the Admin's child session through the executor's core, the confirmation token minted inside (the `preset.save` with `"confirm": true` pattern); (2) a fourth hub-own write as `INTERNAL_SESSION_ID` | **(1)** | Every write judged: role, the installer lock and the device lock apply, the history names the Admin, and the closed list of three stays closed (OD-P2o) |
| **OD-P3h** | **Failure policy** | (1) stop a group rollout at the first failure, never retry, alarm `rollout_failed` critical, offer the export's recall, never apply it; (2) continue with the other members; (3) retry once; (4) recall the export automatically when settings changed | **(1)** | A bad image should take out one device, not a rack; the no-replay rule is the hub's since HB0 (`HUB.md:792`); a recall across firmware versions can itself be refused, so it is a person's decision with the result in front of them |
| **OD-P3i** | **Identifying the board in BOOTSEL** | (1) refuse to start while any board is in BOOTSEL; after entry exactly one must appear, on RP2350 with the DSPi's serial; `EXCLUSIVE_AND_EJECT` first; (2) match by bus location only; (3) flash whatever appears | **(1)** | OD-P2b's "refuse when more than one boot-mode device is present", made checkable before and after; the RP2040 boot ROM has no unique serial, so uniqueness is the only proof there; the eject stops the kernel's mass-storage probe that made the boot ROM stop answering on the bench (`dspi-firmware-env`), which with nobody at the rack would mean a replug nobody can do |
| **OD-P3j** | **U1's name and scope** | (1) a web screen "Overview", web only in phase 3; (2) "System" as PRO1 says; (3) all clients now | **(1)** | "System" is the alink system view and glossary term; "Overview" says what it is. PRO1 says web first, and the Qt, Android and Swift versions are one WP each in a later batch (Swift on the VM, kept separate) |
| **OD-P3k** | **`FleetBackend` and the fleet screens** | (1) not widened: the hub's families reach the page through the session's core, as P2WB1 built it; models keyed by (hub id, serial); (2) `FleetBackend` gains `inventory`, `health`, `alarms`, `groups` as PLAN-P2 section 3 wrote | **(1)** | No client speaks the hub's JSON (phase 2's rule); a fleet server of phase 4 would merge hubs in a core of its own, not in the page. Changing it now is a shim between our own components |
| **OD-P3l** | **REST in phase 3, then the freeze (OD-P2a (ii))** | (1) add group reads and writes, bulk apply, rollout and firmware **reads**; no rollout start or upload over REST; freeze after P3-C; (2) everything over REST too; (3) freeze now without groups | **(1)** | A flash is an Admin's console act with a dialog; a venue controller has no business starting one, and the 64 KiB body cap stays. Groups and applies are what integrators asked for (PRO1 I1, F4). Freezing after the fleet endpoints avoids a v2 for groups, which was OD-P2a's reason |
| **OD-P3m** | **The health board's data** | (1) P2AL's rules are the thresholds; the health poll gains counters, clip per minute and last change; no FW6; (2) thresholds of the board's own; (3) FW6 in the firmware | **(1)** | One set of thresholds, set in one place and alarmed from it; the counters cost one 30 s read inside the budget; a wire change for a 30 s reading is not worth it (PLAN-P2's FW6 note) |
| **OD-P3n** | **Rollout over plaintext** | (1) `tls_only` for every `fw.*` message in every mode; (2) as other Admin messages (plaintext allowed in `on` and compat) | **(1)** | A flash command and an image upload in the clear on a venue Wi-Fi is the one thing in this phase A2 could turn into a bricked rack. The cost is one check on a session flag SECSRV already computes per connection |
| **OD-P3o** | **Where the "before" export lives** | (1) the hub's snapshot store, counting against its caps, `conflict` and no flash when full; (2) a folder of the rollout's own; (3) in `rollout.json` | **(1)** | It is a snapshot: listed, recallable and copied home with the existing calls and UI; a full store is a condition to fix, not to flash through |
| **OD-P3p** | **Several hubs** | (1) not in phase 3; `hub` stays in every answer; (2) a hub list in the page now | **(1)** | PLAN-P2 section 3 and PRO1 phase 4: only if asked. The proxy and the `hub` field make it additive later |
| **OD-P3q** | **How the writer is proven on hardware** | (1) P3HIL: the K340 re-flashed once through the hub with the image it already runs, backups first, by the coordinator with the owner's go; (2) a new image; (3) no hardware run | **(1)** | Nothing changes on the unit but the path, so a fault shows as a difference in the flash read or the settings; the standing rules (1.20 V, backup, verify) are kept to the letter; the container WPs cannot touch USB |
| **OD-P3r** | **Flashing while audio may be playing** | (1) refuse unless the device is master-muted or the request carries `while_live: true` (the dialog spells it out); (2) the hub mutes first (a hub-own write); (3) no check | **(1)** | The reboot cuts audio for about 30 s whatever is done; the question is whether a person chose that. The hub muting would be a fourth hub-own write that the reboot undoes anyway (the firmware boots with its stored state) |

---

## 5. What not to do in phase 3

- Change `device.list`, `/dspi/` paths, `fw.install`'s answer or any DSPi Link 1.2 message's meaning; everything is
  additive and announced in `caps`.
- Write `EXT_CORE_VREG`, type `OVERVOLT` or the guard's phrase on anyone's behalf, flash an image the guard does not
  clear outright, flash two devices at once, mount a bootloader drive on the hub, or retry a flash.
- Add a hub-own write, a new error code, a standing link between devices, or a role on a group.
- Start a rollout or upload an image over REST, or accept a `fw.*` message over plaintext.
- Widen `FleetBackend`, let the fleet screens become a second console (a device opens in the existing session view), or
  build fleet pages in Qt, Android or Swift.
- Build a fleet database, a cloud relay or a second hub page for several hubs (phase 4, only if asked).
- Open a pull request to Weeb Labs for anything in this phase.

---

## 6. Owner questions

**None.** Every decision in section 4 has a firm recommendation.

**One owner action (not a question):** a go for P3HIL, the one real rollout of the K340 PCB unit through the hub with
the image it already runs (same SHA-256, 1.20 V), backups first, by the coordinator. The standing go of 2026-11-14
covers alink USB-direct test images only, so this flash is asked for once, when P3-C has merged.
