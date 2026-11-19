//! Microphone calibration files (WP P5CAL, PLAN-P5 T7).
//!
//! Parses the common calibration text formats into points, puts them on a
//! [`LogGrid`], applies them to a measurement and turns a sensitivity or a
//! calibrator tone into a dB SPL scale.
//!
//! Formats (detected from the text):
//! - **miniDSP UMIK-1 / UMIK-2** (`umik`): a first line
//!   `"Sens Factor =-1.378dB, AGain =18dB, SERNO: 7012345"` (the `AGain` part only
//!   on a UMIK-2), then `frequency dB [phase]` lines. miniDSP ships a 0° and a 90°
//!   file per microphone; the 90° one says so in its file name (`…_90deg.txt`),
//!   which is how [`MicAngle::Deg90`] is recognised (or a comment or header line
//!   saying `90deg` / `90°`).
//! - **REW calibration text** (`rew`): `frequency dB [phase]` separated by spaces
//!   or tabs; lines starting with `*`, `#` or `"` are comments; an optional
//!   `Sensitivity -12.34 dBFS` line (REW's own convention for analogue
//!   microphones: the dBFS level at 94 dB SPL).
//! - **CSV** (`csv`): comma, semicolon (then a decimal comma is allowed) or tab
//!   separated, with at most one header line before the data.
//! - **Dayton Audio iMM-6C / UMM-6** (`dayton`, WP P5DAYTON): a first line
//!   `*1000Hz<TAB>-36.1`, the microphone's sensitivity at 1 kHz in dBV/Pa
//!   (spaces or tabs, `1000 Hz` and `1kHz` too), usually CRLF and a blank line,
//!   then `frequency dB` lines as REW text. A UMM-6 file may also carry a
//!   `Sens Factor =…dB, SERNO: …` line: that is the USB microphone's dBFS
//!   convention as on a UMIK and wins for the dB SPL scale (the file is then
//!   read as `umik`, vendor Dayton Audio). A sensitivity that is not finite or
//!   outside [`DAYTON_MIN_DBV_PA`]..=[`DAYTON_MAX_DBV_PA`] is refused.
//!
//! Files are untrusted (PLAN-P5 section 3): at most [`MAX_FILE_BYTES`] and
//! [`MAX_FILE_LINES`]; a non-finite value is refused with its line number; points
//! outside [`CAL_MIN_HZ`] to [`CAL_MAX_HZ`] are ignored (and counted); points are
//! sorted and de-duplicated (the first of equal frequencies is kept). Errors name
//! the line, never the file's contents.

use crate::types::{GRID_MAX_HZ, GRID_MIN_HZ, LogGrid, MeasureError, Response};
use serde::{Deserialize, Serialize};

#[cfg(test)]
mod tests;

/// The largest calibration, target or compensation file accepted, bytes (1 MiB).
pub const MAX_FILE_BYTES: usize = 1024 * 1024;
/// The most lines such a file may have (comments included).
pub const MAX_FILE_LINES: usize = 8192;
/// Points below this frequency are ignored, Hz.
pub const CAL_MIN_HZ: f64 = GRID_MIN_HZ;
/// Points above this frequency are ignored, Hz.
pub const CAL_MAX_HZ: f64 = GRID_MAX_HZ;
/// The level of a 94 dB calibrator, dB SPL.
pub const CALIBRATOR_94_DB: f64 = 94.0;
/// The level of a 114 dB calibrator, dB SPL.
pub const CALIBRATOR_114_DB: f64 = 114.0;
/// The most samples of a calibrator capture [`spl_offset_from_tone`] analyses
/// (the last ones are used; 65536 is about 1.4 s at 48 kHz).
pub const MAX_TONE_SAMPLES: usize = 65_536;
/// The fewest samples a calibrator capture must have.
pub const MIN_TONE_SAMPLES: usize = 4096;
/// The lowest Dayton-style 1 kHz sensitivity accepted, dBV/Pa (WP P5DAYTON).
pub const DAYTON_MIN_DBV_PA: f64 = -70.0;
/// The highest Dayton-style 1 kHz sensitivity accepted, dBV/Pa.
pub const DAYTON_MAX_DBV_PA: f64 = 0.0;
/// The input's full-scale level assumed when only a sensitivity in dBV/Pa is
/// known, dBV: a full-scale sine (0 dBFS) taken as 1 V RMS. The real input's
/// full-scale voltage and gain are not known to the host, so the scale it
/// gives is labelled approximate like every sensitivity scale (WP P5DAYTON).
pub const ASSUMED_FULL_SCALE_DBV: f64 = 0.0;

