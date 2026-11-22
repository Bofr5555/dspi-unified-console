//! The demo's simulated microphone (WP P5DEMO, PLAN-P5 OD-P5q): on a demo
//! connection only, a microphone in P5SIM's "three-way speaker in a room"
//! world that hears what the demo DSPi's generator plays. Turned on with
//! [`DspiCore::measure_sim_set`], it renders the generator configuration
//! each START of the measurement session staged (read back from the demo
//! device) through the world and the outputs' chains, and feeds
//! [`DspiCore::measure_capture_push`] with frame indices at the pace of the
//! core's clock seam, from [`DspiCore::measure_tick`]. A client then pushes
//! nothing: while the simulated microphone is on, its own pushes are ignored.
//! Every screen labels it [`SIM_MIC_LABEL`].
//!
//! **Never on a real device:** it turns on only over a connection of the
//! fake kind (`TransportKind::Fake`, the demo device) and turns itself off at
//! the first feed that finds another connection (or another demo serial).
//! Nothing is played and no audio device is opened anywhere (S1): the world
//! is arithmetic. Deterministic from the seed: the same generator
//! configuration at the same frame gives the same samples.
//!
//! **Feature:** the renderer is behind `measure-sim` (P5SIM's `sim` in
//! `dspi-measure`); without it the calls exist and answer
//! [`DspiError::Unsupported`], so every binding has one shape.
//!
//! **Locks:** the feed's own mutex is taken before the measurement slot and
//! `inner` (it pushes through the public calls) and only with `try_lock`
//! where a listener could re-enter, so a listener calling back never
//! deadlocks: a re-entrant tick just skips the feed.

use std::cell::Cell;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard, TryLockError};

use serde::Serialize;

use crate::{DspiCore, DspiError, Result};

/// What every screen calls the simulated microphone.
pub const SIM_MIC_LABEL: &str = "Simulated microphone";

/// The world it stands in (P5SIM's `World::three_way_room`).
pub const SIM_WORLD: &str = "three_way_room";

/// The simulated microphone's state (`measure_sim_info`, `measure_sim_set`).
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MeasureSimInfo {
    /// This build has it and the open connection is a demo device.
    pub available: bool,
    /// It is on: it answers the generator, the client pushes nothing.
    pub on: bool,
    /// What the screens call it ([`SIM_MIC_LABEL`]).
    pub label: String,
    /// The world it stands in ([`SIM_WORLD`]).
    pub world: String,
    /// The seed its world and noise are drawn from.
    pub seed: u64,
    /// Why the last generator configuration could not be rendered (it then
    /// hears only the room's noise), if one.
    pub issue: Option<String>,
}

thread_local! {
    /// The feed is pushing on this thread: its pushes are not a client's.
    static FEEDING: Cell<bool> = const { Cell::new(false) };
}

/// The simulated microphone of one core.
#[derive(Default)]
pub(crate) struct SimSlot {
    on: AtomicBool,
    seed: AtomicU64,
    feed: Mutex<Option<Feed>>,
}

impl SimSlot {
    fn lock(&self) -> MutexGuard<'_, Option<Feed>> {
        self.feed.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// The feed, unless someone (a listener re-entering) holds it.
    fn try_lock(&self) -> Option<MutexGuard<'_, Option<Feed>>> {
        match self.feed.try_lock() {
            Ok(g) => Some(g),
            Err(TryLockError::Poisoned(p)) => Some(p.into_inner()),
            Err(TryLockError::WouldBlock) => None,
        }
    }
}

#[cfg_attr(feature = "uniffi", uniffi::export)]
impl DspiCore {
    /// Turn the demo's simulated microphone on (`true`, its world drawn from
    /// `seed`) or off. On, it answers the demo DSPi's generator through
    /// P5SIM's three-way speaker in a room and feeds the measurement session
    /// from `measure_tick`: the client pushes no capture (its pushes are
    /// ignored) and labels the microphone "Simulated microphone". Off, the
    /// client's own capture is the microphone again.
    ///
    /// Errors: [`DspiError::Unsupported`] to turn it on over anything but a
    /// demo connection (a DSPi on USB, MIDI or through a hub) or in a build
    /// without it; [`DspiError::Disconnected`] with nothing open;
    /// [`DspiError::InvalidArgument`] while a measurement is running.
    pub fn measure_sim_set(&self, on: bool, seed: u64) -> Result<MeasureSimInfo> {
        if self.measure_state().active {
            return Err(DspiError::InvalidArgument {
                msg: "a measurement is running: stop it first".into(),
            });
        }
        if on {
            self.measure_sim_turn_on(seed)?;
        } else {
            self.measure_sim.on.store(false, Ordering::SeqCst);
            *self.measure_sim.lock() = None;
        }
        Ok(self.measure_sim_info())
    }

