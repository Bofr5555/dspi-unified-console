//! The effects tools (WP MCP2b): one get for all of them, one set per
//! effect (loudness, crossfeed, leveller, psychoacoustic bass, subharmonic
//! synth, tube, upmixer, limiter), and the effects' live reads.
//!
//! Each set tool takes any of its effect's parameters, plans them as the
//! core's setters will write them ([`Step`]), and the policy judges the whole
//! call first (`policy::prejudge`): turning on an effect that can add level,
//! or raising its level while it is on, is risky, as is turning a limiter
//! off or raising its ceiling (`policy`'s effect rules). The steps then run
//! as one undo group, put back if one fails ([`run_steps`]). The crossfeed's
//! output pair map is a raw write the core's hook does not see: it has the
//! static class `policy::require_change`.
//!
//! [`run_steps`] is shared with the other MCP2b tools that write through the
//! core's typed setters.

use dspi_core::consts::{
    PSYBASS_PARAM_CHARACTER, PSYBASS_PARAM_CUTOFF, PSYBASS_PARAM_DRIVE, PSYBASS_PARAM_HARMONICS,
    PSYBASS_PARAM_ORIGINAL, SUBHARM_PARAM_BOOST, SUBHARM_PARAM_CEILING, SUBHARM_PARAM_DEPTH,
    SUBHARM_PARAM_ENABLED, SUBHARM_PARAM_HIGH, SUBHARM_PARAM_HOLD, SUBHARM_PARAM_LINK,
    SUBHARM_PARAM_LOW, SUBHARM_PARAM_MASK, SUBHARM_PARAM_SELECT, SUBHARM_PARAM_TOP,
    UPMIX_PARAM_ATTACK, UPMIX_PARAM_CENTER_MODE, UPMIX_PARAM_DECORRELATION,
    UPMIX_PARAM_DETECTOR_HPF, UPMIX_PARAM_ENABLED, UPMIX_PARAM_PRESENCE, UPMIX_PARAM_RELEASE,
    UPMIX_PARAM_STRENGTH, UPMIX_PARAM_SURROUND_DELAY, UPMIX_PARAM_SURROUND_HPF,
    UPMIX_PARAM_SURROUND_LPF, UPMIX_PARAM_SURROUND_MODE, UPMIX_PARAM_THRESHOLD, UPMIX_PARAM_WIDTH,
};
use dspi_core::{DspState, DspiCore, ParamValue};
use dspi_proto::registry::{Kind as RegKind, by_path};
use serde_json::{Map, Value, json};

use crate::policy::{self, WriteKey};
use crate::registry::{Call, Tool, ToolClass, ToolError, ToolResult};
use crate::tools::edit::{newest, own_ids_since, schema, write_result_schema};
use crate::tools::read::{empty_schema, num, object_schema, state};

/// The effects tools, in `tools/list` order.
pub static TOOLS: &[&Tool] = &[
    &GET_EFFECTS,
    &SET_LOUDNESS,
    &SET_CROSSFEED,
    &SET_LEVELLER,
    &SET_PSYBASS,
    &SET_SUBHARMONIC,
    &SET_TUBE,
    &SET_UPMIXER,
    &SET_LIMITER,
    &GET_LIMITER_METER,
    &GET_UPMIX_STATUS,
    &GET_SUBHARM_HEADROOM,
];

// ------------------------------------------------------------- the steps

/// A typed setter call: the setter's effect on `DspiCore`.
pub(crate) type Setter = Box<dyn Fn(&DspiCore) -> dspi_core::Result<()>>;

/// One setter call and what the core's hook will see of it.
pub(crate) struct Step {
    /// The values it writes (path and indices are what the hook matches).
    pub(crate) keys: Vec<WriteKey>,
    /// The call.
    pub(crate) run: Setter,
}

/// Judge `keys` on `before` together with a static class's outcome
/// (`policy::require` or `require_change`, already called), so one
/// confirmation asks about both: a refusal wins, then a hold.
pub(crate) fn judge_with(
    call: &Call<'_>,
    before: &DspState,
    keys: &[WriteKey],
    statics: Result<(), ToolError>,
) -> Result<(), ToolError> {
    let judged = policy::prejudge(&call.judging, before, keys);
    let refused = policy::lock(&call.judging).refused.is_some();
    match (statics, judged) {
        (_, Err(e)) if refused => Err(e),
        (Err(e), _) | (Ok(()), Err(e)) => Err(e),
        (Ok(()), Ok(())) => Ok(()),
    }
}

