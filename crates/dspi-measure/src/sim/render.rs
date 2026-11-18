//! [`render`]: the simulated world from the generator's bursts to the blocks a
//! client pushes (WP P5SIM).

use super::dsp::{Kernel, SimFilter, coeffs_finite, normal, run_biquad};
use super::{
    Acoustics, Capture, CaptureBlock, CaptureFault, FaultEvent, MAX_BURSTS, MAX_CAPTURE_FRAMES,
    MAX_DEVICE_FRAMES, MAX_FAULTS, MAX_FILTERS, MAX_REFLECTIONS, MAX_RESEATS, MAX_SIM_OUTPUTS,
    MAX_SLIP_FRAMES, Stimulus, World, coupler_filters,
};
use crate::types::{BandSpec, ChannelChain, MeasureError, SectionModel};

/// Frames per chunk of the delay stage's silence check.
const SKIP_CHUNK: usize = 1024;

/// The most bands a chain's PEQ or crossover bank may hold here.
const MAX_CHAIN_BANDS: usize = 32;

fn invalid(what: impl Into<String>) -> MeasureError {
    MeasureError::Invalid(what.into())
}

fn in_range(v: f64, lo: f64, hi: f64) -> bool {
    v.is_finite() && v >= lo && v <= hi
}

fn db_to_lin(db: f64) -> f64 {
    10f64.powf(db / 20.0)
}

/// Render what the microphone captures while the generator plays `stimulus`
/// on a device whose outputs carry `chains` (one per [`World::outputs`]), the
/// chains' filters taken from `model` at `device_rate_hz`.
///
/// The device side runs at the device rate: each output's bursts, its chain
/// (PEQ and crossover sections from `model`, output gain, output delay rounded
/// to whole samples as the device runs it), device-side slips, then its
/// driver and path (fractional delays by a windowed-sinc kernel). The
/// acoustics and the microphone act on the sum; the capture then samples it on
/// the microphone's clock ([`World::capture_position`]), adds the noise, clips
/// at full scale, applies the host-side faults and cuts blocks.
///
/// Deterministic: the same inputs give the same capture.
///
/// Errors: [`MeasureError::Invalid`] for any parameter outside its documented
/// range or non-finite, a chain count other than the world's outputs, a band
/// `model` cannot model, a model whose rate is not the device rate when a
/// chain has sections, a burst on an output the world lacks, a timeline past
/// [`MAX_DEVICE_FRAMES`] or a capture past [`MAX_CAPTURE_FRAMES`], and a chain
/// whose sections are not stable (non-finite output).
pub fn render(
    world: &World,
    stimulus: &Stimulus<'_>,
    chains: &[ChannelChain],
    model: &dyn SectionModel,
    device_rate_hz: u32,
) -> Result<Capture, MeasureError> {
    validate_world(world)?;
    validate_run(world, stimulus, chains, model, device_rate_hz)?;
    let fd = f64::from(device_rate_hz);
    // Bounded by MAX_DEVICE_FRAMES (checked above).
    let len = stimulus.frames as usize;
    // The capture's size is known before anything is allocated.
    let plan = plan_capture(world, len, fd)?;
    let kernel = Kernel::new(1.0);

    let mut acc = vec![0f32; len];
    let mut y = vec![0f32; len];
    for (o, (path, chain)) in world.outputs.iter().zip(chains).enumerate() {
        y.iter_mut().for_each(|v| *v = 0.0);
        let mut any = false;
        for b in stimulus.bursts.iter().filter(|b| b.output == o) {
            any = true;
            let start = usize::try_from(b.start_frame).unwrap_or(usize::MAX);
            for (dst, src) in y.iter_mut().skip(start).zip(b.samples) {
                *dst += src * b.gain;
            }
        }
        if !any {
            continue;
        }
        // The device chain: PEQ then crossover (linear, so the order does not
        // change the result), output gain, output delay in whole samples.
        for band in chain.peq.iter().chain(&chain.crossover) {
            for c in model.sections(band) {
                if !coeffs_finite(&c) {
                    return Err(invalid("the model gave a non-finite section"));
                }
                run_biquad(&c, &mut y);
            }
        }
        let delay = (f64::from(chain.delay_ms) * fd / 1000.0).round() as usize;
        shift_right(&mut y, delay);
        // The device's DAC stream slips: every output alike.
        for f in &world.faults {
            if let CaptureFault::DeviceSlip {
                at_device_frame,
                frames,
            } = *f
            {
                device_slip(&mut y, at_device_frame, frames);
            }
        }
        // The driver and its path.
        for f in path.driver.filters() {
            run_biquad(&f.design(fd), &mut y);
        }
        let sign = if path.driver.inverted { -1.0 } else { 1.0 };
        let gain = sign
            * db_to_lin(f64::from(chain.gain_db))
            * db_to_lin(path.driver.sensitivity_db + path.gain_db);
        let d = (path.driver.offset_ms + path.delay_ms) * fd / 1000.0;
        add_delayed(&mut acc, &y, d, gain, &kernel);
    }

    // The acoustics on the sum.
    let mut out = match &world.acoustics {
        Acoustics::Free => acc,
        Acoustics::Room { modes, reflections } => {
            for m in modes {
                run_biquad(&m.filter().design(fd), &mut acc);
            }
            if reflections.is_empty() {
                acc
            } else {
                y.copy_from_slice(&acc);
                for r in reflections {
                    add_delayed(&mut y, &acc, r.delay_ms * fd / 1000.0, r.gain, &kernel);
                }
                y
            }
        }
        Acoustics::Coupler {
            coupler,
            leak_hz,
            reseat,
        } => {
            for f in coupler_filters(*coupler) {
                run_biquad(&f.design(fd), &mut acc);
            }
            let leak = leak_hz.get(*reseat).copied().unwrap_or(0.0);
            if leak > 0.0 {
                let hp = SimFilter::HighPass { freq: leak, q: 0.6 };
                run_biquad(&hp.design(fd), &mut acc);
            }
            acc
        }
    };
    for f in &world.mic.response {
        run_biquad(&f.design(fd), &mut out);
    }
    if !out.iter().all(|v| v.is_finite()) {
        return Err(invalid("the chain is not stable"));
    }
    capture(world, &out, &plan)
}

