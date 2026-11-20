# Security

*WP SECDOC, 2026-11-16. The user-facing summary of the security plan
([`docs/core-merge/PLAN-SEC.md`](../core-merge/PLAN-SEC.md): threat model, design, decisions). Where this page and the
code disagree, the code is right and the page is the bug.*

What can be read or changed by whom, what this project encrypts and how, and what stays in the clear on purpose. The
hub's own switches are in [hub.md](hub.md) (Encryption), the protocol fields in [hub-api.md](hub-api.md) section 2.

## What is protected, and from whom

**What matters:** control of the amplifiers (volume, mute, presets, the core voltage), the hub's credentials (console
tokens, REST API keys, the MQTT password, webhook secrets), the hub's integrity (a hub that could be made to flash or
misconfigure devices), and the client machines (what a downloaded app, firmware image or AutoEQ file can do).

**Who, most likely first:**

| | Attacker | Can |
|---|---|---|
| A1 | someone passive on the network: a guest device, a compromised IoT box, a shared venue or café Wi-Fi | read every unencrypted byte |
| A2 | someone active on the network: ARP or ND spoofing, a rogue mDNS responder, a DNS hijack | pose as a hub, sit in the middle, replay |
| A3 | a web page in your browser | CSRF, DNS rebinding, Web MIDI SysEx if you allowed it |
| A4 | other software on a client machine | open a MIDI port, bind a USB device |
| A5 | the supply chain | a tampered app update, firmware image or database |
| A6 | someone holding the board | out of scope beyond what the hardware can do |

**Out of scope:** remote access from the internet (use a VPN; the hub drops peers that are not on a local network),
protecting you from your own Admin, and a malicious hub machine (whoever owns the hub's machine owns the devices on its
USB ports).

**The goals.** Against A1 and A2, nothing a client or the hub sends can be read or changed, and no credential works
anywhere except where it was issued. The one window left is the first pairing, narrowed by the fingerprint, the QR code
and (native clients) a PAKE. Against A3, a browser reaches nothing the hub does not serve it. Against A5, every
executable thing downloaded is checked against a key the project controls. Against A4 and A6, the dangerous device
operations are gated where the firmware can do it cheaply.

## DSPi Link is encrypted

The hub serves TLS 1.3 on its one port (11915): the WebSocket, `/dspi/v1/info`, the web pages and the REST mirror, all
on the same port as before. Every client of this project connects with TLS first: the desktop app, Android, iOS and
macOS (all through the one Rust core, which does the pinning), `uconsole`, and the web console.

- **The hub's key.** Each hub makes one P-256 key at its first start with TLS and keeps it in `<store>/tls/key.pem`
  (0600, in a 0700 folder). Its certificate is self-signed; no certificate authority is involved.
- **Pinning.** A client pins the SHA-256 of the hub's public key (its SubjectPublicKeyInfo, as RFC 7469 and curl's
  `--pinnedpubkey`) when it pairs, and from then on accepts only that key, checked in the TLS handshake **before** its
  token is sent. A pinned hub is never reached in plaintext, whatever the network says. The certificate's names and
  dates are not what is checked (a Pi with no clock battery boots in 1970), so the hub can issue its certificate again
  (a new name, near its end after 825 days) with no client noticing.
- **The fingerprint** is the pin's short form for people: `SHA256:` and four groups, `SHA256:K7Q2-M9XD-4TNB-0ZRA`.
  The hub shows it with every PIN and on its status line; the client shows the one it sees. They must match.
