//! Limits profiles (WP P1LM, PRO1 S1): the ranges an installer narrows so a
//! system cannot be pushed past what it was set up for.
//!
//! A **profile** belongs to one device (by serial) and is kept host-side: a
//! `.dspipreset` document of the one family (`meta.kind = "limits"`, P1DF)
//! with a `limits` block, and carried in an exported document's
//! `meta.limits`. It may narrow, per output, the gain (never above the
//! registry's +20 dB), the delay range, a high-pass the output must keep (a
//! lowest corner and a least slope) and a limiter that must stay on at or
//! below a ceiling; for the whole device the PEQ boost and the master volume.
//! It also names the pages an installer locks (P1LK reads them) and the
//! installer's note and contact.
//!
//! **The PIN is not in the profile.** Its hash is kept beside it in a
//! host-only file (`limits.pin.json`), salted per device with a random salt
//! and the serial, stretched with PBKDF2-HMAC-SHA256, and never read into a
//! document: not an export, a snapshot, a template or a diagnostics bundle.
//!
//! This module is the shared, pure half: the profile, its document, the
//! checks and clamps on plain values (so the core and the hub judge alike),
//! and on native targets the files ([`LimitsStore`]). Every file is written
//! atomically (a temporary file, then a rename), so a torn write leaves the
//! previous one.
//!
//! **A corrupt or missing profile** (OD-P1k): [`LimitsStore::load`] falls back
//! to the last good copy (`limits.good.dspipreset`) when one exists, and says
//! so; without one the device runs unlimited and the caller shows a standing
//! notice. The last resort for a forgotten PIN (OD-P1c) is deleting the
//! device's folder, which removes the profile, its good copy and the PIN.
#![warn(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

use dspi_proto::FilterType;
use dspi_proto::registry::{Kind, by_path};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// The document's block that holds a profile (`meta.kind = "limits"`), and
/// the key under `meta` an exported document carries it in.
pub const BLOCK: &str = "limits";

/// The profile's file in a device's folder.
pub const PROFILE_FILE: &str = "limits.dspipreset";

/// The last profile that read cleanly, the fallback for a corrupt one.
pub const GOOD_FILE: &str = "limits.good.dspipreset";

/// The PIN's hash: host-only, never in a document.
pub const PIN_FILE: &str = "limits.pin.json";

// -- WP P1LK --
/// The installer's recovery code's hash (OD-P1c), beside the PIN's: host-only,
/// never in a document. The code itself is shown once, when the PIN is set.
pub const RECOVERY_FILE: &str = "limits.recovery.json";

// -- WP P1DKFIX --
/// The wrong-PIN count and refusal window, beside the PIN's hash (F6): so a
/// restart does not reset the window. Host-only, never in a document.
pub const TRIES_FILE: &str = "limits.tries.json";

// -- WP P1DKFIX --
/// Wrong PINs allowed before the refusal window opens (the core's and the
/// hub's rule alike).
pub const LOCK_FREE_TRIES: u32 = 3;

// -- WP P1DKFIX --
/// The first refusal window, ms; each further wrong PIN doubles it.
pub const LOCK_FIRST_WAIT_MS: u64 = 30_000;

// -- WP P1DKFIX --
/// The longest refusal window, ms.
pub const LOCK_MAX_WAIT_MS: u64 = 15 * 60_000;

// -- WP P1DKFIX --
/// The refusal window wrong PIN number `wrong` opens: none for the
/// [`LOCK_FREE_TRIES`] free ones, then [`LOCK_FIRST_WAIT_MS`] doubling to at
/// most [`LOCK_MAX_WAIT_MS`].
pub fn lock_window_for(wrong: u32) -> Option<std::time::Duration> {
    let past = wrong.checked_sub(LOCK_FREE_TRIES)?.checked_sub(1)?;
    let factor = 1u64.checked_shl(past.min(20)).unwrap_or(u64::MAX);
    let ms = LOCK_FIRST_WAIT_MS
        .saturating_mul(factor)
        .min(LOCK_MAX_WAIT_MS);
    Some(std::time::Duration::from_millis(ms))
}

// -- WP P1DKFIX --
/// Wrong PINs as kept across a restart: the count since the last good PIN,
/// and the end of the refusal window on the wall clock (ms since the epoch;
/// 0: none). A reader trusts no window longer than [`LOCK_MAX_WAIT_MS`] from
/// its own clock.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredTries {
    /// Wrong PINs since the last good one.
    pub wrong: u32,
    /// The end of the refusal window, ms since the Unix epoch; 0: none.
    #[serde(default)]
    pub until_unix_ms: u64,
}

