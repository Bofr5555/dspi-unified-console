//! WP TPL1: the shipped generic and build templates, their generator and
//! their checks.
//!
//! Every template in `assets/templates/` but the K340's is written by
//! [`generate`] from [`SPECS`]: the demo device's settings, laid out for the
//! platform, reset to a clean start (no EQ, every effect off, master volume
//! 0 dB, user volume -20 dB), then the template's routing, crossovers and
//! names, and for a build its hardware I/O. The output is reproducible (fixed
//! `savedUtc` and `appVersion`); [`wp_tpl1_shipped_templates_match_the_generator`]
//! fails when a committed file drifts from it. To rewrite the files, run in
//! the container:
//!
//! ```text
//! DSPI_TPL1_OUT=/src/assets/templates \
//!   cargo test -p dspi-core --lib wp_tpl1_write_templates -- --ignored
//! ```
#![allow(clippy::arithmetic_side_effects)]

use std::sync::Arc;

use dspi_proto::enums::InputSource;
use dspi_proto::xover::{self, Family, Meta};
use dspi_proto::{FilterType, Platform};
use dspi_session::output_label::slot_name;
use dspi_transport::FakeOptions;
use serde_json::Value;

use super::*;
use crate::consts::{IMPORT_HARDWARE, IMPORT_VOLUMES};
use crate::records::{DspState, FilterParams};

// ── What the templates are ────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Plat {
    /// Pico 2: 8 USB inputs, outputs 1-8 on four S/PDIF or I2S slots, 9 PDM.
    Rp2350,
    /// Pico: 2 USB inputs, outputs 1-4 on two S/PDIF or I2S slots, 5 PDM.
    Rp2040,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Topo {
    /// L and R full range on outputs 1-2.
    Stereo,
    /// Mains high-passed at 80 Hz, the PDM output as the sub.
    Sub21,
    /// Mains high-passed at 80 Hz, two subs on outputs 3-4.
    Sub22,
    /// Tweeters on outputs 1-2, woofers on 3-4, 2 kHz.
    TwoWay,
    /// Tweeters 1-2, woofers 3-4, mids 5-6; 300 Hz and 3 kHz.
    ThreeWay,
    /// Stereo on outputs 1-2 with crossfeed on.
    Headphones,
}

/// A hardware build the firmware's README defines (DSPi-bofr `README.md`,
/// "Wiring Guide", "I2S Output", "PCM1808 ADC Module on Pico 2").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Build {
    /// No hardware I/O: the device's own is left alone.
    Generic,
    /// One PCM5102A on output slot 1 (I2S, GPIO 6), the PDM sub on GPIO 10.
    Pcm5102aSub,
    /// Two PCM5102A on output slots 1 and 2 (I2S, GPIO 6 and 7).
    TwoPcm5102a,
    /// A PCM1808 ADC on the I2S input (GPIO 4, MCK GPIO 13 at 256x) and a
    /// PCM5102A on output slot 1.
    Pcm1808Pcm5102a,
}

struct Spec {
    /// The id after `builtin:`, also the file's stem.
    id: &'static str,
    name: &'static str,
    plat: Plat,
    topo: Topo,
    build: Build,
}

const fn spec(id: &'static str, name: &'static str, plat: Plat, topo: Topo, build: Build) -> Spec {
    Spec {
        id,
        name,
        plat,
        topo,
        build,
    }
}

use Build::*;
use Plat::*;
use Topo::*;

/// The templates WP TPL1 ships, in [`TEMPLATES`]' order.
const SPECS: &[Spec] = &[
    spec(
        "stereo-rp2350",
        "Stereo, full range (RP2350)",
        Rp2350,
        Stereo,
        Generic,
    ),
    spec(
        "stereo-sub-rp2350",
        "Stereo + sub (2.1), 80 Hz (RP2350)",
        Rp2350,
        Sub21,
        Generic,
    ),
    spec(
        "stereo-2sub-rp2350",
        "Stereo + 2 subs (2.2), 80 Hz (RP2350)",
        Rp2350,
        Sub22,
        Generic,
    ),
    spec(
        "2way-rp2350",
        "Stereo 2-way active, 2 kHz (RP2350)",
        Rp2350,
        TwoWay,
        Generic,
    ),
    spec(
        "3way-rp2350",
        "Stereo 3-way active, 300 Hz / 3 kHz (RP2350)",
        Rp2350,
        ThreeWay,
        Generic,
    ),
    spec(
        "headphones-rp2350",
        "Headphones with crossfeed (RP2350)",
        Rp2350,
        Headphones,
        Generic,
    ),
    spec(
        "stereo-rp2040",
        "Stereo, full range (RP2040)",
        Rp2040,
        Stereo,
        Generic,
    ),
    spec(
        "stereo-sub-rp2040",
        "Stereo + sub (2.1), 80 Hz (RP2040)",
        Rp2040,
        Sub21,
        Generic,
    ),
    spec(
        "stereo-2sub-rp2040",
        "Stereo + 2 subs (2.2), 80 Hz (RP2040)",
        Rp2040,
        Sub22,
        Generic,
    ),
    spec(
        "2way-rp2040",
        "Stereo 2-way active, 2 kHz (RP2040)",
        Rp2040,
        TwoWay,
        Generic,
    ),
    spec(
        "headphones-rp2040",
        "Headphones with crossfeed (RP2040)",
        Rp2040,
        Headphones,
        Generic,
    ),
    spec(
        "build-pico2-pcm5102a-sub",
        "Build: Pico 2 + PCM5102A DAC + PDM sub (2.1)",
        Rp2350,
        Sub21,
        Pcm5102aSub,
    ),
    spec(
        "build-pico-pcm5102a-sub",
        "Build: Pico + PCM5102A DAC + PDM sub (2.1)",
        Rp2040,
        Sub21,
        Pcm5102aSub,
    ),
    spec(
        "build-pico2-2x-pcm5102a-2way",
        "Build: Pico 2 + 2x PCM5102A, 2-way active",
        Rp2350,
        TwoWay,
        TwoPcm5102a,
    ),
    spec(
        "build-pico-2x-pcm5102a-2way",
        "Build: Pico + 2x PCM5102A, 2-way active",
        Rp2040,
        TwoWay,
        TwoPcm5102a,
    ),
    spec(
        "build-pico2-pcm1808-pcm5102a",
        "Build: Pico 2 + PCM1808 ADC + PCM5102A DAC (analog in)",
        Rp2350,
        Stereo,
        Pcm1808Pcm5102a,
    ),
];

/// Fixed, so the files are reproducible.
const SAVED_UTC: &str = "2026-11-14T00:00:00Z";
const APP_VERSION: &str = "dspi-core shipped template (WP TPL1)";

