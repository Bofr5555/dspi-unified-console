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
//! The request (what the app wants analysed) and the listener survive a
//! reconnect, as in the old core; `rta_start` must be called again for the
//! new connection.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, Weak};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use dspi_proto::generated::opcodes as op;
use dspi_proto::packets::RtaBandFrame;
use dspi_session::rta::{self as engine, Budget, Options, Request, RtaEngine, ViewId};
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
/// Per poll: the bin frame may take a few chunks (a Link transfer is small),
/// but the lock is held for the poll, so it is bounded.
const BUDGET: Budget = Budget {
    max_bin_chunks: 4,
    max_time: Duration::from_millis(25),
};

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
    pub fn rta_set_request(&self, request: RtaRequest) {
        let signal = {
            let mut inner = self.lock();
            if inner.rta.request == request {
                return;
            }
            inner.rta.request = request;
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
}

/// The analyser's state and its worker thread (WP C9).
pub(crate) struct RtaState {
    /// The session's analyser host half; the core's subscription is `view`.
    engine: RtaEngine,
    /// The core's one subscription to the engine, while the request's mask
    /// is not 0.
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
        }
    }
}

impl RtaState {
    /// Forget the device (keeps the request and the subscription), bumping
    /// both snapshot versions so the app redraws.
    fn reset(&mut self) {
        self.engine.disconnect();
        self.caps = empty_caps();
        self.snapshot = RtaSnapshot {
            version: self.snapshot.version.wrapping_add(1),
            telemetry_version: self.snapshot.telemetry_version.wrapping_add(1),
            ..empty_snapshot()
        };
    }

    /// Feed the request into the engine: options against the caps, one
    /// subscription while the mask is not 0. Idempotent.
    fn sync_request(&mut self) {
        let req = self.request;
        let fft_order = match (req.fft_order, self.engine.caps()) {
            (0, _) => None,
            (o, Some(c)) => Some(o.clamp(c.order_min, c.order_max)),
            (o, None) => Some(o),
        };
        self.engine.set_options(Options {
            fft_order,
            avg_ms: req.avg_ms,
            peak_decay_db_s: req.peak_decay_db_s,
        });
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
    let req = st.request;
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
    s.config_rejected = e.rejected();
    s.refresh_interval_s = e.refresh_interval().as_secs_f32();
    s
}

// ── The bands-all fallback ─────────────────────────────────────────────

/// The connection's transport as the engine sees it, with the firmware's
/// USB-only all-channel band read (`0x0F`) replaced by one `0x0B` per
/// requested channel when the device refuses it (a STALL: a bridge behind
/// DSPi Link answers it ERROR; vendor_commands.c, `REQ_RTA_GET_BANDS_ALL`).
/// On a non-USB link the refusal is permanent, so `refused` remembers it and
/// later polls go per channel at once; over USB a STALL means the bulk lock
/// is busy, so the next poll tries `0x0F` again.
struct BandsFallback<'a> {
    t: &'a mut dyn Transport,
    /// The channels the engine asks for (its wanted mask).
    mask: u16,
    refused: &'a mut bool,
}

impl BandsFallback<'_> {
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

impl Transport for BandsFallback<'_> {
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
}

impl RtaWorker {
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
fn run(core: &Weak<DspiCore>, signal: &Signal, listeners: &Listeners, id: u64) {
    let mut probed = false;
    let mut refused = false;
    while !signal.stopped() {
        let Some(c) = core.upgrade() else {
            return;
        };
        let polled = {
            let mut inner = c.lock();
            poll(&mut inner, id, &mut probed, &mut refused, Instant::now())
        };
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
    // The engine is moved out for the poll, so the connection can be
    // borrowed as a `Writable` alongside it.
    let mut eng = std::mem::take(&mut inner.rta.engine);
    let alive = (|| {
        let mut w = inner.writable_conn().ok()?;
        let t = w.conn().transport();
        if !*probed {
            *probed = true;
            eng.connect(t);
            return Some(true);
        }
        let mask = eng.wanted().map_or(0, |c| c.channel_mask);
        let mut ft = BandsFallback { t, mask, refused };
        let report = eng.tick(&mut ft, now, BUDGET);
        Some(!report.disconnected)
    })();
    inner.rta.engine = eng;

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
}
