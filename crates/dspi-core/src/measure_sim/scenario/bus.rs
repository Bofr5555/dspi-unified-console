//! P11 (WP FBUS, FSTORY section 3S.2): the function test's host audio bus.
//!
//! The music a user plays comes from other programs (a streaming app, a
//! player, a game) through the computer's output devices into a DSPi's
//! inputs. The bus stands in for that path:
//!
//! - **the bus file** (`DSPI_SIM_BUS`, else `bus.json` in `DSPI_SIM_OUT`),
//!   written by the harness and the sound-source stand-ins
//!   (`tools/functest/bus.py`), names the computer's **output devices**
//!   (each channel mapped to a DSPi input of the scenario, or to nowhere) and
//!   the **streams** playing to them (app, title, channels, loudness LUFS,
//!   true peak, content, play/pause, app volume); it is read again whenever
//!   it changes;
//! - every 100 ms the bus sums the streams per device (app volume, the OS
//!   volume unless it is the device's own USB volume, mutes, up- and
//!   down-mixing), hands each DSPi input an octave-band level and crest, and
//!   runs them through each DSPi's matrix and playback chain (P5,
//!   [`super::playback`]) to every output and on through the scenario's
//!   drivers and room to **dB SPL at the seat**;
//! - the demo DSPis' **meters** show those input and output peaks (what the
//!   measurement's preflight reads as programme audio);
//! - **UAC volume coupling**: an output device whose OS volume is the
//!   DSPi's USB volume (`hardwareVolume`) moves that DSPi's user volume when
//!   its volume changes, as the device-side write the USB path makes; and
//!   the OS shows the DSPi's user volume back;
//! - **logs**: `bus.jsonl` (events, and one line per 100 ms while anything
//!   plays: what plays where, dBFS per input and output, the limiter, dB SPL
//!   per driver and at the seat, clipping); programme in the **damage
//!   oracle** (`oracle.jsonl`): programme reaching a tweeter below its safe
//!   band beyond what a 12 dB/octave high-pass at the band's edge leaves, or
//!   clipping an output, is a `VIOLATION` as a test signal's would be;
//! - [`host_view`]: what the computer's audio settings show, for
//!   `dspi_get_host_audio` (dspi-mcp `sim-bus`).
//!
//! Levels are arithmetic on octave bands, not samples: loudness is taken as
//! the channels' RMS (BS.1770 without its K-weighting), streams add as
//! uncorrelated power, and a peak is the RMS plus the programme's crest.
//! Test-only (`sim-scenario`), as the rest of the scenario.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use dspi_transport::clock::Instant;
use serde::Deserialize;
use serde_json::{Value, json};

use super::playback::{after_chain_db, limiter_db, loudness_bands, psybass};
use super::{DeviceSpec, OutputSpec, Room, bands_response, decoded, lock, path_response, r1};
use crate::records::DspState;

/// The octave bands the bus carries, Hz.
pub(crate) const BANDS: [f64; 10] = [
    31.5, 63.0, 125.0, 250.0, 500.0, 1000.0, 2000.0, 4000.0, 8000.0, 16000.0,
];
/// How often the bus looks and logs.
const TICK: Duration = Duration::from_millis(100);
/// An input peak at or over this clips, dBFS (FSTORY 3S.2).
const INPUT_CLIP_DBFS: f64 = -0.1;
/// An output peak at or over this clips, dBFS.
const OUTPUT_CLIP_DBFS: f64 = -0.5;
/// Below this a level is silence, dBFS.
const FLOOR_DBFS: f64 = -120.0;
/// The tweeter rule's slope, dB/octave, and the slack the bus allows it, dB.
const RULE_DB_PER_OCTAVE: f64 = 12.0;
const RULE_SLACK_DB: f64 = 3.0;

fn default_os() -> String {
    "linux".into()
}
fn default_spl() -> f64 {
    115.0
}
fn hundred() -> f64 {
    100.0
}
fn default_lufs() -> f64 {
    -14.0
}
fn default_peak() -> f64 {
    -1.0
}
fn default_content() -> String {
    "music".into()
}
fn default_state() -> String {
    "play".into()
}

/// The bus file.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct BusSpec {
    /// linux, windows or macos: the app-volume law and the host view.
    #[serde(default = "default_os")]
    pub(crate) os: String,
    /// What the host view says answered ("simulated" when empty).
    #[serde(default)]
    pub(crate) audio_system: String,
    /// The dB SPL at the seat that 0 dBFS at the simulated microphone is.
    #[serde(default = "default_spl")]
    pub(crate) mic_spl_at_0_dbfs: f64,
    #[serde(default)]
    pub(crate) outputs: Vec<BusOutput>,
    #[serde(default)]
    pub(crate) streams: Vec<BusStream>,
}

/// One output device of the computer.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct BusOutput {
    /// What the OS shows.
    pub(crate) name: String,
    /// The OS's id ("" for the name).
    #[serde(default)]
    pub(crate) id: String,
    /// Text that holds its USB serial ("" none), as udev's
    /// `Vendor_Product_Serial`: how the app matches a DSPi to it.
    #[serde(default)]
    pub(crate) serial: String,
    pub(crate) channels: u8,
    #[serde(default)]
    pub(crate) rate_hz: Option<u32>,
    #[serde(default)]
    pub(crate) default: bool,
    #[serde(default = "hundred")]
    pub(crate) volume_pct: f64,
    #[serde(default)]
    pub(crate) muted: bool,
    /// Its OS volume is the device's own USB volume (the DSPi's user volume).
    #[serde(default)]
    pub(crate) hardware_volume: bool,
    /// Where each of its channels goes; none: nowhere (laptop speakers).
    #[serde(default)]
    pub(crate) map: Vec<MapEntry>,
}