/// The user volume every template starts at (owner 2026-11-14).
const USER_VOLUME_DB: f32 = -20.0;
/// Each of L and R into a sub: two in-phase full-scale inputs sum to
/// 0.99 of full scale (-6 dB each would sum to +0.02 dB).
const SUB_SUM_DB: f32 = -6.1;
/// The tweeters start this far down: they are the most delicate outputs and
/// usually the most sensitive drivers.
const TWEETER_TRIM_DB: f32 = -6.0;

const SUB_XOVER_HZ: f32 = 80.0;
const SUBSONIC_HZ: f32 = 20.0;
const TWO_WAY_HZ: f32 = 2000.0;
// -- WP TPL1F -- two octaves under the crossover (review F2): 14 degrees
// of phase there, a sum dip of about 0.1 dB (one octave cost about 0.8 dB).
const TWO_WAY_GUARD_HZ: f32 = 500.0;
const THREE_WAY_LOW_HZ: f32 = 300.0;
const THREE_WAY_HIGH_HZ: f32 = 3000.0;
const THREE_WAY_GUARD_HZ: f32 = 750.0;

// ── Building one ──────────────────────────────────────────────────────

fn parse(text: &str) -> Value {
    serde_json::from_str(text).unwrap()
}

fn flat() -> FilterParams {
    FilterParams {
        filter_type: FilterType::Flat.to_raw(),
        bypass: false,
        freq: 1000.0,
        q: 0.707,
        gain: 0.0,
        qp: 0.707,
    }
}

fn xo(family: Family, order: u8, high_pass: bool, freq: f32) -> FilterParams {
    FilterParams {
        filter_type: Meta {
            family,
            order,
            high_pass,
        }
        .to_type(),
        freq,
        ..flat()
    }
}

fn lr4_lp(freq: f32) -> FilterParams {
    xo(Family::LinkwitzRiley, 4, false, freq)
}

fn lr4_hp(freq: f32) -> FilterParams {
    xo(Family::LinkwitzRiley, 4, true, freq)
}

/// A 12 dB/oct Butterworth high pass: the tweeters' guard and the subs'
/// subsonic filter.
fn bw2_hp(freq: f32) -> FilterParams {
    xo(Family::Butterworth, 2, true, freq)
}

/// The `channelId` of output `o`.
fn output_channel_id(o: u8) -> i64 {
    i64::from(dspi_session::preset_file::channel_id::for_output(o))
}

/// The demo device's settings as a document of `plat`: the RP2350 capture
/// as it is, or laid out as an RP2040 (2 inputs, 5 outputs).
fn seed_document(plat: Plat) -> String {
    let dev = FakeDevice::new(Capture::demo(), FakeOptions::default());
    let core = DspiCore::new();
    core.open_transport(Box::new(dev), "demo".into(), 0)
        .unwrap();
    core.fetch_all().unwrap();
    let text = core
        .save_as_template("seed".into(), APP_VERSION.into(), 0)
        .unwrap();
    if plat == Rp2350 {
        return text;
    }
    let mut doc = parse(&text);
    doc["meta"]["platform"] = "RP2040".into();
    doc["meta"]["inputChannelCount"] = 2.into();
    doc["meta"]["outputChannelCount"] = 5.into();
    let obj = doc.as_object_mut().unwrap();
    // The RP2350's pins and its upmixer do not apply.
    obj.remove("io");
    obj.remove("upmix");
    let last = output_channel_id(5);
    let channels: Vec<Value> = doc["channels"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["channelId"].as_i64().unwrap() < last)
        .cloned()
        .collect();
    doc["channels"] = Value::Array(channels);
    let matrix: Vec<Value> = doc["matrix"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["input"].as_u64().unwrap() < 2 && e["output"].as_u64().unwrap() < 5)
        .cloned()
        .collect();
    doc["matrix"] = Value::Array(matrix);
    doc.to_string()
}

fn seeded(plat: Plat) -> Arc<DspiCore> {
    let core = DspiCore::new();
    core.open_document(seed_document(plat), "seed.dspipreset".into())
        .unwrap();
    core
}

/// Output slot count and the data pins of `plat`'s default wiring (README
/// "Wiring Guide"): one per slot, then the PDM output's.
fn slots(plat: Plat) -> (usize, Vec<u8>) {
    match plat {
        Rp2350 => (4, vec![6, 7, 8, 9, 10]),
        Rp2040 => (2, vec![6, 7, 10]),
    }
}

fn i2s_slots(build: Build) -> usize {
    match build {
        Generic => 0,
        Pcm5102aSub | Pcm1808Pcm5102a => 1,
        TwoPcm5102a => 2,
    }
}

/// The hardware I/O a build defines, on the state.
fn build_state(s: &mut DspState, sp: &Spec) {
    if sp.build == Generic {
        return;
    }
    let (n_slots, pins) = slots(sp.plat);
    for (k, t) in s.output_types.iter_mut().enumerate() {
        *t = u8::from(k < i2s_slots(sp.build) && k < n_slots);
    }
    for (k, p) in pins.iter().enumerate() {
        if let Some(at) = s.output_pins.get_mut(k) {
            *at = *p;
        }
    }
    s.i2s_bck_pin = 14;
    s.mck_enabled = false;
    if sp.build == Pcm1808Pcm5102a {
        // README "PCM1808 ADC Module on Pico 2": I2S input on GPIO 4, the
        // DSPi the clock master, MCK on GPIO 13 at 256x.
        s.input_source = InputSource::I2s.to_raw();
        s.i2s_clock_mode = 0;
        s.i2s_input_channels = 2;
        s.i2s_input_rate = 1;
        if let Some(p) = s.i2s_rx_pins.get_mut(0) {
            *p = 4;
        }
        s.mck_enabled = true;
        s.mck_pin = 13;
        s.mck_multiplier = 1;
    }
}

/// A clean start: no EQ, no crossovers, nothing routed, every output off
/// and at 0 dB, every effect off, master volume 0 dB, user volume -20 dB,
/// the firmware's default names.
fn reset(s: &mut DspState, sp: &Spec) {
    let n_in = usize::from(s.num_input_channels);
    let n_out = usize::from(s.num_output_channels);
    s.bypass = false;
    s.preamp_db = 0.0;
    s.input_preamp_db.iter_mut().for_each(|p| *p = 0.0);
    s.master_volume_db = 0.0;
    s.user_volume_db = USER_VOLUME_DB;
    s.user_mute = false;
    s.loudness_enabled = false;
    s.crossfeed_enabled = false;
    s.leveller_enabled = false;
    s.psybass_enabled = false;
    for bank in s.filters.iter_mut().chain(s.xover.iter_mut()) {
        bank.iter_mut().for_each(|b| *b = flat());
    }
    for row in &mut s.matrix_routing {
        row.iter_mut().for_each(|r| *r = false);
    }
    for row in &mut s.matrix_gain {
        row.iter_mut().for_each(|g| *g = 0.0);
    }
    for row in &mut s.matrix_invert {
        row.iter_mut().for_each(|i| *i = false);
    }
    s.channel_delays.iter_mut().for_each(|d| *d = 0.0);
    for o in 0..n_out {
        s.output_enabled[o] = false;
        s.output_muted[o] = false;
        s.output_gain_db[o] = 0.0;
        s.output_delay_ms[o] = 0.0;
        s.limiter_enabled[o] = false;
    }
    for i in 0..n_in {
        s.channel_names[i] = match (sp.build, i) {
            (Pcm1808Pcm5102a, 0) => "I2S L".into(),
            (Pcm1808Pcm5102a, 1) => "I2S R".into(),
            _ => format!("USB {}", i + 1),
        };
    }
    // -- WP TPL1F -- a generic template's unused outputs are named neutrally
    // (review F3): the device's slot types are not known. A build's are its
    // own, so they keep the firmware's slot names.
    for o in 0..n_out {
        s.channel_names[n_in + o] = match sp.build {
            Generic => format!("Out {}", o + 1),
            _ => slot_name(o, n_out, &s.output_types),
        };
    }
}

