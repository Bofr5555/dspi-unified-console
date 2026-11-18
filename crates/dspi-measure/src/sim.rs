//! The simulated acoustic world, feature `sim` (WP P5SIM).
//!
//! A deterministic stand-in for the real thing, so every measurement test, the
//! demo device (P5DEMO) and the web e2e's fake microphone can run without a
//! DSPi, a speaker or a microphone (PLAN-P5 section 4):
//!
//! - **the device side:** each output's measured chain (PEQ, crossover, output
//!   gain and delay, [`ChannelChain`]) through the caller's [`SectionModel`],
//!   at the device rate; the generator's bursts ([`Stimulus`]) replace the
//!   matrix mix, so the chain's programme feed is not applied (RC1);
//! - **the acoustics:** a [`Driver`] per output (sensitivity, mechanical
//!   high-pass, roll-off, breakup peak, acoustic offset, polarity), the
//!   output's path (distance as a delay, a level), then the [`Acoustics`]: a
//!   room (modes with decay, up to two reflections), a free field (near-field),
//!   or a headphone on a [`Coupler`] with a seal leak per reseat;
//! - **the microphone:** its own response ([`Microphone`]) and a matching
//!   calibration text ([`Microphone::calibration_text`]), a noise floor;
//! - **the capture:** the microphone's clock offset in ppm against the
//!   device's, a latency, the capture rate, blocks with the frame indices a
//!   client would report, and the capture faults of review RC5e
//!   ([`CaptureFault`]): a dropped block, a repeated block, a step slip, slow
//!   ratio wander and a device-side slip. [`Capture::events`] says where each
//!   one landed and whether the frame indices show it.
//!
//! [`render`] is a pure function of its inputs: noise comes from the world's
//! seed through a counter-based generator, so the same world renders the same
//! capture on every machine (up to libm's last place).
//!
//! **Clock convention.** A microphone `ppm` above zero samples fast: one second
//! of device time is `rate_hz * (1 + ppm·1e-6)` capture frames. Capture frame
//! `k` (the microphone's own count, before any fault) samples the device time
//! `k / (rate_hz·(1 + ppm·1e-6)) - latency`, plus the wander term
//! ([`CaptureFault::RatioWander`]). [`World::capture_position`] gives that
//! mapping, the ground truth a timing test compares with.

use serde::{Deserialize, Serialize};

use crate::types::{BandSpec, Coeffs, Coupler, MeasureError, SectionModel};

mod dsp;
mod render;
mod worlds;

#[cfg(test)]
mod tests;

pub use dsp::SimFilter;
pub use render::render;
pub use worlds::{flat_chain, flat_chains, three_way_chains, two_way_chains};

/// The most outputs a world has (a run measures at most 16).
pub const MAX_SIM_OUTPUTS: usize = 16;
/// The longest device timeline [`render`] accepts, frames (120 s at 96 kHz fits).
pub const MAX_DEVICE_FRAMES: u64 = 12 * 1024 * 1024;
/// The longest capture [`render`] produces, frames (the core's capture bound,
/// 8 Mi frames, PLAN-P5 section 1.1 step 10).
pub const MAX_CAPTURE_FRAMES: u64 = 8 * 1024 * 1024;
/// The most bursts a [`Stimulus`] holds.
pub const MAX_BURSTS: usize = 4096;
/// The most filters in one list (room modes, coupler, microphone).
pub const MAX_FILTERS: usize = 16;
/// The most reflections a room has.
pub const MAX_REFLECTIONS: usize = 2;
/// The most capture faults a world has.
pub const MAX_FAULTS: usize = 16;
/// The most reseats a headphone world has.
pub const MAX_RESEATS: usize = 9;
/// The largest slip a fault may have, frames.
pub const MAX_SLIP_FRAMES: i32 = 1024 * 1024;

