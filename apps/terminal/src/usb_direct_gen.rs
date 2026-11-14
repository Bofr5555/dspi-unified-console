//! `uconsole usb-direct generate` (WP AUSB3): the computer's side of a
//! USB-direct home theatre, written from one installation file.
//!
//! A central computer plays one multichannel programme (5.1, 7.1, ...) to
//! several DSPi units over USB; each unit drives a few speakers and runs as an
//! alink USB-direct unit (DSPi-bofr `extension_channel_spec.md` 2.9): mode 2
//! locks its clock and its phase to the USB Start of Frame, and it plays every
//! sample D samples after it arrived. This command writes, and only writes
//! files (nothing here opens a unit or plays audio):
//!
//! - `pipewire/` a PipeWire drop-in: one combined sink with the programme's
//!   channels that splits them into one stream per unit (combine-stream),
//!   the graph held at the units' rate (no resampling) with a fixed quantum;
//! - `wireplumber/` a WirePlumber 0.5 drop-in: each unit's sink opened in its
//!   exact USB format, one clock and one driver group for all of them, no
//!   resampling, never suspended (a reopened stream moves its start offset);
//! - `units/<id>.dspi` a Terminal script per unit: input source USB and the
//!   matrix crosspoints that put each USB channel on its output;
//! - `hub-system-model.json` the hub's system model (`system.set`): the units
//!   as USB-direct, mode, D, the lock-error action, the speakers;
//! - `SETTINGS.md` the per-unit settings sheet and the steps, for people.
//!
//! The guide is `docs/integrator/usb-direct-home-theater.md`.
#![warn(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use dspi_cmd::COMMAND;
use dspi_hub::system::model::SystemModel;
use dspi_proto::ext::alink_usb::{DEFAULT_D, d_limits};
use serde_json::{Map, Value, json};

use crate::exit;

/// The most units one installation file takes (a hub's model takes 256; one
/// computer's USB carries far fewer, see the guide's capacity table).
pub const MAX_UNITS: usize = 32;

/// Channel names PipeWire knows (SPA's short names), besides `AUX0`..`AUX63`.
const POSITIONS: &[&str] = &[
    "MONO", "FL", "FR", "FC", "LFE", "SL", "SR", "FLC", "FRC", "RC", "RL", "RR", "TC", "TFL",
    "TFC", "TFR", "TRL", "TRC", "TRR", "RLC", "RRC", "FLW", "FRW", "LFE2", "FLH", "FCH", "FRH",
    "TFLC", "TFRC", "TSL", "TSR", "LLFE", "RLFE", "BC", "BLC", "BRC",
];

/// The `layout` names and their channels, in PipeWire's order.
const LAYOUTS: &[(&str, &[&str])] = &[
    ("2.0", &["FL", "FR"]),
    ("2.1", &["FL", "FR", "LFE"]),
    ("4.0", &["FL", "FR", "RL", "RR"]),
    ("5.1", &["FL", "FR", "FC", "LFE", "RL", "RR"]),
    ("7.1", &["FL", "FR", "FC", "LFE", "RL", "RR", "SL", "SR"]),
    (
        "7.1.4",
        &[
            "FL", "FR", "FC", "LFE", "RL", "RR", "SL", "SR", "TFL", "TFR", "TRL", "TRR",
        ],
    ),
];

/// What the units' chip offers over USB (DSPi-bofr `usb_descriptors.c`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Chip {
    /// 2 channels at 44.1/48/96 kHz; 4 (16 or 24 bit), 6 and 8 (16 bit) at
    /// 48 kHz only. 8 inputs, 9 outputs (8 S/PDIF + PDM).
    Rp2350,
    /// 2 channels only. 2 inputs, 5 outputs (4 S/PDIF + PDM).
    Rp2040,
}

impl Chip {
    fn inputs(self) -> u8 {
        match self {
            Self::Rp2350 => 8,
            Self::Rp2040 => 2,
        }
    }

    fn outputs(self) -> u8 {
        match self {
            Self::Rp2350 => 9,
            Self::Rp2040 => 5,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Rp2350 => "rp2350",
            Self::Rp2040 => "rp2040",
        }
    }
}

/// The USB format a unit's stream is opened in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Format {
    pub channels: u8,
    pub bits: u8,
}

impl Format {
    /// PipeWire's `audio.format`: S24LE is packed 3-byte, as the units take it.
    fn pipewire(self) -> &'static str {
        if self.bits == 24 { "S24LE" } else { "S16LE" }
    }
}

/// The best format that carries `n` channels: 24 bit where the firmware
/// offers it (stereo, 4 channels), else 16 bit.
pub fn pick_format(chip: Chip, rate: u32, n: usize) -> Result<Format, String> {
    let f = |channels, bits| Ok(Format { channels, bits });
    match (chip, n) {
        (_, 0) => Err("no channels".into()),
        (_, 1..=2) => f(2, 24),
        (Chip::Rp2040, _) => Err(format!("{n} channels: an RP2040 unit takes 2 over USB")),
        (Chip::Rp2350, 3..=8) if rate != 48_000 => Err(format!(
            "{n} channels at {rate} Hz: more than 2 channels only at 48 kHz"
        )),
        (Chip::Rp2350, 3..=4) => f(4, 24),
        (Chip::Rp2350, 5..=6) => f(6, 16),
        (Chip::Rp2350, 7..=8) => f(8, 16),
        (Chip::Rp2350, _) => Err(format!("{n} channels: a unit takes at most 8 over USB")),
    }
}

/// `EXT_ALINK_USB.mode` as the installation asks it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Mode 2: rate and phase lock, sample-aligned (48 and 96 kHz).
    PhaseLock,
    /// Mode 1: rate lock only; the units' phases are whatever the streams
    /// started with.
    RateLock,
}

impl Mode {
    fn as_str(self) -> &'static str {
        match self {
            Self::PhaseLock => "phase_lock",
            Self::RateLock => "rate_lock",
        }
    }
}

/// One speaker channel of a unit.
#[derive(Debug, Clone, PartialEq)]
pub struct Channel {
    /// The programme's channel (`FL`, `LFE`, ...).
    pub channel: String,
    /// The unit's USB input (0-based), from the order given unless named.
    pub input: u8,
    /// The unit's output (0-based; 8 is the PDM sub on an RP2350).
    pub output: u8,
    /// Speaker to listening position, metres (the hub's distance delays).
    pub distance_m: f32,
}

