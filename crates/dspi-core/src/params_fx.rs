//! Effects: psybass, limiter, upmixer, subharmonic synth, tube.
//!
//! Every setter follows the same three steps (PLAN §1.4, WPs C3/C4):
//! 1. quantize and clamp the value the way the old desktop core did, which
//!    is also what the firmware stores (cited per table below), refusing a
//!    NaN or an unknown parameter id with [`DspiError::InvalidArgument`]
//!    before any traffic;
//! 2. send it through [`Inner::writable_conn`] and the session's fast path
//!    (`Session::send`, which also refuses a feature the firmware lacks with
//!    [`DspiError::Unsupported`]);
//! 3. update the mirror the way the firmware updates its own copy (tube
//!    type rows, `custom_reset_if`, limiter link groups), then re-encode
//!    `bulk_raw`.
//!
//! The mirror is only touched after the device accepted the transfer. The
//! firmware ACKs some writes it then ignores; its `PARAM_CHANGED` echo (WP
//! C7) corrects the mirror in that case.
//!
//! Owned by WP C4 (docs/core-merge/PLAN.md).

use dspi_proto::generated::opcodes as op;
use dspi_proto::generated::{limiter, ranges as rg, subharm, tube, upmix};
use dspi_proto::registry::{Kind, by_path};
use dspi_proto::value::Value;

use crate::consts::*;
use crate::records::*;
use crate::{DspiCore, DspiError, Inner, Result, state};

// ── Parameter tables ────────────────────────────────────────────────

/// Registry path and firmware range of each `PSYBASS_PARAM_*`
/// (psybass.h:37-48; `vendor_commands.c` clamps every one).
const PSYBASS: [(&str, f32, f32); PSYBASS_PARAM_COUNT] = [
    (
        "bass.cutoff",
        rg::PSYBASS_CUTOFF_MIN,
        rg::PSYBASS_CUTOFF_MAX,
    ),
    (
        "bass.harmonics",
        rg::PSYBASS_HARMONICS_MIN,
        rg::PSYBASS_HARMONICS_MAX,
    ),
    ("bass.drive", rg::PSYBASS_DRIVE_MIN, rg::PSYBASS_DRIVE_MAX),
    (
        "bass.character",
        rg::PSYBASS_CHARACTER_MIN,
        rg::PSYBASS_CHARACTER_MAX,
    ),
    (
        "bass.original",
        rg::PSYBASS_ORIGINAL_MIN,
        rg::PSYBASS_ORIGINAL_MAX,
    ),
];

/// Registry path and range of each `UPMIX_PARAM_*`, indexed by id. The
/// firmware stores the raw float (`REQ_UPMIX_SET_PARAM`) and clamps only
/// when it computes coefficients (upmix.h:88-109), so the value is clamped
/// here, as the old core did, and the image holds what the audio path uses.
const UPMIX: [(&str, f32, f32); UPMIX_PARAM_COUNT] = [
    ("up.on", 0.0, 1.0),
    ("up.center_mode", 0.0, upmix::UPMIX_CENTER_OFF as f32),
    ("up.surround_mode", 0.0, 2.0),
    (
        "up.strength",
        rg::UPMIX_STRENGTH_MIN,
        rg::UPMIX_STRENGTH_MAX,
    ),
    ("up.width", rg::UPMIX_WIDTH_MIN, rg::UPMIX_WIDTH_MAX),
    ("up.threshold", rg::UPMIX_THRESH_MIN, rg::UPMIX_THRESH_MAX),
    ("up.attack", rg::UPMIX_ATTACK_MIN, rg::UPMIX_ATTACK_MAX),
    ("up.release", rg::UPMIX_RELEASE_MIN, rg::UPMIX_RELEASE_MAX),
    ("up.det_hpf", rg::UPMIX_DET_HPF_MIN, rg::UPMIX_DET_HPF_MAX),
    (
        "up.sur_delay",
        rg::UPMIX_SUR_DELAY_MIN,
        rg::UPMIX_SUR_DELAY_MAX,
    ),
    ("up.sur_hpf", rg::UPMIX_SUR_HPF_MIN, rg::UPMIX_SUR_HPF_MAX),
    ("up.sur_lpf", rg::UPMIX_SUR_LPF_MIN, rg::UPMIX_SUR_LPF_MAX),
    ("up.decorr", rg::UPMIX_DECORR_MIN, rg::UPMIX_DECORR_MAX),
    (
        "up.presence",
        rg::UPMIX_PRESENCE_MIN,
        rg::UPMIX_PRESENCE_MAX,
    ),
];

/// Registry path and range of each `SUBHARM_PARAM_*` (host ids, one request
/// each), indexed by id (subharm.h:64-83).
const SUBHARM: [(&str, f32, f32); SUBHARM_PARAM_COUNT] = [
    ("sub.on", 0.0, 1.0),
    ("sub.mask", 0.0, 65535.0),
    ("sub.low", rg::SUBHARM_LEVEL_MIN, rg::SUBHARM_LEVEL_MAX),
    ("sub.high", rg::SUBHARM_LEVEL_MIN, rg::SUBHARM_LEVEL_MAX),
    ("sub.top", rg::SUBHARM_LEVEL_MIN, rg::SUBHARM_LEVEL_MAX),
    ("sub.boost", rg::SUBHARM_BOOST_MIN, rg::SUBHARM_BOOST_MAX),
    ("sub.select", 0.0, subharm::SUBHARM_SELECT_MODE_MAX as f32),
    ("sub.depth", rg::SUBHARM_DEPTH_MIN, rg::SUBHARM_DEPTH_MAX),
    ("sub.hold", rg::SUBHARM_HOLD_MIN, rg::SUBHARM_HOLD_MAX),
    (
        "sub.ceiling",
        rg::SUBHARM_CEILING_MIN,
        rg::SUBHARM_CEILING_MAX,
    ),
    ("sub.link", 0.0, 1.0),
];

/// Registry path and range of each `TUBE_PARAM_*`, indexed by the firmware's
/// own index (tube.h:36-59, `tube_set_param`).
const TUBE: [(&str, f32, f32); TUBE_PARAM_COUNT] = [
    ("tube.on", 0.0, 1.0),
    ("tube.mask", 0.0, 65535.0),
    ("tube.type", 0.0, tube::TUBE_TYPE_MAX as f32),
    ("tube.drive", rg::TUBE_DRIVE_MIN, rg::TUBE_DRIVE_MAX),
    ("tube.bias", rg::TUBE_BIAS_MIN, rg::TUBE_BIAS_MAX),
    ("tube.asym", rg::TUBE_ASYM_MIN, rg::TUBE_ASYM_MAX),
    (
        "tube.hardness",
        rg::TUBE_HARDNESS_MIN,
        rg::TUBE_HARDNESS_MAX,
    ),
    ("tube.sag", rg::TUBE_SAG_MIN, rg::TUBE_SAG_MAX),
    ("tube.rectifier", 0.0, tube::TUBE_RECT_MAX as f32),
    ("tube.xfmr", 0.0, 1.0),
    (
        "tube.damping",
        rg::TUBE_XFMR_DAMPING_MIN,
        rg::TUBE_XFMR_DAMPING_MAX,
    ),
    (
        "tube.resonance",
        rg::TUBE_XFMR_RES_MIN,
        rg::TUBE_XFMR_RES_MAX,
    ),
    ("tube.mix", rg::TUBE_MIX_MIN, rg::TUBE_MIX_MAX),
    ("tube.trim", rg::TUBE_TRIM_MIN, rg::TUBE_TRIM_MAX),
];

/// Registry path of each `LIMITER_PARAM_*`, indexed by id (limiter.h:11-17).
const LIMITER: [&str; limiter::LIMITER_NUM_PARAMS as usize] =
    ["limit.on", "limit.threshold", "limit.release", "limit.link"];

/// The tube indices `tube_set_param` treats as character knobs, in the
/// order of a type row: moving one drops the type to Custom
/// (`custom_reset_if`, tube.c).
const TUBE_CHARACTER: [u16; 4] = [
    tube::TUBE_PARAM_BIAS_PCT,
    tube::TUBE_PARAM_ASYM_DB,
    tube::TUBE_PARAM_HARDNESS_PCT,
    tube::TUBE_PARAM_SAG_PCT,
];

