//! The spectrum analyser and the test signal generator on a simulated device
//! (WP CG22), so a demo shows the Analysis tools the firmware has.
//!
//! On with [`FakeOptions::analysis`](super::FakeOptions::analysis), which
//! [`FakeDevice::demo`](super::FakeDevice::demo) sets. Off, `0x08-0x0F` and
//! `0xA4-0xA8` stall as on firmware without them.
//!
//! **Analyser** (protocol V3, `rta.h`, `rta.c`, `vendor_commands.c`): caps, the
//! band-centre table, config SET (validated, clamped, announced with
//! `NOTIFY_EVT_RTA_CONFIG` tagged `HOST_SET`) and GET, band frames one at a time
//! (`0x0B`) and packed (`0x0F`), the bin frame read from a byte offset (`0x0C`),
//! status (not a read), and STOP, START and RESET. Off at boot; a band or bin read
//! starts it (unless `RTA_FLAG_MANUAL`) and it stops itself 5 s after the last
//! one. Bands 0-13 are the bass bank's (10-200 Hz), the rest the FFT's;
//! `first_band` is 0 and 34 bands are in use, as at the demo's 48 kHz.
//!
//! **Signal generator** (`siggen.h`, `siggen.c`): caps (the firmware's type table,
//! RP2350 budget), config SET (validated and sanitised as `siggen_stage_config`)
//! and GET, CONTROL and status, with `NOTIFY_EVT_SIGGEN_STATE` on every start and
//! stop. It never starts by itself: only `SIGGEN_CTL_START` plays, a preset load or
//! factory reset stops it, and nothing else does either.
//!
//! **Time.** Everything moves with [`crate::clock`] and is computed from the clock
//! reading of the request, so the same requests at the same times give the same
//! bytes. Nothing runs between requests: a frame, a stop or a completion is
//! worked out when the next request looks.
//!
//! Where it differs from the firmware:
//!
//! - **Levels are made up**, not transformed: a steady music-like spectrum
//!   (energy in the bass bank below 200 Hz, falling 3 dB per octave above 250 Hz,
//!   rolling off above 14 kHz), slightly lower on each later channel, with a slow
//!   drift on the average and a per-frame jitter on the instantaneous level,
//!   drawn from a hash of channel, frame and band. A running generator replaces
//!   it on the outputs it plays (at the output tap): its tone in the band and
//!   bins of its frequency, or noise spread over the bands.
//! - **Rotation is fixed at the start.** The channels that are live when the
//!   analyser starts take turns, one 2^order-sample capture each; one that goes
//!   dead later (an output disabled) keeps its turn until the next start.
//! - **Applied at once.** A SET takes effect before the transfer returns (the
//!   firmware applies it on the next main-loop pass), and a generator SET or
//!   START while running restarts at once, with no 5 ms fade.
//! - **Generator timing** follows the config (duration, sweep length, pattern
//!   period, gaps, repeats, walk) for status, completion and the sweep frequency.
//!   A channel-ID cycle is one second whatever channel it is on; a stepped sweep's
//!   frequency glides rather than steps.

use std::time::Duration;

use crate::clock::Instant;

use dspi_proto::generated::{notify as nfy, opcodes as op, rta as g, siggen as sg};
use dspi_proto::packets::{
    RtaBandFrame, RtaBinFrame, RtaBinHeader, RtaCaps, RtaConfig, RtaStatus, SiggenCapsHeader,
    SiggenConfig, SiggenParamDesc, SiggenStatus, SiggenTypeDesc,
};

use super::{O_ENABLED, OUTPUT_LEN, State};

/// The demo's sample rate (`REQ_GET_STATUS` sub-query 15 says the same).
const SAMPLE_RATE: u32 = 48_000;
/// Bands in use at 44.1 and 48 kHz (`RtaBandFrame::n_bands`).
const N_BANDS: u8 = 34;
const MAX_BANDS: usize = RtaBandFrame::MAX_BANDS;
const BASS_BANDS: u8 = g::RTA_BASS_BANDS as u8;
const LEVEL_ZERO: u8 = g::RTA_LEVEL_ZERO_DBFS as u8;
const ORDER_MIN: u8 = g::RTA_ORDER_MIN as u8;
const ORDER_MAX: u8 = g::RTA_ORDER_MAX as u8;
const IDLE_TIMEOUT: Duration = Duration::from_millis(g::RTA_IDLE_TIMEOUT_MS as u64);
/// `RTA_BIN_FRAME_MAX`: the 16-byte header, 512 bins and the tail.
const BIN_FRAME_MAX: u16 = 529;
const STATE_IDLE: u8 = g::RTA_STATE_IDLE as u8;
const STATE_CAPTURING: u8 = g::RTA_STATE_CAPTURING as u8;
const CH_NONE: u8 = g::RTA_CH_NONE as u8;
const FLAG_MANUAL: u8 = g::RTA_FLAG_MANUAL as u8;
const NEVER: u16 = RtaBandFrame::NEVER;

/// `rta_band_centre_tab` (rta_tables.h): nominal third-octave centres, Hz.
const CENTRES: [u16; MAX_BANDS] = [
    10, 13, 16, 20, 25, 32, 40, 50, 63, 80, 100, 125, 160, 200, 250, 315, 400, 500, 630, 800, 1000,
    1250, 1600, 2000, 2500, 3150, 4000, 5000, 6300, 8000, 10000, 12500, 16000, 20000, 25000, 31500,
    40000,
];

/// The analyser and the generator.
pub(super) struct Analysis {
    rta: Rta,
    siggen: Siggen,
}

impl Analysis {
    pub(super) fn new() -> Self {
        Self {
            rta: Rta::new(),
            siggen: Siggen::default(),
        }
    }
}

// ------------------------------------------------------------------ analyser

struct Rta {
    /// The applied configuration (`rta_init`'s at boot).
    cfg: RtaConfig,
    /// The engine's current or last run; `None` after a clear with nothing
    /// started since.
    run: Option<Run>,
    /// The last band or bin read (or start): the auto-off clock.
    last_read: Option<Instant>,
    /// The last `RTA_CTL_RESET_AVG`: a frame published before it reads empty.
    cleared_at: Option<Instant>,
}

#[derive(Clone, Copy)]
struct Run {
    armed: Instant,
    /// The channels taking turns, fixed at the start.
    live: u16,
    /// When the engine stopped; `None` while it runs.
    stopped: Option<Instant>,
}

impl Rta {
    fn new() -> Self {
        Self {
            cfg: RtaConfig {
                tap: g::RTA_TAP_OUTPUT as u8,
                // Every output; narrowed to the tap's width when read.
                channel_mask: u16::MAX,
                fft_order: ORDER_MAX,
                avg_ms: 250,
                peak_decay_db_s: 20,
                flags: 0,
            },
            run: None,
            last_read: None,
            cleared_at: None,
        }
    }

    fn running(&self) -> bool {
        self.run.is_some_and(|r| r.stopped.is_none())
    }

    /// Auto-off: no read for 5 s stops the engine, unless it is manual.
    fn settle(&mut self, now: Instant) {
        if self.cfg.flags & FLAG_MANUAL != 0 {
            return;
        }
        if let (Some(run), Some(read)) = (self.run.as_mut(), self.last_read)
            && run.stopped.is_none()
            && now.saturating_duration_since(read) > IDLE_TIMEOUT
        {
            run.stopped = Some(read.checked_add(IDLE_TIMEOUT).unwrap_or(now));
        }
    }

    fn stop(&mut self, now: Instant) {
        if let Some(run) = self.run.as_mut()
            && run.stopped.is_none()
        {
            run.stopped = Some(now);
        }
    }

    /// `clear_channel_state` and `arm`: a fresh run over what is live now.
    fn arm(&mut self, now: Instant, live: u16) {
        self.cleared_at = None;
        self.run = (live != 0).then_some(Run {
            armed: now,
            live,
            stopped: None,
        });
        // A start counts as a read.
        self.last_read = Some(now);
    }

    /// One 2^order capture at 48 kHz.
    fn frame_period(&self) -> Duration {
        let n = 1u64 << self.cfg.fft_order.clamp(ORDER_MIN, ORDER_MAX);
        Duration::from_micros(
            n.saturating_mul(1_000_000)
                .checked_div(u64::from(SAMPLE_RATE))
                .unwrap_or(0),
        )
    }

    /// Frames published by `run` up to `now`.
    fn published(&self, run: &Run, now: Instant) -> u64 {
        let end = run.stopped.map_or(now, |s| s.min(now));
        let period = self.frame_period().as_micros().max(1);
        let n = end
            .saturating_duration_since(run.armed)
            .as_micros()
            .checked_div(period)
            .unwrap_or(0);
        u64::try_from(n).unwrap_or(u64::MAX)
    }
}

