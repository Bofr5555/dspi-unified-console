//! The edit tools (WP MCP2a): EQ, crossover, delay, gain, polarity, mute,
//! output enable, routing, volume and undo.
//!
//! Each tool turns its arguments into a [`Plan`] of the core's typed setters
//! on a copy of the device's state (so a "free" slot is free after the
//! writes before it), the policy judges the whole plan
//! ([`policy::prejudge`]) and only then are the setters called, as one undo
//! group when there are several. A setter that fails halfway puts back what
//! the call had written. `dspi_apply_changes` (`tools/apply.rs`) runs
//! several tools' plans as one.
//!
//! Results have one shape: `{ok, changes: [{what, ..., before, after}],
//! undo_id}`; refusals are tool errors that say why and what to do.

use dspi_core::consts::BANDS_PER_CHANNEL;
use dspi_core::{DspState, DspiCore, FilterParams, HistorySource, ParamValue};
use dspi_proto::packets::MatrixRoutePacket;
use dspi_proto::xover::{self, Family, Meta};
use serde_json::{Map, Value, json};

use crate::policy::{self, WriteKey};
use crate::registry::{Call, Tool, ToolClass, ToolError, ToolResult};
use crate::tools::read::{
    band_json, num, object_schema, state, target, target_props, type_word, xover_json,
};

/// The edit tools, in `tools/list` order.
pub static TOOLS: &[&Tool] = &[
    &SET_PEQ_BAND,
    &CLEAR_PEQ_BAND,
    &SET_PEQ_BYPASS,
    &SET_EQ_BYPASS,
    &SET_CROSSOVER,
    &SET_DELAY,
    &SET_GAIN,
    &SET_POLARITY,
    &SET_MUTE,
    &SET_OUTPUT_ENABLED,
    &SET_ROUTE,
    &SET_VOLUME,
    &SET_USER_MUTE,
    &UNDO,
];

/// A tool's planner: its arguments onto a plan.
pub(crate) type Planner = fn(&mut Plan, &Map<String, Value>) -> Result<(), ToolError>;

/// The planners `dspi_apply_changes` takes, by `op` (the tool's name
/// without `dspi_`).
pub(crate) static PLANNERS: &[(&str, Planner)] = &[
    ("set_peq_band", plan_peq_band),
    ("clear_peq_band", plan_clear_peq_band),
    ("set_peq_bypass", plan_peq_bypass),
    ("set_eq_bypass", plan_eq_bypass),
    ("set_crossover", plan_crossover),
    ("set_delay", plan_delay),
    ("set_gain", plan_gain),
    ("set_polarity", plan_polarity),
    ("set_mute", plan_mute),
    ("set_output_enabled", plan_output_enabled),
    ("set_route", plan_route),
    ("set_volume", plan_volume),
    ("set_user_mute", plan_user_mute),
];

// --------------------------------------------------------------- the plan

/// One typed setter call.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Op {
    /// PEQ band `band` (0..9) of wire channel `ch`.
    Peq {
        ch: u8,
        band: u8,
        p: FilterParams,
    },
    /// Crossover band `xband` (0..3) of an output's wire channel `ch`.
    Xover {
        ch: u8,
        xband: u8,
        p: FilterParams,
    },
    /// One band's bypass (PEQ 0..9, crossover 20..23).
    BandBypass {
        ch: u8,
        band: u8,
        bypass: bool,
    },
    /// The device's EQ bypass.
    EqBypass(bool),
    OutGain {
        o: u8,
        db: f32,
    },
    InTrim {
        i: u8,
        db: f32,
    },
    OutDelay {
        o: u8,
        ms: f32,
    },
    InDelay {
        ch: u8,
        ms: f32,
    },
    OutMute {
        o: u8,
        muted: bool,
    },
    OutEnable {
        o: u8,
        on: bool,
    },
    Route {
        i: u8,
        o: u8,
        on: bool,
        gain: f32,
        invert: bool,
    },
    UserVolume(f32),
    MasterVolume(f32),
    UserMute(bool),
}

fn band_writes(ch: u8, band: u8, p: &FilterParams) -> Vec<WriteKey> {
    let ix = [ch, band];
    let lt = p.filter_type == 11;
    let mut w = vec![
        WriteKey::new("eq.type", &ix, ParamValue::Choice(p.filter_type)),
        WriteKey::new("eq.bypass", &ix, ParamValue::Bool(p.bypass)),
        WriteKey::new("eq.freq", &ix, ParamValue::Float(p.freq)),
        WriteKey::new("eq.q", &ix, ParamValue::Float(p.q)),
        WriteKey::new(
            if lt { "eq.fp" } else { "eq.gain" },
            &ix,
            ParamValue::Float(p.gain),
        ),
    ];
    if lt {
        w.push(WriteKey::new("eq.qp", &ix, ParamValue::Float(p.qp)));
    }
    w
}

impl Op {
    // -- WP FFIX --
    /// 0 for an op that silences (a mute, an output off, a route off), 2 for
    /// one that lets sound through (an unmute, an output on, a route on), 1
    /// for the rest: the order [`execute`] runs a plan's ops in.
    fn opens(&self) -> u8 {
        match *self {
            Op::OutMute { muted, .. } | Op::UserMute(muted) => {
                if muted {
                    0
                } else {
                    2
                }
            }
            Op::OutEnable { on, .. } | Op::Route { on, .. } => {
                if on {
                    2
                } else {
                    0
                }
            }
            _ => 1,
        }
    }