// ── Quantizers (pure) ───────────────────────────────────────────────

fn invalid(msg: impl Into<String>) -> DspiError {
    DspiError::InvalidArgument { msg: msg.into() }
}

/// A NaN is refused: the firmware ignores one for the tube and the limiter
/// and would store it for the others (the old core sent it). Infinities are
/// clamped like any other value.
fn not_nan(value: f32, what: &str) -> Result<f32> {
    if value.is_nan() {
        Err(invalid(format!("{what}: not a number")))
    } else {
        Ok(value)
    }
}

/// The registry path and the stored value of a psybass write.
fn psybass_quantize(param: u8, value: f32) -> Result<(&'static str, f32)> {
    let &(path, lo, hi) = PSYBASS
        .get(usize::from(param))
        .ok_or_else(|| invalid(format!("unknown psybass parameter {param}")))?;
    Ok((path, not_nan(value, path)?.clamp(lo, hi)))
}

/// The registry path and the stored value of an upmixer write. The enable
/// is "not zero" (`v != 0.0f`, as `REQ_UPMIX_SET_PARAM` tests it); the
/// modes round like `lrintf` (ties to even); an out-of-range centre
/// mode becomes adaptive (`upmix_clamp_center_mode`: a bad value must not
/// switch the centre off), the surround mode is clamped; the presence goes
/// in 0.5 dB steps (`upmix_presence_encode`).
fn upmix_quantize(id: u8, value: f32) -> Result<(&'static str, f32)> {
    let &(path, lo, hi) = UPMIX
        .get(usize::from(id))
        .ok_or_else(|| invalid(format!("unknown upmixer parameter {id}")))?;
    let v = not_nan(value, path)?;
    let v = match id {
        UPMIX_PARAM_CENTER_MODE => {
            let m = v.round_ties_even();
            if (lo..=hi).contains(&m) {
                m
            } else {
                upmix::UPMIX_CENTER_ADAPTIVE as f32
            }
        }
        UPMIX_PARAM_ENABLED => f32::from(u8::from(v != 0.0)),
        UPMIX_PARAM_SURROUND_MODE => v.round_ties_even().clamp(lo, hi),
        UPMIX_PARAM_PRESENCE => (v.clamp(lo, hi) * 2.0).round_ties_even() / 2.0,
        _ => v.clamp(lo, hi),
    };
    Ok((path, v))
}

/// The registry path and the stored value of a subharm write: the flags,
/// the mask and the selectivity mode are whole numbers.
fn subharm_quantize(id: u8, value: f32) -> Result<(&'static str, f32)> {
    let &(path, lo, hi) = SUBHARM
        .get(usize::from(id))
        .ok_or_else(|| invalid(format!("unknown subharm parameter {id}")))?;
    let v = not_nan(value, path)?;
    let whole = matches!(
        id,
        SUBHARM_PARAM_ENABLED | SUBHARM_PARAM_MASK | SUBHARM_PARAM_SELECT | SUBHARM_PARAM_LINK
    );
    Ok((
        path,
        if whole {
            v.round().clamp(lo, hi)
        } else {
            v.clamp(lo, hi)
        },
    ))
}

/// The registry path and the stored value of a tube write. The flags, the
/// mask, the type and the rectifier are whole numbers (`round_clamp_u`:
/// clamp, then round half up).
fn tube_quantize(idx: u8, value: f32) -> Result<(&'static str, f32)> {
    let &(path, lo, hi) = TUBE
        .get(usize::from(idx))
        .ok_or_else(|| invalid(format!("unknown tube parameter {idx}")))?;
    let v = not_nan(value, path)?;
    let whole = matches!(
        u16::from(idx),
        tube::TUBE_PARAM_ENABLED
            | tube::TUBE_PARAM_OUTPUT_MASK
            | tube::TUBE_PARAM_TUBE_TYPE
            | tube::TUBE_PARAM_RECTIFIER
            | tube::TUBE_PARAM_XFMR_ENABLED
    );
    Ok((
        path,
        if whole {
            v.clamp(lo, hi).round()
        } else {
            v.clamp(lo, hi)
        },
    ))
}

/// Put a quantized tube value into `DspState.tube` as `tube_set_param` does:
/// a type 1..16 loads its row into bias, asymmetry, hardness and sag (Custom
/// keeps them); a character knob that actually changes drops the type to
/// Custom.
fn tube_apply(t: &mut [f32], idx: u8, v: f32) {
    let custom = tube::TUBE_TYPE_CUSTOM as f32;
    let type_slot = tube::TUBE_PARAM_TUBE_TYPE as usize;
    let i = usize::from(idx);
    if u16::from(idx) == tube::TUBE_PARAM_TUBE_TYPE {
        // `v` is a whole number 0..=16 here; Custom has no row.
        if let Some(row) = dspi_session::tube::type_row(v as u8) {
            let values = [row.bias_pct, row.asym_db, row.hardness_pct, row.sag_pct];
            for (&k, rv) in TUBE_CHARACTER.iter().zip(values) {
                if let Some(slot) = t.get_mut(usize::from(k)) {
                    *slot = rv;
                }
            }
        }
    } else if TUBE_CHARACTER.contains(&u16::from(idx))
        && t.get(i).is_some_and(|&old| old != v)
        && let Some(ty) = t.get_mut(type_slot)
    {
        *ty = custom;
    }
    if let Some(slot) = t.get_mut(i) {
        *slot = v;
    }
}

/// The registry path and the stored value of a limiter write
/// (`lm_set_one`): enable is "not zero", threshold -30..0 dB, release
/// 10..1000 ms, link group 0..4 (clamp, then round half up).
fn limiter_quantize(param: u8, value: f32) -> Result<(&'static str, f32)> {
    let path = *LIMITER
        .get(usize::from(param))
        .ok_or_else(|| invalid(format!("unknown limiter parameter {param}")))?;
    let v = not_nan(value, path)?;
    let v = match u16::from(param) {
        limiter::LIMITER_PARAM_ENABLED => f32::from(u8::from(v != 0.0)),
        limiter::LIMITER_PARAM_THRESHOLD_DB => {
            v.clamp(rg::LIMITER_THRESHOLD_MIN, rg::LIMITER_THRESHOLD_MAX)
        }
        limiter::LIMITER_PARAM_RELEASE_MS => {
            v.clamp(rg::LIMITER_RELEASE_MIN, rg::LIMITER_RELEASE_MAX)
        }
        _ => v.clamp(0.0, limiter::LIMITER_LINK_GROUP_MAX as f32).round(),
    };
    Ok((path, v))
}

/// The limiter fields of the mirror, cut to the device's outputs.
struct Limiters<'a> {
    enabled: &'a mut [bool],
    group: &'a mut [u8],
    threshold: &'a mut [f32],
    release: &'a mut [f32],
}