/// Where a channel stands in a run: its frame count and the frame index and
/// publish time of its latest frame.
struct Turn {
    count: u64,
    index: u64,
    at: Instant,
}

/// The bits of `mask`, ascending.
fn channels(mask: u16) -> impl Iterator<Item = u8> {
    (0u8..16).filter(move |c| mask & (1u16 << *c) != 0)
}

impl State {
    fn tap_width(&self, tap: u8) -> u8 {
        let w = if tap == g::RTA_TAP_INPUT as u8 {
            self.num_inputs()
        } else {
            self.num_outputs()
        };
        w.min(16)
    }

    fn tap_valid_mask(&self, tap: u8) -> u16 {
        let w = u32::from(self.tap_width(tap));
        u16::try_from((1u32 << w).saturating_sub(1)).unwrap_or(u16::MAX)
    }

    /// `compute_live_mask`: the active inputs (the meter block's count), or
    /// the enabled outputs, that the config selects.
    fn rta_live_mask(&self, cfg: &RtaConfig) -> u16 {
        let live = if cfg.tap == g::RTA_TAP_INPUT as u8 {
            let n = self
                .status
                .last()
                .copied()
                .unwrap_or(0)
                .min(self.tap_width(cfg.tap));
            u16::try_from((1u32 << n).saturating_sub(1)).unwrap_or(u16::MAX)
        } else {
            (0..self.tap_width(cfg.tap))
                .filter(|o| {
                    self.locate("outputs", OUTPUT_LEN, usize::from(*o), O_ENABLED)
                        .and_then(|off| self.image.get(off))
                        .is_some_and(|b| *b != 0)
                })
                .fold(0u16, |m, o| m | (1u16 << o))
        };
        live & cfg.channel_mask & self.tap_valid_mask(cfg.tap)
    }

    fn analysis(&mut self) -> Option<&mut Analysis> {
        self.analysis.as_mut()
    }

    /// The analyser's and generator's GETs: `None` for another opcode or a
    /// device without them, `Some(None)` to stall.
    pub(super) fn get_analysis(&mut self, opcode: u8, value: u16) -> Option<Option<Vec<u8>>> {
        let now = Instant::now();
        self.analysis.as_ref()?;
        self.settle_analysis(now);
        Some(match opcode {
            op::REQ_RTA_GET_CONFIG => {
                let mut cfg = self.analysis()?.rta.cfg;
                cfg.channel_mask &= self.tap_valid_mask(cfg.tap);
                Some(cfg.encode().to_vec())
            }
            op::REQ_RTA_GET_CAPS => self.rta_caps(value),
            op::REQ_RTA_GET_BANDS => {
                self.rta_note_read(now);
                u8::try_from(value)
                    .ok()
                    .and_then(|ch| self.rta_band_frame(ch, now))
                    .map(|f| f.encode().to_vec())
            }
            op::REQ_RTA_GET_BANDS_ALL => {
                self.rta_note_read(now);
                let rta = &self.analysis.as_ref()?.rta;
                let live = rta
                    .run
                    .filter(|r| r.stopped.is_none())
                    .map_or(0, |r| r.live);
                Some(
                    channels(live)
                        .filter_map(|c| self.rta_band_frame(c, now))
                        .flat_map(|f| f.encode())
                        .collect(),
                )
            }
            op::REQ_RTA_GET_BINS => {
                self.rta_note_read(now);
                let frame = self.rta_bin_frame(now);
                frame
                    .get(usize::from(value)..)
                    .filter(|b| !b.is_empty())
                    .map(<[u8]>::to_vec)
            }
            op::REQ_RTA_GET_STATUS => Some(self.rta_status(now).encode().to_vec()),
            op::REQ_RTA_CONTROL => self.rta_control(value, now).then(|| vec![1]),
            op::REQ_SIGGEN_GET_CONFIG => Some(self.analysis()?.siggen.config().encode().to_vec()),
            op::REQ_SIGGEN_GET_STATUS => {
                let outputs = self.num_outputs();
                Some(
                    self.analysis()?
                        .siggen
                        .status(now, outputs)
                        .encode()
                        .to_vec(),
                )
            }
            op::REQ_SIGGEN_GET_CAPS => self.siggen_caps(value),
            op::REQ_SIGGEN_CONTROL => {
                let outputs = self.num_outputs();
                let a = self.analysis()?;
                let ok = a.siggen.control(value, now, outputs);
                self.flush_siggen_events();
                ok.then(|| vec![1])
            }
            _ => return None,
        })
    }

    /// The analyser's and generator's SETs: `None` for another opcode or a
    /// device without them, `Some(false)` to refuse.
    pub(super) fn set_analysis(&mut self, opcode: u8, data: &[u8]) -> Option<bool> {
        let now = Instant::now();
        self.analysis.as_ref()?;
        self.settle_analysis(now);
        Some(match opcode {
            op::REQ_RTA_SET_CONFIG => self.rta_set_config(data, now),
            op::REQ_SIGGEN_SET_CONFIG => {
                let outputs = self.num_outputs();
                let a = self.analysis()?;
                let ok = a.siggen.stage(data, now, outputs);
                self.flush_siggen_events();
                ok
            }
            _ => return None,
        })
    }

    /// A preset load or factory reset: the generator stops
    /// (`SIGGEN_STOP_PRESET`) and a running analyser restarts (`rta_restart`).
    pub(super) fn analysis_on_preset(&mut self) {
        let now = Instant::now();
        if self.analysis.is_none() {
            return;
        }
        self.settle_analysis(now);
        let cfg = match self.analysis() {
            Some(a) if a.rta.running() => Some(a.rta.cfg),
            _ => None,
        };
        let live = cfg.map(|c| self.rta_live_mask(&c));
        if let Some(a) = self.analysis() {
            if let Some(live) = live {
                a.rta.arm(now, live);
            }
            a.siggen.stop_now(now, sg::SIGGEN_STOP_PRESET as u8);
        }
        self.flush_siggen_events();
    }

    fn settle_analysis(&mut self, now: Instant) {
        if let Some(a) = self.analysis() {
            a.rta.settle(now);
            a.siggen.settle(now);
        }
        self.flush_siggen_events();
    }

    /// `NOTIFY_EVT_SIGGEN_STATE`s the generator has to send.
    fn flush_siggen_events(&mut self) {
        let events = match self.analysis() {
            Some(a) => std::mem::take(&mut a.siggen.events),
            None => return,
        };
        for [state, reason, kind, channel] in events {
            self.push(vec![
                nfy::NOTIFY_V2_VERSION as u8,
                nfy::NOTIFY_EVT_SIGGEN_STATE as u8,
                0,
                0,
                state,
                reason,
                kind,
                channel,
            ]);
        }
    }

    // ---------------------------------------------------------- RTA requests

    fn rta_caps(&self, value: u16) -> Option<Vec<u8>> {
        if value == 0 {
            return Some(
                RtaCaps {
                    input_channels: self.num_inputs(),
                    output_channels: self.num_outputs(),
                    order_min: ORDER_MIN,
                    order_max: ORDER_MAX,
                    order_default: ORDER_MAX,
                    bass_bands: BASS_BANDS,
                    max_bands: MAX_BANDS as u8,
                    level_zero: LEVEL_ZERO,
                    dynamic_range_db: 120,
                    idle_timeout_ms: g::RTA_IDLE_TIMEOUT_MS,
                    max_bin_frame: BIN_FRAME_MAX,
                    bass_dynamic_range_db: 70,
                }
                .encode()
                .to_vec(),
            );
        }
        // A chunk of 32 centres; past the table (or a set high byte) stalls.
        let first = usize::from(u8::try_from(value).ok()?.checked_sub(1)?).checked_mul(32)?;
        let chunk = CENTRES.get(first..)?.iter().take(32);
        let bytes: Vec<u8> = chunk.flat_map(|c| c.to_le_bytes()).collect();
        (!bytes.is_empty()).then_some(bytes)
    }

