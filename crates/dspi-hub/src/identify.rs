//! Identify (WP P2ID, PLAN-P2 F2, `docs/integrator/hub-api.md` section 4): make one
//! DSPi blink, and play its quiet identify tone when asked.
//!
//! | Message | Role | Answer |
//! |---|---|---|
//! | `device.identify {id, serial or handle, seconds, tone_output?}` | Control, Admin | `ok {v, serial, status}` |
//!
//! `status` is the device's `EXT_IDENTIFY` IN read after the write: `{remaining_s,
//! last_tone, chip, chip_rev, board, hw_rev}` (`null` if that read failed; the blink
//! was sent). `seconds` 0 stops a blink; past [`IDENTIFY_MAX_S`] it is shortened, as
//! the firmware does.
//!
//! **One of the hub's own writes** (PLAN-P2 M3, OD-P2o): the extension opcode is
//! Admin-only in the policy, so the hub sends `EXT_IDENTIFY` OUT as
//! [`INTERNAL_SESSION_ID`] on the caller's behalf, never as the caller. The blink goes
//! out whatever the lock (the write is let past it once, as a limits hold is); a
//! request with `tone_output` answers `locked` while another session holds the
//! device's lock (the caller's own lock is no bar). The `EXT_LIST` and `EXT_IDENTIFY`
//! reads around it pass the lock the same way (the router locks reads out too). The firmware's tone is 1 kHz at
//! -40 dBFS through that output's chain, and only when asked; firmware without
//! `EXT_IDENTIFY` answers `unsupported` with nothing sent, and there is no generator
//! fallback (OD-P2i).
//!
//! Refusals, in this order: `denied` (a Viewer), `bad_request` (a malformed request,
//! a tone output the device does not have, a serial and a handle naming different
//! devices), `no_device` (an unknown handle), `not_found` (a serial the hub does not
//! know), `offline` (known, not attached), `unsupported` (its `EXT_LIST` lacks
//! `EXT_IDENTIFY`, or it stalls `EXT_LIST`: stock firmware), `rate_limited` (within
//! [`RATE_WINDOW`] of the last request sent to that device, with `retry_in_ms`),
//! `locked` (the tone, under another session's lock). A device that refuses the write
//! answers `device` with `status`, the DSPi Link status name.
//!
//! Each identify that went out is one history record on the bus
//! ([`BusMessage::Event`], the record's `id` 0: not yet stored; P2EV stores it):
//! source `identify`, `session` and `by` the caller.
//!
//! The service is on with [`crate::ServiceConfig::identify`] and adds the cap
//! [`CAP`].
#![warn(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use dspi_link::frames::CmdRequest;
use dspi_link::{Dir, status};
use dspi_proto::ext::{IDENTIFY_MAX_S, IdentifyRequest, IdentifyStatus};
use dspi_proto::generated::{ext as gext, opcodes as op};
use dspi_session::events::{EventKind, EventRecord};
use serde_json::{Value, json};

use crate::bus::{Bus, BusMessage};
use crate::device::DeviceShared;
use crate::hub::HubHandle;
use crate::registry::{DeviceState, RegisteredDevice};
use crate::service::{DeviceEvent, HubService, ServiceConfig, ServiceSetup};
use crate::session::{Completion, INTERNAL_SESSION_ID, Role, Session, SessionCtx};

/// The family's cap in `hello.caps`.
pub const CAP: &str = "identify";
/// The message.
pub const MESSAGE: &str = "device.identify";
/// At most one identify per device in this window.
pub const RATE_WINDOW: Duration = Duration::from_secs(2);
/// `source` of the history record.
pub const RECORD_SOURCE: &str = "identify";
/// How long one transfer is waited for.
const TRANSFER_WAIT: Duration = Duration::from_secs(3);
/// `EXT_LIST`'s longest reply read: 64 sub-commands.
const EXT_LIST_LEN: u16 = 128;
/// A serial longer than this is not one.
const MAX_SERIAL_BYTES: usize = 64;
/// Devices remembered (support, last request); far past the hub's 255 handles.
const MAX_REMEMBERED: usize = 1024;

/// The service, when the config asks for it.
pub fn service(config: &ServiceConfig, setup: &ServiceSetup<'_>) -> Option<Arc<dyn HubService>> {
    config
        .identify
        .then(|| Arc::new(Identify::new(Arc::clone(setup.bus))) as Arc<dyn HubService>)
}

/// What a `device.identify` asks, parsed and bounded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdentifyAsk {
    /// The device by serial.
    pub serial: Option<String>,
    /// The device by handle.
    pub handle: Option<u8>,
    /// Blink seconds, shortened to [`IDENTIFY_MAX_S`]; 0 stops.
    pub seconds: u8,
    /// The output to play the tone on.
    pub tone_output: Option<u8>,
}