/// The calibration file's format, as detected. Serialised `umik`, `rew`,
/// `csv`, `dayton`: a `mic` document's `format` is this type
/// (`doc::MicFormat`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CalFormat {
    /// miniDSP UMIK-1 / UMIK-2 text with a `Sens Factor` line.
    Umik,
    /// REW calibration text (whitespace separated, `*` `#` `"` comments).
    Rew,
    /// Comma, semicolon or tab separated values, optionally with a header line.
    Csv,
    /// Dayton Audio iMM-6C / UMM-6 text: a `*1000Hz` sensitivity line (dBV/Pa)
    /// and no `Sens Factor` line (WP P5DAYTON, additive).
    Dayton,
}

/// The incidence the calibration is for. Serialised as the number of
/// degrees, `0` or `90`, as a `mic` document's `angle` holds it (review
/// P5-A: one spelling in `cal` and in `doc`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(into = "u16", try_from = "u16")]
pub enum MicAngle {
    /// On axis (0°): the microphone pointing at the source.
    #[default]
    Deg0,
    /// 90° incidence: the microphone pointing at the ceiling (room measurements).
    Deg90,
}

impl MicAngle {
    /// The angle in degrees.
    pub fn degrees(self) -> u16 {
        match self {
            MicAngle::Deg0 => 0,
            MicAngle::Deg90 => 90,
        }
    }

    /// The angle of a number of degrees: 0 or 90, nothing else.
    pub fn from_degrees(degrees: u16) -> Option<MicAngle> {
        match degrees {
            0 => Some(MicAngle::Deg0),
            90 => Some(MicAngle::Deg90),
            _ => None,
        }
    }
}

impl From<MicAngle> for u16 {
    fn from(a: MicAngle) -> u16 {
        a.degrees()
    }
}

impl TryFrom<u16> for MicAngle {
    type Error = String;

    fn try_from(degrees: u16) -> Result<MicAngle, String> {
        MicAngle::from_degrees(degrees).ok_or_else(|| format!("angle {degrees} (0 or 90)"))
    }
}

/// One point of a calibration, target or compensation curve.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CurvePoint {
    /// Frequency, Hz.
    pub freq_hz: f64,
    /// Level, dB.
    pub db: f64,
    /// Phase, degrees, when the file gives it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phase_deg: Option<f64>,
}

/// A parsed curve file: the points and what the text said about itself.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ParsedCurve {
    /// The detected format.
    pub format: CalFormat,
    /// The points, sorted by frequency, de-duplicated, inside
    /// [`CAL_MIN_HZ`]..=[`CAL_MAX_HZ`]; each has a phase exactly when
    /// [`ParsedCurve::has_phase`] is true.
    pub points: Vec<CurvePoint>,
    /// Every data line carried a phase column (otherwise phases are dropped).
    pub has_phase: bool,
    /// Data points outside the frequency range that were ignored.
    pub ignored_out_of_range: usize,
    /// Data points dropped because their frequency repeated an earlier one.
    pub duplicates_dropped: usize,
    /// A UMIK `Sens Factor` (dB) or a REW `Sensitivity` (dBFS at 94 dB SPL).
    pub sensitivity_db: Option<f64>,
    /// A Dayton-style `*1000Hz` line: the sensitivity at 1 kHz, dBV/Pa
    /// (WP P5DAYTON, additive), within
    /// [`DAYTON_MIN_DBV_PA`]..=[`DAYTON_MAX_DBV_PA`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sensitivity_dbv_pa: Option<f64>,
    /// A UMIK-2 `AGain` (dB): the microphone's gain the sensitivity is for.
    pub gain_db: Option<f64>,
    /// The microphone's serial number (`SERNO`), ASCII letters, digits and `-`.
    pub serial: Option<String>,
    /// The file says it is for 90° incidence (its name or a comment).
    pub angle: MicAngle,
}

impl ParsedCurve {
    /// The sensitivity the dB SPL scale uses, as dBFS at 94 dB SPL: the
    /// `Sens Factor` or `Sensitivity` line when there is one, else a Dayton
    /// dBV/Pa sensitivity referred to [`ASSUMED_FULL_SCALE_DBV`]
    /// (`dBV/Pa - full scale dBV`, a 94 dB SPL tone being 1 Pa).
    pub fn spl_sensitivity_db(&self) -> Option<f64> {
        self.sensitivity_db.or_else(|| {
            self.sensitivity_dbv_pa
                .map(|s| s - ASSUMED_FULL_SCALE_DBV)
                .filter(|s| s.is_finite())
        })
    }

    /// The sensitivity in the file's own convention, as a `mic` document's
    /// `sensitivityDb` keeps it: dBFS at 94 dB SPL for `umik` and `rew`, dBV/Pa
    /// for `dayton`.
    pub fn stated_sensitivity_db(&self) -> Option<f64> {
        match self.format {
            CalFormat::Dayton => self.sensitivity_dbv_pa,
            CalFormat::Umik | CalFormat::Rew | CalFormat::Csv => self.sensitivity_db,
        }
    }
}

/// A microphone calibration: a parsed file and who made it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MicCalibration {
    /// The parsed file.
    pub curve: ParsedCurve,
    /// The vendor when the format names one (`miniDSP` for a UMIK file).
    pub vendor: Option<String>,
}

