//! The alignment engine (WP SW2, SYS2 3.1-3.2 and 3.5, `alink_protocol_spec.md`
//! 7.2): from the model and what the devices report, each unit's
//! `EXT_ALINK_PHASE` (`d_local`, `hops_offset`) and its outputs' `out.delay`, so
//! that every unit plays alink frame F at the same instant and every speaker in
//! a room is heard at once at the listening position. Pure: the I/O is in
//! `system::run`.
//!
//! In frames at the stream's rate, for a unit `u` behind re-emitters with hop
//! delays `d_1..d_h` (`h` = its hops), with latency after the alink `L_u` and its
//! room's lip-sync offset `S_u`:
//!
//! - frame F reaches `u` at `t_F + sum(d_k)`, so its effective delay
//!   `E_u = D - sum(d_k) + (L_max - L_u) + S_u` makes it heard at
//!   `t_F + D + L_max + S_u`, the same for every unit of a room;
//! - `d_local = D - h` (one frame per hop, 2.8.5) and `hops_offset = E_u -
//!   d_local`, which is `-sum(d_k - 1)` plus the latency and lip-sync terms;
//!   past the firmware's -64..64 the excess moves into `d_local`, since the
//!   firmware uses only the sum;
//! - D, unless configured, is the smallest that puts every unit's `E_u` at or
//!   above its `d_local_min`; a unit whose `E_u` is outside its
//!   `d_local_min..=d_local_max` is reported and not written;
//! - `out.delay` of a speaker is `(r_max - r) / c` with `r_max` the furthest
//!   speaker of its room (of its unit, without a room).

use std::collections::BTreeMap;

use dspi_proto::ext::alink::HOPS_OFFSET_MAX;
use serde::Serialize;

use super::model::{SystemModel, Unit};

/// The speed of sound the distance delays use, m/s (20 °C).
pub const SPEED_OF_SOUND: f64 = 343.0;
/// D when nothing reports a limit (SYS2 3.2's default for the 1 ms option).
pub const DEFAULT_D: u16 = 128;

// -- fixup sw2x (F3) --
/// The longest output delay a unit plays, samples: the firmware caps
/// `MAX_DELAY_SAMPLES - 1` (2047 on an RP2350, `dsp_pipeline.c`), and the PDM
/// sub's alignment takes from it, so the hub keeps one sample under. The unit
/// stores the host's milliseconds and caps only the samples, so a read-back
/// cannot reveal the cap: the hub must.
pub const MAX_OUT_DELAY_SAMPLES: u32 = 2046;

/// What the hub read from one device, by serial. Every field is optional: a
/// device offline, without the alink, or not reporting it.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Observed {
    /// Attached to the hub now.
    pub online: bool,
    /// `EXT_ALINK_INFO.role`'s name.
    pub role: Option<String>,
    /// `EXT_ALINK_INFO.d_local_min`.
    pub d_local_min: Option<u16>,
    /// `EXT_ALINK_INFO.d_local_max`.
    pub d_local_max: Option<u16>,
    /// `EXT_ALINK_STATUS.state`'s name.
    pub state: Option<String>,
    /// `EXT_ALINK_STATUS.hops`.
    pub hops: Option<u8>,
    /// `EXT_ALINK_STATUS.hop_delay_frames` (offset 50).
    pub hop_delay_frames: Option<u8>,
    /// `EXT_ALINK_STATUS.sender_id`.
    pub sender_id: Option<u8>,
    /// `EXT_ALINK_STATUS.rate_code`.
    pub rate_code: Option<u8>,
    /// `EXT_ALINK_STATUS.crc16_errors`.
    pub crc16_errors: Option<u32>,
    /// `EXT_ALINK_STATUS.loss_count`.
    pub loss_count: Option<u32>,
    /// `EXT_ALINK_STATUS.phase_error_frames`.
    pub phase_error_frames: Option<f32>,
    /// The alink frame (`EXT_FRAME_COUNTER`, else `EXT_ALINK_STATUS`).
    pub frame: Option<u64>,
    /// `EXT_ALINK_PHASE` as read: mode, d_local, hops_offset, flags.
    pub phase: Option<(u8, u16, i16, u8)>,
    /// The listed sub-commands include `EXT_ALINK_CTRL_MSG` OUT (a sender build).
    pub superframe_path: bool,
    /// The device announces 0xE1 (`EXT_NOTIFY_FEATURES` bit 2).
    pub state_events: bool,
    // -- fixup sw2x (F1) --
    /// A sender's system mute (`EXT_ALINK_CTRL_MSG` IN flags bit 0).
    pub system_mute: Option<bool>,
    // -- WP SW3 -- for the clients' "Clock and audio links" panel
    /// The rest of `EXT_ALINK_STATUS`'s counters and loop terms.
    pub stats: Option<AlinkStats>,
    /// A receiver's trim scan at alink lock (`EXT_ALINK_DIAG` version 1).
    pub trim_scan: Option<TrimScan>,
    // -- fixup sw3x --
    /// A sender's superframe apply still waiting for its frame: it takes no
    /// other until that frame has passed (2.8.9).
    pub pending_apply_frame: Option<u64>,
    // -- WP HWLK1F --
    /// Whether the device's firmware has the audio link at all: `present`
    /// (`EXT_ALINK_INFO` answers), `none` (its `EXT_LIST` names no 0x8000 block,
    /// or the block stalls: a K340 on bofr-main 470a127, stock firmware), or
    /// `None` when that could not be told (offline, no answer).
    pub alink: Option<AlinkFirmware>,
    // -- WP AUSB1 --
    /// For a unit the model has as USB-direct: whether its firmware has the
    /// USB-direct block (`EXT_LIST` names 0x8010), as [`Self::alink`].
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usb_direct: Option<AlinkFirmware>,
    /// The USB-direct block as read.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usb: Option<UsbSeen>,
}

