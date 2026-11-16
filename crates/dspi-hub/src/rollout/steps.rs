//! A rollout's steps (WP P3RO, PLAN-P3 section 1.1, `hub-api.md` 21.3): `fw.rollout.start`
//! runs steps 1 and 2 of the first device on the asking connection, then one thread runs
//! the rest, one device after another, stopped at the first failure.
//!
//! Every device request goes through the executor as the rollout's Admin principal's
//! child session; the boot ROM is reached through the [`super::BootScan`] and
//! [`super::Writer`] seams only. Nothing is retried; the core voltage is read, never
//! written.

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use dspi_core::{
    BootDevice, ConfirmOp, FirmwareGuardBoard, FirmwareGuardBoardSource, FirmwareGuardDecision,
    FirmwareGuardReason, ParamValue, firmware_guard_check,
};
use dspi_session::events::{EventKind, EventRecord};
use serde_json::{Value, json};

use super::images::ImageInfo;
use super::{
    After, BACK_WITHIN, Before, BoardRecord, DeviceEntry, FIND_EVERY, FIND_FOR, Failed, ImageRef,
    Record, Running, Shared, StepDone, VregRecord, cut, lock,
};
use crate::api::ApiError;
use crate::bus::BusMessage;
use crate::hub::HubHandle;
use crate::registry::DeviceState;
use crate::session::{Role, Session};

/// The stock core voltage: a board that needs more is at risk, and has its setting read
/// back after the flash.
const STOCK_MV: u16 = 1150;
/// How long the waits sleep between looks (real time), checking the stop flag.
const TICK: Duration = Duration::from_millis(100);

/// The image a rollout puts on: its facts and bytes.
pub(super) struct Image {
    info: ImageInfo,
    bytes: Vec<u8>,
}

/// What steps 1 and 2 of a device found.
struct Cleared {
    board: FirmwareGuardBoard,
    check_text: String,
    guard_text: String,
}

/// One rollout as its thread runs it.
struct Run {
    sh: Arc<Shared>,
    hub: HubHandle,
    principal: Arc<Session>,
    id: String,
    image: Image,
    while_live: bool,
    serials: Vec<String>,
}

fn volts(mv: u16) -> String {
    format!("{}.{:02} V", mv / 1000, mv % 1000 / 10)
}

fn source_name(s: FirmwareGuardBoardSource) -> &'static str {
    match s {
        FirmwareGuardBoardSource::NoDevice => "noDevice",
        FirmwareGuardBoardSource::Bootloader => "bootloader",
        FirmwareGuardBoardSource::ReadFailed => "readFailed",
        FirmwareGuardBoardSource::ExtCoreVreg => "extCoreVreg",
        FirmwareGuardBoardSource::StatusBlock => "statusBlock",
    }
}

fn reason_name(r: FirmwareGuardReason) -> &'static str {
    match r {
        FirmwareGuardReason::ImageInvalid => "image_invalid",
        FirmwareGuardReason::NoRisk => "no_risk",
        FirmwareGuardReason::ImageUnknown => "image_unknown",
        FirmwareGuardReason::ImageLower => "image_lower",
        FirmwareGuardReason::BoardUnknown => "board_unknown",
        FirmwareGuardReason::Overvolt => "overvolt",
    }
}

fn image_refused(reason: &str, msg: impl Into<String>) -> ApiError {
    ApiError::new("image_refused", msg).with("reason", json!(reason))
}

/// `EXT_CORE_VREG` as the record keeps it.
fn vreg_record(v: &dspi_core::CoreVregInfo) -> VregRecord {
    let stored = if v.stored_sel == 0 {
        v.default_vreg
    } else {
        v.stored_sel
    };
    VregRecord {
        live_mv: dspi_proto::ext::vreg_mv(v.live_vreg),
        stored_mv: dspi_proto::ext::vreg_mv(stored),
        default_mv: dspi_proto::ext::vreg_mv(v.default_vreg),
    }
}

/// A document without what differs between two captures of the same settings (`meta`:
/// the time and the app; `snapshot`: the capture's id and time).
fn settings_of(doc: &str) -> Option<Value> {
    let mut v: Value = serde_json::from_str(doc).ok()?;
    let map = v.as_object_mut()?;
    map.remove("meta");
    map.remove("snapshot");
    Some(v)
}

// ── fw.rollout.start ────────────────────────────────────────────────────────

