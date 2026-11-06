//! The scripted parts of LINK-HUB-FINDINGS.md section 5. All durations are in
//! real-hub seconds (`Target::s`). Each scenario's number is its item there.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use dspi_link::socket::TcpConnector;
use dspi_link::{address, status, AuthState, LinkClient, LinkError, LinkEvents, RetryAdvice, SessionState};
use serde_json::json;

use crate::guard::{self, get, Get};
use crate::proxy::StallProxy;
use crate::target::{
    forget_pairing, run, run_unguarded, wait_until, Forgotten, Gate, Outcome, Rec, Skip, Target, CLIENT_NAME,
    TEMP_CLIENT_NAME,
};

/// Both real hubs: close 1001 after this long without a client frame.
const HUB_PEER_TIMEOUT: f64 = 30.0;
/// `dspi_link::client`'s limits.
const REQUEST_TIMEOUT: f64 = 10.0;
const PROBE_TIMEOUT: f64 = 5.0;
const SILENCE_LIMIT: f64 = 35.0;
/// The hub's lockout after 5 failures, and the client's backoff for it.
const LOCKOUT: f64 = 60.0;
/// Upper limit on a retrying connect: a held slot's life, plus a backoff step
/// (the steps reach 16 s by then).
const RECONNECT_WITHIN: f64 = HUB_PEER_TIMEOUT + 30.0;
/// How often the slot scenario asks. The backoff's growing steps would hide when
/// the slot came free.
const BUSY_RETRY: f64 = 5.0;

/// More than the client's silence limit twice over.
const IDLE_SECS: f64 = 75.0;
const POLL_EVERY: f64 = 0.25;
/// More sessions than any hub with a cap has (bridge: 4, or 8 with PSRAM).
const MAX_SLOTS_TRIED: usize = 16;

fn proxy_for(t: &Target) -> Result<StallProxy, Skip> {
    StallProxy::start(&t.host, t.port).map_err(|e| Skip(format!("no proxy to the hub: {e}")))
}

fn board_of(c: &LinkClient) -> u8 {
    let d = c.devices();
    let online = d.iter().find(|d| d.state != "offline");
    online.unwrap_or_else(|| panic!("no board online, and the scenario needs one: {d:?}")).handle as u8
}

/// Reachable, says who it is, and gets the token right: a lookup after the hello
/// sees that hub's id, and a hub whose id has no token is sent none (F5).
pub fn smoke(t: &Target) -> Outcome {
    run(t, "smoke", &[], |t, rec| {
        let info = address::fetch_info(&t.host, t.port, Duration::from_secs(5)).expect("the info endpoint");
        let asked = Arc::new(Mutex::new(Vec::new()));
        let seen = Arc::clone(&asked);
        let mut cfg = t.session_config();
        cfg.token_for = Some(Arc::new(move |id: &str| {
            seen.lock().unwrap().push(id.to_owned());
            None
        }));
        let options = t.options(CLIENT_NAME, None);
        let (c, state) = LinkClient::connect_with(&TcpConnector::default(), &t.url, &options, None, &cfg)
            .expect("connect with no token");
        let hello = c.hello().clone();
        rec.note(json!({
            "kind": hello.hub.kind, "proto": [hello.proto.major, hello.proto.minor], "auth": hello.auth,
            "pin_source": hello.pin_source, "caps": hello.caps, "max_inflight": hello.limits.max_inflight,
            "max_frame": hello.limits.max_frame, "state": format!("{state:?}"),
        }));
        assert!(!hello.hub.id.is_empty(), "a hub says its id");
        assert_eq!(info.hub.id, hello.hub.id, "the info endpoint and the hello agree");
        if hello.pin_required() {
            assert_eq!(
                *asked.lock().unwrap(),
                std::slice::from_ref(&hello.hub.id),
                "the lookup saw the id from the hello"
            );
            assert!(matches!(state, AuthState::NeedsPairing { .. }), "no token, nothing refused: {state:?}");
        } else {
            assert_eq!(state, AuthState::Ready);
        }
        c.close();
        Ok(())
    })
}

