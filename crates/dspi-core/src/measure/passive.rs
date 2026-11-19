//! The passive microphone RTA (WP P5CURV, PLAN-P5 T5 and section 1.5): the
//! microphone RTA of programme audio, with no stimulus. It is what a client
//! in read-only mode, on a read-only connection or a Viewer through a hub is
//! offered instead of a measurement.
//!
//! It **never touches the device**: no generator, no lock, no read, no write.
//! The client pushes its microphone's blocks ([`DspiCore::measure_rta_passive_push`])
//! and reads the spectrum ([`DspiCore::measure_rta_passive_frame`]). The maths
//! is the live view's (P5LIVE): `dspi_measure::rta::MicRta` with the same FFT
//! length, averaging presets, hold and compare, the microphone's calibration
//! and dB SPL scale from its `mic` document, and the frame's curves made by
//! the live view's own `curve` (the live display grid, so a client draws both
//! the same way). Nothing is kept past [`DspiCore::measure_rta_passive_stop`],
//! and no audio is kept at all, only the running average (S12).
//!
//! It is independent of a measurement session and the live view: neither
//! starts, stops nor blocks the other. Its state is a leaf lock (nothing is
//! taken under it) and it calls no listener.

use std::sync::MutexGuard;

use dspi_measure::doc::MicDoc;
use dspi_measure::rta::{Averaging, MicRta, RtaConfig};
use dspi_measure::types::Response;
use serde::{Deserialize, Serialize};

use super::live::curve;
use super::{MeasureLiveRta, MeasureRtaAveraging, MeasureSlot};
use crate::{DspiCore, DspiError, Result};

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_p5curv;

/// The FFT length, as the live view's RTA.
const FFT_LEN: usize = 16_384;
/// The capture rate when the setup names none, Hz.
const RATE_DEFAULT: u32 = 48_000;
/// The capture rates accepted, Hz (as a measurement's).
const RATE_RANGE: std::ops::RangeInclusive<u32> = 8_000..=192_000;
/// Bands per octave when the setup names none.
const BANDS_DEFAULT: u8 = 3;

/// What the passive RTA analyses (`measure_rta_passive_start`). Every number
/// 0 (or absent) is the default.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct MeasureRtaPassiveSetup {
    /// The microphone's `mic` document's text; "" for an uncalibrated one
    /// (dBFS). A calibrated one gives dB SPL when its scale is known.
    pub mic_document: String,
    /// The capture's rate, Hz (8000 to 192000; 0: 48000).
    pub capture_rate_hz: u32,
    /// The averaging preset.
    pub averaging: MeasureRtaAveraging,
    /// Bands per octave (1 to 48; 0: 3).
    pub bands_per_octave: u8,
}

/// Where the passive RTA stands.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MeasureRtaPassiveState {
    /// It runs.
    pub active: bool,
    /// The capture's rate, Hz.
    pub capture_rate_hz: u32,
    /// Bands per octave.
    pub bands_per_octave: u8,
    /// The averaging preset.
    pub averaging: MeasureRtaAveraging,
    /// FFT frames averaged since the start (or the last change of averaging).
    pub frames: u64,
    /// Non-finite samples seen (analysed as silence).
    pub nonfinite_samples: u64,
    /// A spectrum is held for comparison.
    pub held: bool,
    /// The spectrum is dB SPL (a calibrated microphone), else dBFS.
    pub spl: bool,
}

/// What the passive RTA shows (`measure_rta_passive_frame`).
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MeasureRtaPassiveFrame {
    /// Its state.
    pub state: MeasureRtaPassiveState,
    /// The spectrum, the held copy and their difference, as the live view's
    /// RTA gives them; `None` when it does not run.
    pub rta: Option<MeasureLiveRta>,
}

/// The passive RTA's state (in [`MeasureSlot`]).
pub(crate) struct Passive {
    setup: MeasureRtaPassiveSetup,
    rate: u32,
    bands: u8,
    mic: Option<Response>,
    spl_offset_db: Option<f64>,
    rta: MicRta,
    held: Option<Response>,
}