/// `fw.rollout.start`: refused early (no record, no alarm) unless steps 1 and 2 of the
/// first device pass; then the record, and the rest on the rollout's thread.
pub(super) fn start(
    sh: &Arc<Shared>,
    hub: &HubHandle,
    session: &Arc<Session>,
    msg: &Value,
) -> Result<Value, ApiError> {
    if session.role() < Role::Admin {
        return Err(ApiError::new(
            "denied",
            "a firmware rollout is an admin's: this role may not start one",
        ));
    }
    let image_name = msg
        .get("image")
        .and_then(Value::as_str)
        .ok_or_else(|| ApiError::bad("image is a stored image's name"))?
        .to_owned();
    let while_live = match msg.get("while_live") {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(_) => return Err(ApiError::bad("while_live is true or false")),
    };
    {
        let mut st = lock(&sh.state);
        if st.starting || st.running.is_some() {
            return Err(ApiError::new(
                "busy",
                "a rollout runs on this hub; one at a time",
            ));
        }
        st.starting = true;
    }
    let started = begin(sh, hub, session, msg, &image_name, while_live);
    if started.is_err() {
        lock(&sh.state).starting = false;
    }
    started
}

/// The target: `(target text, serials in order)`.
fn targets(sh: &Shared, hub: &HubHandle, msg: &Value) -> Result<(String, Vec<String>), ApiError> {
    let group = match msg.get("group") {
        None | Some(Value::Null) => None,
        Some(Value::String(g)) => Some(crate::groups::group_id(g).unwrap_or(g).to_owned()),
        Some(_) => return Err(ApiError::bad("group is a group's id (\"g1\")")),
    };
    let device = msg.get("serial").is_some_and(|v| !v.is_null())
        || msg.get("handle").is_some_and(|v| !v.is_null());
    match (group, device) {
        (Some(_), true) => Err(ApiError::bad("name a device or a group, not both")),
        (None, false) => Err(ApiError::bad("serial (or handle) or group is required")),
        (None, true) => {
            let (serial, _) = crate::api::target(hub, msg)?;
            Ok((serial.clone(), vec![serial]))
        }
        (Some(g), false) => {
            let members = sh
                .groups
                .as_ref()
                .and_then(|r| r.members(&g))
                .ok_or_else(|| ApiError::new("not_found", format!("no group {g}")))?;
            if members.is_empty() {
                return Err(ApiError::bad(format!("group {g} has no members")));
            }
            Ok((format!("{}{g}", crate::groups::TARGET_PREFIX), members))
        }
    }
}

fn begin(
    sh: &Arc<Shared>,
    hub: &HubHandle,
    session: &Arc<Session>,
    msg: &Value,
    image_name: &str,
    while_live: bool,
) -> Result<Value, ApiError> {
    let (target, serials) = targets(sh, hub, msg)?;
    let (info, bytes) = sh
        .images
        .load(image_name, sh.clock.unix_seconds())
        .map_err(|e| {
            if e.code == "image_refused" {
                image_refused("image_invalid", e.msg)
            } else {
                ApiError::new(e.code, e.msg)
            }
        })?;
    let image = Image { info, bytes };
    let by = hub
        .session_label(session.id())
        .by()
        .unwrap_or_else(|| format!("Session {}", session.id()));
    // The rollout's principal: the Admin's role, client and name, a remote-range id. It
    // hears nothing (nobody reads its outbox).
    let principal = hub
        .open_principal(Role::Admin, session.client_id(), &by)
        .map_err(|_| ApiError::internal("the hub is stopping"))?;
    hub.inner.relay.remove_session(principal.id());
    let first = serials.first().cloned().unwrap_or_default();
    let cleared = match check_and_guard(sh, hub, &principal, &image, while_live, &first) {
        Ok(c) => c,
        Err((_, e)) => {
            hub.close_session(principal.id());
            hub.wake_services();
            return Err(e);
        }
    };
    let now = sh.now_iso();
    let record = {
        let mut st = lock(&sh.state);
        let id = format!("ro{}", st.next_id);
        st.next_id = st.next_id.saturating_add(1);
        let mut devices: Vec<DeviceEntry> =
            serials.iter().map(|s| DeviceEntry::pending(s)).collect();
        if let Some(d) = devices.first_mut() {
            d.state = "running".into();
            d.step = "export".into();
            d.steps = vec![
                step_ok("check", &now, &cleared.check_text),
                step_ok("guard", &now, &cleared.guard_text),
            ];
        }
        let record = Record {
            id: id.clone(),
            target,
            image: ImageRef {
                name: image.info.name.clone(),
                sha256: image.info.sha256.clone(),
                firmware_version: image.info.firmware_version.clone(),
            },
            by,
            while_live,
            state: "running".into(),
            started_at: now,
            ended_at: None,
            devices,
        };
        st.records.push(record.clone());
        while st.records.len() > super::MAX_RECORDS {
            st.records.remove(0);
        }
        st.starting = false;
        st.running = Some(Running {
            id,
            image: image.info.name.clone(),
            cancelled: false,
            past_entry: false,
        });
        sh.persist_locked(&st);
        record
    };
    log::warn!(
        "hub rollout: {} started by {}: {} onto {}",
        record.id,
        record.by,
        record.image.name,
        record.target
    );
    sh.announce(hub, &record);
    let principal_id = principal.id();
    let run = Run {
        sh: Arc::clone(sh),
        hub: hub.clone(),
        principal,
        id: record.id.clone(),
        image,
        while_live,
        serials,
    };
    let spawned = std::thread::Builder::new()
        .name("hub-rollout".into())
        .spawn(move || run.run(cleared.board));
    match spawned {
        Ok(handle) => {
            let old = lock(&sh.worker).replace(handle);
            if let Some(old) = old {
                let _ = old.join();
            }
        }
        Err(e) => {
            // Nothing was touched yet: the record ends failed, the slot is free again.
            log::error!("hub rollout: no thread for {}: {e}", record.id);
            let ended = sh.now_iso();
            sh.update(hub, &record.id, |r| {
                for d in &mut r.devices {
                    d.state = "not_tried".into();
                }
                r.state = "failed".into();
                r.ended_at = Some(ended);
            });
            lock(&sh.state).running = None;
            hub.close_session(principal_id);
            return Err(ApiError::internal("the rollout could not start"));
        }
    }
    Ok(json!({"rollout": record.to_json()}))
}