    /// The simulated microphone's state, without I/O.
    pub fn measure_sim_info(&self) -> MeasureSimInfo {
        let available =
            cfg!(any(test, feature = "measure-sim")) && self.measure_sim_demo().is_some();
        let issue = self
            .measure_sim
            .try_lock()
            .and_then(|g| g.as_ref().and_then(Feed::issue));
        MeasureSimInfo {
            available,
            on: self.measure_sim_on(),
            label: SIM_MIC_LABEL.to_owned(),
            world: SIM_WORLD.to_owned(),
            seed: self.measure_sim.seed.load(Ordering::SeqCst),
            issue,
        }
    }
}

impl DspiCore {
    /// The open connection's serial when it is the demo device (the fake
    /// kind), else `None`.
    fn measure_sim_demo(&self) -> Option<String> {
        let mut inner = self.lock();
        let conn = inner.conn().ok()?;
        (conn.caps.kind == dspi_transport::TransportKind::Fake).then(|| conn.serial.clone())
    }

    /// The simulated microphone owns the capture: a client's push is
    /// ignored (`measure_capture_push`).
    pub(crate) fn measure_sim_owns_capture(&self) -> bool {
        !FEEDING.with(Cell::get) && self.measure_sim_on()
    }

    /// It is on, and the open connection is still a demo device. One left on
    /// when the demo was replaced by a real connection is off from here on
    /// (review P5-C1: before the first tick too, so a screen never reads
    /// "on" over a real DSPi and a client's first pushes are not dropped).
    fn measure_sim_on(&self) -> bool {
        if !self.measure_sim.on.load(Ordering::SeqCst) {
            return false;
        }
        if self.measure_sim_demo().is_none() {
            self.measure_sim.on.store(false, Ordering::SeqCst);
            return false;
        }
        true
    }

    #[cfg(not(any(test, feature = "measure-sim")))]
    fn measure_sim_turn_on(&self, _seed: u64) -> Result<()> {
        Err(DspiError::Unsupported)
    }

    #[cfg(not(any(test, feature = "measure-sim")))]
    pub(crate) fn measure_sim_feed(&self) {}
}

/// Without the feature there is nothing to feed.
#[cfg(not(any(test, feature = "measure-sim")))]
pub(crate) struct Feed;

#[cfg(not(any(test, feature = "measure-sim")))]
impl Feed {
    fn issue(&self) -> Option<String> {
        None
    }
}

#[cfg(any(test, feature = "measure-sim"))]
pub(crate) use world::Feed;

// -- WP FUNC1 -- the function test's home theatre
#[cfg(feature = "sim-scenario")]
pub(crate) mod scenario;

#[cfg(any(test, feature = "measure-sim"))]
mod world {
    use std::time::Duration;

    use dspi_measure::sim::{Burst, Capture, SimFilter, Stimulus, World, apply_filters, render};
    use dspi_measure::stimulus::{LogSweep, SweepConfig, WalkSchedule, peak_linear};
    use dspi_measure::types::{ChannelChain, SectionModel};
    use dspi_transport::clock::Instant;

    use super::{FEEDING, Ordering};
    use crate::measure_doc::CoreSectionModel;
    use crate::records::SiggenConfig;
    use crate::{DspiCore, DspiError, Result};