// -- WP AUSB1 --
/// A USB-direct unit's `EXT_ALINK_USB`, `EXT_ALINK_USB_STATUS` and
/// `EXT_ALINK_USB_DIAG` as the hub read them (`extension_channel_spec.md` 2.9).
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UsbSeen {
    /// `off`, `rate_lock` or `phase_lock`: the mode written.
    pub mode: String,
    /// D, samples.
    pub d: u16,
    /// A phase error beyond 2 samples re-anchors.
    pub auto_reanchor: bool,
    /// Why mode 2 runs as mode 1, if it does: `d_below_min`, `d_above_max`,
    /// `rate_unsupported`.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub config_flags: Vec<String>,
    /// `off`, `no_sof`, `pull_in`, `rate_locked`, `anchoring`,
    /// `phase_locked`, `rejected`.
    pub state: String,
    /// 0 free, 1 anchoring, 2 locked, 3 re-anchor pending.
    pub phase_state: u8,
    /// 0 = 44.1, 1 = 48, 2 = 96 kHz.
    pub rate_code: u8,
    /// The host's SOF rate against the unit's crystal, ppm.
    pub host_ppm: f32,
    /// The soft VCXO's correction, ppm.
    pub vcxo_ppm: f32,
    /// The phase error, samples.
    pub phase_error: f32,
    /// The bus frame at the read.
    pub bus_frame: u16,
    /// The USB sample index S at the read.
    pub sample: u64,
    /// The D range phase lock takes now.
    pub d_min: u16,
    /// Its top.
    pub d_max: u16,
    /// The output ring's fill, samples.
    pub fill: u16,
    /// A stream plays.
    pub stream: bool,
    /// No SOF for 50 ms: holding the timeline.
    pub holdover: bool,
    /// The bus frame the stream's content began in (2.9.3); none without one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream_start_bus_frame: Option<u16>,
    /// Silence inserted minus late samples dropped since then.
    pub stream_shift: i32,
    /// Faded gaps since boot.
    pub gap_count: u16,
    /// Anchors since boot.
    pub anchors: u32,
    /// Re-anchors since boot.
    pub reanchors: u32,
    /// Silence inserted for missing packets, samples since boot.
    pub missed_samples: u32,
    /// Samples dropped as too late, since boot.
    pub late_samples: u32,
    /// `EXT_ALINK_USB_DIAG`, when it reads (layout version 1 or 2).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diag: Option<UsbDiagSeen>,
    // -- WP AUSB2 -- lock error (the fork's WP USB3b, 2.9.2)
    /// What the unit does in lock error: `mute` or `play_unaligned`
    /// (`EXT_ALINK_USB` byte 4; `mute` on firmware before WP USB3b).
    pub on_lock_error: String,
    /// In lock error now (STATUS flags bit 6): mode 2 that cannot be in step.
    pub lock_error: bool,
    /// Why, while it is: `host_rejected`, `config` or `pending`
    /// ([`lock_error_cause`]).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lock_error_cause: Option<String>,
    /// The unit refused "play unaligned" when the hub wrote it (firmware
    /// before WP USB3b, where byte 4 is reserved): it mutes in lock error.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub play_unaligned_unsupported: bool,
}

// -- WP AUSB2 --
/// Why a unit is in lock error (2.9.2), from its state: the host rejected
/// (state 6), a phase error pending (phase state 3), else a configuration
/// that cannot phase-lock (44.1 kHz, D outside its limits).
pub fn lock_error_cause(state: &str, phase_state: u8) -> &'static str {
    if state == "rejected" {
        "host_rejected"
    } else if phase_state == 3 {
        "pending"
    } else {
        "config"
    }
}

// -- WP AUSB2 --
/// [`lock_error_cause`]'s words.
pub fn lock_error_words(cause: &str) -> &'static str {
    match cause {
        "host_rejected" => "the host's rate is out of range",
        "pending" => "a phase error is still being pulled in",
        "config" => "its rate or D cannot phase-lock",
        _ => "cause unknown",
    }
}

// -- WP AUSB2 --
/// The alarm's and the issue's sentence for unit `name` in lock error, doing
/// `on_lock_error` (`mute` or `play_unaligned`), for `cause`.
pub fn lock_error_message(name: &str, on_lock_error: &str, cause: Option<&str>) -> String {
    let doing = if on_lock_error == "play_unaligned" {
        "playing unaligned"
    } else {
        "muted"
    };
    format!(
        "USB-direct unit {name} can't lock \u{2014} {doing} ({})",
        lock_error_words(cause.unwrap_or(""))
    )
}

impl UsbSeen {
    /// Phase-locked: scenes go at a bus frame, and the start offsets count.
    pub fn phase_locked(&self) -> bool {
        self.state == "phase_locked" && matches!(self.phase_state, 2 | 3)
    }
}

// -- WP AUSB1 --
/// `EXT_ALINK_USB_DIAG`'s counters as the hub shows them.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UsbDiagSeen {
    /// The latest timeline end's cause; none yet.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_cause: Option<String>,
    /// Every timeline end since boot or a clear.
    pub ends: u32,
    /// SOF absences of 50 ms or more.
    pub holdovers: u16,
    /// Those ridden through.
    pub rides: u16,
    /// USB suspends.
    pub suspends: u16,
    /// USB bus resets.
    pub bus_resets: u16,
    /// Disconnects.
    pub unplugs: u16,
    /// The longest absence ridden through, µs.
    pub longest_ride_us: u32,
    // -- WP AUSB2 -- layout 2 (the fork's WP USB3b); 0 from layout 1
    /// The longest absence ridden through, ms.
    pub longest_ride_ms: u32,
    /// Entries into lock error.
    pub lock_errors: u16,
    /// RP2350 SOF timestamps the unit repaired.
    pub bad_sof_readings: u16,
    /// The layout read (1 or 2).
    pub version: u8,
}

// -- WP HWLK1F --
/// Whether a device's firmware has the audio link ([`Observed::alink`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AlinkFirmware {
    /// `EXT_ALINK_INFO` answers.
    Present,
    /// No audio link in this firmware: never locked, never at a frame.
    #[serde(rename = "none")]
    Absent,
}

impl Observed {
    // -- WP HWLK1F --
    /// The firmware is known to have no audio link.
    pub fn without_alink(&self) -> bool {
        self.alink == Some(AlinkFirmware::Absent)
    }

    // -- WP AUSB1 --
    /// The firmware is known to have no USB-direct block.
    pub fn without_usb_direct(&self) -> bool {
        self.usb_direct == Some(AlinkFirmware::Absent)
    }
}

// -- WP SW3 --
/// `EXT_ALINK_STATUS`'s counters (from boot or the last stats reset, wrapping
/// at 2^32) and loop terms, as the "Clock and audio links" panel shows them.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AlinkStats {
    /// The line class, 1-3 (L1-L3); 0 unknown.
    pub line_class: u8,
    /// Audio link channels in the stream; 0 unknown.
    pub channels: u8,
    /// Entries into locked.
    pub lock_count: u32,
    /// Bad frame heads.
    pub crc8_errors: u32,
    /// Commas found 1-2 bits away.
    pub bit_slips: u32,
    /// Rejected superframes.
    pub superframe_errors: u32,
    /// The soft VCXO's correction, ppm.
    pub vcxo_ppm: f32,
    /// The output ring's fill, frames.
    pub fill_frames: u16,
}