/// One channel of an output device into one DSPi input.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct MapEntry {
    /// 0-based.
    pub(crate) channel: u8,
    /// The scenario device's serial.
    pub(crate) serial: String,
    /// 0-based.
    pub(crate) input: u8,
}

/// One app's stream.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct BusStream {
    pub(crate) id: String,
    pub(crate) app: String,
    #[serde(default)]
    pub(crate) title: String,
    pub(crate) channels: u8,
    /// Integrated loudness, LUFS.
    #[serde(default = "default_lufs")]
    pub(crate) lufs: f64,
    /// True peak, dBTP.
    #[serde(default = "default_peak")]
    pub(crate) true_peak_dbtp: f64,
    /// music, jazz, edm, film, speech, game, pink, sweep, or tone:HZ.
    #[serde(default = "default_content")]
    pub(crate) content: String,
    /// play or pause.
    #[serde(default = "default_state")]
    pub(crate) state: String,
    /// The output device's name or id ("" or unknown: it plays nowhere).
    #[serde(default)]
    pub(crate) output: String,
    #[serde(default = "hundred")]
    pub(crate) volume_pct: f64,
    #[serde(default)]
    pub(crate) muted: bool,
    /// Per channel, dB on top of the loudness (-200: silent).
    #[serde(default)]
    pub(crate) channel_db: Vec<f64>,
    /// Loudness over time from `startedUnixS`: the last point at or before
    /// now holds.
    #[serde(default)]
    pub(crate) envelope: Vec<EnvPoint>,
    #[serde(default)]
    pub(crate) started_unix_s: Option<f64>,
    /// A test-signal file played through the OS (the local player's trap).
    #[serde(default)]
    pub(crate) test_signal: bool,
}

/// A point of a stream's loudness envelope.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct EnvPoint {
    pub(crate) at_s: f64,
    pub(crate) lufs: f64,
    #[serde(default)]
    pub(crate) true_peak_dbtp: Option<f64>,
}

impl BusStream {
    fn playing(&self) -> bool {
        self.state == "play" && !self.muted && self.volume_pct > 0.0
    }
}

impl BusSpec {
    fn output(&self, name: &str) -> Option<&BusOutput> {
        if name.is_empty() {
            return None;
        }
        self.outputs
            .iter()
            .find(|o| o.name == name || (!o.id.is_empty() && o.id == name))
    }

    /// The app- and OS-volume law: PipeWire's and Core Audio's cubic
    /// slider, Windows' linear scalar.
    fn slider(&self, pct: f64) -> f64 {
        let x = (pct / 100.0).clamp(0.0, 1.0);
        if self.os == "windows" { x } else { x * x * x }
    }
}

/// The OS volume of a coupled output as the DSPi's user volume, dB.
pub(crate) fn pct_to_user_db(pct: f64) -> f32 {
    let x = (pct / 100.0).clamp(0.0, 1.0);
    if x <= 0.0 {
        return -60.0;
    }
    (60.0 * x.log10()).clamp(-60.0, 0.0) as f32
}

/// The DSPi's user volume as its OS volume, percent.
pub(crate) fn user_db_to_pct(db: f32) -> f64 {
    let db = f64::from(db.clamp(-60.0, 0.0));
    if db <= -60.0 {
        return 0.0;
    }
    (100.0 * 10f64.powf(db / 60.0) * 10.0).round() / 10.0
}

// ------------------------------------------------------------ the signal

/// A signal as the bus carries it: power per octave band (full scale 1) and
/// its peak amplitude.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(crate) struct Sig {
    pub(crate) bands: [f64; 10],
    pub(crate) peak: f64,
}

impl Sig {
    fn power(&self) -> f64 {
        self.bands.iter().sum()
    }

    pub(crate) fn rms_db(&self) -> f64 {
        db10(self.power())
    }

    pub(crate) fn peak_db(&self) -> f64 {
        db20(self.peak)
    }

    /// Peak over RMS, dB.
    fn crest_db(&self) -> f64 {
        if self.power() <= 0.0 {
            0.0
        } else {
            (self.peak_db() - self.rms_db()).max(0.0)
        }
    }

    fn add(&mut self, o: &Sig, power: f64) {
        for (a, b) in self.bands.iter_mut().zip(o.bands) {
            *a += b * power;
        }
        self.peak += o.peak * power.sqrt();
    }
}

fn db10(p: f64) -> f64 {
    if p > 0.0 { 10.0 * p.log10() } else { -200.0 }
}

fn db20(a: f64) -> f64 {
    if a > 0.0 { 20.0 * a.log10() } else { -200.0 }
}

/// A content's power per octave band, dB, before it is scaled to its level.
fn shape(content: &str) -> [f64; 10] {
    match content {
        "edm" | "party" => [-2.0, 0.0, 0.0, -2.0, -3.0, -4.0, -5.0, -7.0, -10.0, -14.0],
        "speech" | "dialogue" => [
            -40.0, -30.0, -14.0, -4.0, 0.0, -1.0, -4.0, -9.0, -15.0, -24.0,
        ],
        "film" => [-6.0, -4.0, -3.0, -2.0, 0.0, -1.0, -3.0, -6.0, -10.0, -16.0],
        "lfe" => [
            0.0, 0.0, -3.0, -24.0, -48.0, -72.0, -96.0, -120.0, -120.0, -120.0,
        ],
        "game" => [-6.0, -3.0, -2.0, -2.0, 0.0, -1.0, -2.0, -5.0, -9.0, -14.0],
        "pink" | "sweep" | "noise" => [0.0; 10],
        c if c.starts_with("tone:") => {
            let hz: f64 = c.trim_start_matches("tone:").parse().unwrap_or(1000.0);
            let near = BANDS
                .iter()
                .enumerate()
                .min_by(|a, b| {
                    (a.1.ln() - hz.max(1.0).ln())
                        .abs()
                        .total_cmp(&(b.1.ln() - hz.max(1.0).ln()).abs())
                })
                .map_or(5, |(i, _)| i);
            let mut v = [-200.0; 10];
            if let Some(x) = v.get_mut(near) {
                *x = 0.0;
            }
            v
        }
        // music, jazz, anything else
        _ => [-8.0, -4.0, -2.0, -1.0, 0.0, -1.0, -3.0, -6.0, -10.0, -16.0],
    }
}