- **The hub id** (`hid`) stays the hub's *name*, which anyone can echo. The pinned key is its *identity*.
- **A changed key** is refused (`hub_key_changed`: the client says the hub's key changed), with re-pairing as the only
  way on (no "connect anyway").
  A rotated key is the one exception: a hub that rotates its key (`uconsole hub tls rotate`) announces the new one,
  signed by the old one, for 30 days, and a client that verifies that signature with the key it pinned follows it by
  itself.
- **No downgrade.** A hub that advertises TLS (`tls=1` in its DNS-SD record) or is pinned is reached over TLS only; if
  TLS fails the client reports `tls_refused` and does not try plaintext. And a token
  once used over TLS is refused by the hub over plaintext from then on, so an attacker who blocks TLS cannot collect it.
- **Pair Again stays encrypted** (owner decision 2026-11-17). Pairing again after `hub_key_changed` or `tls_refused`
  (or a refused token) drops the token and the pin but keeps a record that the hub was pinned: the core's
  `link_credential_unpaired` record (`v2..<old pin>.s`, no token). With it the hub is reached over TLS only, any key, and
  the new pairing pins the new key. A hub that then refuses TLS (an attacker blocking it, or TLS turned off) is refused
  with `tls_required_pinned` **before any PIN or token goes out**; the client's only offer is **Forget Hub**.
- **Forget Hub is the only way back to plaintext.** It deletes the hub's whole record (token, key and the TLS-only
  mark), and every client says so where it offers it: the hub can then be paired without encryption, with the red
  "(unsecured)" bar. Desktop, Android, iOS and macOS offer it in the dialog after `tls_required_pinned` (Android also
  under Settings > Networking > Paired Hubs), `uconsole` asks the same question in the terminal, and the web console's
  `uconsole web` proxy forgets its pin when the hub is removed from the page's list. A plain `v1` token (a hub never
  paired over TLS) keeps nothing when its pairing is dropped.
- **The record rules however the hub is reached.** A damaged `v2` record carries no token and no key but still marks
  its hub TLS-only. A hub dialled by address names itself in its hello; when a record here says that hub is TLS-only
  (pinned, or pinned before), `uconsole`, the desktop app and Android dial it again with that record even if the answer
  asked for no PIN, so no unencrypted or unchecked session opens in a pinned hub's name. What no client can do is
  recognise a known hub that answers under another id, or under none: that is a first contact, shown with its id, the
  key on the PIN prompt or the red "(unsecured)" bar.