    /// What it writes, as the core's hook will show it.
    fn writes(&self) -> Vec<WriteKey> {
        use ParamValue::{Bool, Bytes, Float};
        match *self {
            Op::Peq { ch, band, p } => band_writes(ch, band, &p),
            Op::Xover { ch, xband, p } => band_writes(ch, xband.saturating_add(20), &p),
            Op::BandBypass { ch, band, bypass } => {
                vec![WriteKey::new("eq.bypass", &[ch, band], Bool(bypass))]
            }
            Op::EqBypass(b) => vec![WriteKey::new("bypass", &[], Bool(b))],
            Op::OutGain { o, db } => vec![WriteKey::new("out.gain", &[o], Float(db))],
            Op::InTrim { i, db } => vec![WriteKey::new("pre", &[i], Float(db))],
            Op::OutDelay { o, ms } => vec![WriteKey::new("out.delay", &[o], Float(ms))],
            Op::InDelay { ch, ms } => vec![WriteKey::new("ch.delay", &[ch], Float(ms))],
            Op::OutMute { o, muted } => vec![WriteKey::new("out.mute", &[o], Bool(muted))],
            Op::OutEnable { o, on } => vec![WriteKey::new("out.enable", &[o], Bool(on))],
            Op::Route {
                i,
                o,
                on,
                gain,
                invert,
            } => {
                let p = MatrixRoutePacket {
                    input: i,
                    output: o,
                    enabled: on,
                    phase_invert: invert,
                    gain_db: gain,
                };
                vec![WriteKey::new("mix", &[i, o], Bytes(p.encode().to_vec()))]
            }
            Op::UserVolume(db) => vec![WriteKey::new("vol.user", &[], Float(db))],
            Op::MasterVolume(db) => vec![WriteKey::new("vol.master", &[], Float(db))],
            Op::UserMute(m) => vec![WriteKey::new("vol.mute", &[], Bool(m))],
        }
    }

    /// Call the setter.
    fn run(&self, core: &DspiCore) -> dspi_core::Result<()> {
        match *self {
            Op::Peq { ch, band, p } => core.set_filter(ch, band, p),
            Op::Xover { ch, xband, p } => core.set_crossover(ch, xband, p),
            Op::BandBypass { ch, band, bypass } => core.set_band_bypass(ch, band, bypass),
            Op::EqBypass(b) => core.set_bypass(b),
            Op::OutGain { o, db } => core.set_output_gain(o, db),
            Op::InTrim { i, db } => core.set_input_preamp(i, db),
            Op::OutDelay { o, ms } => core.set_output_delay(o, ms),
            Op::InDelay { ch, ms } => core.set_delay(ch, ms),
            Op::OutMute { o, muted } => core.set_output_mute(o, muted),
            Op::OutEnable { o, on } => core.set_output_enable(o, on).map(|_| ()),
            Op::Route {
                i,
                o,
                on,
                gain,
                invert,
            } => core.set_matrix_route(i, o, on, gain, invert),
            Op::UserVolume(db) => core.set_user_volume(db),
            Op::MasterVolume(db) => core.set_master_volume(db),
            Op::UserMute(m) => core.set_user_mute(m),
        }
    }

    /// The change in the result: what, and the value before and after.
    fn change(&self, before: &DspState, after: &DspState) -> Value {
        let n_in = before.num_input_channels;
        let at = |v: &[f32], i: u8| num(v.get(usize::from(i)).copied().unwrap_or(0.0));
        let flag = |v: &[bool], i: u8| json!(v.get(usize::from(i)).copied().unwrap_or(false));
        match *self {
            Op::Peq { ch, band, .. } | Op::BandBypass { ch, band, .. } if band < 20 => {
                let b = |st: &DspState| {
                    policy::band_of(st, ch, band)
                        .map_or(Value::Null, |f| band_json(usize::from(band), f))
                };
                json!({"what": "peq_band", "channel": ch, "slot": band,
                       "before": b(before), "after": b(after)})
            }
            Op::Xover { ch, .. } | Op::BandBypass { ch, .. } | Op::Peq { ch, .. } => {
                let x = |st: &DspState| {
                    st.xover.get(usize::from(ch)).map_or(Value::Null, |row| {
                        json!(
                            row.iter()
                                .enumerate()
                                .filter_map(|(i, f)| xover_json(i, f))
                                .collect::<Vec<_>>()
                        )
                    })
                };
                json!({"what": "crossover", "output": ch.saturating_sub(n_in),
                       "before": x(before), "after": x(after)})
            }
            Op::EqBypass(_) => {
                json!({"what": "eq_bypass", "before": before.bypass, "after": after.bypass})
            }
            Op::OutGain { o, .. } => json!({"what": "output_gain_db", "output": o,
                "before": at(&before.output_gain_db, o), "after": at(&after.output_gain_db, o)}),
            Op::InTrim { i, .. } => json!({"what": "input_gain_db", "input": i,
                "before": at(&before.input_preamp_db, i), "after": at(&after.input_preamp_db, i)}),
            Op::OutDelay { o, .. } => json!({"what": "output_delay_ms", "output": o,
                "before": at(&before.output_delay_ms, o), "after": at(&after.output_delay_ms, o)}),
            Op::InDelay { ch, .. } => json!({"what": "input_delay_ms", "input": ch,
                "before": at(&before.channel_delays, ch), "after": at(&after.channel_delays, ch)}),
            Op::OutMute { o, .. } => json!({"what": "output_muted", "output": o,
                "before": flag(&before.output_muted, o), "after": flag(&after.output_muted, o)}),
            Op::OutEnable { o, .. } => json!({"what": "output_enabled", "output": o,
                "before": flag(&before.output_enabled, o), "after": flag(&after.output_enabled, o)}),
            Op::Route { i, o, .. } => {
                let r = |st: &DspState| {
                    let row = |v: &[Vec<bool>]| {
                        v.get(usize::from(i))
                            .and_then(|r| r.get(usize::from(o)))
                            .copied()
                            .unwrap_or(false)
                    };
                    json!({"on": row(&st.matrix_routing),
                           "gain_db": num(st.matrix_gain.get(usize::from(i))
                                .and_then(|r| r.get(usize::from(o))).copied().unwrap_or(0.0)),
                           "invert": row(&st.matrix_invert)})
                };
                json!({"what": "route", "input": i, "output": o,
                       "before": r(before), "after": r(after)})
            }
            Op::UserVolume(_) => json!({"what": "user_volume_db",
                "before": num(before.user_volume_db), "after": num(after.user_volume_db)}),
            Op::MasterVolume(_) => json!({"what": "master_volume_db",
                "before": num(before.master_volume_db), "after": num(after.master_volume_db)}),
            Op::UserMute(_) => json!({"what": "user_mute",
                "before": before.user_mute, "after": after.user_mute}),
        }
    }
}

