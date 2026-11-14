//! Scenes applied at a frame (WP SW2, SYS2 3.4, `extension_channel_spec.md`
//! 2.8.7 and 2.8.9): which alink frame, and which writes reach every unit. Pure:
//! the I/O is in `system::run`.
//!
//! - **The frame.** The newest frame any of the scene's units (or its sender)
//!   reports, plus the model's lead, rounded up to a superframe boundary. The
//!   units hear one counter, so their frames agree to within the reads' round
//!   trip, which the lead covers many times over.
//! - **The superframe path.** A scene of one op for every unit behind one
//!   sender goes to that sender as one `EXT_ALINK_CTRL_MSG`: no control-path
//!   latency, the same frame everywhere. Taken only when the sender is reachable
//!   and lists the OUT form, every unit of the model is behind it, and the op is
//!   not `UNMUTE`.
//! - **The per-unit path** otherwise: one `EXT_APPLY_AT_FRAME` per unit and op,
//!   at most four per unit (the model refuses more).
//! - **The system mute** (fixup sw2x F1). Every `EXT_ALINK_CTRL_MSG` write sets
//!   the sender's system-mute bit from its `system_flags`, op or not, so the
//!   superframe write carries the bit as it is ([`SenderSeen::system_mute`]),
//!   unless the scene itself says otherwise ([`super::model::Scene::system_mute`]).
//!   A scene that cannot read the bit does not take the superframe path.
//! - **The lead** (fixup sw2x F2) is the caller's: the model's, or more when the
//!   writes measured slower ([`plan`]).
//! - **Units without the audio link** (WP HWLK1F): a firmware without it
//!   takes no frame, so its ops are written at once with the unit's own
//!   commands ([`ScenePlan::direct`]) and the rest goes at the frame. Ops on
//!   units with the alink that no frame could be read for are listed
//!   ([`ScenePlan::unscheduled`]) instead of refusing the scene: it is refused
//!   only when no unit can apply any of it.
//! - **Why not `UNMUTE` through the sender.** BULKFIX's guard
//!   (`dspi_session::bulk_gain`) sits on each unit's own transport: an unmute
//!   sent per unit passes it (the suspect outputs' gains are re-sent first, or
//!   the op is refused), one broadcast through the sender would not.

use std::collections::{BTreeMap, BTreeSet};

use dspi_proto::ext::alink::{ApplyOp, ApplyRequest, SUPERFRAME_FRAMES, ctrl_msg_min_frame};
use serde::Serialize;

use super::model::{SystemModel, scene_op};

/// The sender a scene may go through: its serial, its frame counter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SenderSeen {
    /// The model's sender id.
    pub id: String,
    /// Its serial.
    pub serial: String,
    /// The frame it is sending.
    pub frame: u64,
    // -- fixup sw2x (F1) --
    /// Its system mute now (`EXT_ALINK_CTRL_MSG` IN flags bit 0).
    pub system_mute: bool,
}

/// One write of a plan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlannedOp {
    /// The device to write: a unit, or the sender on the superframe path.
    pub serial: String,
    /// The unit's id, or the sender's.
    pub element: String,
    /// The op's name.
    pub op: String,
    /// Its arg.
    pub arg: u8,
    /// Its value.
    pub value: i16,
}

impl PlannedOp {
    /// The request at `frame`.
    pub fn request(&self, frame: u64) -> Option<ApplyRequest> {
        Some(ApplyRequest {
            frame,
            op: scene_op(&self.op)?,
            arg: self.arg,
            value: self.value,
        })
    }
}

/// How a scene will be applied.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScenePlan {
    /// The scene.
    pub scene: String,
    /// The alink frame every op acts on.
    pub frame: u64,
    /// `superframe`, `per_unit`, or `none` (a scene that only sets the system
    /// mute).
    pub path: String,
    /// The writes, in order.
    pub ops: Vec<PlannedOp>,
    // -- fixup sw2x --
    /// The lead the frame was chosen with, frames.
    pub lead_frames: u64,
    /// What the scene does to the system mute; `None`: leaves it alone.
    pub system_mute: Option<bool>,
    /// The `system_flags` the superframe write carries.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system_flags: Option<u8>,
    // -- WP HWLK1F --
    /// Ops on units whose firmware has no audio link: written at once with
    /// the unit's own commands, not at [`Self::frame`].
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub direct: Vec<PlannedOp>,
    /// Ops on units with the alink that cannot be scheduled: no unit of
    /// theirs reports a frame and no sender is reachable. Not written.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub unscheduled: Vec<PlannedOp>,
    // -- WP AUSB1 --
    /// The bus frame the USB-direct units' ops act on (`EXT_APPLY_AT_FRAME`
    /// time base 2, sample 0 of that frame): the same instant on every unit of
    /// the host.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usb_frame: Option<u16>,
    /// The writes to phase-locked USB-direct units, at [`Self::usb_frame`].
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub usb_ops: Vec<PlannedOp>,
    /// The lead the bus frame was chosen with, samples.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usb_lead_samples: Option<u64>,
}