/// One output: on, named, trimmed, its crossover slots, fed from `routes`
/// (input, gain dB).
fn output(
    s: &mut DspState,
    o: usize,
    name: &str,
    gain_db: f32,
    xovers: &[FilterParams],
    routes: &[(usize, f32)],
) {
    let wire = usize::from(s.num_input_channels) + o;
    s.output_enabled[o] = true;
    s.output_gain_db[o] = gain_db;
    s.channel_names[wire] = name.into();
    for (k, x) in xovers.iter().enumerate() {
        s.xover[wire][k] = *x;
    }
    for &(i, g) in routes {
        s.matrix_routing[i][o] = true;
        s.matrix_gain[i][o] = g;
    }
}

const L: usize = 0;
const R: usize = 1;

fn topology(s: &mut DspState, topo: Topo) {
    let pdm = usize::from(s.num_output_channels) - 1;
    let sub = |s: &mut DspState, o: usize, name: &str| {
        output(
            s,
            o,
            name,
            0.0,
            &[lr4_lp(SUB_XOVER_HZ), bw2_hp(SUBSONIC_HZ)],
            &[(L, SUB_SUM_DB), (R, SUB_SUM_DB)],
        );
    };
    let mains = |s: &mut DspState, xovers: &[FilterParams]| {
        output(s, 0, "Left", 0.0, xovers, &[(L, 0.0)]);
        output(s, 1, "Right", 0.0, xovers, &[(R, 0.0)]);
    };
    match topo {
        Stereo => mains(s, &[]),
        Sub21 => {
            mains(s, &[lr4_hp(SUB_XOVER_HZ)]);
            sub(s, pdm, "Sub");
        }
        Sub22 => {
            mains(s, &[lr4_hp(SUB_XOVER_HZ)]);
            sub(s, 2, "Sub 1");
            sub(s, 3, "Sub 2");
        }
        TwoWay => {
            let tw = [lr4_hp(TWO_WAY_HZ), bw2_hp(TWO_WAY_GUARD_HZ)];
            output(s, 0, "Tweeter L", TWEETER_TRIM_DB, &tw, &[(L, 0.0)]);
            output(s, 1, "Tweeter R", TWEETER_TRIM_DB, &tw, &[(R, 0.0)]);
            let wf = [lr4_lp(TWO_WAY_HZ)];
            output(s, 2, "Woofer L", 0.0, &wf, &[(L, 0.0)]);
            output(s, 3, "Woofer R", 0.0, &wf, &[(R, 0.0)]);
        }
        ThreeWay => {
            let tw = [lr4_hp(THREE_WAY_HIGH_HZ), bw2_hp(THREE_WAY_GUARD_HZ)];
            output(s, 0, "Tweeter L", TWEETER_TRIM_DB, &tw, &[(L, 0.0)]);
            output(s, 1, "Tweeter R", TWEETER_TRIM_DB, &tw, &[(R, 0.0)]);
            let wf = [lr4_lp(THREE_WAY_LOW_HZ)];
            output(s, 2, "Woofer L", 0.0, &wf, &[(L, 0.0)]);
            output(s, 3, "Woofer R", 0.0, &wf, &[(R, 0.0)]);
            let mid = [lr4_hp(THREE_WAY_LOW_HZ), lr4_lp(THREE_WAY_HIGH_HZ)];
            output(s, 4, "Mid L", 0.0, &mid, &[(L, 0.0)]);
            output(s, 5, "Mid R", 0.0, &mid, &[(R, 0.0)]);
        }
        Headphones => {
            output(s, 0, "Headphone L", 0.0, &[], &[(L, 0.0)]);
            output(s, 1, "Headphone R", 0.0, &[], &[(R, 0.0)]);
            // The Default preset (BS2B, 700 Hz, 4.5 dB) with ITD, on pair 1.
            s.crossfeed_enabled = true;
            s.crossfeed_preset = 0;
            s.crossfeed_freq = 700.0;
            s.crossfeed_feed = 4.5;
            s.crossfeed_itd = true;
            s.crossfeed_output_pair_mask = 1;
        }
    }
}

const CHECK: &str = "Check levels, polarity, delays and crossover points for your drivers; \
                     user volume starts at -20 dB.";

fn status(sp: &Spec) -> String {
    let pdm = match sp.plat {
        Rp2350 => 9,
        Rp2040 => 5,
    };
    let what = match sp.topo {
        Stereo => "L and R full range on outputs 1-2".to_string(),
        Sub21 => format!(
            "outputs 1-2 high-passed at 80 Hz (LR4); PDM sub on output {pdm}, low-passed at \
             80 Hz (LR4) from L+R at -6.1 dB each, 20 Hz subsonic, 0 ms delay (PDM needs an RC \
             filter)"
        ),
        Sub22 => "outputs 1-2 high-passed at 80 Hz (LR4); subs on outputs 3-4, low-passed at \
                  80 Hz (LR4) from L+R at -6.1 dB each, 20 Hz subsonic, 0 ms delay"
            .to_string(),
        TwoWay => "LR4 at 2 kHz; tweeters on outputs 1-2 at -6 dB with a 500 Hz 12 dB/oct guard \
                   high-pass, woofers on 3-4"
            .to_string(),
        ThreeWay => "LR4 at 300 Hz and 3 kHz; tweeters on outputs 1-2 at -6 dB with a 750 Hz \
                     12 dB/oct guard high-pass, woofers on 3-4, mids on 5-6"
            .to_string(),
        Headphones => "outputs 1-2 with crossfeed on (Default: 700 Hz, 4.5 dB, ITD)".to_string(),
    };
    let board = match sp.plat {
        Rp2350 => "Pico 2",
        Rp2040 => "Pico",
    };
    match sp.build {
        Generic => format!(
            "Generic starting point for any {} board, input and hardware I/O left alone: {what}. \
             {CHECK}",
            Platform::from_id(u8::from(sp.plat == Rp2350)).name()
        ),
        Pcm5102aSub => format!(
            "Starting point for a {board} with a PCM5102A on slot 1 (I2S data GPIO 6, BCK 14, \
             LRCK 15, no MCK) and the PDM sub on GPIO 10: {what}. {CHECK}"
        ),
        TwoPcm5102a => format!(
            "Starting point for a {board} with two PCM5102A on slots 1-2 (I2S data GPIO 6 and \
             7, BCK 14, LRCK 15, no MCK): {what}. {CHECK}"
        ),
        Pcm1808Pcm5102a => format!(
            "Starting point for a {board} with a PCM1808 ADC on the I2S input (data GPIO 4, \
             MCK GPIO 13 at 256x, 48 kHz) and a PCM5102A on slot 1 (I2S data GPIO 6, BCK 14, \
             LRCK 15): {what}. {CHECK}"
        ),
    }
}

