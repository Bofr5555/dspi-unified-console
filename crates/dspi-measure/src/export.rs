//! REW-compatible text export of a response (WP P5RESP; PLAN-P5 T1).
//!
//! The format REW writes with "Export measurement as text" and reads with
//! "Import frequency response": comment lines starting with `*`, then one
//! point a line, `frequency magnitude [phase]`, separated by a space, the
//! frequency in Hz, the magnitude in dB (dB SPL when calibrated), the phase
//! in degrees wrapped to (-180, 180]. A response without phase gives two
//! columns. The text is what the user saves; it is not a document
//! (PLAN-P5 section 1.6) and holds derived curves only (S12).
//!
//! Deterministic: no date or host data is added; the caller puts what it
//! wants in the title and notes.

use crate::response::{check, smooth};
use crate::types::{MeasureError, Response};
use std::fmt::Write as _;

/// The longest title, source or note line kept, characters (longer ones are
/// cut).
pub const MAX_HEADER_CHARS: usize = 256;
/// The most note lines.
pub const MAX_NOTES: usize = 16;

/// What goes into the export besides the curve.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RewExport {
    /// The measurement's name, for the header.
    pub title: String,
    /// What was measured, e.g. "Output 2 (L high)"; empty for none.
    pub source: String,
    /// Smoothing to apply before export, as an octave fraction (1 to 48);
    /// `None` exports the curve as it is.
    pub smoothing: Option<u8>,
    /// Further header lines (at most [`MAX_NOTES`]).
    pub notes: Vec<String>,
    /// Leave out the points the response marks untrusted.
    pub trusted_only: bool,
}

/// One header line's text: control characters as spaces, at most
/// [`MAX_HEADER_CHARS`] characters, trimmed.
fn clean(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .take(MAX_HEADER_CHARS)
        .collect::<String>()
        .trim()
        .to_string()
}

fn wrap_phase(deg: f64) -> f64 {
    // Into (-180, 180].
    let w = (deg + 180.0).rem_euclid(360.0) - 180.0;
    if w <= -180.0 { 180.0 } else { w }
}