impl MicCalibration {
    /// The calibration on `grid` (see [`curve_to_grid`]).
    pub fn to_response(&self, grid: &LogGrid) -> Result<Response, MeasureError> {
        curve_to_grid(&self.curve.points, grid)
    }

    /// The dB SPL scale this calibration gives without a calibrator step
    /// (see [`spl_scale`]).
    pub fn spl_scale(&self) -> SplScale {
        spl_scale(self.curve.spl_sensitivity_db(), None)
    }
}

/// Parses a microphone calibration file given as bytes: refuses a file over
/// [`MAX_FILE_BYTES`], a binary file (NUL or other control bytes) and text that
/// is not UTF-8, then [`parse_calibration`].
///
/// `source_name` is the file's name (it may say `_90deg`); it is not stored.
pub fn parse_calibration_bytes(
    bytes: &[u8],
    source_name: &str,
) -> Result<MicCalibration, MeasureError> {
    let text = text_from_bytes(bytes)?;
    parse_calibration(text, source_name)
}

/// Parses a microphone calibration file's text (any of the [`CalFormat`]s).
///
/// Errors: [`MeasureError::Parse`] for a file over the bounds, a binary file, a
/// line that is not numbers where data is expected, a non-finite value, or no
/// point in range.
pub fn parse_calibration(text: &str, source_name: &str) -> Result<MicCalibration, MeasureError> {
    let curve = parse_curve(text, source_name)?;
    let vendor = match curve.format {
        _ if curve.sensitivity_dbv_pa.is_some() => Some("Dayton Audio".to_string()),
        CalFormat::Umik => Some("miniDSP".to_string()),
        CalFormat::Rew | CalFormat::Csv | CalFormat::Dayton => None,
    };
    Ok(MicCalibration { curve, vendor })
}

/// Checks bytes for the bounds, binary content and UTF-8; strips a UTF-8 BOM.
///
/// Errors: [`MeasureError::Parse`].
pub fn text_from_bytes(bytes: &[u8]) -> Result<&str, MeasureError> {
    if bytes.len() > MAX_FILE_BYTES {
        return Err(MeasureError::Parse(format!(
            "file of {} bytes (at most {MAX_FILE_BYTES})",
            bytes.len()
        )));
    }
    if bytes.iter().any(|&b| is_binary_byte(b)) {
        return Err(MeasureError::Parse("binary file, not text".to_string()));
    }
    let text = std::str::from_utf8(bytes).map_err(|e| {
        MeasureError::Parse(format!("not UTF-8 text (at byte {})", e.valid_up_to()))
    })?;
    Ok(text.strip_prefix('\u{feff}').unwrap_or(text))
}

fn is_binary_byte(b: u8) -> bool {
    b < 0x20 && !matches!(b, b'\t' | b'\n' | b'\r' | 0x0c)
}

fn is_binary_char(c: char) -> bool {
    u8::try_from(c).is_ok_and(is_binary_byte) || c == '\u{7f}' || c == '\u{fffd}'
}

