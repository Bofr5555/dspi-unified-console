//! The hub family `system.measure.*` (PLAN-AT section 1.5, review R9e): the
//! request, reply and event bodies, one definition that WP ATHUB serves and
//! WP ATCORE sends. Seam: types, names and the lease clamp only.
//!
//! Device replies travel raw (`SiggenStatus`, `EXT_SIGGEN_TIMELINE`,
//! `EXT_TIME_BASE` entry, events), as the unit sent them: the client decodes
//! them with `dspi_proto`'s codecs, so the hub adds no second decoder.
//! Unknown fields are ignored (additive changes stay compatible).

use serde::{Deserialize, Serialize};

/// The hub's `hello.caps` entry for this family.
pub const CAP: &str = "system_measure";

/// Message types (`"t"`).
pub mod t {
    /// Open a session over a group of units.
    pub const OPEN: &str = "system.measure.open";
    /// Stage one unit's generator config.
    pub const STAGE: &str = "system.measure.stage";
    /// Schedule segments on the time base.
    pub const RUN: &str = "system.measure.run";
    /// Poll: renews the lease, returns every unit's state and a time pin.
    pub const STATUS: &str = "system.measure.status";
    /// Stop every unit now.
    pub const ABORT: &str = "system.measure.abort";
    /// End the session.
    pub const CLOSE: &str = "system.measure.close";
    /// Event: the hub aborted on its own.
    pub const ABORTED: &str = "system.measure.aborted";
}

/// Refusal codes beyond hub-api's common ones (`locked`, `installer_lock`,
/// `denied`, `offline`, `unsupported`, `bad_request`).
pub mod code {
    /// The units do not share one time base numbering (gate G1).
    pub const NOT_ONE_DOMAIN: &str = "not_one_domain";
    /// A unit's time base is not `LOCKED`.
    pub const NOT_LOCKED: &str = "not_locked";
    /// A unit does not list `EXT_SIGGEN_TIMELINE`.
    pub const NO_TIMELINE: &str = "no_timeline";
    /// A config the static rules refuse (infinite, `RAW`, over the cap, a
    /// source's time-base slot in the mask).
    pub const BAD_CONFIG: &str = "bad_config";
    /// A segment closer than the lead to the same unit's previous one, or a
    /// start nearer than the lead.
    pub const BAD_SCHEDULE: &str = "bad_schedule";
}

/// The shortest lease the hub accepts, ms.
pub const LEASE_MS_MIN: u32 = 500;
/// The longest lease the hub accepts, ms.
pub const LEASE_MS_MAX: u32 = 5_000;
/// The lease when the request names none, ms.
pub const LEASE_MS_DEFAULT: u32 = 2_000;

/// The lease the hub applies for a requested one: clamped to
/// [`LEASE_MS_MIN`]..=[`LEASE_MS_MAX`], [`LEASE_MS_DEFAULT`] when absent.
pub fn clamp_lease_ms(requested: Option<u32>) -> u32 {
    requested
        .unwrap_or(LEASE_MS_DEFAULT)
        .clamp(LEASE_MS_MIN, LEASE_MS_MAX)
}

/// `system.measure.open`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenRequest {
    /// The group's units by USB serial.
    pub units: Vec<String>,
    /// The lease asked for, ms (clamped by [`clamp_lease_ms`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lease_ms: Option<u32>,
}

/// One unit as the open read it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OpenUnit {
    /// Its serial.
    pub serial: String,
    /// The `EXT_TIME_BASE` entry the group uses (G1): its time base.
    pub time_base: u8,
    /// Its role: 0 follower, 1 source.
    pub role: u8,
    /// Its contract state.
    pub state: u8,
    /// Play-out D, samples (0 for a source).
    pub d: u16,
    /// `EXT_SIGGEN_TIMELINE.arm_lead_ms`.
    pub arm_lead_ms: u16,
    /// Outputs the unit has.
    pub outputs: u8,
    /// The entry's T at the read, in the unit's own numbering.
    pub t: u64,
    /// The unit's T minus the domain's T (USB origins; 0 on the line and S/PDIF).
    pub origin_offset: i64,
    /// `t - output_frames`, low 32 bits.
    pub t_minus_output: u32,
    /// `disturbances` at the read.
    pub disturbances: u16,
}