- **A token goes only to the hub it belongs to** (WP LINKFIX, 2026-11-19; fail closed). Records are kept by the hub's
  id, and a client dialling by address sends no token before the hello names the hub and its record is read. The core
  enforces it too: `link_connect` with a bare `v1` token and no expected hub id fails with `token_without_hub_id`
  before anything is dialled (pair again, or dial with the hub's id), so a plaintext token never goes to whoever
  answers at an address. A `v2` credential is still checked by its key pin before its token goes out.

**Pairing.** The hub shows a 6-digit PIN for 2 minutes, with its key's fingerprint and a QR code of the pairing link
`dspi://pair?h=<hid>&a=<host:port>&k=<key>&p=<PIN>`:

- Typed PIN: compare the fingerprint the client shows with the hub's before you enter the PIN.
- QR code (phones): the link carries the key and the PIN out of band, so pairing by QR cannot be intercepted. A
  `dspi://pair` link is a hint, never a bypass: its key is checked against the TLS key, and the pairing screen still
  shows the hub and fingerprint and needs a tap.
- Native clients pair with a PAKE (`auth.pair2`, SPAKE2 bound to the TLS session, WP SECPAKE): a man in the middle who
  relays the TLS session learns nothing that lets it test PINs offline, and fails at its first guess. Browsers cannot
  reach the TLS session's keys and keep `auth.pair` over TLS (with the fingerprint check).
- 5 wrong PINs from anywhere close the window (the hub says so: open it again knowingly), and
  wrong PINs are also limited per address. A stale client retrying a revoked token is counted apart and never closes a
  window.

**Tokens.** A token is 32 random bytes; the hub keeps only its SHA-256. A console's token unused for 180 days is
forgotten (`--token-idle-days`), a client can swap its token for a fresh one at any time (`auth.rotate`, done after a key
rotation), and changing between PIN and open access still revokes every token. Clients keep their credential in the
platform's store: Keychain, Android Keystore, Windows Credential Manager or the Secret Service, a 0600 file, or the
browser's `localStorage`.

## When a connection is not encrypted

Some hubs cannot encrypt: an upstream WeebLabs hub (DSPi Link 1.2 has no TLS), a hub started with `--tls off`, or one in
upstream compatibility mode (below). Our clients still connect to them, and say so:

- **The connection bar turns red** and its text ends in **"(unsecured)"**, in every client, so it is said in words, not
  only by colour. A small info button beside it explains, calmly: the connection to this hub is not encrypted; that is
  fine on a trusted local network or over a VPN, but on any other network someone could read or change what is sent,
  including the pairing PIN and settings. Where the hub can do TLS it says how to turn it on, or that compatibility mode
  is on. (The text is the core's notice table, `dlink.unsecured`, `dlink.unsecured.info`, `dlink.unsecured.compat`.)
- A hub on this same machine (loopback: `uconsole web`, `--loopback`) is not shown as unsecured: nothing crosses a
  network.
- An unpinned client that finds TLS refused (an upstream hub) connects in plaintext as before; a pinned one never does.

### Upstream compatibility mode

`uconsole hub --upstream-compat`, **off by default**, lets upstream DSPi Link clients (which cannot encrypt) use our
hub. It is a security downgrade, and the hub starts with a large warning listing exactly what it turns off; the
clients' hub settings show the same list (`dlink.upstream_compat`):

- TLS is no longer required: the hub also serves unencrypted DSPi Link (`ws://`, `http://`) on its port.
- On those connections nothing is encrypted: anyone on the network can read the traffic, PINs, tokens and API keys
  included, and change it in transit.
- The hub's key is not pinned on them: an unencrypted client cannot tell this hub from another machine posing as it.
- Pairing with a PIN is allowed without encryption, so a listener can take the token a new client is given.
- Every REST API key works without encryption, not only keys minted `plaintext_ok`.
- The hub does not advertise that it requires TLS (no `tls=1` in its DNS-SD record).

Our own clients still connect to such a hub with TLS and a pin; only the unencrypted connections lose the protections.
Use it on a network you trust, only while you need it. `--tls off` on a network address is the same and more (nothing
is encrypted, no key exists) and gets its own warning.

## The browser

A browser cannot pin a key. There are three ways to the web console, all working:

1. **From the hub, `https://<hub>:11915/console/`** (`--web`). The certificate is self-signed, so the browser warns
   once per hub and certificate; accept it after comparing the fingerprint the page shows with the hub's. A plain
   `http://` page gets the hub's `426` page instead (with `--tls on` or upstream compatibility it works, with the red
   bar).
2. **`uconsole web`, `http://localhost:11916/console/`** (recommended). A local *pinning proxy*: the page talks to this
   machine, and `uconsole` talks to the other hubs over TLS pinned to their keys, so the browser gets real pinning, a
   secure context (WebUSB works) and no warning. It listens on loopback only, never on a network address, since it
   holds every hub's pin.
3. **A static HTTPS host** with its own certificate and a `wss://` fleet proxy (`apps/web/DEPLOY.md`).

For all three: every request's `Host` must be the hub's own name or an address, so a DNS-rebinding page cannot read
`/dspi/v1/info` or the assets (421); a WebSocket `Origin` must be the hub itself; the pages' CSP allows no inline script
and no framing.

## Integrations: REST, curl, Home Assistant, Python

REST (`--rest`) is served on the same TLS port, `Authorization: Bearer <key>` only, any `Origin` refused. On a hub that
requires TLS a plaintext request is answered `426`; with `--tls on` only a key minted `plaintext_ok` works in plaintext
(for a legacy script on a trusted network). API keys are shown once, kept hashed, revocable, and do not expire.

On the hub's machine: `uconsole hub tls fingerprint` prints the pin in curl's form (`sha256//...`), `uconsole hub tls cert
> hub.pem` the certificate. Run either again after a key rotation.

- **curl:** `curl --pinnedpubkey 'sha256//<b64>' -k https://hub:11915/api/v1/health -H 'Authorization: Bearer ...'`.
  The pin is checked in the handshake, before the key is sent; `-k` only skips the name and date checks the pin
  replaces. Or `--cacert hub.pem` with a name the certificate carries (the hub's `.local` name, `localhost`).
  [`examples/hub_curl.sh`](examples/hub_curl.sh).
- **Python:** the exported certificate as the only trust anchor, `ssl.create_default_context(cafile="hub.pem")`; with
  `requests`, `verify="hub.pem"` (which checks the name too). [`examples/hub_rest.py`](examples/hub_rest.py).
- **Home Assistant:** MQTT discovery (hub-api.md section 13) is the better fit: the hub connects out to your broker over
  TLS and checks the broker's certificate. Its REST integrations take only `verify_ssl: true|false`, no certificate file
  or pin, so with REST use `https://` and `verify_ssl: false`: encrypted against listeners, but not checked against a
  machine posing as the hub. Give it a key of its own with the least role it needs.
  [`examples/home-assistant.yaml`](examples/home-assistant.yaml).

These examples run against a hub that requires TLS in CI (`apps/terminal/tests/wp_secdoc_examples.rs`; the fleet's
endpoints: `wp_p3frz_examples.rs`).

## Notifications out: webhooks and MQTT

- **Webhooks** go over `https://` (TLS 1.2 or 1.3), verified against the public roots, a CA you paste (`ca_pem`), or a key
  pin (`pin_sha256`) for a receiver on your network. `http://` only with `"plaintext": true` and only to a private,
  loopback or link-local address, checked after the name is resolved, on every send. The HMAC signature still proves
  the body came from the hub. No redirects are followed.
- **MQTT** over TLS (port 8883) with the same three trust options and an optional client certificate. Without TLS the
  password goes only to a broker on loopback, unless `allow_plaintext_password` is set. Settings written before TLS keep
  working, with a one-time notice.

Details: hub-api.md sections 8 and 13.

## What stays in the clear, and why

- **USB, the extension commands, USB audio.** The cable is the trust boundary: whoever holds it can send any command,
  so encrypting it would protect nothing. Protection is by the hosts' guards (the firmware guard, confirmation tokens,
  the core-voltage rule) and the firmware's per-transport gates.
- **USB MIDI SysEx, UART, I2C.** The same, with a wider local audience (any MIDI app, a web page you gave Web MIDI SysEx
  permission). The firmware refuses bootloader entry over them, and core-voltage SET and TRIAL above 1.20 V (FWSEC1,
  `bofr-main`); over USB nothing changed. No network client may raise the core voltage above 1.20 V, whatever its role:
  do that on the hub's machine.
- **The audio link.** A point-to-point fibre or SFP line inside a rack. A MAC would need a secret in every board, which
  an RP2040 cannot keep, and would stop only someone who can already splice the fibre.
- **mDNS / DNS-SD.** Plaintext by nature, and treated as a hint only: a spoofed record cannot make a pinned client send
  its token anywhere.
- **Loopback** (`--loopback`, `uconsole web` to its browser, a broker on the same machine).
- **Hubs that cannot do TLS** (upstream, `--tls off`, compatibility mode), shown "(unsecured)".
- **A future network bridge on the device side** (an ESP32 with Wi-Fi, say) may *not* be plaintext: it would be a DSPi
  Link hub, and must speak TLS.

## Downloads and firmware

- **Releases** carry `SHA256SUMS` signed with minisign (`docs/release/`); the release script fails without the public
  key.
- **App updates (macOS)** are checked with the project's own Ed25519 key, not upstream's (a test key until the first
  signed release; the checker stays off until then), and the update feed must be https or loopback.
- **Bundled firmware** is checked against a manifest of SHA-256 digests (`firmware/manifest.json`) before every flash; a
  bundled name with another digest is refused. A file you pick yourself is labelled "not verified" and still needs the
  usual confirmations. The manifest ships in the same bundle as the images, so it catches mix-ups and corruption, not a
  swapped bundle: that is the job of the platforms' code signing and the CI digest test.
- **AutoEQ data** is fetched over https only, and parsed as data, never run.
- **RP2350 secure boot** (signed images, OTP) is deliberately not used: burning OTP is permanent, and a lost key would
  end firmware updates for good.

### The hub's firmware rollout

<!-- WP P3FRZ (PLAN-P3 F6 and section 3, OD-P2b, OD-P3e, OD-P3g, OD-P3n) -->
A hub started with `--rollout` ([hub.md](hub.md)) can update the firmware of the DSPis on its own USB with nobody at
the rack: an Admin starts it from a console, the hub does one device at a time, exports its settings to the hub's
snapshots first, checks the device after it restarts, and raises the `rollout_failed` alarm when it does not come
back. The steps: [hub-api.md](hub-api.md) section 21. A hub that could be made to flash devices is the integrity risk
named at the top of this page, so the rollout is held to these rules:

- **Off unless asked.** No `--rollout`, no rollout: the hub answers no `fw.*` message and no console shows it.
- **TLS only.** Every `fw.*` message and the REST firmware reads are refused over plaintext (`tls_required`) in every
  mode, upstream compatibility and `plaintext_ok` keys included; the hub will not start with `--rollout` when it
  serves no TLS. A flash command or an image never crosses the network in the clear.
- **Admins only, and judged.** Uploading, deleting and starting are Admin's; Control and Viewer may watch. Admin is
  given only on the hub's machine. The bootloader entry is the Admin's own write through the hub, judged by the
  role, the installer lock and the device lock like any client's, and recorded in the device's history under the
  Admin's name; it is not one of the hub's own writes. **REST cannot start a rollout or upload an image** (`405`).
- **Verified images only, by default.** An image goes on a device only when the release's manifest lists its name and
  bytes, as for the apps' bundled firmware (above). `--allow-unverified-firmware` lifts that
  for a bench and makes the hub start with a warning; every list still labels each image "not verified". Each image
  is also checked as a UF2 for the device's chip.
- **The firmware guard, with nobody to type.** The hub asks the same guard a person meets when flashing from an app,
  with no typed word and no typed phrase: only an image the guard clears outright goes on, and the writer asks it again
  before writing. So an image stamped above the board's core voltage or above 1.20 V, an unstamped or lower image on a
  1.20 V board (a K340), or an unknown board is refused; a K340 takes only a 1.20 V image.
- **One board, the right board.** No rollout starts while any board is in BOOTSEL; after the bootloader entry exactly
  one must appear, and on RP2350 it must report the device's serial. The image is read back and compared before the
  board restarts.
- **What the hub never does:** write the core voltage (it only reads it back, and a 1.20 V board must read 1.20 V
  after), type `OVERVOLT` or the guard's phrase for anyone, flash two devices at once, mount a bootloader drive, retry
  a flash, or recall the export by itself (a person decides, with the result in front of them).
- **No image bytes in logs, events or the diagnostics bundle.** `rollout.json` (names, serials, hashes, step texts) is
  in the bundle; the images are not.

What a stolen Admin credential could still do: start a rollout of an image the hub already holds, over TLS, verified
(unless the hub was started with the bench switch), on a muted device or with `while_live` spelled out, recorded with
the client's name and alarmed on failure. Revoke the client (`uconsole hub revoke <cid>`) and keep Admin tokens on
machines you control; an unused one expires after 180 days.

## Secrets at rest

| Secret | Where | Rule |
|---|---|---|
| The hub's TLS key | `<store>/tls/key.pem` | 0600, folder 0700; never in a diagnostics bundle, log line, event or `Debug` output |
| Token and API key hashes | `hub-store.json` | 0600; hashes only |
| Webhook secrets, MQTT password and client key | `outbound.json`, `mqtt.json` | 0600; shown as `set` / `not set`; removed from diagnostics |
| The `uconsole web` proxy's pins | `<store>/proxy-pins.json` | 0600, folder 0700; public keys, but the list of hubs is yours |
| Client credentials | the platform's store (above) | the credential holds the token and the pin; a damaged one is no credential (pair again, over TLS only); after Pair Again a token-less record keeps a pinned hub TLS-only until Forget Hub |

On Windows the hub's files get no explicit ACL and inherit the profile's under `%LOCALAPPDATA%`.