// -- WP AUSB1 --
/// A phase-locked USB-direct unit as read for a scene: its sample index S
/// now and its samples per bus frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UsbSeenNow {
    /// S at the read.
    pub sample: u64,
    /// Samples per bus frame (48 or 96).
    pub spf: u16,
}

/// The longest lead a bus-frame target takes: 2048 frames less the 64 that
/// count as past, and a margin of 64 for the reads (1.92 s).
pub const USB_MAX_LEAD_FRAMES: u64 = 2048 - 64 - 64;

// -- WP AUSB1 --
/// The bus frame `lead` samples after the newest S of `usb` (rounded up to a
/// frame boundary; the lead cut to [`USB_MAX_LEAD_FRAMES`]), and the lead used.
pub fn usb_target(usb: &[UsbSeenNow], lead: u64) -> Option<(u16, u64)> {
    let newest = usb.iter().max_by_key(|u| u.sample)?;
    let spf = u64::from(newest.spf);
    let lead = lead.min(USB_MAX_LEAD_FRAMES.saturating_mul(spf));
    let target = newest
        .sample
        .checked_add(lead)?
        .checked_next_multiple_of(spf)?;
    let (bus, _) = dspi_proto::ext::alink_usb::bus_of_sample(target, newest.spf)?;
    Some((bus, lead))
}

/// Why a scene cannot be planned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SceneRefused {
    /// No scene with that id.
    NoScene,
    /// No unit of the scene reports a frame (none locked or reachable).
    NoFrame,
}

/// Plan `scene_id`. `frames` holds each reachable unit's alink frame by serial;
/// `sender` the sender if it can take the superframe path; `lead` the frames
/// between the newest frame read and the frame the ops act on.
pub fn plan(
    model: &SystemModel,
    scene_id: &str,
    frames: &BTreeMap<String, u64>,
    sender: Option<&SenderSeen>,
    lead: u64,
) -> Result<ScenePlan, SceneRefused> {
    plan_mixed(model, scene_id, frames, sender, lead, &BTreeSet::new())
}

