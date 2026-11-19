//! The demo hub's room (WP ATDEMO): a simulated microphone that hears the
//! generators of the demo's own fake units (WP ATFAKE) during a
//! `system.measure` session, so a client can measure several units end to
//! end with no microphone and no sound anywhere. Shipped demo code behind the
//! feature `demo-room`; installed only by `uconsole --demo hub --alink
//! --with-measure-units` ([`HubHandle::set_system_measure_demo_room`]).
//!
//! **What it hears.** Every generator start a demo unit recorded (op 9 at
//! its T, `FakeRemote::measure_timed_starts`) is rendered with WP ATSIM's
//! world (the three-way room's acoustics, a typical microphone, each
//! output's distance to the microphone plus the `out.delay` the unit holds
//! when it starts) and laid onto one capture per session: capture frame `k`
//! is what the microphone hears at the domain's sample `base + k`, delivered
//! up to the domain's T now less the microphone's latency (40 ms), with the room's
//! noise added. As WP ATE2E's room, but in real time and with the clock exact.
//!
//! **Fail closed.** `system.measure.demo_mic` answers only for a session
//! whose every unit is one of the demo units the room was installed with:
//! the same attachment (the hub's device object, not just the serial), so a
//! real DSPi (or anything attached later under a demo serial) is never
//! heard and the request is refused `unsupported`. Without a room the hub
//! offers no `system_measure_demo_mic` cap and refuses the message too.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex, Weak};

use dspi_link::Dir;
use dspi_measure::sim::system::{SimUnit, SystemWorld, UnitLink, render_system};
use dspi_measure::sim::{Microphone, ProtoModel, World, flat_chains};
use dspi_measure::stimulus::{LogSweep, SweepConfig, WalkSchedule};
use dspi_measure::system::messages::demo::{DemoMicReply, DemoMicRequest, FULL_SCALE, MAX_FRAMES};
use dspi_measure::system::{DomainId, SystemSchedule, UnitIndex};
use dspi_proto::generated::opcodes as op;
use dspi_proto::packets::SiggenConfig;
use dspi_transport::fake::FakeRemote;
use serde_json::Value;

use super::io::Io;
use super::{MSession, MeasureHub, err, guard, ok_of, to_domain, urefs};
use crate::device::DeviceShared;
use crate::hub::HubHandle;
use crate::session::Session;

/// What the answers call the room.
pub const ROOM: &str = "the demo hub's simulated room (three-way speakers, one microphone)";
/// The microphone's input latency, ms. Short: a client's time pin is read
/// after its read of the room, so its apparent latency is this plus that
/// wait (a loaded browser's, a debug hub's), and the analysis looks for the
/// first sweep at most 500 ms after its START. Every start up to the T a
/// read delivers to is rendered first (see `read`), so none is heard late.
const LAG_MS: u64 = 40;
/// A render starts this many frames before its start's T.
const MARGIN: u64 = 480;
/// The noise at the capture, peak of a uniform distribution (-85 dBFS RMS).
const NOISE: f32 = 9.7e-5;
/// The most capture a session keeps ahead of what was read, frames (60 s
/// at 48 kHz): a client that stops reading does not grow the room.
const MAX_AIR: usize = 2_880_000;
/// Starts remembered per unit.
const MAX_SEEN: usize = 512;
/// Each output's gain into the room, dB (as ATE2E's).
const OUTPUT_GAIN_DB: f64 = -6.0;

/// One demo unit the room hears.
#[derive(Clone)]
pub struct DemoRoomUnit {
    /// Its serial, as attached.
    pub serial: String,
    /// The fake's other side: its recorded starts.
    pub remote: FakeRemote,
    /// Each output's acoustic delay to the microphone, ms (outputs beyond
    /// the list are 0.4 ms further each than the last).
    pub acoustic_ms: Vec<f64>,
}

struct RoomUnit {
    unit: DemoRoomUnit,
    /// The attachment the room was installed for.
    dev: Weak<DeviceShared>,
}