    /// The generator's pink noise (`SIGGEN_TYPE_PINK`).
    const SIG_PINK: u8 = 3;
    /// The generator's log sweep (`SIGGEN_TYPE_LOG_SWEEP`).
    const SIG_LOG_SWEEP: u8 = 4;
    /// The most capture one feed pushes, seconds: a client that stalls
    /// catches up over several ticks, as a real microphone's backlog does.
    const MAX_FEED_S: u64 = 10;
    /// The demo room's noise floor, dBFS RMS at the microphone.
    const DEMO_NOISE_DBFS: f64 = -100.0;
    /// `SIGGEN_STOP_COMPLETED`: the run ended by itself.
    const STOP_COMPLETED: u8 = 2;
    // -- WP FUNC1 -- how long an output's own filters ring on, device frames
    const OWN_TAIL: usize = 24_000;
    /// The post-roll rendered after a run, device frames beyond its last gap.
    const TAIL_FRAMES: u64 = 4_800;

    /// A microphone in the world: what it hears, block by block. Pure and
    /// deterministic: no clock, no device.
    #[derive(Debug, Clone)]
    pub(crate) struct SimMic {
        world: World,
        seed: u64,
        noise: u64,
        renders: u64,
        seg: Option<(Capture, usize)>,
        /// The generator was stopped early: nothing it plays reaches the
        /// microphone from this capture frame on.
        cut: Option<u64>,
        issue: Option<String>,
        // -- WP FUNC1 -- each output's own filters (none but a scenario's)
        filters: Vec<Vec<SimFilter>>,
    }

    impl SimMic {
        /// A microphone in the three-way room drawn from `seed`, for a device
        /// with `outputs` outputs, capturing at `rate_hz`.
        pub(crate) fn new(seed: u64, outputs: usize, rate_hz: u32) -> SimMic {
            let mut world = World::three_way_room(seed);
            let base = world.outputs.clone();
            let n = outputs.clamp(1, dspi_measure::sim::MAX_SIM_OUTPUTS);
            world.outputs = (0..n)
                .filter_map(|i| {
                    let mut p = base.get(i.checked_rem(base.len())?)?.clone();
                    if i >= base.len() {
                        p.name = format!("out {i}");
                    }
                    Some(p)
                })
                .collect();
            // A quiet room: the spike front ends play at the start level only
            // (-60 dBFS), where the world's -85 dBFS left the sweep too
            // close to the noise for the cycle-identity search.
            world.noise_dbfs = Some(DEMO_NOISE_DBFS);
            world.clock.rate_hz = rate_hz;
            world.clock.block_frames = (rate_hz / 50).max(1);
            world.clock.first_frame = 0;
            SimMic {
                world,
                seed,
                // xorshift needs a state other than 0.
                noise: (seed ^ 0x2545_F491_4F6C_DD1D).max(1),
                renders: 0,
                seg: None,
                cut: None,
                issue: None,
                filters: Vec::new(),
            }
        }

        // -- WP FUNC1 --
        /// A microphone in a scenario's room, for one of its devices.
        #[cfg(feature = "sim-scenario")]
        fn in_scenario(serial: &str, outputs: usize, rate_hz: u32) -> Option<SimMic> {
            let room = super::scenario::room()?;
            let (world, filters) = room.world(room.device(serial)?, outputs, rate_hz);
            let seed = world.seed;
            Some(SimMic {
                world,
                seed,
                filters,
                ..SimMic::new(seed, outputs, rate_hz)
            })
        }

        /// What the next start plays, for the other devices' microphones.
        #[cfg(feature = "sim-scenario")]
        fn heard(&self) -> Vec<f32> {
            let seg = self.seg.as_ref();
            seg.map(|(c, _)| c.samples.clone()).unwrap_or_default()
        }

        /// Why the last configuration was not rendered.
        pub(crate) fn issue(&self) -> Option<String> {
            self.issue.clone()
        }

        /// A segment is playing that no stop has cut yet.
        pub(crate) fn uncut(&self) -> bool {
            self.seg.is_some() && self.cut.is_none()
        }

        /// The generator was stopped before its end around capture frame
        /// `frame`: what it would still have played is not heard after the
        /// sound already on its way (the microphone's latency) and a short
        /// fade (the generator's is 5 ms).
        pub(crate) fn stopped_at(&mut self, frame: u64) {
            let rate = f64::from(self.world.clock.rate_hz);
            let after = ((self.world.clock.latency_ms + 10.0) * rate / 1000.0).max(0.0) as u64;
            self.cut = Some(frame.saturating_add(after));
        }