/// The reply to `system.measure.open`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OpenReply {
    /// The session id.
    pub session: String,
    /// The group's time base.
    pub time_base: u8,
    /// The group's domain id (C5).
    pub domain: u32,
    /// The lease the hub applies, ms.
    pub lease_ms: u32,
    /// The units, in the request's order.
    pub units: Vec<OpenUnit>,
}

/// `system.measure.stage`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StageRequest {
    /// The session.
    pub session: String,
    /// The unit.
    pub serial: String,
    /// The 36-byte `SiggenConfig`.
    pub config: Vec<u8>,
}

/// One segment of `system.measure.run`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunSegment {
    /// Index into the open's unit list.
    pub unit: u8,
    /// Domain samples after the run's `t0`.
    pub offset: u64,
}

/// `system.measure.run`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunRequest {
    /// The session.
    pub session: String,
    /// The segments in time order (a summed run: equal offsets, distinct units).
    pub segments: Vec<RunSegment>,
    /// The lead the client asks for, ms (the hub may use more).
    pub lead_ms: u32,
    // -- WP ATSUMOFF --
    /// The segments are one summed run whose parts may start apart (each of
    /// a distinct unit, in time order, at most
    /// [`super::MAX_SUM_SPREAD_MS`] after the first): the client brings
    /// the outputs' arrivals together. Absent (false): the rules above.
    #[serde(default, skip_serializing_if = "core::ops::Not::not")]
    pub summed: bool,
}

/// Where the hub put one segment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunSegmentReply {
    /// Index into the open's unit list.
    pub unit: u8,
    /// Its start in the domain's numbering.
    pub t: u64,
    /// Its start in the unit's own numbering (what its timeline will report).
    pub t_unit: u64,
}

/// The reply to `system.measure.run`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunReply {
    /// The run's origin in the domain's numbering.
    pub t0: u64,
    /// Every segment, in the request's order.
    pub segments: Vec<RunSegmentReply>,
}

/// `system.measure.status`, `system.measure.close`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionRequest {
    /// The session.
    pub session: String,
}

/// A device event the hub saw on a unit since the last status.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnitEvent {
    /// The notification's event id (0xE0 clip, 0xE1, 0xE3, 0xE4, 0x07
    /// generator, ...), or 0 for a hub-side observation.
    pub event: u8,
    /// The packet as received.
    pub raw: Vec<u8>,
}

/// One unit in a status reply.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UnitStatus {
    /// Index into the open's unit list.
    pub unit: u8,
    /// `SiggenStatus`, 16 bytes as read.
    pub siggen: Vec<u8>,
    /// `EXT_SIGGEN_TIMELINE`, 40 bytes as read.
    pub timeline: Vec<u8>,
    /// The group's `EXT_TIME_BASE` entry, 24 bytes as read.
    pub time_base_entry: Vec<u8>,
    /// `output_frames` of the same `EXT_TIME_BASE` read.
    pub output_frames: u32,
    /// Output peaks, dBFS, by output.
    pub output_peaks_dbfs: Vec<f32>,
    /// Any clip flag set since the last status.
    pub clipped: bool,
    /// Limiter gain reduction by output, dB.
    pub limiter_gr_db: Vec<f32>,
    /// Events since the last status.
    pub events: Vec<UnitEvent>,
}

/// The reply to `system.measure.status`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StatusReply {
    /// The domain T the hub read in this call (the time pin).
    pub t: u64,
    /// The hub's clock at that read, us.
    pub hub_us: u64,
    /// Every unit.
    pub units: Vec<UnitStatus>,
}

/// `system.measure.abort`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AbortRequest {
    /// The session.
    pub session: String,
    /// Why, in words (logged, shown to other sessions).
    pub reason: String,
}

/// What an abort did on one unit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AbortUnit {
    /// Index into the open's unit list.
    pub unit: u8,
    /// `STOP_NOW` and `CANCEL_ALL` were accepted.
    pub stopped: bool,
    /// Why not, when not.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// The reply to `system.measure.abort`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AbortReply {
    /// Every unit.
    pub units: Vec<AbortUnit>,
}

