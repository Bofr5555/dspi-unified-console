# The hub API

*WP P2DOC, 2026-11-15: the contract of PRO phase 2 ([`docs/core-merge/PLAN-P2.md`](../core-merge/PLAN-P2.md)). Status:
**v1, experimental** (OD-P2a): additive changes only, frozen as v1 once phase 3's fleet needs are in. Each family's
section is **draft** until the WP named in its heading builds it and marks it **built**; a draft section is the shape
that WP implements, not a promise that the hub answers it today.*

The hub ([hub.md](hub.md)) keeps running with no console open. This page is everything another program can ask it beyond
the DSPi Link 1.2 messages: its inventory of devices, Identify, health, the shared history, alarms and where they are sent,
parameters, presets and snapshots by name, schedules, MQTT and Home Assistant, a REST mirror, and the hub's statistics.
The DSPi Link spec is DSPi-Link's `spec/dspi_link_protocol_spec.md` (in this repository
`apps/apple/Documentation/dspi_link_protocol_spec.md`); where this page and the code disagree, the code is right and the
page is the bug.

Reference: `crates/dspi-hub` (one module per family, registered as a service in `service.rs`), the consoles' typed calls in
`crates/dspi-core/src/hub_server.rs`.

## Contents

| § | Family | Messages | Cap | Built by | State |
|---|---|---|---|---|---|
| 1 | [Conventions](#1-conventions) | | | P2DOC | built (this page) |
| 2 | [Encryption and pairing](#2-encryption-and-pairing) | `auth.pair`, `auth.pair2`, `auth.rotate`, `hello.hub.key_next` | | SECSRV, SECPAIR, SECPAKE; text SECDOC | built (`auth.pair2`: SECPAKE) |
| 3 | [Inventory](#3-inventory) | `inventory.*` | `inventory` | P2IN | built |
| 4 | [Identify](#4-identify) | `device.identify` | `identify` | P2ID | draft |
| 3 | [Inventory](#3-inventory) | `inventory.*` | `inventory` | P2IN | draft |
| 4 | [Identify](#4-identify) | `device.identify` | `identify` | P2ID | built (P2ID) |
| 5 | [Health](#5-health) | `health.get` | `health` | P2HP | draft |
| 6 | [Event history](#6-event-history) | `event.*` | `events` | P2EV (records: P2EVW) | built |
| 7 | [Alarms](#7-alarms) | `alarm.*` | `alarms` | P2AL | draft |
| 8 | [Notifications out](#8-notifications-out) | `outbound.*` | `outbound` | P2WH | draft |
| 9 | [Parameters](#9-parameters) | `param.*` | `param_api` | P2PA | built (P2PA) |
| 10 | [Presets](#10-presets) | `preset.*` | `param_api` | P2PA | built (P2PA) |
| 11 | [Snapshots on the hub](#11-snapshots-on-the-hub) | `snapshot.*` | `hub_snapshots` | P2SS (store), P2PA (recall) | built (P2SS store, registered by P2PA; P2PA recall) |
| 8 | [Notifications out](#8-notifications-out) | `outbound.*` | `outbound` | P2WH | built |
| 9 | [Parameters](#9-parameters) | `param.*` | `param_api` | P2PA | draft |
| 10 | [Presets](#10-presets) | `preset.*` | `param_api` | P2PA | draft |
| 11 | [Snapshots on the hub](#11-snapshots-on-the-hub) | `snapshot.*` | `hub_snapshots` | P2SS (store), P2PA (recall) | draft |
| 12 | [Schedules](#12-schedules) | `schedule.*` | `schedule` | P2SC | built (P2SC) |
| 13 | [MQTT and Home Assistant](#13-mqtt-and-home-assistant) | `mqtt.*` | `mqtt` | P2MQ, P2HA | built (P2MQ: 13.1; P2HA: 13.2) |
| 14 | [REST and API keys](#14-rest-and-api-keys) | `auth.mint`, `/api/v1/` | `api_keys`, `rest` | P2RS | built (P2RS) |
| 15 | [Statistics and the hub's name](#15-statistics-and-the-hubs-name) | `hub.stats`, `hub.rename` | `hub_stats` | P2HS | built |
| 16 | [Scripting](#16-scripting) | `uconsole api` | | P2TM | built (P2TM) |
| 17 | [Deviations from the DSPi Link spec](#17-deviations-from-the-dspi-link-spec) | | | P2DOC | built (this page) |
| 18 | [Every message](#18-every-message) | | | P2DOC | built (this page) |
| 19 | [Groups](#19-groups) | `group.list`, `group.set`, `group.delete`, `group.changed`; `group:<id>` schedule targets | `groups` | P3GR; text P3DOC | built (P3GR; text P3DOC) |
| 20 | [Group writes and bulk apply](#20-group-writes-and-bulk-apply) | `group.param.*`, `group.preset.load`, `group.apply*` | `group_params`, `group_apply` | P3GP (20.2), P3BA (20.3); text P3DOC | draft |
| 21 | [Firmware rollout](#21-firmware-rollout) | `fw.image.*`, `fw.rollout.*` (TLS only) | `rollout` | P3RO (writer P3PB); text P3DOC | built (P3RO; the writer P3PB) |

---

## 1. Conventions

*State: built (P2DOC). Every family below follows this section; a family section states only where it differs.*

### 1.1 Transport and envelope

- The primary API is JSON over the hub's DSPi Link WebSocket, `/dspi/v1`, after `hello` and authentication (spec 5, 6,
  7.1): one object per text frame, a string `t`, an integer `id` on requests, replies `ok` or `err` echoing `id`, events
  without `id`. JSON keys are `snake_case`. Unknown fields are ignored by both sides; an unknown request type answers
  `unknown_type`; a client drops unknown event types.
- `id` is only ever the request id. A field naming one thing of a family uses the family's noun instead (`snapshot`,
  `alarm`, `rule`, `target`), as `system.alarm.ack {alarm}` does; inside a returned object its own `id` is fine.
- Every answer and every event of these families carries `"v": 1` (the P1HA and P1HL convention). Every event and
  every REST answer also carries `hub`, the hub's id from `hello.hub.id`, so a later fleet server can merge several hubs
  without reshaping anything (PLAN-P2 section 3).
- Objects that belong to an existing file format keep that format's spelling inside the message: a document
  ([document-format.md](document-format.md)), a limits profile, a snapshot index entry (`savedMs`, `inputChannels`, ...),
  an event record ([event-log.md](event-log.md)). Everything else is `snake_case`.
- Times in messages are ISO-8601 UTC with `Z` (`"2026-11-15T09:30:00Z"`), as spec 7.1 says. The one exception is an event
  record, which keeps its file format's `t` (ms since the Unix epoch). Schedule times are local wall-clock times and say
  so (section 12).
- Optional REST mirror: section 14. Every REST request is translated onto the same handlers, with the same roles, checks,
  answers and codes.

### 1.2 Versioning and caps

A hub that has a family lists its cap in `hello.caps`. A client checks the cap before sending a family's messages; a
hub without it answers `unknown_type` (an older hub) or `unsupported`. Caps of this page:

| Cap | Family | Announced |
|---|---|---|
| `inventory` | section 3 | always, on a hub that has it |
| `identify` | section 4 | always; a device without `EXT_IDENTIFY` answers `unsupported` |
| `health` | section 5 | always |
| `events` | section 6 | always |
| `alarms` | section 7 | always (the audio link's `system.alarms` keep working beside it) |
| `outbound` | section 8 | always; no target is set by default |
| `param_api` | sections 9 and 10 | always |
| `hub_snapshots` | section 11 (`snapshot` is spec 7.5's cache, a different thing) | always |
| `schedule` | section 12 | always; no rule by default |
| `mqtt` | section 13 | always; the broker is off by default |
| `api_keys` | section 14 (`auth.mint`, `kind` in `auth.list`) | always |
| `rest` | section 14 (`/api/v1/`) | only while the hub runs with `--rest` |
| `hub_stats` | section 15 (`hub.stats`, `hub.rename`) | always |

Within v1 a family only gains fields, messages and codes; nothing changes meaning. A breaking change would be a new
cap (`param_api_2`), never a silent one.

### 1.3 Addressing a device

- By **serial** (`"serial": "E46058388B1A2E2C"`), the stable name of a DSPi across replugs, hubs and runs. Every
  request that addresses one device takes `serial`.
- **`handle` is also accepted** where DSPi Link has one (an online device, spec 2 and 7.4). Given both, they must name
  the same device (`bad_request` otherwise). An offline device has no handle: address it by serial.
- A serial the hub has never seen (or has forgotten) is `not_found`; one it knows but that is not attached now is
  `offline`; an unknown handle is `no_device` (spec 7.1).
- Several devices at once (schedules today, phase 3's groups) answer with the **multi-target result**, one row per
  device, in the order asked:

```json
{"t": "ok", "id": 9, "v": 1, "results": [
  {"serial": "E46058388B1A2E2C", "ok": true},
  {"serial": "E6614103E7A1B2C3", "ok": false, "code": "offline", "msg": "not attached"}
]}
```

  A row may carry more fields of its own (`value`, `action`). A partial failure is reported, never retried later
  (section 1.7). `group:<id>` targets are expanded to the group's members (section 19.3, built: P3GR); an unknown
  group is one `not_found` row, and a run answers at most 256 rows, the last `bad_request` "truncated" when there was
  more.

### 1.4 Values

Plain JSON values; the parameter's registry kind decides (`crates/dspi-proto/src/registry.rs`, `param.list` shows it):

| Kind | Value in JSON | Example |
|---|---|---|
| `float` | number, in the registry's unit, inside its range | `-20.0` (dB) |
| `int` | whole number, inside its range | `120` |
| `bool` | `true` or `false` | `true` |
| `choice` | the choice's word from the registry, or its number; answers give both (`value` the number, `choice` the word) | `"lowshelf"` or `3` |
| `mask` | whole number, one bit per channel or output | `5` |
| `text` | string, at most the registry's length | `"Stage left"` |
| `trigger` | `null` or no `value` | |
| `bytes`, `status` | not written by the API (`bad_request`); `param.get` answers `bytes` as `value_b64` | |

`idx` is an array of whole numbers, as many as the path's `indices` (`[]` or absent for none): `[2]` for output 3,
`[0, 4]` for channel 0 band 4. Indices count from 0.

### 1.5 Roles

The hub's three roles ([hub.md](hub.md), Roles): **viewer** reads, **control** also changes settings, **admin** does
everything, including the hub's own settings. Each message's row names the least role it needs; a lower one answers
`denied`. A PIN pairing gives at most control; admin is given on the hub's machine. An API key has the role it was minted
with (section 14).

### 1.6 Errors

`{"t": "err", "id": 7, "v": 1, "code": "<code>", "msg": "<one line for a person>"}`, with more fields where a row says.
`msg` is for logs and people; programs branch on `code`.

| Code | Meaning | Origin |
|---|---|---|
| `unauthenticated` | no token or key, or a revoked one | spec 7.1 |
| `denied` | the role does not allow it (and the voltage rule, `EXT_CORE_VREG`) | spec 7.1 |
| `bad_request` | malformed, a missing or wrong-typed field, a value outside the registry's range, a path not written by the API | spec 7.1 |
| `unknown_type` | the hub does not know the message | spec 7.1 |
| `no_device` | an unknown handle | spec 7.1 |
| `not_found` | an unknown serial, snapshot, rule, target, alarm or key | new |
| `offline` | the device is known but not attached; nothing was sent or queued | new |
| `locked` | another session holds the device's lock (spec 7.7) | spec 7.1 |
| `busy` | the session's in-flight limit, or one guard per device | spec 7.1 |
| `rate_limited` | too many requests (`retry_in_ms` when known) | spec 7.1 |
| `conflict` | the hub's state disagrees: a differing snapshot with the same id, a cap of a store reached, a pending guarded apply, a newer revision | new |
| `unsupported` | the device or the hub lacks it (no `EXT_IDENTIFY`, `fw.install`; a rollout of a device not on the hub's own USB, section 21) | spec 7.1 |
| `confirmation_required` | a gated write (`preset.save`, `preset.delete`, `dev.save`, `dev.save.io`) without `"confirm": true` from an admin | the core's token rule |
| `limit_exceeded` | the write crosses the device's limits profile; `limit` names it. Nothing was sent | P1HL |
| `installer_lock` | the device is locked and the write is not volume, mute or preset | P1HL |
| `image_refused` | a whole image (`0xA1`, `0xA3`: a recall, an import) from control on a device with a profile or a lock; also a firmware image the rollout will not put on a device, with `reason` (section 21.3) | P1HL, P3RO |
| `internal` | the hub failed; the log says why | spec 7.1 |
| `tls_required` | a PIN pairing, `auth.rotate`, a `tls_only` token or an API key over plaintext where the hub refuses it (section 2.1); every `fw.*` message over plaintext, in every TLS mode (section 21.1); also the reason of close 4003 | SECSRV, SECPAIR, P3RO |

A device's own refusal of a command (a DSPi Link status such as `STALL`) answers `err` with `code` `device` and `status` the
DSPi Link status name (spec 8.1).

### 1.7 Who writes: principals and attribution

- Every write of this API is a **principal's**: the asking session (or a child session with its role, lock rights and
  client, for the parameter API), a REST key's session, an MQTT key's session, or `Schedule: <name>` (admin). Each has a
  session id from the hub's remote range, so roles, the limits, the lock and the voltage rule judge it exactly as a
  console's write on DSPi Link. Nothing in this API reaches a raw opcode or `EXT_CORE_VREG`.
- **The hub's own writes are a closed list of three**, sent as the hub's internal session: the guarded apply's revert
  (P1HA), **identify** (section 4) and **mute on clip storm** (section 7). Each is recorded in the history with the caller
  or the rule in `by`.
- A write is attributed: other sessions see it as a foreign write (`origin` in the notification, spec 8.3), their undo
  as "changed elsewhere", and the hub's history records `session` and `by` (the client's name, the key's name, the
  schedule's or rule's name).
- The hub never queues or replays a write: a target that is offline fails now (`offline`), and the answer says so.
- The hub never sends `REQ_CLEAR_CLIPS` and never writes the core voltage. It flashes firmware only as an Admin's
  rollout (section 21: its device requests are the Admin's principal's, never one of the hub's own writes);
  `fw.install` stays `unsupported`.

### 1.8 Bounds

Every request, list and store is bounded; past a bound a request answers `bad_request` (a request too large) or
`conflict` (a store full), never a partial write. Pages default to 100 rows and take at most 500. A data file the hub
cannot read is kept aside as `<name>.bad-<time>`, and the hub runs with defaults and a line in its log.

| Store (in the hub's `--store` folder) | Cap | Section |
|---|---|---|
| `inventory.json` | 1024 devices (the oldest offline ones dropped past it) | 3 |
| `events/` | 2 MiB per device (512 KiB per file, 3 rotated), 128 MiB in all, oldest rotated file first | 6 |
| `alarms.json` | 64 KiB; 64 cleared alarms kept | 7 |
| `outbound.json` | 16 targets, 64 queued deliveries each | 8 |
| `snapshots/` | 256 KiB per document, 200 per device, 64 MiB in all | 11 |
| `schedule.json` | 256 rules of at most 16 actions | 12 |
| `mqtt.json` | one broker | 13 |
| `hub-store.json` | 64 paired clients and API keys together | 14 |
| `groups.json` | 64 groups of 64 members | 19 |
| `rollout.json` | the newest 64 rollout records (and at most 4 MiB: the oldest go first) | 21 |
| `firmware/` | 4 images of at most 8 MiB, and their `index.json` | 21.2 |

Each file is written atomically and is 0600 on Unix. A diagnostics bundle ([diagnostics.md](diagnostics.md)) carries
each of them with every secret replaced by `set` or `not set`.

---

## 2. Encryption and pairing

*State: built (SECSRV, SECDISC, SECCORE, SECPAIR, SECWEB; text WP SECDOC, which replaces P2DOC's "On an unencrypted
network"). `crates/dspi-hub/src/server.rs` (the port, modes, `426`, 4003, `/info`), `auth.rs` (pairing, `tls_only`,
idle expiry, `auth.rotate`), `tls_identity.rs` (the key), `crates/dspi-link/src/tls.rs` and `credential.rs` (the pin, the
credential, the pairing link). Why and what users see: [security.md](security.md); the switches:
[hub.md](hub.md), Encryption.*

Everything on the hub's port is TLS 1.3 by default (`uconsole hub --tls require`): the DSPi Link WebSocket, `/dspi/v1/info`,
the pages and the REST mirror (section 14), on the same port, decided by a connection's first byte. MQTT (section 13) and
webhooks (section 8) go out over TLS too. A program that talks to the hub:

- **Pins the hub's key.** The pin is SHA-256 of the key's SubjectPublicKeyInfo: base64url in messages and QR codes,
  `sha256//<base64>` for curl, `SHA256:XXXX-XXXX-XXXX-XXXX` for people. On the hub's machine `uconsole hub tls
  fingerprint` prints all three and `uconsole hub tls cert` the certificate (PEM). Check the key in the TLS handshake,
  before any token or API key is sent; the certificate is self-signed, so check the key (or trust only that certificate),
  not a name or a certificate authority. Our clients do this in the core (`dspi-link`), with the pin kept in the
  credential below.
- **Never falls back to plaintext** for a hub it pinned or one that advertises `tls=1`.
- **Keeps the hub on a local network:** the hub drops peers that are not local; for remote access use a VPN, never router
  port forwarding (spec 6.4).
- **Gives each integration its own API key** with the least role it needs, so it can be revoked alone (`auth.revoke`).
  API keys and webhook secrets are made by the hub and shown once; the hub keeps a hash of a key, and a webhook secret
  only in its 0600 file. REST refuses every request that carries an `Origin` header (section 14).

### 2.1 Modes

| Over plaintext | `require` (the default) | `on` | `on` + upstream compatibility | `off` |
|---|---|---|---|---|
| `/dspi/v1/info`, pages | HTTP `426` (`Upgrade: TLS/1.3`; the body names `https://` and the fingerprint) | yes | yes | yes |
| WebSocket with `auth.token` | upgraded, then closed 4003 `tls_required` | yes, unless the token is `tls_only` | as `on` | yes |
| `auth.pair` | (closed above) | `err` `tls_required` | yes | yes |
| `auth.rotate` | (closed above) | `err` `tls_required` | yes | yes |
| REST with an API key | `426` (the same page) | `426` with `code` `tls_required`, unless the key was minted `plaintext_ok` | yes | yes |
| DNS-SD `tls=1` | yes | yes | no | no |

Over TLS everything is served in every mode but `off` (which answers a ClientHello with a plain `400`). Upstream
compatibility (`--upstream-compat`, off by default, refused with `require`) is the owner-approved switch for upstream
DSPi Link clients, which cannot encrypt; the hub starts with a warning listing what it turns off
(`dspi_hub::server::UPSTREAM_COMPAT_EFFECTS`, the notice `dlink.upstream_compat`), and `/dspi/v1/info` says
`"upstream_compat": true` so every client's hub settings show it. Over plaintext a request's `Host` must be one of the
hub's addresses, its `.local` name or a `--host-name`, else `421` (DNS rebinding); over TLS any name is fine.

`GET /dspi/v1/info` gains `tls` (whether TLS is served), `tls_spki` (the pin, base64url, while it is), `upstream_compat`
and `proxy` (a `uconsole web` pinning proxy, section 2.5). It stays readable over plaintext in `on`, so a client can learn
the pin it should see; that is a hint for display, never trust: the pin that counts is the one set at pairing.

### 2.2 Pairing

- **`auth.pair {pin, name?, role?}`** over TLS, as before (spec 6.2), and the `ok` gains **`hub_key`**: the pin of the key
  this connection met. A client checks it against the key its TLS verified (a mismatch means a proxy in between) and
  stores it as its pin. Over plaintext it is refused in `on` (`tls_required`, before the PIN is looked at: no guess is
  counted, the window stays open).
- The hub shows the PIN with the key's fingerprint and the **pairing link** `dspi://pair?h=<hid>&a=<host:port>&k=<pin,
  base64url>&p=<PIN>` (`uconsole hub --json`: the `pairing` event's `fingerprint` and `pairLink`; a QR code on a
  terminal). A client that takes a link connects with `k` as the pin, so the TLS key must match it before the PIN is
  sent (`hub_key_unverified` otherwise); it still shows the hub's name and fingerprint and asks for a tap. The core's
  parser: `dspi_link::credential::parse_pair_link`, `link_parse_pair_link` in the bindings.
- **`auth.pair2`** (WP SECPAKE): PAKE pairing for native clients, SPAKE2 with the PIN as the password, the hub's id and
  the client's name as the identities, the transcript bound to the TLS session (exporter label `EXPORTER-dspi-pair`).
  Both sides confirm a MAC before the token is issued, so a man in the middle who relays two TLS legs fails at its first
  guess and learns nothing to search offline; 5 failures end the window as for `auth.pair`. Browsers, which cannot
  reach the TLS exporter, and upstream clients keep `auth.pair`. Its messages are SECPAKE's block below.

<!-- WP SECPAKE -->
#### PAKE pairing: `auth.pair2`

*State: built (SECPAKE; PLAN-SEC OD-SECc (3)). Code: `crates/dspi-link/src/pake.rs` (both sides), the hub's
`auth.pair2` in `crates/dspi-hub/src/server.rs` and `auth.rs`; tests `crates/dspi-link/src/pake/tests_wp_secpake.rs`,
`crates/dspi-hub/src/auth/tests_wp_secpake.rs`, `crates/dspi-hub/tests/wp_secpake.rs`.*

A PIN typed into `auth.pair` is hidden by TLS from a listener, but not from a machine in the middle: a client pairing
for the first time has no pin yet and trusts the first key it meets, so a proxy that terminates TLS on both sides reads
the PIN and pairs itself. `auth.pair2` sends no PIN. Client and hub run SPAKE2 with the PIN as the password, and both
confirm the agreed key with a MAC bound to **this** TLS connection, so a proxy fails at its first guess, and what it
sees lets it test no other PIN offline. Five failed confirmations end the window, as five wrong PINs do.

- **TLS only.** Over plaintext the answer is `tls_required` (whatever the plaintext policy: there is no exporter).
  A hub that serves TLS lists the cap `pair2` in `hello.caps`.
- **Our native clients use only `auth.pair2` over TLS**, never `auth.pair` there, whatever `caps` says (a proxy could
  strip the cap). Over plaintext (an upstream WeebLabs hub, `--tls off`) they use `auth.pair`.
- **`auth.pair` is unchanged**: browsers, which cannot reach the TLS exporter, keep it over TLS; upstream consoles keep
  it in plaintext on a compatibility-mode hub. Its rules (OD-SECk) are section 2's.

**The cryptography.** All fields are base64url without padding.

| Item | Value |
|---|---|
| Group | SPAKE2 over Ed25519 (RustCrypto `spake2` 0.5, its M, N and key hash) |
| Password | SHA-256 of the PIN's ASCII digits (what the hub keeps of a window's PIN) |
| Identities | A: the client's `name` exactly as sent (UTF-8); B: the hub's `hid` |
| SPAKE2 message | 33 bytes: a side byte (`A` 0x41 from the client, `B` 0x42 from the hub) and a compressed point |
| Channel binding | TLS exporter (RFC 8446 7.5), label `EXPORTER-dspi-pair`, no context, 32 bytes |
| Transcript | SHA-256 over, each prefixed by its length as a big-endian u64: `dspi-pair2 v1`, `hid`, `name`, `role` as sent (`""` if absent), the client's message, the hub's message |
| Keys | PRK = HKDF-SHA256-Extract(salt = exporter, IKM = the SPAKE2 key); client key = Expand(PRK, `dspi-pair2 confirm client` ‖ transcript, 32); hub key = Expand(PRK, `dspi-pair2 confirm hub` ‖ transcript, 32) |
| Confirmations | HMAC-SHA256(key, transcript), 32 bytes; checked in constant time |

**The exchange.** Before authentication, on one connection:

| Step | Client sends | Hub answers |
|---|---|---|
| 1 | `{"t":"auth.pair2","id":1,"step":1,"name":"Kitchen iPad","role":"control","pake":"<33 bytes>"}` | `{"t":"ok","id":1,"pake":"<33 bytes>"}` |
| 2 | `{"t":"auth.pair2","id":2,"step":2,"confirm":"<32 bytes>"}` | `auth.pair`'s `ok` (`session`, `role`, `token`, `hub_key`, `policy`) plus `"confirm":"<32 bytes>"` |

- **The client confirms first.** Step 1 tells the client nothing it can test a PIN with, so it counts nothing. Step 2
  is the PIN attempt: a confirmation that is missing, malformed or does not verify is answered `unauthenticated` (no
  `msg`) and counts as a wrong PIN, per address and against the window, in the same ledger as `auth.pair`.
- The client checks the hub's `confirm` (and `hub_key` against the key TLS met) before it keeps the token; one that
  does not verify means the token is dropped and the session closed.
- With no window open, step 1 is answered as usual and step 2 fails as a wrong PIN. A new window between the steps also
  fails step 2. The role is cut to `control` as with `auth.pair`; the token is `tls_only`.
- Step 1 again replaces the exchange in progress. One connection may start at most 8 (`rate_limited` after that).
- Other errors: `bad_request` (a step other than 1 or 2, a step 2 with no step 1, a `pake` that is not a 33-byte
  message or not a point, an unknown `role`), `rate_limited` (the address is shut out), `busy` (the client table is
  full), `internal`.

**After a key rotation** (section 2.4, `hello.hub.key_next`) the core's client, once it has accepted the new key, calls
`auth.rotate` in the same connect and stores the fresh token (PLAN-SEC 4.9).
<!-- /WP SECPAKE -->

- Limits, unchanged in kind: a PIN works for 2 minutes; 5 wrong PINs from anywhere end the window, and from one address
  (an IPv6 /64) within 60 s shut that address out for 60 s. Wrong tokens are counted in a ledger of their own, so a client
  retrying a revoked token cannot end a pairing window.

### 2.3 Tokens and `auth.rotate`

- `auth.token {token}` over TLS answers `ok` with `hub_key`, as `auth.pair` does.
- **`tls_only`**: a console's token issued over TLS, or used over TLS once, is refused over plaintext from then on
  (`err` `tls_required`, "connect with TLS"), so an attacker who blocks TLS cannot collect it from a client that already
  knows the hub encrypts. API keys are not marked: REST has `plaintext_ok`.
- **Idle expiry**: a console's token unused for 180 days (`--token-idle-days`; 0: never) is forgotten and must pair
  again; never while the hub's clock is implausible. API keys do not expire.
- **`auth.rotate {}`**, any role, on an authenticated session with a token: `ok {token, hub_key?}`, a fresh token for the
  same client, its role and `tls_only` kept; the old one is dead once this is answered. Over plaintext only where
  pairing is (`tls_required` otherwise). `bad_request` on a session with no token (open access). A client calls it after
  following a key rotation, and may call it at any time.
- Changing between PIN and open access still revokes every token.

### 2.4 Key rotation: `hello.hub.key_next`

`uconsole hub tls rotate` (on the hub's machine) makes a new key and serves it at once. For 30 days `hello.hub` carries

```json
"key_next": {"spki": "<the new pin, base64url>", "prev_spki": "<the old key's DER SPKI, base64url>",
             "sig": "<the old key's ECDSA P-256 signature, base64url>"}
```

where `sig` (ASN.1 DER) is over `"dspi-key-rotate\0" || hid || new_pin` (the hub id's UTF-8 bytes, then the new
pin's 32 bytes; `dspi_link::tls::rotation_message`, `verify_rotation`). A client that pinned the old key re-pins only if `prev_spki` hashes to its pin and the signature
verifies with it, before any token is sent; either because it saw `key_next` on an earlier connection, or because the
handshake met a new key, which it then completes without sending the token, reads `hello`, and checks. Anything else is
`hub_key_changed`: pair again. `rotate --forget-old` (the old key was exposed) announces nothing, so every client pairs
again.

### 2.5 What a client keeps, and what it shows

- **The credential** (opaque to platforms, kept in their existing token stores): `v1` is a bare token from before TLS;
  `v2.<token>.<pin>.<flags>[.<next>]` holds the token, the pin, `t` (TLS required) and a verified successor key. A `v1`
  credential that meets a TLS hub is upgraded to `v2` on that connection (the first-use moment for a client paired before
  TLS), and the platform saves it when it changes (`link_security().credential`). A damaged `v2` is no credential: pair
  again.
- **The errors** the core gives: `hub_key_changed` (another key: pair again), `hub_key_unverified` (a pairing link's key
  did not match), `tls_refused` (TLS failed where it is required: no plaintext retry), `tls_required` (the hub's own
  refusal, or its 4003 close).
- **`link_security()`**: `encrypted`, `hub_key` (the fingerprint), `hub_key_new` (show the fingerprint once),
  `credential`. **When `encrypted` is false** (an upstream hub, `--tls off`, compatibility mode) every client's connection
  bar is **red** and its text ends in **"(unsecured)"**, with an info button explaining it (the notices `dlink.unsecured`,
  `dlink.unsecured.info`, `dlink.unsecured.compat`); a hub on the same machine (loopback) is not shown so.
- **The browser** cannot pin: the web console on `https://<hub>:11915/console/` asks the user to accept the certificate
  once and shows the fingerprint to compare; `uconsole web` (`http://localhost:11916`) relays
  `ws://localhost:11916/hub/<target>/dspi/v1` to other hubs over pinned TLS (`crates/dspi-hub/src/proxy.rs`), loopback
  only. `GET /hub/<target>/dspi/v1/security` tells the page whether that link is encrypted and its fingerprint.

### 2.6 Examples

`curl`, Python and Home Assistant against a TLS hub: section 16.3 and [security.md](security.md). For any HTTP client:
export the certificate once (`uconsole hub tls cert > hub.pem`), then either pin the key (`curl --pinnedpubkey
'sha256//...' -k`) or trust only that certificate (`curl --cacert hub.pem`, Python `cafile="hub.pem"`, `requests`
`verify="hub.pem"`); export again after a key rotation.


---

## 3. Inventory

*State: built (P2IN). Off unless the hub's `ServiceConfig::inventory` is set; announced as `inventory` when on.*

Every DSPi the hub has ever seen, online or not, persisted in `inventory.json`. `device.list` (spec 7.4: handles, a
60 s grace) is unchanged; the inventory is keyed by serial and outlives a replug and a restart of the hub.

| `t` | Role | Request | Answer |
|---|---|---|---|
| `inventory.list` | viewer | `serial?` or `handle?` (one device), `offset?` (default 0), `limit?` (default 100, at most 500) | `devices`: records, online first, then by last seen (newest first), then by serial; `total`; `next_offset` (`null` on the last page). A page also stops before about 512 KiB |
| `inventory.set` | admin | `serial` or `handle`, `location?`, `note?`, `tags?` (each replaces its field; `null` clears; absent keeps) | `device`: the record |
| `inventory.forget` | admin | `serial` or `handle`, `with_data?` (default `false`) | `serial`, `forgot`: a list of what went, in order (`"record"`, then `"events"` if it had history files, then `"snapshots"` if asked and it had any). `conflict` while the device is online; `not_found` for a serial not in the inventory |

A record:

| Field | Meaning |
|---|---|
| `serial` | the key |
| `handle` | the DSPi Link handle while online, else `null` |
| `online` | `true` while attached |
| `name` | the device's name as `device.list` gave it when last seen |
| `platform`, `fw`, `fw_beta`, `outputs` | from `REQ_GET_PLATFORM` (spec 7.4); `fw_beta` absent when unknown |
| `board`, `hw_rev`, `chip`, `chip_rev` | from `EXT_IDENTIFY` where the firmware lists it, else absent (`hw_rev` also when the build set none) |
| `location`, `note` | set by an admin, at most 256 characters each, any Unicode but control characters (a note may hold newlines and tabs); `null` when unset |
| `tags` | at most 16 strings of 1 to 32 characters, trimmed, duplicates dropped |
| `first_seen`, `last_seen` | times; `last_seen` is now while online, else when it went offline |
| `last_reset` | `{reason, prev_uptime_s?, at}` from `EXT_RESET_INFO` (reasons as section 5; `at` the reading's time less the uptime), else absent |

A value past a bound answers `bad_request` and changes nothing. `inventory.forget` removes the record and the device's
history files (`events/`); its snapshots on the hub only with `with_data: true` (each with its `snapshot.changed`). If a
file cannot be removed, the answer is `internal` and the record stays, so the forget can be tried again. Event to every
session: `inventory.changed {v, hub, serial, device}` (`device` `null` when forgotten, or dropped past the 1024-device
cap, oldest offline first, with a line in the hub's log). A firmware change updates the record and puts a `firmware`
history record on the hub's internal bus (stored by the history, section 6).

The file is written atomically, 0600 on Unix, one record per line inside one JSON object, so a torn file still reads
record by record: the hub keeps the whole file aside as `inventory.json.bad-<unix seconds>`, keeps what it could read, and
writes the file again. The diagnostics bundle carries it as `hub/inventory.json`.

## 4. Identify

*State: built (P2ID): `crates/dspi-hub/src/identify.rs`, on when the hub's `ServiceConfig::identify` is set.*

| `t` | Role | Request | Answer |
|---|---|---|---|
| `device.identify` | control | `serial` or `handle`, `seconds` (0 stops; at most 120, longer is shortened), `tone_output?` (an output index) | `serial`, `status`: `{remaining_s, last_tone, chip, chip_rev, board, hw_rev}` read after the write |

- `status` is `null` if the read after the write failed (the write went out). `last_tone` is the output of the last tone
  since the device started, or `null`; `chip` is `"RP2040"`, `"RP2350"` or `"chip <n>"`.

- The LED blinks for `seconds`. With `tone_output` the device also plays its quiet identify tone once (1 kHz, -40 dBFS,
  400 ms, through that output's whole chain); only when asked. There is no tone on firmware without `EXT_IDENTIFY`.
- It is one of the hub's own writes (section 1.7): the extension opcode is admin-only in the policy, so the hub sends
  it, not the caller. The blink goes out whatever the lock; the tone answers `locked` while another session holds the
  device's lock.
- `unsupported` (nothing sent) when the device's `EXT_LIST` lacks `EXT_IDENTIFY` or stalls (stock firmware; read once
  per attach); `bad_request` for a tone output the device does not have; `rate_limited` (with `retry_in_ms`) within 2 s
  of the last request sent to that device, whoever asked. A refused request does not count against the 2 s.
- Addressing as section 1.3: an unknown serial is `not_found` (a serial known only to the inventory, past the 60 s grace,
  is also `not_found` here), one known but not attached `offline`. A device that refuses the write answers `device` with
  `status`.
- Recorded in the history with the caller in `by` (and `session`): a record with `source` `identify`, `value` the
  seconds and `channel` the tone's output, put on the hub's bus for the history to store.

## 5. Health

*State: built (P2HP).*

On attach the hub reads the device's `EXT_LIST`, then `EXT_IDENTIFY` (board, hardware revision, chip) and
`EXT_RESET_INFO` where listed; firmware without the extension channel is asked nothing more. Then it reads the status
every 2 s, the temperature every 30 s and the reset record (uptime, hang count) every 60 s. The status read is a poll on
the same poll group the consoles' meters use (`REQ_GET_STATUS` 9 at the length a console asks for, so one read whoever
asks), inside the poll budget; a DSPi Link poll runs at 1 Hz at least, so the hub polls at 1 Hz and reports every 2 s.
The temperature and the reset record are single reads on the device's queue, not polls: a 1 Hz poll for a 30 s reading
would be 30 times the reads. If the poll budget refuses the hub's poll, it reads the status itself every 2 s and asks
again later. The hub's reads pass a session's device lock (a read changes nothing the lock protects). It reads; it
never writes, and never clears the clip latch.

| `t` | Role | Request | Answer |
|---|---|---|---|
| `health.get` | viewer | `serial?` or `handle?` (both: they must name the same device) | `devices`: one record per online device (or the one asked) |

A record: `serial`, `handle`, `at` (the newest read), `cpu_pct` (the busier of the two cores), `clip_mask` (the sticky
latch: "clipped since someone cleared", one bit per channel), `temp_c` (to 0.01 °C), `uptime_s`, `hang_count`,
`unresponsive` (`true` once the device has given no answer for 10 s while attached, until its next answer),
`unresponsive_since` (while `unresponsive`), `reset` (`{reason, prev_uptime_s?}` from the attach, or from the read that
found the uptime gone backwards). A field the firmware does not give, or not yet read, is absent. Reset reasons:
`power_on`, `brownout`, `run_pin`, `watchdog`, `software`, `debugger`, `glitch`, `unknown`.
`uptime_s` and `hang_count` come from the reset-info read, every 60 s: they are as old as that read (`at` is the
newest read of any field), so two records 2 s apart usually carry the same uptime, and the uptime moves in 60 s
steps.

<!-- WP P3DOC -->
**For the health board** (*built: P3HB*, PLAN-P3 F7, OD-P3m). A record gains three fields, each absent when the device or
the hub has nothing for it; no cap changes (`health` as before), and a client that does not know them ignores them
(section 1.1):

| Field | Meaning | Read |
|---|---|---|
| `counters` | `{underruns, starvation}`: the output underrun counters (PDM ring, PDM DMA, S/PDIF) added up, and the stream starvation total, since the device started; the status counters a diagnostics bundle reads (`device/stats.json`). Absent until the first read, and on firmware without them | one more single read every 30 s on the device's queue, inside the poll budget, passing the lock like the other reads |
| `clip_per_min` | clip events (`0xE0` notifications) in the last 60 s, a whole number; absent on firmware that has only the sticky latch (no `EXT_NOTIFY_FEATURES` bit 1), where `clip_mask` is the only clip reading | counted from the notifications the hub already receives; no read |
| `last_change` | `{at, by, text}`: the device's newest history record that changed something (section 6), its time, who (`by`: a client's, key's, schedule's or rule's name) and one line for a person; a coalesced slider sweep counts once | from the history records on the hub's bus; no read |

The board's thresholds are the alarm rules (section 7, `alarm.rules.get`): the hub keeps no second set, and a client
colours a cell from the rules it reads. The hub still writes nothing here and never clears the clip latch; `Health` on
the bus carries the same three fields for the alarms.

Errors: `not_found` for a serial the hub does not know, `offline` for one it knows that is not attached, `no_device` for
an unknown handle, `bad_request` for a wrong type or a serial and handle of two devices.

On the hub's internal bus (for its other services, not on the wire): `Facts` once per attach and again when the uptime
went backwards between two reset reads (a reset not seen as a detach); `Health` every 2 s; `Unresponsive` once per
silence. An embedder turns the service on with `ServiceConfig::health` (`HealthConfig::default()` has the periods above).

## 6. Event history

*State: built (P2EV; the record kinds and the writer: P2EVW). Reference: `crates/dspi-hub/src/history.rs`.*

One shared log per device in `events/`, in the event log's own file format and lock rule ([event-log.md](event-log.md)),
so a diagnostics bundle and `event_log_export` read it unchanged. The writer is `EventStore`
(`crates/dspi-session/src/events/store.rs`, built in WP P2EVW). The hub writes what it sees: attach and detach,
notifications with the writing session and its name, clip events, temperature and counter steps, resets with their
reason, firmware changes, recalls, identify, schedule runs, alarms.

| `t` | Role | Request | Answer |
|---|---|---|---|
| `event.list` | viewer | `serial`, `after_id?` (default 0), `limit?` (default 100, at most 500) | `serial`, `records` (oldest first; with `after_id` 0 the newest `limit`), `more` (`true` if more follow `records`' last `id`), `newest_id` (the highest id the log holds; 0 when empty) |
| `event.watch` | viewer | `on` (bool), `serial?` (all devices when absent) | `on`, `serial` |
| `event.export` | admin | `serial`, `after_id?` | `serial`, `text` (the file format, whole records, at most about 48 KiB), `next_after_id` (`null` at the end), `newest_id` |

Event while watching: `event.added {v, hub, serial, record}`. A watch ends with the session. A device may be named
by `handle` instead of `serial` (section 1.3); a serial with no log on the hub is `not_found`.

**Ids can start again at 1** for a device: after `inventory.forget` removed its files, or after the total cap removed
every file of it. A client paging with an `after_id` above `newest_id` knows the numbering started
again and reads from 0. `after_id` 0 is a first read (the newest page); to page from a device's oldest record, take its
id from a first read with a large `limit`, or export.

A record held back for coalescing (a slider sweep, as the format does) is written out by any read of that device, or
when its second has passed. Bus messages the history could not take in time, and records it could not write, become one
`events_dropped` record per loss episode (`count` all of it) in the log of each device online (each device seen, when
none is). The record is written when the episode ends (the history has caught up), or once a minute while it lasts.

A device's `Facts` (its attach) record a `reset` when its uptime shows it restarted since the log last saw it, and a
`firmware` record when its version differs from the log's; the inventory's firmware change (P2IN) and the facts'
make one record, whichever comes first. One restart is one `reset` record, whether the facts or the next health
reading sees it first; a health reading that repeats the last uptime (section 5: it refreshes every 60 s) is no
restart.

Records are the file format's, with what phase 2 adds: the field **`by`** (the name of the client, key, schedule or rule
that caused it, next to `session`) and the kinds **`unresponsive`**, **`alarm`**, **`schedule`**, **`access`** and
**`events_dropped`** (`count` the records the hub could not write). An older reader reads the new kinds as `unknown`.

## 7. Alarms

*State: built (P2AL, `crates/dspi-hub/src/device_alarms.rs`).*

Device alarms beside the audio link's, on an alarm book of their own. Rules in `alarms.json` (admin). The service is
on with `ServiceConfig::alarms`.

| `t` | Role | Request | Answer |
|---|---|---|---|
| `alarm.list` | viewer | `serial?`, `state?` (`active`: raised and acknowledged; `cleared`; `all`, the default) | `alarms`: device and audio link alarms together, those on first (newest first), then the cleared (last cleared first), at most 500. With `serial`, the alarms about that device only |
| `alarm.ack` | control | `alarm` (its `id`) | `alarm` (as it is now); `not_found` for an alarm that is not on |
| `alarm.rules.get` | viewer | | `rules` |
| `alarm.rules.set` | admin | `rules` (whole or in part; a code left out keeps its rule, a field left out keeps its value) | `rules`; `bad_request` naming the field for a wrong type or range, and nothing changes |

`system.alarms` and `system.alarm.ack` ([audio-link.md](audio-link.md)) keep working for the audio link's alarms. Event to
every session: `alarm.changed {v, hub, alarm}` when one is raised, raised again (`count`), acknowledged or cleared,
`alarm` the whole object below (the same object goes to notification targets, section 8, and MQTT, section 13). The
audio link's alarms are announced the same way, read from its book once a second (`system.alarm` still goes out too).

An alarm in this family:

| Field | Meaning |
|---|---|
| `id` | a string, `"device:<n>"` or `"alink:<n>"`, unique for the hub's run |
| `source` | `device` or `alink` |
| `code` | below, or an audio link code |
| `subject`, `serial` | what it is about: the serial for a device's alarm (`serial` the same), the rule's id for `schedule_failed`, `hub` for `clock_unset` (no `serial` for those two, nor for the audio link's) |
| `severity` | `warning` or `critical` |
| `message` | one line for a person |
| `state` | `raised`, `acknowledged`, `cleared` |
| `raised_at`, `cleared_at`, `acknowledged_by` | times, and who acknowledged it (the session's client or principal name) |
| `count` | how often it was raised again while on |

| Code | Severity | Raised | Cleared | Default rule |
|---|---|---|---|---|
| `device_offline` | critical | a device seen in this run detached for `after_s` | attached again | on, `after_s` 30 |
| `unresponsive` | critical | attached, silent for `after_s` (the health poll's report, section 5, comes after 10 s) | its next reading, or detached | on, `after_s` 10 |
| `device_restarted` | warning | the device's facts show a watchdog, glitch or brown-out reset that happened during this hub run or in the last 10 minutes (one restart seen again on a re-attach is one alarm; a new one counts again) | acknowledged | on |
| `clip` | warning | firmware with clip events: `per_minute` events within 60 s (a storm); firmware without them: once, when the sticky latch is set ("clipped since cleared") | a storm: a minute under the threshold; the latch: when someone clears it (the hub never does, section 1.7) | on, `per_minute` 10 |
| `temperature` | critical | at or above `above_c` | 5 °C under it, or detached | on, `above_c` 70 |
| `cpu` | warning | at or above `above_pct` for `for_s` | a reading under it, or detached | on, `above_pct` 90, `for_s` 10 |
| `schedule_failed` | warning | a schedule's run with a failed action (`subject` the rule's id) | acknowledged | on |
| `clock_unset` | warning | the hub's clock reads before its build date (2026-11-15 in this build); schedules wait | the clock is set | on |
| `rollout_failed` | critical | a device's firmware rollout failed (section 21.3; `message` names the step) | a later rollout of the device ends `ok`, or the device attaches with the image's firmware (not after a failed verify: then only a rollout that ends `ok`); acknowledging keeps it on | always on: it has no rule, and `alarm.rules.*` do not list it (P3RO) |

Rules: `{"<code>": {"on": bool, ...thresholds}, "clip": {"on": true, "per_minute": 10, "mute_on_storm": false}, ...}`
with the fields the table names: `after_s` (1 to 86400), `per_minute` (1 to 1000), `above_c` (0 to 150),
`above_pct` (1 to 100), `for_s` (1 to 3600), `mute_on_storm` (bool). A code or field this hub does not know is ignored
(section 1.1). A rule turned off clears its alarms. **Mute on clip storm** is off by default; when on, a `clip` storm
mutes the device once per storm (the user mute, `vol.mute`), as one of the hub's own writes (section 1.7): the mute
passes the lock, the hub never unmutes, the alarm's message says the hub muted it, and the history records the write
with `by` `"Alarm rule: mute on clip storm"` (`source` `mute_on_storm`). Every alarm change is also a history record
(section 6) on the device it is about.

## 8. Notifications out

*State: built (P2WH, `crates/dspi-hub/src/outbound/`; HTTPS by SECOUT, `outbound/tls.rs`; MQTT publishing is
section 13).*

Opt-in targets in `outbound.json` (admin); none by default, and with none the hub runs no delivery thread. Deliveries
go from a queue of their own (64 per target, the oldest dropped and counted past that), never from a device's worker;
each target has its own sender, so a dead or slow target never holds the bus or another target up. A failed try (no
connection, a timeout, a status other than 2xx) is tried again after the target's backoff, 1 s doubling to 5 min and
reset by a success; after 10 tries the delivery is dropped and counted in `dropped`. Queued deliveries are not kept
across a restart; the targets are.

| `t` | Role | Request | Answer |
|---|---|---|---|
| `outbound.get` | admin | | `targets` (secrets as `set` or `not set`), each with `queued`, `dropped`, `last` (`{at, ok, status?, msg?}`, or `null` before the first try), `encrypted`, `trust` (https only: `webpki`, `ca` or `pin`) and `notice?` |
| `outbound.set` | admin | `targets` (the whole list, at most 16) | `targets`, and `secrets` `{<target id>: "<secret>"}` for the new ones: shown in this answer only |
| `outbound.test` | admin | `target` (its `id`) | `delivered` (bool), `status?`, `msg?` (one delivery of kind `test`, now; recorded as its `last`) |

A target: `id` (1 to 32 of `A-Z a-z 0-9 _ -`; left out, the hub gives `t1`, `t2`, ...), `name` (at most 64
characters; the id when left out), `kind` (`webhook`: a JSON POST; `push`: an ntfy-style `text/plain` POST of
`text`), `url` (`https://`, or `http://` with `plaintext`; at most 2048 bytes, no credentials in it), `enabled`
(default `true`; a disabled target's queue is emptied), `kinds` (any of `alarm.raised`, `alarm.cleared`, `restart`,
`schedule.ran`, `schedule.failed`; every kind when left out), the transport fields below, and `secret`:

| `secret` in `outbound.set` | Means |
|---|---|
| `"new"` | the hub makes one (32 random bytes as 64 hex characters) and shows it in `secrets`, once |
| `"set"` or left out | keep this id's secret, if it has one (so `outbound.get`'s list can be sent back as it is) |
| `"not set"` or `null` | no secret: no signature |
| anything else | `bad_request`: secrets are made by the hub, never typed |

A refusal is `bad_request` with `msg` naming the field and `target` the entry's index; nothing is kept then. A target
whose `id` stays keeps its queue, counts and backoff, except that a changed `kind` drops what is queued (counted in
`dropped`: a body made for one kind is never sent as the other), and a changed `url`, `plaintext`, `ca_pem` or
`pin_sha256` starts the backoff again, so a corrected URL is tried at once.

**Transport** (SECOUT, PLAN-SEC 4.8, OD-SECe/f). `https://` is the norm: TLS 1.2 or 1.3, SNI from the host (an IP
literal is checked against the certificate's IP addresses), ALPN `http/1.1`, no redirects. What it trusts:

| Field | Trust |
|---|---|
| neither | the public CAs (`webpki-roots`, the same on every hub platform), and the host name |
| `ca_pem` | the CA or self-signed certificate(s) pasted here instead (PEM, at most 16, 32 KiB), and the host name. A receiver whose certificate is exactly one of them (a self-signed one, also when OpenSSL 3's `req -x509` marked it `CA:TRUE`) is accepted with its host name checked, not its dates; any other certificate must chain to them, dates checked. A CA certificate presented as the receiver's own that is not given here is refused with a reason saying so |
| `pin_sha256` | the receiver's key: `sha256//<base64>` of its SubjectPublicKeyInfo, as curl's `--pinnedpubkey` (64 hex characters are taken too and stored in that form); no CA or host name check. Get it with `openssl x509 -in cert.pem -pubkey -noout \| openssl pkey -pubin -outform der \| openssl dgst -sha256 -binary \| base64` |

`ca_pem` and `pin_sha256` are each a trust of their own: give one. Dates are checked with the CAs, not with a pin.

`http://` needs `"plaintext": true` and goes only to a private (RFC 1918, IPv6 `fc00::/7`), loopback or link-local
address. That is checked on the resolved address of every send, so a DNS change cannot send a body across the internet
in clear: other addresses are left out, and with none left the try fails with `msg` saying why (nothing was sent). An
`http://` IP literal outside those ranges is refused by `outbound.set` at once. Such a target is `"encrypted": false`
in `outbound.get`. A receiver on a public address (an IPv6 global address on the LAN too) needs `https://`.
`100.64.0.0/10` (carrier-grade NAT, which Tailscale uses) is not private here, so a plaintext webhook to a Tailscale
address is refused: use `https://` (a `tailscale cert` certificate) or `pin_sha256`.

The HMAC signature below is sent over both: it authenticates the hub to the receiver, whatever the transport.

**Targets from before TLS.** An `http://` target written by an earlier hub (no `plaintext` field) loads as
`"plaintext": true`, the file is written back with the flag, a warning goes to the log, and `outbound.get` shows a
`notice` on it until the next `outbound.set`. One on a public address is kept but each try is refused, as above:
change it to `https://`.

Which bus events are sent: an alarm raised (`alarm.raised`; a `device_restarted` one is `restart` first) or cleared
(`alarm.cleared`; an acknowledgement is not sent), and a schedule's run (`schedule.ran`; one with a failed action is
`schedule.failed` first). A target gets each event once, as the first of its kinds it picked.

A webhook's body: `{"v": 1, "hub": "<id>", "kind": "alarm.raised", "serial": "...", "at": "...", "text": "...",
"alarm": {...}}` (`serial` and `alarm` where they apply; `alarm` is section 7's object with ISO times). A schedule's
event carries `"schedule": {"rule", "name", "by"?, "results", "results_total"}` (the first 64 results) instead of
`alarm`. `text` is at most 1024 characters. Headers `X-DSPi-Timestamp: <unix seconds>` (the time of that try) and, for
a target with a secret, `X-DSPi-Signature: sha256=<hex HMAC-SHA256(secret, timestamp + "." + body)>`, keyed with the
secret's text as shown (its 64 characters as bytes). A push target gets the same headers. A receiver checks the
signature and that the timestamp is recent. The hub reads only the status line of the reply (at most 8 KiB of the
decrypted stream; no line end within them is a failure); a 2xx is delivered. Timeouts: 5 s to connect, 10 s in all,
the name lookup and the TLS handshake included.

## 9. Parameters

*State: built (P2PA). `crates/dspi-hub/src/api/` (feature `api` of `dspi-hub`; the binary that serves the
API turns it on), tests `crates/dspi-hub/tests/wp_p2pa_api.rs`.*

Read and write any registry parameter by path ([parameter-api.md](parameter-api.md)). A write runs through the same core
code as a console's (`write_param`, every typed setter's guard), inside the hub, as a child session of the caller: same
role, same client and name (`by`), its own session id from the hub's remote range. The limits, the installer lock, the
device lock and the voltage rule then judge it as they judge any client, and other sessions see it as a foreign write.
A read is the device's own answer (`dspi_session::Session::read`, as that child session).

| `t` | Role | Request | Answer |
|---|---|---|---|
| `param.list` | viewer | `serial` | `serial`, `params`: one row per registry path the device offers |
| `param.get` | viewer | `serial`, `path`, `idx?` | `serial`, `path`, `idx`, `value` (and `choice`, or `value_b64`) |
| `param.get_many` | viewer | `serial`, `items`: `[{path, idx?}]`, at most 64 | `serial`, `values`: `[{path, idx, ok, value?, choice?, code?, msg?}]`, in order |
| `param.set` | control | `serial`, `path`, `idx?`, `value`, `confirm?` | `serial`, `path`, `idx`, `value`: the value read back from the device |

A `param.list` row (the web binding's `describe` row): `path`, `label`, `plain`, `kind`, `unit`, `min`, `max`
(`choices: [{value, label}]` for a choice, `max` for text), `readable`, `writable`, `routed` (`false`: not written by
the API; the row's `use` names the message or call that writes it), `gated` (written only with `"confirm": true` by an
admin), `indices` (how many), `layout` (the index ranges on this device, e.g. `[{"name": "output", "count": 9}]`;
`count` is `null` where only the device knows), `hazard` (`none`, `audible`, `deferred`, `flash`, `reconfig`,
`irreversible`). A path the device's firmware lacks (a feature it did not answer, an RP2350-only path on an RP2040)
is not listed.

- `bad_request`: a path not in the registry, the wrong number of indices or one the device does not have, a value of the
  wrong kind or outside the range (with `min` and `max`), a path the API does not write (`use` names what does, where
  the core has a call for it), or raw bytes. Not written by the API at all: `bytes` and `status` kinds, every
  extension-opcode path, and `meters.clear` (the clip latch is shared by every console: the hub never clears it).
- Gated paths (`preset.save`, `preset.delete`, `dev.save`, `dev.save.io`) are written only by an admin with
  `"confirm": true`: the API's explicit confirmation (the hub's core mints and spends the confirmation token itself).
  Otherwise `confirmation_required` (admin) or `denied`.
- On a locked device, control may set `vol.master`, `vol.user`, `vol.mute`, `out.mute` and load a preset only
  (`installer_lock` otherwise). Past a limit: `limit_exceeded` with `limit`, nothing sent.
- **The child session acts for its caller** in two things only: a device lock (spec 7.7) the caller holds lets its
  API writes through (another session's lock answers `locked`), and a PIN unlock (`limits.unlock`) the caller made
  is the child's too. Both end when the caller's session does; a child never keeps them. The role, the limits and
  the voltage rule judge the child as any session.
- A guarded apply pending on the device (another session's) refuses `param.set`, `preset.load`, `preset.save` and
  `snapshot.recall` with `conflict`; reads go on.
- Rate: one request at a time per connection (each is answered before the next is read), each on the caller's child
  session, which the in-flight limit (`hello.limits.max_inflight`) bounds as any session's.
- The hub keeps one core per (caller, device), opened on first use (a full attach: the probe and an image read) and
  closed after 60 s unused, when the caller's session ends or when the device goes; at most 32 at once, then `busy`.

## 10. Presets

*State: built (P2PA).*

| `t` | Role | Request | Answer |
|---|---|---|---|
| `preset.list` | viewer | `serial` | `serial`, `slots`: `[{slot, name, used}]`, `active` (the loaded slot or `null`), `startup` (`{mode: "slot", slot}` or `{mode: "last_active"}`) |
| `preset.load` | control | `serial`, `slot` or `name` | `serial`, `slot`, `name` |
| `preset.save` | admin | `serial`, `slot`, `name?`, `confirm: true` | `serial`, `slot`, `name` |

Slots are 0 to 9. A name matches exactly (case kept) among the used slots; two slots with the name are `conflict`, none
is `not_found`. Names are read from the device for each used slot. A preset load is held after the fact on a device
with a limits profile, as P1HL does (`limits.held` follows). `preset.save` without `"confirm": true` is
`confirmation_required`; `name` (at most 31 bytes) names the slot after the save. The audio link's scenes stay
`system.scene.apply {scene}` ([audio-link.md](audio-link.md)).

## 11. Snapshots on the hub

*State: draft (P2SS: the store and its messages; P2PA: `snapshot.recall`).*

The hub's shared copy of each device's snapshots, in `snapshots/`, in the client store's own format (one folder per
serial, `index.json`, one `<id>.dspipreset` per snapshot; `crates/dspi-session/src/snapshot_store.rs`). **The hub's copy
wins** (OD-P1b): a client that holds the same id with different content shows the hub's and keeps its own as a local
copy; nothing is overwritten silently.

### 11.1 The store (P2SS)

`snapshot` is a snapshot's store id (as `alarm` is an alarm's in `alarm.ack`); `id` is the request id, as on every
message. An entry is the index's own object: `{id, name, note, serial, savedMs, platform, inputChannels,
outputChannels, wireFormat}`.

| `t` | Role | Request | Answer |
|---|---|---|---|
| `snapshot.list` | viewer | `serial` | `serial`, `snapshots`: entries, oldest first (the order of the client store's `index.json`) |
| `snapshot.get` | viewer | `snapshot` | `snapshot`: the entry, `document` (the document's text, compact JSON) |
| `snapshot.put` | control | `serial`, `document` (its text, or the JSON object) | `snapshot`: the entry, `created` (`false` when the hub already held this snapshot with the same content) |
| `snapshot.rename` | control | `snapshot`, `name` | `snapshot`: the entry |
| `snapshot.note` | control | `snapshot`, `note` | `snapshot`: the entry |
| `snapshot.delete` | admin | `snapshot` | `serial`, `snapshot`: the id |

- `document` is filed as a client's store files it: `meta.kind = "snapshot"` and a `snapshot` block naming the same
  serial as the message and the client's id. A document that is not such a snapshot, or names another device's serial,
  is `bad_request`.
- **Two size caps (P2SSFIX).** On the wire, a document is at most **61 440 bytes** (`max_frame` 65 536 less 4 KiB for the
  message around it) in its **compact** JSON form, the form `snapshot.get` answers; `snapshot.put` and `snapshot.get` of
  a larger one are `conflict`, the message naming the limit and the document's size ("a snapshot document is at most
  61440 bytes on the DSPi Link (compact JSON); this one is N"). There is no chunking. A client measures the compact form
  before it sends and **refuses locally with the same message**, sending nothing (P2CC1). What the hub files is the
  document pretty-printed (the client store's own form, about 1.6 times the compact size), at most 256 KiB; rename and
  note are measured on that form too. A demo-device snapshot is about 50 KB compact and 83 KB filed.
- The id is the client's, taken from the document and kept (capture-time ids are unique, so one snapshot keeps one id
  on both sides); the hub never makes one. The name and note are the document's; change them with `snapshot.rename`
  and `snapshot.note`. An id the hub holds with different content, or for another device, is `conflict`: the hub's
  copy stays, never overwritten.
- Names (at most 256 characters) and notes (at most 4096) take any Unicode. A cap reached (200 per device, 64 MiB in
  all) is `conflict`.
- Event to every session: `snapshot.changed {v, hub, serial, snapshot, change}` (`change`: `put`, `rename`, `note`,
  `delete`; `snapshot` the id).

### 11.2 Recall on the hub (P2PA)

*State: built (P2PA). The store's messages (11.1) are registered as the `snapshot` family on a hub with a data folder;
each change is broadcast as `snapshot.changed` right after the answer is made.*

| `t` | Role | Request | Answer |
|---|---|---|---|
| `snapshot.recall` | control | `serial`, `snapshot` or `name`, `guard?` (bool), `timeout_ms?` | `serial`, `snapshot`, `name`, `image` (whether the image was written), `lines` (what changed), `notes` (what was not written, and why), `guard` (its id, with `guard: true`) |

The hub writes the image and the settings outside it, in a recall's order, as the caller's child session. On a device
with a limits profile or a lock, control's recall is `image_refused` (nothing written); an admin's is written and then
held. With `guard: true` a guarded apply ([hub.md](hub.md), Guarded apply) begins first, as the caller (`timeout_ms`
as `apply.begin`'s, 30 s by default); confirm it with `apply.confirm`, or it reverts. A recall that fails after its
guard began is reverted at once. A device that is not attached gets nothing (`offline`). `name` matches exactly among
the device's snapshots (two are `conflict`); a snapshot of another device is `not_found`. Recorded in the history as a
`recall` with its `lines`, the caller's name in `by`.

A document reaches the hub in one DSPi Link text frame (at most 64 KiB, `hello.limits.max_frame`): send it compact,
or as the JSON object. The demo device's snapshot runs to about 80 KB pretty-printed, about 50 KB compact.

## 12. Schedules

*State: built (P2SC). `crates/dspi-hub/src/schedule.rs` (on with `ServiceConfig::schedule`; the runs need the
parameter API's executor, feature `api`), tests `crates/dspi-hub/src/schedule/tests_wp_p2sc.rs` (the clock rules)
and `crates/dspi-hub/tests/wp_p2sc_schedule.rs` (through the server).*

Rules in `schedule.json` (admin). A rule runs as the principal `Schedule: <name>` with the admin role, so the limits judge
its writes (an image held after the fact, as for any admin).

| `t` | Role | Request | Answer |
|---|---|---|---|
| `schedule.list` | viewer | | `rules`, each with `next_run` (UTC, `null` when none within a week or while the clock is unset) and `last` (`{at, ok}`, with `skipped: true` for a time that was skipped; `null` before the first); `zone` (the time zone the rules are read in), `local_time` (the hub's local time now, `YYYY-MM-DDTHH:MM`), `clock_unset` |
| `schedule.set` | admin | `rule` (a new one without `id`; with `id`, that rule replaced whole) | `rule`, as listed |
| `schedule.delete` | admin | `rule` (its `id`) | `rule` (the id) |
| `schedule.run_now` | admin | `rule` (its `id`) | `rule`, `name`, `at`, `by`, `results` (the run's, as `schedule.ran`); `conflict` while the clock is unset |

A rule:

```json
{"id": "r1", "name": "Doors open", "enabled": true,
 "when": {"days": ["mon", "tue", "wed", "thu", "fri"], "time": "18:30"},
 "actions": [{"t": "preset.load", "name": "Live"},
             {"t": "param.set", "path": "vol.master", "value": -12.0}],
 "targets": ["E46058388B1A2E2C", "E6614103E7A1B2C3"]}
```

- `id` is the hub's (`r1`, `r2`, ...; never reused). `name`: 1 to 48 characters. `enabled` (default `true`): a rule
  that is off keeps its place and never runs on its schedule; `run_now` still runs it.
- `when` is `{days, time}` (local `HH:MM` on the given days, `"mon"` to `"sun"`) or `{once: "2026-12-24T17:00"}` (a
  local date and time). A rule whose `when` is changed may run today at its new time.
- `actions` (1 to 16), each a message of this page with its device field left out: `preset.load {slot or name}`,
  `snapshot.recall {snapshot or name}` (no guard), `system.scene.apply {scene}` (no target needed), `param.set {path,
  idx?, value}` for `vol.master`, `vol.user`, `vol.mute` and `out.mute` only. Other fields are dropped. A value outside
  the registry's range is reported by the run (`bad_request`), as the message itself answers it.
- `targets`: at most 16 serials, each once (16 actions on 16 devices answer 256 rows, one frame); `group:<id>` entries
  are taken too (any well-formed id) and expanded to the members when the rule runs (section 19.3, built: P3GR). A rule with any device action needs a target.
- More than 256 rules is `conflict`; a malformed rule `bad_request`; an unknown `id` `not_found`.

**The clock** (OD-P2h): local time in the hub machine's time zone (its zone database: `TZ`, `/etc/localtime`). At start,
and when the clock first reads a believable time, a rule missed by up to 5 minutes runs; older ones do not. A forward jump
of the clock (more wall time than the hub's own steady time passed) skips what it passed and records it (`last.skipped`,
and a `schedule` line "skipped" in each target's history); a backward jump never runs a rule twice in a day (a rule runs at
most once per local date). A local time that does not exist (spring forward) runs at the first minute after the gap (02:30
on a 02:00-to-03:00 day runs at 03:00); one that happens twice (fall back) runs once, at the first. While the clock reads
before the build date nothing runs (`run_now` answers `conflict`) and `schedule.list` says `clock_unset: true`; the
`clock_unset` alarm itself is the alarms family's (section 7, P2AL), raised and cleared there against the same build date. The hub looks at the clock at least every 30 s, so a jump is seen within that.

Each run answers per action and target: `results: [{serial, action, ok, code?, msg?}]`, where `action` is a string
naming the action by its position in `actions` and its `t` (`"0:preset.load"`), in the rule's order (each action on
every target, then the next action). An offline target fails at once (`offline`; a serial the hub has never seen is
`not_found`) and is not tried later; a target with a pending guarded apply is `conflict`; a limit's refusal is
`limit_exceeded`, nothing sent. A scene's row is one per unit it wrote (`ok` when every write of it went through), or one
row with `serial` `""` when the scene was refused as a whole (`not_found`: no such scene). `msg` is at most 80
characters. Event to every session: `schedule.ran {v, hub, rule, name, at, by?, results}` (`rule` the rule's id, since
events carry no `id`; `by` the admin for `run_now`, `null` on its schedule). Each run is recorded in the history (one
`schedule` record per device, `by` the admin or `Schedule: <name>`); a failure raises `schedule_failed`.

## 13. MQTT and Home Assistant

*State: 13.1 built (P2MQ, `crates/dspi-hub/src/mqtt/service.rs`; TLS: SECOUT, wired by MQWIRE); 13.2 built (P2HA,
`crates/dspi-hub/src/mqtt/ha.rs`; tests `crates/dspi-hub/tests/wp_p2ha_home_assistant.rs`).*

### 13.1 The broker (P2MQ)

MQTT 3.1.1, off by default, settings in `mqtt.json` (0600), over TLS or plain TCP (the `tls` object below).

| `t` | Role | Request | Answer |
|---|---|---|---|
| `mqtt.get` | admin | | `settings` (the password as `set` or `not set`) |
| `mqtt.set` | admin | `settings` (in part; `password` a string sets it, `null` clears it) | `settings` |
| `mqtt.status` | admin | | `state` (`off`, `connecting`, `connected`, `failed`), `msg?` (e.g. a refused password), `since`, `queued`, `dropped`, `encrypted` |

Settings: `enabled`, `broker` (address), `port` (8883 with TLS, 1883 without), `client_id`, `username`,
`password`, `base_topic` (default `dspi/<hub id>`), `tls` (below), `ha` (section 13.2).

`tls` (SECOUT, PLAN-SEC 4.8; `crates/dspi-hub/src/mqtt/client.rs`, `MqttTls`):

| Field | Means |
|---|---|
| `mode` | `on`: TLS 1.2 or 1.3, the broker verified before a byte of CONNECT is sent. `off` (default): plain TCP |
| `ca_pem` | with `on`: the CA or self-signed certificate(s) to trust instead of the public CAs; the host name is checked. A broker presenting exactly one of them (self-signed, `CA:TRUE` or not) is accepted, as for webhooks (section 8) |
| `pin_sha256` | with `on`: the broker's key, as for webhooks (section 8); replaces the CA and host name check. One of `ca_pem` and `pin_sha256` at most |
| `client_cert_pem`, `client_key_pem` | with `on`: a client certificate and its key (PEM; PKCS#8, SEC1 or PKCS#1). The key is kept in the 0600 `mqtt.json`, shown as `set` or `not set` (sending `"set"` back keeps it) and never in the diagnostics bundle |
| `client_cert_file`, `client_key_file` | instead: paths to PEM files; the key file must not be readable by group or others (0600) |
| `allow_plaintext_password` | with `off`: send the password to a broker that is not on loopback too |

Without TLS the password goes only to a loopback broker (`localhost`, `127.0.0.1`, `::1`) unless
`allow_plaintext_password` is set; otherwise the connection is not made at all and `mqtt.status`'s `msg` says why. A
broker with no password may be plain anywhere, shown as `encrypted: false`. A `mqtt.json` written before TLS (no
`tls`) with a broker that is not on loopback loads with `allow_plaintext_password: true` and a one-time notice, so it
keeps working (OD-SECo); the hub writes the field back, so the notice comes once: it is in the hub's log and, for
that run, in `mqtt.get`'s and `mqtt.status`'s `notice` (until `tls` is set). `mqtt.status`'s `encrypted` is true while
the connection is TLS. Without a `port` (or with `null`) the port is 8883 with TLS and 1883 without; `mqtt.get`
shows the port in use, so sending its `settings` back keeps that port fixed.

- `mqtt.get` shows the values in use: `client_id` and `base_topic` are the defaults when not set (`client_id`:
  `dspi` and the first 19 letters and digits of the hub's id). In `mqtt.set`, `null` sets either back to its default.
- `password` in `mqtt.set`: a string sets it; `null` or `"not set"` clears it; `"set"` keeps it, so `mqtt.get`'s
  `settings` can be sent back as they are. A password needs a `username` (MQTT 3.1.1).
- `enabled: true` needs a `broker`. A refusal is `bad_request` with `msg` naming the field; nothing is changed then.
  A field not listed here is ignored. `tls`, unlike the other settings, is **replaced whole** by `mqtt.set`: a
  field left out of it becomes none (no CA, no pin, no client certificate, `mode` off, `allow_plaintext_password`
  false); only `client_key_pem: "set"` keeps the stored key. So send back every `tls` field you want kept.
  `mqtt.get` shows `ca_pem`, `pin_sha256`, `client_cert_pem` and the file paths in full (none of them is a secret);
  only `client_key_pem` is shown as `set` or `not set`.
- Limits: `broker` 253 bytes of `A-Z a-z 0-9 . - _ :`; `client_id` 1 to 64 of `A-Z a-z 0-9 . - _`; `username` and
  `password` 1 to 256 bytes, no control characters; `base_topic` a topic name of at most 128 bytes, no wildcard, no
  empty level, not starting with `$`.
- A change of `ha` alone does not reconnect; any other change ends the connection (`offline`, below) and starts
  a new one. `mqtt.set` answers once the old connection has ended (at most a few seconds).
- `state`: `connecting` while trying (again), with `msg` the last reason; `failed` when the broker refused the
  connection (a wrong password, a refused client id), with its reason in `msg`; the hub keeps trying, with a backoff
  of 1 s doubling to 60 s. `since` is when `state` last changed (ISO-8601); `queued` and `dropped` are the publishes
  waiting while the broker is away (at most 256) and those dropped past that, oldest first.

Topics: `<base>/status` is `online` (retained, QoS 1, on every connection) and the will `offline` (retained),
which the broker publishes when the hub goes away unannounced; a clean stop or reconfiguration publishes `offline`
(retained) itself. Each alarm and history record the hub publishes goes to `<base>/<serial>/event` as JSON (an alarm
as section 7's object with `"kind": "alarm"`, QoS 1, its every change: raised, acknowledged, cleared; a record as
`{"kind": "event", "serial", "record": {...}}`, QoS 0). Both carry `v` and `hub`; an alarm also `at`, the time of this
change. An alarm about no one device (the audio link's) goes to `<base>/hub/event`. Nothing is retained but the
status. Without Home Assistant the hub subscribes to nothing: nothing that arrives over MQTT reaches a device
(section 13.2's `set` topics act as an API key's principal).

### 13.2 Home Assistant (P2HA)

`ha: {"enabled": bool, "key": <cid>}` in `mqtt.set`: the API key (section 14) whose principal `set` topics act as.
Turned on without `key` (or with `null`), the hub mints one named `Home Assistant` with the control role and answers
its `cid` in `settings.ha.key` (the key itself is never shown: Home Assistant needs only its principal; revoke or
change its role like any key). A `key` must be an API key the hub has (`bad_request` for a console's `cid` or an
unknown one); a hub with no network store has no keys (`unsupported`). Changing `ha` does not reconnect; turning it
off removes every entity (below) and ignores `set` messages from then on.

Per device the hub sees (online, or offline within its grace), retained discovery under
`homeassistant/<component>/<hub>_<serial>_<entity>/config` (letters, digits, `-`, `_` of the hub's id and the
serial), each with a device block (`identifiers: ["dspi_<serial>"]`, the device's name, `serial_number`,
`sw_version` the firmware, `model` the board and hardware revision from the facts or else the platform,
`manufacturer: "DSPi"`) and `availability`: `<base>/status` (section 13.1) **and** `<base>/<serial>/availability`
(`online`/`offline`, retained), `availability_mode: all`.

| Entity | Component | State (`<base>/<serial>/<entity>/state`, retained) | `set` (`<base>/<serial>/<entity>/set`) |
|---|---|---|---|
| `master_volume` | `number`, `min`/`max` from the registry (`vol.master`: -128 to 0 dB), step 0.5 | dB, from the master volume's notifications | a number in that range: `param.set vol.master` |
| `mute` | `switch` | `ON`/`OFF`, from the user mute's notifications | `ON`/`OFF`: `param.set vol.mute` |
| `preset` | `select`, `options` the used slots' names (an unnamed one `Slot <n>`, a repeated name ` (slot <n>)`); left out while no slot is used | the loaded preset's option, from `PRESET_LOADED` | an option: `preset.load {slot}` |
| `identify` | `button` | | `PRESS`: `device.identify` (10 s, no tone) |
| `temperature` | `sensor` (°C) | from health | |
| `cpu` | `sensor` (%) | from health | |
| `clip` | `binary_sensor` (`problem`) | `ON` while the clip latch is set (the hub never clears it) | |

- **Who writes.** Every `set` is sent through the control plane, as a WebSocket or REST request is, on the key's
  principal session (a remote-range session id, the key's role, attributed to the key's name): the role, limits
  (section 9's refusals), the installer lock (which lets volume, mute and preset through), the device lock, a
  pending guarded apply (`conflict`) and the read-only rule judge it. A refused set changes nothing, and the hub
  publishes the state again as it is, so Home Assistant's control moves back; the refusal is in the hub's log. A
  Viewer-role key, or a revoked one, writes nothing. The hub reads the volume, the mute and the preset names as the
  same principal when a device comes online, after a preset is loaded and when its settings image changed.
- **What is taken.** A `set` payload of at most 64 bytes of text, exactly as the table says; anything else, a
  retained `set` (a write is never replayed), a topic that is not `<base>/<serial>/<entity>/set` of a settable entity
  of a device with entities, is dropped. At most 64 messages wait for the hub (the oldest go); a newer set of the
  same entity of a device replaces one still waiting.
- **Removal.** A device that is not attached is unavailable (`<base>/<serial>/availability` `offline`), also past
  the hub's 60 s grace; its entities stay. Only when an Admin forgets it (`inventory.forget`, section 3) are its
  configs, states and availability cleared with empty retained payloads, so Home Assistant removes it; it is
  discovered again if it comes back.
- **After a restart.** When Home Assistant starts and on every connection the hub lists the devices it knows from
  the inventory (`inventory.list` as the key's principal; a hub without the inventory uses its device list) and
  publishes each one, `offline` when it is not attached now. So a device unplugged while the hub was down is shown
  unavailable, not with the `online` the broker kept from before. A device the hub never knew (another hub's store)
  is left as the broker has it.
- On every connection the hub publishes every entity again. A clean stop or a lost hub leaves the entities
  retained and unavailable (`<base>/status` is `offline`).

## 14. REST and API keys

*State: built (P2RS). `crates/dspi-hub/src/rest.rs`, the `/api/v1/` route and body read in `server.rs`, the client
kind in `auth.rs` and `store.rs`; tests `crates/dspi-hub/tests/wp_p2rs_rest.rs`. **TLS: SECSRV** (PLAN-SEC): the
mirror is read and answered over the server's stream seam, so WP SECSRV serves it over TLS on the same port, adds the
`Host` check on every route and gives `plaintext_ok` its effect (built; section 2.1).*

### 14.1 API keys

An API key is a paired client of kind `api`, minted without a PIN: the same table, the same cap of 64 shared with the
consoles, the same `auth.revoke` and `auth.set_role`. `auth.list` rows gain `kind` (`console` or `api`) and
`plaintext_ok`; `online` is also true for a key whose REST session is open.

| `t` | Role | Request | Answer |
|---|---|---|---|
| `auth.mint` | admin | `name`, `role` (never above the minting session's), `plaintext_ok?` (bool, default `false`) | `v`, `cid`, `name` (as cleaned), `role`, `kind: "api"`, `plaintext_ok`, `key`: shown in this answer only |

- The key is 32 bytes from the hub's CSPRNG, base64url (43 characters), made by the hub, never typed. The hub keeps
  only its SHA-256 (`hub-store.json`, beside the consoles' token hashes) and compares every stored hash in constant
  time. A key is for REST (and MQTT's Home Assistant `set` topics) only: `auth.token` with a key on DSPi Link is
  `unauthenticated`, and a console's token is no REST bearer (`401`).
- The 65th client of either kind is refused: `auth.mint` answers `conflict`, a PIN pairing `busy` (spec 7.3). Revoke
  one first.
- `plaintext_ok` (OD-SECk; WP SECDOC): on a hub with `--tls on`, a key without it is refused over plaintext with
  `426` `tls_required`, and one minted with it works there too (for a legacy script on a trusted network). With
  `require` no key works over plaintext; with upstream compatibility or `off` every key does (section 2.1). Keys minted
  before TLS have it false.
- `name` and `role` are required (`bad_request`); a role above the caller's is `denied`. Rotation: mint a new key,
  then `auth.revoke` the old one.
- In the embedding app: `AuthStore::mint_api_key` (`NetworkControl::auth`). `uconsole hub api-key new|list|revoke`
  is P2TM's (section 16).

### 14.2 The REST mirror

Off by default (`NetworkConfig::rest`; `uconsole hub --rest`, P2TM); on the DSPi Link port (OD-P2p). While off,
`/api/v1/` answers `404` and `hello.caps` has no `rest`.

- `Authorization: Bearer <key>` on every request (an API key; a console's token is `401`), even on a hub with open access.
  A missing, wrong or revoked key is `401` with `WWW-Authenticate: Bearer`. Five wrong keys from one address within a
  minute shut it out for a minute (`429`, `Retry-After: 60`), as for `auth.token`.
- **Any request with an `Origin` header is `403`**, whatever its value, before the key is looked at. The web console
  uses DSPi Link through its own binding; no browser page needs REST, and there is no CORS (no
  `Access-Control-Allow-*` header is ever sent).
- Bodies: JSON objects, `Content-Length` only, at most 64 KiB (`ServerLimits::rest_max_body`), all of it within 3 s
  of the head (`rest_body_timeout`). Over the cap: `413`, unread. No length on a `PUT`, or any `Transfer-Encoding`:
  `411`. A length that is not one (signs, lists, two headers): `400`. Too slow: `408`. Only the length's bytes are
  read; a body longer than its length is not taken for a next request: the connection closes after the answer, as it
  does after every `PUT` and `POST`. A `POST` without a body is fine. A `GET` may be kept alive.
- An answer's body is the WebSocket answer without `t` and `id`, plus `v` and `hub`. An error's body is
  `{"v": 1, "hub": "<id>", "code": "<code>", "msg": "..."}` with the codes of section 1.6 and the message's own
  fields (`limit`, ...); errors of the mirror itself use the same shape.
- Rate-limited per key: 10 requests a second on average, 20 at once (`rest_rate`, `rest_burst`); past it `429` with
  `Retry-After` (seconds) and `retry_in_ms`.
- **Who writes.** Each key in use has one session of its own, a principal from the hub's remote range with the key's
  client id and name (`by` in the history, `origin` for other sessions), judged by its role, the limits, the lock and
  the voltage rule as any client. It stays open for 60 s after the key's last request (`rest_idle`), so the parameter
  API's core for it stays warm, and closes at once when the key is revoked. A key's requests run one at a time, as a
  WebSocket's do; one that waits past the response time is `503` `busy`.
- A family the hub does not run (its service off) answers `501` with `unknown_type`.
- **A guarded recall** (`{"guard": true}`) belongs to the key's session: only **the same key** can confirm or cancel
  it (`.../apply/confirm`, `.../apply/cancel`), and it must do so within both the guard's timeout (`timeout_ms`, 30 s
  by default) and `rest_idle` (60 s) of its last request. Otherwise it reverts: at its timeout, or when the key's idle
  session closes.

The endpoints (PLAN-P2 I1; more may be added within v1, each onto an existing message). Path segments and query values
are percent-decoded (`device%3A3`).

| Method and path | Message |
|---|---|
| `GET /api/v1/devices?serial=&offset=<n>&limit=<n>` | `inventory.list` (paged and filtered as it is) |
| `GET /api/v1/devices/{serial}/params/{path}?idx=0,1` | `param.get` |
| `PUT /api/v1/devices/{serial}/params/{path}?idx=0,1` body `{"value": ..., "confirm"?: true}` | `param.set` (only `value` and `confirm` are taken from the body) |
| `POST /api/v1/devices/{serial}/presets/{n}/load` | `preset.load {slot: n}` |
| `POST /api/v1/devices/{serial}/snapshots/{id}/recall` body `{"guard"?: true, "timeout_ms"?: n}` | `snapshot.recall` |
| `POST /api/v1/devices/{serial}/apply/confirm` | `apply.confirm {handle}` (the attached device's; `409` `offline` otherwise) |
| `POST /api/v1/devices/{serial}/apply/cancel` | `apply.cancel {handle}` |
| `GET /api/v1/devices/{serial}/events?after=<id>&limit=<n>` | `event.list` (`after` is `after_id`) |
| `GET /api/v1/alarms?serial=&state=` | `alarm.list` (P2AL's; `501` until it is built) |
| `POST /api/v1/alarms/{id}/ack` | `alarm.ack` (P2AL's) |
| `GET /api/v1/health?serial=` | `health.get` |

<!-- WP P3DOC --> **The fleet's endpoints** (*draft: P3RS*, PLAN-P3 OD-P3l), within v1 and additive, each onto a message of
sections 19 to 21 with its role, checks, answer and codes. `{id}` is a group's id (`g1`), `{run}` a bulk apply's run,
`{rollout}` a rollout's id:

| Method and path | Message |
|---|---|
| `GET /api/v1/groups` | `group.list` |
| `GET /api/v1/groups/{id}` | `group.list {group: id}` (`404` `not_found` for an unknown group) |
| `PUT /api/v1/groups/{id}/params/{path}?idx=0,1` body `{"value": ..., "offsets"?: {"<serial>": <dB>}}` | `group.param.set {group: id, ...}` (only `value` and `offsets` are taken from the body) |
| `POST /api/v1/groups/{id}/presets/{n}/load` body `{"map"?: {"<serial>": <slot or name>}}` | `group.preset.load {group: id, slot: n, map?}` |
| `POST /api/v1/groups/{id}/apply` body `{"snapshot"?: "<id>", "name"?: "...", "document"?: "...", "guard"?: true, "timeout_ms"?: n}` | `group.apply {group: id, ...}`; a document must fit the 64 KiB body cap (`413` otherwise): send a large one to the hub's store first and apply it by `snapshot` |
| `POST /api/v1/runs/{run}/confirm` | `group.apply.confirm {run}` |
| `POST /api/v1/runs/{run}/cancel` | `group.apply.cancel {run}` |
| `GET /api/v1/rollouts` | `fw.rollout.status` (TLS only) |
| `GET /api/v1/rollouts/{rollout}` | `fw.rollout.status {rollout}` (TLS only) |
| `GET /api/v1/firmware` | `fw.image.list` (TLS only) |

- **A guarded group apply belongs to the key's session**, as a guarded recall does above: only the same key confirms or
  cancels its run, within the guards' deadlines and `rest_idle` of its last request; another key's
  `/runs/{run}/confirm` is `404` `not_found`, and the run reverts when the key's idle session closes.
- **No rollout start and no image upload over REST** (OD-P3l): a `POST` or `PUT` on `/api/v1/rollouts...` or
  `/api/v1/firmware...` is `405` with `Allow: GET`; there is no path for `fw.rollout.start`, `fw.rollout.cancel`,
  `fw.image.put` or `fw.image.delete`. A flash is an Admin's act in a console, and the body cap stays 64 KiB.
- **The firmware reads are TLS only** (section 21.1): over plaintext they are `426` with `code` `tls_required` in every
  mode, a key minted `plaintext_ok` and upstream compatibility included.

| Code | HTTP status |
|---|---|
| `ok` | 200 |
| `bad_request` | 400 (408 a body too slow, 411 no length, 413 a body over the cap) |
| `unauthenticated` | 401 |
| `denied`, `limit_exceeded`, `installer_lock`, `image_refused`, `confirmation_required`, an `Origin` header | 403 |
| `not_found`, `no_device`, an unknown path | 404 |
| a method a path does not take (with `Allow`) | 405 |
| `conflict`, `offline` | 409 |
| `locked` | 423 |
| `rate_limited` | 429 |
| `internal`, any other code | 500 |
| `unsupported`, `unknown_type` | 501 |
| `busy`, `device` | 503 |

```sh
curl -s -H "Authorization: Bearer $KEY" http://hub.local:11915/api/v1/devices/E46058388B1A2E2C/params/vol.master
curl -s -X PUT -H "Authorization: Bearer $KEY" -d '{"value": -20}' \
     http://hub.local:11915/api/v1/devices/E46058388B1A2E2C/params/vol.master
```

## 15. Statistics and the hub's name

*State: built (P2HS).*

Spec 7.9 as written, now answered. Both are Admin only: Control and Viewer get `denied`. The cap is `hub_stats`;
a hub built with the service off (`ServiceConfig::stats`) answers both `unsupported`, as hubs before it did.

| `t` | Role | Request | Answer |
|---|---|---|---|
| `hub.stats` | admin | | `sessions`, `uptime_s`, `devices`: `[{handle, serial, cmds, errors, avg_rtt_ms, notify_dropped}]` (`serial` added, section 17) |
| `hub.rename` | admin | `name` | `name`. The new name is saved, in the next `hello` and `/dspi/v1/info`, and announced again over mDNS |

```json
{"t": "hub.stats", "id": 13}
{"t": "ok", "id": 13, "v": 1, "sessions": 3, "uptime_s": 8812,
 "devices": [{"handle": 0, "serial": "E46058388B1A2E2C", "cmds": 18233, "errors": 2, "avg_rtt_ms": 1.4, "notify_dropped": 0}]}
{"t": "hub.rename", "id": 14, "name": "Studio Mac"}
{"t": "ok", "id": 14, "v": 1, "name": "Studio Mac"}
```

What the fields count:

- `sessions`: network sessions open now, principals (API keys, schedules) included; the embedding app's own is not.
- `uptime_s`: seconds since the hub started.
- One row per **attached** device, by handle; a device offline inside its grace has no row. The counts are since
  the device attached and start again at a re-attach.
  - `cmds`: commands that ran on the device. A command refused before it reached the device (role, lock,
    limits, queue timeout) is not counted.
  - `errors`: those of them whose transfer failed (stall, timeout, disconnect, bus error).
  - `avg_rtt_ms`: their mean time on the transport, in milliseconds, three decimals; 0 before the first. Time in
    the device's queue is not included.
  - `notify_dropped`: this device's notifications that a full session outbox dropped (the session is sent
    `resync` reason 0 for them, spec 8.6), over every session.

`hub.rename`: control characters are dropped and the ends trimmed, as the embedders do with a start name. Empty
after that, not a string, or longer than 64 characters is `bad_request`, and nothing changes. The name is in use at
once (every later `hello` and `/dspi/v1/info`; sessions already open are not told), stored in the hub's store
(`hub-store.json`), where an embedder started without a name picks it up next time, and the mDNS record is
announced again under it (paced like every advertisement change, so a browser can see it a couple of seconds
later). An embedder that passes a name at every start (`uconsole hub --name`) puts its own back at the next start.

## 16. Scripting

*State: built (P2TM). `apps/terminal/src/api.rs` (`uconsole api`), `apps/terminal/src/hub.rs` (the hub's switches,
`api-key`, the prompt), `crates/dspi-cmd/src/api.rs` (the line's grammar); examples in
[`examples/`](examples/), run against a demo hub by `apps/terminal/tests/wp_p2tm_examples.rs`.*

No script engine in the hub (OD-P2g): a script calls the API from outside, and every rule the hub enforces (role,
limits, the lock, the voltage rule, attribution) applies to it as to a console.

### 16.1 `uconsole hub`

A hub started by `uconsole hub` runs every family of this page. Each can be turned off; REST is off until asked:

| Flag | Family | Default |
|---|---|---|
| `--no-health` | health (section 5) | on |
| `--no-inventory` | inventory (section 3) | on |
| `--no-identify` | Identify (section 4) | on |
| `--no-history` | event history (section 6) | on |
| `--no-alarms` | alarms (section 7) | on |
| `--no-schedule` | schedules (section 12) | on |
| `--rest` | the REST mirror (section 14.2) | off |
| `--data <dir>` | where the families keep their files | the `--store` folder |

The parameter API, presets and snapshots (sections 9 to 11), statistics (15), MQTT (13) and notifications out (8) are
always there; MQTT and notifications send nothing until an admin sets them up. A family turned off answers
`unknown_type` (REST: `501`) and leaves its cap out of `hello`.

At a running hub's prompt (a terminal) the person at the hub's machine types `inventory`, `alarms` (those on),
`schedule` (the rules and their next runs) and `api-key new <name> [viewer|control|admin]`, `api-key list`,
`api-key revoke <cid>`. The reads go as a viewer principal named `Hub console`. With the hub stopped, the same keys from
a shell: `uconsole hub api-key new|list|revoke` (with `--store` when it is not the default; while the hub runs these
are refused, as `uconsole hub role` is, since the running hub holds its store). A key is minted with the control role
unless another is named, and is shown **once**, in that answer only. `api-key revoke` revokes keys only; a console is
`revoke <cid>`'s.

### 16.2 `uconsole api`

```sh
uconsole --hub [NAME] api <t> ['<json>' | -] [--json]
uconsole --link HOST[:PORT] api <t> ['<json>' | -] [--json]
```

Sends one message of this page (`t` and the JSON object of its fields, or `-` to read the object from standard input)
on this console's own DSPi Link session, paired as any console (`--pin`, or the PIN asked for; the token is kept), and
prints the answer: its fields as indented JSON, or with `--json` the hub's whole `ok` object on one line. A refusal is
one line on standard error (with `--json`, `{"t": "err", "code", "msg"}` on standard output). `--device <serial>` puts
`serial` into a message that has neither `serial` nor `handle`. `--read-only` lets only the hub's reads out.

The exit status follows `code` (section 1.6), so a script branches on it:

| Status | `code` |
|---|---|
| 0 | `ok` |
| 1 | the command line itself (no type, fields that are not a JSON object) |
| 2 | `not_found`, `no_device` |
| 3 | any other refusal: `conflict`, `confirmation_required`, `device`, the client read-only mode |
| 4 | no hub reached, `timeout`, the connection closed, `internal` |
| 5 | `unauthenticated`, `denied` |
| 6 | `offline` |
| 7 | `limit_exceeded`, `installer_lock`, `image_refused` |
| 8 | `bad_request`, `unknown_type`, `unsupported` |
| 9 | `busy`, `rate_limited`, `locked` |

```sh
uconsole --hub api inventory.list
uconsole --hub api param.get '{"serial": "E46058388B1A2E2C", "path": "vol.master"}'
uconsole --hub --device E46058388B1A2E2C api preset.load '{"name": "Live"}'
uconsole --hub api schedule.set - < doors-open.json
```

A refusal's fields beyond `code` and `msg` (`limit`, `retry_in_ms`) are not printed yet: the DSPi Link client keeps
only those two.

### 16.3 Examples

- [`examples/hub-api.sh`](examples/hub-api.sh): a shell script with `uconsole api`, branching on the exit status.
- [`examples/hub_rest.py`](examples/hub_rest.py): Python over REST with an API key, standard library only.
- [`examples/home-assistant.yaml`](examples/home-assistant.yaml): Home Assistant's `rest_command` and REST sensors for a
  hub without a broker (MQTT discovery, section 13, is the better fit where there is one).

<!-- WP SECDOC --> All three examples work against a hub that requires TLS, and are run so by
`apps/terminal/tests/wp_secdoc_examples.rs` (and over plaintext by `wp_p2tm_examples.rs`):

- [`examples/hub_curl.sh`](examples/hub_curl.sh): curl, pinned to the hub's key with `--pinnedpubkey`.
- `hub_rest.py` with `DSPI_CA=hub.pem`: the hub's own certificate as the only trust anchor.
- `home-assistant.yaml`: `https://` with `verify_ssl: false` (Home Assistant's REST integrations take no certificate file
  or pin), so it is encrypted but does not check the hub's key; MQTT discovery (section 13) avoids that.
- `hub-api.sh`: `uconsole api` pins the key at pairing, as every console does.

Without TLS (`--tls off`, upstream compatibility, a key minted `plaintext_ok`) anything on the port can be read by
someone on the same network, API keys included: then keep the hub on a network you trust.

## 17. Deviations from the DSPi Link spec

*State: built (P2DOC). What this hub does that DSPi Link 1.2 does not define. Everything else is additive (new message
types under new caps, new fields), which the spec allows (spec 10).*

| Spec | It says | This hub | Why |
|---|---|---|---|
| 4.2 | "Nothing else is defined" on the port; "deliberately no REST API for device control" | `/api/v1/` on the DSPi Link port, with `--rest` only (section 14) | REST is what curl, scripts and Home Assistant speak. It is a thin translator onto the same handlers as the WebSocket messages, so the spec's reason holds: one path for serialisation, locking and attribution. It stays outside the protocol's `/dspi/` namespace |
| 4.2 | `/` and static assets for an optional browser UI (`web=1`) | with `--web`: the hub's page under `/hub/` and the web console under `/console/` | the multi-device view is the web console served by the hub |
| 7.9 | `hub.stats` device rows `{handle, cmds, errors, avg_rtt_ms, notify_dropped}` | each row also carries `serial` | an offline-safe key for a row, as every other family uses |
| 7.1 | error codes `unauthenticated` to `internal` | adds `not_found`, `offline`, `conflict`, `confirmation_required`, `limit_exceeded`, `installer_lock`, `image_refused`, `device` (section 1.6) | additive; a client treats an unknown code as a failure with `msg` |
| 3.1, 4, 6, 7.2, 7.3 | TLS only when advertised; TOFU keyed by `hid`; no rotation | TLS 1.3 by default on the same port, the key pinned at pairing, `tls=1`, `426`, close 4003, `ok.hub_key`, `hello.hub.key_next`, `auth.rotate`, `auth.pair2`, idle expiry, `tls_only`, the `Host` rule (section 2; every item: [hub.md](hub.md), Security deviations) | the security plan (PLAN-SEC, OD-SECa); upstream clients use a hub in upstream compatibility mode |

Unchanged: `device.list` and its 60 s grace (the inventory is a separate family), every `/dspi/` path, every DSPi Link 1.2
message's meaning, `fw.install` (`unsupported`), and the binary frames.

## 18. Every message

*State: built (P2DOC). Every request type `t` and event of phase 2, with its least role and its section. Phase 3's rows
(WP P3DOC, at the end) are **draft** until the WP named in the row builds them.*

| `t` | Kind | Role | Cap | § |
|---|---|---|---|---|
| `inventory.list` | request | viewer | `inventory` | 3 |
| `inventory.set` | request | admin | `inventory` | 3 |
| `inventory.forget` | request | admin | `inventory` | 3 |
| `inventory.changed` | event | | `inventory` | 3 |
| `device.identify` | request | control | `identify` | 4 |
| `health.get` | request | viewer | `health` | 5 |
| `event.list` | request | viewer | `events` | 6 |
| `event.watch` | request | viewer | `events` | 6 |
| `event.export` | request | admin | `events` | 6 |
| `event.added` | event | | `events` | 6 |
| `alarm.list` | request | viewer | `alarms` | 7 |
| `alarm.ack` | request | control | `alarms` | 7 |
| `alarm.rules.get` | request | viewer | `alarms` | 7 |
| `alarm.rules.set` | request | admin | `alarms` | 7 |
| `alarm.changed` | event | | `alarms` | 7 |
| `system.alarms`, `system.alarm.ack`, `system.alarm` | existing | viewer, control | | 7, [audio-link.md](audio-link.md) |
| `outbound.get` | request | admin | `outbound` | 8 |
| `outbound.set` | request | admin | `outbound` | 8 |
| `outbound.test` | request | admin | `outbound` | 8 |
| `param.list` | request | viewer | `param_api` | 9 |
| `param.get` | request | viewer | `param_api` | 9 |
| `param.get_many` | request | viewer | `param_api` | 9 |
| `param.set` | request | control | `param_api` | 9 |
| `preset.list` | request | viewer | `param_api` | 10 |
| `preset.load` | request | control | `param_api` | 10 |
| `preset.save` | request | admin | `param_api` | 10 |
| `system.scene.apply` | existing | control, op by op | | 10, [audio-link.md](audio-link.md) |
| `snapshot.list` | request | viewer | `hub_snapshots` | 11.1 |
| `snapshot.get` | request | viewer | `hub_snapshots` | 11.1 |
| `snapshot.put` | request | control | `hub_snapshots` | 11.1 |
| `snapshot.rename` | request | control | `hub_snapshots` | 11.1 |
| `snapshot.note` | request | control | `hub_snapshots` | 11.1 |
| `snapshot.delete` | request | admin | `hub_snapshots` | 11.1 |
| `snapshot.changed` | event | | `hub_snapshots` | 11.1 |
| `snapshot.recall` | request | control | `hub_snapshots` | 11.2 |
| `schedule.list` | request | viewer | `schedule` | 12 |
| `schedule.set` | request | admin | `schedule` | 12 |
| `schedule.delete` | request | admin | `schedule` | 12 |
| `schedule.run_now` | request | admin | `schedule` | 12 |
| `schedule.ran` | event | | `schedule` | 12 |
| `mqtt.get` | request | admin | `mqtt` | 13.1 |
| `mqtt.set` | request | admin | `mqtt` | 13.1 |
| `mqtt.status` | request | admin | `mqtt` | 13.1 |
| `auth.mint` | request | admin | `api_keys` | 14.1 |
| `auth.list` (gains `kind`, `plaintext_ok`) | existing | admin | `api_keys` | 14.1 |
| `auth.pair` (gains `ok.hub_key`) | existing | | | 2.2 |
| `auth.pair2` (WP SECPAKE) | request, before authentication, TLS only | (pairs as `control`) | `pair2` | 2.2 |
| `auth.token` (gains `ok.hub_key`) | existing | | | 2.3 |
| `auth.rotate` | request | viewer | | 2.3 |
| `hello.hub.key_next` | field | | | 2.4 |
| `hub.stats` | request | admin | `hub_stats` | 15 |
| `hub.rename` | request | admin | `hub_stats` | 15 |
| `fw.install` | existing | | | stays `unsupported` |
<!-- WP P3DOC: phase 3's rows, draft until the WP in the last column builds them -->
| `health.get` (gains `counters`, `clip_per_min`, `last_change`) | existing | viewer | `health` | 5 (built: P3HB) |
| `schedule.set` (`targets` take `group:<id>`) | existing | admin | `schedule`, `groups` | 19.3 (built: P3GR) |
| `group.list` | request | viewer | `groups` | 19.2 (built: P3GR) |
| `group.set` | request | admin | `groups` | 19.2 (built: P3GR) |
| `group.delete` | request | admin | `groups` | 19.2 (built: P3GR) |
| `group.changed` | event | | `groups` | 19.2 (built: P3GR) |
| `group.param.get` | request | viewer | `group_params` | 20.2 (draft: P3GP) |
| `group.param.set` | request | control | `group_params` | 20.2 (draft: P3GP) |
| `group.preset.load` | request | control | `group_params` | 20.2 (draft: P3GP) |
| `group.apply` | request | control | `group_apply` | 20.3 (draft: P3BA) |
| `group.apply.confirm` | request | control | `group_apply` | 20.3 (draft: P3BA) |
| `group.apply.cancel` | request | control | `group_apply` | 20.3 (draft: P3BA) |
| `fw.image.put` | request, TLS only | admin | `rollout` | 21.2 (built: P3RO) |
| `fw.image.list` | request, TLS only | viewer | `rollout` | 21.2 (built: P3RO) |
| `fw.image.delete` | request, TLS only | admin | `rollout` | 21.2 (built: P3RO) |
| `fw.rollout.start` | request, TLS only | admin | `rollout` | 21.3 (built: P3RO) |
| `fw.rollout.status` | request, TLS only | viewer | `rollout` | 21.3 (built: P3RO) |
| `fw.rollout.cancel` | request, TLS only | admin | `rollout` | 21.3 (built: P3RO) |
| `fw.rollout.changed` | event, encrypted sessions only | | `rollout` | 21.3 (built: P3RO) |

## 19. Groups

*State: built (P3GR, `crates/dspi-hub/src/groups.rs`; text WP P3DOC). PLAN-P3 F3, OD-P3a, OD-P3b. Sections 19 to 21
are PRO phase 3 ([`docs/core-merge/PLAN-P3.md`](../core-merge/PLAN-P3.md)): the fleet in the web console.*

A group is a named set of devices, kept by the hub: a name, a note and members, nothing more. A group has no role of its
own (the three roles are per client, OD-P3a), no nesting, and a device may be in many groups. Groups are what the
fan-outs of section 20, schedule targets (19.3) and group rollouts (section 21) address.

### 19.1 What sections 19 to 21 share

- **Additive to DSPi Link 1.2.** New types under new caps, each announced in `hello.caps` only while its service runs:

  | Cap | Family | Announced |
  |---|---|---|
  | `groups` | section 19 (and `group:<id>` schedule targets) | when the hub embedder turns `ServiceConfig::groups` on (`uconsole hub`: with P3TM, every hub with a data folder unless `--no-groups`) |
  | `group_params` | section 20.2 | with `groups` and the parameter API |
  | `group_apply` | section 20.3 | with `groups` and the hub's snapshots |
  | `rollout` | section 21 | only with `uconsole hub --rollout`, on a hub that serves TLS (21.1) |

  A hub without a family answers `unknown_type` (older hubs) or `unsupported`, and our clients send nothing without the
  cap (`hub_caps()`). Answers carry `"v": 1`; events carry `v` and `hub`.
- **No new error code.** Every answer of sections 19 to 21 uses the codes of section 1.6, and only those: `denied`,
  `bad_request`, `unsupported`, `not_found`, `offline`, `conflict`, `rate_limited`, `locked`, `busy`, `limit_exceeded`,
  `installer_lock`, `image_refused`, `confirmation_required`, `tls_required`, `device`, `internal`. A program written
  against section 1.6 handles all of them. Two codes are used in a new place, and the section says so: `image_refused`
  for a firmware image the rollout will not put on a device (21.3), and `tls_required` for any `fw.*` message over
  plaintext (21.1).
- **Field names follow section 1.1:** `id` is only the request id. A field naming a group is `group`, a bulk apply's run
  `run`, a rollout `rollout`, a stored image `image` (its name). In an event the family's noun is the whole object, as
  in `alarm.changed`. (PLAN-P3 writes `group.set {id?}`, `group.delete {id}`, `fw.rollout.status {id?}`; this page
  spells them `group` and `rollout`.)
- **Several devices answer with `results`** (section 1.3): one row per device, in the group's member order or the
  order of `targets`; a row may carry fields of its own. At most 256 rows; a request that would answer more is
  `bad_request` with nothing sent. A partial failure is reported, never retried.
- **Every device write is a principal's** (section 1.7): the caller's child session through the parameter API's
  executor (section 9), with the caller's role, client and name, its own session id from the hub's remote range, judged
  per device by the role, the limits profile, the installer lock, the device lock and the voltage rule, and recorded in
  that device's history with the caller in `by`. **The hub's own writes stay the closed list of three**: nothing in
  sections 19 to 21 is one of them, not even the rollout's bootloader entry (21.3, step 4).
- **One device after another.** Fan-outs, applies and rollouts go through members in order, one at a time (OD-P3d), and
  a session runs one group request at a time (a second answers `busy` while the first runs). Nothing here is a standing
  link between devices (OD-P3c): each request writes once, now.
- **Roles:**

  | Role | May |
  |---|---|
  | viewer | `group.list`, `group.param.get`, `fw.image.list`, `fw.rollout.status` |
  | control | also `group.param.set`, `group.preset.load`, `group.apply`, `group.apply.confirm`, `group.apply.cancel` |
  | admin | also `group.set`, `group.delete`, `fw.image.put`, `fw.image.delete`, `fw.rollout.start`, `fw.rollout.cancel` |

  A lower role answers `denied` and nothing is sent. Admin is given on the hub's machine only (section 1.5).
- **Stores:** `groups.json`, `rollout.json` and `firmware/` in the hub's data folder ([hub.md](hub.md), The store),
  each written atomically, 0600 on Unix, with the corrupt-file rule of section 1.8. The diagnostics bundle carries
  `groups.json` and `rollout.json` (names, serials, hashes, step texts: no secret) as `hub/groups.json` and
  `hub/rollout.json`; it never carries an image. No image byte is ever written to a log, an event or a `Debug` line.

### 19.2 Messages

| `t` | Role | Request | Answer |
|---|---|---|---|
| `group.list` | viewer | `group?` (one group's id) | `groups`: every group, by id (`g1`, `g2`, ...), or the one asked; `not_found` for an unknown id |
| `group.set` | admin | `group`: `{id?, name, note?, members}`; without `id` a new group, with `id` that group replaced whole | `group`: as stored, `created` (bool) |
| `group.delete` | admin | `group` (its id) | `group` (the id) |

A group:

| Field | Meaning |
|---|---|
| `id` | the hub's: `g1`, `g2`, ...; never reused, also after a delete |
| `name` | 1 to 48 characters after trimming, no control characters; need not be unique (address a group by `id`) |
| `note` | at most 256 characters, any Unicode but control characters (newlines and tabs allowed, as in inventory notes); `null` when unset |
| `members` | 0 to 64 serials, each one in the inventory (section 3) and each once, kept in the order given: the order fan-outs and rollouts use |

- A member that is not an inventory serial is `not_found` (the `msg` names it); a serial given twice, a 65th member, a
  name or note out of bounds, or a member that is not a serial (a `group:` entry: groups do not nest) is `bad_request`;
  a 65th group is `conflict`; an unknown `id` in `group.set` or `group.delete` is `not_found`. A refusal changes
  nothing.
- A device that goes offline stays a member. A device forgotten (`inventory.forget`) leaves every group it was in, each with a `group.changed`; a record the
  inventory drops past its cap does not yet (its drop is not on the bus; a P3-B fix in `inventory.rs`).
- `inventory.list` is unchanged and has no `groups` field (OD-P3b): `group.list` is the one source of membership, and a
  client joins it to the inventory by serial.
- Event to every session: `group.changed {v, hub, group, change}`: `group` the whole group as it is now (as it was, for
  a delete), `change` one of `set`, `delete`, `forgot` (a member was forgotten).
- `groups.json` is one JSON object, at most 64 groups. A torn file reads what it can, the whole file kept aside as
  `groups.json.bad-<time>`.

```json
{"t": "group.set", "id": 21, "group": {"name": "Bar", "note": "Ceiling speakers",
 "members": ["E46058388B1A2E2C", "E6614103E7A1B2C3"]}}
{"t": "ok", "id": 21, "v": 1, "created": true, "group": {"id": "g1", "name": "Bar", "note": "Ceiling speakers",
 "members": ["E46058388B1A2E2C", "E6614103E7A1B2C3"]}}
```

### 19.3 `group:<id>` in schedule targets

A schedule rule's `targets` (section 12) may hold `group:<id>` beside serials, 16 entries in all as before; before phase
3 such a target answered `unsupported`, and a rule made then simply starts working.

- A group is resolved **when the rule runs**: a change of its members counts from the next run. `schedule.set` takes any
  well-formed `group:<id>`; an unknown or deleted group is reported by the run.
- Each action runs on every target in order, a group expanded to its members in member order. A device reached twice
  (named directly and through a group, or through two groups) is written once per action, at its first place.
- An unknown group is one row per action, `{"serial": "group:<id>", "action": ..., "ok": false, "code": "not_found"}`;
  an empty group answers no rows. Offline members fail at once (`offline`), as any target.
- A run answers at most 256 rows. A run that would go past it stops there: its 256th row is
  `{"serial": "", "action": ..., "ok": false, "code": "bad_request", "msg": "truncated: ..."}`, and the actions past it
  are not sent. The `schedule_failed` alarm is raised as for any failed row.
- A `serial` field of a single-device message never takes a group.

## 20. Group writes and bulk apply

*State: draft (20.2: P3GP, `crates/dspi-hub/src/api/group.rs`, cap `group_params`; 20.3: P3BA,
`crates/dspi-hub/src/api/bulk.rs`, cap `group_apply`; text WP P3DOC). PLAN-P3 F4, F5, OD-P3c, OD-P3d. Section 19.1
applies.*

### 20.1 Addressing and rows

Every message of this section names its devices by exactly one of:

- `group`: a group's id (section 19). Its members, in member order. An unknown group is `not_found` and nothing is sent;
  an empty group answers `results: []`.
- `targets`: 1 to 16 serials, each once.

Both, neither, a serial twice or more than 16 is `bad_request`. Each member is judged on its own, and one member's
refusal never stops the others. What a request as a whole answers, before anything is sent: `denied` (the role),
`bad_request` (malformed, a value outside the registry's range, `offsets` on a path that is not in dB), `not_found` (the
group), `busy` (this session's other group request still running). What a row answers:

| Row `code` | When | Sent to that device |
|---|---|---|
| (none, `ok: true`) | written, read or applied | yes |
| `not_found` | a `targets` serial the hub does not know; a preset `name` or slot the device does not have | nothing |
| `offline` | the device is known but not attached; nothing waits for it | nothing |
| `bad_request` | on a path that is not in dB, the value is outside this device's own range (an RP2040's input trim): only dB values are clamped (20.2) | nothing |
| `unsupported` | the device lacks the path, or (20.3) cannot take the document: another platform, a wire format or output count its core refuses | nothing |
| `limit_exceeded` | the write crosses the device's limits profile; `limit` names it | nothing |
| `installer_lock` | the device is installer-locked and the write is not volume, mute or preset | nothing |
| `image_refused` | (20.3) a whole image from a control caller on a device with a limits profile or a lock (P1HL) | nothing |
| `locked` | another session holds the device's lock | nothing |
| `conflict` | the device has a pending guarded apply, the caller's own included | nothing |
| `device` | the device refused the command; `status` names the DSPi Link status | as the device took it |
| `internal` | the hub failed; its log says why | unknown |

### 20.2 Linked parameters (P3GP)

One-shot fan-outs: the hub writes each member once, now, and keeps no link between them (OD-P3c); a later change on one
member is not copied to the others.

| `t` | Role | Request | Answer |
|---|---|---|---|
| `group.param.get` | viewer | `group` or `targets`, `path`, `idx?` | `path`, `idx`, `results`: rows with `value` (and `choice` or `value_b64`, as `param.get`) |
| `group.param.set` | control | `group` or `targets`, `path`, `idx?`, `value`, `offsets?`: `{"<serial>": <dB>}` | `path`, `idx`, `results`: rows with `value` (what this device was sent and read back, as `param.set`) and `clamped: true` where the offset was clamped |
| `group.preset.load` | control | `group` or `targets`, `slot` or `name`, `map?`: `{"<serial>": <slot or name>}` | `results`: rows with `slot` (the slot loaded) |

- Each member is `param.set` or `preset.load` of sections 9 and 10, as the caller's child session: the same paths (only
  those written by the API), the same values (section 1.4) and the same guards. A gated path (`"confirm": true` in
  `param.set`) is not taken here: `bad_request`, naming `param.set`.
- `value` must be inside the registry's range (`bad_request` for the request otherwise). **`offsets`** are allowed only
  on a path whose unit is dB (`vol.master`, `vol.user`, `out.gain`, `pre`, `eq.gain`, ...), else `bad_request`. A
  member of a dB path gets `value` plus its offset (0 without one), clamped into that device's own range; the row's
  `value` is what was sent, `clamped: true` when the clamp moved it. A serial in `offsets` or `map` that the request does not address is
  `bad_request`.
- `map` gives a member its own slot (0 to 9) or preset name; the others get `slot` or `name`. A name is matched among
  that device's presets, as `preset.load` matches it.

```json
{"t": "group.param.set", "id": 22, "group": "g1", "path": "vol.master", "value": -20.0,
 "offsets": {"E6614103E7A1B2C3": -3.0}}
{"t": "ok", "id": 22, "v": 1, "path": "vol.master", "idx": [], "results": [
  {"serial": "E46058388B1A2E2C", "ok": true, "value": -20.0},
  {"serial": "E6614103E7A1B2C3", "ok": false, "code": "limit_exceeded", "limit": "vol.master", "msg": "..."}]}
```

### 20.3 Bulk apply (P3BA)

One snapshot or document applied to every member, each with its own way back.

| `t` | Role | Request | Answer |
|---|---|---|---|
| `group.apply` | control | `group` or `targets`; exactly one of `snapshot` (a store id), `name`, `document` (its text, or the JSON object); `guard?` (bool), `timeout_ms?` (1000 to 600000, default 30000) | `run`, `results`: rows with `image`, `lines`, `notes` (as `snapshot.recall`'s answer) and `guard` (its id, with `guard: true`); `confirm_by` (with `guard: true`: the earliest guard deadline, a time) |
| `group.apply.confirm` | control | `run` | `run`, `results`: one row per guard of the run, `ok` when kept; `conflict` for one that had already reverted |
| `group.apply.cancel` | control | `run` | `run`, `results`: one row per guard of the run, reverted now; `conflict` for one that had already reverted |

- **The source.** `snapshot` is a snapshot id of the hub's store (section 11.1), of any device; `name` must match
  exactly one snapshot in the whole store (two are `conflict`, none `not_found`); `document` must fit one frame (section
  11.2). One shared snapshot goes to every member: the document is applied as the core applies one from another device,
  so its wire compatibility and output count decide each row (`unsupported` when that device's core refuses it).
- **Each member** takes section 11.2's recall as the caller's child session: from control, a device with a limits
  profile or a lock is that row's `image_refused` and gets nothing; from admin, the image is written and then held.
  Each recall is a `recall` record in that device's history, with its `lines` and the caller's name in `by`.
- **With `guard: true`, one guarded apply per device** ([hub.md](hub.md), Guarded apply; OD-P3d), begun as the caller
  immediately before that device's write and reverting to that device's own captured state; all of them belong to the
  caller's session. A device's deadline is its start plus `timeout_ms`. One `group.apply.confirm {run}` keeps every
  guard of the run still pending; `group.apply.cancel {run}` reverts them now. A guard not confirmed by its deadline
  reverts on its own, and every pending guard of a session's runs reverts when the session closes. A recall that fails
  after its guard began is reverted at once, and its row is `ok: false`.
- **Sequential:** the answer comes after the last member. On a large group the first members' guards may run out before
  the run ends: such a row carries `reverted: true`, and the answer's `msg` says to raise `timeout_ms` (at most 600 s).
- `run` is the hub's (`a1`, `a2`, ...), unique for the hub's run, and is answered with or without a guard. An unknown
  run, or another session's, is `not_found`; a run whose guards are all settled answers its rows again. Over REST, a
  run belongs to the key's session (section 14.2).

```json
{"t": "group.apply", "id": 23, "group": "g1", "name": "Doors open", "guard": true, "timeout_ms": 120000}
{"t": "ok", "id": 23, "v": 1, "run": "a4", "confirm_by": "2026-11-16T20:02:01Z", "results": [
  {"serial": "E46058388B1A2E2C", "ok": true, "image": true, "lines": ["..."], "notes": [], "guard": "17"},
  {"serial": "E6614103E7A1B2C3", "ok": false, "code": "offline", "msg": "not attached"}]}
{"t": "group.apply.confirm", "id": 24, "run": "a4"}
{"t": "ok", "id": 24, "v": 1, "run": "a4", "results": [{"serial": "E46058388B1A2E2C", "ok": true}]}
```

## 21. Firmware rollout

*State: built (P3RO, `crates/dspi-hub/src/rollout.rs`, `rollout/steps.rs`, `rollout/images.rs`, feature `rollout`; the
PICOBOOT writer: P3PB, `crates/dspi-core/src/picoboot.rs`; text WP P3DOC, the built details P3RO). PLAN-P3 F6 and section 1.1, OD-P2b, OD-P3e to OD-P3i, OD-P3n, OD-P3o, OD-P3r. Section 19.1 applies.*

The hub updates the firmware of a DSPi on its own USB with nobody at the rack: one device at a time, its settings
exported first, the firmware guard asked with nobody to type, the device verified after it restarts, and an alarm when it
does not come back. An Admin starts it; anyone may watch it. The spec's `fw.install` stays `unsupported` and
`fw_install` stays out of `hello.caps` (OD-P3f): this is our own family, under the cap `rollout`.

### 21.1 Where it runs, and TLS only

- **Off by default.** The cap `rollout` is announced only on a hub started with `--rollout` ([hub.md](hub.md)) that
  serves TLS (`--tls require`, or `on` with or without upstream compatibility). A `--tls off` hub has no rollout. The
  cap is in every session's `hello`, plaintext ones too, so a console can say why the rollout is closed to it.
- **TLS only, in every mode** (OD-P3n). Every `fw.*` request, the reads included, over a plaintext connection answers
  `tls_required` and does nothing: in `on` and in upstream compatibility alike, whatever the token (`tls_only` or not)
  or API key (`plaintext_ok` or not). `fw.rollout.changed` is sent to encrypted sessions only. REST: section 14.2.
- **Only the hub's own USB devices.** A device the hub reaches some other way (the demo device, MIDI, the audio link)
  is `unsupported`.
- **Verified images only, by default** (OD-P3e). A stored image is flashed only when the release's manifest lists its
  name and bytes (`verified: true`, SECFWV) unless the hub runs with `--allow-unverified-firmware`, the owner's bench
  switch, which the hub shows as a warning ([hub.md](hub.md)).

### 21.2 The image store

| `t` | Role | Request | Answer |
|---|---|---|---|
| `fw.image.put` | admin | `name`, `seq`, `data`, `last` (bool) | each chunk: `name`, `seq`, `received` (bytes so far); with `last: true`: `image` (below), `created` |
| `fw.image.list` | viewer | | `images` (oldest first), `max_images` (4), `max_bytes` (8388608) |
| `fw.image.delete` | admin | `image` (its name) | `image` (the name) |

- **Upload in chunks.** `name`: 1 to 64 characters of `A-Z a-z 0-9 . - _ +`, ending in `.uf2` (the manifest's names:
  `verified` is judged by name and bytes). `seq` starts at 0 and goes up by one. `data` is base64 (RFC 4648, padded),
  at most 49 152 characters (36 KiB of image), so a chunk fits a 64 KiB frame; `last: true` on the final chunk.
- One upload per session at a time (a `seq` 0 while one is open is `busy`). A gap or a repeat in `seq` is `bad_request`
  and drops the upload; so does an image past 8 MiB. An upload not finished within 2 min of its first chunk, or whose
  session closes, is dropped; a later chunk of it is `not_found`.
- **On the last chunk** the hub checks the whole image: `uf2_inspect` must find it valid (`bad_request`, with its
  problem); a bundled name with other bytes (a damaged copy, `verify_bundled`'s mismatch) is `bad_request`. The same
  name with the same bytes answers `created: false`; the same name with other bytes is `conflict` (delete it first); a
  fifth image is `conflict`.
- `fw.image.delete` of an image a running rollout uses is `conflict`; an unknown name `not_found`.
- `uconsole hub firmware add FILE` (P3TM, section 16) puts a file in from the hub's machine with the same checks: the
  one way in that needs no console.

An image:

| Field | Meaning |
|---|---|
| `name` | as uploaded |
| `size` | bytes |
| `sha256` | of the whole file, lowercase hex |
| `firmware_version` | the version string `uf2_inspect` finds in the image (`""` when it has none) |
| `platform` | `RP2040` or `RP2350`, from the UF2 family |
| `stamp_mv` | the core-voltage stamp the firmware guard reads from the image, in mV; `null` when unstamped (which the guard counts as 1150) |
| `verified` | `true` when the release's manifest lists this name and these bytes; `false`: shown as "not verified" |
| `added_at`, `added_by` | when, and who (the client's name; `Hub console` for `uconsole hub firmware add`) |

### 21.3 A rollout, step by step

| `t` | Role | Request | Answer |
|---|---|---|---|
| `fw.rollout.start` | admin | `serial` (or `handle`) or `group`, `image` (a stored image's name), `while_live?` (bool, default `false`) | `rollout`: the record, `state` `running` |
| `fw.rollout.status` | viewer | `rollout?` (an id) | `rollouts`: the records, newest first (64 kept), or the one asked (`not_found` otherwise) |
| `fw.rollout.cancel` | admin | `rollout` | `rollout`: the record as it is now |

Event to every encrypted session: `fw.rollout.changed {v, hub, rollout}`, the whole record, at every step.

`fw.rollout.start` answers after steps 1 and 2 of its first device (a few hundred milliseconds). A refused start
answers its code, touches nothing, leaves no record and raises no alarm. After `ok` the rollout runs on its own:
follow it with `fw.rollout.changed` or `fw.rollout.status`. A group is the same, one member after another in member
order, **stopped at the first failure** (OD-P3h); a later member's steps 1 and 2 run when its turn comes, and a refusal
there ends the rollout `failed` at that step. Only one rollout runs on a hub at a time.

The steps, as a client sees them in the record (`step`):

| # | `step` | What the hub does | Refused or failed with |
|---|---|---|---|
| 1 | `check` | The device must be online on this hub's USB and answering, not under another session's lock, with no pending guarded apply, and muted (its mute, `vol.mute`, on, or its master volume at -128 dB) unless the request says `"while_live": true` (OD-P3r). No other rollout may run, and **no board may be in BOOTSEL already** (`bootloader_counts` 0 and 0). The image must be in the store, valid, of the device's platform, and verified unless the hub allows unverified images | `unsupported` (not on this hub's USB), `offline` (not attached, or not answering), `locked`, `conflict` (a pending guarded apply; a board already in BOOTSEL), `confirmation_required` (not muted, no `while_live`), `busy` (another rollout), `not_found` (no such image), `image_refused` with `reason` `platform` or `not_verified` |
| 2 | `guard` | Reads the board as the Admin's child session (`firmware_guard_board`: `EXT_CORE_VREG`, else the status block) and asks the firmware guard **with no typed word and no typed phrase**: only "clear" goes on. The board record is kept for step 6, since a board in BOOTSEL cannot be asked | `image_refused` with `reason` the guard's: `image_invalid`, `overvolt` (stamped above 1.20 V and above the board), `image_unknown` (unstamped on a board above 1.15 V), `image_lower` (stamped lower), `board_unknown`; `msg` is the guard's own message. A K340 at 1.20 V takes exactly a 1.20 V image; a stock board (1.15 V) takes a 1.20 V image |
| 3 | `export` | Captures the device's settings into the hub's snapshot store as "Before firmware <version>, <date>" (OD-P3o), and keeps its firmware version, `EXT_LIST`, board and `EXT_CORE_VREG` (live, stored, default) in the record | `conflict` (the store is full: nothing further is done), `device` |
| 4 | `bootloader` | Sends the device to its bootloader as **the Admin's child session** (`enter_bootloader_confirmed`, the token minted inside; OD-P3g), judged by the role, the installer lock and the device lock like any client's write. The device leaves the bus. From here on a cancel is refused | `installer_lock`, `locked`, `device` |
| 5 | `find` | Looks for the board in BOOTSEL every 500 ms for up to 15 s. **Exactly one** must appear (OD-P3i); on RP2350 its USB serial must be the device's serial (on RP2040 the boot ROM's serial is fixed, and uniqueness is the only check) | failed: none, more than one, or another serial. Nothing is written; the device may be left in BOOTSEL, and the alarm says so |
| 6 | `flash` | Writes the image with the core's PICOBOOT writer: exclusive access with eject, out of XIP, erase the sectors the image covers, write, read back and compare, restart. The writer asks the guard again with step 2's board record | failed with the writer's step; a read-back that differs fails before the restart, leaving the board in BOOTSEL |
| 7 | `wait` | Waits for the device to be online again, within 60 s | failed: did not come back |
| 8 | `verify` | On the new attachment: the firmware version is the image's; `EXT_LIST` is not shorter than before; when the board needed 1.20 V, `EXT_CORE_VREG` reads stored and live 1200 mV. Then the settings are compared with the export: `settings` `unchanged`, `changed` or `unknown` | failed: another version, a shorter `EXT_LIST`, a core voltage that is not 1.20 V. `settings: changed` is not a failure: it is reported, and nothing is recalled |

**`fw.rollout.cancel`** stops a rollout before step 4 of the device it is working on: that device ends `cancelled`
(its export stays in the store), the members after it `not_tried`, the rollout `cancelled`. From step 4 on, and for a
finished rollout, it is `conflict`: a flash is never interrupted.

**At the end of each device** its history gets a `firmware` record with the Admin's client name in `by` and the steps'
texts. A failure raises the alarm **`rollout_failed`** (critical, `subject` and `serial` the device, `message` with the
step), cleared when a later rollout of that device ends `ok` or the device comes back with the expected firmware; and a
group rollout stops, its remaining members `not_tried`. **The hub never recalls the export itself** (OD-P3h): when
`settings` is `changed` a console offers "Recall the export", which is `snapshot.recall` of the step-3 snapshot, judged
as any recall.

A record:

| Field | Meaning |
|---|---|
| `id` | the hub's: `ro1`, `ro2`, ...; never reused |
| `target` | the serial, or `group:<id>` |
| `image` | `{name, sha256, firmware_version}` |
| `by`, `while_live` | the Admin's client name; as asked |
| `state` | `running`, `ok`, `failed`, `cancelled` |
| `started_at`, `ended_at` | times; `ended_at` `null` while running |
| `devices` | one entry per device, in order (below) |

A device's entry: `serial`; `state` (`pending`, `running`, `ok`, `failed`, `cancelled`, `not_tried`); `step` (the step
it is at, or ended at); `steps`: `[{step, at, ok, text}]`, one per step done; `before`: `{fw, ext_list, board:
{required_mv, source}, vreg: {live_mv, stored_mv, default_mv} or null, export}` (`export` the snapshot's id);
`after`: `{fw, ext_list, vreg, settings}` once verified; `failed`: `{step, code?, text}` or `null`. `text` is one line
for logs and people; a client shows the core's notice for the step and the code instead. `rollout.json` keeps the newest
64 records.

What one device costs: about 30 s (steps 2 to 4 a few hundred milliseconds, the BOOTSEL wait 5 to 15 s, the write about
2 s for a 630 KB image, the restart and attach 2 to 5 s). The hub serves everything else meanwhile; the device is offline
to every client for that time, as in any restart.

```json
{"t": "fw.rollout.start", "id": 31, "serial": "E46058388B1A2E2C", "image": "dspi-rp2350-1.2.0.uf2"}
{"t": "ok", "id": 31, "v": 1, "rollout": {"id": "ro1", "target": "E46058388B1A2E2C", "state": "running",
 "image": {"name": "dspi-rp2350-1.2.0.uf2", "sha256": "...", "firmware_version": "1.2.0"}, "by": "Bench console",
 "while_live": false, "started_at": "2026-11-16T20:10:00Z", "ended_at": null,
 "devices": [{"serial": "E46058388B1A2E2C", "state": "running", "step": "export", "steps": [
   {"step": "check", "at": "2026-11-16T20:10:00Z", "ok": true, "text": "The device and the image can go ahead"},
   {"step": "guard", "at": "2026-11-16T20:10:00Z", "ok": true, "text": "The image suits this board (1.20 V)"}],
  "before": null, "after": null, "failed": null}]}}
```

### 21.4 What the hub never does

- Write `EXT_CORE_VREG` (it reads it back), type `OVERVOLT` or the guard's phrase for anyone, or flash an image the
  guard does not clear outright.
- Flash two devices at once, a device while another board is in BOOTSEL, or a device not on its own USB; mount a
  bootloader drive.
- Retry a flash, a fan-out row or an apply; recall the export after a flash.
- Take a `fw.*` message over plaintext, or start a rollout or take an image over REST.
- Send `REQ_CLEAR_CLIPS`, keep a standing link between devices, or add a write of its own: the rollout's device writes
  are the Admin's child session's (section 19.1).

### 21.5 As built (P3RO)

What the code does where the text above leaves room, or is stricter than it:

- **Whose requests.** A rollout opens one principal session for its whole run (section 1.7): the Admin role, the
  starting Admin's client and name, an id from the remote range. Every device request of every step (the reads of
  steps 1, 2, 3 and 8, the bootloader entry of step 4) is that principal's child session through the parameter API's
  executor, so the run goes on when the Admin's console closes, the history names the Admin, and the hub's own
  writes stay the closed list of three. The bootloader step's entry in `steps` carries `session`, the child's id.
- **The locks, in step 1 and again just before step 4:** the installer lock (P1HL) judges the bootloader entry as
  any write, so the rollout's Admin principal passes it, as an Admin's write does (an `installer_lock` refusal from
  the hub would still end the step). A device lock held by any session, the asking console's included, is `locked`
  (the rollout's principal is another session). Any pending guarded apply is `conflict`, the asking console's
  included.
- **`image_refused`** carries `reason` in every case: `platform` and `not_verified` (step 1); `image_invalid` also for a
  stored image whose bytes changed on disk since it was added (it is read again and hashed at every start) and for an
  image with no DSPi version (the verify could not be done); the guard's reasons (step 2).
- **Mute** (step 1) is read from the device: `vol.mute` on, or `vol.master` at -127.5 dB or below.
- **Find** (step 5) fails at once when more than one board is in BOOTSEL, and when the one board is another chip than
  the image's or, on RP2350, reports another serial; it fails after 15 s when none appears.
- **Verify** (step 8) checks the version as the core does after any update (`firmware_version_matches`); the core
  voltage is read back when the board record needed more than 1.15 V, and both the live and the stored setting must
  be the board record's. `settings` compares a capture of the device made then with the export, leaving out each
  document's `meta` and `snapshot` blocks (the capture's time and id).
- **A later member of a group** refused in its steps 1 or 2 ends that device `failed` at that step (with the code),
  raises `rollout_failed` for it although nothing was sent to it, and stops the rollout.
- **Uploads:** besides one per session, at most 2 are open on the hub at once (each holds its image in memory until
  the last chunk); a third `seq` 0 is `busy`.
- **A hub stopped during a rollout** reads the record back on its next start as `failed` at the step it was at, its
  device's alarm raised, the members after it `not_tried`.
- **Embedding:** `ServiceConfig::rollout: Some(RolloutConfig { allow_unverified_firmware, boot_scan, writer, .. })`
  under the hub crate's feature `rollout`; `boot_scan` and `writer` default to the core's `picoboot_devices` and
  `picoboot_flash_confirmed` (tests inject fakes there). `serves_tls` is set by the hub from its TLS mode.
  `dspi_hub::rollout::images::add_file` is the image store's way in from the hub's machine (P3TM's
  `uconsole hub firmware add`), with the upload's checks.
- **Not yet in the core's lists:** `rollout_failed` has no row in the core's `ALARM_CODES` and notices
  (`alarm.rollout_failed`) until P3CC adds them; it is therefore kept out of `device_alarms::CODES` and the rules.
