//! Spectrum analyser (RTA, firmware protocol V3).
//!
//! Owned by WP C9 (docs/core-merge/PLAN.md).
//!
//! The analyser runs on the device and is polled over EP0; the host half is
//! the session's [`RtaEngine`] (configuration push and confirm, frame and bin
//! reads, bin averaging, staleness, STOP when nobody watches). This module
//! drives that engine from a worker thread and turns its state into the old
//! desktop core's records ([`RtaCapsInfo`], [`RtaSnapshot`]):
//!
//! - [`DspiCore::rta_start`] spawns the worker for the open connection (on
//!   compatible firmware only: a writable connection, PLAN §1.5). Its first
//!   poll reads the caps and the band-centre table.
//! - Every [`TICK`] the worker takes the `Inner` lock, runs one
//!   [`RtaEngine::tick`] on the connection's transport (through
//!   `Inner::writable_conn`: the analyser's configuration push and STOP are
//!   writes), rebuilds the snapshot, and drops the lock. Holding the lock for
//!   the poll also serialises it against chunked bulk transfers, which any
//!   other request would abort on the device.
//! - Over DSPi Link the poll runs with the lock released, through the
//!   connection's [`crate::SideChannel`]: a hub has no chunked bulk to
//!   protect, and a round trip (or a stalled network) must not stall the
//!   core. The engine is out of `Inner` for that time. Band frames then come
//!   from the hub's `poll.subscribe` feed rather than from tunnelled reads
//!   (WP C15; `link.rs`).
//! - When the caps, the picture or the telemetry changed, the worker calls
//!   the [`RtaListener`] **after** dropping the lock (PLAN §1.2).
//! - The all-channel band read (`0x0F`) is USB-only on the firmware: a
//!   bridge behind DSPi Link answers it ERROR (a STALL here). The worker
//!   then reads the channels one by one with `0x0B` ([`BandsFallback`]); on
//!   a non-USB link the refusal is remembered for the connection.
//! - C1's close and displace paths take the worker out under the lock
//!   ([`take_worker`], which also forgets the device's caps and picture) and
//!   stop it with the lock released ([`RtaWorker::stop`]): it is joined, and
//!   the listener is woken once so the app sees the reset.
//!
//! - Where there are no threads (`wasm32-unknown-unknown`), `rta_start`
//!   installs the same worker without one, and the caller's timer runs each
//!   turn through `DspiCore::rta_tick` (WP WB2).
//!
//! The request (what the app wants analysed) and the listener survive a
//! reconnect, as in the old core; `rta_start` must be called again for the
//! new connection.
//!
//! The device holds one analyser configuration, and the last console to push
//! one drives it; every other console follows (WP CG5; the Swift console's
//! `rta_sharing_spec.md` section 3). An `NOTIFY_EVT_RTA_CONFIG` from someone
//! else, or on firmware without the event a readback that stops agreeing,
//! moves this core to following: [`RtaState`] then feeds the engine the
//! device's configuration instead of the request's, and the worker's
//! [`PollTransport`] drops the engine's push, so the snapshot shows what the
//! analyser is doing and nothing is written over the other console.
//! [`DspiCore::rta_take_control`], or the request changing, ends it. The rules
//! are [`dspi_session::rta_sharing`]'s (WP CG19), which the Terminal uses too.
//! The last choice made wins (WP ANLAST): a new choice is pending until its
//! echo comes through the device's event sequence, and another console's event
//! drained before that echo was applied earlier, so it does not make this core
//! follow.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, Weak};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use dspi_transport::clock::Instant;

use dspi_proto::generated::notify as nfy;
use dspi_proto::generated::opcodes as op;
use dspi_proto::packets::{RtaBandFrame, RtaCaps, RtaConfig};
use dspi_session::rta::{self as engine, Budget, Options, Request, RtaEngine, ViewId};
use dspi_session::rta_sharing::{Held, PollShare, Sharing};
use dspi_transport::{DeviceDescriptor, Transport, TransportCaps, TransportError, TransportKind};

use crate::callbacks::RtaListener;
use crate::consts::{RTA_FLOOR_DB, RTA_MAX_BANDS, RTA_MAX_BINS, RTA_MAX_CHANNELS, RTA_TAP_INPUT};
use crate::records::*;
use crate::{DspiCore, Inner, Listeners};

/// How often the worker polls while the app wants frames (the old core's
/// 60 ms tick). A new request wakes it early.
pub(crate) const TICK: Duration = Duration::from_millis(60);
/// How often the worker looks again while nothing is requested; a request
/// wakes it at once.
const IDLE_TICK: Duration = Duration::from_millis(500);
/// `NOTIFY_EVT_RTA_CONFIG`, which the session's decoder delivers as
/// `Event::Unknown` (payload from byte 4: the applied `RtaConfig`, then the
/// source; notify.c, `NOTIFY_EVT_RTA_CONFIG`).
pub(crate) const EVT_RTA_CONFIG: u8 = nfy::NOTIFY_EVT_RTA_CONFIG as u8;
/// Where the source byte sits in that event's payload.
pub(crate) const RTA_CONFIG_SOURCE_AT: usize = RtaConfig::SIZE;
/// Per poll: the bin frame may take a few chunks (a Link transfer is small),
/// but the lock is held for the poll, so it is bounded.
const BUDGET: Budget = Budget {
    max_bin_chunks: 4,
    max_time: Duration::from_millis(25),
};

#[cfg_attr(feature = "uniffi", uniffi::export)]
impl DspiCore {
    /// Register (Some) or clear (None) the analyser wake-up listener. Once it
    /// returns after None, no call is in progress (on another thread).
    /// Replacing (Some -> Some) does not wait, and the old listener may then
    /// be released on the RTA thread when its running call ends.
    ///
    /// The listener is called on the RTA worker thread (and once, for the
    /// reset, on the thread that closes or replaces the device) without the
    /// core lock held; it may read the core (`rta_get_snapshot`,
    /// `get_state`) but must not block.
    pub fn rta_set_listener(&self, listener: Option<Arc<dyn RtaListener>>) {
        self.listeners.rta.set(listener);
    }

    /// Probe the open device's analyser and start polling it, on a worker
    /// thread. Only on compatible firmware (a writable connection, PLAN
    /// §1.5); a no-op without a device, on a read-only connection, or while
    /// a worker was already started for this connection. The worker reads
    /// the caps first (`rta_get_caps().supported` stays false on firmware
    /// without an analyser) and ends on disconnect, on close and on reopen;
    /// call this again after each connect.
    ///
    /// The worker holds a weak reference to the core for its per-poll lock.
    /// Never fails: a worker thread that cannot be spawned is logged and the
    /// analyser stays unsupported.
    pub fn rta_start(&self) {
        let mut inner = self.lock();
        // -- WP P1DKFIX -- F7: the analyser runs while the installer's lock is on.
        let _analyser = crate::limits::LockScope::allow();
        let _viewing = crate::read_only::Viewing::allow(); // -- WP ROMODE --
        if inner.rta.worker.is_some() || inner.writable_conn().is_err() {
            return;
        }
        inner.rta.next_id = inner.rta.next_id.wrapping_add(1);
        let id = inner.rta.next_id;
        match RtaWorker::spawn(self.me.clone(), Arc::clone(&self.listeners), id) {
            Ok(worker) => inner.rta.worker = Some(worker),
            Err(e) => log::warn!("cannot start the spectrum analyser worker: {e}"),
        }
    }

    /// What to analyse; mask 0 releases the analyser (the worker stops it
    /// on the device). Kept across reconnects. Channel bits beyond the tap's
    /// width are ignored; an `fft_order` of 0 means the device's default,
    /// and one outside the device's range is clamped into it. Wakes the
    /// worker at once.
    ///
    /// A request that changes what this console asks for takes the analyser
    /// back from a console being followed ([`Self::rta_get_ownership`]).
    /// Releasing it (mask 0) and asking for the same thing again does not.
    ///
    /// Mask 0 with `watching` set (WP CG30): a view shows the analyser with
    /// no channel chosen. It asks for nothing of its own, so while driving
    /// the analyser is released as with mask 0; while following, the
    /// device's configuration is still polled and in the snapshot.
    pub fn rta_set_request(&self, request: RtaRequest) {
        let signal = {
            let mut inner = self.lock();
            if inner.rta.request == request {
                return;
            }
            inner.rta.request = request;
            inner.rta.judge_request();
            inner.rta.worker.as_ref().map(|w| Arc::clone(&w.signal))
        };
        if let Some(signal) = signal {
            signal.kick();
        }
    }

    /// The device's analyser as the worker probed it: `supported` false
    /// until then, and on firmware without one. `centres` has
    /// `RTA_MAX_BANDS` entries, the first `max_bands` in Hz, the rest 0.
    pub fn rta_get_caps(&self) -> RtaCapsInfo {
        self.lock().rta.caps.clone()
    }

    /// The latest picture and telemetry. `version` changes only when what is
    /// drawn changed, `telemetry_version` when the status line did. Levels
    /// are dBFS; a silent channel reads `RTA_FLOOR_DB`.
    pub fn rta_get_snapshot(&self) -> RtaSnapshot {
        self.lock().rta.snapshot.clone()
    }

    /// Whether this console drives the analyser or follows another one's
    /// configuration. While following, the snapshot shows the device's tap
    /// and channels rather than the request's. The [`RtaListener`] is woken
    /// when this changes.
    pub fn rta_get_ownership(&self) -> RtaOwnership {
        self.lock().rta.ownership()
    }

    /// Stop following and push this console's own request on the next poll.
    /// Wakes the worker at once; does nothing while already driving.
    pub fn rta_take_control(&self) {
        let signal = {
            let mut inner = self.lock();
            inner.rta.take_control();
            inner.rta.worker.as_ref().map(|w| Arc::clone(&w.signal))
        };
        if let Some(signal) = signal {
            signal.kick();
        }
    }
}

// -- WP WB2 --
/// The analyser without threads (`wasm32-unknown-unknown`). Not exported over
/// UniFFI or the C ABI; the web binding (WB3) calls it.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
impl DspiCore {
    /// One turn of the worker [`DspiCore::rta_start`] installed, as its thread
    /// takes natively: one poll (the probe on the first), then the
    /// [`RtaListener`] if the caps or the picture changed, with the lock
    /// released. Returns how long to wait before the next call: [`TICK`]
    /// while frames are wanted, longer while nothing is, zero after a request
    /// changed. `None` when there is no worker or it has ended (disconnect,
    /// close, a read-only connection); call `rta_start` after the next
    /// connect.
    ///
    /// Call it from a timer, never from inside a listener.
    pub fn rta_tick(&self) -> Option<Duration> {
        let (id, signal, turns) = {
            let inner = self.lock();
            let w = inner.rta.worker.as_ref()?;
            (w.id, Arc::clone(&w.signal), Arc::clone(&w.turns))
        };
        let mut turns = turns.lock().unwrap_or_else(|p| p.into_inner());
        if turns.done || signal.stopped() {
            return None;
        }
        *signal.kicked.lock().unwrap_or_else(|p| p.into_inner()) = false;
        let Turns {
            probed, refused, ..
        } = &mut *turns;
        let polled = poll_once(self, id, probed, refused);
        if polled.changed && !signal.stopped() {
            self.listeners.rta.call(|l| l.on_frame());
        }
        if !polled.more {
            turns.done = true;
            return None;
        }
        // A request or a stop came during the poll: go again at once.
        if *signal.kicked.lock().unwrap_or_else(|p| p.into_inner()) {
            return Some(Duration::ZERO);
        }
        Some(if polled.idle { IDLE_TICK } else { TICK })
    }
}