/// The capture's clock, kernel and size.
struct CapturePlan {
    clock: ClockMap,
    kernel: Kernel,
    /// Step slips by the first frame they shift.
    slips: Vec<(u64, i64)>,
    /// Blocks.
    blocks_n: usize,
    /// Frames (`blocks_n` whole blocks).
    total: usize,
}

fn plan_capture(world: &World, len: usize, fd: f64) -> Result<CapturePlan, MeasureError> {
    let clock = ClockMap::new(world, fd);
    let fc = f64::from(world.clock.rate_hz);
    let kernel = Kernel::new((fc / fd).min(1.0));
    let block = u64::from(world.clock.block_frames.max(1));
    let mut slips: Vec<(u64, i64)> = world
        .faults
        .iter()
        .filter_map(|f| match *f {
            CaptureFault::StepSlip { at_frame, frames } => Some((at_frame, i64::from(frames))),
            _ => None,
        })
        .collect();
    slips.sort_by_key(|s| s.0);
    let repeated: u64 = slips
        .iter()
        .map(|s| if s.1 < 0 { s.1.unsigned_abs() } else { 0 })
        .sum();
    // Frames until the device timeline and the kernel's tail have been heard.
    let k_end = clock.inverse(len as f64 + kernel.half() as f64).ceil();
    if !(k_end.is_finite() && k_end >= 0.0 && k_end <= MAX_CAPTURE_FRAMES as f64) {
        return Err(invalid(format!(
            "the capture would pass {MAX_CAPTURE_FRAMES} frames"
        )));
    }
    let needed = (k_end as u64).saturating_add(repeated).max(1);
    let blocks_n = needed.div_ceil(block);
    let total = blocks_n.saturating_mul(block);
    if total > MAX_CAPTURE_FRAMES {
        return Err(invalid(format!(
            "a capture of {total} frames (at most {MAX_CAPTURE_FRAMES})"
        )));
    }
    // Both bounded by MAX_CAPTURE_FRAMES.
    Ok(CapturePlan {
        clock,
        kernel,
        slips,
        blocks_n: blocks_n as usize,
        total: total as usize,
    })
}