/// One unit.
#[derive(Debug, Clone, PartialEq)]
pub struct Unit {
    pub id: String,
    pub name: String,
    pub serial: String,
    pub chip: Chip,
    pub play_unaligned: bool,
    pub channels: Vec<Channel>,
    pub format: Format,
}

/// A whole installation, checked.
#[derive(Debug, Clone, PartialEq)]
pub struct Installation {
    pub name: String,
    pub description: String,
    pub rate: u32,
    pub positions: Vec<String>,
    pub quantum: u32,
    pub mode: Mode,
    /// D in samples (phase lock only).
    pub d: Option<u16>,
    pub keep_aligned: bool,
    pub units: Vec<Unit>,
    /// Things worth saying that do not stop the files being written.
    pub warnings: Vec<String>,
}

fn is_position(p: &str) -> bool {
    POSITIONS.contains(&p)
        || p.strip_prefix("AUX")
            .and_then(|n| {
                n.parse::<u8>()
                    .ok()
                    .filter(|n| *n < 64)
                    .map(|n| n.to_string())
            })
            .is_some_and(|n| p.get(3..) == Some(n.as_str()))
}

fn word_ok(what: &str, s: &str, max: usize) -> Result<(), String> {
    let first_ok = s
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit());
    if !first_ok
        || s.len() > max
        || !s
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
    {
        return Err(format!(
            "{what} {s:?}: 1-{max} of a-z, 0-9, '-' and '_', starting with a letter or digit"
        ));
    }
    Ok(())
}

/// Text that goes into a PipeWire string, the model and Markdown.
fn label_ok(what: &str, s: &str) -> Result<(), String> {
    if s.chars().count() > 64
        || s.chars()
            .any(|c| c.is_control() || matches!(c, '"' | '\\' | '|' | '`'))
    {
        return Err(format!(
            "{what} {s:?}: at most 64 characters, no control characters, '\"', '\\\\', '|' or '`'"
        ));
    }
    Ok(())
}

fn serial_ok(s: &str) -> Result<(), String> {
    if s.is_empty() || s.len() > 32 || !s.chars().all(|c| c.is_ascii_alphanumeric()) {
        return Err(format!(
            "serial {s:?}: the unit's USB serial, 1-32 letters and digits (16 hex digits on a DSPi)"
        ));
    }
    Ok(())
}

fn fields(v: &Value, what: &str, allowed: &[&str]) -> Result<Map<String, Value>, String> {
    let Some(m) = v.as_object() else {
        return Err(format!("{what}: an object"));
    };
    if let Some(k) = m.keys().find(|k| !allowed.contains(&k.as_str())) {
        return Err(format!(
            "{what}: unknown field {k:?} (known: {})",
            allowed.join(", ")
        ));
    }
    Ok(m.clone())
}

fn string(m: &Map<String, Value>, key: &str, what: &str) -> Result<Option<String>, String> {
    match m.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s.clone())),
        Some(_) => Err(format!("{what}: {key} is a string")),
    }
}

fn uint(m: &Map<String, Value>, key: &str, what: &str, max: u64) -> Result<Option<u64>, String> {
    match m.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => v
            .as_u64()
            .filter(|n| *n <= max)
            .map(Some)
            .ok_or_else(|| format!("{what}: {key} is a whole number 0..{max}")),
    }
}

fn boolean(m: &Map<String, Value>, key: &str, what: &str) -> Result<Option<bool>, String> {
    match m.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Bool(b)) => Ok(Some(*b)),
        Some(_) => Err(format!("{what}: {key} is true or false")),
    }
}

/// Samples per 1 ms USB frame, for D's limits; `None` at 44.1 kHz.
fn spf(rate: u32) -> Option<u16> {
    match rate {
        48_000 => Some(48),
        96_000 => Some(96),
        _ => None,
    }
}