fn step_ok(step: &str, at: &str, text: &str) -> StepDone {
    StepDone {
        step: step.into(),
        at: at.into(),
        ok: true,
        text: cut(text),
        session: None,
    }
}

// ── Steps 1 and 2 ───────────────────────────────────────────────────────────

/// Steps 1 and 2 of `serial`: the step that refused, and why.
fn check_and_guard(
    sh: &Shared,
    hub: &HubHandle,
    principal: &Arc<Session>,
    image: &Image,
    while_live: bool,
    serial: &str,
) -> Result<Cleared, (&'static str, ApiError)> {
    let check_text =
        check(sh, hub, principal, image, while_live, serial).map_err(|e| ("check", e))?;
    let (board, guard_text) = guard(sh, hub, principal, image, serial).map_err(|e| ("guard", e))?;
    Ok(Cleared {
        board,
        check_text,
        guard_text,
    })
}

/// The device's lock and a pending guard: what must be clear right up to the bootloader
/// entry. The installer lock is the hub's to judge on the entry itself, as for any write:
/// the rollout's Admin principal passes it (PLAN-P3 step 4).
fn locks_clear(hub: &HubHandle, serial: &str, handle: u8) -> Result<(), ApiError> {
    if let Some(dev) = hub.device_by_serial(serial)
        && dev.router.lock_holder(Instant::now()).is_some()
    {
        return Err(ApiError::new(
            "locked",
            "a session holds this device's lock; release it first",
        ));
    }
    if hub
        .apply_status(handle)
        .get("active")
        .is_some_and(|a| !a.is_null())
    {
        return Err(ApiError::new(
            "conflict",
            "a guarded apply is pending on this device; confirm or cancel it first",
        ));
    }
    Ok(())
}