/// The analyser's state and its worker thread (WP C9).
pub(crate) struct RtaState {
    /// The session's analyser host half; the core's subscription is `view`.
    engine: RtaEngine,
    /// The core's one subscription to the engine, while the request's mask
    /// is not 0, or while following for a watching request.
    view: Option<ViewId>,
    /// What the app asked for last (survives reconnects).
    request: RtaRequest,
    /// What `rta_get_caps` returns.
    caps: RtaCapsInfo,
    /// What `rta_get_snapshot` returns.
    snapshot: RtaSnapshot,
    /// The worker polling the open connection, once started.
    worker: Option<RtaWorker>,
    /// The id of the last worker spawned: a worker polls only while it is
    /// still the one installed here, so a worker taken out under the lock
    /// never touches the connection that replaced its own.
    next_id: u64,
    /// The worker that has `engine` out for a poll with the lock released
    /// (Link only); `engine` is then a placeholder.
    engine_out: Option<u64>,
    /// Driving or following (WP CG5).
    sharing: Sharing,
    /// What the listener was last woken for.
    published: RtaOwnership,
}

impl Default for RtaState {
    fn default() -> Self {
        Self {
            engine: RtaEngine::new(),
            view: None,
            request: RtaRequest::default(),
            caps: empty_caps(),
            snapshot: empty_snapshot(),
            worker: None,
            next_id: 0,
            engine_out: None,
            sharing: Sharing::default(),
            published: RtaOwnership::default(),
        }
    }
}

impl RtaState {
    /// Forget the device (keeps the request and the subscription), bumping
    /// both snapshot versions so the app redraws.
    fn reset(&mut self) {
        self.engine.disconnect();
        self.sharing.reset();
        self.published = RtaOwnership::default();
        self.caps = empty_caps();
        self.snapshot = RtaSnapshot {
            version: self.snapshot.version.wrapping_add(1),
            telemetry_version: self.snapshot.telemetry_version.wrapping_add(1),
            ..empty_snapshot()
        };
    }

    /// The configuration the request asks for; `None` while it is released
    /// or the caps are unknown.
    fn own_config(&self) -> Option<RtaConfig> {
        let (req, caps) = (&self.request, &self.caps);
        let channel_mask = effective_mask(req, caps);
        if channel_mask == 0 || !caps.supported {
            return None;
        }
        let fft_order = match req.fft_order {
            0 => caps.order_default,
            o => o.clamp(caps.order_min, caps.order_max),
        };
        Some(RtaConfig {
            tap: req.tap,
            channel_mask,
            fft_order,
            avg_ms: req.avg_ms,
            peak_decay_db_s: req.peak_decay_db_s,
            flags: 0,
        })
    }

    /// A change of what this console asks for takes the analyser back. The
    /// same thing asked for again, or nothing asked for, does not.
    fn judge_request(&mut self) {
        let own = self.own_config();
        self.sharing.judge_own(own);
    }

    /// Stop following; the engine then wants the request again and pushes it.
    fn take_control(&mut self) {
        self.sharing.take_control();
    }

    /// The caps as the sharing rules read them; `None` while unknown. From
    /// `caps`, since the engine is out for a poll over Link when an event comes.
    fn sharing_caps(&self) -> Option<RtaCaps> {
        let c = &self.caps;
        c.supported.then(|| RtaCaps {
            input_channels: c.input_channels,
            output_channels: c.output_channels,
            order_min: c.order_min,
            order_max: c.order_max,
            ..RtaCaps::default()
        })
    }

    /// An `NOTIFY_EVT_RTA_CONFIG`, with its payload: the configuration the
    /// device just applied. `own`: the echo rule says we wrote it. `origin`:
    /// the session the hub attributes it to, 0 when unknown. Ignored without
    /// a worker, whose start begins with a push of its own anyway.
    pub(crate) fn note_config(&mut self, payload: &[u8], own: bool, origin: u16) {
        let Some(worker_signal) = self.worker.as_ref().map(|w| Arc::clone(&w.signal)) else {
            return;
        };
        self.judge_request();
        let caps = self.sharing_caps();
        if !self
            .sharing
            .note_config(payload, own, origin, caps.as_ref())
        {
            log::debug!("ignoring a malformed analyser configuration event");
            return;
        }
        worker_signal.kick();
    }

    /// Judge what a finished poll saw. Firmware without the event shows
    /// another host only here; with the event this just gets there first,
    /// and the event then supplies the origin.
    fn after_poll(&mut self, share: &PollShare) {
        let caps = self.sharing_caps();
        self.sharing.finish_poll(share, caps.as_ref());
        // -- WP ANLAST -- a pending choice the device refused is not applied.
        self.sharing.engine_gave_up(self.engine.rejected());
    }

    /// What `rta_get_ownership` returns.
    fn ownership(&self) -> RtaOwnership {
        self.sharing
            .following()
            .map_or_else(RtaOwnership::default, |f| RtaOwnership {
                following: true,
                origin: f.origin,
                tap: f.config.tap,
                mask: f.config.channel_mask,
                fft_order: f.config.fft_order,
            })
    }

    /// The request with the tap and channels the device really analyses:
    /// what is polled and drawn. Released stays released; watching with
    /// nothing chosen draws what is followed (WP CG30).
    fn analysed(&self) -> RtaRequest {
        match self.sharing.following() {
            Some(f) if self.request.mask != 0 || self.request.watching => RtaRequest {
                tap: f.config.tap,
                mask: f.config.channel_mask,
                ..self.request
            },
            _ => self.request,
        }
    }

    /// Feed the request into the engine: options against the caps, one
    /// subscription while the mask is not 0. While following, the device's
    /// configuration instead. Idempotent.
    fn sync_request(&mut self) {
        self.judge_request();
        let req = self.analysed();
        let options = match self.sharing.following() {
            Some(f) => f.options(),
            None => Options {
                fft_order: match (req.fft_order, self.engine.caps()) {
                    (0, _) => None,
                    (o, Some(c)) => Some(o.clamp(c.order_min, c.order_max)),
                    (o, None) => Some(o),
                },
                avg_ms: req.avg_ms,
                peak_decay_db_s: req.peak_decay_db_s,
            },
        };
        self.engine.set_options(options);
        let mask = effective_mask(&req, &self.caps);
        if mask == 0 {
            if let Some(view) = self.view.take() {
                self.engine.release(view);
            }
            return;
        }
        let want = Request {
            tap: req.tap,
            mask,
            wants_bins: req.wants_bins,
        };
        match self.view {
            Some(view) => self.engine.update(view, want),
            None => self.view = Some(self.engine.subscribe(want)),
        }
    }

    /// Store `next`, bumping the versions that changed; true if any did.
    fn publish(&mut self, mut next: RtaSnapshot) -> bool {
        let picture = !same_picture(&next, &self.snapshot);
        let telemetry = !same_telemetry(&next, &self.snapshot);
        next.version = self.snapshot.version.wrapping_add(u32::from(picture));
        next.telemetry_version = self
            .snapshot
            .telemetry_version
            .wrapping_add(u32::from(telemetry));
        self.snapshot = next;
        picture || telemetry
    }
}

/// The request's channel mask within the tap's width (the whole mask while
/// the caps are unknown).
fn effective_mask(req: &RtaRequest, caps: &RtaCapsInfo) -> u16 {
    if !caps.supported {
        return req.mask;
    }
    let width = if req.tap == RTA_TAP_INPUT {
        caps.input_channels
    } else {
        caps.output_channels
    };
    let valid = 1u32
        .checked_shl(u32::from(width.min(16)))
        .map_or(u32::MAX, |b| b.saturating_sub(1));
    req.mask & u16::try_from(valid).unwrap_or(u16::MAX)
}

/// Whether bit `ch` of `mask` is set.
fn has(mask: u16, ch: u8) -> bool {
    mask.checked_shr(u32::from(ch)).unwrap_or(0) & 1 != 0
}

/// Caps of a device not (yet) probed: unsupported, `RTA_MAX_BANDS` zero
/// centres.
fn empty_caps() -> RtaCapsInfo {
    RtaCapsInfo {
        centres: vec![0; RTA_MAX_BANDS],
        ..RtaCapsInfo::default()
    }
}

/// A silent channel: every band at the floor.
fn silent_frame(channel: u8, n_bands: u8) -> RtaChannelFrame {
    RtaChannelFrame {
        channel,
        n_bands,
        silent: true,
        avg_db: vec![RTA_FLOOR_DB; RTA_MAX_BANDS],
        peak_db: vec![RTA_FLOOR_DB; RTA_MAX_BANDS],
    }
}

/// The old core's empty snapshot: `RTA_MAX_CHANNELS` silent frames,
/// `RTA_MAX_BINS` bins at the floor, `first_band` 0xFF (unknown).
fn empty_snapshot() -> RtaSnapshot {
    RtaSnapshot {
        frames: vec![silent_frame(0, 0); RTA_MAX_CHANNELS],
        bins_db: vec![RTA_FLOOR_DB; RTA_MAX_BINS],
        first_band: 0xFF,
        ..RtaSnapshot::default()
    }
}

/// Whether what is drawn is the same (the old core's `same_picture`).
fn same_picture(a: &RtaSnapshot, b: &RtaSnapshot) -> bool {
    a.tap == b.tap
        && a.count == b.count
        && a.frames.get(..usize::from(a.count)) == b.frames.get(..usize::from(b.count))
        && a.populated == b.populated
        && a.bins_count == b.bins_count
        && a.bins_channel == b.bins_channel
        && a.bins_sample_rate == b.bins_sample_rate
        && a.bins_db.get(..usize::from(a.bins_count)) == b.bins_db.get(..usize::from(b.bins_count))
}

/// Whether the status line is the same (the old core's `same_telemetry`).
fn same_telemetry(a: &RtaSnapshot, b: &RtaSnapshot) -> bool {
    a.running == b.running
        && a.live_count == b.live_count
        && a.live_mask == b.live_mask
        && a.frames_per_s == b.frames_per_s
        && a.busy_us_per_s == b.busy_us_per_s
        && a.last_frame_us == b.last_frame_us
        && a.bass_busy_us_per_s == b.bass_busy_us_per_s
        && a.sample_rate == b.sample_rate
        && a.first_band == b.first_band
        && a.fft_order == b.fft_order
        && a.config_rejected == b.config_rejected
}

/// The caps record from the engine's probe.
fn caps_info(e: &RtaEngine) -> RtaCapsInfo {
    let Some(c) = e.caps() else {
        return empty_caps();
    };
    let mut centres: Vec<u16> = e.centres().iter().copied().take(RTA_MAX_BANDS).collect();
    centres.resize(RTA_MAX_BANDS, 0);
    RtaCapsInfo {
        supported: true,
        input_channels: c.input_channels,
        output_channels: c.output_channels,
        order_min: c.order_min,
        order_max: c.order_max,
        order_default: c.order_default,
        bass_bands: c.bass_bands,
        max_bands: c.max_bands,
        dynamic_range_db: c.dynamic_range_db,
        bass_dynamic_range_db: c.bass_dynamic_range_db,
        centres,
    }
}

/// Bands with a level at this rate and transform size (bit b = band b): the
/// bass bank always, an FFT band only when a bin centre falls inside its
/// edges; none below `first_band`, none at all for an unknown rate (the old
/// core's `populated_bands`, on the session's `band_populated`).
fn populated_bands(
    bass_bands: u8,
    n_bands: u8,
    first_band: u8,
    sample_rate: u32,
    fft_order: u8,
) -> u64 {
    if first_band == 0xFF
        || sample_rate == 0
        || !(engine::ORDER_MIN..=engine::ORDER_MAX).contains(&fft_order)
    {
        return 0;
    }
    let top = usize::from(n_bands).min(RTA_MAX_BANDS);
    (usize::from(first_band)..top)
        .filter(|&b| {
            engine::band_populated(
                b,
                f64::from(sample_rate),
                fft_order,
                usize::from(bass_bands),
            )
        })
        .fold(0u64, |m, b| {
            m | u32::try_from(b)
                .ok()
                .and_then(|b| 1u64.checked_shl(b))
                .unwrap_or(0)
        })
}

/// Band levels in dBFS, `RTA_MAX_BANDS` of them, the first `bands` from the
/// device and the rest at the floor.
fn levels(e: &RtaEngine, v: &[u8], bands: usize) -> Vec<f32> {
    let mut out = vec![RTA_FLOOR_DB; RTA_MAX_BANDS];
    for (o, b) in out.iter_mut().zip(v.iter().take(bands)) {
        // The record's precision; levels come in 0.5 dB steps.
        *o = e.level_db(*b) as f32;
    }
    out
}

