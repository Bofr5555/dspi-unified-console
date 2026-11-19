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

// -- WP ATDEMO --
/// The demo hub's simulated microphone (WP ATDEMO): a room that hears the
/// demo units' generators, offered only by `uconsole --demo hub --alink
/// --with-measure-units` and only for a session whose every unit is one of
/// that hub's own demo units. Never on a hub with real devices in the
/// session: the hub refuses it (`unsupported`) and the core then plays
/// nothing.
pub mod demo {
    use serde::{Deserialize, Serialize};

    /// The hub's `hello.caps` entry: it has a demo room.
    pub const CAP: &str = "system_measure_demo_mic";
    /// `system.measure.demo_mic`: what the room's microphone heard.
    pub const T: &str = "system.measure.demo_mic";
    /// The most frames one answer carries (it fits one Link frame).
    pub const MAX_FRAMES: u32 = 4_096;
    /// The samples' full scale: 24-bit integers, as a microphone's.
    pub const FULL_SCALE: f32 = 8_388_608.0;

    /// `system.measure.demo_mic`: either `units` (may the room hear these
    /// units? nothing is read) or a `session` and the capture frame to read
    /// from.
    #[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
    pub struct DemoMicRequest {
        /// The session whose capture is read ("" with `units`).
        #[serde(default)]
        pub session: String,
        /// The units to ask about, without a session.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        pub units: Vec<String>,
        /// The first capture frame wanted (the session's frames from 0).
        #[serde(default)]
        pub from: u64,
    }

    /// The answer: the capture from `first_frame` (empty when nothing more
    /// is heard yet), at `rate_hz`, as 24-bit integers ([`FULL_SCALE`]).
    #[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
    pub struct DemoMicReply {
        /// The room may hear the units (always true in an answer).
        pub ok: bool,
        /// What the room is, in words.
        #[serde(default)]
        pub room: String,
        /// The first frame of `samples`.
        #[serde(default)]
        pub first_frame: u64,
        /// The capture's rate, Hz.
        #[serde(default)]
        pub rate_hz: u32,
        /// The samples.
        #[serde(default)]
        pub samples: Vec<i32>,
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
}