/// Judge `steps` (planned on `before`) whole, with `statics` (a static
/// class's outcome, [`judge_with`]), then run them: one undo group labelled
/// `label` for several, everything the call wrote put back if one fails (as
/// `edit::execute`). `changes` goes into the result as it is.
pub(crate) fn run_steps(
    call: &mut Call<'_>,
    before: &DspState,
    steps: Vec<Step>,
    label: &str,
    changes: Value,
    statics: Result<(), ToolError>,
) -> ToolResult {
    if steps.is_empty() {
        return Err(ToolError::new("nothing to change: pass at least one value"));
    }
    let keys: Vec<WriteKey> = steps.iter().flat_map(|s| s.keys.clone()).collect();
    judge_with(call, before, &keys, statics)?;
    let core = call.core()?;
    let first = newest(core);
    let group = steps.len() > 1;
    if group {
        if let Some(owner) = core.undo_group_owner()
            && owner != call.actor
        {
            return Err(ToolError::new(
                "the user is in the middle of a change on this device; nothing was changed. \
                 Try again in a moment.",
            ));
        }
        core.begin_group(label.to_owned());
    }
    let mut failed = None;
    for s in &steps {
        if let Err(e) = (s.run)(core) {
            failed = Some(e);
            break;
        }
    }
    if group {
        core.end_group();
    }
    let ids = own_ids_since(core, call.actor, first);
    if let Some(e) = failed {
        let err = policy::core_error(&call.judging, e);
        if ids.is_empty() {
            return Err(err);
        }
        let mut undone = true;
        for id in ids.iter().rev() {
            undone &= core.revert_entry(*id).is_ok_and(|m| m.entry.is_some());
        }
        return Err(ToolError::new(if undone {
            format!(
                "{} The changes this call made before it were put back.",
                err.0
            )
        } else {
            format!(
                "{} Some changes this call made before it could not be put back; \
                 read the device again (dspi_get_effects).",
                err.0
            )
        }));
    }
    Ok(json!({
        "ok": true,
        "device": call.device_text(),
        "changes": changes,
        "undo_id": ids.last(),
        "undo_ids": ids,
    }))
}

// ------------------------------------------------------------ the tables

/// How a parameter is passed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// true or false.
    Bool,
    /// A number; the core clamps it to the device's range.
    Number,
    /// A whole number (a choice or a mode).
    Int,
    /// A list of outputs (from 0), sent as a bit mask.
    Outputs,
}

/// One parameter of an effect.
struct Param {
    /// The argument's name.
    arg: &'static str,
    /// The registry path the core's hook sees.
    path: &'static str,
    /// The setter's parameter id.
    id: u8,
    kind: Kind,
    /// For the schema: unit and range in words.
    desc: &'static str,
}

/// One effect's set tool.
struct Effect {
    /// The effect, in the result and History label.
    word: &'static str,
    /// It is per output (the limiter): `output` (or "all") picks which.
    per_output: bool,
    params: &'static [Param],
    /// The setter: (core, output, id, value).
    set: fn(&DspiCore, u8, u8, f32) -> dspi_core::Result<()>,
}

const fn p(arg: &'static str, path: &'static str, id: u8, kind: Kind, desc: &'static str) -> Param {
    Param {
        arg,
        path,
        id,
        kind,
        desc,
    }
}

static LOUDNESS: Effect = Effect {
    word: "loudness",
    per_output: false,
    params: &[
        p(
            "reference_spl",
            "loud.ref",
            1,
            Kind::Number,
            "dB SPL, 40..100",
        ),
        p(
            "intensity_pct",
            "loud.intensity",
            2,
            Kind::Number,
            "%, 0..200",
        ),
        p(
            "outputs",
            "loud.mask",
            3,
            Kind::Outputs,
            "the outputs it processes",
        ),
        p(
            "enabled",
            "loud.on",
            0,
            Kind::Bool,
            "on or off; adds level when on",
        ),
    ],
    set: |core, _, id, v| match id {
        0 => core.set_loudness(v != 0.0),
        1 => core.set_loudness_ref(v),
        2 => core.set_loudness_intensity(v),
        _ => core.set_loudness_mask(mask16(v)),
    },
};

static CROSSFEED: Effect = Effect {
    word: "crossfeed",
    per_output: false,
    params: &[
        p("preset", "cf.preset", 1, Kind::Int, "0..3 (3 is custom)"),
        p(
            "freq_hz",
            "cf.freq",
            2,
            Kind::Number,
            "Hz, the custom preset's cut-off",
        ),
        p(
            "feed_db",
            "cf.feed",
            3,
            Kind::Number,
            "dB, how much crosses over (custom preset)",
        ),
        p("itd", "cf.itd", 4, Kind::Bool, "interaural time delay"),
        p(
            "output_pairs",
            "cf.outputs",
            5,
            Kind::Int,
            "bit mask of output pairs",
        ),
        p("enabled", "cf.on", 0, Kind::Bool, "on or off"),
    ],
    set: |core, _, id, v| match id {
        0 => core.set_crossfeed(v != 0.0),
        1 => core.set_crossfeed_preset(byte(v)),
        2 => core.set_crossfeed_freq(v),
        3 => core.set_crossfeed_feed(v),
        4 => core.set_crossfeed_itd(v != 0.0),
        _ => core.set_crossfeed_outputs(byte(v)),
    },
};