/// One loudspeaker driver (or a headphone's transducer).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Driver {
    /// Level at the microphone for a full-scale DAC signal in the pass band, dB
    /// (the sensitivity, the distance's loss and the microphone's gain in
    /// one; -200 to 40).
    pub sensitivity_db: f64,
    /// The mechanical (box) high-pass, Hz; 0 for none.
    pub highpass_hz: f64,
    /// Its Q (a sealed box's Qtc).
    pub highpass_q: f64,
    /// The natural high-frequency roll-off, Hz; 0 for none.
    pub rolloff_hz: f64,
    /// Its Q.
    pub rolloff_q: f64,
    /// The cone's breakup peak; `None` for none.
    pub breakup: Option<SimFilter>,
    /// The acoustic centre's offset behind the reference plane, ms (a woofer
    /// sits behind a tweeter on a flat baffle; 0 to 100).
    pub offset_ms: f64,
    /// Wired with inverted polarity.
    pub inverted: bool,
}

impl Driver {
    /// A driver that changes nothing: 0 dB, no filters, no offset.
    pub fn flat() -> Driver {
        Driver {
            sensitivity_db: 0.0,
            highpass_hz: 0.0,
            highpass_q: 0.707,
            rolloff_hz: 0.0,
            rolloff_q: 0.707,
            breakup: None,
            offset_ms: 0.0,
            inverted: false,
        }
    }

    /// The driver's filters, in order.
    pub fn filters(&self) -> Vec<SimFilter> {
        let mut f = Vec::new();
        if self.highpass_hz > 0.0 {
            f.push(SimFilter::HighPass {
                freq: self.highpass_hz,
                q: self.highpass_q,
            });
        }
        if self.rolloff_hz > 0.0 {
            f.push(SimFilter::LowPass {
                freq: self.rolloff_hz,
                q: self.rolloff_q,
            });
        }
        if let Some(b) = self.breakup {
            f.push(b);
        }
        f
    }
}

/// One output of the world: which driver it drives and how it reaches the
/// microphone.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OutputPath {
    /// A name for reports ("L low").
    pub name: String,
    /// The speaker this output belongs to (0 left, 1 right, ...): the ways of
    /// one speaker share a number.
    pub speaker: u8,
    /// The driver.
    pub driver: Driver,
    /// The time of flight to the microphone, ms (added to the driver's offset;
    /// 0 to 1000).
    pub delay_ms: f64,
    /// The path's level, dB (distance and angle; -200 to 40).
    pub gain_db: f64,
}

/// A room mode: a resonance with a decay time.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RoomMode {
    /// Frequency, Hz.
    pub freq: f64,
    /// Level at the microphone, dB (negative for a position near a node).
    pub gain_db: f64,
    /// Decay time T60, s (held to 0.02 to 2): sets the resonance's Q.
    pub t60_s: f64,
}

impl RoomMode {
    /// The mode as a peak: `Q = pi * f * T60 / ln(1000)`.
    pub fn filter(&self) -> SimFilter {
        let t60 = if self.t60_s.is_finite() {
            self.t60_s.clamp(0.02, 2.0)
        } else {
            0.02
        };
        SimFilter::Peak {
            freq: self.freq,
            // The factor first (at most 0.91): a huge finite frequency
            // must not overflow on the way (review P5-A, F5).
            q: self.freq * (std::f64::consts::PI * t60 / 1000f64.ln()),
            gain_db: self.gain_db,
        }
    }
}

/// A reflection: a delayed, scaled copy of the direct sound.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Reflection {
    /// Later than the direct sound by, ms (0.05 to 100).
    pub delay_ms: f64,
    /// Linear gain (-1 to 1; negative inverts).
    pub gain: f64,
}

/// What lies between the drivers and the microphone.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum Acoustics {
    /// No room: a near-field or anechoic measurement.
    Free,
    /// A room.
    Room {
        /// Modes (at most [`MAX_FILTERS`]).
        modes: Vec<RoomMode>,
        /// Reflections (at most [`MAX_REFLECTIONS`]).
        reflections: Vec<Reflection>,
    },
    /// Headphones on a coupler.
    Coupler {
        /// The coupler type: picks the coupler's own response
        /// ([`coupler_filters`]).
        coupler: Coupler,
        /// The seal leak per reseat: a bass loss as a second-order high-pass
        /// at this corner, Hz; 0 is a perfect seal (1 to [`MAX_RESEATS`]
        /// entries).
        leak_hz: Vec<f64>,
        /// Which reseat this render is (an index into `leak_hz`).
        reseat: usize,
    },
}