/// Delay `y` by `n` whole samples in place (zeros shifted in).
fn shift_right(y: &mut [f32], n: usize) {
    if n == 0 {
        return;
    }
    let len = y.len();
    if n >= len {
        y.iter_mut().for_each(|v| *v = 0.0);
        return;
    }
    y.copy_within(0..len.saturating_sub(n), n);
    y.iter_mut().take(n).for_each(|v| *v = 0.0);
}

/// The device's output slips at `at` by `frames`: inserted silence (> 0) or
/// skipped samples (< 0); the timeline's length is kept.
fn device_slip(y: &mut [f32], at: u64, frames: i32) {
    let len = y.len();
    let Ok(at) = usize::try_from(at) else {
        return;
    };
    if at >= len || frames == 0 {
        return;
    }
    let n = frames.unsigned_abs() as usize;
    if frames > 0 {
        let end = at.saturating_add(n).min(len);
        if end < len {
            y.copy_within(at..len.saturating_sub(n), end);
        }
        if let Some(gap) = y.get_mut(at..end) {
            gap.iter_mut().for_each(|v| *v = 0.0);
        }
    } else {
        let from = at.saturating_add(n);
        if from < len {
            y.copy_within(from..len, at);
            let tail = len.saturating_sub(n);
            if let Some(t) = y.get_mut(tail..) {
                t.iter_mut().for_each(|v| *v = 0.0);
            }
        } else if let Some(t) = y.get_mut(at..) {
            t.iter_mut().for_each(|v| *v = 0.0);
        }
    }
}

/// `dst[t] += gain * src(t - d)` with a fractional delay `d >= 0` (samples).
fn add_delayed(dst: &mut [f32], src: &[f32], d: f64, gain: f64, kernel: &Kernel) {
    // Only where src is not zero (plus the kernel's reach) can dst change.
    let Some(first) = src.iter().position(|v| *v != 0.0) else {
        return;
    };
    let last = src.iter().rposition(|v| *v != 0.0).unwrap_or(first);
    let n = d.floor();
    let frac = d - n;
    if frac == 0.0 {
        let n = n as usize;
        for (t, s) in src
            .iter()
            .enumerate()
            .skip(first)
            .take(last.saturating_sub(first).saturating_add(1))
        {
            if let Some(v) = dst.get_mut(t.saturating_add(n)) {
                *v = (f64::from(*v) + gain * f64::from(*s)) as f32;
            }
        }
        return;
    }
    // The fraction is the same for every t: one set of weights.
    let w: Vec<f64> = kernel.weights(1.0 - frac);
    // t - d = (t - n - 1) + (1 - frac).
    let reach = kernel.half() as f64;
    let lo = (first as f64 + d - reach).floor().max(0.0) as usize;
    let hi = (last as f64 + d + reach).ceil().max(0.0) as usize;
    let span = hi.saturating_sub(lo).saturating_add(1);
    let reach = kernel.half().saturating_add(1);
    let shift = n as usize;
    let end = lo.saturating_add(span).min(dst.len());
    let Some(region) = dst.get_mut(lo..end) else {
        return;
    };
    // In chunks, skipping those whose source is silent (a walked run plays
    // each output a fraction of the time).
    for (ci, chunk) in region.chunks_mut(SKIP_CHUNK).enumerate() {
        let t0 = lo.saturating_add(ci.saturating_mul(SKIP_CHUNK));
        let t1 = t0.saturating_add(chunk.len());
        let s0 = t0
            .saturating_sub(shift)
            .saturating_sub(reach)
            .min(src.len());
        let s1 = t1
            .saturating_sub(shift)
            .saturating_add(reach)
            .min(src.len());
        if src.get(s0..s1).is_none_or(|s| s.iter().all(|v| *v == 0.0)) {
            continue;
        }
        for (k, v) in chunk.iter_mut().enumerate() {
            let t = t0.saturating_add(k) as f64;
            let x = kernel.apply(src, t - n - 1.0, &w);
            *v = (f64::from(*v) + gain * x) as f32;
        }
    }
}

/// The microphone's clock against the device's.
pub(crate) struct ClockMap {
    /// Device rate.
    fd: f64,
    /// Capture rate (nominal).
    fc: f64,
    /// Constant offset, fraction (ppm·1e-6).
    e0: f64,
    /// Latency, device frames.
    lat: f64,
    /// Wander terms: amplitude (fraction), period (capture frames).
    wander: Vec<(f64, f64)>,
}