/// Read and check an installation file (JSON, see the guide).
pub fn parse(text: &str) -> Result<Installation, String> {
    let v: Value =
        serde_json::from_str(text).map_err(|e| format!("not a JSON installation file: {e}"))?;
    let top = fields(
        &v,
        "the installation",
        &[
            "name",
            "description",
            "rate",
            "layout",
            "channels",
            "quantum",
            "mode",
            "d",
            "keepAligned",
            "units",
        ],
    )?;
    let what = "the installation";
    let mut warnings = Vec::new();

    let name = string(&top, "name", what)?.ok_or("the installation: a name is needed")?;
    word_ok("name", &name, 32)?;
    let description = string(&top, "description", what)?.unwrap_or_else(|| name.clone());
    label_ok("description", &description)?;

    let rate = uint(&top, "rate", what, 192_000)?.unwrap_or(48_000);
    let rate = match rate {
        44_100 | 48_000 | 96_000 => rate as u32,
        r => return Err(format!("rate {r}: 44100, 48000 or 96000")),
    };

    let positions: Vec<String> = match (top.get("layout"), top.get("channels")) {
        (Some(_), Some(_)) => return Err("give a layout or channels, not both".into()),
        (None, None) => return Err("a layout (e.g. \"5.1\") or a channels list is needed".into()),
        (Some(l), None) => {
            let l = l.as_str().ok_or("layout is a string, e.g. \"7.1\"")?;
            let (_, chans) = LAYOUTS.iter().find(|(n, _)| *n == l).ok_or_else(|| {
                let names: Vec<_> = LAYOUTS.iter().map(|(n, _)| *n).collect();
                format!("layout {l:?}: one of {}", names.join(", "))
            })?;
            chans.iter().map(|c| (*c).to_string()).collect()
        }
        (None, Some(c)) => {
            let list = c
                .as_array()
                .ok_or("channels is a list of channel names, e.g. [\"FL\", \"FR\"]")?;
            let mut out = Vec::new();
            for p in list {
                let p = p.as_str().ok_or("channels: names, e.g. \"FL\"")?;
                if !is_position(p) {
                    return Err(format!(
                        "channel {p:?}: not a PipeWire channel name (FL, FR, FC, LFE, RL, RR, SL, SR, ..., AUX0..AUX63)"
                    ));
                }
                out.push(p.to_string());
            }
            out
        }
    };
    if positions.is_empty() || positions.len() > 64 {
        return Err("the programme has 1..64 channels".into());
    }
    let mut seen = BTreeSet::new();
    if let Some(p) = positions.iter().find(|p| !seen.insert(p.as_str())) {
        return Err(format!("channel {p} is listed twice"));
    }

    let default_q = if rate == 96_000 { 512 } else { 256 };
    let quantum = uint(&top, "quantum", what, 8192)?.unwrap_or(default_q) as u32;
    if !(32..=8192).contains(&quantum) || !quantum.is_power_of_two() {
        return Err(format!(
            "quantum {quantum}: a power of two, 32..8192 (samples per graph cycle)"
        ));
    }

    let mode = match string(&top, "mode", what)?.as_deref() {
        None | Some("phase_lock") => Mode::PhaseLock,
        Some("rate_lock") => Mode::RateLock,
        Some(m) => return Err(format!("mode {m:?}: phase_lock or rate_lock")),
    };
    let d_given = uint(&top, "d", what, u64::from(u16::MAX))?;
    let d = match (mode, spf(rate)) {
        (Mode::PhaseLock, None) => {
            return Err(
                "44.1 kHz cannot phase-lock (no whole number of samples per USB frame): \
                 use 48000, or mode rate_lock (not sample-aligned)"
                    .into(),
            );
        }
        (Mode::PhaseLock, Some(spf)) => {
            let (lo, hi) = d_limits(spf);
            let d = match d_given {
                Some(d) => d as u16,
                None if (lo..=hi).contains(&DEFAULT_D) => DEFAULT_D,
                None => lo,
            };
            if !(lo..=hi).contains(&d) {
                return Err(format!("d {d}: {lo}..{hi} samples at {rate} Hz"));
            }
            Some(d)
        }
        (Mode::RateLock, _) => {
            if d_given.is_some() {
                return Err("d is for mode phase_lock".into());
            }
            warnings.push(
                "mode rate_lock: the units share the computer's rate but not its phase; \
                 their speakers are not sample-aligned"
                    .to_string(),
            );
            None
        }
    };
    let keep_aligned = boolean(&top, "keepAligned", what)?.unwrap_or(true);

    let list = top
        .get("units")
        .and_then(Value::as_array)
        .ok_or("units: a list of units is needed")?;
    if list.is_empty() || list.len() > MAX_UNITS {
        return Err(format!("units: 1..{MAX_UNITS}"));
    }
    let mut units: Vec<Unit> = Vec::new();
    for (i, u) in list.iter().enumerate() {
        let unit = parse_unit(u, i, rate)?;
        if unit.id == name || units.iter().any(|x| x.id == unit.id) {
            return Err(format!(
                "unit id {:?} is used twice (or is the installation's name)",
                unit.id
            ));
        }
        if units.iter().any(|x| x.serial == unit.serial) {
            return Err(format!("serial {} is used twice", unit.serial));
        }
        if let Some(c) = unit
            .channels
            .iter()
            .find(|c| !positions.contains(&c.channel))
        {
            return Err(format!(
                "unit {}: channel {} is not in the programme ({})",
                unit.id,
                c.channel,
                positions.join(" ")
            ));
        }
        units.push(unit);
    }
    for p in &positions {
        let n = units
            .iter()
            .flat_map(|u| &u.channels)
            .filter(|c| &c.channel == p)
            .count();
        if n == 0 {
            warnings.push(format!("channel {p} goes to no unit: it is not heard"));
        }
    }
    if units.iter().any(|u| u.chip == Chip::Rp2040) && mode == Mode::PhaseLock {
        warnings.push(
            "RP2040 units take their SOF time from an interrupt (1-4 us), not a timestamp: \
             a few microseconds of skew against RP2350 units"
                .to_string(),
        );
    }

    Ok(Installation {
        name,
        description,
        rate,
        positions,
        quantum,
        mode,
        d,
        keep_aligned,
        units,
        warnings,
    })
}

fn parse_unit(v: &Value, i: usize, rate: u32) -> Result<Unit, String> {
    let what = format!("unit {}", i.saturating_add(1));
    let m = fields(
        v,
        &what,
        &["id", "name", "serial", "chip", "onLockError", "channels"],
    )?;
    let id = string(&m, "id", &what)?.ok_or_else(|| format!("{what}: an id is needed"))?;
    word_ok("unit id", &id, 32)?;
    let what = format!("unit {id}");
    let name = string(&m, "name", &what)?.unwrap_or_else(|| id.clone());
    label_ok(&format!("{what}: name"), &name)?;
    let serial = string(&m, "serial", &what)?
        .ok_or_else(|| format!("{what}: its USB serial is needed (uconsole list)"))?;
    serial_ok(&serial)?;
    let chip = match string(&m, "chip", &what)?.as_deref() {
        None | Some("rp2350") => Chip::Rp2350,
        Some("rp2040") => Chip::Rp2040,
        Some(c) => return Err(format!("{what}: chip {c:?}: rp2350 or rp2040")),
    };
    let play_unaligned = match string(&m, "onLockError", &what)?.as_deref() {
        None | Some("mute") => false,
        Some("play_unaligned") => true,
        Some(a) => return Err(format!("{what}: onLockError {a:?}: mute or play_unaligned")),
    };
    let list = m
        .get("channels")
        .and_then(Value::as_array)
        .ok_or_else(|| format!("{what}: channels, a list of {{channel, output}}"))?;
    let format = pick_format(chip, rate, list.len()).map_err(|e| format!("{what}: {e}"))?;

    let mut channels: Vec<Channel> = Vec::new();
    for (k, c) in list.iter().enumerate() {
        let cw = format!("{what}, channel {}", k.saturating_add(1));
        let cm = fields(c, &cw, &["channel", "input", "output", "distanceM"])?;
        let channel =
            string(&cm, "channel", &cw)?.ok_or_else(|| format!("{cw}: a channel name"))?;
        let input = match uint(&cm, "input", &cw, 255)? {
            Some(n) => n as u8,
            None => u8::try_from(k).map_err(|_| format!("{cw}: too many"))?,
        };
        if input >= format.channels {
            return Err(format!(
                "{cw}: input {input}: its USB stream has inputs 0..{}",
                format.channels.saturating_sub(1)
            ));
        }
        let output = uint(&cm, "output", &cw, 255)?
            .ok_or_else(|| format!("{cw}: an output is needed"))? as u8;
        if output >= chip.outputs() {
            return Err(format!(
                "{cw}: output {output}: an {} unit has outputs 0..{}",
                chip.as_str().to_uppercase(),
                chip.outputs().saturating_sub(1)
            ));
        }
        let distance_m = match cm.get("distanceM") {
            None | Some(Value::Null) => 0.0,
            Some(d) => d
                .as_f64()
                .filter(|d| (0.0..=100.0).contains(d))
                .ok_or_else(|| format!("{cw}: distanceM is 0..100 metres"))?
                as f32,
        };
        if channels.iter().any(|x| x.input == input) {
            return Err(format!("{cw}: input {input} is used twice"));
        }
        if channels.iter().any(|x| x.output == output) {
            return Err(format!("{cw}: output {output} is used twice"));
        }
        if channels.iter().any(|x| x.channel == channel) {
            return Err(format!("{cw}: channel {channel} is listed twice"));
        }
        channels.push(Channel {
            channel,
            input,
            output,
            distance_m,
        });
    }
    Ok(Unit {
        id,
        name,
        serial,
        chip,
        play_unaligned,
        channels,
        format,
    })
}

