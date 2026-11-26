# Hosting the web console

Three ways to serve uConsole for DSPi's web console, all from one build (`dist/`). Nothing here
publishes anything: **publishing is a release step of its own**; these are build
artefacts and how to serve them.

## The build

```bash
DSPI_WEB_SOURCE_URL=https://…/source apps/web/tools/dist.sh <out-dir>
```

The core in the cross container, the page in the pinned Playwright image; no tests
and no test pages. `DSPI_WEB_SOURCE_URL` is the GPL source link About shows (a release
build refuses to build without one; `mount({ sourceUrl })` overrides it per host).
`<out-dir>` holds:

| File | What |
|---|---|
| `index.html`, `app.js`, `page.css` | the standalone page |
| `manifest.json` | the web app manifest (Add to Home Screen) |
| `favicon.ico`, `favicon.svg`, `apple-touch-icon.png`, `icon-192.png`, `icon-512.png` | the standalone page's icons |
| `dspi-web.js`, `dspi-web.css` | the library, for embedding (`mount()`) |
| `worker.js`, `dspi_wasm.wasm` | one Worker per session; the core (about 6.9 MB, 1.9 MB gzip) |
| `capture-worklet.js` | the measurement microphone's AudioWorklet |
| `autoeq_database.json` | the built-in AutoEQ database (about 5.4 MB, fetched only when AutoEQ is opened) |
| `THIRD-PARTY-NOTICES.txt` | Preact's licence and the Rust crates in the core; serve it with the rest |
| `SOURCE.txt` | (DRAFT) where the source of this build is and that it comes with the console; serve it with the rest |
| `*.map` | source maps (optional) |
| `SHA256SUMS` | of the files above |

Every URL inside is relative, so the directory can sit at any path. File names carry no
hash: serve them with `Cache-Control: no-cache` and a validator (ETag or Last-Modified),
not a long max-age.

## 1. From a hub: `uconsole hub --web`

```bash
uconsole hub --web --console <out-dir>      # a hub on the LAN
```

Three paths:

| Path | What |
|---|---|
| `/` | a small landing page: "uConsole for DSPi" and "Hub", and one line saying which is which |
| `/console/` | this console, which links to `/hub/` ("Hub page") |
| `/hub/` | the hub's own page: upstream's DSPi Link web page, built unmodified and compiled into the hub (`crates/dspi-hub/tools/build-link-web.sh`, `assets/web/hub/PROVENANCE.json`; MIT, in the hub's `/THIRD-PARTY-NOTICES.txt`). It finds its hub at the address it was loaded from, so it needs no setting |

mDNS still says `web=1`: `/` leads to a UI. `/` and `/hub/` have their own policies, the
narrowest each works under: the landing page no scripts at all (`default-src 'none';
style-src 'self'; ...`), upstream's page `script-src 'self'; connect-src 'self'
wss://<the address>; style-src 'self'` (no wasm; `ws://` over plain http). Its hashed `assets/` are cached
`immutable`; the rest `no-cache`. `/hub` and `/console` redirect to their slash.

**TLS.** A hub on the network requires TLS unless told otherwise (`--tls require`, the
default; `off` on loopback), so the console is at `https://<hub>:11915/console/`. The
certificate is the hub's own, self-signed: the browser warns once per hub and certificate;
accept it after comparing the fingerprint with the hub's (`uconsole hub tls fingerprint`).
A plain `http://` request gets the hub's `426` page saying what to open instead. With
`--tls on` or `--upstream-compat` plain `http` is served too, and the console marks that
hub "(unsecured)". [`docs/integrator/security.md`](../../docs/integrator/security.md) (The
browser) has the details.

The hub fills in the console's `<meta name="dspi-hub">` with its own Link address
(`wss://…` over TLS, `ws://…` over plain http), so the page lists that hub first, and a
browser lets the page's WebSocket through the hub's `Origin` check because the page is the
hub's own. One address may hold 16 plain HTTP sockets at once (a browser loads a page's
files over up to six connections, and pre-connects more) and 4 WebSockets that have not
signed in yet. What the hub sends with every console file: the right
`Content-Type` (`application/wasm` for the core), `nosniff`, `no-cache` with an ETag (304
when unchanged), `Referrer-Policy: no-referrer`, and this policy (`wss://` over TLS,
`ws://` over plain http):