/// Parses any curve file (calibration, target or compensation): the shared
/// line parser of this module and [`crate::target`].
///
/// Errors as [`parse_calibration`].
pub fn parse_curve(text: &str, source_name: &str) -> Result<ParsedCurve, MeasureError> {
    if text.len() > MAX_FILE_BYTES {
        return Err(MeasureError::Parse(format!(
            "file of {} bytes (at most {MAX_FILE_BYTES})",
            text.len()
        )));
    }
    if text.chars().any(is_binary_char) {
        return Err(MeasureError::Parse("binary file, not text".to_string()));
    }
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);

    let mut points: Vec<CurvePoint> = Vec::new();
    let mut all_phase = true;
    let mut ignored = 0usize;
    let mut sensitivity_db = None;
    let mut sensitivity_dbv_pa = None;
    let mut gain_db = None;
    let mut serial = None;
    let mut umik = false;
    let mut csv = false;
    let mut header_seen = false;
    let mut angle = if mentions_90(source_name) {
        MicAngle::Deg90
    } else {
        MicAngle::Deg0
    };

    for (i, raw) in text.lines().enumerate() {
        let line_no = i.saturating_add(1);
        if line_no > MAX_FILE_LINES {
            return Err(MeasureError::Parse(format!(
                "more than {MAX_FILE_LINES} lines"
            )));
        }
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        let lower = line.to_lowercase();
        if lower.contains("sens factor") {
            umik = true;
            sensitivity_db = Some(finite_after(&lower, "sens factor", line_no)?.ok_or_else(
                || MeasureError::Parse(format!("line {line_no}: Sens Factor without a number")),
            )?);
            gain_db = finite_after(&lower, "again", line_no)?;
            serial = serial_after(line);
            if mentions_90(&lower) {
                angle = MicAngle::Deg90;
            }
            continue;
        }
        if lower.trim_start_matches('"').starts_with("sensitivity") {
            sensitivity_db = Some(finite_after(&lower, "sensitivity", line_no)?.ok_or_else(
                || MeasureError::Parse(format!("line {line_no}: Sensitivity without a number")),
            )?);
            continue;
        }
        // Review P5-E: only in the file's head, where a Dayton file has it.
        // After the first data row a `*` line is the comment it always was
        // (a REW note such as `*1000Hz 0 dB` is then never a sensitivity, nor
        // a reason to refuse the file).
        if points.is_empty()
            && let Some(s) = dayton_sensitivity(line, line_no)?
        {
            if sensitivity_dbv_pa.is_some() {
                return Err(MeasureError::Parse(format!(
                    "line {line_no}: a second 1 kHz sensitivity"
                )));
            }
            sensitivity_dbv_pa = Some(s);
            continue;
        }
        if line.starts_with(['*', '#', '"', '\'']) {
            if mentions_90(&lower) {
                angle = MicAngle::Deg90;
            }
            continue;
        }
        let (sep, decimal_comma) = if line.contains(';') {
            (Some(';'), true)
        } else if line.contains(',') {
            (Some(','), false)
        } else {
            (None, false)
        };
        let fields: Vec<&str> = match sep {
            Some(s) => line
                .split(s)
                .map(str::trim)
                .filter(|f| !f.is_empty())
                .collect(),
            None => line.split_whitespace().collect(),
        };
        let parsed: Vec<Option<f64>> = fields
            .iter()
            .take(3)
            .map(|f| parse_number(f, decimal_comma))
            .collect();
        let first_is_number = matches!(parsed.first(), Some(Some(_)));
        if !first_is_number {
            // One header line before any data (a CSV or a tab file's titles).
            if points.is_empty() && ignored == 0 && !header_seen {
                header_seen = true;
                csv = true;
                if mentions_90(&lower) {
                    angle = MicAngle::Deg90;
                }
                continue;
            }
            return Err(MeasureError::Parse(format!("line {line_no}: not a number")));
        }
        if sep.is_some() {
            csv = true;
        }
        let freq = parsed.first().copied().flatten();
        let db = parsed.get(1).copied().flatten();
        let (Some(freq), Some(db)) = (freq, db) else {
            return Err(MeasureError::Parse(format!(
                "line {line_no}: expected a frequency and a level"
            )));
        };
        let phase = match parsed.get(2) {
            Some(Some(p)) => Some(*p),
            Some(None) => {
                return Err(MeasureError::Parse(format!("line {line_no}: not a number")));
            }
            None => None,
        };
        if !freq.is_finite() || !db.is_finite() || phase.is_some_and(|p| !p.is_finite()) {
            return Err(MeasureError::Parse(format!(
                "line {line_no}: non-finite value"
            )));
        }
        if !(CAL_MIN_HZ..=CAL_MAX_HZ).contains(&freq) {
            ignored = ignored.saturating_add(1);
            continue;
        }
        if phase.is_none() {
            all_phase = false;
        }
        points.push(CurvePoint {
            freq_hz: freq,
            db,
            phase_deg: phase,
        });
    }

    if points.is_empty() {
        return Err(MeasureError::Parse(if ignored > 0 {
            format!("no point between {CAL_MIN_HZ} Hz and {CAL_MAX_HZ} Hz")
        } else {
            "no data points".to_string()
        }));
    }
    // Stable sort: equal frequencies keep file order, so the first one is kept.
    points.sort_by(|a, b| a.freq_hz.total_cmp(&b.freq_hz));
    let before = points.len();
    points.dedup_by(|later, first| later.freq_hz == first.freq_hz);
    let duplicates_dropped = before.saturating_sub(points.len());
    if !all_phase {
        for p in &mut points {
            p.phase_deg = None;
        }
    }
    let format = if umik {
        CalFormat::Umik
    } else if sensitivity_dbv_pa.is_some() {
        CalFormat::Dayton
    } else if csv {
        CalFormat::Csv
    } else {
        CalFormat::Rew
    };
    Ok(ParsedCurve {
        format,
        points,
        has_phase: all_phase,
        ignored_out_of_range: ignored,
        duplicates_dropped,
        sensitivity_db,
        sensitivity_dbv_pa,
        gain_db,
        serial,
        angle,
    })
}

