# PLAN-PG: both protocol generations in every component

*WP PLANPG, 2026-11-18, written on `main` at `c06c140f`. The inputs are the two upstream cross-checks
(`work/upstream-xcheck/FIRMWARE.md`, `work/upstream-xcheck/APPS.md`), upstream firmware `origin/release/v1.1.7` at
`103ba37` (1.1.7 beta 1, untagged: `firmware/DSPi/vendor/`, `Documentation/protocol/dspi_commands.json`,
`legacy_map.json`, `Documentation/Features/vendor_protocol_migration_guide.md`), upstream DSPi-Link at `4aec5ab`
(spec 1.3, `policy/commands.json`, JS library 1.3.0), DSPi-Zero at `192df73`, our firmware `bofr-main` `8ba8b72` plus
PAGEGUARD (`wp/PAGEGUARD` `df53ede`, finished today), and this repository's code, cited `file:line`. Plan plus one
compiling seam (section 2, "The seam"). No other code.*

*Revised after the Fable plan review (`work/review-planpg/REPORT.md`, ACCEPT WITH CHANGES): every required change R1 to
R10 is applied (cited where it lands, "R6a" and so on), and the optional suggestions S1 to S7 are adopted (S1 no probe
through an L12 hub, S2 the `LAST_RESULT` match, S3 the legacy front's switch, S4 the early `.data` reading, S5 the
manual's K340 warning, S6 the board name from `hardware.rs`, S7 the wording). The waves are re-sized (section 2).*

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
expected outcome, tested without hardware; a smoke run on a real 1.1.7 board comes early, one HIL WP checks the real
boards at the end.

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
| Where a connection is built | the raw `Box<dyn Transport>` is made at 10 sites (R2): `crates/dspi-core/src/{device.rs:139,328,421, usb_pipe.rs:254, link.rs:1376, host_transport.rs:155}`, `crates/dspi-hub/src/usb.rs:76`, `crates/dspi-wasm/src/usb.rs:341`, `crates/dspi-tui/src/shell/connect.rs:909,910,1574`, `apps/terminal/src/doctor.rs:156`; the probe functions take `&mut dyn Transport` (`crates/dspi-session/src/probe.rs:218-540`) and cannot wrap | **one wrapping point**, `dspi_transport::paged::open` (PGWIRE, section 1.2) |
| Probe | `probe` reads `REQ_GET_PLATFORM` first (`probe.rs:566`); nothing knows `SYS.PROTOCOL_INFO`; the Link re-attach shortcut `ProbeKey`/`Reusable` (`probe.rs:252-300`) is keyed without a generation | PGWIRE |
| Feature detection | "probe and cache": a STALL means absent (`probe.rs:1-10`, `:630-667`) | kept: the adapter refuses absent paged commands locally (section 1.2) |
| Extension channel I/O | one place, `dspi_session::ext` (`crates/dspi-session/src/ext.rs`), codecs in `dspi_proto::ext` | unchanged above the adapter (OD-PGc) |
| Our hub and `wIndex` | **dropped**: `execute` forwards `bRequest`/`wValue` only (`crates/dspi-hub/src/device.rs:571-596`), and the test `wp_hb0_windex_is_ignored` (`device.rs:871`) asserts it. A paged request from a 1.3 client therefore reaches a legacy device as the old opcode with the same number (e.g. a `ROUTE.MASTER_VOLUME_SAVED` GET 0x0553 runs `FACTORY_RESET` 0x53). PAGEGUARD cannot see it (the page is gone before the device) | **PGHUB0, first** |
| The Swift hub and `wIndex` | **forwarded verbatim** to USB (`apps/apple/DSPi Console/Link/USBHubDevice.swift:41-54`) while `LinkPolicy.bundled` classes by `bRequest`; runs under `-CoreHub NO` / `-CoreTransport NO` (`DSPi_ConsoleApp.swift:39`, `:243-283`). An unguarded G1 device behind it runs the same-number opcode of a paged request | **PGSWHUB0, first** (coordinator, R1) |
| Hub policy | one table keyed by the 8-bit opcode (`crates/dspi-hub/src/policy.rs:28`, `:68-145`), lock scope (`:210`), core-voltage rule (`:230`), busy reads (`:79`); `tests/fixtures/link/policy/commands.json` is the 1.2 table; polls are shared without `idx` in the key (`polls.rs:10`) | PGHPOL, PGHDEV |
| Link minor | `PROTO_MINOR` 2 (`crates/dspi-link/src/messages.rs:15`) is sent by **both** our client and our hub (`crates/dspi-hub/src/server.rs:61`, `:3873`, `:3903`) | split in PGLINKC; the hub's raised in PGHDEV (R4) |
| Link client | `DeviceInfo` has no `protocol` (`messages.rs:202`); `policy.denied` parsed as 8-bit codes (`client.rs:973-978`); batched reads (`control_in_batch`, LK2/LK4) make a Link connect one or two round trips | PGLINKC |
| Fake device | 1.1.6-shaped, `FakeOptions.extensions` is G1b vs G1s (`crates/dspi-transport/src/fake.rs:217-268`); about 800 opcode references across `fake/` | PGFAKE1, PGFAKE2 add the paged generations |
| Raw paths | `raw_get`/`raw_set` take an 8-bit opcode (`crates/dspi-core/src/raw.rs:69`, `:125`), exported over UniFFI and C; Swift's `CoreBackedTransport.swift` tunnels its own opcodes through them; Terminal `uconsole raw`, the TUI's `ReadOnlyTransport` (`crates/dspi-tui/src/shell/read_only.rs:342`) | keep (through the adapter); add id forms (PGCORE) |
| Host transport | `HostTransport` (`crates/dspi-core/src/host_transport.rs:86-88`), opcode only. SW-M3 is merged (PLAN-P3.md:63) but `HostTransport` and the Swift opt-out paths still ship | stays legacy-only; the opt-outs retire per owner decision P2 (OD-PGk) |
| Measurement | siggen and analyser through typed core calls on opcodes 0xA4-0xA8 and the RTA set (`crates/dspi-core/src/stats.rs:255-397`, `rta.rs`); PLAN-P5's WPs build on them | unchanged above the adapter (PGMEAS) |
| Updater, rollout | bootloader entry 0xF0 (write-as-read) through the gated call (`crates/dspi-core/src/hardware.rs:217`), the guard (`firmware_guard.rs`), manifest (`firmware_manifest.rs`), PICOBOOT (`picoboot.rs`), hub rollout (`crates/dspi-hub/src/rollout.rs`, `rollout/steps.rs:905`), a client-tunnelled 0xF0 through the router (`router.rs:183`) | PGFWUP, PGHDEV |
| Firmware | `bofr-main` speaks 1.1.6 + extension 0x00; PAGEGUARD refuses every non-zero page on USB EP0, both dispatchers, UART, I2C, MIDI and the sender (its report, table "What is guarded"); 0xE0 is refused too; K340 `.data` is 99 512 of 100 352 bytes | the port (FWPG1-6) |
| Registry | firmware `103ba37` `dspi_commands.json`: **224** commands (13 pin commands added 2026-11-18); DSPi-Link's copy (firmware `4eea7df`) and its 1.3 policy table: 211; `legacy_map.json`: 246 opcodes, 248 adapters by direction (192 `direct`, 27 `act_*`, 15 `split`, 14 `custom`), byte-identical in both repos | PGPROTO pins one firmware commit (R8) |

**Corrections to the cross-checks found while checking:**
- APPS.md (C4, R5) says upstream reserves no page for third parties. **It does:** pages 0xE0-0xEF are "private use for
  forks and ports, never allocated upstream" (FIRMWARE.md section 0; upstream `vendor_command_overhaul_plan.md:132`).
  The extension page is 0xE0 (OD-PGc).
- APPS.md R2 sizes the core adapter at 11 units and R1-R4 at 17. That estimate covers USB and the Link client only. The
  owner's requirement adds: the hub serving both generations to both Link versions with re-keyed judges, fakes for
  every generation, the hazard guards in both our hubs, the extension envelope, the updater per generation and board,
  every client, and a compat matrix. **This plan is 49 units of app WPs plus 2 of HIL, and 19.25 of firmware plus 1.5
  of HIL** (section 2, "Waves").
- FIRMWARE.md estimates the port at "about 5 WPs". Dual-stack (OD-PGb) adds one (FWPG3, the legacy front), the sender
  and the envelope spec add a little: 7 firmware WPs and one firmware HIL.
- The hub hazard is live today, not only after a 1.3 client exists, and in **two** hubs (ours and the Swift opt-out
  hub): any client that sets a `wIndex` high byte reaches our legacy devices with the wrong command. PAGEGUARD's report
  raised the first; the plan review found the second. PGHUB0 and PGSWHUB0 close them.

---

## 1. The design

### 1.1 Generation detection (the probe)

The generation is decided once per connection, **in `dspi_transport::paged::open`, on the raw wire, before any other
request** (section 1.2). The probe is safe on every generation: 1.1.6 has no IN handler for `bRequest` 0x10 (on
`bofr-main` 0x10 is `SET_SUBHARM`, OUT only; the sender has none either, `sender_vendor.c:351-380`) and STALLs;
PAGEGUARD STALLs it too (status 0x02 on UART, I2C and MIDI); 1.1.7 answers 16 bytes starting `DSPi` (migration guide
4.2-4.4).

1. **USB, USB MIDI, WebUSB** (the link carries pages, `Transport::carries_pages`): send `SYS.PROTOCOL_INFO` (GET
   `0x0110`, 16 bytes). A timeout or busy answer is retried three times about 150 ms apart (a flash write may blind the
   device); after that the open fails with a transfer error. **A failure never falls back to a guess.**
2. **The reply decodes as paged** (`ProtocolInfo::decode` = `ProbeReply::Paged`, seam): the device is **paged**. Read
   `SYS.PLATFORM` (8 bytes asked, section 1.9), `SYS.PAGE_LIST` and `SYS.PAGE_COMMANDS` for every listed page, once,
   batched; keep them for the connection. If page 0xE0 is listed, read `EXT_LIST` through the tunnel `0xE010` (ext id
   0): the device has our extensions (G2b), else none (G2s). `open` returns a `PagedTransport`.
3. **`UnknownMajor`** (a later protocol major): refuse with the notice "this device's firmware uses a newer protocol;
   update this app". **`Garbled`** (an answer that is not a protocol-info block, on a direct link): refuse with
   "unrecognised device". Send nothing else in either case: something that answers a page-1 request directly is not
   1.1.6 firmware.
4. **The probe STALLs** (USB) or answers status 0x02 (MIDI): the device is a **legacy candidate**. Read
   `REQ_GET_PLATFORM` (8 bytes asked) as today. If it answers, the device is **legacy** and `open` returns a
   `LegacyGuard`; the extension probe follows as today (`EXT_LIST` on 0x00): G1b or G1s. If it STALLs as well: refuse
   with "unrecognised device".
5. **Over DSPi Link the hub's device object decides**, and the client sends no probe (S1): a device with
   `protocol.major == 1` is paged (discovery as step 2, through the hub); one without `protocol` is legacy if the hub
   forwards to legacy devices (an L12 hub, or OH with the cap `legacy_devices`), and otherwise is shown with the notice
   "this hub serves only firmware 1.1.7 and later" and sent nothing (an L13 hub answers `0x86` to every CMD for it).
6. **Through an L12 hub the client speaks legacy only, whatever the device is, and sends no probe** (OD-PGe, S1): it
   reads `REQ_GET_PLATFORM`; if that is refused (STALL, `DENIED`, 0x02, a timeout after retries) the notice is "this
   device does not answer the 1.1.6 protocol; firmware 1.1.7 needs a DSPi Link 1.3 hub", never "unrecognised device".
   Real upstream Console hubs forward `wIndex` verbatim but class by `bRequest` (guide 13.3.3), our hub before PGHUB0
   drops it: neither can be trusted with a page. A G2b device answers `REQ_GET_PLATFORM` on its legacy stack and is
   used as a legacy device, extensions included.
7. **The generation belongs to the live connection** (`Generation`, seam), lives in the type of the wrapper
   (`PagedTransport` or `LegacyGuard`, readable through `Transport::generation()`), and is re-decided on every
   reconnect, after every firmware update and after every bootloader return. **It is never cached across connections,
   by serial, or in a registry record.** The one cache that exists, `ProbeKey`/`Reusable` (the Link re-attach
   shortcut), gains the generation, the protocol version and the page list, and a `Reusable` from one generation is
   never applied to the other (PGWIRE, R6a).
8. **A generation can change under a live connection** (a re-flash behind a hub or behind a host-supplied
   `MidiPipe`/`-MIDIRelay` that survives the device's reboot). Over DSPi Link a `device.changed`/`device.added` with a
   different `protocol` rebuilds the wrapper, and `0x86` or `0x10` on a request to a device that worked a moment ago
   is "generation changed, re-attach", never a retry (PGLINKC, R6c). Directly, the device's own guard is the backstop:
   that is why every legacy bofr image we offer contains PAGEGUARD (section 1.8, R6d).

**The two hard rules, each enforced in two places:** never a paged request (other than the probe) to a legacy device
(`LegacyGuard` refuses every paged call locally; both our hubs refuse page != 0 toward a legacy device, PGHUB0 and
PGSWHUB0); never a page-0 request from this client to a paged device (`PagedTransport` sends page 0 never: every
legacy call becomes a plan of paged steps or a local refusal). **The fakes fail the test by default** on a violation
(section 4).

### 1.2 One typed command layer: one wrapping point, the adapter under the 1.1.6-shaped calls

```
 core / session / hub internals      control_in(opcode, wValue, len), control_out(...), control_in_batch(...)   (unchanged, ~480 call sites)
            |
 every other wrapper: BackgroundTransport, Held, Guarded, PollTransport, PrincipalTransport, ReadOnlyTransport, ...
            |                                         |
   LegacyGuard<T>  (legacy devices)          PagedTransport<T>  (paged devices)
   passes legacy calls,                       legacy call -> dspi_proto::paged::plan(...) -> paged steps on T,
   refuses every *_paged call                 reply re-assembled into the 1.1.6 shape; *_paged calls pass through
            \___________ dspi_transport::paged::open(raw) decides (section 1.1) ___________/
            |
 raw wire T: UsbTransport, MidiTransport, LinkTransport, WebUsbTransport, FakeDevice, MockTransport, HostTransport's adapter
            (control_in/out = page 0;  control_in_paged / control_out_paged / control_in_batch_paged = page from the CmdId)
```

- **The layering rule:** raw wire, then the generation wrapper made by `paged::open`, then every other wrapper.
  `open(raw) -> Result<Box<dyn Transport>>` is called at each of the 10 construction sites (table above); the existing
  probe functions then run unchanged on the wrapped transport (on a paged device `REQ_GET_PLATFORM` goes through the
  planner). This guarantees "probe first" also for callers that never call `probe` (`ExtState::probe` in
  `crates/dspi-tui/src/settings/overview.rs:38`, `crates/dspi-hub/src/api/exec.rs:129`). Wrappers above the generation
  wrapper need the paged calls only for the raw-id path (PGCORE). PGWIRE writes `open` and edits the 10 sites in PG-A
  (paged devices refused with a notice until PG-B2); PGADAPT changes only its inside.
- **The planner is pure and generated** (`dspi_proto::paged`, PGPROTO). `build.rs` reads the vendored
  `dspi_commands.json` (the registry: id, directions, address kind, GET length, SET bounds, flags, build condition,
  page revision) and `legacy_map.json` (246 opcodes, 248 adapters by direction) and emits typed constants
  (`paged::cmds::ROUTE_MASTER_VOLUME` and so on) and the row tables. `plan(opcode, dir, value, payload_or_len,
  &PageSet)` returns a small step machine: one or more paged requests and how to build the 1.1.6 reply from their
  answers (the status byte of an `act_*` row from its status family, `GET_STATUS` fields from their structs and
  offsets). The 14 `custom` rows are written by hand from the migration guide sections 6.3 and 6.5 (PGCUST). **Our
  extension row** is part of the planner: opcode 0x00 becomes the tunnel `0xE010` with the same `wValue` and payload
  when page 0xE0 is in the `PageSet`, else a local STALL (so `dspi_session::ext` sees "no extension channel", exactly
  as on G1s).
- **The reverse view** (`paged::legacy_equivalent(id, dir, value, payload)`), used by the hub's judges (section 1.6):
  a paged request as the 1.1.6 request with the same effect, `None` when it has none (new commands such as
  `SYS.PIN_STATE`). Written with the planner, property-tested against it.
- **`PagedTransport<T>`** (PGADAPT, `crates/dspi-transport/src/paged.rs`): runs plans on `T`'s paged calls; refuses
  locally, as `Stalled { opcode }`, every legacy call whose target is not in the `SYS.PAGE_COMMANDS` bitmaps, so the
  probe-and-cache feature detection above it keeps working without a STALL on the wire (upstream: do not probe by
  STALL, guide 4.7); after a real STALL on USB or MIDI reads `SYS.LAST_RESULT` once and trusts it only when its id,
  direction and `wValue` match the failed request (S2; the record is per transport, not per host, guide 5.2), giving
  a typed `TransportError::Refused { id, result }` (new variant), else a plain `Stalled`; retries only what upstream
  allows (0x01, 0x04, 0x17, 0x84 over Link; `VC_ERR_BUSY` 0x17 on USB through the existing busy window); never sends a
  GET with `wLength` 0; **pre-checks SET lengths only when the page's revision from `SYS.PAGE_LIST` equals the
  vendored one** (a higher revision may grow a payload at the end, guide 4.7: then it sends and lets the device
  judge, R8b); refuses NaN and infinity in float payloads locally where the registry types the field.
- **Batched reads** (R3): `Transport::control_in_batch_paged(&[PagedRead])` (default: sequential; `LinkTransport`
  keeps several on the wire, PGLINKC). `PagedTransport::control_in_batch` maps a batch of single-step plans to one
  paged batch, so a Link connect to a paged device costs no more round trips than the legacy connect plus one
  discovery batch.
- **Paged-only commands** that have no 1.1.6 form (discovery, `SYS.LAST_RESULT`, the pin queries) are called through
  typed functions over `control_in_paged`; this plan adds only discovery and `LAST_RESULT`. Pin queries are section 8.
- **Bulk:** the blob is unchanged (wire format 32, 6136 bytes). `0xA0`/`0xA1` map to `STORE.ALL_PARAMS` 0x0240 and
  `0xA2`/`0xA3` to `STORE.ALL_PARAMS_CHUNK` 0x0241 (`SET_ALL_PARAMS` is a custom row for its 1.1.6 length quirks).
- **Flash save:** 1.1.7 has no `SAVE_PARAMS`; the custom row is `STORE.PRESET_ACTIVE` then `STORE.PRESET_SAVE` to that
  slot (`legacy_map.json` `save_params`). The gated core call is unchanged.
- **Notifications:** packet format unchanged; the core learns `NOTIFY_EVT_PIN_MAP_CHANGED` 0x11 (re-read pins,
  PGCORE). Our 0xE0-0xE3 events stay.

### 1.3 Our extensions over both envelopes (OD-PGc)

| Generation | Envelope | Who uses it |
|---|---|---|
| G1b | `REQ_EXTENSION` `bRequest` 0x00, `wIndex` 2, `wValue` = ext id | every host, as today |
| G2b, paged stack | page 0xE0, `bRequest` 0x10 (id `0xE010`), `wIndex` 0xE002, `wValue` = ext id; GET and SET; payloads, `EXT_LIST`, ids 0x0000-0x0006 and 0x8000-0x8012 byte-identical; refusals as `VC_ERR_*` (0x10-0x1A), never 0x02/0x03 (an upstream 1.3 bridge drops a device that answers those, DSPi-Link `4aec5ab`) | our clients (through the planner's ext row) |
| G2b, legacy stack | `REQ_EXTENSION` 0x00 on page 0, served by the same handlers | legacy hosts only (older apps, the ESP32 panel over UART, an L12 hub) |
| G1s, G2s | none: the probe finds no channel | |

Ext notifications 0xE0-0xE3 and `EXT_NOTIFY_FEATURES` are unchanged. `SYS.PAGE_LIST` on G2b lists page 0xE0 (FWPG2),
so a paged host can see it. Over an L13 upstream hub page 0xE0 is "absent from its embedded table" and so `config`
in both directions (spec 1.3 section 9.3): admin only, as `REQ_EXTENSION` is on an L12 hub today
(`crates/dspi-hub/src/policy.rs:3-10`); our clients treat a `DENIED` read as "no extension channel" as today.

### 1.4 The DSPi Link client, 1.2 and 1.3 (PGLINKC)

- **Version (R4):** the client gets its own constant `CLIENT_PROTO_MINOR` = 3 in its `hello`; `PROTO_MINOR`, which our
  hub sends (`server.rs:61`), stays 2 until PGHDEV raises it together with `protocol`, `ext`, `legacy_devices` and
  `denied_ids`, so our hub never claims 1.3 without behaving as one. The client reads the hub's minor: 2 or lower is an
  L12 hub; 3 is an L13 hub, or OH when `caps` names ours.
- **Device object:** `DeviceInfo` gains `protocol: Option<{major, minor}>` and our additive `ext: Option<bool>` (OH
  only). Section 1.1 steps 5-6 decide the generation from it.
- **Platform over Link:** the client reads 8 platform bytes itself (`REQ_GET_PLATFORM` or `SYS.PLATFORM`) rather than
  trusting the listed 7-byte `platform` (spec 7.4), so `probe_quick`'s `listed` path sees the board id (R7c).
- **CMD and polls:** `wIndex` = `(page << 8) | 2` for paged ids (`CmdId::w_index`, seam); page 0 for legacy; a GET
  always carries a non-zero `wLength`; bulk always single-shot (`STORE.ALL_PARAMS`, never the chunk form over Link,
  spec 8.2); batches through `control_in_batch_paged`. A poll on a paged device subscribes to the paged GET of the
  planner's single-request plans; a legacy poll whose plan is several requests is refused at subscribe time with a
  typed error (OD-PGo).
- **Policy lists:** `policy.denied_ids` (`[id, dir]`) is parsed next to `policy.denied`; greying-out uses `denied` for
  a legacy device and `denied_ids` for a paged one. OH fills both (section 1.5); an L13 upstream hub sends `denied`
  empty.
- **Statuses:** 0x10-0x1A are typed (`TransportError::Refused`); `0x86` for a device without `protocol` on an L13 hub
  is the notice of section 1.1 step 5; `0x86`/`0x10` on a device that answered before is a re-attach (section 1.1
  step 8). Retries only for 0x01, 0x04, 0x17, 0x84.
- **The fake hub** (`crates/dspi-link/src/fakehub.rs`) gets modes: `L12Forward` (policy by `bRequest`, forwards
  `wIndex` verbatim, as upstream Console hubs), `L12Drop` (drops `wIndex`, as our hub before PGHUB0), both tested with
  a **control-role** session, and `L13` (upstream rules: probes, `protocol`, forwards nothing to legacy, `0x86`, policy
  by id, `denied` empty, can swap a device's generation under a handle). OH itself is tested in-process (section 4).

### 1.5 Our hubs: both generations, both Link versions (PGHUB0, PGSWHUB0, PGHPOL, PGHDEV)

**PGHUB0 (now, before anything else):** refuse every CMD and every `poll.subscribe` whose `wIndex` high byte is not 0
with `0x86 UNSUPPORTED` (the 1.3 rule for legacy devices), since every device our hub attaches today is legacy. That
closes the hazard whatever else lands. `wp_hb0_windex_is_ignored` changes to assert the refusal for 0xFFFF and the
pass-through for 0 and 2 (the Mac's `WINDEX_GLOBAL` 0 must keep working).

**PGSWHUB0 (now, coordinator on the Mac VM):** the same rule in the Swift opt-out hub (`CommandRouter.swift`,
`USBHubDevice.swift`, `PollScheduler.swift`), with one XCTest. The Swift hub gets nothing else: it stays legacy-only
until it retires (owner decision P2).

**Afterwards (PGHDEV):** the hub opens every device with `paged::open` on attach (the hub is a client of its devices).
**The generation belongs to the live transport handle, never to the registry or inventory record (keyed by serial);
a device is neither listed nor forwardable before its attach probe has finished** (a CMD during it is answered
`NO_DEVICE`, not sent). Forwarding by the request's page, the session's hello minor and the device's generation:

| Request from a client | Legacy device (G1s, G1b, Zero, sender) | Paged device (G2s, G2b) |
|---|---|---|
| page 0, session hello minor <= 2 (L12 clients: upstream 1.1.6 consoles, our older apps, the vendored 1.2 hub page) | forwarded as today, judged by the 1.2 table | **translated by the hub's adapter** (`PagedTransport`'s legacy path), judged by the **stricter** of the 1.2 class and the 1.3 classes of the commands the plan reaches (OD-PGh); the legacy extension 0x00 becomes the tunnel when the device has page 0xE0 |
| page 0, session hello minor >= 3 (L13 clients, our current clients) | forwarded as today, judged by the 1.2 table | **`0x10`**, as an upstream 1.3 hub: a 1.3 client with a lost or zeroed page never becomes a different, state-changing command, and our own client sees a generation change instead of silently riding the translation (R5a) |
| page != 0 | **refused `0x86`** (PGHUB0's rule, kept) | forwarded verbatim (`control_*_paged`), judged by the 1.3 table; firmware statuses 0x08-0x7F forwarded verbatim; a USB STALL explained by `SYS.LAST_RESULT` |
| a bootloader entry (0xF0 or 0x0130) toward a device whose board id is not 0 or unknown | `DENIED` (section 1.9) | `DENIED` |

- **Hub-originated requests** (health, identify, alarms, limits, apply guard, bulk gain, the local hub's direct path,
  P3RO's rollout) keep their 1.1.6-shaped `CmdRequest`s and reach a paged device through the device's
  `PagedTransport`: they are "minor <= 2" in the table's sense.
- **Device object:** `protocol` for a paged device (spec 1.3); absent for a legacy one, which OH still lists and
  forwards to (deviation from spec 1.3 section 7.4, needed by the owner's requirement, OD-PGg); our additive `ext`
  field; cap `legacy_devices`; `hello.proto.minor` 3 (R4); 8 platform bytes read by the hub itself for the board id.
- **Re-attach on a generation change:** detach serial X as G1b and attach X as G2s (and the reverse) rebuilds the
  handle; the old wrapper is never reused.
- **Upstream clients (best effort):** L12 clients keep working with every device, paged ones included, through the
  translation; L13 clients see paged devices with `protocol` and leave legacy devices alone (spec-conformant). The hub
  page we serve (DSPi-Link web build at `86f10e3`, 1.2, a minor-2 client) keeps working with both generations through
  the translation: **do not re-vendor it**.
- **Polls** are shared by `(id, wValue)` with the page in the key (`polls.rs:10` today has none); **analyser sharing**
  (spec 7.15) by id on paged devices, the swallowed `MEAS.RTA_STOP` answered `OK` with no payload; **bulk relay** on
  0x0240 as on 0xA0/0xA1; **busy reads** from both tables.

### 1.6 The tables, re-keyed on (page, cmd) (PGHPOL, PGCORE)

**One rule for every judge: judge the request by its 1.1.6 equivalent when it has one; a translated request by the
stricter of the two classes.**

| Table | Today | After |
|---|---|---|
| Hub policy (`policy.rs`) | `BTreeMap<u8, Row>` from the 1.2 `commands.json` | `classify(CmdId, Dir)`: page 0 by the 1.2 table (kept, renamed `commands-1.2.json`); page != 0 by the 1.3 table vendored from DSPi-Link (`policy/commands.json`, MIT, provenance recorded) plus our private rows (`policy/private.json`: `0xE010` GET and SET `config`); anything absent is `config` in both directions (spec 9.3; today the 13 pin commands). `denied` from the 1.2 table, `denied_ids` from the 1.3 and private tables |
| `mutates` | by opcode, write-as-reads counted | page 0 as today; page != 0: SET only (every 1.1.7 GET is a read) |
| Lock scope (`in_lock_scope`, `policy.rs:210`) | opcode list | `legacy_equivalent` then the same list; no equivalent: outside the scope (refused for Control while locked) |
| Core-voltage rule (`core_vreg_write`, `policy.rs:230`) | `0x00` SET with `wValue` `EXT_CORE_VREG` | the same, and the tunnel `0xE010` SET with the same `wValue` (its equivalent) |
| Limits judge (P1HL, `crates/dspi-hub/src/limits.rs`), apply guard, bulk gain | decode by opcode and payload | fed the equivalent; a paged write with no equivalent that the limits profile would cover is `limits_unknown` (refused) when a profile is set |
| Core read-only mode (`crates/dspi-core/src/read_only.rs`) | `is_pure_read(opcode)` | typed calls unchanged (they are 1.1.6-shaped above the adapter); the new `raw_get_id` passes any GET (all are reads on 1.1.7; the tunnel GET only reads), `raw_set_id` is refused |
| Raw gates (`check_not_gated`, `raw.rs:71`) | gated opcodes (bootloader, factory reset, saves) | `raw_set_id`: gated when its equivalent is gated, or when `SYS.COMMAND_INFO` flags it `FLASH` or `RESTART`, or it has no equivalent and is not a plain value SET |
| TUI `ReadOnlyTransport` (`read_only.rs:342`) | deny by default by opcode | sits above the generation wrapper (unchanged); forwards `control_in_paged` for GETs, refuses `control_out_paged` |
| Android `WriteGuardedCore` | classifies exports | `raw_set_id` write, `raw_get_id` read, the generation getters read |

### 1.7 Measurement on paged firmware (PGMEAS)

PLAN-P5's session drives the generator and the analyser through typed core calls (`stats.rs:255-397`, `rta.rs`), which
sit above the adapter: on a paged device `SIGGEN_CONTROL` 0xA6 becomes `MEAS.SIGGEN_START`/`STOP`/`STOP_NOW`
(0x0948-0x094A, each a single `act_addr` SET), `SIGGEN_SET_CONFIG` becomes `MEAS.SIGGEN_CONFIG` 0x0940 (36 bytes,
exact), the analyser `MEAS.RTA_*` 0x0920-0x092A; `NOTIFY_EVT_SIGGEN_STATE` is unchanged. No P5 file changes. PGMEAS
runs P5's session and live tests against the G2s and G2b fakes, checks that a START is still one transfer (cycle
identity, P5 section 1.1 step 5, depends on it), and adds one line to PLAN-P5's "Firmware-dependent items" table.
Through a hub the generator's class is `control` in both tables.

### 1.8 Firmware updater and rollout, per generation and board (PGFWUP)

- **Bootloader entry** is `REQ_ENTER_BOOTLOADER` 0xF0 on a legacy device and `SYS.ENTER_BOOTLOADER` 0x0130 (a
  zero-length SET) on a paged one; the planner maps one onto the other, so the core's gated call is unchanged. On our
  firmware it stays USB-vendor only (FWSEC1; on G2b by `VC_F_SET_USB_ONLY`, FWPG2). **No BOOTSEL boards** are blocked
  at three points (section 1.9).
- **Images:** `firmware_manifest.rs` entries gain `lineage` (`upstream` or `bofr`) and `generation` (`legacy` or
  `paged`). The K340 legacy image stays available after the port as `bofr-116-final`, which **must contain PAGEGUARD**
  (`df53ede`); the manifest never offers a legacy-generation bofr image without it (R6d: a host-supplied MIDI pipe can
  outlive a re-flash, and a stale `PagedTransport` would then send paged requests to a downgraded board that only its
  guard can refuse).
- **The guard** (`firmware_guard.rs`) adds: a board without BOOTSEL is never offered an image (section 1.9); **bofr to
  upstream** (lineage loss: extensions, alink, the 1.20 V setting, and the directory: stock firmware reads our
  directory as missing, so IO, control-interface and Control Surfaces config fall back to defaults) **asks the typed
  confirmation**, on top of the existing voltage rule, which already asks it for a stock image on an at-risk board
  (`firmware_guard.rs:13-22`); a **generation change** is shown with what will work afterwards (e.g. "an L12 hub cannot
  serve firmware 1.1.7").
- **Rollout (P3RO):** after the reboot the hub re-opens the device (section 1.1 step 7) and verifies version and, on
  our lineage, `EXT_LIST` and the 1.20 V read-back through whichever envelope the new generation has; a group may mix
  generations; a board without BOOTSEL is excluded with a reason.

### 1.9 Boards without BOOTSEL, the DSPi-Zero, the alink sender (PGZERO, PGFWUP)

**The rule (R7a):** a board whose platform byte 7 (board id) is present and not 0, or whose USB descriptor names a
board this build does not know as a standard Pico, **never enters the bootloader, is never offered an image and is
never in a rollout**, for 0xF0 and for 0x0130 alike (a Zero on 1.1.7 later). Unknown fails closed. A byte 7 that is
absent (every Pico firmware so far answers 4 to 7 bytes) means board 0. The core and the hub always ask 8 platform
bytes, over Link too (section 1.4). **Choke points, each with a test on the Zero fake (R7b):** the core's gated entry
(`crates/dspi-core/src/hardware.rs:217`, so every client and the raw path inherit it), the hub rollout's step 4
(`crates/dspi-hub/src/rollout/steps.rs:905`), and the hub router for a client-tunnelled 0xF0 or 0x0130 from any client,
upstream ones included (`router.rs:183`), answered `DENIED`.

**DSPi-Zero** (upstream, GPL-3.0, bring-up): same VID:PID, product string "Weeb Labs DSPi Zero", `bcdDevice` 0x0300,
platform byte 1, board id 1 in byte 7; its submodule is 1.1.6-generation today, so it is a legacy device for the probe
(and will be paged when it moves to 1.1.7: nothing here assumes either). Minimum support (APPS 3.1): the board-id
decode (PGFAKE2, PG-B1), the descriptor fields where the platform gives them (rusb, WebUSB, Android), the name "DSPi
Zero" from `hardware.rs` (shown in caps, the inventory and About, S6), the blocking above; pin pickers keep RP2350 pins
(the Zero refuses the ones it lacks, a refusal we already show). The rest waits for upstream's platform id.

**The alink sender** (`firmware/DSPi-Sender`, ours, same VID:PID, guarded by PAGEGUARD): a G1b device for the probe
today; after FWPG4 a dual-stack paged device with its small vendor set on the paged stack and the tunnel. It is a row of
the compat matrix (section 4).

### 1.10 The firmware port onto 1.1.7 (FWPG0-FWPG6, FWPGHIL)

Plan now, **start FWPG1 when upstream tags 1.1.7** (or after a quiet week on `release/v1.1.7`; its tip moved today).
Before the tag, one throw-away build of the 1.1.7 beta for the K340 target reads its `.data` (S4, coordinator, 0.25):
if upstream's `vendor/` layer alone exceeds the 840 bytes left, FWPG1 grows by RAM work. All firmware WPs are in
`~/prog/DSPi-bofr`, branch `wp/<id>` from `bofr-main`, never pushed by a WP, never flashed by a WP (memory
`k340-flash-standing-go` covers reviewed alink-usb images only; a port image needs the owner's go, FWPGHIL).

- **A merge, not a rebase:** upstream's tag is merged into `bofr-main` (FWPG1). Before it, the last 1.1.6-generation
  commit, **which contains PAGEGUARD**, is tagged `bofr-116-final`.
- **Dual-stack** (OD-PGb): page 0 keeps the 1.1.6 opcodes for legacy hosts, served by a **legacy front generated from
  the same `legacy_map.json`** that dispatches onto upstream's paged handlers (`ctrl_dispatch_*` with a `wIndex`), not
  by keeping 1.1.6's `vendor_commands.c`. The 14 custom rows are by hand in C. Tables are `const` (flash). A persisted
  switch turns the legacy front off (default on, S3) once no legacy host is left. Upstream's own page-0 walk test is
  changed to expect our legacy answers (recorded as our divergence).
- **Extensions on page 0xE0** (`pages/ext.def`, `vc_page_ext.c`): one tunnel command `0x10`, GET and SET, `wValue` =
  extension id checked by our own handler; refusals as `VC_ERR_*`; the same handlers serve `REQ_EXTENSION` 0x00 on
  page 0.
- **PAGEGUARD's guard is replaced**, not extended: on 1.1.7 the dispatcher refuses unknown pages itself; page 0 goes to
  the legacy front, 0xE0 to ours. (PAGEGUARD refuses 0xE0 on the 1.1.6 line, correctly: there the extension channel
  stays on 0x00.)
- **FWSEC1** becomes `VC_F_SET_USB_ONLY` on `SYS.ENTER_BOOTLOADER` and a check inside the tunnel's SET for
  `EXT_CORE_VREG` SET and TRIAL above 1.20 V (a flag is per command); `core_vreg_request_transport_ok()` stays;
  **MIDI counts as off-USB** (`CTRL_SOURCE_MIDI` joins `VC_NUM_SOURCES`, added in FWPG1). The legacy front's 0xF0 is
  gated the same way.
- **1.20 V on the K340** stays (`core_vreg_boot_apply()` replaces both hardcoded `vreg_set_voltage(VREG_VOLTAGE_1_15)`
  calls); upstream's save paths zero `reserved[2]` of the latency block, where our selection lives, so a host test
  proves every paged save path keeps our private directory bytes and that a zeroed or corrupt selection still boots
  the K340 build at 1.20 V (FWPG5).
- **Pins:** the alink, scope marker (GPIO 22/21), identify LED and GP23 become `vc_pins.c` claims.
- **Directory numbering** (OD-PGl): ours moves to a private band (`0x80 | n`) with a one-time migration from our
  V26/V27.
- **USB MIDI** on `ctrl_dispatch_*` with `wIndex` (the SysEx frame already carries it), chunk sessions keyed on
  `STORE.ALL_PARAMS_CHUNK` 0x0241 on the paged stack and on 0xA2/0xA3 through the legacy front.
- **Generated files** (`vc_ids.h`, `vc_page_table.inc`, `Documentation/protocol/dspi_commands.json`) and
  `CMakeLists.txt` are owned by FWPG1; afterwards **only the WP that adds `.def` rows regenerates them** (FWPG2 in FW-B,
  nobody else in any wave).
- **The legacy front's precondition (R10d):** dual-stack is safe on a direct link, where the page byte is honoured.
  Behind anything that drops the page (our hub builds before PGHUB0, the Swift hub before PGSWHUB0, an unknown relay) a
  paged request would reach the legacy front as the same-number 1.1.6 opcode, which stock 1.1.7 would have refused. So
  **no G2b board is used with a hub build older than PGHUB0 and PGSWHUB0** (FWPGHIL's precondition; section 6).

### 1.11 What works where: the expected matrix

Our clients (every client, through the core):

| Firmware \ route | USB | USB MIDI | L12 hub | L13 upstream hub | OH (our hub) |
|---|---|---|---|---|---|
| G1s | full, no ext | (no MIDI on stock) | full, no ext | notice: hub serves 1.1.7+ only; nothing sent | full, no ext |
| G1b (guarded or not), sender | full + ext | full + ext | full; ext admin-only (as today) | notice, nothing sent (a bridge does not list it) | full + ext |
| G2s | full (adapter), no ext | (no MIDI on stock) | notice: does not answer 1.1.6, needs a 1.3 hub; only `REQ_GET_PLATFORM` sent | full (adapter), no ext | full (adapter), no ext |
| G2b | full (adapter) + ext (tunnel) | full + ext (paged over MIDI) | full as a legacy device (its legacy stack) + ext admin-only | full + ext admin-only (0xE0 is `config` there) | full + ext |
| Zero (G1s today) | full as G1s, named "DSPi Zero", no update | — | full, no update | notice | full, no update or rollout; a tunnelled bootloader entry `DENIED` |
| G2s at a newer page revision | full; SET lengths judged by the device | — | as G2s | full | full |

OH with upstream clients (best effort):

| Client \ device on OH | G1s, G1b, Zero, sender | G2s, G2b |
|---|---|---|
| L12 client (minor <= 2: upstream 1.1.6 consoles, our older apps, the vendored hub page) | as today | works through the hub's translation; legacy ext 0x00 reaches G2b's tunnel |
| L13 client (minor 3: JS library 1.3.0, future consoles) | listed without `protocol`: the client sends nothing (spec-conformant) | full by page; a page-0 request answers `0x10`; ext invisible to it unless it uses 0xE010 (`config`) |

Other paths:
- **The Swift opt-out paths** (`-CoreTransport NO`, `-CoreHub NO`): legacy only, page 0 only. Safe: on G2s the device
  shows as dead (page 0 refused); after PGSWHUB0 the Swift hub refuses a client's page toward any device. Retired per
  owner decision P2.
- **Old hosts directly on our firmware** (an old app, the ESP32 panel over UART at control-interface v22, `dspi_poke`):
  work on G2b through its legacy stack; on G2s they see a dead device (page 0 refused), which is upstream's design.

---

## 2. Work packages

**Shared rules for every PG WP.** `PLAN-CLIENTS.md` section 1, `WP-BRIEF.md`, the wave1 brief
(`work/wave1-brief.md`: containers only, no tmpfs, no devices, scoped checks, never push, the commit trailer), PLAN.md
section 1, and:
- **The hard rules of section 1.1 fail tests by themselves (R9a):** every fake generation panics on a violation (a
  paged request other than the probe on a legacy generation, a page-0 request on `Stock117`), and a fake poisoned by a
  violation fails in `Drop`; a test that means to send one opts in with `allow_violation()`. No WP needs to remember an
  assertion.
- **Generated, not transcribed:** every paged id, length, bound, flag, page revision and adapter row comes from the
  vendored JSON through `build.rs`; hand-written code exists only for the 14 custom rows, the extension row and the
  discovery decoders, each citing the migration guide section it implements. The seam's literals are checked against
  the generated table (PGPROTO).
- **Vendored files and pins (R8):** `crates/dspi-proto/firmware/protocol/` holds `dspi_commands.json` and
  `legacy_map.json` from **one** firmware commit of `release/v1.1.7` (today `103ba37`, 224 commands), taken from the
  firmware repo (GPL-3.0 project files, data), not from DSPi-Link's copy; `crates/dspi-proto/firmware/PROVENANCE.toml`
  gets a section of its own for them with the commit and the **counts recorded at vendoring time** (commands per page,
  opcodes, adapters by kind), which the coverage test compares against, never literals in this plan; `SHA256SUMS`
  rows. The 1.3 policy table and fixtures come from DSPi-Link `4aec5ab` (MIT, `NOTICE.md` row). **The legacy headers
  stay pinned at `bofr-116-final`:** after the port `config.h` on `bofr-main` has no `REQ_*` defines, and `build.rs`
  generates every legacy opcode from them; `docs/terminal/firmware-bump.md` says so (PGPROTO). `ci/licence-inventory.py`
  passes. Re-vendoring at upstream's tag, and our fork's regenerated `dspi_commands.json` with the 0xE0 row after
  FWPG2, is PGREV (section 8).
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
  so every existing transport and wrapper compiles unchanged and fails closed. `generation()` and
  `control_in_batch_paged` are added by PGWIRE and PGADAPT in their additive lines.

### Wave PG-A: the hazards, the wire, the wrapping point, the registry (start now)

**PGHUB0: Our hub refuses a page toward legacy devices.** [TODO] Size 0.75. Wave PG-A. **Priority: first.** **Owns:**
`crates/dspi-hub/src/device.rs` (`execute` and its tests, incl. `wp_hb0_windex_is_ignored` at `:871`),
`crates/dspi-hub/src/polls.rs` (the subscribe check), new `crates/dspi-hub/tests/wp_pghub0_page_guard.rs`.
**Implements:** section 1.5 "PGHUB0". **Done when:** every one of the 15 dangerous reads of the migration guide 4.1
(e.g. GET `0x0553`, `0x0892`, `0x0891`, `0x0890`) and the SET `0x0844` sent through the real hub in-process to a fake
that counts requests answer `0x86` with **zero** requests reaching the fake; a page-0 request from a Mac-style client
(`wIndex` 0) still works; a poll on page 0x05 is refused; the old test asserts the new rule; `cargo test -p dspi-hub`
scoped to the touched tests passes. **Deps:** none.

**PGSWHUB0: The Swift hub refuses a page (coordinator, Mac VM).** [TODO] Size 0.5. Wave PG-A. **Priority: first, with
PGHUB0.** **Owns:** `apps/apple/DSPi Console/Link/{CommandRouter,USBHubDevice,PollScheduler}.swift`, one new XCTest.
**Implements:** section 1.5 "PGSWHUB0". **Done when:** the XCTest sends GET `0x0553` and a poll on page 0x05 through the
Swift hub to its `FakeTransport` and gets `0x86` with no USB call; `wIndex` 0 and 2 pass; `ci/macvm.sh macos` builds.
**Deps:** none.

**PGWIRE: Every transport carries the page; `paged::open`; the probe; `LegacyGuard`.** [TODO] Size 4. Wave PG-A.
**Owns:** `crates/dspi-transport/src/{usb,midi,link,mock}.rs` (the paged methods and `carries_pages`), new
`crates/dspi-transport/src/paged.rs` (`open`, `LegacyGuard`, a placeholder `PagedTransport` that refuses with the
notice), additive lines in `crates/dspi-transport/src/lib.rs` (the `mod`, `Transport::generation()` default `None`),
`fake.rs` (the paged methods for the existing legacy fake: the page is ignored and the same-number opcode runs, as
1.1.6 does, and the fake panics unless `allow_violation`; new `FakeOptions.page_guard` = PAGEGUARD: STALL),
`crates/dspi-wasm/src/usb.rs` (`UsbHost` gains the `index`; the `open` call at `:341`), `apps/web/src/core/usb.ts`
(index from the call) and `apps/web/test/pages/webusb-mock.ts`, **the 10 construction sites** (`crates/dspi-core/src/
{device.rs,usb_pipe.rs,link.rs,host_transport.rs}`, `crates/dspi-hub/src/usb.rs`, `crates/dspi-tui/src/shell/connect.rs`,
`apps/terminal/src/doctor.rs`: the `open` call only), `crates/dspi-session/src/probe.rs` (`ProbeKey`/`Reusable` keyed by
generation, R6a; no `Capabilities` change), one notices block. **Implements:** sections 1.1 steps 1-4 and 7 and 1.2's
layering; **until PGADAPT lands, a paged device is refused** with the notice "this device runs firmware 1.1.7, which
this version cannot control yet" and nothing else is sent (APPS R1). **Done when:** the probe's SETUP bytes equal the
guide's on USB (mock) and WebUSB (mock); G1s and G1b fakes (guarded and not) are legacy after exactly one probe
request and `generation()` says so through `open`; a mock that answers the guide's 16 bytes is refused with the notice
and receives nothing after the probe; `UnknownMajor` and `Garbled` send nothing more; a probe timeout is retried three
times then fails (never legacy); a `Reusable` made on one generation is refused on the other; `LegacyGuard` refuses
every paged call with no request on the wire; `cargo test -p dspi-transport -p dspi-session` and the core's connect
tests; `ci/check.sh --step web` unit tests for the WebUSB index. **Deps:** the seam.

**PGPROTO: The registry, the planner, the reverse view.** [TODO] Size 4. Wave PG-A. **Owns:**
`crates/dspi-proto/src/paged.rs` (beyond the seam) and new `src/paged/` (except `custom.rs`, PGCUST's),
`crates/dspi-proto/build.rs` (an additive section), `crates/dspi-proto/firmware/protocol/` (new, vendored),
`crates/dspi-proto/firmware/PROVENANCE.toml` (the new section) and `SHA256SUMS` rows, `crates/dspi-proto/Cargo.toml`
(`serde_json` as a build-dependency), `NOTICE.md` row, `docs/terminal/firmware-bump.md`. **Implements:** section 1.2:
generated `paged::cmds` constants and metadata incl. page revisions; result codes and status families; decoders for
`SYS.PAGE_LIST`, `SYS.PAGE_COMMANDS` (bitmaps), `SYS.COMMAND_INFO`, `SYS.LAST_RESULT`, `SYS.PLATFORM`; `PageSet`;
the planner for `direct`, `act_value`, `act_addr`, `act_packed`, `split` and our extension row, with `custom` rows
delegated to `paged::custom` (a stub returning "unsupported" until PGCUST); `legacy_equivalent` for the same rows.
**Done when:** a coverage test compares every page, command and adapter count with `PROVENANCE.toml` and fails on any
row the generator could not read; the seam's literals equal the generated ones; transcribed vectors from the guide (3.2
before/after bytes, 4.6, 6.4 split rows, 6.6 write-as-reads, 7.1 reply lengths) pass; `proptest_*`: no plan step on
page 0, no zero-length GET, SET lengths in bounds, planner-then-reverse identity on every invertible row; the ext row
tunnels only with 0xE0 in the `PageSet`; `cargo build -p dspi-proto --target wasm32-unknown-unknown` in the cross
profile. **Deps:** the seam.

**FWPG0: The envelope in the extension spec (firmware repo, docs only).** [TODO] Size 0.5. Wave PG-A. **Owns:** in
`~/prog/DSPi-bofr`, branch `wp/FWPG0` from `bofr-main`: `Documentation/Features/extension_channel_spec.md` (new
section "Envelopes": 1.1.6 `REQ_EXTENSION` 0x00, 1.1.7 page 0xE0 tunnel `0xE010`, `wValue` = id, payloads unchanged,
GET and SET, refusals as `VC_ERR_*` never 0x02/0x03, `SYS.PAGE_LIST` lists 0xE0, MIDI and the legacy stack, the FWSEC1
rules incl. `EXT_CORE_VREG` TRIAL). **Done when:** the section states every field of sections 1.3 and 1.10, and
PGPROTO's ext row and PGFAKE1's tunnel cite it. **Deps:** none.

### Wave PG-B1: the custom rows, the fakes, the hub's tables

**PGCUST: The 14 custom rows.** [TODO] Size 3. Wave PG-B1. **Owns:** `crates/dspi-proto/src/paged/custom.rs` (and
`custom/` for tests). **Implements:** forward and reverse for `GET_CS_DISPLAY_CFG`, `SET_EQ_PARAM`, `GET_EQ_PARAM`,
`SET_PREAMP` (retired: per-channel `ROUTE.PREAMP` after `SYS.PLATFORM`), `GET_STATUS` (every selector to its paged
struct and offset; the `DIAG` selectors are absent on a release build and refuse), `SAVE_PARAMS`,
`SET_MATRIX_ROUTE`, `GET_MATRIX_ROUTE`, `CLEAR_CLIPS`, `PRESET_GET_STARTUP`, `SET_ALL_PARAMS` (its 1.1.6 length rules),
`GET_USB_ERROR_STATS`, `RESET_USB_ERROR_STATS`, `GET_INPUT_RATE`, each from the guide's sections 6.3 and 6.5 (cited).
**Done when:** each row has a vector test transcribed from the guide; multi-step rows are step machines tested with
scripted replies including a refusal in the middle (no half-applied write is reported as success); `proptest_*` as the
shared rules. **Deps:** PGPROTO.

**PGFAKE1: The paged fakes: registry rows, discovery, strict rules, the tunnel.** [TODO] Size 3.5. Wave PG-B1.
**Owns:** new `crates/dspi-transport/src/fake/{paged,generation}.rs`, one `mod` line and one `FakeOptions` field
(`generation`) in `fake.rs`, `fake/diag.rs` only for the tunnel's ext dispatch. **Implements:**
`FakeGeneration::{Stock116, Bofr116 { page_guard }, Stock117 { revision }, Bofr117}`. The paged front is **its own
inverse code**, not `dspi_proto::paged::legacy_equivalent` (so the planner and the fake cannot share a bug), driven by
the registry for the 1.1.7 rules: page 0 refused (`VC_ERR_UNKNOWN`) on `Stock117`, legacy page 0 on `Bofr117`; exact
SET lengths; GETs with `wLength` 0 refused; NaN refused; `SYS.LAST_RESULT`; `PAGE_LIST` (with a settable page
revision, R8b) / `PAGE_COMMANDS` / `COMMAND_INFO`; the tunnel on `Bofr117`, refusing with `VC_ERR_*`; panics on a
violation unless `allow_violation`. **Done when:** a cross-test holds the fake's inverse equal to `legacy_equivalent`
on every `direct`, `act_*` and `split` row; each generation answers the guide's vectors; `cargo test -p
dspi-transport`. **Deps:** PGPROTO.

**PGFAKE2: Custom-row inverse, Zero, Sender, the board id.** [TODO] Size 2.75. Wave PG-B1. **Owns:** new
`crates/dspi-transport/src/fake/{paged_custom,boards}.rs` (one `mod` line each in `fake/paged.rs`, agreed with PGFAKE1
by name in this entry), `crates/dspi-proto/src/packets.rs` (`PlatformInfo` up to 8 bytes, the board id, R7d).
**Implements:** the fake's native handlers for the 14 custom rows (not PGCUST's code), `FakeGeneration::Zero` (8-byte
platform, product string, `bcdDevice` 0x0300, a bootloader entry that never comes back as BOOTSEL and counts as a
violation), `FakeGeneration::Sender` (its vendor subset; guarded on G1, dual-stack variant per FWPG4's spec).
**Done when:** each custom row answers the guide's vectors; a 4-, 6-, 7- and 8-byte platform reply decodes (byte 7
absent = board 0); the Zero's bootloader entry fails the test by default. **Deps:** PGPROTO.

**PGHPOL: The hub's tables re-keyed.** [TODO] Size 3.25. Wave PG-B1. **Owns:** `crates/dspi-hub/src/policy.rs`,
`tests/fixtures/link/policy/` (the 1.2 table renamed `commands-1.2.json`, new `commands-1.3.json` vendored, new
`private.json`). **Implements:** section 1.6's hub rows as new functions (`classify_id`, `mutates_id`,
`in_lock_scope_id`, `core_vreg_write_id`, `denied_ids`, `judge_view` = the equivalent fed to limits and the apply
guard, `classify_translated` = the stricter of the 1.2 class and the 1.3 classes the plan reaches); the old functions
stay until PGHDEV switches the router. **Done when:** every row of the 1.3 table classes as upstream's; every registry
id absent from the vendored 1.3 table (today the 13 pin commands) is `config` in both directions; **for every
`legacy_map` row and direction the 1.2 class is at least as strict as the 1.3 class of every paged command the plan
reaches** (a test; 0 exceptions today, R5b), and `classify_translated` returns the stricter of the two on a
synthetic tightened row; `0xE010` SET with `EXT_CORE_VREG` triggers the voltage rule exactly as 0x00 does; lock scope
by equivalent; `ROUTE.MASTER_VOLUME_SAVED` GET (0x0553) is `read`, never `FACTORY_RESET`; `cargo test -p dspi-hub` on
the policy tests. **Deps:** PGPROTO.

### Wave PG-B2: the adapter and the Link client

**PGADAPT: `PagedTransport`, wired in.** [TODO] Size 4. Wave PG-B2. **Owns:** `crates/dspi-transport/src/paged.rs`
(PGWIRE's placeholder becomes the adapter; `open`'s paged branch does discovery), additive lines in
`crates/dspi-transport/src/lib.rs` (`TransportError::Refused { id, result }` and its message,
`control_in_batch_paged` default, `PagedRead`), the notices block (PGWIRE's "cannot control yet" removed).
**Implements:** section 1.2's transport rules incl. S2's `LAST_RESULT` match and R8b's revision rule. **Done when:**
against PGFAKE1/2's G2s and G2b fakes the whole existing connect, readout, bulk round trip, a typed write of each
registry entry, preset save/load, the gated save, the bootloader entry (to the fake's reboot) and the extension reads
(G2b) pass unchanged; absent commands are refused locally with no request on the wire; a paged STALL with a matching
`LAST_RESULT` yields `Refused`, a non-matching one `Stalled`; a fake at a higher page revision accepts a longer SET
that the vendored length would refuse; a batch of single-step plans is one paged batch. **Deps:** PG-B1.

**PGLINKC: The DSPi Link client, 1.2 and 1.3.** [TODO] Size 3.5. Wave PG-B2. **Owns:** `crates/dspi-link/src/
{client.rs,messages.rs,fakehub.rs}`, `crates/dspi-transport/src/link.rs`. **Implements:** section 1.4 and section 1.1
steps 5, 6 and 8. **Done when:** in both `L12` modes, with a control-role session, a G2s device gets the "needs a 1.3
hub" notice after one `REQ_GET_PLATFORM` and nothing else, and **no request with a page** leaves the client; a G2b
device is used as legacy with ext admin-only; in `L13` mode a device without `protocol` gets the notice and no CMD; a
paged device's CMDs carry `wIndex` `(page<<8)|2` and a non-zero GET length; connecting to G2s through the `L13` fake
hub uses no more round trips than the legacy connect plus one discovery batch (asserted count, R3); `denied_ids`
greys out on a paged device and `denied` on a legacy one; statuses 0x10-0x1A are typed; retries only for the four
codes; a multi-request poll plan is refused at subscribe; the fake hub swapping a handle's generation makes the client
re-attach and never sends the old generation's request to the new device; **the in-process hub suites pass unchanged
after PGLINKC alone** (our hub still says minor 2, R4); `cargo test -p dspi-link -p dspi-transport --features link`.
**Deps:** PG-B1, PGADAPT (same wave: the review batch merges PGADAPT first; PGLINKC's paged-device tests run after it).

**PGSMOKE: First contact with real 1.1.7 (coordinator).** [TODO] Size 0.5. After PG-B2. Not an agent WP. A spare Pico
2 on stock 1.1.7 beta (never the K340): `uconsole` over USB: probe, discovery, readout, one write, one refused write
with `LAST_RESULT`, a bulk round trip. A difference from the documents becomes a fix in PG-C (R9d).

### Wave PG-C: the hub's devices, the core surface, the updater, the Zero, measurement

**Carried over from the PG-B review (`work/review-pgb/REPORT.md`); each is part of the named WP's done line:**
- **PGCORE:** the three DSPi Link notices `transfer.link_needs_13_hub`, `transfer.link_hub_paged_only` and
  `transfer.link_generation_changed` in `assets/notices/notices.json`, mapped in `crates/dspi-core/src/error.rs` through
  `dspi_transport::link::LinkNotice::of` (the `tests_wp_p1nt` "transfer" family then covers `paged::Refusal` and
  `LinkNotice`); on `LinkNotice::GenerationChanged` (`LinkTransport::needs_reattach`) the core's Link attach builds a
  new `LinkTransport` and calls `paged::open` again, never a retry on the old one (section 1.1 step 8); the listed
  identity uses `LinkTransport::read_platform` (8 bytes) for the board id, not the hub's 7-byte `platform` (R7c);
  meter and analyser polls on a paged device are planned with `dspi_transport::link::paged_poll` and their payloads
  fed through the same plan; `ProbeKey`/`Reusable` gains the page list (section 1.1 step 7; today it has the
  generation and the protocol version only).
- **PGHDEV:** the router gates on the original request (`mutates_id`), not on `mutates(judge_view)` (a paged status
  read whose equivalent is a 1.2 `config` row would be refused for a locked Control session); a mutating paged SET
  whose `judge_view` is `None` is refused whenever a limits profile is set (a `STORE.ALL_PARAMS` SET of another
  length has no view); a page-0 request toward a paged device is never classed by `classify_id` alone: branch on the
  generation and the hello minor first, then `classify_translated`; `mutates_id` treats a GET of an id outside the
  registry and the tunnel as a write (upstream's "every GET is a read" covers its own pages); `status_of` forwards a
  `TransportError::Refused` code (0x10-0x1A) verbatim instead of `ERROR`.
- **PGZERO, PGFWUP:** a paged Zero cannot be told from a Pico by `SYS.PLATFORM` (the registry fixes it at 7 bytes;
  PGFAKE2's paged Zero answers 7): the product string and `bcdDevice` identify it where the descriptor is at hand
  (USB, WebUSB, Android, our hub's own attach), and our hub passes the board on in its device object; behind an
  upstream hub it is unknown, which the manual says. The legacy readers ask `PlatformInfo::FULL_LEN` (8) everywhere
  (`REQUEST_LEN` is still 7 after PG-B, so a legacy Zero reads as board 0 until then).
- **PGSMOKE:** the two things PG-B took from upstream's source and not from a device: USB MIDI answers a paged
  refusal with the `VC_ERR_*` code as the frame status (`vc_core.c` `ctrl_dispatch_*`), and the probe is never refused
  `VC_ERR_BUSY` on USB (the adapter reads no `SYS.LAST_RESULT` after a STALLed probe).

**PGHDEV: Our hub serves both generations.** [TODO] Size 5. Wave PG-C. **Owns:** `crates/dspi-hub/src/{router.rs,
device.rs,polls.rs,registry.rs,usb.rs,local.rs,snapshot.rs}`, the hello/caps/device-object block of `server.rs` and
`PROTO_MINOR`'s use there, the analyser-sharing code it touches. **Implements:** section 1.5 after PGHUB0: `open` on
attach, the generation on the live handle, no forwarding before the attach probe, the forwarding table incl. the
minor rule and the stricter-of judge, `protocol`/`ext`/`legacy_devices`, minor 3, both denial lists, statuses,
`LAST_RESULT`, bulk relay, polls keyed by page, analyser sharing by id, the router's `DENIED` for a bootloader entry
toward a board without BOOTSEL, the router on PGHPOL's functions. **Done when:** the in-process hub with each fake
generation passes: an L13-style paged client works on G2s and G2b, gets `0x86` on G1s/G1b and `0x10` for a page-0
request to G2; an L12-style (minor 2) client works on all of them (translated on G2); our client works on all; detach
G1b serial X and attach G2s serial X (and the reverse): no paged request reaches the G1b fake and no page-0 request
the G2s fake; a CMD during the attach probe is `NO_DEVICE` and not sent; two polls on the same command number on
different pages are two subscriptions; a tunnelled 0xF0 and 0x0130 to the Zero fake are `DENIED`; a G2b's tunnel is
reachable for admin only; the hub's existing suites pass on G1 fakes unchanged; `cargo test -p dspi-hub` scoped.
**Deps:** PGHUB0, PG-B1, PG-B2.

**PGCORE: The core's surface for the generation.** [TODO] Size 2.5. Wave PG-C. **Owns:** `crates/dspi-core/src/raw.rs`
(`raw_get_id`, `raw_set_id`), `read_only.rs` (the raw id rules), `info.rs` (the generation from
`Transport::generation()`, protocol version, page list and ext envelope in device info; the board name taken from
`hardware.rs`), `notify.rs` (event 0x11), the forwarding of the paged calls and `generation()` in the production
wrappers (`crates/dspi-core/src/link.rs` `BackgroundTransport` and `RtaLinkTransport`, `crates/dspi-core/src/rta.rs`
`PollTransport`, `crates/dspi-session/src/{bulk_gain,rta_sharing}.rs` `Guarded`/`Held`, `apps/terminal/src/core_route.rs`
`CoreTransport`, `crates/dspi-tui/src/shell/read_only.rs`, `crates/dspi-hub/src/api/transport.rs`), the bindings (UniFFI,
C, wasm) under markers, the notices block. **Done when:** the gates of section 1.6 hold (a raw `0x0130` SET is gated; a
raw GET passes in read-only mode; a raw SET does not); the device info shows each generation from the fakes through
every wrapper; a 0x11 notification re-reads the pins; `ci/drift.sh write` clean; the Android contract regenerated and
classified. **Deps:** PG-B2.

**PGFWUP: Updater and rollout per generation and board.** [TODO] Size 2.5. Wave PG-C. **Owns:**
`crates/dspi-core/src/{firmware_guard.rs,firmware_manifest.rs,firmware.rs,hardware.rs}`, `crates/dspi-hub/src/rollout.rs`
and `crates/dspi-hub/src/rollout/`. **Implements:** sections 1.8 and 1.9's core and rollout choke points, the board
name in `hardware.rs`. **Done when:** the bootloader entry reaches each fake generation's reboot (0xF0 on legacy,
0x0130 on paged); on the Zero fake the gated entry refuses (raw path included), the guard offers no image and the
rollout excludes it with its reason, for a legacy and a paged Zero alike; an unknown board id fails closed; bofr to
upstream asks the typed confirmation; the manifest refuses a legacy bofr image without the PAGEGUARD stamp; the
rollout re-opens after reboot and verifies `EXT_LIST` and the 1.20 V read-back through the new generation's envelope;
a mixed group rolls out; existing P3RO tests pass. **Deps:** PG-B2.

**PGZERO: DSPi-Zero identification.** [TODO] Size 0.75. Wave PG-C. **Owns:** the `DeviceDescriptor` product and
`bcdDevice` fields (`crates/dspi-transport/src/lib.rs` additive, `usb.rs`, `crates/dspi-wasm/src/usb.rs`, the Android
descriptor read in `apps/android/` where the fd is opened), the inventory row in `crates/dspi-hub/src/inventory.rs`.
**Done when:** the Zero fake is named "DSPi Zero" in caps, About and the inventory (the name from `hardware.rs`); a Zero
behind a link is still recognised by byte 7. **Deps:** PG-B1.

**PGMEAS: Measurement on paged firmware.** [TODO] Size 0.75. Wave PG-C. **Owns:** new
`crates/dspi-core/src/measure/tests_wp_pgmeas.rs` (one `mod` line in `measure.rs` under a marker),
`docs/core-merge/PLAN-P5.md` (one row in "Firmware-dependent items"). **Implements:** section 1.7. **Done when:** P5's
session and live tests run green on the G2s and G2b fakes; START is one transfer; the generator config refuses a wrong
length locally. **Deps:** PG-B2; P5SES and P5LIVE merged (else PGMEAS waits; it is not on PG's critical path).

### Wave PG-D: the matrix, the clients, the docs

**Carried over from the PG-C review (`work/review-pgc/REPORT.md`); each is part of the named WP's done line:**
- **A new small WP, before the client WPs (PGFWX, 1 unit; owns the bindings and `WriteGuardedCore`):** export
  `FirmwareTarget`, `DspiCore::firmware_target` and `firmware_update_check` over UniFFI, C and wasm. After PG-C only the
  hub rollout calls the update check; every client's flash path (Qt, Android, web, Terminal `uconsole flash`, Swift,
  `picoboot.rs`) still asks `firmware_guard_check`, the voltage rule alone. So a client does not yet ask the typed
  confirmation for bofr to upstream on a board the voltage rule clears (a Pico on our firmware at 1.15 V), does not show
  the generation change, and still offers an image to a board without BOOTSEL (its bootloader entry is refused by the
  core's gate, so nothing is flashed). **PGQT, PGAND, PGWEB, PGTM, PGSWIFT** switch their update screens to it.
- **PGSWIFT:** the Swift opt-out paths send `REQ_ENTER_BOOTLOADER` themselves (`Commands.swift`
  `enterBootloaderForInstall` and `enterBootloader`, the branch without a core) with no board check: a legacy DSPi-Zero
  on `-CoreTransport NO` is sent 0xF0. Read 8 platform bytes there and refuse a board id that is not 0, or remove the
  branch with the opt-out (owner decision P2).
- **PGMAT:** the cells "paged Zero behind our hub" (the core on top knows it through `DeviceDescriptor::hub_board`: the
  app's local path, a principal's child, our Link client) and "paged Zero behind an upstream 1.3 hub" (unknown: it reads
  as a Pico and `SYS.ENTER_BOOTLOADER` would go out; the manual says so, PGDOC).
- **PGDOC:** `hub-api.md`: the device object's additive `board`, the rollout's device state `excluded`, the reasons
  `no_bootsel` and `lineage_loss`; the manual: a paged Zero behind an upstream hub, and that `raw_set_id` refuses every
  command flagged `FLASH`, `RESTART` or `BULK` (use the 1.1.6 form through `raw_set`, which the adapter translates).
- **PGTM, PGQT, PGWEB, PGAND:** `device_protocol()` costs one platform read per call (PG-C merge): call it once per
  connection, off the UI thread; `raw_set_id` reads `SYS.COMMAND_INFO` per call.
- **PGTM:** `PrincipalTransport` and `LocalTransport` say the generation but no `paged_discovery()`: a core on them
  shows a paged device with an empty page list and the ext envelope from the 1.1.6-shaped probe.

**PGFWX: The update check over the bindings.** [DONE] Size 1. Wave PG-D, before the client WPs. **Owns:** the
bindings of the update check (UniFFI derives and exports in `crates/dspi-core/src/firmware_guard.rs`, new
`crates/dspi-capi/src/fns_misc/firmware_update.rs`, new `crates/dspi-wasm/src/firmware.rs`), `WriteGuardedCore`,
`dspi-ffi`'s `NOT_EXPORTED`. **Did:** exported `FirmwareTarget` (a record; its board is now `board_id: Option<u8>`,
platform byte 7, with `FirmwareTarget::board()` for Rust, so no remote type is needed), `FirmwareUpdateVerdict`,
`FirmwareUpdateRule`, `FirmwareLineage`, `FirmwareGeneration`, `DspiCore::firmware_target` and
`firmware_update_check` over UniFFI; `dspi_firmware_target` and `dspi_firmware_update_check` (the target as the JSON
`dspi_firmware_target` gave; null or missing fields are unknown, so no image) in the C ABI; `firmwareTarget` and
`firmwareUpdateCheck {image, target, typed}` in wasm. Both classified as reads (`PASS_THROUGH`). The Android contract
generator types a record taken by reference (`&FirmwareTarget`) as the record. Review PG-C finding 8:
`firmware_target` reads `EXT_LIST` once for the lineage and the voltage record, and `FirmwareTarget::may_be_ours`
treats a lineage that could not be read on an open board (not BOOTSEL, not "no device") as ours, so the typed
confirmation is asked. **Not done (by scope):** no client's update screen switched (PGQT, PGAND, PGWEB, PGTM,
PGSWIFT); `dspi_firmware_flash_confirmed` and `flash_confirmed` still gate the drive write on the voltage record
alone, so a client that switches its screen must flash only on the update check's `allowed`.

**PGMAT: The compat matrix.** [TODO] Size 3. Wave PG-D. **Owns:** new `crates/dspi-core/tests/wp_pgmat_matrix.rs`,
new `crates/dspi-hub/tests/wp_pgmat_hub.rs`, vendored DSPi-Link fixtures under `tests/fixtures/link/upstream-1.3/`
(MIT). **Implements:** section 4's matrix, every cell of section 1.11: our core over USB (mock wire), MIDI (pipe fake),
the fake hub's `L12Forward`, `L12Drop` and `L13` modes and OH in-process, against each fake generation incl. Zero,
sender and a higher page revision; OH against an L12-style client (our `dspi-link` client sending minor 2) and an L13
client replaying DSPi-Link's 1.3 fixtures. Per cell: connect, the expected notice or state read, one control write,
an ext read where expected, a bulk round trip, the bootloader offered or blocked, a generator START where expected.
**Done when:** the matrix table is generated from the test run (`work/wp-PGMAT/matrix.md`) and equals section 1.11;
no fake panicked. **Deps:** PG-C.

**PGWEB: Web.** [TODO] Size 1. Wave PG-D. **Owns:** the device-info and raw screens in `apps/web/src/screens/`
(generation row, raw by id on paged devices), Playwright e2e with the WebUSB mock answering the probe as each
generation. **Done when:** `ci/check.sh --step web` (Chromium and Firefox, 375 px) shows each generation and each
notice; no paged request in the G1 mock's log. **Deps:** PGCORE.

**PGQT: Qt.** [TODO] Size 0.75. Wave PG-D. **Owns:** the Qt About/device-info controller and view, the raw dialog.
**Done when:** `ci/check.sh --step qt`, `--step qt-offscreen` with the fake generations. **Deps:** PGCORE.

**PGTM: Terminal and TUI.** [DONE] Size 0.75. Wave PG-D. **Owns:** `apps/terminal/src/main.rs` (`uconsole info`
generation, `uconsole raw --id 0x0550`), the TUI's About screen. **Done when:** scoped `cargo test -p uconsole` and
`-p dspi-tui` with each fake. **Deps:** PGCORE. **Did (WP PGTM):** new `apps/terminal/src/protocol_info.rs`:
`uconsole info` (rows and `--json` from `device_protocol()`, one call per connection; a paged device with an empty
page list, as on the hub's `LocalTransport`/`PrincipalTransport`, gets a note saying why), `uconsole raw [out] --id
<0xPPCC|PAGE.NAME>` through `raw_get_id`/`raw_set_id` with the core's refusals in words (`VC_ERR_*` by name); `raw
<op>` above 0xFF is refused instead of truncated. Settings > About shows the same rows (`DeviceProtocolView` in
`dspi-tui`, filled by the Terminal's `connected_on`). `uconsole flash` judges with `firmware_target` +
`firmware_update_check`, flashes only on `verdict.allowed`, asks `LINEAGE_PHRASE` on the terminal without
`--accept-core-voltage-risk` (never typed for the person), says the generation change, and offers no image (nor a
bootloader reboot) to a board without BOOTSEL.

**PGAND: Android.** [TODO] Size 0.5. Wave PG-D. **Owns:** the device-info row in the Android settings feature,
`WriteGuardedCore` entries (if not done by PGCORE's contract step). **Done when:** the module's unit tests through the
Gradle slot wrapper. **Deps:** PGCORE.

**PGSWIFT: Swift (coordinator, Mac VM).** [TODO] Size 1. Wave PG-D. **Owns:** the Swift device-info row; the
core-backed paths' tests against the paged fakes. Its `CoreBackedTransport` needs no code (its opcodes go through the
adapter). **Done when:** `ci/macvm.sh macos` builds and the core-backed tests pass with a G2s and a G2b fake.
**Deps:** PGCORE, PGSWHUB0.

**PGDOC: Docs.** [TODO] Size 1.25. Wave PG-D. **Owns:** `docs/integrator/hub-api.md` (section 3 device fields,
section 17 deviations: legacy forwarding, translation for minor-2 sessions only, both denial lists, minor 3),
`docs/core-merge/HUB.md` (wIndex section), `docs/PARITY.md` (rows), `docs/terminal/wire-format.md` (paged section),
the user manual's notices page incl. S5 (an upstream console or hub can still offer stock firmware to a K340: it sees
"legacy, update available"; our components cannot prevent that). **Done when:** every new field, cap and deviation in
PGHDEV is documented and the docs build step passes. **Deps:** PGHDEV.

### HIL

**PGHIL: The real boards (coordinator and owner).** [TODO] Size 1.5. After PG-D. Not an agent WP. **Runs:** (1) the
K340 on `bofr-main` with PAGEGUARD (needs the owner's flash go): USB, USB MIDI, OH from another machine; the 15
dangerous reads sent raw through OH are refused; (2) the spare Pico 2 on stock 1.1.7 (beta or tag): USB in the web, Qt
and Terminal, OH; a generator run; (3) a stock 1.1.6 Pico if available; (4) the alink sender; (5) a DSPi-Zero only if
the owner has one. **Done when:** section 1.11's cells for these boards match, recorded in `work/pghil/REPORT.md`.

### Firmware WPs (`~/prog/DSPi-bofr`; start when upstream tags 1.1.7)

**FWPG1: Merge upstream 1.1.7 into `bofr-main`, build green.** [TODO] Size 3.25. Wave FW-A. **Owns:** the merge and
its 12 conflicted paths (FIRMWARE.md section 3), `CMakeLists.txt`, the generated `vc_ids.h`/`vc_page_table.inc`/
`Documentation/protocol/dspi_commands.json`, `vc_core.h` (`CTRL_SOURCE_MIDI` in `VC_NUM_SOURCES`), stubs for every
extension handler. First tag `bofr-116-final` on the PAGEGUARD merge. **Done when:** `bofr-116-final` contains `df53ede`;
the K340 (`-DDSPI_USB_MIDI=ON -DDSPI_VREG_MV=1200`), pico2 and pico builds link with 0 warnings; `bash
firmware/tests/run.sh` passes (tests of not-yet-ported code disabled with a reason); `core_vreg_boot_apply()`
replaces both 1.15 V calls; RAM check passes. **Deps:** the upstream tag; S4's `.data` reading.

**FWPG2: Extension page 0xE0 and the FWSEC1 flags.** [TODO] Size 2.75. Wave FW-B. **Owns:** new `vendor/pages/ext.def`,
`vendor/vc_page_ext.c`, the `sys.def` flag line, the regenerated files of FWPG1's list, removal of `transport_gate.h`'s
opcode table, `firmware/tests/test_ext_page.c`. **Done when:** host tests: every ext id answers through `0xE010`
byte-identically to the 1.1.6 golden replies; every refusal on the tunnel is a `VC_ERR_*` (never 0x02/0x03);
`SYS.PAGE_LIST` lists 0xE0; `SYS.ENTER_BOOTLOADER`, and an `EXT_CORE_VREG` SET or TRIAL above 1.20 V through the tunnel,
are refused off the USB vendor path, MIDI included. **Deps:** FWPG1, FWPG0.

**FWPG5: Pins, servo, size, directory, 1.20 V.** [TODO] Size 3.25. Wave FW-B. **Owns:** our claims in `vendor/vc_pins.c`,
`input_servo.c` (upstream's seed and our alink smoothing), the linker placement lists, `scripts/ram_baseline.json`,
`flash_storage.c` (the directory renumbering and migration, OD-PGl), the IDLE1 re-check, new
`firmware/tests/test_private_dir.c`. **Done when:** `SYS.PIN_CLAIMS` reports the alink, scope, identify and GP23 pins
claimed (host test); a V26 and a V27 image migrate once to the private version and an upstream-layout directory is not
misread; every paged save path taken from upstream (`STORE.*`, `OUTPUT.*` directory writes) keeps the core-voltage
selection in the latency block's `reserved[2]` and the alink blocks; a K340 build with the selection zeroed or corrupt
boots at 1.20 V (host test of the boot path); RAM check PASS with `.data` within budget. **Deps:** FWPG1.

**FWPG4: USB MIDI and the sender on the paged dispatcher.** [TODO] Size 3. Wave FW-C. **Owns:**
`firmware/DSPi/midi_control.c`, `firmware/tests/test_midi_control.c`, `firmware/DSPi-Sender/*`, `tools/midi_ctrl/*`.
**Done when:** host tests: a paged request over MIDI reaches its handler with its `wIndex`; chunk sessions on 0x0241;
`CTRL_SOURCE_MIDI` in `SYS.LAST_RESULT`; the FWSEC1 refusals of FWPG2 hold over MIDI; the sender answers
`SYS.PROTOCOL_INFO`, `SYS.PLATFORM`, its vendor subset on the paged stack and the tunnel, and keeps page 0. **Deps:**
FWPG2.

**FWPG3: The legacy front (dual-stack).** [TODO] Size 4.5. Wave FW-D. **Owns:** new `vendor/vc_legacy.c`,
`vendor/vc_legacy_custom.c`, a generator mode in `scripts/gen_vendor_protocol.py` emitting the `const` table from
`legacy_map.json`, the persisted on/off switch (S3), `firmware/tests/test_legacy_front.c`, upstream's page-0 test
adapted. **Done when:** replaying upstream's golden 1.1.6 captures (commit `7571871`) through the legacy front gives
the same replies, except rows the map marks as changed; `REQ_EXTENSION` 0x00 reaches FWPG2's handlers; 0xF0 is gated as
in FWPG2; with the switch off page 0 refuses as stock 1.1.7; the tables are in flash (the `.data` delta is 0).
**Deps:** FWPG2, FWPG4 (the MIDI chunk path).

**FWPG6: Tools, tests, docs.** [TODO] Size 2. Wave FW-D. **Owns:** `tools/dspi_poke`, `tools/dspi_test` (take
upstream's, add `--page 0xE0`), `tools/alink/sender_usb.py`, `Documentation/` sections we re-add after taking
upstream's (commands.md, control_interfaces_spec.md, notification spec), `extension_channel_spec.md`'s status line.
**Done when:** the docs describe the dual stack, its switch and the private page; the tools run against the host test
harness. **Deps:** FWPG2, FWPG4, FWPG5.

**FWPGHIL: The port on the K340 (coordinator and owner).** [TODO] Size 1.5. After FW-D and PGMAT. **Precondition:** no
hub build older than PGHUB0, and no Swift hub older than PGSWHUB0, is on the owner's network (section 1.10, R10d).
Backup the configuration, flash with the owner's go, check: 1.20 V read back, directory migrated, alink USB-direct,
our clients over USB, MIDI and OH as G2b, an old app build and `dspi_poke` on the legacy stack, the sender, 30 minutes
without a hang. Then `bofr-main` is pushed to the Bofr5555 fork (memory `push-forks`), never to Weeb Labs.

### Waves

No two WPs in one wave own the same file, and no WP depends on another WP of its own wave except PGLINKC on PGADAPT in
PG-B2 (merged first in the batch, stated in the entry). PG-B1 and PG-B2 share one Fable review. `notices.json` takes one
block per WP; bindings and `mod` lines are additive under markers. PGHUB0 and PGSWHUB0 run and merge first.

| Wave | WPs | Size | Needs |
|---|---|---|---|
| PG-A | **PGHUB0 0.75, PGSWHUB0 0.5 (coordinator)** first; PGWIRE 4, PGPROTO 4, FWPG0 0.5 | 9.75 | `main` with this plan's seam |
| PG-B1 | PGCUST 3, PGFAKE1 3.5, PGFAKE2 2.75, PGHPOL 3.25 | 12.5 | PG-A |
| PG-B2 | PGADAPT 4, PGLINKC 3.5 | 7.5 | PG-B1; one Fable review for PG-B1 and PG-B2 |
| — | **PGSMOKE** (coordinator, spare Pico 2 on stock 1.1.7 beta) | (0.5) | PG-B2 |
| PG-C | PGHDEV 5, PGCORE 2.5, PGFWUP 2.5, PGZERO 0.75, PGMEAS 0.75 | 11.5 | PG-B2, PGSMOKE's findings; the full gate after PG-C |
| PG-D | PGMAT 3, PGWEB 1, PGQT 0.75, PGTM 0.75, PGAND 0.5, PGDOC 1.25; PGSWIFT 1 (coordinator) | 8.25 | PG-C |
| — | **PGHIL** (coordinator and owner) | (1.5) | PG-D |
| — | S4: `.data` of a 1.1.7 beta K340 build (coordinator) | (0.25) | before FW-A |
| FW-A | FWPG1 3.25 | 3.25 | upstream tags 1.1.7 |
| FW-B | FWPG2 2.75, FWPG5 3.25 | 6 | FW-A |
| FW-C | FWPG4 3 | 3 | FWPG2 |
| FW-D | FWPG3 4.5, FWPG6 2 | 6.5 | FW-B, FW-C |
| — | **FWPGHIL** (coordinator and owner) | (1.5) | FW-D, PGMAT, PGHUB0 and PGSWHUB0 deployed |
| | **App: 20 agent WPs + 2 coordinator WPs (PGSWHUB0, PGSWIFT)** | **49** (+2 HIL) | core, transports, Link client and updater 29.75; hub 9; Swift 1.5; clients 3; matrix and docs 4.25; Zero 0.75; measurement 0.75; FWPG0 counted in firmware |
| | **Firmware: 7 WPs** (FWPG0 in PG-A) | **19.25** (+1.5 HIL, +0.25 S4) | |

The app waves need no firmware: G2b is a fake built to FWPG0's spec until FWPGHIL. Each wave ends with one Fable batch
review (PG-B1 and PG-B2 together); the full gate runs after PG-C and after PG-D. If fewer agents run, PGWIRE, PGPROTO
and PGFAKE1 are the long poles.

---

## 3. The security bar and the compatibility policy

- **The probe is the safety boundary** (section 1.1), made at one wrapping point on the raw wire, enforced twice for
  each direction; the fakes fail any test that crosses it. A failed or ambiguous probe sends nothing more.
- **Our hubs never run a request whose page they cannot honour:** page != 0 to a legacy device is `0x86` (PGHUB0,
  PGSWHUB0); a page-0 request to a paged device is translated only for minor-2 sessions and the hub itself, judged by
  the stricter class, and is `0x10` for everyone else.
- **Every judge sees the 1.1.6 equivalent** (section 1.6), so the lock, the limits profile, the apply guard and the
  core-voltage rule cannot be bypassed by sending the paged form; a paged write with no equivalent is judged by class
  and is outside the lock scope.
- **Generation changes are seen, never assumed away:** the generation belongs to the live handle; caches are keyed by
  it; a changed device is re-attached; every legacy bofr image carries PAGEGUARD.
- **The bootloader and the core voltage stay USB-vendor only on our firmware** (FWSEC1 on both stacks, MIDI counted as
  off-USB, TRIAL included). The K340 never runs stock firmware (PGSMOKE and PGHIL use a spare board). A board without
  BOOTSEL never enters its bootloader from any of our components.
- **Untrusted data:** probe replies, discovery bitmaps, `LAST_RESULT`, hub device objects and policy lists are parsed
  without panics, bounded, and fuzzed with the planner's `proptest_*`.
- **PLAN-SEC unchanged:** TLS, pinning, roles; the new fields are additive.

**The compatibility policy (owner, 2026-11-15) and the dual-protocol requirement (2026-11-18):**
- **No shims between our components:** our client always speaks a device's native generation; our hub translates only
  for minor-2 sessions (upstream 1.1.6 consoles, our older builds) and its own internal requests.
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
| G1s | `FakeDevice` with `extensions: false` (exists); paged calls run the same-number opcode and panic unless allowed | PGWIRE |
| G1b without guard | `extensions: true` (exists); as above | PGWIRE |
| G1b with PAGEGUARD | `page_guard: true`: STALL / status 0x02 for any page | PGWIRE |
| G2s | `FakeGeneration::Stock117 { revision }`: paged, strict 1.1.7 rules, page 0 refused (panics unless allowed) | PGFAKE1 |
| G2b | `FakeGeneration::Bofr117`: paged + tunnel + legacy page 0 | PGFAKE1 |
| custom rows (both) | native handlers, not PGCUST's code | PGFAKE2 |
| Zero | `FakeGeneration::Zero`: 8-byte platform, product string, foreign bootloader (panics unless allowed) | PGFAKE2 |
| Sender | `FakeGeneration::Sender`: vendor subset, guarded (and dual-stack per FWPG4) | PGFAKE2 |
| L12 hubs | `fakehub` `L12Forward` (upstream Console: page forwarded, class by `bRequest`) and `L12Drop` (page dropped), control-role session | PGLINKC |
| L13 hub | `fakehub` `L13`: upstream 1.3 rules, can swap a handle's generation | PGLINKC |
| OH | the real hub in-process | PGHDEV, PGMAT |
| Swift hub | its `FakeTransport` under XCTest | PGSWHUB0 |
| L12 client | our `dspi-link` client sending minor 2 | PGMAT |
| L13 client | DSPi-Link 1.3 fixtures replayed | PGMAT |

Independence of the oracles: the planner (PGPROTO, PGCUST) and the paged fake (PGFAKE1, PGFAKE2) are separate code from
the same upstream data; the guide's byte examples are transcribed by hand as vectors; DSPi-Link's fixtures come from an
independent implementation; firmware host tests replay upstream's golden 1.1.6 captures (FWPG3). What no fake proves:
that the real 1.1.7 firmware answers as its documents say (PGSMOKE early, PGHIL), that a real L12/L13 hub behaves as
modelled, and that the port runs on the K340 (FWPGHIL).

---

## 5. Decisions

Every decision carries a firm recommendation; following the owner's rule of 2026-11-15 they are taken as recommended
unless the owner objects.

| # | Decision | Options | Recommendation | Why |
|---|---|---|---|---|
| **OD-PGa** | **Where the paged protocol enters** | (1) an adapter transport under the existing 1.1.6-shaped calls, generated from `legacy_map.json`, made at one wrapping point `paged::open`; (2) rewrite every call site to paged ids with a legacy adapter underneath; (3) two parallel call layers | **(1)** | About 480 call sites, the fake, the hub's internal requests and Swift's raw tunnel keep working unchanged; `legacy_map.json` exists for exactly this; one place decides, at every construction site, before anything else is sent |
| **OD-PGb** | **The port's shape** | (1) dual-stack: paged + page-0 legacy front, with a switch to turn it off; (2) paged only, as upstream | **(1)** | Keeps old app builds (the Android closed test), the ESP32 panel over UART, `dspi_poke`, L12 hubs and the sender's old hosts working on our boards. On a **direct link** the page byte is honoured, so a page-0 request means exactly its 1.1.6 meaning. Behind anything that drops the page it does not hold: a paged request would reach the legacy front as the same-number opcode, which stock 1.1.7 would refuse; hence the precondition that no hub older than PGHUB0 and PGSWHUB0 is used with a G2b board (section 1.10), and the switch (S3). Built as a generated front onto the paged handlers, tables in flash |
| **OD-PGc** | **The extension envelope on paged firmware** | (1) page 0xE0, tunnel `0xE010`, `wValue` = ext id, payloads unchanged, refusals `VC_ERR_*`; (2) one paged command per extension on 0xE0; (3) keep 0x00 on page 0 only | **(1)** | Ids, payloads and `EXT_LIST` stay byte-identical; one planner row; upstream reserves 0xE0-0xEF for forks; `VC_ERR_*` keeps an upstream 1.3 bridge from dropping the board |
| **OD-PGd** | **Which envelope our client uses on G2b** | (1) paged (incl. the tunnel); (2) legacy, since G2b has it | **(1)** | One code path per generation; the same client works on G2s; the legacy stack is for legacy hosts |
| **OD-PGe** | **Paged devices through an L12 hub** | (1) legacy only, no probe; (2) probe, paged when it answers | **(1)** | A 1.2 hub classifies by `bRequest` (wrong authorisation) and may forward or drop the page; no page-carrying request goes through a hub we cannot trust with one (S1); G2b still works through its legacy stack |
| **OD-PGf** | **Feature detection on paged firmware** | (1) `PAGE_COMMANDS` bitmaps, local refusal shaped as a STALL to the layers above; (2) probe by STALL on the wire | **(1)** | Upstream says not to probe by STALL; the bitmaps are read once; the layers above stay unchanged |
| **OD-PGg** | **Our hub and legacy devices** | (1) list and forward (deviation from spec 1.3 7.4) with `protocol` absent and the cap `legacy_devices`; (2) follow 1.3 and forward nothing | **(1)** | The owner's requirement: our clients must reach `bofr-main` boards through our hub. Upstream 1.3 clients still send nothing to them |
| **OD-PGh** | **Page-0 client requests to a paged device on our hub** | (1) translate for sessions with hello minor <= 2 and the hub's own requests, judged by the stricter of the 1.2 and 1.3 classes; `0x10` for minor >= 3; (2) translate for everyone; (3) `0x10` for everyone | **(1)** | Keeps upstream 1.1.6 consoles, our older apps and the vendored 1.2 hub page working with 1.1.7 boards, with no re-vendoring; a 1.3 client's lost page never turns into a different command; our own client sees a generation change instead of riding a shim; the stricter-of rule means a later upstream tightening cannot be bypassed with the page-0 form |
| **OD-PGi** | **How the hub's judges see paged requests** | (1) through the 1.1.6 equivalent (`legacy_equivalent`); (2) a second set of paged judges | **(1)** | One set of rules; the lock, limits and voltage rules cannot be bypassed by the paged form; unmappable writes are judged conservatively |
| **OD-PGj** | **Vendoring** | (1) the JSON files from one firmware commit in `dspi-proto/firmware/protocol/`, counts in `PROVENANCE.toml`, generated by `build.rs` (`serde_json` build-dependency); legacy headers pinned at `bofr-116-final`; (2) a checked-in generated file | **(1)** | Same rule as the headers: a firmware bump is a file copy; the coverage test shows what changed |
| **OD-PGk** | **`HostTransport` and the Swift opt-out paths** | (1) `HostTransport` stays legacy-only (`carries_pages` false, wrapped by `open` like every raw wire); the Swift opt-out hub gets only PGSWHUB0's page refusal; both retire per owner decision P2; (2) give them paged support | **(1)** | SW-M3 is merged but these still ship; they serve the Swift hub's session 1, Swift's demo and the opt-outs; a paged device behind them fails closed |
| **OD-PGl** | **Our flash directory versions at the port** | (1) renumber to a private band (`0x80 | n`) with a one-time migration; (2) keep V26/V27 and hope upstream's next differs in size | **(1)** | Upstream's next directory is also "V26" (`ac3-dts-decode` already has one); the CRC length check would be luck, not design |
| **OD-PGm** | **Which board runs stock 1.1.7 in PGSMOKE and PGHIL** | (1) a spare Pico 2; (2) the K340 | **(1)** | Stock firmware runs the K340 at 1.15 V (hangs) and loses its directory configuration |
| **OD-PGn** | **When the firmware port starts** | (1) at upstream's 1.1.7 tag or a quiet week; app waves now; (2) now on the beta | **(1)** | The beta's tip moved today (pin queries); the app side needs no firmware (fakes to FWPG0's spec) |
| **OD-PGo** | **Polls whose legacy form needs several paged reads** | (1) refused at subscribe with a typed error; (2) hub-side composition | **(1)** | Every poll the clients use today maps to one paged GET; composition would be a new hub feature for nothing |
| **OD-PGp** | **Boards without BOOTSEL** | (1) keyed on the board id (byte 7 present and not 0) and unknown descriptors, fail closed, at the core's gated entry, the rollout and the hub router; (2) keyed on the name "Zero" | **(1)** | A Zero on 1.1.7 later, a future board, and any client (upstream ones included) through our hub are covered; naming is display only |
| **OD-PGq** | **Older SET lengths against newer firmware** | (1) local length pre-check only at the vendored page revision; (2) always | **(1)** | A revision bump may grow a payload at the end; a stale table must not refuse valid writes to a 1.1.8 device |

---

## 6. What not to do

- Send anything paged (other than the probe on a direct link) to a device that has not answered the probe with the
  magic and major 1; send a probe or any page through an L12 hub; send a page-0 request from our client to a paged
  device; guess a generation after a probe timeout.
- Build a raw transport anywhere without `paged::open`.
- Forward a request with a non-zero page to a legacy device, from either of our hubs or anywhere else.
- Cache a generation across connections, by serial, in a registry record, or across a firmware update; apply a
  `Reusable` across generations.
- Translate a page-0 request for a minor-3 session.
- Hand-transcribe a paged id, length or adapter row; let the paged fake reuse the planner's reverse code; refuse a SET
  locally on a page whose revision is newer than the vendored one.
- Re-vendor the DSPi-Link hub page at 1.3; drop `policy.denied` for 1.2 clients.
- Flash stock firmware on a K340; enter the bootloader of a board whose id is not 0; offer a legacy bofr image without
  PAGEGUARD; flash a port image without the owner's go.
- Use a G2b board on a network with a hub build older than PGHUB0 or a Swift hub older than PGSWHUB0.
- Answer a paged refusal on our firmware with 0x02 or 0x03.
- Keep 1.1.6's `vendor_commands.c` as the legacy stack; put the legacy front's tables in RAM.
- Open a pull request to Weeb Labs, or ask them to reserve anything (0xE0-0xEF is documented for forks).
- Push anything from a WP.

---

## 7. Owner questions and actions

**No blocking question.** Every decision in section 5 has a firm recommendation.

**Owner actions:**
1. **A go to flash PAGEGUARD** (`wp/PAGEGUARD` `df53ede`, reviewed) on the K340 for PGHIL (not covered by the standing
   go, which is for alink-usb test images). It is also the precondition for ever offering `bofr-116-final` as a
   downgrade image (section 1.8).
2. **A spare Pico 2** (and a Pico, if available) for stock 1.1.7 in PGSMOKE and PGHIL and stock 1.1.6 in PGHIL; a
   DSPi-Zero only if you already have one.
3. **When upstream tags 1.1.7:** the start of FWPG1; later a go for the port image on the K340 in FWPGHIL, with no hub
   older than PGHUB0 or Swift hub older than PGSWHUB0 on the network.

---

## 8. Later (not in this plan's waves)

| Item | Size | Why later |
|---|---|---|
| PGREV: re-vendor the registry and legacy map at upstream's 1.1.7 tag (and each later bump); after FWPG2, vendor our fork's regenerated `dspi_commands.json` so the 0xE0 row is generated too | 0.5 each | the beta moves; the coverage test makes it a file copy |
| Pin availability queries (`SYS.PIN_STATE`, `SYS.PIN_CLAIMS`, `*_PIN_AVAIL`, event 0x11 driving the pin pickers) | 1.5 | paged-only feature; pickers work as today meanwhile |
| Explicit per-command refusals shown to the user from `SYS.LAST_RESULT` (beyond typed errors) | 0.5 | polish |
| DSPi-Zero's own platform id or pin list, when upstream decides | about 2 | APPS 3.1: do not guess now |
| Route choice for one board on several paths, mDNS browse restarts (APPS R7, R8) | 2.5 | independent of the protocol; own plan |
| Migrating our own clients' legacy-only code paths (e.g. `-CoreTransport NO`) | — | retire instead (owner decision P2) |