/// A call's setters, on a copy of the state as they will leave it.
#[derive(Debug, Clone)]
pub(crate) struct Plan {
    /// The state after the ops so far.
    pub(crate) st: DspState,
    pub(crate) ops: Vec<Op>,
    pub(crate) writes: Vec<WriteKey>,
}

/// The most setter calls in one call (MCPDES 3.3).
pub(crate) const MAX_WRITES: usize = 40;

impl Plan {
    pub(crate) fn new(st: DspState) -> Self {
        Self {
            st,
            ops: Vec::new(),
            writes: Vec::new(),
        }
    }

    fn push(&mut self, op: Op) -> Result<(), ToolError> {
        if self.ops.len() >= MAX_WRITES {
            return Err(ToolError::new(format!(
                "at most {MAX_WRITES} changes in one call"
            )));
        }
        let w = op.writes();
        for x in &w {
            policy::apply(&mut self.st, x);
        }
        self.writes.extend(w);
        self.ops.push(op);
        Ok(())
    }
}

/// Judge `plan` (made on `before`), then run it: one undo group for several
/// setters, everything the call wrote put back if one fails. `label` names
/// the group in the History (numbers only).
pub(crate) fn execute(
    call: &mut Call<'_>,
    before: &DspState,
    plan: Plan,
    label: &str,
) -> ToolResult {
    policy::prejudge(&call.judging, before, &plan.writes)?;
    let core = call.core()?;
    let changes: Vec<Value> = plan
        .ops
        .iter()
        .map(|op| op.change(before, &plan.st))
        .collect();
    let first = newest(core);
    let group = plan.ops.len() > 1;
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
    // -- WP FFIX -- FUNC1 bug 1: quiet first, open last, so a batch that
    // protects a tweeter and routes it never plays it unprotected between
    // two writes (each op writes its own value: the end state is the same)
    let mut ops: Vec<&Op> = plan.ops.iter().collect();
    ops.sort_by_key(|op| op.opens());
    for op in ops {
        if let Err(e) = op.run(core) {
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
                 read the device again (dspi_get_channel).",
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

// -- WP MCP2b -- pub(crate): `newest`, `own_ids_since`
/// The newest journal id on `core`, 0 for none.
pub(crate) fn newest(core: &DspiCore) -> u64 {
    core.history(1).first().map_or(0, |e| e.id)
}

/// `actor`'s own journal ids newer than `since`, oldest first, once each.
pub(crate) fn own_ids_since(core: &DspiCore, actor: u32, since: u64) -> Vec<u64> {
    let mut ids: Vec<u64> = core
        .history(200)
        .into_iter()
        .filter(|e| e.id > since && e.actor == actor && e.source == HistorySource::Local)
        .map(|e| e.id)
        .collect();
    ids.sort_unstable();
    ids.dedup();
    ids
}

// ---------------------------------------------------------- the arguments

// -- WP MCP2b -- pub(crate): `u8_arg`, `bool_arg`, `channel`, `schema`, `with_target`,
// `write_result_schema`
pub(crate) fn u8_arg(args: &Map<String, Value>, key: &str) -> Option<u8> {
    args.get(key)
        .and_then(Value::as_u64)
        .map(|v| u8::try_from(v).unwrap_or(u8::MAX))
}

fn f32_arg(args: &Map<String, Value>, key: &str) -> Option<f32> {
    args.get(key).and_then(Value::as_f64).map(|v| v as f32)
}

pub(crate) fn bool_arg(args: &Map<String, Value>, key: &str) -> Option<bool> {
    args.get(key).and_then(Value::as_bool)
}

fn need<T>(v: Option<T>, key: &str) -> Result<T, ToolError> {
    v.ok_or_else(|| ToolError::new(format!("`{key}` is required")))
}

/// The wire channel a call names, as a `u8`, and whether it is an output.
pub(crate) fn channel(
    st: &DspState,
    args: &Map<String, Value>,
) -> Result<(u8, bool, u8), ToolError> {
    let t = target(st, args)?;
    let ch = u8::try_from(t.channel).map_err(|_| ToolError::new("no such channel"))?;
    let index = u8::try_from(t.index).map_err(|_| ToolError::new("no such channel"))?;
    Ok((ch, t.output, index))
}

fn output(st: &DspState, args: &Map<String, Value>) -> Result<u8, ToolError> {
    let o = need(u8_arg(args, "output"), "output")?;
    if o >= st.num_output_channels {
        return Err(ToolError::new(format!(
            "no output {o}; this device has outputs 0..{}",
            st.num_output_channels.saturating_sub(1)
        )));
    }
    Ok(o)
}

fn input(st: &DspState, args: &Map<String, Value>) -> Result<u8, ToolError> {
    let i = need(u8_arg(args, "input"), "input")?;
    if i >= st.num_input_channels {
        return Err(ToolError::new(format!(
            "no input {i}; this device has inputs 0..{}",
            st.num_input_channels.saturating_sub(1)
        )));
    }
    Ok(i)
}

/// The PEQ type words the tools take, by code.
const PEQ_TYPES: &[u8] = &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13];

fn peq_type_words() -> Vec<Value> {
    PEQ_TYPES.iter().map(|c| json!(type_word(*c))).collect()
}

fn peq_type(word: &str) -> Option<u8> {
    PEQ_TYPES.iter().copied().find(|c| type_word(*c) == word)
}

/// A PEQ slot: a number, or "free" for the first flat one.
fn slot(st: &DspState, ch: u8, args: &Map<String, Value>, free_ok: bool) -> Result<u8, ToolError> {
    match args.get("slot") {
        Some(Value::String(s)) if s == "free" && free_ok => st
            .filters
            .get(usize::from(ch))
            .and_then(|row| {
                row.iter()
                    .take(BANDS_PER_CHANNEL)
                    .position(|b| b.filter_type == 0)
            })
            .and_then(|i| u8::try_from(i).ok())
            .ok_or_else(|| {
                ToolError::new(
                    "no PEQ slot is free on this channel; clear one or pass a slot number",
                )
            }),
        Some(v) => v
            .as_u64()
            .and_then(|n| u8::try_from(n).ok())
            .filter(|n| usize::from(*n) < BANDS_PER_CHANNEL)
            .ok_or_else(|| {
                ToolError::new(if free_ok {
                    "`slot` is a PEQ slot 0..9, or \"free\""
                } else {
                    "`slot` is a PEQ slot 0..9"
                })
            }),
        None => Err(ToolError::new("`slot` is required")),
    }
}

fn slot_schema(free_ok: bool) -> Value {
    if free_ok {
        json!({"description": "A PEQ slot 0..9, or \"free\" for the first flat one",
               "oneOf": [{"type": "integer", "minimum": 0, "maximum": 9}, {"const": "free"}]})
    } else {
        json!({"type": "integer", "minimum": 0, "maximum": 9, "description": "A PEQ slot 0..9"})
    }
}

pub(crate) fn schema(props: Map<String, Value>, required: &[&str]) -> Value {
    json!({"type": "object", "properties": props, "required": required,
           "additionalProperties": false})
}

pub(crate) fn with_target(extra: Value) -> Map<String, Value> {
    let mut m = target_props();
    if let Value::Object(e) = extra {
        m.extend(e);
    }
    m
}

pub(crate) fn write_result_schema() -> Value {
    json!({"type": "object", "required": ["ok", "changes"],
           "properties": {"ok": {"type": "boolean"}, "changes": {"type": "array"},
                          "undo_id": {"type": ["integer", "null"]}}})
}

/// The tool for `name` with these parts (all edit tools are writes on a
/// device and share the result).
macro_rules! edit_tool {
    ($id:ident, $name:literal, $title:literal, $desc:expr, $schema:expr, $plan:ident, $label:literal) => {
        static $id: Tool = Tool {
            name: $name,
            title: $title,
            description: $desc,
            class: ToolClass::Write,
            device: true,
            input_schema: $schema,
            output_schema: write_result_schema,
            handler: |call, args| run_planner(call, args, $plan, $label),
        };
    };
}

/// A planner's tool call: plan on the device's state, then execute.
fn run_planner(
    call: &mut Call<'_>,
    args: &Map<String, Value>,
    planner: Planner,
    label: &str,
) -> ToolResult {
    let before = state(call.core()?)?;
    let mut plan = Plan::new(before.clone());
    planner(&mut plan, args)?;
    execute(call, &before, plan, label)
}

// -------------------------------------------------------------------- EQ

edit_tool!(
    SET_PEQ_BAND,
    "dspi_set_peq_band",
    "Set a PEQ band",
    "Set one PEQ slot of a channel (by `output`, `input` or wire `channel`): `slot` 0..9 or \
     \"free\" (the first flat slot), `type`, `freq_hz`, `gain_db` (cut or boost), `q`. A \
     linkwitz_transform takes `fp_hz` and `qp` too. A narrow boost or cut is a peaking band; a \
     bandpass is the output's crossover (dspi_set_crossover). Louder changes and boosts above \
     +6 dB may be held for the user's confirmation in the app.",
    || schema(
        with_target(json!({
            "slot": slot_schema(true),
            "type": {"enum": peq_type_words()},
            "freq_hz": {"type": "number", "minimum": 10, "maximum": 24000},
            "gain_db": {"type": "number", "minimum": -30, "maximum": 20},
            "q": {"type": "number", "minimum": 0.1, "maximum": 20},
            "bypass": {"type": "boolean"},
            "fp_hz": {"type": "number", "minimum": 10, "maximum": 24000},
            "qp": {"type": "number", "minimum": 0.1, "maximum": 20},
        })),
        &["slot", "type", "freq_hz"]
    ),
    plan_peq_band,
    "AI: PEQ band"
);

fn plan_peq_band(plan: &mut Plan, args: &Map<String, Value>) -> Result<(), ToolError> {
    let (ch, _, _) = channel(&plan.st, args)?;
    let band = slot(&plan.st, ch, args, true)?;
    let word = args.get("type").and_then(Value::as_str).unwrap_or("");
    let filter_type = peq_type(word).ok_or_else(|| ToolError::new("unknown `type`"))?;
    let lt = filter_type == 11;
    let p = FilterParams {
        filter_type,
        bypass: bool_arg(args, "bypass").unwrap_or(false),
        freq: need(f32_arg(args, "freq_hz"), "freq_hz")?,
        q: f32_arg(args, "q").unwrap_or(0.707),
        gain: if lt {
            need(f32_arg(args, "fp_hz"), "fp_hz")?
        } else {
            f32_arg(args, "gain_db").unwrap_or(0.0)
        },
        qp: if lt {
            f32_arg(args, "qp").unwrap_or(0.707)
        } else {
            0.707
        },
    };
    plan.push(Op::Peq { ch, band, p })
}

edit_tool!(
    CLEAR_PEQ_BAND,
    "dspi_clear_peq_band",
    "Clear a PEQ band",
    "Set one PEQ slot of a channel to flat (free).",
    || schema(with_target(json!({"slot": slot_schema(false)})), &["slot"]),
    plan_clear_peq_band,
    "AI: clear PEQ band"
);

fn plan_clear_peq_band(plan: &mut Plan, args: &Map<String, Value>) -> Result<(), ToolError> {
    let (ch, _, _) = channel(&plan.st, args)?;
    let band = slot(&plan.st, ch, args, false)?;
    let was = policy::band_of(&plan.st, ch, band)
        .copied()
        .unwrap_or(policy::FLAT);
    let p = FilterParams {
        filter_type: 0,
        bypass: false,
        ..was
    };
    plan.push(Op::Peq { ch, band, p })
}

edit_tool!(
    SET_PEQ_BYPASS,
    "dspi_set_peq_bypass",
    "Bypass a PEQ band",
    "Bypass (true) or re-enable one PEQ slot of a channel, keeping its settings.",
    || schema(
        with_target(json!({"slot": slot_schema(false), "bypass": {"type": "boolean"}})),
        &["slot", "bypass"]
    ),
    plan_peq_bypass,
    "AI: PEQ bypass"
);

fn plan_peq_bypass(plan: &mut Plan, args: &Map<String, Value>) -> Result<(), ToolError> {
    let (ch, _, _) = channel(&plan.st, args)?;
    let band = slot(&plan.st, ch, args, false)?;
    let bypass = need(bool_arg(args, "bypass"), "bypass")?;
    plan.push(Op::BandBypass { ch, band, bypass })
}

edit_tool!(
    SET_EQ_BYPASS,
    "dspi_set_eq_bypass",
    "EQ bypass",
    "The device's EQ bypass: true passes the inputs' PEQ by (the outputs' filters and \
     crossovers stay).",
    || schema(
        {
            let mut m = Map::new();
            m.insert("bypass".into(), json!({"type": "boolean"}));
            m
        },
        &["bypass"]
    ),
    plan_eq_bypass,
    "AI: EQ bypass"
);

fn plan_eq_bypass(plan: &mut Plan, args: &Map<String, Value>) -> Result<(), ToolError> {
    plan.push(Op::EqBypass(need(bool_arg(args, "bypass"), "bypass")?))
}

// -------------------------------------------------------------- crossover

fn side_schema() -> Value {
    json!({"type": ["object", "null"],
           "description": "null removes it; leave it out to keep it",
           "required": ["family", "order", "freq_hz"],
           "properties": {
               "family": {"enum": ["linkwitz_riley", "butterworth", "bessel"]},
               "order": {"type": "integer", "minimum": 1, "maximum": 8},
               "freq_hz": {"type": "number", "minimum": 10, "maximum": 24000}},
           "additionalProperties": false})
}

edit_tool!(
    SET_CROSSOVER,
    "dspi_set_crossover",
    "Set an output's crossover (bandpass)",
    "An output's crossover: its `high_pass` and `low_pass`, each {family, order, freq_hz} \
         (linkwitz_riley 2/4/6/8, butterworth 1..8, bessel 2/4/6/8), or null to remove it; \
         leave one out to keep it. Both together are a bandpass: this is the tool for \"add a \
         bandpass\". On an output the user's speaker setup calls a tweeter the high-pass may \
         move, also lower, but stays at 1 kHz or above (a higher floor from the limits profile) \
         and 12 dB/octave or steeper: anything else is refused; a midrange's floor is 80 Hz. On \
         an output with no confirmed role, a high-pass at 1 kHz or above is taken for a \
         tweeter's: lowering, removing or flattening it is held for the user (refused under \
         \"never ask\").",
    || {
        let mut m = Map::new();
        m.insert(
            "output".into(),
            json!({"type": "integer", "minimum": 0, "description": "An output, from 0"}),
        );
        m.insert("high_pass".into(), side_schema());
        m.insert("low_pass".into(), side_schema());
        schema(m, &["output"])
    },
    plan_crossover,
    "AI: crossover"
);

fn side(v: &Value, high_pass: bool) -> Result<FilterParams, ToolError> {
    let which = if high_pass { "high_pass" } else { "low_pass" };
    let bad = |what: &str| ToolError::new(format!("`{which}`: {what}"));
    let o = v.as_object().ok_or_else(|| bad("an object or null"))?;
    if o.keys()
        .any(|k| !matches!(k.as_str(), "family" | "order" | "freq_hz"))
    {
        return Err(bad("takes family, order and freq_hz only"));
    }
    let family = match o.get("family").and_then(Value::as_str) {
        Some("linkwitz_riley") => Family::LinkwitzRiley,
        Some("butterworth") => Family::Butterworth,
        Some("bessel") => Family::Bessel,
        _ => return Err(bad("`family` is linkwitz_riley, butterworth or bessel")),
    };
    let order = o
        .get("order")
        .and_then(Value::as_u64)
        .and_then(|n| u8::try_from(n).ok())
        .ok_or_else(|| bad("`order` is required"))?;
    let ok = match family {
        Family::Butterworth => (1..=8).contains(&order),
        _ => matches!(order, 2 | 4 | 6 | 8),
    };
    if !ok {
        return Err(bad(
            "`order`: linkwitz_riley and bessel take 2, 4, 6 or 8; butterworth 1..8",
        ));
    }
    let freq = o
        .get("freq_hz")
        .and_then(Value::as_f64)
        .filter(|f| f.is_finite() && (10.0..=24_000.0).contains(f))
        .ok_or_else(|| bad("`freq_hz` is 10..24000"))? as f32;
    let filter_type = Meta {
        family,
        order,
        high_pass,
    }
    .to_type();
    Ok(FilterParams {
        filter_type,
        bypass: false,
        freq,
        q: 0.707,
        gain: 0.0,
        qp: 0.707,
    })
}

fn plan_crossover(plan: &mut Plan, args: &Map<String, Value>) -> Result<(), ToolError> {
    let o = output(&plan.st, args)?;
    let ch = plan.st.num_input_channels.saturating_add(o);
    if !args.contains_key("high_pass") && !args.contains_key("low_pass") {
        return Err(ToolError::new("give `high_pass`, `low_pass` or both"));
    }
    for (key, hp) in [("high_pass", true), ("low_pass", false)] {
        let Some(v) = args.get(key) else {
            continue;
        };
        let row: Vec<FilterParams> = plan
            .st
            .xover
            .get(usize::from(ch))
            .cloned()
            .unwrap_or_default();
        let is_side =
            |f: &FilterParams| xover::meta(f.filter_type).is_some_and(|m| m.high_pass == hp);
        let slots: Vec<u8> = row
            .iter()
            .enumerate()
            .filter(|(_, f)| is_side(f))
            .filter_map(|(i, _)| u8::try_from(i).ok())
            .collect();
        if v.is_null() {
            for x in slots {
                let was = row.get(usize::from(x)).copied().unwrap_or(policy::FLAT);
                let p = FilterParams {
                    filter_type: 0,
                    bypass: false,
                    ..was
                };
                plan.push(Op::Xover { ch, xband: x, p })?;
            }
            continue;
        }
        let p = side(v, hp)?;
        let x = match slots.first() {
            Some(x) => *x,
            None => row
                .iter()
                .position(|f| f.filter_type == 0)
                .and_then(|i| u8::try_from(i).ok())
                .ok_or_else(|| ToolError::new("no crossover slot is free on this output"))?,
        };
        plan.push(Op::Xover { ch, xband: x, p })?;
    }
    Ok(())
}

// ----------------------------------------------- delay, gain, mute, enable

fn out_or_in_schema(key: &str, spec: Value) -> Value {
    let mut m = Map::new();
    m.insert(
        "output".into(),
        json!({"type": "integer", "minimum": 0, "description": "An output, from 0"}),
    );
    m.insert(
        "input".into(),
        json!({"type": "integer", "minimum": 0, "description": "An input, from 0"}),
    );
    m.insert(key.into(), spec);
    schema(m, &[key])
}

/// `output` or `input`, one of them.
fn out_or_in(st: &DspState, args: &Map<String, Value>) -> Result<(bool, u8), ToolError> {
    match (args.contains_key("output"), args.contains_key("input")) {
        (true, false) => Ok((true, output(st, args)?)),
        (false, true) => Ok((false, input(st, args)?)),
        _ => Err(ToolError::new("name one of `output` or `input`")),
    }
}

edit_tool!(
    SET_DELAY,
    "dspi_set_delay",
    "Set a delay",
    "An output's or an input's delay in ms (up to the device's maximum). A step of more than \
     20 ms may be held for the user's confirmation.",
    || out_or_in_schema(
        "delay_ms",
        // -- WP FEAT2 -- the longest delay line there is; a device with a
        // shorter one refuses past its own (`DspiCore::max_delay_ms`)
        json!({"type": "number", "minimum": 0,
               "maximum": dspi_proto::Platform::Rp2350.max_delay_ms()})
    ),
    plan_delay,
    "AI: delay"
);

fn plan_delay(plan: &mut Plan, args: &Map<String, Value>) -> Result<(), ToolError> {
    let (out, n) = out_or_in(&plan.st, args)?;
    let ms = need(f32_arg(args, "delay_ms"), "delay_ms")?;
    plan.push(if out {
        Op::OutDelay { o: n, ms }
    } else {
        Op::InDelay { ch: n, ms }
    })
}

edit_tool!(
    SET_GAIN,
    "dspi_set_gain",
    "Set a gain",
    "An output's gain (-60..+20 dB) or an input's trim (-60..+24 dB). Lowering passes; raising \
     may be held for the user's confirmation in the app.",
    || out_or_in_schema(
        "gain_db",
        json!({"type": "number", "minimum": -60, "maximum": 24})
    ),
    plan_gain,
    "AI: gain"
);

fn plan_gain(plan: &mut Plan, args: &Map<String, Value>) -> Result<(), ToolError> {
    let (out, n) = out_or_in(&plan.st, args)?;
    let db = need(f32_arg(args, "gain_db"), "gain_db")?;
    plan.push(if out {
        Op::OutGain { o: n, db }
    } else {
        Op::InTrim { i: n, db }
    })
}

fn output_flag_schema(key: &str) -> Value {
    let mut m = Map::new();
    m.insert(
        "output".into(),
        json!({"type": "integer", "minimum": 0, "description": "An output, from 0"}),
    );
    m.insert(key.into(), json!({"type": "boolean"}));
    schema(m, &["output", key])
}

edit_tool!(
    SET_POLARITY,
    "dspi_set_polarity",
    "Set an output's polarity",
    "Invert (true) or restore an output's polarity: set on every route that feeds it.",
    || output_flag_schema("inverted"),
    plan_polarity,
    "AI: polarity"
);

fn plan_polarity(plan: &mut Plan, args: &Map<String, Value>) -> Result<(), ToolError> {
    let o = output(&plan.st, args)?;
    let invert = need(bool_arg(args, "inverted"), "inverted")?;
    let routes: Vec<(u8, f32)> = (0..plan.st.num_input_channels)
        .filter(|i| {
            plan.st
                .matrix_routing
                .get(usize::from(*i))
                .and_then(|r| r.get(usize::from(o)))
                .copied()
                .unwrap_or(false)
        })
        .map(|i| {
            let g = plan
                .st
                .matrix_gain
                .get(usize::from(i))
                .and_then(|r| r.get(usize::from(o)))
                .copied()
                .unwrap_or(0.0);
            (i, g)
        })
        .collect();
    if routes.is_empty() {
        return Err(ToolError::new(format!(
            "no input feeds output {o}; route one to it first (dspi_set_route)"
        )));
    }
    for (i, gain) in routes {
        plan.push(Op::Route {
            i,
            o,
            on: true,
            gain,
            invert,
        })?;
    }
    Ok(())
}

edit_tool!(
    SET_MUTE,
    "dspi_set_mute",
    "Mute an output",
    "Mute (true) or unmute an output. Muting passes; an unmute may be held for the user's \
     confirmation.",
    || output_flag_schema("muted"),
    plan_mute,
    "AI: mute"
);

fn plan_mute(plan: &mut Plan, args: &Map<String, Value>) -> Result<(), ToolError> {
    let o = output(&plan.st, args)?;
    let muted = need(bool_arg(args, "muted"), "muted")?;
    plan.push(Op::OutMute { o, muted })
}

edit_tool!(
    SET_OUTPUT_ENABLED,
    "dspi_set_output_enabled",
    "Turn an output on or off",
    "Enable or disable an output. Turning one on may be held for the user's confirmation; the \
     device may refuse an enable that collides with another output.",
    || output_flag_schema("enabled"),
    plan_output_enabled,
    "AI: output on/off"
);

fn plan_output_enabled(plan: &mut Plan, args: &Map<String, Value>) -> Result<(), ToolError> {
    let o = output(&plan.st, args)?;
    let on = need(bool_arg(args, "enabled"), "enabled")?;
    plan.push(Op::OutEnable { o, on })
}

// ---------------------------------------------------------------- routing

edit_tool!(
    SET_ROUTE,
    "dspi_set_route",
    "Set a route",
    "One crosspoint of the routing matrix: `input` to `output`, `on`, with `gain_db` and \
     `invert` (each kept as it is when left out). A new route or a raised gain may be held for \
     the user's confirmation.",
    || {
        let mut m = Map::new();
        m.insert("input".into(), json!({"type": "integer", "minimum": 0}));
        m.insert("output".into(), json!({"type": "integer", "minimum": 0}));
        m.insert("on".into(), json!({"type": "boolean"}));
        m.insert(
            "gain_db".into(),
            json!({"type": "number", "minimum": -60, "maximum": 20}),
        );
        m.insert("invert".into(), json!({"type": "boolean"}));
        schema(m, &["input", "output", "on"])
    },
    plan_route,
    "AI: route"
);

fn plan_route(plan: &mut Plan, args: &Map<String, Value>) -> Result<(), ToolError> {
    let i = input(&plan.st, args)?;
    let o = output(&plan.st, args)?;
    let on = need(bool_arg(args, "on"), "on")?;
    let cell = |v: &Vec<Vec<bool>>| {
        v.get(usize::from(i))
            .and_then(|r| r.get(usize::from(o)))
            .copied()
            .unwrap_or(false)
    };
    let gain = f32_arg(args, "gain_db").unwrap_or_else(|| {
        plan.st
            .matrix_gain
            .get(usize::from(i))
            .and_then(|r| r.get(usize::from(o)))
            .copied()
            .unwrap_or(0.0)
    });
    let invert = bool_arg(args, "invert").unwrap_or_else(|| cell(&plan.st.matrix_invert));
    plan.push(Op::Route {
        i,
        o,
        on,
        gain,
        invert,
    })
}

// ----------------------------------------------------------------- volume

edit_tool!(
    SET_VOLUME,
    "dspi_set_volume",
    "Set the volume",
    "The `user` volume (-60..0 dB, what the computer's volume drives) or the device's `master` \
     volume (-127..0 dB in 0.5 dB steps; -128 mutes). Lowering passes; raising may be held for \
     the user's confirmation in the app.",
    || {
        let mut m = Map::new();
        m.insert("volume".into(), json!({"enum": ["user", "master"]}));
        m.insert(
            "db".into(),
            json!({"type": "number", "minimum": -128, "maximum": 0}),
        );
        schema(m, &["volume", "db"])
    },
    plan_volume,
    "AI: volume"
);

fn plan_volume(plan: &mut Plan, args: &Map<String, Value>) -> Result<(), ToolError> {
    let db = need(f32_arg(args, "db"), "db")?;
    plan.push(match args.get("volume").and_then(Value::as_str) {
        Some("master") => Op::MasterVolume(db),
        Some("user") => Op::UserVolume(db.clamp(-60.0, 0.0)),
        _ => return Err(ToolError::new("`volume` is user or master")),
    })
}

edit_tool!(
    SET_USER_MUTE,
    "dspi_set_user_mute",
    "Mute the volume",
    "Mute (true) or unmute the user volume. An unmute may be held for the user's confirmation.",
    || {
        let mut m = Map::new();
        m.insert("muted".into(), json!({"type": "boolean"}));
        schema(m, &["muted"])
    },
    plan_user_mute,
    "AI: volume mute"
);

fn plan_user_mute(plan: &mut Plan, args: &Map<String, Value>) -> Result<(), ToolError> {
    plan.push(Op::UserMute(need(bool_arg(args, "muted"), "muted")?))
}

// ------------------------------------------------------------------- undo

/// The most of an agent's own changes `dspi_undo` looks back through.
const UNDO_LOOKBACK: usize = 20;

static UNDO: Tool = Tool {
    name: "dspi_undo",
    title: "Undo this agent's change",
    description: "Undo one of this agent's own changes on a device: `undo_id` from a write's \
                  result or dspi_get_activity, or by default this agent's newest change not yet \
                  undone. The user's changes, and other agents', are never undone by an agent. \
                  Undoing a cut is a boost: it is judged like any change.",
    class: ToolClass::Write,
    device: true,
    input_schema: || {
        let mut m = Map::new();
        m.insert("undo_id".into(), json!({"type": "integer", "minimum": 1}));
        json!({"type": "object", "properties": m, "additionalProperties": false})
    },
    output_schema: object_schema,
    handler: undo,
};

fn undo(call: &mut Call<'_>, args: &Map<String, Value>) -> ToolResult {
    let core = call.core()?;
    let st = state(core)?;
    let history = core.history(1000);
    let given = args.get("undo_id").and_then(Value::as_u64);
    let candidates: Vec<u64> = match given {
        Some(id) => vec![id],
        None => {
            let mut ids: Vec<u64> = Vec::new();
            for e in history
                .iter()
                .filter(|e| e.actor == call.actor && e.source == HistorySource::Local && e.undoable)
            {
                if !ids.contains(&e.id) {
                    ids.push(e.id);
                }
            }
            ids.truncate(UNDO_LOOKBACK);
            ids
        }
    };
    if candidates.is_empty() {
        return Err(ToolError::new(
            "this agent has no change to undo on this device",
        ));
    }
    for id in candidates {
        let values: Vec<_> = history.iter().filter(|e| e.id == id).collect();
        if values.is_empty() {
            return Err(ToolError::new(
                "no change with that undo_id in this device's history (dspi_get_history lists them)",
            ));
        }
        if values.iter().any(|e| e.actor != call.actor) {
            return Err(ToolError::new(
                "that change is not this agent's; only the user can undo it. Do not retry.",
            ));
        }
        // -- WP FUNC2b -- (FUNC2b S15) a change with no value before it to put back (one
        // written to every output at once) read as "already undone", and the default walk
        // went on to undo an older change instead, answering ok
        if values.iter().any(|e| e.before.is_none()) {
            let what = values
                .iter()
                .find_map(|e| e.label.clone())
                .unwrap_or_else(|| values.first().map_or_else(String::new, |e| e.path.clone()));
            return Err(ToolError::new(format!(
                "this agent's change {id} ({what}) cannot be undone: the device's history has no \
                 value from before it to put back (it was written to every output at once). \
                 Nothing was undone. Set it back with the tool that made it, per output, or \
                 ask the user to."
            )));
        }
        let writes: Vec<WriteKey> = values
            .iter()
            .filter_map(|e| {
                e.before
                    .clone()
                    .map(|b| WriteKey::new(&e.path, &e.indices, b))
            })
            .collect();
        policy::prejudge(&call.judging, &st, &writes)?;
        let core = call.core()?;
        let moved = core
            .revert_entry(id)
            .map_err(|e| policy::core_error(&call.judging, e))?;
        if moved.entry.is_some() {
            return Ok(json!({"ok": true, "device": call.device_text(), "undone": id}));
        }
        if !moved.skipped.is_empty() {
            return Err(ToolError::new(
                "someone has changed those values since; nothing was undone",
            ));
        }
        if given.is_some() {
            return Err(ToolError::new("that change has already been undone"));
        }
    }
    Err(ToolError::new(
        "this agent's recent changes on this device are already undone",
    ))
}
