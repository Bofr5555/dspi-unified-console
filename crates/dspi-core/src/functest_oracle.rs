//! -- WP FUNC2a -- The function test's config-time oracle (FSTORY P5a).
//!
//! The scenario's damage oracle (`measure_sim::scenario`) samples each device's
//! image every few milliseconds. This one looks at the core's own mirror after
//! **every journal entry**, whoever made it, so a setting that would let
//! programme reach a tweeter (or a midrange) below its safe band is caught even
//! when nothing plays and however briefly it lasted. It judges by the
//! scenario's drivers (what is really wired), not by the speaker setup the user
//! confirmed: the damage is physical.
//!
//! The rule, per output whose driver is a tweeter (`role` "high": 1 kHz) or a
//! midrange ("mid": 80 Hz), as the MCP hard rule states it: while the output
//! can play (routed, enabled, not muted), its shape (the worst routed input's
//! EQ, the output's EQ and crossover; not the gains, which are level) may rise
//! nowhere below 0.75 × the floor more than 3 dB above a 12 dB/octave
//! high-pass at the floor. Breaking it is a `VIOLATION`; the same shape on an
//! output that cannot play now is a `WARN`. Full-scale programme at the volume
//! set over the driver's test-signal cap is a `WARN` too (`how`: "level").
//!
//! It writes `config-oracle.jsonl` to `DSPI_SIM_OUT`: one `entry` line per
//! journal entry (device, path, indices, actor) and the findings after it.
//! Only with the `sim-scenario` feature and a scenario loaded; nothing else
//! calls it.

use std::io::Write as _;
use std::sync::{Mutex, PoisonError};

use dspi_measure::sim::section_response;
use dspi_measure::types::{BandSpec, SectionModel};
use serde_json::{Value, json};

use crate::measure_doc::CoreSectionModel;
use crate::records::DspState;

const FS: f64 = 48_000.0;
/// How far a chain may rise above a 12 dB/octave high-pass at the floor
/// (the MCP rule's `BELOW_FLOOR_MARGIN_DB`).
const MARGIN_DB: f64 = 3.0;

static FILE: Mutex<()> = Mutex::new(());

/// The floor of a driver role, Hz; `None` for one with no floor.
fn floor_of(role: &str) -> Option<f64> {
    match role {
        "high" => Some(1000.0),
        "mid" => Some(80.0),
        _ => None,
    }
}

/// Judge `s` (the mirror of device `serial`) after the journal entry of
/// `path`/`indices` by `actor`, and log it.
pub(crate) fn journalled(serial: &str, s: &DspState, path: &str, indices: &[u8], actor: u32) {
    let Some(room) = crate::measure_sim::scenario::room() else {
        return;
    };
    let Some(dev) = room.spec.devices.iter().find(|d| d.serial == serial) else {
        return;
    };
    let mut lines = vec![json!({"kind": "entry", "device": dev.name, "path": path,
        "indices": indices, "actor": actor})];
    for x in &dev.outputs {
        let Some(floor) = floor_of(&x.role) else {
            continue;
        };
        let o = usize::from(x.output);
        let plays = routed(s, o) && !off(s, o);
        let (rise, at) = worst_rise(s, o, floor);
        if rise > MARGIN_DB {
            let why = format!(
                "its EQ and crossover pass {rise:.1} dB more than a 12 dB/octave high-pass at {floor:.0} Hz, at {at:.0} Hz{}",
                if plays { "" } else { " (it plays nothing now)" }
            );
            lines.push(
                json!({"kind": if plays { "VIOLATION" } else { "WARN" }, "how": "config",
                "device": dev.name, "serial": dev.serial, "output": o, "driver": x.name,
                "path": path, "actor": actor, "why": [why]}),
            );
        }
        if plays {
            let volume =
                f64::from(s.user_volume_db.min(0.0)) + f64::from(s.master_volume_db.min(0.0));
            let peak = programme_peak(s, o) + volume;
            if peak > x.safe.cap_dbfs {
                lines.push(json!({"kind": "WARN", "how": "level", "device": dev.name,
                    "serial": dev.serial, "output": o, "driver": x.name, "path": path, "actor": actor,
                    "why": [format!("full-scale programme reaches {peak:.1} dBFS at the volume set, over its cap of {:.0} dBFS", x.safe.cap_dbfs)]}));
            }
        }
    }
    write(&lines);
}