    fn rta_set_config(&mut self, data: &[u8], now: Instant) -> bool {
        if data.len() != RtaConfig::SIZE {
            return false;
        }
        let Ok(mut c) = RtaConfig::decode(data) else {
            return false;
        };
        let tap_ok = c.tap == g::RTA_TAP_INPUT as u8 || c.tap == g::RTA_TAP_OUTPUT as u8;
        if !tap_ok
            || !(ORDER_MIN..=ORDER_MAX).contains(&c.fft_order)
            || c.channel_mask & self.tap_valid_mask(c.tap) == 0
        {
            return false;
        }
        // `apply_config`, at once.
        c.avg_ms = c.avg_ms.min(10_000);
        c.peak_decay_db_s = c.peak_decay_db_s.min(100);
        c.channel_mask &= self.tap_valid_mask(c.tap);
        let live = self.rta_live_mask(&c);
        let Some(old) = self.analysis.as_ref().map(|a| a.rta.cfg) else {
            return false;
        };
        let restart = c.tap != old.tap
            || c.channel_mask != (old.channel_mask & self.tap_valid_mask(old.tap))
            || c.fft_order != old.fft_order;
        let Some(a) = self.analysis() else {
            return false;
        };
        a.rta.cfg = c;
        if restart {
            if a.rta.running() {
                a.rta.arm(now, live);
            } else {
                a.rta.run = None;
                a.rta.cleared_at = None;
            }
        }
        // Every applied SET is announced, even an identical one.
        let mut p = vec![
            nfy::NOTIFY_V2_VERSION as u8,
            nfy::NOTIFY_EVT_RTA_CONFIG as u8,
            0,
            0,
        ];
        p.extend_from_slice(&c.encode());
        p.push(nfy::PARAM_SRC_HOST_SET as u8);
        self.push(p);
        true
    }

    /// `rta_note_read`: keepalive, and a start unless manual.
    fn rta_note_read(&mut self, now: Instant) {
        let start = match self.analysis.as_ref() {
            Some(a) => !a.rta.running() && a.rta.cfg.flags & FLAG_MANUAL == 0,
            None => return,
        };
        let live = match self.analysis.as_ref() {
            Some(a) if start => self.rta_live_mask(&a.rta.cfg),
            _ => 0,
        };
        if let Some(a) = self.analysis() {
            if start {
                a.rta.arm(now, live);
            } else {
                a.rta.last_read = Some(now);
            }
        }
    }

    fn rta_control(&mut self, value: u16, now: Instant) -> bool {
        let live = match self.analysis.as_ref() {
            Some(a) => self.rta_live_mask(&a.rta.cfg),
            None => return false,
        };
        let Some(a) = self.analysis() else {
            return false;
        };
        match u16::from(value as u8) {
            g::RTA_CTL_STOP => a.rta.stop(now),
            g::RTA_CTL_START => {
                if !a.rta.running() {
                    a.rta.arm(now, live);
                }
            }
            g::RTA_CTL_RESET_AVG => a.rta.cleared_at = Some(now),
            _ => return false,
        }
        true
    }

    fn rta_turn(rta: &Rta, ch: u8, now: Instant) -> Option<Turn> {
        let run = rta.run.as_ref()?;
        if run.live & (1 << ch.min(15)) == 0 {
            return None;
        }
        let lanes = u64::from(run.live.count_ones()).max(1);
        let lane = u64::from((run.live & ((1u16 << ch.min(15)).saturating_sub(1))).count_ones());
        let published = rta.published(run, now);
        let count = published
            .checked_sub(lane)?
            .checked_sub(1)?
            .checked_div(lanes)?
            .saturating_add(1);
        let index = lane.saturating_add(count.saturating_sub(1).saturating_mul(lanes));
        let span = rta
            .frame_period()
            .saturating_mul(u32::try_from(index.saturating_add(1)).unwrap_or(u32::MAX));
        Some(Turn {
            count,
            index,
            at: run.armed.checked_add(span)?,
        })
    }

    /// `rta_get_band_frame`: `None` (a stall) for a channel past the tap.
    fn rta_band_frame(&self, ch: u8, now: Instant) -> Option<RtaBandFrame> {
        let a = self.analysis.as_ref()?;
        let rta = &a.rta;
        if ch >= self.tap_width(rta.cfg.tap) {
            return None;
        }
        let mut f = RtaBandFrame {
            channel: ch,
            seq: 0,
            n_bands: N_BANDS,
            age_ms: NEVER,
            avg: [0; MAX_BANDS],
            peak: [0; MAX_BANDS],
        };
        let Some(turn) = Self::rta_turn(rta, ch, now) else {
            return Some(f);
        };
        f.seq = turn.count as u8;
        if rta.cleared_at.is_some_and(|c| turn.at < c) {
            return Some(f);
        }
        let age = now.saturating_duration_since(turn.at).as_millis();
        f.age_ms = u16::try_from(age).unwrap_or(NEVER).min(NEVER - 1);
        let picture = self.picture(ch, now);
        let t = turn
            .at
            .saturating_duration_since(rta.run.as_ref()?.armed)
            .as_secs_f32();
        let smooth = rta.cfg.avg_ms > 0;
        let hold = rta.cfg.peak_decay_db_s > 0;
        for b in 0..N_BANDS {
            let base = picture.band_db(b);
            let seed = (u64::from(ch) << 40) ^ (turn.index << 8) ^ u64::from(b);
            let inst = base + 2.5 * noise(seed);
            let drift = 1.5 * (t * 1.3 + f32::from(b) * 0.7 + f32::from(ch)).sin();
            let avg = if smooth { base + drift } else { inst };
            let peak = if hold {
                inst.max(base + 4.0 + 1.5 * (t * 0.45 + f32::from(b) * 1.1).sin())
            } else {
                inst
            };
            if let (Some(x), Some(p)) = (
                f.avg.get_mut(usize::from(b)),
                f.peak.get_mut(usize::from(b)),
            ) {
                *x = level(avg);
                *p = level(peak);
            }
        }
        Some(f)
    }

    /// The published bin frame: `16 + N/2 + 1` bytes, all zero with a 0xFF
    /// tail until the first frame of this run.
    fn rta_bin_frame(&self, now: Instant) -> Vec<u8> {
        let Some(rta) = self.analysis.as_ref().map(|a| &a.rta) else {
            return Vec::new();
        };
        let order = rta.cfg.fft_order.clamp(ORDER_MIN, ORDER_MAX);
        let n_bins: u16 = 1 << order.saturating_sub(1);
        let blank = || {
            let mut b = vec![0; usize::from(n_bins).saturating_add(RtaBinHeader::SIZE + 1)];
            if let Some(t) = b.last_mut() {
                *t = 0xFF;
            }
            b
        };
        let Some(run) = rta.run.as_ref() else {
            return blank();
        };
        let published = rta.published(run, now);
        let Some(last) = published.checked_sub(1) else {
            return blank();
        };
        let lanes = u64::from(run.live.count_ones()).max(1);
        let lane = last.checked_rem(lanes).unwrap_or(0);
        let ch = channels(run.live)
            .nth(usize::try_from(lane).unwrap_or(0))
            .unwrap_or(0);
        // `next_seq`: 1, 2, .. 254, 0, 1, .., never 0xFF.
        let seq = (published % 255) as u8;
        let picture = self.picture(ch, now);
        let bin_hz = SAMPLE_RATE as f32 / f32::from(n_bins) / 2.0;
        let levels = (0..n_bins)
            .map(|k| {
                let hz = f32::from(k) * bin_hz;
                let seed = (u64::from(ch) << 40) ^ (last << 16) ^ u64::from(k);
                level(picture.bin_db(hz, bin_hz) + 3.0 * noise(seed))
            })
            .collect();
        RtaBinFrame {
            header: RtaBinHeader {
                channel: ch,
                seq,
                fft_order: order,
                sample_rate_hz: SAMPLE_RATE,
                n_bins,
            },
            levels,
        }
        .encode()
    }

    fn rta_status(&self, now: Instant) -> RtaStatus {
        let Some(rta) = self.analysis.as_ref().map(|a| &a.rta) else {
            return RtaStatus::default();
        };
        let order = rta.cfg.fft_order;
        let mut s = RtaStatus {
            state: STATE_IDLE,
            tap: rta.cfg.tap,
            channel: CH_NONE,
            idle_ms: rta.last_read.map_or(NEVER, |r| {
                u16::try_from(now.saturating_duration_since(r).as_millis())
                    .unwrap_or(NEVER)
                    .min(NEVER - 1)
            }),
            sample_rate_hz: SAMPLE_RATE,
            first_band: 0,
            ..RtaStatus::default()
        };
        if let Some(run) = rta.run.as_ref().filter(|r| r.stopped.is_none()) {
            let lanes = u64::from(run.live.count_ones()).max(1);
            let next = rta.published(run, now).checked_rem(lanes).unwrap_or(0);
            let period = rta.frame_period().as_micros().max(1);
            let fps =
                u16::try_from(1_000_000u128.checked_div(period).unwrap_or(0)).unwrap_or(u16::MAX);
            // About 1.9 ms a transform at order 10 on an RP2350.
            let frame_us: u16 = 1900 >> ORDER_MAX.saturating_sub(order);
            s.state = STATE_CAPTURING;
            s.channel = channels(run.live)
                .nth(usize::try_from(next).unwrap_or(0))
                .unwrap_or(CH_NONE);
            s.live_mask = run.live;
            s.live_count = run.live.count_ones() as u8;
            s.frames_per_s = fps;
            s.last_frame_us = frame_us;
            s.busy_us_per_s = fps.saturating_mul(frame_us);
            s.bass_busy_us_per_s = 1200u16.saturating_mul(u16::from(s.live_count));
        }
        s
    }

