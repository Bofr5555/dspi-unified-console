# The Rust Link hub: design

WP HB0, `crates/dspi-hub`. HB1, HB2, HB3, HB5 and HB6 build on this
(`PLAN-CLIENTS.md` section 4.2). Spec: DSPi-Link `spec/dspi_link_protocol_spec.md`
1.2, cited below as "spec". References: the Swift hub, the ESP32 hub, `fakehub.rs`.

**What runs today (HB0 to HB3).** The app's session as before (`tests/wp_hb0_app_path.rs`), and now the
network side: the server, pairing, roles, commands, notifications, polls, snapshots and names, over real
sockets (`tests/wp_hb1_server.rs`, `tests/wp_hb1_vreg_wire.rs`, `tests/wp_hb3_meter_feed.rs`), and
advertising and what a browser is served (HB5: `tests/wp_hb5_advertise.rs`, `tests/wp_hb5_web.rs`). The
upstream web page is served at `/hub/` since WB7 (section 9, "WB7 as built").

## 1. Decisions in one place

- **A hub exists exactly while sharing is on** (decided at review). With sharing off, the app drives USB
  directly, as on every platform without a hub. This differs from the Swift console,
  whose hub is always in the path (section 10, R1). Starting and stopping the hub
  hands the devices over (section 5).
- **Plain threads, blocking I/O, no async runtime** (R2).
- **One thread owns each transport.** Nobody else can reach it, so ordering, chunk
  sessions and "no lock across a transfer" follow from the structure, not from care.
- **The hub knows nothing of `dspi-core` unless asked.** `dspi-hub` depends on proto,
  transport and link, and on `dspi-session` without its default features, for the voltage
  rule (section 13). Feature `app` adds the bridge to `DspiCore`, for Qt. A headless
  `uconsole hub` does not link the embedder API.
- **PIN pairing is the default, open access is opt-in** (`AccessMode::default()`).
  This departs from spec 6.1 ("hubs default to `none`"), by owner's decision. A client
  cannot tell except by `hello.auth`, so nothing else changes.

## 2. Threads and who owns what

| Thread | How many | Owns | Blocks on | Ends when |
|---|---|---|---|---|
| `hub-dev-<h>` worker | 1 per attached device | **the `Box<dyn Transport>`**, so the USB claim | job queue, then the transfer | `Job::Stop`, or a panic. Its exit drops the transport |
| `hub-rx-<h>` reader | 1 per device that has a stream | **the `NotificationSource`** | the endpoint, 100 ms at a time | stop flag, or `Disconnected` |
| `hub-supervisor` | 1 | **the `DeviceBus`**; the only thread that tears down a lost device | its channel, with a scan tick (500 ms) | `SupMsg::Stop` |
| `hub-events` | 1 | calling the `HubListener` | its channel | `EventMsg::Stop`, last of all |
| `hub-accept` (HB1) | 1 | the listening socket | `accept` | `stop_accepting` |
| `hub-conn-rx-<id>`, `hub-conn-tx-<id>` (HB1) | 1 per plain HTTP socket (at most 48, HB9); 2 per WebSocket with a session (at most 16) | the read half; the session's outbox and the write half | the socket; `Outbox::pop` | the outbox closes, or the peer goes |
| `hub-refuse-<id>` (HB1) | 0–4, short-lived | a socket past the limits | its request, 1 s at most | it has answered 503 |
| `mdns-sd`'s daemon (HB5) | 1, with `mdns` and while advertised (not on Windows) | the responder's sockets | its poller | `Advertiser` dropped; joined then |
| `dspi-dnssd-advertise` (DISC2) | 1, with `mdns` and while advertised, on Windows | the registration with Windows' DNS Client, interface notifications | its channel, with retry and self-check timers | `Advertiser` dropped; joined after the goodbye: normally milliseconds, about 9 s at worst if Windows acknowledges nothing (review d2 F5) |
| the app's (not ours) | | core's reader, `poll_hotplug` caller | | |

A count for one device, one app and two clients is 2 + 2 + 1 + 4 = 9 threads, idle
while nothing happens.

**Why the worker has the polls too.** `Polls` is a pure scheduler. The worker waits on
`recv_timeout(next_due)`, so a poll takes its turn with the commands and there is no
second thread that could touch a transport (the Swift scheduler has its own lock for
this and holds it across device reads).

**Who may do what to a transport:**

| Transport | Owner in a hub | Who else |
|---|---|---|
| USB (interface 2 claimed) | its worker. `DeviceBus::open` hands it over on the supervisor | Nobody: `DspiCore` goes through `LocalTransport`. Another process gets a busy error, and the hub's own claim fails the same way if another program holds it (`inaccessible`) |
| `FakeDevice` (demo) | its worker, after `HubHandle::attach` | A `FakeRemote` in tests |
| MIDI port, later a sideload fd | its worker, after `HubHandle::attach` | The port's receive callback feeds it, as now |

The firmware's other USB interfaces are the OS's: the hub claims only interface 2, like
`UsbTransport`. **Bootloader flows** (`firmware.rs`) use a different USB device, not a
claimed one, and reach it without the hub. `enter_bootloader` is an admin command like
any other, after which the device leaves the bus (section 7).

## 3. Lock order

Take downwards only. No level is held across a transfer, a join, a thread spawn's
result wait, or a listener call.

| Level | Lock | Protects | Notes |
|---|---|---|---|
| L0 | `DspiCore::Inner` (not ours) | the core | **Held across `LocalTransport` calls**, so it waits on workers. Hence the rule below |
| L0b | `HubInner::advertiser` (HB5) | the responder | Above L1, as `names_saving` is: the device list is read under it. Taken by the supervisor, `publish`, and an upgrade's `Origin` check. Its longest hold is a goodbye on a conflict, about a second at most |
| L0c | `HubInner::locks_syncing` (HUBFLAKE4) | the order of lock changes | Above L1: held across a whole `sync_locks`, its reads of the routers, its registry write and its `device.changed` broadcast, so a sync returns only once what it found is sent. Never taken under L1, L2 or L3 |
| L1 | `HubInner::state` | registry, attachments, sessions | Spawning a device's threads happens here; it does not wait for them |
| L1b | `AuthStore` (HB1) | clients, pairing | Never held into L1 |
| L2 | `Router::state` | inflight, lock table, writers | Per device. Taken on submit and around the call, not across it |
| L2b | `DeviceShared::identity`, `SnapshotCache`, `Polls` | | Per device, leaf except for L4 |
| L3 | `Relay::subscribers` | who follows what | Pushes to outboxes under it, since a push cannot block |
| L4 | `Outbox::state` | one consumer's queue | Leaf. Nothing blocks under it |
| leaf | `HubInner::listener`, `Hub::threads` | | `threads` is held across a whole `stop`, but no hub thread takes it |
| leaf | `HubLimits::state` (P1HL) | limits profiles, PIN hashes, unlocks, holds due | Taken on submit (before L2) and by the device's reader; nothing is taken under it. Its `saving` sits above L1b: a save copies under `state`, then writes through `AuthStore` with `state` released. The PIN's slow hash runs with it released <!-- WP P1HL --> |

**The one cross-crate rule: no hub thread ever calls `DspiCore`.** The core waits on the
hub with `Inner` held; the reverse edge is what would deadlock. The core calls the hub
through `DeviceBroker` with `Inner` released, and `AppLink` calls the core from the
embedder's thread. Tests: `wp_hb0_the_core_lock_is_free_while_the_broker_opens` in
`dspi-core` for the broker's side; `wp_hb0_a_listener_calling_the_core_during_traffic_does_not_deadlock`
for a core `NotifyListener` while the hub is busy.