/// The LFE channel's index in a 5.1 or 7.1 stream.
const LFE: usize = 3;

/// A stream's channels, at its level `now` (Unix seconds), before any volume.
pub(crate) fn stream_channels(st: &BusStream, now: f64) -> Vec<Sig> {
    let n = usize::from(st.channels.max(1));
    let (lufs, tp) = st
        .started_unix_s
        .map_or((st.lufs, st.true_peak_dbtp), |t0| {
            let at = now - t0;
            st.envelope
                .iter()
                .rfind(|p| p.at_s <= at)
                .map_or((st.lufs, st.true_peak_dbtp), |p| {
                    (p.lufs, p.true_peak_dbtp.unwrap_or(st.true_peak_dbtp))
                })
        });
    let lfe = |c: usize| n >= 6 && c == LFE;
    let offset = |c: usize| st.channel_db.get(c).copied().unwrap_or(0.0);
    // BS.1770: the LFE is not counted; each full channel at the same RMS
    let full = (0..n).filter(|c| !lfe(*c) && offset(*c) > -100.0).count();
    let base = lufs + 0.691 - 10.0 * (full.max(1) as f64).log10();
    (0..n)
        .map(|c| {
            let rms = base + offset(c);
            let peak = (tp + offset(c)).max(rms + 3.0);
            let sh = shape(if lfe(c) { "lfe" } else { st.content.as_str() });
            let total: f64 = sh.iter().map(|d| 10f64.powf(d / 10.0)).sum();
            let p = 10f64.powf(rms / 10.0);
            let mut s = Sig {
                peak: 10f64.powf(peak / 20.0),
                ..Sig::default()
            };
            for (b, d) in s.bands.iter_mut().zip(sh) {
                *b = p * 10f64.powf(d / 10.0) / total;
            }
            if rms < -150.0 {
                s = Sig::default();
            }
            s
        })
        .collect()
}

/// A stream of `from` channels on a device of `to`: which stream channels
/// feed each device channel, with their power gain.
fn mix(from: usize, to: usize) -> Vec<Vec<(usize, f64)>> {
    if from <= to {
        return (0..to)
            .map(|c| if c < from { vec![(c, 1.0)] } else { Vec::new() })
            .collect();
    }
    match to {
        // Mono: everything, at half power per channel pair
        1 => vec![
            (0..from)
                .filter(|c| from < 6 || *c != LFE)
                .map(|c| (c, 0.5))
                .collect(),
        ],
        // 5.1 or 7.1 to stereo: L + C/√2 + Ls/√2, R + C/√2 + Rs/√2
        2 if from >= 6 => vec![
            vec![(0, 1.0), (2, 0.5), (4, 0.5)],
            vec![(1, 1.0), (2, 0.5), (5, 0.5)],
        ],
        _ => (0..to).map(|c| vec![(c, 1.0)]).collect(),
    }
}

// ------------------------------------------------------------- one look

/// One scenario device as the bus sees it.
pub(crate) struct Unit<'a> {
    pub(crate) dev: &'a DeviceSpec,
    pub(crate) state: &'a DspState,
    /// The outputs the generator plays on now (it replaces programme there).
    pub(crate) generator: u16,
}

/// A stream after its volumes.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct StreamLevel {
    pub(crate) id: String,
    pub(crate) app: String,
    pub(crate) output: String,
    pub(crate) playing: bool,
    pub(crate) rms_db: f64,
    pub(crate) peak_db: f64,
    /// "Fronts in 1", ...: where its sound arrives ([] nowhere).
    pub(crate) reaches: Vec<String>,
    pub(crate) test_signal: bool,
}

/// A DSPi input.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct InputLevel {
    pub(crate) serial: String,
    pub(crate) device: String,
    pub(crate) input: usize,
    pub(crate) sig: Sig,
    pub(crate) clip: bool,
}

/// A DSPi output.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct OutputLevel {
    pub(crate) serial: String,
    pub(crate) device: String,
    pub(crate) output: usize,
    pub(crate) driver: Option<String>,
    pub(crate) sig: Sig,
    pub(crate) limiter_db: f64,
    pub(crate) clip: bool,
    pub(crate) spl_db: Option<f64>,
    /// The generator plays here: programme is replaced.
    pub(crate) generator: bool,
    /// Programme below the driver's safe band beyond the rule: the band, Hz,
    /// its peak, dBFS, and by how much it beats a 12 dB/octave high-pass.
    pub(crate) below: Option<(f64, f64, f64)>,
}

/// One look at the bus.
#[derive(Debug, Clone, PartialEq, Default)]
pub(crate) struct Look {
    pub(crate) streams: Vec<StreamLevel>,
    pub(crate) inputs: Vec<InputLevel>,
    pub(crate) outputs: Vec<OutputLevel>,
    pub(crate) seat_spl_db: Option<f64>,
}