/// The coupler's own response, by parameters: **ours**, not a standard's data
/// (OD-P5p), shaped like the main features of such couplers (the ear-canal
/// resonance, a pinna notch, a sealed coupler's pressure gain).
pub fn coupler_filters(coupler: Coupler) -> Vec<SimFilter> {
    match coupler {
        Coupler::Iec60318_4 => vec![
            SimFilter::LowShelf {
                freq: 120.0,
                q: 0.7,
                gain_db: 3.0,
            },
            SimFilter::Peak {
                freq: 2800.0,
                q: 1.8,
                gain_db: 10.0,
            },
            SimFilter::Peak {
                freq: 8500.0,
                q: 3.0,
                gain_db: -6.0,
            },
        ],
        Coupler::FlatPlate => vec![
            SimFilter::Peak {
                freq: 3500.0,
                q: 1.2,
                gain_db: 5.0,
            },
            SimFilter::HighShelf {
                freq: 10000.0,
                q: 0.7,
                gain_db: -3.0,
            },
        ],
        Coupler::Hats => vec![
            SimFilter::Peak {
                freq: 2700.0,
                q: 1.5,
                gain_db: 12.0,
            },
            SimFilter::Peak {
                freq: 5000.0,
                q: 2.0,
                gain_db: 4.0,
            },
            SimFilter::Peak {
                freq: 9000.0,
                q: 4.0,
                gain_db: -8.0,
            },
        ],
    }
}

/// The measurement microphone.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Microphone {
    /// Its own response (at most [`MAX_FILTERS`] filters).
    pub response: Vec<SimFilter>,
    /// The `Sens Factor` its calibration text states, dB. The simulator does
    /// not model absolute SPL: this number only travels into the text.
    pub sens_factor_db: f64,
    /// The serial its calibration text states.
    pub serial: String,
}

/// A calibration file's layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CalFormat {
    /// miniDSP UMIK style: a quoted `"Sens Factor =…dB, AGain=0dB, SERNO: …"`
    /// line, then `frequency<TAB>dB` lines.
    Umik,
    /// REW style: `*` comment lines, then `frequency dB` lines.
    Rew,
}

impl Microphone {
    /// A microphone with a flat response.
    pub fn flat() -> Microphone {
        Microphone {
            response: Vec::new(),
            sens_factor_db: 0.0,
            serial: "SIM-0000000".to_string(),
        }
    }

    /// A typical small measurement capsule: a roll-off below 20 Hz and a
    /// high-frequency rise and roll-off.
    pub fn typical() -> Microphone {
        Microphone {
            response: vec![
                SimFilter::HighPass { freq: 14.0, q: 0.6 },
                SimFilter::Peak {
                    freq: 11000.0,
                    q: 1.2,
                    gain_db: 2.5,
                },
                SimFilter::HighShelf {
                    freq: 17000.0,
                    q: 0.7,
                    gain_db: -2.0,
                },
            ],
            sens_factor_db: -1.25,
            serial: "SIM-7000417".to_string(),
        }
    }

    /// The microphone's response at `freq` when the world runs at `fs`, dB
    /// (0 for a non-finite frequency or rate).
    pub fn response_db(&self, freq: f64, fs: f64) -> f64 {
        self.response.iter().map(|f| f.magnitude_db(freq, fs)).sum()
    }

    /// A calibration text that matches this microphone in a world rendered at
    /// the device rate `fs`: its response at 1/12 octave from 10 Hz to 20 kHz,
    /// two decimals. As in the vendors' files, the values are the
    /// microphone's deviation (what a reader subtracts).
    ///
    /// Errors: [`MeasureError::Invalid`] for a rate outside 8 to 192 kHz or a
    /// non-finite filter or sensitivity.
    pub fn calibration_text(&self, format: CalFormat, fs: f64) -> Result<String, MeasureError> {
        if !(8000.0..=192_000.0).contains(&fs) {
            return Err(MeasureError::Invalid(format!("rate {fs} Hz")));
        }
        if !self.response.iter().all(SimFilter::is_finite) || !self.sens_factor_db.is_finite() {
            return Err(MeasureError::Invalid("non-finite microphone".into()));
        }
        let serial: String = self
            .serial
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
            .take(32)
            .collect();
        let mut out = match format {
            CalFormat::Umik => format!(
                "\"Sens Factor ={:.4}dB, AGain=0dB, SERNO: {serial}\"\n",
                self.sens_factor_db
            ),
            CalFormat::Rew => {
                format!("* Simulated microphone {serial} (dspi-measure sim)\n* Freq(Hz) dB\n")
            }
        };
        let sep = match format {
            CalFormat::Umik => '\t',
            CalFormat::Rew => ' ',
        };
        // 10 Hz * 2^(i/12) up to 20 kHz.
        for i in 0..=132u32 {
            let f = 10.0 * (f64::from(i) / 12.0).exp2();
            if f > 20_000.0 {
                break;
            }
            let db = self.response_db(f, fs);
            out.push_str(&format!("{f:.3}{sep}{db:.2}\n"));
        }
        Ok(out)
    }
}