        /// The block length, frames (20 ms).
        pub(crate) fn block_frames(&self) -> u64 {
            u64::from(self.world.clock.block_frames)
        }

        /// The generator plays `cfg` from capture frame `at` through
        /// `chains` (one per world output) at `device_rate_hz`: render what
        /// the microphone hears. A configuration it cannot render leaves the
        /// room's noise and an issue.
        pub(crate) fn play(
            &mut self,
            cfg: &SiggenConfig,
            chains: &[ChannelChain],
            device_rate_hz: u32,
            at: u64,
        ) {
            self.renders = self.renders.wrapping_add(1);
            let mut w = self.world.clone();
            w.clock.first_frame = at;
            w.seed = self.seed.wrapping_add(self.renders);
            self.cut = None;
            // The world has an output per device output; the chains follow.
            let model = CoreSectionModel;
            let chains: Vec<ChannelChain> = (0..w.outputs.len())
                .map(|o| {
                    let mut c = chains.get(o).cloned().unwrap_or_else(flat_chain);
                    c.peq.retain(|b| model.models(b));
                    c.crossover.retain(|b| model.models(b));
                    c
                })
                .collect();
            match self.capture_of(&w, cfg, &chains, device_rate_hz) {
                Ok(c) => {
                    self.seg = Some((c, 0));
                    self.issue = None;
                }
                Err(e) => {
                    self.seg = None;
                    self.issue = Some(e);
                }
            }
        }

        fn capture_of(
            &mut self,
            w: &World,
            cfg: &SiggenConfig,
            chains: &[ChannelChain],
            rate: u32,
        ) -> std::result::Result<Capture, String> {
            let amp = peak_linear(cfg.level_db);
            let outs: Vec<usize> = (0..w.outputs.len())
                .filter(|o| bit(cfg.channel_mask, *o))
                .collect();
            let first_out = *outs
                .first()
                .ok_or("the generator plays no output of the world")?;
            let model = CoreSectionModel;
            let err = |e: dspi_measure::types::MeasureError| e.to_string();
            match cfg.signal_type {
                SIG_PINK => {
                    let n = u64::from(cfg.duration_ms)
                        .saturating_mul(u64::from(rate))
                        .checked_div(1000)
                        .unwrap_or(0);
                    let n = usize::try_from(n.min(dspi_measure::sim::MAX_CAPTURE_FRAMES))
                        .map_err(|e| e.to_string())?;
                    let mut pink = pink_noise(n, amp / 8.0, &mut self.noise);
                    // -- WP FUNC1 --
                    if let Some(f) = self.filters.get(first_out) {
                        apply_filters(f, f64::from(rate), &mut pink);
                    }
                    let stim = Stimulus {
                        bursts: vec![Burst {
                            output: first_out,
                            start_frame: 0,
                            samples: &pink,
                            gain: 1.0,
                        }],
                        frames: (n as u64).saturating_add(u64::from(rate / 2)),
                    };
                    render(w, &stim, chains, &model, rate).map_err(err)
                }
                SIG_LOG_SWEEP => {
                    let sc = SweepConfig {
                        f1_hz: cfg.p.first().copied().unwrap_or(20.0),
                        f2_hz: cfg.p.get(1).copied().unwrap_or(20_000.0),
                        duration_ms: cfg.duration_ms,
                        gap_ms: cfg.gap_ms,
                    };
                    let sweep = LogSweep::new(rate, &sc).map_err(err)?;
                    let scaled = |first: bool| -> Vec<f32> {
                        sweep
                            .render_cycle(first)
                            .iter()
                            .map(|v| *v as f32 * amp)
                            .collect()
                    };
                    // -- WP FUNC1 -- each output's own filters over its samples
                    let own = |o: usize, first: bool| -> Vec<f32> {
                        let mut v = scaled(first);
                        if let Some(f) = self.filters.get(o).filter(|f| !f.is_empty()) {
                            v.resize(v.len().saturating_add(OWN_TAIL), 0.0);
                            apply_filters(f, f64::from(rate), &mut v);
                        }
                        v
                    };
                    let signals: Vec<(usize, Vec<f32>, Vec<f32>)> =
                        outs.iter().map(|o| (*o, own(*o, true), own(*o, false))).collect();
                    let signal = |o: usize, first: bool| -> &[f32] {
                        let s = signals.iter().find(|s| s.0 == o);
                        s.map_or(&[][..], |s| if first { &s.1 } else { &s.2 })
                    };
                    let tail = u64::from(sweep.gap_samples).saturating_add(TAIL_FRAMES);
                    if cfg.flags & crate::consts::SIGGEN_FLAG_WALK != 0 {
                        let schedule =
                            WalkSchedule::new(sweep, cfg.channel_mask, cfg.repeat).map_err(err)?;
                        let bursts = schedule
                            .cycles()
                            .iter()
                            .map(|c| Burst {
                                output: usize::from(c.output),
                                start_frame: c.start,
                                samples: signal(usize::from(c.output), c.index == 0),
                                gain: 1.0,
                            })
                            .collect();
                        let stim = Stimulus {
                            bursts,
                            frames: schedule.total_samples().saturating_add(tail),
                        };
                        render(w, &stim, chains, &model, rate).map_err(err)
                    } else {
                        // -- WP FUNC1 -- as `Stimulus::summed`, each output its own
                        let len = signals.iter().map(|s| s.1.len()).max().unwrap_or(0);
                        let stim = Stimulus {
                            bursts: outs
                                .iter()
                                .map(|o| Burst {
                                    output: *o,
                                    start_frame: 0,
                                    samples: signal(*o, true),
                                    gain: if bit(cfg.invert_mask, *o) { -1.0 } else { 1.0 },
                                })
                                .collect(),
                            frames: (len as u64).saturating_add(tail),
                        };
                        render(w, &stim, chains, &model, rate).map_err(err)
                    }
                }
                t => Err(format!(
                    "the simulated microphone renders sweeps and pink noise, not signal type {t}"
                )),
            }
        }