// -- WP HWLK1F --
/// [`plan`] for an installation where the units with these serials
/// (`no_alink`) have no audio link in their firmware: their ops go in
/// [`ScenePlan::direct`], the rest is planned as [`plan`] does. Refused with
/// [`SceneRefused::NoFrame`] only when there is nothing to write at once and
/// nothing can be scheduled.
pub fn plan_mixed(
    model: &SystemModel,
    scene_id: &str,
    frames: &BTreeMap<String, u64>,
    sender: Option<&SenderSeen>,
    lead: u64,
    no_alink: &BTreeSet<String>,
) -> Result<ScenePlan, SceneRefused> {
    let scene = model.scene(scene_id).ok_or(SceneRefused::NoScene)?;
    let mut ops = Vec::new();
    let mut direct = Vec::new();
    for a in &scene.actions {
        for u in model.resolve_target(&a.target).unwrap_or_default() {
            let op = PlannedOp {
                serial: u.serial.clone(),
                element: u.id.clone(),
                op: a.op.clone(),
                arg: a.arg,
                value: a.value,
            };
            if no_alink.contains(&u.serial) {
                direct.push(op);
            } else {
                ops.push(op);
            }
        }
    }
    let base = ops
        .iter()
        .filter_map(|o| frames.get(&o.serial))
        .copied()
        .max();
    let superframe = sender.filter(|s| {
        let [action] = scene.actions.as_slice() else {
            return false;
        };
        // -- WP AUSB1 -- every unit on the audio link: USB-direct ones have no path
        let every = model
            .alink_units()
            .all(|u| model.path_to(&u.id).is_some_and(|(from, _)| from == s.id));
        // -- WP HWLK1F -- every unit with the alink: the others take it at once
        let covers = model.resolve_target(&action.target).is_some_and(|t| {
            model
                .units
                .iter()
                .filter(|u| !no_alink.contains(&u.serial))
                .all(|u| t.iter().any(|x| x.id == u.id))
        });
        every && covers && scene_op(&action.op).is_some_and(|op| op != ApplyOp::Unmute)
    });
    if ops.is_empty() {
        return Ok(ScenePlan {
            scene: scene.id.clone(),
            frame: 0,
            path: if direct.is_empty() { "none" } else { "direct" }.into(),
            ops,
            lead_frames: 0,
            system_mute: scene.system_mute,
            system_flags: None,
            direct,
            unscheduled: Vec::new(),
            usb_frame: None,
            usb_ops: Vec::new(),
            usb_lead_samples: None,
        });
    }
    let round = |f: u64| f.checked_next_multiple_of(SUPERFRAME_FRAMES);
    if let Some(s) = superframe {
        let base = base.unwrap_or(s.frame).max(s.frame);
        let min = ctrl_msg_min_frame(s.frame).unwrap_or(u64::MAX);
        let frame = base
            .checked_add(lead)
            .and_then(round)
            .unwrap_or(u64::MAX)
            .max(min);
        let first = ops.first().cloned();
        let muted = scene.system_mute.unwrap_or(s.system_mute);
        return Ok(ScenePlan {
            scene: scene.id.clone(),
            frame,
            lead_frames: lead,
            system_mute: scene.system_mute,
            system_flags: Some(u8::from(muted)),
            path: "superframe".into(),
            ops: first
                .map(|o| PlannedOp {
                    serial: s.serial.clone(),
                    element: s.id.clone(),
                    ..o
                })
                .into_iter()
                .collect(),
            direct,
            unscheduled: Vec::new(),
            usb_frame: None,
            usb_ops: Vec::new(),
            usb_lead_samples: None,
        });
    }
    let frame = base.and_then(|b| b.checked_add(lead)).and_then(round);
    let Some(frame) = frame else {
        // -- WP HWLK1F -- what can be written at once still is
        if direct.is_empty() {
            return Err(SceneRefused::NoFrame);
        }
        return Ok(ScenePlan {
            scene: scene.id.clone(),
            frame: 0,
            path: "direct".into(),
            ops: Vec::new(),
            lead_frames: 0,
            system_mute: scene.system_mute,
            system_flags: None,
            direct,
            unscheduled: ops,
            usb_frame: None,
            usb_ops: Vec::new(),
            usb_lead_samples: None,
        });
    };
    Ok(ScenePlan {
        scene: scene.id.clone(),
        frame,
        path: "per_unit".into(),
        ops,
        lead_frames: lead,
        system_mute: scene.system_mute,
        system_flags: None,
        direct,
        unscheduled: Vec::new(),
        usb_frame: None,
        usb_ops: Vec::new(),
        usb_lead_samples: None,
    })
}

// -- WP AUSB1 --
/// [`plan_mixed`] for an installation with USB-direct units: those in `usb`
/// (phase-locked, read now) take their ops at one bus frame
/// ([`ScenePlan::usb_frame`], [`ScenePlan::usb_ops`]); the other USB-direct
/// units must be in `no_alink` and take theirs at once
/// ([`ScenePlan::direct`]). The audio link's units are planned as
/// [`plan_mixed`] does.
pub fn plan_usb(
    model: &SystemModel,
    scene_id: &str,
    frames: &BTreeMap<String, u64>,
    sender: Option<&SenderSeen>,
    lead: u64,
    no_alink: &BTreeSet<String>,
    usb: &BTreeMap<String, UsbSeenNow>,
) -> Result<ScenePlan, SceneRefused> {
    let mut aside = no_alink.clone();
    aside.extend(usb.keys().cloned());
    let mut plan = plan_mixed(model, scene_id, frames, sender, lead, &aside)?;
    let (usb_ops, direct): (Vec<_>, Vec<_>) = std::mem::take(&mut plan.direct)
        .into_iter()
        .partition(|o| usb.contains_key(&o.serial));
    plan.direct = direct;
    if usb_ops.is_empty() {
        return Ok(plan);
    }
    let seen: Vec<UsbSeenNow> = usb_ops
        .iter()
        .filter_map(|o| usb.get(&o.serial).copied())
        .collect();
    let (bus, lead) = usb_target(&seen, lead).ok_or(SceneRefused::NoFrame)?;
    plan.usb_frame = Some(bus);
    plan.usb_lead_samples = Some(lead);
    plan.usb_ops = usb_ops;
    if plan.ops.is_empty() && plan.direct.is_empty() {
        plan.path = "usb_frame".into();
    }
    Ok(plan)
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects, clippy::indexing_slicing)]
mod tests_wp_sw2 {
    use super::super::align::tests_wp_sw2::cinema;
    use super::super::model::*;
    use super::*;