impl Limiters<'_> {
    fn len(&self) -> usize {
        self.group.len()
    }

    fn group_of(&self, k: usize) -> u8 {
        self.group.get(k).copied().unwrap_or(0)
    }

    /// `lm_set_one`, with `v` already quantized.
    fn set_one(&mut self, k: usize, param: u16, v: f32) {
        match param {
            limiter::LIMITER_PARAM_ENABLED => {
                if let Some(s) = self.enabled.get_mut(k) {
                    *s = v != 0.0;
                }
            }
            limiter::LIMITER_PARAM_THRESHOLD_DB => {
                if let Some(s) = self.threshold.get_mut(k) {
                    *s = v;
                }
            }
            limiter::LIMITER_PARAM_RELEASE_MS => {
                if let Some(s) = self.release.get_mut(k) {
                    *s = v;
                }
            }
            _ => {
                if let Some(s) = self.group.get_mut(k) {
                    // A whole number 0..=4.
                    *s = v as u8;
                }
            }
        }
    }

    /// `lm_group_peer`: the lowest output other than `k` in group `gr`.
    fn peer(&self, k: usize, gr: u8) -> Option<usize> {
        (0..self.len()).find(|&m| m != k && self.group_of(m) == gr)
    }

    /// `lm_copy_settings`: enable, threshold and release of `src`.
    fn copy(&mut self, dst: usize, src: usize) {
        if let (Some(&e), Some(&t), Some(&r)) = (
            self.enabled.get(src),
            self.threshold.get(src),
            self.release.get(src),
        ) {
            self.set_one(dst, limiter::LIMITER_PARAM_ENABLED, f32::from(u8::from(e)));
            self.set_one(dst, limiter::LIMITER_PARAM_THRESHOLD_DB, t);
            self.set_one(dst, limiter::LIMITER_PARAM_RELEASE_MS, r);
        }
    }

    /// `lm_gang_groups`: every member of a group copies its lowest member.
    fn gang(&mut self) {
        for k in 0..self.len() {
            let gr = self.group_of(k);
            if gr == 0 {
                continue;
            }
            if let Some(lead) = self.peer(k, gr).filter(|&lead| lead < k) {
                self.copy(k, lead);
            }
        }
    }

    /// `limiter_set_param` after its checks: `output` is one of the
    /// device's outputs or `LIMITER_ALL_OUTPUTS`; `v` is quantized.
    fn set(&mut self, output: u8, param: u8, v: f32) {
        let param = u16::from(param);
        let link = param == limiter::LIMITER_PARAM_LINK_GROUP;
        let o = usize::from(output);
        if u16::from(output) == limiter::LIMITER_ALL_OUTPUTS {
            for k in 0..self.len() {
                self.set_one(k, param, v);
            }
            if link {
                self.gang();
            }
        } else if link {
            // Joining a group adopts its settings; the first member keeps
            // its own.
            self.set_one(o, param, v);
            let gr = self.group_of(o);
            if gr != 0
                && let Some(peer) = self.peer(o, gr)
            {
                self.copy(o, peer);
            }
        } else {
            let gr = self.group_of(o);
            for k in 0..self.len() {
                if k == o || (gr != 0 && self.group_of(k) == gr) {
                    self.set_one(k, param, v);
                }
            }
        }
    }
}

/// The limiter fields of `s` for its device outputs.
fn limiters(s: &mut DspState) -> Limiters<'_> {
    let n = usize::from(s.num_output_channels)
        .min(s.limiter_enabled.len())
        .min(s.limiter_link_group.len())
        .min(s.limiter_threshold_db.len())
        .min(s.limiter_release_ms.len());
    Limiters {
        enabled: s.limiter_enabled.get_mut(..n).unwrap_or_default(),
        group: s.limiter_link_group.get_mut(..n).unwrap_or_default(),
        threshold: s.limiter_threshold_db.get_mut(..n).unwrap_or_default(),
        release: s.limiter_release_ms.get_mut(..n).unwrap_or_default(),
    }
}

/// The session value for a quantized `v` at `path`, by the registry's kind.
fn value_for(path: &str, v: f32) -> Result<Value> {
    let d = by_path(path).ok_or_else(|| invalid(format!("unknown parameter {path}")))?;
    // `v` is quantized to the parameter's range, so the casts are exact.
    Ok(match d.kind {
        Kind::Bool => Value::Bool(v != 0.0),
        Kind::Choice(_) => Value::Choice(v as u8),
        Kind::Int { .. } => Value::Int(v as i64),
        Kind::Mask => Value::Mask(v as u32),
        _ => Value::Float(v),
    })
}

/// Little-endian `u16` number `k` of `d`.
fn u16_at(d: &[u8], k: usize) -> Option<u16> {
    let at = k.checked_mul(2)?;
    let b = d.get(at..at.checked_add(2)?)?;
    Some(u16::from_le_bytes([*b.first()?, *b.get(1)?]))
}

fn short_reply(what: &str, got: usize, want: usize) -> DspiError {
    DspiError::Transfer {
        msg: format!("{what}: {got} bytes, expected {want}"),
    }
}

/// `UpmixStatus` (upmix.h): active, parked reason, then correlation and
/// balance in Q14 and the three gains in Q15. `None` under 12 bytes.
fn decode_upmix_status(d: &[u8]) -> Option<UpmixStatus> {
    let q14 = |k: usize| u16_at(d, k).map(|v| f32::from(v as i16) / 16384.0);
    let q15 = |k: usize| u16_at(d, k).map(|v| f32::from(v) / 32767.0);
    Some(UpmixStatus {
        active: *d.first()? != 0,
        parked_reason: *d.get(1)?,
        correlation: q14(1)?,
        balance: q14(2)?,
        center_gain: q15(3)?,
        ls_gain: q15(4)?,
        rs_gain: q15(5)?,
    })
}

/// `n` little-endian `u16`s of `d` scaled by `scale`, padded with zeros to
/// [`MAX_OUTPUTS`] entries; a short reply is a transfer error.
fn meter(d: &[u8], n: usize, scale: f32, what: &str) -> Result<Vec<f32>> {
    let mut out = vec![0.0f32; MAX_OUTPUTS];
    for (k, v) in out.iter_mut().enumerate().take(n) {
        let raw = u16_at(d, k).ok_or_else(|| short_reply(what, d.len(), n.saturating_mul(2)))?;
        *v = f32::from(raw) / scale;
    }
    Ok(out)
}

impl Inner {
    /// Send a quantized effects value through the fast path, then let
    /// `update` patch the mirror and re-encode `bulk_raw`.
    fn fx_write(
        &mut self,
        path: &str,
        indices: &[u8],
        v: f32,
        update: impl FnOnce(&mut DspState),
    ) -> Result<()> {
        let value = value_for(path, v)?;
        self.writable_conn()?
            .session()?
            .send(path, indices, value)?;
        update(&mut self.state);
        self.state.bulk_raw = state::encode_bulk(&self.state);
        Ok(())
    }

    /// `Unsupported` when the probed firmware lacks `feature`. A connection
    /// without a session (unmappable image) is not judged: its read decides.
    fn require_feature(&mut self, feature: &str) -> Result<()> {
        match self.conn()?.session() {
            Ok(s) => crate::require_feature(s, feature),
            Err(_) => Ok(()),
        }
    }

    /// Read a `u16`-per-output meter: `num_output_channels` (at most
    /// [`MAX_OUTPUTS`]) entries.
    fn read_meter(&mut self, opcode: u8, value: u16, scale: f32, what: &str) -> Result<Vec<f32>> {
        let n = usize::from(self.state.num_output_channels).min(MAX_OUTPUTS);
        // At most 2 * MAX_OUTPUTS bytes.
        let len = u16::try_from(n.saturating_mul(2)).unwrap_or(u16::MAX);
        let d = self.conn()?.get(opcode, value, len)?;
        meter(&d, n, scale, what)
    }
}

#[cfg_attr(feature = "uniffi", uniffi::export)]
impl DspiCore {
    /// Set one psychoacoustic-bass parameter. `param`: `PSYBASS_PARAM_*`
    /// (cutoff Hz 30..300, harmonics dB -24..12, drive dB 0..18, character %
    /// 0..100, original dB -60..0); the value is clamped to that range.
    ///
    /// Errors: `InvalidArgument` (unknown `param`, NaN), `Disconnected`,
    /// `ReadOnly`, `Unsupported` (no psychoacoustic bass), `Transfer`.
    pub fn set_psybass_param(&self, param: u8, value: f32) -> Result<()> {
        let (path, v) = psybass_quantize(param, value)?;
        self.fx_lock().fx_write(path, &[], v, |s| match param {
            PSYBASS_PARAM_CUTOFF => s.psybass_cutoff_hz = v,
            PSYBASS_PARAM_HARMONICS => s.psybass_harmonics_db = v,
            PSYBASS_PARAM_DRIVE => s.psybass_drive_db = v,
            PSYBASS_PARAM_CHARACTER => s.psybass_character_pct = v,
            _ => s.psybass_original_db = v,
        })
    }

    /// Which outputs psychoacoustic bass processes: bit k = output k.
    ///
    /// Errors: `Disconnected`, `ReadOnly`, `Unsupported`, `Transfer`.
    pub fn set_psybass_mask(&self, mask: u16) -> Result<()> {
        self.fx_lock()
            .fx_write("bass.mask", &[], f32::from(mask), |s| {
                s.psybass_output_mask = mask;
            })
    }

    /// Turn psychoacoustic bass on or off.
    ///
    /// Errors: `Disconnected`, `ReadOnly`, `Unsupported`, `Transfer`.
    pub fn set_psybass_enabled(&self, enabled: bool) -> Result<()> {
        self.fx_lock()
            .fx_write("bass.on", &[], f32::from(u8::from(enabled)), |s| {
                s.psybass_enabled = enabled;
            })
    }