impl ClockMap {
    pub(crate) fn new(world: &World, fd: f64) -> ClockMap {
        let fc = f64::from(world.clock.rate_hz);
        let wander = world
            .faults
            .iter()
            .filter_map(|f| match *f {
                CaptureFault::RatioWander {
                    amplitude_ppm,
                    period_s,
                } => Some((amplitude_ppm * 1e-6, period_s * fc)),
                _ => None,
            })
            .collect();
        ClockMap {
            fd,
            fc,
            e0: world.clock.ppm * 1e-6,
            lat: world.clock.latency_ms * fd / 1000.0,
            wander,
        }
    }

    /// The device position (frames) capture frame `k` samples.
    pub(crate) fn pos(&self, k: f64) -> f64 {
        let r = self.fd / self.fc;
        let mut p = r * k / (1.0 + self.e0) - self.lat;
        for &(a, per) in &self.wander {
            let w = 2.0 * std::f64::consts::PI / per;
            p -= r * a * (1.0 - (w * k).cos()) / w;
        }
        p
    }

    fn dpos(&self, k: f64) -> f64 {
        let r = self.fd / self.fc;
        let mut d = r / (1.0 + self.e0);
        for &(a, per) in &self.wander {
            let w = 2.0 * std::f64::consts::PI / per;
            d -= r * a * (w * k).sin();
        }
        d
    }

    /// The capture frame (the microphone's own count) that samples device
    /// position `p`.
    pub(crate) fn inverse(&self, p: f64) -> f64 {
        let mut k = (p + self.lat) * self.fc * (1.0 + self.e0) / self.fd;
        for _ in 0..6 {
            let d = self.dpos(k);
            if !(d.is_finite() && d > 0.0) {
                break;
            }
            k -= (self.pos(k) - p) / d;
        }
        k
    }
}

/// Sample the device-rate signal on the microphone's clock and cut the blocks.
fn capture(world: &World, sig: &[f32], plan: &CapturePlan) -> Result<Capture, MeasureError> {
    let CapturePlan {
        clock,
        kernel,
        slips,
        blocks_n,
        total,
    } = plan;
    let (blocks_n, total) = (*blocks_n, *total);
    let block = world.clock.block_frames.max(1) as usize;
    let block64 = block as u64;
    let origin = world.clock.first_frame;

    let sigma = world.noise_dbfs.map_or(0.0, db_to_lin);
    let mut frames = Vec::with_capacity(total);
    let mut clip = vec![false; blocks_n];
    let mut off: i64 = 0;
    let mut next_slip = 0usize;
    for j in 0..total {
        let ju = j as u64;
        while let Some(&(at, n)) = slips.get(next_slip) {
            if at > ju {
                break;
            }
            off = off.saturating_add(n);
            next_slip = next_slip.saturating_add(1);
        }
        let k = (j as i64).saturating_add(off);
        let mut v = if k >= 0 {
            kernel.interp(sig, clock.pos(k as f64))
        } else {
            0.0
        };
        if sigma > 0.0 {
            v += sigma * normal(world.seed, k as u64);
        }
        // -- WP P5RAMPFIX -- rumble, by the capture's own frame index.
        if world.rumble.is_some() {
            v += super::rumble_at(world, origin.wrapping_add(ju));
        }
        if v.abs() >= 1.0
            && let Some(c) = clip.get_mut(j.checked_div(block).unwrap_or(0))
        {
            *c = true;
        }
        frames.push(v.clamp(-1.0, 1.0) as f32);
    }

    let dropped: Vec<u64> = world
        .faults
        .iter()
        .filter_map(|f| match *f {
            CaptureFault::DroppedBlock { at_frame } => {
                Some(at_frame.checked_div(block64).unwrap_or(0))
            }
            _ => None,
        })
        .collect();
    let repeats: Vec<u64> = world
        .faults
        .iter()
        .filter_map(|f| match *f {
            CaptureFault::RepeatedBlock { at_frame } => {
                Some(at_frame.checked_div(block64).unwrap_or(0))
            }
            _ => None,
        })
        .collect();

    let mut samples = Vec::with_capacity(total);
    let mut blocks = Vec::with_capacity(blocks_n);
    for (b, chunk) in frames.chunks(block).enumerate() {
        let bu = b as u64;
        let first_frame = origin.saturating_add(bu.saturating_mul(block64));
        let clipped = clip.get(b).copied().unwrap_or(false);
        if dropped.contains(&bu) {
            continue;
        }
        let times = if repeats.contains(&bu) { 2 } else { 1 };
        for _ in 0..times {
            blocks.push(CaptureBlock {
                first_frame,
                offset: samples.len(),
                len: chunk.len(),
                clipped,
            });
            samples.extend_from_slice(chunk);
        }
    }

    let total_u = total as u64;
    let events = world
        .faults
        .iter()
        .map(|f| {
            let (frame, visible, inside) = match *f {
                CaptureFault::DroppedBlock { at_frame }
                | CaptureFault::RepeatedBlock { at_frame } => {
                    let b = at_frame.checked_div(block64).unwrap_or(0);
                    (b.saturating_mul(block64), true, at_frame < total_u)
                }
                CaptureFault::StepSlip { at_frame, .. } => (at_frame, false, at_frame < total_u),
                CaptureFault::RatioWander { .. } => (0, false, true),
                CaptureFault::DeviceSlip {
                    at_device_frame, ..
                } => {
                    let k = clock.inverse(at_device_frame as f64).ceil().max(0.0);
                    let inside = (at_device_frame as f64) < sig.len() as f64 && k < total as f64;
                    (k.min(total as f64) as u64, false, inside)
                }
            };
            FaultEvent {
                fault: *f,
                frame: origin.saturating_add(frame),
                visible_in_frame_indices: visible,
                in_capture: inside,
            }
        })
        .collect();

    Ok(Capture {
        rate_hz: world.clock.rate_hz,
        samples,
        blocks,
        events,
    })
}