/// Item 1 (F1): a healthy idle session outlives the silence limit, with or
/// without a board.
pub fn idle_session(t: &Target) -> Outcome {
    run(t, "1_idle", &[], |t, rec| {
        let c = t.session()?;
        let expected = if t.no_board { SessionState::NoBoard } else { SessionState::Online };
        assert_eq!(c.session_state(), expected);
        let start = Instant::now();
        let dropped = wait_until(t.s(IDLE_SECS), || c.is_closed());
        rec.note(json!({"idle_s": start.elapsed().as_secs_f64(), "dropped": dropped, "reason": c.closed_reason()}));
        assert!(!dropped, "dropped after {:?}: {:?}", start.elapsed(), c.closed_reason());
        assert_eq!(c.session_state(), expected);
        c.close();
        Ok(())
    })
}

/// A PIN that is not the hub's, if the hub's is known.
fn wrong_pin(t: &Target) -> &'static str {
    if t.pin.as_deref() == Some("000000") {
        "000001"
    } else {
        "000000"
    }
}

/// Item 2 (F2): five wrong PINs, then the paired client from the same address.
/// The token is refused "not now", not "bad", and works again after the lockout.
/// On a window hub this also ends the open window.
pub fn wrong_pin_keeps_token(t: &Target) -> Outcome {
    run(t, "2_wrong_pin", &[Gate::Lockout], |t, rec| {
        let c = t.session()?;
        let pin_required = c.hello().pin_required();
        c.close();
        if !pin_required {
            return Err(Skip("an open hub has no PIN".into()));
        }
        let token = t.token.clone().ok_or_else(|| Skip("needs a token".into()))?;
        let (c, state) = t.dial(&t.url, None, None).expect("connect with no token");
        assert!(matches!(state, AuthState::NeedsPairing { .. }), "{state:?}");
        let mut refused = 0;
        for _ in 0..5 {
            match c.pair(wrong_pin(t), "viewer") {
                Err(LinkError::WrongPin { .. }) => refused += 1,
                // Failures from just before the run count too
                Err(LinkError::RateLimited { .. }) => break,
                Err(e) => panic!("a wrong PIN answered {e:?}"),
                Ok(_) => panic!("the \"wrong\" PIN paired: revoke `{}` on the hub", CLIENT_NAME),
            }
        }
        c.close();
        assert!(refused > 0, "no attempt got as far as a wrong PIN: already locked out");

        let locked_at = Instant::now();
        let e = t.dial(&t.url, Some(token.clone()), None).err().expect("the token is refused during the lockout");
        assert!(matches!(e, LinkError::RateLimited { .. }), "{e:?}");
        let advice = e.retry_advice();
        assert_eq!(advice, RetryAdvice::Retry { min_delay: Duration::from_secs(60) });

        // Restore: wait it out, and the same token is good. The Ready here is
        // what F2 was about; a lost token would show as TokenRefused
        thread::sleep(t.s(LOCKOUT));
        let (c, busy) = t.session_within(&t.url, t.s(LOCKOUT) * 2)?;
        rec.note(json!({"wrong_pins": refused, "back_after_s": locked_at.elapsed().as_secs_f64(), "busy": busy}));
        c.close();
        Ok(())
    })
}

/// Reads at a steady rate, as a UI does, recording what each one took.
struct Poller {
    stop: Arc<AtomicBool>,
    worst_ms: Arc<AtomicU64>,
    reads: Arc<AtomicU64>,
    thread: thread::JoinHandle<()>,
}

impl Poller {
    fn start(t: &Target, c: &LinkClient, handle: u8) -> Poller {
        let stop = Arc::new(AtomicBool::new(false));
        let worst_ms = Arc::new(AtomicU64::new(0));
        let reads = Arc::new(AtomicU64::new(0));
        let (c, every) = (c.clone(), t.s(POLL_EVERY));
        let (s, w, r) = (Arc::clone(&stop), Arc::clone(&worst_ms), Arc::clone(&reads));
        let thread = thread::spawn(move || {
            while !s.load(Ordering::SeqCst) && !c.is_closed() {
                let start = Instant::now();
                let answer = get(&c, handle, &guard::PLATFORM);
                w.fetch_max(start.elapsed().as_millis() as u64, Ordering::SeqCst);
                if answer.status == status::OK {
                    r.fetch_add(1, Ordering::SeqCst);
                }
                thread::sleep(every);
            }
        });
        Poller { stop, worst_ms, reads, thread }
    }