// -- WP P1DKFIX --
impl StoredTries {
    /// How long the window has left at `now_unix_ms`, at most
    /// [`LOCK_MAX_WAIT_MS`] (a clock set back never stretches it).
    pub fn left_ms(&self, now_unix_ms: u64) -> u64 {
        self.until_unix_ms
            .saturating_sub(now_unix_ms)
            .min(LOCK_MAX_WAIT_MS)
    }
}

/// The largest profile document read.
const MAX_DOC_BYTES: usize = 1 << 20;

/// The longest note, contact or page id kept, in characters.
const MAX_TEXT_CHARS: usize = 4000;

/// How far past a limit a value may sit and still pass: the setters quantize
/// to 0.1 dB and 0.0001 ms, and `f32` sums wander below that.
const SLACK: f32 = 1e-3;

/// What one output may do. `None` leaves that part as the device allows.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct OutputLimits {
    /// The output, 0-based.
    pub output: u8,
    /// The highest output gain, dB (at most +20, the registry's).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_gain_db: Option<f32>,
    /// The output must keep an active high-pass at or above this corner, Hz.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub highpass_min_hz: Option<f32>,
    /// ... and at least this steep, dB per octave (6 to 48); with no corner
    /// it means nothing.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub highpass_min_slope_db: Option<u8>,
    /// The shortest output delay, ms.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delay_min_ms: Option<f32>,
    /// The longest output delay, ms.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delay_max_ms: Option<f32>,
    /// The output's limiter must stay on with its threshold at or below this
    /// ceiling, dBFS.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limiter_ceiling_db: Option<f32>,
}

/// One device's limits profile. It never holds a PIN or its hash.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct LimitsProfile {
    /// The device it is for.
    pub serial: String,
    /// Per-output limits; an output not named is not limited.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub outputs: Vec<OutputLimits>,
    /// The most any PEQ band (peaking or shelf, on any channel) may boost, dB.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_peq_boost_db: Option<f32>,
    /// The highest master volume, dB (muting is always allowed).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub master_volume_max_db: Option<f32>,
    /// The pages an installer locks, by the mode manifest's page ids (P1LK).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub locked_pages: Vec<String>,
    /// The installer's note for whoever looks after the system.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub note: String,
    /// How to reach the installer.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub contact: String,
}

/// A value outside a profile's limit: why a write is refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Violation {
    /// Which limit: `out_gain`, `eq_boost`, `highpass`, `delay`, `limiter`
    /// or `master_volume`.
    pub code: &'static str,
    /// What and why, in words, for a notice's detail line.
    pub msg: String,
}

impl std::fmt::Display for Violation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.msg)
    }
}

/// Why a profile document could not be read.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LimitsError {
    /// Not JSON, too large, or a block of the wrong shape.
    #[error("the limits profile is unreadable: {0}")]
    Unreadable(String),
    /// A document with no profile in it.
    #[error("the document holds no limits profile")]
    NoProfile,
}

/// One band as the high-pass and boost rules see it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BandView {
    /// The wire `FilterType`.
    pub filter_type: u8,
    /// Bypassed by the user.
    pub bypass: bool,
    /// The corner or centre, Hz.
    pub freq: f32,
}

/// The slope of a high-pass type in dB per octave; `None` for anything else.
pub fn highpass_slope(filter_type: u8) -> Option<u8> {
    match filter_type {
        5 => Some(12), // FILTER_HIGHPASS (second order)
        13 => Some(6), // FILTER_HIGHPASS1
        33..=39 if filter_type % 2 == 1 => {
            // LR2..LR8: order 2, 4, 6, 8
            let order = (filter_type.saturating_sub(33) / 2)
                .saturating_add(1)
                .saturating_mul(2);
            Some(order.saturating_mul(6))
        }
        41..=55 if filter_type % 2 == 1 => {
            // BW1..BW8
            let order = (filter_type.saturating_sub(41) / 2).saturating_add(1);
            Some(order.saturating_mul(6))
        }
        57..=63 if filter_type % 2 == 1 => {
            // Bessel 2, 4, 6, 8
            let order = (filter_type.saturating_sub(57) / 2)
                .saturating_add(1)
                .saturating_mul(2);
            Some(order.saturating_mul(6))
        }
        _ => None,
    }
}

/// The Linkwitz-Riley high-pass crossover type at least `slope` dB/oct steep
/// (LR2 12, LR4 24, LR6 36, LR8 48): what a clamp adds where an output has
/// no high-pass at all.
pub fn lr_highpass_for(slope: u8) -> u8 {
    match slope {
        0..=12 => 33,
        13..=24 => 35,
        25..=36 => 37,
        _ => 39,
    }
}