fn write(lines: &[Value]) {
    let Some(dir) = std::env::var_os("DSPI_SIM_OUT") else {
        return;
    };
    let _held = FILE.lock().unwrap_or_else(PoisonError::into_inner);
    let path = std::path::Path::new(&dir).join("config-oracle.jsonl");
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let text: String = lines.iter().map(|l| format!("{l}\n")).collect();
        let _ = f.write_all(text.as_bytes());
    }
}

/// The most output `o`'s shape rises over the reference below 0.75 × `floor`,
/// dB, and where.
fn worst_rise(s: &DspState, o: usize, floor: f64) -> (f64, f64) {
    let chain = crate::measure::chain_of(s, u8::try_from(o).unwrap_or(u8::MAX));
    let inputs: Vec<Vec<BandSpec>> = (0..usize::from(s.num_input_channels))
        .filter(|&i| at(&s.matrix_routing, i, o))
        .map(|i| input_bands(s, i))
        .collect();
    let mut worst = (f64::NEG_INFINITY, 0.0);
    let mut f = 20.0;
    while f <= floor * 0.75 {
        let own = db(bands(&chain.peq, f)) + db(bands(&chain.crossover, f));
        let input = inputs.iter().map(|b| db(bands(b, f))).fold(
            if inputs.is_empty() {
                0.0
            } else {
                f64::NEG_INFINITY
            },
            f64::max,
        );
        let rise = own + input - reference_db(f, floor);
        if rise > worst.0 {
            worst = (rise, f);
        }
        f *= 2f64.powf(1.0 / 24.0);
    }
    worst
}

/// A second-order Butterworth high-pass at `floor`, dB at `f`.
fn reference_db(f: f64, floor: f64) -> f64 {
    let x = (f / floor).powi(2);
    10.0 * (x * x / ((1.0 - x).powi(2) + 2.0 * x)).log10()
}

/// The peak, over frequency, of full-scale programme on every routed input
/// summed at output `o` (gains included), dBFS before the volume.
fn programme_peak(s: &DspState, o: usize) -> f64 {
    let chain = crate::measure::chain_of(s, u8::try_from(o).unwrap_or(u8::MAX));
    let mut peak = f64::NEG_INFINITY;
    let mut f = 20.0;
    while f <= 20_000.0 {
        let own =
            db(bands(&chain.peq, f)) + db(bands(&chain.crossover, f)) + f64::from(chain.gain_db);
        let mut sum = 0.0;
        for i in (0..usize::from(s.num_input_channels)).filter(|&i| at(&s.matrix_routing, i, o)) {
            let gain = s
                .matrix_gain
                .get(i)
                .and_then(|r| r.get(o))
                .copied()
                .unwrap_or(0.0)
                + s.input_preamp_db.get(i).copied().unwrap_or(0.0)
                + s.preamp_db;
            sum += 10f64.powf((db(bands(&input_bands(s, i), f)) + f64::from(gain)) / 20.0);
        }
        peak = peak.max(own + 20.0 * sum.max(1e-15).log10());
        f *= 2f64.powf(1.0 / 12.0);
    }
    peak
}

fn input_bands(s: &DspState, i: usize) -> Vec<BandSpec> {
    s.filters
        .get(i)
        .map(|b| {
            b.iter()
                .map(|f| BandSpec {
                    filter_type: f.filter_type,
                    bypass: f.bypass || s.bypass,
                    freq: f.freq,
                    q: f.q,
                    gain: f.gain,
                    qp: f.qp,
                })
                .collect()
        })
        .unwrap_or_default()
}