/// Step 1 (`hub-api.md` 21.3).
fn check(
    sh: &Shared,
    hub: &HubHandle,
    principal: &Arc<Session>,
    image: &Image,
    while_live: bool,
    serial: &str,
) -> Result<String, ApiError> {
    sh.refresh_health();
    let devices = hub.devices();
    let d = devices
        .iter()
        .find(|d| d.identity.serial == serial)
        .ok_or_else(|| ApiError::new("not_found", format!("no device {serial} is known")))?;
    if d.state != DeviceState::Online {
        return Err(ApiError::offline());
    }
    if d.identity.vid == 0 {
        return Err(ApiError::new(
            "unsupported",
            "this device is not on this hub's own USB; only those are flashed",
        ));
    }
    if sh.is_unresponsive(serial) {
        return Err(ApiError::new(
            "offline",
            "the device is not answering; nothing was sent",
        ));
    }
    locks_clear(hub, serial, d.handle)?;
    let platform = i8::try_from(d.identity.platform).unwrap_or(-1);
    if image.info.platform_id() != platform {
        return Err(image_refused(
            "platform",
            format!(
                "{} is an {} image; this device is not",
                image.info.name, image.info.platform
            ),
        ));
    }
    if !image.info.verified && !sh.allow_unverified {
        return Err(image_refused(
            "not_verified",
            format!(
                "{} is not verified (the release's manifest does not list it); this hub flashes \
                 verified images only",
                image.info.name
            ),
        ));
    }
    if image.info.firmware_version.is_empty() {
        return Err(image_refused(
            "image_invalid",
            "the image carries no DSPi version, so the device could not be verified after",
        ));
    }
    let boards = sh.scan.boards().len();
    if boards != 0 {
        return Err(ApiError::new(
            "conflict",
            format!("{boards} board(s) are in BOOTSEL already; a rollout starts with none"),
        ));
    }
    if !while_live {
        let muted = sh.exec.run(hub, principal, serial, |cx| {
            let reader = cx.reader()?;
            let mute = reader
                .read("vol.mute", &[])
                .map(ParamValue::from)
                .map_err(|e| ApiError::new("device", e.to_string()))?;
            let master = reader
                .read("vol.master", &[])
                .map(ParamValue::from)
                .map_err(|e| ApiError::new("device", e.to_string()))?;
            Ok(matches!(mute, ParamValue::Bool(true))
                || matches!(master, ParamValue::Float(db) if db <= -127.5))
        })?;
        if !muted {
            return Err(ApiError::new(
                "confirmation_required",
                "the device is not muted: mute it first, or start with \"while_live\": true to \
                 accept that its audio stops for about 30 s",
            ));
        }
    }
    Ok("The device and the image can go ahead".into())
}

/// Step 2: the board read as the principal's child, the guard asked with nothing typed.
fn guard(
    sh: &Shared,
    hub: &HubHandle,
    principal: &Arc<Session>,
    image: &Image,
    serial: &str,
) -> Result<(FirmwareGuardBoard, String), ApiError> {
    let board = sh.exec.run(hub, principal, serial, |cx| {
        Ok(cx.core()?.firmware_guard_board())
    })?;
    let verdict = firmware_guard_check(image.bytes.clone(), board.clone(), String::new());
    if verdict.decision != FirmwareGuardDecision::Clear || !verdict.allowed {
        log::warn!(
            "hub rollout: the guard refused {} for {serial}: {}",
            image.info.name,
            verdict.message
        );
        return Err(image_refused(reason_name(verdict.reason), verdict.message));
    }
    let text = if board.required_mv >= STOCK_MV {
        format!("The image suits this board ({})", volts(board.required_mv))
    } else {
        "The image suits this board".to_owned()
    };
    Ok((board, text))
}

// ── The rollout's thread ────────────────────────────────────────────────────

/// How a device's part ended.
enum End {
    Ok(String),
    Failed {
        step: &'static str,
        code: Option<String>,
        text: String,
    },
    /// Cancelled (the record says so already) or the hub is stopping.
    Stopped,
}

impl Run {
    fn stopping(&self) -> bool {
        self.sh.stop.load(Ordering::SeqCst)
    }

    fn cancelled(&self) -> bool {
        lock(&self.sh.state)
            .running
            .as_ref()
            .is_none_or(|r| r.id != self.id || r.cancelled)
    }

    fn at(&self, i: usize, step: &str) -> bool {
        let step = step.to_owned();
        self.sh.update(&self.hub, &self.id, |r| {
            if let Some(d) = r.devices.get_mut(i) {
                d.state = "running".into();
                d.step = step;
            }
        })
    }

    fn done(&self, i: usize, step: &str, text: &str, session: Option<u32>) -> bool {
        let at = self.sh.now_iso();
        let s = StepDone {
            step: step.into(),
            at,
            ok: true,
            text: cut(text),
            session,
        };
        self.sh.update(&self.hub, &self.id, |r| {
            if let Some(d) = r.devices.get_mut(i) {
                d.steps.push(s);
            }
        })
    }