// -- WP SW3 --
/// The trim scan at alink lock (`dspi_proto::ext::alink::TrimScan`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrimScan {
    /// `none`, `running`, `done` or `no_clean_trim`.
    pub state: String,
    /// Lowest clean trim, -9..+9 (3.26 ns per step).
    pub trim_lo: i8,
    /// Highest clean trim.
    pub trim_hi: i8,
    /// The trim in use.
    pub trim_used: i8,
    /// Bit i set = trim i - 9 was clean.
    pub clean_mask: u32,
}

/// One unit's computed settings.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UnitPlan {
    /// The unit's id.
    pub unit: String,
    /// Its serial.
    pub serial: String,
    /// Re-emitters between the sender and the unit, from the model.
    pub hops: u8,
    /// `sum(d_k)`: frames the re-emitters add.
    pub path_delay_frames: u32,
    /// Its latency after the alink, frames.
    pub latency_frames: i64,
    /// Its room's lip-sync offset, frames.
    pub lip_sync_frames: i64,
    /// `E_u`.
    pub effective: i64,
    /// What to write: phase lock, this `d_local`, this `hops_offset`.
    pub d_local: u16,
    /// What to write.
    pub hops_offset: i16,
    /// Distance delays: (output, ms).
    pub out_delays: Vec<(u8, f32)>,
    // -- fixup sw2x (F3) --
    /// Outputs whose distance delay was longer than the unit holds
    /// ([`MAX_OUT_DELAY_SAMPLES`]) and was cut to it: that speaker is not
    /// aligned (issue `delay_over_max`).
    pub clamped_outputs: Vec<u8>,
    /// Whether the plan may be written (in range, reachable).
    pub writable: bool,
    // -- WP AUSB1 --
    /// `alink` or `usb_direct`. A USB-direct unit's delay is `usb_d`, not
    /// `d_local` and `hops_offset` (0 for it).
    pub transport: String,
    /// A USB-direct unit's D to write, samples (`EXT_ALINK_USB.d`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usb_d: Option<u16>,
    /// The mode to write with it: the model's, else the unit's.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usb_mode: Option<String>,
    /// How many samples earlier its stream's content plays than the latest
    /// USB-direct unit's (2.9.3, U6): added to its outputs' delays.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_offset_samples: Option<i64>,
}

/// A finding: something the installer should look at.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Issue {
    /// The element it is about, if one.
    pub subject: Option<String>,
    /// A stable code: `limits_unknown`, `no_alink` (WP HWLK1F), `delay_out_of_range`,
    /// `hop_delay_assumed`, `hops_mismatch`, `sender_mismatch`, `offline`,
    /// `d_too_small`; USB-direct (WP AUSB1, FIXM1): `mixed_time_bases`,
    /// `usb_no_stream`, `usb_start_offsets`, `start_offset_no_outputs`,
    /// `usb_rate_differs`, `usb_start_offsets_changed`; WP AUSB2:
    /// `usb_lock_error`, `usb_lock_error_unsupported`.
    pub code: String,
    /// For people.
    pub message: String,
}

/// The engine's answer.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Alignment {
    /// The system delay D, frames.
    pub d_frames: u16,
    /// Whether D was picked by the engine.
    pub d_auto: bool,
    /// The rate the µs figures were converted at.
    pub rate_hz: u32,
    /// `L_max`, frames.
    pub latency_max_frames: i64,
    /// Per unit, in model order.
    pub units: Vec<UnitPlan>,
    /// Findings.
    pub issues: Vec<Issue>,
}

fn issue(subject: Option<&str>, code: &str, message: String) -> Issue {
    Issue {
        subject: subject.map(str::to_owned),
        code: code.into(),
        message,
    }
}

/// µs to frames at `rate`, rounded to the nearest.
pub fn us_to_frames(us: i64, rate: u32) -> i64 {
    let n = i128::from(us).saturating_mul(i128::from(rate));
    // Round half away from zero, in integers.
    let half = if n < 0 { -500_000 } else { 500_000 };
    i64::try_from(n.saturating_add(half) / 1_000_000).unwrap_or(0)
}

/// The stream's rate from a `rate_code`.
pub fn rate_of(code: u8) -> Option<u32> {
    match code {
        0 => Some(44_100),
        1 => Some(48_000),
        2 => Some(96_000),
        _ => None,
    }
}

/// The rate the units agree on, else the model's.
fn stream_rate(model: &SystemModel, obs: &BTreeMap<String, Observed>) -> u32 {
    let mut counts: BTreeMap<u32, usize> = BTreeMap::new();
    for u in &model.units {
        if let Some(r) = obs
            .get(&u.serial)
            .and_then(|o| o.rate_code.or_else(|| o.usb.as_ref().map(|s| s.rate_code)))
            .and_then(rate_of)
        {
            let n = counts.entry(r).or_default();
            *n = n.saturating_add(1);
        }
    }
    counts
        .into_iter()
        .max_by_key(|(_, n)| *n)
        .map_or(model.rate_hz, |(r, _)| r)
}

/// A re-emitter's hop delay: read, else configured, else 1 (with an issue).
fn hop_delay(
    model: &SystemModel,
    obs: &BTreeMap<String, Observed>,
    id: &str,
    issues: &mut Vec<Issue>,
) -> u32 {
    let (serial, configured) = match model.fan_outs.iter().find(|f| f.id == id) {
        Some(f) => (f.serial.as_deref(), f.hop_delay_frames),
        None => match model.unit(id) {
            Some(u) => (Some(u.serial.as_str()), u.hop_delay_frames),
            None => (None, None),
        },
    };
    let read = serial
        .and_then(|s| obs.get(s))
        .and_then(|o| o.hop_delay_frames)
        .filter(|d| *d > 0);
    if let Some(d) = read.or(configured) {
        return u32::from(d);
    }
    if !issues
        .iter()
        .any(|i| i.code == "hop_delay_assumed" && i.subject.as_deref() == Some(id))
    {
        issues.push(issue(
            Some(id),
            "hop_delay_assumed",
            format!("{id}: its hop delay is neither reported nor configured; 1 frame assumed"),
        ));
    }
    1
}