/// The microphone's clock and the client's blocks.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MicClock {
    /// The capture's nominal rate, Hz (8 to 192 kHz).
    pub rate_hz: u32,
    /// The microphone clock's offset from the device's, ppm (±1000).
    pub ppm: f64,
    /// Device time 0 is captured this much later, ms (0 to 10 000): the USB,
    /// the DAC, the host's buffers. Air is in the paths' delays.
    pub latency_ms: f64,
    /// Frames per block the client pushes (1 to 8192; 128 for an
    /// AudioWorklet).
    pub block_frames: u32,
    /// The first block's frame index as the client reports it (an
    /// AudioWorklet's `currentFrame` does not start at zero).
    pub first_frame: u64,
}

/// A capture fault (review RC5e). `at_frame` counts from the capture's first
/// frame; [`FaultEvent::frame`] adds `MicClock::first_frame`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum CaptureFault {
    /// The client loses the block holding this frame (a worklet overload): the
    /// next block's index jumps. **Visible** in the frame indices.
    DroppedBlock {
        /// A frame in the lost block.
        at_frame: u64,
    },
    /// The client pushes the block holding this frame twice, with the same
    /// index. **Visible** in the frame indices.
    RepeatedBlock {
        /// A frame in the repeated block.
        at_frame: u64,
    },
    /// The host's clock bridge drops (`frames` > 0) or repeats (`frames` < 0)
    /// microphone frames before the client counts them (a browser or PipeWire
    /// bridging the microphone's clock to another one): every later frame is
    /// shifted. **Not visible** in the frame indices.
    StepSlip {
        /// The first frame that is shifted.
        at_frame: u64,
        /// Frames dropped (positive) or repeated (negative), at most
        /// ±[`MAX_SLIP_FRAMES`].
        frames: i32,
    },
    /// The clock ratio wanders slowly (an adaptive resampler): the
    /// microphone's instantaneous offset is `ppm + amplitude_ppm·sin(2π t /
    /// period_s)` (to first order). **Not visible** in the frame indices.
    RatioWander {
        /// The wander's amplitude, ppm (at most 1000).
        amplitude_ppm: f64,
        /// Its period, s (0.1 to 100 000).
        period_s: f64,
    },
    /// The device's output stream slips at this device frame: an underrun
    /// inserts `frames` > 0 of silence, a skip drops `-frames` (a ring
    /// underrun, the pump handing over to a host stream). Every output moves
    /// together. **Not visible** in the frame indices.
    DeviceSlip {
        /// The device frame (of the stimulus' timeline) where it happens.
        at_device_frame: u64,
        /// Frames inserted (positive) or skipped (negative), at most
        /// ±[`MAX_SLIP_FRAMES`].
        frames: i32,
    },
}

/// The whole simulated world: deterministic for a given value, seed included.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct World {
    /// A name for reports and goldens.
    pub name: String,
    /// One path per device output, output 0 first (1 to [`MAX_SIM_OUTPUTS`]).
    pub outputs: Vec<OutputPath>,
    /// The room, the free field, or the coupler.
    pub acoustics: Acoustics,
    /// The microphone.
    pub mic: Microphone,
    /// The noise floor at the capture, dBFS RMS (white, -200 to 0); `None`
    /// for none.
    pub noise_dbfs: Option<f64>,
    /// The capture's clock and blocks.
    pub clock: MicClock,
    /// The capture faults (at most [`MAX_FAULTS`]).
    pub faults: Vec<CaptureFault>,
    /// The noise's seed.
    pub seed: u64,
    // -- WP P5RAMPFIX --
    /// Low-frequency rumble at the capture (traffic, ventilation, the
    /// building), as the first real microphone heard it; `None` for none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rumble: Option<Rumble>,
}