    fn run(self, first: FirmwareGuardBoard) {
        let mut board = Some(first);
        let mut ok = true;
        for i in 0..self.serials.len() {
            let serial = self.serials.get(i).cloned().unwrap_or_default();
            // A new device: a cancel holds again until its bootloader entry.
            if let Some(r) = lock(&self.sh.state)
                .running
                .as_mut()
                .filter(|r| r.id == self.id)
            {
                r.past_entry = false;
            }
            let end = self.device(i, &serial, board.take());
            match end {
                End::Ok(text) => self.finish_device(i, &serial, None, &text),
                End::Failed { step, code, text } => {
                    self.finish_device(i, &serial, Some((step, code)), &text);
                    ok = false;
                    break;
                }
                End::Stopped => {
                    ok = false;
                    break;
                }
            }
        }
        let ended = self.sh.now_iso();
        let state = if ok { "ok" } else { "failed" };
        // -- review p3d -- the slot is freed with the record's end, before it is announced
        self.sh.end(&self.hub, &self.id, |r| {
            for d in &mut r.devices {
                if d.state == "pending" {
                    d.state = "not_tried".into();
                }
                // Stopped with the hub before it ended.
                if d.state == "running" {
                    d.state = "failed".into();
                    d.failed = Some(Failed {
                        step: d.step.clone(),
                        code: None,
                        text: "The hub stopped during this device's rollout".into(),
                    });
                }
            }
            r.state = state.into();
            r.ended_at = Some(ended);
        });
        self.hub.close_session(self.principal.id());
        self.hub.wake_services();
        log::warn!("hub rollout: {} ended", self.id);
    }

    /// Record a device's end: its entry, its history `firmware` record, `RolloutEnded`.
    fn finish_device(
        &self,
        i: usize,
        serial: &str,
        failed: Option<(&'static str, Option<String>)>,
        text: &str,
    ) {
        let at = self.sh.now_iso();
        let failed_entry = failed.as_ref().map(|(step, code)| Failed {
            step: (*step).to_owned(),
            code: code.clone(),
            text: cut(text),
        });
        let mut lines = Vec::new();
        let written = self.sh.update(&self.hub, &self.id, |r| {
            if let Some(d) = r.devices.get_mut(i) {
                if let Some((step, _)) = &failed {
                    d.steps.push(StepDone {
                        step: (*step).to_owned(),
                        at,
                        ok: false,
                        text: cut(text),
                        session: None,
                    });
                    d.step = (*step).to_owned();
                    d.state = "failed".into();
                } else {
                    d.state = "ok".into();
                }
                d.failed = failed_entry;
                lines = d
                    .steps
                    .iter()
                    .map(|s| format!("{}: {}", s.step, s.text))
                    .collect();
            }
        });
        if !written {
            return;
        }
        let fw = self.image.info.firmware_version.clone();
        let summary = match &failed {
            None => format!(
                "Firmware {fw} put on by the hub ({}): {}",
                self.id,
                cut(text)
            ),
            Some((step, _)) => format!(
                "Firmware rollout {} of {fw} failed at {step}: {}",
                self.id,
                cut(text)
            ),
        };
        let mut record = EventRecord::new(EventKind::Firmware, self.sh.clock.unix_ms(), summary);
        record.lines = lines;
        record.session = Some(u32::from(self.principal.id()));
        record.by = self.hub.session_label(self.principal.id()).by();
        record.source = Some("rollout".into());
        self.sh.bus.publish(BusMessage::Event {
            serial: serial.to_owned(),
            record,
        });
        self.sh.bus.publish(BusMessage::RolloutEnded {
            serial: serial.to_owned(),
            ok: failed.is_none(),
            step: failed.map_or("verify", |(s, _)| s).to_owned(),
            text: cut(text),
            firmware_version: fw,
        });
    }

    /// One device, steps 1 (for a later member) to 8.
    fn device(&self, i: usize, serial: &str, cleared: Option<FirmwareGuardBoard>) -> End {
        let fail = |step: &'static str, e: ApiError| End::Failed {
            step,
            code: Some(e.code.to_owned()),
            text: e.msg,
        };
        let board = match cleared {
            Some(b) => b,
            None => {
                if self.cancelled() || self.stopping() || !self.at(i, "check") {
                    return End::Stopped;
                }
                match check_and_guard(
                    &self.sh,
                    &self.hub,
                    &self.principal,
                    &self.image,
                    self.while_live,
                    serial,
                ) {
                    Ok(c) => {
                        if !(self.done(i, "check", &c.check_text, None)
                            && self.done(i, "guard", &c.guard_text, None))
                        {
                            return End::Stopped;
                        }
                        c.board
                    }
                    Err((step, e)) => return fail(step, e),
                }
            }
        };
        // Step 3: the export.
        if self.cancelled() || self.stopping() || !self.at(i, "export") {
            return End::Stopped;
        }
        let before = match self.export(serial, &board) {
            Ok(b) => b,
            Err(e) => return fail("export", e),
        };
        let export_text = format!(
            "Settings exported to the hub's snapshot {}",
            before.export.as_deref().unwrap_or("")
        );
        let before_kept = before.clone();
        if !self.sh.update(&self.hub, &self.id, |r| {
            if let Some(d) = r.devices.get_mut(i) {
                d.before = Some(before_kept);
            }
        }) || !self.done(i, "export", &export_text, None)
        {
            return End::Stopped;
        }
        // Step 4: past here nothing cancels.
        {
            let mut st = lock(&self.sh.state);
            match st.running.as_mut() {
                Some(r) if r.id == self.id && !r.cancelled && !self.stopping() => {
                    r.past_entry = true;
                }
                _ => return End::Stopped,
            }
        }
        let _ = self.at(i, "bootloader");
        // Subscribed before the device leaves, so its return is not missed.
        let back = self.sh.bus.subscribe_filtered("rollout-wait", 64, |m| {
            matches!(m, BusMessage::DeviceOnline { .. })
        });
        let session = match self.enter(serial) {
            Ok(s) => s,
            Err(e) => return fail("bootloader", e),
        };
        let _ = self.done(
            i,
            "bootloader",
            "Sent to its bootloader; it leaves the bus",
            Some(session),
        );
        // Step 5.
        let _ = self.at(i, "find");
        let target = match self.find(serial) {
            Ok(t) => t,
            Err(text) => {
                return End::Failed {
                    step: "find",
                    code: None,
                    text,
                };
            }
        };
        let _ = self.done(
            i,
            "find",
            &format!("Found in BOOTSEL at {}", target.bus_location),
            None,
        );
        // Step 6.
        let _ = self.at(i, "flash");
        let verdict = firmware_guard_check(self.image.bytes.clone(), board.clone(), String::new());
        if !verdict.allowed || verdict.decision != FirmwareGuardDecision::Clear {
            return End::Failed {
                step: "flash",
                code: Some("image_refused".into()),
                text: cut(&verdict.message),
            };
        }
        // Only a return after the write counts.
        let _ = back.drain();
        let report = match self.sh.writer.flash(&self.image.bytes, &target, &board) {
            Ok(r) => r,
            Err(text) => {
                return End::Failed {
                    step: "flash",
                    code: None,
                    text: cut(&text),
                };
            }
        };
        let _ = self.done(
            i,
            "flash",
            &format!(
                "Written and read back: {} sectors in {} ms; restarting",
                report.sectors, report.elapsed_ms
            ),
            None,
        );
        // Step 7.
        let _ = self.at(i, "wait");
        if let Err(text) = self.wait(serial, &back) {
            return End::Failed {
                step: "wait",
                code: None,
                text,
            };
        }
        let _ = self.done(i, "wait", "Back on the bus", None);
        // Step 8.
        let _ = self.at(i, "verify");
        match self.verify(serial, &before, &board) {
            Ok((after, text)) => {
                let _ = self.sh.update(&self.hub, &self.id, |r| {
                    if let Some(d) = r.devices.get_mut(i) {
                        d.after = Some(after);
                    }
                });
                let _ = self.done(i, "verify", &text, None);
                End::Ok(text)
            }
            Err((after, text)) => {
                let _ = self.sh.update(&self.hub, &self.id, |r| {
                    if let Some(d) = r.devices.get_mut(i) {
                        d.after = after;
                    }
                });
                End::Failed {
                    step: "verify",
                    code: None,
                    text,
                }
            }
        }
    }

