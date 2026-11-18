# PLAN-PG: both protocol generations in every component

*WP PLANPG, 2026-11-18, written on `main` at `c06c140f`. The inputs are the two upstream cross-checks
(`work/upstream-xcheck/FIRMWARE.md`, `work/upstream-xcheck/APPS.md`), upstream firmware `origin/release/v1.1.7` at
`103ba37` (1.1.7 beta 1, untagged: `firmware/DSPi/vendor/`, `Documentation/protocol/dspi_commands.json`,
`legacy_map.json`, `Documentation/Features/vendor_protocol_migration_guide.md`), upstream DSPi-Link at `4aec5ab`
(spec 1.3, `policy/commands.json`, JS library 1.3.0), DSPi-Zero at `192df73`, our firmware `bofr-main` `8ba8b72` plus
PAGEGUARD (`wp/PAGEGUARD` `df53ede`, finished today), and this repository's code, cited `file:line`. Plan plus one
compiling seam (section 2, "The seam"). No other code.*

**The owner's requirement (2026-11-18), binding:** *"make sure we stay compatible with both the older protocol (with
and without our extensions) and the new upstream protocol. This applies to all of our components."* (memory
`dual-protocol-policy`). Every component (firmware, core, transports, DSPi Link client, our hub, every client, the
updater and the rollout) must work with:

| Id | Firmware | Protocol |
|---|---|---|
| **G1s** | stock upstream 1.1.6 (and older), incl. DSPi-Zero preview builds | old: page 0 only, no extensions |
| **G1b** | our `bofr-main` (1.1.6 generation), with PAGEGUARD (refuses any non-zero `wIndex` high byte) or an older image without it; also the alink sender (`firmware/DSPi-Sender`, same VID:PID, guarded too) | old + our extension channel `REQ_EXTENSION` 0x00 |
| **G2s** | stock upstream 1.1.7 | paged; mandatory `SYS.PROTOCOL_INFO` probe; page 0 refuses everything |
| **G2b** | our future port of `bofr-main` onto 1.1.7 | **dual-stack** (OD-PGb): paged + page 0 legacy (incl. 0x00); extensions also on private page 0xE0, tunnel `0xE010`, `wValue` = extension id |

and, on the DSPi Link side, with:

| Id | Hub | Link |
|---|---|---|
| **L12** | upstream 1.2 hubs: bridge `bridge-v0.1.0`, Mac and Windows consoles v1.1.6, the ESP32 media player v1.5.0 | 1.2: legacy opcodes, policy by `bRequest` |
| **L13** | upstream 1.3 hubs: DSPi-Link head (bridge, JS library 1.3.0), later Mac/Windows | 1.3: policy by command id, `protocol` in the device object, forwards nothing to a legacy device |
| **OH** | our hub (`crates/dspi-hub`), hub-api v1 frozen: additive only | 1.3 plus our families; serves both generations (OD-PGg) |

**Goal.** One core decides, per connection and before anything else is sent, which generation a device speaks, and
from then on speaks only that generation to it; every layer above the transport keeps its existing 1.1.6-shaped
calls, and an adapter generated from upstream's own `legacy_map.json` turns them into paged requests on a paged
device. Our extensions keep their ids and payloads and change only their envelope. Our hub serves both generations to
both DSPi Link versions. Our firmware is ported onto 1.1.7 as a dual-stack image. Every combination has a fake and an
expected outcome, tested without hardware; one HIL WP checks the real boards.

**Rules this plan builds on:** PLAN.md section 1 (the core owns logic; typed writes through `Writable`; read-only and
locks before anything is sent; no panic on device, network, file or caller data); PLAN-CLIENTS section 1 (web rules:
device I/O behind `Transport`, no ungated OS calls); the compat policy (memory `owner-compat-policy`: our clients must
keep working against an upstream Weeb Labs hub and over USB; our hub against upstream clients is best effort; no shims
between our own components; no PRs to Weeb Labs); the K340 rules (1.20 V core, never stock firmware on a K340, memory
`k340-brownout`); the process (memory `agent-process`: parallel Opus WPs in waves of disjoint files, one Fable review
per wave, toolchains in containers, no tmpfs, no devices in a WP). **Naming:** "alink" is the audio link, "DSPi Link"
the network protocol (memory `alink-dlink-naming`); "legacy" means the 1.1.6 generation, "paged" the 1.1.7
generation; a "page-0 request" is a 1.1.6-shaped one.

**What exists today (checked on `c06c140f`, `bofr-main` `8ba8b72`, PAGEGUARD `df53ede`):**

| Need | State | Where |
|---|---|---|
| A page on the wire | **None anywhere.** `Transport` has no `wIndex` (`crates/dspi-transport/src/lib.rs:1-5`, `:297-317`); every backend sends `wIndex` = 2 (`usb.rs:186`, `midi.rs:495`, `link.rs:466`, WebUSB `apps/web/src/core/usb.ts:211`) | the seam adds the paged calls (section 2) |
| Probe | `probe` reads `REQ_GET_PLATFORM` first (`crates/dspi-session/src/probe.rs:566`); nothing knows `SYS.PROTOCOL_INFO` | PGWIRE |
| Feature detection | "probe and cache": a STALL means absent (`probe.rs:1-10`, `:630-667`) | kept: the adapter refuses absent paged commands locally (section 1.2) |
| Extension channel I/O | one place, `dspi_session::ext` (`crates/dspi-session/src/ext.rs`), codecs in `dspi_proto::ext` | unchanged above the adapter (OD-PGc) |
| Our hub and `wIndex` | **dropped**: `execute` forwards `bRequest`/`wValue` only (`crates/dspi-hub/src/device.rs:571-596`), and the test `wp_hb0_windex_is_ignored` (`device.rs:871`) asserts it. A paged request from a 1.3 client therefore reaches a legacy device as the old opcode with the same number (e.g. a `ROUTE.MASTER_VOLUME_SAVED` GET 0x0553 runs `FACTORY_RESET` 0x53). PAGEGUARD cannot see it (the page is gone before the device) | **PGHUB0, first** |
| Hub policy | one table keyed by the 8-bit opcode (`crates/dspi-hub/src/policy.rs:28`, `:68-145`), lock scope (`:210`), core-voltage rule (`:230`), busy reads (`:79`); `tests/fixtures/link/policy/commands.json` is the 1.2 table | PGHPOL |
| Link client | `PROTO_MINOR` 2 (`crates/dspi-link/src/messages.rs:15`); `DeviceInfo` has no `protocol` (`messages.rs:202`); `policy.denied` parsed as 8-bit codes (`client.rs:973-978`) | PGLINKC |
| Fake device | 1.1.6-shaped, `FakeOptions.extensions` is G1b vs G1s (`crates/dspi-transport/src/fake.rs:217-268`); about 800 opcode references across `fake/` | PGFAKE adds the paged generations |
| Raw paths | `raw_get`/`raw_set` take an 8-bit opcode (`crates/dspi-core/src/raw.rs:69`, `:125`), exported over UniFFI and C; Swift's `CoreBackedTransport.swift` tunnels its own opcodes through them; Terminal `uconsole raw`, the TUI's `ReadOnlyTransport` (`crates/dspi-tui/src/shell/read_only.rs:342`) | keep (through the adapter); add id forms (PGCORE) |
| Host transport | `HostTransport` (`crates/dspi-core/src/host_transport.rs:86-88`), opcode only, retires at SW-M3 | stays legacy-only (OD-PGk) |
| Measurement | siggen and analyser through typed core calls on opcodes 0xA4-0xA8 and the RTA set (`crates/dspi-core/src/stats.rs:255-397`, `rta.rs`); PLAN-P5's WPs build on them | unchanged above the adapter (PGMEAS) |
| Updater, rollout | bootloader entry 0xF0 (write-as-read), the guard (`crates/dspi-core/src/firmware_guard.rs`), manifest (`firmware_manifest.rs`), PICOBOOT (`picoboot.rs`), hub rollout (`crates/dspi-hub/src/rollout.rs`) | PGFWUP |
| Firmware | `bofr-main` speaks 1.1.6 + extension 0x00; PAGEGUARD refuses every non-zero page on USB EP0, both dispatchers, UART, I2C, MIDI and the sender (its report, table "What is guarded"); 0xE0 is refused too; K340 `.data` is 99 512 of 100 352 bytes | the port (FWPG1-6) |

**Corrections to the cross-checks found while checking:**
- APPS.md (C4, R5) says upstream reserves no page for third parties. **It does:** pages 0xE0-0xEF are "private use for
  forks and ports, never allocated upstream" (FIRMWARE.md section 0, read from the overhaul plan). The extension page is
  0xE0 (OD-PGc).
- APPS.md R2 sizes the core adapter at 11 units and R1-R4 at 17. That estimate covers USB and the Link client only. The
  owner's requirement adds: the hub serving both generations to both Link versions with re-keyed judges, fakes for
  every generation, the hazard guard in our hub, the extension envelope, the updater per generation and board, every
  client, and a compat matrix. **This plan is 42 units of app WPs plus 1.5 HIL, and 18 units of firmware** (section 2,
  "Waves").
- FIRMWARE.md estimates the port at "about 5 WPs". Dual-stack (OD-PGb) adds one (FWPG3, the legacy front), the sender
  and the envelope spec add a little: 7 firmware WPs and one firmware HIL.
- The hub hazard above is live today, not only after a 1.3 client exists: any client that sets a `wIndex` high byte
  (a raw tool, a future upstream console) reaches our legacy devices with the wrong command. PAGEGUARD's report raised
  it; PGHUB0 closes it.

---

## 1. The design