impl Look {
    pub(crate) fn playing(&self) -> bool {
        self.streams.iter().any(|s| s.playing)
    }
}

/// The bus at `now` (Unix seconds) for `units`; `acoustic(x, f)` is output
/// `x`'s power gain from the amplifier to the microphone at `f`.
pub(crate) fn look(
    spec: &BusSpec,
    units: &[Unit<'_>],
    now: f64,
    acoustic: &dyn Fn(&OutputSpec, f64) -> f64,
) -> Look {
    let mut out = Look::default();
    // (serial, input) -> signal; and where each output device's channel goes
    let mut inputs: BTreeMap<(String, usize), Sig> = BTreeMap::new();
    for st in &spec.streams {
        let dev = spec.output(&st.output);
        let mut level = StreamLevel {
            id: st.id.clone(),
            app: st.app.clone(),
            output: dev.map_or_else(String::new, |d| d.name.clone()),
            playing: st.playing(),
            rms_db: -200.0,
            peak_db: -200.0,
            reaches: Vec::new(),
            test_signal: st.test_signal,
        };
        if !level.playing {
            out.streams.push(level);
            continue;
        }
        let mut power = spec.slider(st.volume_pct).powi(2);
        if let Some(d) = dev
            && !d.hardware_volume
        {
            power *= spec.slider(d.volume_pct).powi(2);
        }
        if dev.is_some_and(|d| d.muted) {
            power = 0.0;
        }
        let chans = stream_channels(st, now);
        let mut total = Sig::default();
        for c in &chans {
            total.add(c, power);
        }
        level.rms_db = total.rms_db();
        level.peak_db = chans
            .iter()
            .map(|c| c.peak_db() + db10(power))
            .fold(-200.0, f64::max);
        if let Some(d) = dev
            && power > 0.0
        {
            for (dc, feeds) in mix(chans.len(), usize::from(d.channels.max(1)))
                .iter()
                .enumerate()
            {
                let mut s = Sig::default();
                for (sc, g) in feeds {
                    if let Some(c) = chans.get(*sc) {
                        s.add(c, power * g);
                    }
                }
                if s.power() <= 0.0 {
                    continue;
                }
                for m in d.map.iter().filter(|m| usize::from(m.channel) == dc) {
                    let at = (m.serial.clone(), usize::from(m.input));
                    inputs.entry(at).or_default().add(&s, 1.0);
                    let name = units
                        .iter()
                        .find(|u| u.dev.serial == m.serial)
                        .map_or(m.serial.as_str(), |u| u.dev.name.as_str());
                    level
                        .reaches
                        .push(format!("{name} in {}", m.input.saturating_add(1)));
                }
            }
        }
        level.reaches.dedup();
        out.streams.push(level);
    }
    // A USB or S/PDIF input clips at full scale
    for ((serial, input), sig) in &mut inputs {
        let clip = sig.peak_db() >= INPUT_CLIP_DBFS;
        if sig.peak > 1.0 {
            sig.peak = 1.0;
        }
        let device = units
            .iter()
            .find(|u| u.dev.serial == *serial)
            .map_or_else(|| serial.clone(), |u| u.dev.name.clone());
        out.inputs.push(InputLevel {
            serial: serial.clone(),
            device,
            input: *input,
            sig: *sig,
            clip,
        });
    }
    let mut seat = 0.0;
    let mut any_spl = false;
    for u in units {
        let s = u.state;
        let n_in = usize::from(s.num_input_channels);
        let feeds: Vec<(usize, Sig)> = (0..n_in)
            .filter_map(|i| inputs.get(&(u.dev.serial.clone(), i)).map(|g| (i, *g)))
            .collect();
        if feeds.is_empty() {
            continue;
        }
        for o in 0..usize::from(s.num_output_channels) {
            let x = u.dev.output(o);
            let generator = u32::try_from(o)
                .ok()
                .and_then(|b| 1u16.checked_shl(b))
                .is_some_and(|m| u.generator & m != 0);
            let mut level = OutputLevel {
                serial: u.dev.serial.clone(),
                device: u.dev.name.clone(),
                output: o,
                driver: x.map(|x| x.name.clone()),
                sig: Sig::default(),
                limiter_db: 0.0,
                clip: false,
                spl_db: None,
                generator,
                below: None,
            };
            let Some(vol) = after_chain_db(s, o).filter(|_| !generator) else {
                if generator {
                    out.outputs.push(level);
                }
                continue;
            };
            let vol_power = 10f64.powf(vol / 10.0);
            let loud = loudness_bands(s, o);
            let pb = psybass(s, o);
            let mut crest: f64 = 0.0;
            for (i, g) in &feeds {
                let mut contrib = Sig::default();
                for (b, f) in BANDS.iter().enumerate() {
                    let (re, im) = path_response(s, *i, o, *f);
                    let mut p = re * re + im * im;
                    if let Some((low, _)) = &pb {
                        let (lr, li) = bands_response(low, *f);
                        p *= lr * lr + li * li;
                    }
                    let (lr, li) = bands_response(&loud, *f);
                    p *= (lr * lr + li * li) * vol_power;
                    if let (Some(dst), Some(src)) =
                        (contrib.bands.get_mut(b), g.bands.get(b).copied())
                    {
                        *dst = src * p;
                    }
                }
                if contrib.power() > 0.0 {
                    crest = crest.max(g.crest_db());
                }
                // The programme below a driver's safe band, input by input
                if let Some(x) = x
                    && x.safe.below_hz > 0.0
                {
                    let worst = below_rule(
                        s,
                        (*i, o),
                        (x.safe.below_hz, x.safe.below_dbfs),
                        g,
                        &contrib,
                    );
                    if let Some(w) = worst
                        && level.below.is_none_or(|b| w.2 > b.2)
                    {
                        level.below = Some(w);
                    }
                }
                for (a, b) in level.sig.bands.iter_mut().zip(contrib.bands) {
                    *a += b;
                }
            }
            // Psychoacoustic bass: the low band's energy again as harmonics
            if let Some((_, (lo, hi, harm))) = pb {
                let low: f64 = BANDS
                    .iter()
                    .zip(level.sig.bands)
                    .filter(|(f, _)| **f < lo)
                    .map(|(_, p)| p)
                    .sum();
                let into: Vec<usize> = (0..BANDS.len())
                    .filter(|b| BANDS.get(*b).is_some_and(|f| *f >= lo && *f <= hi))
                    .collect();
                let into = if into.is_empty() {
                    BANDS.iter().position(|f| *f >= lo).into_iter().collect()
                } else {
                    into
                };
                let each = low * 10f64.powf(harm / 10.0) / (into.len().max(1) as f64);
                for b in into {
                    if let Some(p) = level.sig.bands.get_mut(b) {
                        *p += each;
                    }
                }
            }
            // A peak: the RMS plus the programme's crest (the largest of the
            // inputs that reach it)
            let peak = level.sig.rms_db() + crest;
            level.sig.peak = 10f64.powf(peak / 20.0);
            if let Some(t) = limiter_db(s, o)
                && peak > t
            {
                level.limiter_db = peak - t;
                let k = 10f64.powf(-level.limiter_db / 10.0);
                level.sig.bands.iter_mut().for_each(|p| *p *= k);
                level.sig.peak = 10f64.powf(t / 20.0);
                if let Some(b) = level.below.as_mut() {
                    b.1 -= level.limiter_db;
                }
            }
            level.clip = level.sig.peak_db() >= OUTPUT_CLIP_DBFS;
            if let Some(x) = x {
                let mic: f64 = BANDS
                    .iter()
                    .zip(level.sig.bands)
                    .map(|(f, p)| p * acoustic(x, *f))
                    .sum();
                if mic > 0.0 {
                    level.spl_db = Some(db10(mic) + spec.mic_spl_at_0_dbfs);
                    seat += mic;
                    any_spl = true;
                }
            }
            if level.sig.rms_db() > FLOOR_DBFS || level.clip {
                out.outputs.push(level);
            }
        }
    }
    out.seat_spl_db = any_spl.then(|| db10(seat) + spec.mic_spl_at_0_dbfs);
    out
}

/// The worst band below `below_hz` on the path `i` -> `o`: `(band Hz, its
/// peak dBFS, dB over the rule)`, when it beats a 12 dB/octave high-pass at
/// `below_hz` (with slack), the path's response there against its own best
/// at or above `below_hz`, and its peak is over `limit` dBFS.
fn below_rule(
    s: &DspState,
    (i, o): (usize, usize),
    (below_hz, limit): (f64, f64),
    feed: &Sig,
    through: &Sig,
) -> Option<(f64, f64, f64)> {
    let power = |f: f64| {
        let (re, im) = path_response(s, i, o, f);
        re * re + im * im
    };
    let top = super::grid(12.0)
        .into_iter()
        .filter(|f| *f >= below_hz)
        .map(power)
        .fold(0.0, f64::max);
    if top <= 0.0 {
        return None;
    }
    let mut worst: Option<(f64, f64, f64)> = None;
    for (b, f) in BANDS.iter().enumerate() {
        if *f >= below_hz * 0.75 {
            continue;
        }
        let rel = db10(power(*f) / top);
        let over = rel + RULE_DB_PER_OCTAVE * (below_hz / f).log2() - RULE_SLACK_DB;
        let band = through.bands.get(b).copied().unwrap_or(0.0);
        if over <= 0.0 || band <= 0.0 {
            continue;
        }
        let peak = db10(band) + feed.crest_db();
        if peak > limit && worst.is_none_or(|w| over > w.2) {
            worst = Some((*f, peak, over));
        }
    }
    worst
}

// ------------------------------------------------------------ the logs

fn sig_json(s: &Sig) -> (f64, f64) {
    (r1(s.rms_db().max(-200.0)), r1(s.peak_db().max(-200.0)))
}

/// The tick's line for `bus.jsonl`.
pub(crate) fn look_json(l: &Look) -> Value {
    json!({
        "kind": "tick",
        "streams": l.streams.iter().map(|s| json!({
            "id": s.id, "app": s.app, "output": s.output, "playing": s.playing,
            "rmsDbfs": r1(s.rms_db), "peakDbfs": r1(s.peak_db), "reaches": s.reaches,
            "testSignal": s.test_signal,
        })).collect::<Vec<_>>(),
        "inputs": l.inputs.iter().map(|x| {
            let (rms, peak) = sig_json(&x.sig);
            json!({"device": x.device, "serial": x.serial, "input": x.input,
                   "rmsDbfs": rms, "peakDbfs": peak, "clip": x.clip})
        }).collect::<Vec<_>>(),
        "outputs": l.outputs.iter().map(|x| {
            let (rms, peak) = sig_json(&x.sig);
            json!({"device": x.device, "serial": x.serial, "output": x.output,
                   "driver": x.driver, "rmsDbfs": rms, "peakDbfs": peak,
                   "limiterDb": r1(x.limiter_db), "clip": x.clip,
                   "splDb": x.spl_db.map(r1), "generator": x.generator,
                   "bandsDbfs": x.sig.bands.iter().map(|p| r1(db10(*p))).collect::<Vec<_>>()})
        }).collect::<Vec<_>>(),
        "seatSplDb": l.seat_spl_db.map(r1),
        "bandsHz": BANDS,
    })
}

/// What changed between two bus files, as events.
pub(crate) fn changes(old: &BusSpec, new: &BusSpec) -> Vec<Value> {
    let mut v = Vec::new();
    for s in &new.streams {
        match old.streams.iter().find(|o| o.id == s.id) {
            None => v.push(json!({"what": "stream", "stream": s.id, "app": s.app,
                "title": s.title, "state": s.state, "output": s.output,
                "channels": s.channels, "lufs": s.lufs, "testSignal": s.test_signal})),
            Some(o) => {
                if o.state != s.state {
                    v.push(
                        json!({"what": s.state, "stream": s.id, "app": s.app, "output": s.output}),
                    );
                }
                if o.output != s.output {
                    v.push(json!({"what": "move", "stream": s.id, "app": s.app,
                        "from": o.output, "to": s.output}));
                }
                if o.volume_pct != s.volume_pct || o.muted != s.muted {
                    v.push(json!({"what": "stream-volume", "stream": s.id,
                        "from": o.volume_pct, "to": s.volume_pct, "muted": s.muted}));
                }
                if o.title != s.title || o.lufs != s.lufs || o.content != s.content {
                    v.push(json!({"what": "track", "stream": s.id, "title": s.title,
                        "lufs": s.lufs, "content": s.content}));
                }
            }
        }
    }
    for o in &old.streams {
        if !new.streams.iter().any(|s| s.id == o.id) {
            v.push(json!({"what": "stream-gone", "stream": o.id, "app": o.app}));
        }
    }
    for d in &new.outputs {
        if let Some(o) = old.outputs.iter().find(|o| o.name == d.name) {
            if o.volume_pct != d.volume_pct || o.muted != d.muted {
                v.push(json!({"what": "output-volume", "output": d.name,
                    "from": o.volume_pct, "to": d.volume_pct, "muted": d.muted}));
            }
            if o.default != d.default && d.default {
                v.push(json!({"what": "default", "output": d.name}));
            }
        } else {
            v.push(json!({"what": "output", "output": d.name, "channels": d.channels}));
        }
    }
    v
}

// ------------------------------------------------------------ the running bus

struct Running {
    path: PathBuf,
    stamp: Option<(SystemTime, u64)>,
    spec: Option<BusSpec>,
    last: Option<Instant>,
    was_playing: bool,
    /// Serials whose meters the bus set.
    metered: BTreeSet<String>,
    /// Oracle episodes going on: (serial, output, kind).
    episodes: BTreeSet<(String, usize, &'static str)>,
}

static RUNNING: OnceLock<Option<Mutex<Running>>> = OnceLock::new();

fn running() -> Option<&'static Mutex<Running>> {
    RUNNING
        .get_or_init(|| {
            let path = std::env::var_os("DSPI_SIM_BUS")
                .map(PathBuf::from)
                .or_else(|| {
                    std::env::var_os("DSPI_SIM_OUT").map(|d| PathBuf::from(d).join("bus.json"))
                })?;
            Some(Mutex::new(Running {
                path,
                stamp: None,
                spec: None,
                last: None,
                was_playing: false,
                metered: BTreeSet::new(),
                episodes: BTreeSet::new(),
            }))
        })
        .as_ref()
}

fn now_unix() -> f64 {
    dspi_transport::clock::system_time()
        .duration_since(UNIX_EPOCH)
        .map_or(0.0, |d| d.as_secs_f64())
}

impl Room {
    fn bus_note(&self, line: Value) {
        let Some(dir) = &self.out else { return };
        let mut line = line;
        if let Some(o) = line.as_object_mut() {
            let t = self.started.elapsed().as_secs_f64();
            o.insert("t".into(), json!((t * 1000.0).round() / 1000.0));
        }
        let file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(dir.join("bus.jsonl"));
        if let Ok(mut f) = file {
            use std::io::Write as _;
            let _ = writeln!(f, "{line}");
        }
    }

    /// The amplifier-to-microphone power gain of output `x` at `f`.
    fn acoustic_power(&self, x: &OutputSpec, f: f64) -> f64 {
        let room: Vec<_> = self
            .spec
            .modes
            .iter()
            .map(dspi_measure::sim::RoomMode::filter)
            .collect();
        let (re, im) = self.acoustic(x, &room, f);
        re * re + im * im
    }
}

/// One look of the bus, from the oracle's thread: at most every 100 ms.
pub(crate) fn tick(room: &Room) {
    let Some(m) = running() else { return };
    let mut r = lock(m);
    if r.last.is_some_and(|t| t.elapsed() < TICK) {
        return;
    }
    r.last = Some(Instant::now());
    // The file, again when it changed
    let stamp = std::fs::metadata(&r.path)
        .ok()
        .and_then(|m| Some((m.modified().ok()?, m.len())));
    if stamp != r.stamp {
        r.stamp = stamp;
        match stamp.map(|_| std::fs::read_to_string(&r.path)) {
            None => {}
            Some(Ok(text)) => match serde_json::from_str::<BusSpec>(&text) {
                Ok(spec) => {
                    let old = r.spec.clone().unwrap_or_default();
                    if r.spec.is_none() {
                        room.bus_note(json!({"kind": "event", "what": "load",
                            "outputs": spec.outputs.iter().map(|o| &o.name).collect::<Vec<_>>()}));
                    }
                    for e in changes(&old, &spec) {
                        let mut e = e;
                        if let Some(o) = e.as_object_mut() {
                            o.insert("kind".into(), json!("event"));
                        }
                        room.bus_note(e);
                    }
                    if r.spec.is_some() {
                        couple(room, &old, &spec);
                    }
                    r.spec = Some(spec);
                }
                Err(e) => {
                    room.bus_note(json!({"kind": "error", "what": format!("the bus file: {e}")}))
                }
            },
            Some(Err(e)) => {
                room.bus_note(json!({"kind": "error", "what": format!("the bus file: {e}")}))
            }
        }
    }
    let Some(spec) = r.spec.clone() else { return };
    // The devices as they are now
    let live: Vec<(usize, dspi_transport::FakeRemote, DspState, u16)> = lock(&room.live)
        .iter()
        .filter_map(|l| {
            let s = decoded(&l.remote.image())?;
            let generator = match (
                l.remote.measure_generator_running(),
                l.remote.measure_generator_config(),
            ) {
                (Some(_), Some(c)) => c.channel_mask,
                _ => 0,
            };
            Some((l.spec, l.remote.clone(), s, generator))
        })
        .collect();
    let units: Vec<Unit<'_>> = live
        .iter()
        .filter_map(|(i, _, s, g)| {
            Some(Unit {
                dev: room.spec.devices.get(*i)?,
                state: s,
                generator: *g,
            })
        })
        .collect();
    let l = look(&spec, &units, now_unix(), &|x, f| room.acoustic_power(x, f));
    let playing = l.playing();
    // The meters of every device the bus feeds
    let fed: BTreeSet<String> = spec
        .outputs
        .iter()
        .flat_map(|o| o.map.iter().map(|m| m.serial.clone()))
        .collect();
    for (i, remote, s, _) in &live {
        let Some(dev) = room.spec.devices.get(*i) else {
            continue;
        };
        if !fed.contains(&dev.serial) {
            continue;
        }
        let n_in = usize::from(s.num_input_channels);
        let n_out = usize::from(s.num_output_channels);
        let mut clip = 0u32;
        let ins: Vec<f32> = (0..n_in)
            .map(|k| {
                l.inputs
                    .iter()
                    .find(|x| x.serial == dev.serial && x.input == k)
                    .map_or(-200.0, |x| {
                        if x.clip {
                            clip |= 1u32.checked_shl(k as u32).unwrap_or(0);
                        }
                        x.sig.peak_db() as f32
                    })
            })
            .collect();
        let outs: Vec<f32> = (0..n_out)
            .map(|k| {
                l.outputs
                    .iter()
                    .find(|x| x.serial == dev.serial && x.output == k && !x.generator)
                    .map_or(-200.0, |x| {
                        if x.clip {
                            clip |= 1u32.checked_shl(n_in.saturating_add(k) as u32).unwrap_or(0);
                        }
                        x.sig.peak_db() as f32
                    })
            })
            .collect();
        let active = ins.iter().filter(|d| **d > -120.0).count();
        if active > 0 || r.metered.contains(&dev.serial) {
            remote.sim_set_meters(&ins, &outs, clip, u8::try_from(active).unwrap_or(u8::MAX));
            if active > 0 {
                r.metered.insert(dev.serial.clone());
            } else {
                r.metered.remove(&dev.serial);
            }
        }
    }
    if playing || r.was_playing {
        room.bus_note(look_json(&l));
    }
    r.was_playing = playing;
    oracle(room, &mut r.episodes, &l);
}

/// The damage oracle's lines for programme: once when an episode starts,
/// once when it ends.
fn oracle(room: &Room, episodes: &mut BTreeSet<(String, usize, &'static str)>, l: &Look) {
    let mut now = BTreeSet::new();
    for x in &l.outputs {
        let base = json!({"how": "programme", "device": x.device, "serial": x.serial,
            "output": x.output, "driver": x.driver});
        let mut lines = Vec::new();
        if let Some((hz, peak, over)) = x.below {
            lines.push(("below", "VIOLATION", format!(
                "programme reaches it at {peak:.1} dBFS peak in the {hz:.0} Hz band, {over:.1} dB more than a 12 dB/octave high-pass at its safe band's edge leaves")));
        }
        if x.clip {
            lines.push((
                "clip",
                "VIOLATION",
                format!("programme clips it: {:.1} dBFS peak", x.sig.peak_db()),
            ));
        }
        if let Some(d) = x.driver.as_ref().and_then(|_| {
            room.device(&x.serial)
                .and_then(|d| d.output(x.output))
                .filter(|o| x.sig.peak_db() > o.safe.cap_dbfs + 0.05)
        }) {
            lines.push((
                "cap",
                "WARN",
                format!(
                    "programme peaks at {:.1} dBFS, over the {:.0} dBFS a test signal may reach",
                    x.sig.peak_db(),
                    d.safe.cap_dbfs
                ),
            ));
        }
        for (key, kind, why) in lines {
            let k = (x.serial.clone(), x.output, key);
            if !episodes.contains(&k) {
                let mut line = base.clone();
                if let Some(o) = line.as_object_mut() {
                    o.insert("kind".into(), json!(kind));
                    o.insert("why".into(), json!([why]));
                    o.insert("rmsDbfs".into(), json!(r1(x.sig.rms_db())));
                    o.insert("peakDbfs".into(), json!(r1(x.sig.peak_db())));
                }
                room.note(line);
            }
            now.insert(k);
        }
    }
    for x in &l.inputs {
        let k = (x.serial.clone(), x.input, "input-clip");
        if x.clip {
            if !episodes.contains(&k) {
                room.note(json!({"kind": "WARN", "how": "programme", "device": x.device,
                    "serial": x.serial, "input": x.input,
                    "why": [format!("the source clips input {}: {:.1} dBFS peak", x.input.saturating_add(1), x.sig.peak_db())]}));
            }
            now.insert(k);
        }
    }
    for k in episodes.difference(&now) {
        room.note(
            json!({"kind": "info", "how": "programme", "serial": k.0, "channel": k.1,
            "what": format!("{} ended", k.2)}),
        );
    }
    *episodes = now;
}

/// UAC volume coupling: an output whose OS volume is a DSPi's USB volume
/// moved; that DSPi's user volume follows, as the device-side write.
fn couple(room: &Room, old: &BusSpec, new: &BusSpec) {
    for d in new.outputs.iter().filter(|d| d.hardware_volume) {
        let Some(o) = old.outputs.iter().find(|o| o.name == d.name) else {
            continue;
        };
        if o.volume_pct == d.volume_pct {
            continue;
        }
        let serials: BTreeSet<&str> = d.map.iter().map(|m| m.serial.as_str()).collect();
        let db = pct_to_user_db(d.volume_pct);
        for serial in serials {
            let live = lock(&room.live);
            let remote = live
                .iter()
                .find(|l| {
                    room.spec
                        .devices
                        .get(l.spec)
                        .is_some_and(|x| x.serial == serial)
                })
                .map(|l| l.remote.clone());
            drop(live);
            if let Some(r) = remote {
                let ok = r.measure_host_volume(db);
                room.bus_note(json!({"kind": "event", "what": "uac-volume", "output": d.name,
                    "serial": serial, "volumePct": d.volume_pct, "userVolumeDb": r1(f64::from(db)), "ok": ok}));
            }
        }
    }
}

// ------------------------------------------------------------ the host view

/// One output device as the computer's audio settings show it.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SimHostOutput {
    /// The OS's id.
    pub id: String,
    /// The name the OS shows.
    pub name: String,
    /// Text holding its USB serial ("" none).
    pub serial: String,
    /// The OS's default output.
    pub is_default: bool,
    /// Its channel count.
    pub channels: u32,
    /// Its sample rate, Hz.
    pub rate_hz: u32,
    /// Something plays to it.
    pub running: bool,
    /// The OS volume, percent (a coupled DSPi's user volume).
    pub volume_pct: f64,
    /// Muted at the OS.
    pub muted: bool,
    /// The OS volume is the device's USB volume.
    pub hardware_volume: bool,
    /// What plays to it.
    pub streams: Vec<SimHostStream>,
}

/// One app's stream as the computer shows it.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SimHostStream {
    /// The app.
    pub app: String,
    /// What it plays.
    pub title: String,
    /// Its channel count.
    pub channels: u32,
    /// Its own volume, percent.
    pub volume_pct: f64,
    /// Muted.
    pub muted: bool,
    /// Playing (else paused).
    pub playing: bool,
}

