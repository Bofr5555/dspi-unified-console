//! P5 (WP FBUS, FSTORY section 5): what a demo DSPi of the function test does
//! after its matrix, for programme and test signals alike, as the firmware
//! does it (`audio_pipeline.c`, `siggen.h`): the generator replaces the mix
//! on its outputs before the output chain, so both go through
//!
//! - the output's PEQ and crossover (skipped for the generator with
//!   `SIGGEN_FLAG_RAW`), then psychoacoustic bass (pre-crossover in the
//!   firmware; the order of linear stages does not change a level);
//! - output trim × user volume × master volume, all zero when the output is
//!   muted, the user mute is on, or the master volume is at its mute floor;
//!   nothing at all on a disabled output (the generator skips it too);
//! - loudness (volume-keyed shelves, post-gain), then the output limiter.
//!
//! Volume follows the firmware's table: 1 dB steps, the step below the value
//! (`update_user_volume` floors), and the bottom step (-60 dB) is silence.
//! Loudness and psychoacoustic bass are modelled as gain (their shelves and
//! the harmonics' energy), not as their exact curves: enough for levels.
//! The limiter is a gain reduction at the signal's peak (a brickwall
//! limiter's steady state), not a sample-by-sample process.
//!
//! Test-only: the function test's scenario (`sim-scenario`) only.

use dspi_measure::types::{BandSpec, ChannelChain};
use dspi_proto::enums::FilterType;

use super::{chain_db, grid};
use crate::records::DspState;

/// What a silent output's gain is, dB.
pub(crate) const SILENT_DB: f32 = -200.0;
/// The firmware's user-volume range: -60 dB (silence) to 0 dB.
const USER_MIN_DB: f32 = -60.0;
/// The master volume's mute floor (`MASTER_VOL_MUTE_DB`).
const MASTER_MUTE_DB: f32 = -128.0;
/// `SIGGEN_FLAG_RAW`: the generator skips PEQ and crossover.
const FLAG_RAW: u8 = 0x01;
/// Loudness shelves (`loudness.c`): low at 200 Hz, high at 6 kHz, Q 0.707.
const LOUD_LOW_HZ: f32 = 200.0;
const LOUD_HIGH_HZ: f32 = 6_000.0;
/// dB of shelf per dB the volume is under the reference, at 100 % intensity:
/// ISO 226 at 50 Hz and 10 kHz, roughly, between 40 and 90 phon.
const LOUD_LOW_PER_DB: f32 = 0.25;
const LOUD_HIGH_PER_DB: f32 = 0.08;
/// The softest effective level loudness compensates for, phon.
const LOUD_MIN_PHON: f32 = 20.0;

fn bit(mask: u16, o: usize) -> bool {
    u32::try_from(o)
        .ok()
        .and_then(|b| 1u16.checked_shl(b))
        .is_some_and(|m| mask & m != 0)
}

/// The user volume the firmware applies for `db`, dB; `None`: silence.
pub(crate) fn user_volume_db(db: f32) -> Option<f64> {
    let db = if db.is_finite() { db } else { 0.0 };
    let step = (db.clamp(USER_MIN_DB, 0.0) - USER_MIN_DB).floor();
    (step >= 1.0).then(|| f64::from(step + USER_MIN_DB))
}

/// The user and master volume and the user mute together, dB; `None`:
/// silence.
pub(crate) fn volume_db(s: &DspState) -> Option<f64> {
    if s.user_mute || !s.master_volume_db.is_finite() || s.master_volume_db <= MASTER_MUTE_DB {
        return None;
    }
    Some(user_volume_db(s.user_volume_db)? + f64::from(s.master_volume_db))
}

/// Output `o` plays nothing: disabled or muted.
pub(crate) fn silent(s: &DspState, o: usize) -> bool {
    !s.output_enabled.get(o).copied().unwrap_or(false)
        || s.output_muted.get(o).copied().unwrap_or(false)
}

/// What follows output `o`'s chain, dB (volume; `None`: silence).
pub(crate) fn after_chain_db(s: &DspState, o: usize) -> Option<f64> {
    if silent(s, o) {
        return None;
    }
    volume_db(s)
}