// -- WP P5RAMPFIX --
/// The most partials of a [`Rumble`].
pub const RUMBLE_PARTIALS: u32 = 24;

/// Low-frequency rumble (WP P5RAMPFIX): [`RUMBLE_PARTIALS`] sines of equal
/// level at seeded frequencies from 20 Hz to `top_hz` with seeded phases, so
/// its beats wander like a real room's; nothing above `top_hz`. A function
/// of the capture's frame index ([`rumble_at`]), so consecutive renders and
/// a client's silence between them join without a seam.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Rumble {
    /// Its level, dBFS RMS (-200 to 0).
    pub dbfs: f64,
    /// Its highest partial, Hz (25 to 400).
    pub top_hz: f64,
}

/// The rumble of `world` at capture frame `frame` (the frame index a client
/// reports, [`MicClock::first_frame`] included); 0 without rumble or for one
/// out of range.
pub fn rumble_at(world: &World, frame: u64) -> f64 {
    let Some(r) = world.rumble else {
        return 0.0;
    };
    if !(r.dbfs.is_finite() && (-200.0..=0.0).contains(&r.dbfs))
        || !(r.top_hz.is_finite() && (25.0..=400.0).contains(&r.top_hz))
    {
        return 0.0;
    }
    let fs = f64::from(world.clock.rate_hz.max(1));
    // Each partial's peak: the sum's RMS is the level.
    let amp = 10f64.powf(r.dbfs / 20.0) * (2.0 / f64::from(RUMBLE_PARTIALS)).sqrt();
    let t = frame as f64 / fs;
    (0..RUMBLE_PARTIALS)
        .map(|i| {
            let u = dsp::uniform(world.seed ^ 0x5255_4D42, u64::from(i));
            let v = dsp::uniform(world.seed ^ 0x5048_4153, u64::from(i));
            let f = 20.0 + (r.top_hz - 20.0) * u;
            // Whole turns dropped, the seeded start phase added.
            let turns = (f * t).fract() + v;
            amp * (std::f64::consts::TAU * turns).sin()
        })
        .sum()
}

/// Where a fault landed in a rendered capture.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FaultEvent {
    /// The fault.
    pub fault: CaptureFault,
    /// The first reported frame index it affects (`MicClock::first_frame`
    /// added): the lost or repeated block's first frame, the first shifted
    /// frame, the first frame of device time after a device slip, or the
    /// capture's first frame for a wander.
    pub frame: u64,
    /// Whether a client sees it in the frame indices (a gap or a repeat).
    pub visible_in_frame_indices: bool,
    /// Whether it fell inside the capture at all.
    pub in_capture: bool,
}

/// One block as a client pushes it to `measure_capture_push`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureBlock {
    /// The block's first frame index, as the client reports it.
    pub first_frame: u64,
    /// Where its samples start in [`Capture::samples`].
    pub offset: usize,
    /// Its length, frames.
    pub len: usize,
    /// A sample reached full scale (the converter clipped).
    pub clipped: bool,
}

/// A rendered capture: blocks in the order a client pushes them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Capture {
    /// The capture's nominal rate, Hz.
    pub rate_hz: u32,
    /// Every pushed sample, block after block (a repeated block twice, a
    /// dropped one not at all), clamped to ±1.
    pub samples: Vec<f32>,
    /// The blocks.
    pub blocks: Vec<CaptureBlock>,
    /// Where each of the world's faults landed, in the world's order.
    pub events: Vec<FaultEvent>,
}

impl Capture {
    /// One block's samples (empty for a block not of this capture).
    pub fn block_samples(&self, block: &CaptureBlock) -> &[f32] {
        let end = block.offset.saturating_add(block.len);
        self.samples.get(block.offset..end).unwrap_or(&[])
    }