static LEVELLER: Effect = Effect {
    word: "leveller",
    per_output: false,
    params: &[
        p("amount_pct", "lev.amount", 1, Kind::Number, "%, 0..100"),
        p(
            "speed",
            "lev.speed",
            2,
            Kind::Int,
            "0 slow, 1 medium, 2 fast",
        ),
        p(
            "max_gain_db",
            "lev.maxgain",
            3,
            Kind::Number,
            "dB; above 0 it adds level",
        ),
        p("lookahead", "lev.lookahead", 4, Kind::Bool, "look-ahead"),
        p(
            "gate_db",
            "lev.gate",
            5,
            Kind::Number,
            "dB, the gate threshold",
        ),
        p("enabled", "lev.on", 0, Kind::Bool, "on or off"),
    ],
    set: |core, _, id, v| match id {
        0 => core.set_leveller_enabled(v != 0.0),
        1 => core.set_leveller_amount(v),
        2 => core.set_leveller_speed(byte(v)),
        3 => core.set_leveller_max_gain(v),
        4 => core.set_leveller_lookahead(v != 0.0),
        _ => core.set_leveller_gate(v),
    },
};

/// Psychoacoustic bass: the setter's ids 0..4, then the mask (5) and the
/// switch (6), which have setters of their own.
static PSYBASS: Effect = Effect {
    word: "psychoacoustic bass",
    per_output: false,
    params: &[
        p(
            "cutoff_hz",
            "bass.cutoff",
            PSYBASS_PARAM_CUTOFF,
            Kind::Number,
            "Hz, 30..300",
        ),
        p(
            "harmonics_db",
            "bass.harmonics",
            PSYBASS_PARAM_HARMONICS,
            Kind::Number,
            "dB, -24..12",
        ),
        p(
            "drive_db",
            "bass.drive",
            PSYBASS_PARAM_DRIVE,
            Kind::Number,
            "dB, 0..18",
        ),
        p(
            "character_pct",
            "bass.character",
            PSYBASS_PARAM_CHARACTER,
            Kind::Number,
            "%, 0..100",
        ),
        p(
            "original_db",
            "bass.original",
            PSYBASS_PARAM_ORIGINAL,
            Kind::Number,
            "dB, -60..0",
        ),
        p(
            "outputs",
            "bass.mask",
            5,
            Kind::Outputs,
            "the outputs it processes",
        ),
        p(
            "enabled",
            "bass.on",
            6,
            Kind::Bool,
            "on or off; adds level when on",
        ),
    ],
    set: |core, _, id, v| match id {
        5 => core.set_psybass_mask(mask16(v)),
        6 => core.set_psybass_enabled(v != 0.0),
        _ => core.set_psybass_param(id, v),
    },
};

/// The subharmonic synth: the setter's ids, and solo (11), which has a
/// setter of its own.
static SUBHARMONIC: Effect = Effect {
    word: "subharmonic synth",
    per_output: false,
    params: &[
        p(
            "outputs",
            "sub.mask",
            SUBHARM_PARAM_MASK,
            Kind::Outputs,
            "the outputs it feeds",
        ),
        p(
            "low_db",
            "sub.low",
            SUBHARM_PARAM_LOW,
            Kind::Number,
            "dB, the low octave's level",
        ),
        p(
            "high_db",
            "sub.high",
            SUBHARM_PARAM_HIGH,
            Kind::Number,
            "dB, the high octave's level",
        ),
        p(
            "top_db",
            "sub.top",
            SUBHARM_PARAM_TOP,
            Kind::Number,
            "dB, the top octave's level",
        ),
        p(
            "boost_db",
            "sub.boost",
            SUBHARM_PARAM_BOOST,
            Kind::Number,
            "dB",
        ),
        p(
            "select",
            "sub.select",
            SUBHARM_PARAM_SELECT,
            Kind::Int,
            "the select mode",
        ),
        p(
            "depth",
            "sub.depth",
            SUBHARM_PARAM_DEPTH,
            Kind::Number,
            "the depth",
        ),
        p(
            "hold",
            "sub.hold",
            SUBHARM_PARAM_HOLD,
            Kind::Number,
            "the hold",
        ),
        p(
            "ceiling_db",
            "sub.ceiling",
            SUBHARM_PARAM_CEILING,
            Kind::Number,
            "dB",
        ),
        p(
            "link",
            "sub.link",
            SUBHARM_PARAM_LINK,
            Kind::Bool,
            "link the channels",
        ),
        p("solo", "sub.solo", 11, Kind::Bool, "hear the synth alone"),
        p(
            "enabled",
            "sub.on",
            SUBHARM_PARAM_ENABLED,
            Kind::Bool,
            "on or off; adds level when on",
        ),
    ],
    set: |core, _, id, v| match id {
        11 => core.set_subharm_solo(v != 0.0),
        _ => core.set_subharm_param(id, v),
    },
};