    /// What channel `ch` at the analyser's tap carries now.
    fn picture(&self, ch: u8, now: Instant) -> Picture {
        let Some(a) = self.analysis.as_ref() else {
            return Picture::Program { ch };
        };
        if a.rta.cfg.tap == g::RTA_TAP_OUTPUT as u8
            && let Some(p) = a.siggen.picture(ch, now, self.num_outputs())
        {
            return p;
        }
        Picture::Program { ch }
    }

    fn siggen_caps(&self, value: u16) -> Option<Vec<u8>> {
        if value == 0xFFFF {
            let outputs = self.num_outputs().min(16);
            return Some(
                SiggenCapsHeader {
                    version: sg::SIGGEN_CFG_VERSION as u8,
                    type_count: TYPE_COUNT,
                    output_channels: outputs,
                    multitone_max: MULTITONE_MAX,
                    valid_channel_mask: u16::try_from((1u32 << outputs).saturating_sub(1))
                        .unwrap_or(u16::MAX),
                }
                .encode()
                .to_vec(),
            );
        }
        type_desc(u8::try_from(value).ok()?).map(|d| d.encode().to_vec())
    }
}

/// A level byte: 0.5 dB steps from `LEVEL_ZERO`, 0 the floor.
fn level(db: f32) -> u8 {
    (f32::from(LEVEL_ZERO) + 2.0 * db).round().clamp(0.0, 255.0) as u8
}

/// A deterministic value in -1..1 from `seed` (splitmix64).
fn noise(seed: u64) -> f32 {
    let mut z = seed.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^= z >> 31;
    (z >> 40) as f32 / (1u64 << 23) as f32 - 1.0
}

/// The band whose centre is nearest `hz` on a log scale.
fn band_of(hz: f32) -> u8 {
    let mut best = (0u8, f32::MAX);
    for (b, c) in (0u8..).zip(CENTRES.iter().take(usize::from(N_BANDS))) {
        let d = (hz.max(1.0) / f32::from(*c)).log2().abs();
        if d < best.1 {
            best = (b, d);
        }
    }
    best.0
}

fn centre(b: u8) -> f32 {
    f32::from(CENTRES.get(usize::from(b)).copied().unwrap_or(1000))
}

/// What a channel carries.
enum Picture {
    /// The demo's programme: music-like, a little lower on each later channel.
    Program { ch: u8 },
    /// The generator's tones at a peak level.
    Tones { hz: [f32; 2], db: f32 },
    /// The generator's noise or patterns: `db` per band, tilted by `tilt` dB
    /// per octave from 1 kHz.
    Spread { db: f32, tilt: f32 },
}

/// Silence at the generator's outputs, away from its tones.
const RESIDUAL_DB: f32 = -110.0;

impl Picture {
    fn band_db(&self, b: u8) -> f32 {
        let f = centre(b);
        match self {
            Self::Program { ch } => {
                let mut db = -18.0 - 1.5 * f32::from(ch % 6);
                if f > 250.0 {
                    db -= 3.0 * (f / 250.0).log2();
                }
                if f > 14_000.0 {
                    db -= 12.0 * (f / 14_000.0).log2();
                }
                if f < 31.5 {
                    db -= 9.0 * (31.5 / f).log2();
                }
                db
            }
            Self::Tones { hz, db } => {
                if hz.iter().any(|h| *h > 0.0 && band_of(*h) == b) {
                    *db
                } else {
                    RESIDUAL_DB
                }
            }
            Self::Spread { db, tilt } => (db + tilt * (f / 1000.0).log2()).min(0.0),
        }
    }

    /// A bin `bin_hz` wide at `hz`: a band's power shared over its bins, or a
    /// tone in the bin it falls in.
    fn bin_db(&self, hz: f32, bin_hz: f32) -> f32 {
        if let Self::Tones { hz: tones, db } = self {
            let near = tones
                .iter()
                .filter(|t| **t > 0.0)
                .map(|t| ((t - hz) / bin_hz).abs())
                .fold(f32::MAX, f32::min);
            return if near <= 0.5 {
                *db
            } else if near <= 1.5 {
                db - 12.0
            } else {
                RESIDUAL_DB
            };
        }
        let b = band_of(hz);
        // A third of an octave is 0.23 times its centre wide.
        let share = 10.0 * (bin_hz / (0.2316 * centre(b))).log10();
        self.band_db(b) + share.min(0.0)
    }
}

// ---------------------------------------------------------- signal generator

const TYPE_COUNT: u8 = 15;
/// `SIGGEN_MULTITONE_MAX` on an RP2350.
const MULTITONE_MAX: u8 = 16;
/// `SIGGEN_FADE_MS`.
const FADE: Duration = Duration::from_millis(sg::SIGGEN_FADE_MS as u64);
/// `SiggenState` (siggen.h), an enum the generator does not lift.
const SG_IDLE: u8 = 0;
const SG_FADE_IN: u8 = 1;
const SG_RUN: u8 = 2;
const SG_GAP: u8 = 3;
const SG_FADE_OUT: u8 = 4;
/// `SiggenType` (siggen.h).
const SINE: u8 = 0;
const SQUARE: u8 = 1;
const WHITE: u8 = 2;
const PINK: u8 = 3;
const SWEEP_LOG: u8 = 4;
const SWEEP_LIN: u8 = 5;
const SWEEP_STEP: u8 = 6;
const IMPULSE: u8 = 7;
const CLICKS_ALT: u8 = 8;
const POLARITY: u8 = 9;
const TONE_BURST: u8 = 10;
const TONE_PAIR: u8 = 11;
const MULTITONE: u8 = 12;
const CHANNEL_ID: u8 = 14;

const TIMING_CONTINUOUS: u8 = sg::SIGGEN_TIMING_CONTINUOUS as u8;
const TIMING_SWEEP: u8 = sg::SIGGEN_TIMING_SWEEP as u8;
const TIMING_PATTERN: u8 = sg::SIGGEN_TIMING_PATTERN as u8;

/// `caps_types` (siggen.c): `(name, timing, params)` with params as
/// `(semantic, min, max, default)`.
type Pd = (u16, f32, f32, f32);
const NONE: Pd = (sg::SIGGEN_PARAM_UNUSED, 0.0, 0.0, 0.0);
const FREQ: u16 = sg::SIGGEN_PARAM_FREQ_HZ;
const MS: u16 = sg::SIGGEN_PARAM_MS;
const CYCLES: u16 = sg::SIGGEN_PARAM_CYCLES;
const COUNT: u16 = sg::SIGGEN_PARAM_COUNT;
const TYPES: [(&str, u8, [Pd; 4]); TYPE_COUNT as usize] = [
    (
        "sine",
        TIMING_CONTINUOUS,
        [(FREQ, 1.0, 40000.0, 1000.0), NONE, NONE, NONE],
    ),
    (
        "square",
        TIMING_CONTINUOUS,
        [(FREQ, 1.0, 40000.0, 100.0), NONE, NONE, NONE],
    ),
    ("white", TIMING_CONTINUOUS, [NONE; 4]),
    ("pink", TIMING_CONTINUOUS, [NONE; 4]),
    (
        "swp-log",
        TIMING_SWEEP,
        [
            (FREQ, 1.0, 40000.0, 20.0),
            (FREQ, 1.0, 40000.0, 20000.0),
            NONE,
            NONE,
        ],
    ),
    (
        "swp-lin",
        TIMING_SWEEP,
        [
            (FREQ, 1.0, 40000.0, 20.0),
            (FREQ, 1.0, 40000.0, 20000.0),
            NONE,
            NONE,
        ],
    ),
    (
        "swp-stp",
        TIMING_SWEEP,
        [
            (FREQ, 1.0, 40000.0, 20.0),
            (FREQ, 1.0, 40000.0, 20000.0),
            (COUNT, 1.0, 24.0, 3.0),
            (MS, 20.0, 10000.0, 250.0),
        ],
    ),
    (
        "impulse",
        TIMING_PATTERN,
        [(MS, 10.0, 60000.0, 500.0), NONE, NONE, NONE],
    ),
    (
        "clk-alt",
        TIMING_PATTERN,
        [(MS, 10.0, 60000.0, 500.0), NONE, NONE, NONE],
    ),
    (
        "polarty",
        TIMING_PATTERN,
        [
            (MS, 1.0, 100.0, 5.0),
            (MS, 10.0, 60000.0, 500.0),
            NONE,
            NONE,
        ],
    ),
    (
        "burst",
        TIMING_PATTERN,
        [
            (FREQ, 10.0, 40000.0, 1000.0),
            (CYCLES, 1.0, 1000.0, 8.0),
            (CYCLES, 0.0, 1000.0, 8.0),
            (CYCLES, 0.0, 100.0, 2.0),
        ],
    ),
    (
        "tonpair",
        TIMING_CONTINUOUS,
        [
            (FREQ, 1.0, 40000.0, 60.0),
            (FREQ, 1.0, 40000.0, 7000.0),
            (sg::SIGGEN_PARAM_RATIO, 0.1, 10.0, 4.0),
            NONE,
        ],
    ),
    (
        "multi",
        TIMING_CONTINUOUS,
        [
            (COUNT, 2.0, MULTITONE_MAX as f32, 10.0),
            (FREQ, 1.0, 40000.0, 20.0),
            (FREQ, 1.0, 40000.0, 20000.0),
            NONE,
        ],
    ),
    (
        "isp",
        TIMING_CONTINUOUS,
        [(sg::SIGGEN_PARAM_PATTERN, 0.0, 1.0, 0.0), NONE, NONE, NONE],
    ),
    (
        "chan-id",
        TIMING_PATTERN,
        [(MS, 30.0, 1000.0, 120.0), NONE, NONE, NONE],
    ),
];

