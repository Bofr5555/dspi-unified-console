//! The demo hub's simulated microphone for a system measurement (WP ATDEMO):
//! the room of `uconsole --demo hub --alink --with-measure-units` hears the
//! demo units' generators (the hub renders every start its fake units
//! recorded) and this core reads what it heard with
//! `system.measure.demo_mic` on every tick, pushing it into the session as a
//! microphone's capture. The client then opens no microphone and pushes
//! nothing (a push is ignored while it is on). Every screen labels it
//! [`crate::SIM_MIC_LABEL`] ("Simulated microphone"), as P5DEMO's.
//!
//! **Never with a real device** (fail closed twice): the core offers it only
//! when the hub's `hello.caps` carry `system_measure_demo_mic` (the demo
//! hub's room), turns it off when the session's hub no longer does, and asks
//! the hub before every session whether the room hears every unit of it;
//! the hub answers that only for its own demo attachments, so a session with
//! any other unit is refused before anything plays, and a refusal during a
//! session aborts it. Nothing is played or captured on this machine.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use dspi_measure::system::messages::demo::{self, DemoMicReply, DemoMicRequest};

use super::session::SysSession;
use super::{Hub, MeasureSystemIssue};
use crate::{DspiCore, DspiError, Result};

/// The notice of a refused simulated microphone.
pub(crate) const KEY_REFUSED: &str = "measure.system.sim_refused";
/// Reads of the room per tick at most (a client that ticked late catches up).
const MAX_READS: usize = 8;

/// The simulated microphone of a system measurement (`measure_system_sim_info`).
/// Through the web and C bindings only (not UniFFI: no Android or Swift
/// screen measures several units yet).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MeasureSystemSimInfo {
    /// The session's hub is the demo hub with its simulated room.
    pub available: bool,
    /// It is on: the room is the microphone, the client pushes nothing.
    pub on: bool,
    /// What the screens call it ([`crate::SIM_MIC_LABEL`]).
    pub label: String,
    /// What the room is, in words (once a session asked the hub).
    pub room: String,
    /// Why the last session could not use it, if it could not.
    pub issue: Option<String>,
}

/// The slot's part (taken with the system slot).
#[derive(Debug, Default)]
pub(crate) struct SystemSim {
    on: bool,
    room: String,
    issue: Option<String>,
}

fn has_cap(hub: &dyn Hub) -> bool {
    hub.caps().is_some_and(|c| c.iter().any(|x| x == demo::CAP))
}

/// The refusal: the hub's own code, in words.
fn unsupported(msg: impl Into<String>) -> DspiError {
    DspiError::Link {
        code: "unsupported".into(),
        msg: msg.into(),
    }
}

fn invalid(msg: impl Into<String>) -> DspiError {
    DspiError::InvalidArgument { msg: msg.into() }
}

impl SystemSim {
    /// It is on (without asking the hub).
    pub(crate) fn is_on(&self) -> bool {
        self.on
    }

    /// On, and still on a hub with the room: off from here on when not.
    pub(crate) fn on(&mut self, hub: &dyn Hub) -> bool {
        if self.on && !has_cap(hub) {
            self.on = false;
        }
        self.on
    }

    fn info(&self, hub: &dyn Hub) -> MeasureSystemSimInfo {
        let available = has_cap(hub);
        MeasureSystemSimInfo {
            available,
            on: self.on && available,
            label: crate::SIM_MIC_LABEL.to_owned(),
            room: self.room.clone(),
            issue: self.issue.clone(),
        }
    }

    /// Before a session: may the room hear every one of `serials`? An error
    /// (nothing plays) when not.
    pub(crate) fn probe(&mut self, hub: &mut dyn Hub, serials: &[String]) -> Result<()> {
        let req = DemoMicRequest {
            units: serials.to_vec(),
            ..DemoMicRequest::default()
        };
        let r = hub
            .request(demo::T, serde_json::to_value(&req).unwrap_or(Value::Null))
            .and_then(|v| {
                serde_json::from_value::<DemoMicReply>(v).map_err(|e| DspiError::Link {
                    code: "protocol".into(),
                    msg: format!("the hub's simulated microphone cannot be read: {e}"),
                })
            });
        match r {
            Ok(r) if r.ok => {
                self.room = r.room;
                self.issue = None;
                Ok(())
            }
            Ok(_) => Err(self.refused("the hub's room does not hear these units")),
            Err(e) => Err(self.refused(&e.to_string())),
        }
    }