/// Whether a band of `filter_type` boosts by its gain (peaking and shelves;
/// not the Linkwitz Transform, whose gain field is a frequency).
pub fn is_boost_type(filter_type: u8) -> bool {
    FilterType::from_raw(filter_type).uses_gain()
}

fn registry_range(path: &str) -> Option<(f32, f32)> {
    match by_path(path)?.kind {
        Kind::Float { min, max, .. } => Some((min, max)),
        _ => None,
    }
}

/// `v` held to `lo..=hi`; NaN and infinity read as no limit.
fn within(v: Option<f32>, lo: f32, hi: f32) -> Option<f32> {
    v.filter(|x| x.is_finite()).map(|x| x.clamp(lo, hi))
}

fn cut(s: &str) -> String {
    s.chars().take(MAX_TEXT_CHARS).collect()
}

fn db(v: f32) -> String {
    format!("{} dB", (v * 10.0).round() / 10.0)
}

fn ms(v: f32) -> String {
    format!("{} ms", (v * 10_000.0).round() / 10_000.0)
}

fn out_name(o: u8) -> String {
    format!("output {}", u16::from(o).saturating_add(1))
}

impl LimitsProfile {
    /// The profile with every number held to what the registry allows (a
    /// gain limit never above +20 dB), NaN and infinity dropped, a delay
    /// range put the right way round, outputs named once (the last wins),
    /// texts cut to length. A profile is always normalised before use.
    pub fn normalised(mut self) -> Self {
        let (gmin, gmax) = registry_range("out.gain").unwrap_or((-60.0, 20.0));
        let (_, boost_max) = registry_range("eq.gain").unwrap_or((-24.0, 24.0));
        let (fmin, fmax) = registry_range("eq.freq").unwrap_or((10.0, 24_000.0));
        let (tmin, tmax) = registry_range("limit.threshold").unwrap_or((-30.0, 0.0));
        let (vmin, vmax) = registry_range("vol.master").unwrap_or((-128.0, 0.0));
        let mut seen: Vec<OutputLimits> = Vec::new();
        for mut o in std::mem::take(&mut self.outputs) {
            o.max_gain_db = within(o.max_gain_db, gmin, gmax);
            o.highpass_min_hz = within(o.highpass_min_hz, fmin, fmax);
            o.highpass_min_slope_db = o.highpass_min_slope_db.map(|s| s.clamp(6, 48));
            o.delay_min_ms = within(o.delay_min_ms, 0.0, f32::MAX);
            o.delay_max_ms = within(o.delay_max_ms, 0.0, f32::MAX);
            if let (Some(lo), Some(hi)) = (o.delay_min_ms, o.delay_max_ms)
                && lo > hi
            {
                o.delay_min_ms = Some(hi);
                o.delay_max_ms = Some(lo);
            }
            o.limiter_ceiling_db = within(o.limiter_ceiling_db, tmin, tmax);
            seen.retain(|s| s.output != o.output);
            seen.push(o);
        }
        seen.sort_by_key(|o| o.output);
        self.outputs = seen;
        self.max_peq_boost_db = within(self.max_peq_boost_db, 0.0, boost_max);
        self.master_volume_max_db = within(self.master_volume_max_db, vmin, vmax);
        self.note = cut(&self.note);
        self.contact = cut(&self.contact);
        self.locked_pages = self
            .locked_pages
            .iter()
            .map(|p| cut(p))
            .filter(|p| !p.is_empty())
            .collect();
        self
    }

    /// The limits of `output`, if the profile names it.
    pub fn output(&self, output: u8) -> Option<&OutputLimits> {
        self.outputs.iter().find(|o| o.output == output)
    }

    /// Whether the profile limits anything a write can cross (a profile of
    /// only a note and locked pages limits no value).
    pub fn limits_values(&self) -> bool {
        self.max_peq_boost_db.is_some()
            || self.master_volume_max_db.is_some()
            || self.outputs.iter().any(|o| {
                o.max_gain_db.is_some()
                    || o.highpass_min_hz.is_some()
                    || o.delay_min_ms.is_some()
                    || o.delay_max_ms.is_some()
                    || o.limiter_ceiling_db.is_some()
            })
    }

    // ── Checks on plain values ────────────────────────────────────────────

    /// An output gain of `v` dB on `output`.
    pub fn check_output_gain(&self, output: u8, v: f32) -> Result<(), Violation> {
        match self.output(output).and_then(|o| o.max_gain_db) {
            Some(max) if v > max + SLACK => Err(Violation {
                code: "out_gain",
                msg: format!(
                    "{} gain {} is above the limit of {}",
                    out_name(output),
                    db(v),
                    db(max)
                ),
            }),
            _ => Ok(()),
        }
    }