fn type_desc(id: u8) -> Option<SiggenTypeDesc> {
    let (name, timing_model, params) = TYPES.get(usize::from(id))?;
    let mut d = SiggenTypeDesc {
        id,
        name: (*name).into(),
        timing_model: *timing_model,
        params: [SiggenParamDesc::default(); 4],
    };
    for (slot, (semantic, min, max, default)) in d.params.iter_mut().zip(params) {
        *slot = SiggenParamDesc {
            semantic: *semantic as u8,
            min: *min,
            max: *max,
            default: *default,
        };
    }
    Some(d)
}

fn timing(kind: u8) -> u8 {
    TYPES
        .get(usize::from(kind))
        .map_or(TIMING_CONTINUOUS, |t| t.1)
}

/// Parameter `i` of `cfg`, its type's default when 0 (as the firmware reads it).
fn param(cfg: &SiggenConfig, i: usize) -> f32 {
    let v = [cfg.p1, cfg.p2, cfg.p3, cfg.p4]
        .get(i)
        .copied()
        .unwrap_or(0.0);
    if v != 0.0 {
        return v;
    }
    TYPES
        .get(usize::from(cfg.signal_type))
        .and_then(|t| t.2.get(i))
        .map_or(0.0, |p| p.3)
}

#[derive(Default)]
struct Siggen {
    /// The applied config, once one was accepted (`cfg_valid`).
    cfg: Option<SiggenConfig>,
    /// The current or last run.
    run: Option<SgRun>,
    /// `SIGGEN_STOP_*` of the last stop.
    stop_reason: u8,
    /// `[state, reason, type, channel]` to announce.
    events: Vec<[u8; 4]>,
}

#[derive(Clone)]
struct SgRun {
    cfg: SiggenConfig,
    started: Instant,
    /// A STOP's fade-out began here.
    fading: Option<Instant>,
    /// Idle from here.
    ended: Option<Instant>,
}

impl SgRun {
    fn walking(&self) -> bool {
        self.cfg.flags & SiggenConfig::FLAG_WALK != 0 || self.cfg.signal_type == CHANNEL_ID
    }

    /// One cycle: a sweep, a pattern period, a walked output's turn. `None`
    /// for a continuous signal that is not walked.
    fn cycle(&self) -> Option<Duration> {
        let c = &self.cfg;
        let ms = |v: f32| Duration::try_from_secs_f32(v / 1000.0).unwrap_or(Duration::ZERO);
        let walk = self.walking();
        let d = match c.signal_type {
            SWEEP_LOG | SWEEP_LIN => Duration::from_millis(u64::from(c.duration_ms)),
            SWEEP_STEP => {
                let (f1, f2) = (param(c, 0), param(c, 1));
                let steps = ((f2 / f1).log2().abs() * param(c, 2)).ceil().max(0.0);
                ms((steps + 1.0) * param(c, 3))
            }
            IMPULSE | CLICKS_ALT => ms(param(c, 0)),
            POLARITY => ms(param(c, 1).max(2.0 * param(c, 0))),
            TONE_BURST => {
                let f = param(c, 0).max(1.0);
                ms((param(c, 1) + param(c, 2)) * 1000.0 / f)
            }
            CHANNEL_ID => Duration::from_secs(1),
            _ if walk => Duration::from_millis(u64::from(c.duration_ms.max(1000))),
            _ => return None,
        };
        Some(d.max(FADE.saturating_mul(2)))
    }

    fn gap(&self) -> Duration {
        Duration::from_millis(u64::from(self.cfg.gap_ms))
    }

    /// Cycles to play; 0 is until stopped.
    fn target(&self, outputs: u8) -> u32 {
        let r = u32::from(self.cfg.repeat_count);
        if self.walking() {
            r.saturating_mul(self.walk_mask(outputs).count_ones())
        } else {
            r
        }
    }

    fn walk_mask(&self, outputs: u8) -> u16 {
        let valid = u16::try_from((1u32 << outputs.min(16)).saturating_sub(1)).unwrap_or(u16::MAX);
        self.cfg.channel_mask & valid
    }

    /// When it ends by itself, if it does.
    fn natural_end(&self, outputs: u8) -> Option<Instant> {
        let len = match self.cycle() {
            None => match self.cfg.duration_ms {
                0 => return None,
                ms => Duration::from_millis(u64::from(ms)),
            },
            Some(cycle) => {
                let n = self.target(outputs);
                if n == 0 {
                    return None;
                }
                cycle
                    .saturating_mul(n)
                    .saturating_add(self.gap().saturating_mul(n.saturating_sub(1)))
            }
        };
        self.started.checked_add(len)
    }

    /// Time played up to `now`.
    fn elapsed(&self, now: Instant) -> Duration {
        let end = self.ended.map_or(now, |e| e.min(now));
        end.saturating_duration_since(self.started)
    }

    /// Cycles done and whether `now` is in a gap, with the time into the cycle.
    fn position(&self, now: Instant) -> (u32, bool, Duration) {
        let Some(cycle) = self.cycle() else {
            return (0, false, self.elapsed(now));
        };
        let period = cycle.saturating_add(self.gap()).as_micros().max(1);
        let e = self.elapsed(now).as_micros();
        let done = u32::try_from(e.checked_div(period).unwrap_or(0)).unwrap_or(u32::MAX);
        let into =
            Duration::from_micros(u64::try_from(e.checked_rem(period).unwrap_or(0)).unwrap_or(0));
        (done, into >= cycle, into)
    }

    fn active_channel(&self, now: Instant, outputs: u8) -> u8 {
        if !self.walking() || self.ended.is_some() {
            return 0xFF;
        }
        let mask = self.walk_mask(outputs);
        let n = mask.count_ones().max(1);
        let (done, _, _) = self.position(now);
        channels(mask)
            .nth(usize::try_from(done.checked_rem(n).unwrap_or(0)).unwrap_or(0))
            .unwrap_or(0xFF)
    }

    /// The sweep's frequency `into` its cycle.
    fn sweep_freq(&self, into: Duration) -> f32 {
        let c = &self.cfg;
        let (f1, f2) = (param(c, 0), param(c, 1));
        let x = match self.cycle() {
            Some(cycle) if !cycle.is_zero() => (into.as_secs_f32() / cycle.as_secs_f32()).min(1.0),
            _ => 0.0,
        };
        match c.signal_type {
            SWEEP_LIN => f1 + (f2 - f1) * x,
            _ => f1 * (f2 / f1).powf(x),
        }
    }
}

impl Siggen {
    fn config(&self) -> SiggenConfig {
        self.cfg.clone().unwrap_or_default()
    }

    fn running(&self) -> bool {
        self.run.as_ref().is_some_and(|r| r.ended.is_none())
    }

    fn announce(&mut self, state: u8, reason: u8, kind: u8, channel: u8) {
        self.events.push([state, reason, kind, channel]);
    }

    /// Completion, and a fade-out ending.
    fn settle(&mut self, now: Instant) {
        // Every output there could be; the real count only narrows a walk.
        self.settle_with(now, 16);
    }

    fn settle_with(&mut self, now: Instant, outputs: u8) {
        let Some(run) = self.run.as_mut().filter(|r| r.ended.is_none()) else {
            return;
        };
        let natural = run.natural_end(outputs);
        let faded = run.fading.and_then(|f| f.checked_add(FADE));
        let (at, reason) = match (natural, faded) {
            (Some(n), Some(f)) if n < f => (n, sg::SIGGEN_STOP_COMPLETED),
            (_, Some(f)) => (f, sg::SIGGEN_STOP_HOST),
            (Some(n), None) => (n, sg::SIGGEN_STOP_COMPLETED),
            (None, None) => return,
        };
        if at > now {
            return;
        }
        run.ended = Some(at);
        let kind = run.cfg.signal_type;
        self.stop_reason = reason as u8;
        self.announce(SG_IDLE, reason as u8, kind, 0xFF);
    }