/// Event `system.measure.aborted`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AbortedEvent {
    /// The session.
    pub session: String,
    /// The unit that caused it, if one did.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unit: Option<u8>,
    /// Why: `lease`, `disturbance`, `clip`, `foreign`, `offline`, `closed`,
    /// `abort`.
    pub reason: String,
}

// -- WP ATGUARD --
/// The hub's guard of a system apply (`system.guard.*`, hub-api section
/// 23.4): the hub files the model and its revision, and sets it back and
/// aligns at a deadline unless the owner confirms; a lost connection or a
/// stopping hub reverts too. One guard at a time; while it is pending, a
/// `system.set` from another session is refused `conflict`.
pub mod guard {
    use serde::{Deserialize, Serialize};

    /// The hub's `hello.caps` entry: it answers `system.guard.*`.
    pub const CAP: &str = "system_apply_guard";
    /// File the model and start the countdown (Admin).
    pub const BEGIN: &str = "system.guard.begin";
    /// Keep what was applied (the owner).
    pub const CONFIRM: &str = "system.guard.confirm";
    /// Revert now (the owner): the filed model set again, then aligned.
    pub const CANCEL: &str = "system.guard.cancel";
    /// The pending guard and the last outcome (any role).
    pub const STATUS: &str = "system.guard.status";
    /// Event to every session: a guard ended.
    pub const ENDED: &str = "system.guard.ended";
    /// The shortest countdown the hub grants, ms.
    pub const TIMEOUT_MS_MIN: u32 = 1_000;
    /// The longest countdown the hub grants, ms (as `apply.begin`'s).
    pub const TIMEOUT_MS_MAX: u32 = 600_000;
    /// The countdown when `begin` names none, ms.
    pub const TIMEOUT_MS_DEFAULT: u32 = 300_000;

    /// The countdown the hub grants for a requested one.
    pub fn clamp_timeout_ms(requested: Option<u32>) -> u32 {
        requested
            .unwrap_or(TIMEOUT_MS_DEFAULT)
            .clamp(TIMEOUT_MS_MIN, TIMEOUT_MS_MAX)
    }

    /// `system.guard.begin`.
    #[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
    pub struct BeginRequest {
        /// The countdown, ms ([`clamp_timeout_ms`]).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub timeout_ms: Option<u32>,
    }

    /// `begin`'s answer.
    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
    pub struct BeginReply {
        /// The guard's id.
        pub guard: u64,
        /// The model's revision filed.
        pub revision: u64,
        /// The countdown granted, ms.
        pub timeout_ms: u32,
    }

    /// `confirm` and `cancel`: the guard they end.
    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
    pub struct GuardRequest {
        /// The guard's id, from `begin`.
        pub guard: u64,
    }

    /// How a guard ended: `confirm`'s and `cancel`'s answer, the event
    /// `system.guard.ended`, `status`'s `last`.
    #[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
    pub struct Outcome {
        /// The guard's id.
        pub guard: u64,
        /// `confirmed`, `timeout`, `session_closed`, `cancelled`,
        /// `hub_stopping`.
        pub reason: String,
        /// `kept`, `reverted`, `revert_failed` (the model was set back but
        /// a unit's write failed), `not_written` (the model changed on the
        /// hub meanwhile, or could not be saved: nothing was set back).
        pub result: String,
        /// The model's revision filed at `begin`.
        pub revision_filed: u64,
        /// The model's revision now (after a revert, the revert's).
        pub revision: u64,
        /// The alignment's writes that were `ok` or `unchanged`.
        #[serde(default)]
        pub writes_ok: u32,
        /// The others, in words.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        pub write_failures: Vec<String>,
        /// What to know, in words.
        #[serde(default, skip_serializing_if = "String::is_empty")]
        pub message: String,
        /// Unix seconds.
        #[serde(default)]
        pub at: u64,
    }

