# Security plan: encrypted DSPi Link, outbound TLS, signed downloads

*WP PLANSEC, 2026-11-15, written on `main` at `5778bae0`. Plan only: no code. P2WH facts are taken from `wp/P2WH`
(`d31c5551`), which is not merged yet. Firmware facts come from `~/prog/DSPi-bofr` (`bofr-main` `8223224`) and
`~/prog/DSPi` (upstream `ec26bf0`, v1.1.5). Firmware paths are relative to `firmware/DSPi/` in those repositories.
"ref-swift" is the plan's Swift reference export `~/.cache/dspi-build/work/ref-swift` (branch ext-channel-116 at `48cf7c5`);
the upstream console it derives from is also at `~/prog/dspi-mobile/_ref/DSPi-Console-b5`. Revised after the Fable review
`work/review-plansec/REPORT.md` (accept with changes: M1-M4, S1-S11, F1-F5 applied; every OD recorded in section 7).*

**Owner, 2026-11-15:** "The fact that the DSPi links are unencrypted we cannot do anything about (or can we? Examine what is
available in the firmware and underlying systems). But connections from our clients to hubs and other comm we should be
able to follow proper best practices for security."

**Short answer.**
- **The device links (USB, USB MIDI, alink) are local.** Nobody on the network can reach them, and encrypting them would
  protect nothing: a host that has the cable already controls the device. What can be done there is *authorisation*,
  not encryption, meaning which local software may do the dangerous things (section 2.1, OD-SECh/i).
- **DSPi Link is host software.** Hub and clients are ours on both ends, so TLS 1.3 with pinned keys is entirely possible
  and is the main change this plan makes.
- **The other network paths** (webhooks, MQTT, REST, the web console, downloads) also get current best practice.

This plan **supersedes, if the owner accepts OD-SECa:**
- the owner's "DSPi Link is not encrypted anyway, so no hub-identity pinning" (`PLAN-P2.md:23-24`, `:550`)
- OD-P2f, "no outbound TLS in phase 2" (`PLAN-P2.md:530`)
- the "no TLS" parts of OD-P2e (`PLAN-P2.md:529`)

It does not stop phase 2. P2-B merges as built. Section 6 lists what P2-C WPs should do differently so this plan lands
without rework.

---

## 1. Threat model

**Assets**
- Control of the amplifiers: volume, mute, presets, and the core voltage (a K340 at an unsafe core voltage can hang or
  get damaged; it runs 1.20 V).
- The hub's credentials: Link tokens (Control or Admin), REST API keys, the MQTT password, webhook secrets.
- The hub's integrity: a hub that can be made to flash or misconfigure devices.
- Client machines: what a downloaded app, firmware or AutoEQ file can do.

**Attackers, in order of how likely they are**
1. **A1, passive on the LAN.** A guest device, a compromised IoT box, a shared venue Wi-Fi, or a café network the laptop
   joined. It can read every plaintext frame.
2. **A2, active on the LAN.** ARP or ND spoofing, a rogue mDNS responder, a DHCP or DNS hijack. It can impersonate a hub,
   sit in the middle, and replay.
3. **A3, a web page in the user's browser.** CSRF, DNS rebinding, a malicious page with Web MIDI SysEx permission.
4. **A4, other local software on a client machine.** Any process can open a class-compliant MIDI port. On Windows,
   WinUSB binds without admin rights.
5. **A5, the supply chain.** A tampered app update, firmware image or AutoEQ database, or a compromised GitHub release.
6. **A6, physical.** Someone with the board in hand. This is out of scope beyond noting what the hardware cannot resist.

**Out of scope**
- Remote access from the internet. Spec 6.4 says to use a VPN, and `is_local_peer` already drops non-local peers
  (`crates/dspi-hub/src/server.rs:316-334`, `:679`).
- Protecting a user from their own Admin.
- A malicious hub machine. Whoever owns the hub host owns the devices on its USB ports.

**Goals.** Against A1 and A2, nothing a client or the hub sends can be read or changed, and no credential works anywhere
except where it was issued. The only window left is the first pairing, which OD-SECc narrows. Against A3, a browser can
reach nothing it is not served. Against A5, every downloaded executable artefact is verified against a key the project
controls. Against A4 and A6, the dangerous device operations are gated where the firmware can do it cheaply.

---

## 2. Inventory: every communication path

"Exposure" is who can reach the path without being on a USB cable or the board.

| # | Path | Carries | Exposure | Today | Proposed |
|---|---|---|---|---|---|
| D1 | USB vendor EP0 (`REQ_*`, about 247 codes) | Every parameter, preset or flash save, factory reset, pin config, `0xF0` bootloader | Local: the host the cable goes to | No authentication on any request (`vendor_commands.c`; `config.h:146-670`). `REQ_ENTER_BOOTLOADER 0xF0` → `reset_usb_boot(0,0)` (`vendor_commands.c:3643-3650`; upstream `:2624`). WinUSB auto-binds with no driver (`usb_descriptors.c:701-810`) | Stays plaintext: encryption is meaningless on a cable. Host-side guards stay (firmware guard, CG8, voltage rule). Firmware gates per OD-SECh |
| D2 | Extension opcode `0x00` (fork) | LIST, crossfeed, `EXT_CORE_VREG`, identify, reset info, fixes; alink and USB-sync blocks (`config.h:146-172`) | Local; reachable over every control path (`vendor_commands.c:1160`, `:2434`) | `EXT_CORE_VREG` SET accepts up to `CORE_VREG_CAP` **1.30 V** and persists it (`core_vreg.h:28-34`, `:56-71`; `core_vreg.c:53`, `:145`), although the header gives 1.21 V as the absolute maximum. The firmware itself escalates to `CORE_VREG_AUTO_CAP` 1.25 V after two watchdog hangs (`core_vreg.h:60-61`, `core_vreg.c:115-124`), and the host's `OVERVOLT` flow offers up to 1.30 V (`crates/dspi-session/src/overvolt.rs:130`) | Per-transport gate (OD-SECh, FWSEC1): off USB, SET and TRIAL capped at 1.20 V. USB, CAP, AUTO_CAP and `OVERVOLT` unchanged |
| D3 | USB MIDI SysEx (fork, `DSPI_USB_MIDI`, ON in the K340 build) | A tunnel to the **whole** vendor dispatcher, `0xF0` and `0x00` included (`midi_control.c:1-17`; `vendor_commands.c:5019-5035`, `:5053-5134`; `usb_midi_control_spec.md:225-236`) | Local, but **any process** with MIDI access, and a browser page that holds Web MIDI SysEx permission. Class-compliant, no driver | Only filter: manufacturer ID `0x7D` (`midi_control.c:41-56`); `CTRL_CMD_REFUSED` blocks only `0xA2/0xA3` on MIDI (`vendor_commands.c:5027-5031`) | OD-SECh: `0xF0` refused; `EXT_CORE_VREG` SET and TRIAL above 1.20 V refused; AUTO, TRIAL_END and CLEAR allowed. iOS uses only this transport and cannot flash anyway (MIDI spec 4.5, `ios-no-driverkit`) |
| D4 | UART / I2C control (disabled by default) | Same dispatcher minus `0xF5 0xF7 0xA2 0xA3` (`vendor_commands.c:5012-5016`). **`0xF0` is reachable** | Wired to whatever is on those pins (for example a future ESP32 front panel, `commands.md:4-5`) | CRC16 for integrity only (`ctrl_cmd_usb_only`, `vendor_commands.c:5012-5016`) | Same gate as D3 |
| D5 | alink (fork): 8b/10b over PIO, optical or SFP (`alink_protocol_spec.md:6-9`, `:46`) | Up to 64 audio channels plus a 32-byte superframe with apply ops: MUTE, PRESET_LOAD, MASTER_VOLUME, DELAY, RATE (`:267-332`) | Physical point-to-point fibre or SFP inside a rack | CRC-8/16 only (`:179-187`) | **Stays plaintext**, by decision, with no MAC. See section 4.11 |
| D6 | ESP32 / Wi-Fi / Bluetooth rig | — | — | **None in the firmware**: no cyw43, lwIP or btstack linked (they ship in the SDK `lib/` only). Docs only mention an external ESP32 bridge over UART, I2C or USB | Any future network bridge is a network service and must speak DSPi Link with TLS (section 4.11) |
| N1 | DSPi Link WebSocket `ws://<hub>:11915/dspi/v1` | PIN (`auth.pair`), token (in the `ok` reply and in every `auth.token`), every command, parameter and event | **LAN** (binds `::`, `server.rs:223`, `:355-380`; non-local peers dropped) | Plaintext; `tungstenite` built with `handshake` only (`crates/dspi-link/Cargo.toml:30-33`, `crates/dspi-hub/Cargo.toml:49`). `"tls": false` hard-coded (`server.rs:2306-2321`). Clients reject `tls=1` (`crates/dspi-link/src/discovery.rs:259-262`) | TLS 1.3 on the same port, a per-hub P-256 key, the SPKI pin bound to pairing, `--tls off\|on\|require` (sections 4.1-4.4) |
| N2 | Hub identity | `hid` (random UUIDv4, `store.rs:181-197`) in mDNS `hid`, `/info` and `hello` | LAN | `expected_hub_id` (`crates/dspi-core/src/link.rs:568-597`, `:973-983`) is **not authentication**: `hid` is public, so an impostor echoes it and receives the stored token | `hid` stays the *name*; the pinned SPKI hash becomes the *identity* (section 4.3) |
| N3 | Pairing (PIN) | 6 digits, 120 s window, SHA-256 of the PIN kept, constant-time compare; 5 failures per address per 60 s, then a 60 s block; 5 failures from anywhere kill the window (`auth.rs:25-43`, `:211-220`, `:262-282`, `:388-397`) | LAN | Good online brute-force resistance (about 5·10⁻⁶ per window). The PIN crosses the network in clear, and A2 can relay it, which is a full MITM at pairing. Anyone can kill every window (a DoS, `hostile_lan.rs:400`) | TLS hides the PIN. The fingerprint is shown with the PIN, plus a QR code; PAKE pairing in SEC-D (OD-SECc). The DoS is kept, as it is safe-failing (section 4.5) |
| N4 | Tokens | 32 random bytes, base64url; the hub keeps SHA-256 (`auth.rs:410-425`, `store.rs:174-179`). Never expire (`store.rs:29-37`) | LAN | Bearer token with no binding: A1 sniffs it and uses it from any LAN host until it is revoked | Only ever sent inside pinned TLS. Idle expiry and `auth.rotate` (OD-SECg) |
| N5 | `/dspi/v1/info` and the static `--web` pages, `/console/` | Hub id, name, auth mode, device serials and names; the console's JS and wasm | LAN; ACAO `*` on `/info` (spec 3.2) | Plain GETs do **not** validate `Host` (`own_authority`, `server.rs:1271-1277`, only shapes the CSP), so a DNS-rebinding page can read them. The WebSocket is safe through the strict Origin check (`server.rs:302-310`, `:1302-1327`) | `Host` validated on every request. Served over HTTPS on the same port. CSP `connect-src 'self' wss://<own>` (section 4.6) |
| N6 | REST `/api/v1` (P2RS, planned) | Bearer API keys, parameter reads and writes | LAN, same port (OD-P2p) | Planned plaintext; any `Origin` refused (`PLAN-P2.md:329-343`) | Same TLS listener. In `require` mode plaintext REST is refused. curl uses `--pinnedpubkey`, Home Assistant uses the exported cert (section 4.7) |
| N7 | Webhooks (P2WH, built on `wp/P2WH`) | Alarm and event bodies, HMAC-SHA256 signature over `ts.body` | Outbound to wherever the target is | `http://` only; `https://` refused with an OD-P2f message (`outbound/http.rs:43-45`, `:79`) | HTTPS by default, verified with webpki roots, or a per-target CA or pin. `http://` only with an explicit `plaintext: true`, to private or loopback addresses (OD-SECe/f). HMAC kept |
| N8 | MQTT (P2MQC client built, P2MQ service planned) | Username and password in CONNECT, alarm and event payloads, `set` topics (later) | Outbound to a broker | Plain TCP (`mqtt/mod.rs:1-2`, `client.rs:4`, `:713`); password redacted in Debug and logs (`client.rs:19`, `:118-125`) | MQTT over TLS (8883), with webpki roots, a CA file or a pin, and an optional client certificate (section 4.8) |
| N9 | DNS-SD `_dspi._tcp` | `v hid kind auth n web path d` (`discovery.rs:337-356`) | LAN multicast | Spoofable. A rogue responder can advertise the victim's `hid` (the core then sends the token to it) | Pinned clients no longer care: a wrong key fails before any token goes out. New `tlsk` key (section 4.4) |
| N10 | Mac app as hub (Swift NIO `LinkServer`, `LinkServer.swift:13-18`, `:123-125`) | As N1 | LAN | Plaintext; `"tls": false` (`:331`) | Retired by SW-M3 (`wp/SWM3`: the Rust hub through `crates/dspi-hub/src/ffi.rs`), which brings TLS with it. The Swift server is never given TLS |
| N11 | Swift URLSession fallback client (`-CoreTransport NO`, `LinkClient.swift:5`, `:68`, `:153`) | As N1 | LAN | Builds `wss://` from TXT `tls` (`HubBrowser.swift:45-56`) but has no trust delegate, so a self-signed hub fails | Stays a debug fallback, plaintext only. It refuses a hub whose stored record says TLS is required |
| N12 | App update (Swift, `AppUpdateChecker.swift:17`, `enabled = false`) | Release list from the GitHub API; `.zip` plus Ed25519 `.sig` | Internet, HTTPS | The Ed25519 check (`AppUpdateInstaller.swift:78-83`) uses **upstream Weeb Labs' public key** (`:19`, identical in ref-swift). `SecStaticCodeCheckValidity` with no requirement (`:98-110`). A feed override with no scheme check (`:23`, `:208`) | The fork's own key (OD-SECj). A designated requirement once there is a Team ID. The override must be https or loopback |
| N13 | Qt update check (`AppUpdater.cpp:147-150`) | Check and link only | Internet | https enforced, redirects checked | Unchanged |
| N14 | Release integrity (`ci/release-sums.sh`, `ci/minisign.sh`) | `SHA256SUMS` plus `.minisig` | — | `docs/release/minisign.pub` **does not exist**, so the verification step is skipped (`release-sums.sh:92`) | The owner creates the key. The script fails without it (SECREL) |
| N15 | Firmware images | Bundled `.uf2` files (Apple, Qt, Android assets), or one the user picks | Local files | SHA-256 pinned only in tests (`apps/android/docs/SUPPLY-CHAIN.md:61-75`). **No check before flashing**, only parse, family and voltage guard (`crates/dspi-core/src/firmware_guard.rs:1-14`) | A manifest of bundled hashes checked before every flash. A user-picked image is shown as "not verified". The manifest gets a signature if downloads are ever added (SECFWV) |
| N16 | AutoEQ database fetch | GitHub API and raw files | Internet, HTTPS | https enforced in core, Swift, Android and terminal (`crates/dspi-session/src/autoeq.rs:1195`, `:1234`; `AutoEQCoreUpdate.swift:29-37`; `HttpAutoEqFetch.kt:21-77`; `crates/dspi-tui/src/autoeq_fetch.rs:36-40`). **Qt has no scheme check** (`AutoEqController.cpp:679-686`) | Qt https-only, with loopback allowed for its smoke test (SECQTA). Content checks stay. The data is not executable, so no signature |
| N17 | Web console from a static HTTPS host | `wss://` through the fleet proxy (`apps/web/DEPLOY.md:105-107`) | Internet or intranet | Real cert, already TLS | Unchanged |
| N18 | `uconsole web` (`http://localhost:11916/console/`) | The browser talks to remote hubs with `ws://` | Loopback page, LAN sockets | A secure context, so WebUSB works (`DEPLOY.md:83`) | Becomes a local **pinning proxy** in `dspi-hub` (`uconsole web` is `uconsole hub --local`, `apps/terminal/src/hub.rs:28-32`): the page talks `ws://localhost`, and the hub process talks pinned TLS to the other hubs (section 4.6) |