fn shelf(filter_type: FilterType, freq: f32, gain: f32) -> BandSpec {
    BandSpec {
        filter_type: filter_type.to_raw(),
        bypass: false,
        freq,
        q: 0.707,
        gain,
        qp: 0.0,
    }
}

/// Loudness on output `o` as two shelves (none when off, masked out or at
/// the reference).
pub(crate) fn loudness_bands(s: &DspState, o: usize) -> Vec<BandSpec> {
    if !s.loudness_enabled || !bit(s.loudness_output_mask, o) {
        return Vec::new();
    }
    // Keyed on the user volume's step alone (`effective_vol_index`)
    let vol = user_volume_db(s.user_volume_db).map_or(USER_MIN_DB, |v| v as f32);
    let reference = s.loudness_ref_spl;
    let phon = (reference + vol).clamp(LOUD_MIN_PHON, reference.max(LOUD_MIN_PHON));
    let under = (reference - phon).max(0.0) * (s.loudness_intensity / 100.0).max(0.0);
    if under <= 0.0 {
        return Vec::new();
    }
    vec![
        shelf(FilterType::LowShelf, LOUD_LOW_HZ, under * LOUD_LOW_PER_DB),
        shelf(
            FilterType::HighShelf,
            LOUD_HIGH_HZ,
            under * LOUD_HIGH_PER_DB,
        ),
    ]
}

/// Psychoacoustic bass on output `o`: the original low band's gain as a
/// shelf at the cutoff, and the band its harmonics land in, with their mix
/// level, `(low Hz, high Hz, dB)` (the firmware band-limits them to the
/// cutoff and four times it). `None` when off or masked out.
pub(crate) fn psybass(s: &DspState, o: usize) -> Option<(Vec<BandSpec>, (f64, f64, f64))> {
    if !s.psybass_enabled || !bit(s.psybass_output_mask, o) || s.psybass_cutoff_hz <= 0.0 {
        return None;
    }
    let cut = s.psybass_cutoff_hz;
    let low = (s.psybass_original_db.abs() > 0.01)
        .then(|| shelf(FilterType::LowShelf, cut, s.psybass_original_db.min(0.0)))
        .into_iter()
        .collect();
    let c = f64::from(cut);
    Some((low, (c, c * 4.0, f64::from(s.psybass_harmonics_db))))
}

/// The output limiter's threshold on `o`, dBFS, when it is on.
pub(crate) fn limiter_db(s: &DspState, o: usize) -> Option<f64> {
    s.limiter_enabled
        .get(o)
        .copied()
        .unwrap_or(false)
        .then(|| f64::from(s.limiter_threshold_db.get(o).copied().unwrap_or(-1.0)))
}

/// What the generator's signal on output `o` goes through (`flags`: the
/// generator's), up to the limiter; `None` when it plays nothing there.
pub(crate) fn signal_chain(s: &DspState, o: usize, flags: u8) -> Option<ChannelChain> {
    let vol = after_chain_db(s, o)?;
    let mut c = crate::measure::chain_of(s, u8::try_from(o).ok()?);
    if flags & FLAG_RAW != 0 {
        c.peq.clear();
        c.crossover.clear();
    } else if let Some((low, _)) = psybass(s, o) {
        // Psychoacoustic bass is skipped for RAW, so are its harmonics here:
        // a sweep's are not modelled (they are its own frequency's multiples)
        c.peq.extend(low);
    }
    if flags & FLAG_RAW == 0 {
        c.peq.extend(loudness_bands(s, o));
    }
    c.gain_db = (f64::from(c.gain_db) + vol) as f32;
    Some(c)
}

/// One output's test signal at the amplifier.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct SignalPeak {
    /// The peak, dBFS, after the limiter, and where.
    pub(crate) peak_db: f64,
    pub(crate) peak_hz: f64,
    /// The peak below `below_hz`, dBFS, and where.
    pub(crate) below_db: f64,
    pub(crate) below_hz: f64,
    /// The limiter's reduction, dB (0: none).
    pub(crate) limiter_db: f64,
}