/// The `io` keys a build defines; the rest stay the device's.
fn build_io_keys(build: Build) -> &'static [&'static str] {
    match build {
        Generic => &[],
        Pcm5102aSub | TwoPcm5102a => &["i2sBckPin", "mckEnabled", "outputPins", "outputSlotTypes"],
        Pcm1808Pcm5102a => &[
            "i2sBckPin",
            "i2sClockMode",
            "i2sInputChannels",
            "i2sInputRateHz",
            "i2sRxPins",
            "mckEnabled",
            "mckMultiplier",
            "mckPin",
            "outputPins",
            "outputSlotTypes",
        ],
    }
}

/// The document's text after the export: fixed `savedUtc`, the status line,
/// no pair map, and the `io` block the build defines (none for a generic
/// template, so the device's hardware I/O is left alone).
fn finish(text: &str, sp: &Spec) -> String {
    let mut doc = parse(text);
    doc["meta"]["savedUtc"] = SAVED_UTC.into();
    doc["meta"][STATUS_KEY] = status(sp).into();
    if let Some(cf) = doc["crossfeed"].as_object_mut() {
        cf.remove("pairMap");
    }
    // -- WP TPL1F -- the input side is the device's (review F1): no LG Sound
    // Sync, and an input source only where the build wires one.
    if let Some(g) = doc["global"].as_object_mut() {
        g.remove("lgSoundSyncEnabled");
        if sp.build != Pcm1808Pcm5102a {
            g.remove("inputSource");
        }
    }
    let keys = build_io_keys(sp.build);
    let obj = doc.as_object_mut().unwrap();
    if keys.is_empty() {
        obj.remove("io");
    } else {
        let io = obj.get_mut("io").and_then(Value::as_object_mut).unwrap();
        io.retain(|k, _| keys.contains(&k.as_str()));
        let (n_slots, _) = slots(sp.plat);
        if let Some(types) = io.get_mut("outputSlotTypes").and_then(Value::as_array_mut) {
            types.truncate(n_slots);
        }
        let (_, pins) = slots(sp.plat);
        if let Some(p) = io.get_mut("outputPins").and_then(Value::as_array_mut) {
            p.truncate(pins.len());
        }
        // Only the first I2S input pair is wired.
        if let Some(rx) = io.get_mut("i2sRxPins").and_then(Value::as_array_mut) {
            rx.truncate(1);
        }
    }
    let mut out = serde_json::to_string_pretty(&doc).unwrap();
    out.push('\n');
    out
}

/// Template `sp`'s text.
fn generate(sp: &Spec) -> String {
    let core = seeded(sp.plat);
    {
        let mut inner = core.lock();
        let s = &mut inner.state;
        build_state(s, sp);
        reset(s, sp);
        topology(s, sp.topo);
    }
    let text = core
        .save_as_template(sp.name.into(), APP_VERSION.into(), 0)
        .unwrap();
    finish(&text, sp)
}

/// Writes every template of [`SPECS`] to `DSPI_TPL1_OUT` (a folder).
#[test]
#[ignore = "writes the shipped templates to DSPI_TPL1_OUT"]
fn wp_tpl1_write_templates() {
    let Ok(out) = std::env::var("DSPI_TPL1_OUT") else {
        return;
    };
    for sp in SPECS {
        let path = std::path::Path::new(&out).join(format!("{}.dspipreset", sp.id));
        std::fs::write(path, generate(sp)).unwrap();
    }
}

// ── Checks ────────────────────────────────────────────────────────────

fn shipped(id: &str) -> &'static str {
    TEMPLATES
        .iter()
        .find(|(n, _)| *n == id)
        .map(|(_, t)| *t)
        .unwrap_or_else(|| panic!("{id} is not in TEMPLATES"))
}

fn opened(sp: &Spec) -> (Arc<DspiCore>, Value) {
    let core = DspiCore::new();
    let reply = core
        .open_document(shipped(sp.id).into(), format!("{}.dspipreset", sp.id))
        .unwrap();
    (core, parse(&reply))
}

/// A document saved from a template's session, for a diff with the
/// template: without the hardware I/O and input side the template leaves
/// alone (a session holds the demo's).
fn as_shipped(saved: &str, sp: &Spec) -> String {
    let mut doc = parse(saved);
    if sp.build == Generic
        && let Some(obj) = doc.as_object_mut()
    {
        obj.remove("io");
    }
    // -- WP TPL1F -- nor the input side a template leaves alone.
    if let Some(g) = doc["global"].as_object_mut() {
        g.remove("lgSoundSyncEnabled");
        if sp.build != Pcm1808Pcm5102a {
            g.remove("inputSource");
        }
    }
    doc.to_string()
}

#[test]
fn wp_tpl1_shipped_templates_match_the_generator() {
    for sp in SPECS {
        assert!(
            generate(sp) == shipped(sp.id),
            "{} differs from its generator: run wp_tpl1_write_templates (see the module doc)",
            sp.id
        );
    }
    // Every shipped template is the generator's, or the K340's.
    let ids: Vec<&str> = TEMPLATES.iter().map(|(n, _)| *n).collect();
    let mut want: Vec<&str> = SPECS.iter().map(|s| s.id).collect();
    want.push("k340");
    assert_eq!(ids, want);
}