/// The tube: the firmware's own indices (tube.h).
static TUBE: Effect = Effect {
    word: "tube",
    per_output: false,
    params: &[
        p(
            "outputs",
            "tube.mask",
            1,
            Kind::Outputs,
            "the outputs it processes",
        ),
        p("type", "tube.type", 2, Kind::Int, "the tube type"),
        p("drive", "tube.drive", 3, Kind::Number, "the drive"),
        p("bias_pct", "tube.bias", 4, Kind::Number, "%"),
        p("asymmetry_db", "tube.asym", 5, Kind::Number, "dB"),
        p("hardness_pct", "tube.hardness", 6, Kind::Number, "%"),
        p("sag_pct", "tube.sag", 7, Kind::Number, "%"),
        p(
            "rectifier",
            "tube.rectifier",
            8,
            Kind::Int,
            "the rectifier type",
        ),
        p(
            "transformer",
            "tube.xfmr",
            9,
            Kind::Bool,
            "the output transformer",
        ),
        p(
            "damping",
            "tube.damping",
            10,
            Kind::Number,
            "the transformer's damping",
        ),
        p(
            "resonance_hz",
            "tube.resonance",
            11,
            Kind::Number,
            "Hz, the transformer's resonance",
        ),
        p("mix_pct", "tube.mix", 12, Kind::Number, "%, wet"),
        p(
            "trim_db",
            "tube.trim",
            13,
            Kind::Number,
            "dB, the output trim",
        ),
        p(
            "enabled",
            "tube.on",
            0,
            Kind::Bool,
            "on or off; adds drive when on",
        ),
    ],
    set: |core, _, id, v| core.set_tube_param(id, v),
};

static UPMIXER: Effect = Effect {
    word: "upmixer",
    per_output: false,
    params: &[
        p(
            "center_mode",
            "up.center_mode",
            UPMIX_PARAM_CENTER_MODE,
            Kind::Int,
            "the centre mode",
        ),
        p(
            "surround_mode",
            "up.surround_mode",
            UPMIX_PARAM_SURROUND_MODE,
            Kind::Int,
            "0..2",
        ),
        p(
            "strength",
            "up.strength",
            UPMIX_PARAM_STRENGTH,
            Kind::Number,
            "the strength",
        ),
        p(
            "width",
            "up.width",
            UPMIX_PARAM_WIDTH,
            Kind::Number,
            "the width",
        ),
        p(
            "threshold",
            "up.threshold",
            UPMIX_PARAM_THRESHOLD,
            Kind::Number,
            "the threshold",
        ),
        p(
            "attack_ms",
            "up.attack",
            UPMIX_PARAM_ATTACK,
            Kind::Number,
            "ms",
        ),
        p(
            "release_ms",
            "up.release",
            UPMIX_PARAM_RELEASE,
            Kind::Number,
            "ms",
        ),
        p(
            "detector_hpf_hz",
            "up.det_hpf",
            UPMIX_PARAM_DETECTOR_HPF,
            Kind::Number,
            "Hz",
        ),
        p(
            "surround_delay_ms",
            "up.sur_delay",
            UPMIX_PARAM_SURROUND_DELAY,
            Kind::Number,
            "ms",
        ),
        p(
            "surround_hpf_hz",
            "up.sur_hpf",
            UPMIX_PARAM_SURROUND_HPF,
            Kind::Number,
            "Hz",
        ),
        p(
            "surround_lpf_hz",
            "up.sur_lpf",
            UPMIX_PARAM_SURROUND_LPF,
            Kind::Number,
            "Hz",
        ),
        p(
            "decorrelation",
            "up.decorr",
            UPMIX_PARAM_DECORRELATION,
            Kind::Number,
            "the decorrelation",
        ),
        p(
            "presence",
            "up.presence",
            UPMIX_PARAM_PRESENCE,
            Kind::Number,
            "the presence",
        ),
        p(
            "enabled",
            "up.on",
            UPMIX_PARAM_ENABLED,
            Kind::Bool,
            "on or off",
        ),
    ],
    set: |core, _, id, v| core.set_upmix_param(id, v),
};