    fn start(&mut self, cfg: SiggenConfig, now: Instant, outputs: u8) {
        let run = SgRun {
            cfg,
            started: now,
            fading: None,
            ended: None,
        };
        let kind = run.cfg.signal_type;
        let channel = run.active_channel(now, outputs);
        self.run = Some(run);
        self.stop_reason = sg::SIGGEN_STOP_NONE as u8;
        self.announce(SG_RUN, sg::SIGGEN_STOP_NONE as u8, kind, channel);
    }

    /// Ends the run at once with `reason`, announcing it.
    fn stop_now(&mut self, now: Instant, reason: u8) {
        let Some(run) = self.run.as_mut().filter(|r| r.ended.is_none()) else {
            return;
        };
        run.ended = Some(now);
        let kind = run.cfg.signal_type;
        self.stop_reason = reason;
        self.announce(SG_IDLE, reason, kind, 0xFF);
    }

    /// `siggen_stage_config`: validated and sanitised; a running generator
    /// restarts with it. Never starts an idle one.
    fn stage(&mut self, data: &[u8], now: Instant, outputs: u8) -> bool {
        let Ok(mut c) = SiggenConfig::decode(data) else {
            return false;
        };
        let valid = u16::try_from((1u32 << outputs.min(16)).saturating_sub(1)).unwrap_or(u16::MAX);
        if c.version != sg::SIGGEN_CFG_VERSION as u8
            || c.signal_type >= TYPE_COUNT
            || c.channel_mask & valid == 0
        {
            return false;
        }
        c.channel_mask &= valid;
        c.invert_mask &= c.channel_mask;
        if !(-120.0..=0.0).contains(&c.level_db) {
            c.level_db = if c.level_db > 0.0 { 0.0 } else { -120.0 };
        }
        let descs = TYPES
            .get(usize::from(c.signal_type))
            .map_or([NONE; 4], |t| t.2);
        for (v, (semantic, min, max, _)) in [&mut c.p1, &mut c.p2, &mut c.p3, &mut c.p4]
            .into_iter()
            .zip(descs)
        {
            if semantic == sg::SIGGEN_PARAM_UNUSED || v.is_nan() {
                *v = 0.0;
            } else if *v != 0.0 {
                *v = v.clamp(min, max);
            }
        }
        if timing(c.signal_type) == TIMING_SWEEP && c.duration_ms == 0 {
            return false;
        }
        if self.running() {
            self.stop_now(now, sg::SIGGEN_STOP_RECONFIG as u8);
            self.start(c.clone(), now, outputs);
        }
        self.cfg = Some(c);
        true
    }

    /// `siggen_control`.
    fn control(&mut self, value: u16, now: Instant, outputs: u8) -> bool {
        match u16::from(value as u8) {
            sg::SIGGEN_CTL_START => {
                let Some(cfg) = self.cfg.clone() else {
                    return false;
                };
                if self.running() {
                    self.stop_now(now, sg::SIGGEN_STOP_RECONFIG as u8);
                }
                self.start(cfg, now, outputs);
            }
            sg::SIGGEN_CTL_STOP => {
                let in_gap = self
                    .run
                    .as_ref()
                    .filter(|r| r.ended.is_none())
                    .map(|r| r.position(now).1);
                match in_gap {
                    // The gap is silent: no fade.
                    Some(true) => self.stop_now(now, sg::SIGGEN_STOP_HOST as u8),
                    Some(false) => {
                        if let Some(run) = self.run.as_mut()
                            && run.fading.is_none()
                        {
                            run.fading = Some(now);
                        }
                    }
                    None => {}
                }
            }
            sg::SIGGEN_CTL_STOP_NOW => self.stop_now(now, sg::SIGGEN_STOP_HOST as u8),
            _ => return false,
        }
        true
    }

    fn status(&mut self, now: Instant, outputs: u8) -> SiggenStatus {
        self.settle_with(now, outputs);
        let mut s = SiggenStatus {
            stop_reason: self.stop_reason,
            signal_type: self.cfg.as_ref().map_or(0, |c| c.signal_type),
            ..SiggenStatus::default()
        };
        let Some(run) = self.run.as_ref() else {
            return s;
        };
        let (done, in_gap, into) = run.position(now);
        s.elapsed_ms = u32::try_from(run.elapsed(now).as_millis()).unwrap_or(u32::MAX);
        s.cycles_done = u16::try_from(done).unwrap_or(u16::MAX);
        if run.ended.is_some() {
            return s;
        }
        s.state = if run.fading.is_some() {
            SG_FADE_OUT
        } else if in_gap {
            SG_GAP
        } else if run.elapsed(now) < FADE && timing(run.cfg.signal_type) != TIMING_PATTERN {
            SG_FADE_IN
        } else {
            SG_RUN
        };
        s.signal_type = run.cfg.signal_type;
        s.active_channel = run.active_channel(now, outputs);
        if timing(run.cfg.signal_type) == TIMING_SWEEP && !in_gap {
            s.current_freq = run.sweep_freq(into);
        }
        s
    }

    /// What output `ch` plays now, if the generator has it.
    fn picture(&self, ch: u8, now: Instant, outputs: u8) -> Option<Picture> {
        let run = self.run.as_ref().filter(|r| r.ended.is_none())?;
        let c = &run.cfg;
        let on = if run.walking() {
            run.active_channel(now, outputs) == ch
        } else {
            c.channel_mask & (1 << ch.min(15)) != 0
        };
        let (_, in_gap, into) = run.position(now);
        if !on || in_gap {
            return None;
        }
        let db = c.level_db;
        Some(match c.signal_type {
            SINE | SQUARE | TONE_BURST => Picture::Tones {
                hz: [param(c, 0), 0.0],
                db,
            },
            SWEEP_LOG | SWEEP_LIN | SWEEP_STEP => Picture::Tones {
                hz: [run.sweep_freq(into), 0.0],
                db,
            },
            TONE_PAIR => Picture::Tones {
                hz: [param(c, 0), param(c, 1)],
                db: db - 3.0,
            },
            WHITE => Picture::Spread {
                db: db - 18.0,
                tilt: 3.0,
            },
            PINK => Picture::Spread {
                db: db - 15.0,
                tilt: 0.0,
            },
            MULTITONE => Picture::Spread {
                db: db - 10.0,
                tilt: 0.0,
            },
            _ => Picture::Spread {
                db: db - 25.0,
                tilt: 0.0,
            },
        })
    }
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects, clippy::indexing_slicing)]
mod tests_wp_cg22 {
    use super::super::{Capture, FakeDevice, FakeOptions};
    use super::*;
    use crate::{Transport, TransportError};

