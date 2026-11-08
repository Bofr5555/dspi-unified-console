//! What a scenario runs against: the hub's address and credentials, the time
//! scale, what is allowed, and the measurements it records.

use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use dspi_link::socket::{TcpConnector, WsConnector, WsMessage, WsRecv};
use dspi_link::{AuthState, ConnectOptions, LinkClient, LinkError, LinkEvents, SessionConfig, Timing};
use serde_json::{json, Value};

/// How the harness appears in the hub's client list. Revocation matches names
/// exactly.
pub const CLIENT_NAME: &str = "dspi-link real-hub harness";
/// The pairing scenarios' own records, so removing them leaves the run's token.
pub const TEMP_CLIENT_NAME: &str = "dspi-link real-hub harness temp";

/// A hub refused the token. Every further try would count as a failed attempt
/// and walk this address into a lockout, so the rest of the run skips instead.
static TOKEN_REFUSED: AtomicBool = AtomicBool::new(false);

/// A scenario did not run, and why. A skip is not a failure.
#[derive(Debug)]
pub struct Skip(pub String);

pub type Outcome = Result<(), Skip>;

/// What differs between a real hub and the fake behind the same scenario.
pub trait Bench: Send + Sync {
    /// Take the board away or bring it back: the fake does it, a person does it
    /// on a real hub.
    fn board(&self, present: bool);
    /// Sessions the hub holds, where that can be seen.
    fn slots_in_use(&self) -> Option<usize> {
        None
    }
    /// What can only be checked from inside the hub.
    fn after_scenario(&self) {}
}

pub struct Target {
    pub host: String,
    pub port: u16,
    pub url: String,
    /// Durations run this many times faster than on a real hub.
    pub scale: u32,
    /// Margin for scheduling and socket ticks, not scaled.
    pub slack: Duration,
    pub token: Option<String>,
    pub pin: Option<String>,
    pub allow_pair: bool,
    pub allow_lockout: bool,
    pub no_board: bool,
    pub operator: bool,
    pub report: Option<PathBuf>,
    pub bench: Box<dyn Bench>,
}

impl Target {
    /// `secs` of real-hub time, on this target.
    pub fn s(&self, secs: f64) -> Duration {
        Duration::from_secs_f64(secs / f64::from(self.scale))
    }

    /// A delay from `RetryAdvice`, which is in real-hub time.
    pub fn scaled(&self, d: Duration) -> Duration {
        d / self.scale
    }

    /// Production's timers, scaled.
    pub fn session_config(&self) -> SessionConfig {
        let timing = if self.scale == 1 {
            Timing::default()
        } else {
            let d = Timing::default();
            Timing {
                request_timeout: d.request_timeout / self.scale,
                bulk_timeout: d.bulk_timeout / self.scale,
                ping_interval: d.ping_interval / self.scale,
                silence_limit: d.silence_limit / self.scale,
                keepalive_tick: d.keepalive_tick / self.scale,
                connect_budget: d.connect_budget / self.scale,
            }
        };
        SessionConfig { timing, ..SessionConfig::default() }
    }

    pub fn options(&self, name: &str, token: Option<String>) -> ConnectOptions {
        ConnectOptions {
            client_name: name.into(),
            app: "dspi-link real_hub".into(),
            app_version: env!("CARGO_PKG_VERSION").into(),
            token,
        }
    }

    /// One connect attempt at `url`.
    pub fn dial(
        &self,
        url: &str,
        token: Option<String>,
        events: Option<Arc<dyn LinkEvents>>,
    ) -> Result<(LinkClient, AuthState), LinkError> {
        self.dial_as(CLIENT_NAME, url, token, events)
    }

    /// [`Self::dial`] under another client name.
    pub fn dial_as(
        &self,
        name: &str,
        url: &str,
        token: Option<String>,
        events: Option<Arc<dyn LinkEvents>>,
    ) -> Result<(LinkClient, AuthState), LinkError> {
        let options = self.options(name, token);
        LinkClient::connect_with(&TcpConnector::default(), url, &options, events, &self.session_config())
    }