// ------------------------------------------------------------ the files

const HEADER: &str = "Written by `uconsole usb-direct generate` (WP AUSB3)";

fn node_base(inst: &Installation) -> String {
    format!("dspi-usb-direct.{}", inst.name)
}

/// The unit's ALSA nodes and card in PipeWire carry its USB serial in their
/// names (`alsa_output.usb-<vendor>_<product>_<serial>-00.pro-output-0`).
fn node_match(u: &Unit) -> String {
    format!("~alsa_output.usb-.*_{}-00.*", u.serial)
}

fn card_match(u: &Unit) -> String {
    format!("~alsa_card.usb-.*_{}-00", u.serial)
}

fn aux_list(n: u8) -> String {
    (0..n)
        .map(|i| format!("AUX{i}"))
        .collect::<Vec<_>>()
        .join(" ")
}

/// The unit's channels by USB input.
fn by_input(u: &Unit) -> Vec<&Channel> {
    let mut v: Vec<&Channel> = u.channels.iter().collect();
    v.sort_by_key(|c| c.input);
    v
}

/// D as milliseconds, for people.
fn d_ms(inst: &Installation, d: u16) -> f64 {
    f64::from(d) * 1000.0 / f64::from(inst.rate)
}

fn khz(rate: u32) -> String {
    match rate {
        44_100 => "44.1 kHz".into(),
        r => format!("{} kHz", r.checked_div(1000).unwrap_or(0)),
    }
}

/// `pipewire/dspi-usb-direct-<name>.conf`.
pub fn pipewire_conf(inst: &Installation) -> String {
    let base = node_base(inst);
    let mut s = String::new();
    let _ = writeln!(
        s,
        "# {HEADER}: installation {:?}.\n\
         # Copy to ~/.config/pipewire/pipewire.conf.d/ (with the WirePlumber file next to it)\n\
         # and restart PipeWire. Guide: docs/integrator/usb-direct-home-theater.md\n\
         #\n\
         # One sink, \"{}\", with the programme's channels. combine-stream splits it\n\
         # into one stream per DSPi unit; each stream's channels go to the unit's USB\n\
         # inputs AUX0.. as its settings sheet (SETTINGS.md) lists them.\n\
         #\n\
         # The whole graph runs at {} Hz with a fixed quantum of {} samples ({:.2} ms),\n\
         # so nothing between this sink and the units is resampled. A player at\n\
         # another rate is resampled once, into this sink.",
        inst.name,
        inst.description,
        inst.rate,
        inst.quantum,
        f64::from(inst.quantum) * 1000.0 / f64::from(inst.rate)
    );
    let _ = writeln!(
        s,
        "context.properties = {{\n    \
         default.clock.rate          = {r}\n    \
         default.clock.allowed-rates = [ {r} ]\n    \
         default.clock.quantum       = {q}\n    \
         default.clock.min-quantum   = {q}\n    \
         default.clock.max-quantum   = {q}\n\
         }}\n",
        r = inst.rate,
        q = inst.quantum
    );
    let _ = writeln!(
        s,
        "context.modules = [\n    \
         {{   name = libpipewire-module-combine-stream\n        \
         args = {{\n            \
         combine.mode = sink\n            \
         node.name = \"{base}\"\n            \
         node.description = \"{}\"\n            \
         # The units are aligned by their own D and by the hub, not by PipeWire.\n            \
         combine.latency-compensate = false\n            \
         combine.props = {{\n                \
         audio.position = [ {} ]\n                \
         audio.rate = {}\n                \
         node.force-quantum = {}\n                \
         node.lock-quantum = true\n                \
         node.lock-rate = true\n            \
         }}\n            \
         stream.props = {{\n                \
         resample.disable = true\n                \
         node.lock-quantum = true\n                \
         node.lock-rate = true\n            \
         }}\n            \
         stream.rules = [",
        inst.description,
        inst.positions.join(" "),
        inst.rate,
        inst.quantum
    );
    for u in &inst.units {
        let chans = by_input(u);
        let from: Vec<&str> = chans.iter().map(|c| c.channel.as_str()).collect();
        let to: Vec<String> = chans.iter().map(|c| format!("AUX{}", c.input)).collect();
        let _ = writeln!(
            s,
            "                # {} ({}): serial {}\n                \
             {{   matches = [ {{ media.class = \"Audio/Sink\" node.name = \"{}\" }} ]\n                    \
             actions = {{ create-stream = {{\n                        \
             node.name = \"{base}.{}\"\n                        \
             combine.audio.position = [ {} ]\n                        \
             audio.position = [ {} ]\n                    \
             }} }}\n                \
             }}",
            u.id,
            u.name,
            u.serial,
            node_match(u),
            u.id,
            from.join(" "),
            to.join(" ")
        );
    }
    let _ = writeln!(s, "            ]\n        }}\n    }}\n]");
    s
}