impl MeasureSlot {
    fn passive_lock(&self) -> MutexGuard<'_, Option<Passive>> {
        self.passive.lock().unwrap_or_else(|p| p.into_inner())
    }
}

/// The preset's averaging, as the live view's RTA has it.
fn averaging_of(a: MeasureRtaAveraging) -> Averaging {
    match a {
        MeasureRtaAveraging::Fast => Averaging::FAST,
        MeasureRtaAveraging::Slow => Averaging::SLOW,
        MeasureRtaAveraging::Infinite => Averaging::INFINITE,
    }
}

fn invalid(msg: impl Into<String>) -> DspiError {
    DspiError::InvalidArgument { msg: msg.into() }
}

impl Passive {
    fn new(setup: MeasureRtaPassiveSetup) -> Result<Passive> {
        let rate = match setup.capture_rate_hz {
            0 => RATE_DEFAULT,
            r if RATE_RANGE.contains(&r) => r,
            r => return Err(invalid(format!("a capture rate of {r} Hz"))),
        };
        let bands = match setup.bands_per_octave {
            0 => BANDS_DEFAULT,
            b if b <= dspi_measure::rta::MAX_BANDS_PER_OCTAVE => b,
            b => return Err(invalid(format!("{b} RTA bands per octave (1 to 48)"))),
        };
        let (mic, spl_offset_db) = if setup.mic_document.trim().is_empty() {
            (None, None)
        } else {
            let doc = MicDoc::from_text(&setup.mic_document)
                .map_err(|e| invalid(format!("the microphone document: {e}")))?;
            (Some(doc.mic.correction()), doc.mic.spl_scale().offset_db)
        };
        let rta = Self::rta(rate, bands, setup.averaging, mic.as_ref(), spl_offset_db)?;
        Ok(Passive {
            setup,
            rate,
            bands,
            mic,
            spl_offset_db,
            rta,
            held: None,
        })
    }

    fn rta(
        rate: u32,
        bands: u8,
        averaging: MeasureRtaAveraging,
        mic: Option<&Response>,
        spl_offset_db: Option<f64>,
    ) -> Result<MicRta> {
        let cfg = RtaConfig {
            sample_rate: f64::from(rate),
            fft_len: FFT_LEN,
            bands_per_octave: bands,
            averaging: averaging_of(averaging),
        };
        MicRta::new(cfg, mic, spl_offset_db).map_err(|e| invalid(e.to_string()))
    }

    fn state(&self) -> MeasureRtaPassiveState {
        MeasureRtaPassiveState {
            active: true,
            capture_rate_hz: self.rate,
            bands_per_octave: self.bands,
            averaging: self.setup.averaging,
            frames: self.rta.frames(),
            nonfinite_samples: self.rta.nonfinite_samples(),
            held: self.held.is_some(),
            spl: self.spl_offset_db.is_some(),
        }
    }

    fn rta_view(&self) -> MeasureLiveRta {
        let current = self.rta.spectrum();
        let compare = current
            .as_ref()
            .zip(self.held.as_ref())
            .and_then(|(c, h)| dspi_measure::rta::compare(c, h).ok());
        MeasureLiveRta {
            current: current.as_ref().and_then(|s| curve(s, 0)),
            held: self.held.as_ref().and_then(|s| curve(s, 0)),
            compare: compare.as_ref().and_then(|s| curve(s, 0)),
            frames: self.rta.frames(),
            averaging: self.setup.averaging,
        }
    }
}