```
default-src 'none'; script-src 'self' 'wasm-unsafe-eval'; worker-src 'self';
connect-src 'self' wss://<the address the page was asked at>; style-src 'self';
img-src 'self' data:; font-src 'self'; base-uri 'none'; form-action 'none';
frame-ancestors 'none'
```

No directory listing (only files are keys; `/console/test/` is a 404), no hidden files,
no symbolic links, only the file kinds a build writes. The files are read once, whole,
when the hub starts. Without a console found, `/console/` is a 404 (the hub says so when
it starts; `/` and `/hub/` are served all the same). Without `--console` a hub looks in `$DSPI_WEB_CONSOLE`, then `web/`
beside the `uconsole` binary, then `../share/uconsole/web/` above it; an installer puts the dist
in one of those.

## 2. On this machine: `uconsole web`

```bash
uconsole web [--demo] [--console <out-dir>]   # = uconsole hub --local
```

A hub on loopback only, serving the console at `http://localhost:11916/console/`.
`http://localhost` is a secure context, so this one page has **WebUSB and Link
together**: a DSPi on this machine's USB through the browser (Chromium), and this hub's
devices over Link. The hub holds no USB device itself, so the browser can claim it;
`--share-usb` makes the hub hold them instead (any browser, over Link); `--demo` shares
the simulated device. PIN pairing as any hub (`--open` to skip it on a machine with no
other users). Its store is separate from `uconsole hub`'s, so both can run at once. The
port stays the same between runs, so the page's origin, its hub token and the DSPis the
browser was allowed on USB stay too. The page reaches other hubs through this one: it
relays the page's WebSocket to each hub over TLS checked against the key it pinned there,
so the browser gets real pinning and no certificate warning. That is why it listens on
loopback only.

## 3. A static host or a fleet server (HTTPS)

Serve the directory as static files over HTTPS. What the host must do:

- `Content-Type: application/wasm` for `.wasm` (browsers refuse to stream-compile it
  otherwise), `text/javascript` for `.js`, `text/css`, `text/html; charset=utf-8`.
- A policy at least as wide as: `script-src 'self' 'wasm-unsafe-eval'; worker-src 'self';
  connect-src 'self' wss://<where your Link endpoints are>; style-src 'self';
  img-src 'self' data:`, and `manifest-src 'self'` if the page should offer Add to Home
  Screen. Add the host page's own needs when embedding.
- `no-cache` with a validator, as above.

The standalone page then runs with the demo device and WebUSB (a secure page). It can
list hubs, but a hub accepts a browser's WebSocket only from its own pages (Link spec 4,
the `Origin` rule), and an `https` page may not open `ws://` at all. So a fleet product
**carries Link to its hubs through its own origin**: an endpoint on the server, e.g.
`wss://fleet.example/hubs/<id>/dspi/v1`, that the server connects onwards to the hub as a
native client (no browser `Origin`; the server's own token from pairing). The console
accepts `wss://` addresses for that. The server then decides who may reach
which hub: it is the sign-in and access control, the hub sees one client.

### Embedding with `mount()`

```html
<link rel="stylesheet" href="/static/dspi-web/dspi-web.css" />
<div id="dspi"></div>
<script type="module">
  import { mount } from "/static/dspi-web/dspi-web.js";
  const console = mount(document.getElementById("dspi"), {
    backend: myFleetBackend,          // installations, access, user, hubCredentials, hubDevices
    title: "Acme venues",
    // sourceUrl: defaults to the build's DSPI_WEB_SOURCE_URL
  });
</script>
```

`worker.js` and `dspi_wasm.wasm` must sit beside `dspi-web.js` (or pass `workerUrl`).
`autoeq_database.json` must sit beside `dspi-web.js` too, and `capture-worklet.js` beside `worker.js`.
The `FleetBackend` contract is in `README.md` ("Embedding in a fleet server") and
`src/fleet/types.ts`: hub URLs are the server's own `wss://` endpoints; `hubCredentials`
hands the console the token the server holds, which the console never stores;
`access()` returns `view` for a read-only role. The console never signs anyone in.

### Licence duties when serving it

The console as a whole is distributed under the GNU GPL version 3 (Bofr5555's own
contributions under version 3 or later, with additional terms: [`NOTICE.md`](../../NOTICE.md)).
Whoever serves it to others gives the source: keep the About link (`DSPI_WEB_SOURCE_URL` or
`sourceUrl`) pointing at the exact source of the build, and serve `THIRD-PARTY-NOTICES.txt`
and `SOURCE.txt` with it.