**Listeners** run on `hub-events` with nothing held, one at a time, and a panic costs
that call. A listener must not block. It is embedder code, not a hub thread in the sense
above: `hub-events` runs no worker, so a `HubListener` calling the core cannot deadlock
(`wp_hb0_a_hub_listener_calling_the_core_during_traffic_does_not_deadlock`), but it holds
up every later event while it does. `Hub::stop` from one is refused with a log line (it
would join its own thread). For the same reason **dropping the last `Hub` on `hub-events`
leaks the hub**: `Drop` calls `stop`, which is refused, and the threads run on. HB6 must not
let a listener own the last `Arc<Hub>`. `Completion` closures run on a worker and must not
block.

**Order** across sessions is the worker's queue and nothing else:
`wp_hb0_commands_run_in_arrival_order_across_sessions` has 16 commands in flight from two
sessions behind a device that holds the first.

## 4. The app's session and the echo rule

`AppLink::attach` installs a `DeviceBroker` in the core (`dspi-core/src/hotplug.rs`).
From then on `select_device`, `scan_devices` and `poll_hotplug`'s enumeration and
reopen go through the broker, and everything else in the core is unchanged. It still
ends in `attach_probed` with a `Box<dyn Transport>`, namely a `LocalTransport`:

```
DspiCore ── LocalTransport::control_* ── Router::admit ── job queue ── worker ── USB
   ▲                                                                              │
   └ core's reader ◄ LocalNotifications ◄ Outbox ◄ Relay ◄ hub-rx (origin) ◄ endpoint
```

- **`caps()`:** `kind: Link`, `chunked_bulk: false`, `remote: false`.
- **`session_id()`:** `Some(1)` (`LOCAL_SESSION_ID`). Network sessions start at 2. 0 is
  "nobody", 0xFFFF the hub's own.
- **`read_attributed`:** `origin` from the relay.

That is the whole mechanism. `dspi_transport::is_own_write(Link, _, origin, session, _)`
is "origin == session and != 0", and `EchoFilter` is skipped over Link (PLAN 1.2). Not
one line of echo logic knows it is in-process. The app differs from a network session
in two ways only: it is exempt from `max_inflight` (it blocks on every command, and has
no way to retry `RATE_LIMITED`), and a RESYNC reaches it as a synthetic
`BULK_INVALIDATED` with source `UNKNOWN`, origin 0, on which the core re-reads
(the same recovery as `HubTransport.swift`).

**Attribution** follows spec 8.3: writers in dispatch order, the oldest within 500 ms,
merged where the firmware coalesces. Two narrowings, both toward "foreign", which only
costs a re-read:

- **A `BULK_INVALIDATED` takes only a bulk write and a `PARAM_CHANGED` only a scalar
  one.** Upstream's single queue can give one client's unconsumed scalar entry another
  client's bulk write. That client then skips the re-read it needs and stays stale. A
  scalar attributed wrongly is harmless (echoes are applied anyway, PLAN 1.2); a bulk
  one is not.
- **Only the hub's own path counts** (`relay::is_hub_path`): `HOST_SET` on USB, `MIDI`
  on MIDI, `BULK_SET` on either. A knob, a preset, or a MIDI host beside a USB hub has
  origin 0. Upstream lists `HOST_SET`, `BULK_SET`, `UART` regardless of the path.
- **A write the firmware never answers on that path takes no place in the queue**
  (`router::SILENT`, WP UPW1): no event at all (`RTA_CONTROL`, the saves, the control
  surfaces' definitions), only events without a source (names, CS_AUX, the signal
  generator), or a notification tagged `PRESET`, `FACTORY` or `UNKNOWN` (preset load and
  delete, factory reset, and what the main loop applies after the request's source is
  cleared: EQ, band bypass, the clock modes, the input source, the DAC mute). Left in the
  queue, such an entry took the next console's change for its own echo, and a client
  that drops its own echoes by `origin` never showed it. Upstream's Windows hub found
  this for `RTA_CONTROL` (f1e18ed1).

**Bulk.** The app reads and writes single-shot like any hub client. The worker turns
`0xA0`/`0xA1` into a chunked session inside one job, where the transport has chunks, so
nothing can land between chunks (spec 9.1.1). That is required on USB everywhere here,
as `UsbTransport` caps a transfer at 4 KB. A client's own `0xA2`/`0xA3` pass through.

**Busy windows.** On USB a stall means either "inside a flash blackout, try again" or
"this build lacks the command", and over Link a stall is final (PLAN 1.2), so only the hub
can tell them apart. It decides by two things together:

- **The opcode:** a parameter read the direct path wraps, that is a registry entry's GET
  (`readout.rs`, `write.rs`) or the platform read (`Policy::retries_stall`). Bulk chunks
  always get the window, inside `read_chunked`.
- **A write has just run on this device** (`BusyWindow`, 1 s since a command that
  `Policy::mutates`). The opcode is not enough. The core reads many of the same opcodes
  bare at connect and takes the stall as its answer: the demo stalls `0x0A`, `0x2D`, `0x7A`,
  `0x86`, `0xA7`, `0xA8` and `0xF9`, all registry reads, plus `0x43`, where
  `probe_band_count` stalls on purpose. On the opcode alone, that connect took 4.2 s.
  A blackout comes from a write, so the second test keeps the case that needs the
  window: a read-back confirming a write that started one.

Everything else gets one transfer and its answer, so an unsupported read costs one
round trip and no client can hold the queue with one. A connect through the hub on the demo
takes about 2 ms (`tests/wp_hb0_connect_time.rs`, alone in its binary so nothing hides it).

What this gives up: a blackout the hub did not cause (a knob, a preset load at the device)
can answer a parameter read `ERROR` once, which a client re-reads past; and a read of a
missing command right after a write costs 600 ms, once. Writes and write-as-read commands are
never repeated.

A hub TIMEOUT reaches the core as `TimedOut { retries: 4 }`, so the core runs no window of
its own on top. On a read outside the window that is one USB timeout (1 s, not spec 8.2's
"at least 2 s": section 8).

**`Policy::mutates`** decides attribution and the busy window from `commands.json`, with one exception:
`0x00`'s IN side reads, though rule 7 makes it `config` (PLAN 1.2). Otherwise each of the
core's extension probes would queue a writer, could claim another client's scalar, and
would hold the busy window open.