/// Puts curve points on `grid`: linear in dB (and degrees) over log
/// frequency between points; outside the points' range the end value is held
/// and the point is marked not valid. `points` must be strictly ascending in
/// frequency (as [`parse_curve`] gives them). Phase is given when every point
/// has one.
///
/// Errors: [`MeasureError::Invalid`] for a bad grid, no points, or a
/// non-finite or unsorted point.
pub fn curve_to_grid(points: &[CurvePoint], grid: &LogGrid) -> Result<Response, MeasureError> {
    let freqs = grid.freqs()?;
    if points.is_empty() {
        return Err(MeasureError::Invalid("no curve points".to_string()));
    }
    let finite = points.iter().all(|p| {
        p.freq_hz.is_finite()
            && p.freq_hz > 0.0
            && p.db.is_finite()
            && p.phase_deg.is_none_or(f64::is_finite)
    });
    let sorted = points.windows(2).all(|w| match w {
        [a, b] => a.freq_hz < b.freq_hz,
        _ => true,
    });
    if !finite || !sorted {
        return Err(MeasureError::Invalid(
            "curve points must be finite and strictly ascending".to_string(),
        ));
    }
    let with_phase = points.iter().all(|p| p.phase_deg.is_some());
    let mut mag = Vec::with_capacity(freqs.len());
    let mut phase = Vec::with_capacity(freqs.len());
    let mut valid = Vec::with_capacity(freqs.len());
    for &f in &freqs {
        let (m, p, v) = interpolate(points, f);
        mag.push(m);
        phase.push(p);
        valid.push(v);
    }
    if mag.iter().chain(&phase).any(|v| !v.is_finite()) {
        return Err(MeasureError::Invalid(
            "curve values out of range".to_string(),
        ));
    }
    Ok(Response {
        grid: *grid,
        mag_db: mag,
        phase_deg: with_phase.then_some(phase),
        valid: Some(valid),
        spl: false,
    })
}

/// The value of sorted, non-empty `points` at `f`: (dB, phase or 0, inside).
fn interpolate(points: &[CurvePoint], f: f64) -> (f64, f64, bool) {
    let at = |p: &CurvePoint| (p.db, p.phase_deg.unwrap_or(0.0));
    let (Some(first), Some(last)) = (points.first(), points.last()) else {
        return (0.0, 0.0, false);
    };
    if f < first.freq_hz {
        let (m, p) = at(first);
        return (m, p, false);
    }
    if f > last.freq_hz {
        let (m, p) = at(last);
        return (m, p, false);
    }
    let idx = points.partition_point(|p| p.freq_hz < f);
    let hi = points.get(idx);
    let lo = idx.checked_sub(1).and_then(|i| points.get(i));
    match (lo, hi) {
        (Some(a), Some(b)) => {
            let span = (b.freq_hz / a.freq_hz).log2();
            let t = if span > 0.0 {
                ((f / a.freq_hz).log2() / span).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let (ma, pa) = at(a);
            let (mb, pb) = at(b);
            (ma + (mb - ma) * t, pa + (pb - pa) * t, true)
        }
        (None, Some(p)) | (Some(p), None) => {
            let (m, ph) = at(p);
            (m, ph, true)
        }
        (None, None) => (0.0, 0.0, false),
    }
}

/// Removes a microphone's (or a coupler's) own response from a measurement:
/// `measured - correction` in dB, and in phase when both have phase (a
/// magnitude-only correction keeps the measured phase). The result is valid
/// where both are; `spl` is the measurement's.
///
/// Errors: [`MeasureError::Invalid`] when the grids or lengths differ, or a
/// value is not finite.
pub fn apply_correction(
    measured: &Response,
    correction: &Response,
) -> Result<Response, MeasureError> {
    if measured.grid != correction.grid || measured.mag_db.len() != correction.mag_db.len() {
        return Err(MeasureError::Invalid(
            "measurement and correction are on different grids".to_string(),
        ));
    }
    let n = measured.mag_db.len();
    let phases_ok = [&measured.phase_deg, &correction.phase_deg]
        .iter()
        .all(|p| p.as_ref().is_none_or(|v| v.len() == n));
    let valids_ok = [&measured.valid, &correction.valid]
        .iter()
        .all(|v| v.as_ref().is_none_or(|v| v.len() == n));
    if !phases_ok || !valids_ok {
        return Err(MeasureError::Invalid(
            "response arrays of unequal length".to_string(),
        ));
    }
    let all_finite = |v: &[f64]| v.iter().all(|x| x.is_finite());
    if !all_finite(&measured.mag_db)
        || !all_finite(&correction.mag_db)
        || !measured.phase_deg.as_deref().is_none_or(all_finite)
        || !correction.phase_deg.as_deref().is_none_or(all_finite)
    {
        return Err(MeasureError::Invalid(
            "non-finite response value".to_string(),
        ));
    }
    let mag: Vec<f64> = measured
        .mag_db
        .iter()
        .zip(&correction.mag_db)
        .map(|(m, c)| m - c)
        .collect();
    let phase: Option<Vec<f64>> = match (&measured.phase_deg, &correction.phase_deg) {
        (Some(m), Some(c)) => Some(m.iter().zip(c).map(|(m, c)| m - c).collect()),
        (Some(m), None) => Some(m.clone()),
        (None, _) => None,
    };
    if !all_finite(&mag) || !phase.as_deref().is_none_or(all_finite) {
        return Err(MeasureError::Invalid(
            "corrected values out of range".to_string(),
        ));
    }
    let valid = match (&measured.valid, &correction.valid) {
        (Some(a), Some(b)) => Some(a.iter().zip(b).map(|(a, b)| *a && *b).collect()),
        (Some(a), None) => Some(a.clone()),
        (None, Some(b)) => Some(b.clone()),
        (None, None) => None,
    };
    Ok(Response {
        grid: measured.grid,
        mag_db: mag,
        phase_deg: phase,
        valid,
        spl: measured.spl,
    })
}

/// Where a dB SPL scale comes from.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "basis")]
pub enum SplBasis {
    /// A calibrator step (94 or 114 dB SPL at 1 kHz) on this input path.
    Calibrator {
        /// The calibrator's level, dB SPL.
        reference_db: f64,
    },
    /// The calibration file's sensitivity: approximate, since the host's
    /// input gain (an OS slider, a browser's processing) is not known.
    Sensitivity,
    /// Neither: levels are relative dB.
    Relative,
}

/// How captured levels become dB SPL: `dB SPL = dBFS + offset_db`, with dBFS
/// as in [`level_dbfs`] (a full-scale sine reads 0 dBFS).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SplScale {
    /// The offset, dB; `None` when levels are relative.
    pub offset_db: Option<f64>,
    /// True unless a calibrator step set the offset (labelled "approximate").
    pub approximate: bool,
    /// Where the offset comes from.
    pub basis: SplBasis,
}