/// `wireplumber/dspi-usb-direct-<name>.conf`.
pub fn wireplumber_conf(inst: &Installation) -> String {
    let base = node_base(inst);
    let mut s = String::new();
    let _ = writeln!(
        s,
        "# {HEADER}: installation {:?}.\n\
         # Copy to ~/.config/wireplumber/wireplumber.conf.d/ (WirePlumber 0.5) and restart\n\
         # WirePlumber. Guide: docs/integrator/usb-direct-home-theater.md\n\
         #\n\
         # Each unit: the pro-audio profile, its sink opened in exactly the USB format\n\
         # below, one clock name and one group for all units (they all run on the\n\
         # computer's USB frame clock, so no rate matching and no resampling), and\n\
         # never suspended: a stream that closes and opens again moves its start\n\
         # offset, which the hub then has to align again.\n\
         monitor.alsa.rules = [",
        inst.name
    );
    for (i, u) in inst.units.iter().enumerate() {
        let driver = if i == 0 { 2100 } else { 2000 };
        let _ = writeln!(
            s,
            "    # {} ({}): serial {}, {} channels, {} bit, {} Hz\n    \
             {{   matches = [ {{ device.name = \"{}\" }} ]\n        \
             actions = {{ update-props = {{\n            \
             device.profile = \"pro-audio\"\n        \
             }} }}\n    \
             }}\n    \
             {{   matches = [ {{ node.name = \"{}\" }} ]\n        \
             actions = {{ update-props = {{\n            \
             node.description = \"{} (DSPi USB-direct)\"\n            \
             audio.format = \"{}\"\n            \
             audio.rate = {}\n            \
             audio.channels = {}\n            \
             audio.position = [ {} ]\n            \
             resample.disable = true\n            \
             clock.name = \"{base}\"\n            \
             node.group = \"{base}\"\n            \
             priority.driver = {driver}\n            \
             session.suspend-timeout-seconds = 0\n            \
             node.pause-on-idle = false\n        \
             }} }}\n    \
             }}",
            u.id,
            u.name,
            u.serial,
            u.format.channels,
            u.format.bits,
            inst.rate,
            card_match(u),
            node_match(u),
            u.name,
            u.format.pipewire(),
            inst.rate,
            u.format.channels,
            aux_list(u.format.channels)
        );
    }
    let _ = writeln!(s, "]");
    s
}

/// `units/<id>.dspi`: input source and crosspoints. Every input of each used
/// output is written, so the unit's earlier routing cannot add to it.
pub fn unit_script(inst: &Installation, u: &Unit) -> String {
    let mut s = String::new();
    let _ = writeln!(
        s,
        "# {HEADER}: installation {:?}, unit {} ({}).\n\
         # Run it on this unit only:\n\
         #   {COMMAND} --device {} -f {}.dspi\n\
         # then save the unit's preset (Ctrl-S in the interface, or preset.save <slot>).\n\
         # Mode, D and the lock-error action are not here: the hub writes them (SETTINGS.md).\n\
         in.source usb",
        inst.name, u.id, u.name, u.serial, u.id
    );
    let mut outs: Vec<&Channel> = u.channels.iter().collect();
    outs.sort_by_key(|c| c.output);
    for c in outs {
        let _ = writeln!(
            s,
            "# output {}: {} (USB input {})",
            c.output, c.channel, c.input
        );
        for i in 0..u.chip.inputs() {
            if i == c.input {
                let _ = writeln!(s, "mix {i} {} on 0", c.output);
            } else {
                let _ = writeln!(s, "mix {i} {} off", c.output);
            }
        }
    }
    s
}

/// The hub's system model for this installation, checked by the hub's own
/// validation.
pub fn hub_model(inst: &Installation) -> Result<Value, String> {
    let units: Vec<Value> = inst
        .units
        .iter()
        .map(|u| {
            let speakers: Vec<Value> = u
                .channels
                .iter()
                .map(|c| json!({"output": c.output, "distanceM": c.distance_m}))
                .collect();
            json!({
                "id": u.id,
                "name": u.name,
                "serial": u.serial,
                "room": inst.name,
                "transport": "usb_direct",
                "usbMode": inst.mode.as_str(),
                "onLockError": if u.play_unaligned { "play_unaligned" } else { "mute" },
                "speakers": speakers,
            })
        })
        .collect();
    let v = json!({
        "version": dspi_hub::system::model::MODEL_VERSION,
        "dFrames": inst.d,
        "rateHz": inst.rate,
        "rooms": [{"id": inst.name, "name": inst.description}],
        "units": units,
        "keepUsbAligned": inst.keep_aligned,
    });
    let model: SystemModel =
        serde_json::from_value(v).map_err(|e| format!("the hub's model: {e}"))?;
    model
        .validate()
        .map_err(|e| format!("the hub's model: {e}"))?;
    serde_json::to_value(&model).map_err(|e| format!("the hub's model: {e}"))
}