### 1.1 Generation detection (the probe)

The core decides the generation once per connection, in `dspi_session::probe`, before any other request. The probe is
safe on every generation: 1.1.6 has no IN handler for `bRequest` 0x10 and STALLs; PAGEGUARD STALLs it too (status 0x02
on UART, I2C and MIDI); 1.1.7 answers 16 bytes starting `DSPi` (migration guide 4.2-4.4).

1. **If the link carries pages** (`Transport::carries_pages`, seam): send `SYS.PROTOCOL_INFO` (GET `0x0110`, 16
   bytes). A timeout or busy answer is retried three times about 150 ms apart (a flash write may blind the device);
   after that the connect fails with a transfer error. **A failure never falls back to a guess.**
2. **The reply decodes as paged** (`ProtocolInfo::decode` = `ProbeReply::Paged`, seam): the device is **paged**. Read
   `SYS.PLATFORM`, `SYS.PAGE_LIST` and `SYS.PAGE_COMMANDS` for every listed page, once, batched; keep them for the
   connection. If page 0xE0 is listed, read `EXT_LIST` through the tunnel `0xE010` (ext id 0): the device has our
   extensions (G2b), else none (G2s). The connection is wrapped in `PagedTransport` (section 1.2).
3. **`UnknownMajor`** (a later protocol major): refuse with the notice "this device's firmware uses a newer protocol;
   update this app". **`Garbled`** (an answer that is not a protocol-info block): refuse with "unrecognised device".
   Send nothing else in either case: something that answers a page-1 request is not 1.1.6 firmware.
4. **The probe STALLs** (USB), answers status 0x02 (MIDI), or the link cannot carry pages: the device is a **legacy
   candidate**. Read `REQ_GET_PLATFORM` as today. If it answers, the device is **legacy**; the extension probe follows
   as today (`EXT_LIST` on 0x00): G1b or G1s. If it STALLs as well: refuse with "unrecognised device" (it may be a
   1.1.7 device behind a link that dropped the page).
5. **Over DSPi Link** the hub's device object decides first (section 1.4): a device with `protocol.major == 1` is paged
   without a client probe; one without `protocol` is legacy if the hub forwards to legacy devices (an L12 hub, or OH
   with the cap `legacy_devices`), and otherwise is shown with the notice "this hub serves only firmware 1.1.7 and
   later" and sent nothing (an L13 hub answers `0x86` to every CMD for it).
6. **Through an L12 hub the client speaks legacy only, whatever the device is** (OD-PGe): a 1.2 hub classifies by
   `bRequest`, and our own hub until PGHUB0 even drops the page. The client still sends the probe (harmless) to word the
   notice: a paged answer followed by a refused `REQ_GET_PLATFORM` is "this hub is too old for this device's firmware
   (DSPi Link 1.2 cannot carry firmware 1.1.7)". A G2b device answers `REQ_GET_PLATFORM` on its legacy stack and is
   then used as a legacy device, extensions included.
7. **The generation is fixed for the connection** (`Generation`, seam) and lives in the type of the wrapper
   (`PagedTransport` or `LegacyGuard`); it is re-probed on every reconnect, after every firmware update and after every
   bootloader return. It is never cached across connections or by serial.

**The two hard rules, each enforced in two places:** never a paged request (other than the probe) to a legacy device
(`LegacyGuard` refuses every paged call locally; our hub refuses page != 0 toward a legacy device, PGHUB0); never a
page-0 request from this client to a paged device (`PagedTransport` sends page 0 never: every legacy call becomes a
plan of paged steps or a local refusal). The fakes record violations (section 4) and every matrix test asserts none.

### 1.2 One typed command layer: the adapter under the 1.1.6-shaped calls

```
 core / session / hub internals      control_in(opcode, wValue, len), control_out(...), control_in_batch(...)   (unchanged, ~480 call sites)
            |                                         |
   LegacyGuard<T>  (legacy devices)          PagedTransport<T>  (paged devices)
   passes legacy calls,                       legacy call -> dspi_proto::paged::plan(...) -> paged steps on T,
   refuses every *_paged call                 reply re-assembled into the 1.1.6 shape; *_paged calls pass through
            |                                         |
 raw wire T: UsbTransport, MidiTransport, LinkTransport, WebUsbTransport, FakeDevice, MockTransport
            (control_in/out = page 0;  control_in_paged/out_paged = page from the CmdId)
```

- **The planner is pure and generated** (`dspi_proto::paged`, PGPROTO). `build.rs` reads the vendored
  `dspi_commands.json` (the registry: id, directions, address kind, GET length, SET bounds, flags, build condition) and
  `legacy_map.json` (246 opcodes: 192 `direct`, 27 `act_*`, 15 `split`, 14 `custom`) and emits typed constants
  (`paged::cmds::ROUTE_MASTER_VOLUME` and so on) and the row tables. `plan(opcode, dir, value, payload_or_len, &PageSet)`
  returns a small step machine: one or more paged requests and how to build the 1.1.6 reply from their answers (the
  status byte of an `act_*` row from its status family, `GET_STATUS` fields from their structs and offsets). The 14
  `custom` rows are written by hand from the migration guide sections 6.3 and 6.5 (PGCUST). **Our extension row** is
  part of the planner: opcode 0x00 becomes the tunnel `0xE010` with the same `wValue` and payload when page 0xE0 is in
  the `PageSet`, else a local STALL (so `dspi_session::ext` sees "no extension channel", exactly as on G1s).
- **The reverse view** (`paged::legacy_equivalent(id, dir, value, payload)`), used by the hub's judges (section 1.6):
  a paged request as the 1.1.6 request with the same effect, `None` when it has none (new commands such as
  `SYS.PIN_STATE`). Written with the planner, property-tested against it (planner then reverse is the identity on every
  invertible row).
- **`PagedTransport<T>`** (PGADAPT, `crates/dspi-transport/src/paged.rs`): runs plans on `T`'s paged calls; refuses
  locally, as `Stalled { opcode }`, every legacy call whose target is not in the `SYS.PAGE_COMMANDS` bitmaps, so the
  probe-and-cache feature detection above it keeps working without a STALL on the wire (upstream: do not probe by
  STALL, migration guide 4.7); after a real STALL on USB or MIDI reads `SYS.LAST_RESULT` once and returns a typed
  `TransportError::Refused { id, result }` (new variant, PGADAPT); retries only what upstream allows (0x01, 0x04, 0x17,
  0x84 over Link; `VC_ERR_BUSY` 0x17 on USB through the existing busy window); never sends a GET with `wLength` 0;
  checks SET lengths against the registry before sending (exact lengths are enforced by 1.1.7); refuses NaN and
  infinity in float payloads locally where the registry types the field (1.1.7 refuses them on single SETs).
- **Paged-only commands** that have no 1.1.6 form (discovery, `SYS.LAST_RESULT`, the pin queries `SYS.PIN_STATE`
  0x0150 and `SYS.PIN_CLAIMS` 0x0151) are called through typed functions over `control_in_paged`; this plan adds only
  the discovery and `LAST_RESULT` ones. Pin queries are section 8 ("Later").
- **Bulk:** the blob is unchanged (wire format 32, 6136 bytes). `0xA0`/`0xA1` map to `STORE.ALL_PARAMS` 0x0240 and
  `0xA2`/`0xA3` to `STORE.ALL_PARAMS_CHUNK` 0x0241 (`SET_ALL_PARAMS` is a custom row for its 1.1.6 length quirks).
- **Flash save:** 1.1.7 has no `SAVE_PARAMS`; the custom row is `STORE.PRESET_ACTIVE` then `STORE.PRESET_SAVE` to that
  slot (`legacy_map.json` `save_params`). The gated core call is unchanged.
- **Notifications:** packet format unchanged; the core ignores unknown events today and learns
  `NOTIFY_EVT_PIN_MAP_CHANGED` 0x11 (re-read pins, PGCORE). Our 0xE0-0xE3 events stay.

### 1.3 Our extensions over both envelopes (OD-PGc)

