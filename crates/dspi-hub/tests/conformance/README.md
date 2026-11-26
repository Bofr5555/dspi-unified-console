# Hub conformance

Three DSPi Link clients against this hub, on real sockets, through one list of
scenarios. The hub serves the demo device (`FakeDevice::demo`) at handle 0, PIN
access, window PINs.

| Client | Source | Where it runs | How |
|---|---|---|---|
| ours | `crates/dspi-link` | the build container | `ci/run.sh 'cargo test -p dspi-hub --test conformance'` |
| TypeScript | upstream `@dspi/link` 1.2.0, WeebLabs/DSPi-Link `86f10e38`, unmodified | Node 24 from the official image, pinned by digest | `crates/dspi-hub/tests/conformance/run-clients.sh ts` |
| Swift | `apps/apple/DSPi Console/Link/LinkClient.swift` and the files it needs, unmodified | the macOS VM (`macvm`), built with `swiftc` | `crates/dspi-hub/tests/conformance/run-clients.sh swift` |

`run-clients.sh` builds the test binary with `ci/run.sh`, runs its ignored
`serve::conformance_serve_for_other_clients` in a container of the same image (no
USB), and runs the client. The client prints `@ctl <command>` for what only the hub's
side can do (a new pairing window, a change at the device, a revoke, making a client admin, a stop); the
script passes it to the hub through files (`serve.rs`) and writes back the hub's
JSON line. No npm download: Node runs upstream's `.ts` source with
`--experimental-transform-types`, and `ts/register.mjs` maps its `./x.js` imports to
`./x.ts`; the source comes from a local clone with `git archive`, checked against its
tree hash. The Swift run takes the VM's matrix lock (`~/work/matrix/lock`, as
`ci/macvm.sh`) for its build and run, and reaches the hub through `ssh -R` to a port
published on the host's loopback only. `swift/Shims.swift` stands in for three app
names the client's files mention and the run never reaches. Like `ci/macvm.sh`,
`run-clients.sh swift` exits 3 before it builds anything when the VM cannot run the
client (no ssh, or nobody logged in to its GUI). `ci/check.sh --step conformance-ts`
runs the TypeScript client; its Node image is `NODE` in `ci/container/images.conf`.

## The scenarios

| Id | What | ours | TS | Swift |
|---|---|---|---|---|
| C1 | hello: proto 1, the hub's id, `auth: pin`, `cmd` and no `fw_install` in caps, `max_frame` | yes | yes | yes |
| C2 | a wrong PIN is the typed wrong-PIN error and the session waits; the right one then pairs | yes | yes | yes |
| C3 | pairing as admin with a PIN gives control; made admin on the hub's side (the driver's `admin <client name>`, as `uconsole hub role <cid> admin`), its token reconnects as admin and lists the device: handle 0, online, a serial | yes | yes | yes |
| C4 | the token reconnects with no PIN, as a new session with the same role | yes | yes | yes |
| C5 | GET_PLATFORM answers 7 bytes; an unknown handle `NO_DEVICE` | yes | yes | yes |
| C6 | a write: the writer's echo and a second session's copy both carry the writer's session as `origin` | yes | yes | yes |
| C7 | a change at the device (a knob) arrives with `origin` 0 | yes | yes | yes |
| C8 | `device.snapshot` carries the bulk image | yes | yes | yes |
| C9 | a poll at 10 Hz delivers; after `poll.unsubscribe` nothing more comes | yes | yes | yes |
| C10 | a lock: another session's write is `LOCKED`, the holder's is not; after release it is not | yes | yes | yes |
| C11 | a viewer reads, and its write is `DENIED` | yes | yes | yes |
| C12 | a revoke closes with 4002, final; the token then asks for pairing again | yes | yes | yes |
| C13 | a hub stop closes with 4003 | yes | yes | yes |
| K1 | the browser-style keepalive (`{"t":"ping"}`, answered `unknown_type`) keeps a session up | n/a | yes | n/a |

Our client also runs L1's faults (`faults.rs`) against this hub, with a TCP proxy in
front of it where a real network would make the fault: a late reply cannot answer the
next command; a cut connection reads as "connection lost" and reconnects; a hub
restart says 4003 and keeps the token good; a stalled link stays bounded and the
probe ends it; the hub's pings keep an idle session up past the client's silence
limit (F1); a token from another hub is a typed `TokenRefused` (F2); a used window
and the lockout are typed (F3); the token is looked up by the id in the hello (F5); a
hub gone before the connect is an error and no event (F6); a full hub is `HubBusy`
and clears (F8); an unplug leaves the session up and answers `NO_DEVICE`. Not
repeated: duplicate frames and a failed or silent device list (F4), which only a fake
hub sends.

`fixtures.rs` gives every upstream message fixture (`tests/fixtures/link/messages`)
an outcome: each request is sent as written and answered as its table says, each
hub message is matched by what this hub sends, and a fixture added upstream fails
the table until someone decides what it means here.

## Last run

2026-11-07, against `main` f6e74fa9:

- ours: 27 passed, 1 ignored (the serve test); 8 more runs, no failure;
- TypeScript: `DONE 14/14`;
- Swift: `DONE 13/13` (Swift 6.3.3, macOS 26 VM).

The differences from the Swift hub these runs and the reading behind them found are in
`docs/core-merge/HUB.md` section 8 ("From HB7's runs").

## Not run here

- The same clients against the Swift hub itself, and anything on hardware: the
  Mac app's hub needs the DSPi on `macr` (`LinkHubHarnessTests`, XC1). HUB.md's
  section 12 item 3 (the supervisor's blocking on a real bus) is measured there too.
- A browser. The TypeScript client runs in Node with Node's own `WebSocket`, which,
  like a browser's, sends no ping frames and no `Origin`.