    /// Step 3: the settings into the hub's snapshot store, and the facts kept.
    fn export(&self, serial: &str, board: &FirmwareGuardBoard) -> Result<Before, ApiError> {
        let date: String = self.sh.now_iso().chars().take(10).collect();
        let name = format!(
            "Before firmware {}, {date}",
            self.image.info.firmware_version
        );
        let note = format!("Exported by the hub before firmware rollout {}", self.id);
        let (doc, ext_list, vreg) = self.sh.exec.run(&self.hub, &self.principal, serial, |cx| {
            let core = cx.core()?;
            let doc = core
                .snapshot_capture_document(name.clone(), note.clone())
                .map_err(ApiError::from)?;
            let ext = core.ext_status().map(|e| e.supported).unwrap_or_default();
            let vreg = core.fetch_core_vreg().ok().map(|v| vreg_record(&v));
            Ok((doc, ext, vreg))
        })?;
        let doc = self.with_store_id(serial, &doc)?;
        let put = self
            .sh
            .snapshots
            .put(serial, &doc)
            .map_err(|e| ApiError::new(e.code(), e.to_string()))?;
        for event in self.sh.snapshots.take_changed(self.hub.hub_id()) {
            self.hub.inner.relay.broadcast_event(&event);
        }
        let fw = self
            .hub
            .devices()
            .into_iter()
            .find(|d| d.identity.serial == serial)
            .map(|d| d.identity.firmware)
            .unwrap_or_default();
        Ok(Before {
            fw,
            ext_list,
            board: BoardRecord {
                required_mv: board.required_mv,
                source: source_name(board.source).into(),
            },
            vreg,
            export: Some(put.entry.id),
        })
    }