/// A calibrator step's result: the offset [`spl_offset_from_tone`] gave and the
/// calibrator's level.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CalibratorStep {
    /// `dB SPL = dBFS + offset_db`.
    pub offset_db: f64,
    /// 94 or 114 dB SPL.
    pub reference_db: f64,
}

/// The dB SPL offset a sensitivity implies: `94 - sensitivity_db`.
///
/// The convention is REW's, which is how miniDSP's own instructions use these
/// files. Read 2026-11-16 in REW's help, "Cal files Preferences"
/// (<https://www.roomeqwizard.com/help/help/html/calfiles.html>): USB
/// microphone files carry a line `Sens Factor =1.2345dB, SERNO: XXXXXXX`, and
/// the sens factor is usually the dBFS reading the microphone gives in a 94 dB
/// calibrator at maximum input volume; REW's own `Sensitivity -12.34 dBFS`
/// line is likewise the dBFS input level at 94 dB SPL, valid only at the same
/// interface gain and input volume (REW help, "Input calibration"). A UMIK-2's
/// `AGain` names the gain the factor is for; it is stored, not added. So a
/// level `L` dBFS reads `L - sensitivity + 94` dB SPL. The host's input gain is
/// not known (a browser cannot read it) and dBFS conventions differ by 3 dB
/// between programs, so this scale is always **approximate** until a
/// calibrator step replaces it.
pub fn sensitivity_offset_db(sensitivity_db: f64) -> f64 {
    CALIBRATOR_94_DB - sensitivity_db
}

/// The SPL scale: a calibrator step's offset when there is one (exact), else
/// the sensitivity's (approximate), else relative dB. A non-finite
/// sensitivity, or a calibrator step with a non-finite offset or a level other
/// than 94 or 114 dB, counts as absent.
pub fn spl_scale(sensitivity_db: Option<f64>, calibrator: Option<CalibratorStep>) -> SplScale {
    if let Some(step) =
        calibrator.filter(|s| s.offset_db.is_finite() && is_calibrator_level(s.reference_db))
    {
        return SplScale {
            offset_db: Some(step.offset_db),
            approximate: false,
            basis: SplBasis::Calibrator {
                reference_db: step.reference_db,
            },
        };
    }
    match sensitivity_db.filter(|s| s.is_finite()) {
        Some(s) => SplScale {
            offset_db: Some(sensitivity_offset_db(s)),
            approximate: true,
            basis: SplBasis::Sensitivity,
        },
        None => SplScale {
            offset_db: None,
            approximate: true,
            basis: SplBasis::Relative,
        },
    }
}

impl SplScale {
    /// `level_dbfs` in dB SPL, or `None` when the scale is relative or the
    /// result is not finite.
    pub fn to_spl(&self, level_dbfs: f64) -> Option<f64> {
        let v = level_dbfs + self.offset_db?;
        v.is_finite().then_some(v)
    }
}

fn is_calibrator_level(db: f64) -> bool {
    db == CALIBRATOR_94_DB || db == CALIBRATOR_114_DB
}

