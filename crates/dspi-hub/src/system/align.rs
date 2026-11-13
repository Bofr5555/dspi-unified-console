//! The alignment engine (WP SW2, SYS2 3.1-3.2 and 3.5, `link_protocol_spec.md`
//! 7.2): from the model and what the devices report, each unit's
//! `EXT_LINK_PHASE` (`d_local`, `hops_offset`) and its outputs' `out.delay`, so
//! that every unit plays link frame F at the same instant and every speaker in
//! a room is heard at once at the listening position. Pure: the I/O is in
//! `system::run`.
//!
//! In frames at the stream's rate, for a unit `u` behind re-emitters with hop
//! delays `d_1..d_h` (`h` = its hops), with latency after the link `L_u` and its
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

use dspi_proto::ext::audio_link::HOPS_OFFSET_MAX;
use serde::Serialize;

use super::model::{SystemModel, Unit};

/// The speed of sound the distance delays use, m/s (20 °C).
pub const SPEED_OF_SOUND: f64 = 343.0;
/// D when nothing reports a limit (SYS2 3.2's default for the 1 ms option).
pub const DEFAULT_D: u16 = 128;

/// What the hub read from one device, by serial. Every field is optional: a
/// device offline, without the link, or not reporting it.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Observed {
    /// Attached to the hub now.
    pub online: bool,
    /// `EXT_LINK_INFO.role`'s name.
    pub role: Option<String>,
    /// `EXT_LINK_INFO.d_local_min`.
    pub d_local_min: Option<u16>,
    /// `EXT_LINK_INFO.d_local_max`.
    pub d_local_max: Option<u16>,
    /// `EXT_LINK_STATUS.state`'s name.
    pub state: Option<String>,
    /// `EXT_LINK_STATUS.hops`.
    pub hops: Option<u8>,
    /// `EXT_LINK_STATUS.hop_delay_frames` (offset 50).
    pub hop_delay_frames: Option<u8>,
    /// `EXT_LINK_STATUS.sender_id`.
    pub sender_id: Option<u8>,
    /// `EXT_LINK_STATUS.rate_code`.
    pub rate_code: Option<u8>,
    /// `EXT_LINK_STATUS.crc16_errors`.
    pub crc16_errors: Option<u32>,
    /// `EXT_LINK_STATUS.loss_count`.
    pub loss_count: Option<u32>,
    /// `EXT_LINK_STATUS.phase_error_frames`.
    pub phase_error_frames: Option<f32>,
    /// The link frame (`EXT_FRAME_COUNTER`, else `EXT_LINK_STATUS`).
    pub frame: Option<u64>,
    /// `EXT_LINK_PHASE` as read: mode, d_local, hops_offset, flags.
    pub phase: Option<(u8, u16, i16, u8)>,
    /// The listed sub-commands include `EXT_LINK_CTRL_MSG` OUT (a sender build).
    pub superframe_path: bool,
    /// The device announces 0xE1 (`EXT_NOTIFY_FEATURES` bit 2).
    pub state_events: bool,
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
    /// Its latency after the link, frames.
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
    /// Whether the plan may be written (in range, reachable).
    pub writable: bool,
}

/// A finding: something the installer should look at.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Issue {
    /// The element it is about, if one.
    pub subject: Option<String>,
    /// A stable code: `limits_unknown`, `delay_out_of_range`,
    /// `hop_delay_assumed`, `hops_mismatch`, `sender_mismatch`, `offline`,
    /// `d_too_small`.
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
            .and_then(|o| o.rate_code)
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
fn out_delays(model: &SystemModel) -> BTreeMap<&str, Vec<(u8, f32)>> {
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
            let delays = u
                .speakers
                .iter()
                .map(|s| {
                    let ms = f64::from(far - s.distance_m) / SPEED_OF_SOUND * 1000.0;
                    // Rounded to 1 µs, so a recomputation writes the same value.
                    (s.output, ((ms * 1000.0).round() / 1000.0) as f32)
                })
                .collect();
            (u.id.as_str(), delays)
        })
        .collect()
}

/// Compute the alignment. `obs` is keyed by serial.
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
    }
    let mut rows = Vec::new();
    for u in &model.units {
        // The model is valid, so the path exists.
        let (sender, reemitters) = model.path_to(&u.id).unwrap_or_default();
        let path = reemitters
            .iter()
            .map(|r| hop_delay(model, obs, r, &mut issues))
            .fold(0u32, u32::saturating_add);
        let hops = u8::try_from(reemitters.len()).unwrap_or(u8::MAX);
        let o = obs.get(&u.serial);
        let limits = o.and_then(|o| Some((o.d_local_min?, o.d_local_max?)));
        if !o.is_some_and(|o| o.online) {
            issues.push(issue(
                Some(&u.id),
                "offline",
                format!("{}: not attached to the hub", u.id),
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
        });
    }
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
    let delays = out_delays(model);
    let units = rows
        .iter()
        .map(|r| {
            let effective = d.saturating_sub(extra(r));
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
            let in_range = r
                .limits
                .is_some_and(|(min, max)| (i64::from(min)..=i64::from(max)).contains(&effective));
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
                out_delays: delays.get(r.unit.id.as_str()).cloned().unwrap_or_default(),
                writable: in_range && d_local_ok.is_some(),
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
}
