//! Scenes applied at a frame (WP SW2, SYS2 3.4, `extension_channel_spec.md`
//! 2.8.7 and 2.8.9): which link frame, and which writes reach every unit. Pure:
//! the I/O is in `system::run`.
//!
//! - **The frame.** The newest frame any of the scene's units (or its sender)
//!   reports, plus the model's lead, rounded up to a superframe boundary. The
//!   units hear one counter, so their frames agree to within the reads' round
//!   trip, which the lead covers many times over.
//! - **The superframe path.** A scene of one op for every unit behind one
//!   sender goes to that sender as one `EXT_LINK_CTRL_MSG`: no control-path
//!   latency, the same frame everywhere. Taken only when the sender is reachable
//!   and lists the OUT form, every unit of the model is behind it, and the op is
//!   not `UNMUTE`.
//! - **The per-unit path** otherwise: one `EXT_APPLY_AT_FRAME` per unit and op,
//!   at most four per unit (the model refuses more).
//! - **The system mute** (fixup sw2x F1). Every `EXT_LINK_CTRL_MSG` write sets
//!   the sender's system-mute bit from its `system_flags`, op or not, so the
//!   superframe write carries the bit as it is ([`SenderSeen::system_mute`]),
//!   unless the scene itself says otherwise ([`super::model::Scene::system_mute`]).
//!   A scene that cannot read the bit does not take the superframe path.
//! - **The lead** (fixup sw2x F2) is the caller's: the model's, or more when the
//!   writes measured slower ([`plan`]).
//! - **Why not `UNMUTE` through the sender.** BULKFIX's guard
//!   (`dspi_session::bulk_gain`) sits on each unit's own transport: an unmute
//!   sent per unit passes it (the suspect outputs' gains are re-sent first, or
//!   the op is refused), one broadcast through the sender would not.

use std::collections::BTreeMap;

use dspi_proto::ext::audio_link::{ApplyOp, ApplyRequest, SUPERFRAME_FRAMES, ctrl_msg_min_frame};
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
    /// Its system mute now (`EXT_LINK_CTRL_MSG` IN flags bit 0).
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
    /// The link frame every op acts on.
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
}

/// Why a scene cannot be planned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SceneRefused {
    /// No scene with that id.
    NoScene,
    /// No unit of the scene reports a frame (none locked or reachable).
    NoFrame,
}

/// Plan `scene_id`. `frames` holds each reachable unit's link frame by serial;
/// `sender` the sender if it can take the superframe path; `lead` the frames
/// between the newest frame read and the frame the ops act on.
pub fn plan(
    model: &SystemModel,
    scene_id: &str,
    frames: &BTreeMap<String, u64>,
    sender: Option<&SenderSeen>,
    lead: u64,
) -> Result<ScenePlan, SceneRefused> {
    let scene = model.scene(scene_id).ok_or(SceneRefused::NoScene)?;
    let mut ops = Vec::new();
    for a in &scene.actions {
        for u in model.resolve_target(&a.target).unwrap_or_default() {
            ops.push(PlannedOp {
                serial: u.serial.clone(),
                element: u.id.clone(),
                op: a.op.clone(),
                arg: a.arg,
                value: a.value,
            });
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
        let every = model
            .units
            .iter()
            .all(|u| model.path_to(&u.id).is_some_and(|(from, _)| from == s.id));
        let covers = model
            .resolve_target(&action.target)
            .is_some_and(|t| t.len() == model.units.len());
        every && covers && scene_op(&action.op).is_some_and(|op| op != ApplyOp::Unmute)
    });
    if ops.is_empty() {
        return Ok(ScenePlan {
            scene: scene.id.clone(),
            frame: 0,
            path: "none".into(),
            ops,
            lead_frames: 0,
            system_mute: scene.system_mute,
            system_flags: None,
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
        });
    }
    let base = base.ok_or(SceneRefused::NoFrame)?;
    let frame = base
        .checked_add(lead)
        .and_then(round)
        .ok_or(SceneRefused::NoFrame)?;
    Ok(ScenePlan {
        scene: scene.id.clone(),
        frame,
        path: "per_unit".into(),
        ops,
        lead_frames: lead,
        system_mute: scene.system_mute,
        system_flags: None,
    })
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