/// The band a generator signal covers: a tone at `f1`, a sweep `f1` to `f2`,
/// anything else full band.
pub(crate) fn signal_band(signal: u8, f1: f32, f2: f32) -> (f64, f64) {
    const TONE: u8 = 1;
    const PINK: u8 = 3;
    const LOG_SWEEP: u8 = 4;
    match signal {
        LOG_SWEEP => (f64::from(f1.min(f2)), f64::from(f1.max(f2))),
        PINK => (20.0, 20_000.0),
        TONE => (f64::from(f1), f64::from(f1)),
        _ => (20.0, 20_000.0),
    }
}

/// The generator's signal at `level_db` over `band` through `chain` (one of
/// [`signal_chain`]'s) and output `o`'s limiter; `below` is the driver's
/// safe band's edge, Hz (0: none).
pub(crate) fn signal_peak(
    s: &DspState,
    o: usize,
    chain: &ChannelChain,
    level_db: f32,
    band: (f64, f64),
    below: f64,
) -> SignalPeak {
    let mut p = SignalPeak {
        peak_db: f64::NEG_INFINITY,
        peak_hz: 0.0,
        below_db: f64::NEG_INFINITY,
        below_hz: 0.0,
        limiter_db: 0.0,
    };
    let (f1, f2) = band;
    for f in grid(96.0) {
        if f < f1 * 0.9999 || f > f2 * 1.0001 {
            continue;
        }
        let db = f64::from(level_db) + chain_db(chain, f);
        if db > p.peak_db {
            (p.peak_db, p.peak_hz) = (db, f);
        }
        if f < below * 0.995 && db > p.below_db {
            (p.below_db, p.below_hz) = (db, f);
        }
    }
    // A tone between two grid points
    if p.peak_db == f64::NEG_INFINITY && f1 > 0.0 {
        let db = f64::from(level_db) + chain_db(chain, f1);
        (p.peak_db, p.peak_hz) = (db, f1);
        if f1 < below * 0.995 {
            (p.below_db, p.below_hz) = (db, f1);
        }
    }
    if let Some(t) = limiter_db(s, o)
        && p.peak_db > t
    {
        p.limiter_db = p.peak_db - t;
        p.peak_db = t;
        p.below_db -= p.limiter_db;
    }
    p
}