/// The level of a signal in dBFS, RMS-based with a full-scale sine at 0 dBFS
/// (AES17): `20 log10(rms * sqrt 2)`. `None` for silence, no samples or a
/// non-finite sample.
pub fn level_dbfs(samples: &[f32]) -> Option<f64> {
    if samples.is_empty() {
        return None;
    }
    let mut sum = 0.0f64;
    for &s in samples {
        let s = f64::from(s);
        if !s.is_finite() {
            return None;
        }
        sum += s * s;
    }
    let rms = (sum / samples.len() as f64).sqrt();
    let db = 20.0 * (rms * std::f64::consts::SQRT_2).log10();
    (rms > 0.0 && db.is_finite()).then_some(db)
}

/// The dB SPL offset from a calibrator capture: `reference_db` (94 or 114 dB
/// SPL) minus the tone's level in dBFS ([`level_dbfs`]'s convention), so that
/// `dB SPL = dBFS + offset`.
///
/// The last [`MAX_TONE_SAMPLES`] samples (at least [`MIN_TONE_SAMPLES`]) are
/// analysed: DC removed, then a Hann-windowed spectrum must have at least 90 %
/// of its power within 900 Hz to 1100 Hz (a calibrator is 1 kHz within a few
/// percent); the tone's level is the total RMS times the square root of that
/// share.
///
/// Errors: [`MeasureError::Invalid`] for a reference other than 94 or 114, a
/// rate outside 8 kHz to 384 kHz, or a non-finite sample;
/// [`MeasureError::NoSignal`] for a capture too short, silent, clipped (a
/// sample at full scale) or not a 1 kHz tone.
pub fn spl_offset_from_tone(
    capture: &[f32],
    rate_hz: u32,
    reference_db: f64,
) -> Result<f64, MeasureError> {
    if !is_calibrator_level(reference_db) {
        return Err(MeasureError::Invalid(
            "a calibrator is 94 or 114 dB SPL".to_string(),
        ));
    }
    if !(8_000..=384_000).contains(&rate_hz) {
        return Err(MeasureError::Invalid(format!("capture rate {rate_hz} Hz")));
    }
    if capture.iter().any(|s| !s.is_finite()) {
        return Err(MeasureError::Invalid("non-finite sample".to_string()));
    }
    if capture.len() < MIN_TONE_SAMPLES {
        return Err(MeasureError::NoSignal(format!(
            "calibrator capture of {} samples (at least {MIN_TONE_SAMPLES})",
            capture.len()
        )));
    }
    let start = capture.len().saturating_sub(MAX_TONE_SAMPLES);
    let tail = capture.get(start..).unwrap_or(&[]);
    if tail.iter().any(|s| s.abs() >= 1.0) {
        return Err(MeasureError::NoSignal(
            "the calibrator capture clipped".to_string(),
        ));
    }
    let n = tail.len();
    let n_f = n as f64;
    let mean = tail.iter().map(|&s| f64::from(s)).sum::<f64>() / n_f;
    let x: Vec<f64> = tail.iter().map(|&s| f64::from(s) - mean).collect();
    let total = x.iter().map(|v| v * v).sum::<f64>() / n_f;
    if total <= 1e-14 {
        return Err(MeasureError::NoSignal(
            "the calibrator capture is silent".to_string(),
        ));
    }

    // Hann-windowed spectrum; the in-band share of the windowed power.
    let mut buf: Vec<rustfft::num_complex::Complex<f64>> = x
        .iter()
        .enumerate()
        .map(|(i, v)| {
            let w = 0.5 - 0.5 * (2.0 * std::f64::consts::PI * i as f64 / n_f).cos();
            rustfft::num_complex::Complex::new(v * w, 0.0)
        })
        .collect();
    // Scalar only (review RC10).
    let mut planner = rustfft::FftPlannerScalar::<f64>::new();
    planner.plan_fft_forward(n).process(&mut buf);
    let half = n / 2;
    let bin_hz = f64::from(rate_hz) / n_f;
    let mut in_band = 0.0;
    let mut all = 0.0;
    for (k, c) in buf.iter().take(half).enumerate().skip(1) {
        let p = c.norm_sqr();
        all += p;
        if (900.0..=1100.0).contains(&(k as f64 * bin_hz)) {
            in_band += p;
        }
    }
    let share = in_band / all;
    if !share.is_finite() || share < 0.9 {
        return Err(MeasureError::NoSignal(
            "the capture is not a 1 kHz calibrator tone".to_string(),
        ));
    }
    let tone_rms = (total * share).sqrt();
    let off = reference_db - 20.0 * (tone_rms * std::f64::consts::SQRT_2).log10();
    if off.is_finite() {
        Ok(off)
    } else {
        Err(MeasureError::NoSignal(
            "the calibrator capture is silent".to_string(),
        ))
    }
}