/// The response as REW text (see the module documentation).
///
/// Errors: [`MeasureError::Invalid`] for a malformed response, a smoothing
/// outside 1..=48 or more than [`MAX_NOTES`] notes;
/// [`MeasureError::NoSignal`] when no point is left to export.
pub fn rew_text(resp: &Response, opts: &RewExport) -> Result<String, MeasureError> {
    if opts.notes.len() > MAX_NOTES {
        return Err(MeasureError::Invalid(format!(
            "{} notes (at most {MAX_NOTES})",
            opts.notes.len()
        )));
    }
    let curve = match opts.smoothing {
        Some(f) => smooth(resp, f)?,
        None => resp.clone(),
    };
    let freqs = check(&curve)?;
    let mut out = String::new();
    // Writing to a String cannot fail; the results are ignored.
    let _ = writeln!(out, "* Measurement data exported by the DSPi console");
    let title = clean(&opts.title);
    if !title.is_empty() {
        let _ = writeln!(out, "* Title: {title}");
    }
    let source = clean(&opts.source);
    if !source.is_empty() {
        let _ = writeln!(out, "* Source: {source}");
    }
    let _ = match opts.smoothing {
        Some(f) => writeln!(out, "* Smoothing: 1/{f} octave"),
        None => writeln!(out, "* Smoothing: none"),
    };
    let _ = writeln!(
        out,
        "* Magnitude: {}",
        if curve.spl { "dB SPL" } else { "dB (relative)" }
    );
    for n in &opts.notes {
        let n = clean(n);
        if !n.is_empty() {
            let _ = writeln!(out, "* Note: {n}");
        }
    }
    let phase = curve.phase_deg.as_ref();
    if phase.is_some() {
        let _ = writeln!(out, "* Freq(Hz) SPL(dB) Phase(degrees)");
    } else {
        let _ = writeln!(out, "* Freq(Hz) SPL(dB)");
    }
    let mut points = 0usize;
    for (i, f) in freqs.iter().enumerate() {
        let trusted = curve
            .valid
            .as_ref()
            .is_none_or(|v| v.get(i).copied().unwrap_or(false));
        if opts.trusted_only && !trusted {
            continue;
        }
        let Some(m) = curve.mag_db.get(i) else {
            continue;
        };
        match phase.and_then(|p| p.get(i)) {
            Some(p) => {
                let _ = writeln!(out, "{f:.6} {m:.3} {:.3}", wrap_phase(*p));
            }
            None => {
                let _ = writeln!(out, "{f:.6} {m:.3}");
            }
        }
        points = points.saturating_add(1);
    }
    if points == 0 {
        return Err(MeasureError::NoSignal("no point to export".into()));
    }
    Ok(out)
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_p5resp {
    use super::*;
    use crate::types::LogGrid;
    use proptest::prelude::*;

    /// A reader written to REW's import rules: lines starting with `*` are
    /// comments, a data line is two or three numbers separated by spaces,
    /// tabs, commas or semicolons, frequencies ascending.
    fn parse_rew(text: &str) -> Vec<(f64, f64, Option<f64>)> {
        let mut out = Vec::new();
        for line in text.lines() {
            let t = line.trim();
            if t.is_empty() || t.starts_with('*') {
                continue;
            }
            let cols: Vec<f64> = t
                .split(|c: char| c.is_whitespace() || c == ',' || c == ';')
                .filter(|s| !s.is_empty())
                .map(|s| s.parse::<f64>().expect("a number"))
                .collect();
            assert!(cols.len() == 2 || cols.len() == 3, "{line}");
            out.push((cols[0], cols[1], cols.get(2).copied()));
        }
        assert!(out.windows(2).all(|w| w[1].0 > w[0].0));
        out
    }

    fn sample() -> Response {
        let f = LogGrid::STANDARD.freqs().unwrap();
        Response {
            grid: LogGrid::STANDARD,
            mag_db: f
                .iter()
                .map(|x| 80.0 + 3.0 * (x / 100.0).ln().sin())
                .collect(),
            phase_deg: Some(f.iter().map(|x| -0.36 * x).collect()),
            valid: Some(f.iter().map(|x| *x >= 20.0).collect()),
            spl: true,
        }
    }

    #[test]
    fn wp_p5resp_export_parses_back_in_rew_columns() {
        let r = sample();
        let text = rew_text(
            &r,
            &RewExport {
                title: "Left\nlow".into(),
                source: "Output 0 (L low)".into(),
                notes: vec!["position 1".into()],
                ..Default::default()
            },
        )
        .unwrap();
        assert!(text.contains("* Title: Left low\n"));
        assert!(text.contains("* Magnitude: dB SPL\n"));
        assert!(text.contains("* Freq(Hz) SPL(dB) Phase(degrees)\n"));
        let pts = parse_rew(&text);
        let f = LogGrid::STANDARD.freqs().unwrap();
        assert_eq!(pts.len(), f.len());
        let ph = r.phase_deg.as_ref().unwrap();
        for (i, (pf, pm, pp)) in pts.iter().enumerate() {
            assert!((pf - f[i]).abs() <= 1e-6 * f[i].max(1.0));
            assert!((pm - r.mag_db[i]).abs() <= 0.0005 + 1e-9);
            let p = pp.unwrap();
            assert!(p > -180.0 && p <= 180.0);
            let d = (p - ph[i]).rem_euclid(360.0);
            assert!(d < 0.001 || d > 360.0 - 0.001, "{i}: {p} vs {}", ph[i]);
        }
    }

    #[test]
    fn wp_p5resp_export_trusted_only_smoothing_and_two_columns() {
        let mut r = sample();
        r.phase_deg = None;
        r.spl = false;
        let text = rew_text(
            &r,
            &RewExport {
                trusted_only: true,
                smoothing: Some(6),
                ..Default::default()
            },
        )
        .unwrap();
        assert!(text.contains("* Smoothing: 1/6 octave\n"));
        assert!(text.contains("* Magnitude: dB (relative)\n"));
        let pts = parse_rew(&text);
        assert!(pts.iter().all(|p| p.0 >= 20.0 && p.2.is_none()));
        assert_eq!(
            pts.len(),
            r.valid.as_ref().unwrap().iter().filter(|v| **v).count()
        );
        // Nothing trusted: nothing to export.
        r.valid = Some(vec![false; r.mag_db.len()]);
        let e = rew_text(
            &r,
            &RewExport {
                trusted_only: true,
                ..Default::default()
            },
        );
        assert!(matches!(e, Err(MeasureError::NoSignal(_))));
        let many = RewExport {
            notes: vec![String::new(); MAX_NOTES + 1],
            ..Default::default()
        };
        assert!(rew_text(&sample(), &many).is_err());
    }

    proptest! {
        #[test]
        fn proptest_wp_p5resp_export_never_panics(
            mag in proptest::collection::vec(proptest::num::f64::ANY, 0..40),
            ppo in 0u16..8,
            smoothing in proptest::option::of(0u8..60),
            title in ".{0,300}",
            trusted_only in any::<bool>(),
        ) {
            let grid = LogGrid { min_hz: 100.0, max_hz: 6400.0, points_per_octave: ppo };
            let n = grid.freqs().map(|f| f.len()).unwrap_or(0);
            let m: Vec<f64> = (0..n).map(|i| mag.get(i).copied().unwrap_or(0.0)).collect();
            let r = Response { grid, phase_deg: Some(m.clone()), valid: Some(m.iter().map(|v| *v > 0.0).collect()), mag_db: m, spl: false };
            if let Ok(t) = rew_text(&r, &RewExport { title, smoothing, trusted_only, ..Default::default() }) {
                let pts = parse_rew(&t);
                prop_assert!(!pts.is_empty());
                prop_assert!(pts.iter().all(|p| p.0.is_finite() && p.1.is_finite()));
            }
        }
    }
}