    fn scene(actions: &[(&str, &str, u8, i16)]) -> Scene {
        Scene {
            id: "evening".into(),
            name: "Evening".into(),
            system_mute: None,
            actions: actions
                .iter()
                .map(|(t, op, arg, value)| SceneAction {
                    target: (*t).into(),
                    op: (*op).into(),
                    arg: *arg,
                    value: *value,
                })
                .collect(),
        }
    }

    fn frames() -> BTreeMap<String, u64> {
        BTreeMap::from([
            ("S-A".into(), 1_000_000),
            ("S-B".into(), 1_000_003),
            ("S-C".into(), 999_990),
        ])
    }

    fn tx(frame: u64) -> SenderSeen {
        SenderSeen {
            id: "tx".into(),
            serial: "S-tx".into(),
            frame,
            system_mute: false,
        }
    }

    #[test]
    fn wp_sw2_one_op_for_everyone_goes_through_the_sender() {
        let mut m = cinema();
        m.scenes = vec![scene(&[("all", "master_volume", 0, -2000)])];
        m.validate().unwrap();
        let p = plan(&m, "evening", &frames(), Some(&tx(1_000_010)), 2400).unwrap();
        assert_eq!(p.path, "superframe");
        assert_eq!(p.ops.len(), 1);
        assert_eq!(p.ops[0].serial, "S-tx");
        assert_eq!(p.frame % 32, 0);
        assert!(p.frame >= 1_000_010 + 2400);
        assert!(p.frame >= ctrl_msg_min_frame(1_000_010).unwrap());
    }

    #[test]
    fn wp_sw2_unmute_several_ops_or_a_subset_go_per_unit() {
        let mut m = cinema();
        m.scenes = vec![scene(&[("all", "unmute", 0, 0)])];
        let p = plan(&m, "evening", &frames(), Some(&tx(1_000_000)), 2400).unwrap();
        assert_eq!(p.path, "per_unit");
        assert_eq!(p.ops.len(), 4);
        // The newest unit frame plus the lead, on a superframe boundary.
        assert_eq!(p.frame, (1_000_003u64 + 2400).next_multiple_of(32));

        m.scenes = vec![scene(&[
            ("unit:A", "mute", 0, 0),
            ("room:cinema", "preset_load", 3, 0),
        ])];
        let p = plan(&m, "evening", &frames(), Some(&tx(1_000_000)), 2400).unwrap();
        assert_eq!(p.path, "per_unit");
        assert_eq!(p.ops.len(), 5);
        assert_eq!(p.ops[0].element, "A");
        assert_eq!(p.ops[1].request(7).unwrap().op, ApplyOp::PresetLoad);

        // No sender reachable: per unit too; nothing reports a frame: refused.
        m.scenes = vec![scene(&[("all", "mute", 0, 0)])];
        assert_eq!(
            plan(&m, "evening", &frames(), None, 2400).unwrap().path,
            "per_unit"
        );
        assert_eq!(
            plan(&m, "evening", &BTreeMap::new(), None, 2400),
            Err(SceneRefused::NoFrame)
        );
        assert_eq!(
            plan(&m, "nope", &frames(), None, 2400),
            Err(SceneRefused::NoScene)
        );
    }

    #[test]
    fn wp_sw2_a_scene_is_checked_with_the_model() {
        let mut m = cinema();
        m.scenes = vec![scene(&[("all", "rate", 1, 0)])];
        assert!(m.validate().is_err(), "RATE is the sender's own");
        m.scenes = vec![scene(&[("all", "master_volume", 0, 5)])];
        assert!(m.validate().is_err());
        m.scenes = vec![scene(&[("room:attic", "mute", 0, 0)])];
        assert!(m.validate().is_err());
        m.scenes = vec![scene(&[("unit:A", "mute", 0, 0); 5])];
        assert!(m.validate().unwrap_err().0.contains("queue"));
        m.scenes = vec![scene(&[("unit:A", "mute", 0, 0); 4])];
        m.validate().unwrap();
    }