    /// `v` held to `output`'s gain limit.
    pub fn clamp_output_gain(&self, output: u8, v: f32) -> f32 {
        match self.output(output).and_then(|o| o.max_gain_db) {
            Some(max) if v > max => max,
            _ => v,
        }
    }

    /// An output delay of `v` ms on `output`.
    pub fn check_delay(&self, output: u8, v: f32) -> Result<(), Violation> {
        let Some(o) = self.output(output) else {
            return Ok(());
        };
        let low = o.delay_min_ms.is_some_and(|lo| v < lo - SLACK);
        let high = o.delay_max_ms.is_some_and(|hi| v > hi + SLACK);
        if low || high {
            let range = match (o.delay_min_ms, o.delay_max_ms) {
                (Some(lo), Some(hi)) => format!("{} to {}", ms(lo), ms(hi)),
                (Some(lo), None) => format!("at least {}", ms(lo)),
                (None, Some(hi)) => format!("at most {}", ms(hi)),
                (None, None) => String::new(),
            };
            return Err(Violation {
                code: "delay",
                msg: format!(
                    "{} delay {} is outside the limit ({range})",
                    out_name(output),
                    ms(v)
                ),
            });
        }
        Ok(())
    }

    /// `v` held to `output`'s delay range.
    pub fn clamp_delay(&self, output: u8, v: f32) -> f32 {
        let Some(o) = self.output(output) else {
            return v;
        };
        let v = o.delay_min_ms.map_or(v, |lo| v.max(lo));
        o.delay_max_ms.map_or(v, |hi| v.min(hi))
    }

    /// A master volume of `v` dB; muting (-128 and below) always passes.
    pub fn check_master_volume(&self, v: f32) -> Result<(), Violation> {
        match self.master_volume_max_db {
            Some(max) if v > -128.0 && v > max + SLACK => Err(Violation {
                code: "master_volume",
                msg: format!("master volume {} is above the limit of {}", db(v), db(max)),
            }),
            _ => Ok(()),
        }
    }

    /// `v` held to the master volume ceiling.
    pub fn clamp_master_volume(&self, v: f32) -> f32 {
        match self.master_volume_max_db {
            Some(max) if v > max => max,
            _ => v,
        }
    }

    /// A band of `filter_type` with `gain` dB.
    pub fn check_boost(&self, filter_type: u8, gain: f32) -> Result<(), Violation> {
        match self.max_peq_boost_db {
            Some(max) if is_boost_type(filter_type) && gain > max + SLACK => Err(Violation {
                code: "eq_boost",
                msg: format!(
                    "an EQ boost of {} is above the limit of {}",
                    db(gain),
                    db(max)
                ),
            }),
            _ => Ok(()),
        }
    }

    /// `gain` held to the boost limit for a band of `filter_type`.
    pub fn clamp_boost(&self, filter_type: u8, gain: f32) -> f32 {
        match self.max_peq_boost_db {
            Some(max) if is_boost_type(filter_type) && gain > max => max,
            _ => gain,
        }
    }

    /// Whether `bands` (every band of `output`, PEQ and crossover) keep the
    /// high-pass the profile asks of it: one band active, a high-pass at
    /// least the least slope steep, its corner at or above the lowest.
    pub fn highpass_kept(&self, output: u8, bands: &[BandView]) -> bool {
        let Some(o) = self.output(output) else {
            return true;
        };
        let Some(min_hz) = o.highpass_min_hz else {
            return true;
        };
        let min_slope = o.highpass_min_slope_db.unwrap_or(6);
        bands.iter().any(|b| {
            !b.bypass
                && b.freq >= min_hz - SLACK
                && highpass_slope(b.filter_type).is_some_and(|s| s >= min_slope)
        })
    }

    /// A change of `output`'s bands from `before` to `after`: refused when
    /// the high-pass the profile asks for was kept and no longer is.
    pub fn check_highpass(
        &self,
        output: u8,
        before: &[BandView],
        after: &[BandView],
    ) -> Result<(), Violation> {
        if !self.highpass_kept(output, after) && self.highpass_kept(output, before) {
            let o = self.output(output);
            let hz = o.and_then(|o| o.highpass_min_hz).unwrap_or_default();
            let slope = o.and_then(|o| o.highpass_min_slope_db).unwrap_or(6);
            return Err(Violation {
                code: "highpass",
                msg: format!(
                    "{} must keep a high-pass of at least {slope} dB/oct at {} Hz or above",
                    out_name(output),
                    hz.round()
                ),
            });
        }
        Ok(())
    }