**Per-platform facts the design relies on**
- **Apple.**
  - iOS ATS has only `NSAllowsLocalNetworking`, and macOS has no ATS keys (`Info.plist`). ATS governs URLSession and
    Network.framework only; the core's Rust sockets are not affected.
  - Local Network permission and `NSBonjourServices` (`_dspi._tcp`) are already declared and unchanged.
  - The Keychain item sets no `kSecAttrAccessible` (`LinkTokenStore.swift:41-60`).
- **Android.**
  - `cleartextTrafficPermitted="false"` (`network_security_config.xml:14`). The Network Security Config does not govern
    Rust `std::net` sockets, and the file's comment says so (its "no Kotlin HTTP" line is stale: `HttpAutoEqFetch`).
  - Tokens are sealed with a Keystore AES-GCM key (`LinkTokenStore.kt:83-131`), and `allowBackup="false"`.
- **Qt.** Windows Credential Manager, the Linux Secret Service (with the `plain` D-Bus session, which is local), or a
  0600 file as fallback (`TokenStore.cpp:84-150`). macOS Qt always uses the file (`TokenStore.h:31`).
- **Terminal.** `~/.config/uconsole/link-tokens.toml`, created 0600 with `create_new` and then renamed
  (`crates/dspi-tui/src/shell/connect.rs:907-990`).
- **Web.** `localStorage` (`apps/web/src/fleet/hub-tokens.ts:17`, `:33-75`). The hub-served CSP is strict
  (`crates/dspi-hub/src/web.rs:454-460`). WebUSB is gated on `isSecureContext` (`apps/web/src/fleet/env.ts:31-32`).
- **Hub store.**
  - `hub-store.json` is created 0600 on Unix (`store.rs:553-561`). On Windows it gets no explicit ACL (`:563-570`), so it
    inherits the profile's, which is acceptable for a file under `%LOCALAPPDATA%`.
  - The folder is created with no mode set (`store.rs:265`), so it gets the umask, usually 0755.
  - Swift's `AuthStore` writes `.atomic` with no mode (`AuthStore.swift:372`), until SW-M3 retires it.

### 2.1 What the firmware and the RP2040/RP2350 can and cannot do

**Crypto in the firmware today**
- None is linked. The link list is `CMakeLists.txt:392-405`. The K340 map file has no SHA, AES, mbedTLS, `pico_rand`
  or OTP use.
- SDK 2.2.0 has `pico_sha256` and `hardware_sha256` (an RP2350 accelerator), `pico_rand`, `pico_mbedtls` and the
  populated `lib/mbedtls`.

**Resources**
- Flash is plentiful: the K340 image is 315 KB of 4 MB (`~/prog/dspi_k340_amplifier/FW-SIZE-ANALYSIS.md:7`, `:28`).
- RAM-resident code is the scarce resource: 1.5 to 7.4 KB is left on RP2350 (`FW-SIZE-ANALYSIS.md:8-15`, `:29`), and
  0.4 to 5.3 KB on RP2040 (`:15`, `:197`).
- Heap: about 111 to 124 KB free (RP2350) and 56 to 62 KB (RP2040).
- CPU: 307.2 MHz; core 0 runs about 40 % in 4-way operation, and overload starts at 90-97 %
  (`low_latency_overhead_plan.md:7`, `:200`, `:210`).

**Identity.** The USB serial is the unique board ID (`main.c:1755`). It is public, and the device holds no per-device
secret.

**What is meaningful**
- **Encrypting USB, MIDI or alink: no.**
  - USB and MIDI end at the host, which is the trust boundary. Any software that can read the cable can also just send
    the commands.
  - The alink is a point-to-point fibre inside the rack, and its audio is not secret.
  - The cost would land in the scarce RAM-code budget and in the audio path's CPU, for nothing.
- **Authenticating host commands with HMAC and a per-device key in OTP: possible on RP2350, not worth it.**
  - Every host would need the key, so the key would sit on the same hosts that A4 runs on.
  - Provisioning is per board, and RP2040 cannot keep a secret at all.
  - It would be pointless unless `0xF0` were gated too.
- **Gating the dangerous operations without crypto: yes, cheap, and the real gain.** Per transport, as the fork already
  gates by `_active_source` (`vendor_commands.c:4763-4766`):
  - Off USB (MIDI, UART, I2C), refuse `0xF0`, and refuse `EXT_CORE_VREG` SET and TRIAL above 1.20 V. AUTO, TRIAL_END
    and CLEAR stay allowed everywhere.
  - Over USB nothing changes: CAP 1.30 V, the firmware's own escalation to AUTO_CAP 1.25 V, and the host's `OVERVOLT`
    word rule all stay. A global 1.20 V ceiling would break both the owner's calibration flow and the self-healing.
  - This is a few hundred bytes of flash code, none of it in RAM (OD-SECh, FWSEC1).
  - **No BOOTSEL-button requirement.** Reading BOOTSEL drives the QSPI chip-select as a GPIO, so the code must run from
    RAM with interrupts off and core 1 out of XIP. On the K340 that means an audio glitch and RAM-resident code in a
    1.5-7 KB budget, and the button is inside the case.
- **Signed boot on RP2350: technically possible, not recommended now.**
  - The bootrom verifies signed images with its SHA-256 hardware, so the audio path pays nothing.
  - It needs OTP burned (`SECURE_BOOT_ENABLE`, boot-key hashes, ideally debug disabled), which is **irreversible**: a lost
    key or a bad provisioning step means no new firmware, ever.
  - Published fault-injection results for the RP2350 mean it would not stop A6 anyway.
  - RP2040 cannot do it at all.
  - The K340 boards are clones of unknown stepping (`PLAN.md:17`).
  - OD-SECi: decided no.
- **Upstream (v1.1.5)** has the same unauthenticated vendor and `0xF0` paths, no MIDI, no `0x00` and no alink. Gates
  belong on `bofr-main`; an upstream pull request is the owner's to open, as for FW2.