/// One session's capture.
struct Stream {
    /// The domain T heard at capture frame 0.
    base_t: u64,
    /// Starts rendered, per session unit: `(run_id, t)`.
    seen: Vec<BTreeSet<(u32, u64)>>,
    /// The capture not read yet, from capture frame `air_base`.
    air: Vec<f32>,
    air_base: u64,
    noise: u64,
}

/// The room of one demo hub.
pub(crate) struct DemoRoom {
    units: Vec<RoomUnit>,
    streams: Mutex<BTreeMap<String, Stream>>,
    seed: u64,
}

impl DemoRoom {
    fn unit(&self, serial: &str) -> Option<&RoomUnit> {
        self.units.iter().find(|u| u.unit.serial == serial)
    }

    /// `serial` is one of the room's units, still the attachment it was
    /// installed for: why not, when not.
    fn refusal(
        &self,
        hub: &HubHandle,
        serial: &str,
        dev: Option<&Arc<DeviceShared>>,
    ) -> Option<String> {
        let Some(u) = self.unit(serial) else {
            return Some(format!(
                "{serial} is not a demo unit of this hub: the simulated microphone hears only the demo hub's own demo units"
            ));
        };
        let installed = u.dev.upgrade();
        let now = hub.device_by_serial(serial);
        let same = match (&installed, &now) {
            (Some(a), Some(b)) => Arc::ptr_eq(a, b) && dev.is_none_or(|d| Arc::ptr_eq(a, d)),
            _ => false,
        };
        (!same)
            .then(|| format!("{serial} is no longer the demo unit the simulated room was made for"))
    }
}

impl HubHandle {
    /// Give this hub the demo's simulated room (WP ATDEMO): `units` are the
    /// demo's own fake units, already attached. From now on the hub offers
    /// `system_measure_demo_mic` and answers `system.measure.demo_mic` for a
    /// session whose every unit is one of these attachments. For `uconsole
    /// --demo hub --alink --with-measure-units` only: never give it a real
    /// device (it cannot be: a unit is a [`FakeRemote`]).
    ///
    /// # Errors
    /// A unit that is not attached under its serial.
    pub fn set_system_measure_demo_room(
        &self,
        units: Vec<DemoRoomUnit>,
        seed: u64,
    ) -> Result<(), String> {
        let mut room = Vec::with_capacity(units.len());
        for u in units {
            let dev = self
                .device_by_serial(&u.serial)
                .ok_or_else(|| format!("{} is not attached", u.serial))?;
            room.push(RoomUnit {
                unit: u,
                dev: Arc::downgrade(&dev),
            });
        }
        let family = self.system_measure();
        *guard(&family.demo) = Some(Arc::new(DemoRoom {
            units: room,
            streams: Mutex::new(BTreeMap::new()),
            seed,
        }));
        Ok(())
    }

    /// This hub has the demo's simulated room (`hello.caps`).
    pub(crate) fn system_measure_has_demo_room(&self) -> bool {
        self.system_measure_existing()
            .is_some_and(|f| guard(&f.demo).is_some())
    }
}

impl MeasureHub {
    /// `system.measure.demo_mic` (WP ATDEMO).
    pub(super) fn demo_mic(
        &self,
        hub: &HubHandle,
        who: &Session,
        id: Option<u64>,
        msg: &Value,
    ) -> String {
        let req: DemoMicRequest = match serde_json::from_value(msg.clone()) {
            Ok(r) => r,
            Err(e) => return err(id, "bad_request", &e.to_string(), None),
        };
        let Some(room) = guard(&self.demo).clone() else {
            return err(
                id,
                "unsupported",
                "this hub has no simulated microphone: it is the demo hub's only",
                None,
            );
        };
        if req.session.is_empty() {
            if req.units.is_empty() {
                return err(id, "bad_request", "a session or units", None);
            }
            for s in &req.units {
                if let Some(why) = room.refusal(hub, s, None) {
                    return err(id, "unsupported", &why, Some(s));
                }
            }
            return ok_of(
                id,
                &DemoMicReply {
                    ok: true,
                    room: ROOM.to_owned(),
                    ..DemoMicReply::default()
                },
            );
        }
        let s = match self.owned(who, &req.session, id) {
            Ok(s) => s,
            Err(e) => return e,
        };
        {
            let live = guard(&self.sessions);
            guard(&room.streams).retain(|k, _| live.contains_key(k));
        }
        match room.read(hub, &s, req.from) {
            Ok(r) => ok_of(id, &r),
            Err((code, why, serial)) => err(id, code, &why, serial.as_deref()),
        }
    }
}