    /// `output`'s limiter `enabled` with `threshold_db`.
    pub fn check_limiter(
        &self,
        output: u8,
        enabled: bool,
        threshold_db: f32,
    ) -> Result<(), Violation> {
        let Some(ceiling) = self.output(output).and_then(|o| o.limiter_ceiling_db) else {
            return Ok(());
        };
        if !enabled {
            return Err(Violation {
                code: "limiter",
                msg: format!("{}'s limiter must stay on", out_name(output)),
            });
        }
        if threshold_db > ceiling + SLACK {
            return Err(Violation {
                code: "limiter",
                msg: format!(
                    "{}'s limiter threshold {} is above the ceiling of {}",
                    out_name(output),
                    db(threshold_db),
                    db(ceiling)
                ),
            });
        }
        Ok(())
    }

    // ── The document ──────────────────────────────────────────────────────

    /// The profile as a document of the one family: `meta.kind = "limits"`
    /// and the profile in the `limits` block. Never a PIN.
    pub fn to_document(&self) -> String {
        let doc = json!({
            "schemaVersion": 1,
            "meta": { "kind": "limits", "name": "Limits profile" },
            BLOCK: self,
        });
        serde_json::to_string_pretty(&doc).unwrap_or_default()
    }

    /// The profile as `meta.limits` carries it in an exported document.
    pub fn to_value(&self) -> Value {
        serde_json::to_value(self).unwrap_or(Value::Null)
    }

    /// Put the profile in `doc`'s `meta.limits` (an export).
    pub fn embed(&self, doc: &mut Value) {
        if let Some(meta) = doc
            .as_object_mut()
            .and_then(|o| o.entry("meta").or_insert_with(|| json!({})).as_object_mut())
        {
            meta.insert(BLOCK.into(), self.to_value());
        }
    }
}

/// Read a profile from a document: a limits document's `limits` block, or
/// any document's `meta.limits` (an export that carries one). Normalised.
pub fn from_document(text: &str) -> Result<LimitsProfile, LimitsError> {
    if text.len() > MAX_DOC_BYTES {
        return Err(LimitsError::Unreadable("the file is too large".into()));
    }
    let doc: Value =
        serde_json::from_str(text).map_err(|e| LimitsError::Unreadable(e.to_string()))?;
    from_value(&doc)
}

/// [`from_document`] for a parsed document.
pub fn from_value(doc: &Value) -> Result<LimitsProfile, LimitsError> {
    if !doc.is_object() {
        return Err(LimitsError::Unreadable("not a JSON object".into()));
    }
    let block = doc
        .get(BLOCK)
        .filter(|_| {
            doc.pointer("/meta/kind")
                .and_then(Value::as_str)
                .is_some_and(|k| k.trim().eq_ignore_ascii_case("limits"))
        })
        .or_else(|| doc.pointer("/meta/limits"))
        .ok_or(LimitsError::NoProfile)?;
    if !block.is_object() {
        return Err(LimitsError::Unreadable(
            "the limits block is not an object".into(),
        ));
    }
    serde_json::from_value::<LimitsProfile>(block.clone())
        .map(LimitsProfile::normalised)
        .map_err(|e| LimitsError::Unreadable(e.to_string()))
}

// ── The PIN's hash ─────────────────────────────────────────────────────────

/// PBKDF2 iterations a new PIN hash is made with. The file keeps its own
/// count, so a later build may raise this without losing a PIN.
pub const PIN_ITERATIONS: u32 = 100_000;

mod kdf {
    //! HMAC-SHA256 and PBKDF2 over it (RFC 2104, RFC 8018), on the `sha2` the
    //! hub already uses: a slow, salted hash with no new dependency.
    use sha2::{Digest, Sha256};

    const BLOCK: usize = 64;

    fn hmac(key: &[u8], parts: &[&[u8]]) -> [u8; 32] {
        let mut k = [0u8; BLOCK];
        if key.len() > BLOCK {
            let h: [u8; 32] = Sha256::digest(key).into();
            k.iter_mut().zip(h).for_each(|(d, s)| *d = s);
        } else {
            k.iter_mut().zip(key).for_each(|(d, s)| *d = *s);
        }
        let mut inner = Sha256::new();
        inner.update(k.map(|b| b ^ 0x36));
        for p in parts {
            inner.update(p);
        }
        let inner: [u8; 32] = inner.finalize().into();
        let mut outer = Sha256::new();
        outer.update(k.map(|b| b ^ 0x5c));
        outer.update(inner);
        outer.finalize().into()
    }