    /// `doc` (captured with no store) with the store id of its capture time
    /// (`snapshot_store::id_for`), one the hub's store does not hold for `serial` yet.
    fn with_store_id(&self, serial: &str, doc: &str) -> Result<String, ApiError> {
        use dspi_session::snapshot_store::{BLOCK, id_for};
        let mut v: Value = serde_json::from_str(doc)
            .map_err(|e| ApiError::internal(format!("the export: {e}")))?;
        let taken: Vec<String> = self
            .sh
            .snapshots
            .list(serial)
            .map_err(|e| ApiError::new(e.code(), e.to_string()))?
            .into_iter()
            .map(|e| e.id)
            .collect();
        let block = v
            .get_mut(BLOCK)
            .and_then(Value::as_object_mut)
            .ok_or_else(|| ApiError::internal("the export has no snapshot block"))?;
        let ms = block.get("savedMs").and_then(Value::as_i64).unwrap_or(0);
        let id = (0..1000u32)
            .map(|n| id_for(ms, n))
            .find(|id| !taken.contains(id))
            .ok_or_else(|| ApiError::new("conflict", "no free snapshot id for the export"))?;
        block.insert("id".into(), json!(id));
        serde_json::to_string_pretty(&v).map_err(|e| ApiError::internal(e.to_string()))
    }

    /// Step 4: `REQ_ENTER_BOOTLOADER` as the principal's child, its token minted and spent
    /// inside. Answers the child's session id.
    fn enter(&self, serial: &str) -> Result<u32, ApiError> {
        let handle = self
            .hub
            .devices()
            .into_iter()
            .find(|d| d.identity.serial == serial && d.state == DeviceState::Online)
            .map(|d| d.handle)
            .ok_or_else(ApiError::offline)?;
        locks_clear(&self.hub, serial, handle)?;
        self.sh.exec.run(&self.hub, &self.principal, serial, |cx| {
            let core = cx.core()?;
            let token = core.confirm_token(ConfirmOp::EnterBootloader);
            core.enter_bootloader_confirmed(Some(token))
                .map_err(ApiError::from)?;
            log::warn!(
                "hub rollout: {serial} sent to its bootloader as session {}",
                cx.child_id()
            );
            Ok(u32::from(cx.child_id()))
        })
    }

    /// Step 5: exactly one board in BOOTSEL, of the device's chip, on RP2350 with the
    /// device's serial.
    fn find(&self, serial: &str) -> Result<BootDevice, String> {
        let platform = self.image.info.platform_id();
        let deadline = self
            .sh
            .clock
            .now()
            .checked_add(FIND_FOR)
            .unwrap_or_else(|| self.sh.clock.now());
        loop {
            if self.stopping() {
                return Err("the hub stopped while looking for the board".into());
            }
            let boards = self.sh.scan.boards();
            match boards.as_slice() {
                [] => {}
                [b] => {
                    if b.platform != platform {
                        return Err(format!(
                            "the board in BOOTSEL is another chip than the image's; nothing was \
                             written (it is still in BOOTSEL at {})",
                            b.bus_location
                        ));
                    }
                    if platform == 1 {
                        let same = b
                            .serial
                            .as_deref()
                            .is_some_and(|s| s.eq_ignore_ascii_case(serial));
                        if !same {
                            return Err(format!(
                                "the board in BOOTSEL is not this device (its serial is {}); \
                                 nothing was written",
                                b.serial.as_deref().unwrap_or("not given")
                            ));
                        }
                    }
                    return Ok(b.clone());
                }
                more => {
                    return Err(format!(
                        "{} boards are in BOOTSEL; nothing was written, and the device may be \
                         left in BOOTSEL",
                        more.len()
                    ));
                }
            }
            if self.sh.clock.now() >= deadline {
                return Err(format!(
                    "no board appeared in BOOTSEL within {} s; the device may be left in \
                     BOOTSEL or still running its old firmware",
                    FIND_FOR.as_secs()
                ));
            }
            std::thread::sleep(FIND_EVERY);
        }
    }