/// `SETTINGS.md`: what each unit is set to, and the steps.
pub fn settings_sheet(inst: &Installation) -> String {
    let mut s = String::new();
    let _ = writeln!(
        s,
        "# USB-direct installation: {}\n\n\
         {HEADER} for the installation `{}`. Nothing in this folder has been\n\
         written to a unit. The guide is `docs/integrator/usb-direct-home-theater.md`.\n",
        inst.description, inst.name
    );
    let _ = writeln!(
        s,
        "- Programme: {} channels ({}) at {}, PipeWire sink `{}` (\"{}\").",
        inst.positions.len(),
        inst.positions.join(" "),
        khz(inst.rate),
        node_base(inst),
        inst.description
    );
    let _ = writeln!(
        s,
        "- Quantum: {} samples ({:.2} ms), fixed.",
        inst.quantum,
        f64::from(inst.quantum) * 1000.0 / f64::from(inst.rate)
    );
    match (inst.mode, inst.d) {
        (Mode::PhaseLock, Some(d)) => {
            let (lo, hi) = spf(inst.rate).map(d_limits).unwrap_or((0, 0));
            let _ = writeln!(
                s,
                "- Mode: 2, phase lock (sample-aligned). D = {d} samples ({:.2} ms) on every unit; \
                 the units take {lo}..{hi} at {}.",
                d_ms(inst, d),
                khz(inst.rate)
            );
        }
        _ => {
            let _ = writeln!(s, "- Mode: 1, rate lock only (not sample-aligned).");
        }
    }
    let _ = writeln!(
        s,
        "- Keep USB-direct units aligned: {}.\n",
        if inst.keep_aligned { "on" } else { "off" }
    );

    let _ = writeln!(
        s,
        "## Units\n\n\
         | Unit | Serial | USB stream | Mode | D | On lock error |\n\
         |---|---|---|---|---|---|"
    );
    for u in &inst.units {
        let d = inst
            .d
            .map(|d| format!("{d} ({:.2} ms)", d_ms(inst, d)))
            .unwrap_or_else(|| "-".into());
        let _ = writeln!(
            s,
            "| {} ({}) | `{}` | {} channels, {} bit, {} ({}) | {} | {} | {} |",
            u.name,
            u.id,
            u.serial,
            u.format.channels,
            u.format.bits,
            khz(inst.rate),
            u.chip.as_str().to_uppercase(),
            match inst.mode {
                Mode::PhaseLock => "2 phase lock",
                Mode::RateLock => "1 rate lock",
            },
            d,
            if u.play_unaligned {
                "play unaligned"
            } else {
                "mute"
            }
        );
    }
    let _ = writeln!(s);
    for u in &inst.units {
        let _ = writeln!(
            s,
            "### {} ({})\n\n\
             | USB input | Channel | Output | Distance |\n\
             |---|---|---|---|",
            u.name, u.id
        );
        for c in by_input(u) {
            let out = if c.output == 8 || (u.chip == Chip::Rp2040 && c.output == 4) {
                format!("{} (PDM sub)", c.output)
            } else {
                let pair = (c.output >> 1).saturating_add(1);
                let side = if c.output & 1 == 0 { "L" } else { "R" };
                format!("{} (pair {pair} {side})", c.output)
            };
            let _ = writeln!(
                s,
                "| {} | {} | {} | {:.2} m |",
                c.input, c.channel, out, c.distance_m
            );
        }
        let unused = u.format.channels.saturating_sub(u.channels.len() as u8);
        if unused > 0 {
            let _ = writeln!(
                s,
                "\nThe stream has {} channels; {} of them carry silence (the format has no {}-channel form).",
                u.format.channels,
                unused,
                u.channels.len()
            );
        }
        let _ = writeln!(s);
    }

    let _ = writeln!(
        s,
        "## Steps\n\n\
         1. **Cables and hubs** as the guide says: a multi-TT hub, active USB 2.0 cables,\n   \
            isolators at the unit end, every unit on one USB controller of this computer.\n\
         2. **PipeWire.** Copy `pipewire/dspi-usb-direct-{n}.conf` to\n   \
            `~/.config/pipewire/pipewire.conf.d/` and `wireplumber/dspi-usb-direct-{n}.conf`\n   \
            to `~/.config/wireplumber/wireplumber.conf.d/`, then\n   \
            `systemctl --user restart wireplumber pipewire pipewire-pulse`.\n   \
            `wpctl status` lists \"{d}\" and one \"... (DSPi USB-direct)\" sink per unit;\n   \
            `pw-top` shows every unit at {r} Hz and quantum {q}. Make \"{d}\" the default sink.\n\
         3. **Routing.** For each unit: `{c} --device <serial> -f units/<id>.dspi`, then save\n   \
            its preset. It sets the input source to USB and the crosspoints above.\n\
         4. **Mode, D and the lock-error action** are written by the hub (Installer, \"USB-direct\n   \
            units\"; Admin): add the units as USB-direct, mode phase lock, system delay D as above,\n   \
            \"When it can't lock\" as above. Or give the hub `hub-system-model.json` (`system.set`,\n   \
            `docs/integrator/audio-link.md`), then **Align**. The hub also equalises the units'\n   \
            stream start offsets with the outputs' delays.\n\
         5. **Check** each unit: `{c} --device <serial> ext usb-direct` says phase-locked, a\n   \
            phase error under half a sample, no growing gap or late counts. The hub's summary\n   \
            says \"USB-direct N of N locked\".\n",
        n = inst.name,
        d = inst.description,
        r = inst.rate,
        q = inst.quantum,
        c = COMMAND
    );
    if !inst.warnings.is_empty() {
        let _ = writeln!(s, "## Warnings\n");
        for w in &inst.warnings {
            let _ = writeln!(s, "- {w}");
        }
    }
    s
}

/// Every file, by its path under the output folder.
pub fn files(inst: &Installation) -> Result<Vec<(String, String)>, String> {
    let model = hub_model(inst)?;
    let model_text =
        serde_json::to_string_pretty(&model).map_err(|e| format!("the hub's model: {e}"))?;
    let mut out = vec![
        (
            format!("pipewire/dspi-usb-direct-{}.conf", inst.name),
            pipewire_conf(inst),
        ),
        (
            format!("wireplumber/dspi-usb-direct-{}.conf", inst.name),
            wireplumber_conf(inst),
        ),
        (
            "hub-system-model.json".to_string(),
            format!("{model_text}\n"),
        ),
        ("SETTINGS.md".to_string(), settings_sheet(inst)),
    ];
    for u in &inst.units {
        out.push((format!("units/{}.dspi", u.id), unit_script(inst, u)));
    }
    Ok(out)
}

/// Write the files under `dir`; an existing file is replaced only with `force`.
pub fn write_all(dir: &Path, files: &[(String, String)], force: bool) -> Result<(), String> {
    if !force && let Some((p, _)) = files.iter().find(|(p, _)| dir.join(p).exists()) {
        return Err(format!(
            "{} exists (--force replaces it)",
            dir.join(p).display()
        ));
    }
    for (p, text) in files {
        let path = dir.join(p);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("could not create {}: {e}", parent.display()))?;
        }
        std::fs::write(&path, text)
            .map_err(|e| format!("could not write {}: {e}", path.display()))?;
    }
    Ok(())
}

fn usage_line() -> String {
    format!(
        "usage: {COMMAND} usb-direct generate <installation.json> [--out <folder>] [--force] [--json]"
    )
}

/// `uconsole usb-direct ...`.
pub fn run(flags: &[&str], json: bool) -> u8 {
    let mut pos: Vec<&str> = Vec::new();
    let mut out: Option<PathBuf> = None;
    let mut it = flags.iter().skip(1);
    while let Some(f) = it.next() {
        match *f {
            "--out" => match it.next() {
                Some(d) => out = Some(PathBuf::from(d)),
                None => {
                    eprintln!("{COMMAND}: --out needs a folder");
                    return exit::USAGE;
                }
            },
            "--json" | "--force" | "--quiet" | "--dry-run" => {}
            "--help" | "-h" => {
                println!("{}", usage_line());
                return exit::OK;
            }
            other if other.starts_with("--") => {
                eprintln!(
                    "{COMMAND}: usb-direct: unknown option {other}\n{}",
                    usage_line()
                );
                return exit::USAGE;
            }
            other => pos.push(other),
        }
    }
    let force = flags.contains(&"--force");
    let dry = flags.contains(&"--dry-run");
    let quiet = flags.contains(&"--quiet");
    let spec = match pos.as_slice() {
        ["generate", spec] => *spec,
        _ => {
            eprintln!("{COMMAND}: {}", usage_line());
            return exit::USAGE;
        }
    };
    let text = match std::fs::read_to_string(spec) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("{COMMAND}: could not read {spec}: {e}");
            return exit::USAGE;
        }
    };
    let inst = match parse(&text) {
        Ok(i) => i,
        Err(e) => {
            eprintln!("{COMMAND}: {spec}: {e}");
            return exit::USAGE;
        }
    };
    let all = match files(&inst) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("{COMMAND}: {spec}: {e}");
            return exit::USAGE;
        }
    };
    let dir = out.unwrap_or_else(|| PathBuf::from(format!("usb-direct-{}", inst.name)));
    if !dry && let Err(e) = write_all(&dir, &all, force) {
        eprintln!("{COMMAND}: {e}");
        return exit::USAGE;
    }
    if json {
        let units: Vec<Value> = inst
            .units
            .iter()
            .map(|u| {
                json!({
                    "id": u.id,
                    "serial": u.serial,
                    "channels": u.format.channels,
                    "bits": u.format.bits,
                    "onLockError": if u.play_unaligned { "play_unaligned" } else { "mute" },
                })
            })
            .collect();
        let doc = json!({
            "folder": dir.display().to_string(),
            "written": !dry,
            "files": all.iter().map(|(p, _)| p.as_str()).collect::<Vec<_>>(),
            "rate": inst.rate,
            "quantum": inst.quantum,
            "mode": inst.mode.as_str(),
            "d": inst.d,
            "units": units,
            "warnings": inst.warnings,
        });
        println!("{doc:#}");
    } else if !quiet {
        let verb = if dry { "Would write" } else { "Wrote" };
        println!("{verb} {} files in {}:", all.len(), dir.display());
        for (p, _) in &all {
            println!("  {p}");
        }
        for w in &inst.warnings {
            println!("warning: {w}");
        }
        println!("Next: {}", dir.join("SETTINGS.md").display());
    }
    exit::OK
}