#[test]
fn wp_tpl1_every_template_opens_as_a_template_document() {
    for sp in SPECS {
        let (core, reply) = opened(sp);
        assert_eq!(reply["ok"], true, "{}", sp.id);
        assert_eq!(reply["kind"], "template", "{}", sp.id);
        assert_eq!(reply["name"], sp.name, "{}", sp.id);
        let notes = reply["notes"].as_array().unwrap();
        assert!(notes.is_empty(), "{}: {notes:?}", sp.id);
        let (platform, ins, outs) = match sp.plat {
            Rp2350 => ("RP2350", 8, 9),
            Rp2040 => ("RP2040", 2, 5),
        };
        assert_eq!(reply["platform"], platform, "{}", sp.id);
        assert_eq!(reply["inputs"], ins, "{}", sp.id);
        assert_eq!(reply["outputs"], outs, "{}", sp.id);
        let s = core.get_state();
        assert_eq!(s.num_output_channels, outs, "{}", sp.id);
        // Saved unedited, it says what the template does.
        let again = core.save_document("v".into(), 0).unwrap();
        let d = parse(&files::config_diff(
            shipped(sp.id).into(),
            as_shipped(&again, sp),
        ));
        assert_eq!(d["count"], 0, "{}: {d:#}", sp.id);
    }
}

#[test]
fn wp_tpl1_every_template_validates() {
    for sp in SPECS {
        let text = shipped(sp.id);
        let doc = parse(text);
        assert_eq!(doc["meta"]["kind"], "template", "{}", sp.id);
        let inspect = parse(&files::config_inspect(text.into()));
        assert_eq!(inspect["ok"], true, "{}: {inspect}", sp.id);
        let status = doc["meta"][STATUS_KEY].as_str().unwrap();
        assert!(
            status.to_lowercase().contains("starting point"),
            "{}",
            sp.id
        );
        assert!(status.contains("Check levels, polarity"), "{}", sp.id);
        assert!(!status.contains("K340") && !sp.name.contains("K340"));
        assert!(doc["crossfeed"].get("pairMap").is_none(), "{}", sp.id);
        // A generic one leaves the device's hardware I/O alone.
        assert_eq!(doc.get("io").is_some(), sp.build != Generic, "{}", sp.id);
    }
    // An RP2350 template imports cleanly onto the RP2350 demo, volumes and
    // hardware I/O included, and the device then holds what it says.
    for sp in SPECS.iter().filter(|s| s.plat == Rp2350) {
        let core = DspiCore::new();
        core.open_transport(
            Box::new(FakeDevice::new(Capture::demo(), FakeOptions::default())),
            "demo".into(),
            0,
        )
        .unwrap();
        core.fetch_all().unwrap();
        let reply =
            parse(&core.config_import_tok(shipped(sp.id).into(), IMPORT_VOLUMES | IMPORT_HARDWARE));
        assert_eq!(reply["ok"], true, "{}: {reply:#}", sp.id);
        assert_eq!(reply["clean"], true, "{}: {reply:#}", sp.id);
        assert!(
            reply["outOfRange"].as_array().unwrap().is_empty(),
            "{}",
            sp.id
        );
        let s = core.get_state();
        assert_eq!(s.user_volume_db, USER_VOLUME_DB, "{}", sp.id);
        let after = core.config_export("v".into(), "v".into(), 0).unwrap();
        let d = parse(&files::config_diff(
            shipped(sp.id).into(),
            as_shipped(&after, sp),
        ));
        assert_eq!(d["count"], 0, "{}: {d:#}", sp.id);
    }
}

#[test]
fn wp_tpl1_volumes_effects_and_eq_start_clean() {
    for sp in SPECS {
        let doc = parse(shipped(sp.id));
        assert_eq!(doc["global"]["userVolumeDb"], -20.0, "{}", sp.id);
        assert_eq!(doc["global"]["masterVolumeDb"], 0.0, "{}", sp.id);
        assert_eq!(doc["global"]["bypass"], false, "{}", sp.id);
        for block in [
            "loudness", "leveller", "psybass", "upmix", "subharm", "tube",
        ] {
            if let Some(b) = doc.get(block) {
                assert_eq!(b["enabled"], false, "{}: {block}", sp.id);
            }
        }
        assert_eq!(
            doc["crossfeed"]["enabled"],
            sp.topo == Headphones,
            "{}",
            sp.id
        );
        let (core, _) = opened(sp);
        let s = core.get_state();
        assert_eq!(s.user_volume_db, USER_VOLUME_DB, "{}", sp.id);
        assert_eq!(s.master_volume_db, 0.0, "{}", sp.id);
        // No room EQ: every PEQ band is off.
        for (ch, bank) in s.filters.iter().enumerate() {
            for b in bank {
                assert_eq!(
                    FilterType::from_raw(b.filter_type),
                    FilterType::Flat,
                    "{} channel {ch}",
                    sp.id
                );
            }
        }
        for o in 0..usize::from(s.num_output_channels) {
            assert!(!s.limiter_enabled[o] && !s.output_muted[o], "{}", sp.id);
            assert_eq!(s.output_delay_ms[o], 0.0, "{}", sp.id);
        }
    }
}