**Device lock.** `LocalTransport::lock` never waits (spec 7.7's refusal is immediate), so
the core taking it under `Inner` is fine, and no `SideChannel` is needed.

**Costs.** Two thread hops per command, negligible next to a USB round trip. The core's
`Inner` is held for the queue as well as the transfer: a client's bulk read can delay
the UI thread's *next* call by tens of milliseconds, which blocking-off-UI-thread
already allows for. The wait is bounded at 10 s for a wedged transport.

**A timed-out command.** Its `Job` carries the caller's deadline, and a worker that reaches
it later answers `TIMEOUT` without touching the device, so a write the app was told had timed
out does not land afterwards (`wp_hb0_a_write_whose_caller_timed_out_never_lands`). One
already inside the device cannot be recalled: spec 8.1's "may or may not have executed".
**A timed-out write over the hub is never retried**, by the hub or the core. A network
client's deadline is its own and unseen, so its queued command still runs, as long as its
session is open (section 13).

## 5. Starting and stopping

**Start:** `Hub::start` then `AppLink::attach`, off the UI thread.

1. `Hub::start` with `HubConfig::initial_scan: false`: threads only. Nothing is scanned or
   claimed, so nothing depends on the OS refusing a second claim of what the app holds.
   (A headless hub leaves it true: its first scan finishes before `start` returns.)
2. `set_device_broker(Some)` closes a direct USB connection and returns its serial.
   Demo, MIDI and Link connections stay.
3. `scan_now`: the hub claims the device, past any backoff, and periodic scans begin.
4. `select_device(serial)` through the broker: a full connect, mirror re-read.

**Stop:** `AppLink::stop_hub(&hub)`, off the UI thread.

1. `set_device_broker(None)` closes the app's connection on the hub.
2. `Hub::stop`, in this order:
   1. `stopping` is set under L1, so an attach or open either finished or fails.
   2. `HubInner::advertiser` is taken and dropped, with no lock held, sending its goodbye (HB5); the
      acceptor stops (HB1).
   3. `Relay::close_all`: network clients get close code 4003, after what is queued;
      the app's streams end.
   4. Connections join, each cut off after 1 s if the peer will not take the frame (HB1).
   5. The supervisor joins, so nothing attaches behind this thread.
   6. Each device: registry offline, router shut, reader joined, then the worker. Queued
      commands answer `NO_DEVICE`. One inside the device finishes, bounded by the
      transport's timeout. **The transport drops here.**
   7. `Stopped` goes out, and `hub-events` joins.
3. `select_device(serial)` directly. The interface is free because step 6 finished.

When `stop` returns, every thread is joined, every command answered exactly once
(`Completion` answers `NO_DEVICE` if dropped), and every transport is gone.
`HubHandle`s stay valid and answer `Stopped` or `NO_DEVICE`. `stop` is idempotent, and a
second caller waits for the first.

**A flow in progress** on the app's connection at either handover fails with
`Disconnected`, as with an unplug. An analyser must be restarted, as after any reconnect
(PLAN-CLIENTS section 1). So QD14 should confirm the toggle during a flow.

**A hub stopped behind the app's back** (`Hub::stop` without `AppLink`, or dropped): the
app sees a lost device; `poll_hotplug` closes and reports it; `select_device` answers
`Disconnected` until the `AppLink` is dropped. Nothing hangs
(`wp_hb0_stop_closes_clients_with_4003_and_ends_the_apps_stream`,
`wp_hb0_a_core_call_during_stop_fails_and_does_not_hang`).

## 6. A slow client

A publisher pushes to an `Outbox` and never blocks, so a slow reader costs only itself.

| What | Bound | On overflow |
|---|---|---|
| `Notify` | 256 per subscriber | **Oldest goes.** Its device is owed one RESYNC (reason 0), per device, after the queue has drained. `seq` is taken at push, so the gap shows too (spec 8.3) |
| `Poll` | one per slot | Newest replaces the queued one |
| `Response`, `Event` | 1024, though `max_inflight` and the rarity of device changes keep them far below | The consumer has stopped reading: the queue is cleared and it gets close code 1011 |
| Socket writes (HB1) | a 10 s stall | Close code 1011 (spec 5) |

A vanished client holds its session (and its lock) until the 30 s dead-peer limit (HB1)
calls `close_session`; or until the lock's own timeout, whichever is first (spec 7.7).
Commands from others are refused `LOCKED` meanwhile, as the spec expects
(`wp_hb2_a_vanished_clients_lock_is_released_by_timeout`). The registry and `device.changed`
learn of a timeout on the supervisor's tick.

Inbound: a client that outruns `max_inflight` gets `RATE_LIMITED`, without the device
being touched (`wp_hb0_max_inflight_answers_rate_limited_without_the_device`).

The queue itself is an unbounded channel by design, since admission bounds it: 8 per
session times the session limit, the app adds 1. What a session left queued does not run
once it has closed, or reconnecting would get round that bound.

**What one hostile client can and cannot do** (`tests/wp_hb2_commands.rs`):

| It tries | What happens |
|---|---|
| A flood of commands | 8 in the queue, the rest `RATE_LIMITED` without the device. Another session's command waits for what was queued ahead of it and no more: order is arrival order, there is no priority to game |
| Reconnecting to get more of the queue | Its queued commands are answered `NO_DEVICE` unrun when the session closes |
| Slow commands | Each is bounded by the transport's timeout. Worst, a write and a stalling read in turn: about 600 ms per pair, 8 in flight |
| Not reading | Costs only itself (the table above) |
| Renewing a lock for ever, or handing it to a second session | One budget per device: at most 120 s locked at a stretch, whoever holds it, then at least 30 s with no lock for anyone (`lock.rs`) |
| Filling the attribution queue | At worst a client's echo looks foreign, which costs that client a re-read. It cannot make anyone skip a re-read they need (section 4) |

An admin can still do what an admin can do: reset the device, for one. These limits keep a
session from shutting the others out, not from misusing its role.

The app can be slow too, and gets the same outbox. Its reader only moves packets to
the core's bounded queue, which has its own `lost` rule.

## 7. Unplug and replug

A device is lost when its reader sees `Disconnected`, a command returns `Disconnected`,
the worker ends (a panicking transport included), or a bus scan no longer lists it. The
first three post `SupMsg::Lost { handle, generation }`; a stale generation is ignored.
Teardown runs on the supervisor because a thread cannot join itself.

1. Under L1: the attachment leaves `attached`, and the registry marks it offline, lock
   cleared, handle kept for 60 s.
2. `Relay::device_gone`: the app's transports on that attachment have their outbox closed,
   so the core's reader sees the stream end after what was queued.
3. `device.changed` to network sessions (HB3). Their commands answer `NO_DEVICE`.
4. Teardown as in section 5.

A scan that fails (`DeviceBus::scan` returns `Err`) means "cannot tell", never an empty bus:
nothing departs and nothing attaches until one answers
(`wp_hb0_a_failed_enumeration_detaches_nothing`).

**The app:** `poll_hotplug` finds the stream ended, closes, reports departed, exactly as
for USB. The broker no longer lists it, so nothing reopens early. The core does not
need to know a hub is involved.

**Replug:** the supervisor scans, opens, reads the identity (three tries over about a
second, then a 2 s backoff, so a booting board cannot stall the supervisor for long), and
starts a new attachment at the same handle. Network sessions get RESYNC reason 1
(spec 8.6). The core's next `poll_hotplug` sees the serial arrive and reopens through the
broker, since `selected` is kept and `vid` is the real one.

**A `LocalTransport` is bound to its attachment**, because it holds the old worker's
channel, not a handle. After a replug it fails with `Disconnected`, and can never write to
the new board (`wp_hb0_a_transport_from_before_the_unplug_never_reaches_the_new_board`).
Network commands go by handle and do reach the new board: that is what RESYNC is for.

**Grace ended:** `device.removed`, handle freed. A replacement board during the grace gets
another handle.

## 8. Differences from the references

| Point | Swift hub | Spec | This hub | Why |
|---|---|---|---|---|
| Default access | PIN | `none` | **PIN** | Owner; the Swift hub agrees |
| Devices | one, at handle 0 | many | many | Plan |
| Hub in the path when not sharing | always | not said | no | R1 |
| Thread per device call | helper queue, abandons a late reply | | none; transports are bounded | R3 |
| RESYNC handle after drops | last published | per device | per device | Swift's is wrong with two devices |
| Attribution | one queue | one queue | scalar and bulk apart; own path only | Section 4 |
| `wIndex` | forwarded | "the hub forwards it" | **ignored**, always 2 on the wire | `Transport` has no `wIndex` by design and the firmware ignores it. Clients do send others: `NetworkTransport.swift:341-363` sends 0 for EQ, volume and more |
| Stall on a read | `ERROR`, as its USB layer reports it | `ERROR` | retried only for a parameter read just after a write | Section 4 |
| USB TIMEOUT wait | 2 s, bulk 5 s | at least 2 s, bulk 5 s | one transfer's 1 s outside the busy window; the window's retries inside it | One transport timeout; the queue is not held for a device that has stopped answering |
| Timed-out app write | still runs later | | **skipped while queued; never retried** | Section 4 |
| `fw_install` cap | not advertised | Console hubs | **not advertised** | `PLAN-CLIENTS` section 2, #2; HB1 must leave it out of `caps` |
| Foreign `Origin` | not checked at this ref | refused | refused (HB1) | Spec 4 |
| A second authentication on a session | a new session replaces it | not said | **`bad_request`**, the session stands (open access: the usual `ok`, same session) | A writer per session has one outbox, and nothing here re-authenticates |
| No `dspi-link-1` at the upgrade | not checked | may refuse | **400**, as the bridge | Everything here sends it |
| Connections | no connection limit | | Two budgets (HB9). WebSockets: 16, 4 unauthenticated per address (/64 on IPv6), counted from the upgrade; 503 at the upgrade past them. Plain HTTP (pages, `/dspi/v1/info`, sockets not yet upgraded): 48, 16 per address; past them an idle one is closed for room, 503 only if none is idle | HUB section 9 |
| A token's hash | of the decoded bytes | of the token | of the token as presented | No decoding branch before the comparison; nothing shares the store |
| `0x00` | unlisted, so `config` | | `config`, reads are not writes | Section 13 |
| `EXT_CORE_VREG` raises from the network | passed | not said | **refused** | Owner's rule, section 13 |
| Holding a lock | renew for ever | "when its session closes or the timeout elapses" | also one budget per device | Section 6 |
| A closed session's queued commands | run | not said | answered unrun | Section 6 |
| `RTA_CONFIG` attribution | any recent writer | the session whose `REQ_RTA_SET_CONFIG` it answers | as the spec | Section 4's reasoning: a follower must not take a change for its own |

Record further differences from HB7's runs here.

<!-- WP HB7 -->
**From HB7's runs** (`crates/dspi-hub/tests/conformance/`: our client, upstream's TypeScript client and the Swift
console's client, all passing against this hub; the Swift hub read at `apps/apple`, not run):

| Point | Swift hub | Spec | This hub | Why |
|---|---|---|---|---|
| `hub.rename` | renames the hub | 7.9, admin; no cap names it | renames the hub (admin), `denied` (others); cap `hub_stats` (WP P2HS, `docs/integrator/hub-api.md` section 15) | Was `unsupported` until P2HS |
| `hub.stats` | `ok`, every field absent | 7.9, admin: sessions, uptime, per-device counts | the spec's fields, plus `serial` per row (admin), `denied` (others) (WP P2HS) | Was `unsupported` until P2HS |
| `auth.revoke` of an unknown `cid` | `ok` | not said | **`bad_request`**, "no such client" | Says the revoke did nothing |
| The bridge's requests (`hub.config.*`, `hub.wifi.*`, `hub.auth.set`, `hub.update`, `hub.restart`, `hub.reset`) | `unknown_type` | `unknown_type` for an unknown type | `unknown_type` | Same |
| `{"t":"ping"}`, the browser keepalive of the TypeScript client | `unknown_type` | | `unknown_type`, which that client counts as a live round trip | Same |

Section 12, item 4 (`origin` narrowed to the own path): none of the three clients minds. Each saw its own write's echo
and another session's copy carry the writer's session, and a change at the device carry 0. The echo rule above the Swift
client (`NetworkTransport.swift`) was not exercised: it needs the app.

**For HB7, on hardware:** the supervisor is the single blocking point for the bus (section
12.3). Decided: it stays until hardware says otherwise, so measure how long a wedged open or
identity read delays another device's loss. Also check the 1 s `BUSY_WINDOW` against a real
preset save.

## 9. Interfaces for later WPs

File ownership follows `PLAN-CLIENTS.md`. Implementations an owner may replace are in
place where the app's path needs them; each file's header says what is left.

**HB1, `server.rs`, `auth.rs`, `store.rs`.**
- Input: `HubHandle::{open_session, close_session, submit, devices, lock_*}`, and
  `Session::outbox()`, `Pop`, `Outbound`.
- Each connection: reader thread decodes frames (`dspi_link::frames`) and calls `submit`
  with `Completion::to_outbox(tag, outbox)`. Writer thread loops on `Outbox::pop`, encodes,
  sends, pings. The `seq` of `Notify` comes from the outbox. POLL's is the writer's own.
- Split tungstenite as `dspi_link::socket` does: two `WebSocket`s on clones, sharing a
  write lock (`Pipe`, role server). `Pipe` is private there, so HB1 either asks for it to be
  exposed or keeps a copy.
- **Origin:** `origin_allowed(header, &OwnNames)`, in the upgrade's header callback,
  answering 403. `OwnNames` is built per upgrade from the machine's interface addresses,
  the mDNS name and loopback, with the bound port. **Never from `Host`** (DNS rebinding,
  spec 4). No header passes; `null` does not. `dspi-hub` needs an interface listing;
  `if-addrs` is already in the lock through `mdns-sd`.
- Peek at the HTTP request first: `/dspi/v1` upgrades, `/dspi/v1/info` answers, others go
  to `web::asset` only with `web` on.
- `hello.auth` comes from `NetworkConfig::access`; `caps` must leave out `fw_install`.
- Stop: `stop_accepting`, then `join` after the hub closes the outboxes (section 5).
- Revoke: `AuthStore::revoke`, then `outbox.close_with(CLOSE_REVOKED, ..)` and
  `close_session`. Role change: `Session::set_role`.
- Limits: the session limit answers HTTP 503 at the upgrade (`LinkError::HubBusy`).
- `device.list` and the events: `RegisteredDevice::to_json` is the spec 7.4 object, and
  `HubInner::publish` already broadcasts `RegistryEvent::to_json`, so nothing here waits on HB3.
- **Security rules, all binding on HB1:**
  - Set tungstenite's `max_message_size` and `max_frame_size` from `limits.max_frame`. Its
    defaults are many megabytes, for a peer that has not authenticated.
  - The connection limit counts **pre-auth sockets too**. A socket unauthenticated for 30 s
    closes with 4001 (spec 5). Otherwise anyone on the LAN fills the slots.
  - `Origin`: with `web: false` the hub has no page, so **every** `Origin` is refused. With
    `web: true` the scheme must be `http` as well as host and port matching (`https://` a
    hub does not serve is not its own page).
  - Compare token hashes in **constant time**.
  - Cap the client-name length, and the number of paired clients. Both are stored on disk and
    listed in UIs, on a client's say-so.
  - **No fixed PIN.** Window PINs only (`pin_source: "window"`); the spec's `fixed` is for
    hubs with no screen, and every hub here has a UI or a CLI to show one.
- New dependencies to state: `tungstenite` and `socket2` at `dspi-link`'s pins, `sha2`, a
  CSPRNG for tokens and PINs. `dspi_link::messages` lacks `Serialize` on hub-side
  replies: ask for it, or keep hub-side writers in `server.rs`.

**HB1 as built, for HB5, HB6 and HB7.** The header of `server.rs` has the table of what a hostile
socket can cost.
- `Hub::network()` gives a `NetworkControl`: `port`, `auth` (`begin_pairing`, `clients`), and `revoke`
  and `set_role`, which reach live sessions. Use those two, not `AuthStore`'s: alone they leave the
  session open. `begin_pairing` returns a `Result`, since a PIN without random numbers is worse than none.
- **One owner of the store.** `Hub::start` opens it once into an `AuthStore`, which `Server::start` is
  handed, and `StoredNames` is HB3's `NameStore` through it. Clients and device names share a file, so two
  writers would each put back what the other changed.
- `NetworkConfig` adds `bind` (tests use loopback), `mdns_name` (**HB5 sets it**, or a page opened at the
  `.local` name is refused) and `limits`.
- `hub.stats` and `hub.rename` answer `unsupported` to an admin: not in this WP's entry. `name` in the
  store has no caller yet, and `HubHandle::name` is fixed at start. (-- WP P2HS -- both answered now;
  `hub.rename` changes `HubHandle::name` and the stored name.)
- Switching `NetworkConfig::access` between starts revokes every token (spec 6.2).
- Residual, accepted (numbers as of HB9): a host with four or more addresses (IPv4, or IPv6 /64s) can
  hold all 16 WebSocket slots for up to 30 s at a time (4 unauthenticated each), and one with three or more
  can hold all 48 plain HTTP slots with responses it reads slowly (up to `response_timeout`, 30 s); silent
  or trickling sockets it holds are closed for anyone's new one. The per-address caps are the only defence
  there. One dual-stack machine is two addresses and cannot do either (`hostile_lan`
  `atk_hb9_a_dual_stack_host_cannot_keep_a_third_peer_from_pairing`). Those already authenticated are
  untouched. The 10 s stall's close 1011 usually cannot be sent, as the failed write may have cut a
  frame in two: the socket just closes.
- Close order: close frame, half-close, up to `close_grace` for the peer's reply, so what the peer has sent
  does not turn the close into a reset that loses the frame.

**HB2, `router.rs`, `policy.rs`, `lock.rs`, `relay.rs`.** Done; section 13. What others need:
- HB1: `lock::handle_message(&hub, &session, &json)` answers `lock.acquire` and `lock.release`
  and is `None` for anything else. `HubHandle::denied(role)` is `policy.denied`.
  `router::MAX_PAYLOAD` is `limits.max_payload`, `hub::MAX_INFLIGHT` is `limits.max_inflight`.
  Also advertise cap `lock`.
- HB3: `Polls::next_due` must be `None` exactly when nothing is subscribed, as its doc says:
  the worker asks `others_poll` only otherwise.
- Keep: admission on the submitter's thread, execution order only from the worker's
  queue, the split of attribution by what a packet can answer.

**HB3, `registry.rs`, `snapshot.rs`, `polls.rs`.**
- `RegistryEvent::to_json` and its broadcast in `HubInner::publish` are done (HB0). HB3
  adds `inputs` and `wire_version` to `RegisteredDevice::to_json` if it reads them.
- Names: `rename` and the store, adoption through the worker as `INTERNAL_SESSION_ID`.
  `board_name` is read in `device::read_identity`, which needs an additive edit in
  HB0's `device.rs`; list it in the report.
- `SnapshotCache`: `note_packet` runs on the reader and must not block. Returning `true` has
  the reader call `DeviceShared::queue_snapshot_read`, coalesced to one pending per device (an
  ask after a read has started gets another, so the image is never older than the last
  ask). The worker reads it in queue order and calls `set_bulk`. No edit in `device.rs`.
- `Polls`: fill the scheduler. `next_due`, `take_due`, `finished` already run on the worker.
  `PollJob` carries the session's `Arc<Outbox>`, so `finished` calls `push_poll` directly.

<!-- WP HB3 -->
**HB3 as built, for HB1 and HB6.** The headers of `polls.rs` and `snapshot.rs` have the reasons.
- **Call these, not `device.polls` or `device.snapshot`:** `HubHandle::poll_subscribe`,
  `poll_unsubscribe`, `snapshot` and `rename`. `poll_subscribe` wakes a worker asleep with no polls.
  `PollRefused::code` and `SnapshotRefused::code` are the `err` codes. `POLL_MAX_HZ` and
  `POLL_BUDGET_BPS` are the `hello.limits`.
- **A `PollJob` is one group's read, not one subscriber's**, so the outboxes sit on the
  subscribers and `finished` fans out. `take_due` gives at most one, so a command waits for one
  read at most. `finished` takes the transport's `Result`, for `poll.error`'s code.
- **One budget per device, shared:** 200 000 B/s and 320 reads/s, the group costing once. The
  reads limit is enforced at `take_due` too, so subscribing in a loop cannot exceed it.
- **Polls stand back from a client's own `0xA2`/`0xA3`** for 500 ms, since any other request
  tears down its chunk session. A snapshot read does not (it is rare, and a torn client restarts).
- **Names:** `HubConfig::names` is a `NameStore`, which a networked hub gets from its store (`StoredNames`) unless given another. A stored name is
  saved with L1 released; `names_saving` sits above L1 and only keeps saves in order. A board's
  `NOTIFY_EVT_DEVICE_NAME` goes from the reader to the supervisor (`SupMsg::BoardName`), since
  the reader must not take L1 or touch a disk.
- **Snapshot:** warm from attach on a networked hub, cold on an in-process one. `status_b64` and
  the device object's `inputs` and `wire_version` are left out, which the spec allows.

**HB5, `advertise.rs`, `web.rs`.** `Advertiser` wraps `MdnsAdvertiser` (on Windows `DnsSdAdvertiser`, DISC2) behind feature `mdns`.
Start it after the server binds, with the bound port. Update it from `HubInner::publish`. `Hub::stop`
drops it first, on the caller's thread, since the goodbye can block for a second. `web::asset` is called
from HB1's HTTP path, and only with `web` on.

<!-- WP HB5 -->
**HB5 as built, for HB6, TM11 and HB7.** The headers of `advertise.rs` and `web.rs` have the reasons.
- **Advertising needs feature `mdns`** on `dspi-hub` (off by default; a build without it advertises
  nothing). HB6 and TM11 turn it on for a hub on the LAN.
- **`NetworkConfig::advertise`:** `Lan` (default) advertises on every interface, except that a hub bound
  to loopback stays silent; `LoopbackOnly` is for tests; `Off`. With advertising, `Hub::start` sets
  `mdns_name` to `dspi_link::discovery::mdns_host_name(hid)` unless it is given, so a page at
  `http://dspi-<12 of the hid>.local:<port>` passes the `Origin` check. After a host name conflict the
  check follows the name the responder holds (`HubHandle::advertised`), and the old one is refused.
- **TXT:** `v`, `hid`, `kind=console`, `auth`, `n` and `d` of the devices online, `web=1` with `web` on,
  and `rev`, which changes with each announcement. `rev` is ours, not the spec's (unknown keys are ignored,
  spec 3.1): mdns-sd's browser shows stale data when an old value comes back without it. `d` is dropped
  past 253 bytes, one TXT string, since mdns-sd refuses the whole service past that, though the spec's
  limit is 400 for the record.