    /// Set one output-limiter parameter of `output` (0-based), or of every
    /// output when `output` is `LIMITER_ALL_OUTPUTS` (0xFF). `param`:
    /// `LIMITER_PARAM_*` (enabled: non-zero is on; threshold dB -30..0;
    /// release ms 10..1000; link group 0..4, 0 = unlinked), clamped as the
    /// firmware does. The mirror follows the firmware's link groups: a write
    /// to one member moves the whole group, joining a group adopts its
    /// settings, and a group set on every output gangs each group to its
    /// lowest member.
    ///
    /// Errors: `InvalidArgument` (unknown `param`, NaN, an output past the
    /// device's), `Disconnected`, `ReadOnly`, `Unsupported` (no limiter),
    /// `Transfer`.
    pub fn set_limiter_param(&self, output: u8, param: u8, value: f32) -> Result<()> {
        let (path, v) = limiter_quantize(param, value)?;
        self.fx_lock().fx_write(path, &[output], v, |s| {
            limiters(s).set(output, param, v);
        })
    }

    /// Gain reduction per output in dB (positive while limiting, 120 = the
    /// display floor), [`MAX_OUTPUTS`] entries; outputs the device does not
    /// have read 0. Works on a read-only connection.
    ///
    /// Errors: `Disconnected`, `Unsupported` (no limiter), `Transfer`
    /// (including a short reply).
    pub fn fetch_limiter_meter(&self) -> Result<Vec<f32>> {
        self.ensure_probed(); // -- WP LK4 --
        let mut inner = self.lock();
        inner.require_feature("output_limiter")?;
        inner.read_meter(
            op::REQ_LIMITER,
            limiter::LIMITER_GET_METER,
            100.0,
            "limiter meter",
        )
    }

    /// Set upmixer parameter `id` (`UPMIX_PARAM_*`); RP2350 only. The value
    /// is clamped to the firmware's range; the enable and the modes are
    /// rounded (an out-of-range centre mode becomes adaptive, as the
    /// firmware does), the presence goes in 0.5 dB steps.
    ///
    /// Errors: `InvalidArgument` (unknown `id`, NaN), `Disconnected`,
    /// `ReadOnly`, `Unsupported` (RP2040), `Transfer`.
    pub fn set_upmix_param(&self, id: u8, value: f32) -> Result<()> {
        let (path, v) = upmix_quantize(id, value)?;
        self.fx_lock().fx_write(path, &[], v, |s| {
            if let Some(slot) = s.upmix.get_mut(usize::from(id)) {
                *slot = v;
            }
        })
    }

    /// The upmixer's live state: active, why it is parked, correlation and
    /// balance (-1..1, 0..1), and the centre and surround gains (0..1). All
    /// zero on an RP2040. Works on a read-only connection.
    ///
    /// Errors: `Disconnected`, `Transfer` (including a reply under 12
    /// bytes).
    pub fn fetch_upmix_status(&self) -> Result<UpmixStatus> {
        let mut inner = self.lock();
        let d = inner.conn()?.get_upto(op::REQ_UPMIX_GET_STATUS, 0, 16)?;
        decode_upmix_status(&d).ok_or_else(|| short_reply("upmixer status", d.len(), 12))
    }

    /// Set subharmonic-synth parameter `id` (`SUBHARM_PARAM_*`), clamped to
    /// its range; the flags, the mask and the selectivity mode are rounded.
    ///
    /// Errors: `InvalidArgument` (unknown `id`, NaN), `Disconnected`,
    /// `ReadOnly`, `Unsupported` (no subharmonic synth), `Transfer`.
    pub fn set_subharm_param(&self, id: u8, value: f32) -> Result<()> {
        let (path, v) = subharm_quantize(id, value)?;
        self.fx_lock().fx_write(path, &[], v, |s| {
            if let Some(slot) = s.subharm.get_mut(usize::from(id)) {
                *slot = v;
            }
        })
    }

    /// Mute the program on the subharm outputs so the synthesized sub is
    /// heard alone. Monitoring only: never saved, not in the bulk image.
    ///
    /// Errors: `Disconnected`, `ReadOnly`, `Unsupported`, `Transfer`.
    pub fn set_subharm_solo(&self, solo: bool) -> Result<()> {
        self.fx_lock()
            .fx_write("sub.solo", &[], f32::from(u8::from(solo)), |s| {
                s.subharm_solo = solo;
            })
    }

    /// How far the current subharm settings can push the signal above its
    /// input, dB (computed by the device). Works on a read-only connection.
    ///
    /// Errors: `Disconnected`, `Unsupported`, `Transfer`.
    pub fn fetch_subharm_headroom(&self) -> Result<f32> {
        self.ensure_probed(); // -- WP LK4 --
        let mut inner = self.lock();
        inner.require_feature("subharmonic_synth")?;
        let d = inner.conn()?.get(op::REQ_GET_SUBHARM_HEADROOM, 0, 4)?;
        let b: [u8; 4] = d
            .get(..4)
            .and_then(|b| b.try_into().ok())
            .ok_or_else(|| short_reply("subharm headroom", d.len(), 4))?;
        Ok(f32::from_le_bytes(b))
    }

    /// Decaying peak of the synthesized sub per output, 0..1,
    /// [`MAX_OUTPUTS`] entries; outputs the device does not have read 0.
    /// Works on a read-only connection.
    ///
    /// Errors: `Disconnected`, `Unsupported`, `Transfer`.
    pub fn fetch_subharm_meter(&self) -> Result<Vec<f32>> {
        self.ensure_probed(); // -- WP LK4 --
        let mut inner = self.lock();
        inner.require_feature("subharmonic_synth")?;
        inner.read_meter(op::REQ_GET_SUBHARM_METER, 0, 32767.0, "subharm meter")
    }

    /// Set tube-preamp parameter `idx` (`TUBE_PARAM_*`), clamped to its
    /// range; the flags, the mask, the type and the rectifier are rounded.
    /// The mirror follows the firmware: choosing a type 1..16 loads its
    /// bias, asymmetry, hardness and sag; changing one of those four drops
    /// the type to Custom (0).
    ///
    /// Errors: `InvalidArgument` (unknown `idx`, NaN), `Disconnected`,
    /// `ReadOnly`, `Unsupported` (no tube preamp), `Transfer`.
    pub fn set_tube_param(&self, idx: u8, value: f32) -> Result<()> {
        let (path, v) = tube_quantize(idx, value)?;
        self.fx_lock()
            .fx_write(path, &[], v, |s| tube_apply(&mut s.tube, idx, v))
    }
}

// -- WP CG32 --
/// The effect a registry path of the tables above belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Fx {
    Psybass,
    Upmix,
    Subharm,
    Tube,
    Limiter,
}