    /// The pending guard in `status`.
    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
    pub struct Active {
        /// The guard's id.
        pub guard: u64,
        /// The session that began it.
        pub session: u64,
        /// The revision filed.
        pub revision_filed: u64,
        /// The revision its owner's last `system.set` left.
        pub revision_left: u64,
        /// Until the hub reverts, ms.
        pub remaining_ms: u64,
        /// It is being reverted.
        pub reverting: bool,
    }

    /// `status`'s answer.
    #[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
    pub struct StatusReply {
        /// The pending guard, if any.
        #[serde(default)]
        pub active: Option<Active>,
        /// The last guard's outcome, if any.
        #[serde(default)]
        pub last: Option<Outcome>,
    }
}

#[cfg(test)]
mod tests_wp_planat {
    use super::*;

    #[test]
    fn wp_planat_lease_is_clamped() {
        assert_eq!(clamp_lease_ms(None), LEASE_MS_DEFAULT);
        assert_eq!(clamp_lease_ms(Some(0)), LEASE_MS_MIN);
        assert_eq!(clamp_lease_ms(Some(60_000)), LEASE_MS_MAX);
        assert_eq!(clamp_lease_ms(Some(1_234)), 1_234);
    }

    #[test]
    fn wp_planat_messages_round_trip_and_ignore_unknown_fields() {
        let r = RunRequest {
            session: "s1".into(),
            segments: vec![
                RunSegment { unit: 0, offset: 0 },
                RunSegment {
                    unit: 1,
                    offset: 48_000,
                },
            ],
            lead_ms: 250,
            summed: false,
        };
        let j = serde_json::to_string(&r).unwrap_or_default();
        let back: Result<RunRequest, _> = serde_json::from_str(&j);
        assert_eq!(back.ok(), Some(r));
        let extra: Result<OpenRequest, _> = serde_json::from_str(r#"{"units":["A"],"future":1}"#);
        assert_eq!(extra.ok().map(|o| o.units), Some(vec!["A".to_string()]));
    }

    // -- WP ATSUMOFF --
    #[test]
    fn wp_atsumoff_summed_is_absent_unless_set_and_round_trips() {
        let mut r = RunRequest {
            session: "s1".into(),
            segments: vec![
                RunSegment { unit: 0, offset: 0 },
                RunSegment {
                    unit: 1,
                    offset: 960,
                },
            ],
            lead_ms: 250,
            summed: false,
        };
        let j = serde_json::to_string(&r).unwrap_or_default();
        assert!(!j.contains("summed"), "{j}");
        r.summed = true;
        let j = serde_json::to_string(&r).unwrap_or_default();
        assert!(j.contains("\"summed\":true"), "{j}");
        let back: Result<RunRequest, _> = serde_json::from_str(&j);
        assert_eq!(back.ok(), Some(r));
        let old: Result<RunRequest, _> =
            serde_json::from_str(r#"{"session":"s","segments":[],"lead_ms":1}"#);
        assert_eq!(old.ok().map(|o| o.summed), Some(false));
    }

    // -- WP ATGUARD --
    #[test]
    fn wp_atguard_timeout_is_clamped_and_bodies_round_trip() {
        assert_eq!(guard::clamp_timeout_ms(None), guard::TIMEOUT_MS_DEFAULT);
        assert_eq!(guard::clamp_timeout_ms(Some(0)), guard::TIMEOUT_MS_MIN);
        assert_eq!(
            guard::clamp_timeout_ms(Some(u32::MAX)),
            guard::TIMEOUT_MS_MAX
        );
        let j = serde_json::to_string(&guard::BeginRequest::default()).unwrap_or_default();
        assert_eq!(j, "{}");
        let o: Result<guard::Outcome, _> = serde_json::from_str(
            r#"{"guard":3,"reason":"timeout","result":"reverted","revision_filed":4,"revision":6,"future":1}"#,
        );
        let o = o.ok();
        assert_eq!(o.as_ref().map(|o| o.writes_ok), Some(0));
        assert_eq!(o.map(|o| o.revision), Some(6));
        let s: Result<guard::StatusReply, _> =
            serde_json::from_str(r#"{"active":null,"last":null}"#);
        assert_eq!(s.ok(), Some(guard::StatusReply::default()));
    }
}