    /// An authenticated session at `url`, retrying as the error advises within
    /// `within`. Returns how many tries the hub answered "busy". Skips when the
    /// hub wants a PIN and the harness has no token.
    pub fn session_within(&self, url: &str, within: Duration) -> Result<(LinkClient, u32), Skip> {
        if self.scale == 1 && TOKEN_REFUSED.load(Ordering::SeqCst) {
            return Err(Skip("the hub refused the token earlier in this run".into()));
        }
        let start = Instant::now();
        let mut busy = 0;
        let mut attempt = 0;
        loop {
            let e = match self.dial(url, self.token.clone(), None) {
                Ok((c, AuthState::Ready)) => return Ok((c, busy)),
                Ok((c, AuthState::NeedsPairing { .. })) => {
                    c.close();
                    return Err(Skip(
                        "the hub wants a PIN and there is no token: run real_setup_pair, or set the token file".into(),
                    ));
                }
                Ok((c, state @ AuthState::TokenRefused { .. })) => {
                    c.close();
                    TOKEN_REFUSED.store(true, Ordering::SeqCst);
                    panic!("the hub refused the harness's token: {state:?}");
                }
                Err(e) => e,
            };
            if matches!(e, LinkError::HubBusy) {
                busy += 1;
            }
            let delay = e.retry_advice().delay(attempt).map(|d| self.scaled(d));
            match delay {
                Some(d) if start.elapsed() + d < within => thread::sleep(d),
                _ => panic!("no session within {within:?} after {:?}: {e}", start.elapsed()),
            }
            attempt += 1;
        }
    }

    /// An authenticated session on the direct address. A lockout left by a
    /// failed earlier scenario is waited out.
    pub fn session(&self) -> Result<LinkClient, Skip> {
        self.session_within(&self.url, self.s(130.0)).map(|(c, _)| c)
    }
}

/// Polls `f` until `within` has passed; whether it came true.
pub fn wait_until(within: Duration, mut f: impl FnMut() -> bool) -> bool {
    let start = Instant::now();
    loop {
        if f() {
            return true;
        }
        if start.elapsed() >= within {
            return false;
        }
        thread::sleep(Duration::from_millis(10));
    }
}

/// One scenario's measurements: JSON lines, on stderr and in the report file.
/// Never given a token or a PIN.
pub struct Rec<'a> {
    target: &'a Target,
    scenario: &'a str,
}

impl<'a> Rec<'a> {
    pub fn new(target: &'a Target, scenario: &'a str) -> Self {
        Self { target, scenario }
    }

    pub fn note(&self, v: Value) {
        let line = json!({"scenario": self.scenario, "hub": self.target.url, "data": v}).to_string();
        eprintln!("real_hub {line}");
        if let Some(path) = &self.target.report {
            match OpenOptions::new().create(true).append(true).open(path) {
                Ok(mut f) => {
                    let _ = writeln!(f, "{line}");
                }
                Err(e) => eprintln!("real_hub: cannot write {}: {e}", path.display()),
            }
        }
    }
}

/// "pass" or "fail" for the report, also when the scenario panics.
struct Verdict<'a>(&'a Rec<'a>);

impl Drop for Verdict<'_> {
    fn drop(&mut self) {
        if thread::panicking() {
            self.0.note(json!({"result": "fail"}));
        }
    }
}

/// What must hold before a scenario may start. Checked before anything is sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Gate {
    /// Shuts other clients out: needs `DSPI_REAL_HUB_ALLOW_LOCKOUT`.
    Lockout,
    /// Adds a client record: needs `DSPI_REAL_HUB_ALLOW_PAIR` and the PIN.
    Pair,
    /// Must know the PIN, so that a "wrong" one cannot be the hub's.
    KnownPin,
    /// Needs a board to read from.
    Board,
    /// Needs no board: `DSPI_REAL_HUB_NO_BOARD`.
    NoBoard,
    /// Needs a person at the bench: `DSPI_REAL_HUB_OPERATOR`.
    Operator,
}