fn bands(bands: &[BandSpec], f: f64) -> (f64, f64) {
    let model = CoreSectionModel;
    bands.iter().fold((1.0, 0.0), |h, b| {
        model.sections(b).iter().fold(h, |h, c| {
            let r = section_response(c, f, FS);
            (h.0 * r.0 - h.1 * r.1, h.0 * r.1 + h.1 * r.0)
        })
    })
}

fn db(v: (f64, f64)) -> f64 {
    10.0 * (v.0 * v.0 + v.1 * v.1).max(1e-30).log10()
}

fn at(m: &[Vec<bool>], i: usize, o: usize) -> bool {
    m.get(i).and_then(|r| r.get(o)).copied().unwrap_or(false)
}

fn routed(s: &DspState, o: usize) -> bool {
    (0..usize::from(s.num_input_channels)).any(|i| at(&s.matrix_routing, i, o))
}

fn off(s: &DspState, o: usize) -> bool {
    !s.output_enabled.get(o).copied().unwrap_or(false)
        || s.output_muted.get(o).copied().unwrap_or(false)
        || s.master_volume_db <= SILENT_MASTER_DB // -- WP FUNC2c --
}

// -- WP FUNC2c -- The master volume at which the device plays nothing: the
// MCP's guard around a preset load (dspi-mcp `LOAD_GUARD_DB`, FFIX), which
// holds the device silent until the loaded preset is judged and its tweeter
// protection put back (by the policy, or by the installer's profile). FSTORY
// S7's lowered-tweeter preset: without this the guard's silent moment was a
// VIOLATION.
const SILENT_MASTER_DB: f32 = -127.0;

#[cfg(test)]
mod tests_wp_func2a {
    use super::*;

    #[test]
    fn wp_func2a_reference_is_a_12_db_per_octave_high_pass() {
        assert!(reference_db(1000.0, 1000.0).abs() - 3.01 < 0.05);
        let octave = reference_db(500.0, 1000.0) - reference_db(250.0, 1000.0);
        assert!((octave - 12.0).abs() < 0.6, "{octave}");
    }

    fn state() -> DspState {
        let mut s = crate::state::default_state();
        for b in s.filters.iter_mut().chain(s.xover.iter_mut()).flatten() {
            b.filter_type = 0;
            b.bypass = false;
            b.gain = 0.0;
        }
        s
    }

    #[test]
    fn wp_func2a_a_lr4_at_the_floor_passes_and_none_or_a_low_one_does_not() {
        let mut s = state();
        let ch = usize::from(s.num_input_channels) + 1;
        // No high-pass at all
        assert!(worst_rise(&s, 1, 1000.0).0 > MARGIN_DB);
        // LR4 (type 35) at 1 kHz: passes; at 500 Hz: does not
        s.xover[ch][0].filter_type = 35;
        s.xover[ch][0].freq = 1000.0;
        assert!(worst_rise(&s, 1, 1000.0).0 <= MARGIN_DB);
        s.xover[ch][0].freq = 500.0;
        assert!(worst_rise(&s, 1, 1000.0).0 > MARGIN_DB);
        // A first-order Butterworth (type 41) at 2 kHz is too shallow
        s.xover[ch][0].filter_type = 41;
        s.xover[ch][0].freq = 2000.0;
        assert!(worst_rise(&s, 1, 1000.0).0 > MARGIN_DB);
        // LR4 at 2.5 kHz: passes
        s.xover[ch][0].filter_type = 35;
        s.xover[ch][0].freq = 2500.0;
        assert!(worst_rise(&s, 1, 1000.0).0 <= MARGIN_DB);
    }

    // -- WP FUNC2c --
    #[test]
    fn wp_func2c_a_silenced_master_plays_nothing() {
        let mut s = state();
        s.output_enabled[1] = true;
        s.output_muted[1] = false;
        s.master_volume_db = 0.0;
        assert!(!off(&s, 1));
        s.master_volume_db = SILENT_MASTER_DB;
        assert!(off(&s, 1));
    }
}