    /// The blocks with their samples, `(first_frame, samples, clipped)`, as a
    /// client calls `measure_capture_push`.
    pub fn pushes(&self) -> impl Iterator<Item = (u64, &[f32], bool)> + '_ {
        self.blocks
            .iter()
            .map(move |b| (b.first_frame, self.block_samples(b), b.clipped))
    }

    /// The pushed samples as a mono WAV file (the web e2e's fake microphone,
    /// a replay golden). A dropped block is missing and a repeated one twice,
    /// as a client that ignored its frame indices would have recorded them.
    ///
    /// Errors: as [`crate::wav::write_wav`].
    pub fn to_wav(&self, format: crate::wav::WavFormat) -> Result<Vec<u8>, MeasureError> {
        crate::wav::write_wav(&self.samples, self.rate_hz, format)
    }
}

/// One generator burst: a signal played on one output from a device frame on.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Burst<'a> {
    /// The output (an index into [`World::outputs`]).
    pub output: usize,
    /// The device frame of its first sample.
    pub start_frame: u64,
    /// The samples, full scale ±1.
    pub samples: &'a [f32],
    /// A gain on the samples (-1 for the generator's `invert_mask`; -16 to 16).
    pub gain: f32,
}

/// What the generator plays: bursts on a device timeline of `frames` frames
/// (the post-roll included). Bursts may share one signal, so a long walked run
/// costs the memory of one sweep.
#[derive(Debug, Clone, PartialEq)]
pub struct Stimulus<'a> {
    /// The bursts (at most [`MAX_BURSTS`]).
    pub bursts: Vec<Burst<'a>>,
    /// The timeline's length, device frames (at most [`MAX_DEVICE_FRAMES`]).
    pub frames: u64,
}

impl<'a> Stimulus<'a> {
    /// A walked run as the generator plays it with `SIGGEN_FLAG_WALK`: the
    /// outputs in turn, one per cycle, `passes` times; a cycle is the signal
    /// plus `gap_frames`, with no gap after the last cycle (`siggen.c:685`);
    /// `start_frame` frames of silence before (the pre-roll) and
    /// `post_frames` after (the post-roll).
    ///
    /// Errors: [`MeasureError::Invalid`] for no outputs, more than
    /// [`MAX_SIM_OUTPUTS`], zero passes, more than [`MAX_BURSTS`] cycles or a
    /// timeline past [`MAX_DEVICE_FRAMES`].
    pub fn walked(
        signal: &'a [f32],
        outputs: &[usize],
        passes: u32,
        gap_frames: u64,
        start_frame: u64,
        post_frames: u64,
    ) -> Result<Stimulus<'a>, MeasureError> {
        if outputs.is_empty() || outputs.len() > MAX_SIM_OUTPUTS || passes == 0 {
            return Err(MeasureError::Invalid(
                "a walked run needs 1 to 16 outputs and a pass".into(),
            ));
        }
        let cycles = (outputs.len() as u64).saturating_mul(u64::from(passes));
        if cycles > MAX_BURSTS as u64 {
            return Err(MeasureError::Invalid(format!("{cycles} cycles")));
        }
        let cycle = (signal.len() as u64).saturating_add(gap_frames);
        let mut bursts = Vec::new();
        let mut at = start_frame;
        for _ in 0..passes {
            for &output in outputs {
                bursts.push(Burst {
                    output,
                    start_frame: at,
                    samples: signal,
                    gain: 1.0,
                });
                at = at.saturating_add(cycle);
            }
        }
        let frames = at.saturating_sub(gap_frames).saturating_add(post_frames);
        if frames > MAX_DEVICE_FRAMES {
            return Err(MeasureError::Invalid(format!(
                "a timeline of {frames} frames (at most {MAX_DEVICE_FRAMES})"
            )));
        }
        Ok(Stimulus { bursts, frames })
    }

    /// A summed run: the signal on every listed output at once, each with its
    /// polarity (`true` inverted, as the generator's `invert_mask`).
    ///
    /// Errors: [`MeasureError::Invalid`] for no outputs, more than
    /// [`MAX_SIM_OUTPUTS`] or a timeline past [`MAX_DEVICE_FRAMES`].
    pub fn summed(
        signal: &'a [f32],
        outputs: &[(usize, bool)],
        start_frame: u64,
        post_frames: u64,
    ) -> Result<Stimulus<'a>, MeasureError> {
        if outputs.is_empty() || outputs.len() > MAX_SIM_OUTPUTS {
            return Err(MeasureError::Invalid(
                "a summed run needs 1 to 16 outputs".into(),
            ));
        }
        let bursts = outputs
            .iter()
            .map(|&(output, inverted)| Burst {
                output,
                start_frame,
                samples: signal,
                gain: if inverted { -1.0 } else { 1.0 },
            })
            .collect();
        let frames = start_frame
            .saturating_add(signal.len() as u64)
            .saturating_add(post_frames);
        if frames > MAX_DEVICE_FRAMES {
            return Err(MeasureError::Invalid(format!("{frames} frames")));
        }
        Ok(Stimulus { bursts, frames })
    }
}