#[cfg(test)]
mod tests_wp_ausb3 {
    use super::*;

    const CINEMA: &str = include_str!("../tests/fixtures/ausb3/cinema-7.1.json");
    const DEN: &str = include_str!("../tests/fixtures/ausb3/den-5.1.json");
    const HIFI96: &str = include_str!("../tests/fixtures/ausb3/hifi-96k.json");

    fn golden_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/ausb3/cinema-7.1.expected")
    }

    /// Every opened brace and bracket closes, outside strings and comments
    /// (SPA JSON's shape; PipeWire is not in the build image).
    fn balanced(text: &str) -> bool {
        let mut stack = Vec::new();
        for line in text.lines() {
            let mut quoted = false;
            for c in line.chars() {
                match c {
                    '"' => quoted = !quoted,
                    '#' if !quoted => break,
                    '{' | '[' if !quoted => stack.push(c),
                    '}' | ']' if !quoted => {
                        let open = if c == '}' { '{' } else { '[' };
                        if stack.pop() != Some(open) {
                            return false;
                        }
                    }
                    _ => {}
                }
            }
            if quoted {
                return false;
            }
        }
        stack.is_empty()
    }

    #[test]
    fn wp_ausb3_cinema_fixture_parses_into_three_units() {
        let inst = parse(CINEMA).unwrap();
        assert_eq!(inst.rate, 48_000);
        assert_eq!(inst.positions.len(), 8);
        assert_eq!(inst.d, Some(192));
        assert_eq!(inst.quantum, 256);
        assert_eq!(inst.units.len(), 3);
        // FL FR FC LFE: 4 channels, 24 bit; the sides and rears: stereo 24 bit.
        assert_eq!(
            inst.units[0].format,
            Format {
                channels: 4,
                bits: 24
            }
        );
        assert_eq!(
            inst.units[1].format,
            Format {
                channels: 2,
                bits: 24
            }
        );
        assert!(inst.units[2].play_unaligned);
        assert!(inst.warnings.is_empty(), "{:?}", inst.warnings);
    }

    #[test]
    fn wp_ausb3_pipewire_splits_the_sink_per_unit_without_resampling() {
        let inst = parse(CINEMA).unwrap();
        let pw = pipewire_conf(&inst);
        assert!(balanced(&pw), "{pw}");
        assert!(pw.contains("name = libpipewire-module-combine-stream"));
        assert!(pw.contains("audio.position = [ FL FR FC LFE RL RR SL SR ]"));
        assert!(pw.contains("default.clock.allowed-rates = [ 48000 ]"));
        assert!(pw.contains("default.clock.min-quantum   = 256"));
        assert!(pw.contains("default.clock.max-quantum   = 256"));
        assert!(pw.contains("node.force-quantum = 256"));
        assert!(pw.contains("resample.disable = true"));
        assert!(pw.contains("combine.latency-compensate = false"));
        // One stream per unit, matched by its serial, channels onto AUX inputs.
        assert_eq!(pw.matches("create-stream").count(), 3);
        assert!(pw.contains("node.name = \"~alsa_output.usb-.*_E6614C311B4A8B2F-00.*\""));
        assert!(pw.contains("combine.audio.position = [ FL FR FC LFE ]"));
        assert!(pw.contains("audio.position = [ AUX0 AUX1 AUX2 AUX3 ]"));
        assert!(pw.contains("combine.audio.position = [ SL SR ]"));
    }

    #[test]
    fn wp_ausb3_wireplumber_pins_each_units_format_and_clock() {
        let inst = parse(CINEMA).unwrap();
        let wp = wireplumber_conf(&inst);
        assert!(balanced(&wp), "{wp}");
        assert_eq!(wp.matches("device.profile = \"pro-audio\"").count(), 3);
        assert!(wp.contains("audio.format = \"S24LE\""));
        assert!(wp.contains("audio.channels = 4"));
        assert_eq!(
            wp.matches("clock.name = \"dspi-usb-direct.cinema\"")
                .count(),
            3
        );
        assert_eq!(wp.matches("session.suspend-timeout-seconds = 0").count(), 3);
        // Exactly one unit is preferred as the driver.
        assert_eq!(wp.matches("priority.driver = 2100").count(), 1);
    }

    #[test]
    fn wp_ausb3_hub_model_is_one_the_hub_takes() {
        for spec in [CINEMA, DEN, HIFI96] {
            let inst = parse(spec).unwrap();
            let v = hub_model(&inst).unwrap();
            let m: SystemModel = serde_json::from_value(v.clone()).unwrap();
            m.validate().unwrap();
            assert_eq!(m.units.len(), inst.units.len());
            assert!(m.units.iter().all(|u| u.usb_direct()));
            assert_eq!(m.d_frames, inst.d);
            assert_eq!(m.rate_hz, inst.rate);
        }
        let v = hub_model(&parse(CINEMA).unwrap()).unwrap();
        assert_eq!(v["units"][2]["onLockError"], "play_unaligned");
        assert_eq!(v["units"][0]["usbMode"], "phase_lock");
        assert_eq!(v["keepUsbAligned"], true);
    }

    #[test]
    fn wp_ausb3_unit_scripts_are_terminal_commands() {
        let ctx = dspi_cmd::Context {
            channel_slugs: Vec::new(),
            num_inputs: 8,
            num_outputs: 9,
            max_bands: 10,
        };
        for spec in [CINEMA, DEN, HIFI96] {
            let inst = parse(spec).unwrap();
            for u in &inst.units {
                let script = unit_script(&inst, u);
                let mut n = 0;
                for line in crate::script_lines(&script) {
                    let toks = dspi_cmd::tokenize(&line);
                    let refs: Vec<&str> = toks.iter().map(String::as_str).collect();
                    dspi_cmd::parse(&refs, &ctx)
                        .unwrap_or_else(|e| panic!("{}: {line}: {e}", u.id));
                    n += 1;
                }
                // in.source + every input for every used output.
                let want = 1 + u.channels.len() * usize::from(u.chip.inputs());
                assert_eq!(n, want, "{}", u.id);
            }
        }
        let inst = parse(CINEMA).unwrap();
        let s = unit_script(&inst, &inst.units[0]);
        assert!(s.contains("mix 3 8 on 0"), "{s}"); // LFE on USB input 3 to the PDM sub
        assert!(s.contains("mix 0 8 off"));
    }

    #[test]
    fn wp_ausb3_formats_follow_the_firmware() {
        let f = |c, r, n| pick_format(c, r, n);
        assert_eq!(
            f(Chip::Rp2350, 48_000, 1).unwrap(),
            Format {
                channels: 2,
                bits: 24
            }
        );
        assert_eq!(
            f(Chip::Rp2350, 48_000, 3).unwrap(),
            Format {
                channels: 4,
                bits: 24
            }
        );
        assert_eq!(
            f(Chip::Rp2350, 48_000, 5).unwrap(),
            Format {
                channels: 6,
                bits: 16
            }
        );
        assert_eq!(
            f(Chip::Rp2350, 48_000, 8).unwrap(),
            Format {
                channels: 8,
                bits: 16
            }
        );
        assert!(f(Chip::Rp2350, 48_000, 9).is_err());
        assert!(f(Chip::Rp2350, 96_000, 3).is_err());
        assert_eq!(
            f(Chip::Rp2350, 96_000, 2).unwrap(),
            Format {
                channels: 2,
                bits: 24
            }
        );
        assert!(f(Chip::Rp2040, 48_000, 3).is_err());
    }

    #[test]
    fn wp_ausb3_96k_takes_d_from_its_own_range() {
        let inst = parse(HIFI96).unwrap();
        assert_eq!(inst.rate, 96_000);
        assert_eq!(inst.d, Some(324)); // d_min at 96 kHz; 192 is out of range there
        assert_eq!(inst.quantum, 512);
    }

    #[test]
    fn wp_ausb3_bad_installations_are_refused_with_the_reason() {
        let base: Value = serde_json::from_str(DEN).unwrap();
        let with = |f: &dyn Fn(&mut Value)| {
            let mut v = base.clone();
            f(&mut v);
            parse(&v.to_string())
        };
        type Edit = Box<dyn Fn(&mut Value)>;
        let cases: Vec<(&str, Edit)> = vec![
            (
                "44.1 kHz cannot phase-lock",
                Box::new(|v| v["rate"] = json!(44_100)),
            ),
            ("d 150: 170..399", Box::new(|v| v["d"] = json!(150))),
            ("d 400: 170..399", Box::new(|v| v["d"] = json!(400))),
            (
                "serial",
                Box::new(|v| v["units"][0]["serial"] = json!("E66\"; x")),
            ),
            (
                "is used twice",
                Box::new(|v| v["units"][1]["serial"] = v["units"][0]["serial"].clone()),
            ),
            (
                "not in the programme",
                Box::new(|v| v["units"][0]["channels"][0]["channel"] = json!("TFL")),
            ),
            (
                "output 9",
                Box::new(|v| v["units"][0]["channels"][0]["output"] = json!(9)),
            ),
            (
                "output 0 is used twice",
                Box::new(|v| v["units"][0]["channels"][1]["output"] = json!(0)),
            ),
            (
                "unknown field",
                Box::new(|v| v["units"][0]["colour"] = json!("red")),
            ),
            (
                "onLockError",
                Box::new(|v| v["units"][0]["onLockError"] = json!("shout")),
            ),
            ("quantum 300", Box::new(|v| v["quantum"] = json!(300))),
            (
                "an RP2040 unit takes 2",
                Box::new(|v| v["units"][0]["chip"] = json!("rp2040")),
            ),
            (
                "d is for mode phase_lock",
                Box::new(|v| v["mode"] = json!("rate_lock")),
            ),
            (
                "a layout or channels, not both",
                Box::new(|v| v["channels"] = json!(["FL"])),
            ),
        ];
        for (want, f) in cases {
            let e = with(f.as_ref()).unwrap_err();
            assert!(e.contains(want), "want {want:?}, got {e:?}");
        }
        // Rate lock at 44.1 kHz is allowed, with a warning.
        let ok = with(&|v| {
            v["rate"] = json!(44_100);
            v["mode"] = json!("rate_lock");
            v.as_object_mut().unwrap().remove("d");
            v["units"][0]["channels"] =
                json!([{"channel": "FL", "output": 0}, {"channel": "FR", "output": 1}]);
            v["units"][1]["channels"] =
                json!([{"channel": "RL", "output": 0}, {"channel": "RR", "output": 1}]);
        })
        .unwrap();
        assert_eq!(ok.d, None);
        assert!(ok.warnings.iter().any(|w| w.contains("not sample-aligned")));
        assert!(ok.warnings.iter().any(|w| w.contains("goes to no unit")));
    }

    /// The whole output for the cinema fixture, as committed. Regenerate with
    /// `AUSB3_BLESS=1 cargo test -p dspi-term wp_ausb3_cinema_output_matches_the_golden_files`.
    #[test]
    fn wp_ausb3_cinema_output_matches_the_golden_files() {
        let all = files(&parse(CINEMA).unwrap()).unwrap();
        let dir = golden_dir();
        if std::env::var_os("AUSB3_BLESS").is_some() {
            write_all(&dir, &all, true).unwrap();
        }
        for (p, text) in &all {
            let want = std::fs::read_to_string(dir.join(p))
                .unwrap_or_else(|e| panic!("{p}: {e} (AUSB3_BLESS=1 writes it)"));
            assert_eq!(&want, text, "{p} differs from the golden file");
        }
    }
}
