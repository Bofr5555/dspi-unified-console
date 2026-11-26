//! Document diff (WP P1DF, PRO1 W5): what differs between two device states,
//! one row per changed value, in the words [`crate::presets`] uses for the
//! unsaved-changes list.
//!
//! The states come from documents or the device: [`crate::files`] reads a
//! document onto a state (as an import would, without sending anything) and
//! hands both states here. A row is `{block, section, item, before, after}`:
//! `block` is the import block that carries the value (`eq`, `crossover`,
//! `routing`, `outputs`, `limiters`, `effects`, `volumes`, `hardware`,
//! `names`, the `IMPORT_*` bits' names), so a preview can show per block what
//! a partial import would change. The summary is the unsaved-changes list's
//! own sentences for the same pair of states.

use dspi_proto::FilterType;
use dspi_proto::generated as g;
use serde_json::{Map, Value, json};

use crate::consts::*;
use crate::presets::{
    PIN_UNSET, band_level, ceiling, cell, channel_label, clock_mode, db, delay, enabled,
    mck_multiplier, on_off, pair_outputs, param, plain, rate_hz, same, same_band, selectivity,
    source_title, volume,
};
use crate::records::{DspState, FilterParams};

/// An import block: its `IMPORT_*` bit and the name rows carry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Block {
    pub(crate) bit: u32,
    pub(crate) key: &'static str,
}

const EQ: Block = Block {
    bit: IMPORT_EQ,
    key: "eq",
};
const CROSSOVER: Block = Block {
    bit: IMPORT_CROSSOVER,
    key: "crossover",
};
const ROUTING: Block = Block {
    bit: IMPORT_ROUTING,
    key: "routing",
};
const OUTPUTS: Block = Block {
    bit: IMPORT_OUTPUTS,
    key: "outputs",
};
const LIMITERS: Block = Block {
    bit: IMPORT_LIMITERS,
    key: "limiters",
};
const EFFECTS: Block = Block {
    bit: IMPORT_EFFECTS,
    key: "effects",
};
const VOLUMES: Block = Block {
    bit: IMPORT_VOLUMES,
    key: "volumes",
};
const HARDWARE: Block = Block {
    bit: IMPORT_HARDWARE,
    key: "hardware",
};
const NAMES: Block = Block {
    bit: IMPORT_NAMES,
    key: "names",
};

/// Every block, in the order a preview lists them.
pub(crate) const BLOCKS: [Block; 9] = [
    EQ, CROSSOVER, ROUTING, OUTPUTS, LIMITERS, EFFECTS, VOLUMES, HARDWARE, NAMES,
];

/// One changed value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Row {
    pub(crate) block: Block,
    pub(crate) section: &'static str,
    pub(crate) item: String,
    pub(crate) before: String,
    pub(crate) after: String,
}

/// The rows for going from `a` to `b`. `sources`: the user's input source
/// names ("" for none). Channel labels are `b`'s.
pub(crate) fn rows(a: &DspState, b: &DspState, sources: &[String]) -> Vec<Row> {
    let mut d = Diff {
        rows: Vec::new(),
        a,
        b,
        block: EQ,
        section: "",
    };
    d.eq();
    d.crossover();
    d.routing(sources);
    d.output_strips();
    d.limiters();
    d.effects();
    d.volumes();
    d.hardware();
    d.names();
    d.rows
}

/// The diff as JSON: `rows` (`{block, section, item, before, after}` each),
/// `blocks` (the number of rows per block, every block named), `summary`
/// (the unsaved-changes list's sentences) and `count`.
pub(crate) fn to_json(rows: &[Row], summary: Vec<String>) -> Map<String, Value> {
    let mut blocks = Map::new();
    for block in BLOCKS {
        let n = rows.iter().filter(|r| r.block == block).count();
        blocks.insert(block.key.into(), json!(n));
    }
    let mut out = Map::new();
    out.insert(
        "rows".into(),
        Value::Array(
            rows.iter()
                .map(|r| {
                    json!({
                        "block": r.block.key,
                        "section": r.section,
                        "item": r.item,
                        "before": r.before,
                        "after": r.after,
                    })
                })
                .collect(),
        ),
    );
    out.insert("count".into(), json!(rows.len()));
    out.insert("blocks".into(), Value::Object(blocks));
    out.insert("summary".into(), json!(summary));
    out
}