---

## 3. What is already in the workspace

- **Rust TLS stack, used today only by `dspi-tui`** for the AutoEQ HTTPS fetch (`crates/dspi-tui/Cargo.toml:25-35`):
  - `rustls 0.23.46` with the ring provider
  - `ureq 3.5.1`
  - `ring 0.17.14`
  - `rustls-webpki 0.103.16`
  - `webpki-roots 1.0.11`
  - `rustls-pki-types 1.15.1`, which can parse PEM with no extra crate
  - The core, session and mobile libraries do not see any of it.
- **Hashing and randomness.** `sha2 0.11.1` and `getrandom 0.4.3`, used in dspi-hub and dspi-session (the wasm build
  pins getrandom in dspi-link). P2WH has a hand-rolled HMAC-SHA256 tested against RFC 4231 (`outbound/hmac.rs`).
- **Absent:**
  - `rcgen` (certificate generation)
  - `rustls-native-certs` and `rustls-platform-verifier`
  - `aws-lc-rs`
  - `hmac`
  - any PAKE crate
  - production certificate, PEM or pin code: the only certificate material is the test DER files under
    `crates/dspi-tui/testdata/tm12/`
- **cargo-deny** (`deny.toml`):
  - crates.io only (`:54-62`)
  - a license allowlist, with exceptions already for ring (ISC), rustls-webpki, untrusted, subtle and webpki-roots
    (CDLA-Permissive-2.0)
  - **no `[advisories]` section**, which matters more once TLS is in the security path (SECDENY)
- **What each platform client needs for a self-signed, pinned hub.** Every client except the browser reaches the hub
  through the Rust core: Android and Qt through UniFFI or the C API, Swift by default (`CoreLink.swift:5-20`, `:247`), and
  the terminal. So pinning is written **once**, in `dspi-link`, as a rustls `ServerCertVerifier`.
  - No OkHttp or Ktor pinning.
  - No Swift `SecTrust` delegate, except for the debug fallback, which stays plaintext.
  - No `QSslConfiguration`.
  - Android's Network Security Config and Apple's ATS do not apply to these sockets.
- **The browser cannot pin.** A self-signed `wss://` connection fails silently until the user has accepted the
  certificate for that origin, by visiting `https://hub:11915/` once. A page served over https cannot open `ws://`
  (mixed content). `http://localhost` counts as a secure context. Section 4.6 builds the web story around these facts.

---

## 4. Design

### 4.1 TLS on the DSPi Link port

- **Same port, both protocols.** The accept path reads the first byte without consuming it: `0x16`, a TLS handshake
  record, goes to TLS; anything else goes to the existing HTTP path.
  - Everything on the port (WebSocket, `/info`, the web pages, REST) is served over both.
  - One firewall rule, no new port, and no change to OD-P2p.
- **TLS 1.3 only**, rustls with the **ring** provider (already licensed in `deny.toml`, already cross-compiled for
  Windows by `dspi-tui`).
  - Default cipher suites: AES-128-GCM, AES-256-GCM, ChaCha20-Poly1305.
  - No session tickets, no 0-RTT, no client authentication on the Link port. Client identity stays the token, now sent
    only inside TLS.
  - ALPN `http/1.1`.