#[cfg_attr(feature = "uniffi", uniffi::export)]
impl DspiCore {
    /// Start the passive microphone RTA (programme audio, no generator): it
    /// never touches the device, so it is allowed in the client read-only
    /// mode, on a read-only connection, for a Viewer through a hub and with
    /// no device at all. A running one starts again with `setup` (its held
    /// copy dropped). Then push the microphone's blocks with
    /// `measure_rta_passive_push` and read `measure_rta_passive_frame`.
    ///
    /// Errors: [`DspiError::InvalidArgument`] for a bad setup (a rate, a
    /// band count, a microphone document that does not read).
    pub fn measure_rta_passive_start(
        &self,
        setup: MeasureRtaPassiveSetup,
    ) -> Result<MeasureRtaPassiveState> {
        let p = Passive::new(setup)?;
        let state = p.state();
        *self.measure.passive_lock() = Some(p);
        Ok(state)
    }

    /// Push one block of the microphone's capture (mono, full scale ±1, at
    /// `rate_hz`) into the passive RTA, analysed at once (no session, no
    /// safety check waits on it). A push while it does not run is ignored.
    ///
    /// Errors: [`DspiError::InvalidArgument`] for a rate other than the
    /// started one, or more than 1 Mi samples in one block (nothing is
    /// analysed).
    pub fn measure_rta_passive_push(
        &self,
        samples: Vec<f32>,
        rate_hz: u32,
    ) -> Result<MeasureRtaPassiveState> {
        let mut guard = self.measure.passive_lock();
        let Some(p) = guard.as_mut() else {
            return Ok(MeasureRtaPassiveState::default());
        };
        if rate_hz != p.rate {
            return Err(invalid(format!(
                "capture at {rate_hz} Hz; the passive RTA was started at {} Hz",
                p.rate
            )));
        }
        p.rta.push(&samples).map_err(|e| invalid(e.to_string()))?;
        Ok(p.state())
    }

    /// What the passive RTA shows: the averaged spectrum in fractional-octave
    /// bands (dBFS, or dB SPL), the held copy and their difference. No I/O.
    pub fn measure_rta_passive_frame(&self) -> MeasureRtaPassiveFrame {
        match self.measure.passive_lock().as_ref() {
            Some(p) => MeasureRtaPassiveFrame {
                state: p.state(),
                rta: Some(p.rta_view()),
            },
            None => MeasureRtaPassiveFrame::default(),
        }
    }

    /// The passive RTA's state. No I/O.
    pub fn measure_rta_passive_state(&self) -> MeasureRtaPassiveState {
        self.measure
            .passive_lock()
            .as_ref()
            .map(Passive::state)
            .unwrap_or_default()
    }

    /// Hold (`true`) the spectrum for comparison, or drop the held copy
    /// (`false`).
    ///
    /// Errors: [`DspiError::InvalidArgument`] when it does not run, or there
    /// is no spectrum to hold yet.
    pub fn measure_rta_passive_hold(&self, on: bool) -> Result<MeasureRtaPassiveState> {
        let mut guard = self.measure.passive_lock();
        let p = guard
            .as_mut()
            .ok_or_else(|| invalid("the passive RTA is not running"))?;
        if on {
            p.held = Some(
                p.rta
                    .spectrum()
                    .ok_or_else(|| invalid("no spectrum to hold yet"))?,
            );
        } else {
            p.held = None;
        }
        Ok(p.state())
    }

    /// The averaging preset; the average starts again (the held copy is
    /// kept).
    ///
    /// Errors: [`DspiError::InvalidArgument`] when it does not run.
    pub fn measure_rta_passive_averaging(
        &self,
        averaging: MeasureRtaAveraging,
    ) -> Result<MeasureRtaPassiveState> {
        let mut guard = self.measure.passive_lock();
        let p = guard
            .as_mut()
            .ok_or_else(|| invalid("the passive RTA is not running"))?;
        p.rta = Passive::rta(p.rate, p.bands, averaging, p.mic.as_ref(), p.spl_offset_db)?;
        p.setup.averaging = averaging;
        Ok(p.state())
    }

    /// Stop the passive RTA and drop its average and held copy: the state
    /// (inactive; never a failure).
    pub fn measure_rta_passive_stop(&self) -> MeasureRtaPassiveState {
        *self.measure.passive_lock() = None;
        MeasureRtaPassiveState::default()
    }
}