/// One crossover slot as a test writes it: family, order, high pass, Hz.
type Xo = (Family, u8, bool, f32);
/// What a test expects of one output, written out by hand (not from
/// [`topology`]): name, trim, crossover slots, and the inputs feeding it
/// with their gain.
type Want = (&'static str, f32, Vec<Xo>, Vec<(usize, f32)>);

fn want(topo: Topo, outs: usize) -> Vec<(usize, Want)> {
    use Family::{Butterworth as Bw, LinkwitzRiley as Lr};
    let sub = |name| {
        (
            name,
            0.0,
            vec![(Lr, 4, false, 80.0), (Bw, 2, true, 20.0)],
            vec![(0, -6.1), (1, -6.1)],
        )
    };
    let hp80 = vec![(Lr, 4, true, 80.0)];
    match topo {
        Stereo => vec![
            (0, ("Left", 0.0, vec![], vec![(0, 0.0)])),
            (1, ("Right", 0.0, vec![], vec![(1, 0.0)])),
        ],
        Sub21 => vec![
            (0, ("Left", 0.0, hp80.clone(), vec![(0, 0.0)])),
            (1, ("Right", 0.0, hp80, vec![(1, 0.0)])),
            (outs - 1, sub("Sub")),
        ],
        Sub22 => vec![
            (0, ("Left", 0.0, hp80.clone(), vec![(0, 0.0)])),
            (1, ("Right", 0.0, hp80, vec![(1, 0.0)])),
            (2, sub("Sub 1")),
            (3, sub("Sub 2")),
        ],
        TwoWay => {
            let tw = vec![(Lr, 4, true, 2000.0), (Bw, 2, true, 500.0)];
            let wf = vec![(Lr, 4, false, 2000.0)];
            vec![
                (0, ("Tweeter L", -6.0, tw.clone(), vec![(0, 0.0)])),
                (1, ("Tweeter R", -6.0, tw, vec![(1, 0.0)])),
                (2, ("Woofer L", 0.0, wf.clone(), vec![(0, 0.0)])),
                (3, ("Woofer R", 0.0, wf, vec![(1, 0.0)])),
            ]
        }
        ThreeWay => {
            let tw = vec![(Lr, 4, true, 3000.0), (Bw, 2, true, 750.0)];
            let wf = vec![(Lr, 4, false, 300.0)];
            let mid = vec![(Lr, 4, true, 300.0), (Lr, 4, false, 3000.0)];
            vec![
                (0, ("Tweeter L", -6.0, tw.clone(), vec![(0, 0.0)])),
                (1, ("Tweeter R", -6.0, tw, vec![(1, 0.0)])),
                (2, ("Woofer L", 0.0, wf.clone(), vec![(0, 0.0)])),
                (3, ("Woofer R", 0.0, wf, vec![(1, 0.0)])),
                (4, ("Mid L", 0.0, mid.clone(), vec![(0, 0.0)])),
                (5, ("Mid R", 0.0, mid, vec![(1, 0.0)])),
            ]
        }
        Headphones => vec![
            (0, ("Headphone L", 0.0, vec![], vec![(0, 0.0)])),
            (1, ("Headphone R", 0.0, vec![], vec![(1, 0.0)])),
        ],
    }
}

#[test]
fn wp_tpl1_routing_names_and_crossovers_are_where_claimed() {
    for sp in SPECS {
        let (core, _) = opened(sp);
        let s = core.get_state();
        let n_in = usize::from(s.num_input_channels);
        let outs = usize::from(s.num_output_channels);
        let wants = want(sp.topo, outs);
        for o in 0..outs {
            let wire = n_in + o;
            let Some((_, (name, trim, xos, routes))) = wants.iter().find(|(w, _)| *w == o) else {
                assert!(!s.output_enabled[o], "{}: output {} is on", sp.id, o + 1);
                assert!(
                    (0..n_in).all(|i| !s.matrix_routing[i][o]),
                    "{}: output {} is fed",
                    sp.id,
                    o + 1
                );
                continue;
            };
            let tag = format!("{} output {}", sp.id, o + 1);
            assert!(s.output_enabled[o], "{tag}");
            assert_eq!(s.channel_names[wire], *name, "{tag}");
            assert_eq!(s.output_gain_db[o], *trim, "{tag}");
            let got: Vec<Xo> = s.xover[wire]
                .iter()
                .filter(|b| b.filter_type != 0)
                .map(|b| {
                    let m = xover::meta(b.filter_type).unwrap();
                    (m.family, m.order, m.high_pass, b.freq)
                })
                .collect();
            assert_eq!(&got, xos, "{tag}");
            let fed: Vec<(usize, f32)> = (0..n_in)
                .filter(|&i| s.matrix_routing[i][o])
                .map(|i| (i, s.matrix_gain[i][o]))
                .collect();
            assert_eq!(&fed, routes, "{tag}");
            assert!((0..n_in).all(|i| !s.matrix_invert[i][o]), "{tag}");
        }
    }
}

/// The response of an output's crossover slots at `hz`, in dB.
fn chain_db(bank: &[FilterParams], hz: f64) -> f64 {
    bank.iter()
        .filter(|b| b.filter_type != 0)
        .map(|b| xover::response_at(hz, b.filter_type, b.freq))
        .sum()
}

// -- WP TPL1F --
/// An output's crossover slots at `hz` as a complex gain (re, im): each
/// section's H(z) = (b0 + b1 z^-1 + b2 z^-2) / (1 + a1 z^-1 + a2 z^-2) at
/// z = e^jw, the firmware's 48 kHz.
fn chain_complex(bank: &[FilterParams], hz: f64) -> (f64, f64) {
    let w = 2.0 * std::f64::consts::PI * hz / dspi_proto::dsp::SAMPLE_RATE;
    let mut acc = (1.0f64, 0.0f64);
    for b in bank.iter().filter(|b| b.filter_type != 0) {
        for c in xover::sections(b.filter_type, b.freq) {
            let num = (
                c.b0 + c.b1 * w.cos() + c.b2 * (2.0 * w).cos(),
                -(c.b1 * w.sin() + c.b2 * (2.0 * w).sin()),
            );
            let den = (
                1.0 + c.a1 * w.cos() + c.a2 * (2.0 * w).cos(),
                -(c.a1 * w.sin() + c.a2 * (2.0 * w).sin()),
            );
            let d = den.0 * den.0 + den.1 * den.1;
            let h = (
                (num.0 * den.0 + num.1 * den.1) / d,
                (num.1 * den.0 - num.0 * den.1) / d,
            );
            acc = (acc.0 * h.0 - acc.1 * h.1, acc.0 * h.1 + acc.1 * h.0);
        }
    }
    acc
}

/// The acoustic sum of `banks` at `hz` (trims at 0 dB, ideal drivers), dB.
fn sum_db(banks: &[&[FilterParams]], hz: f64) -> f64 {
    let (re, im) = banks
        .iter()
        .map(|b| chain_complex(b, hz))
        .fold((0.0, 0.0), |a, h| (a.0 + h.0, a.1 + h.1));
    10.0 * (re * re + im * im).log10()
}

/// The lowest of [`sum_db`] from `lo` to `hi` Hz (1/48 octave steps), and
/// where.
fn worst_sum(banks: &[&[FilterParams]], lo: f64, hi: f64) -> (f64, f64) {
    let mut worst = (f64::MAX, lo);
    let mut hz = lo;
    while hz <= hi {
        let db = sum_db(banks, hz);
        if db < worst.0 {
            worst = (db, hz);
        }
        hz *= 2f64.powf(1.0 / 48.0);
    }
    worst
}

#[test]
fn wp_tpl1f_the_lr4_pairs_sum_flat_with_the_guard_and_subsonic_filters() {
    for sp in SPECS {
        let (core, _) = opened(sp);
        let s = core.get_state();
        let n_in = usize::from(s.num_input_channels);
        let bank = |name: &str| {
            let o = (0..usize::from(s.num_output_channels))
                .find(|&o| s.output_enabled[o] && s.channel_names[n_in + o] == name)
                .unwrap_or_else(|| panic!("{}: no {name}", sp.id));
            s.xover[n_in + o].clone()
        };
        let report = |what: &str, at_fc: f64, worst: (f64, f64), limit: f64| {
            eprintln!(
                "{}: {what}: sum {at_fc:+.3} dB at the crossover, worst {:+.3} dB at {:.0} Hz",
                sp.id, worst.0, worst.1
            );
            assert!(at_fc.abs() < limit && worst.0 > -limit, "{} {what}", sp.id);
        };
        match sp.topo {
            Sub21 | Sub22 => {
                let sub = bank(if sp.topo == Sub21 { "Sub" } else { "Sub 1" });
                let main = bank("Left");
                let b: [&[FilterParams]; 2] = [&sub, &main];
                report(
                    "mains + sub",
                    sum_db(&b, 80.0),
                    worst_sum(&b, 60.0, 300.0),
                    0.25,
                );
            }
            TwoWay => {
                let (tw, wf) = (bank("Tweeter L"), bank("Woofer L"));
                let b: [&[FilterParams]; 2] = [&tw, &wf];
                let plain_tw = [tw[0]];
                let plain: [&[FilterParams]; 2] = [&plain_tw, &wf];
                eprintln!(
                    "{}: tweeter + woofer without the guard: {:+.3} dB at the crossover",
                    sp.id,
                    sum_db(&plain, 2000.0)
                );
                report(
                    "tweeter + woofer",
                    sum_db(&b, 2000.0),
                    worst_sum(&b, 250.0, 16000.0),
                    0.25,
                );
            }
            ThreeWay => {
                let (tw, wf, mid) = (bank("Tweeter L"), bank("Woofer L"), bank("Mid L"));
                let b: [&[FilterParams]; 3] = [&tw, &wf, &mid];
                report(
                    "3-way at 3 kHz",
                    sum_db(&b, 3000.0),
                    worst_sum(&b, 1000.0, 16000.0),
                    0.25,
                );
                // Around 300 Hz a 3-way LR4 sum is not flat even unguarded
                // (the mid's 3 kHz low pass shifts the phase there); hold it
                // to what the plain LR4 network gives.
                let plain_tw = [tw[0]];
                let plain: [&[FilterParams]; 3] = [&plain_tw, &wf, &mid];
                let (g, p) = (sum_db(&b, 300.0), sum_db(&plain, 300.0));
                eprintln!(
                    "{}: 3-way at 300 Hz: {g:+.3} dB (plain LR4 {p:+.3} dB)",
                    sp.id
                );
                assert!((g - p).abs() < 0.1, "{}", sp.id);
            }
            Stereo | Headphones => {}
        }
    }
}

#[test]
fn wp_tpl1_crossover_responses_meet_at_the_claimed_points() {
    for sp in SPECS {
        let (core, _) = opened(sp);
        let s = core.get_state();
        let n_in = usize::from(s.num_input_channels);
        let bank = |name: &str| {
            let o = (0..usize::from(s.num_output_channels))
                .find(|&o| s.output_enabled[o] && s.channel_names[n_in + o] == name)
                .unwrap_or_else(|| panic!("{}: no {name}", sp.id));
            s.xover[n_in + o].clone()
        };
        let near = |got: f64, want: f64, what: &str| {
            assert!((got - want).abs() < 0.3, "{} {what}: {got:.2} dB", sp.id);
        };
        let one = |b: &FilterParams, hz: f64| xover::response_at(hz, b.filter_type, b.freq);
        match sp.topo {
            Sub21 | Sub22 => {
                let sub = bank(if sp.topo == Sub21 { "Sub" } else { "Sub 1" });
                let main = bank("Left");
                // LR4 at 80 Hz: both -6 dB there.
                near(one(&sub[0], 80.0), -6.02, "sub LP at 80 Hz");
                near(chain_db(&main, 80.0), -6.02, "mains at 80 Hz");
                assert!(chain_db(&sub, 320.0) < -40.0, "{}: sub up 2 oct", sp.id);
                assert!(chain_db(&main, 20.0) < -40.0, "{}: mains down 2 oct", sp.id);
                // The subsonic filter: -3 dB at 20 Hz, little at 40 Hz.
                near(one(&sub[1], 20.0), -3.01, "subsonic at 20 Hz");
                assert!(chain_db(&sub, 40.0) > -1.5, "{}: sub at 40 Hz", sp.id);
            }
            TwoWay => {
                let tw = bank("Tweeter L");
                let wf = bank("Woofer L");
                near(chain_db(&wf, 2000.0), -6.02, "woofer at 2 kHz");
                // The guard high-pass costs under 0.1 dB of the tweeter's
                // own level at the crossover (its phase is checked in
                // wp_tpl1f_the_lr4_pairs_sum_flat_...), and adds 12 dB/oct
                // below it.
                let lr_only = one(&tw[0], 2000.0);
                near(lr_only, -6.02, "tweeter LR4 at 2 kHz");
                assert!((chain_db(&tw, 2000.0) - lr_only).abs() < 0.1, "{}", sp.id);
                assert!(chain_db(&tw, 500.0) < -50.0, "{}: tweeter at 500 Hz", sp.id);
            }
            ThreeWay => {
                let tw = bank("Tweeter L");
                let wf = bank("Woofer L");
                let mid = bank("Mid L");
                near(chain_db(&wf, 300.0), -6.02, "woofer at 300 Hz");
                near(chain_db(&mid, 300.0), -6.02, "mid at 300 Hz");
                near(chain_db(&mid, 3000.0), -6.02, "mid at 3 kHz");
                let lr_only = one(&tw[0], 3000.0);
                near(lr_only, -6.02, "tweeter LR4 at 3 kHz");
                assert!((chain_db(&tw, 3000.0) - lr_only).abs() < 0.1, "{}", sp.id);
                assert!(chain_db(&tw, 750.0) < -50.0, "{}: tweeter at 750 Hz", sp.id);
                // The mid's band centre is nearly flat.
                assert!(chain_db(&mid, 950.0) > -0.5, "{}: mid at 950 Hz", sp.id);
            }
            Stereo | Headphones => {}
        }
    }
}

#[test]
fn wp_tpl1_safety_no_clipping_sums_every_tweeter_high_passed_and_core1_free() {
    for sp in SPECS {
        let (core, _) = opened(sp);
        let s = core.get_state();
        let n_in = usize::from(s.num_input_channels);
        let outs = usize::from(s.num_output_channels);
        for o in 0..outs {
            let tag = format!("{} output {}", sp.id, o + 1);
            // In-phase full-scale inputs sum to at most full scale, and the
            // output's own trim only ever cuts.
            let sum: f64 = (0..n_in)
                .filter(|&i| s.matrix_routing[i][o])
                .map(|i| 10f64.powf(f64::from(s.matrix_gain[i][o]) / 20.0))
                .sum();
            assert!(sum <= 1.0, "{tag}: matrix sum {sum:.4}");
            assert!(s.output_gain_db[o] <= 0.0, "{tag}");
            let name = &s.channel_names[n_in + o];
            if s.output_enabled[o] && name.to_lowercase().contains("tweeter") {
                let hp: Vec<&FilterParams> = s.xover[n_in + o]
                    .iter()
                    .filter(|b| xover::meta(b.filter_type).is_some_and(|m| m.high_pass))
                    .collect();
                assert!(hp.len() >= 2, "{tag}: a crossover and a guard high-pass");
                // The guard two octaves under the crossover at the lowest.
                let xo = hp.iter().map(|b| b.freq).fold(0.0f32, f32::max);
                assert!(hp.iter().all(|b| b.freq >= xo / 4.0), "{tag}");
                assert!(s.output_gain_db[o] < 0.0, "{tag}: starts trimmed down");
            }
        }
        // The PDM output and outputs 3..N-1 share Core 1: never both on.
        let pdm = outs - 1;
        if s.output_enabled[pdm] {
            assert!((2..pdm).all(|o| !s.output_enabled[o]), "{}", sp.id);
        }
    }
}

#[test]
fn wp_tpl1_builds_carry_their_hardware_io() {
    for sp in SPECS.iter().filter(|s| s.build != Generic) {
        let doc = parse(shipped(sp.id));
        let io = doc["io"].as_object().unwrap();
        let keys: Vec<&str> = io.keys().map(String::as_str).collect();
        assert_eq!(keys, build_io_keys(sp.build), "{}", sp.id);
        let (types, pins): (Vec<u8>, Vec<u8>) = match (sp.plat, sp.build) {
            (Rp2350, TwoPcm5102a) => (vec![1, 1, 0, 0], vec![6, 7, 8, 9, 10]),
            (Rp2350, _) => (vec![1, 0, 0, 0], vec![6, 7, 8, 9, 10]),
            (Rp2040, TwoPcm5102a) => (vec![1, 1], vec![6, 7, 10]),
            (Rp2040, _) => (vec![1, 0], vec![6, 7, 10]),
        };
        assert_eq!(io["outputSlotTypes"], serde_json::json!(types), "{}", sp.id);
        assert_eq!(io["outputPins"], serde_json::json!(pins), "{}", sp.id);
        assert_eq!(io["i2sBckPin"], 14, "{}", sp.id);
        let (core, _) = opened(sp);
        let s = core.get_state();
        assert_eq!(&s.output_types[..types.len()], &types[..], "{}", sp.id);
        assert_eq!(&s.output_pins[..pins.len()], &pins[..], "{}", sp.id);
        // The unused outputs keep the firmware's slot names.
        let n_in = usize::from(s.num_input_channels);
        if sp.build == TwoPcm5102a {
            let fifth = match sp.plat {
                Rp2350 => "SPDIF 3 L",
                Rp2040 => "PDM",
            };
            assert_eq!(s.channel_names[n_in + 4], fifth, "{}", sp.id);
        }
        if sp.build == Pcm1808Pcm5102a {
            assert_eq!(doc["global"]["inputSource"], InputSource::I2s.to_raw());
            assert_eq!(io["i2sRxPins"], serde_json::json!([4]));
            assert_eq!(io["mckEnabled"], true);
            assert_eq!(io["mckPin"], 13);
            assert_eq!(io["mckMultiplier"], 256);
            assert_eq!(io["i2sClockMode"], 0);
            assert_eq!(io["i2sInputRateHz"], 48000);
            assert_eq!(s.input_source, InputSource::I2s.to_raw());
            assert_eq!(s.channel_names[..2], ["I2S L", "I2S R"]);
        } else {
            assert_eq!(io["mckEnabled"], false, "{}", sp.id);
            assert!(doc["global"].get("inputSource").is_none(), "{}", sp.id);
        }
    }
}

#[test]
fn wp_tpl1_listed_with_name_platform_and_status() {
    let list = parse(&template_list(String::new()));
    let list = list.as_array().unwrap();
    assert_eq!(list.len(), SPECS.len() + 1);
    for sp in SPECS {
        let t = list
            .iter()
            .find(|t| t["id"] == format!("builtin:{}", sp.id))
            .unwrap_or_else(|| panic!("{} not listed", sp.id));
        assert_eq!(t["name"], sp.name);
        assert_eq!(t["builtin"], true);
        assert_eq!(t["status"], status(sp));
        let platform = match sp.plat {
            Rp2350 => "RP2350",
            Rp2040 => "RP2040",
        };
        assert_eq!(t["platform"], platform);
        // The builds are told apart by their name.
        assert_eq!(
            sp.name.starts_with("Build: "),
            sp.build != Generic,
            "{}",
            sp.id
        );
        assert_eq!(
            template_text(format!("builtin:{}", sp.id), String::new()).unwrap(),
            shipped(sp.id)
        );
    }
}

// -- WP TPL1F -- review F1 (the probe's idea, review/tpl1
// `tests/review_tpl1_probe.rs`): a generic template leaves the device's
// input side alone.
#[test]
fn wp_tpl1f_a_generic_import_keeps_the_input_source_and_sound_sync() {
    for sp in SPECS.iter().filter(|s| s.plat == Rp2350) {
        let doc = parse(shipped(sp.id));
        assert!(
            doc["global"].get("lgSoundSyncEnabled").is_none(),
            "{}",
            sp.id
        );
        assert_eq!(
            doc["global"].get("inputSource").is_some(),
            sp.build == Pcm1808Pcm5102a,
            "{}",
            sp.id
        );
        if sp.build != Generic {
            continue;
        }
        for opts in [0, IMPORT_VOLUMES | IMPORT_HARDWARE] {
            let core = DspiCore::new();
            core.open_transport(
                Box::new(FakeDevice::new(Capture::demo(), FakeOptions::default())),
                "demo".into(),
                0,
            )
            .unwrap();
            core.fetch_all().unwrap();
            // A device listening over S/PDIF, Sound Sync off.
            core.set_input_source(InputSource::Spdif.to_raw()).unwrap();
            core.set_lg_sound_sync(false).unwrap();
            let reply = parse(&core.config_import_tok(shipped(sp.id).into(), opts));
            assert_eq!(reply["ok"], true, "{}: {reply:#}", sp.id);
            assert_eq!(reply["clean"], true, "{}: {reply:#}", sp.id);
            let left: Vec<&str> = reply["leftAlone"]
                .as_array()
                .unwrap()
                .iter()
                .filter_map(Value::as_str)
                .collect();
            assert!(left.is_empty(), "{}: {left:?}", sp.id);
            let s = core.get_state();
            assert_eq!(
                s.input_source,
                InputSource::Spdif.to_raw(),
                "{} {opts}",
                sp.id
            );
            assert!(!s.lg_sound_sync_enabled, "{} {opts}", sp.id);
            // The template's own settings did land.
            assert_eq!(s.user_volume_db == USER_VOLUME_DB, opts != 0, "{}", sp.id);
            assert!(s.output_enabled[0] && s.output_enabled[1], "{}", sp.id);
        }
    }
}

// -- WP TPL1F -- the core rule: only an absent key is silent; a present
// but unreadable one is still named.
#[test]
fn wp_tpl1f_an_unreadable_input_source_is_still_named() {
    let mut doc = parse(shipped("stereo-rp2350"));
    doc["global"]["inputSource"] = "spdif".into();
    let core = DspiCore::new();
    core.open_transport(
        Box::new(FakeDevice::new(Capture::demo(), FakeOptions::default())),
        "demo".into(),
        0,
    )
    .unwrap();
    core.fetch_all().unwrap();
    let reply = parse(&core.config_import_tok(doc.to_string(), 0));
    assert_eq!(reply["ok"], true, "{reply:#}");
    assert_eq!(reply["clean"], false, "{reply:#}");
}