/// The simulator's own test stimulus (P5TIME's model of `siggen.c` replaces
/// it from wave P5-B on): an exponential sine sweep from `f1` to `f2` Hz over
/// `duration_s` at `rate_hz`, with raised-cosine fades of `fade_ms` at both
/// ends, peak `amplitude` (0 to 1).
///
/// Errors: [`MeasureError::Invalid`] for a rate outside 8 to 192 kHz, a
/// duration outside 0.01 to 15 s, `0 < f1 < f2 <= rate/2` not holding, a fade
/// longer than half the sweep or an amplitude outside 0 to 1.
pub fn test_sweep(
    rate_hz: u32,
    f1: f64,
    f2: f64,
    duration_s: f64,
    fade_ms: f64,
    amplitude: f64,
) -> Result<Vec<f32>, MeasureError> {
    let fs = f64::from(rate_hz);
    let ok = (8000..=192_000).contains(&rate_hz)
        && (0.01..=15.0).contains(&duration_s)
        && f1.is_finite()
        && f2.is_finite()
        && f1 > 0.0
        && f2 > f1
        && f2 <= fs / 2.0
        && (0.0..=duration_s * 500.0).contains(&fade_ms)
        && (0.0..=1.0).contains(&amplitude);
    if !ok {
        return Err(MeasureError::Invalid("bad sweep parameters".into()));
    }
    // At most 15 s at 192 kHz: bounded.
    let n = (duration_s * fs).round() as usize;
    let k = (f2 / f1).ln();
    let fade = (fade_ms * fs / 1000.0).round();
    let last = n as f64 - 1.0;
    Ok((0..n)
        .map(|i| {
            let t = i as f64 / fs;
            let phase = 2.0 * std::f64::consts::PI * f1 * duration_s / k
                * ((t * k / duration_s).exp() - 1.0);
            let i_f = i as f64;
            let tail = last - i_f;
            let w = if fade > 0.0 && i_f < fade {
                0.5 - 0.5 * (std::f64::consts::PI * i_f / fade).cos()
            } else if fade > 0.0 && tail < fade {
                0.5 - 0.5 * (std::f64::consts::PI * tail.max(0.0) / fade).cos()
            } else {
                1.0
            };
            (amplitude * w * phase.sin()) as f32
        })
        .collect())
}

/// A [`SectionModel`] over `dspi_proto`'s graph maths at 48 kHz: the PEQ
/// shapes 1 to 10, 12 and 13 (`dsp::coefficients`) and the crossover types 32
/// to 63 (`xover::sections`). The Linkwitz Transform (11) and unknown types
/// are not modelled. For tests and the demo; the core's own cascade (P5DOCK)
/// is the real one.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ProtoModel;

impl SectionModel for ProtoModel {
    fn sections(&self, band: &BandSpec) -> Vec<Coeffs> {
        if band.bypass || band.filter_type == 0 || !self.models(band) {
            return Vec::new();
        }
        if dspi_proto::xover::meta(band.filter_type).is_some() {
            return dspi_proto::xover::sections(band.filter_type, band.freq);
        }
        let b = dspi_proto::dsp::Band {
            filter_type: dspi_proto::enums::FilterType::from_raw(band.filter_type),
            freq: band.freq,
            q: band.q,
            gain_db: band.gain,
            bypass: false,
        };
        vec![dspi_proto::dsp::coefficients(&b)]
    }

    fn models(&self, band: &BandSpec) -> bool {
        matches!(band.filter_type, 0..=10 | 12 | 13 | 32..=63)
    }

    fn sample_rate(&self) -> f64 {
        dspi_proto::dsp::SAMPLE_RATE
    }
}