- **Modes (`--tls`, with the same field in the service config and in SW-M3's `HubStartOptions`):**
  - `off`: today's behaviour.
  - `on`: TLS and plaintext both served; `tlsk` advertised.
  - `require`: a plaintext HTTP request gets a short `426` page explaining TLS and showing the fingerprint. A plaintext
    WebSocket upgrade is completed and then closed at once with 4003 `tls_required`, so an older client shows a typed
    reason rather than a handshake error (one short-lived connection, inside the existing budget). `tls=1` is
    advertised.
  - The default is OD-SECb.
- **Budgets.** The TLS handshake counts against `head_timeout` and the per-peer pre-auth budget (`ServerLimits`,
  `server.rs:89-153`), so a slow handshake is a slowloris like any other. Record and frame caps are unchanged
  (`MAX_FRAME` 64 KiB).
- **Threads and the stream split.** Hub and client split one `TcpStream` into reader and writer halves with
  `try_clone()` (`crates/dspi-link/src/socket.rs:136-175`, `:315-345`; hub `server.rs:687`, `:1368`). A rustls
  connection cannot be cloned, so a shared `TlsPipe` holds `Arc<Mutex<rustls::Connection>>` plus both TCP clones:
  - The reader blocks on the socket **outside** the lock, then takes the lock to `read_tls` and
    `process_new_packets`, and writes any post-handshake records (KeyUpdate, alerts) under the existing write lock.
  - The writer takes the lock only to encrypt into a buffer (`write_tls` into a `Vec`), releases it, and then writes
    the bytes to the socket under the write lock alone. **No socket I/O ever happens while the connection lock is
    held**: otherwise a writer blocked on a full socket holds the lock, the reader cannot `process_new_packets`, and two
    peers that both write past their socket buffers deadlock. Socket writes keep the existing write-stall deadline
    (10 s).
  - No new threads (the owner's "no needless threads" rule), no async runtime.
  - The `Pipe` broken flag and its semantics are kept.
- **Cost.** ECDHE P-256 plus one ECDSA signature per connection is about 1 ms on a Raspberry Pi 4 with ring.
  Per-frame AES-GCM on control traffic is negligible. Binary size is the cost: rustls plus ring add roughly 1 MB to
  each native core library. That is an estimate; SECPIPE measures the Android `.so` delta in the cross build.

### 4.2 The hub's key and certificate

- **One P-256 ECDSA key per hub**, generated on first start in TLS mode. ECDSA rather than Ed25519, because browsers do
  not accept Ed25519 certificates.
  - Stored as `<data>/tls/key.pem` 0600, in a folder that is itself 0700. While touching this, `create_dir_all` of the
    store gets 0700 too (`store.rs:265`). On Windows, like `hub-store.json`, the file gets no explicit ACL and inherits
    the profile's under `%LOCALAPPDATA%`; that is accepted.
  - **The pin is SHA-256 of the SubjectPublicKeyInfo** (as in RFC 7469 and curl `--pinnedpubkey`), so the certificate can
    be re-issued with no re-pairing.
- **A self-signed X.509 certificate** made with `rcgen` (OD-SECd).
  - SANs: the mDNS name, `localhost`, and a stable `<hid>.dspi.local`-style name. IP literals only when the address is
    configured as static. Chrome's click-through is per host and certificate, so a certificate re-issued on every DHCP
    change would warn again each time. Chrome requires SANs (no CN fallback).
  - Validity: 825 days, re-issued automatically with the same key when it is within 30 days of expiry or the mDNS name
    changes.
  - **Clients never check certificate dates.** `PinVerifier` checks the SPKI and the handshake signature only: a Pi with
    no battery clock boots in 1970, and a pin does not need dates.
  - No HSTS, ever: HSTS on a self-signed certificate turns the click-through into a hard error.
- **Display form:** `SHA256:` plus the first 80 bits in base32 as four groups (`K7Q2-M9XD-…`), for people. The full hex
  string is for `curl --pinnedpubkey sha256//<base64>` and for QR codes.
- **Key rotation (`uconsole hub tls rotate`, Admin on the hub's machine).** A new key is made, and the hub announces
  `hello.hub.key_next = {spki, sig}`, where `sig` is the old key's ECDSA signature over `"dspi-key-rotate\0" || hid ||
  new_spki`.
  - For 30 days the hub serves the new certificate and keeps announcing `key_next`. A pinned client re-pins in one of
    two ways, both before any token is sent:
    - It saw a valid `key_next` for this key on an earlier connection (verified with the pinned key), and accepts the
      new key directly.
    - **Rotation mode** (`PinVerifier::Rotating(old_spki)`): on a pin mismatch the handshake completes with the key
      reported, the token is withheld, and the client reads `hello.hub.key_next`. It verifies the *pinned* (old) key's
      signature over `"dspi-key-rotate\0" || hid || presented_spki`. If that verifies, it re-pins and proceeds.
  - Anything else fails with `hub_key_changed`: "this hub's key changed", offering re-pairing.
  - `rotate --forget-old` (a compromise) skips the signature, so every client re-pairs.
- **Diagnostics.** `tls/key.pem` is never put in the bundle. The certificate and fingerprint may be included
  (`crates/dspi-core/src/diag.rs`, a marked block).

### 4.3 Identity, pinning and pairing

- **The stored record per hub** goes from `hid -> token` to `hid -> {token, spki, tls_required}`. The core does the
  pinning in `dspi-link` and hands platforms an **opaque credential string** (`v2.<token>.<spki-b64>.<flags>`), which every
  platform's existing token store keeps unchanged: Keychain, Keystore, Credential Manager or Secret Service, 0600 files,
  `localStorage`.
  - Old bare tokens are read as `v1` (no pin). The core returns an upgraded credential through the existing connect
    result, which platforms save, so each platform gains a one-line "save if changed".
- **The connect rule in the core (TLS-first, no downgrade):**
  1. **The record has a pin.**
     - Connect with TLS and verify the SPKI before any application byte. The handshake signature is verified normally,
       so the hub must hold the key.
     - A mismatch goes to rotation mode (section 4.2): re-pin only on a valid signature from the pinned key. Otherwise
       it fails with `hub_key_changed`. The token is **never** sent to an unverified key, and the UI offers re-pairing,
       nothing else. No "connect anyway" button.
     - Plaintext is never tried, whatever the TXT record says.
  2. **The record has no pin** (a v1 credential, or first contact).
     - Try TLS first. If it succeeds, the key is trusted on first use, but **only while pairing or with an existing v1
       token**.
     - With a v1 token the client pins and upgrades the record. The first connection after the upgrade is the one TOFU
       moment for clients paired before TLS. The UI shows the fingerprint once ("Encrypted connection set up; hub key
       K7Q2-…").
     - If TLS is refused (an `off` hub, an upstream hub or the Swift NIO hub), connect in plaintext with an
       **"unencrypted"** badge, exactly as today, with two ratchets against an active attacker who resets the TLS
       attempt to collect a v1 token (OD-SECm):
       - (a) **Advertised TLS never falls back.** If the TXT record carried `tlsk` or `tls=1` and TLS fails, the
         client does not try plaintext ("this hub advertises encryption but refused it").
       - (b) **`tls_only` tokens, hub side.** The first time a token authenticates over TLS, the hub marks it
         `tls_only` (one bool in `StoredClient`), and refuses it over plaintext from then on.
       - Both leave clients that have not been updated working.
  3. **`expected_hub_id` stays**, as the check that the right named hub answered. The pin is what makes that check mean
     something.
- **Pairing binds the key.**
  - The hub prints and shows the fingerprint with the PIN ("PIN 482913 · key K7Q2-M9XD-…") in the terminal
    (`apps/terminal/src/hub.rs:1211-1240`), its JSON event, and the web and Mac settings.
  - The client shows the fingerprint it sees on the PIN entry screen, and pins it.
  - `auth.pair`'s `ok` carries `hub_key` (the SPKI hash), and the client checks it against the TLS key. This catches a
    misconfigured proxy, not an attacker.
  - **QR code:** `dspi://pair?h=<hid>&a=<host:port>&k=<spki-b64>&p=<pin>`, shown by the hub UI and scanned by the phones.
    It carries the pin out of band, so pairing by QR is MITM-proof.
  - **A `dspi://pair` link is a hint, never a bypass.** On Android any app can fire the intent with its own `k=`. The
    `k=` is checked against the TLS key, and the pairing screen always shows the hub name and fingerprint and needs a
    tap.
  - PAKE is OD-SECc.
- **Plaintext pairing.** In `on` mode the hub refuses `auth.pair` over plaintext unless `--allow-plain-pairing` is given
  (for upstream consoles). Existing plaintext tokens keep working in `on` mode. In `require` mode nothing is plaintext.
  OD-SECk.

### 4.4 DNS-SD and the spoofing story

- A new TXT key, `tlsk=<first 16 bytes of the SPKI hash, base64url>`, in `on` mode.
  - `tls=1` only in `require` mode: an upstream Swift client reads `tls=1` as "use `wss://`" (ref-swift
    `HubBrowser.swift:51`, `:549`; `~/prog/dspi-mobile/_ref/DSPi-Console-b5` `HubBrowser.swift:44`, `:264`) and would fail against a self-signed hub, whereas it ignores an unknown key.
  - Our discovery parser lifts its `tls=1` refusal (`discovery.rs:134`, `:259-262`), reads both keys, and ranks TLS
    addresses first.
- **TXT is a hint, never trust.** A spoofer can strip `tlsk` (a downgrade attempt) or put its own key in it.
  - Pinned clients ignore both: rule 1 never looks at TXT.
  - Unpinned clients try TLS anyway (rule 2).
  - A `tlsk` that does not match the TLS key on first contact is shown as a warning ("this hub advertises a different
    key") and blocks one-tap pairing. The discovery result carries the hint and the core compares it (SECDISC owns
    the parse and the flag; the comparison is one line in SECCORE's connect result).
- **A rogue responder advertising a victim's `hid`** now gets a TLS handshake that fails the pin, and no token. The
  remaining effect is a denial of service, which is unavoidable on a shared LAN.
- **The mDNS name conflict quirk** (`advertise.rs:16-30`) is unchanged. The pin removes its security side (a client
  reaching the wrong hub).

### 4.5 Pairing rate limits and lockout

The existing scheme already meets current best practice for a 6-digit PIN with a short window:
- 5 failures per address per minute, then 60 s blocked
- 5 failures anywhere kill the window
- IPv6 counted per /64
- 1024 addresses tracked

Changes:
- `auth.token` failures count separately, so a stale client retrying a revoked token cannot kill a pairing window. They
  are still limited per address.
- The window-kill stays (safe-failing: an attacker can stop pairing but cannot win it). The hub UI says "pairing was
  closed after 5 wrong PINs" so the person reopens it knowingly.
- A pairing attempt must arrive over TLS in `on` and `require` modes (section 4.3).

`hostile_lan.rs` keeps its tests (`:367`, `:400`, `:909`) and gains TLS ones (section 5).

### 4.6 The browser

There are three ways to serve the web console, and all keep working.

**1. From the hub, `https://<hub>:11915/console/`** (OD-SECl).
- The user accepts the self-signed certificate once. The page shows the fingerprint to compare against the hub's
  display, and a short "check this matches your hub" note.
- `Origin` accepts `https://<own name>:<port>` as well as `http://` (`origin_allowed`, `server.rs:302-310`). The CSP
  allows `connect-src 'self' wss://<own>` (`web.rs:149-153`, `:454-459`). The page derives `wss://` from
  `location.protocol`.
- In `on` mode, `http://<hub>/console/` still works, with an "unencrypted" banner and a link to the https page. In
  `require` mode it answers the `426` page.
- **To verify:** that `isSecureContext` (and so WebUSB) is true on an https page whose certificate the user accepted.
  SECWEB's probe checks it in a container browser; if it is false, WebUSB on the hub page stays unavailable, as today.
  Chromium refuses Service Workers on pages with a certificate error. The console uses web workers
  (`worker-src 'self'`), which are not affected. The path to rely on for WebUSB is the `uconsole web` proxy.

**2. `uconsole web` on `http://localhost:11916`** becomes a **local pinning proxy**.
- `uconsole web` is `uconsole hub --local` (`apps/terminal/src/hub.rs:28-32`), and its pages are served by
  `crates/dspi-hub/src/{server,web}.rs`. The proxy therefore lives in `dspi-hub`: new `proxy.rs` plus a marked route
  in `server.rs`.
- The page opens `ws://localhost:11916/hub/<hid>/dspi/v1`, and the hub process relays it to the remote hub over the
  core's pinned TLS (the fleet proxy pattern of `DEPLOY.md:107`).
- The proxy checks Origin and `Host` as the console does. It holds every hub's credential, so it is bound to loopback
  only (`--local`), never to a LAN address.
- This is the recommended local path: a secure context (WebUSB), real pinning, and no click-through.

**3. A static HTTPS host** keeps its `wss://` fleet proxy, with a real certificate.

**For every way:**
- `Host` is validated on every request (`own_authority`'s rule applied to all GETs, not only the CSP), which closes the
  DNS-rebinding read of `/info` and assets.
- Tokens stay in `localStorage`, which the strict CSP protects (no inline script, `frame-ancestors 'none'`).

### 4.7 REST

- REST is served on the same TLS listener.
  - `Authorization: Bearer <key>` only.
  - Any `Origin` refused (unchanged).
  - In `require` mode a plaintext request gets `426`.
  - In `on` mode a plaintext REST request is **refused unless the key was minted with `plaintext_ok: true`** (OD-SECk;
    for a legacy integration on a trusted network). The default is TLS.
- **For integrators** (`docs/integrator/hub-api.md` §2, rewritten by SECDOC):
  - `uconsole hub tls cert > hub.pem` exports the certificate.
  - curl: `curl --pinnedpubkey 'sha256//<b64>' -k https://hub:11915/api/v1/health`, or `--cacert hub.pem`.
  - Home Assistant's REST integration: `verify_ssl` pointed at the PEM.
  - Python: `requests` with `verify='hub.pem'`.
- API keys: shown once, hashed, revocable (P2RS). Rotation is `auth.mint` of a new key, then `auth.revoke` of the old one.

### 4.8 Outbound: webhooks and MQTT

- **One shared connector** (`crates/dspi-hub/src/outbound/tls.rs`, used by both): rustls client, TLS 1.2 and 1.3
  (receivers and brokers in the wild still use 1.2), SNI. Three trust options per target:
  - (a) `webpki-roots`, the default
  - (b) `ca_pem`, a CA or self-signed certificate pasted by an Admin
  - (c) `pin_sha256`, an SPKI pin for a self-signed LAN receiver
  - Hostname verification is always on, except under (c), where the pin replaces it.
  - The trust-store choice is OD-SECf.
- **Webhooks.**
  - `https://` is the default and the norm.
  - `http://` is accepted only with `"plaintext": true` on the target **and** a private, loopback or link-local
    destination, after DNS resolution and checked on every send, so a DNS change cannot move it to the internet in the
    clear. It is shown as "unencrypted" in `outbound.get`. OD-SECe.
  - The HMAC signature and timestamp stay: they authenticate the hub to the receiver whatever the transport.
  - Redirects are not followed (unchanged).
  - Existing `http://` targets created by P2WH load as `plaintext: true` with a one-time notice.
- **MQTT** (`mqtt/client.rs`; the service is P2MQ's).
  - `tls: {mode: off|on, ca_pem?, pin_sha256?, client_cert_pem?, client_key_pem?}`. The port defaults to 8883 when
    `on`.
  - The client certificate and key are stored in `<data>/mqtt.json` (0600, redacted from diagnostics), or as file paths
    to PEMs kept 0600.
  - A broker on loopback may stay plaintext without a warning. A plaintext broker elsewhere is shown as "unencrypted",
    and the password goes out only after an explicit `allow_plaintext_password: true`.
  - **Migration (OD-SECo):** an existing `mqtt.json` with a plaintext broker that is not on loopback (configured under
    OD-P2f) loads with `allow_plaintext_password: true` and a one-time notice, as SECOUT does for `http://` targets, so
    no working setup breaks.
  - Home Assistant (P2HA) uses the same setting.

### 4.9 Secrets at rest and rotation

| Secret | Where | Rule |
|---|---|---|
| Hub TLS key | `<data>/tls/key.pem` | 0600, folder 0700; never in diagnostics; rotate as in section 4.2 |
| Token hashes, API key hashes | `hub-store.json` | 0600 (exists); hashes only |
| Webhook secrets, MQTT password and client key | `outbound.json`, `mqtt.json` | 0600; generated by the hub where possible; shown once; redacted (P2-B and P2-C rules) |
| Client credentials | Keychain / Keystore / Credential Manager / Secret Service / 0600 files / `localStorage` | Exists. Swift adds `kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly` (not synced, not in backups). The Qt macOS fallback file stays 0600 with its warning |
| Release keys | Owner's machine | The Ed25519 update key in the login keychain (`update-signing.swift`); `minisign.key` offline. Never in the repo or CI |

**Token lifetime (OD-SECg)**
- Console tokens are pruned after a configurable idle time (default 180 days of no use; `last_seen` already exists).
- `auth.rotate` lets a client swap its token for a fresh one in place, and a client calls it after a hub key rotation.
- API keys do not expire by default. `auth.list` shows when each was last used.
- Changing the access mode still revokes everything (`auth.rs:300-322`).

### 4.10 Downloads and updates

- **App update (Swift).**
  - Use the fork's own Ed25519 public key, not upstream's (OD-SECj). Ship the change together with the first signed
    fork release.
  - Restrict `updateFeedURLOverride` to https or loopback.
  - Once a Developer ID exists, `SecStaticCodeCheckValidity` gets a designated requirement pinning the Team ID.
  - The checker stays disabled until the repository is public (`AppUpdateChecker.swift:12-17`).
- **Releases.**
  - The owner generates `docs/release/minisign.pub` (a public key; committing it is safe).
  - `ci/release-sums.sh` fails instead of skipping when the key is missing.
  - Qt and the terminal, which only link to releases, show the `SHA256SUMS.minisig` instructions on the release page.
- **Firmware.** A build-time manifest `firmware/manifest.json` of SHA-256 per bundled image, generated from the pinned
  digests the Android supply-chain test already holds (OD-SECn).
  - **What it is and is not:** the manifest ships inside the same bundle as the images, so it cannot detect tampering
    (whoever swaps the `.uf2` swaps the manifest too). It catches build mix-ups and corruption. The real controls are
    platform code signing and the CI digest test, which SECFWV extends from Android to the Qt and Apple bundles.
  - Checked by the core in `firmware_guard` before every flash of a bundled image.
  - A user-picked image is labelled "not a verified DSPi Console image" and still needs the existing confirmations.
  - If firmware downloads are ever added, the manifest is signed with the release key and the core verifies it.
- **AutoEQ.** https everywhere (Qt fixed). It is data, not code, and its parser is hardened, so it gets no signature.

### 4.11 What stays plaintext, and why

- **USB vendor, extension and USB audio.** The cable is the trust boundary: whoever holds it can send commands, so
  encryption adds nothing. Protection is by host guards and the per-transport firmware gates (OD-SECh, FWSEC1).
- **USB MIDI SysEx.** The same, with a wider local audience (any MIDI app, a permitted web page). The answer is gating,
  not crypto.
- **The alink.** A point-to-point optical or SFP line inside a rack. A MAC on the superframe ops would need a shared
  key in every board, from a device that cannot keep a secret on RP2040. It would cost RAM-resident code, and it would
  stop only someone who can already splice the fibre (A6).
- **`--tls off` and the Swift NIO hub, while it exists**, for upstream interop, with an "unencrypted" badge in our
  clients.
- **mDNS** (inherently plaintext; treated as a hint only).
- **Loopback** (`--loopback`, `uconsole web` to the browser, a loopback MQTT broker).
- **Any future network bridge on the device side** (for example an ESP32 with Wi-Fi) is *not* allowed to be plaintext:
  it would be a DSPi Link hub, and the ESP32 has mbedTLS with hardware AES and SHA. Recorded here so nobody adds one in
  the clear.

### 4.12 Compatibility and spec deviations

Each item is recorded in a "Security deviations" section of `docs/integrator/hub.md` and in the spec copy's deviations
note, written by SECDOC. Upstream is told nothing automatically; offering it upstream is the owner's call.

1. `tlsk` TXT key, a new key. Spec 3.1's `tls=1` is used only when plaintext is refused.
2. Same-port TLS and plaintext, decided by the first byte. Spec 4 names `wss://` "when TLS is advertised"; serving both
   is additive.
3. Pins keyed by `hid` **and bound at pairing**. Spec 6.4 says TOFU keyed by `hid`; we pin the SPKI and refuse on change
   with no override. That is stricter, and consistent.
4. `hello.hub.key_next` and `auth.pair` `ok.hub_key`: new optional fields that old clients ignore.
5. `auth.rotate`, idle token expiry, and close code 4003 `tls_required`: additive.
6. Plaintext `auth.pair` refused by default in `on` mode (OD-SECk). Upstream consoles need `--allow-plain-pairing` to
   pair.
7. The `426` answer (HTTP) and the upgrade-then-close 4003 (WebSocket) in `require` mode. Upstream consoles cannot connect to a `require` hub. They can to an `on` hub.
8. `/dspi/v1/info` gains `"tls": true` and `"tls_spki"` when TLS is on. It is still readable over plaintext in `on` mode.

**Existing clients.** Our own clients before SEC-C keep working with `on` hubs over plaintext, and upgrade to TLS and
pins when updated. A `require` hub needs updated clients. The upstream Swift console works with `on` (plaintext,
`--allow-plain-pairing` to pair) and never with `require`.

---

## 5. Work packages

*Status marks (WP P3FRZ, 2026-11-16, checked against `git log --first-parent main` at `462fcde1`): each WP's merge into its
review branch and that branch's merge into `main`; SEC-C and SEC-D's review branches were taken into `main` as they were, so
their merges are on `main`'s first-parent line. SECSW is under review; SECDEF is partial (its mark says what is built).*

Sizes are this project's units (one unit is about one hour of agent time with review); these are estimates. All the
rules of `PLAN-CLIENTS.md` section 1, `WP-BRIEF.md`, the wave1 brief, and `PLAN-P2.md` section 2's hub rules apply:
- containers only, no tmpfs, no devices, scoped checks
- one module per family
- bounded everything
- typed errors, no panics on network data
- the session-id rule
- tests carry the WP id

**New dependencies go in under `// -- WP <ID> --` markers in the owning crate's `Cargo.toml`, and each one needs
`cargo deny check` to pass.** rustls and ring are new to `dspi-link` and `dspi-hub` (native targets only; wasm never
gets them). `rcgen` is new to `dspi-hub` only.

### Wave SEC-A: foundations (parallel with P2-C; touches no P2-C file)

**SECPIPE: TLS stream and pin verifier in `dspi-link`.** [DONE 2026-11-15: `a8ba5a5a` into review/seca, main `2c7dc985`] Size 3.
- **Owns:**
  - new `crates/dspi-link/src/tls.rs` and `tls/tests_wp_secpipe.rs`
  - the `tls` feature in `crates/dspi-link/Cargo.toml`: rustls 0.23 with ring, `std`, TLS 1.3; on for native, never for
    wasm
- **Implements:**
  - server and client `rustls` configs (section 4.1)
  - `PinVerifier: ServerCertVerifier`, in three modes. Every mode verifies the handshake signature through the
    provider's `verify_tls13_signature`, and **no mode checks certificate dates** (notBefore and notAfter are ignored;
    section 4.2).
    - `Pinned(spki)`: the SPKI must match.
    - `FirstUse`: accepts any key and reports the SPKI.
    - `Rotating(old_spki)`: on a mismatch, completes the handshake and reports the presented SPKI. The caller withholds
      the token and checks `key_next` (section 4.2).
  - a `verify_rotation(old_spki, hid, new_spki, sig)` helper for that check
  - `TlsPipe`, the split-safe reader and writer over one `rustls::Connection` (section 4.1), with no socket I/O while
    the connection lock is held, and the existing `Pipe` semantics (deadline, buffered flush, broken flag)
  - `sniff_tls(&TcpStream) -> bool` using `peek`
  - SPKI hash and display helpers (base32 groups, `sha256//` base64)
- **Done when:** in-process loopback tests:
  - a pinned handshake succeeds, and a wrong key fails before any application byte is readable
  - a server without the private key (wrong signature) fails in every mode
  - an expired and a not-yet-valid certificate both pass with the right pin
  - `Rotating` reports the new SPKI; a valid rotation signature verifies, and a forged one or one by another key does
    not
  - 10 000 frames each way concurrently with no corruption
  - **both halves write 1 MiB without reading: the write-stall deadline fires and nothing hangs**
  - a KeyUpdate in the middle of a stream
  - close_notify in both directions
  - a trickled handshake cut at the deadline
  - a non-TLS first byte gives `sniff_tls == false` with no byte consumed
  - `cargo deny check` passes
  - the Android `.so` size delta measured with `DSPI_PROFILE=cross` and reported
- **Deps:** none.

**SECKEY: The hub's TLS identity.** [DONE 2026-11-15: `9f97ab47` into review/seca, main `2c7dc985`] Size 1.5.
- **Owns:**
  - new `crates/dspi-hub/src/tls_identity.rs`
  - the folder `<data>/tls/`
  - `rcgen` in `crates/dspi-hub/Cargo.toml` (ring feature, under a marker; OD-SECd)
- **Implements:** section 4.2:
  - generate a P-256 key, written 0600 in a 0700 folder
  - a self-signed certificate with the SANs of section 4.2; re-issue on an mDNS-name change or near expiry, keeping
    the key
  - SPKI pin
  - `rotate` with the old-key signature over `"dspi-key-rotate\0"||hid||new_spki`, keeping the next and previous key
    for 30 days
  - a corrupt or missing key handled by the shared corrupt-file rule (kept aside, a new key made, a loud log line, an
    event so clients will show "key changed")
  - **SECKEY copies SECPIPE's 10-line SPKI hash helper** behind `pub(crate)`. The merge replaces the copy with
    SECPIPE's function, and a test asserts both give the same hash for one key. There is no other link to SECPIPE.
- **Done when:**
  - modes and permissions checked on Unix
  - a certificate that rustls accepts
  - a re-issued certificate has the same SPKI
  - an IP address change does not re-issue
  - the rotation signature verifies, and a forged one does not
  - a corrupt `key.pem` gives a new key and a `.bad-<time>` copy
  - **no key material in any `Debug` string, log line, event or the diagnostics bundle** (S9; the bundle carries the
    certificate and fingerprint only)
  - `cargo deny check` passes
- **Deps:** none.

**SECOUT: HTTPS webhooks and MQTT over TLS.** [DONE 2026-11-15: `a8d6c0f1` into `wp/MQWIRE` (wired into the MQTT service there), `59a21a53` into review/p2c, main `3b74cec3`] Size 3. SECMQ is folded in: one shared connector and one TLS test fixture.
- **Owns:**
  - new `crates/dspi-hub/src/outbound/tls.rs` (the shared outbound connector)
  - in `outbound/http.rs` and `outbound/mod.rs`, the `https://` path and the `plaintext`, `ca_pem` and `pin_sha256`
    target fields
  - in `crates/dspi-hub/src/mqtt/client.rs`, the inside of the socket open in `MqttClient::start` (`client.rs:713`)
    and `MqttConfig.tls`, with client certificates through `rustls-pki-types` PEM parsing
  - the target and MQTT sections of `docs/integrator/hub-api.md` (§8 and MQTT)
- **Implements:** section 4.8, OD-SECe/f/o:
  - **Webhooks:**
    - `webpki-roots` (already in the lockfile, now also a hub dependency)
    - `http://` only with `plaintext: true` and a private destination, checked after resolution on every send
    - existing targets migrated to `plaintext: true` with a notice
    - the same 5 s / 10 s timeouts and the 8 KiB response cap, applied to the decrypted stream
    - no redirects
  - **MQTT:**
    - the default port is 8883 with TLS
    - `allow_plaintext_password` for any broker that is not on loopback
    - existing configs migrated with a notice (OD-SECo)
    - P2MQ's `MqttClient::start` call unchanged
- **Done when:** tests against an in-test TLS listener and the fake broker wrapped in TLS (certificates made by rcgen in
  the test, or the `tm12` test DERs):
  - **Webhooks:**
    - delivery over https with a pinned key and with a CA
    - a wrong host name refused
    - a wrong pin refused
    - a plaintext public destination refused
    - a plaintext private destination with the flag delivered
    - an old `outbound.json` migrated
    - P2WH's tests still pass, with the `https` refusal test replaced by these
  - **MQTT:**
    - CONNECT over TLS with a CA, with a pin, and with a client certificate
    - a wrong CA refused, and no CONNECT bytes sent
    - plaintext to a non-loopback broker without the flag sends no password
    - an old `mqtt.json` with such a broker migrated to the flag, with a notice
    - the password redacted in every Debug or log
- **Deps:** P2WH merged (P2-B) and P2MQC (merged).

**SECREL: Release keys and the Swift updater key.** [DONE 2026-11-15: `69a2848e` into review/seca, main `2c7dc985`; with TEST keys: the real keys wait for the owner, OD-SECj] Size 1 (0.5 Linux, 0.5 Mac VM).
- **Owns:**
  - `ci/release-sums.sh` (fail instead of skip)
  - `apps/apple/DSPi Console/AppUpdateInstaller.swift` (the public key constant)
  - `AppUpdateChecker.swift` (https or loopback for the override)
  - `apps/apple/Tools/UpdateSigning/README` text
- **Implements:** sections 4.10 and N12/N14.
- **Done when:**
  - the script fails without `minisign.pub`, and passes with a test key in the WP's scratch folder
  - a Swift unit test: a zip signed with the test key verifies, one signed with upstream's key does not
  - an `http://` override is refused
- **Deps:** OD-SECj (owner action, asked 2026-11-15). The owner generates the real keys and commits `minisign.pub`.
  The WP builds with a test key and a constant that is swapped at release.

**SECFWV: Bundled firmware consistency and the CI digest test.** [DONE 2026-11-15: `3f935035` into review/seca, main `2c7dc985`] Size 1.75.
- **Owns:**
  - new `crates/dspi-core/src/firmware_manifest.rs`
  - a generator script under `tools/` that writes the manifest from the pinned digests
  - the manifest file
  - in `firmware_guard.rs`, one call under a marker
  - the CI digest test extended to the Qt (`apps/desktop/qt/firmware/`) and Apple (`apps/apple/DSPi Console/Firmware/`)
    bundles (a `ci/` check step)
- **Implements:** section 4.10, firmware (OD-SECn):
  - `verify_bundled(bytes, name) -> Verified | NotBundled | Mismatch`
  - flashing a bundled name with a mismatching hash is refused
  - a user-picked file is returned as `NotBundled`, so the clients can show the label (each client's label is one
    string, done in the SEC-C client WPs)
  - the docs describe it as a consistency check, not tamper protection
- **Done when:**
  - the manifest matches every bundled `.uf2` in all three apps
  - the CI step fails on a mismatched digest in any of the three
  - a flipped byte is refused
  - a user file is labelled and not refused
- **Deps:** none.

**SECQTA: Qt AutoEQ https-only.** [DONE 2026-11-15: `60e7b770` (with SECDENY) into review/seca, main `2c7dc985`] Size 0.25.
- **Owns:** `apps/desktop/qt/src/controllers/AutoEqController.cpp`'s fetch.
- **Done when:** a non-https URL is refused, a loopback http URL is still allowed for `SmokeAutoEq`, and a redirect to
  http is refused.
- **Deps:** none.

**SECDENY: Advisories in cargo-deny.** [DONE 2026-11-15: `60e7b770` (with SECQTA) into review/seca, main `2c7dc985`] Size 0.25.
- **Owns:** `deny.toml` (`[advisories]`, with the advisory database fetched inside the container, `ci/` step text).
- **Done when:** `ci/check.sh --step deny` runs the advisory check offline-capable (a cached database) and is green, or
  each finding is listed as an ignore with a reason.
- **Deps:** none.

**FWSEC1 (firmware, `bofr-main`): per-transport gates for dangerous ops.** [DONE 2026-11-15, in the firmware repository: `8ba8b72` merge of `wp/FWSEC1` on `bofr-main`] Size 1.5.
- Decided under OD-SECh; the owner has been asked (2026-11-15) to confirm the iOS line.
- **Owns:**
  - in `vendor_commands.c`'s dispatcher, the per-source refusal (`_active_source`, as at `vendor_commands.c:4763-4766`;
    the lists at `:5012-5035`)
  - in `core_vreg.c`, the off-USB check on SET and TRIAL
  - `Documentation/Features/usb_midi_control_spec.md` (section 4.5 and the command table) and
    `control_interfaces_spec.md`
- **Implements:**
  - `REQ_ENTER_BOOTLOADER 0xF0` refused when `_active_source != CTRL_SOURCE_USB` (MIDI, UART, I2C), with
    `CTRL_DISPATCH_BLOCKED` ("refused_transport")
  - `EXT_CORE_VREG` SET and TRIAL off USB only with a value `<= VREG_VOLTAGE_1_20` (or `0`, the build default), refused
    above
  - AUTO, TRIAL_END and CLEAR allowed on every transport
  - USB vendor unchanged: `CORE_VREG_CAP` 1.30 V, `CORE_VREG_AUTO_CAP` 1.25 V and its escalation, and the host's
    `OVERVOLT` rule
  - no BOOTSEL requirement, and no change to `0xF5`/`0xF7` over MIDI
- **Done when:**
  - host tests of the dispatch table, as the firmware's existing host tests do:
    - MIDI SET 1.25 V refused, and SET 1.20 V accepted
    - MIDI and UART `0xF0` refused
    - USB `0xF0` and USB SET 1.30 V unchanged
    - AUTO, TRIAL_END and CLEAR accepted over MIDI
  - the K340 build's RAM-code headroom reported unchanged
  - a HIL run on the K340 within the standing flash go: `0xF0` over vendor still works, and over SysEx it is refused
- **Deps:** none (firmware rules from the `dspi-firmware-env` memory: one branch, `bofr-main`).

Wave SEC-A total: **12.25**. The Mac VM part of SECREL is 0.5.

### Wave SEC-B: the hub speaks TLS (after P2-C merges: it edits `server.rs`, `auth.rs` and `store.rs`, which P2RS owns in P2-C)

**SECSRV: TLS on the Link port.** [DONE 2026-11-16: `0aef15b8` into review/secb, main `948d192e`; review fixes SECBFIXA-C `7591342b`, `5c2c42b5`, `28ebb0f2`] Size 3, or 4 if P2RS has not made the request path generic over the stream (section 6).
- **Owns:**
  - in `crates/dspi-hub/src/server.rs`:
    - the accept path (sniff and wrap)
    - `serve`, `answer` and `upgrade` over P2RS's `HubStream`
    - `--tls` and the service-config field
    - `origin_allowed` for `https`
    - `Host` validation on every request
    - `/info` `tls` and `tls_spki`
    - the `426` page, and the upgrade-then-close 4003 path
    - `ServerLimits` handshake accounting
    - a per-connection "is TLS" flag for SECPAIR
  - `crates/dspi-hub/src/web.rs`'s CSP `wss://`
  - new `crates/dspi-hub/tests/wp_secsrv_tls.rs`
  - additions to `tests/hostile_lan.rs` under a marker
- **Implements:** sections 4.1, 4.6 (server side) and 4.7.
- **Done when:**
  - conformance over TLS for the WebSocket, `/info`, a static page and REST
  - `on` serves both
  - in `require`:
    - plaintext HTTP gets `426`
    - a plaintext WebSocket upgrade is closed with 4003 and the reason
  - a DNS-rebinding `Host` is refused on `/info` and assets
  - an `https://<own>` Origin accepted, and a foreign `https` Origin refused
  - a slow TLS handshake cut by `head_timeout`
  - pre-auth budget starvation with half-open TLS handshakes bounded
  - every existing `wp_hb1_server`, `hostile_lan` and P2RS test passes in both `off` and `on`
- **Deps:** SECPIPE, SECKEY, P2RS.

**SECDISC: Advertising and discovery.** [DONE 2026-11-16: `8ef5bcb9` into review/secb, main `948d192e`] Size 1.
- **Owns:** `HubAdvert::txt` and `parse_service` in `crates/dspi-link/src/discovery.rs` (and `discovery/tests_*`), plus
  `crates/dspi-hub/src/advertise.rs`'s TXT input.
- **Implements:** section 4.4:
  - `tlsk` in `on`, `tls=1` in `require`; both parsed
  - TLS ranked first
  - the `tls=1` refusal removed
  - the discovery result carries `tls_advertised` and the `tlsk` hint, the input to SECCORE's ratchet (a) and its
    mismatch warning
- **Done when:**
  - both keys round-trip
  - a ref-swift-shaped `tls`-less advert still parses
  - a record with a bad `tlsk` is kept, with the hint marked invalid
  - the Windows DNS-SD path is unchanged (its existing tests)
- **Deps:** none (the hint is a string field; no SECPIPE type is needed).

**SECCORE: TLS-first connect and pins in the core.** [DONE 2026-11-16: `6a14c543` into review/secb, main `948d192e`] Size 2.5.
- **Owns:**
  - in `crates/dspi-link/src/socket.rs` and `client.rs`: `wss://`, the `TlsPipe` use, `ConnectOptions.pin` and
    `ConnectOptions.tls_advertised`
  - in `crates/dspi-core/src/link.rs`:
    - the connect rule of section 4.3, including the rotation mode and ratchet (a)
    - the v2 credential
    - the errors `hub_key_changed`, `hub_key_unverified` and `tls_refused`
    - `key_next` handling
    - the `tlsk` comparison
    - the `ok.hub_key` check
    - QR and `dspi://pair` parsing, as a core function the clients call
  - C and UniFFI exports (shared wiring; run `ci/drift.sh write`, regenerate the Android contract, classify in
    `WriteGuardedCore`)
- **Implements:** section 4.3 rules 1 to 3, client side.
- **Done when:** core tests against `fakehub.rs`, extended under a marker to serve TLS:
  - a pinned connect
  - a key change gives `hub_key_changed`, the fake hub proves no token was received, and no plaintext retry happens
    even with a stripped TXT
  - a v1 token upgraded to v2 on first TLS
  - a plaintext-only hub gives an "unencrypted" status with the v1 behaviour
  - with `tls_advertised` set and TLS reset, `tls_refused`, and no plaintext retry
  - a valid `key_next` re-pinned, both from an earlier connection and in rotation mode
  - a forged `key_next` refused with no token sent
  - `expected_hub_id` still checked
  - the QR payload parses, and a mismatched `k=` is refused
  - the wasm build compiles without the `tls` feature
- **Deps:** SECPIPE (SEC-A). `ConnectOptions.tls_advertised` is a plain bool that SECDISC's result fills at merge;
  there is no type dependency within the wave.

SEC-B total: **6.5** (7.5 if SECSRV has to do the stream refactor).

### Wave SEC-C: pairing and clients (after SEC-B)

**SECPAIR: Pairing bound to the key, rotation and lifetimes.** [DONE 2026-11-16: `f1bf4731` into review/secc, on main's first-parent line] Size 2.5.
- **Owns:**
  - in `crates/dspi-hub/src/auth.rs` and `store.rs`:
    - `ok.hub_key`
    - refusing plaintext `auth.pair` unless `allow_plain_pairing`
    - the separate `auth.token` failure count
    - the `tls_only` mark on `StoredClient` (ratchet (b))
    - idle expiry and `auth.rotate`
  - `hello.hub.key_next` in the hello builder (a marked block if it sits in `server.rs`)
  - `apps/terminal/src/hub.rs`'s pairing print (fingerprint, and the QR as Unicode blocks) and
    `uconsole hub tls cert|rotate|fingerprint`
- **Implements:** sections 4.3 (hub side), 4.5 and 4.9, OD-SECg/k/m.
- **Done when:**
  - plaintext `auth.pair` refused in `on` mode, and allowed with the flag
  - `ok.hub_key` equals the TLS SPKI
  - a token used once over TLS is refused over plaintext afterwards
  - revoked-token retries do not kill the pairing window, but 5 wrong PINs do
  - an idle token pruned at the test clock's 180 days
  - `auth.rotate` swaps the hash atomically, and the old token is dead
  - rotation end to end with SECCORE's client
  - a typed-PIN pairing over TLS end to end with each mode
  - **no token, PIN, PIN hash or key material in any log line, event, Debug or diagnostics bundle** (S9)
- **Deps:** SECSRV, SECKEY, SECCORE.

**SECWEB: The web console over TLS, and the `uconsole web` pinning proxy.** [DONE 2026-11-16: `3a94f48b` into review/secc, on main's first-parent line] Size 3.5.
- **Owns:**
  - `apps/web/src/fleet/local.ts` (scheme from `location.protocol`; `wss` for https hubs)
  - the fingerprint display and the "unencrypted" banner in the hub pages (the files P2WB1 and P2WB2 created, under
    markers)
  - new `crates/dspi-hub/src/proxy.rs` and its marked route in `server.rs`: `/hub/<hid>/dspi/v1` relayed to the core's
    pinned connect, served only under `--local`
  - the browser probe script under `apps/web/tests/`
  - not `apps/terminal/src/hub.rs` (that is SECPAIR's)
- **Implements:** section 4.6 and the client half of OD-SECl.
- **Done when:**
  - Playwright (in the container) against a TLS hub with a test CA added to the browser: the console connects over
    `wss`, and the banner and fingerprint show
  - against `uconsole web`, the proxy connects with a pin and refuses a changed key
  - the proxy refuses a foreign Origin or `Host`, and is not reachable on a LAN address
  - the probe reports `isSecureContext` on an https page with an accepted self-signed certificate, and the WebUSB gate
    is set from it
  - a mixed-content `ws://` from an https page is never attempted
- **Deps:** SECSRV, SECCORE, P2WB1/P2WB2 (if they have merged; otherwise the console's existing hub page).

**SECQT, SECAN, SECTM: Pairing and key UI per client.** [DONE 2026-11-16: SECQT `1f686c55`, SECAN `c8d60a48`, SECTM `90611248` into review/secc, on main's first-parent line] Size 1 each (SECTM 0.5).
- **Owns:** each client's pairing dialog and hub list: Qt `LinkController.cpp` plus its QML, Android `LinkSessionEngine`
  plus the pairing screen, and terminal `crates/dspi-tui/src/shell/connect.rs`.
- **Implements:**
  - the fingerprint on the PIN screen
  - a "verified" or "unencrypted" badge in the hub list
  - the `hub_key_changed` and `tls_refused` dialogs (re-pair only)
  - the v2 credential saved when it changes
  - `dspi://pair` links through the core's parser, always with a confirming tap (Android: an intent from a scanner app;
    no camera permission)
  - the firmware "not verified" label (SECFWV)
- **Done when:**
  - each client's unit and UI tests for the three states and the dialogs
  - a v1 credential upgraded and saved
  - Android `LinkTokenStore` round-trip of a v2 credential
  - a `dspi://pair` link with a wrong `k=` refused
- **Deps:** SECCORE, SECFWV. The end-to-end TLS pairing test is SECPAIR's; the clients test against the core.

**SECSW: Swift, the same, on the VM.** [UNDER REVIEW 2026-11-16: `wp/SECSW` merged into `review/swsec` (`6847e658`, with P2SW), not into main; not done] Size 1.5.
- **Owns:** the Swift pairing and hub-list views, `LinkTokenStore.swift` (v2 credential,
  `kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly`), and the iOS `dspi://` URL handling.
- **Done when:**
  - macOS and iOS test bundles on the VM
  - the Keychain item attributes are asserted
  - the `-CoreTransport NO` fallback refuses a `tls_required` record
- **Deps:** SECCORE, SECFWV; SW-M3 merged (the Mac hub becomes the Rust hub, so the NIO server is not extended).

SEC-C total: **10**.

### Wave SEC-D: PAKE, defaults, docs

**SECPAKE: PAKE pairing.** [DONE 2026-11-16: `30b65b62` into review/secd, on main's first-parent line] Size 3. Scheduled, as OD-SECc decides.
- **Owns:**
  - new `crates/dspi-link/src/pake.rs` and `auth.pair2` in `auth.rs`
  - the `spake2` crate (RustCrypto, MIT or Apache-2.0; `curve25519-dalek` and `hkdf` are new too) under markers
- **Implements:** SPAKE2 with the PIN as password; the identities are `hid` and the client name; the transcript is
  bound to the TLS exporter (`export_keying_material`, label `"EXPORTER-dspi-pair"`). Both sides confirm a MAC before
  the token is issued. Native clients use it through the core; browsers keep `auth.pair` over TLS.
- **Done when:**
  - a MITM proxy test (two TLS legs) fails pairing at the first guess and cannot recover the PIN offline
  - a correct PIN pairs
  - 5 failures kill the window as before
  - legacy `auth.pair` still works for browsers and upstream
  - `cargo deny check` passes
- **Deps:** SECPAIR.

**SECDOC: Security docs and deviations.** [DONE 2026-11-16: `10d07f99` into review/secd, on main's first-parent line; the review fixes SECFIX `17f87b8a`] Size 1.
- **Owns:**
  - new `docs/integrator/security.md` (this threat model, user-facing)
  - `docs/integrator/hub.md` (modes, "Security deviations", trusted-network text rewritten)
  - `docs/integrator/hub-api.md` §2 (rewritten from "On an unencrypted network")
  - the remaining superseded prose in `PLAN-P2.md` (marked, not deleted)
- **Done when:** every item of section 4.12 is documented; curl, Home Assistant and Python examples run against a test
  hub in a container.
- **Deps:** SECPAKE's protocol text (soft: SECDOC documents `auth.pair2` from SECPAKE's entry). Otherwise SEC-C.

**SECDEF: The default mode.** [PARTIAL 2026-11-16, checked by WP P3FRZ. No WP of its own was run, and after OD-SECb's
reconsideration (section 7) none is needed for the hub: `uconsole hub` defaults to `require` since SECSRV (`db55f583`,
merged `0aef15b8`, main `948d192e`), and the done-when tests were written by SECDOC (`394afbf6`, merged `10d07f99`):
`apps/terminal/tests/wp_secdef_default.rs` asserts the `require` default of a network `uconsole hub`, and
`crates/dspi-hub/tests/wp_secdef_upgrade.rs` the upgrade of a pre-TLS store in `off`, `on` and `require`. **What remains:**
the Mac app's `HubStartOptions` default. On main `HubStartOptions` (`crates/dspi-hub/src/ffi.rs`) has no TLS field;
P2SW adds it with `require` as the default (`6ccbce1e`), merged into `review/swsec` (`8ab3698f`) and under review. SECDEF
is done when that merges to main with a test of the default] Size 0.5.
- **Owns:** the `--tls` default and the Mac `HubStartOptions` default.
- **Implements:** OD-SECb. The first TLS release ships `on`; this WP is the flip to `require` in the release after all
  clients ship SEC-C. **[Superseded for `uconsole hub` by OD-SECb's reconsideration of 2026-11-15 (section 7): it ships `require` from
  the first TLS release.]**
- **Done when:** the default is asserted in tests, and the upgrade path of an existing hub store is tested (key
  generated on first start, old tokens still valid in `on`).
- **Deps:** SEC-C on all clients.

SEC-D total: **4.5**.

### Waves

| Wave | WPs (parallel) | Size | Needs |
|---|---|---|---|
| SEC-A | SECPIPE 3, SECKEY 1.5, SECOUT 3, SECREL 1, SECFWV 1.75, SECQTA 0.25, SECDENY 0.25, FWSEC1 1.5 | 12.25 | P2-B merged (SECOUT); otherwise `main`. Disjoint from P2-C: SECOUT edits `outbound/*` and `mqtt/client.rs`, while P2AL touches only `diag.rs` and P2MQ only `mqtt/service.rs` and `mqtt.json`. **Runs alongside P2-C.** FWSEC1 is in the firmware repository |
| SEC-B | SECSRV 3 (4), SECDISC 1, SECCORE 2.5 | 6.5 (7.5) | SEC-A, P2-C merged (P2RS's `server.rs` and `auth.rs` edits) |
| SEC-C | SECPAIR 2.5, SECWEB 3.5, SECQT 1, SECAN 1, SECTM 0.5, SECSW 1.5 (VM) | 10 | SEC-B. Best after P2-E (SECWEB and the client WPs use the hub pages and views P2-E creates), or with P2-E's marked hooks (section 6) |
| SEC-D | SECPAKE 3, SECDOC 1, SECDEF 0.5 | 4.5 | SEC-C. SECDEF ships one release after SEC-C reaches every client |
| | **Total: 20 WPs** | **33.25** (34.25) | hub 10 (SECKEY, SECOUT, SECSRV, SECPAIR), dspi-link and core 11.25 (SECPIPE, SECDISC, SECCORE, SECFWV, SECPAKE), web and proxy 3.5, native clients 4.25, release and CI 1.25, firmware 1.5, docs and default 1.5 |

No WP depends on another WP of its own wave (PLAN-P2's M1 rule):
- SECKEY copies SECPIPE's hash helper.
- SECCORE and SECDISC meet only through a bool at merge.
- The client WPs depend on SECCORE (SEC-B) and SECFWV (SEC-A), not on SECPAIR.
- In SEC-D, SECDOC's dependency on SECPAKE is documentation text only.

Each wave ends with one Fable batch review, as in phase 2. The full gate runs after SEC-B (the hub and core complete)
and after SEC-C.

---

## 6. How this meets PRO phase 2

**P2-B (about to merge: P2WH, P2PA, P2HP, P2IN, P2ID, P2HS, P2EV).** Merge as built; no rework.
- P2WH's `https://` refusal (`outbound/http.rs:43-45`, `:79`) and its OD-P2f message are replaced by SECOUT.
- Until then, the message is accurate.

**P2-C (next).** Changes to the entries, all cheap. P2RS and P2MQ can start now with them:
- **P2RS:**
  - (1) Write the body read and `/api/v1/` routing against a `trait HubStream: Read + Write` (with the
    `set_read_timeout` and peer accessors that `serve` needs) rather than `&mut TcpStream` (today
    `answer(shared, id, tcp: &mut TcpStream, …)`, `server.rs:1107`). SECSRV then only changes the accept path; without
    this, SECSRV grows by 1.
  - (2) An API key field `plaintext_ok: bool`: default false, stored with the key, shown in `auth.list`, no effect until
    SECSRV.
  - (3) No `Host` validation of its own: SECSRV does it for every route.
  - (4) The docs section says "TLS: SECSRV" instead of "no TLS".
- **P2MQ:**
  - Call `MqttClient::start` unchanged. It never opens a socket itself; the client thread does that
    (`mqtt/client.rs:713`), and SECOUT changes only the inside of `client.rs`.
  - Reserve `tls: Option<Value>` in `mqtt.json`, kept on rewrite.
  - `mqtt.status` reports `encrypted: false`.
  - Nothing else.
- **P2AL:** the `outbound.json` diagnostics entry also redacts `ca_pem` and `pin_sha256`, alongside the secret (pins are
  not secret, but this keeps the rule simple).
- **P2SC:** unaffected.
- **P2CC1:** `hub_caps()` gains `tls: {mode, spki}` additively when SECSRV lands (SECCORE wires it). No change now.

**P2-D/E/F**
- **P2HA:** uses P2MQ's `MqttClient::start`, so it inherits TLS.
- **P2WB2:** the outbound and MQTT settings forms show `encrypted` and the TLS fields, with marked hooks for SECWEB's
  fingerprint and banner.
- **P2WB1:** the hub page leaves a marked slot for the "unencrypted" banner.
- **P2QT, P2AN:** no hub-security UI; SECQT and SECAN own it.
- **P2SW:** `ServiceConfig` in `HubStartOptions` gains `tls` (SW-M3's file), defaulting to `on` (SECDEF later flips it).

**`PLAN-P2.md`.** Its OD table now records OD-P2f and the "no pinning" and plaintext parts of OD-P2e as superseded by
this plan (OD-SECa, decided). The roles, limits, session-id rule, API keys, HMAC webhook signatures and everything else
in OD-P2e stay. The prose lines at `PLAN-P2.md:23-24` and §5 (`:550`) are marked by SECDOC.

---

## 7. Decisions

**Recorded 2026-11-15** (coordinator, after the Fable review `work/review-plansec/REPORT.md`):
- **Every OD below is DECIDED as recommended.** OD-SECa is decided by the owner's stated intent ("we should be able to
  follow proper best practices for security").
- **OD-SECb reconsidered 2026-11-15** (recorded here by WP P3FRZ, 2026-11-16; it was made while SECSRV was built and was
  written down only in the code): **`uconsole hub` defaults to `require` from the first TLS release**, not `on`, because
  no released client needs plaintext (the owner's compatibility policy: only Android is released, in closed test). `off` stays the default on loopback (and for `uconsole web`); `--tls on` and `--upstream-compat` remain for
  upstream and not-yet-updated clients. Built by SECSRV (`db55f583`, merged `0aef15b8`, main `948d192e`): the code says
  so in `crates/dspi-hub/src/server.rs` (`TlsMode`: "`uconsole hub` defaults to [`TlsMode::Require`] (OD-SECb,
  reconsidered 2026-11-15) and to `Off` on loopback") and `apps/terminal/src/hub.rs` (`TlsOptions::policy`: "`require`
  unless asked otherwise (OD-SECb, reconsidered 2026-11-15: no released client needs plaintext)"). The library's own
  default (`NetworkConfig`, `TlsMode::default()`) stays `Off`, so an embedder chooses. What remains of SECDEF is the
  Mac app's `HubStartOptions` default (SECDEF's mark).
- **Two owner questions remain, both asked 2026-11-15:**
  - **OD-SECj:** the owner generates and holds the release keys. SECREL builds with test keys until then.
  - **OD-SECh:** confirm that iPhone and iPad never need bootloader entry or a core voltage above 1.20 V. The recorded
    `ios-no-driverkit` decision says iOS cannot flash. FWSEC1 is built as decided; flashing it to devices beyond the
    K340 test waits for the confirmation.

| ID | Question | Options | Decision | Why |
|---|---|---|---|---|
| **OD-SECa** | Encrypt DSPi Link at all (reverses the earlier "not encrypted, no pinning"). **DECIDED** (owner's intent, 2026-11-15) | (1) keep plaintext, document it (today); (2) TLS 1.3 with a per-hub self-signed key and SPKI pinning bound to pairing; (3) TLS with a local CA the user installs on every client | **(2)** | The owner's premise that it cannot be helped holds for the device links, not for DSPi Link. Both ends are our software, and every non-browser client goes through one Rust core, so pinning is written once. Today one passive guest on the Wi-Fi gets a never-expiring Control token, and `expected_hub_id` can be fooled by echoing a public UUID. (3) asks every user to install a root CA, which is worse practice than a pin |
| **OD-SECb** | Default `--tls` mode. **DECIDED** | (1) `off`; (2) `on` (both, `tlsk`); (3) `require` | **(2) in the first TLS release; (3) the default in the release after all clients ship SEC-C**; `--tls on` documented for the upstream Swift console **[Reconsidered 2026-11-15: `uconsole hub` defaults to (3) `require` from the first TLS release, `off` on loopback; built by SECSRV `db55f583`. Recorded in the paragraph above this table.]** | `on` keeps the upstream console and not-yet-updated clients working, while updated clients go TLS-first with no downgrade. `require` is the end state; going straight to it would strand every existing install |
| **OD-SECc** | How MITM-resistant first pairing must be. **DECIDED** | (1) TOFU plus the fingerprint shown with the PIN; (2) (1) plus a QR code carrying the pin; (3) a PAKE (SPAKE2) bound to the TLS exporter | **(1) and (2) in SEC-C; (3) scheduled in SEC-D (SECPAKE), not optional**; browsers stay at (1) | A typed PIN after a MITM'd TLS is otherwise relayable. A PIN-derived MAC alone falls to an offline search of 10⁶ PINs, which is why (3) is a PAKE, the method Matter (SPAKE2+) and HomeKit (SRP) use. All native clients share the core, so (3) is one implementation. Browsers cannot reach the TLS exporter |
| **OD-SECd** | Certificate generation. **DECIDED** | (1) `rcgen` (MIT or Apache-2.0; pulls `yasna`, `pem`, `time`); (2) hand-rolled DER; (3) shelling out to `openssl` | **(1)** | Maintained, well-tested X.509 encoding. (2) is security-relevant encoding we would own forever. (3) is a runtime dependency that Windows and the Mac slice lack |
| **OD-SECe** | Webhook `http://`. **DECIDED** | (1) https only; (2) https by default, http only with `plaintext: true` to a private, loopback or link-local destination checked after resolution on every send; (3) both freely | **(2)** | A LAN receiver often has no certificate, and the HMAC still authenticates the hub. A plaintext body to the internet is never acceptable |
| **OD-SECf** | Trust store for webhooks and MQTT. **DECIDED** | (1) `webpki-roots` plus per-target `ca_pem` or `pin_sha256`; (2) the OS store through `rustls-platform-verifier`; (3) both | **(1)**, revisit (2) if a user needs a corporate CA without pasting it | Deterministic and identical on every hub platform, with no new crate. The per-target CA covers corporate and self-signed cases explicitly |
| **OD-SECg** | Token and key lifetime. **DECIDED** | (1) never expire (today); (2) idle expiry for console tokens (180 days, Admin-adjustable), none for API keys, plus `auth.rotate`; (3) hard expiry for everything | **(2)**, plus the `tls_only` ratchet (OD-SECm) | It removes forgotten laptops and phones without breaking a venue's integration at 3 a.m. A lost device should not hold Control forever |
| **OD-SECh** | Firmware gates on dangerous operations (FWSEC1, `bofr-main`). **DECIDED**; owner, asked 2026-11-15 to confirm the iOS line | (1) none (today); (2) per transport: off USB, refuse `0xF0`, and SET and TRIAL of `EXT_CORE_VREG` above 1.20 V; AUTO, TRIAL_END and CLEAR allowed; USB unchanged (CAP 1.30 V, AUTO_CAP 1.25 V escalation, `OVERVOLT`); (3) a global firmware ceiling | **(2)**. No global ceiling, no BOOTSEL requirement, no change to `0xF5`/`0xF7` over MIDI | (2) closes the path where any MIDI app or Web MIDI page can take an amplifier off the bus or push it past the datasheet's absolute maximum. It costs flash code only and follows the existing `_active_source` pattern. A MIDI-only user loses bootloader entry, which iOS cannot use (MIDI spec 4.5, `ios-no-driverkit`), and the range above 1.20 V that needs the typed `OVERVOLT` word on a computer. (3) would break the owner's calibration flow and the firmware's self-escalation. A BOOTSEL requirement needs RAM-resident code and glitches audio (section 2.1) |
| **OD-SECi** | RP2350 secure boot (signed images, OTP). **DECIDED** | (1) no; (2) yes on K340 boards; (3) signed images without burning OTP | **(1)** | Burning OTP is permanent. A lost key ends firmware updates for good, the clone boards' stepping is unknown, published fault injection means it does not stop the physical attacker it targets, and RP2040 cannot do it |
| **OD-SECj** | Release signing keys. **DECIDED**; owner action, asked 2026-11-15 | (1) keep upstream's Ed25519 public key in the fork's updater (today); (2) the fork's own Ed25519 update key plus a minisign key, both owner-held offline, public halves committed | **(2)** | With (1), the fork's updater trusts upstream's releases, and the fork cannot sign its own. The owner's act is generating the keys |
| **OD-SECk** | Plaintext credentials on an `on` hub. **DECIDED** | (1) allow plaintext `auth.pair` and REST; (2) refuse both by default, with `--allow-plain-pairing` and a per-key `plaintext_ok` | **(2)** | Otherwise A1 gets a fresh token every time someone pairs an old client. The flags keep upstream consoles and legacy scripts possible, knowingly |
| **OD-SECl** | The web console's TLS path. **DECIDED** | (1) hub-served https with a one-time certificate click-through; (2) `uconsole web` as a local pinning proxy (in `dspi-hub`); (3) document installing the hub certificate in the OS or browser store | **(1) and (2)**, (3) documented | Browsers cannot pin. (2) gives the browser real pinning and a secure context with no warning. (1) keeps "open the hub's address in a browser" working, at the cost of one warning |
| **OD-SECm** | v1 downgrade at first contact (new, review S1). **DECIDED** | (1) fall back to plaintext whenever TLS fails; (2) ratchets: (a) a hub that advertised TLS never gets a plaintext retry, (b) a token used over TLS is `tls_only` on the hub | **(2)** | Without them, an active attacker resets the TLS attempt and collects a v1 token once. Both ratchets are a few lines each and leave non-updated clients working |
| **OD-SECn** | SECFWV scope (new, review S4). **DECIDED** | (1) present the runtime manifest as tamper protection; (2) keep it as a consistency check, described honestly, and extend the CI digest test to the Qt and Apple bundles; (3) drop it | **(2)** | A manifest inside the bundle cannot detect a swapped bundle. It does catch mix-ups and corruption cheaply. The real controls are code signing and the CI digest test |
| **OD-SECo** | MQTT migration (new, review S3). **DECIDED** | (1) existing non-loopback plaintext brokers stop sending the password until an Admin sets the flag; (2) they get `allow_plaintext_password: true` with a one-time notice | **(2)** | The same rule as webhook `http://` targets. No working setup configured under OD-P2f breaks silently, and the notice says what it means |

---

## 8. Not in this plan

- Remote access, cloud relays, client certificates on the Link port, user accounts on the hub.
- Encrypting USB, MIDI or the alink (section 4.11).
- Firmware-side crypto beyond OD-SECh and OD-SECi.
- Notarisation or a Developer ID for the Mac app. That is the owner's account decision, separate from the update key;
  it is only noted in section 4.10.