        /// The block that starts at capture frame `next`: the rendered
        /// segment's while it lasts, then the room's noise.
        pub(crate) fn block(&mut self, next: u64) -> (u64, Vec<f32>, bool) {
            let seg = self.seg.as_mut().and_then(|(cap, i)| {
                let b = cap.blocks.get(*i).copied()?;
                *i = i.saturating_add(1);
                Some((b.first_frame, cap.block_samples(&b).to_vec(), b.clipped))
            });
            if let Some((first, mut samples, clipped)) = seg {
                if let Some(cut) = self.cut {
                    // From the cut on, the room's noise only.
                    let keep = usize::try_from(cut.saturating_sub(first)).unwrap_or(usize::MAX);
                    if keep < samples.len() {
                        let tail = samples.len().saturating_sub(keep);
                        let noise = self.noise_block(tail);
                        samples.truncate(keep);
                        samples.extend(noise);
                        if keep == 0 {
                            self.seg = None;
                        }
                    }
                }
                return (first, samples, clipped);
            }
            self.seg = None;
            let n = usize::try_from(self.block_frames()).unwrap_or(960);
            (next, self.noise_block(n), false)
        }

        fn noise_block(&mut self, n: usize) -> Vec<f32> {
            let amp = self
                .world
                .noise_dbfs
                .map_or(0.0, |db| 10f32.powf(db as f32 / 20.0) * 3f32.sqrt());
            (0..n)
                .map(|_| amp * (uniform(&mut self.noise) * 2.0 - 1.0))
                .collect()
        }
    }

    fn flat_chain() -> ChannelChain {
        dspi_measure::sim::flat_chain()
    }

    /// Output `o`'s bit of a generator mask.
    fn bit(mask: u16, o: usize) -> bool {
        u32::try_from(o)
            .ok()
            .and_then(|b| 1u16.checked_shl(b))
            .is_some_and(|m| mask & m != 0)
    }

    /// The next xorshift draw, 0 to 1.
    fn uniform(s: &mut u64) -> f32 {
        *s ^= s.wrapping_shl(13);
        *s ^= s.wrapping_shr(7);
        *s ^= s.wrapping_shl(17);
        (*s >> 40) as f32 / (1u64 << 24) as f32
    }