    fn reads(&self) -> u64 {
        self.reads.load(Ordering::SeqCst)
    }

    /// The longest a read took, which is how long a UI would have waited.
    fn finish(self) -> Duration {
        self.stop.store(true, Ordering::SeqCst);
        let _ = self.thread.join();
        Duration::from_millis(self.worst_ms.load(Ordering::SeqCst))
    }
}

/// Item 3 (F7): the network goes dark for `secs` while reading, or idle.
/// Time to detect, the longest wait on a read, and how soon a new session gets in.
/// `secs` below the request timeout is a hiccup, and the session survives it.
pub fn dark_network(t: &Target, secs: f64, polling: bool) -> Outcome {
    let name = format!("3_dark_{secs}s_{}", if polling { "polling" } else { "idle" });
    run(t, &name, &[Gate::Board], |t, rec| {
        let proxy = proxy_for(t)?;
        let (c, _) = t.session_within(&proxy.url(), t.s(RECONNECT_WITHIN))?;
        let handle = board_of(&c);
        let poller = polling.then(|| Poller::start(t, &c, handle));
        if let Some(p) = &poller {
            assert!(wait_until(t.s(5.0) + t.slack, || p.reads() > 0), "no read answered before the stall");
        }

        let detect_limit = if polling { REQUEST_TIMEOUT + PROBE_TIMEOUT } else { SILENCE_LIMIT };
        let survives = secs < REQUEST_TIMEOUT;
        proxy.stall(true);
        let dark_at = Instant::now();
        let detected = if survives {
            let closed = wait_until(t.s(secs), || c.is_closed());
            assert!(!closed, "a {secs} s hiccup ended the session: {:?}", c.closed_reason());
            None
        } else {
            // Stays dark at least until the client has had its chance to notice,
            // or the session would just recover
            let closed = wait_until(t.s(detect_limit) + t.slack, || c.is_closed());
            assert!(closed, "not noticed within {:?}", dark_at.elapsed());
            let at = dark_at.elapsed();
            if let Some(left) = t.s(secs).checked_sub(at) {
                thread::sleep(left);
            }
            Some(at)
        };
        proxy.stall(false);

        if survives {
            let after = get(&c, handle, &guard::PLATFORM);
            assert_eq!(after.status, status::OK, "reads work after the hiccup");
            assert!(!c.is_closed());
        }
        c.close();
        let worst = poller.map(Poller::finish);
        if let Some(worst) = worst {
            assert!(worst <= t.s(REQUEST_TIMEOUT) + t.slack, "a read blocked for {worst:?}");
        }

        let back_at = Instant::now();
        let (fresh, busy) = t.session_within(&t.url, t.s(RECONNECT_WITHIN))?;
        assert_eq!(get(&fresh, board_of(&fresh), &guard::PLATFORM).status, status::OK);
        rec.note(json!({
            "dark_s": secs, "polling": polling,
            "detected_s": detected.map(|d| d.as_secs_f64()),
            "worst_read_s": worst.map(|d| d.as_secs_f64()),
            "reconnect_s": back_at.elapsed().as_secs_f64(), "busy_answers": busy,
        }));
        fresh.close();
        if t.bench.slots_in_use().is_some() {
            let released = wait_until(t.s(HUB_PEER_TIMEOUT) + t.slack, || t.bench.slots_in_use() == Some(0));
            assert!(released, "slots still held: {:?}", t.bench.slots_in_use());
        }
        Ok(())
    })
}