/// The snapshot from the engine's state, for the current request.
fn build_snapshot(st: &RtaState) -> RtaSnapshot {
    let e = &st.engine;
    let req = st.analysed();
    let mask = effective_mask(&req, &st.caps);
    let mut s = empty_snapshot();
    if mask == 0 || !e.supported() {
        return s;
    }
    s.tap = req.tap;
    let status = e.status().copied();
    // Only channels the device analyses at this tap (all requested ones
    // until its status says).
    let live = match status {
        Some(dev) if dev.tap == req.tap => dev.live_mask,
        _ => mask,
    };
    let mut n: u8 = 0;
    for ch in 0..16u8 {
        if !has(mask & live, ch) {
            continue;
        }
        let frame = match e.frame(req.tap, ch) {
            Some(f) if !e.is_stale(ch) => {
                let bands = usize::from(f.n_bands).min(RTA_MAX_BANDS);
                RtaChannelFrame {
                    channel: ch,
                    n_bands: f.n_bands,
                    silent: false,
                    avg_db: levels(e, &f.avg, bands),
                    peak_db: levels(e, &f.peak, bands),
                }
            }
            // Stale (the audio stopped): silence.
            Some(f) => silent_frame(ch, f.n_bands),
            // Not read yet, or never published: left out, as the old core
            // listed only the frames it had read.
            None => continue,
        };
        if let Some(slot) = s.frames.get_mut(usize::from(n)) {
            *slot = frame;
            n = n.saturating_add(1);
        }
    }
    s.count = n;
    let order = e.fft_order();
    let n_bands = s
        .frames
        .iter()
        .take(usize::from(n))
        .map(|f| f.n_bands)
        .max()
        .unwrap_or(0);
    let dev = status.unwrap_or_default();
    let first_band = if status.is_some() {
        dev.first_band
    } else {
        0xFF
    };
    let bass = e.caps().map_or(0, |c| c.bass_bands);
    s.populated = populated_bands(bass, n_bands, first_band, dev.sample_rate_hz, order);

    // Bins: a one-channel request that asked for them.
    if req.wants_bins && mask.count_ones() == 1 {
        let ch = u8::try_from(mask.trailing_zeros()).unwrap_or(0);
        if let Some(b) = e.bins(req.tap, ch) {
            let count = b.smoothed_db.len().min(RTA_MAX_BINS);
            for (o, v) in s.bins_db.iter_mut().zip(b.smoothed_db.iter().take(count)) {
                *o = *v as f32;
            }
            s.bins_count = u16::try_from(count).unwrap_or(0);
            s.bins_channel = b.channel;
            s.bins_sample_rate = b.sample_rate_hz;
        }
    }

    s.running = e.running();
    s.live_count = dev.live_count;
    s.live_mask = dev.live_mask;
    s.frames_per_s = dev.frames_per_s;
    s.busy_us_per_s = dev.busy_us_per_s;
    s.last_frame_us = dev.last_frame_us;
    s.bass_busy_us_per_s = dev.bass_busy_us_per_s;
    s.sample_rate = dev.sample_rate_hz;
    s.first_band = first_band;
    s.fft_order = order;
    // Following is not a refusal, whatever the engine counted on the way.
    s.config_rejected = e.rejected() && st.sharing.following().is_none();
    s.refresh_interval_s = e.refresh_interval().as_secs_f32();
    s
}

// ── The poll's transport ───────────────────────────────────────────────

/// The connection's transport as the engine sees it, with the firmware's
/// USB-only all-channel band read (`0x0F`) replaced by one `0x0B` per
/// requested channel when the device refuses it (a STALL: a bridge behind
/// DSPi Link answers it ERROR; vendor_commands.c, `REQ_RTA_GET_BANDS_ALL`).
/// On a non-USB link the refusal is permanent, so `refused` remembers it and
/// later polls go per channel at once; over USB a STALL means the bulk lock
/// is busy, so the next poll tries `0x0F` again.
///
/// The sharing rules sit beneath it in the session's [`Held`]: while following,
/// the engine's push of the followed configuration is dropped, so it treats that
/// as applied and goes on polling, and every configuration read is noted.
struct PollTransport<'a> {
    t: &'a mut dyn Transport,
    /// The channels the engine asks for (its wanted mask).
    mask: u16,
    refused: &'a mut bool,
}

impl PollTransport<'_> {
    /// One frame per requested channel, back to back as `0x0F` answers. A
    /// channel that fails is left out; none at all is the first error.
    fn per_channel(&mut self) -> dspi_transport::Result<Vec<u8>> {
        let size = u16::try_from(RtaBandFrame::SIZE).unwrap_or(u16::MAX);
        let mut out = Vec::new();
        let mut first_err = None;
        for ch in 0..16u8 {
            if !has(self.mask, ch) {
                continue;
            }
            match self
                .t
                .control_in(op::REQ_RTA_GET_BANDS, u16::from(ch), size)
            {
                Ok(b) => out.extend(b),
                Err(TransportError::Disconnected) => return Err(TransportError::Disconnected),
                Err(e) => {
                    first_err.get_or_insert(e);
                }
            }
        }
        match first_err {
            Some(e) if out.is_empty() => Err(e),
            _ => Ok(out),
        }
    }
}

impl Transport for PollTransport<'_> {
    fn control_in(&mut self, opcode: u8, value: u16, len: u16) -> dspi_transport::Result<Vec<u8>> {
        self.t.control_in(opcode, value, len)
    }

    fn control_in_upto(
        &mut self,
        opcode: u8,
        value: u16,
        max_len: u16,
    ) -> dspi_transport::Result<Vec<u8>> {
        if opcode != op::REQ_RTA_GET_BANDS_ALL {
            return self.t.control_in_upto(opcode, value, max_len);
        }
        if !*self.refused {
            match self.t.control_in_upto(opcode, value, max_len) {
                Err(TransportError::Stalled { .. }) => {
                    *self.refused = self.t.caps().kind != TransportKind::Usb;
                }
                other => return other,
            }
        }
        self.per_channel()
    }

    fn control_out(&mut self, opcode: u8, value: u16, data: &[u8]) -> dspi_transport::Result<()> {
        self.t.control_out(opcode, value, data)
    }

    fn descriptor(&self) -> &DeviceDescriptor {
        self.t.descriptor()
    }

    fn max_transfer(&self) -> usize {
        self.t.max_transfer()
    }

    fn caps(&self) -> TransportCaps {
        self.t.caps()
    }

    fn session_id(&self) -> Option<u16> {
        self.t.session_id()
    }
}

// ── The worker ─────────────────────────────────────────────────────────

/// Stop flag and early wake-up, shared by the worker and the core.
#[derive(Default)]
struct Signal {
    stop: AtomicBool,
    kicked: Mutex<bool>,
    cv: Condvar,
}

impl Signal {
    /// Wake the worker early (a new request, or a stop).
    fn kick(&self) {
        *self.kicked.lock().unwrap_or_else(|p| p.into_inner()) = true;
        self.cv.notify_all();
    }

    fn request_stop(&self) {
        self.stop.store(true, Ordering::SeqCst);
        self.kick();
    }

    fn stopped(&self) -> bool {
        self.stop.load(Ordering::SeqCst)
    }

    /// Sleep up to `d`, returning early on a kick or a stop.
    #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
    fn wait(&self, d: Duration) {
        let guard = self.kicked.lock().unwrap_or_else(|p| p.into_inner());
        let mut guard = if *guard || self.stopped() {
            guard
        } else {
            self.cv
                .wait_timeout(guard, d)
                .map(|(g, _)| g)
                .unwrap_or_else(|p| p.into_inner().0)
        };
        *guard = false;
    }
}

/// The analyser's poll thread (WP C9). Dropping it, or [`RtaWorker::stop`],
/// stops the thread and joins it; that must happen with the `Inner` lock
/// released, because the worker takes the lock for each poll (PLAN §1.2).
pub(crate) struct RtaWorker {
    id: u64,
    signal: Arc<Signal>,
    listeners: Arc<Listeners>,
    thread: Option<JoinHandle<()>>,
    /// The poll's own state where the caller drives it (WP WB2).
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    turns: Arc<Mutex<Turns>>,
}

/// What the worker thread keeps between polls, for a worker the caller
/// drives with [`DspiCore::rta_tick`] (WP WB2).
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
#[derive(Default)]
struct Turns {
    probed: bool,
    refused: bool,
    /// The worker has ended, as its thread would have returned.
    done: bool,
}

impl RtaWorker {
    #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
    fn spawn(core: Weak<DspiCore>, listeners: Arc<Listeners>, id: u64) -> std::io::Result<Self> {
        let signal = Arc::new(Signal::default());
        let (sig, l) = (Arc::clone(&signal), Arc::clone(&listeners));
        let thread = thread::Builder::new()
            .name("dspi-rta".into())
            .spawn(move || run(&core, &sig, &l, id))?;
        Ok(Self {
            id,
            signal,
            listeners,
            thread: Some(thread),
        })
    }

    /// No threads on the web (WP WB2): the worker is polled by
    /// [`DspiCore::rta_tick`], on the caller's timer.
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    fn spawn(_core: Weak<DspiCore>, listeners: Arc<Listeners>, id: u64) -> std::io::Result<Self> {
        Ok(Self {
            id,
            signal: Arc::new(Signal::default()),
            listeners,
            thread: None,
            turns: Arc::default(),
        })
    }

    /// Stop the thread and wait for it, then wake the [`RtaListener`] once
    /// so the app sees the reset caps and picture. Call without holding the
    /// `Inner` lock.
    pub(crate) fn stop(mut self) {
        self.shut_down();
        self.listeners.rta.call(|l| l.on_frame());
    }

    /// Stop and join. When the worker thread itself drops the last reference
    /// to the core (and with it this worker), it is not joined: it returns
    /// as soon as this drop is done.
    fn shut_down(&mut self) {
        self.signal.request_stop();
        if let Some(t) = self.thread.take()
            && t.thread().id() != thread::current().id()
        {
            let _ = t.join();
        }
    }
}

impl Drop for RtaWorker {
    fn drop(&mut self) {
        self.shut_down();
    }
}

/// Take the running worker out of `inner`, for the caller to stop after
/// releasing the lock: WP C1's close and displace paths do
/// `let w = rta::take_worker(&mut inner); drop(inner); if let Some(w) = w {
/// w.stop() }`. `None` while no worker was started.
///
/// Also forgets the device: the caps go back to unsupported and the
/// snapshot to empty (both versions bumped); the request is kept.
pub(crate) fn take_worker(inner: &mut crate::Inner) -> Option<RtaWorker> {
    let worker = inner.rta.worker.take()?;
    inner.rta.reset();
    Some(worker)
}

/// What one poll tells the loop.
struct Polled {
    /// The caps or the snapshot changed: wake the listener.
    changed: bool,
    /// Keep polling.
    more: bool,
    /// Nothing is requested: poll slowly.
    idle: bool,
}

/// The worker thread: poll under the lock, wake the listener with the lock
/// released, sleep.
#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
fn run(core: &Weak<DspiCore>, signal: &Signal, listeners: &Listeners, id: u64) {
    let mut probed = false;
    let mut refused = false;
    while !signal.stopped() {
        let Some(c) = core.upgrade() else {
            return;
        };
        let polled = poll_once(&c, id, &mut probed, &mut refused);
        // Possibly the last reference: the core then drops here, on this
        // thread, with the lock released (see `RtaWorker::shut_down`).
        drop(c);
        if polled.changed && !signal.stopped() {
            listeners.rta.call(|l| l.on_frame());
        }
        if !polled.more {
            return;
        }
        signal.wait(if polled.idle { IDLE_TICK } else { TICK });
    }
}