    /// PBKDF2-HMAC-SHA256, one 32-byte block.
    pub(super) fn pbkdf2(password: &[u8], salt: &[u8], iterations: u32) -> [u8; 32] {
        let mut u = hmac(password, &[salt, &1u32.to_be_bytes()]);
        let mut out = u;
        for _ in 1..iterations.max(1) {
            u = hmac(password, &[&u]);
            out.iter_mut().zip(u).for_each(|(o, x)| *o ^= x);
        }
        out
    }

    #[cfg(test)]
    pub(super) fn hmac_for_test(key: &[u8], msg: &[u8]) -> [u8; 32] {
        hmac(key, &[msg])
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex(text: &str) -> Option<Vec<u8>> {
    if !text.len().is_multiple_of(2) {
        return None;
    }
    (0..text.len())
        .step_by(2)
        .map(|i| {
            text.get(i..i.saturating_add(2))
                .and_then(|h| u8::from_str_radix(h, 16).ok())
        })
        .collect()
}

/// A stored PIN hash: `{version, kdf, iterations, salt, hash}`; the salt is
/// random, and the serial is appended to it before hashing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PinRecord {
    /// The record format, 1.
    pub version: u32,
    /// `pbkdf2-sha256`.
    pub kdf: String,
    /// PBKDF2 iterations.
    pub iterations: u32,
    /// The random salt, hex.
    pub salt: String,
    /// The hash, hex.
    pub hash: String,
}

impl PinRecord {
    /// A record of `pin` for `serial` with `salt` and `iterations`.
    pub fn make(serial: &str, pin: &str, salt: &[u8], iterations: u32) -> Self {
        let full: Vec<u8> = salt.iter().chain(serial.as_bytes()).copied().collect();
        Self {
            version: 1,
            kdf: "pbkdf2-sha256".into(),
            iterations,
            salt: hex(salt),
            hash: hex(&kdf::pbkdf2(pin.as_bytes(), &full, iterations)),
        }
    }

    /// Whether `pin` is the PIN of this record for `serial` (compared in
    /// constant time). An unreadable record matches nothing.
    pub fn matches(&self, serial: &str, pin: &str) -> bool {
        let (Some(salt), Some(want)) = (unhex(&self.salt), unhex(&self.hash)) else {
            return false;
        };
        if self.kdf != "pbkdf2-sha256" || want.len() != 32 || self.iterations == 0 {
            return false;
        }
        let got = Self::make(serial, pin, &salt, self.iterations);
        let got = unhex(&got.hash).unwrap_or_default();
        got.len() == want.len() && got.iter().zip(&want).fold(0u8, |a, (x, y)| a | (x ^ y)) == 0
    }
}

// ── The files ──────────────────────────────────────────────────────────────

/// How a device's profile was found ([`LimitsStore::load`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoadState {
    /// No profile, no good copy: the device runs as today.
    None,
    /// The profile read cleanly.
    Ok,
    /// The profile is missing or unreadable (`why`); the last good copy is
    /// in force. Fails closed.
    FellBack {
        /// Why the profile was not used.
        why: String,
    },
    /// The profile is unreadable (`why`) and no good copy exists: the device
    /// runs unlimited, with a standing notice.
    Unreadable {
        /// Why.
        why: String,
    },
}

/// A device's profile as loaded.
#[derive(Debug, Clone, PartialEq)]
pub struct Loaded {
    /// The profile in force, if any.
    pub profile: Option<LimitsProfile>,
    /// How it was found.
    pub state: LoadState,
}

#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
pub use files::LimitsStore;
// -- WP P1LK --
#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
pub use files::random_bytes;

#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
mod files {
    //! The file side, gated off on the web (no files there).

    use std::fs;
    use std::io;
    use std::path::{Path, PathBuf};

    use super::{
        GOOD_FILE, LimitsProfile, LoadState, Loaded, PIN_FILE, PIN_ITERATIONS, PROFILE_FILE,
        PinRecord, RECOVERY_FILE, StoredTries, TRIES_FILE, from_document,
    };
    use crate::preset_file::save_file;
    use crate::snapshot_store::dir_name;

    /// Profiles and PIN hashes under one folder, one subfolder per serial
    /// (named as the snapshot store and the event log name theirs).
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct LimitsStore {
        root: PathBuf,
    }

    fn create_dir(dir: &Path) -> io::Result<()> {
        let mut b = fs::DirBuilder::new();
        b.recursive(true);
        #[cfg(unix)]
        std::os::unix::fs::DirBuilderExt::mode(&mut b, 0o700);
        b.create(dir)
    }