fn filters_ok(f: &[SimFilter]) -> bool {
    f.len() <= MAX_FILTERS && f.iter().all(SimFilter::is_finite)
}

/// Every parameter of the world inside its documented range.
pub(crate) fn validate_world(w: &World) -> Result<(), MeasureError> {
    if w.outputs.is_empty() || w.outputs.len() > MAX_SIM_OUTPUTS {
        return Err(invalid("a world has 1 to 16 outputs"));
    }
    for p in &w.outputs {
        let d = &p.driver;
        let ok = in_range(d.sensitivity_db, -200.0, 40.0)
            && in_range(d.highpass_hz, 0.0, 1e6)
            && d.highpass_q.is_finite()
            && in_range(d.rolloff_hz, 0.0, 1e6)
            && d.rolloff_q.is_finite()
            && d.breakup.as_ref().is_none_or(SimFilter::is_finite)
            && in_range(d.offset_ms, 0.0, 100.0)
            && in_range(p.delay_ms, 0.0, 1000.0)
            && in_range(p.gain_db, -200.0, 40.0);
        if !ok {
            return Err(invalid(format!("output {:?} is out of range", p.name)));
        }
    }
    match &w.acoustics {
        Acoustics::Free => {}
        Acoustics::Room { modes, reflections } => {
            let ok = modes.len() <= MAX_FILTERS
                && modes
                    .iter()
                    .all(|m| m.freq.is_finite() && m.gain_db.is_finite() && m.t60_s.is_finite())
                && reflections.len() <= MAX_REFLECTIONS
                && reflections
                    .iter()
                    .all(|r| in_range(r.delay_ms, 0.05, 100.0) && in_range(r.gain, -1.0, 1.0));
            if !ok {
                return Err(invalid("the room is out of range"));
            }
        }
        Acoustics::Coupler {
            leak_hz, reseat, ..
        } => {
            let ok = !leak_hz.is_empty()
                && leak_hz.len() <= MAX_RESEATS
                && leak_hz.iter().all(|l| in_range(*l, 0.0, 1e6))
                && *reseat < leak_hz.len();
            if !ok {
                return Err(invalid("the coupler's reseats are out of range"));
            }
        }
    }
    if !filters_ok(&w.mic.response) || !w.mic.sens_factor_db.is_finite() {
        return Err(invalid("the microphone is out of range"));
    }
    if let Some(n) = w.noise_dbfs
        && !in_range(n, -200.0, 0.0)
    {
        return Err(invalid("the noise floor is out of range"));
    }
    // -- WP P5RAMPFIX --
    if let Some(r) = w.rumble
        && !(in_range(r.dbfs, -200.0, 0.0) && in_range(r.top_hz, 25.0, 400.0))
    {
        return Err(invalid("the rumble is out of range"));
    }
    let c = &w.clock;
    let ok = (8000..=192_000).contains(&c.rate_hz)
        && in_range(c.ppm, -1000.0, 1000.0)
        && in_range(c.latency_ms, 0.0, 10_000.0)
        && (1..=8192).contains(&c.block_frames);
    if !ok {
        return Err(invalid("the capture clock is out of range"));
    }
    if w.faults.len() > MAX_FAULTS {
        return Err(invalid("too many faults"));
    }
    for f in &w.faults {
        let ok = match *f {
            CaptureFault::DroppedBlock { .. } | CaptureFault::RepeatedBlock { .. } => true,
            CaptureFault::StepSlip { frames, .. } | CaptureFault::DeviceSlip { frames, .. } => {
                (-MAX_SLIP_FRAMES..=MAX_SLIP_FRAMES).contains(&frames)
            }
            CaptureFault::RatioWander {
                amplitude_ppm,
                period_s,
            } => in_range(amplitude_ppm, 0.0, 1000.0) && in_range(period_s, 0.1, 100_000.0),
        };
        if !ok {
            return Err(invalid(format!("fault {f:?} is out of range")));
        }
    }
    Ok(())
}