impl Gate {
    fn refusal(self, t: &Target) -> Option<&'static str> {
        match self {
            Gate::Lockout if !t.allow_lockout => Some("shuts other clients out: needs DSPI_REAL_HUB_ALLOW_LOCKOUT=1"),
            Gate::Pair if !t.allow_pair => Some("adds a client record: needs DSPI_REAL_HUB_ALLOW_PAIR=1"),
            Gate::Pair if t.pin.is_none() => Some("needs DSPI_REAL_HUB_PIN"),
            Gate::KnownPin if t.pin.is_none() => Some("needs DSPI_REAL_HUB_PIN, so its wrong PIN cannot be the hub's"),
            Gate::Board if t.no_board => Some("needs a board, and DSPI_REAL_HUB_NO_BOARD is set"),
            Gate::NoBoard if !t.no_board => Some("needs DSPI_REAL_HUB_NO_BOARD=1, with no board attached"),
            Gate::Operator if !t.operator => Some("needs a person: DSPI_REAL_HUB_OPERATOR=1 and --nocapture"),
            _ => None,
        }
    }
}

/// The board's settings image, if there is a board and a session to read it.
fn settings(t: &Target, rec: &Rec) -> Option<Vec<u8>> {
    let c = t.session().ok()?;
    let handle = c.devices().iter().find(|d| d.state != "offline").map(|d| d.handle as u8);
    let image = handle.and_then(|h| match c.snapshot(h) {
        Ok(snapshot) => snapshot.bulk(),
        Err(e) => {
            rec.note(json!({"settings_guard": format!("no snapshot: {e}")}));
            None
        }
    });
    c.close();
    image
}

/// Run a scenario: gates first, then the settings guard around it. A difference
/// in the image fails the run. The harness sends no SET, so it would mean another
/// client changed something during the run, or a hole in the guard.
pub fn run(t: &Target, name: &str, gates: &[Gate], f: impl FnOnce(&Target, &Rec) -> Outcome) -> Outcome {
    run_guarded(t, name, gates, true, f)
}

/// [`run`], without comparing the image, for a scenario where the board reboots:
/// it comes back from flash, and unsaved live edits would look like a change.
pub fn run_unguarded(t: &Target, name: &str, gates: &[Gate], f: impl FnOnce(&Target, &Rec) -> Outcome) -> Outcome {
    run_guarded(t, name, gates, false, f)
}

fn run_guarded(
    t: &Target,
    name: &str,
    gates: &[Gate],
    guarded: bool,
    f: impl FnOnce(&Target, &Rec) -> Outcome,
) -> Outcome {
    let rec = Rec::new(t, name);
    if let Some(why) = gates.iter().find_map(|g| g.refusal(t)) {
        rec.note(json!({"result": "skip", "why": why}));
        return Err(Skip(why.into()));
    }
    let _verdict = Verdict(&rec);
    let before = if guarded { settings(t, &rec) } else { None };
    let outcome = f(t, &rec);
    if let Err(Skip(why)) = &outcome {
        rec.note(json!({"result": "skip", "why": why}));
        return outcome;
    }
    if let (Some(before), Some(after)) = (&before, if guarded { settings(t, &rec) } else { None }.as_ref()) {
        let first_change = before.iter().zip(after).position(|(a, b)| a != b);
        assert!(
            before.len() == after.len() && first_change.is_none(),
            "the settings changed during {name}: first at byte {first_change:?}, {} -> {} bytes",
            before.len(),
            after.len()
        );
    }
    t.bench.after_scenario();
    rec.note(json!({"result": "pass", "settings_compared": before.is_some()}));
    outcome
}