type Refused = (&'static str, String, Option<String>);

impl DemoRoom {
    /// What session `s`'s microphone heard from capture frame `from`.
    fn read(&self, hub: &HubHandle, s: &Arc<MSession>, from: u64) -> Result<DemoMicReply, Refused> {
        let (refs, rate, aborted) = {
            let st = guard(&s.st);
            (urefs(&st), st.rate, st.aborted.clone())
        };
        if let Some(why) = aborted {
            return Err(("aborted", why, None));
        }
        let mut members = Vec::with_capacity(refs.len());
        for u in &refs {
            if let Some(why) = self.refusal(hub, &u.serial, Some(&u.dev)) {
                return Err(("unsupported", why, Some(u.serial.clone())));
            }
            if let Some(r) = self.unit(&u.serial) {
                members.push((r, u));
            }
        }
        let Some((first, fref)) = members.first() else {
            return Err(("bad_request", "a session without units".into(), None));
        };
        // The domain's T now, read through the first unit (its own numbering).
        let t_of = || {
            first
                .unit
                .remote
                .time_base_t()
                .map(|(_, t)| to_domain(t, fref.origin))
        };
        let Some(mut t_now) = t_of() else {
            return Err((
                "unsupported",
                format!("{} has no time base", fref.serial),
                Some(fref.serial.clone()),
            ));
        };
        let rate = if rate == 0 { 48_000 } else { rate };
        let lag = u64::from(rate).saturating_mul(LAG_MS) / 1000;
        let mut streams = guard(&self.streams);
        let stream = streams.entry(s.id.clone()).or_insert_with(|| Stream {
            base_t: t_now.saturating_sub(lag),
            seen: Vec::new(),
            air: Vec::new(),
            air_base: 0,
            noise: (self.seed ^ 0xA7DE_4D0E).max(1),
        });
        stream.seen.resize_with(members.len(), BTreeSet::new);
        // Every new start of every unit, rendered where it is heard; then T
        // again, so what is delivered reaches the domain's T now less the
        // microphone's latency however long the renders took (the client's
        // time pin right after this is then current), and a look that
        // renders nothing more proves every start up to that T is laid.
        for _ in 0..4 {
            if !stream.listen(&members, s.owner.id(), rate) {
                break;
            }
            if let Some(t) = t_of() {
                t_now = t;
            }
        }
        // What the microphone has heard by now.
        let heard = t_now.saturating_sub(lag).saturating_sub(stream.base_t);
        if from < stream.air_base {
            return Err((
                "bad_request",
                format!("capture frame {from} was read already"),
                None,
            ));
        }
        let to = heard.min(from.saturating_add(u64::from(MAX_FRAMES)));
        let n = usize::try_from(to.saturating_sub(from)).unwrap_or(0);
        let skip = usize::try_from(from.saturating_sub(stream.air_base)).unwrap_or(usize::MAX);
        let mut samples = Vec::with_capacity(n);
        for j in 0..n {
            let v = stream
                .air
                .get(skip.saturating_add(j))
                .copied()
                .unwrap_or(0.0);
            let noise = NOISE * (uniform(&mut stream.noise) * 2.0 - 1.0);
            let x = ((v + noise) * FULL_SCALE).clamp(-FULL_SCALE, FULL_SCALE - 1.0);
            samples.push(x as i32);
        }
        // What is read is gone.
        let done = skip.saturating_add(n).min(stream.air.len());
        stream.air.drain(..done);
        stream.air_base = from.saturating_add(n as u64);
        Ok(DemoMicReply {
            ok: true,
            room: ROOM.to_owned(),
            first_frame: from,
            rate_hz: rate,
            samples,
        })
    }
}

impl Stream {
    /// Render every new start of `members` (as session `owner`). True when
    /// one was rendered.
    fn listen(
        &mut self,
        members: &[(&RoomUnit, &super::URef)],
        owner: crate::session::SessionId,
        rate: u32,
    ) -> bool {
        let mut rendered = false;
        for (i, (r, u)) in members.iter().enumerate() {
            let starts = r.unit.remote.measure_timed_starts();
            for st in starts.into_iter().filter(|x| x.timed) {
                let Some(seen) = self.seen.get_mut(i) else {
                    break;
                };
                if !seen.insert((st.run_id, st.t)) {
                    continue;
                }
                while seen.len() > MAX_SEEN {
                    seen.pop_first();
                }
                let t = to_domain(st.t, u.origin);
                // Before this session's microphone opened: not heard (an
                // earlier session's, only marked seen).
                if t < self.base_t {
                    continue;
                }
                let io = Io {
                    dev: &u.dev,
                    session: owner,
                };
                let delays = out_delays(&io, st.config.channel_mask);
                rendered = true;
                if let Err(e) = self.play(&r.unit, t, &st.config, &delays, rate) {
                    log::warn!(
                        "demo room: {}'s start at T {t} not rendered: {e}",
                        r.unit.serial
                    );
                }
            }
        }
        rendered
    }

    /// Unit `u` started `c` at domain T `t`: render it into the air.
    fn play(
        &mut self,
        u: &DemoRoomUnit,
        t: u64,
        c: &SiggenConfig,
        out_delay_ms: &[(u8, f64)],
        rate: u32,
    ) -> Result<(), String> {
        let outs: Vec<u8> = (0..16u8).filter(|o| bit(c.channel_mask, *o)).collect();
        let walk = c.flags & SiggenConfig::FLAG_WALK != 0 || outs.len() <= 1;
        // A walk is one render; outputs played together are summed.
        let parts: Vec<u16> = if walk {
            vec![c.channel_mask]
        } else {
            outs.iter()
                .filter_map(|o| 1u16.checked_shl(u32::from(*o)))
                .collect()
        };
        for mask in parts {
            let local = render_one(u, c, mask, out_delay_ms, rate)?;
            // Capture frame of local[0]: T - MARGIN, from the session's base.
            let at = t.saturating_sub(MARGIN);
            if at < self.base_t {
                continue;
            }
            let start = at.saturating_sub(self.base_t);
            let Some(off) = start.checked_sub(self.air_base) else {
                // Partly read already: what is left of it is laid.
                let late =
                    usize::try_from(self.air_base.saturating_sub(start)).unwrap_or(usize::MAX);
                self.lay(0, local.get(late..).unwrap_or(&[]));
                continue;
            };
            let off = usize::try_from(off).map_err(|e| e.to_string())?;
            self.lay(off, &local);
        }
        Ok(())
    }

    fn lay(&mut self, off: usize, local: &[f32]) {
        let need = off.saturating_add(local.len());
        if need > MAX_AIR {
            log::warn!("demo room: the capture is not being read; a start is not heard");
            return;
        }
        if self.air.len() < need {
            self.air.resize(need, 0.0);
        }
        if let Some(dst) = self.air.get_mut(off..need) {
            for (d, s) in dst.iter_mut().zip(local) {
                *d += *s;
            }
        }
    }
}

/// What the microphone hears of `u` playing `c` on the outputs of `mask`,
/// from `MARGIN` frames before the start.
fn render_one(
    u: &DemoRoomUnit,
    c: &SiggenConfig,
    mask: u16,
    out_delay_ms: &[(u8, f64)],
    rate: u32,
) -> Result<Vec<f32>, String> {
    let e = |e: dspi_measure::types::MeasureError| e.to_string();
    let sweep = LogSweep::new(
        rate,
        &SweepConfig {
            f1_hz: c.p1,
            f2_hz: c.p2,
            duration_ms: c.duration_ms,
            gap_ms: c.gap_ms,
        },
    )
    .map_err(e)?;
    let walk = WalkSchedule::new(sweep, mask, c.repeat_count).map_err(e)?;
    let n = walk
        .outputs()
        .iter()
        .map(|o| usize::from(*o).saturating_add(1))
        .max()
        .unwrap_or(1);
    let mut sim = SimUnit::flat(&u.serial, n);
    for (o, p) in sim.outputs.iter_mut().enumerate() {
        let on = u8::try_from(o).is_ok_and(|b| bit(mask, b));
        if on {
            let delay = out_delay_ms
                .iter()
                .find(|(x, _)| usize::from(*x) == o)
                .map_or(0.0, |(_, ms)| *ms);
            p.delay_ms = acoustic_ms(u, o) + delay;
            p.gain_db = OUTPUT_GAIN_DB;
        } else {
            // An output that does not play stays silent.
            p.gain_db = -200.0;
        }
    }
    sim.chains = flat_chains(n);
    sim.playout_frames = 0.0;
    sim.link = UnitLink::Line {
        hops: 0,
        hop_error_frames: 0,
    };
    sim.level_db = c.level_db;
    sim.invert_mask = c.invert_mask;
    let d = DomainId {
        time_base: 0,
        id: 0xDE30,
    };
    let mut world = SystemWorld::free_field(vec![sim], d, 0, rate);
    world.acoustics = World::three_way_room(1).acoustics;
    world.mic = Microphone::typical();
    world.post_roll_ms = 600;
    let schedule = SystemSchedule::single(d, UnitIndex(0), MARGIN, walk).map_err(e)?;
    let r = render_system(&world, &schedule, &ProtoModel).map_err(e)?;
    Ok(r.capture.samples)
}

/// Output `o`'s distance to the microphone, ms.
fn acoustic_ms(u: &DemoRoomUnit, o: usize) -> f64 {
    match u.acoustic_ms.get(o) {
        Some(ms) => *ms,
        None => {
            let last = u.acoustic_ms.last().copied().unwrap_or(3.0);
            let more = o.saturating_sub(u.acoustic_ms.len().saturating_sub(1));
            last + 0.4 * more as f64
        }
    }
}

/// Each output of `mask`'s `out.delay` as the unit holds it, ms (0 when it
/// cannot be read).
fn out_delays(io: &Io<'_>, mask: u16) -> Vec<(u8, f64)> {
    (0..16u8)
        .filter(|o| bit(mask, *o))
        .map(|o| {
            let got = io.exec(
                Dir::Get,
                op::REQ_GET_OUTPUT_DELAY,
                u16::from(o),
                Vec::new(),
                4,
            );
            if let Err(st) = &got {
                log::warn!("demo room: output {o}'s delay not read (status {st}): heard as 0");
            }
            let ms = got
                .ok()
                .and_then(|b| b.get(..4)?.try_into().ok().map(f32::from_le_bytes))
                .filter(|v| v.is_finite() && *v >= 0.0)
                .map_or(0.0, f64::from);
            (o, ms)
        })
        .collect()
}

/// Output `o`'s bit of a generator mask.
fn bit(mask: u16, o: u8) -> bool {
    1u16.checked_shl(u32::from(o))
        .is_some_and(|m| mask & m != 0)
}

/// The next xorshift draw, 0 to 1.
fn uniform(s: &mut u64) -> f32 {
    *s ^= s.wrapping_shl(13);
    *s ^= s.wrapping_shr(7);
    *s ^= s.wrapping_shl(17);
    (*s >> 40) as f32 / (1u64 << 24) as f32
}