    /// Pink noise of `n` samples about `amp` peak (Paul Kellet's filter).
    fn pink_noise(n: usize, amp: f32, seed: &mut u64) -> Vec<f32> {
        let mut b = [0f32; 7];
        (0..n)
            .map(|_| {
                let w = uniform(seed) * 2.0 - 1.0;
                b[0] = 0.99886 * b[0] + w * 0.055_517_9;
                b[1] = 0.99332 * b[1] + w * 0.075_075_9;
                b[2] = 0.96900 * b[2] + w * 0.153_852;
                b[3] = 0.86650 * b[3] + w * 0.310_485_6;
                b[4] = 0.55000 * b[4] + w * 0.532_952_2;
                b[5] = -0.7616 * b[5] - w * 0.016_898;
                let p = b[0] + b[1] + b[2] + b[3] + b[4] + b[5] + b[6] + w * 0.5362;
                b[6] = w * 0.115_926;
                amp * p / 3.5
            })
            .collect()
    }

    /// The feed of one core: the microphone and its pacing.
    pub(crate) struct Feed {
        /// The demo device it was turned on for.
        serial: String,
        seed: u64,
        mic: Option<SimMic>,
        /// The clock at the session's first feed.
        t0: Option<Instant>,
        /// The next capture frame.
        next: u64,
        /// STARTs rendered so far in this session.
        seen: u32,
    }

    impl Feed {
        pub(crate) fn issue(&self) -> Option<String> {
            self.mic.as_ref().and_then(SimMic::issue)
        }

        fn reset(&mut self) {
            self.mic = None;
            self.t0 = None;
            self.next = 0;
            self.seen = 0;
        }
    }

    impl DspiCore {
        pub(super) fn measure_sim_turn_on(&self, seed: u64) -> Result<()> {
            let serial = self.measure_sim_demo_checked()?;
            let mut g = self
                .measure_sim
                .try_lock()
                .ok_or_else(|| DspiError::InvalidArgument {
                    msg: "the simulated microphone is busy".into(),
                })?;
            *g = Some(Feed {
                serial,
                seed,
                mic: None,
                t0: None,
                next: 0,
                seen: 0,
            });
            self.measure_sim.seed.store(seed, Ordering::SeqCst);
            self.measure_sim.on.store(true, Ordering::SeqCst);
            Ok(())
        }

        /// The demo's serial, or why the simulated microphone cannot be on.
        fn measure_sim_demo_checked(&self) -> Result<String> {
            self.lock().conn()?;
            self.measure_sim_demo().ok_or(DspiError::Unsupported)
        }