    fn remove(path: &Path) -> io::Result<()> {
        match fs::remove_file(path) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
            r => r,
        }
    }

    fn read(path: &Path) -> io::Result<Option<String>> {
        match fs::read(path) {
            Ok(b) => Ok(Some(String::from_utf8_lossy(&b).into_owned())),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e),
        }
    }

    impl LimitsStore {
        /// A store in `root`, the client's own data folder. Nothing is
        /// touched until the first write.
        pub fn new(root: impl Into<PathBuf>) -> Self {
            Self { root: root.into() }
        }

        /// The folder of `serial`'s files.
        pub fn dir(&self, serial: &str) -> PathBuf {
            self.root.join(dir_name(serial))
        }

        /// `serial`'s profile: the profile file, else (missing or
        /// unreadable) the last good copy, else none. Never fails: a file
        /// that cannot be read is reported in the state.
        pub fn load(&self, serial: &str) -> Loaded {
            let dir = self.dir(serial);
            let main = read(&dir.join(PROFILE_FILE));
            let parsed = match &main {
                Ok(Some(text)) => Some(from_document(text).map_err(|e| e.to_string())),
                Ok(None) => None,
                Err(e) => Some(Err(e.to_string())),
            };
            let good = || {
                read(&dir.join(GOOD_FILE))
                    .ok()
                    .flatten()
                    .and_then(|t| from_document(&t).ok())
            };
            match parsed {
                Some(Ok(mut p)) => {
                    p.serial = serial.to_owned();
                    Loaded {
                        profile: Some(p),
                        state: LoadState::Ok,
                    }
                }
                Some(Err(why)) => match good() {
                    Some(mut p) => {
                        p.serial = serial.to_owned();
                        Loaded {
                            profile: Some(p),
                            state: LoadState::FellBack { why },
                        }
                    }
                    None => Loaded {
                        profile: None,
                        state: LoadState::Unreadable { why },
                    },
                },
                None => match good() {
                    Some(mut p) => {
                        p.serial = serial.to_owned();
                        Loaded {
                            profile: Some(p),
                            state: LoadState::FellBack {
                                why: "the profile file is missing".into(),
                            },
                        }
                    }
                    None => Loaded {
                        profile: None,
                        state: LoadState::None,
                    },
                },
            }
        }

        /// Keep `profile` for its serial: the profile file, then the good
        /// copy, each written atomically.
        pub fn save(&self, profile: &LimitsProfile) -> io::Result<()> {
            let dir = self.dir(&profile.serial);
            create_dir(&dir)?;
            let text = profile.to_document();
            save_file(&dir.join(PROFILE_FILE), &text)?;
            save_file(&dir.join(GOOD_FILE), &text)
        }

        /// Remove `serial`'s profile and its good copy (the PIN stays; see
        /// [`LimitsStore::clear_pin`]).
        pub fn clear(&self, serial: &str) -> io::Result<()> {
            let dir = self.dir(serial);
            remove(&dir.join(PROFILE_FILE))?;
            remove(&dir.join(GOOD_FILE))
        }

        /// Keep a hash of `pin` for `serial`: a fresh random salt, the
        /// serial, PBKDF2. The PIN itself is never written.
        pub fn set_pin(&self, serial: &str, pin: &str) -> io::Result<()> {
            let mut salt = [0u8; 16];
            getrandom::fill(&mut salt).map_err(|e| io::Error::other(e.to_string()))?;
            self.write_pin(&PinRecord::make(serial, pin, &salt, PIN_ITERATIONS), serial)
        }

        /// Write a PIN record (atomic).
        pub fn write_pin(&self, record: &PinRecord, serial: &str) -> io::Result<()> {
            let dir = self.dir(serial);
            create_dir(&dir)?;
            let text = serde_json::to_string_pretty(record).map_err(io::Error::other)?;
            save_file(&dir.join(PIN_FILE), &text)
        }

        /// `serial`'s PIN record; `None` when none is set. An unreadable
        /// record is an error, never "no PIN".
        pub fn pin(&self, serial: &str) -> io::Result<Option<PinRecord>> {
            let Some(text) = read(&self.dir(serial).join(PIN_FILE))? else {
                return Ok(None);
            };
            serde_json::from_str(&text)
                .map(Some)
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
        }

        /// Whether `pin` is `serial`'s PIN; `None` when no PIN is set.
        pub fn check_pin(&self, serial: &str, pin: &str) -> io::Result<Option<bool>> {
            Ok(self.pin(serial)?.map(|r| r.matches(serial, pin)))
        }

        /// Remove `serial`'s PIN.
        pub fn clear_pin(&self, serial: &str) -> io::Result<()> {
            remove(&self.dir(serial).join(PIN_FILE))
        }

        // -- WP P1LK --
        /// Keep the hash of `serial`'s recovery code (atomic), made as a PIN's.
        pub fn write_recovery(&self, record: &PinRecord, serial: &str) -> io::Result<()> {
            let dir = self.dir(serial);
            create_dir(&dir)?;
            let text = serde_json::to_string_pretty(record).map_err(io::Error::other)?;
            save_file(&dir.join(RECOVERY_FILE), &text)
        }

        // -- WP P1LK --
        /// `serial`'s recovery code record; `None` when none is kept. An
        /// unreadable record is an error.
        pub fn recovery(&self, serial: &str) -> io::Result<Option<PinRecord>> {
            let Some(text) = read(&self.dir(serial).join(RECOVERY_FILE))? else {
                return Ok(None);
            };
            serde_json::from_str(&text)
                .map(Some)
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
        }

        // -- WP P1LK --
        /// Remove `serial`'s recovery code.
        pub fn clear_recovery(&self, serial: &str) -> io::Result<()> {
            remove(&self.dir(serial).join(RECOVERY_FILE))
        }

        // -- WP P1DKFIX --
        /// Keep `serial`'s wrong-PIN count and window (atomic); a count of
        /// none removes the file.
        pub fn write_tries(&self, tries: &StoredTries, serial: &str) -> io::Result<()> {
            if *tries == StoredTries::default() {
                return self.clear_tries(serial);
            }
            let dir = self.dir(serial);
            create_dir(&dir)?;
            let text = serde_json::to_string_pretty(tries).map_err(io::Error::other)?;
            save_file(&dir.join(TRIES_FILE), &text)
        }

        // -- WP P1DKFIX --
        /// `serial`'s wrong-PIN count and window; none when no file is kept.
        /// An unreadable file is an error (the caller fails closed).
        pub fn tries(&self, serial: &str) -> io::Result<Option<StoredTries>> {
            let Some(text) = read(&self.dir(serial).join(TRIES_FILE))? else {
                return Ok(None);
            };
            serde_json::from_str(&text)
                .map(Some)
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
        }

        // -- WP P1DKFIX --
        /// Remove `serial`'s wrong-PIN count.
        pub fn clear_tries(&self, serial: &str) -> io::Result<()> {
            remove(&self.dir(serial).join(TRIES_FILE))
        }
    }

    // -- WP P1LK --
    /// `N` random bytes from the operating system (a salt, a recovery code).
    pub fn random_bytes<const N: usize>() -> io::Result<[u8; N]> {
        let mut b = [0u8; N];
        getrandom::fill(&mut b).map_err(|e| io::Error::other(e.to_string()))?;
        Ok(b)
    }
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_p1lm;