/// A refusal: the error code and a line for people.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refused {
    /// The `code` (`docs/integrator/hub-api.md` section 1.6).
    pub code: &'static str,
    /// The `msg`.
    pub msg: String,
    /// More fields of the error (`retry_in_ms`, `status`).
    pub extra: Option<(&'static str, Value)>,
}

impl Refused {
    fn new(code: &'static str, msg: impl Into<String>) -> Self {
        Self {
            code,
            msg: msg.into(),
            extra: None,
        }
    }

    fn bad(msg: impl Into<String>) -> Self {
        Self::new("bad_request", msg)
    }
}

impl IdentifyAsk {
    /// Parse a request. Never panics; a wrong type or range is `bad_request`.
    pub fn parse(msg: &Value) -> Result<Self, Refused> {
        let serial = match msg.get("serial") {
            None | Some(Value::Null) => None,
            Some(Value::String(s)) if !s.is_empty() && s.len() <= MAX_SERIAL_BYTES => {
                Some(s.clone())
            }
            Some(_) => return Err(Refused::bad("serial must be a device's serial")),
        };
        let handle = match msg.get("handle") {
            None | Some(Value::Null) => None,
            Some(v) => Some(
                v.as_u64()
                    .and_then(|h| u8::try_from(h).ok())
                    .ok_or_else(|| Refused::bad("handle must be a whole number 0 to 255"))?,
            ),
        };
        if serial.is_none() && handle.is_none() {
            return Err(Refused::bad("serial or handle is required"));
        }
        let seconds = msg
            .get("seconds")
            .and_then(Value::as_u64)
            .ok_or_else(|| Refused::bad("seconds must be a whole number, 0 stops"))?;
        let seconds =
            u8::try_from(seconds.min(u64::from(IDENTIFY_MAX_S))).unwrap_or(IDENTIFY_MAX_S);
        let tone_output = match msg.get("tone_output") {
            None | Some(Value::Null) => None,
            Some(v) => Some(
                v.as_u64()
                    .and_then(|o| u8::try_from(o).ok())
                    .filter(|o| *o != dspi_proto::ext::IDENTIFY_TONE_NONE)
                    .ok_or_else(|| Refused::bad("tone_output must be an output index"))?,
            ),
        };
        Ok(Self {
            serial,
            handle,
            seconds,
            tone_output,
        })
    }

    /// The device it names, from the hub's registry (online and offline ones).
    pub fn resolve(&self, devices: &[RegisteredDevice]) -> Result<RegisteredDevice, Refused> {
        let by_handle =
            match self.handle {
                Some(h) => Some(devices.iter().find(|d| d.handle == h).ok_or_else(|| {
                    Refused::new("no_device", format!("no device at handle {h}"))
                })?),
                None => None,
            };
        let by_serial = match &self.serial {
            Some(s) => Some(
                devices
                    .iter()
                    .find(|d| d.identity.serial == *s)
                    .ok_or_else(|| {
                        Refused::new("not_found", "the hub does not know that serial")
                    })?,
            ),
            None => None,
        };
        let device = match (by_handle, by_serial) {
            (Some(a), Some(b)) if a.handle != b.handle => {
                return Err(Refused::bad("serial and handle name different devices"));
            }
            (Some(d), _) | (None, Some(d)) => d.clone(),
            (None, None) => return Err(Refused::bad("serial or handle is required")),
        };
        if device.state != DeviceState::Online {
            return Err(Refused::new("offline", "the device is not attached"));
        }
        if let Some(o) = self.tone_output
            && o >= device.identity.outputs
        {
            return Err(Refused::bad(format!(
                "the device has no output {o} (it has {})",
                device.identity.outputs
            )));
        }
        Ok(device)
    }
}

/// At most one request per device per [`RATE_WINDOW`]. Pure: the caller passes the
/// clock.
#[derive(Debug, Default)]
pub struct RateGate {
    last: BTreeMap<String, Instant>,
}

impl RateGate {
    /// Take the device's turn at `now`, or how long until it has one.
    pub fn take(&mut self, serial: &str, now: Instant) -> Result<(), Duration> {
        if let Some(at) = self.last.get(serial) {
            let since = now.saturating_duration_since(*at);
            if since < RATE_WINDOW {
                return Err(RATE_WINDOW.saturating_sub(since));
            }
        }
        self.last
            .retain(|_, at| now.saturating_duration_since(*at) < RATE_WINDOW);
        if self.last.len() >= MAX_REMEMBERED {
            // Only reached with that many devices asked within one window.
            self.last.clear();
        }
        self.last.insert(serial.to_owned(), now);
        Ok(())
    }
}