| Generation | Envelope | Who uses it |
|---|---|---|
| G1b | `REQ_EXTENSION` `bRequest` 0x00, `wIndex` 2, `wValue` = ext id | every host, as today |
| G2b, paged stack | page 0xE0, `bRequest` 0x10 (id `0xE010`), `wIndex` 0xE002, `wValue` = ext id; GET and SET; payloads, `EXT_LIST`, ids 0x0000-0x0006 and 0x8000-0x8012 byte-identical | our clients (through the planner's ext row) |
| G2b, legacy stack | `REQ_EXTENSION` 0x00 on page 0, served by the same handlers | legacy hosts only (older apps, the ESP32 panel over UART, an L12 hub) |
| G1s, G2s | none: the probe finds no channel | |

Ext notifications 0xE0-0xE3 and `EXT_NOTIFY_FEATURES` are unchanged. `SYS.PAGE_LIST` on G2b lists page 0xE0 (FWPG2),
so a paged host can see it. Over an L13 upstream hub page 0xE0 is "absent from its embedded table" and so `config`
in both directions (spec 1.3 section 9.3): admin only, as `REQ_EXTENSION` is on an L12 hub today
(`crates/dspi-hub/src/policy.rs:3-10`); our clients treat a `DENIED` read as "no extension channel" as today.

### 1.4 The DSPi Link client, 1.2 and 1.3 (PGLINKC)

- **Version:** the client says `hello.proto.minor` 3 and reads the hub's minor. Minor 2 or lower is an L12 hub; minor 3
  is an L13 hub, or OH when `caps` names ours.
- **Device object:** `DeviceInfo` gains `protocol: Option<{major, minor}>` and our additive `ext: Option<bool>` (OH
  only). Section 1.1 step 5 decides the generation from it.
- **CMD and polls:** `wIndex` = `(page << 8) | 2` for paged ids (`CmdId::w_index`, seam); page 0 for legacy; a GET
  always carries a non-zero `wLength`; bulk always single-shot (`STORE.ALL_PARAMS`, never the chunk form over Link,
  spec 8.2). A poll on a paged device subscribes to the paged GET of the planner's single-request plans; a legacy poll
  whose plan is several requests (`GET_STATUS` selectors map to one paged GET each with an offset, so each stays one
  subscription) is refused at subscribe time with a typed error, never half-done.
- **Policy lists:** `policy.denied_ids` (`[id, dir]`) is parsed next to `policy.denied`; greying-out uses `denied` for
  a legacy device and `denied_ids` for a paged one. OH fills both (section 1.5); an L13 upstream hub sends `denied`
  empty.
- **Statuses:** 0x10-0x1A are typed (`VC_ERR_*`, `TransportError::Refused`); 0x02 and 0x03 from an L13 hub mean a
  legacy device or an unexplained STALL; `0x86` for a device without `protocol` on an L13 hub is the notice of section
  1.1 step 5, not a connection failure. Retries only for 0x01, 0x04, 0x17, 0x84.
- **The fake hub** (`crates/dspi-link/src/fakehub.rs`) gets three modes: `L12` (policy by `bRequest`, drops `wIndex`
  like our hub before PGHUB0), `L13` (upstream rules: probes, `protocol`, forwards nothing to legacy, `0x86`, policy by
  id, `denied` empty) and the existing behaviour. OH itself is tested in-process (section 4).

### 1.5 Our hub: both generations, both Link versions (PGHUB0, PGHPOL, PGHDEV)

**PGHUB0 (now, before anything else):** refuse every CMD and every `poll.subscribe` whose `wIndex` high byte is not 0
with `0x86 UNSUPPORTED` (the 1.3 rule for legacy devices), since every device our hub attaches today is legacy. That
closes the hazard whatever else lands. `wp_hb0_windex_is_ignored` changes to assert the refusal for 0xFFFF and the
pass-through for 0 and 2 (the Mac's `WINDEX_GLOBAL` 0 must keep working).

**Afterwards (PGHDEV):** the hub probes every device on attach as section 1.1 (the hub is a client of its devices), and
wraps it in `LegacyGuard` or `PagedTransport`. Forwarding by the request's page and the device's generation:

| Request from a client | Legacy device (G1s, G1b, Zero, sender) | Paged device (G2s, G2b) |
|---|---|---|
| page 0 (L12 clients, our older apps, upstream 1.1.6 consoles) | forwarded as today, judged by the 1.2 table | **translated by the hub's adapter** (`PagedTransport`'s legacy path), judged by the 1.2 table (OD-PGh); the legacy extension 0x00 becomes the tunnel when the device has page 0xE0 |
| page != 0 (L13 clients, our clients to a paged device) | **refused `0x86`** (PGHUB0's rule, kept) | forwarded verbatim (`control_*_paged`), judged by the 1.3 table; firmware statuses 0x08-0x7F forwarded verbatim; a USB STALL explained by `SYS.LAST_RESULT` |

- **Device object:** `protocol` for a paged device (spec 1.3); absent for a legacy one, which OH still lists and
  forwards to (deviation from spec 1.3 section 7.4, needed by the owner's requirement, OD-PGg); our additive `ext`
  field (true when the extension channel answered). Our cap `legacy_devices` tells our clients that OH forwards to
  legacy devices. `hello.proto.minor` 3.
- **Hub-originated requests** (health, identify, alarms, limits, apply guard, bulk gain, the local hub's direct path,
  P3RO's rollout) keep their 1.1.6-shaped `CmdRequest`s and reach a paged device through the device's
  `PagedTransport`; nothing in them changes.
- **Upstream clients (best effort):** L12 clients keep working with every device, paged ones included, through the
  translation; L13 clients see paged devices with `protocol` and leave legacy devices alone (spec-conformant). The hub
  page we serve (DSPi-Link web build at `86f10e3`, 1.2) keeps working with both generations through the translation:
  **do not re-vendor it** (APPS C3(d) said "not before our hub speaks 1.3"; with the translation there is no need at
  all, and the 1.3 page would show our legacy devices without controls).
- **Analyser sharing** (spec 7.15) by id on paged devices; the swallowed `MEAS.RTA_STOP` is answered `OK` with no
  payload as spec 1.3 says. **Bulk relay** on 0x0240 as on 0xA0/0xA1. **Busy reads** from both tables.

### 1.6 The tables, re-keyed on (page, cmd) (PGHPOL, PGCORE)

**One rule for every judge: judge the request by its 1.1.6 equivalent when it has one.**

| Table | Today | After |
|---|---|---|
| Hub policy (`policy.rs`) | `BTreeMap<u8, Row>` from the 1.2 `commands.json` | `classify(CmdId, Dir)`: page 0 by the 1.2 table (kept, renamed `commands-1.2.json`); page != 0 by the 1.3 table vendored from DSPi-Link (`policy/commands.json`, MIT, provenance recorded) plus our private rows (`policy/private.json`: `0xE010` GET and SET `config`); anything absent is `config` (spec 9.3). `denied` from the 1.2 table, `denied_ids` from the 1.3 and private tables |
| `mutates` | by opcode, write-as-reads counted | page 0 as today; page != 0: SET only (every 1.1.7 GET is a read) |
| Lock scope (`in_lock_scope`, `policy.rs:210`) | opcode list | `legacy_equivalent` then the same list; no equivalent: outside the scope (refused for Control while locked) |
| Core-voltage rule (`core_vreg_write`, `policy.rs:230`) | `0x00` SET with `wValue` `EXT_CORE_VREG` | the same, and the tunnel `0xE010` SET with the same `wValue` (its equivalent) |
| Limits judge (P1HL, `crates/dspi-hub/src/limits.rs`), apply guard, bulk gain | decode by opcode and payload | fed the equivalent; a paged write with no equivalent that the limits profile would cover is `limits_unknown` (refused) when a profile is set |
| Core read-only mode (`crates/dspi-core/src/read_only.rs`) | `is_pure_read(opcode)` | typed calls unchanged (they are 1.1.6-shaped above the adapter); the new `raw_get_id` passes any GET (all are reads on 1.1.7; the tunnel GET only reads), `raw_set_id` is refused |
| Raw gates (`check_not_gated`, `raw.rs:71`) | gated opcodes (bootloader, factory reset, saves) | `raw_set_id`: gated when its equivalent is gated, or when `SYS.COMMAND_INFO` flags it `FLASH` or `RESTART`, or it has no equivalent and is not a plain value SET |
| TUI `ReadOnlyTransport` (`read_only.rs:342`) | deny by default by opcode | sits above the adapter (unchanged); forwards `control_in_paged` for GETs, refuses `control_out_paged` |
| Android `WriteGuardedCore` | classifies exports | `raw_set_id` write, `raw_get_id` read, the generation getters read |

### 1.7 Measurement on paged firmware (PGMEAS)

PLAN-P5's session drives the generator and the analyser through typed core calls (`stats.rs:255-397`, `rta.rs`), which
sit above the adapter: on a paged device `SIGGEN_CONTROL` 0xA6 becomes `MEAS.SIGGEN_START`/`STOP`/`STOP_NOW`
(0x0948-0x094A), `SIGGEN_SET_CONFIG` becomes `MEAS.SIGGEN_CONFIG` 0x0940 (36 bytes, exact), the analyser `MEAS.RTA_*`
0x0920-0x092A; `NOTIFY_EVT_SIGGEN_STATE` is unchanged. No P5 file changes. PGMEAS runs P5's session and live tests
against the G2s and G2b fakes, checks that a START is still one transfer (cycle identity, P5 section 1.1 step 5,
depends on it), and adds one line to PLAN-P5's "Firmware-dependent items" table. Through a hub the generator's class
is `control` in both tables.

### 1.8 Firmware updater and rollout, per generation and board (PGFWUP)

- **Bootloader entry** is `REQ_ENTER_BOOTLOADER` 0xF0 on a legacy device and `SYS.ENTER_BOOTLOADER` 0x0130 (a
  zero-length SET) on a paged one; the planner maps one onto the other, so the core's gated call is unchanged. On our
  firmware it stays USB-vendor only (FWSEC1; on G2b by `VC_F_SET_USB_ONLY`, FWPG2).
- **Images:** `firmware_manifest.rs` entries gain `lineage` (`upstream` or `bofr`) and `generation` (`legacy` or
  `paged`); the K340 legacy image stays available after the port (tag `bofr-116-final`, FWPG1) for a downgrade.
- **The guard** (`firmware_guard.rs`) adds: a **DSPi-Zero is never flashed** (refused, notice "update the SD card
  image", section 1.9); **bofr to upstream** (lineage loss: extensions, alink, the 1.20 V setting, and the directory:
  stock firmware reads our V27 directory as missing, so IO, control-interface and Control Surfaces config fall back to
  defaults) needs the typed confirmation, on top of the existing voltage rule that already blocks stock images on an
  at-risk board; a **generation change** is shown with what will work afterwards (e.g. "an L12 hub cannot serve
  firmware 1.1.7").
- **Rollout (P3RO):** after the reboot the hub re-probes (section 1.1 step 7) and verifies version and, on our
  lineage, `EXT_LIST` and the 1.20 V read-back through whichever envelope the new generation has; a group may mix
  generations; a Zero is excluded with a reason.

### 1.9 DSPi-Zero and the alink sender (PGZERO)

**DSPi-Zero** (upstream, GPL-3.0, bring-up): same VID:PID, product string "Weeb Labs DSPi Zero", `bcdDevice` 0x0300,
platform byte 1, byte 7 of an 8-byte `GET_PLATFORM` = board 1; its submodule is 1.1.6-generation today, so it is a
legacy device for the probe (and will be paged when it moves to 1.1.7: nothing here assumes either). Minimum support
(APPS 3.1, 1.25 units): read the product string and `bcdDevice` from the USB descriptor where the platform gives them
(rusb, WebUSB, Android), read 8 platform bytes, name the board "DSPi Zero" in caps, the inventory and About; **never
enter its bootloader** (it reboots into the SD card's loader, `CAFE:5A4C`, not an RP2350 BOOTSEL) and block the
updater and the rollout (PGFWUP); pin pickers keep RP2350 pins (the Zero refuses the ones it lacks, a refusal we
already show). The rest waits for upstream's platform id.

**The alink sender** (`firmware/DSPi-Sender`, ours, same VID:PID, guarded by PAGEGUARD): a G1b device for the probe
today; after FWPG4 a dual-stack paged device with its small vendor set on the paged stack and the tunnel. It is a row of
the compat matrix (section 4).

### 1.10 The firmware port onto 1.1.7 (FWPG0-FWPG6, FWPGHIL)

Plan now, **start FWPG1 when upstream tags 1.1.7** (or after a quiet week on `release/v1.1.7`; its tip moved today). All
firmware WPs are in `~/prog/DSPi-bofr`, branch `wp/<id>` from `bofr-main`, never pushed by a WP, never flashed by a WP
(memory `k340-flash-standing-go` covers reviewed alink-usb images only; a port image needs the owner's go, FWPGHIL).

- **A merge, not a rebase:** upstream's tag is merged into `bofr-main` (FWPG1); the last 1.1.6-generation commit is
  tagged `bofr-116-final`.
- **Dual-stack** (OD-PGb): page 0 keeps the 1.1.6 opcodes for legacy hosts, served by a **legacy front generated from
  the same `legacy_map.json`** that dispatches onto upstream's paged handlers (`ctrl_dispatch_*` with a `wIndex`), not
  by keeping 1.1.6's `vendor_commands.c` (4.9k lines and a second set of semantics). The 14 custom rows are by hand in C.
  Tables are `const` (flash): K340 `.data` has 840 bytes left. Upstream's own page-0 walk test is changed to expect our
  legacy answers (recorded as our divergence).
- **Extensions on page 0xE0** (`pages/ext.def`, `vc_page_ext.c`): one tunnel command `0x10`, GET and SET, `wValue` =
  extension id checked by our own handler (upstream's dispatcher range-checks `wValue` as an address); the same
  handlers serve `REQ_EXTENSION` 0x00 on page 0.
- **PAGEGUARD's guard is replaced**, not extended: on 1.1.7 the dispatcher refuses unknown pages itself; page 0 goes to
  the legacy front, 0xE0 to ours. (PAGEGUARD refuses 0xE0 on the 1.1.6 line, correctly: there the extension channel
  stays on 0x00.)
- **FWSEC1** becomes `VC_F_SET_USB_ONLY` on `SYS.ENTER_BOOTLOADER` and on the tunnel's SET where it carries
  `EXT_CORE_VREG` (a check inside our handler, since a flag is per command); `core_vreg_request_transport_ok()` stays;
  **MIDI counts as off-USB** for these flags (`CTRL_SOURCE_MIDI` joins `VC_NUM_SOURCES`). The legacy front's 0xF0 is
  gated the same way.
- **1.20 V on the K340** stays (`core_vreg_boot_apply()` replaces both hardcoded `vreg_set_voltage(VREG_VOLTAGE_1_15)`
  calls; FIRMWARE.md section 3 found the auto-merge keeps it).
- **Pins:** the alink, scope marker (GPIO 22/21), identify LED and GP23 become `vc_pins.c` claims, so `SYS.PIN_CLAIMS`
  never calls them free.
- **Directory numbering** (OD-PGl): ours moves to a private band (`0x80 | n`) at the port, with a one-time migration
  from our V26/V27 recognised by size, CRC and our block marker, before upstream's next directory change takes "V26".
- **USB MIDI** on `ctrl_dispatch_*` with `wIndex` (the SysEx frame already carries it), chunk sessions keyed on
  `STORE.ALL_PARAMS_CHUNK` 0x0241 on the paged stack and on 0xA2/0xA3 through the legacy front.

### 1.11 What works where: the expected matrix

Our clients (every client, through the core):

| Firmware \ route | USB | USB MIDI | L12 hub | L13 upstream hub | OH (our hub) |
|---|---|---|---|---|---|
| G1s | full, no ext | (no MIDI on stock) | full, no ext | notice: hub serves 1.1.7+ only; nothing sent | full, no ext |
| G1b (guarded or not), sender | full + ext | full + ext | full; ext admin-only (as today) | notice, nothing sent (a bridge does not list it) | full + ext |
| G2s | full (adapter), no ext | (no MIDI on stock) | notice: hub too old for 1.1.7; nothing more sent | full (adapter), no ext | full (adapter), no ext |
| G2b | full (adapter) + ext (tunnel) | full + ext (paged over MIDI) | full as a legacy device (its legacy stack) + ext admin-only | full + ext admin-only (0xE0 is `config` there) | full + ext |
| Zero (G1s today) | full as G1s, named "DSPi Zero", no update | — | full, no update | notice | full, no update or rollout |

OH with upstream clients (best effort):

| Client \ device on OH | G1s, G1b, Zero, sender | G2s, G2b |
|---|---|---|
| L12 client (upstream 1.1.6 consoles, our older apps, the vendored hub page) | as today | works through the hub's translation; legacy ext 0x00 reaches G2b's tunnel |
| L13 client (JS library 1.3.0, future consoles) | listed without `protocol`: the client sends nothing (spec-conformant) | full; ext invisible to it unless it uses 0xE010 (`config`) |

Old hosts directly on our firmware: an old app, the ESP32 panel (UART, control-interface v22) or `dspi_poke` keep
working on G2b through its legacy stack; on G2s they see a dead device (page 0 refused), which is upstream's design.

---

## 2. Work packages

**Shared rules for every PG WP.** `PLAN-CLIENTS.md` section 1, `WP-BRIEF.md`, the wave1 brief
(`work/wave1-brief.md`: containers only, no tmpfs, no devices, scoped checks, never push, the commit trailer), PLAN.md
section 1, and:
- **The hard rules of section 1.1 are tested in every WP that touches a transport, the probe, the hub or a fake:** a
  test that would let a paged request reach a legacy fake, or a page-0 request from our client reach a paged-only fake,
  fails. The fakes' violation counters (PGFAKE; PGWIRE's for the legacy fake) are asserted zero.
- **Generated, not transcribed:** every paged id, length, bound, flag and adapter row comes from the vendored JSON
  through `build.rs`; hand-written code exists only for the 14 custom rows, the extension row and the discovery
  decoders, each citing the migration guide section it implements. The seam's literals are checked against the
  generated table (PGPROTO).
- **Vendored files** (`crates/dspi-proto/firmware/protocol/`): `dspi_commands.json`, `legacy_map.json` from firmware
  `release/v1.1.7` at a pinned commit (GPL-3.0 project files, data); `policy/commands.json` and the fixtures from
  DSPi-Link `4aec5ab` (MIT). `PROVENANCE.toml` and `SHA256SUMS` rows; `ci/licence-inventory.py` passes. Re-vendoring
  when upstream tags 1.1.7 is PGREV (section 8).
- **Typed errors, no panics** on device or hub data; every planner and decoder has a `proptest_*` (no panic, no page-0
  step, no zero-length GET, SET length within bounds).
- **Bindings** (C, UniFFI, wasm) are shared and additive under markers: `ci/drift.sh write`, regenerate the Android
  contract, classify new methods in `WriteGuardedCore`. Apple bindings are regenerated by the coordinator at merge.
- **Notices:** `assets/notices/notices.json`, one contiguous block per WP, no host-device nouns.
- **Tests carry the WP id** (`wp_pgwire_*` and so on).

Sizes are this project's units (about one hour of agent time with review); estimates.

### The seam (committed with this plan)

- `crates/dspi-proto/src/paged.rs`: `CmdId` (`new`, `page`, `cmd`, `w_index`, `from_request`, `is_legacy`),
  `LEGACY_PAGE`, `PRIVATE_PAGES`, `EXT_PAGE` 0xE0, `EXT_TUNNEL` 0xE010, `SYS_PROTOCOL_INFO` 0x0110,
  `PROTOCOL_INFO_LEN`, `PROTOCOL_MAGIC`, `PROTOCOL_MAJOR`, `ProtocolInfo::decode` to `ProbeReply`
  (`Paged`/`UnknownMajor`/`Garbled`), `Generation` (`Legacy`/`Paged`); tests on the migration guide's own probe bytes.
  One `mod` line in `lib.rs`.
- `crates/dspi-transport/src/lib.rs`: three default methods on `Transport`: `carries_pages()` (false),
  `control_in_paged(id, value, max_len)` and `control_out_paged(id, value, data)` (both
  `Err(TransportError::Incompatible(NO_PAGES))`), and `pub use dspi_proto::paged::CmdId`. Nothing implements them yet,
  so every existing transport and wrapper compiles unchanged and fails closed.

### Wave PG-A: the hazard, the wire, the registry (start now)

**PGHUB0: Our hub refuses a page toward legacy devices.** [TODO] Size 0.75. Wave PG-A. **Priority: first.** **Owns:**
`crates/dspi-hub/src/device.rs` (`execute` and its tests, incl. `wp_hb0_windex_is_ignored` at `:871`),
`crates/dspi-hub/src/polls.rs` (the subscribe check), new `crates/dspi-hub/tests/wp_pghub0_page_guard.rs`.
**Implements:** section 1.5 "PGHUB0": a CMD or poll whose `wIndex` high byte is not 0 is answered `0x86 UNSUPPORTED`
before policy, lock or transport; `wIndex` 0 and 2 pass as today. **Done when:** every one of the 15 dangerous reads of
the migration guide 4.1 (e.g. GET `0x0553`, `0x0892`, `0x0891`, `0x0890`) and the SET `0x0844` sent through the real hub
in-process to a fake that counts requests answer `0x86` with **zero** requests reaching the fake; a page-0 request
from a Mac-style client (`wIndex` 0) still works; a poll on page 0x05 is refused; the old test asserts the new rule;
`cargo test -p dspi-hub` scoped to the touched tests passes. **Deps:** none.

**PGWIRE: Every transport carries the page; the probe; the generation guard.** [TODO] Size 3. Wave PG-A. **Owns:**
`crates/dspi-transport/src/{usb,midi,link,mock}.rs` (the paged methods and `carries_pages`), `fake.rs` (the paged
methods for the existing legacy fake: the page is ignored and the same-number opcode runs, as 1.1.6 does, counted as a
violation; new `FakeOptions.page_guard` = PAGEGUARD: STALL), `crates/dspi-wasm/src/usb.rs` (`UsbHost` gains the
`index`), `apps/web/src/core/usb.ts` (index from the call) and `apps/web/test/pages/webusb-mock.ts`,
`crates/dspi-session/src/probe.rs` (section 1.1 steps 1-4 and 7, `Capabilities.generation`), one notices block.
**Implements:** the wire calls (USB `wIndex` = `id.w_index()`; MIDI frame `index`; Link CMD `w_index`; WebUSB `index`);
the probe; **until PGADAPT lands, a paged device is refused** with the notice "this device runs firmware 1.1.7, which
this version cannot control yet" and nothing else is sent (APPS R1). **Done when:** the probe's SETUP bytes equal the
guide's on USB (mock) and WebUSB (mock); G1s and G1b fakes (guarded and not) are legacy after exactly one probe
request; a mock that answers the guide's 16 bytes is paged and receives nothing after the probe; `UnknownMajor` and
`Garbled` send nothing more; a probe timeout is retried three times then fails (never legacy); the legacy fake's
violation counter is zero across `cargo test -p dspi-session` and `-p dspi-core` connect tests; `ci/check.sh --step web`
unit tests for the WebUSB index. **Deps:** the seam.

**PGPROTO: The registry, the planner, the reverse view.** [TODO] Size 4. Wave PG-A. **Owns:**
`crates/dspi-proto/src/paged.rs` (beyond the seam) and new `src/paged/` (except `custom.rs`, PGCUST's),
`crates/dspi-proto/build.rs` (an additive section), `crates/dspi-proto/firmware/protocol/` (new, vendored),
`firmware/PROVENANCE.toml` and `SHA256SUMS` rows, `crates/dspi-proto/Cargo.toml` (`serde_json` as a build-dependency),
`NOTICE` row. **Implements:** section 1.2: generated `paged::cmds` constants and metadata; result codes and status
families; decoders for `SYS.PAGE_LIST`, `SYS.PAGE_COMMANDS` (bitmaps), `SYS.COMMAND_INFO`, `SYS.LAST_RESULT`,
`SYS.PLATFORM` (the 1.1.6 layout); `PageSet`; the planner for `direct`, `act_value`, `act_addr`, `act_packed`, `split`
and our extension row, with `custom` rows delegated to `paged::custom` (a stub returning "unsupported" until PGCUST);
`legacy_equivalent` for the same rows. **Done when:** a coverage test lists all 211 commands and 246 opcodes and fails
on any row the generator could not read; the seam's literals equal the generated ones; transcribed vectors from the
guide (3.2 before/after bytes, 4.6, 6.4 split rows, 6.6 write-as-reads, 7.1 reply lengths) pass; `proptest_*`: no
plan step on page 0, no zero-length GET, SET lengths in bounds, planner-then-reverse identity on every invertible row;
the ext row tunnels only with 0xE0 in the `PageSet`; `cargo build -p dspi-proto --target wasm32-unknown-unknown` in the
cross profile. **Deps:** the seam.

**FWPG0: The envelope in the extension spec (firmware repo, docs only).** [TODO] Size 0.5. Wave PG-A. **Owns:** in
`~/prog/DSPi-bofr`, branch `wp/FWPG0` from `bofr-main`: `Documentation/Features/extension_channel_spec.md` (new
section "Envelopes": 1.1.6 `REQ_EXTENSION` 0x00, 1.1.7 page 0xE0 tunnel `0xE010`, `wValue` = id, payloads unchanged,
GET and SET, `SYS.PAGE_LIST` lists 0xE0, MIDI and the legacy stack). **Done when:** the section states every field of
section 1.3 and the FWSEC1 flags of section 1.10, and PGPROTO's ext row and PGFAKE's G2b tunnel cite it. **Deps:** none.

### Wave PG-B: the adapter, the custom rows, the fakes, the Link client, the hub's tables

**PGCUST: The 14 custom rows.** [TODO] Size 3. Wave PG-B. **Owns:** `crates/dspi-proto/src/paged/custom.rs` (and
`custom/` for tests). **Implements:** forward and reverse for `GET_CS_DISPLAY_CFG`, `SET_EQ_PARAM`, `GET_EQ_PARAM`,
`SET_PREAMP` (retired: per-channel `ROUTE.PREAMP` after `SYS.PLATFORM`), `GET_STATUS` (every selector to its paged
struct and offset; the `DIAG` selectors are absent on a release build and refuse), `SAVE_PARAMS`,
`SET_MATRIX_ROUTE`, `GET_MATRIX_ROUTE`, `CLEAR_CLIPS`, `PRESET_GET_STARTUP`, `SET_ALL_PARAMS` (its 1.1.6 length rules),
`GET_USB_ERROR_STATS`, `RESET_USB_ERROR_STATS`, `GET_INPUT_RATE`, each from the guide's sections 6.3 and 6.5 (cited).
**Done when:** each row has a vector test transcribed from the guide; multi-step rows are step machines tested with
scripted replies including a refusal in the middle (no half-applied write is reported as success); `proptest_*` as the
shared rules. **Deps:** PGPROTO.

**PGADAPT: `PagedTransport` and `LegacyGuard`, wired into the connect.** [TODO] Size 3.5. Wave PG-B. **Owns:** new
`crates/dspi-transport/src/paged.rs`, additive lines in `crates/dspi-transport/src/lib.rs` (the `mod`, the new
`TransportError::Refused { id, result }` and its message), `crates/dspi-session/src/probe.rs` (step 2's discovery
reads and the wrapping; PGWIRE's refusal of paged devices removed), the notices block. **Implements:** section 1.2's
transport rules and section 1.1's wrapping. **Done when:** against PGFAKE's G2s and G2b fakes the whole existing
connect, readout, bulk round trip, a typed write of each registry entry, preset save/load, the gated save, the
bootloader entry (to the fake's reboot) and the extension reads (G2b) pass unchanged; absent commands are refused
locally with no request on the wire; a paged STALL reads `SYS.LAST_RESULT` once and yields `Refused`; `LegacyGuard`
refuses every paged call with no request on the wire; both violation counters zero. **Deps:** PGPROTO, PGCUST's
interface (custom rows may still be stubs at PGADAPT's start; its done criteria run after PGCUST merges, same review
batch), PGFAKE (same wave: PGADAPT's tests land in `crates/dspi-session/tests/wp_pgadapt_*.rs` and run after PGFAKE's
merge; the review batch orders PGFAKE first).

**PGFAKE: A fake for every generation.** [TODO] Size 3.5. Wave PG-B. **Owns:** new
`crates/dspi-transport/src/fake/{paged,generation}.rs`, one `mod` line and one `FakeOptions` field (`generation`) in
`fake.rs`, `fake/diag.rs` only for the tunnel's ext dispatch. **Implements:** `FakeGeneration::{Stock116, Bofr116 {
page_guard }, Stock117, Bofr117 (dual-stack), Zero, Sender}`. The paged front is **its own inverse code**, not
`dspi_proto::paged::legacy_equivalent` (so the planner and the fake cannot share a bug), driven by the registry for the
1.1.7 rules: page 0 refused (`VC_ERR_UNKNOWN`) on `Stock117`, legacy page 0 on `Bofr117`; exact SET lengths; GETs with
`wLength` 0 refused; NaN refused; `SYS.LAST_RESULT`; `PAGE_LIST`/`PAGE_COMMANDS`/`COMMAND_INFO`; the tunnel on
`Bofr117`; `Zero` with the 8-byte platform, the product string and a bootloader that never comes back as BOOTSEL;
`Sender` with its vendor subset. Violation counters: a paged request on a legacy generation, a page-0 request on
`Stock117`. **Done when:** a cross-test holds the fake's inverse equal to `legacy_equivalent` on every `direct` row;
each generation answers the guide's vectors; `cargo test -p dspi-transport`. **Deps:** PGPROTO.

**PGLINKC: The DSPi Link client, 1.2 and 1.3.** [TODO] Size 3. Wave PG-B. **Owns:** `crates/dspi-link/src/{client.rs,
messages.rs,fakehub.rs}`, `crates/dspi-transport/src/link.rs`. **Implements:** section 1.4 and section 1.1 steps 5-6.
**Done when:** against the fake hub in `L12` mode a G2s device is the "hub too old" notice after the probe and nothing
else; a G2b device is used as legacy with ext admin-only; in `L13` mode a device without `protocol` gets the notice and
no CMD; a paged device's CMDs carry `wIndex` `(page<<8)|2` and a non-zero GET length; `denied_ids` greys out on a paged
device and `denied` on a legacy one; statuses 0x10-0x1A are typed; retries only for the four codes; a multi-request poll
plan is refused at subscribe; `cargo test -p dspi-link -p dspi-transport --features link`. **Deps:** PGPROTO; the
adapter for paged devices over Link arrives with PGADAPT (its Link tests run after both merge; same batch).

**PGHPOL: The hub's tables re-keyed.** [TODO] Size 3. Wave PG-B. **Owns:** `crates/dspi-hub/src/policy.rs`,
`tests/fixtures/link/policy/` (the 1.2 table renamed `commands-1.2.json`, new `commands-1.3.json` vendored, new
`private.json`). **Implements:** section 1.6's hub rows as new functions (`classify_id`, `mutates_id`,
`in_lock_scope_id`, `core_vreg_write_id`, `denied_ids`, `judge_view` = the equivalent fed to limits and the apply
guard); the old functions stay until PGHDEV switches the router. **Done when:** every row of the 1.3 table classes as
upstream's; an id absent from both is `config`; `0xE010` SET with `EXT_CORE_VREG` triggers the voltage rule exactly as
0x00 does; lock scope by equivalent (a paged `ROUTE.MASTER_VOLUME` SET is in scope, a paged pin query is not);
`ROUTE.MASTER_VOLUME_SAVED` GET (0x0553) is `read`, never confused with `FACTORY_RESET`; `cargo test -p dspi-hub` on
the policy tests. **Deps:** PGPROTO.

### Wave PG-C: the hub's devices, the core surface, the updater, the Zero, measurement

**PGHDEV: Our hub serves both generations.** [TODO] Size 4. Wave PG-C. **Owns:** `crates/dspi-hub/src/{router.rs,
device.rs,polls.rs,registry.rs,usb.rs,local.rs,snapshot.rs}`, the hello/caps/device-object block of `server.rs`, the
analyser-sharing code it touches. **Implements:** section 1.5 after PGHUB0: probe on attach, the wrappers, the
forwarding table, translation for page-0 clients to paged devices, `protocol`/`ext`/`legacy_devices`, minor 3, both
denial lists, statuses, `LAST_RESULT`, bulk relay, polls and analyser sharing by id, the router on PGHPOL's functions.
**Done when:** the in-process hub with each PGFAKE generation passes: an L13-style paged client works on G2s and G2b
and gets `0x86` on G1s/G1b; an L12-style client works on all of them (translated on G2); our client works on all; the
violation counters stay zero; the hub's existing suites pass on G1 fakes unchanged; a G2b's tunnel is reachable for
admin only; `cargo test -p dspi-hub` scoped. **Deps:** PGHUB0, PGADAPT, PGFAKE, PGHPOL, PGLINKC.

**PGCORE: The core's surface for the generation.** [TODO] Size 2.5. Wave PG-C. **Owns:** `crates/dspi-core/src/raw.rs`
(`raw_get_id`, `raw_set_id`), `read_only.rs` (the raw id rules), `info.rs` (the generation, protocol version, page
list and ext envelope in device info), `notify.rs` (event 0x11), the forwarding of the paged calls in the production
wrappers on the raw-id path (`crates/dspi-core/src/link.rs` `BackgroundTransport`, `apps/terminal/src/core_route.rs`
`CoreTransport`, `crates/dspi-tui/src/shell/read_only.rs`, `crates/dspi-hub/src/api/transport.rs`), the bindings
(UniFFI, C, wasm) under markers, the notices block. **Done when:** the gates of section 1.6 hold (a raw `0x0130` SET
is gated; a raw GET passes in read-only mode; a raw SET does not); the device info shows each generation from the
fakes; a 0x11 notification re-reads the pins; `ci/drift.sh write` clean; the Android contract regenerated and
classified. **Deps:** PGADAPT.

**PGFWUP: Updater and rollout per generation and board.** [TODO] Size 2. Wave PG-C. **Owns:**
`crates/dspi-core/src/{firmware_guard.rs,firmware_manifest.rs,firmware.rs}`, `crates/dspi-hub/src/rollout.rs`.
**Implements:** section 1.8. **Done when:** the bootloader entry reaches each fake generation's reboot (0xF0 on legacy,
0x0130 on paged); a Zero is refused by the guard and excluded from a rollout with its reason; bofr to upstream asks
the typed confirmation; the rollout re-probes after reboot and verifies `EXT_LIST` and the 1.20 V read-back through the
new generation's envelope; a mixed group rolls out; existing P3RO tests pass. **Deps:** PGADAPT, PGFAKE.

**PGZERO: DSPi-Zero minimum.** [TODO] Size 1.25. Wave PG-C. **Owns:** `crates/dspi-proto/src/packets.rs`
(`PlatformInfo` up to 8 bytes, board id), the `DeviceDescriptor` product and `bcdDevice` fields
(`crates/dspi-transport/src/lib.rs` additive, `usb.rs`, `crates/dspi-wasm/src/usb.rs`), `crates/dspi-core/src/hardware.rs`
(the board name), the inventory row in `crates/dspi-hub/src/inventory.rs`. **Implements:** section 1.9's
identification (PGFWUP does the blocking). **Done when:** the `Zero` fake is named "DSPi Zero" in caps, About and the
inventory; a 7-byte platform reply still decodes; a Zero without the descriptor fields (a link) is still recognised by
byte 7. **Deps:** PGFAKE.

**PGMEAS: Measurement on paged firmware.** [TODO] Size 0.75. Wave PG-C. **Owns:** new
`crates/dspi-core/src/measure/tests_wp_pgmeas.rs` (one `mod` line in `measure.rs` under a marker),
`docs/core-merge/PLAN-P5.md` (one row in "Firmware-dependent items"). **Implements:** section 1.7. **Done when:** P5's
session and live tests run green on the G2s and G2b fakes; START is one transfer; the generator config refuses a wrong
length locally. **Deps:** PGADAPT, PGFAKE; P5SES and P5LIVE merged (else PGMEAS waits; it is not on PG's critical path).

### Wave PG-D: the matrix, the clients, the docs

**PGMAT: The compat matrix.** [TODO] Size 3. Wave PG-D. **Owns:** new `crates/dspi-core/tests/wp_pgmat_matrix.rs`,
new `crates/dspi-hub/tests/wp_pgmat_hub.rs`, vendored DSPi-Link fixtures under `tests/fixtures/link/upstream-1.3/`
(MIT). **Implements:** section 4's matrix, every cell of section 1.11: our core over USB (mock wire), MIDI (pipe fake),
the fake hub's `L12` and `L13` modes and OH in-process, against each fake generation; OH against an L12-style client
(our `dspi-link` client pinned to minor 2 behaviour) and an L13 client replaying DSPi-Link's 1.3 fixtures. Per cell:
connect, the expected notice or state read, one control write, an ext read where expected, a bulk round trip, the
bootloader offered or blocked, a generator START where expected. **Done when:** the matrix table is generated from the
test run (`work/wp-PGMAT/matrix.md`) and equals section 1.11; violation counters zero in every cell. **Deps:** PG-C.

**PGWEB: Web.** [TODO] Size 1. Wave PG-D. **Owns:** the device-info and raw screens in `apps/web/src/screens/`
(generation row, raw by id on paged devices), Playwright e2e with the WebUSB mock answering the probe as each
generation. **Done when:** `ci/check.sh --step web` (Chromium and Firefox, 375 px) shows each generation and each
notice; no paged request in the G1 mock's log. **Deps:** PGCORE.

**PGQT: Qt.** [TODO] Size 0.75. Wave PG-D. **Owns:** the Qt About/device-info controller and view, the raw dialog.
**Done when:** `ci/check.sh --step qt`, `--step qt-offscreen` with the fake generations. **Deps:** PGCORE.

**PGTM: Terminal and TUI.** [TODO] Size 0.75. Wave PG-D. **Owns:** `apps/terminal/src/main.rs` (`uconsole info`
generation, `uconsole raw --id 0x0550`), the TUI's About screen. **Done when:** scoped `cargo test -p uconsole` and
`-p dspi-tui` with each fake. **Deps:** PGCORE.

**PGAND: Android.** [TODO] Size 0.5. Wave PG-D. **Owns:** the device-info row in the Android settings feature,
`WriteGuardedCore` entries (if not done by PGCORE's contract step). **Done when:** the module's unit tests through the
Gradle slot wrapper. **Deps:** PGCORE.

**PGSWIFT: Swift (coordinator, Mac VM).** [TODO] Size 0.5. Wave PG-D. **Owns:** the Swift device-info row. Its
`CoreBackedTransport` needs nothing (its opcodes go through the adapter). **Done when:** `ci/macvm.sh macos` builds and
the core-backed tests pass. **Deps:** PGCORE.

**PGDOC: Docs.** [TODO] Size 1.25. Wave PG-D. **Owns:** `docs/integrator/hub-api.md` (section 3 device fields,
section 17 deviations: legacy forwarding, translation, both denial lists, minor 3), `docs/core-merge/HUB.md` (wIndex
section), `docs/PARITY.md` (rows), `docs/wire-format.md` (paged section), the user manual's notices page.
**Done when:** every new field, cap and deviation in PGHDEV is documented and the docs build step passes. **Deps:**
PGHDEV.

### HIL

**PGHIL: The real boards (coordinator and owner).** [TODO] Size 1.5. After PG-D. Not an agent WP. **Runs:** (1) the
K340 on `bofr-main` with PAGEGUARD (needs the owner's flash go): USB, USB MIDI, OH from another machine; the 15
dangerous reads sent raw through OH are refused; (2) a spare Pico 2 (never the K340, OD-PGm) on stock 1.1.7 (beta or
tag): USB in the web, Qt and Terminal, OH; a generator run; (3) a stock 1.1.6 Pico if available; (4) the alink sender;
(5) a DSPi-Zero only if the owner has one. **Done when:** section 1.11's cells for these boards match, recorded in
`work/pghil/REPORT.md`.

### Firmware WPs (`~/prog/DSPi-bofr`; start when upstream tags 1.1.7)

**FWPG1: Merge upstream 1.1.7 into `bofr-main`, build green.** [TODO] Size 3. Wave FW-A. **Owns:** the merge and its
12 conflicted paths (FIRMWARE.md section 3), `CMakeLists.txt`, stubs for every extension handler. Tag `bofr-116-final`
first. **Done when:** the K340 (`-DDSPI_USB_MIDI=ON -DDSPI_VREG_MV=1200`), pico2 and pico builds link with 0 warnings;
`bash firmware/tests/run.sh` passes (tests of not-yet-ported code disabled with a reason); `core_vreg_boot_apply()`
replaces both 1.15 V calls; RAM check passes. **Deps:** the upstream tag.

**FWPG2: Extension page 0xE0 and the FWSEC1 flags.** [TODO] Size 2.5. Wave FW-B. **Owns:** new `vendor/pages/ext.def`,
`vendor/vc_page_ext.c`, the `sys.def` flag line, removal of `transport_gate.h`'s opcode table, `firmware/tests/test_ext_page.c`.
**Done when:** host tests: every ext id answers through `0xE010` byte-identically to the 1.1.6 golden replies;
`SYS.PAGE_LIST` lists 0xE0; `SYS.ENTER_BOOTLOADER` and an `EXT_CORE_VREG` tunnel SET above 1.20 V are refused off the
USB vendor path, MIDI included. **Deps:** FWPG1, FWPG0.

**FWPG4: USB MIDI and the sender on the paged dispatcher.** [TODO] Size 3. Wave FW-B. **Owns:**
`firmware/DSPi/midi_control.c`, `firmware/tests/test_midi_control.c`, `firmware/DSPi-Sender/*`, `tools/midi_ctrl/*`.
**Done when:** host tests: a paged request over MIDI reaches its handler with its `wIndex`; chunk sessions on 0x0241;
`CTRL_SOURCE_MIDI` in `SYS.LAST_RESULT`; the sender answers `SYS.PROTOCOL_INFO`, `SYS.PLATFORM`, its vendor subset on
the paged stack and the tunnel, and keeps page 0. **Deps:** FWPG1.

**FWPG5: Pins, servo, size, directory.** [TODO] Size 3. Wave FW-B. **Owns:** our claims in `vendor/vc_pins.c`,
`input_servo.c` (upstream's seed and our alink smoothing), the linker placement lists, `scripts/ram_baseline.json`,
`flash_storage.c` (the directory renumbering and migration, OD-PGl), the IDLE1 re-check. **Done when:** `SYS.PIN_CLAIMS`
reports the alink, scope, identify and GP23 pins claimed (host test); a V26 and a V27 image migrate once to the private
version and an upstream-layout directory is not misread (host tests); RAM check PASS with `.data` within budget.
**Deps:** FWPG1.

**FWPG3: The legacy front (dual-stack).** [TODO] Size 4. Wave FW-C. **Owns:** new `vendor/vc_legacy.c`,
`vendor/vc_legacy_custom.c`, a generator mode in `scripts/gen_vendor_protocol.py` emitting the `const` table from
`legacy_map.json`, `firmware/tests/test_legacy_front.c`, upstream's page-0 test adapted. **Done when:** replaying
upstream's golden 1.1.6 captures (commit `7571871`) through the legacy front gives the same replies, except rows the
map marks as changed; `REQ_EXTENSION` 0x00 reaches FWPG2's handlers; 0xF0 is gated as in FWPG2; the tables are in flash
(the `.data` delta is 0). **Deps:** FWPG2, FWPG4 (the MIDI chunk path).

**FWPG6: Tools, tests, docs.** [TODO] Size 2. Wave FW-C. **Owns:** `tools/dspi_poke`, `tools/dspi_test` (take
upstream's, add `--page 0xE0`), `tools/alink/sender_usb.py`, `Documentation/` sections we re-add after taking
upstream's (commands.md, control_interfaces_spec.md, notification spec), `extension_channel_spec.md`'s status line.
**Done when:** the docs describe the dual stack and the private page; the tools run against the host test harness.
**Deps:** FWPG2-FWPG5.

**FWPGHIL: The port on the K340 (coordinator and owner).** [TODO] Size 1.5. After FW-C and PGMAT. Backup the
configuration, flash with the owner's go, check: 1.20 V read back, directory migrated, alink USB-direct, our clients
over USB, MIDI and OH as G2b, an old app build and `dspi_poke` on the legacy stack, the sender, 30 minutes without a
hang. Then `bofr-main` is pushed to the Bofr5555 fork (memory `push-forks`), never to Weeb Labs.

### Waves

No two WPs in one wave own the same file. Where a WP's tests need another WP of its wave (PGADAPT on PGFAKE and PGCUST,
PGLINKC on PGADAPT), the entry says so and the review batch merges in that order; the code itself depends only on
earlier waves and the seam. `notices.json` takes one block per WP; bindings and `mod` lines are additive under markers.
PGHUB0 is the first WP to run and merge.

| Wave | WPs | Size | Needs |
|---|---|---|---|
| PG-A | **PGHUB0 0.75** (first), PGWIRE 3, PGPROTO 4, FWPG0 0.5 | 8.25 | `main` with this plan's seam |
| PG-B | PGCUST 3, PGADAPT 3.5, PGFAKE 3.5, PGLINKC 3, PGHPOL 3 | 16 | PG-A |
| PG-C | PGHDEV 4, PGCORE 2.5, PGFWUP 2, PGZERO 1.25, PGMEAS 0.75 | 10.5 | PG-B; the full gate after PG-C |
| PG-D | PGMAT 3, PGWEB 1, PGQT 0.75, PGTM 0.75, PGAND 0.5, PGDOC 1.25; PGSWIFT 0.5 (coordinator) | 7.75 | PG-C |
| — | **PGHIL** (coordinator and owner) | (1.5) | PG-D |
| FW-A | FWPG1 3 | 3 | upstream tags 1.1.7 |
| FW-B | FWPG2 2.5, FWPG4 3, FWPG5 3 | 8.5 | FW-A |
| FW-C | FWPG3 4, FWPG6 2 | 6 | FW-B |
| — | **FWPGHIL** (coordinator and owner) | (1.5) | FW-C and PGMAT |
| | **App: 19 agent WPs + PGSWIFT** | **42** (+1.5 HIL) | core, transports, Link client and updater 24.5; hub 7.75; clients 3.5; matrix and docs 4.25; Zero 1.25; measurement 0.75 |
| | **Firmware: 7 WPs** (FWPG0 in PG-A) | **18** (+1.5 HIL) | |

The app waves need no firmware: G2b is a fake built to FWPG0's spec until FWPGHIL. Each wave ends with one Fable batch
review; the full gate runs after PG-C and after PG-D. If fewer agents run, PGPROTO and PGFAKE are the long poles.

---

## 3. The security bar and the compatibility policy

- **The probe is the safety boundary** (section 1.1), enforced twice for each direction, and every fake counts
  violations. A failed or ambiguous probe sends nothing more.
- **Our hub never runs a request whose page it cannot honour:** page != 0 to a legacy device is `0x86` (PGHUB0); a
  page-0 request to a paged device is translated by the same planner our clients use, judged by the 1.2 table.
- **Every judge sees the 1.1.6 equivalent** (section 1.6), so the lock, the limits profile, the apply guard and the
  core-voltage rule cannot be bypassed by sending the paged form; a paged write with no equivalent is judged by class
  and is outside the lock scope.
- **The bootloader and the core voltage stay USB-vendor only on our firmware** (FWSEC1 on both stacks, MIDI counted as
  off-USB). The K340 never runs stock firmware (PGHIL uses a spare board).
- **Untrusted data:** probe replies, discovery bitmaps, `LAST_RESULT`, hub device objects and policy lists are parsed
  without panics, bounded, and fuzzed with the planner's `proptest_*`.
- **PLAN-SEC unchanged:** TLS, pinning, roles; the new fields are additive.

**The compatibility policy (owner, 2026-11-15) and the dual-protocol requirement (2026-11-18):**
- **No shims between our components:** our client always speaks a device's native generation; the hub's translation
  exists only for legacy clients and its own internal requests.
- **Our clients against an upstream hub keep working:** L12 hubs as today; L13 hubs with paged devices through the
  adapter; with legacy devices on an L13 hub, the spec forbids forwarding, so the client says so (section 1.1 step 5).
- **USB to the hardware keeps working:** all four generations (section 1.11).
- **Our hub against upstream clients, best effort, here better than required:** L12 clients reach both generations;
  L13 clients reach paged devices.
- **hub-api v1 stays frozen:** `protocol`, `ext`, `legacy_devices`, `denied_ids` and minor 3 are additive; our hub
  keeps filling `policy.denied` for 1.2 clients.

---

## 4. Testable without hardware: the fakes and the matrix

| Generation / hub | Fake | Owner |
|---|---|---|
| G1s | `FakeDevice` with `extensions: false` (exists); paged calls run the same-number opcode, counted | PGWIRE |
| G1b without guard | `extensions: true` (exists); as above | PGWIRE |
| G1b with PAGEGUARD | `page_guard: true`: STALL / status 0x02 for any page | PGWIRE |
| G2s | `FakeGeneration::Stock117`: paged, strict 1.1.7 rules, page 0 refused, counted | PGFAKE |
| G2b | `FakeGeneration::Bofr117`: paged + tunnel + legacy page 0 | PGFAKE |
| Zero | `FakeGeneration::Zero`: 1.1.6, 8-byte platform, product string, foreign bootloader | PGFAKE |
| Sender | `FakeGeneration::Sender`: vendor subset, guarded (and dual-stack after FWPG4) | PGFAKE |
| L12 hub | `fakehub` `L12` mode: policy by `bRequest`, drops `wIndex` | PGLINKC |
| L13 hub | `fakehub` `L13` mode: upstream 1.3 rules | PGLINKC |
| OH | the real hub in-process | PGHDEV, PGMAT |
| L12 client | our `dspi-link` client in minor-2 behaviour | PGMAT |
| L13 client | DSPi-Link 1.3 fixtures replayed | PGMAT |

Independence of the oracles: the planner (PGPROTO, PGCUST) and the paged fake (PGFAKE) are separate code from the same
upstream data; the guide's byte examples are transcribed by hand as vectors; DSPi-Link's fixtures come from an
independent implementation; firmware host tests replay upstream's golden 1.1.6 captures (FWPG3). What no fake proves:
that the real 1.1.7 firmware answers as its documents say, that a real L12/L13 hub behaves as modelled, and that the
port runs on the K340: PGHIL and FWPGHIL.

---

## 5. Decisions

Every decision carries a firm recommendation; following the owner's rule of 2026-11-15 they are taken as recommended
unless the owner objects.

| # | Decision | Options | Recommendation | Why |
|---|---|---|---|---|
| **OD-PGa** | **Where the paged protocol enters** | (1) an adapter transport under the existing 1.1.6-shaped calls, generated from `legacy_map.json`; (2) rewrite every call site to paged ids with a legacy adapter underneath; (3) two parallel call layers | **(1)** | About 480 call sites, the fake, the hub's internal requests and Swift's raw tunnel keep working unchanged; `legacy_map.json` exists for exactly this; one place decides. (2) is weeks of churn for the same behaviour. New paged-only features get typed paged calls as they come |
| **OD-PGb** | **The port's shape** | (1) dual-stack: paged + page-0 legacy front; (2) paged only, as upstream | **(1)** | Keeps old app builds (the Android closed test), the ESP32 panel over UART, `dspi_poke`, L12 hubs and the sender's old hosts working on our boards. The safety argument for page-0 refusal does not apply: on our firmware the page byte is honoured, so a page-0 request means exactly its 1.1.6 meaning. Built as a generated front onto the paged handlers, not a second copy of 1.1.6's handlers, and its tables in flash |
| **OD-PGc** | **The extension envelope on paged firmware** | (1) page 0xE0, tunnel `0xE010`, `wValue` = ext id, payloads unchanged; (2) one paged command per extension on 0xE0; (3) keep 0x00 on page 0 only | **(1)** | Ids, payloads and `EXT_LIST` stay byte-identical; one planner row; upstream reserves 0xE0-0xEF for forks; (2) multiplies our `.def` rows for no gain; (3) is unreachable on stock dispatchers and invisible to paged discovery |
| **OD-PGd** | **Which envelope our client uses on G2b** | (1) paged (incl. the tunnel); (2) legacy, since G2b has it | **(1)** | One code path per generation; the same client works on G2s; the legacy stack is for legacy hosts |
| **OD-PGe** | **Paged devices through an L12 hub** | (1) legacy only, probe only for the notice; (2) paged when the probe answers | **(1)** | A 1.2 hub classifies by `bRequest` (wrong authorisation) and may drop the page (ours did); G2b still works through its legacy stack |
| **OD-PGf** | **Feature detection on paged firmware** | (1) `PAGE_COMMANDS` bitmaps, local refusal shaped as a STALL to the layers above; (2) probe by STALL on the wire | **(1)** | Upstream says not to probe by STALL; the bitmaps are read once; the layers above stay unchanged |
| **OD-PGg** | **Our hub and legacy devices** | (1) list and forward (deviation from spec 1.3 7.4) with `protocol` absent and the cap `legacy_devices`; (2) follow 1.3 and forward nothing | **(1)** | The owner's requirement: our clients must reach `bofr-main` boards through our hub. Upstream 1.3 clients still send nothing to them, so it is safe for them |
| **OD-PGh** | **Page-0 client requests to a paged device on our hub** | (1) translate with the planner, judged by the 1.2 table; (2) answer `0x10` as upstream | **(1)** | The planner is already in the hub for its own requests; L12 clients (upstream 1.1.6 consoles, our older apps, the vendored 1.2 hub page) keep working with 1.1.7 boards; no re-vendoring of the hub page |
| **OD-PGi** | **How the hub's judges see paged requests** | (1) through the 1.1.6 equivalent (`legacy_equivalent`); (2) a second set of paged judges | **(1)** | One set of rules; the lock, limits and voltage rules cannot be bypassed by the paged form; unmappable writes are judged conservatively |
| **OD-PGj** | **Vendoring** | (1) the JSON files in `dspi-proto/firmware/protocol/` with provenance, generated by `build.rs` (`serde_json` build-dependency); (2) a checked-in generated file | **(1)** | Same rule as the headers: a firmware bump is a file copy; the coverage test shows what changed |
| **OD-PGk** | **`HostTransport` (UniFFI)** | (1) stays legacy-only (`carries_pages` false), retires at SW-M3; (2) add paged methods to the foreign trait | **(1)** | It serves the Swift hub's session 1 and Swift's demo, both retiring; a paged device behind it fails closed with the "unrecognised" notice |
| **OD-PGl** | **Our flash directory versions at the port** | (1) renumber to a private band (`0x80 | n`) with a one-time migration; (2) keep V26/V27 and hope upstream's next differs in size | **(1)** | Upstream's next directory is also "V26" (`ac3-dts-decode` already has one); the CRC length check would be luck, not design |
| **OD-PGm** | **Which board runs stock 1.1.7 in HIL** | (1) a spare Pico 2; (2) the K340 | **(1)** | Stock firmware runs the K340 at 1.15 V (hangs) and loses its V27 directory configuration |
| **OD-PGn** | **When the firmware port starts** | (1) at upstream's 1.1.7 tag or a quiet week; app waves now; (2) now on the beta | **(1)** | The beta's tip moved today (pin queries); the app side needs no firmware (fakes to FWPG0's spec) |
| **OD-PGo** | **Polls whose legacy form needs several paged reads** | (1) refused at subscribe with a typed error; (2) hub-side composition | **(1)** | Every poll the clients use today maps to one paged GET; composition would be a new hub feature for nothing |

---

## 6. What not to do

- Send anything paged (other than the probe) to a device that has not answered the probe with the magic and major 1;
  send a page-0 request from our client to a paged device; guess a generation after a probe timeout.
- Forward a request with a non-zero page to a legacy device, from our hub or anywhere else.
- Cache a generation across connections, by serial, or across a firmware update.
- Hand-transcribe a paged id, length or adapter row; let the paged fake reuse the planner's reverse code.
- Re-vendor the DSPi-Link hub page at 1.3; drop `policy.denied` for 1.2 clients.
- Flash stock firmware on a K340; enter a DSPi-Zero's bootloader; flash a port image without the owner's go.
- Keep 1.1.6's `vendor_commands.c` as the legacy stack; put the legacy front's tables in RAM.
- Open a pull request to Weeb Labs, or ask them to reserve anything (0xE0-0xEF is documented for forks).
- Push anything from a WP.

---

## 7. Owner questions and actions

**No blocking question.** Every decision in section 5 has a firm recommendation.

**Owner actions:**
1. **A go to flash PAGEGUARD** (`wp/PAGEGUARD` `df53ede`, reviewed) on the K340 for PGHIL (not covered by the standing
   go, which is for alink-usb test images).
2. **A spare Pico 2** (and a Pico, if available) for stock 1.1.7 and stock 1.1.6 in PGHIL; a DSPi-Zero only if you
   already have one.
3. **When upstream tags 1.1.7:** the start of FWPG1; later a go for the port image on the K340 in FWPGHIL.

---

## 8. Later (not in this plan's waves)

| Item | Size | Why later |
|---|---|---|
| PGREV: re-vendor the registry and legacy map at upstream's 1.1.7 tag (and each later bump) | 0.5 | the beta moves; the coverage test makes it a file copy |
| Pin availability queries (`SYS.PIN_STATE`, `SYS.PIN_CLAIMS`, `*_PIN_AVAIL`, event 0x11 driving the pin pickers) | 1.5 | paged-only feature; pickers work as today meanwhile |
| Explicit per-command refusals shown to the user from `SYS.LAST_RESULT` (beyond typed errors) | 0.5 | polish |
| DSPi-Zero's own platform id or pin list, when upstream decides | about 2 | APPS 3.1: do not guess now |
| Route choice for one board on several paths, mDNS browse restarts (APPS R7, R8) | 2.5 | independent of the protocol; own plan |
| Migrating our own clients' legacy-only code paths (e.g. `-CoreTransport NO`) | — | retire instead (owner decision P2) |