/// Speakers' distance delays, by unit id.
/// Distance delays by unit id, each cut to what a unit holds at `rate`; the
/// cut ones are listed with their unit.
#[allow(clippy::type_complexity)]
fn out_delays<'a>(
    model: &'a SystemModel,
    rate: u32,
    offsets: &BTreeMap<String, i64>,
) -> BTreeMap<&'a str, (Vec<(u8, f32)>, Vec<u8>)> {
    let max_ms = f64::from(MAX_OUT_DELAY_SAMPLES) * 1000.0 / f64::from(rate.max(1));
    let mut furthest: BTreeMap<String, f32> = BTreeMap::new();
    let key = |u: &Unit| match &u.room {
        Some(r) => format!("room:{r}"),
        None => format!("unit:{}", u.id),
    };
    for u in &model.units {
        for s in &u.speakers {
            let f = furthest.entry(key(u)).or_insert(0.0);
            *f = f.max(s.distance_m);
        }
    }
    model
        .units
        .iter()
        .map(|u| {
            let far = furthest.get(&key(u)).copied().unwrap_or(0.0);
            // -- WP AUSB1 -- a USB-direct stream that plays early waits this long more
            let early_ms = offsets
                .get(&u.id)
                .map_or(0.0, |o| *o as f64 * 1000.0 / f64::from(rate.max(1)));
            let mut clamped = Vec::new();
            let delays = u
                .speakers
                .iter()
                .map(|s| {
                    let mut ms = f64::from(far - s.distance_m) / SPEED_OF_SOUND * 1000.0 + early_ms;
                    if ms > max_ms {
                        clamped.push(s.output);
                        // Rounded down, so the unit's cap is never reached.
                        ms = (max_ms * 1000.0).floor() / 1000.0;
                    }
                    // Rounded to 1 µs, so a recomputation writes the same value.
                    (s.output, ((ms * 1000.0).round() / 1000.0) as f32)
                })
                .collect();
            (u.id.as_str(), (delays, clamped))
        })
        .collect()
}