/// A band in a row: "Bell 1000 Hz, Q 0.71, +3.0 dB"; "Off" for a flat one.
pub(crate) fn band_text(p: &FilterParams) -> String {
    let t = FilterType::from_raw(p.filter_type);
    if t == FilterType::Flat {
        return "Off".to_owned();
    }
    let mut text = format!("{} {} Hz, Q {:.2}", t.display_name(), plain(p.freq), p.q);
    if t.is_linkwitz() {
        text.push_str(&format!(", target Q {:.2}", p.qp));
    }
    if t.uses_gain() {
        text.push_str(&format!(", {}", db(p.gain)));
    }
    if p.bypass {
        text.push_str(" (bypassed)");
    }
    text
}

fn gpio(p: u8) -> String {
    format!("GPIO {p}")
}

fn mask16(m: u16) -> String {
    format!("0x{m:04X}")
}

fn mask8(m: u8) -> String {
    format!("0x{m:02X}")
}

/// A crosspoint: "on, -3.0 dB, inverted", "off".
fn crosspoint((on, gain, invert): (bool, f32, bool)) -> String {
    if !on {
        return "off".to_owned();
    }
    let mut text = format!("on, {}", db(gain));
    if invert {
        text.push_str(", inverted");
    }
    text
}

struct Diff<'a> {
    rows: Vec<Row>,
    a: &'a DspState,
    b: &'a DspState,
    block: Block,
    section: &'static str,
}