/// Whether text says "90 degrees": `90deg`, `90 deg`, `90_deg`, `90°`,
/// `90degree` (and not `190deg`).
fn mentions_90(s: &str) -> bool {
    let chars: Vec<char> = s.to_lowercase().chars().collect();
    let sep = |c: &char| c.is_whitespace() || *c == '_' || *c == '-';
    chars.windows(2).enumerate().any(|(i, w)| {
        if w != ['9', '0'] {
            return false;
        }
        let prev_digit = i
            .checked_sub(1)
            .and_then(|p| chars.get(p))
            .is_some_and(char::is_ascii_digit);
        let rest: String = chars
            .iter()
            .skip(i.saturating_add(2))
            .skip_while(|c| sep(c))
            .take(3)
            .collect();
        !prev_digit && (rest == "deg" || rest.starts_with('°'))
    })
}

/// A number written after `key` in `lower` (skipping spaces, `=` and `:`);
/// `Ok(None)` when the key or the number is absent; an error when the number
/// is malformed or not finite.
fn finite_after(lower: &str, key: &str, line_no: usize) -> Result<Option<f64>, MeasureError> {
    let Some(pos) = lower.find(key) else {
        return Ok(None);
    };
    let rest = lower.get(pos.saturating_add(key.len())..).unwrap_or("");
    let rest = rest.trim_start_matches(|c: char| c.is_whitespace() || c == '=' || c == ':');
    let end = rest
        .find(|c: char| !(c.is_ascii_digit() || matches!(c, '+' | '-' | '.' | 'e')))
        .unwrap_or(rest.len());
    let mut num = rest.get(..end).unwrap_or("");
    // "…dB": a trailing 'e' cannot start an exponent without digits.
    while let Some(stripped) = num.strip_suffix('e') {
        num = stripped;
    }
    if num.is_empty() {
        return Ok(None);
    }
    match num.parse::<f64>() {
        Ok(v) if v.is_finite() => Ok(Some(v)),
        Ok(_) => Err(MeasureError::Parse(format!(
            "line {line_no}: non-finite value"
        ))),
        Err(_) => Err(MeasureError::Parse(format!("line {line_no}: not a number"))),
    }
}

/// A Dayton-style sensitivity line, `*1000Hz<TAB>-36.1` (also `* 1000 Hz`,
/// `*1kHz`, spaces or tabs, an optional `dBV/Pa` or `dB` after the number):
/// `Ok(None)` when the line is not one (or names no number: then it stays a
/// comment), the value in dBV/Pa when it is, an error for a value that is not
/// finite or outside [`DAYTON_MIN_DBV_PA`]..=[`DAYTON_MAX_DBV_PA`].
fn dayton_sensitivity(line: &str, line_no: usize) -> Result<Option<f64>, MeasureError> {
    let Some(rest) = line.strip_prefix('*') else {
        return Ok(None);
    };
    let lower = rest.trim_start().to_ascii_lowercase();
    let Some(after) = ["1000hz", "1000 hz", "1khz", "1 khz"]
        .iter()
        .find_map(|k| lower.strip_prefix(k))
    else {
        return Ok(None);
    };
    // The frequency must end there (`*1000Hz`, not `*1000Hzx`).
    if !after.is_empty() && !after.starts_with(|c: char| c.is_whitespace() || c == ':') {
        return Ok(None);
    }
    let after = after.trim_start_matches(|c: char| c.is_whitespace() || c == ':');
    let token = after.split_whitespace().next().unwrap_or("");
    let num = ["dbv/pa", "dbv", "db"]
        .iter()
        .find_map(|u| token.strip_suffix(u))
        .unwrap_or(token);
    let Some(v) = parse_number(num, false) else {
        return Ok(None);
    };
    if !v.is_finite() {
        return Err(MeasureError::Parse(format!(
            "line {line_no}: non-finite sensitivity"
        )));
    }
    if !(DAYTON_MIN_DBV_PA..=DAYTON_MAX_DBV_PA).contains(&v) {
        return Err(MeasureError::Parse(format!(
            "line {line_no}: a sensitivity of {v} dBV/Pa \
             ({DAYTON_MIN_DBV_PA} to {DAYTON_MAX_DBV_PA})"
        )));
    }
    Ok(Some(v))
}

/// The `SERNO` of a UMIK header: ASCII letters, digits and `-`, at most 32.
fn serial_after(line: &str) -> Option<String> {
    let lower = line.to_ascii_lowercase();
    let pos = lower.find("serno")?;
    let rest = line.get(pos.saturating_add(5)..)?;
    let rest = rest.trim_start_matches(|c: char| c.is_whitespace() || c == '=' || c == ':');
    let s: String = rest
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '-')
        .take(32)
        .collect();
    (!s.is_empty()).then_some(s)
}

/// A number field; `decimal_comma` accepts `1,5` for 1.5. Accepts `inf` and
/// `NaN` as numbers so that the caller refuses them as non-finite.
fn parse_number(field: &str, decimal_comma: bool) -> Option<f64> {
    let f = field.trim().trim_matches('"');
    if f.is_empty() || f.len() > 64 {
        return None;
    }
    if decimal_comma && f.contains(',') {
        f.replace(',', ".").parse().ok()
    } else {
        f.parse().ok()
    }
}