/// The identify service.
pub struct Identify {
    bus: Arc<Bus>,
    /// Whether each attached device's `EXT_LIST` names `EXT_IDENTIFY`; forgotten when
    /// it comes or goes (its firmware may have changed).
    supported: Mutex<BTreeMap<String, bool>>,
    rate: Mutex<RateGate>,
}

fn guard<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    // Every section under these locks leaves its data whole before it could panic.
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The hub's own session (PLAN-P2 M3): Admin, never limited by in-flight.
fn internal() -> SessionCtx {
    SessionCtx {
        id: INTERNAL_SESSION_ID,
        role: Role::Admin,
        exempt_inflight: true,
    }
}

/// A transfer's failure: its Link status, and whether the device stalled it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Failed {
    status: u8,
    stalled: bool,
}

fn request(handle: u8, dir: Dir, value: u16, payload: Vec<u8>, len: u16) -> CmdRequest {
    CmdRequest {
        tag: 0,
        handle,
        dir,
        b_request: op::REQ_EXTENSION,
        w_value: value,
        w_index: dspi_proto::VENDOR_INTERFACE,
        w_length: match dir {
            Dir::Get => len,
            Dir::Set => u16::try_from(payload.len()).unwrap_or(u16::MAX),
        },
        payload,
    }
}

/// One transfer as the hub's own session, waited for (on the asking connection's
/// reader, never a hub thread).
fn exec(dev: &DeviceShared, req: CmdRequest, pass_lock: bool) -> Result<Vec<u8>, Failed> {
    let stalled = Arc::new(AtomicBool::new(false));
    let (tx, rx) = mpsc::sync_channel(1);
    let done = Completion::new(0, move |r| {
        let _ = tx.send(r);
    })
    .noting_stall(Arc::clone(&stalled));
    if pass_lock {
        // Exactly this transfer, once, whatever the lock (OD-P2o): the router locks
        // reads out too, and a lock never stops the blink or the reads around it.
        dev.router.pass_hold(&req);
    }
    dev.submit(
        internal(),
        req,
        done,
        Instant::now().checked_add(TRANSFER_WAIT),
    );
    let failed = |status| Failed {
        status,
        stalled: status == status::ERROR && stalled.load(Ordering::SeqCst),
    };
    match rx.recv_timeout(TRANSFER_WAIT.saturating_add(Duration::from_secs(1))) {
        Ok(r) if r.status == status::OK => Ok(r.payload),
        Ok(r) => Err(failed(r.status)),
        Err(_) => Err(failed(status::TIMEOUT)),
    }
}

/// A failed transfer as the answer's refusal.
fn refusal_of(f: Failed) -> Refused {
    match f.status {
        status::NO_DEVICE => Refused::new("offline", "the device went away"),
        status::LOCKED => Refused::new("locked", "another session holds the device's lock"),
        status::DENIED => Refused::new("denied", "the hub refused the write"),
        s => Refused {
            code: "device",
            msg: format!("the device answered {}", status::name(s)),
            extra: Some(("status", json!(status::name(s)))),
        },
    }
}

/// `status` of the answer.
fn status_json(s: &IdentifyStatus) -> Value {
    json!({
        "remaining_s": s.remaining_s,
        "last_tone": s.last_tone,
        "chip": s.chip.label(),
        "chip_rev": s.chip_rev,
        "board": s.board,
        "hw_rev": s.hw_rev,
    })
}

/// The history record of one identify.
pub fn record(ask: &IdentifyAsk, session: u16, by: Option<String>, t_ms: i64) -> EventRecord {
    let what = if ask.seconds == 0 {
        "Identify stopped".to_owned()
    } else {
        format!("Identify: blink {} s", ask.seconds)
    };
    let text = match (ask.tone_output, &by) {
        (Some(o), Some(by)) => format!(
            "{what}, tone on output {}, by {by}",
            u16::from(o).saturating_add(1)
        ),
        (Some(o), None) => format!("{what}, tone on output {}", u16::from(o).saturating_add(1)),
        (None, Some(by)) => format!("{what}, by {by}"),
        (None, None) => what,
    };
    let mut r = EventRecord::new(EventKind::Notification, t_ms, text);
    r.source = Some(RECORD_SOURCE.to_owned());
    r.session = Some(u32::from(session));
    r.by = by;
    r.channel = ask.tone_output;
    r.value = Some(f64::from(ask.seconds));
    r
}

impl Identify {
    /// A service publishing its records on `bus`.
    pub fn new(bus: Arc<Bus>) -> Self {
        Self {
            bus,
            supported: Mutex::new(BTreeMap::new()),
            rate: Mutex::new(RateGate::default()),
        }
    }