/// One poll: with the lock held, unless the connection has a side channel.
fn poll_once(c: &DspiCore, id: u64, probed: &mut bool, refused: &mut bool) -> Polled {
    let now = Instant::now();
    let mut inner = c.lock();
    let side = inner
        .conn
        .as_ref()
        .and_then(|conn| conn.side.clone())
        .filter(|_| inner.rta.worker.as_ref().is_some_and(|w| w.id == id));
    let Some(side) = side else {
        return poll(&mut inner, id, probed, refused, now);
    };
    // A displaced worker still has the engine out: wait for its poll to end.
    if inner.rta.engine_out.is_some() {
        return Polled {
            changed: false,
            more: true,
            idle: false,
        };
    }
    // The analyser's configuration push and STOP are writes; the installer's
    // lock lets them through (-- WP P1DKFIX -- F7: they change no audio).
    let _analyser = crate::limits::LockScope::allow();
    let _viewing = crate::read_only::Viewing::allow(); // -- WP ROMODE --
    if inner.writable_conn().is_err() {
        return Polled {
            changed: false,
            more: false,
            idle: true,
        };
    }
    if *probed {
        inner.rta.sync_request();
    }
    let mut share = inner.rta.sharing.begin_poll();
    let mut eng = std::mem::take(&mut inner.rta.engine);
    inner.rta.engine_out = Some(id);
    drop(inner);

    let alive = {
        let mask = eng.wanted().map_or(0, |w| w.channel_mask);
        let mut t = side.rta_transport(mask);
        if *probed {
            let mut held = Held::new(&mut *t, &mut share);
            let mut ft = PollTransport {
                t: &mut held,
                mask,
                refused,
            };
            !eng.tick(&mut ft, now, BUDGET).disconnected
        } else {
            *probed = true;
            eng.connect(&mut *t);
            true
        }
    };

    let mut inner = c.lock();
    inner.rta.engine_out = None;
    if inner.rta.worker.as_ref().is_none_or(|w| w.id != id) {
        // Closed or displaced meanwhile: `take_worker`'s reset missed this
        // engine. The subscription stays with it, as after any reset.
        eng.disconnect();
        inner.rta.engine = eng;
        return Polled {
            changed: false,
            more: false,
            idle: true,
        };
    }
    inner.rta.engine = eng;
    inner.rta.after_poll(&share);
    finish_poll(&mut inner, Some(alive))
}

/// One poll, under the lock: the probe on the first, then one engine tick
/// and a new snapshot.
fn poll(inner: &mut Inner, id: u64, probed: &mut bool, refused: &mut bool, now: Instant) -> Polled {
    let stop = Polled {
        changed: false,
        more: false,
        idle: true,
    };
    // Taken out by close or reopen (under this lock): never touch the
    // connection that replaced ours.
    if inner.rta.worker.as_ref().is_none_or(|w| w.id != id) {
        return stop;
    }
    if *probed {
        inner.rta.sync_request();
    }
    let mut share = inner.rta.sharing.begin_poll();
    // The engine is moved out for the poll, so the connection can be
    // borrowed as a `Writable` alongside it.
    let mut eng = std::mem::take(&mut inner.rta.engine);
    let alive = (|| {
        // -- WP P1DKFIX -- F7: the push and STOP pass the installer's lock.
        let _analyser = crate::limits::LockScope::allow();
        let _viewing = crate::read_only::Viewing::allow(); // -- WP ROMODE --
        let mut w = inner.writable_conn().ok()?;
        let t = w.conn().transport();
        if !*probed {
            *probed = true;
            eng.connect(t);
            return Some(true);
        }
        let mask = eng.wanted().map_or(0, |c| c.channel_mask);
        let mut held = Held::new(t, &mut share);
        let mut ft = PollTransport {
            t: &mut held,
            mask,
            refused,
        };
        let report = eng.tick(&mut ft, now, BUDGET);
        Some(!report.disconnected)
    })();
    inner.rta.engine = eng;
    inner.rta.after_poll(&share);
    finish_poll(inner, alive)
}