/// The computer's audio, simulated.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SimHostAudio {
    /// linux, windows or macos.
    pub os: String,
    /// What answers ("PipeWire 1.6").
    pub audio_system: String,
    /// The output devices.
    pub outputs: Vec<SimHostOutput>,
}

/// The host view of `spec`; `user_db(serial)` is a DSPi's user volume (a
/// coupled output shows it as its OS volume).
pub(crate) fn host_of(spec: &BusSpec, user_db: &dyn Fn(&str) -> Option<f32>) -> SimHostAudio {
    let outputs = spec
        .outputs
        .iter()
        .map(|d| {
            let streams: Vec<SimHostStream> = spec
                .streams
                .iter()
                .filter(|s| spec.output(&s.output).is_some_and(|o| o.name == d.name))
                .map(|s| SimHostStream {
                    app: s.app.clone(),
                    title: s.title.clone(),
                    channels: u32::from(s.channels),
                    volume_pct: s.volume_pct,
                    muted: s.muted,
                    playing: s.state == "play",
                })
                .collect();
            let coupled = d
                .hardware_volume
                .then(|| d.map.first().and_then(|m| user_db(&m.serial)))
                .flatten();
            SimHostOutput {
                id: if d.id.is_empty() {
                    d.name.clone()
                } else {
                    d.id.clone()
                },
                name: d.name.clone(),
                serial: d.serial.clone(),
                is_default: d.default,
                channels: u32::from(d.channels),
                rate_hz: d.rate_hz.unwrap_or(48_000),
                running: streams.iter().any(|s| s.playing),
                volume_pct: coupled.map_or(d.volume_pct, user_db_to_pct),
                muted: d.muted,
                hardware_volume: d.hardware_volume,
                streams,
            }
        })
        .collect();
    SimHostAudio {
        os: spec.os.clone(),
        audio_system: if spec.audio_system.is_empty() {
            "PipeWire".into() // -- WP ODEC -- no sim mark for agents
        } else {
            spec.audio_system.clone()
        },
        outputs,
    }
}

/// The computer's simulated audio, while a scenario and a bus file are
/// loaded; `None` otherwise (the real reader answers).
pub fn host_view() -> Option<SimHostAudio> {
    let room = super::room()?;
    let path = lock(running()?).path.clone();
    let spec: BusSpec = serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()?;
    let user = |serial: &str| -> Option<f32> {
        let live = lock(&room.live);
        let l = live.iter().find(|l| {
            room.spec
                .devices
                .get(l.spec)
                .is_some_and(|d| d.serial == serial)
        })?;
        Some(decoded(&l.remote.image())?.user_volume_db)
    };
    Some(host_of(&spec, &user))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::float_cmp)]
mod tests_wp_fbus;