- **Changes are paced**, at most one per 2 s, the latest winning. A name conflict re-registers under
  `Name (2)`, `(3)` and so on, made by `Advertiser`, not by mdns-sd, whose own rename keeps answering for
  the old name with this hub's `hid` (mixed case) and does not defend the new one. Dots and backslashes in
  the instance name become `-` and `/`, as mdns-sd never sees a conflict over them. `hello` keeps the
  configured name.
- **mdns-sd 0.21.7 faults this works round**, worth reporting upstream (the owner's call): renames looked
  up in lower case and stored as given; `name_change` splitting an escaped dot; names with `\.` never
  compared equal to what is read off the wire; a returning TXT value not reported by its browser; a
  TXT string past 255 bytes refusing the service. CG2's `MdnsSource` clients get the browser one, which
  `rev` covers for our hubs only.
<!-- WP DISC2 -->
- **On Windows the responder is Windows' own** (`dspi_link::discovery::DnsSdAdvertiser`, picked by
  `LanAdvertiser`): `DnsServiceRegister` in the DNS Client service, so advertising needs no firewall rule
  (the hub's TCP port still does). It differs from mdns-sd as follows (measured in the VM, DISC2):
  - A TXT change is a goodbye and a new registration, 1.2 s apart (`REREGISTER_GAP`), since Windows sets
    the cache-flush bit on the shared PTR record and mdns-sd browsers would otherwise drop the hub; the
    clients' 3 s removal grace hides the gap. `rev` is still sent and does no harm.
  - On a name conflict with another machine Windows renames to `Name(1)`, and `Advertiser` then takes
    `Name (2)` as above. Against another registration on the same machine Windows renames nothing and keeps
    answering for the first; the advertiser's self-check (2 s after each registration, then every 30 s)
    sees another `hid` and registers as `Name (2)` itself. A renamed registration is withdrawn by its new
    name: by the old one nothing goes, and Windows answers for it after the process has ended.
  - Windows drops registrations when an adapter goes down, with no callback (upstream's app loses its too).
    The advertiser registers again 3 s after the last interface change (`NotifyIpInterfaceChange`), and the
    self-check catches anything else: back on the LAN 8–10 s after the adapter returns.
  - Stopping a browse (`WinSession` dropped, from the Qt UI thread in `dspi_link_browse_stop`) waits up to
    2 s for dnsapi's cancelled callback; it comes within milliseconds in every run measured.
  - `AdvertiseScope::LoopbackOnly` is refused, so `tests/wp_hb5_advertise.rs` does not build for Windows.
- **Tests stay on loopback.** mdns-sd joins the mDNS group on every interface when its daemon starts,
  before any can be turned off, so a test sends IGMP and MLD membership reports there, though no mDNS
  record. `tests/wp_hb5_advertise.rs` and `wp_hb5_web.rs` pass in a container with only `lo`
  (`podman run --network=none`).
- **Web:** `web::asset` serves only what is compiled into its table, `/` being `/index.html`, never under
  `/dspi/`, with `nosniff` and `no-cache`. HB5 served a placeholder page of ours there; since WB7 `/` is a
  landing page leading to the console and to upstream's page (section 9, "WB7 as built").
- **The upstream bundle** (as HB5 left it; WB7 bundles it at `/hub/` on the owner's decision of
  2026-11-08, from this pin), not included then (the owner would rather have one base than ship an external
  project). If approved it goes in as its built `dist/` as entries of `web::FILES`, its licence in
  `NOTICE.md`. Pin: WeebLabs/DSPi-Link commit `86f10e38dd760d01d4f589e4b606a5250b2d523d` (2026-11-02,
  upstream `HEAD` on 2026-11-07; no web release tag exists, only `bridge-v0.1.0`), `js/apps/web` is
  `@dspi/web` 1.2.0, MIT. `git archive 86f10e38 js | sha256sum` =
  `19ab666a0cbd4d02ccf03dde63cbc89ee9a8b921066be60f5af2df57cb77a13a`; trees `js` `128dca29d020ff7d709c35f422573fef0083df7e`,
  `js/apps/web` `426818fcb1c09b9ad3e85354a8e6f9c65cb331de`; `js/package-lock.json` blob
  `ffe94e4ac22c9b6554c849b5f319689ade8e6d44`. It is source: `npm ci && npm run build` in `js/` (Vite 7,
  TypeScript 5.9) makes `dist/` with `index.html` and the bridge's `setup.html`. The built files' hashes are
  not taken, as that needs npm's downloads, which nothing here needed yet.
- **By hand in a browser:** `wp_hb5_serve_the_placeholder_for_a_browser` (ignored) serves the demo device on
  `127.0.0.1` with no advertising, `DSPI_HB5_PORT`, `DSPI_HB5_MDNS_NAME` and `DSPI_HB5_SERVE_SECS` from the
  environment. Run in a `--network=host` container, with headless Chrome given
  `--host-resolver-rules="MAP <name>.local 127.0.0.1"`, the page reached the hub at `127.0.0.1`,
  `localhost` and the `.local` name, and was refused at another name for the same address, as a DNS
  rebinding page would be.

**HB6, `dspi-capi/src/fns_hub.rs`.** `Hub::start(HubConfig)`, `Hub::stop`, `HubListener`
as a C callback, `HubHandle::devices`, `AuthStore`'s methods, `AppLink::attach` and
`stop_hub` (feature `app`), `UsbBus` as `HubConfig::bus`. All of them block: off the Qt
UI thread. A `dspi_hub_*` listener must not call `dspi_hub_stop`, and must not own the last
reference to the hub (section 3). Set `initial_scan: false` when an app is to attach.

<!-- WP HB6 -->
**HB6 as built, for QD14, TM11 and SW-M3.** The header of `fns_hub.rs` is the contract; the feature is `dspi-capi/hub`.
- **`FfiHub` is a handle that outlives each run,** with `Hub` in one slot and nowhere else. A read and the listener hold a
  `HubHandle` or a `NetworkControl`, whose drop cannot stop a hub, so `hub-events` never owns one. There is no `Arc<Hub>`. A
  stop takes `Hub` out of the slot before it waits, as the listener may be reading.
- **In the callback only `dspi_hub_status`, `_devices` and `_clients` work.** The rest answer `false`, -1 or `in_callback`.
- **Settings apply at a start.** A running hub has its name, port and access for life: stop and start to change them.
- **A change of access mode revokes every token** (section 9): `dspi_hub_saved_settings` gives `tokensRevokedIfPin` and
  `tokensRevokedIfOpen` beforehand, by the rule `AuthStore::new` applies, and the start's `tokensRevoked` says what it cost. QD14
  must warn when the number for the chosen mode is not 0.
- **Start does not scan.** Nothing is claimed until the port is bound and the listener is in place, whatever the options.
  With an app: `HUB_OPT_APP`, then `dspi_hub_attach_app`.
- **Logs.** The library's logger holds `tungstenite` at debug, since its trace level prints whole frames, PINs and tokens
  included. A host that installs its own logger, or a binary with its own (`uconsole hub`), must do the same.
- **Not offered yet:** the web bundle and `mdns_name`, HB5's. Add them to `config_for` and as options when HB5 lands.
- **QD14 has to add `hub`** to the `dspi-capi` feature lists that build the plain app: `apps/desktop/packaging/*`, the Arch
  `PKGBUILD`, `ci/github-actions-build.yml` and the `windows-msvc` step. The smoke builds have it.

<!-- WP WB7 -->
**WB7 as built: three paths** (owner, 2026-11-08). `/` is a landing page (`assets/web/index.html`,
`landing.css`; no scripts) with two links, "uConsole for DSPi" (`/console/`; "DSPi Console" before the rename) and "Hub" (`/hub/`), and a line saying
which is which; HB5's placeholder text is gone; `web=1` stays. **`/hub/` is upstream's DSPi Link web page**
(`js/apps/web`, `@dspi/web` 1.2.0, MIT) at the commit the conformance run pins (`86f10e38`, `js` tree
`128dca29`), built by `tools/build-link-web.sh` from `git archive` (tree checked) with `npm ci --ignore-scripts`
from its own lockfile and Vite in `NODE` (images.conf, by digest); reproducible (`--check` rebuilds and
compares byte for byte). Output committed in `assets/web/hub/` (index.html and the three files it loads, 80 KB;
`setup.html`, the bridge's Wi-Fi page, left out) with `LICENSE` and `PROVENANCE.json` (every file's SHA-256, which
`web::tests_wp_wb7` holds the compiled bytes to), and the generated table `src/link_web.rs`. It finds its hub at
`location.host`: no configuration. Policy for `/hub/`: `script-src 'self'; connect-src 'self' ws://<Host>;
style-src 'self'` (its inline styles are set through the DOM, which CSP does not stop); for `/`: no scripts.
`/hub/assets/*` (hashed names) are cached `immutable`. `/THIRD-PARTY-NOTICES.txt` carries DSPi-Link's MIT text.
WB7 had `ServerLimits::max_preauth_per_peer_web` = 8 in place of 4 while `web` was on (one browser was
answered 503 for `app.js` with 4); review c6r found that two addresses then held all 16 slots (WB7-1).
HB9 replaced it with the two budgets below.

<!-- WP HB9 -->
**HB9 as built: page loads apart from the WebSocket pre-auth budget** (review c6r WB7-1, coordinator's
decision). Every socket starts as **plain HTTP** and is counted in that budget only: `max_http` 48, of which
`max_http_per_peer` 16 from one address (one thread each). A request that upgrades moves the socket into the
**WebSocket** budget at the upgrade, and only there (`Shared::promote`): `max_connections` 16, of which
`max_preauth_per_peer` 4 unauthenticated from one address, web on or off; past either the upgrade is
answered 503 (`HubBusy`). An HTTP socket is `Idle` (waiting for a request to start, since when), `Head`
(a head has started) or `Busy` (being answered). Past an address's share its own idlest socket is closed to
make room (`Idle` before `Head`, oldest first); past the total, the idlest of the address with the most;
only if every candidate is `Busy` the answer is 503 on a refusing thread, as before. Times: the first
request's head within `head_timeout` (10 s) of accept; a later one must start within `keepalive_idle` (5 s);
any head must end within `request_timeout` (3 s) of its first byte; an answer is written with each write
progressing within `write_stall` (10 s) and all of it within `response_timeout` (30 s), no longer "until
`auth_timeout` from open". `auth_timeout` (30 s) now runs from the upgrade. **Keep-alive** only when the
request asks for it (`Connection: keep-alive`, which browsers send) on an HTTP/1.1 GET without a body; the
answer then says `Keep-Alive: timeout=5`, and anything else is answered `Connection: close` as before, so a
client that reads to the end of the stream is unaffected. `NetworkControl::connections` counts both kinds;
`websockets` the WebSocket budget's. `hostile_lan` `atk_hb9_*` and the e2e
"HB9: four browser windows on one address" (four contexts, three rounds, nothing 503 or failed) cover it.
The e2e's hosting groups each start their own hubs on free ports (`ownHub` in `hosting.spec.ts`), so
`--repeat-each` copies in parallel workers no longer share one (WB7-3).

**WB7: the web console.** `NetworkConfig::console`
(`Option<Arc<web::Console>>`) serves `apps/web`'s built dist at `/console/` while `web` is on. **The files come at run time**, from a directory
the embedder names (`Console::load`), read whole once at start, never per request: the Rust build needs no
Node, the console and the hub version apart, and a request path is only a key, as for `FILES` (only plain
names, served kinds, no hidden files, no symlinks, at most 512 files and 64 MB; directories are no keys,
so no listing). The console's `index.html` gets `<meta name="dspi-hub" content="ws://<Host>">` (the
request's `Host`, only when the `Origin` rule's own names accept it; else left empty) and
`<meta name="dspi-hub-page" content="/hub/">`. Every console response: CSP `default-src 'none'; script-src
'self' 'wasm-unsafe-eval'; worker-src 'self'; connect-src 'self' ws://<Host>; style-src 'self'; ...
frame-ancestors 'none'`, `nosniff`, `no-cache` with an ETag (304), `Referrer-Policy: no-referrer`;
`/console` redirects to `/console/`. The `Origin` check is untouched. The C ABI (HB6) does not offer
`console` yet; a desktop app that wants it adds a path option in `config_for`. The Terminal: `uconsole hub
--web [--console DIR]`, `uconsole web` (= `--local`: loopback, port 11916, no USB bus unless `--share-usb`).

**TM11.** `dspi-hub` without `app`. **SW-M3.** A UniFFI wrapper over the same; `DeviceBroker`
is a plain Rust trait today and needs no export if the wrapper's `AppLink` stays in Rust.

<!-- WP SW-M3 -->
**SW-M3 as built.** `dspi-hub/src/ffi.rs`, feature `uniffi`, namespace `dspi_hub`, carried by `dspi-ffi`'s feature
`hub`, which only the Mac's slices build (`apps/apple/build-xcframework.sh`): no hub on iOS or in Android's Play build.
`HubHost` is `FfiHub` one for one (one object, many runs; `Hub` in one slot; reads and the listener hold a `HubHandle` or a
`NetworkControl`; calls but reads refused in the listener, `InCallback`), with typed records and `HubHostError` in place of
JSON. `attach_app` takes the app's `DspiCore` (a type of the `dspi_core` namespace), so `AppLink` stays in Rust; one addition
for demos and conformance runs, `demo_turn_knob`. Mac app: `Core/CoreHub.swift` (`CoreHubService`) runs it while sharing is
on, as Qt does (R1): the local core opens USB itself (`CoreBackedTransport` `Link.usb`) and is attached on start; Bonjour,
power assertions and preferences stay Swift. The Swift hub is the opt-out (`-CoreTransport NO`, `-CoreHub NO`).

**P1HA, `apply_guard.rs`, for P1SA and P1HL.** `apply.begin/confirm/cancel/status` and the
`apply.ended` event (`docs/integrator/hub.md`, "Guarded apply"); cap `apply_guard`, `v: 1`.
`apply_guard::handle_message` is chained after `lock::handle_message` in the connection's reader.
The timer runs on AUSB2's `hub-system` thread (`system_poll_loop` wakes at the first guard's
deadline, and on a message while the hub is not stopping); a closed session and `Hub::stop`
revert at once. The revert is one `0xA1` as `INTERNAL_SESSION_ID`, let past any lock by
`Router::pass_revert` (consumed by that admit): P1HL's lock scope and limits must let it pass too.
It goes to the online device with the captured serial only. `ApplyGuards` is a leaf lock.