    /// Whether the device's firmware has `EXT_IDENTIFY`: its `EXT_LIST`, read once
    /// per attach. A stall (stock firmware) is no; any other failure is not an answer.
    fn supports(&self, serial: &str, dev: &DeviceShared) -> Result<bool, Refused> {
        if let Some(known) = guard(&self.supported).get(serial) {
            return Ok(*known);
        }
        let req = request(
            dev.handle,
            Dir::Get,
            gext::EXT_LIST,
            Vec::new(),
            EXT_LIST_LEN,
        );
        let has = match exec(dev, req, true) {
            Ok(b) => dspi_proto::ext::decode_list(&b).contains(&gext::EXT_IDENTIFY),
            Err(Failed { stalled: true, .. }) => false,
            Err(f) => {
                let mut r = refusal_of(f);
                r.msg = format!("could not read the device's extensions: {}", r.msg);
                return Err(r);
            }
        };
        let mut known = guard(&self.supported);
        if known.len() >= MAX_REMEMBERED {
            known.clear();
        }
        known.insert(serial.to_owned(), has);
        Ok(has)
    }

    fn identify(&self, hub: &HubHandle, session: &Session, msg: &Value) -> Result<Value, Refused> {
        if session.role() == Role::Viewer {
            return Err(Refused::new("denied", "identify needs the control role"));
        }
        let ask = IdentifyAsk::parse(msg)?;
        let device = ask.resolve(&hub.devices())?;
        let serial = device.identity.serial.clone();
        let dev = hub
            .device_by_serial(&serial)
            .ok_or_else(|| Refused::new("offline", "the device is not attached"))?;
        if !self.supports(&serial, &dev)? {
            return Err(Refused::new(
                "unsupported",
                "this device's firmware has no identify",
            ));
        }
        if ask.tone_output.is_some()
            && let Some(holder) = dev.router.lock_holder(Instant::now())
            && holder != session.id()
        {
            return Err(Refused::new(
                "locked",
                "another session holds the device's lock: the tone waits, a blink does not",
            ));
        }
        if let Err(wait) = guard(&self.rate).take(&serial, hub.clock().now()) {
            let ms = u64::try_from(wait.as_millis()).unwrap_or(u64::MAX).max(1);
            return Err(Refused {
                code: "rate_limited",
                msg: "one identify per device every 2 s".into(),
                extra: Some(("retry_in_ms", json!(ms))),
            });
        }
        let out = IdentifyRequest {
            seconds: ask.seconds,
            tone_output: ask.tone_output,
        };
        let write = request(
            dev.handle,
            Dir::Set,
            gext::EXT_IDENTIFY,
            out.encode().to_vec(),
            0,
        );
        exec(&dev, write, true).map_err(refusal_of)?;
        let by = hub.session_label(session.id()).by();
        self.bus.publish(BusMessage::Event {
            serial: serial.clone(),
            record: record(&ask, session.id(), by, hub.clock().unix_ms()),
        });
        let read = request(
            dev.handle,
            Dir::Get,
            gext::EXT_IDENTIFY,
            Vec::new(),
            u16::try_from(IdentifyStatus::SIZE).unwrap_or(u16::MAX),
        );
        let status = exec(&dev, read, true)
            .ok()
            .and_then(|b| IdentifyStatus::decode(&b))
            .map_or(Value::Null, |s| status_json(&s));
        Ok(json!({"serial": serial, "status": status}))
    }
}

impl HubService for Identify {
    fn name(&self) -> &'static str {
        "identify"
    }

    fn caps(&self) -> Vec<&'static str> {
        vec![CAP]
    }

    fn handle_message(
        &self,
        hub: &HubHandle,
        session: &Arc<Session>,
        msg: &Value,
    ) -> Option<String> {
        if msg.get("t")?.as_str()? != MESSAGE {
            return None;
        }
        let id = msg.get("id").cloned().unwrap_or(Value::Null);
        let mut reply = match self.identify(hub, session, msg) {
            Ok(body) => body,
            Err(r) => {
                let mut e = json!({"t": "err", "code": r.code, "msg": r.msg});
                if let (Some((k, v)), Some(map)) = (r.extra, e.as_object_mut()) {
                    map.insert(k.into(), v);
                }
                e
            }
        };
        if let Some(map) = reply.as_object_mut() {
            map.entry("t").or_insert_with(|| json!("ok"));
            map.insert("id".into(), id);
            map.insert("v".into(), json!(1));
        }
        Some(reply.to_string())
    }

    fn on_device(&self, event: &DeviceEvent) {
        let serial = match event {
            DeviceEvent::Online { serial, .. }
            | DeviceEvent::Offline { serial, .. }
            | DeviceEvent::Removed { serial, .. } => serial,
        };
        guard(&self.supported).remove(serial);
    }
}

#[cfg(test)]
#[path = "identify/tests_wp_p2id.rs"]
mod tests_wp_p2id;