/// The chains the simulated microphone renders a generator start through:
/// `chains` (the core's, one per output) with the device's volume, mutes,
/// RAW, loudness, psychoacoustic bass and the limiter's reduction at the
/// signal's peak. An output the generator plays nothing on gets
/// [`SILENT_DB`].
#[allow(clippy::too_many_arguments)]
pub(crate) fn signal_chains(
    s: &DspState,
    signal: u8,
    mask: u16,
    flags: u8,
    level_db: f32,
    f1: f32,
    f2: f32,
    chains: Vec<ChannelChain>,
) -> Vec<ChannelChain> {
    let band = signal_band(signal, f1, f2);
    chains
        .into_iter()
        .enumerate()
        .map(|(o, c)| {
            if !bit(mask, o) {
                return c;
            }
            match signal_chain(s, o, flags) {
                Some(mut sc) => {
                    let p = signal_peak(s, o, &sc, level_db, band, 0.0);
                    sc.gain_db = (f64::from(sc.gain_db) - p.limiter_db) as f32;
                    sc
                }
                None => ChannelChain {
                    gain_db: SILENT_DB,
                    ..c
                },
            }
        })
        .collect()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests_wp_fbus {
    use super::*;

    fn state() -> DspState {
        let mut s = crate::state::default_state();
        s.user_volume_db = 0.0;
        s.master_volume_db = 0.0;
        s.user_mute = false;
        s.loudness_enabled = false;
        s.psybass_enabled = false;
        s.output_enabled.fill(true);
        s.output_muted.fill(false);
        s.output_gain_db.fill(0.0);
        s.limiter_enabled.fill(false);
        for b in s.filters.iter_mut().chain(s.xover.iter_mut()).flatten() {
            b.filter_type = 0;
        }
        s
    }

    #[test]
    fn wp_fbus_user_volume_follows_the_firmware_table() {
        assert_eq!(user_volume_db(0.0), Some(0.0));
        assert_eq!(user_volume_db(-20.0), Some(-20.0));
        // The step below: -20.5 plays as -21
        assert_eq!(user_volume_db(-20.5), Some(-21.0));
        assert_eq!(user_volume_db(-59.0), Some(-59.0));
        assert_eq!(user_volume_db(-60.0), None);
        assert_eq!(user_volume_db(-90.0), None);
        assert_eq!(user_volume_db(6.0), Some(0.0));
    }

    #[test]
    fn wp_fbus_mutes_and_master_silence_an_output() {
        let mut s = state();
        s.user_volume_db = -20.0;
        s.master_volume_db = -6.0;
        assert_eq!(after_chain_db(&s, 0), Some(-26.0));
        s.output_muted[0] = true;
        assert_eq!(after_chain_db(&s, 0), None);
        assert_eq!(after_chain_db(&s, 1), Some(-26.0));
        s.output_enabled[1] = false;
        assert_eq!(after_chain_db(&s, 1), None);
        s.user_mute = true;
        assert_eq!(after_chain_db(&s, 2), None);
        s.user_mute = false;
        s.master_volume_db = -128.0;
        assert_eq!(after_chain_db(&s, 2), None);
    }

    #[test]
    fn wp_fbus_a_test_signal_goes_through_volume_trim_and_the_limiter() {
        let mut s = state();
        s.user_volume_db = -20.0;
        s.output_gain_db[0] = -3.0;
        let c = signal_chain(&s, 0, 0).unwrap();
        let p = signal_peak(&s, 0, &c, -12.0, (20.0, 20_000.0), 1000.0);
        assert!((p.peak_db - -35.0).abs() < 0.01, "{p:?}");
        assert!((p.below_db - -35.0).abs() < 0.01, "{p:?}");
        assert_eq!(p.limiter_db, 0.0);
        // At full volume a -1 dBFS limiter holds a -0 dBFS sweep
        s.user_volume_db = 0.0;
        s.output_gain_db[0] = 0.0;
        s.limiter_enabled[0] = true;
        s.limiter_threshold_db[0] = -6.0;
        let c = signal_chain(&s, 0, 0).unwrap();
        let p = signal_peak(&s, 0, &c, 0.0, (20.0, 20_000.0), 0.0);
        assert!((p.peak_db - -6.0).abs() < 0.01 && (p.limiter_db - 6.0).abs() < 0.01);
        // The render's chains: reduced by the limiter, silence where muted
        s.output_muted[1] = true;
        let chains = (0..3).map(|o| crate::measure::chain_of(&s, o)).collect();
        let got = signal_chains(&s, 4, 0b011, 0, 0.0, 20.0, 20_000.0, chains);
        assert!((got[0].gain_db - -6.0).abs() < 0.01);
        assert_eq!(got[1].gain_db, SILENT_DB);
        assert_eq!(got[2].gain_db, 0.0);
    }

    #[test]
    fn wp_fbus_raw_skips_the_crossover_and_loudness_boosts_at_low_volume() {
        let mut s = state();
        let n_in = usize::from(s.num_input_channels);
        // A 2 kHz high-pass in the output's crossover bank
        let hp = &mut s.xover[n_in][0];
        hp.filter_type = 35; // LR4 high pass (crossover type)
        hp.freq = 2000.0;
        let c = signal_chain(&s, 0, 0).unwrap();
        assert!(chain_db(&c, 200.0) < -60.0);
        let raw = signal_chain(&s, 0, FLAG_RAW).unwrap();
        assert!(chain_db(&raw, 200.0).abs() < 0.01);
        // Loudness at -40 dB under an 80 phon reference: bass up
        let mut s = state();
        s.loudness_enabled = true;
        s.loudness_output_mask = 0xFFFF;
        s.loudness_ref_spl = 80.0;
        s.loudness_intensity = 100.0;
        s.user_volume_db = -40.0;
        let c = signal_chain(&s, 0, 0).unwrap();
        let bass = chain_db(&c, 40.0) + 40.0;
        let mid = chain_db(&c, 1000.0) + 40.0;
        assert!(bass > 8.0 && mid.abs() < 2.0, "bass {bass}, mid {mid}");
        s.loudness_output_mask = 0;
        assert!(loudness_bands(&s, 0).is_empty());
    }
}