    fn lock(d: &FakeDevice) -> std::sync::MutexGuard<'_, State> {
        d.shared.lock()
    }

    fn cfg(tap: u8, mask: u16) -> RtaConfig {
        RtaConfig {
            tap,
            channel_mask: mask,
            fft_order: 10,
            avg_ms: 300,
            peak_decay_db_s: 12,
            flags: 0,
        }
    }

    #[test]
    fn wp_cg22_caps_and_centres_are_v3s() {
        let mut d = FakeDevice::demo();
        let caps = RtaCaps::decode(&d.control_in(op::REQ_RTA_GET_CAPS, 0, 16).unwrap()).unwrap();
        assert_eq!(
            (caps.bass_bands, caps.max_bands, caps.level_zero),
            (14, 37, 243)
        );
        assert_eq!(
            (caps.order_min, caps.order_max, caps.max_bin_frame),
            (8, 10, 529)
        );
        let c1 = d.control_in(op::REQ_RTA_GET_CAPS, 1, 64).unwrap();
        let c2 = d.control_in_upto(op::REQ_RTA_GET_CAPS, 2, 64).unwrap();
        assert_eq!(
            &c1[26..28],
            &200u16.to_le_bytes(),
            "band 13 is 200 Hz, the bass bank's last"
        );
        assert_eq!(c2.len(), 10, "five centres in the second chunk");
        assert!(d.control_in(op::REQ_RTA_GET_CAPS, 3, 64).is_err());
        assert!(d.control_in(op::REQ_RTA_GET_CAPS, 0x101, 64).is_err());
    }

    #[test]
    fn wp_cg22_off_at_boot_a_read_starts_it_and_it_stops_itself() {
        let d = FakeDevice::demo();
        let t0 = Instant::now();
        let mut s = lock(&d);
        assert_eq!(s.rta_status(t0).state, STATE_IDLE);
        assert_eq!(s.rta_status(t0).idle_ms, NEVER);
        s.rta_note_read(t0);
        let st = s.rta_status(t0 + Duration::from_millis(100));
        assert_eq!(st.state, STATE_CAPTURING);
        assert_ne!(st.live_mask, 0, "the demo has enabled outputs");
        assert_eq!(
            (st.sample_rate_hz, st.first_band, st.frames_per_s),
            (48_000, 0, 46)
        );
        s.settle_analysis(t0 + Duration::from_millis(5_001));
        assert_eq!(
            s.rta_status(t0 + Duration::from_millis(5_001)).state,
            STATE_IDLE
        );
        // MANUAL: no start from a read, no auto-off.
        let manual = RtaConfig {
            flags: FLAG_MANUAL,
            ..cfg(1, 0b11)
        };
        assert!(s.rta_set_config(&manual.encode(), t0));
        s.rta_note_read(t0);
        assert_eq!(s.rta_status(t0).state, STATE_IDLE);
        assert!(s.rta_control(g::RTA_CTL_START, t0));
        s.settle_analysis(t0 + Duration::from_secs(60));
        assert_eq!(
            s.rta_status(t0 + Duration::from_secs(60)).state,
            STATE_CAPTURING
        );
        assert!(s.rta_control(0, t0 + Duration::from_secs(61)));
        assert_eq!(s.rta_status(t0 + Duration::from_secs(61)).state, STATE_IDLE);
        assert!(!s.rta_control(9, t0), "unknown action stalls");
    }

    #[test]
    fn wp_cg22_frames_are_deterministic_and_the_bass_bank_has_energy() {
        let (a, b) = (FakeDevice::demo(), FakeDevice::demo());
        let t0 = Instant::now();
        let (mut sa, mut sb) = (lock(&a), lock(&b));
        for s in [&mut sa, &mut sb] {
            // The demo's live pair, outputs 0 and 1.
            assert!(s.rta_set_config(&cfg(1, 0b011).encode(), t0));
            s.rta_note_read(t0);
        }
        let t = t0 + Duration::from_millis(500);
        let (fa, fb) = (
            sa.rta_band_frame(0, t).unwrap(),
            sb.rta_band_frame(0, t).unwrap(),
        );
        assert_eq!(fa, fb, "the same clock gives the same frame");
        assert_eq!(sa.rta_bin_frame(t), sb.rta_bin_frame(t));
        assert_eq!(fa.n_bands, 34);
        assert!(fa.age_ms < 50, "{}", fa.age_ms);
        // About 23 frames in 500 ms over two channels.
        assert!((10..=13).contains(&fa.seq), "{}", fa.seq);
        let db = |l: u8| (f32::from(l) - 243.0) / 2.0;
        let bass: Vec<f32> = fa.avg[..14].iter().map(|l| db(*l)).collect();
        assert!(
            bass[6..14].iter().all(|d| (-26.0..-10.0).contains(d)),
            "{bass:?}"
        );
        assert!(db(fa.avg[30]) < bass[10] - 10.0, "falls towards the top");
        assert!(fa.peak[..34].iter().zip(&fa.avg).all(|(p, a)| p >= a));
        assert_eq!(fa.avg[34..], [0; 3], "unused bands read the floor");
        // Later frames move.
        let later = sa
            .rta_band_frame(0, t + Duration::from_millis(400))
            .unwrap();
        assert_ne!(later.avg, fa.avg);
        // Channel 2 is not selected: never a frame.
        assert_eq!(sa.rta_band_frame(2, t).unwrap().age_ms, NEVER);
        // RESET_AVG empties every channel until its next frame.
        assert!(sa.rta_control(2, t));
        assert_eq!(sa.rta_band_frame(0, t).unwrap().age_ms, NEVER);
        assert_ne!(
            sa.rta_band_frame(0, t + Duration::from_millis(100))
                .unwrap()
                .age_ms,
            NEVER
        );
    }

    #[test]
    fn wp_cg22_bins_are_whole_frames_with_a_matching_tail() {
        let mut d = FakeDevice::demo();
        let blank = d.control_in(op::REQ_RTA_GET_BINS, 0, 529).unwrap();
        assert_eq!(blank.len(), 529);
        assert_eq!(blank[528], 0xFF, "no frame yet");
        std::thread::sleep(Duration::from_millis(60));
        let b = d.control_in(op::REQ_RTA_GET_BINS, 0, 529).unwrap();
        let f = RtaBinFrame::decode(&b).unwrap();
        assert_eq!(
            (f.header.n_bins, f.header.fft_order, f.header.sample_rate_hz),
            (512, 10, 48_000)
        );
        assert!(
            d.control_in(op::REQ_RTA_GET_BINS, 529, 16).is_err(),
            "past the end stalls"
        );
        assert_eq!(
            d.control_in_upto(op::REQ_RTA_GET_BINS, 520, 64)
                .unwrap()
                .len(),
            9
        );
    }

    #[test]
    fn wp_cg22_set_config_is_validated_clamped_and_echoed() {
        let mut d = FakeDevice::demo();
        let mut n = d.notifications().unwrap();
        let mut bad = cfg(1, 0).encode();
        assert!(
            d.control_out(op::REQ_RTA_SET_CONFIG, 0, &bad).is_err(),
            "empty mask"
        );
        bad = cfg(2, 1).encode();
        assert!(
            d.control_out(op::REQ_RTA_SET_CONFIG, 0, &bad).is_err(),
            "bad tap"
        );
        bad = RtaConfig {
            fft_order: 11,
            ..cfg(1, 1)
        }
        .encode();
        assert!(
            d.control_out(op::REQ_RTA_SET_CONFIG, 0, &bad).is_err(),
            "bad order"
        );
        assert!(
            d.control_out(op::REQ_RTA_SET_CONFIG, 0, &cfg(1, 1).encode()[..11])
                .is_err()
        );
        let wide = RtaConfig {
            avg_ms: 60_000,
            peak_decay_db_s: 200,
            ..cfg(0, 0xFFFF)
        };
        d.control_out(op::REQ_RTA_SET_CONFIG, 0, &wide.encode())
            .unwrap();
        let back =
            RtaConfig::decode(&d.control_in(op::REQ_RTA_GET_CONFIG, 0, 12).unwrap()).unwrap();
        assert_eq!((back.avg_ms, back.peak_decay_db_s), (10_000, 100));
        assert_eq!(back.channel_mask, 0xFF, "the demo's eight inputs");
        let p = n.read(Duration::from_millis(100)).unwrap();
        assert_eq!(p.len(), 17);
        assert_eq!(p[1], nfy::NOTIFY_EVT_RTA_CONFIG as u8);
        assert_eq!(&p[4..16], &back.encode());
        assert_eq!(p[16], nfy::PARAM_SRC_HOST_SET as u8);
    }

    #[test]
    fn wp_cg22_bands_all_is_packed_frames_of_the_live_channels() {
        let mut d = FakeDevice::demo();
        d.control_out(op::REQ_RTA_SET_CONFIG, 0, &cfg(1, 0b0110).encode())
            .unwrap();
        let first = d
            .control_in_upto(op::REQ_RTA_GET_BANDS_ALL, 0, 16 * 82)
            .unwrap();
        let frames = RtaBandFrame::decode_all(&first).unwrap();
        let live = lock(&d).rta_status(Instant::now()).live_mask;
        assert_eq!(live & !0b0110, 0);
        assert_eq!(frames.len(), live.count_ones() as usize);
        assert!(
            d.control_in(op::REQ_RTA_GET_BANDS, 9, 82).is_err(),
            "past the outputs"
        );
        assert!(d.control_in(op::REQ_RTA_GET_BANDS, 0x100, 82).is_err());
    }

    #[test]
    fn wp_cg22_off_stalls_as_firmware_without_them() {
        let mut d = FakeDevice::new(Capture::demo(), FakeOptions::default());
        for opcode in [
            op::REQ_RTA_GET_CAPS,
            op::REQ_SIGGEN_GET_STATUS,
            op::REQ_SIGGEN_GET_CAPS,
        ] {
            assert!(matches!(
                d.control_in(opcode, 0, 8),
                Err(TransportError::Stalled { .. })
            ));
        }
    }

    // ---------------------------------------------------------- generator

    fn sine(mask: u16) -> SiggenConfig {
        SiggenConfig {
            signal_type: SINE,
            channel_mask: mask,
            level_db: -20.0,
            p1: 1000.0,
            ..SiggenConfig::default()
        }
    }

    /// The next `NOTIFY_EVT_SIGGEN_STATE`, skipping other events.
    fn next_siggen(n: &mut Box<dyn crate::NotificationSource>) -> Vec<u8> {
        loop {
            let p = n.read(Duration::from_millis(100)).unwrap();
            assert!(!p.is_empty(), "no generator event");
            if p[1] == nfy::NOTIFY_EVT_SIGGEN_STATE as u8 {
                return p;
            }
        }
    }

    fn status(d: &mut FakeDevice) -> SiggenStatus {
        SiggenStatus::decode(&d.control_in(op::REQ_SIGGEN_GET_STATUS, 0, 16).unwrap()).unwrap()
    }

    #[test]
    fn wp_cg22_siggen_caps_follow_the_firmware_table() {
        let mut d = FakeDevice::demo();
        let h =
            SiggenCapsHeader::decode(&d.control_in(op::REQ_SIGGEN_GET_CAPS, 0xFFFF, 8).unwrap())
                .unwrap();
        assert_eq!(
            (h.type_count, h.output_channels, h.multitone_max),
            (15, 9, 16)
        );
        assert_eq!(h.valid_channel_mask, 0x1FF);
        let step =
            SiggenTypeDesc::decode(&d.control_in(op::REQ_SIGGEN_GET_CAPS, 6, 62).unwrap()).unwrap();
        assert_eq!((step.name.as_str(), step.timing_model), ("swp-stp", 1));
        assert_eq!(step.params[3].max, 10_000.0);
        assert!(d.control_in(op::REQ_SIGGEN_GET_CAPS, 15, 62).is_err());
        assert!(d.control_in(op::REQ_SIGGEN_GET_CAPS, 0x100, 62).is_err());
    }

    #[test]
    fn wp_cg22_siggen_never_starts_by_itself() {
        let mut d = FakeDevice::demo();
        assert_eq!(status(&mut d).state, SG_IDLE);
        let none =
            SiggenConfig::decode(&d.control_in(op::REQ_SIGGEN_GET_CONFIG, 0, 36).unwrap()).unwrap();
        assert_eq!(none, SiggenConfig::default(), "no config yet");
        assert!(
            d.control_in(op::REQ_SIGGEN_CONTROL, 1, 1).is_err(),
            "START without a config stalls"
        );
        d.control_out(op::REQ_SIGGEN_SET_CONFIG, 0, &sine(0b11).encode())
            .unwrap();
        std::thread::sleep(Duration::from_millis(20));
        assert_eq!(status(&mut d).state, SG_IDLE, "a SET only stages");
        // Nor does a read of anything else, or the analyser.
        let _ = d.control_in(op::REQ_RTA_GET_BANDS_ALL, 0, 16 * 82);
        assert_eq!(status(&mut d).state, SG_IDLE);
    }

    #[test]
    fn wp_cg22_siggen_set_is_sanitised_as_the_firmware_does() {
        let mut d = FakeDevice::demo();
        let refused = [
            SiggenConfig {
                version: 2,
                ..sine(1)
            },
            SiggenConfig {
                signal_type: 15,
                ..sine(1)
            },
            sine(0),
            sine(0xFE00),
            SiggenConfig {
                signal_type: SWEEP_LOG,
                duration_ms: 0,
                ..sine(1)
            },
        ];
        for c in refused {
            assert!(
                d.control_out(op::REQ_SIGGEN_SET_CONFIG, 0, &c.encode())
                    .is_err(),
                "{c:?}"
            );
        }
        let wild = SiggenConfig {
            channel_mask: 0xFFFF,
            invert_mask: 0xFFFF,
            level_db: f32::NAN,
            p1: 90_000.0,
            p2: 5.0,
            ..sine(1)
        };
        d.control_out(op::REQ_SIGGEN_SET_CONFIG, 0, &wild.encode())
            .unwrap();
        let c =
            SiggenConfig::decode(&d.control_in(op::REQ_SIGGEN_GET_CONFIG, 0, 36).unwrap()).unwrap();
        assert_eq!((c.channel_mask, c.invert_mask), (0x1FF, 0x1FF));
        assert_eq!((c.level_db, c.p1, c.p2), (-120.0, 40_000.0, 0.0));
    }

    #[test]
    fn wp_cg22_siggen_plays_stops_and_announces() {
        let mut d = FakeDevice::demo();
        let mut n = d.notifications().unwrap();
        d.control_out(op::REQ_SIGGEN_SET_CONFIG, 0, &sine(0b1).encode())
            .unwrap();
        assert_eq!(d.control_in(op::REQ_SIGGEN_CONTROL, 1, 1).unwrap(), [1]);
        let p = next_siggen(&mut n);
        assert_eq!(&p[4..], &[SG_RUN, 0, SINE, 0xFF]);
        std::thread::sleep(Duration::from_millis(10));
        let s = status(&mut d);
        assert_eq!((s.state, s.signal_type), (SG_RUN, SINE));
        // The analyser sees the tone at output 0, in the 1 kHz band.
        let t = Instant::now();
        {
            let mut st = lock(&d);
            assert!(st.rta_set_config(&cfg(1, 0b11).encode(), t));
            st.rta_note_read(t);
            let f = st
                .rta_band_frame(0, t + Duration::from_millis(100))
                .unwrap();
            let db = |l: u8| (f32::from(l) - 243.0) / 2.0;
            assert!((db(f.avg[20]) + 20.0).abs() < 2.0, "{}", db(f.avg[20]));
            assert!(db(f.avg[10]) < -100.0);
            let other = st
                .rta_band_frame(1, t + Duration::from_millis(100))
                .unwrap();
            assert!(db(other.avg[10]) > -40.0, "output 1 plays the programme");
        }
        // STOP fades out, then idles with HOST.
        d.control_in(op::REQ_SIGGEN_CONTROL, 0, 1).unwrap();
        assert_eq!(status(&mut d).state, SG_FADE_OUT);
        std::thread::sleep(Duration::from_millis(10));
        let s = status(&mut d);
        assert_eq!(
            (s.state, s.stop_reason),
            (SG_IDLE, sg::SIGGEN_STOP_HOST as u8)
        );
        let p = next_siggen(&mut n);
        assert_eq!(&p[4..], &[SG_IDLE, sg::SIGGEN_STOP_HOST as u8, SINE, 0xFF]);
        assert!(d.control_in(op::REQ_SIGGEN_CONTROL, 7, 1).is_err());
    }

    #[test]
    fn wp_cg22_siggen_timing_under_the_clock() {
        let t0 = Instant::now();
        let mut g = Siggen::default();
        let sweep = SiggenConfig {
            signal_type: SWEEP_LOG,
            channel_mask: 1,
            duration_ms: 1000,
            repeat_count: 2,
            gap_ms: 500,
            p1: 20.0,
            p2: 20_000.0,
            ..SiggenConfig::default()
        };
        assert!(g.stage(&sweep.encode(), t0, 9));
        assert!(g.control(1, t0, 9));
        let at = |ms: u64| t0 + Duration::from_millis(ms);
        let s = g.status(at(500), 9);
        assert_eq!(s.state, SG_RUN);
        assert!((s.current_freq - 632.5).abs() < 1.0, "{}", s.current_freq);
        assert_eq!(g.status(at(1200), 9).state, SG_GAP);
        let s = g.status(at(1600), 9);
        assert_eq!((s.state, s.cycles_done), (SG_RUN, 1));
        let s = g.status(at(2600), 9);
        assert_eq!((s.state, s.stop_reason, s.elapsed_ms), (SG_IDLE, 2, 2500));
        assert_eq!(g.events.last(), Some(&[SG_IDLE, 2, SWEEP_LOG, 0xFF]));
        // A timed sine completes; a walked one moves output by output.
        let timed = SiggenConfig {
            duration_ms: 300,
            ..sine(1)
        };
        assert!(g.stage(&timed.encode(), t0, 9));
        assert!(g.control(1, t0, 9));
        assert_eq!(g.status(at(2), 9).state, SG_FADE_IN);
        assert_eq!(g.status(at(301), 9).state, SG_IDLE);
        let walk = SiggenConfig {
            flags: SiggenConfig::FLAG_WALK,
            duration_ms: 1000,
            ..sine(0b1010)
        };
        assert!(g.stage(&walk.encode(), t0, 9));
        assert!(g.control(1, t0, 9));
        assert_eq!(g.status(at(100), 9).active_channel, 1);
        assert_eq!(g.status(at(1100), 9).active_channel, 3);
        assert_eq!(g.status(at(2100), 9).active_channel, 1);
    }

    #[test]
    fn wp_cg22_a_preset_load_stops_the_generator() {
        let mut d = FakeDevice::demo();
        d.control_out(op::REQ_SIGGEN_SET_CONFIG, 0, &sine(1).encode())
            .unwrap();
        d.control_in(op::REQ_SIGGEN_CONTROL, 1, 1).unwrap();
        d.control_in(op::REQ_PRESET_LOAD, 0, 1).unwrap();
        let s = status(&mut d);
        assert_eq!(
            (s.state, s.stop_reason),
            (SG_IDLE, sg::SIGGEN_STOP_PRESET as u8)
        );
    }
}