/// How [`forget_pairing`] ended.
#[derive(Debug, PartialEq, Eq)]
pub enum Forgotten {
    /// This many records with the names asked for were revoked. The session ends
    /// when its own record goes, so a later one of the same name can remain.
    Revoked(usize),
    /// The hub refused client management (not admin, or no such message): remove
    /// the record in the hub's own UI.
    Refused(String),
}

fn request(pair: &mut dspi_link::socket::WsPair, msg: Value) -> Result<Value, String> {
    let id = msg["id"].clone();
    pair.0.send(WsMessage::Text(msg.to_string())).map_err(|e| e.to_string())?;
    for _ in 0..30 {
        match pair.1.recv() {
            WsRecv::Message(WsMessage::Text(t)) => {
                let v: Value = serde_json::from_str(&t).map_err(|e| e.to_string())?;
                if v["id"] == id && (v["t"] == "ok" || v["t"] == "err") {
                    return Ok(v);
                }
            }
            WsRecv::Closed { code, .. } => return Err(format!("closed ({code:?})")),
            _ => {}
        }
    }
    Err("no answer".into())
}

/// Revoke the harness's client records named `names`, in that order, and no
/// others: spec 7.3 `auth.list` and `auth.revoke` on a bare socket, since
/// `LinkClient` has no client management. Needs an admin session. Name the record
/// that `token` belongs to last: revoking it closes the session (4002).
pub fn forget_pairing(connector: &dyn WsConnector, url: &str, token: &str, names: &[&str]) -> Forgotten {
    let refused = |why: String| Forgotten::Refused(why);
    let mut pair = match connector.connect(url, dspi_link::client::SUB_PROTOCOL) {
        Ok(p) => p,
        Err(e) => return refused(e.to_string()),
    };
    let hello = json!({"t": "hello", "proto": {"major": 1, "minor": 2},
                       "client": {"name": CLIENT_NAME, "app": "dspi-link real_hub", "version": "0"}});
    if let Err(e) = pair.0.send(WsMessage::Text(hello.to_string())) {
        return refused(e.to_string());
    }
    let mut hello_seen = false;
    for _ in 0..30 {
        if let WsRecv::Message(WsMessage::Text(t)) = pair.1.recv() {
            if serde_json::from_str::<Value>(&t).is_ok_and(|v| v["t"] == "hello") {
                hello_seen = true;
                break;
            }
        }
    }
    if !hello_seen {
        pair.0.close(1000);
        return refused("no hello".into());
    }
    let outcome = (|| {
        let auth = request(&mut pair, json!({"t": "auth.token", "id": 1, "token": token}))?;
        if auth["t"] != "ok" {
            return Err(format!("auth.token: {}", auth["code"]));
        }
        let list = request(&mut pair, json!({"t": "auth.list", "id": 2}))?;
        if list["t"] != "ok" {
            return Err(format!("auth.list: {}", list["code"]));
        }
        let clients = list["clients"].as_array().cloned().unwrap_or_default();
        let ours: Vec<Value> = names
            .iter()
            .flat_map(|name| clients.iter().filter(move |c| c["name"].as_str() == Some(*name)))
            .map(|c| c["cid"].clone())
            .collect();
        let mut revoked = 0;
        for (id, cid) in (3u64..).zip(&ours) {
            match request(&mut pair, json!({"t": "auth.revoke", "id": id, "cid": cid})) {
                Ok(v) if v["t"] == "ok" => revoked += 1,
                // Our own record: the hub closes this session at once
                Err(why) if why.contains("4002") => {
                    revoked += 1;
                    break;
                }
                Ok(v) => return Err(format!("auth.revoke: {}", v["code"])),
                Err(why) => return Err(format!("auth.revoke: {why}")),
            }
        }
        Ok(revoked)
    })();
    pair.0.close(1000);
    match outcome {
        Ok(n) => Forgotten::Revoked(n),
        Err(why) => refused(why),
    }
}