fn band_ok(b: &BandSpec, model: &dyn SectionModel) -> bool {
    b.bypass || b.filter_type == 0 || model.models(b)
}

fn validate_run(
    world: &World,
    stim: &Stimulus<'_>,
    chains: &[ChannelChain],
    model: &dyn SectionModel,
    rate: u32,
) -> Result<(), MeasureError> {
    if !(8000..=192_000).contains(&rate) {
        return Err(invalid(format!("device rate {rate} Hz")));
    }
    if chains.len() != world.outputs.len() {
        return Err(invalid(format!(
            "{} chains for {} outputs",
            chains.len(),
            world.outputs.len()
        )));
    }
    let mut any_sections = false;
    for c in chains {
        let ok = c.peq.len() <= MAX_CHAIN_BANDS
            && c.crossover.len() <= MAX_CHAIN_BANDS
            && in_range(f64::from(c.gain_db), -120.0, 40.0)
            && in_range(f64::from(c.delay_ms), 0.0, 1000.0);
        if !ok {
            return Err(invalid("a chain is out of range"));
        }
        for b in c.peq.iter().chain(&c.crossover) {
            if !band_ok(b, model) {
                return Err(invalid(format!(
                    "filter type {} is not modelled",
                    b.filter_type
                )));
            }
            if !b.bypass && b.filter_type != 0 {
                any_sections = true;
            }
        }
    }
    let mr = model.sample_rate();
    if any_sections && !(mr.is_finite() && (mr - f64::from(rate)).abs() <= 0.5) {
        return Err(invalid(format!(
            "the model runs at {mr} Hz, the device at {rate} Hz"
        )));
    }
    if stim.frames == 0 || stim.frames > MAX_DEVICE_FRAMES {
        return Err(invalid(format!(
            "a timeline of {} frames (1 to {MAX_DEVICE_FRAMES})",
            stim.frames
        )));
    }
    if stim.bursts.len() > MAX_BURSTS {
        return Err(invalid("too many bursts"));
    }
    let mut checked: Option<(*const f32, usize)> = None;
    for b in &stim.bursts {
        if b.output >= world.outputs.len() || !(b.gain.is_finite() && b.gain.abs() <= 16.0) {
            return Err(invalid("a burst is out of range"));
        }
        let key = (b.samples.as_ptr(), b.samples.len());
        if checked != Some(key) {
            if !b.samples.iter().all(|v| v.is_finite()) {
                return Err(invalid("a burst has a non-finite sample"));
            }
            checked = Some(key);
        }
    }
    Ok(())
}