/// Compute the alignment. `obs` is keyed by serial.
///
/// -- WP AUSB1 -- A USB-direct unit (`transport: usb_direct`) has no path on
/// the audio link: its effective delay `E_u = D + (L_max - L_u) + S_u` is the
/// D it is given (`EXT_ALINK_USB.d`, samples, which are frames at the stream's
/// rate), inside its `d_min..=d_max`, so D is one system delay for both. The
/// start offsets of the USB-direct streams (2.9.3, U6) are equalised by
/// delaying the earlier units' outputs.
pub fn align(model: &SystemModel, obs: &BTreeMap<String, Observed>) -> Alignment {
    let rate = stream_rate(model, obs);
    let mut issues = Vec::new();
    let lip_us = |u: &Unit| {
        u.room
            .as_ref()
            .and_then(|r| model.rooms.iter().find(|x| &x.id == r))
            .and_then(|r| r.lip_sync_us)
            .unwrap_or(model.lip_sync_us)
    };

    struct Row<'a> {
        unit: &'a Unit,
        hops: u8,
        path: u32,
        latency: i64,
        lip: i64,
        limits: Option<(u16, u16)>,
        // -- fixup hwlk1f (F4) -- an offline unit's facts are as last seen: not written
        online: bool,
        // -- WP AUSB1 --
        /// A USB-direct unit, and the mode it is to run.
        usb: Option<String>,
        /// Its firmware has the block and it was read.
        usb_seen: bool,
    }
    let mut rows = Vec::new();
    for u in &model.units {
        let o = obs.get(&u.serial);
        // -- WP AUSB1 --
        if u.usb_direct() {
            let seen = o.and_then(|o| o.usb.as_ref());
            let mode = u
                .usb_mode
                .clone()
                .or_else(|| seen.map(|s| s.mode.clone()))
                .unwrap_or_else(|| "off".into());
            let phase = mode == "phase_lock";
            let limits = seen.filter(|_| phase).map(|s| (s.d_min, s.d_max));
            if !o.is_some_and(|o| o.online) {
                issues.push(issue(
                    Some(&u.id),
                    "offline",
                    format!("{}: not attached to the hub", u.id),
                ));
            } else if o.is_some_and(Observed::without_usb_direct) {
                issues.push(issue(
                    Some(&u.id),
                    "no_usb_direct",
                    format!(
                        "{}: its firmware has no USB-direct block; it is not aligned and takes scenes at once",
                        u.id
                    ),
                ));
            } else if seen.is_none() {
                issues.push(issue(
                    Some(&u.id),
                    "limits_unknown",
                    format!("{}: its USB-direct state could not be read", u.id),
                ));
            } else if let Some(s) = seen.filter(|s| s.lock_error) {
                // -- WP AUSB2 -- it was asked to be in step and cannot
                let name = if u.name.is_empty() { &u.id } else { &u.name };
                issues.push(issue(
                    Some(&u.id),
                    "usb_lock_error",
                    lock_error_message(name, &s.on_lock_error, s.lock_error_cause.as_deref()),
                ));
            } else if !phase {
                issues.push(issue(
                    Some(&u.id),
                    "usb_not_phase_locked",
                    format!(
                        "{}: USB-direct {}: not aligned to the sample; scenes reach it at once",
                        u.id,
                        if mode == "rate_lock" {
                            "rate lock only"
                        } else {
                            "off"
                        }
                    ),
                ));
            }
            // -- WP AUSB2 -- "play unaligned" asked of firmware that cannot
            if u.on_lock_error == super::model::OnLockError::PlayUnaligned
                && seen.is_some_and(|s| s.play_unaligned_unsupported)
            {
                let name = if u.name.is_empty() { &u.id } else { &u.name };
                issues.push(issue(
                    Some(&u.id),
                    USB_LOCK_ERROR_UNSUPPORTED,
                    format!(
                        "{name}: its firmware cannot play unaligned in lock error, so it mutes \
                         (update it, or set it to mute)"
                    ),
                ));
            }
            rows.push(Row {
                unit: u,
                hops: 0,
                path: 0,
                latency: us_to_frames(i64::from(u.latency()), rate),
                lip: us_to_frames(i64::from(lip_us(u)), rate),
                limits,
                online: o.is_some_and(|o| o.online),
                usb: Some(mode),
                usb_seen: seen.is_some(),
            });
            continue;
        }
        // The model is valid, so the path exists.
        let (sender, reemitters) = model.path_to(&u.id).unwrap_or_default();
        let path = reemitters
            .iter()
            .map(|r| hop_delay(model, obs, r, &mut issues))
            .fold(0u32, u32::saturating_add);
        let hops = u8::try_from(reemitters.len()).unwrap_or(u8::MAX);
        let limits = o.and_then(|o| Some((o.d_local_min?, o.d_local_max?)));
        if !o.is_some_and(|o| o.online) {
            issues.push(issue(
                Some(&u.id),
                "offline",
                format!("{}: not attached to the hub", u.id),
            ));
        } else if o.is_some_and(Observed::without_alink) {
            // -- WP HWLK1F -- said as what it is, not as unread limits
            issues.push(issue(
                Some(&u.id),
                "no_alink",
                format!(
                    "{}: its firmware has no audio link; it is not aligned and takes scenes at once",
                    u.id
                ),
            ));
        } else if limits.is_none() {
            issues.push(issue(
                Some(&u.id),
                "limits_unknown",
                format!("{}: its delay limits could not be read", u.id),
            ));
        }
        if let Some(seen) = o.and_then(|o| o.hops)
            && seen != hops
        {
            issues.push(issue(
                Some(&u.id),
                "hops_mismatch",
                format!(
                    "{}: the stream says {seen} re-emitters, the model {hops}",
                    u.id
                ),
            ));
        }
        let want = model
            .senders
            .iter()
            .find(|s| s.id == sender)
            .and_then(|s| s.sender_id);
        if let (Some(want), Some(seen)) = (want, o.and_then(|o| o.sender_id))
            && want != seen
            && o.is_some_and(|o| o.state.as_deref() == Some("locked"))
        {
            issues.push(issue(
                Some(&u.id),
                "sender_mismatch",
                format!(
                    "{}: hears sender {seen:#04x}, the model says {sender} ({want:#04x})",
                    u.id
                ),
            ));
        }
        rows.push(Row {
            unit: u,
            hops,
            path,
            latency: us_to_frames(i64::from(u.latency()), rate),
            lip: us_to_frames(i64::from(lip_us(u)), rate),
            limits,
            online: o.is_some_and(|o| o.online),
            usb: None,
            usb_seen: false,
        });
    }
    // -- WP AUSB1 -- one system delay for both transports: said, with what it leaves out
    if rows.iter().any(|r| r.usb.is_some()) && rows.iter().any(|r| r.usb.is_none()) {
        issues.push(issue(
            None,
            "mixed_time_bases",
            "USB-direct units play D samples after a sample reaches them, audio link units D frames \
             after the sender sends it; the sender's own USB latency is not counted, so check a \
             mixed room with a test click and enter the measured latency"
                .into(),
        ));
    }
    let offsets = start_offsets(model, obs, &mut issues);
    let l_max = rows.iter().map(|r| r.latency).max().unwrap_or(0);
    // E_u = D - extra_u, with extra_u = path - (L_max - L) - lip.
    let extra = |r: &Row| {
        i64::from(r.path)
            .saturating_sub(l_max.saturating_sub(r.latency))
            .saturating_sub(r.lip)
    };
    let needed = rows
        .iter()
        .filter_map(|r| {
            let (min, _) = r.limits?;
            // d_local = D - hops must not go below 0 either.
            Some(
                i64::from(min)
                    .saturating_add(extra(r))
                    .max(i64::from(r.hops)),
            )
        })
        .max();
    let (d, d_auto) = match model.d_frames {
        Some(d) => (i64::from(d), false),
        None => (needed.unwrap_or(i64::from(DEFAULT_D)).max(0), true),
    };
    let d = d.min(i64::from(u16::MAX));
    if let (false, Some(n)) = (d_auto, needed)
        && n > d
    {
        issues.push(issue(
            None,
            "d_too_small",
            format!("D = {d} frames is below what the units need ({n})"),
        ));
    }
    let delays = out_delays(model, rate, &offsets);
    for u in &model.units {
        if let Some((_, clamped)) = delays.get(u.id.as_str()).filter(|(_, c)| !c.is_empty()) {
            issues.push(issue(
                Some(&u.id),
                "delay_over_max",
                format!(
                    "{}: outputs {clamped:?} need a longer distance delay than the {} samples a unit \
                     holds at {rate} Hz; cut to it, so those speakers are not aligned",
                    u.id, MAX_OUT_DELAY_SAMPLES
                ),
            ));
        }
    }
    let units = rows
        .iter()
        .map(|r| {
            let effective = d.saturating_sub(extra(r));
            let in_range = r
                .limits
                .is_some_and(|(min, max)| (i64::from(min)..=i64::from(max)).contains(&effective));
            let out_delays = delays
                .get(r.unit.id.as_str())
                .map(|(d, _)| d.clone())
                .unwrap_or_default();
            let clamped_outputs = delays
                .get(r.unit.id.as_str())
                .map(|(_, c)| c.clone())
                .unwrap_or_default();
            // -- WP AUSB1 --
            if let Some(mode) = &r.usb {
                let phase = mode == "phase_lock";
                if phase && r.limits.is_some() && !in_range {
                    issues.push(issue(
                        Some(&r.unit.id),
                        "delay_out_of_range",
                        format!(
                            "{}: needs a USB-direct delay of {effective} samples, outside {:?}",
                            r.unit.id, r.limits
                        ),
                    ));
                }
                let offset = offsets.get(&r.unit.id).copied();
                return UnitPlan {
                    unit: r.unit.id.clone(),
                    serial: r.unit.serial.clone(),
                    hops: 0,
                    path_delay_frames: 0,
                    latency_frames: r.latency,
                    lip_sync_frames: r.lip,
                    effective,
                    d_local: 0,
                    hops_offset: 0,
                    out_delays,
                    clamped_outputs,
                    writable: r.online && r.usb_seen && (!phase || in_range),
                    transport: "usb_direct".into(),
                    usb_d: if phase {
                        u16::try_from(effective).ok()
                    } else {
                        None
                    },
                    usb_mode: Some(mode.clone()),
                    start_offset_samples: offset,
                };
            }
            let mut d_local = d.saturating_sub(i64::from(r.hops));
            let mut offset = effective.saturating_sub(d_local);
            let (min, max) = (
                i64::from(HOPS_OFFSET_MAX.saturating_neg()),
                i64::from(HOPS_OFFSET_MAX),
            );
            if offset.abs() > max {
                offset = offset.clamp(min, max);
                d_local = effective.saturating_sub(offset);
            }
            let d_local_ok = u16::try_from(d_local).ok();
            if r.limits.is_some() && !in_range {
                issues.push(issue(
                    Some(&r.unit.id),
                    "delay_out_of_range",
                    format!(
                        "{}: needs an effective delay of {effective} frames, outside {:?}",
                        r.unit.id, r.limits
                    ),
                ));
            }
            UnitPlan {
                unit: r.unit.id.clone(),
                serial: r.unit.serial.clone(),
                hops: r.hops,
                path_delay_frames: r.path,
                latency_frames: r.latency,
                lip_sync_frames: r.lip,
                effective,
                d_local: d_local_ok.unwrap_or(0),
                hops_offset: i16::try_from(offset).unwrap_or(0),
                out_delays,
                clamped_outputs,
                writable: in_range && d_local_ok.is_some() && r.online,
                transport: "alink".into(),
                usb_d: None,
                usb_mode: None,
                start_offset_samples: None,
            }
        })
        .collect();
    Alignment {
        d_frames: u16::try_from(d).unwrap_or(u16::MAX),
        d_auto,
        rate_hz: rate,
        latency_max_frames: l_max,
        units,
        issues,
    }
}