impl Diff<'_> {
    fn at(&mut self, block: Block, section: &'static str) {
        self.block = block;
        self.section = section;
    }

    fn push(&mut self, item: impl Into<String>, before: String, after: String) {
        self.rows.push(Row {
            block: self.block,
            section: self.section,
            item: item.into(),
            before,
            after,
        });
    }

    fn num(&mut self, item: impl Into<String>, a: f32, b: f32, show: impl Fn(f32) -> String) {
        if !same(a, b) {
            self.push(item, show(a), show(b));
        }
    }

    fn unit(&mut self, item: &str, a: f32, b: f32, unit: &str) {
        self.num(item, a, b, |v| format!("{}{unit}", plain(v)));
    }

    fn flag(&mut self, item: impl Into<String>, a: bool, b: bool) {
        if a != b {
            self.push(item, enabled(a).to_owned(), enabled(b).to_owned());
        }
    }

    fn text(&mut self, item: impl Into<String>, a: String, b: String) {
        if a != b {
            self.push(item, a, b);
        }
    }

    fn label(&self, ch: usize) -> String {
        channel_label(self.b, ch).unwrap_or_else(|| format!("Ch {ch}"))
    }

    fn inputs(&self) -> usize {
        usize::from(self.b.num_input_channels)
    }

    fn outputs(&self) -> usize {
        usize::from(self.b.num_output_channels)
    }

    fn bank(&mut self, ch: usize, of: fn(&DspState) -> &Vec<Vec<FilterParams>>, width: usize) {
        let empty = Vec::new();
        let (x, y) = (
            of(self.a).get(ch).unwrap_or(&empty),
            of(self.b).get(ch).unwrap_or(&empty),
        );
        let name = self.label(ch);
        for k in 0..width {
            let (p, q) = (x.get(k), y.get(k));
            let differs = match (p, q) {
                (Some(p), Some(q)) => !same_band(p, q),
                (None, None) => false,
                _ => true,
            };
            if differs {
                let show = |b: Option<&FilterParams>| b.map_or_else(|| "—".to_owned(), band_text);
                self.push(
                    format!("{name} band {}", k.saturating_add(1)),
                    show(p),
                    show(q),
                );
            }
        }
    }

    fn eq(&mut self) {
        self.at(EQ, "EQ");
        if self.a.bypass != self.b.bypass {
            self.push(
                "Master EQ bypass",
                on_off(self.a.bypass).to_owned(),
                on_off(self.b.bypass).to_owned(),
            );
        }
        let total = self.inputs().saturating_add(self.outputs());
        for ch in 0..total {
            self.bank(ch, |s| &s.filters, BANDS_PER_CHANNEL);
        }
    }

    fn crossover(&mut self) {
        self.at(CROSSOVER, "Crossover");
        let first = self.inputs();
        for ch in first..first.saturating_add(self.outputs()) {
            self.bank(ch, |s| &s.xover, MAX_XOVER_BANDS);
        }
    }

    fn routing(&mut self, sources: &[String]) {
        const LABELS: [&str; 8] = ["L", "R", "FC", "LFE", "BL", "BR", "SL", "SR"];
        let (a, b) = (self.a, self.b);
        self.at(ROUTING, "Inputs");
        for ch in 0..self.inputs() {
            let label = LABELS.get(ch).map_or_else(
                || format!("In {}", ch.saturating_add(1)),
                |l| (*l).to_owned(),
            );
            let at = |s: &DspState| s.input_preamp_db.get(ch).copied().unwrap_or(0.0);
            self.num(format!("Preamp {label}"), at(a), at(b), db);
        }
        if a.input_source != b.input_source {
            let others_on = b.spdif_inputs_enabled & 0x0E != 0;
            let title = |s: u8| source_title(s, sources, others_on);
            self.push("Input source", title(a.input_source), title(b.input_source));
        }
        self.flag(
            "LG Sound Sync",
            a.lg_sound_sync_enabled,
            b.lg_sound_sync_enabled,
        );
        for ch in 0..self.inputs() {
            let at = |s: &DspState| s.channel_delays.get(ch).copied().unwrap_or(0.0);
            let name = self.label(ch);
            self.num(format!("{name} delay"), at(a), at(b), |v| {
                format!("{} ms", delay(v))
            });
        }
        self.at(ROUTING, "Matrix");
        let first = self.inputs();
        for i in 0..self.inputs() {
            for o in 0..self.outputs() {
                let point = |s: &DspState| {
                    (
                        cell(&s.matrix_routing, i, o).copied().unwrap_or(false),
                        cell(&s.matrix_gain, i, o).copied().unwrap_or(0.0),
                        cell(&s.matrix_invert, i, o).copied().unwrap_or(false),
                    )
                };
                let (x, y) = (point(a), point(b));
                if x.0 != y.0 || !same(x.1, y.1) || x.2 != y.2 {
                    let item = format!(
                        "{} → {}",
                        self.label(i),
                        self.label(first.saturating_add(o))
                    );
                    self.push(item, crosspoint(x), crosspoint(y));
                }
            }
        }
    }

    fn output_strips(&mut self) {
        let (a, b) = (self.a, self.b);
        self.at(OUTPUTS, "Outputs");
        let first = self.inputs();
        for o in 0..self.outputs() {
            let wire = first.saturating_add(o);
            let name = self.label(wire);
            let flag = |v: &[bool]| v.get(o).copied().unwrap_or(false);
            let num = |v: &[f32]| v.get(o).copied().unwrap_or(0.0);
            self.flag(
                name.clone(),
                flag(&a.output_enabled),
                flag(&b.output_enabled),
            );
            let (ma, mb) = (flag(&a.output_muted), flag(&b.output_muted));
            if ma != mb {
                let word = |m: bool| if m { "muted" } else { "unmuted" }.to_owned();
                self.push(format!("{name} mute"), word(ma), word(mb));
            }
            self.num(
                format!("{name} gain"),
                num(&a.output_gain_db),
                num(&b.output_gain_db),
                db,
            );
            let ms = |v: f32| format!("{} ms", delay(v));
            let (da, dbl) = (num(&a.output_delay_ms), num(&b.output_delay_ms));
            self.num(format!("{name} delay"), da, dbl, ms);
            // The channel's delay is the output's too (presets.rs, SWAPFIX):
            // said once.
            if same(da, dbl) {
                let ch = |s: &DspState| s.channel_delays.get(wire).copied().unwrap_or(0.0);
                self.num(format!("{name} delay"), ch(a), ch(b), ms);
            }
        }
    }

    fn limiters(&mut self) {
        let (a, b) = (self.a, self.b);
        self.at(LIMITERS, "Limiters");
        let first = self.inputs();
        for o in 0..self.outputs() {
            let name = self.label(first.saturating_add(o));
            let flag = |v: &[bool]| v.get(o).copied().unwrap_or(false);
            let num = |v: &[f32]| v.get(o).copied().unwrap_or(0.0);
            let group = |v: &[u8]| v.get(o).copied().unwrap_or(0);
            self.flag(
                format!("{name} limiter"),
                flag(&a.limiter_enabled),
                flag(&b.limiter_enabled),
            );
            self.num(
                format!("{name} limiter threshold"),
                num(&a.limiter_threshold_db),
                num(&b.limiter_threshold_db),
                |v| format!("{v:.1} dBFS"),
            );
            self.num(
                format!("{name} limiter release"),
                num(&a.limiter_release_ms),
                num(&b.limiter_release_ms),
                |v| format!("{} ms", plain(v)),
            );
            self.text(
                format!("{name} limiter link"),
                dspi_session::state::limiter_link_group_name(group(&a.limiter_link_group)),
                dspi_session::state::limiter_link_group_name(group(&b.limiter_link_group)),
            );
        }
    }

    fn effects(&mut self) {
        self.loudness_and_crossfeed();
        self.leveller();
        self.psybass();
        self.upmix();
        self.subharm();
        self.tube();
    }

    fn loudness_and_crossfeed(&mut self) {
        let (a, b) = (self.a, self.b);
        self.at(EFFECTS, "Loudness");
        self.flag("Loudness", a.loudness_enabled, b.loudness_enabled);
        self.text(
            "Loudness outputs",
            mask16(a.loudness_output_mask),
            mask16(b.loudness_output_mask),
        );
        self.unit(
            "Loudness ref SPL",
            a.loudness_ref_spl,
            b.loudness_ref_spl,
            "",
        );
        self.unit(
            "Loudness intensity",
            a.loudness_intensity,
            b.loudness_intensity,
            "%",
        );

        self.at(EFFECTS, "Crossfeed");
        self.flag("Crossfeed", a.crossfeed_enabled, b.crossfeed_enabled);
        self.text(
            "Crossfeed preset",
            a.crossfeed_preset.to_string(),
            b.crossfeed_preset.to_string(),
        );
        self.unit(
            "Crossfeed frequency",
            a.crossfeed_freq,
            b.crossfeed_freq,
            " Hz",
        );
        self.unit("Crossfeed feed", a.crossfeed_feed, b.crossfeed_feed, "");
        self.flag("Crossfeed ITD", a.crossfeed_itd, b.crossfeed_itd);
        self.text(
            "Crossfeed output pairs",
            mask8(a.crossfeed_output_pair_mask),
            mask8(b.crossfeed_output_pair_mask),
        );
        for p in 0..CROSSFEED_MAP_PAIRS {
            let show = |m: &[u8]| {
                let (l, r) = pair_outputs(m, p);
                format!("{}/{}", l.saturating_add(1), r.saturating_add(1))
            };
            self.text(
                format!("Crossfeed pair {} outputs", p.saturating_add(1)),
                show(&a.crossfeed_pair_map),
                show(&b.crossfeed_pair_map),
            );
        }
    }

    fn leveller(&mut self) {
        let (a, b) = (self.a, self.b);
        self.at(EFFECTS, "Volume Leveller");
        self.flag("Volume Leveller", a.leveller_enabled, b.leveller_enabled);
        self.unit("Leveller amount", a.leveller_amount, b.leveller_amount, "%");
        let speed = |v: u8| {
            ["Slow", "Medium", "Fast"]
                .get(usize::from(v))
                .map_or_else(|| v.to_string(), |n| (*n).to_owned())
        };
        self.text(
            "Leveller speed",
            speed(a.leveller_speed),
            speed(b.leveller_speed),
        );
        self.unit(
            "Leveller max gain",
            a.leveller_max_gain_db,
            b.leveller_max_gain_db,
            " dB",
        );
        self.flag(
            "Leveller lookahead",
            a.leveller_lookahead,
            b.leveller_lookahead,
        );
        self.unit(
            "Leveller gate",
            a.leveller_gate_db,
            b.leveller_gate_db,
            " dB",
        );
        self.text(
            "Leveller detector channels",
            mask8(a.leveller_detector_mask),
            mask8(b.leveller_detector_mask),
        );
        self.text(
            "Leveller apply channels",
            mask8(a.leveller_apply_mask),
            mask8(b.leveller_apply_mask),
        );
    }

    fn psybass(&mut self) {
        let (a, b) = (self.a, self.b);
        self.at(EFFECTS, "Psychoacoustic Bass");
        self.flag("Psychoacoustic Bass", a.psybass_enabled, b.psybass_enabled);
        self.text(
            "Psybass outputs",
            mask16(a.psybass_output_mask),
            mask16(b.psybass_output_mask),
        );
        self.unit(
            "Psybass cutoff",
            a.psybass_cutoff_hz,
            b.psybass_cutoff_hz,
            " Hz",
        );
        self.unit(
            "Psybass harmonics",
            a.psybass_harmonics_db,
            b.psybass_harmonics_db,
            " dB",
        );
        self.unit(
            "Psybass drive",
            a.psybass_drive_db,
            b.psybass_drive_db,
            " dB",
        );
        self.unit(
            "Psybass character",
            a.psybass_character_pct,
            b.psybass_character_pct,
            "%",
        );
        self.unit(
            "Psybass original bass",
            a.psybass_original_db,
            b.psybass_original_db,
            " dB",
        );
    }

    fn upmix(&mut self) {
        let (a, b) = (&self.a.upmix, &self.b.upmix);
        self.at(EFFECTS, "Stereo Upmixer");
        self.flag(
            "Stereo Upmixer",
            param(a, UPMIX_PARAM_ENABLED) != 0.0,
            param(b, UPMIX_PARAM_ENABLED) != 0.0,
        );
        let modes: [(u8, &str, [&str; 3]); 2] = [
            (
                UPMIX_PARAM_CENTER_MODE,
                "Centre mode",
                ["Sinner", "Logician", "Off"],
            ),
            (
                UPMIX_PARAM_SURROUND_MODE,
                "Surround mode",
                ["Off", "Sinner", "Logician"],
            ),
        ];
        for (id, what, names) in modes {
            let name = |v: f32| {
                let m = v as i64;
                usize::try_from(m)
                    .ok()
                    .and_then(|i| names.get(i))
                    .map_or_else(|| m.to_string(), |n| (*n).to_owned())
            };
            self.text(what, name(param(a, id)), name(param(b, id)));
        }
        for (id, what, unit) in [
            (UPMIX_PARAM_STRENGTH, "Centre strength", "%"),
            (UPMIX_PARAM_WIDTH, "Centre width", "%"),
            (UPMIX_PARAM_THRESHOLD, "Correlation threshold", "%"),
            (UPMIX_PARAM_ATTACK, "Centre attack", " ms"),
            (UPMIX_PARAM_RELEASE, "Centre release", " ms"),
            (UPMIX_PARAM_DETECTOR_HPF, "Detector HPF", " Hz"),
            (UPMIX_PARAM_SURROUND_DELAY, "Surround delay", " ms"),
            (UPMIX_PARAM_SURROUND_HPF, "Surround HPF", " Hz"),
            (UPMIX_PARAM_SURROUND_LPF, "Surround LPF", " Hz"),
            (UPMIX_PARAM_DECORRELATION, "Decorrelation", "%"),
            (UPMIX_PARAM_PRESENCE, "Centre presence", " dB"),
        ] {
            self.unit(what, param(a, id), param(b, id), unit);
        }
    }

    fn subharm(&mut self) {
        let (a, b) = (&self.a.subharm, &self.b.subharm);
        self.at(EFFECTS, "Subharmonic Synthesizer");
        self.flag(
            "Subharmonic Synthesizer",
            param(a, SUBHARM_PARAM_ENABLED) != 0.0,
            param(b, SUBHARM_PARAM_ENABLED) != 0.0,
        );
        self.text(
            "Subharm outputs",
            mask16(param(a, SUBHARM_PARAM_MASK) as u16),
            mask16(param(b, SUBHARM_PARAM_MASK) as u16),
        );
        for (id, band) in [
            (SUBHARM_PARAM_LOW, "24-36 Hz"),
            (SUBHARM_PARAM_HIGH, "36-56 Hz"),
            (SUBHARM_PARAM_TOP, "56-80 Hz"),
        ] {
            self.num(
                format!("Subharm {band}"),
                param(a, id),
                param(b, id),
                band_level,
            );
        }
        self.text(
            "Subharm selectivity",
            selectivity(param(a, SUBHARM_PARAM_SELECT) as i64).to_owned(),
            selectivity(param(b, SUBHARM_PARAM_SELECT) as i64).to_owned(),
        );
        self.unit(
            "Subharm selectivity depth",
            param(a, SUBHARM_PARAM_DEPTH),
            param(b, SUBHARM_PARAM_DEPTH),
            "%",
        );
        self.unit(
            "Subharm selectivity hold",
            param(a, SUBHARM_PARAM_HOLD),
            param(b, SUBHARM_PARAM_HOLD),
            " ms",
        );
        self.num(
            "Subharm sub ceiling",
            param(a, SUBHARM_PARAM_CEILING),
            param(b, SUBHARM_PARAM_CEILING),
            ceiling,
        );
        let link = |v: f32| if v != 0.0 { "linked" } else { "independent" }.to_owned();
        self.text(
            "Subharm pair link",
            link(param(a, SUBHARM_PARAM_LINK)),
            link(param(b, SUBHARM_PARAM_LINK)),
        );
        self.unit(
            "Subharm LF boost",
            param(a, SUBHARM_PARAM_BOOST),
            param(b, SUBHARM_PARAM_BOOST),
            " dB",
        );
    }

    fn tube(&mut self) {
        use g::tube as t;
        let (a, b) = (&self.a.tube, &self.b.tube);
        let at = |v: &[f32], id: u16| param(v, u8::try_from(id).unwrap_or(u8::MAX));
        self.at(EFFECTS, "Tube Modeller");
        self.flag(
            "Tube Modeller",
            at(a, t::TUBE_PARAM_ENABLED) != 0.0,
            at(b, t::TUBE_PARAM_ENABLED) != 0.0,
        );
        self.text(
            "Tube outputs",
            mask16(at(a, t::TUBE_PARAM_OUTPUT_MASK) as u16),
            mask16(at(b, t::TUBE_PARAM_OUTPUT_MASK) as u16),
        );
        self.text(
            "Tube type",
            dspi_session::tube::type_name(at(a, t::TUBE_PARAM_TUBE_TYPE) as u8),
            dspi_session::tube::type_name(at(b, t::TUBE_PARAM_TUBE_TYPE) as u8),
        );
        for (id, what, unit) in [
            (t::TUBE_PARAM_DRIVE_DB, "Tube drive", " dB"),
            (t::TUBE_PARAM_BIAS_PCT, "Tube bias", "%"),
            (t::TUBE_PARAM_ASYM_DB, "Tube asymmetry", " dB"),
            (t::TUBE_PARAM_HARDNESS_PCT, "Tube knee hardness", "%"),
            (t::TUBE_PARAM_SAG_PCT, "Tube sag", "%"),
        ] {
            self.unit(what, at(a, id), at(b, id), unit);
        }
        self.text(
            "Tube rectifier",
            dspi_session::tube::rectifier_name(at(a, t::TUBE_PARAM_RECTIFIER) as u8),
            dspi_session::tube::rectifier_name(at(b, t::TUBE_PARAM_RECTIFIER) as u8),
        );
        self.flag(
            "Tube output stage",
            at(a, t::TUBE_PARAM_XFMR_ENABLED) != 0.0,
            at(b, t::TUBE_PARAM_XFMR_ENABLED) != 0.0,
        );
        self.unit(
            "Tube damping factor",
            at(a, t::TUBE_PARAM_XFMR_DAMPING),
            at(b, t::TUBE_PARAM_XFMR_DAMPING),
            "",
        );
        self.unit(
            "Tube speaker resonance",
            at(a, t::TUBE_PARAM_XFMR_RES_HZ),
            at(b, t::TUBE_PARAM_XFMR_RES_HZ),
            " Hz",
        );
        self.unit(
            "Tube mix",
            at(a, t::TUBE_PARAM_MIX_PCT),
            at(b, t::TUBE_PARAM_MIX_PCT),
            "%",
        );
        self.unit(
            "Tube output trim",
            at(a, t::TUBE_PARAM_TRIM_DB),
            at(b, t::TUBE_PARAM_TRIM_DB),
            " dB",
        );
    }

    fn volumes(&mut self) {
        let (a, b) = (self.a, self.b);
        self.at(VOLUMES, "Volume");
        self.num(
            "Master Volume",
            a.master_volume_db,
            b.master_volume_db,
            volume,
        );
        self.num("User Volume", a.user_volume_db, b.user_volume_db, volume);
    }

    fn pins(&mut self, item: &str, a: &[u8], b: &[u8], width: usize) {
        for (k, (x, y)) in a.iter().zip(b).take(width).enumerate() {
            if x != y {
                self.push(
                    format!("{item} {}", k.saturating_add(1)),
                    gpio(*x),
                    gpio(*y),
                );
            }
        }
    }

    fn hardware(&mut self) {
        let (a, b) = (self.a, self.b);
        self.at(HARDWARE, "Hardware I/O");
        let pins = usize::from(b.num_pin_outputs);
        self.pins("Output pin", &a.output_pins, &b.output_pins, pins);
        let kind = |t: u8| if t == 1 { "I2S" } else { "S/PDIF" }.to_owned();
        for (k, (x, y)) in a.output_types.iter().zip(&b.output_types).enumerate() {
            self.text(
                format!("Output slot {} type", k.saturating_add(1)),
                kind(*x),
                kind(*y),
            );
        }
        self.text("BCK pin", gpio(a.i2s_bck_pin), gpio(b.i2s_bck_pin));
        self.text(
            "BCK pin (slave)",
            gpio(a.i2s_bck_pin_slave),
            gpio(b.i2s_bck_pin_slave),
        );
        self.text(
            "I2S clock mode",
            clock_mode(a.i2s_clock_mode).to_owned(),
            clock_mode(b.i2s_clock_mode).to_owned(),
        );
        self.text(
            "I2S clock pin mode",
            a.i2s_clock_pin_mode.to_string(),
            b.i2s_clock_pin_mode.to_string(),
        );
        self.flag("MCK", a.mck_enabled, b.mck_enabled);
        self.text("MCK pin", gpio(a.mck_pin), gpio(b.mck_pin));
        self.text(
            "MCK multiplier",
            format!("{}x", mck_multiplier(a.mck_multiplier)),
            format!("{}x", mck_multiplier(b.mck_multiplier)),
        );
        self.pins("S/PDIF RX pin", &a.spdif_rx_pins, &b.spdif_rx_pins, 4);
        for k in 1..4u32 {
            let on = |m: u8| m & 1u8.checked_shl(k).unwrap_or(0) != 0;
            self.flag(
                format!("S/PDIF {} input", k.saturating_add(1)),
                on(a.spdif_inputs_enabled),
                on(b.spdif_inputs_enabled),
            );
        }
        self.text(
            "I2S input channels",
            a.i2s_input_channels.to_string(),
            b.i2s_input_channels.to_string(),
        );
        self.pins("I2S RX pin, pair", &a.i2s_rx_pins, &b.i2s_rx_pins, 4);
        self.text(
            "I2S rate",
            format!("{} Hz", rate_hz(a.i2s_input_rate)),
            format!("{} Hz", rate_hz(b.i2s_input_rate)),
        );
        self.flag("ADAT output", a.adat_out_enabled, b.adat_out_enabled);
        self.text("ADAT pin", gpio(a.adat_out_pin), gpio(b.adat_out_pin));
        self.flag("ADAT input", a.adat_input_enabled, b.adat_input_enabled);
        let pin = |p: u8| {
            if p == PIN_UNSET {
                "unset".to_owned()
            } else {
                gpio(p)
            }
        };
        self.text(
            "ADAT input pin",
            pin(a.adat_input_pin),
            pin(b.adat_input_pin),
        );
        self.text(
            "ADAT input clock mode",
            clock_mode(a.adat_input_clock_mode).to_owned(),
            clock_mode(b.adat_input_clock_mode).to_owned(),
        );
        self.flag("External DAC mute", a.dac_mute_enabled, b.dac_mute_enabled);
        let level = |low: bool| if low { "active low" } else { "active high" }.to_owned();
        self.text(
            "External DAC mute level",
            level(a.dac_mute_active_low),
            level(b.dac_mute_active_low),
        );
        self.text(
            "External DAC mute pin",
            gpio(a.dac_mute_pin),
            gpio(b.dac_mute_pin),
        );
        self.text(
            "External DAC mute hold",
            format!("{} ms", a.dac_mute_hold_ms),
            format!("{} ms", b.dac_mute_hold_ms),
        );
        self.text(
            "External DAC mute release",
            format!("{} ms", a.dac_mute_release_ms),
            format!("{} ms", b.dac_mute_release_ms),
        );
    }

    fn names(&mut self) {
        let (a, b) = (self.a, self.b);
        self.at(NAMES, "Names");
        let total = self.inputs().saturating_add(self.outputs());
        for ch in 0..total {
            let at = |s: &DspState| s.channel_names.get(ch).cloned().unwrap_or_default();
            let (x, y) = (at(a), at(b));
            if x != y {
                let item = format!("Ch {ch}");
                self.push(item, format!("'{x}'"), format!("'{y}'"));
            }
        }
    }
}