    // -- WP HWLK1F --
    /// A unit without the audio link takes its ops at once; the others are
    /// planned as before, through the sender when every unit with the alink is
    /// covered; with no frame anywhere, what can go at once still does.
    #[test]
    fn wp_hwlk1f_units_without_the_link_go_at_once() {
        let mut m = cinema();
        m.scenes = vec![scene(&[("all", "master_volume", 0, -2000)])];
        let k = BTreeSet::from(["S-C".to_owned()]);
        let p = plan_mixed(&m, "evening", &frames(), Some(&tx(1_000_000)), 2400, &k).unwrap();
        assert_eq!(p.path, "superframe");
        assert_eq!(p.direct.len(), 1);
        assert_eq!(p.direct[0].element, "C");
        // Every unit with the alink, C left out of the target: still the sender.
        m.groups = vec![Group {
            id: "linked".into(),
            name: String::new(),
            units: vec!["A".into(), "B".into(), "R".into()],
            note: String::new(),
        }];
        m.scenes = vec![scene(&[("group:linked", "mute", 0, 0)])];
        let p = plan_mixed(&m, "evening", &frames(), Some(&tx(1_000_000)), 2400, &k).unwrap();
        assert_eq!((p.path.as_str(), p.direct.len()), ("superframe", 0));
        let p = plan(&m, "evening", &frames(), Some(&tx(1_000_000)), 2400).unwrap();
        assert_eq!(
            p.path, "per_unit",
            "without C known, the group is not everyone"
        );
        // Only C: at once. No frame anywhere: C at once, the rest unscheduled.
        m.scenes = vec![scene(&[("unit:C", "out_delay", 0, 48)])];
        let p = plan_mixed(&m, "evening", &BTreeMap::new(), None, 2400, &k).unwrap();
        assert_eq!(
            (p.path.as_str(), p.ops.len(), p.direct.len()),
            ("direct", 0, 1)
        );
        m.scenes = vec![scene(&[("all", "mute", 0, 0)])];
        let p = plan_mixed(&m, "evening", &BTreeMap::new(), None, 2400, &k).unwrap();
        assert_eq!(
            (p.path.as_str(), p.direct.len(), p.unscheduled.len()),
            ("direct", 1, 3)
        );
        assert_eq!(
            plan(&m, "evening", &BTreeMap::new(), None, 2400),
            Err(SceneRefused::NoFrame),
            "nothing at once and nothing scheduled: refused"
        );
    }

    // -- fixup sw2x (F1) --
    /// The superframe write carries the sender's mute as it is, unless the
    /// scene sets it; a scene of only a mute has no frame.
    #[test]
    fn wp_sw2x_the_superframe_write_keeps_the_mute() {
        let mut m = cinema();
        m.scenes = vec![scene(&[("all", "master_volume", 0, -2000)])];
        let muted = SenderSeen {
            system_mute: true,
            ..tx(1_000_000)
        };
        let p = plan(&m, "evening", &frames(), Some(&muted), 2400).unwrap();
        assert_eq!((p.system_flags, p.system_mute), (Some(1), None));
        let p = plan(&m, "evening", &frames(), Some(&tx(1_000_000)), 2400).unwrap();
        assert_eq!(p.system_flags, Some(0));
        m.scenes[0].system_mute = Some(false);
        let p = plan(&m, "evening", &frames(), Some(&muted), 2400).unwrap();
        assert_eq!((p.system_flags, p.system_mute), (Some(0), Some(false)));
        m.scenes[0].actions.clear();
        let p = plan(&m, "evening", &BTreeMap::new(), Some(&muted), 2400).unwrap();
        assert_eq!((p.path.as_str(), p.ops.len()), ("none", 0));
        // A lead given is a lead used.
        m.scenes = vec![scene(&[("unit:A", "mute", 0, 0)])];
        let p = plan(&m, "evening", &frames(), None, 9600).unwrap();
        assert_eq!(p.frame, (1_000_000u64 + 9600).next_multiple_of(32));
        assert_eq!(p.lead_frames, 9600);
    }
}