/// Item 4 (F8): every slot taken. 503 is a typed "busy" with a retry; a closed
/// session frees its slot at once, and a vanished one only when the hub gives up
/// on it. Web UI tabs (H4) stay manual.
pub fn slots_full(t: &Target) -> Outcome {
    run(t, "4_slots_full", &[Gate::Lockout], |t, rec| {
        let proxy = proxy_for(t)?;
        let (vanishing, _) = t.session_within(&proxy.url(), t.s(RECONNECT_WITHIN))?;
        let kind = vanishing.hello().hub.kind.clone();
        let token = || t.token.clone();
        let mut held: Vec<LinkClient> = Vec::new();
        let mut refused = None;
        while held.len() < MAX_SLOTS_TRIED {
            match t.dial(&t.url, token(), None) {
                Ok((c, _)) => held.push(c),
                Err(e) => {
                    refused = Some(e);
                    break;
                }
            }
        }
        let close_all = |held: &[LinkClient]| held.iter().for_each(LinkClient::close);
        let Some(e) = refused else {
            rec.note(json!({"slots": "no cap seen", "sessions": held.len() + 1, "kind": kind}));
            close_all(&held);
            vanishing.close();
            assert_ne!(kind, "bridge", "a bridge has a session cap");
            return Ok(());
        };
        let slots = held.len() + 1;
        assert_eq!(e, LinkError::HubBusy, "with {slots} sessions");
        let advice = e.retry_advice();
        assert_eq!(advice, RetryAdvice::Retry { min_delay: dspi_link::HUB_BUSY_BACKOFF });

        let closed_at = Instant::now();
        held.pop().expect("a bridge has at least two slots").close();
        let reopened = wait_until(t.s(2.0) + t.slack, || match t.dial(&t.url, token(), None) {
            Ok((c, _)) => {
                held.push(c);
                true
            }
            Err(_) => false,
        });
        assert!(reopened, "a closed session's slot stayed taken for {:?}", closed_at.elapsed());
        let closed_slot_s = closed_at.elapsed().as_secs_f64();

        proxy.stall(true);
        let vanished_at = Instant::now();
        let still_busy = t.dial(&t.url, token(), None).err();
        assert_eq!(still_busy, Some(LinkError::HubBusy), "a vanished client keeps its slot for now");
        let mut busy = 1;
        let c = loop {
            match t.dial(&t.url, token(), None) {
                Ok((c, _)) => break c,
                Err(LinkError::HubBusy) => busy += 1,
                Err(e) => panic!("while the slot was held: {e}"),
            }
            let waited = vanished_at.elapsed();
            assert!(waited < t.s(HUB_PEER_TIMEOUT + 2.0 * BUSY_RETRY) + t.slack, "still full after {waited:?}");
            thread::sleep(t.s(BUSY_RETRY));
        };
        let vanished_slot_s = vanished_at.elapsed().as_secs_f64();
        rec.note(json!({"slots": slots, "closed_slot_free_s": closed_slot_s,
                        "vanished_slot_free_s": vanished_slot_s, "busy_answers": busy}));
        c.close();
        close_all(&held);
        vanishing.close();
        proxy.stall(false);
        Ok(())
    })
}

/// Item 6, first half (F9): pairing with no board. The PIN is accepted and the
/// session says "no board", which is not a failure.
pub fn no_board_pairing(t: &Target) -> Outcome {
    run(t, "6_no_board_pairing", &[Gate::Pair, Gate::NoBoard], |t, rec| {
        let pin = t.pin.clone().expect("Gate::Pair");
        let (c, state) = t.dial_as(TEMP_CLIENT_NAME, &t.url, None, None).expect("connect");
        if !state.needs_pin() {
            c.close();
            return Err(Skip("an open hub has no PIN".into()));
        }
        let token = c.pair(&pin, "viewer").expect("the PIN is accepted without a board");
        assert_eq!(c.session_state(), SessionState::NoBoard);
        assert!(!c.is_closed());
        c.close();
        note_forgotten(t, rec, &token);
        Ok(())
    })
}