/// The effect and parameter id of `path` in the tables above, for
/// `DspiCore::write_param`: the id its `set_*_param` takes.
pub(crate) fn fx_param(path: &str) -> Option<(Fx, u8)> {
    let id = |paths: &[&str]| {
        paths
            .iter()
            .position(|p| *p == path)
            .and_then(|i| u8::try_from(i).ok())
    };
    let ranged = |t: &[(&'static str, f32, f32)]| t.iter().map(|r| r.0).collect::<Vec<_>>();
    let tables: [(Fx, Option<u8>); 5] = [
        (Fx::Psybass, id(&ranged(&PSYBASS))),
        (Fx::Upmix, id(&ranged(&UPMIX))),
        (Fx::Subharm, id(&ranged(&SUBHARM))),
        (Fx::Tube, id(&ranged(&TUBE))),
        (Fx::Limiter, id(&LIMITER)),
    ];
    tables.into_iter().find_map(|(fx, i)| i.map(|i| (fx, i)))
}

// -- WP LK4 --
impl DspiCore {
    /// The lock for an effect write: every effect is a probed feature, so a
    /// DSPi Link bind still completing its probe completes it first (the write
    /// then goes, or is refused as unsupported, on what the device said).
    fn fx_lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.ensure_probed();
        self.lock()
    }
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_c4 {
    use std::sync::{Arc, Mutex};

    use dspi_proto::wire_compat::WireRead;
    use dspi_transport::{
        Capture, DeviceDescriptor, FakeDevice, FakeOptions, FakeRemote, NotificationSource,
        Transport, TransportCaps,
    };
    use proptest::prelude::*;

    use super::*;

    const ALL: u8 = limiter::LIMITER_ALL_OUTPUTS as u8;
    const ON: u8 = limiter::LIMITER_PARAM_ENABLED as u8;
    const THR: u8 = limiter::LIMITER_PARAM_THRESHOLD_DB as u8;
    const REL: u8 = limiter::LIMITER_PARAM_RELEASE_MS as u8;
    const LINK: u8 = limiter::LIMITER_PARAM_LINK_GROUP as u8;
    const T_TYPE: u8 = tube::TUBE_PARAM_TUBE_TYPE as u8;
    const T_BIAS: u8 = tube::TUBE_PARAM_BIAS_PCT as u8;
    const T_SAG: u8 = tube::TUBE_PARAM_SAG_PCT as u8;
    const T_RECT: u8 = tube::TUBE_PARAM_RECTIFIER as u8;
    const T_DRIVE: u8 = tube::TUBE_PARAM_DRIVE_DB as u8;
    const T_MASK: u8 = tube::TUBE_PARAM_OUTPUT_MASK as u8;

    /// Bias, asymmetry, hardness and sag of tube types 1..16, from the
    /// firmware's `tube_rows` (tube.c:44-65) via the old core's table.
    const TUBE_ROWS: [[f32; 4]; 16] = [
        [10.0, 3.0, 40.0, 15.0],
        [8.0, 3.0, 35.0, 12.0],
        [5.0, 2.0, 55.0, 10.0],
        [7.0, 4.0, 25.0, 15.0],
        [5.0, 5.0, 20.0, 8.0],
        [7.0, 6.0, 15.0, 10.0],
        [10.0, 3.0, 30.0, 15.0],
        [3.0, 2.0, 60.0, 5.0],
        [2.0, 0.0, 75.0, 12.0],
        [3.0, 1.0, 65.0, 15.0],
        [0.0, 0.0, 50.0, 25.0],
        [0.0, 0.0, 60.0, 30.0],
        [0.0, 0.0, 55.0, 18.0],
        [0.0, 0.0, 35.0, 30.0],
        [0.0, 0.0, 45.0, 10.0],
        [12.0, 6.0, 10.0, 12.0],
    ];

    // ── Rigs ────────────────────────────────────────────────────────

    /// What the [`Rig`] transport saw and what it answers instead of the
    /// fake (the subharm meter and headroom, which the fake does not model).
    #[derive(Default)]
    struct Script {
        log: Vec<(bool, u8, u16)>,
        replies: Vec<(u8, Vec<u8>)>,
    }

    /// A [`FakeDevice`] that logs every transfer and can be scripted per IN
    /// opcode.
    struct Rig {
        inner: FakeDevice,
        script: Arc<Mutex<Script>>,
    }

    impl Transport for Rig {
        fn control_in(&mut self, op: u8, value: u16, len: u16) -> dspi_transport::Result<Vec<u8>> {
            let mut s = self.script.lock().unwrap();
            s.log.push((false, op, value));
            if let Some((_, r)) = s.replies.iter().find(|(o, _)| *o == op) {
                return Ok(r.clone());
            }
            drop(s);
            self.inner.control_in(op, value, len)
        }
        fn control_in_upto(
            &mut self,
            op: u8,
            value: u16,
            len: u16,
        ) -> dspi_transport::Result<Vec<u8>> {
            self.script.lock().unwrap().log.push((false, op, value));
            self.inner.control_in_upto(op, value, len)
        }
        fn control_out(&mut self, op: u8, value: u16, data: &[u8]) -> dspi_transport::Result<()> {
            self.script.lock().unwrap().log.push((true, op, value));
            self.inner.control_out(op, value, data)
        }
        fn descriptor(&self) -> &DeviceDescriptor {
            self.inner.descriptor()
        }
        fn notifications(&self) -> Option<Box<dyn NotificationSource>> {
            None
        }
        fn caps(&self) -> TransportCaps {
            self.inner.caps()
        }
    }

    /// A probed demo device (RP2350, 9 outputs, every effect present).
    fn rig_with(options: FakeOptions) -> (Arc<DspiCore>, FakeRemote, Arc<Mutex<Script>>) {
        let dev = FakeDevice::new(Capture::demo(), options);
        let remote = dev.remote();
        let script = Arc::new(Mutex::new(Script::default()));
        let rig = Rig {
            inner: dev,
            script: Arc::clone(&script),
        };
        let core = DspiCore::new();
        core.attach_probed(Box::new(rig), "DEMO".into(), 0).unwrap();
        script.lock().unwrap().log.clear();
        (core, remote, script)
    }

    fn rig() -> (Arc<DspiCore>, FakeRemote, Arc<Mutex<Script>>) {
        rig_with(FakeOptions::default())
    }

    fn mirror(core: &DspiCore) -> DspState {
        core.lock().state.clone()
    }

    /// The device's own copy, decoded: the read-back.
    fn device(remote: &FakeRemote) -> DspState {
        let mut s = state::default_state();
        state::decode_bulk(&remote.image(), &mut s).unwrap();
        s
    }

    /// The mirror's `bulk_raw`, decoded: it must agree with the mirror.
    fn image_of_mirror(core: &DspiCore) -> DspState {
        let m = mirror(core);
        let mut s = state::default_state();
        state::decode_bulk(&m.bulk_raw, &mut s).unwrap();
        s
    }

    fn traffic(script: &Arc<Mutex<Script>>) -> usize {
        script.lock().unwrap().log.len()
    }

    fn is_invalid<T: std::fmt::Debug>(r: Result<T>) -> bool {
        matches!(r, Err(DspiError::InvalidArgument { .. }))
    }

    // ── Quantizers against the firmware's rules ─────────────────────

    #[test]
    fn wp_c4_psybass_clamps_to_the_firmware_ranges() {
        let q = |p, v| psybass_quantize(p, v).unwrap().1;
        assert_eq!(q(PSYBASS_PARAM_CUTOFF, 10.0), 30.0);
        assert_eq!(q(PSYBASS_PARAM_CUTOFF, 900.0), 300.0);
        assert_eq!(q(PSYBASS_PARAM_HARMONICS, -30.0), -24.0);
        assert_eq!(q(PSYBASS_PARAM_HARMONICS, 20.0), 12.0);
        assert_eq!(q(PSYBASS_PARAM_DRIVE, 19.0), 18.0);
        assert_eq!(q(PSYBASS_PARAM_CHARACTER, -1.0), 0.0);
        assert_eq!(q(PSYBASS_PARAM_ORIGINAL, 3.0), 0.0);
        assert_eq!(q(PSYBASS_PARAM_ORIGINAL, f32::NEG_INFINITY), -60.0);
        assert_eq!(q(PSYBASS_PARAM_DRIVE, 7.25), 7.25);
        assert!(is_invalid(psybass_quantize(5, 1.0)));
        assert!(is_invalid(psybass_quantize(0, f32::NAN)));
    }

    #[test]
    fn wp_c4_upmix_modes_and_presence_follow_the_firmware() {
        let q = |id, v| upmix_quantize(id, v).unwrap().1;
        // upmix_clamp_center_mode: out of range is adaptive, not OFF.
        assert_eq!(q(UPMIX_PARAM_CENTER_MODE, 7.0), 1.0);
        assert_eq!(q(UPMIX_PARAM_CENTER_MODE, -1.0), 1.0);
        assert_eq!(q(UPMIX_PARAM_CENTER_MODE, 2.0), 2.0);
        // lrintf: ties to even.
        assert_eq!(q(UPMIX_PARAM_CENTER_MODE, 0.5), 0.0);
        assert_eq!(q(UPMIX_PARAM_CENTER_MODE, 1.5), 2.0);
        // The surround mode clamps instead.
        assert_eq!(q(UPMIX_PARAM_SURROUND_MODE, 7.0), 2.0);
        assert_eq!(q(UPMIX_PARAM_SURROUND_MODE, -3.0), 0.0);
        // Half-dB steps, ±12 dB (upmix_presence_encode).
        assert_eq!(q(UPMIX_PARAM_PRESENCE, 3.3), 3.5);
        assert_eq!(q(UPMIX_PARAM_PRESENCE, 3.25), 3.0);
        assert_eq!(q(UPMIX_PARAM_PRESENCE, 20.0), 12.0);
        assert_eq!(q(UPMIX_PARAM_PRESENCE, -20.0), -12.0);
        // upmix.h:88-109.
        assert_eq!(q(UPMIX_PARAM_ATTACK, 0.0), 1.0);
        assert_eq!(q(UPMIX_PARAM_RELEASE, 9000.0), 2000.0);
        assert_eq!(q(UPMIX_PARAM_THRESHOLD, 99.0), 95.0);
        assert_eq!(q(UPMIX_PARAM_SURROUND_DELAY, 30.0), 20.0);
        assert_eq!(q(UPMIX_PARAM_DETECTOR_HPF, 5000.0), 1000.0);
        assert_eq!(q(UPMIX_PARAM_SURROUND_LPF, 10.0), 1000.0);
        assert_eq!(q(UPMIX_PARAM_ENABLED, 0.7), 1.0);
        // `v != 0.0f` (vendor_commands.c, REQ_UPMIX_SET_PARAM), not rounding.
        assert_eq!(q(UPMIX_PARAM_ENABLED, 0.3), 1.0);
        assert_eq!(q(UPMIX_PARAM_ENABLED, -2.0), 1.0);
        assert_eq!(q(UPMIX_PARAM_ENABLED, 0.0), 0.0);
        assert!(is_invalid(upmix_quantize(14, 0.0)));
        assert!(is_invalid(upmix_quantize(0, f32::NAN)));
    }

    #[test]
    fn wp_c4_subharm_clamps_and_rounds_like_the_old_core() {
        let q = |id, v| subharm_quantize(id, v).unwrap().1;
        assert_eq!(q(SUBHARM_PARAM_HOLD, 10.0), 50.0);
        assert_eq!(q(SUBHARM_PARAM_HOLD, 500.0), 400.0);
        assert_eq!(q(SUBHARM_PARAM_LOW, -40.0), -30.0);
        assert_eq!(q(SUBHARM_PARAM_TOP, 13.0), 12.0);
        assert_eq!(q(SUBHARM_PARAM_BOOST, 7.0), 6.0);
        assert_eq!(q(SUBHARM_PARAM_CEILING, 1.0), 0.0);
        assert_eq!(q(SUBHARM_PARAM_CEILING, -50.0), -40.0);
        assert_eq!(q(SUBHARM_PARAM_DEPTH, 150.0), 100.0);
        // SUBHARM_SELECT_MODE_MAX.
        assert_eq!(q(SUBHARM_PARAM_SELECT, 7.0), 2.0);
        assert_eq!(q(SUBHARM_PARAM_SELECT, 0.6), 1.0);
        assert_eq!(q(SUBHARM_PARAM_MASK, 70000.0), 65535.0);
        assert_eq!(q(SUBHARM_PARAM_ENABLED, 0.4), 0.0);
        assert_eq!(q(SUBHARM_PARAM_LINK, 3.0), 1.0);
        assert!(is_invalid(subharm_quantize(11, 0.0)));
        assert!(is_invalid(subharm_quantize(2, f32::NAN)));
    }

    #[test]
    fn wp_c4_tube_clamps_like_tube_set_param() {
        let q = |id, v| tube_quantize(id, v).unwrap().1;
        assert_eq!(q(T_RECT, 2.6), 3.0);
        assert_eq!(q(T_RECT, 9.0), 3.0);
        assert_eq!(q(T_TYPE, 20.0), 16.0);
        assert_eq!(q(T_TYPE, -2.0), 0.0);
        assert_eq!(q(T_DRIVE, -50.0), -30.0);
        assert_eq!(q(T_DRIVE, 30.0), 24.0);
        assert_eq!(q(T_BIAS, -150.0), -100.0);
        assert_eq!(q(T_MASK, 1e9), 65535.0);
        assert_eq!(q(tube::TUBE_PARAM_XFMR_DAMPING as u8, 0.0), 1.0);
        assert_eq!(q(tube::TUBE_PARAM_XFMR_RES_HZ as u8, 200.0), 150.0);
        assert_eq!(q(tube::TUBE_PARAM_TRIM_DB as u8, -13.0), -12.0);
        assert!(is_invalid(tube_quantize(TUBE_PARAM_COUNT as u8, 0.0)));
        assert!(is_invalid(tube_quantize(T_DRIVE, f32::NAN)));
    }

    #[test]
    fn wp_c4_tube_type_rows_and_custom_reset() {
        // Every type loads the firmware's row; Custom keeps the knobs.
        for (k, row) in TUBE_ROWS.iter().enumerate() {
            let mut t = state::default_state().tube;
            tube_apply(&mut t, T_TYPE, (k + 1) as f32);
            assert_eq!(&t[4..8], row, "type {}", k + 1);
            assert_eq!(t[2], (k + 1) as f32);
        }
        let mut t = state::default_state().tube;
        tube_apply(&mut t, T_TYPE, 16.0);
        // The same value: custom_reset_if sees no change.
        tube_apply(&mut t, T_BIAS, 12.0);
        assert_eq!(t[2], 16.0);
        // A real change drops to Custom.
        tube_apply(&mut t, T_SAG, 40.0);
        assert_eq!(t[2], 0.0);
        assert_eq!(t[7], 40.0);
        tube_apply(&mut t, T_TYPE, 0.0);
        assert_eq!(t[7], 40.0, "Custom keeps the values");
        // A non-character knob never touches the type.
        tube_apply(&mut t, T_TYPE, 3.0);
        tube_apply(&mut t, T_DRIVE, 5.0);
        assert_eq!(t[2], 3.0);
    }

    #[test]
    fn wp_c4_limiter_values_clamp_like_lm_set_one() {
        let q = |p, v| limiter_quantize(p, v).unwrap().1;
        assert_eq!(q(ON, 0.3), 1.0);
        assert_eq!(q(ON, -2.0), 1.0);
        assert_eq!(q(ON, 0.0), 0.0);
        assert_eq!(q(THR, -40.0), -30.0);
        assert_eq!(q(THR, 5.0), 0.0);
        assert_eq!(q(REL, 5.0), 10.0);
        assert_eq!(q(REL, 5000.0), 1000.0);
        assert_eq!(q(LINK, 2.5), 3.0);
        assert_eq!(q(LINK, 9.0), 4.0);
        assert_eq!(q(LINK, -1.0), 0.0);
        assert!(is_invalid(limiter_quantize(4, 0.0)));
        assert!(is_invalid(limiter_quantize(THR, f32::NAN)));
    }

    fn lim_state(n: u8) -> DspState {
        let mut s = state::default_state();
        s.num_output_channels = n;
        s
    }

    #[test]
    fn wp_c4_limiter_link_groups_follow_limiter_set_param() {
        let mut s = lim_state(4);
        limiters(&mut s).set(1, THR, -9.0);
        // Joining a group whose first member is the joiner keeps its own.
        limiters(&mut s).set(1, LINK, 2.0);
        assert_eq!(s.limiter_threshold_db[1], -9.0);
        // Joining an existing group adopts the group's settings.
        limiters(&mut s).set(3, LINK, 2.0);
        assert_eq!(s.limiter_threshold_db[3], -9.0);
        // A write to one member moves the whole group, nothing else.
        limiters(&mut s).set(3, THR, -12.0);
        assert_eq!(s.limiter_threshold_db[1], -12.0);
        assert_eq!(s.limiter_threshold_db[0], 0.0);
        limiters(&mut s).set(1, ON, 1.0);
        assert!(s.limiter_enabled[3] && s.limiter_enabled[1] && !s.limiter_enabled[0]);
        // An unlinked output moves alone.
        limiters(&mut s).set(0, REL, 50.0);
        assert_eq!(s.limiter_release_ms[0], 50.0);
        assert_eq!(s.limiter_release_ms[1], 100.0);
        // Every output: each group gangs to its lowest member (output 0).
        limiters(&mut s).set(ALL, LINK, 1.0);
        assert!(s.limiter_link_group[..4].iter().all(|&g| g == 1));
        assert!(s.limiter_threshold_db[..4].iter().all(|&t| t == 0.0));
        assert!(s.limiter_release_ms[..4].iter().all(|&r| r == 50.0));
        assert!(s.limiter_enabled[..4].iter().all(|&e| !e));
        // Outputs past the device's are never touched.
        assert_eq!(s.limiter_link_group[4], 0);
        limiters(&mut s).set(ALL, THR, -3.0);
        assert!(s.limiter_threshold_db[..4].iter().all(|&t| t == -3.0));
        assert_eq!(s.limiter_threshold_db[4], 0.0);
    }

    #[test]
    fn wp_c4_limiter_mirror_survives_short_vectors() {
        let mut s = lim_state(9);
        s.limiter_threshold_db.truncate(2);
        limiters(&mut s).set(ALL, THR, -6.0);
        limiters(&mut s).set(8, THR, -6.0);
        assert_eq!(s.limiter_threshold_db, vec![-6.0, -6.0]);
    }

    #[test]
    fn wp_c4_status_and_meters_decode() {
        let mut d = vec![1u8, 0];
        for v in [8192i16 as u16, 16384, 32767, 0, 16384] {
            d.extend(v.to_le_bytes());
        }
        let st = decode_upmix_status(&d).unwrap();
        assert!(st.active);
        assert_eq!(st.correlation, 0.5);
        assert_eq!(st.balance, 1.0);
        assert_eq!(st.center_gain, 1.0);
        assert_eq!(st.rs_gain, 16384.0 / 32767.0);
        assert!(decode_upmix_status(&d[..11]).is_none());
        let neg = [0u8, 3, 0x00, 0xC0, 0, 0, 0, 0, 0, 0, 0, 0];
        assert_eq!(decode_upmix_status(&neg).unwrap().correlation, -1.0);

        let m = meter(&[0x10, 0x27, 0, 0], 2, 100.0, "m").unwrap();
        assert_eq!(m.len(), MAX_OUTPUTS);
        assert_eq!(m[0], 100.0);
        assert!(matches!(
            meter(&[0, 0, 0], 2, 100.0, "m"),
            Err(DspiError::Transfer { .. })
        ));
    }

    proptest! {
        #[test]
        fn proptest_wp_c4_quantizers_stay_in_the_firmware_ranges(
            id in 0u8..20,
            v in any::<f32>(),
        ) {
            type Check = (Result<(&'static str, f32)>, Option<(f32, f32)>);
            let checks: [Check; 5] = [
                (psybass_quantize(id, v), PSYBASS.get(usize::from(id)).map(|r| (r.1, r.2))),
                (upmix_quantize(id, v), UPMIX.get(usize::from(id)).map(|r| (r.1, r.2))),
                (subharm_quantize(id, v), SUBHARM.get(usize::from(id)).map(|r| (r.1, r.2))),
                (tube_quantize(id, v), TUBE.get(usize::from(id)).map(|r| (r.1, r.2))),
                (limiter_quantize(id, v), LIMITER.get(usize::from(id)).map(|_| (-30.0, 1000.0))),
            ];
            for (r, range) in checks {
                match (r, range) {
                    (Ok((path, q)), Some((lo, hi))) => {
                        prop_assert!(!v.is_nan());
                        prop_assert!(q >= lo && q <= hi, "{path}: {v} -> {q}");
                        // The session accepts what the quantizer produces.
                        let d = by_path(path).unwrap();
                        prop_assert!(d.kind.validate(&value_for(path, q).unwrap()).is_ok());
                    }
                    (Err(_), Some(_)) => prop_assert!(v.is_nan()),
                    (Err(_), None) => {}
                    (Ok(_), None) => prop_assert!(false, "id {id} accepted"),
                }
            }
        }

        #[test]
        fn proptest_wp_c4_limiter_groups_never_panic(
            n in 0u8..12,
            ops in proptest::collection::vec((0u8..12, 0u8..4, -50.0f32..2000.0), 0..24),
        ) {
            let mut s = lim_state(n);
            for (o, p, v) in ops {
                let o = if o == 11 { ALL } else { o };
                let (_, q) = limiter_quantize(p, v).unwrap();
                limiters(&mut s).set(o, p, q);
            }
            // Linked outputs agree after any sequence ending in a gang.
            limiters(&mut s).set(ALL, LINK, 1.0);
            let k = usize::from(n).min(MAX_OUTPUTS);
            prop_assert!(s.limiter_threshold_db[..k].windows(2).all(|w| w[0] == w[1]));
        }
    }

    // ── Against the fake device ─────────────────────────────────────

    #[test]
    fn wp_c4_psybass_setters_reach_the_device_and_the_mirror() {
        let (core, remote, _) = rig();
        core.set_psybass_enabled(true).unwrap();
        core.set_psybass_mask(0x0005).unwrap();
        core.set_psybass_param(PSYBASS_PARAM_CUTOFF, 1000.0)
            .unwrap();
        core.set_psybass_param(PSYBASS_PARAM_HARMONICS, -3.5)
            .unwrap();
        core.set_psybass_param(PSYBASS_PARAM_DRIVE, 9.0).unwrap();
        core.set_psybass_param(PSYBASS_PARAM_CHARACTER, 75.0)
            .unwrap();
        core.set_psybass_param(PSYBASS_PARAM_ORIGINAL, -70.0)
            .unwrap();
        for s in [mirror(&core), device(&remote), image_of_mirror(&core)] {
            assert!(s.psybass_enabled);
            assert_eq!(s.psybass_output_mask, 0x0005);
            assert_eq!(s.psybass_cutoff_hz, 300.0);
            assert_eq!(s.psybass_harmonics_db, -3.5);
            assert_eq!(s.psybass_drive_db, 9.0);
            assert_eq!(s.psybass_character_pct, 75.0);
            assert_eq!(s.psybass_original_db, -60.0);
        }
    }

    #[test]
    fn wp_c4_upmix_setters_reach_the_device_and_the_mirror() {
        let (core, remote, _) = rig();
        let writes: [(u8, f32, f32); 6] = [
            (UPMIX_PARAM_ENABLED, 1.0, 1.0),
            (UPMIX_PARAM_CENTER_MODE, 2.0, 2.0),
            (UPMIX_PARAM_SURROUND_MODE, 9.0, 2.0),
            (UPMIX_PARAM_PRESENCE, -4.3, -4.5),
            (UPMIX_PARAM_STRENGTH, 60.0, 60.0),
            (UPMIX_PARAM_SURROUND_DELAY, 40.0, 20.0),
        ];
        for (id, v, _) in writes {
            core.set_upmix_param(id, v).unwrap();
        }
        // An out-of-range centre mode lands on adaptive on both sides.
        core.set_upmix_param(UPMIX_PARAM_CENTER_MODE, 5.0).unwrap();
        for s in [mirror(&core), device(&remote), image_of_mirror(&core)] {
            for (id, _, want) in writes.iter().filter(|w| w.0 != UPMIX_PARAM_CENTER_MODE) {
                assert_eq!(s.upmix[usize::from(*id)], *want, "upmix {id}");
            }
            assert_eq!(s.upmix[usize::from(UPMIX_PARAM_CENTER_MODE)], 1.0);
        }
        let st = core.fetch_upmix_status().unwrap();
        assert!(st.active);
    }

    #[test]
    fn wp_c4_subharm_setters_reach_the_device_and_the_mirror() {
        let (core, remote, _) = rig();
        let writes: [(u8, f32, f32); 8] = [
            (SUBHARM_PARAM_ENABLED, 1.0, 1.0),
            (SUBHARM_PARAM_MASK, 3.0, 3.0),
            (SUBHARM_PARAM_LOW, -50.0, -30.0),
            (SUBHARM_PARAM_BOOST, 2.5, 2.5),
            (SUBHARM_PARAM_SELECT, 9.0, 2.0),
            (SUBHARM_PARAM_HOLD, 10.0, 50.0),
            (SUBHARM_PARAM_CEILING, -6.0, -6.0),
            (SUBHARM_PARAM_LINK, 0.0, 0.0),
        ];
        for (id, v, _) in writes {
            core.set_subharm_param(id, v).unwrap();
        }
        for s in [mirror(&core), device(&remote), image_of_mirror(&core)] {
            for (id, _, want) in writes {
                assert_eq!(s.subharm[usize::from(id)], want, "subharm {id}");
            }
        }
    }

    #[test]
    fn wp_c4_tube_setters_reach_the_device_and_the_mirror() {
        let (core, remote, _) = rig();
        core.set_tube_param(tube::TUBE_PARAM_ENABLED as u8, 1.0)
            .unwrap();
        core.set_tube_param(T_MASK, 6.0).unwrap();
        core.set_tube_param(T_DRIVE, 99.0).unwrap();
        core.set_tube_param(T_RECT, 2.4).unwrap();
        for s in [mirror(&core), device(&remote), image_of_mirror(&core)] {
            assert_eq!(s.tube[0], 1.0);
            assert_eq!(s.tube[1], 6.0);
            assert_eq!(s.tube[3], 24.0);
            assert_eq!(s.tube[8], 2.0);
        }
        // A type change loads its row in the mirror (the fake does not
        // model rows: its type alone is compared).
        core.set_tube_param(T_TYPE, 16.0).unwrap();
        assert_eq!(device(&remote).tube[2], 16.0);
        let m = mirror(&core);
        assert_eq!(&m.tube[4..8], &TUBE_ROWS[15]);
        assert_eq!(image_of_mirror(&core).tube, m.tube);
        core.set_tube_param(T_SAG, 33.0).unwrap();
        assert_eq!(mirror(&core).tube[2], 0.0, "custom_reset_if");
        assert_eq!(device(&remote).tube[7], 33.0);
    }

    #[test]
    fn wp_c4_limiter_setters_follow_link_groups_on_the_device() {
        let (core, remote, _) = rig();
        core.set_limiter_param(ALL, THR, -3.0).unwrap();
        core.set_limiter_param(1, THR, -9.0).unwrap();
        core.set_limiter_param(1, LINK, 2.0).unwrap();
        core.set_limiter_param(4, LINK, 2.0).unwrap();
        core.set_limiter_param(4, THR, -40.0).unwrap();
        core.set_limiter_param(4, REL, 5.0).unwrap();
        core.set_limiter_param(1, ON, 0.5).unwrap();
        let (m, d) = (mirror(&core), device(&remote));
        let n = usize::from(m.num_output_channels);
        assert_eq!(n, 9);
        for s in [&m, &d, &image_of_mirror(&core)] {
            assert_eq!(s.limiter_threshold_db[1], -30.0);
            assert_eq!(s.limiter_threshold_db[4], -30.0);
            assert_eq!(s.limiter_threshold_db[0], -3.0);
            assert_eq!(s.limiter_release_ms[1], 10.0);
            assert!(s.limiter_enabled[4] && !s.limiter_enabled[0]);
        }
        // The mirror and the device agree on every output.
        assert_eq!(m.limiter_threshold_db[..n], d.limiter_threshold_db[..n]);
        assert_eq!(m.limiter_release_ms[..n], d.limiter_release_ms[..n]);
        assert_eq!(m.limiter_link_group[..n], d.limiter_link_group[..n]);
        assert_eq!(m.limiter_enabled[..n], d.limiter_enabled[..n]);
        assert_eq!(core.fetch_limiter_meter().unwrap(), vec![0.0; MAX_OUTPUTS]);
    }

    #[test]
    fn wp_c4_bad_arguments_send_nothing() {
        let (core, remote, script) = rig();
        let before = remote.image();
        assert!(is_invalid(core.set_psybass_param(9, 1.0)));
        assert!(is_invalid(
            core.set_upmix_param(UPMIX_PARAM_COUNT as u8, 1.0)
        ));
        assert!(is_invalid(
            core.set_subharm_param(SUBHARM_PARAM_COUNT as u8, 1.0)
        ));
        assert!(is_invalid(core.set_tube_param(TUBE_PARAM_COUNT as u8, 1.0)));
        assert!(is_invalid(core.set_limiter_param(0, 4, 1.0)));
        assert!(is_invalid(core.set_limiter_param(0, THR, f32::NAN)));
        assert!(is_invalid(core.set_tube_param(T_DRIVE, f32::NAN)));
        assert_eq!(traffic(&script), 0);
        // An output past the device's: the session refuses it unsent.
        assert!(is_invalid(core.set_limiter_param(9, THR, -1.0)));
        assert_eq!(traffic(&script), 0);
        assert_eq!(remote.image(), before);
    }

    #[test]
    fn wp_c4_read_only_and_disconnected_are_refused() {
        let core = DspiCore::new();
        assert_eq!(core.set_psybass_enabled(true), Err(DspiError::Disconnected));
        assert_eq!(core.fetch_limiter_meter(), Err(DspiError::Disconnected));
        assert_eq!(core.fetch_upmix_status(), Err(DspiError::Disconnected));

        let (core, remote, script) = rig();
        core.lock().conn().unwrap().wire = Some(WireRead::Older);
        let before = mirror(&core);
        assert_eq!(core.set_tube_param(T_DRIVE, 1.0), Err(DspiError::ReadOnly));
        assert_eq!(
            core.set_limiter_param(ALL, ON, 1.0),
            Err(DspiError::ReadOnly)
        );
        assert_eq!(core.set_subharm_solo(true), Err(DspiError::ReadOnly));
        assert_eq!(traffic(&script), 0);
        assert_eq!(mirror(&core), before);
        // Reads still work on a read-only connection.
        core.fetch_limiter_meter().unwrap();
        core.fetch_upmix_status().unwrap();
        drop(remote);
    }

    #[test]
    fn wp_c4_a_stalled_write_leaves_the_mirror_alone() {
        // Subharm solo is not modelled by the fake: it stalls the SET.
        let (core, _, _) = rig();
        let before = mirror(&core);
        assert!(matches!(
            core.set_subharm_solo(true),
            Err(DspiError::Transfer { .. })
        ));
        assert_eq!(mirror(&core), before);
    }

    #[test]
    fn wp_c4_an_acked_but_ignored_write_is_proven_by_read_back() {
        // With `ack_unknown_sets` the fake ACKs the solo SET it does not
        // model, as the firmware ACKs what it drops: the call succeeds and
        // only a read-back could tell. The mirror records the request.
        let (core, remote, script) = rig_with(FakeOptions {
            ack_unknown_sets: true,
            ..FakeOptions::default()
        });
        core.set_subharm_solo(true).unwrap();
        assert!(mirror(&core).subharm_solo);
        assert!(
            script
                .lock()
                .unwrap()
                .log
                .contains(&(true, op::REQ_SET_SUBHARM_SOLO, 0))
        );
        // A modelled write, by contrast, is visible in the device's copy.
        core.set_subharm_param(SUBHARM_PARAM_DEPTH, 42.0).unwrap();
        assert_eq!(
            device(&remote).subharm[usize::from(SUBHARM_PARAM_DEPTH)],
            42.0
        );
    }

    #[test]
    fn wp_c4_subharm_meter_and_headroom_read_the_device() {
        let (core, _, script) = rig();
        let mut meter_reply = Vec::new();
        for k in 0..9u16 {
            meter_reply.extend((k * 1000).to_le_bytes());
        }
        script.lock().unwrap().replies = vec![
            (
                op::REQ_GET_SUBHARM_HEADROOM,
                (-4.5f32).to_le_bytes().to_vec(),
            ),
            (op::REQ_GET_SUBHARM_METER, meter_reply),
        ];
        assert_eq!(core.fetch_subharm_headroom().unwrap(), -4.5);
        let m = core.fetch_subharm_meter().unwrap();
        assert_eq!(m.len(), MAX_OUTPUTS);
        assert_eq!(m[2], 2000.0 / 32767.0);
        assert_eq!(m[8], 8000.0 / 32767.0);

        // A short reply is a transfer error, not a panic.
        script.lock().unwrap().replies = vec![
            (op::REQ_GET_SUBHARM_HEADROOM, vec![0, 0]),
            (op::REQ_GET_SUBHARM_METER, vec![0; 5]),
        ];
        assert!(matches!(
            core.fetch_subharm_headroom(),
            Err(DspiError::Transfer { .. })
        ));
        assert!(matches!(
            core.fetch_subharm_meter(),
            Err(DspiError::Transfer { .. })
        ));
    }
}