// -- WP AUSB1 --
/// The start offsets of the USB-direct streams (2.9.3, U6), by unit id: how
/// many samples earlier each phase-locked unit's content plays than the
/// latest one's, from each unit's stream start bus frame and `stream_shift`.
/// Units online, phase-locked, with a stream on the timeline, at one rate;
/// the others are left out (said when they would count).
fn start_offsets(
    model: &SystemModel,
    obs: &BTreeMap<String, Observed>,
    issues: &mut Vec<Issue>,
) -> BTreeMap<String, i64> {
    let mut starts = Vec::new();
    let mut spf_seen = None;
    for u in model.units.iter().filter(|u| u.usb_direct()) {
        let Some(seen) = obs
            .get(&u.serial)
            .filter(|o| o.online)
            .and_then(|o| o.usb.as_ref())
            .filter(|s| s.phase_locked())
        else {
            continue;
        };
        let (Some(start), Some(spf)) = (
            seen.stream_start_bus_frame,
            dspi_proto::ext::alink_usb::samples_per_frame(seen.rate_code),
        ) else {
            issues.push(issue(
                Some(&u.id),
                "usb_no_stream",
                format!(
                    "{}: no stream on its timeline: its start offset is not known (0 assumed)",
                    u.id
                ),
            ));
            continue;
        };
        // -- WP FIXM1 -- (review ausb1 F3) said, never dropped silently
        if let Some(first) = spf_seen.filter(|s| *s != spf) {
            issues.push(issue(
                Some(&u.id),
                "usb_rate_differs",
                format!(
                    "{}: its USB stream runs at {} samples per bus frame, the first USB-direct \
                     unit's at {first}: its start offset is not equalised with theirs",
                    u.id, spf
                ),
            ));
            continue;
        }
        spf_seen = Some(spf);
        starts.push((u.id.clone(), (start, seen.stream_shift)));
    }
    let (Some(spf), Some((_, reference))) = (spf_seen, starts.first().cloned()) else {
        return BTreeMap::new();
    };
    // How much earlier each plays than the reference; the latest needs nothing.
    let lead: Vec<(String, i64)> = starts
        .iter()
        .map(|(id, s)| {
            (
                id.clone(),
                dspi_proto::ext::alink_usb::content_lead(*s, reference, spf),
            )
        })
        .collect();
    let latest = lead.iter().map(|(_, l)| *l).min().unwrap_or(0);
    let offsets: BTreeMap<String, i64> = lead
        .into_iter()
        .map(|(id, l)| (id, l.saturating_sub(latest)))
        .collect();
    if offsets.values().any(|o| *o != 0) {
        issues.push(issue(
            None,
            "usb_start_offsets",
            "USB-direct streams started apart: the earlier units' outputs are delayed to match. \
             The offsets move whenever the host reopens a stream or a unit anchors again; the hub \
             then says so (or, with \"Keep USB-direct units aligned\", writes the delays again)"
                .into(),
        ));
    }
    for (id, o) in &offsets {
        let speakers = model.unit(id).is_some_and(|u| !u.speakers.is_empty());
        if *o != 0 && !speakers {
            issues.push(issue(
                Some(id),
                "start_offset_no_outputs",
                format!(
                    "{id}: its stream plays {o} samples early, but the model lists no speakers \
                     to delay: add its outputs"
                ),
            ));
        }
    }
    offsets
}

// -- WP AUSB2 --
/// The issue raised when a unit refused "play unaligned" (firmware before
/// the fork's WP USB3b).
pub const USB_LOCK_ERROR_UNSUPPORTED: &str = "usb_lock_error_unsupported";

// -- WP FIXM1 --
/// The issue raised when USB-direct start offsets moved since the hub last
/// wrote them ([`usb_offsets_moved`]).
pub const USB_OFFSETS_CHANGED: &str = "usb_start_offsets_changed";

// -- WP FIXM1 --
/// The start offsets of an alignment's USB-direct units, by unit id: those
/// with a stream on the timeline ([`UnitPlan::start_offset_samples`]).
pub fn usb_offsets(alignment: &Alignment) -> BTreeMap<String, i64> {
    alignment
        .units
        .iter()
        .filter(|p| p.transport == "usb_direct")
        .filter_map(|p| Some((p.unit.clone(), p.start_offset_samples?)))
        .collect()
}

// -- WP FIXM1 --
/// The USB-direct units whose start offset now differs from the one the hub
/// wrote (`written`; a unit missing there was written with none, 0). Units
/// without a stream now are not judged (`usb_no_stream` says so). Offsets
/// are relative, so streams that all moved together change nothing.
pub fn usb_offsets_moved(
    written: &BTreeMap<String, i64>,
    now: &BTreeMap<String, i64>,
) -> Vec<String> {
    now.iter()
        .filter(|(id, o)| written.get(*id).copied().unwrap_or(0) != **o)
        .map(|(id, _)| id.clone())
        .collect()
}

// -- WP FIXM1 --
/// The issue for [`usb_offsets_moved`]'s `moved` units.
pub fn usb_offsets_changed_issue(moved: &[String]) -> Issue {
    issue(
        None,
        USB_OFFSETS_CHANGED,
        format!(
            "USB-direct start offsets changed \u{2014} Align again ({} moved since the last \
             alignment; their speakers' delays no longer match)",
            moved.join(", ")
        ),
    )
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects, clippy::indexing_slicing)]
pub(crate) mod tests_wp_sw2 {
    use super::super::model::*;
    use super::*;

    fn unit(id: &str, upstream: &str, room: &str) -> Unit {
        Unit {
            id: id.into(),
            name: String::new(),
            serial: format!("S-{id}"),
            room: Some(room.into()),
            upstream: upstream.into(),
            transport: Default::default(),
            usb_mode: None,
            on_lock_error: Default::default(), // -- WP AUSB2 --
            output: None,
            repeater: false,
            hop_delay_frames: None,
            slots: None,
            latency_us: 0,
            measured_latency_us: None,
            speakers: Vec::new(),
        }
    }