/// The end of a poll: publish what the engine now knows. `alive`: `None` when
/// the connection could not be polled, else whether the device is still there.
fn finish_poll(inner: &mut Inner, alive: Option<bool>) -> Polled {
    let stop = Polled {
        changed: false,
        more: false,
        idle: true,
    };
    // Not writable any more, or gone: nothing to poll.
    let Some(alive) = alive else {
        return stop;
    };
    let st = &mut inner.rta;
    let mut changed = false;
    let caps = caps_info(&st.engine);
    if caps != st.caps {
        st.caps = caps;
        changed = true;
    }
    if !st.engine.supported() {
        // No analyser on this firmware: report it once and end.
        return Polled { changed, ..stop };
    }
    let next = build_snapshot(st);
    changed |= st.publish(next);
    let ownership = st.ownership();
    if ownership != st.published {
        st.published = ownership;
        changed = true;
    }
    Polled {
        changed,
        more: alive,
        idle: st.view.is_none(),
    }
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_c9 {
    use super::*;
    use dspi_proto::packets::{RtaBinFrame, RtaBinHeader, RtaCaps, RtaConfig, RtaStatus};
    use dspi_proto::wire_compat::WireRead;
    use std::sync::atomic::AtomicUsize;

    const CENTRES: [u16; 37] = [
        16, 20, 25, 31, 40, 50, 63, 80, 100, 125, 160, 200, 250, 315, 400, 500, 630, 800, 1000,
        1250, 1600, 2000, 2500, 3150, 4000, 5000, 6300, 8000, 10000, 12500, 16000, 20000, 25000,
        31500, 40000, 50000, 63000,
    ];

    /// The device side, shared with the test.
    #[derive(Default)]
    struct Dev {
        config: Option<RtaConfig>,
        log: Vec<(u8, u16)>,
        stops: usize,
        gone: bool,
    }

    /// A scripted analyser: answers by opcode and `wValue`, applies a
    /// pushed configuration at once, refuses `0x0F` when `bands_all` is
    /// false (a bridge), and has no analyser at all when `rta` is false.
    struct RtaDevice {
        dev: Arc<Mutex<Dev>>,
        kind: TransportKind,
        rta: bool,
        bands_all: bool,
        desc: DeviceDescriptor,
    }

    fn caps() -> RtaCaps {
        RtaCaps {
            input_channels: 2,
            output_channels: 4,
            order_min: 8,
            order_max: 10,
            order_default: 10,
            bass_bands: 14,
            max_bands: 37,
            level_zero: 243,
            dynamic_range_db: 120,
            idle_timeout_ms: 5000,
            max_bin_frame: 529,
            bass_dynamic_range_db: 70,
        }
    }

    fn frame(ch: u8) -> RtaBandFrame {
        RtaBandFrame {
            channel: ch,
            seq: 1,
            n_bands: 34,
            age_ms: 10,
            avg: [200 + ch; RtaBandFrame::MAX_BANDS],
            peak: [243; RtaBandFrame::MAX_BANDS],
        }
    }

    impl RtaDevice {
        fn new(kind: TransportKind, bands_all: bool) -> (Self, Arc<Mutex<Dev>>) {
            let dev = Arc::new(Mutex::new(Dev::default()));
            (
                Self {
                    dev: Arc::clone(&dev),
                    kind,
                    rta: true,
                    bands_all,
                    desc: DeviceDescriptor {
                        serial: "RTA".into(),
                        bus_id: String::new(),
                        address: 0,
                    },
                },
                dev,
            )
        }

        fn answer(&mut self, opcode: u8, value: u16) -> dspi_transport::Result<Vec<u8>> {
            let mut d = self.dev.lock().unwrap();
            d.log.push((opcode, value));
            if d.gone {
                return Err(TransportError::Disconnected);
            }
            let stall = Err(TransportError::Stalled { opcode });
            if !self.rta {
                return stall;
            }
            let mask = d.config.map_or(0, |c| c.channel_mask);
            match opcode {
                op::REQ_RTA_GET_CAPS if value == 0 => Ok(caps().encode().to_vec()),
                op::REQ_RTA_GET_CAPS => Ok(CENTRES
                    .iter()
                    .skip((usize::from(value) - 1) * 32)
                    .take(32)
                    .flat_map(|c| c.to_le_bytes())
                    .collect()),
                op::REQ_RTA_GET_CONFIG => match d.config {
                    Some(c) => Ok(c.encode().to_vec()),
                    None => stall,
                },
                op::REQ_RTA_GET_STATUS => Ok(RtaStatus {
                    state: u8::from(d.config.is_some()),
                    tap: d.config.map_or(0, |c| c.tap),
                    live_count: mask.count_ones() as u8,
                    live_mask: mask,
                    frames_per_s: 90,
                    sample_rate_hz: 48_000,
                    ..RtaStatus::default()
                }
                .encode()
                .to_vec()),
                op::REQ_RTA_GET_BANDS => Ok(frame(value as u8).encode().to_vec()),
                op::REQ_RTA_GET_BANDS_ALL if self.bands_all => Ok((0..16u8)
                    .filter(|c| has(mask, *c))
                    .flat_map(|c| frame(c).encode())
                    .collect()),
                // A flat 512-bin frame at -21.5 dBFS for the one channel,
                // sliced at the byte offset in wValue.
                op::REQ_RTA_GET_BINS if mask.count_ones() == 1 => Ok(RtaBinFrame {
                    header: RtaBinHeader {
                        channel: mask.trailing_zeros() as u8,
                        seq: 7,
                        fft_order: 10,
                        sample_rate_hz: 48_000,
                        n_bins: 512,
                    },
                    levels: vec![200; 512],
                }
                .encode()
                .into_iter()
                .skip(usize::from(value))
                .collect()),
                op::REQ_RTA_CONTROL => {
                    d.stops += 1;
                    d.config = None;
                    Ok(vec![0])
                }
                _ => stall,
            }
        }
    }

    /// A Link-like side channel: every poll is slow, and goes to a new
    /// `RtaDevice` view of the same device.
    struct SlowSide {
        dev: Arc<Mutex<Dev>>,
        polls: AtomicUsize,
    }

    impl crate::SideChannel for SlowSide {
        fn lock(&self, _: &str, _: Duration) -> crate::Result<()> {
            Ok(())
        }
        fn unlock(&self) {}
        fn rta_transport(self: Arc<Self>, _: u16) -> Box<dyn Transport> {
            self.polls.fetch_add(1, Ordering::SeqCst);
            std::thread::sleep(Duration::from_millis(300));
            Box::new(RtaDevice {
                dev: Arc::clone(&self.dev),
                kind: TransportKind::Link,
                rta: true,
                bands_all: false,
                desc: DeviceDescriptor {
                    serial: "S".into(),
                    bus_id: "link:x".into(),
                    address: 0,
                },
            })
        }
        fn meters(&self, _: u16) -> crate::Result<crate::MeterRead> {
            Ok(crate::MeterRead::default())
        }
    }

    #[test]
    fn wp_c15_over_link_the_worker_polls_with_the_lock_released() {
        let (t, dev) = RtaDevice::new(TransportKind::Link, false);
        let core = DspiCore::new();
        core.attach(Box::new(t), "S".into(), 0);
        let side = Arc::new(SlowSide {
            dev,
            polls: AtomicUsize::new(0),
        });
        {
            let mut inner = core.lock();
            let conn = inner.conn().unwrap();
            conn.wire = Some(WireRead::Exact);
            conn.side = Some(side.clone());
        }
        core.rta_start();
        let start = Instant::now();
        while start.elapsed() < Duration::from_millis(900) {
            let t = Instant::now();
            let _ = core.rta_get_caps();
            assert!(
                t.elapsed() < Duration::from_millis(150),
                "{:?}",
                t.elapsed()
            );
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(side.polls.load(Ordering::SeqCst) >= 2);
        assert!(core.rta_get_caps().supported, "the probe went through");
        // The engine is put back, or thrown away with the old worker.
        core.disconnect();
        assert!(core.lock().rta.engine_out.is_none());
        assert!(core.lock().rta.worker.is_none());
    }

    impl Transport for RtaDevice {
        fn control_in(
            &mut self,
            opcode: u8,
            value: u16,
            len: u16,
        ) -> dspi_transport::Result<Vec<u8>> {
            let mut b = self.answer(opcode, value)?;
            b.truncate(len as usize);
            Ok(b)
        }

        fn control_out(
            &mut self,
            opcode: u8,
            value: u16,
            data: &[u8],
        ) -> dspi_transport::Result<()> {
            let mut d = self.dev.lock().unwrap();
            d.log.push((opcode, value));
            if d.gone {
                return Err(TransportError::Disconnected);
            }
            if opcode == op::REQ_RTA_SET_CONFIG && self.rta {
                d.config = Some(RtaConfig::decode(data).unwrap());
                return Ok(());
            }
            Err(TransportError::Stalled { opcode })
        }

        fn descriptor(&self) -> &DeviceDescriptor {
            &self.desc
        }

        fn caps(&self) -> TransportCaps {
            TransportCaps {
                kind: self.kind,
                chunked_bulk: self.kind == TransportKind::Usb,
                remote: self.kind == TransportKind::Link,
            }
        }
    }

    /// A core with `t` attached as a compatible (writable) connection.
    fn core_with(t: RtaDevice) -> Arc<DspiCore> {
        let core = DspiCore::new();
        core.attach(Box::new(t), "RTA".into(), 0);
        core.lock().conn().unwrap().wire = Some(WireRead::Exact);
        core
    }

    fn wait_for(what: &str, mut f: impl FnMut() -> bool) {
        let until = Instant::now() + Duration::from_secs(5);
        while !f() {
            assert!(Instant::now() < until, "timed out waiting for {what}");
            thread::sleep(Duration::from_millis(5));
        }
    }

    fn request(mask: u16) -> RtaRequest {
        RtaRequest {
            tap: engine::TAP_OUTPUT,
            mask,
            wants_bins: false,
            fft_order: 10,
            avg_ms: 300,
            peak_decay_db_s: 12,
            watching: false,
        }
    }

    fn worker_finished(core: &DspiCore) -> bool {
        core.lock()
            .rta
            .worker
            .as_ref()
            .and_then(|w| w.thread.as_ref())
            .is_some_and(|t| t.is_finished())
    }

    fn count(dev: &Arc<Mutex<Dev>>, opcode: u8) -> usize {
        dev.lock()
            .unwrap()
            .log
            .iter()
            .filter(|(o, _)| *o == opcode)
            .count()
    }

    struct Counter(AtomicUsize);
    impl RtaListener for Counter {
        fn on_frame(&self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }

    #[test]
    fn wp_c9_records_start_empty_with_the_old_sizes() {
        let core = DspiCore::new();
        let caps = core.rta_get_caps();
        assert!(!caps.supported);
        assert_eq!(caps.centres.len(), RTA_MAX_BANDS);
        let s = core.rta_get_snapshot();
        assert_eq!(s.frames.len(), RTA_MAX_CHANNELS);
        assert_eq!(s.bins_db.len(), RTA_MAX_BINS);
        assert_eq!(s.first_band, 0xFF);
        assert!(
            s.frames
                .iter()
                .all(|f| f.silent && f.avg_db[0] == RTA_FLOOR_DB)
        );
    }

    #[test]
    fn wp_c9_start_without_a_device_or_read_only_does_nothing() {
        let core = DspiCore::new();
        core.rta_start();
        assert!(core.lock().rta.worker.is_none());
        let (t, _) = RtaDevice::new(TransportKind::Usb, true);
        core.attach(Box::new(t), "RTA".into(), 0);
        core.rta_start(); // `attach` leaves `wire` None: read-only
        assert!(core.lock().rta.worker.is_none());
    }

    #[test]
    fn wp_c9_probes_caps_and_polls_frames_over_usb() {
        let (t, dev) = RtaDevice::new(TransportKind::Usb, true);
        let core = core_with(t);
        let wakes = Arc::new(Counter(AtomicUsize::new(0)));
        core.rta_set_listener(Some(wakes.clone()));
        core.rta_set_request(request(0b0101));
        core.rta_start();
        wait_for("caps", || core.rta_get_caps().supported);
        let caps = core.rta_get_caps();
        assert_eq!((caps.input_channels, caps.output_channels), (2, 4));
        assert_eq!((caps.max_bands, caps.bass_bands), (37, 14));
        assert_eq!(caps.centres[..], CENTRES[..]);
        wait_for("frames", || core.rta_get_snapshot().count == 2);
        let s = core.rta_get_snapshot();
        assert_eq!(s.tap, engine::TAP_OUTPUT);
        assert_eq!((s.frames[0].channel, s.frames[1].channel), (0, 2));
        assert!(!s.frames[0].silent);
        // (200 - 243) / 2 dBFS; bands past n_bands stay at the floor.
        assert_eq!(s.frames[0].avg_db[0], -21.5);
        assert_eq!(s.frames[1].avg_db[0], -20.5);
        assert_eq!(s.frames[0].peak_db[0], 0.0);
        assert_eq!(s.frames[0].avg_db[36], RTA_FLOOR_DB);
        assert_eq!(s.fft_order, 10);
        assert!(s.version > 0);
        assert!(wakes.0.load(Ordering::SeqCst) > 0);
        wait_for("telemetry", || core.rta_get_snapshot().running);
        let s = core.rta_get_snapshot();
        assert_eq!(
            (s.live_mask, s.frames_per_s, s.sample_rate),
            (0b0101, 90, 48_000)
        );
        assert_eq!(s.first_band, 0);
        assert_ne!(s.populated & 1, 0, "bass bank");
        assert!(!s.config_rejected);
        assert_eq!(
            dev.lock().unwrap().config.unwrap().channel_mask,
            0b0101,
            "the request was pushed"
        );
        assert!(count(&dev, op::REQ_RTA_GET_BANDS_ALL) > 0);
        assert_eq!(count(&dev, op::REQ_RTA_GET_BANDS), 0);
    }

    // -- WP ROMODE --
    #[test]
    fn wp_romode_the_analyser_runs_in_the_client_read_only_mode() {
        let (t, dev) = RtaDevice::new(TransportKind::Usb, true);
        let core = core_with(t);
        core.set_client_read_only(true).unwrap();
        core.rta_set_request(request(0b0101));
        core.rta_start();
        wait_for("caps", || core.rta_get_caps().supported);
        wait_for("frames", || core.rta_get_snapshot().count == 2);
        assert_eq!(
            dev.lock().unwrap().config.unwrap().channel_mask,
            0b0101,
            "the request was pushed"
        );
        assert_eq!(core.client_read_only_refusals(), 0);
        // Everything else stays refused.
        assert_eq!(
            core.set_master_volume(-3.0),
            Err(crate::DspiError::ReadOnly)
        );
    }

    #[test]
    fn wp_c9_a_one_channel_request_gets_smoothed_bins() {
        let (t, _) = RtaDevice::new(TransportKind::Usb, true);
        let core = core_with(t);
        core.rta_set_request(RtaRequest {
            wants_bins: true,
            ..request(0b100)
        });
        core.rta_start();
        wait_for("bins", || core.rta_get_snapshot().bins_count == 512);
        let s = core.rta_get_snapshot();
        assert_eq!((s.bins_channel, s.bins_sample_rate), (2, 48_000));
        assert_eq!(s.bins_db.len(), RTA_MAX_BINS);
        assert!(s.bins_db.iter().all(|d| (d + 21.5).abs() < 1e-3));
        // Asking for two channels drops the bins.
        core.rta_set_request(RtaRequest {
            wants_bins: true,
            ..request(0b110)
        });
        wait_for("no bins", || core.rta_get_snapshot().bins_count == 0);
    }

    #[test]
    fn wp_c9_mask_zero_stops_the_device_and_empties_the_picture() {
        let (t, dev) = RtaDevice::new(TransportKind::Usb, true);
        let core = core_with(t);
        core.rta_set_request(request(0b1));
        core.rta_start();
        wait_for("frames", || core.rta_get_snapshot().count == 1);
        core.rta_set_request(request(0));
        wait_for("STOP", || dev.lock().unwrap().stops == 1);
        wait_for("empty", || core.rta_get_snapshot().count == 0);
        assert!(!core.rta_get_snapshot().running);
    }

    #[test]
    fn wp_c9_bridge_refuses_bands_all_and_falls_back_per_channel() {
        let (t, dev) = RtaDevice::new(TransportKind::Link, false);
        let core = core_with(t);
        core.rta_set_request(request(0b0110));
        core.rta_start();
        wait_for("frames", || core.rta_get_snapshot().count == 2);
        let s = core.rta_get_snapshot();
        assert_eq!((s.frames[0].channel, s.frames[1].channel), (1, 2));
        assert!(!s.frames[0].silent && !s.frames[1].silent);
        // A few more polls: over a link the refusal is remembered.
        let before = count(&dev, op::REQ_RTA_GET_BANDS);
        wait_for("more polls", || {
            count(&dev, op::REQ_RTA_GET_BANDS) >= before + 6
        });
        assert_eq!(
            count(&dev, op::REQ_RTA_GET_BANDS_ALL),
            1,
            "0x0F is tried once, then never again on this link"
        );
        let d = dev.lock().unwrap();
        assert!(d.log.contains(&(op::REQ_RTA_GET_BANDS, 1)));
        assert!(d.log.contains(&(op::REQ_RTA_GET_BANDS, 2)));
        assert!(!d.log.contains(&(op::REQ_RTA_GET_BANDS, 0)));
    }

    #[test]
    fn wp_c9_a_usb_stall_falls_back_for_that_read_only() {
        let (t, dev) = RtaDevice::new(TransportKind::Usb, false);
        let core = core_with(t);
        core.rta_set_request(request(0b11));
        core.rta_start();
        wait_for("frames", || core.rta_get_snapshot().count == 2);
        wait_for("0x0F tried again", || {
            count(&dev, op::REQ_RTA_GET_BANDS_ALL) >= 3
        });
    }

    #[test]
    fn wp_c9_firmware_without_an_analyser_stays_unsupported() {
        let (mut t, dev) = RtaDevice::new(TransportKind::Usb, true);
        t.rta = false;
        let core = core_with(t);
        core.rta_set_request(request(1));
        core.rta_start();
        wait_for("worker exit", || worker_finished(&core));
        assert!(!core.rta_get_caps().supported);
        assert_eq!(core.rta_get_snapshot().count, 0);
        // Only the caps probe went out.
        assert_eq!(dev.lock().unwrap().log, vec![(op::REQ_RTA_GET_CAPS, 0)]);
        // Starting again for the same connection is a no-op.
        core.rta_start();
        thread::sleep(TICK);
        assert_eq!(dev.lock().unwrap().log.len(), 1);
    }

    #[test]
    fn wp_c9_close_joins_the_worker_and_resets_the_records() {
        let (t, dev) = RtaDevice::new(TransportKind::Usb, true);
        let core = core_with(t);
        let wakes = Arc::new(Counter(AtomicUsize::new(0)));
        core.rta_set_listener(Some(wakes.clone()));
        core.rta_set_request(request(1));
        core.rta_start();
        wait_for("frames", || core.rta_get_snapshot().count == 1);
        core.close_connection();
        let version = core.rta_get_snapshot().version;
        let woken = wakes.0.load(Ordering::SeqCst);
        assert!(core.lock().rta.worker.is_none());
        assert!(!core.rta_get_caps().supported);
        let s = core.rta_get_snapshot();
        assert_eq!(s.count, 0);
        assert!(!s.running);
        // Joined: nothing is polled, published or woken after close returned.
        let n = dev.lock().unwrap().log.len();
        thread::sleep(TICK * 3);
        assert_eq!(dev.lock().unwrap().log.len(), n);
        assert_eq!(core.rta_get_snapshot().version, version);
        assert_eq!(wakes.0.load(Ordering::SeqCst), woken);
        // The request survives for the next connection.
        assert_eq!(core.lock().rta.request.mask, 1);
    }

    #[test]
    fn wp_c9_reset_bumps_both_versions_and_wakes_the_listener() {
        let core = DspiCore::new();
        let wakes = Arc::new(Counter(AtomicUsize::new(0)));
        core.rta_set_listener(Some(wakes.clone()));
        let (t, _) = RtaDevice::new(TransportKind::Usb, true);
        core.attach(Box::new(t), "RTA".into(), 0);
        core.lock().conn().unwrap().wire = Some(WireRead::Exact);
        core.rta_start();
        let worker = {
            let mut inner = core.lock();
            let before = (
                inner.rta.snapshot.version,
                inner.rta.snapshot.telemetry_version,
            );
            let w = take_worker(&mut inner);
            let after = (
                inner.rta.snapshot.version,
                inner.rta.snapshot.telemetry_version,
            );
            assert_eq!((after.0, after.1), (before.0 + 1, before.1 + 1));
            w
        };
        let woken = wakes.0.load(Ordering::SeqCst);
        worker.unwrap().stop();
        assert_eq!(wakes.0.load(Ordering::SeqCst), woken + 1);
        assert!(take_worker(&mut core.lock()).is_none());
    }

    #[test]
    fn wp_c9_reopen_stops_the_old_worker_and_restart_probes_the_new_device() {
        let (t, old) = RtaDevice::new(TransportKind::Usb, true);
        let core = core_with(t);
        core.rta_set_request(request(1));
        core.rta_start();
        wait_for("frames", || core.rta_get_snapshot().count == 1);
        // The displace path (`install`) takes and stops the worker.
        let (t2, new) = RtaDevice::new(TransportKind::Usb, true);
        let mut conn = crate::Connection::new(Box::new(t2), "RTA2".into(), 0);
        conn.wire = Some(WireRead::Exact);
        core.install(conn, crate::state::default_state(), "RTA2".into(), 0);
        assert!(core.lock().rta.worker.is_none());
        let n = old.lock().unwrap().log.len();
        core.rta_start();
        wait_for("frames on the new device", || {
            core.rta_get_snapshot().count == 1
        });
        assert_eq!(
            old.lock().unwrap().log.len(),
            n,
            "the old device is left alone"
        );
        assert!(count(&new, op::REQ_RTA_GET_CAPS) > 0);
        assert_eq!(new.lock().unwrap().config.unwrap().channel_mask, 1);
    }

    #[test]
    fn wp_c9_disconnect_ends_the_worker() {
        let (t, dev) = RtaDevice::new(TransportKind::Usb, true);
        let core = core_with(t);
        core.rta_set_request(request(1));
        core.rta_start();
        wait_for("frames", || core.rta_get_snapshot().count == 1);
        dev.lock().unwrap().gone = true;
        wait_for("worker exit", || worker_finished(&core));
        core.close_connection();
        assert!(core.lock().rta.worker.is_none());
    }

    /// PLAN §1.2: the listener runs with the lock released, so reading the
    /// core from inside it does not deadlock, on the worker thread or on the
    /// closing one.
    #[test]
    fn wp_c9_a_listener_reading_the_core_does_not_deadlock() {
        struct Reader {
            core: Mutex<Weak<DspiCore>>,
            reads: AtomicUsize,
        }
        impl RtaListener for Reader {
            fn on_frame(&self) {
                let core = self.core.lock().unwrap().upgrade();
                if let Some(c) = core {
                    let _ = c.get_state();
                    let _ = c.rta_get_snapshot();
                    let _ = c.rta_get_caps();
                    self.reads.fetch_add(1, Ordering::SeqCst);
                }
            }
        }
        let (t, _) = RtaDevice::new(TransportKind::Usb, true);
        let core = core_with(t);
        let reader = Arc::new(Reader {
            core: Mutex::new(Arc::downgrade(&core)),
            reads: AtomicUsize::new(0),
        });
        core.rta_set_listener(Some(reader.clone()));
        core.rta_set_request(request(0b11));
        core.rta_start();
        wait_for("listener reads", || {
            reader.reads.load(Ordering::SeqCst) >= 3
        });
        let before = reader.reads.load(Ordering::SeqCst);
        core.close_connection();
        assert!(reader.reads.load(Ordering::SeqCst) > before);
    }

    /// Dropping the core while the worker runs ends the worker (it holds
    /// only a weak reference), even when the worker's own reference was the
    /// last one.
    #[test]
    fn wp_c9_dropping_the_core_ends_the_worker() {
        let (t, dev) = RtaDevice::new(TransportKind::Usb, true);
        let core = core_with(t);
        core.rta_set_request(request(1));
        core.rta_start();
        wait_for("frames", || core.rta_get_snapshot().count == 1);
        let weak = Arc::downgrade(&core);
        drop(core);
        wait_for("core freed", || weak.upgrade().is_none());
        let n = dev.lock().unwrap().log.len();
        thread::sleep(TICK * 3);
        assert_eq!(dev.lock().unwrap().log.len(), n);
    }

    #[test]
    fn wp_c9_publish_bumps_only_what_changed() {
        let mut st = RtaState::default();
        let base = st.snapshot.clone();
        assert!(!st.publish(base.clone()));
        let mut t = base;
        t.frames_per_s = 90;
        assert!(st.publish(t.clone()));
        assert_eq!((st.snapshot.version, st.snapshot.telemetry_version), (0, 1));
        let mut p = t;
        p.count = 1;
        assert!(st.publish(p));
        assert_eq!((st.snapshot.version, st.snapshot.telemetry_version), (1, 1));
    }

    #[test]
    fn wp_c9_populated_bands_follow_the_old_core() {
        // 1024 points at 48 kHz: bass bank plus every FFT band up to 20 kHz.
        assert_eq!(populated_bands(14, 34, 0, 48_000, 10), (1u64 << 34) - 1);
        // 256 points at 44.1 kHz: band 16 (400 Hz) holds no bin.
        let m = populated_bands(14, 34, 0, 44_100, 8);
        assert_eq!(m & (1 << 16), 0);
        assert_ne!(m & (1 << 13), 0, "bass bands are always populated");
        assert_eq!(populated_bands(14, 34, 0xFF, 48_000, 10), 0);
        assert_eq!(populated_bands(14, 34, 2, 48_000, 10) & 0b11, 0);
    }

    #[test]
    fn wp_c9_the_mask_is_limited_to_the_tap() {
        let mut caps = empty_caps();
        caps.supported = true;
        caps.input_channels = 2;
        caps.output_channels = 16;
        let mut r = request(0xFFFF);
        assert_eq!(effective_mask(&r, &caps), 0xFFFF);
        r.tap = RTA_TAP_INPUT;
        assert_eq!(effective_mask(&r, &caps), 0b11);
        caps.supported = false;
        assert_eq!(effective_mask(&r, &caps), 0xFFFF);
    }

    // ── WP CG5: following another console, without a hub ──────────────

    fn config(tap: u8, channel_mask: u16) -> RtaConfig {
        RtaConfig {
            tap,
            channel_mask,
            fft_order: 10,
            avg_ms: 300,
            peak_decay_db_s: 12,
            flags: 0,
        }
    }

    fn event(c: RtaConfig, source: u8) -> Vec<u8> {
        let mut p = c.encode().to_vec();
        p.push(source);
        p
    }

    fn driving_on(mask: u16, kind: TransportKind) -> (Arc<DspiCore>, Arc<Mutex<Dev>>) {
        let (t, dev) = RtaDevice::new(kind, true);
        let core = core_with(t);
        core.rta_set_request(request(mask));
        core.rta_start();
        wait_for("frames", || core.rta_get_snapshot().count > 0);
        wait_for("the push", || dev.lock().unwrap().config.is_some());
        // -- WP ANLAST -- and its echo, as firmware with the event sends it: the
        // choice is no longer pending, so another console's event is later.
        wait_for("caps", || core.rta_get_caps().supported);
        let pushed = dev.lock().unwrap().config.unwrap();
        core.lock().rta.note_config(&event(pushed, 1), true, 0);
        assert!(!core.lock().rta.sharing.pending());
        (core, dev)
    }

    fn pushes(dev: &Arc<Mutex<Dev>>) -> usize {
        count(dev, op::REQ_RTA_SET_CONFIG)
    }

    fn readbacks(dev: &Arc<Mutex<Dev>>) -> usize {
        count(dev, op::REQ_RTA_GET_CONFIG)
    }

    #[test]
    fn wp_cg5_a_readback_that_stops_agreeing_means_another_host() {
        let (core, dev) = driving_on(0b1, TransportKind::Usb);
        let seen = readbacks(&dev);
        wait_for("agreement", || {
            readbacks(&dev) > seen && core.lock().rta.sharing.agreed()
        });
        let n = pushes(&dev);
        dev.lock().unwrap().config = Some(config(engine::TAP_OUTPUT, 0b10));
        wait_for("following", || core.rta_get_ownership().following);
        let o = core.rta_get_ownership();
        assert_eq!((o.origin, o.mask, o.fft_order), (0, 0b10, 10));
        // Follows further changes in place, and never writes over them.
        dev.lock().unwrap().config = Some(config(engine::TAP_INPUT, 0b11));
        wait_for("the new tap", || {
            core.rta_get_ownership().tap == engine::TAP_INPUT
        });
        wait_for("the new picture", || {
            let s = core.rta_get_snapshot();
            s.tap == engine::TAP_INPUT && s.count == 2
        });
        let seen = readbacks(&dev);
        wait_for("more polls", || readbacks(&dev) >= seen + 2);
        assert_eq!(pushes(&dev), n, "a follower does not push");
        assert!(!core.rta_get_snapshot().config_rejected);
    }

    // ── WP CG30: watching with nothing chosen ─────────────────────────

    fn watching(mask: u16) -> RtaRequest {
        RtaRequest {
            watching: true,
            ..request(mask)
        }
    }

    #[test]
    fn wp_cg30_watching_with_nothing_chosen_polls_what_is_followed() {
        let (core, dev) = driving_on(0b1, TransportKind::Usb);
        let seen = readbacks(&dev);
        wait_for("agreement", || {
            readbacks(&dev) > seen && core.lock().rta.sharing.agreed()
        });
        dev.lock().unwrap().config = Some(config(engine::TAP_OUTPUT, 0b110));
        wait_for("following", || core.rta_get_ownership().following);
        // The view stays, its choice cleared: nothing taken back.
        let n = pushes(&dev);
        core.rta_set_request(watching(0));
        assert!(core.rta_get_ownership().following);
        wait_for("their picture", || {
            let s = core.rta_get_snapshot();
            s.tap == engine::TAP_OUTPUT && s.count == 2
        });
        // Still polled, so a change there shows here.
        dev.lock().unwrap().config = Some(config(engine::TAP_INPUT, 0b11));
        wait_for("the new picture", || {
            let s = core.rta_get_snapshot();
            s.tap == engine::TAP_INPUT && s.count == 2
        });
        let seen = readbacks(&dev);
        wait_for("more polls", || readbacks(&dev) >= seen + 2);
        assert_eq!(pushes(&dev), n, "nothing of ours is pushed");
        assert!(core.rta_get_ownership().following);
        // Not watching: released, as before.
        core.rta_set_request(request(0));
        wait_for("released", || core.lock().rta.view.is_none());
    }

    #[test]
    fn wp_cg30_watching_with_nothing_chosen_while_driving_asks_for_nothing() {
        let (core, dev) = driving_on(0b1, TransportKind::Usb);
        core.rta_set_request(watching(0));
        wait_for("the stop", || dev.lock().unwrap().stops > 0);
        wait_for("an empty picture", || core.rta_get_snapshot().count == 0);
        assert!(core.lock().rta.view.is_none());
        let transfers = dev.lock().unwrap().log.len();
        thread::sleep(TICK * 4);
        assert_eq!(dev.lock().unwrap().log.len(), transfers, "no polls");
        assert!(!core.rta_get_ownership().following);
    }

    #[test]
    fn wp_cg5_a_refused_push_is_not_following() {
        // SET is taken but never applied: the readback never agrees.
        struct Refusing(RtaDevice);
        impl Transport for Refusing {
            fn control_in(&mut self, o: u8, v: u16, l: u16) -> dspi_transport::Result<Vec<u8>> {
                self.0.control_in(o, v, l)
            }
            fn control_out(&mut self, o: u8, v: u16, _: &[u8]) -> dspi_transport::Result<()> {
                let refused = config(engine::TAP_OUTPUT, 0b1000);
                self.0.dev.lock().unwrap().log.push((o, v));
                self.0.dev.lock().unwrap().config = Some(refused);
                Ok(())
            }
            fn descriptor(&self) -> &DeviceDescriptor {
                self.0.descriptor()
            }
            fn caps(&self) -> TransportCaps {
                self.0.caps()
            }
        }
        let (t, dev) = RtaDevice::new(TransportKind::Usb, true);
        let core = DspiCore::new();
        core.attach(Box::new(Refusing(t)), "RTA".into(), 0);
        core.lock().conn().unwrap().wire = Some(WireRead::Exact);
        core.rta_set_request(request(0b1));
        core.rta_start();
        wait_for("rejected", || core.rta_get_snapshot().config_rejected);
        assert!(!core.rta_get_ownership().following);
        assert_eq!(pushes(&dev), 3);
    }

    #[test]
    fn wp_cg5_events_are_judged_by_owner_and_shape() {
        let (core, dev) = driving_on(0b1, TransportKind::Usb);
        let note = |c: RtaConfig, own: bool, origin: u16| {
            core.lock().rta.note_config(&event(c, 1), own, origin);
        };
        // Ours: confirmed, still driving.
        note(config(engine::TAP_OUTPUT, 0b1), true, 0);
        assert!(!core.rta_get_ownership().following);
        // An echo of ours that no longer matches is an older push.
        note(config(engine::TAP_OUTPUT, 0b10), true, 0);
        assert!(!core.rta_get_ownership().following);
        // Malformed or empty: nothing.
        core.lock().rta.note_config(&[3, 1], false, 9);
        note(config(engine::TAP_OUTPUT, 0), false, 9);
        assert!(!core.rta_get_ownership().following);
        // Someone else's, with its origin, also over a released request.
        core.rta_set_request(request(0));
        // -- WP FLAKE5 -- the device holds what its event says (a readback
        // of ours meanwhile would rightly end the following).
        dev.lock().unwrap().config = Some(config(engine::TAP_OUTPUT, 0b10));
        note(config(engine::TAP_OUTPUT, 0b10), false, 9);
        let o = core.rta_get_ownership();
        assert!(o.following);
        assert_eq!((o.origin, o.tap, o.mask), (9, engine::TAP_OUTPUT, 0b10));
        // Closing the connection forgets it.
        core.close_connection();
        assert_eq!(core.rta_get_ownership(), RtaOwnership::default());
        let _ = dev;
    }

    /// Decodes, but no device with these caps (2 in, 4 out, order 8..=10)
    /// could hold it.
    fn impossible() -> Vec<RtaConfig> {
        let bad = |tap, channel_mask, fft_order| RtaConfig {
            fft_order,
            ..config(tap, channel_mask)
        };
        vec![
            bad(255, 1, 32),
            bad(engine::TAP_INPUT, 0x8000, 31),
            bad(2, 1, 10),
            bad(engine::TAP_OUTPUT, 1, 7),
            bad(engine::TAP_OUTPUT, 1, 11),
            bad(engine::TAP_OUTPUT, 0, 10),
            bad(engine::TAP_OUTPUT, 0b1_0000, 10),
            bad(engine::TAP_OUTPUT, 0b1_0001, 10),
            bad(engine::TAP_INPUT, 0b100, 10),
        ]
    }

    #[test]
    fn wp_cg5_an_event_no_device_could_send_is_ignored() {
        let (core, dev) = driving_on(0b1, TransportKind::Usb);
        for c in impossible() {
            core.lock().rta.note_config(&event(c, 1), false, 9);
            assert_eq!(core.rta_get_ownership(), RtaOwnership::default(), "{c:?}");
        }
        // The edges of what is possible are followed.
        let top = RtaConfig {
            fft_order: 8,
            ..config(engine::TAP_INPUT, 0b11)
        };
        // -- WP FLAKE5 -- the device applied it, as the event says: otherwise
        // the worker's next readback still shows ours, and the core rightly
        // goes back to driving before the ownership is read.
        dev.lock().unwrap().config = Some(top);
        core.lock().rta.note_config(&event(top, 1), false, 9);
        assert_eq!(core.rta_get_ownership().mask, 0b11);
        // And an impossible one leaves that alone.
        for c in impossible() {
            core.lock().rta.note_config(&event(c, 1), false, 7);
            let o = core.rta_get_ownership();
            assert_eq!(
                (o.origin, o.tap, o.mask, o.fft_order),
                (9, 0, 0b11, 8),
                "{c:?}"
            );
        }
    }

    #[test]
    fn wp_cg5_a_readback_no_device_could_send_is_ignored() {
        let (core, _dev) = driving_on(0b1, TransportKind::Usb);
        wait_for("caps", || core.rta_get_caps().supported);
        // One lock throughout: the worker's own polls stay out of this.
        let mut inner = core.lock();
        let st = &mut inner.rta;
        let read = |st: &mut RtaState, c: RtaConfig| {
            let mut share = st.sharing.begin_poll();
            share.note_readback(c);
            st.after_poll(&share);
        };
        // The device agrees, so a change from here means another host.
        st.judge_request();
        read(st, config(engine::TAP_OUTPUT, 0b1));
        assert!(st.sharing.agreed());
        for c in impossible() {
            read(st, c);
            assert_eq!(st.ownership(), RtaOwnership::default(), "{c:?}");
        }
        read(st, config(engine::TAP_OUTPUT, 0b10));
        assert_eq!(st.ownership().mask, 0b10, "a possible one is followed");
        for c in impossible() {
            read(st, c);
            assert_eq!(st.ownership().mask, 0b10, "{c:?}");
        }
    }

    // ── WP CG19: the session's rules, in the core ─────────────────────

    /// The review's question: the core compared `effective_mask` where the
    /// session compares against a bit-width mask. For a known device both say
    /// exactly what the firmware can report (`tap_valid_mask`, `apply_config`,
    /// rta.c). Unknown caps are where `effective_mask` lets a mask through.
    #[test]
    fn wp_cg19_the_session_decides_as_effective_mask_did() {
        for (input, output) in [(2u8, 5u8), (8, 9), (1, 16), (0, 3)] {
            let mut info = empty_caps();
            info.supported = true;
            info.input_channels = input;
            info.output_channels = output;
            info.order_min = 8;
            info.order_max = 10;
            let st = RtaState {
                caps: info.clone(),
                ..RtaState::default()
            };
            let caps = st.sharing_caps().unwrap();
            for tap in [engine::TAP_INPUT, engine::TAP_OUTPUT] {
                for mask in 0..=u16::MAX {
                    let c = config(tap, mask);
                    let shape = RtaRequest {
                        tap,
                        mask,
                        ..RtaRequest::default()
                    };
                    let old = mask != 0 && effective_mask(&shape, &info) == mask;
                    let mut sharing = Sharing::default();
                    assert_eq!(
                        sharing.note_config(&c.encode(), false, 9, Some(&caps)),
                        old,
                        "{tap} {mask:#x}"
                    );
                }
            }
        }
        // Unknown caps: effective_mask passes everything, the rules nothing.
        let st = RtaState::default();
        assert_eq!(st.sharing_caps(), None);
        assert_eq!(effective_mask(&request(0xFFFF), &st.caps), 0xFFFF);
    }

    /// The epoch check, through the worker's own steps: a poll that overlaps a
    /// change, as a Link poll does with the lock released.
    #[test]
    fn wp_cg19_a_reading_overtaken_by_a_change_is_not_applied() {
        let (core, _dev) = driving_on(0b1, TransportKind::Usb);
        wait_for("caps", || core.rta_get_caps().supported);
        let mut inner = core.lock();
        let st = &mut inner.rta;
        st.judge_request();
        let moved = config(engine::TAP_OUTPUT, 0b10);
        let mut agree = st.sharing.begin_poll();
        agree.note_readback(config(engine::TAP_OUTPUT, 0b1));
        st.after_poll(&agree);
        assert!(st.sharing.agreed());

        // Take control while the poll ran: the reading is older than that.
        let mut share = st.sharing.begin_poll();
        share.note_readback(moved);
        st.take_control();
        st.after_poll(&share);
        assert_eq!(st.ownership(), RtaOwnership::default());

        // An event while the poll ran: the event's word stands.
        let mut share = st.sharing.begin_poll();
        share.note_readback(moved);
        st.note_config(&event(config(engine::TAP_OUTPUT, 0b1000), 1), false, 9);
        st.after_poll(&share);
        assert_eq!((st.ownership().origin, st.ownership().mask), (9, 0b1000));

        // No change meanwhile: the reading counts.
        let mut share = st.sharing.begin_poll();
        share.note_readback(moved);
        st.after_poll(&share);
        assert_eq!((st.ownership().origin, st.ownership().mask), (0, 0b10));
    }

    #[test]
    fn wp_cg5_a_listener_is_woken_when_ownership_changes() {
        let (core, _dev) = driving_on(0b1, TransportKind::Usb);
        let wakes = Arc::new(Counter(AtomicUsize::new(0)));
        core.rta_set_listener(Some(wakes.clone()));
        thread::sleep(TICK * 2);
        let before = wakes.0.load(Ordering::SeqCst);
        core.lock()
            .rta
            .note_config(&event(config(engine::TAP_OUTPUT, 0b10), 1), false, 9);
        wait_for("the wake", || wakes.0.load(Ordering::SeqCst) > before);
        assert_eq!(core.lock().rta.published.origin, 9);
    }
}

/// WP CG5's done criterion: two consoles on one fake hub.
#[cfg(test)]
#[cfg(feature = "link")]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_cg5 {
    use super::*;
    use dspi_link::fakehub::{FakeHub, HubConfig, req};

    const DIR_SET: u8 = dspi_link::frames::Dir::Set as u8;

    fn hub() -> FakeHub {
        FakeHub::start(HubConfig {
            rta: true,
            ..HubConfig::default()
        })
        .unwrap()
    }

    /// A console on the hub, its analyser polling, with `mask` requested.
    /// Returns it with its session id.
    fn console(hub: &FakeHub, name: &str, mask: u16) -> (Arc<DspiCore>, u16) {
        let before = hub.sessions();
        let core = DspiCore::new();
        core.link_connect(
            hub.url(),
            name.into(),
            "test".into(),
            String::new(),
            String::new(),
        )
        .unwrap();
        let d = core.link_devices().remove(0);
        core.link_select_device(d.handle, d.serial).unwrap();
        core.fetch_all().unwrap();
        core.rta_set_request(request(mask));
        core.rta_start();
        let sid = hub
            .sessions()
            .into_iter()
            .find(|s| !before.contains(s))
            .unwrap();
        (core, sid)
    }

    fn request(mask: u16) -> RtaRequest {
        RtaRequest {
            tap: engine::TAP_OUTPUT,
            mask,
            wants_bins: false,
            fft_order: 10,
            avg_ms: 300,
            peak_decay_db_s: 12,
            watching: false,
        }
    }

    /// Run the consoles' notification handling until `f` holds, as an app
    /// does on its wake-ups.
    fn until(what: &str, consoles: &[&Arc<DspiCore>], mut f: impl FnMut() -> bool) {
        let end = Instant::now() + Duration::from_secs(8);
        loop {
            for c in consoles {
                c.process_notifications();
            }
            if f() {
                return;
            }
            assert!(Instant::now() < end, "timed out waiting for {what}");
            thread::sleep(Duration::from_millis(5));
        }
    }

    /// Settle: long enough for several polls, notifications handled.
    fn settle(consoles: &[&Arc<DspiCore>]) {
        let end = Instant::now() + TICK * 6;
        while Instant::now() < end {
            for c in consoles {
                c.process_notifications();
            }
            thread::sleep(Duration::from_millis(5));
        }
    }

    fn pushes(hub: &FakeHub, sid: u16) -> usize {
        hub.log()
            .iter()
            .filter(|e| e.0 == sid && e.1 == DIR_SET && e.2 == req::RTA_SET_CONFIG)
            .count()
    }

    fn device_mask(hub: &FakeHub) -> u16 {
        let c = hub.rta_config();
        u16::from_le_bytes([c[2], c[3]])
    }

    fn channels(s: &RtaSnapshot) -> Vec<u8> {
        s.frames
            .iter()
            .take(usize::from(s.count))
            .map(|f| f.channel)
            .collect()
    }

    /// -- WP QD8b -- `version` moves when the picture does, not on each poll: under a
    /// constant signal a client cannot read "still polling" from it. A test sees
    /// polling by changing what the hub's bands read.
    #[test]
    fn wp_qd8b_a_constant_signal_keeps_the_version_a_new_level_moves_it() {
        let hub = hub();
        let (a, _) = console(&hub, "a", 0b0001);
        until("a's picture", &[&a], || {
            channels(&a.rta_get_snapshot()) == [0]
        });
        settle(&[&a]);
        let first = a.rta_get_snapshot();
        settle(&[&a]);
        assert_eq!(a.rta_get_snapshot().version, first.version);
        hub.set_rta_level(100);
        until("the new level", &[&a], || {
            let now = a.rta_get_snapshot();
            now.version != first.version && now.frames[0].avg_db[0] != first.frames[0].avg_db[0]
        });
    }

    #[test]
    fn wp_cg5_another_session_switches_state_ours_does_not_and_take_control_works() {
        let hub = hub();
        let (a, sid_a) = console(&hub, "a", 0b0001);
        until("a's picture", &[&a], || {
            channels(&a.rta_get_snapshot()) == [0]
        });
        // Our own echo arrives and changes nothing.
        until("a's push", &[&a], || pushes(&hub, sid_a) > 0);
        settle(&[&a]);
        assert_eq!(a.rta_get_ownership(), RtaOwnership::default());

        // Another session pushes: a follows, with the origin, and shows it.
        let (b, sid_b) = console(&hub, "b", 0b0110);
        until("a following", &[&a, &b], || a.rta_get_ownership().following);
        let o = a.rta_get_ownership();
        assert_eq!(
            (o.origin, o.tap, o.mask, o.fft_order),
            (sid_b, engine::TAP_OUTPUT, 0b0110, 10)
        );
        until("a's new picture", &[&a, &b], || {
            channels(&a.rta_get_snapshot()) == [1, 2]
        });
        // The driver stays the driver, and the follower does not push.
        let n = pushes(&hub, sid_a);
        settle(&[&a, &b]);
        assert!(!b.rta_get_ownership().following);
        assert_eq!(pushes(&hub, sid_a), n, "a follower does not push");
        assert_eq!(device_mask(&hub), 0b0110);

        // Take control: a pushes its own request again, and b follows.
        a.rta_take_control();
        assert!(!a.rta_get_ownership().following);
        until("a's push again", &[&a, &b], || device_mask(&hub) == 0b0001);
        until("b following", &[&a, &b], || b.rta_get_ownership().following);
        assert_eq!(b.rta_get_ownership().origin, sid_a);
        assert_eq!(b.rta_get_ownership().mask, 0b0001);
        until("a's own picture", &[&a, &b], || {
            channels(&a.rta_get_snapshot()) == [0]
        });
        settle(&[&a, &b]);
        assert!(
            !a.rta_get_ownership().following,
            "ours does not switch state"
        );
        assert_eq!(device_mask(&hub), 0b0001, "no fight");
    }

    #[test]
    fn wp_cg5_only_a_new_choice_takes_the_analyser_back() {
        let hub = hub();
        let (a, sid_a) = console(&hub, "a", 0b0001);
        until("a's push", &[&a], || pushes(&hub, sid_a) > 0);
        let (b, _) = console(&hub, "b", 0b0110);
        until("a following", &[&a, &b], || a.rta_get_ownership().following);
        let n = pushes(&hub, sid_a);

        // A display hidden and shown again is not a choice.
        a.rta_set_request(request(0));
        settle(&[&a, &b]);
        assert!(a.rta_get_ownership().following, "survives the release");
        a.rta_set_request(request(0b0001));
        settle(&[&a, &b]);
        assert!(a.rta_get_ownership().following);
        assert_eq!(pushes(&hub, sid_a), n);
        assert_eq!(device_mask(&hub), 0b0110);

        // Picking something else is.
        a.rta_set_request(request(0b1000));
        assert!(!a.rta_get_ownership().following);
        until("a's choice", &[&a, &b], || device_mask(&hub) == 0b1000);
    }

    /// WP CG30: a view on screen with its choice cleared draws, live, what
    /// another session chooses, and asks for nothing.
    #[test]
    fn wp_cg30_watching_with_nothing_chosen_draws_another_sessions_choice() {
        let hub = hub();
        let (a, sid_a) = console(&hub, "a", 0b0001);
        until("a's push", &[&a], || pushes(&hub, sid_a) > 0);
        a.rta_set_request(RtaRequest {
            watching: true,
            ..request(0)
        });
        until("a released", &[&a], || a.lock().rta.view.is_none());
        let n = pushes(&hub, sid_a);

        let (b, sid_b) = console(&hub, "b", 0b0110);
        until("a following", &[&a, &b], || a.rta_get_ownership().following);
        assert_eq!(a.rta_get_ownership().origin, sid_b);
        until("b's channels on a", &[&a, &b], || {
            channels(&a.rta_get_snapshot()) == [1, 2]
        });
        let first = a.rta_get_snapshot();
        hub.set_rta_level(100);
        until("a new level on a", &[&a, &b], || {
            let now = a.rta_get_snapshot();
            now.version != first.version && now.frames[0].avg_db[0] != first.frames[0].avg_db[0]
        });
        b.rta_set_request(request(0b1000));
        until("b's new choice on a", &[&a, &b], || {
            channels(&a.rta_get_snapshot()) == [3]
        });
        settle(&[&a, &b]);
        assert_eq!(pushes(&hub, sid_a), n, "a pushes nothing");
        assert!(a.rta_get_ownership().following);
        assert!(!b.rta_get_ownership().following);
        assert_eq!(device_mask(&hub), 0b1000);

        // Without the flag the view is gone: released, though still following.
        a.rta_set_request(request(0));
        until("a released again", &[&a, &b], || {
            a.lock().rta.view.is_none()
        });
        assert!(a.rta_get_ownership().following);
    }

    #[test]
    fn wp_cg5_someone_choosing_what_we_want_ends_following_without_a_push() {
        let hub = hub();
        let (a, sid_a) = console(&hub, "a", 0b0001);
        until("a's push", &[&a], || pushes(&hub, sid_a) > 0);
        let (b, _) = console(&hub, "b", 0b0110);
        until("a following", &[&a, &b], || a.rta_get_ownership().following);
        let n = pushes(&hub, sid_a);
        // b picks a's channels, with other averaging: a's choice all the same.
        b.rta_set_request(RtaRequest {
            avg_ms: 100,
            ..request(0b0001)
        });
        until("a driving", &[&a, &b], || !a.rta_get_ownership().following);
        settle(&[&a, &b]);
        assert_eq!(pushes(&hub, sid_a), n, "the device already holds it");
        assert!(!a.rta_get_ownership().following);
        assert!(!b.rta_get_ownership().following);
        until("a's picture", &[&a, &b], || {
            channels(&a.rta_get_snapshot()) == [0]
        });
    }

    // -- WP ANLAST --
    /// The last choice made wins: B chooses while the event of A's earlier push
    /// still waits unread in B's queue, and B drives (before ANLAST B followed
    /// A, its own push held back). A's push after that makes B follow.
    #[test]
    fn wp_anlast_a_choice_made_while_an_earlier_push_is_unread_drives() {
        let hub = hub();
        let (a, sid_a) = console(&hub, "a", 0b0001);
        until("a's push", &[&a], || pushes(&hub, sid_a) > 0);
        let (b, sid_b) = console(&hub, "b", 0b0110);
        until("a following", &[&a, &b], || a.rta_get_ownership().following);
        settle(&[&a, &b]);

        // A chooses and its push reaches the device; B reads nothing meanwhile.
        a.rta_set_request(request(0b1000));
        until("a's choice on the device", &[&a], || {
            device_mask(&hub) == 0b1000
        });
        // B chooses with A's event still unread, then drains it.
        b.rta_set_request(request(0b0010));
        b.process_notifications();
        assert!(!b.rta_get_ownership().following, "A's push was earlier");
        until("b's choice on the device", &[&a, &b], || {
            device_mask(&hub) == 0b0010 && a.rta_get_ownership().following
        });
        settle(&[&a, &b]);
        assert!(!b.rta_get_ownership().following, "b drives");
        assert_eq!(a.rta_get_ownership().origin, sid_b);
        assert_eq!(a.rta_get_ownership().mask, 0b0010);
        assert_eq!(device_mask(&hub), 0b0010, "no fight");

        // A push of A's after B's choice: B follows, as before.
        a.rta_set_request(request(0b0100));
        until("b following a", &[&a, &b], || {
            b.rta_get_ownership().following
        });
        settle(&[&a, &b]);
        assert_eq!(b.rta_get_ownership().origin, sid_a);
        assert!(!a.rta_get_ownership().following);
        assert_eq!(device_mask(&hub), 0b0100);
    }

    // -- WP ANLAST --
    /// Two consoles choosing at once agree on one winner, the one whose push
    /// the device applied last, and neither believes it drives while it
    /// follows the other.
    #[test]
    fn wp_anlast_two_consoles_choosing_at_once_converge() {
        let hub = hub();
        let (a, sid_a) = console(&hub, "a", 0b0001);
        until("a's push", &[&a], || pushes(&hub, sid_a) > 0);
        let (b, sid_b) = console(&hub, "b", 0b0110);
        until("a following", &[&a, &b], || a.rta_get_ownership().following);
        settle(&[&a, &b]);
        for round in 0..6u16 {
            let (ma, mb) = if round % 2 == 0 {
                (0b1000, 0b0010)
            } else {
                (0b0100, 0b0011)
            };
            a.rta_set_request(request(ma));
            b.rta_set_request(request(mb));
            until("one driver", &[&a, &b], || {
                a.rta_get_ownership().following != b.rta_get_ownership().following
            });
            settle(&[&a, &b]);
            let (fa, fb) = (a.rta_get_ownership(), b.rta_get_ownership());
            assert_ne!(fa.following, fb.following, "round {round}: one driver");
            let (driver_mask, follower, driver_sid) = if fa.following {
                (mb, fa, sid_b)
            } else {
                (ma, fb, sid_a)
            };
            assert_eq!(device_mask(&hub), driver_mask, "round {round}");
            assert_eq!(
                (follower.origin, follower.mask),
                (driver_sid, driver_mask),
                "round {round}"
            );
        }
    }

    // -- WP ANLASTQ --
    /// What Qt's side switch does while following, when the side switched to
    /// has nothing chosen: a watching request with no channels, then Take
    /// control. The console stops following (the Qt smoke "qp9: a choice while
    /// following takes control"), and follows the next push again.
    #[test]
    fn wp_anlastq_a_side_switch_to_nothing_while_following_takes_control() {
        let hub = hub();
        let (a, sid_a) = console(&hub, "a", 0b0001);
        until("a's push", &[&a], || pushes(&hub, sid_a) > 0);
        let (b, sid_b) = console(&hub, "b", 0b0110);
        until("a following", &[&a, &b], || a.rta_get_ownership().following);
        settle(&[&a, &b]);
        a.rta_set_request(RtaRequest {
            watching: true,
            ..request(0)
        });
        a.rta_take_control();
        settle(&[&a, &b]);
        assert!(!a.rta_get_ownership().following, "took control");
        assert!(!a.lock().rta.sharing.pending());
        assert!(
            !b.rta_get_ownership().following,
            "nothing of a's was pushed"
        );
        assert_eq!(pushes(&hub, sid_a), 1);
        // b's next push is followed.
        b.rta_set_request(request(0b1000));
        until("a following b again", &[&a, &b], || {
            a.rta_get_ownership().following && a.rta_get_ownership().origin == sid_b
        });
    }
}