    /// Step 7: the device online again within [`BACK_WITHIN`].
    fn wait(&self, serial: &str, back: &crate::bus::Subscription) -> Result<(), String> {
        let deadline = self
            .sh
            .clock
            .now()
            .checked_add(BACK_WITHIN)
            .unwrap_or_else(|| self.sh.clock.now());
        loop {
            if self.stopping() {
                return Err("the hub stopped while the device restarted".into());
            }
            while let Some(m) = back.try_recv() {
                if matches!(&*m, BusMessage::DeviceOnline { serial: s, .. } if s == serial)
                    && self.hub.device_by_serial(serial).is_some()
                {
                    return Ok(());
                }
            }
            if self.sh.clock.now() >= deadline {
                return Err(format!(
                    "the device did not come back within {} s",
                    BACK_WITHIN.as_secs()
                ));
            }
            if let Some(m) = back.recv_timeout(TICK)
                && matches!(&*m, BusMessage::DeviceOnline { serial: s, .. } if s == serial)
                && self.hub.device_by_serial(serial).is_some()
            {
                return Ok(());
            }
        }
    }

    /// Step 8: the version, `EXT_LIST`, the core voltage on a board that needs more than
    /// 1.15 V, then the settings against the export.
    #[allow(clippy::type_complexity)]
    fn verify(
        &self,
        serial: &str,
        before: &Before,
        board: &FirmwareGuardBoard,
    ) -> Result<(After, String), (Option<After>, String)> {
        let version = self.image.info.firmware_version.clone();
        let read = self.sh.exec.run(&self.hub, &self.principal, serial, |cx| {
            let core = cx.core()?;
            let matches = core
                .firmware_version_matches(&version)
                .map_err(ApiError::from)?;
            let ext = core.ext_status().map(|e| e.supported).unwrap_or_default();
            let vreg = core.fetch_core_vreg().ok().map(|v| vreg_record(&v));
            let doc = core
                .snapshot_capture_document("After the firmware rollout".into(), String::new())
                .ok();
            Ok((matches, ext, vreg, doc))
        });
        let (matches, ext_list, vreg, doc) = match read {
            Ok(r) => r,
            Err(e) => {
                return Err((
                    None,
                    cut(&format!("the device could not be read back: {}", e.msg)),
                ));
            }
        };
        let fw = self
            .hub
            .devices()
            .into_iter()
            .find(|d| d.identity.serial == serial)
            .map(|d| d.identity.firmware)
            .unwrap_or_default();
        let settings = match (
            doc.as_deref().and_then(settings_of),
            before
                .export
                .as_deref()
                .and_then(|id| self.sh.snapshots.get(id).ok())
                .and_then(|(_, text)| settings_of(&text)),
        ) {
            (Some(now), Some(then)) if now == then => "unchanged",
            (Some(_), Some(_)) => "changed",
            _ => "unknown",
        };
        let after = After {
            fw: fw.clone(),
            ext_list,
            vreg,
            settings: settings.into(),
        };
        if !matches {
            return Err((
                Some(after),
                format!("the device came back with firmware {fw}, not {version}"),
            ));
        }
        if after.ext_list.len() < before.ext_list.len() {
            return Err((
                Some(after.clone()),
                format!(
                    "the extension channel lists {} commands, fewer than the {} before",
                    after.ext_list.len(),
                    before.ext_list.len()
                ),
            ));
        }
        let mut lines = vec![format!("firmware {version}")];
        lines.push(format!(
            "extension channel {} commands",
            after.ext_list.len()
        ));
        if board.required_mv > STOCK_MV {
            let want = Some(board.required_mv);
            let v = after.vreg;
            let back = v.is_some_and(|v| v.live_mv == want && v.stored_mv == want);
            if !back {
                let seen = v.map_or_else(
                    || "nothing (no core voltage read)".to_owned(),
                    |v| {
                        format!(
                            "live {} and stored {}",
                            v.live_mv.map_or_else(|| "unknown".into(), volts),
                            v.stored_mv.map_or_else(|| "unknown".into(), volts)
                        )
                    },
                );
                return Err((
                    Some(after),
                    format!(
                        "this board needs {}, and the core voltage read back {seen}",
                        volts(board.required_mv)
                    ),
                ));
            }
            lines.push(format!(
                "core voltage {} read back",
                volts(board.required_mv)
            ));
        }
        lines.push(format!("settings {settings}"));
        Ok((after, format!("Verified: {}", lines.join(", "))))
    }
}