    fn refused(&mut self, why: &str) -> DspiError {
        let m = format!("the simulated microphone cannot be used: {why}");
        self.issue = Some(m.clone());
        unsupported(m)
    }

    /// During a session: read what the room heard since the session's last
    /// frame and push it. A refusal aborts the session; a session the hub
    /// ended is the status poll's to report.
    pub(crate) fn feed(&mut self, hub: &mut dyn Hub, s: &mut SysSession) {
        for _ in 0..MAX_READS {
            if !s.wants_capture() || s.hub_session().is_empty() {
                return;
            }
            let req = DemoMicRequest {
                session: s.hub_session().to_owned(),
                from: s.next_capture_frame(),
                ..DemoMicRequest::default()
            };
            let got = hub
                .request(demo::T, serde_json::to_value(&req).unwrap_or(Value::Null))
                .and_then(|v| {
                    serde_json::from_value::<DemoMicReply>(v).map_err(|e| DspiError::Link {
                        code: "protocol".into(),
                        msg: format!("the hub's simulated microphone cannot be read: {e}"),
                    })
                });
            let r = match got {
                Ok(r) => r,
                Err(DspiError::Link { code, msg }) if code == "unsupported" => {
                    let m = format!("the simulated microphone cannot be used: {msg}");
                    self.issue = Some(m.clone());
                    s.abort_with(hub, MeasureSystemIssue::new("", KEY_REFUSED, None, m));
                    return;
                }
                Err(e) => {
                    log::debug!("system measurement: the simulated microphone: {e}");
                    return;
                }
            };
            if r.samples.is_empty() {
                return;
            }
            let n = r.samples.len();
            let samples: Vec<f32> = r
                .samples
                .iter()
                .map(|v| *v as f32 / demo::FULL_SCALE)
                .collect();
            s.push(hub, r.first_frame, &samples, r.rate_hz, false);
            if n < demo::MAX_FRAMES as usize {
                return;
            }
        }
    }
}

impl DspiCore {
    /// The demo hub's simulated microphone for a measurement across units:
    /// whether this core's hub has it (only `uconsole --demo hub --alink
    /// --with-measure-units` does) and whether it is on. No I/O.
    pub fn measure_system_sim_info(&self) -> MeasureSystemSimInfo {
        let mut slot = self.system_slot();
        let mut sim = std::mem::take(&mut slot.sim);
        let info = self.with_hub(&mut slot, |hub| {
            sim.on(hub);
            sim.info(hub)
        });
        slot.sim = sim;
        info
    }

    /// Turn the demo hub's simulated microphone on or off for the next
    /// measurements across units. On, the hub's room is the microphone: the
    /// client opens none and its pushes are ignored; label it "Simulated
    /// microphone". Before every session the hub is asked whether its room
    /// hears every unit; it hears only its own demo units, so with any other
    /// unit the session does not start.
    ///
    /// Errors: [`DspiError::Link`] `unsupported` to turn it on where the hub
    /// has no room (any hub but the demo's); [`DspiError::InvalidArgument`]
    /// while a measurement across units runs.
    pub fn measure_system_sim_set(&self, on: bool) -> Result<MeasureSystemSimInfo> {
        let mut slot = self.system_slot();
        if slot.session.as_ref().is_some_and(SysSession::active) {
            return Err(invalid(
                "a measurement across units is running: stop it first",
            ));
        }
        let mut sim = std::mem::take(&mut slot.sim);
        let r = self.with_hub(&mut slot, |hub| {
            if on && !has_cap(hub) {
                return Err(unsupported(
                    "the simulated microphone is the demo hub's only (uconsole --demo hub --alink --with-measure-units)",
                ));
            }
            sim.on = on;
            sim.issue = None;
            Ok(sim.info(hub))
        });
        slot.sim = sim;
        r
    }
}