        /// Push what the clock says the microphone has heard since the last
        /// feed (from `measure_tick`, before the session is driven).
        pub(crate) fn measure_sim_feed(&self) {
            if !self.measure_sim.on.load(Ordering::SeqCst) {
                return;
            }
            let Some(mut g) = self.measure_sim.try_lock() else {
                return;
            };
            let Some(feed) = g.as_mut() else {
                return;
            };
            // Never on a real device: still the demo it was turned on for.
            if self.measure_sim_demo().as_deref() != Some(feed.serial.as_str()) {
                self.measure_sim.on.store(false, Ordering::SeqCst);
                *g = None;
                return;
            }
            let st = self.measure_state();
            // A new session (one started and cleared between two ticks
            // included) is heard from a fresh pacing.
            let fresh = st.generator_starts < feed.seen || (st.frames == 0 && feed.next > 0);
            if !st.active || fresh {
                feed.reset();
            }
            if !st.active {
                return;
            }
            let t0 = *feed.t0.get_or_insert_with(Instant::now);
            let rate = st.capture_rate_hz.max(1);
            if feed.mic.is_none() {
                let outputs = usize::from(self.lock().state.num_output_channels);
                // -- WP FUNC1 -- in the scenario's room when there is one
                #[cfg(feature = "sim-scenario")]
                let mic = SimMic::in_scenario(&feed.serial, outputs, rate);
                #[cfg(not(feature = "sim-scenario"))]
                let mic = None;
                feed.mic =
                    Some(mic.unwrap_or_else(|| SimMic::new(feed.seed, outputs, rate)));
            }
            if st.generator_starts > feed.seen {
                feed.seen = st.generator_starts;
                self.measure_sim_render(feed);
            } else if feed.mic.as_ref().is_some_and(SimMic::uncut) {
                // A stop before the end (the session's STOP_NOW, a ramp step
                // cut short): it came after the last push, so at `next`.
                let stopped = self
                    .siggen_get_status()
                    .is_ok_and(|g| g.state == 0 && g.stop_reason != STOP_COMPLETED);
                if stopped && let Some(mic) = feed.mic.as_mut() {
                    mic.stopped_at(feed.next);
                    #[cfg(feature = "sim-scenario")] // -- WP FUNC1 --
                    if let Some(room) = super::scenario::room() {
                        room.stops(&feed.serial);
                    }
                }
            }
            // After the render (a whole run at once can take a while): the
            // generator went on playing meanwhile, and so did the room.
            let now = Instant::now();
            // Still the demo after the render (review P5-C1): nothing of the
            // simulated room is ever pushed into a session on another device.
            if self.measure_sim_demo().as_deref() != Some(feed.serial.as_str()) {
                self.measure_sim.on.store(false, Ordering::SeqCst);
                *g = None;
                return;
            }
            let Some(mic) = feed.mic.as_mut() else {
                return;
            };
            let elapsed = now.saturating_duration_since(t0);
            let due = frames_in(elapsed, rate);
            let most = feed
                .next
                .saturating_add(u64::from(rate).saturating_mul(MAX_FEED_S));
            let due = due.min(most);
            FEEDING.with(|f| f.set(true));
            while feed.next < due {
                #[cfg_attr(not(feature = "sim-scenario"), allow(unused_mut))]
                let (first, mut samples, clipped) = mic.block(feed.next);
                // -- WP FUNC1 -- one room: the other devices are heard too
                #[cfg(feature = "sim-scenario")]
                if let Some(room) = super::scenario::room() {
                    let at = t0 + Duration::from_secs_f64(first as f64 / f64::from(rate));
                    room.mix(&feed.serial, at, rate, &mut samples);
                }
                feed.next = first.saturating_add(samples.len() as u64);
                if samples.is_empty() {
                    break;
                }
                // A block is 20 ms: never over the push's bound.
                let _ = self.measure_capture_push(first, samples, rate, clipped);
            }
            FEEDING.with(|f| f.set(false));
        }

        /// The generator was started: render what it plays, from the
        /// configuration the demo device holds, through the chains the core
        /// mirrors, from the next capture frame (the session's START frame).
        fn measure_sim_render(&self, feed: &mut Feed) {
            let Some(mic) = feed.mic.as_mut() else {
                return;
            };
            let cfg = match self.siggen_get_config() {
                Ok(c) => c,
                Err(e) => {
                    mic.seg = None;
                    mic.issue = Some(format!("the generator's configuration: {e}"));
                    return;
                }
            };
            let s = self.get_state();
            let chains: Vec<ChannelChain> = (0..s.num_output_channels)
                .map(|o| crate::measure::chain_of(&s, o))
                .collect();
            let rate = self
                .fetch_stat(crate::consts::STAT_SAMPLE_RATE)
                .ok()
                .filter(|r| *r > 0)
                .unwrap_or(48_000);
            // -- WP FUNC1 --
            #[cfg(feature = "sim-scenario")]
            if let Some(room) = super::scenario::room() {
                room.started(&feed.serial, &cfg);
                mic.world.clock.latency_ms =
                    room.latency_ms(&feed.serial, mic.renders.wrapping_add(1));
            }
            mic.play(&cfg, &chains, rate, feed.next);
            #[cfg(feature = "sim-scenario")]
            if let Some(room) = super::scenario::room() {
                let r = mic.world.clock.rate_hz;
                let from = feed.t0.unwrap_or_else(Instant::now)
                    + Duration::from_secs_f64(feed.next as f64 / f64::from(r.max(1)));
                room.plays(&feed.serial, from, r, mic.heard());
            }
        }
    }

    /// Frames at `rate` in `d`, rounded down.
    fn frames_in(d: Duration, rate: u32) -> u64 {
        let ns = d
            .as_nanos()
            .saturating_mul(u128::from(rate))
            .checked_div(1_000_000_000)
            .unwrap_or(0);
        u64::try_from(ns).unwrap_or(u64::MAX)
    }
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_review_p5c1;
#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_p5demo;