/// Item 6, second half (F10): the board loses power and comes back in the middle
/// of a session. The session itself stays up. A preset save stays manual: it
/// writes flash.
pub fn board_flap(t: &Target) -> Outcome {
    run_unguarded(t, "6_board_flap", &[Gate::Operator, Gate::Board], |t, rec| {
        let c = t.session()?;
        let handle = board_of(&c);
        let start = Instant::now();
        t.bench.board(false);
        let gone = wait_until(t.s(120.0), || c.offline_count(u32::from(handle)) > 0);
        assert!(gone, "the hub never reported the board offline");
        let offline_after_s = start.elapsed().as_secs_f64();
        assert!(!c.is_closed(), "losing the board ended the session: {:?}", c.closed_reason());
        assert_ne!(c.session_state(), SessionState::DeviceListUnknown);

        t.bench.board(true);
        let back = wait_until(t.s(120.0), || c.session_state() == SessionState::Online);
        assert!(back, "the board did not come back: {:?}", c.session_state());
        assert!(!c.is_closed());
        assert_eq!(get(&c, board_of(&c), &guard::PLATFORM).status, status::OK);
        rec.note(json!({"offline_after_s": offline_after_s, "offline_count": c.offline_count(u32::from(handle)),
                        "total_s": start.elapsed().as_secs_f64()}));
        c.close();
        Ok(())
    })
}

/// What one read of the attach sweep came to.
fn read_row(c: &LinkClient, handle: u8, row: &Get, sweep: &mut Sweep) -> Vec<u8> {
    let mut answer = get(c, handle, row);
    for _ in 0..3 {
        if answer.status != status::RATE_LIMITED && answer.status != status::BUSY {
            break;
        }
        thread::sleep(Duration::from_millis(100));
        answer = get(c, handle, row);
    }
    if row.len > guard::BRIDGE_GET_LIMIT {
        sweep.over_limit.push(format!("{} asks {} B", row.name, row.len));
    }
    match answer.status {
        status::OK => sweep.answered += 1,
        // Firmware without the feature, or a hub's policy: the core takes that
        // as "absent"
        status::ERROR | status::BLOCKED | status::DENIED | status::OVERSIZE | status::UNSUPPORTED => {
            sweep.absent.push(row.name)
        }
        other => sweep.failed.push(format!("{}: status 0x{other:02X}", row.name)),
    }
    answer.payload
}

#[derive(Default)]
struct Sweep {
    answered: u32,
    absent: Vec<&'static str>,
    over_limit: Vec<String>,
    failed: Vec<String>,
}

/// Item 7: does any GET of the attach exceed what a bridge carries? Each is read
/// as the core reads it. Nothing is written.
pub fn attach_gets(t: &Target) -> Outcome {
    run(t, "7_attach_gets", &[Gate::Board], |t, rec| {
        let c = t.session()?;
        let handle = board_of(&c);
        let wire_version = c.devices().iter().find(|d| u32::from(handle) == d.handle).and_then(|d| d.wire_version);
        let mut sweep = Sweep::default();
        for row in guard::ATTACH {
            let reply = read_row(&c, handle, row, &mut sweep);
            if row.name == guard::CS_CAPS_HEADER.name && reply.len() >= 4 {
                read_row(&c, handle, &guard::cs_caps_full(reply[2]), &mut sweep);
            }
            if row.name == guard::SIGGEN_CAPS_HEADER.name && reply.len() >= 2 {
                for i in 0..reply[1] {
                    read_row(&c, handle, &guard::siggen_desc(i), &mut sweep);
                }
            }
        }
        for slot in 0..guard::PRESET_SLOTS {
            read_row(&c, handle, &guard::preset_name(slot), &mut sweep);
        }

        // The one read allowed past the limit
        let bulk = get(&c, handle, &guard::BULK);
        assert_eq!(bulk.status, status::OK, "the bulk read");
        assert!(bulk.payload.len() > usize::from(guard::BRIDGE_GET_LIMIT), "truncated to {} B", bulk.payload.len());
        if let Some(version) = wire_version {
            assert_eq!(u32::from(bulk.payload[0]), version, "the image's version is the device's");
        }
        let platform = get(&c, handle, &guard::PLATFORM);
        assert_eq!(platform.status, status::OK);
        assert!(platform.payload.len() >= 4, "platform: {} B", platform.payload.len());
        rec.note(json!({
            "answered": sweep.answered, "absent": sweep.absent, "over_132": sweep.over_limit,
            "failed": sweep.failed, "bulk_b": bulk.payload.len(),
        }));
        c.close();
        assert!(sweep.failed.is_empty(), "{:?}", sweep.failed);
        assert!(sweep.over_limit.is_empty(), "{:?}", sweep.over_limit);
        Ok(())
    })
}