    /// A sender, a fan-out (3 frames) behind it, a repeater (1 frame) behind
    /// the fan-out; units A (direct), B (behind the fan-out), C (behind both).
    pub(crate) fn cinema() -> SystemModel {
        let mut rep = unit("R", "fan", "cinema");
        rep.repeater = true;
        SystemModel {
            rooms: vec![Room {
                id: "cinema".into(),
                name: "Cinema".into(),
                lip_sync_us: None,
            }],
            senders: vec![Sender {
                id: "tx".into(),
                name: String::new(),
                serial: Some("S-tx".into()),
                sender_id: Some(0x42),
            }],
            fan_outs: vec![FanOut {
                id: "fan".into(),
                name: String::new(),
                serial: Some("S-fan".into()),
                upstream: "tx".into(),
                output: Some(1),
                hop_delay_frames: None,
            }],
            units: vec![
                unit("A", "tx", "cinema"),
                unit("B", "fan", "cinema"),
                rep,
                unit("C", "R", "cinema"),
            ],
            ..SystemModel::default()
        }
    }

    fn seen(min: u16, hops: u8, hop_delay: u8) -> Observed {
        Observed {
            online: true,
            d_local_min: Some(min),
            d_local_max: Some(1024),
            hops: Some(hops),
            hop_delay_frames: Some(hop_delay),
            sender_id: Some(0x42),
            rate_code: Some(1),
            state: Some("locked".into()),
            ..Observed::default()
        }
    }

    fn obs() -> BTreeMap<String, Observed> {
        BTreeMap::from([
            ("S-fan".into(), seen(0, 0, 3)),
            ("S-A".into(), seen(112, 0, 0)),
            ("S-B".into(), seen(112, 1, 0)),
            ("S-R".into(), seen(112, 1, 1)),
            ("S-C".into(), seen(112, 2, 0)),
        ])
    }

    /// Every unit hears frame F at t_F + path and plays it E later: the sums
    /// agree, so all four play F at once, and the firmware's split holds.
    #[test]
    fn wp_sw2_units_behind_fan_outs_and_repeaters_play_together() {
        let m = cinema();
        m.validate().unwrap();
        let a = align(&m, &obs());
        assert!(a.issues.is_empty(), "{:?}", a.issues);
        assert!(a.d_auto);
        // C is behind 3 + 1 frames: D = 112 + 4.
        assert_eq!(a.d_frames, 116);
        for p in &a.units {
            assert_eq!(i64::from(p.path_delay_frames) + p.effective, 116, "{p:?}");
            assert_eq!(i64::from(p.d_local) + i64::from(p.hops_offset), p.effective);
            assert!(p.writable);
        }
        let c = &a.units[3];
        assert_eq!((c.hops, c.d_local, c.hops_offset), (2, 114, -2));
        let b = &a.units[1];
        // 2.8.5: -sum(d_k - 1) for one fan-out of 3 frames.
        assert_eq!((b.d_local, b.hops_offset), (115, -2));
        assert_eq!((a.units[0].d_local, a.units[0].hops_offset), (116, 0));
    }

    #[test]
    fn wp_sw2_latency_lip_sync_and_a_configured_d() {
        let mut m = cinema();
        // B's amplifier adds 250 µs = 12 frames; the room 10 ms of lip sync.
        m.units[1].measured_latency_us = Some(250);
        m.units[1].latency_us = 99_999;
        m.rooms[0].lip_sync_us = Some(10_000);
        m.d_frames = Some(200);
        let a = align(&m, &obs());
        assert!(!a.d_auto && a.d_frames == 200);
        assert_eq!(a.latency_max_frames, 12);
        for p in &a.units {
            // Heard at t_F + D + L_max + S for every unit.
            assert_eq!(
                i64::from(p.path_delay_frames) + p.effective + p.latency_frames,
                200 + 12 + 480,
                "{p:?}"
            );
        }
        // B: its own 12 frames of latency need no extra delay; the others 12.
        assert_eq!(a.units[0].effective, 200 + 12 + 480);
        // Past -64..64 the remainder moved into d_local.
        assert_eq!(a.units[0].hops_offset, 64);
        assert_eq!(a.units[0].d_local, 200 + 12 + 480 - 64);
        // Over 1024 with D 200 + lip sync 480 + 12? 692: within.
        assert!(a.units.iter().all(|p| p.writable));

        // Too small a D for the limits: reported, those units not writable.
        m.d_frames = Some(100);
        m.rooms[0].lip_sync_us = None;
        m.units[1].measured_latency_us = None;
        m.units[1].latency_us = 0;
        let a = align(&m, &obs());
        assert!(a.issues.iter().any(|i| i.code == "d_too_small"));
        assert!(a.units.iter().all(|p| !p.writable));
    }

    #[test]
    fn wp_sw2_hop_delays_fall_back_and_topology_is_checked() {
        let m = cinema();
        let mut o = obs();
        o.get_mut("S-fan").unwrap().hop_delay_frames = None;
        o.get_mut("S-C").unwrap().hops = Some(1);
        o.get_mut("S-B").unwrap().sender_id = Some(0x43);
        o.get_mut("S-A").unwrap().online = false;
        let a = align(&m, &o);
        let codes: Vec<&str> = a.issues.iter().map(|i| i.code.as_str()).collect();
        assert!(codes.contains(&"hop_delay_assumed"));
        assert_eq!(
            codes.iter().filter(|c| **c == "hop_delay_assumed").count(),
            1
        );
        assert!(codes.contains(&"hops_mismatch"));
        assert!(codes.contains(&"sender_mismatch"));
        assert!(codes.contains(&"offline"));
        // The fan-out counted 1 frame: B's path is 1.
        assert_eq!(a.units[1].path_delay_frames, 1);

        // A configured hop delay is used when the device does not say.
        let mut m = cinema();
        m.fan_outs[0].hop_delay_frames = Some(2);
        let a = align(&m, &o);
        assert_eq!(a.units[1].path_delay_frames, 2);
    }

    #[test]
    fn wp_sw2_speaker_distances_become_output_delays_per_room() {
        let mut m = cinema();
        m.units[0].speakers = vec![
            Speaker {
                output: 0,
                distance_m: 3.43,
            },
            Speaker {
                output: 1,
                distance_m: 2.0,
            },
        ];
        m.units[1].speakers = vec![Speaker {
            output: 2,
            distance_m: 1.715,
        }];
        let a = align(&m, &obs());
        assert_eq!(a.units[0].out_delays[0], (0, 0.0));
        assert!((a.units[0].out_delays[1].1 - 4.169).abs() < 0.001);
        assert!((a.units[1].out_delays[0].1 - 5.0).abs() < 0.001);
        assert!(a.units.iter().all(|p| p.clamped_outputs.is_empty()));
    }