// -- WP P1DKFIX --
#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::arithmetic_side_effects)]
mod tests_wp_p1dkfix {
    use super::*;

    #[test]
    fn wp_p1dkfix_one_window_rule_for_the_core_and_the_hub() {
        for n in 0..=LOCK_FREE_TRIES {
            assert_eq!(lock_window_for(n), None, "{n}");
        }
        let ms = |n| lock_window_for(n).map(|d| d.as_millis() as u64);
        assert_eq!(ms(LOCK_FREE_TRIES + 1), Some(LOCK_FIRST_WAIT_MS));
        assert_eq!(ms(LOCK_FREE_TRIES + 2), Some(2 * LOCK_FIRST_WAIT_MS));
        assert_eq!(ms(u32::MAX), Some(LOCK_MAX_WAIT_MS));
        // A kept window is never longer than the longest from the reader's clock.
        let t = StoredTries {
            wrong: 9,
            until_unix_ms: u64::MAX,
        };
        assert_eq!(t.left_ms(1_000), LOCK_MAX_WAIT_MS);
        assert_eq!(
            StoredTries {
                wrong: 4,
                until_unix_ms: 5_000
            }
            .left_ms(6_000),
            0
        );
    }

    #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
    #[test]
    fn wp_p1dkfix_the_try_count_file_round_trips_and_clears() {
        let d = std::env::temp_dir()
            .join("wp-p1dkfix-session")
            .join(format!(
                "tries-{}-{:?}",
                std::process::id(),
                std::thread::current().id()
            ));
        let _ = std::fs::remove_dir_all(&d);
        let store = LimitsStore::new(&d);
        assert_eq!(store.tries("S1").unwrap(), None);
        let t = StoredTries {
            wrong: 5,
            until_unix_ms: 123,
        };
        store.write_tries(&t, "S1").unwrap();
        assert_eq!(store.tries("S1").unwrap(), Some(t));
        store.write_tries(&StoredTries::default(), "S1").unwrap();
        assert_eq!(store.tries("S1").unwrap(), None, "none kept removes it");
        let _ = std::fs::remove_dir_all(&d);
    }
}