#[derive(Default)]
struct TokenEvents {
    tokens: Mutex<Vec<String>>,
    cut: Mutex<Option<Arc<StallProxy>>>,
}

impl LinkEvents for TokenEvents {
    fn token_issued(&self, _hub_id: &str, token: &str) {
        self.tokens.lock().unwrap().push(token.to_owned());
        if let Some(proxy) = &*self.cut.lock().unwrap() {
            proxy.cut();
        }
    }
}

fn note_forgotten(t: &Target, rec: &Rec, token: &str) {
    let outcome = forget_pairing(&TcpConnector::default(), &t.url, token, &[TEMP_CLIENT_NAME]);
    if let Forgotten::Refused(why) = &outcome {
        eprintln!("real_hub: remove the client `{TEMP_CLIENT_NAME}` in the hub's own UI ({why})");
    }
    rec.note(json!({"pairing_removed": format!("{outcome:?}")}));
}

fn pairing_session(t: &Target, url: &str, ev: &Arc<TokenEvents>) -> Result<LinkClient, Skip> {
    let events = Arc::clone(ev) as Arc<dyn LinkEvents>;
    let (c, state) = t.dial_as(TEMP_CLIENT_NAME, url, None, Some(events)).expect("connect");
    if !state.needs_pin() {
        c.close();
        return Err(Skip("an open hub has no PIN".into()));
    }
    Ok(c)
}

/// Item 8 (F4): the link dies the moment the `ok` to `auth.pair` arrives. The
/// token is safe, and the next connect needs no PIN. A window hub's PIN is used up
/// by this: open a new window for each pairing scenario.
pub fn pair_cut_at_ok(t: &Target) -> Outcome {
    run(t, "8_pair_cut_at_ok", &[Gate::Pair], |t, rec| {
        let pin = t.pin.clone().expect("Gate::Pair");
        let proxy = Arc::new(proxy_for(t)?);
        let ev = Arc::new(TokenEvents::default());
        *ev.cut.lock().unwrap() = Some(Arc::clone(&proxy));
        let c = pairing_session(t, &proxy.url(), &ev)?;
        let token = c.pair(&pin, "viewer").expect("the token comes back although the link died");
        assert!(!token.is_empty());
        assert!(wait_until(t.s(REQUEST_TIMEOUT) + t.slack, || c.is_closed()), "the cut went unnoticed");
        assert_eq!(*ev.tokens.lock().unwrap(), std::slice::from_ref(&token), "the listener had it first");

        let (fresh, state) = t.dial(&t.url, Some(token.clone()), None).expect("reconnect");
        assert_eq!(state, AuthState::Ready, "no second PIN");
        fresh.close();
        note_forgotten(t, rec, &token);
        Ok(())
    })
}

/// Item 8 (F4), the other way: the reply is late, so `pair` has given up. It
/// still reaches the listener, and the token works.
pub fn pair_late_reply(t: &Target) -> Outcome {
    run(t, "8_pair_late_reply", &[Gate::Pair], |t, rec| {
        let pin = t.pin.clone().expect("Gate::Pair");
        let proxy = proxy_for(t)?;
        let ev = Arc::new(TokenEvents::default());
        let c = pairing_session(t, &proxy.url(), &ev)?;
        proxy.stall_replies(true);
        let e = c.pair(&pin, "viewer").unwrap_err();
        assert_eq!(e, LinkError::Timeout, "pair gives up while the reply waits");
        proxy.stall_replies(false);
        assert!(
            wait_until(t.s(PROBE_TIMEOUT) + t.slack, || !ev.tokens.lock().unwrap().is_empty()),
            "the late token never reached the listener"
        );
        let token = ev.tokens.lock().unwrap()[0].clone();
        c.close();

        let (fresh, state) = t.dial(&t.url, Some(token.clone()), None).expect("reconnect");
        assert_eq!(state, AuthState::Ready, "the late token is good");
        fresh.close();
        note_forgotten(t, rec, &token);
        Ok(())
    })
}