    // -- fixup sw2x (F3) --
    /// 15 m between the nearest and furthest speaker is 43.7 ms, over the 2046
    /// samples (42.6 ms) a unit holds at 48 kHz: cut, and reported.
    #[test]
    fn wp_sw2x_a_distance_delay_over_the_maximum_is_cut_and_reported() {
        let mut m = cinema();
        m.units[0].speakers = vec![
            Speaker {
                output: 0,
                distance_m: 16.0,
            },
            Speaker {
                output: 1,
                distance_m: 1.0,
            },
        ];
        let a = align(&m, &obs());
        let p = &a.units[0];
        assert_eq!(p.clamped_outputs, vec![1]);
        let max_ms = 2046.0 / 48.0;
        assert!(p.out_delays[1].1 <= max_ms && p.out_delays[1].1 > max_ms - 0.01);
        assert!(a.issues.iter().any(|i| i.code == "delay_over_max"));
    }

    #[test]
    fn wp_sw2_units_without_limits_or_rate() {
        let m = cinema();
        let a = align(&m, &BTreeMap::new());
        assert_eq!(a.d_frames, DEFAULT_D);
        assert_eq!(a.rate_hz, 48_000);
        assert!(a.units.iter().all(|p| !p.writable));
        assert_eq!(us_to_frames(250, 48_000), 12);
        assert_eq!(us_to_frames(-250, 48_000), -12);
        assert_eq!(
            us_to_frames(i64::MAX, 96_000),
            us_to_frames(i64::MAX, 96_000)
        );
    }

    // -- fixup sw2x --
    /// LK6's limits (2026-11-13): 257..399 frames under the default 4 ms latency
    /// option, 81..431 under 1 ms, at 48 kHz. The engine takes D from what each
    /// unit reports, never the 128 default: here 257 + C's 4 frames of path.
    #[test]
    fn wp_sw2x_d_follows_the_units_reported_limits() {
        let m = cinema();
        let mut o = obs();
        for (serial, min, max) in [
            ("S-A", 257, 399),
            ("S-B", 81, 431),
            ("S-R", 81, 431),
            ("S-C", 257, 399),
        ] {
            let x = o.get_mut(serial).unwrap();
            x.d_local_min = Some(min);
            x.d_local_max = Some(max);
        }
        let a = align(&m, &o);
        assert!(a.issues.is_empty(), "{:?}", a.issues);
        assert_eq!(a.d_frames, 261);
        assert!(a.units.iter().all(|p| p.writable && p.effective >= 257 - 4));
        // Under the 4 ms option a configured D of 128 is refused, not written.
        let mut m = cinema();
        m.d_frames = Some(128);
        let a = align(&m, &o);
        assert!(a.issues.iter().any(|i| i.code == "d_too_small"));
        assert!(!a.units[0].writable);
    }
}

// -- WP FIXM1 --
#[cfg(test)]
#[allow(clippy::indexing_slicing)]
mod tests_wp_fixm1 {
    use super::super::model::*;
    use super::*;

    fn usb_unit(id: &str) -> Unit {
        Unit {
            id: id.into(),
            name: String::new(),
            serial: format!("S-{id}"),
            room: Some("main".into()),
            upstream: String::new(),
            transport: UnitTransport::UsbDirect,
            usb_mode: Some("phase_lock".into()),
            on_lock_error: Default::default(), // -- WP AUSB2 --
            output: None,
            repeater: false,
            hop_delay_frames: None,
            slots: None,
            latency_us: 0,
            measured_latency_us: None,
            speakers: vec![Speaker {
                output: 0,
                distance_m: 3.0,
            }],
        }
    }

    fn seen(rate_code: u8, start: u16) -> Observed {
        Observed {
            online: true,
            usb: Some(UsbSeen {
                mode: "phase_lock".into(),
                d: 192,
                state: "phase_locked".into(),
                phase_state: 2,
                rate_code,
                d_min: 170,
                d_max: 399,
                stream: true,
                stream_start_bus_frame: Some(start),
                ..UsbSeen::default()
            }),
            ..Observed::default()
        }
    }

    fn model() -> SystemModel {
        SystemModel {
            rooms: vec![Room {
                id: "main".into(),
                name: "Main".into(),
                lip_sync_us: None,
            }],
            units: vec![usb_unit("U1"), usb_unit("U2"), usb_unit("U3")],
            ..SystemModel::default()
        }
    }

    /// A unit at another rate is said, not dropped silently.
    #[test]
    fn wp_fixm1_a_usb_unit_at_another_rate_gets_an_issue() {
        let obs = BTreeMap::from([
            ("S-U1".to_owned(), seen(1, 100)),
            ("S-U2".to_owned(), seen(1, 102)),
            ("S-U3".to_owned(), seen(2, 100)),
        ]);
        let a = align(&model(), &obs);
        let rate: Vec<_> = a
            .issues
            .iter()
            .filter(|i| i.code == "usb_rate_differs")
            .collect();
        assert_eq!(rate.len(), 1, "{:?}", a.issues);
        assert_eq!(rate[0].subject.as_deref(), Some("U3"));
        assert!(
            rate[0].message.contains("96 samples per bus frame"),
            "{}",
            rate[0].message
        );
        let offsets = usb_offsets(&a);
        assert_eq!(
            offsets,
            BTreeMap::from([("U1".to_owned(), 96), ("U2".to_owned(), 0)]),
            "U3 is not equalised"
        );
    }

    /// Moved offsets are found against what was written; streams that all
    /// moved together, or a unit without a stream now, are not.
    #[test]
    fn wp_fixm1_moved_offsets_are_told_apart() {
        let written = BTreeMap::from([("U1".to_owned(), 96), ("U2".to_owned(), 0)]);
        let same = written.clone();
        assert!(usb_offsets_moved(&written, &same).is_empty());
        let moved = BTreeMap::from([("U1".to_owned(), 0), ("U2".to_owned(), 480)]);
        assert_eq!(usb_offsets_moved(&written, &moved), ["U1", "U2"]);
        // A unit not written had none (0); one without a stream now is not judged.
        let new_unit = BTreeMap::from([("U1".to_owned(), 96), ("U3".to_owned(), 48)]);
        assert_eq!(usb_offsets_moved(&written, &new_unit), ["U3"]);
        let i = usb_offsets_changed_issue(&["Rear L/R".to_owned()]);
        assert_eq!(i.code, USB_OFFSETS_CHANGED);
        assert!(
            i.message.starts_with(
                "USB-direct start offsets changed \u{2014} Align again (Rear L/R moved"
            )
        );
    }
}