## 10. Considered and rejected

- **R1. Hub always in the path (Swift's way).** One code path, and the app is always a
  session. Rejected: Android and iOS have no hub, so the core's direct path stays
  regardless and now has a second one beside it for most Qt users who never share; every
  non-sharing user would carry the hub's risk and a Link-kind transport's different
  retry rules. The cost of this choice is the handover in section 5, and that the
  app's path is tested in two modes. Open to the reviewer.
- **R2. An async runtime.** No crate in the workspace has one; `tungstenite`, `dspi-link`
  and `mdns-sd` here are all blocking; `Transport` is blocking and must be on its own
  thread regardless. Thread counts are small (section 2). A runtime would bring its own
  rules for no gain at this scale.
- **R3. A helper thread per device call with an abandon timeout (Swift's `runWithTimeout`).**
  A transfer abandoned on a live device leaves it running beside the next one: exactly
  what the worker exists to prevent. `Transport` already promises bounded calls, which USB,
  MIDI and Link keep. A `Transport` that breaks the promise wedges its own device's queue,
  and `LocalTransport`'s 10 s keeps that from wedging the app.
- **R4. `Arc<Mutex<Box<dyn Transport>>>` shared by sessions.** No worker thread, but a
  queue lock held across a transfer: this makes "strict arrival order" a matter of mutex
  fairness, and each caller would block its own socket's reader during another's bulk read.
- **R5. `DspiCore` as the owner, the hub as a guest on it.** The obvious reuse: the hub calls
  the core. Rejected: one core owns one connection, so two devices would need two cores, and
  the hub would write through `Inner`. The core's mirror and its per-opcode API sit on a
  surface the hub must tunnel byte for byte (spec 1.1). It also makes a headless hub link
  the embedder for nothing.
- **R6. A new `TransportKind::Local`.** Precise, but every `match` on the kind changes
  (`is_own_write`, retries, RTA fallback), and each is a place to get the echo rule wrong.
  Using `Link` means the rule has one form, and it is the one that is already tested
  against real hubs. `remote: false` says the rest.
- **R7. The app through loopback WebSocket.** No new transport, and the same bytes as a
  client. Rejected: a local PIN or an exempt token to handle, a port the app depends on,
  a framing cost on every meter read, and Link's reconnect supervisor around a local
  object. Nothing is gained by serialising to oneself.
- **R8. `dspi-core` depending on `dspi-hub`, with `DspiCore::start_hub`.** Rejected: it pulls
  `tungstenite`-server code and more into the mobile graph unless feature-gated, and
  section 1's dependency direction is what keeps the core's iOS build clean. The broker is
  all the core needs to know.
- **R9. The broker's fields on `Inner` directly.** `Inner` is shared. They live on
  `HotplugState`, inside `hotplug.rs`, which this WP owns.
- **R10. Per-session relay threads (Swift's serial queue per session).** The connection's
  writer thread already is one, and the app's reader likewise.
- **R11. A bounded job queue.** It would need a blocking or a failing send on a session's
  reader; admission (`max_inflight`, `LOCKED`, role) already bounds it, with the right
  status codes.
- **R12. Dropping RESYNC for the app and relying on firmware `seq` gaps.** A gap shows only
  when a later packet arrives; a tail loss would go unseen until then.

## 11. The web rule

`PLAN-CLIENTS` section 1, "**Do not make the web harder**": session and core code takes time
through the clock seam, keeps device I/O behind `Transport`, adds no OS call outside a gated
module, and no thread where a caller's poll will do. The core side of this WP keeps to it:
`DeviceBroker` adds no thread, no clock and no OS call to `dspi-core`, and costs nothing with no
broker set. `dspi-hub` is outside that rule's reach: a hub needs threads and sockets, is not in
the web graph, and no session or core code depends on it. Its pure parts (`Registry`,
`LockTable`, `Router`) take the clock as an argument anyway.

## 12. For the reviewer

Things worth attacking, as the author sees them:

1. R1: sharing off means no hub. If reviewers want the Swift shape, `AppLink` becomes
   permanent and `set_device_broker` is called once; the rest stands.
2. `Inner` held across the queue (section 4, Costs). A viewer's flood of slow reads could
   slow UI calls. `max_inflight` bounds it to 8 per session.
3. The supervisor does the bus's blocking work. A wedged open or identity read delays the
   handling of another device's loss by seconds, not forever. **Decided: left as is until
   hardware says otherwise; HB7 measures it.** A short-lived attach thread is the fix.
4. Attribution is still a hint. Section 4's narrowing is a deviation in client-visible
   `origin`: HB7 should confirm the TypeScript and Swift clients do not mind.
5. `wIndex` is ignored, not forwarded (section 8). `Transport` cannot carry it, and the
   firmware does not look at it.
6. Section 4's busy window is narrower than "the opcodes the direct path wraps": those opcodes
   alone made a connect take 4.2 s, so a write must also have just run.

## 13. Commands, roles and the voltage rule

**Where things are decided.** On the submitter's thread, without the device (`Router::admit`):
size (`TOO_LARGE`), role (`DENIED`), lock (`LOCKED`), then `max_inflight` (`RATE_LIMITED`). On the
worker, in queue order, with the device available (`Router::before_exec`): a closed session's
leftovers, the analyser's STOP (spec 7.15) and the voltage rule. The worker passes what they need
as `ExecUnit`, so `before_exec` takes no router lock across a transfer.

**How `0x00` is classed, for client authors.** `REQ_EXTENSION` is not in upstream's table
(`policy/commands.json`), and spec section 10 rule 7 makes an unlisted command `config`. So:

- **Admin only, both directions, for every sub-command**, reads included. `EXT_LIST`,
  `EXT_NOTIFY_FEATURES` and the voltage status are no exception.
- A viewer or control client is answered `DENIED`. Treat a refused read as "this device has no
  extension channel" (`dspi-core/src/ext.rs` does), and hide the features behind it.
- `policy.denied` lists `[0, 0]` and `[0, 1]` for those roles, although upstream's list has no
  unlisted codes, so a client can grey the controls out first.
- The IN side only reads (`ext_handle_get` in the firmware), so a read of `0x00` is not a write:
  it takes no attribution and opens no busy window (`Policy::mutates`).
- This is what the table says today, not a judgement of each sub-command. If control users should
  have the crossfeed pair map, say, that needs a per-sub-command class, which the table's format
  does not have.

A direction the table does not list for a code is `config` too, as in the Swift hub.

**The voltage rule. Provisional, for the owner to confirm.** The owner's rule (`PLAN-CLIENTS`
CG14, `dspi_session::overvolt`): a target above 1200 mV and above the unit's current voltage needs
the typed word `OVERVOLT`, an unreadable current counting as needing it, with no host-side record.
It is judged in the client's core, so a raw `EXT_CORE_VREG` from a remote client would skip it, and a
hub cannot type a word for anyone. The hub therefore **refuses every write the rule would ask a word
for from a network session**:

- **The same rule, the same read.** `overvolt::op_needs_word` on `dspi_session::ext::read_core_vreg`,
  read on the worker, immediately before the write, in the same turn of the queue, so no other
  session's command lands between. The firmware's request is deferred to its main loop, so the status
  can miss a request sent just before. That can only hide a pending change, never a raise past what
  the unit had: it is the same limit every client has.
- **For every role on the network.** A viewer or control client is refused by role first; an admin
  here.
- **Not for the app's session (`LOCAL_SESSION_ID`).** Someone at the machine who typed the word must not
  be refused because sharing happens to be on. This is safe because the session is this process's own
  core, whose `core_vreg_op` is the only path that encodes these requests: it judges the word itself,
  on a status read through this same queue, and `raw_set` refuses them, as do the C ABI's calls, which go
  through the core. So the app behaves the same with sharing on or off
  (`tests/wp_hb2_app_vreg.rs`). The limit: `HubHandle::open_local` is public Rust, so code linked into
  the same process could send raw bytes. That is the embedder's own trust boundary, not a remote one.
  One thing differs from the worker's read: another session's command can land between the core's read
  and its write. No other session can raise the voltage, so that cannot make the judgement wrong upward.
- **The answer is `DENIED`**, with no device write. Not `BLOCKED`: the spec gives that to a device
  refusing a command on its transport, and `map_status` reads it as a stall, "this build lacks it",
  whereas the core reads `Denied` as refused and says so.
- **What it can't judge, it refuses.** A payload of the wrong length or an op it does not know: the
  firmware refuses those too, and a later build might not. `wValue` is compared whole, as the firmware
  does, so `0x0102` is not this command.
- **What still passes, for an admin:** anything at or below 1200 mV, a level or lower target,
  `TRIAL_END`, `AUTO`, `CLEAR`, and `SET 0` on a readable unit. `AUTO` lets the firmware raise by
  itself after repeated hangs, which the rule does not ask about (CG14).

`policy.denied` cannot express this: it depends on the unit. A client finds out by the answer, and
CG11's flow then hides itself until the next connection (`dspi-core/src/vreg.rs`).

So a raise past 1.20 V is not possible from a remote client until the owner says otherwise, and
remains possible, with the word, from the app on the machine. Both halves are provisional, pending
the owner.

**A write is never replayed.** `run_command` makes one call and answers what it got: a stall,
a timeout, a bus error or a lost device alike. The only repeats are parameter reads inside a busy
window, and `Policy::retries_stall` refuses anything that mutates (`wp_hb2_a_failed_write_is_never_replayed`,
`wp_hb2_a_write_as_read_is_not_retried_and_a_parameter_read_is`). A queued write dropped with its device
answers `NO_DEVICE` and does not land on the board that comes back
(`wp_hb2_a_write_queued_when_the_device_left_does_not_land_on_its_return`).

**Locks.** Spec 7.7, plus the budget (section 6). `device.changed` carries `locked_by` as the
router has it, set under L1 with L2 released, whenever a lock changes through the hub and on the
supervisor's tick, which is how a timeout and the app's own lock show. It is not an embedder event.