/// Every output, for the limiter (`LIMITER_ALL_OUTPUTS`).
const ALL_OUTPUTS: u8 = 0xFF;

static LIMITER: Effect = Effect {
    word: "limiter",
    per_output: true,
    params: &[
        p(
            "threshold_db",
            "limit.threshold",
            1,
            Kind::Number,
            "dB, the ceiling, -30..0",
        ),
        p(
            "release_ms",
            "limit.release",
            2,
            Kind::Number,
            "ms, 10..1000",
        ),
        p(
            "link_group",
            "limit.link",
            3,
            Kind::Int,
            "0 unlinked, 1..4 a link group",
        ),
        p(
            "enabled",
            "limit.on",
            0,
            Kind::Bool,
            "on or off; off may be held for the user",
        ),
    ],
    set: |core, o, id, v| core.set_limiter_param(o, id, v),
};

fn mask16(v: f32) -> u16 {
    if v.is_finite() && v >= 0.0 {
        v.min(f32::from(u16::MAX)) as u16
    } else {
        0
    }
}

fn byte(v: f32) -> u8 {
    if v.is_finite() && v >= 0.0 {
        v.min(f32::from(u8::MAX)) as u8
    } else {
        0
    }
}

// --------------------------------------------------------- the set tools

fn effect_schema(e: &Effect) -> Value {
    let mut m = Map::new();
    if e.per_output {
        m.insert(
            "output".into(),
            json!({"description": "An output from 0, or \"all\"",
                   "oneOf": [{"type": "integer", "minimum": 0}, {"const": "all"}]}),
        );
    }
    for q in e.params {
        let desc = described(q);
        let spec = match q.kind {
            Kind::Bool => json!({"type": "boolean", "description": desc}),
            Kind::Number => json!({"type": "number", "description": desc}),
            Kind::Int => json!({"type": "integer", "minimum": 0, "description": desc}),
            Kind::Outputs => json!({"type": "array", "items": {"type": "integer", "minimum": 0},
                                    "description": q.desc}),
        };
        m.insert(q.arg.into(), spec);
    }
    if std::ptr::eq(e, &CROSSFEED) {
        m.insert(
            "pair_map".into(),
            json!({"type": "array", "items": {"type": "integer", "minimum": 0},
                   "description": "The output pairs the crossfeed feeds, as the device's pair \
                                   map bytes (dspi_get_effects shows it)"}),
        );
    }
    let required: &[&str] = if e.per_output { &["output"] } else { &[] };
    schema(m, required)
}

/// `q`'s description with the registry's range or choices (the core
/// clamps a number to that range).
fn described(q: &Param) -> String {
    let extra = match by_path(q.path).map(|d| &d.kind) {
        Some(RegKind::Float { min, max, .. }) => format!(" (the device takes {min}..{max})"),
        Some(RegKind::Int { min, max, .. }) => format!(" (the device takes {min}..{max})"),
        Some(RegKind::Choice(options)) => format!(
            " ({})",
            options
                .iter()
                .map(|(v, l)| format!("{v} {l}"))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        _ => String::new(),
    };
    format!("{}{extra}", q.desc)
}

/// The value `q` sets, as the setter takes it.
fn param_value(st: &DspState, q: &Param, v: &Value) -> Result<f32, ToolError> {
    let bad = || ToolError::new(format!("`{}` has the wrong type", q.arg));
    match q.kind {
        Kind::Bool => v
            .as_bool()
            .map(|b| if b { 1.0 } else { 0.0 })
            .ok_or_else(bad),
        Kind::Number => v
            .as_f64()
            .filter(|x| x.is_finite())
            .map(|x| x as f32)
            .ok_or_else(bad),
        Kind::Int => v
            .as_u64()
            .and_then(|x| u8::try_from(x).ok())
            .map(f32::from)
            .ok_or_else(bad),
        Kind::Outputs => {
            let mut mask: u32 = 0;
            for o in v.as_array().ok_or_else(bad)? {
                let o = o.as_u64().ok_or_else(bad)?;
                if o >= u64::from(st.num_output_channels) {
                    return Err(ToolError::new(format!(
                        "`{}`: no output {o}; this device has outputs 0..{}",
                        q.arg,
                        st.num_output_channels.saturating_sub(1)
                    )));
                }
                mask |= 1u32
                    .checked_shl(u32::try_from(o).unwrap_or(31))
                    .unwrap_or(0);
            }
            Ok(mask as f32)
        }
    }
}

/// `q`'s value as the hook shows it.
fn key_value(q: &Param, v: f32) -> ParamValue {
    match q.kind {
        Kind::Bool => ParamValue::Bool(v != 0.0),
        Kind::Number => ParamValue::Float(v),
        Kind::Int => ParamValue::Choice(byte(v)),
        Kind::Outputs => ParamValue::Mask(v as u32),
    }
}

fn set_effect(call: &mut Call<'_>, args: &Map<String, Value>, e: &'static Effect) -> ToolResult {
    let before = state(call.core()?)?;
    let output = if e.per_output {
        match args.get("output") {
            Some(Value::String(s)) if s == "all" => ALL_OUTPUTS,
            Some(v) => v
                .as_u64()
                .and_then(|o| u8::try_from(o).ok())
                .filter(|o| *o < before.num_output_channels)
                .ok_or_else(|| {
                    ToolError::new(format!(
                        "`output` is an output 0..{} or \"all\"",
                        before.num_output_channels.saturating_sub(1)
                    ))
                })?,
            None => return Err(ToolError::new("`output` is required")),
        }
    } else {
        0
    };
    let indices: Vec<u8> = if e.per_output {
        vec![output]
    } else {
        Vec::new()
    };
    let mut steps = Vec::new();
    let mut changes = Vec::new();
    // The switch is last in each table: levels first, then on or off.
    for q in e.params {
        let Some(given) = args.get(q.arg) else {
            continue;
        };
        let v = param_value(&before, q, given)?;
        let set = e.set;
        let (o, id) = (output, q.id);
        steps.push(Step {
            keys: vec![WriteKey::new(q.path, &indices, key_value(q, v))],
            run: Box::new(move |core| set(core, o, id, v)),
        });
        changes.push(
            json!({"what": q.arg, "before": current(&before, q.path, output),
                            "set": given}),
        );
    }
    let mut statics = Ok(());
    if std::ptr::eq(e, &CROSSFEED)
        && let Some(map) = args.get("pair_map")
    {
        let bytes: Vec<u8> = map
            .as_array()
            .and_then(|a| {
                a.iter()
                    .map(|b| b.as_u64().and_then(|b| u8::try_from(b).ok()))
                    .collect()
            })
            .ok_or_else(|| ToolError::new("`pair_map` is a list of bytes 0..255"))?;
        // A raw write the core's hook does not see (review RA, MCP0 3).
        statics = policy::require_change(&call.judging, "Change the crossfeed's output pairs", &[]);
        changes.push(
            json!({"what": "pair_map", "before": before.crossfeed_pair_map,
                            "set": bytes}),
        );
        steps.push(Step {
            keys: Vec::new(),
            run: Box::new(move |core| core.set_crossfeed_pair_map(bytes.clone())),
        });
    }
    let label = format!("AI: {}", e.word);
    run_steps(call, &before, steps, &label, json!(changes), statics)
}

macro_rules! effect_tool {
    ($id:ident, $effect:ident, $name:literal, $title:literal, $desc:expr) => {
        static $id: Tool = Tool {
            name: $name,
            title: $title,
            description: $desc,
            class: ToolClass::Write,
            device: true,
            input_schema: || effect_schema(&$effect),
            output_schema: write_result_schema,
            handler: |call, args| set_effect(call, args, &$effect),
        };
    };
}

effect_tool!(
    SET_LOUDNESS,
    LOUDNESS,
    "dspi_set_loudness",
    "Set loudness",
    "Loudness compensation: on or off, its reference level, intensity and outputs. Pass only \
     what changes. Turning it on, or raising the reference or intensity while it is on, adds \
     level and may be held for the user's confirmation."
);
effect_tool!(
    SET_CROSSFEED,
    CROSSFEED,
    "dspi_set_crossfeed",
    "Set crossfeed",
    "Headphone crossfeed: on or off, preset, the custom cut-off and feed, ITD, and the output \
     pairs (`output_pairs` mask, or the device's `pair_map`). Pass only what changes."
);
effect_tool!(
    SET_LEVELLER,
    LEVELLER,
    "dspi_set_leveller",
    "Set the leveller",
    "The volume leveller: on or off, amount, speed, maximum gain, look-ahead and gate. With a \
     maximum gain above 0 dB it adds level: turning it on then, or raising the maximum gain, \
     may be held for the user's confirmation."
);
effect_tool!(
    SET_PSYBASS,
    PSYBASS,
    "dspi_set_psybass",
    "Set psychoacoustic bass",
    "Psychoacoustic bass: on or off, cut-off, harmonics, drive, character, the original bass's \
     level and outputs. Turning it on, or raising harmonics, drive or the original level while \
     it is on, adds level and may be held for the user's confirmation."
);
effect_tool!(
    SET_SUBHARMONIC,
    SUBHARMONIC,
    "dspi_set_subharmonic",
    "Set the subharmonic synth",
    "The subharmonic synth: on or off, outputs, the octaves' levels, boost, select, depth, hold, \
     ceiling, link and solo. Turning it on, or raising a level, the boost or the ceiling while \
     it is on, adds level and may be held for the user's confirmation."
);
effect_tool!(
    SET_TUBE,
    TUBE,
    "dspi_set_tube",
    "Set the tube",
    "The tube stage: on or off, outputs, type, drive, character, rectifier, transformer, mix and \
     trim. Turning it on, or raising drive or trim while it is on, may be held for the user's \
     confirmation."
);
effect_tool!(
    SET_UPMIXER,
    UPMIXER,
    "dspi_set_upmixer",
    "Set the upmixer",
    "The stereo-to-surround upmixer (RP2350 only): on or off and its parameters. Pass only what \
     changes."
);
effect_tool!(
    SET_LIMITER,
    LIMITER,
    "dspi_set_limiter",
    "Set an output limiter",
    "An output's limiter (`output`, or \"all\"): on or off, threshold (the ceiling), release and \
     link group. Lowering the ceiling passes; turning a limiter off or raising its ceiling may \
     be held for the user's confirmation. The installer's limits profile may hold the values."
);

// -------------------------------------------------------------- the reads

/// The value `path` holds in `st` (for output `o`, a limiter's), as JSON.
fn current(st: &DspState, path: &str, o: u8) -> Value {
    let at = |v: &[f32], i: u8| num(v.get(usize::from(i)).copied().unwrap_or(0.0));
    let flag = |v: &[f32], i: u8| json!(v.get(usize::from(i)).is_some_and(|x| *x != 0.0));
    let outs = |mask: u32| -> Value {
        json!(
            (0..st.num_output_channels)
                .filter(|k| mask & 1u32.checked_shl(u32::from(*k)).unwrap_or(0) != 0)
                .collect::<Vec<_>>()
        )
    };
    let lim_o = if o == ALL_OUTPUTS { 0 } else { o };
    let lim = |v: &[f32]| num(v.get(usize::from(lim_o)).copied().unwrap_or(0.0));
    match path {
        "loud.on" => json!(st.loudness_enabled),
        "loud.ref" => num(st.loudness_ref_spl),
        "loud.intensity" => num(st.loudness_intensity),
        "loud.mask" => outs(u32::from(st.loudness_output_mask)),
        "cf.on" => json!(st.crossfeed_enabled),
        "cf.preset" => json!(st.crossfeed_preset),
        "cf.freq" => num(st.crossfeed_freq),
        "cf.feed" => num(st.crossfeed_feed),
        "cf.itd" => json!(st.crossfeed_itd),
        "cf.outputs" => json!(st.crossfeed_output_pair_mask),
        "lev.on" => json!(st.leveller_enabled),
        "lev.amount" => num(st.leveller_amount),
        "lev.speed" => json!(st.leveller_speed),
        "lev.maxgain" => num(st.leveller_max_gain_db),
        "lev.lookahead" => json!(st.leveller_lookahead),
        "lev.gate" => num(st.leveller_gate_db),
        "bass.on" => json!(st.psybass_enabled),
        "bass.mask" => outs(u32::from(st.psybass_output_mask)),
        "bass.cutoff" => num(st.psybass_cutoff_hz),
        "bass.harmonics" => num(st.psybass_harmonics_db),
        "bass.drive" => num(st.psybass_drive_db),
        "bass.character" => num(st.psybass_character_pct),
        "bass.original" => num(st.psybass_original_db),
        "sub.solo" => json!(st.subharm_solo),
        "limit.on" => json!(
            st.limiter_enabled
                .get(usize::from(lim_o))
                .copied()
                .unwrap_or(false)
        ),
        "limit.threshold" => lim(&st.limiter_threshold_db),
        "limit.release" => lim(&st.limiter_release_ms),
        "limit.link" => json!(
            st.limiter_link_group
                .get(usize::from(lim_o))
                .copied()
                .unwrap_or(0)
        ),
        _ => {
            let table = match path.split('.').next() {
                Some("sub") => (SUBHARMONIC.params, &st.subharm),
                Some("tube") => (TUBE.params, &st.tube),
                Some("up") => (UPMIXER.params, &st.upmix),
                _ => return Value::Null,
            };
            match table.0.iter().find(|q| q.path == path) {
                Some(q) if q.kind == Kind::Bool => flag(table.1, q.id),
                Some(q) if q.kind == Kind::Outputs => outs(
                    table
                        .1
                        .get(usize::from(q.id))
                        .map_or(0, |x| if *x >= 0.0 { *x as u32 } else { 0 }),
                ),
                Some(q) => at(table.1, q.id),
                None => Value::Null,
            }
        }
    }
}

/// One effect's values, by argument name.
fn effect_json(st: &DspState, e: &Effect, o: u8) -> Value {
    let mut m = Map::new();
    for q in e.params {
        m.insert(q.arg.into(), current(st, q.path, o));
    }
    Value::Object(m)
}

static GET_EFFECTS: Tool = Tool {
    name: "dspi_get_effects",
    title: "Read the effects",
    description: "Every effect's settings: loudness, crossfeed (with its pair map), leveller, \
                  psychoacoustic bass, subharmonic synth, tube, upmixer, and each output's \
                  limiter. The argument names are those the dspi_set_<effect> tools take.",
    class: ToolClass::Read,
    device: true,
    input_schema: empty_schema,
    output_schema: object_schema,
    handler: |call, _| {
        let core = call.core()?;
        let st = state(core)?;
        let mut crossfeed = effect_json(&st, &CROSSFEED, 0);
        if let Some(m) = crossfeed.as_object_mut() {
            m.insert("pair_map".into(), json!(st.crossfeed_pair_map));
            // What a named preset runs; freq_hz and feed_db are the custom
            // pair the set tool writes.
            m.insert("running_freq_hz".into(), num(st.crossfeed_freq));
            m.insert("running_feed_db".into(), num(st.crossfeed_feed));
            if let Ok(custom) = core.crossfeed_custom_pair() {
                m.insert("freq_hz".into(), num(custom.freq_hz));
                m.insert("feed_db".into(), num(custom.feed_db));
            }
        }
        let limiters: Vec<Value> = (0..st.num_output_channels)
            .map(|o| {
                let mut v = effect_json(&st, &LIMITER, o);
                if let Some(m) = v.as_object_mut() {
                    m.insert("output".into(), json!(o));
                }
                v
            })
            .collect();
        Ok(json!({
            "device": call.device_text(),
            "loudness": effect_json(&st, &LOUDNESS, 0),
            "crossfeed": crossfeed,
            "leveller": effect_json(&st, &LEVELLER, 0),
            "psybass": effect_json(&st, &PSYBASS, 0),
            "subharmonic": effect_json(&st, &SUBHARMONIC, 0),
            "tube": effect_json(&st, &TUBE, 0),
            "upmixer": if st.upmix.is_empty() { Value::Null } else { effect_json(&st, &UPMIXER, 0) },
            "limiters": limiters,
        }))
    },
};

static GET_LIMITER_METER: Tool = Tool {
    name: "dspi_get_limiter_meter",
    title: "Read the limiters' gain reduction",
    description: "Each output's limiter gain reduction now, in dB (0 when not limiting).",
    class: ToolClass::Read,
    device: true,
    input_schema: empty_schema,
    output_schema: object_schema,
    handler: |call, _| {
        let core = call.core()?;
        let st = state(core)?;
        let meter = core
            .fetch_limiter_meter()
            .map_err(|e| policy::core_error(&call.judging, e))?;
        let n = usize::from(st.num_output_channels);
        let gr: Vec<Value> = meter.iter().take(n).map(|x| num(*x)).collect();
        Ok(json!({"device": call.device_text(), "gain_reduction_db": gr}))
    },
};

static GET_UPMIX_STATUS: Tool = Tool {
    name: "dspi_get_upmix_status",
    title: "Read the upmixer's status",
    description: "Whether the upmixer runs (and why not), and its live correlation, balance and \
                  centre and surround gains.",
    class: ToolClass::Read,
    device: true,
    input_schema: empty_schema,
    output_schema: object_schema,
    handler: |call, _| {
        let core = call.core()?;
        state(core)?;
        let s = core
            .fetch_upmix_status()
            .map_err(|e| policy::core_error(&call.judging, e))?;
        Ok(json!({
            "device": call.device_text(),
            "active": s.active,
            "parked_reason": s.parked_reason,
            "correlation": num(s.correlation),
            "balance": num(s.balance),
            "center_gain": num(s.center_gain),
            "left_surround_gain": num(s.ls_gain),
            "right_surround_gain": num(s.rs_gain),
        }))
    },
};

static GET_SUBHARM_HEADROOM: Tool = Tool {
    name: "dspi_get_subharm_headroom",
    title: "Read the subharmonic synth's headroom",
    description: "How much headroom the subharmonic synth has left now, in dB.",
    class: ToolClass::Read,
    device: true,
    input_schema: empty_schema,
    output_schema: object_schema,
    handler: |call, _| {
        let core = call.core()?;
        state(core)?;
        let h = core
            .fetch_subharm_headroom()
            .map_err(|e| policy::core_error(&call.judging, e))?;
        Ok(json!({"device": call.device_text(), "headroom_db": num(h)}))
    },
};
