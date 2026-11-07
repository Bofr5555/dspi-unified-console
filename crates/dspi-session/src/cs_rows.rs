//! The control-surface editing rules as data (WP CG20).
//!
//! [`cs_model`](crate::cs_model) holds the rules as functions, and every client
//! that could not call them kept a copy: Qt's `CsHelper.qml`, the Swift tab. The
//! copies drift. This module says what a page shows for one record, and what an
//! edit makes of that record, so a client adds only the drawing:
//!
//! - [`describe`]: the rows of a record, each with its id, words and what it offers.
//! - [`edit`]: the next record after a row's value changed, or a refusal if the
//!   row or the value is not one [`describe`] offers.
//! - [`new_ir`], [`new_step`], [`new_page`] and `cs_model`'s
//!   [`make_binding`]: a fresh record that the firmware takes.
//!
//! It holds no I/O, no clock and no threads. The row words are the ones the Terminal
//! (`dspi-tui/src/settings/surfaces.rs`) and Qt show, and they are the Console's.
//!
//! Row ids, by record:
//!
//! | Record | Rows |
//! |---|---|
//! | control | `type noun target band action event gpio0 gpio1 value choice step spanOn spanMin spanMax onDelay offDelay delayNote bright reverse accel wrap repeat linkAbs groupAll invert invertShared` |
//! | IR receiver | `type gpio0 invert` |
//! | display wiring | `type model sda address` |
//! | aux output | `type gpio0 invert limit linear onDelay offDelay delayNote boot savedNote startsOn startLevel drivenBy driver.N` |
//! | remote button | `noun target band action value choice step wrap repeat` |
//! | macro step | `noun action target band value choice step wrap preDelay` |
//! | display page | `noun target band large bar` |
//! | display settings | `mode home dwell overlay overlayAny editTimeout editGated gateNote brightness labelAlign valueAlign` |
//! | group | `kind member.N noChannels noMembers usedBy` |
//!
//! Numbers in rows and in edits are in natural units: dB, Hz, seconds, percent.

use serde::Serialize;

use dspi_proto::packets::{display_flags, page_flags};

use crate::DeviceState;
use crate::cs_model::{
    ALIGN_NAMES, CsContext, CsData, DELAY_MAX_SECONDS, DISPLAY_ADDRESSES, DISPLAY_MIN_DWELL,
    EVENT_NAMES, LED_BRIGHT_MAX, NOUN_CATEGORIES, STEP_DELAY_MAX_SECONDS, act, aux_extras,
    band_name, band_options, binding_actions, binding_gestures, bool_label, compatible_groups,
    decode_delay, decode_step, decode_step_delay, decode_value, default_action,
    default_ir_operands, default_macro_step, default_operands, default_step, default_step_operands,
    delays_allowed, display_default_address, display_is_graphic, display_model_name,
    display_models, display_page_nouns, display_page_summary, encode_delay, encode_step,
    encode_step_delay, encode_value, enum_value_label, flag, fmt_unit, group_channel_name,
    group_menu_label, group_summary, i2c_sda_candidates, invert_detail, invert_title,
    ir_verb_phrase, is_aux, is_indicator, kind, macro_step_actions, macro_step_nouns,
    macro_step_summary, make_binding, noun, noun_menu_label, noun_name, noun_range, noun_unit,
    operand_kind, page_bar_allowed, picker_addresses, pin_candidates, pin_detail, repeat_allowed,
    sanitize_repeat, set_ir_noun, set_noun_in, slot_owner, switch_aux_kind, target,
    target_addresses, target_label, target_noun, ty, type_name, unit_decimals, unit_is_log,
    unit_min_step, unit_scroll_step, unit_symbol, valid_actions, valid_nouns, verb_phrase,
};
use crate::surfaces::{
    AUX_LEVEL_MAX_Q8, CsBinding, CsDisplayCfg, CsDisplayPage, CsGroup, CsMacroStep, GPIO_UNUSED,
    IrCommand,
};

/// A target picker's value for group `g`: channel `t` is `t`, group `g` is this
/// plus `g`. No channel space reaches it.
pub const GROUP_VALUE: i32 = 1000;

/// Words for the delay rows' warning, shared by an indicator and an aux output.
pub const DELAY_WARNING: &str = "Applying, reverting, or rebooting briefly releases the pin and \
                                 restarts the timing from off. Driving an amplifier trigger, that \
                                 is a power cycle.";
/// Shown when editing is gated and nothing can lift the gate.
pub const EDIT_GATE_WARNING: &str = "Nothing can arm editing, so a Browse/Adjust control can only \
                                     browse pages. Bind a button or remote key to Allow Editing.";

/// One thing a pick row offers.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Choice {
    /// What [`edit`] takes for it.
    pub value: i32,
    /// Its words, inside its section when it has one.
    pub text: String,
    /// The heading it sits under.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub section: Option<String>,
    /// Who else holds this pin. An owner on an offered pin is a button sharing
    /// another button's GPIO; on a disabled one, the reason.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owner: Option<String>,
    /// Shown but not offered: [`edit`] refuses it.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub disabled: bool,
}

impl Choice {
    fn new(value: i32, text: impl Into<String>) -> Self {
        Self {
            value,
            text: text.into(),
            section: None,
            owner: None,
            disabled: false,
        }
    }
}

/// What a row holds.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum RowBody {
    /// One of a list. `text` is the current value in full, for the closed picker.
    Pick {
        value: i32,
        text: String,
        options: Vec<Choice>,
    },
    /// A number in natural units.
    Number {
        value: f64,
        min: f64,
        max: f64,
        step: f64,
        unit: String,
        decimals: u8,
    },
    /// On or off. A row that may not be turned on says so in `enabled`.
    Toggle { on: bool, enabled: bool },
    /// Words only; `detail` holds them.
    Note { warning: bool },
}

/// One row of a page.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Row {
    /// Stable: what [`edit`] takes, and what a client may key a layout on.
    pub id: String,
    pub title: String,
    pub detail: String,
    #[serde(flatten)]
    pub body: RowBody,
}

/// A record being edited.
#[derive(Debug, Clone, PartialEq)]
pub enum Draft {
    Binding(CsBinding),
    Ir(IrCommand),
    Step(CsMacroStep),
    Page(CsDisplayPage),
    Display(CsDisplayCfg),
    Group(CsGroup),
}

/// What a page shows for one record.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Description {
    /// The record's name when it has no name of its own.
    pub title: String,
    /// The sentence a collapsed card or a list row shows.
    pub summary: String,
    pub rows: Vec<Row>,
}

/// Why [`edit`] did not change the record.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum EditError {
    /// The record has no such row just now, or the row does not offer the value.
    #[error("{id} does not offer {value}")]
    NotOffered { id: String, value: f64 },
}

fn pick(id: &str, title: &str, detail: impl Into<String>, value: i32, options: Vec<Choice>) -> Row {
    let text = options
        .iter()
        .find(|o| o.value == value)
        .map(|o| o.text.clone())
        .unwrap_or_default();
    Row {
        id: id.into(),
        title: title.into(),
        detail: detail.into(),
        body: RowBody::Pick {
            value,
            text,
            options,
        },
    }
}

fn toggle(id: &str, title: &str, detail: impl Into<String>, on: bool) -> Row {
    Row {
        id: id.into(),
        title: title.into(),
        detail: detail.into(),
        body: RowBody::Toggle { on, enabled: true },
    }
}

fn note(id: &str, detail: impl Into<String>, warning: bool) -> Row {
    Row {
        id: id.into(),
        title: String::new(),
        detail: detail.into(),
        body: RowBody::Note { warning },
    }
}

#[derive(Debug, Clone, Copy)]
struct Num {
    value: f64,
    min: f64,
    max: f64,
    step: f64,
    decimals: u8,
}

fn number(id: &str, title: &str, detail: impl Into<String>, unit: &str, n: Num) -> Row {
    Row {
        id: id.into(),
        title: title.into(),
        detail: detail.into(),
        body: RowBody::Number {
            value: n.value,
            min: n.min,
            max: n.max,
            step: n.step,
            unit: unit.into(),
            decimals: n.decimals,
        },
    }
}

/// Which record's rules and words apply to a binding-shaped view.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Mode {
    Binding,
    Ir,
    Step,
}

/// A remote button as a binding, as the firmware's `cs_ir_view`.
fn ir_view(c: &IrCommand) -> CsBinding {
    CsBinding {
        component: ty::IR,
        noun: c.noun,
        action: c.action,
        flags: c.flags,
        target: c.target,
        index: c.index,
        value: c.value,
        step: c.step,
        ..Default::default()
    }
}

/// A macro step as a binding, as the firmware's `cs_macro_step_view`.
fn step_view(s: &CsMacroStep) -> CsBinding {
    CsBinding {
        component: ty::BUTTON,
        noun: s.noun,
        action: s.action,
        flags: s.flags,
        target: s.target,
        index: s.index,
        value: s.value,
        step: s.step,
        ..Default::default()
    }
}

/// The nouns as sectioned choices: the families in the Console's order, and
/// whatever no family claims under "Other".
fn noun_options(valid: &[u8], t: u8) -> Vec<Choice> {
    let mut groups: Vec<(&str, Vec<u8>)> = NOUN_CATEGORIES
        .iter()
        .map(|c| {
            let ns: Vec<u8> = c
                .nouns
                .iter()
                .copied()
                .filter(|n| valid.contains(n))
                .collect();
            (c.name, ns)
        })
        .filter(|(_, ns)| !ns.is_empty())
        .collect();
    let others: Vec<u8> = valid
        .iter()
        .copied()
        .filter(|n| !NOUN_CATEGORIES.iter().any(|c| c.nouns.contains(n)))
        .collect();
    if !others.is_empty() {
        groups.push(("Other", others));
    }
    let one = groups.len() <= 1;
    groups
        .into_iter()
        .flat_map(|(cat, ns)| {
            ns.into_iter().map(move |n| Choice {
                section: Some(cat.to_string()),
                // Alone, a family's name says nothing, so the item reads in full.
                ..Choice::new(
                    i32::from(n),
                    if one {
                        noun_name(n, t)
                    } else {
                        noun_menu_label(cat, n, t)
                    },
                )
            })
        })
        .collect()
}

/// The function picker. `text` is always the full name: the closed picker has no
/// family heading to lean on.
fn noun_row(title: &str, detail: &str, n: u8, valid: &[u8], t: u8) -> Row {
    let mut r = pick("noun", title, detail, i32::from(n), noun_options(valid, t));
    if let RowBody::Pick { text, .. } = &mut r.body {
        *text = noun_name(n, t);
    }
    r
}

/// What a record points at, for [`target_rows`].
#[derive(Debug, Clone, Copy)]
struct Aim<'a> {
    noun: u8,
    own: Option<usize>,
    action: u8,
    target: u8,
    index: u8,
    grouped: bool,
    /// The end of the row's sentence: "this control affects."
    what: &'a str,
    /// A band on a group says it means every member.
    band_group_note: bool,
}

/// The merged channel and group picker, and the band under it. A group since
/// emptied or re-kinded stays listed, so the picker never moves the record by itself.
fn target_rows(cx: &dyn CsContext, cs: &CsData, aim: Aim<'_>) -> Vec<Row> {
    let Aim {
        noun: n,
        own,
        action,
        target: tgt,
        index: idx,
        grouped,
        what,
        band_group_note,
    } = aim;
    let mut rows = Vec::new();
    let Some(nd) = cs.noun_desc(n).copied() else {
        return rows;
    };
    if !nd.is_targeted() {
        return rows;
    }
    // TRIGGER is never groupable: the firmware has no targeted trigger.
    let mut groups = if action == act::TRIGGER {
        Vec::new()
    } else {
        compatible_groups(cs, n)
    };
    let alone = groups.is_empty() && !grouped;
    if grouped && !groups.contains(&tgt) {
        groups.push(tgt);
    }
    let section = if nd.target_kind == target::AUX {
        "Auxiliary Outputs"
    } else {
        "Channels"
    };
    let mut options: Vec<Choice> = picker_addresses(cs, n, own, tgt, grouped)
        .into_iter()
        .map(|a| Choice {
            section: Some(section.to_string()),
            ..Choice::new(i32::from(a), target_label(cx.state(), cs, &nd, a))
        })
        .collect();
    options.extend(groups.iter().map(|g| Choice {
        section: Some("Groups".to_string()),
        ..Choice::new(
            GROUP_VALUE.saturating_add(i32::from(*g)),
            group_menu_label(cs, *g),
        )
    }));
    let value = if grouped {
        GROUP_VALUE.saturating_add(i32::from(tgt))
    } else {
        i32::from(tgt)
    };
    let noun_word = target_noun(&nd);
    rows.push(if alone {
        pick(
            "target",
            noun_word,
            format!("Which {} {what}", noun_word.to_lowercase()),
            value,
            options,
        )
    } else {
        pick(
            "target",
            "Channel or Group",
            format!("Which channel, or named set of channels, {what}"),
            value,
            options,
        )
    });
    if nd.has_band() {
        let bands = band_options(cs, cx.state(), n, tgt, grouped);
        rows.push(pick(
            "band",
            "Band",
            if band_group_note && grouped {
                format!(
                    "Which filter band {}, on every member of the group.",
                    what.trim_end_matches('.')
                )
            } else {
                format!("Which filter band {what}")
            },
            i32::from(idx),
            bands
                .into_iter()
                .map(|b| Choice::new(i32::from(b), band_name(b)))
                .collect(),
        ));
    }
    rows
}

/// The choice row of a bool or enum noun.
fn choice_row(cx: &dyn CsContext, cs: &CsData, v: &CsBinding, title: &str, detail: &str) -> Row {
    let k = operand_kind(cs, v.noun);
    if k == kind::BOOL {
        return pick(
            "choice",
            title,
            detail,
            i32::from(v.value != 0),
            vec![
                Choice::new(0, bool_label(v.noun, false)),
                Choice::new(1, bool_label(v.noun, true)),
            ],
        );
    }
    let count = cs
        .noun_desc(v.noun)
        .map(|d| d.enum_count.max(1))
        .unwrap_or(1);
    pick(
        "choice",
        title,
        detail,
        i32::from(v.value),
        (0..count)
            .map(|c| Choice::new(i32::from(c), enum_value_label(cx, cs, v.noun, i32::from(c))))
            .collect(),
    )
}

fn value_num(cs: &CsData, n: u8, q: i16) -> Num {
    let u = noun_unit(cs, n);
    let (lo, hi) = noun_range(cs, n);
    Num {
        value: decode_value(q, u),
        min: lo,
        max: hi,
        step: unit_scroll_step(u),
        decimals: unit_decimals(u) as u8,
    }
}

/// The smallest step a noun's unit offers: a log unit counts in octaves.
fn min_step(u: u8) -> f64 {
    if unit_is_log(u) {
        1.0 / 48.0
    } else {
        unit_min_step(u)
    }
}

/// What a press or detent changes: the rows for value, selection, step and span.
fn operand_rows(cx: &dyn CsContext, cs: &CsData, mode: Mode, v: &CsBinding) -> Vec<Row> {
    let mut rows = Vec::new();
    let k = operand_kind(cs, v.noun);
    let u = noun_unit(cs, v.noun);
    let (lo, hi) = noun_range(cs, v.noun);
    let span = format!("({} to {}).", fmt_unit(lo, u), fmt_unit(hi, u));
    let per = match mode {
        Mode::Binding => "detent/press",
        _ => "press",
    };
    match v.action {
        act::STEP | act::INC | act::DEC => {
            if k == kind::CONTINUOUS {
                let log = unit_is_log(u);
                let detail = match (mode, log) {
                    (Mode::Step, true) => "How far each run moves it, in octaves.".to_string(),
                    (Mode::Step, false) => "How far each run moves it.".to_string(),
                    (_, true) => format!("Ratio per {per}, in octaves."),
                    (_, false) => format!("Amount added or removed per {per}."),
                };
                rows.push(number(
                    "step",
                    "Step Size",
                    detail,
                    if log { "oct" } else { unit_symbol(u) },
                    Num {
                        value: if v.step == 0 {
                            default_step(u)
                        } else {
                            decode_step(v.step, u)
                        },
                        min: min_step(u),
                        max: (hi - lo).abs().max(1.0),
                        step: if log {
                            default_step(u)
                        } else {
                            unit_scroll_step(u)
                        },
                        decimals: if log { 3 } else { unit_decimals(u) as u8 },
                    },
                ));
            } else if k == kind::ENUM && mode != Mode::Step {
                let max = cs
                    .noun_desc(v.noun)
                    .map(|d| d.enum_count.max(2).saturating_sub(1))
                    .unwrap_or(1);
                rows.push(number(
                    "step",
                    "Step Size",
                    format!("Positions advanced per {per}."),
                    "",
                    Num {
                        value: if v.step <= 0 { 1.0 } else { f64::from(v.step) },
                        min: 1.0,
                        max: f64::from(max),
                        step: 1.0,
                        decimals: 0,
                    },
                ));
            }
        }
        act::SET | act::MOMENTARY => {
            let verb = if v.action == act::MOMENTARY {
                "Hold Value"
            } else {
                "Set To"
            };
            let press = mode != Mode::Step;
            match k {
                kind::CONTINUOUS => rows.push(number(
                    "value",
                    verb,
                    if press {
                        format!("Level each press applies {span}")
                    } else {
                        format!("The value this step applies {span}")
                    },
                    unit_symbol(u),
                    value_num(cs, v.noun, v.value),
                )),
                kind::BOOL => rows.push(choice_row(
                    cx,
                    cs,
                    v,
                    verb,
                    if press {
                        "The state a press applies."
                    } else {
                        "The state this step applies."
                    },
                )),
                kind::ENUM => rows.push(choice_row(
                    cx,
                    cs,
                    v,
                    verb,
                    if press {
                        "The selection a press applies."
                    } else {
                        "The selection this step applies."
                    },
                )),
                _ => {}
            }
        }
        act::IND_EQUALS => match k {
            kind::BOOL => rows.push(choice_row(
                cx,
                cs,
                v,
                "Light When",
                "Light the LED when the function is in this state.",
            )),
            kind::ENUM => rows.push(choice_row(
                cx,
                cs,
                v,
                "Light When",
                "Light the LED for this selection.",
            )),
            _ => {}
        },
        act::IND_ABOVE if k == kind::CONTINUOUS => {
            // A threshold on the floor compares true even in silence. The floor
            // is a legal value, so say what it does rather than refuse it.
            let floor = cs.noun_desc(v.noun).map(|d| d.min_q).unwrap_or(i16::MIN);
            let tail = if v.value <= floor {
                format!(
                    " At the {} floor this is always true, so the LED stays lit.",
                    fmt_unit(lo, u)
                )
            } else {
                String::new()
            };
            rows.push(number(
                "value",
                "Light Above",
                format!("Light the LED once the value reaches this. {span}{tail}"),
                unit_symbol(u),
                value_num(cs, v.noun, v.value),
            ));
        }
        act::ADJUST | act::IND_LEVEL if k == kind::CONTINUOUS && mode == Mode::Binding => {
            let level = v.action == act::IND_LEVEL;
            let custom = v.range_min != 0 || v.range_max != 0;
            rows.push(toggle(
                "spanOn",
                if level {
                    "Brightness Range"
                } else {
                    "Limit Range"
                },
                format!(
                    "Map onto a portion of the {} to {} range.",
                    fmt_unit(lo, u),
                    fmt_unit(hi, u)
                ),
                custom,
            ));
            if custom {
                rows.push(number(
                    "spanMin",
                    "Minimum",
                    if level {
                        "Value mapped to the LED fully off."
                    } else {
                        "Level at the fully counter-clockwise position."
                    },
                    unit_symbol(u),
                    value_num(cs, v.noun, v.range_min),
                ));
                rows.push(number(
                    "spanMax",
                    "Maximum",
                    if level {
                        "Value mapped to the LED fully lit."
                    } else {
                        "Level at the fully clockwise position."
                    },
                    unit_symbol(u),
                    value_num(cs, v.noun, v.range_max),
                ));
            }
        }
        _ => {}
    }
    rows
}

fn delay_rows(b: &CsBinding, word: &str) -> Vec<Row> {
    let cap = format!(
        "Up to {} min {} s.",
        DELAY_MAX_SECONDS / 60,
        DELAY_MAX_SECONDS % 60
    );
    let seconds = |raw: u16| Num {
        value: f64::from(decode_delay(raw)),
        min: 0.0,
        max: f64::from(DELAY_MAX_SECONDS),
        step: 1.0,
        decimals: 0,
    };
    let mut rows = vec![
        number(
            "onDelay",
            "Turn-On Delay",
            format!(
                "Hold off until the condition has been true this long. Any interruption restarts \
                 the wait. {cap}"
            ),
            "s",
            seconds(b.on_delay),
        ),
        number(
            "offDelay",
            "Turn-Off Delay",
            format!(
                "Stay {word} until the condition has been false this long - long enough to hold \
                 an amplifier trigger on through quiet passages. {cap}"
            ),
            "s",
            seconds(b.off_delay),
        ),
    ];
    if b.on_delay != 0 || b.off_delay != 0 {
        rows.push(note("delayNote", DELAY_WARNING, false));
    }
    rows
}

/// The toggles of a control, remote button or step. Each flag shows only where
/// the firmware takes it (control_surfaces.c:1736-1745, 1839-1860, 2064-2084).
fn flag_rows(cs: &CsData, mode: Mode, v: &CsBinding) -> Vec<Row> {
    let mut rows = Vec::new();
    let k = operand_kind(cs, v.noun);
    let on = |mask: u8| v.flags & mask != 0;
    let binding = mode == Mode::Binding;
    if binding && (v.component == ty::POT || v.component == ty::ENCODER) {
        rows.push(toggle(
            "reverse",
            "Reverse Direction",
            if v.component == ty::POT {
                "Clockwise decreases the value."
            } else {
                "Clockwise steps down."
            },
            on(flag::REVERSE),
        ));
    }
    if binding && v.component == ty::ENCODER {
        rows.push(toggle(
            "accel",
            "Acceleration",
            "Fast spins move in larger steps; slow spins stay fine.",
            on(flag::ACCEL),
        ));
    }
    if k == kind::ENUM && matches!(v.action, act::STEP | act::INC | act::DEC) {
        rows.push(toggle(
            "wrap",
            "Wrap Around",
            "Step past the last position back to the first.",
            on(flag::WRAP),
        ));
    }
    match mode {
        Mode::Binding if repeat_allowed(v) => rows.push(toggle(
            "repeat",
            "Repeat While Held",
            "After holding ~0.4 s the press repeats automatically.",
            on(flag::REPEAT),
        )),
        Mode::Ir if v.action == act::INC || v.action == act::DEC => rows.push(toggle(
            "repeat",
            "Repeat While Held",
            "Holding the remote button repeats the step.",
            on(flag::REPEAT),
        )),
        _ => {}
    }
    if binding && on(flag::GROUP) {
        if v.action == act::ADJUST && k == kind::CONTINUOUS {
            rows.push(toggle(
                "linkAbs",
                "Match Members Exactly",
                "Drive every member to the same value. Off keeps the offsets between them, moving \
                 the group's average to the knob.",
                on(flag::LINK_ABS),
            ));
        }
        if v.action == act::IND_EQUALS || v.action == act::IND_ABOVE {
            rows.push(toggle(
                "groupAll",
                "Require Every Member",
                "Light only when all members match. Off lights when any one does.",
                on(flag::GROUP_ALL),
            ));
        }
    }
    if binding {
        rows.push(toggle(
            "invert",
            invert_title(v.component),
            invert_detail(v.component),
            on(flag::INVERT),
        ));
    }
    rows
}

/// A pin picker: what may be chosen, then the rest of the header dimmed with its
/// owner. The current pin is always listed.
fn pin_row(
    cx: &dyn CsContext,
    cs: &CsData,
    drafts: &[CsBinding],
    slot: usize,
    second: bool,
    title: &str,
    detail: &str,
) -> Row {
    let offered = pin_candidates(cx, cs, drafts, slot, second);
    let b = drafts.get(slot).cloned().unwrap_or_default();
    let current = if second { b.gpio[1] } else { b.gpio[0] };
    let owner = slot_owner(slot);
    let mut options: Vec<Choice> = Vec::new();
    if !offered.contains(&current) {
        options.push(Choice::new(
            i32::from(current),
            if current == GPIO_UNUSED {
                "None".to_string()
            } else {
                format!("GPIO {current}")
            },
        ));
    }
    let td = cs.type_desc(b.component).copied().unwrap_or_default();
    let adc = td.pin_class == dspi_proto::packets::CsTypeDesc::PINCLASS_ADC;
    let all: Vec<u8> = if adc {
        crate::cs_model::ADC_PINS.to_vec()
    } else {
        cx.valid_pins()
    };
    for p in all {
        let who = cx.pin_owner(p, &owner);
        options.push(Choice {
            owner: who,
            disabled: !offered.contains(&p),
            ..Choice::new(i32::from(p), format!("GPIO {p}"))
        });
    }
    pick(
        if second { "gpio1" } else { "gpio0" },
        title,
        detail,
        i32::from(current),
        options,
    )
}

/// The live buttons other than `slot` on `pin`: a button may share a GPIO with
/// them, one gesture each, wired the same way (control_surfaces.c:1364-1377).
fn pin_mates(cs: &CsData, slot: usize, pin: u8) -> Vec<CsBinding> {
    cs.bindings
        .iter()
        .enumerate()
        .filter(|(s, b)| {
            *s != slot
                && u8::try_from(*s).is_ok_and(|n| cs.status.is_slot_active(n))
                && b.component == ty::BUTTON
                && b.pins().contains(&pin)
        })
        .map(|(_, b)| b.clone())
        .collect()
}

/// The `drafts` the pin rules take: the device's bindings with this draft in its slot.
fn with_draft(cs: &CsData, slot: usize, b: &CsBinding) -> Vec<CsBinding> {
    let mut drafts = cs.bindings.clone();
    if drafts.len() <= slot {
        drafts.resize(slot.saturating_add(1), CsBinding::default());
    }
    if let Some(d) = drafts.get_mut(slot) {
        *d = b.clone();
    }
    drafts
}

/// Types a slot may become: every type of its page (controls, or aux outputs),
/// minus a container another slot holds. The firmware allows one receiver and
/// one display (control_surfaces.c:1636-1639, 1661-1664).
pub fn type_options(cs: &CsData, slot: usize, b: &CsBinding) -> Vec<u8> {
    let aux = is_aux(b.component);
    let taken = |t: u8| {
        cs.bindings
            .iter()
            .enumerate()
            .any(|(s, o)| s != slot && o.component == t)
    };
    cs.real_types()
        .into_iter()
        .filter(|t| is_aux(*t) == aux)
        .filter(|t| !matches!(*t, ty::IR | ty::DISPLAY) || !taken(*t))
        .collect()
}

fn type_row(cs: &CsData, slot: usize, b: &CsBinding) -> Row {
    let mut options: Vec<Choice> = type_options(cs, slot, b)
        .into_iter()
        .map(|t| Choice::new(i32::from(t), type_name(t)))
        .collect();
    if !options.iter().any(|o| o.value == i32::from(b.component)) {
        options.insert(
            0,
            Choice::new(i32::from(b.component), type_name(b.component)),
        );
    }
    pick(
        "type",
        "Component",
        "What is wired to the GPIO.",
        i32::from(b.component),
        options,
    )
}

fn control_rows(cx: &dyn CsContext, cs: &CsData, slot: usize, b: &CsBinding) -> Vec<Row> {
    let drafts = with_draft(cs, slot, b);
    let mut rows = vec![noun_row(
        "Controls",
        "The device function this control drives.",
        b.noun,
        &valid_nouns(cs, b.component),
        b.component,
    )];
    rows.extend(target_rows(
        cx,
        cs,
        Aim {
            noun: b.noun,
            own: Some(slot),
            action: b.action,
            target: b.target,
            index: b.index,
            grouped: b.flags & flag::GROUP != 0,
            what: "this control affects.",
            band_group_note: true,
        },
    ));
    let acts = binding_actions(cs, b.component, b.noun, b.event);
    if acts.len() > 1 {
        let (title, detail) = if is_indicator(b.component) {
            ("Indicates", "How the LED reflects the function.")
        } else if b.component == ty::BUTTON {
            ("On Press", "What a press does.")
        } else {
            ("Behavior", "How this control drives the function.")
        };
        rows.push(action_row(cs, title, detail, b, acts));
    }
    if b.component == ty::BUTTON {
        rows.push(pick(
            "event",
            "Gesture",
            "Which press gesture triggers this. Bind several to one button GPIO for multiple \
             functions.",
            i32::from(b.event),
            binding_gestures(b)
                .into_iter()
                // A gesture another button on this pin has is refused
                // (control_surfaces.c:1374): the pin's own is always listed.
                .filter(|e| {
                    *e == b.event || !pin_mates(cs, slot, b.gpio[0]).iter().any(|m| m.event == *e)
                })
                .map(|e| {
                    Choice::new(
                        i32::from(e),
                        EVENT_NAMES.get(usize::from(e)).copied().unwrap_or("Press"),
                    )
                })
                .collect(),
        ));
    }
    let two_pin = cs.type_desc(b.component).is_some_and(|t| t.pin_count >= 2);
    if two_pin {
        rows.push(pin_row(
            cx,
            cs,
            &drafts,
            slot,
            false,
            "GPIO A",
            "Encoder channel A.",
        ));
        rows.push(pin_row(
            cx,
            cs,
            &drafts,
            slot,
            true,
            "GPIO B",
            "Encoder channel B.",
        ));
    } else {
        rows.push(pin_row(
            cx,
            cs,
            &drafts,
            slot,
            false,
            "GPIO",
            pin_detail(b.component),
        ));
    }
    rows.extend(operand_rows(cx, cs, Mode::Binding, b));
    if delays_allowed(b.component, b.action) {
        rows.extend(delay_rows(b, "lit"));
    }
    if b.component == ty::LED_PWM {
        rows.push(limit_row(
            "bright",
            "Brightness Limit",
            "Cap on how bright this LED gets, as a share of full. Everything below the cap \
             scales with it.",
            b,
        ));
    }
    rows.extend(flag_rows(cs, Mode::Binding, b));
    // Buttons sharing a pin share its wiring: the firmware refuses one whose
    // sense differs (control_surfaces.c:1373), so the sense follows the pin.
    if b.component == ty::BUTTON && !pin_mates(cs, slot, b.gpio[0]).is_empty() {
        rows.retain(|r| r.id != "invert");
        rows.push(note(
            "invertShared",
            "This GPIO is shared with another button, which sets its wiring.",
            false,
        ));
    }
    rows
}

fn action_row(cs: &CsData, title: &str, detail: &str, v: &CsBinding, acts: Vec<u8>) -> Row {
    let is_enum = operand_kind(cs, v.noun) == kind::ENUM;
    pick(
        "action",
        title,
        detail,
        i32::from(v.action),
        acts.into_iter()
            .map(|a| {
                Choice::new(
                    i32::from(a),
                    crate::cs_model::action_name(a, v.noun, is_enum),
                )
            })
            .collect(),
    )
}

fn limit_row(id: &str, title: &str, detail: &str, b: &CsBinding) -> Row {
    number(
        id,
        title,
        detail,
        "%",
        Num {
            value: if b.base_bright == 0 {
                f64::from(LED_BRIGHT_MAX)
            } else {
                f64::from(b.base_bright)
            },
            min: 1.0,
            max: f64::from(LED_BRIGHT_MAX),
            step: 5.0,
            decimals: 0,
        },
    )
}

fn display_wiring_rows(cx: &dyn CsContext, cs: &CsData, slot: usize, b: &CsBinding) -> Vec<Row> {
    let drafts = with_draft(cs, slot, b);
    let mut pairs: Vec<Choice> = i2c_sda_candidates(cx, &drafts, slot)
        .into_iter()
        .map(|p| Choice::new(i32::from(p), format!("GPIO {p} / {}", p.saturating_add(1))))
        .collect();
    if !pairs.iter().any(|o| o.value == i32::from(b.gpio[0])) {
        pairs.insert(
            0,
            Choice::new(
                i32::from(b.gpio[0]),
                format!("GPIO {} / {}", b.gpio[0], b.gpio[0].saturating_add(1)),
            ),
        );
    }
    let mut addresses = vec![Choice::new(0, "Default")];
    addresses.extend(
        DISPLAY_ADDRESSES
            .iter()
            .map(|a| Choice::new(i32::from(*a), format!("0x{a:X}"))),
    );
    vec![
        pick(
            "model",
            "Model",
            "The type of display connected.",
            i32::from(b.index),
            display_models(cs)
                .into_iter()
                .map(|m| Choice::new(i32::from(m), display_model_name(m)))
                .collect(),
        ),
        // SCL is not offered: the pin mux fixes it as SDA + 1, so a second
        // picker could only build a pair the device rejects.
        pick(
            "sda",
            "SDA/SCL Pins",
            "Clock and data pins are chosen in fixed pairs.",
            i32::from(b.gpio[0]),
            pairs,
        ),
        pick(
            "address",
            "Address",
            format!(
                "7-bit I2C address. Default is the model's usual one (0x{:X}).",
                display_default_address(b.index)
            ),
            i32::from(b.value),
            addresses,
        ),
    ]
}

/// True when a binding, remote key or macro step drives this aux slot.
fn drives(n: u8, flags: u8, tgt: u8, slot: usize) -> bool {
    (n == noun::AUX || n == noun::AUX_LEVEL) && flags & flag::GROUP == 0 && usize::from(tgt) == slot
}

fn aux_rows(cx: &dyn CsContext, cs: &CsData, slot: usize, b: &CsBinding) -> Vec<Row> {
    let drafts = with_draft(cs, slot, b);
    let dimmable = b.component == ty::AUX_PWM;
    let mut rows = vec![
        pin_row(
            cx,
            cs,
            &drafts,
            slot,
            false,
            "GPIO",
            pin_detail(b.component),
        ),
        toggle(
            "invert",
            invert_title(b.component),
            invert_detail(b.component),
            b.flags & flag::INVERT != 0,
        ),
    ];
    if dimmable {
        rows.push(limit_row(
            "limit",
            "Level Limit",
            "Cap on the output's duty as a share of full. Everything below the cap scales with it.",
            b,
        ));
        rows.push(toggle(
            "linear",
            "Linear Response",
            "Off: the level follows the eye's curve, right for a lamp. On: duty is proportional \
             to the level, right for a fan or heater.",
            b.extras & aux_extras::LINEAR != 0,
        ));
    }
    rows.extend(delay_rows(b, "on"));
    let saved = b.extras & aux_extras::BOOT_SAVED != 0;
    rows.push(pick(
        "boot",
        "At Power-On",
        "What this output does when the device starts up.",
        i32::from(saved),
        vec![Choice::new(0, "Fixed"), Choice::new(1, "As Last Saved")],
    ));
    if saved {
        rows.push(note(
            "savedNote",
            "Saving takes a copy of the switch and level as they are at that moment, and the \
             output comes back that way after a restart. Changing them afterwards does not move \
             the stored values until the next save.",
            false,
        ));
    } else {
        rows.push(toggle(
            "startsOn",
            "Starts On",
            "Leave this off for anything that should never wake with the device, such as an \
             amplifier trigger.",
            b.extras & aux_extras::BOOT_ON != 0,
        ));
        if dimmable {
            rows.push(number(
                "startLevel",
                "Starting Level",
                "The level this output comes up at.",
                "%",
                Num {
                    value: (f64::from(b.value.max(0)) / 256.0).round(),
                    min: 0.0,
                    max: 100.0,
                    step: 5.0,
                    decimals: 0,
                },
            ));
        }
    }
    rows.push(note(
        "drivenBy",
        "Controls on the Control Surfaces page, remote keys and macros pointed at this output.",
        false,
    ));
    rows.extend(driver_rows(cs, slot, dimmable));
    rows
}

fn driver_rows(cs: &CsData, slot: usize, dimmable: bool) -> Vec<Row> {
    let mut rows = Vec::new();
    for (s, o) in cs.bindings.iter().enumerate() {
        if o.is_empty() || !drives(o.noun, o.flags, o.target, slot) {
            continue;
        }
        let name = cs.name(s);
        let name = if name.is_empty() {
            type_name(o.component)
        } else {
            name
        };
        let is_enum = operand_kind(cs, o.noun) == kind::ENUM;
        rows.push(note(
            &format!("driver.{s}"),
            format!(
                "{name} - {} on {}",
                crate::cs_model::action_name(o.action, o.noun, is_enum),
                noun_name(o.noun, o.component)
            ),
            false,
        ));
    }
    let remotes = cs
        .ir
        .iter()
        .filter(|c| !c.is_empty() && drives(c.noun, c.flags, c.target, slot))
        .count();
    let macros: Vec<String> = cs
        .macros
        .iter()
        .enumerate()
        .filter(|(_, m)| {
            m.active_steps()
                .iter()
                .any(|st| drives(st.noun, st.flags, st.target, slot))
        })
        .map(|(i, _)| cs.macro_name(i))
        .collect();
    let mut parts: Vec<String> = Vec::new();
    if remotes > 0 {
        parts.push(format!(
            "{remotes} remote key{}",
            if remotes == 1 { "" } else { "s" }
        ));
    }
    if !macros.is_empty() {
        parts.push(format!(
            "the macro{} {}",
            if macros.len() == 1 { "" } else { "s" },
            macros.join(", ")
        ));
    }
    if !parts.is_empty() {
        rows.push(note(
            "driver.other",
            format!("Also driven by {}.", parts.join(" and ")),
            false,
        ));
    }
    if rows.is_empty() {
        rows.push(note(
            "driver.none",
            if dimmable {
                "Nothing yet. Add a button on \"Aux Switch\" or an encoder or fader on \"Aux \
                 Level\" and point it at this output."
            } else {
                "Nothing yet. Add a button or switch on \"Aux Switch\" and point it at this output."
            },
            false,
        ));
    }
    rows
}

fn ir_rows(cx: &dyn CsContext, cs: &CsData, c: &IrCommand) -> Vec<Row> {
    let v = ir_view(c);
    let mut rows = vec![noun_row(
        "Controls",
        "The device function this remote button drives.",
        c.noun,
        &valid_nouns(cs, ty::IR),
        ty::IR,
    )];
    rows.extend(target_rows(
        cx,
        cs,
        Aim {
            noun: c.noun,
            own: None,
            action: c.action,
            target: c.target,
            index: c.index,
            grouped: c.flags & flag::GROUP != 0,
            what: "this affects.",
            band_group_note: false,
        },
    ));
    let acts = valid_actions(cs, ty::IR, c.noun);
    if acts.len() > 1 {
        rows.push(action_row(
            cs,
            "On Press",
            "What pressing the remote button does.",
            &v,
            acts,
        ));
    }
    rows.extend(operand_rows(cx, cs, Mode::Ir, &v));
    rows.extend(flag_rows(cs, Mode::Ir, &v));
    rows
}

fn step_rows(cx: &dyn CsContext, cs: &CsData, s: &CsMacroStep) -> Vec<Row> {
    let v = step_view(s);
    let mut rows = vec![noun_row(
        "Change",
        "Which function this step changes.",
        s.noun,
        &macro_step_nouns(cs),
        ty::BUTTON,
    )];
    let acts = macro_step_actions(cs, s.noun);
    if acts.len() > 1 {
        rows.push(action_row(
            cs,
            "How",
            "What this step does to it.",
            &v,
            acts,
        ));
    }
    rows.extend(target_rows(
        cx,
        cs,
        Aim {
            noun: s.noun,
            own: None,
            action: s.action,
            target: s.target,
            index: s.index,
            grouped: s.flags & flag::GROUP != 0,
            what: "this step affects.",
            band_group_note: false,
        },
    ));
    rows.extend(operand_rows(cx, cs, Mode::Step, &v));
    rows.extend(flag_rows(cs, Mode::Step, &v));
    rows.push(number(
        "preDelay",
        "Wait Before",
        "Delay after the previous step before this one runs.",
        "s",
        Num {
            value: decode_step_delay(s.pre_delay),
            min: 0.0,
            max: STEP_DELAY_MAX_SECONDS,
            step: 0.1,
            decimals: 2,
        },
    ));
    rows
}

fn page_rows(cx: &dyn CsContext, cs: &CsData, p: &CsDisplayPage) -> Vec<Row> {
    let mut rows = vec![noun_row(
        "Shows",
        "The value this page shows.",
        p.noun,
        &display_page_nouns(cs),
        ty::NONE,
    )];
    rows.extend(target_rows(
        cx,
        cs,
        Aim {
            noun: p.noun,
            own: None,
            action: act::SET,
            target: p.target,
            index: p.index,
            grouped: p.flags & page_flags::GROUP != 0,
            what: "this page shows.",
            band_group_note: false,
        },
    ));
    // Only a graphic panel has a large font. With no display yet, say what it does.
    let graphic = cs
        .bindings
        .iter()
        .find(|b| b.component == ty::DISPLAY)
        .is_none_or(|b| display_is_graphic(b.index));
    if graphic {
        rows.push(toggle(
            "large",
            "Large value",
            "Draw the value in the large font, on a graphic panel.",
            p.flags & page_flags::LARGE != 0,
        ));
    }
    let bar_ok = page_bar_allowed(cs, p.noun);
    rows.push(Row {
        id: "bar".into(),
        title: "Level bar".into(),
        detail: if bar_ok {
            "Draws the value's position in its range as a bar."
        } else {
            "Only a value with a range can be drawn as a bar."
        }
        .into(),
        body: RowBody::Toggle {
            on: p.flags & page_flags::BAR != 0,
            enabled: bar_ok,
        },
    });
    rows
}

/// Whether anything can lift the display's edit gate.
fn can_arm_editing(cs: &CsData) -> bool {
    let writes = |n: u8, a: u8| {
        n == noun::DISPLAY_EDIT
            && a != act::IND_EQUALS
            && a != act::IND_ABOVE
            && a != act::IND_LEVEL
    };
    cs.bindings.iter().any(|b| writes(b.noun, b.action))
        || cs.ir.iter().any(|c| writes(c.noun, c.action))
        || cs
            .macros
            .iter()
            .any(|m| m.active_steps().iter().any(|s| writes(s.noun, s.action)))
}

fn uses_page_value(cs: &CsData) -> bool {
    cs.bindings
        .iter()
        .any(|b| !b.is_empty() && b.noun == noun::PAGE_VALUE)
        || cs
            .ir
            .iter()
            .any(|c| !c.is_empty() && c.noun == noun::PAGE_VALUE)
}

fn display_rows(state: &DeviceState, cs: &CsData, c: &CsDisplayCfg) -> Vec<Row> {
    let tenths = |raw: u16, min: f64| Num {
        value: f64::from(raw) / 10.0,
        min,
        max: 655.0,
        step: 1.0,
        decimals: 1,
    };
    let mut rows = vec![pick(
        "mode",
        "Idle Behavior",
        "What the panel rests on between changes.",
        i32::from(c.mode),
        vec![
            Choice::new(0, "One page"),
            Choice::new(1, "Cycle Dashboard"),
            Choice::new(2, "Cycle All"),
        ],
    )];
    if c.mode == 0 {
        rows.push(pick(
            "home",
            "Page",
            "Which page rests on screen.",
            i32::from(c.home_page),
            (0..cs.max_pages)
                .map(|i| {
                    let one = u16::from(i).saturating_add(1);
                    let text = match cs.pages.get(usize::from(i)) {
                        Some(p) if p.is_active() => {
                            format!("{one}. {}", display_page_summary(state, cs, p))
                        }
                        _ => format!("Page {one} (empty)"),
                    };
                    Choice::new(i32::from(i), text)
                })
                .collect(),
        ));
    } else {
        rows.push(number(
            "dwell",
            "Cycle Every",
            "How long each page stays up.",
            "s",
            tenths(c.dwell, f64::from(DISPLAY_MIN_DWELL) / 10.0),
        ));
    }
    rows.push(number(
        "overlay",
        "Pop-Up Hold",
        "Duration for which a change remains on-screen. Zero turns pop-ups off.",
        "s",
        tenths(c.overlay_hold, 0.0),
    ));
    if c.overlay_hold > 0 {
        rows.push(toggle(
            "overlayAny",
            "All Changes Pop-Up",
            "Show changes made by a knob, button or remote key even if the parameter doesn't \
             correspond to a dashboard page.",
            c.flags & display_flags::OVERLAY_ANY != 0,
        ));
    }
    rows.push(number(
        "editTimeout",
        "Editing Times Out",
        "Disarm editing after this long untouched. Zero leaves it armed until switched off.",
        "s",
        tenths(c.edit_timeout, 0.0),
    ));
    rows.push(toggle(
        "editGated",
        "Arm Before Editing",
        "When enabled, an encoder or button browses pages unless Allow Editing is toggled. When \
         disabled, an encoder or button will always adjust the displayed value.",
        c.flags & display_flags::EDIT_GATED != 0,
    ));
    if c.flags & display_flags::EDIT_GATED != 0 && uses_page_value(cs) && !can_arm_editing(cs) {
        rows.push(note("gateNote", EDIT_GATE_WARNING, true));
    }
    rows.push(number(
        "brightness",
        "Brightness",
        "OLED contrast, applied when the panel next starts. Zero is the driver default.",
        "",
        Num {
            value: f64::from(c.brightness),
            min: 0.0,
            max: 255.0,
            step: 8.0,
            decimals: 0,
        },
    ));
    let aligns = || -> Vec<Choice> {
        (0..)
            .zip(ALIGN_NAMES.iter())
            .map(|(i, n)| Choice::new(i, *n))
            .collect()
    };
    rows.push(pick(
        "labelAlign",
        "Name Alignment",
        "Horizontal placement of the current page's name.",
        i32::from(c.label_align()),
        aligns(),
    ));
    rows.push(pick(
        "valueAlign",
        "Value Alignment",
        "Horizontal placement of the current page's value.",
        i32::from(c.value_align()),
        aligns(),
    ));
    rows
}

const GROUP_KINDS: [(&str, u8); 3] = [
    ("Inputs", target::INPUT_CH),
    ("Outputs", target::OUTPUT_CH),
    ("All Channels", target::DSP_CH),
];

fn group_rows(state: &DeviceState, cs: &CsData, g: usize, d: &CsGroup) -> Vec<Row> {
    let mut rows = vec![pick(
        "kind",
        "Channel Type",
        "Which set of channels the members are numbered in.",
        i32::from(d.target_kind),
        GROUP_KINDS
            .iter()
            .map(|(n, k)| Choice::new(i32::from(*k), *n))
            .collect(),
    )];
    let count = cs.group_channel_count(d.target_kind);
    if count == 0 {
        rows.push(note(
            "noChannels",
            "No channels of this type on the connected device.",
            false,
        ));
    }
    for ch in 0..count {
        let bit = 1u32.checked_shl(u32::from(ch)).unwrap_or(0);
        rows.push(toggle(
            &format!("member.{ch}"),
            &group_channel_name(state, d.target_kind, ch),
            "",
            d.member_mask & bit != 0,
        ));
    }
    // The firmware refuses a group with no members (control_surfaces.c:2216-2217),
    // so a client holds Apply back until there is one.
    if count > 0 && d.member_mask == 0 {
        rows.push(note(
            "noMembers",
            "Pick at least one channel to apply.",
            true,
        ));
    }
    let used = cs
        .bindings
        .iter()
        .filter(|b| !b.is_empty() && b.flags & flag::GROUP != 0 && usize::from(b.target) == g)
        .count();
    if used > 0 {
        rows.push(note(
            "usedBy",
            format!(
                "Used by {used} control{}. Emptying this group or changing its channel type \
                 deactivates them until it fits again.",
                if used == 1 { "" } else { "s" }
            ),
            false,
        ));
    }
    rows
}

/// What a page shows for `draft`. `slot` is the binding slot, remote button,
/// page or group index; a macro step and the display settings ignore it.
pub fn describe(cx: &dyn CsContext, cs: &CsData, slot: usize, draft: &Draft) -> Description {
    let state = cx.state();
    match draft {
        Draft::Binding(b) => {
            let mut rows = vec![type_row(cs, slot, b)];
            match b.component {
                ty::NONE => {}
                ty::IR => {
                    let drafts = with_draft(cs, slot, b);
                    rows.push(pin_row(
                        cx,
                        cs,
                        &drafts,
                        slot,
                        false,
                        "GPIO",
                        pin_detail(ty::IR),
                    ));
                    rows.push(toggle(
                        "invert",
                        invert_title(ty::IR),
                        invert_detail(ty::IR),
                        b.flags & flag::INVERT != 0,
                    ));
                }
                ty::DISPLAY => rows.extend(display_wiring_rows(cx, cs, slot, b)),
                t if is_aux(t) => rows.extend(aux_rows(cx, cs, slot, b)),
                _ => rows.extend(control_rows(cx, cs, slot, b)),
            }
            Description {
                title: type_name(b.component),
                summary: if b.is_empty() {
                    String::new()
                } else {
                    verb_phrase(state, cs, b)
                },
                rows,
            }
        }
        Draft::Ir(c) => Description {
            title: "Remote Button".into(),
            summary: ir_verb_phrase(state, cs, c),
            rows: ir_rows(cx, cs, c),
        },
        Draft::Step(s) => Description {
            title: "Step".into(),
            summary: macro_step_summary(state, cs, s),
            rows: step_rows(cx, cs, s),
        },
        Draft::Page(p) => Description {
            title: "Page".into(),
            summary: display_page_summary(state, cs, p),
            rows: page_rows(cx, cs, p),
        },
        Draft::Display(c) => Description {
            title: "Display".into(),
            summary: String::new(),
            rows: display_rows(state, cs, c),
        },
        Draft::Group(g) => Description {
            title: cs.group_name(slot),
            summary: group_summary(state, g),
            rows: group_rows(state, cs, slot, g),
        },
    }
}

/// A fresh remote button: not learned yet, on the first function a remote can
/// drive. The firmware takes it once a learn gives it a code.
pub fn new_ir(cs: &CsData) -> IrCommand {
    let n = valid_nouns(cs, ty::IR)
        .first()
        .copied()
        .unwrap_or(noun::USER_VOLUME);
    default_ir_operands(
        cs,
        &IrCommand {
            noun: n,
            target: first_address(cs, n),
            action: default_action(cs, ty::IR, n),
            ..Default::default()
        },
    )
}

/// A fresh macro step, already valid.
pub fn new_step(cs: &CsData) -> CsMacroStep {
    default_macro_step(cs)
}

/// A fresh display page, active and already valid.
pub fn new_page(cs: &CsData) -> CsDisplayPage {
    let n = display_page_nouns(cs)
        .first()
        .copied()
        .unwrap_or(noun::USER_VOLUME);
    CsDisplayPage {
        noun: n,
        target: first_address(cs, n),
        index: 0,
        flags: page_flags::ACTIVE,
    }
}

fn first_address(cs: &CsData, n: u8) -> u8 {
    target_addresses(cs, n, None).first().copied().unwrap_or(0)
}

/// One edit, already checked against its row: the option picked, the number
/// clamped to the row's range, and whether a toggle is on.
#[derive(Debug, Clone, Copy)]
struct Change<'a> {
    id: &'a str,
    picked: Option<i32>,
    number: f64,
    on: bool,
}

fn set_bit(flags: &mut u8, mask: u8, on: bool) {
    if on {
        *flags |= mask;
    } else {
        *flags &= !mask;
    }
}

/// The value as a row's option, or a refusal. The current value is always taken.
fn offered(row: &Row, value: f64) -> Option<i32> {
    let RowBody::Pick { options, .. } = &row.body else {
        return None;
    };
    // Option values fit an i32, so a saturating cast cannot match one by accident.
    let v = value.round() as i32;
    options
        .iter()
        .find(|o| o.value == v && !o.disabled)
        .map(|o| o.value)
}

/// `(target, grouped)` from a target picker's value.
fn target_value(v: i32) -> (u8, bool) {
    let grouped = v >= GROUP_VALUE;
    let t = if grouped {
        v.saturating_sub(GROUP_VALUE)
    } else {
        v
    };
    (u8::try_from(t).unwrap_or(0), grouped)
}

/// The rows every binding-shaped record has in common. False when `id` is not one.
fn edit_shared(cx: &dyn CsContext, cs: &CsData, b: &mut CsBinding, change: Change<'_>) -> bool {
    let Change {
        id,
        picked,
        number,
        on,
    } = change;
    let u = noun_unit(cs, b.noun);
    let (lo, hi) = noun_range(cs, b.noun);
    match id {
        "target" => {
            let Some(v) = picked else { return true };
            let (t, grouped) = target_value(v);
            set_bit(&mut b.flags, flag::GROUP, grouped);
            if !grouped {
                // The modifiers exist only on a group; left set, the firmware
                // refuses the whole record (control_surfaces.c:1577-1579).
                b.flags &= !(flag::LINK_ABS | flag::GROUP_ALL);
            }
            b.target = t;
            if cs.noun_desc(b.noun).is_some_and(|d| d.has_band()) {
                let opts = band_options(cs, cx.state(), b.noun, b.target, grouped);
                if !opts.contains(&b.index) {
                    b.index = opts.first().copied().unwrap_or(0);
                }
            }
        }
        "band" => b.index = picked.and_then(|v| u8::try_from(v).ok()).unwrap_or(b.index),
        "value" => b.value = encode_value(number.clamp(lo, hi), u),
        "choice" => {
            b.value = picked
                .and_then(|v| i16::try_from(v).ok())
                .unwrap_or(b.value)
        }
        "step" => {
            b.step = if operand_kind(cs, b.noun) == kind::ENUM {
                number.round().max(1.0) as i16
            } else {
                encode_step(number.max(min_step(u)), u)
            };
        }
        "wrap" => set_bit(&mut b.flags, flag::WRAP, on),
        "repeat" => set_bit(&mut b.flags, flag::REPEAT, on),
        _ => return false,
    }
    true
}

/// Move a span's edge and keep the span one the firmware takes: `min < max`
/// (control_surfaces.c:1599-1603). The other edge gives way.
fn set_span(cs: &CsData, b: &mut CsBinding, is_min: bool, number: f64) {
    let u = noun_unit(cs, b.noun);
    let nd = cs.noun_desc(b.noun).copied().unwrap_or_default();
    let (lo, hi) = noun_range(cs, b.noun);
    let q = encode_value(number.clamp(lo, hi), u);
    if is_min {
        b.range_min = q.min(nd.max_q.saturating_sub(1));
        b.range_max = b.range_max.max(b.range_min.saturating_add(1));
    } else {
        b.range_max = q.max(nd.min_q.saturating_add(1));
        b.range_min = b.range_min.min(b.range_max.saturating_sub(1));
    }
}

fn edit_binding(
    cx: &dyn CsContext,
    cs: &CsData,
    slot: usize,
    b: &CsBinding,
    change: Change<'_>,
) -> CsBinding {
    let Change {
        id,
        picked,
        number,
        on,
    } = change;
    let mut n = b.clone();
    if edit_shared(cx, cs, &mut n, change) {
        return n;
    }
    let as_u8 = |v: Option<i32>, old: u8| v.and_then(|x| u8::try_from(x).ok()).unwrap_or(old);
    match id {
        "type" => {
            let t = as_u8(picked, b.component);
            if t != b.component {
                n = if is_aux(b.component) && is_aux(t) {
                    switch_aux_kind(b, t)
                } else {
                    make_binding(cx, cs, &with_draft(cs, slot, b), slot, t)
                };
            }
        }
        "noun" => {
            let v = as_u8(picked, b.noun);
            if v != b.noun {
                n = set_noun_in(cs, b, v, Some(slot));
            }
        }
        "action" => {
            let a = as_u8(picked, b.action);
            if a != b.action {
                n.action = a;
                n = default_operands(cs, &n);
            }
        }
        "event" => {
            n.event = as_u8(picked, b.event);
            // Repeat While Held is a short-press flag; its row goes with the
            // gesture, so the flag must too.
            sanitize_repeat(&mut n);
        }
        "gpio0" => {
            n.gpio[0] = as_u8(picked, b.gpio[0]);
            let mates = pin_mates(cs, slot, n.gpio[0]);
            if n.component == ty::BUTTON
                && let Some(m) = mates.first()
            {
                // Take the pin's wiring, and a gesture nobody on it has.
                set_bit(&mut n.flags, flag::INVERT, m.flags & flag::INVERT != 0);
                if mates.iter().any(|o| o.event == n.event)
                    && let Some(e) = binding_gestures(&n)
                        .into_iter()
                        .find(|e| !mates.iter().any(|o| o.event == *e))
                {
                    n.event = e;
                    sanitize_repeat(&mut n);
                }
            }
        }
        "gpio1" => n.gpio[1] = as_u8(picked, b.gpio[1]),
        "sda" => {
            let sda = as_u8(picked, b.gpio[0]);
            n.gpio = [sda, sda.saturating_add(1)];
        }
        "model" => {
            // 0 means the model's usual address: a model change carries that
            // convention rather than stranding the old model's default.
            if b.value != 0 && b.value == i16::from(display_default_address(b.index)) {
                n.value = 0;
            }
            n.index = as_u8(picked, b.index);
        }
        "address" => {
            n.value = picked
                .and_then(|v| i16::try_from(v).ok())
                .unwrap_or(b.value)
        }
        "spanOn" => {
            let nd = cs.noun_desc(b.noun).copied().unwrap_or_default();
            n.range_min = if on { nd.min_q } else { 0 };
            n.range_max = if on { nd.max_q } else { 0 };
        }
        "spanMin" => set_span(cs, &mut n, true, number),
        "spanMax" => set_span(cs, &mut n, false, number),
        "onDelay" => n.on_delay = encode_delay(number.max(0.0) as u32),
        "offDelay" => n.off_delay = encode_delay(number.max(0.0) as u32),
        "bright" | "limit" => {
            n.base_bright = number.round().clamp(1.0, f64::from(LED_BRIGHT_MAX)) as u8;
        }
        "reverse" => set_bit(&mut n.flags, flag::REVERSE, on),
        "accel" => set_bit(&mut n.flags, flag::ACCEL, on),
        "linkAbs" => set_bit(&mut n.flags, flag::LINK_ABS, on),
        "groupAll" => set_bit(&mut n.flags, flag::GROUP_ALL, on),
        "invert" => set_bit(&mut n.flags, flag::INVERT, on),
        "linear" => set_bit(&mut n.extras, aux_extras::LINEAR, on),
        "boot" => set_bit(&mut n.extras, aux_extras::BOOT_SAVED, picked == Some(1)),
        "startsOn" => set_bit(&mut n.extras, aux_extras::BOOT_ON, on),
        "startLevel" => {
            let q8 = (number * 256.0)
                .round()
                .clamp(0.0, f64::from(AUX_LEVEL_MAX_Q8));
            n.value = q8 as i16;
        }
        _ => {}
    }
    n
}

fn edit_ir(cx: &dyn CsContext, cs: &CsData, c: &IrCommand, change: Change<'_>) -> IrCommand {
    let Change { id, picked, .. } = change;
    let as_u8 = |v: Option<i32>, old: u8| v.and_then(|x| u8::try_from(x).ok()).unwrap_or(old);
    match id {
        "noun" => {
            let n = as_u8(picked, c.noun);
            if n == c.noun {
                c.clone()
            } else {
                set_ir_noun(cs, c, n)
            }
        }
        "action" => {
            let a = as_u8(picked, c.action);
            if a == c.action {
                return c.clone();
            }
            let mut n = c.clone();
            n.action = a;
            default_ir_operands(cs, &n)
        }
        _ => {
            let mut v = ir_view(c);
            edit_shared(cx, cs, &mut v, change);
            IrCommand {
                flags: v.flags,
                target: v.target,
                index: v.index,
                value: v.value,
                step: v.step,
                ..c.clone()
            }
        }
    }
}

fn edit_step(cx: &dyn CsContext, cs: &CsData, s: &CsMacroStep, change: Change<'_>) -> CsMacroStep {
    let Change {
        id, picked, number, ..
    } = change;
    let as_u8 = |v: Option<i32>, old: u8| v.and_then(|x| u8::try_from(x).ok()).unwrap_or(old);
    match id {
        "noun" => {
            let n = as_u8(picked, s.noun);
            if n == s.noun {
                return s.clone();
            }
            let acts = macro_step_actions(cs, n);
            let mut st = s.clone();
            st.noun = n;
            st.target = first_address(cs, n);
            st.index = 0;
            st.flags &= !flag::GROUP;
            if !acts.contains(&st.action) {
                st.action = acts.first().copied().unwrap_or(act::SET);
            }
            default_step_operands(cs, &st)
        }
        "action" => {
            let a = as_u8(picked, s.action);
            if a == s.action {
                return s.clone();
            }
            let mut st = s.clone();
            st.action = a;
            default_step_operands(cs, &st)
        }
        "preDelay" => {
            let mut st = s.clone();
            st.pre_delay = encode_step_delay(number.max(0.0));
            st
        }
        _ => {
            let mut v = step_view(s);
            edit_shared(cx, cs, &mut v, change);
            CsMacroStep {
                flags: v.flags,
                target: v.target,
                index: v.index,
                value: v.value,
                step: v.step,
                ..s.clone()
            }
        }
    }
}

fn edit_page(
    cx: &dyn CsContext,
    cs: &CsData,
    p: &CsDisplayPage,
    id: &str,
    picked: Option<i32>,
    on: bool,
) -> CsDisplayPage {
    let mut n = p.clone();
    match id {
        "noun" => {
            let v = picked.and_then(|x| u8::try_from(x).ok()).unwrap_or(p.noun);
            if v != p.noun {
                n.noun = v;
                n.target = first_address(cs, v);
                n.index = 0;
                n.flags &= !page_flags::GROUP;
                // A bar needs a range; left set, the whole page is refused
                // (control_surfaces_display.c:1168-1172).
                if !page_bar_allowed(cs, v) {
                    n.flags &= !page_flags::BAR;
                }
            }
        }
        "target" => {
            let (t, grouped) = target_value(picked.unwrap_or(i32::from(p.target)));
            set_bit(&mut n.flags, page_flags::GROUP, grouped);
            n.target = t;
            if cs.noun_desc(p.noun).is_some_and(|d| d.has_band()) {
                let opts = band_options(cs, cx.state(), p.noun, t, grouped);
                if !opts.contains(&n.index) {
                    n.index = opts.first().copied().unwrap_or(0);
                }
            }
        }
        "band" => n.index = picked.and_then(|v| u8::try_from(v).ok()).unwrap_or(p.index),
        "large" => set_bit(&mut n.flags, page_flags::LARGE, on),
        "bar" => set_bit(&mut n.flags, page_flags::BAR, on),
        _ => {}
    }
    n
}

fn edit_display(
    c: &CsDisplayCfg,
    id: &str,
    picked: Option<i32>,
    number: f64,
    on: bool,
) -> CsDisplayCfg {
    let mut n = c.clone();
    let as_u8 = |old: u8| picked.and_then(|x| u8::try_from(x).ok()).unwrap_or(old);
    let tenths = (number * 10.0).round().clamp(0.0, 6553.0) as u16;
    match id {
        "mode" => {
            n.mode = as_u8(c.mode);
            // The firmware refuses a dwell under its floor in either cycle mode,
            // so entering one from a fixed page has to lift it.
            if n.mode != 0 {
                n.dwell = n.dwell.max(DISPLAY_MIN_DWELL);
            }
        }
        "home" => n.home_page = as_u8(c.home_page),
        "dwell" => n.dwell = tenths.max(DISPLAY_MIN_DWELL),
        "overlay" => n.overlay_hold = tenths,
        "overlayAny" => set_bit(&mut n.flags, display_flags::OVERLAY_ANY, on),
        "editTimeout" => n.edit_timeout = tenths,
        "editGated" => set_bit(&mut n.flags, display_flags::EDIT_GATED, on),
        "brightness" => n.brightness = number.round().clamp(0.0, 255.0) as u8,
        "labelAlign" => n.set_label_align(as_u8(c.label_align())),
        "valueAlign" => n.set_value_align(as_u8(c.value_align())),
        _ => {}
    }
    n
}

fn edit_group(g: &CsGroup, id: &str, picked: Option<i32>, on: bool) -> CsGroup {
    let mut n = g.clone();
    if id == "kind" {
        let k = picked
            .and_then(|x| u8::try_from(x).ok())
            .unwrap_or(g.target_kind);
        if k != g.target_kind {
            n.target_kind = k;
            // Members are numbered in the kind's own space.
            n.member_mask = 0;
        }
    } else if let Some(ch) = id
        .strip_prefix("member.")
        .and_then(|c| c.parse::<u32>().ok())
    {
        let bit = 1u32.checked_shl(ch).unwrap_or(0);
        if on {
            n.member_mask |= bit;
        } else {
            n.member_mask &= !bit;
        }
    }
    n
}

/// The record after row `id` took `value`: an option's value, a number in natural
/// units, or non-zero for on. A number is clamped to its row's range.
///
/// Errors: [`EditError::NotOffered`] when [`describe`] has no such row for this
/// record, the row is words only, the option is not offered, or a toggle may not
/// be turned on. Rows can vanish with an edit, so a client asks again after each.
pub fn edit(
    cx: &dyn CsContext,
    cs: &CsData,
    slot: usize,
    draft: &Draft,
    id: &str,
    value: f64,
) -> Result<Draft, EditError> {
    let refused = || EditError::NotOffered {
        id: id.to_string(),
        value,
    };
    if !value.is_finite() {
        return Err(refused());
    }
    let d = describe(cx, cs, slot, draft);
    let row = d.rows.iter().find(|r| r.id == id).ok_or_else(refused)?;
    let picked = match &row.body {
        RowBody::Pick { value: current, .. } => {
            if value.round() as i32 == *current {
                return Ok(draft.clone());
            }
            Some(offered(row, value).ok_or_else(refused)?)
        }
        RowBody::Note { .. } => return Err(refused()),
        RowBody::Toggle { enabled, .. } => {
            if !enabled && value != 0.0 {
                return Err(refused());
            }
            None
        }
        RowBody::Number { .. } => None,
    };
    let number = match &row.body {
        RowBody::Number { min, max, .. } => value.clamp(*min, max.max(*min)),
        _ => value,
    };
    let on = value != 0.0;
    let change = Change {
        id,
        picked,
        number,
        on,
    };
    Ok(match draft {
        Draft::Binding(b) => Draft::Binding(edit_binding(cx, cs, slot, b, change)),
        Draft::Ir(c) => Draft::Ir(edit_ir(cx, cs, c, change)),
        Draft::Step(s) => Draft::Step(edit_step(cx, cs, s, change)),
        Draft::Page(p) => Draft::Page(edit_page(cx, cs, p, id, picked, on)),
        Draft::Display(c) => Draft::Display(edit_display(c, id, picked, number, on)),
        Draft::Group(g) => Draft::Group(edit_group(g, id, picked, on)),
    })
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_cg20 {
    use super::*;
    use crate::cs_model::{StaticContext, demo, evt};

    fn fixture() -> (CsData, DeviceState) {
        (demo::data(), demo::state())
    }

    fn cx(state: &DeviceState) -> StaticContext<'_> {
        StaticContext {
            state,
            claims: &[],
            control_i2c_sda: None,
            preset_names: &[],
        }
    }

    fn row<'a>(d: &'a Description, id: &str) -> Option<&'a Row> {
        d.rows.iter().find(|r| r.id == id)
    }

    fn options(r: &Row) -> Vec<i32> {
        match &r.body {
            RowBody::Pick { options, .. } => options
                .iter()
                .filter(|o| !o.disabled)
                .map(|o| o.value)
                .collect(),
            _ => Vec::new(),
        }
    }

    /// A momentary button. Which noun takes it depends on the demo's table.
    fn momentary_button(cs: &CsData) -> CsBinding {
        let n = valid_nouns(cs, ty::BUTTON)
            .into_iter()
            .find(|n| valid_actions(cs, ty::BUTTON, *n).contains(&act::MOMENTARY))
            .expect("a noun that takes Hold");
        CsBinding {
            component: ty::BUTTON,
            noun: n,
            action: act::MOMENTARY,
            gpio: [2, GPIO_UNUSED],
            ..Default::default()
        }
    }

    #[test]
    fn cg20_hold_is_only_offered_on_a_short_press() {
        let (cs, st) = fixture();
        let c = cx(&st);
        let b = momentary_button(&cs);
        let d = describe(&c, &cs, 0, &Draft::Binding(b.clone()));
        assert_eq!(options(row(&d, "event").unwrap()), vec![evt::PRESS as i32]);
        assert!(options(row(&d, "action").unwrap()).contains(&(act::MOMENTARY as i32)));

        let long = CsBinding {
            action: act::TOGGLE,
            event: evt::LONG,
            ..b
        };
        let d = describe(&c, &cs, 0, &Draft::Binding(long.clone()));
        assert!(!options(row(&d, "action").unwrap()).contains(&(act::MOMENTARY as i32)));
        assert_eq!(options(row(&d, "event").unwrap()).len(), 3);
        // And an edit cannot get there either.
        assert!(
            edit(
                &c,
                &cs,
                0,
                &Draft::Binding(long),
                "action",
                act::MOMENTARY as f64
            )
            .is_err()
        );
    }

    #[test]
    fn cg20_a_long_press_drops_the_repeat_flag() {
        let (cs, st) = fixture();
        let c = cx(&st);
        let n = valid_nouns(&cs, ty::BUTTON)
            .into_iter()
            .find(|n| valid_actions(&cs, ty::BUTTON, *n).contains(&act::INC))
            .unwrap();
        let b = CsBinding {
            component: ty::BUTTON,
            noun: n,
            action: act::INC,
            flags: flag::REPEAT,
            gpio: [2, GPIO_UNUSED],
            ..Default::default()
        };
        let Draft::Binding(after) = edit(
            &c,
            &cs,
            0,
            &Draft::Binding(b),
            "event",
            f64::from(evt::LONG),
        )
        .unwrap() else {
            panic!("a binding");
        };
        assert_eq!(after.flags & flag::REPEAT, 0);
        assert_eq!(after.event, evt::LONG);
    }

    #[test]
    fn cg20_rows_that_are_gone_refuse_an_edit() {
        let (cs, st) = fixture();
        let c = cx(&st);
        let b = make_binding(&c, &cs, &cs.bindings, 0, ty::BUTTON);
        let draft = Draft::Binding(b);
        // A button has no span, and "gpio1" belongs to an encoder.
        assert!(edit(&c, &cs, 0, &draft, "spanMin", 1.0).is_err());
        assert!(edit(&c, &cs, 0, &draft, "gpio1", 3.0).is_err());
        assert!(edit(&c, &cs, 0, &draft, "nonsense", 3.0).is_err());
        assert!(edit(&c, &cs, 0, &draft, "noun", f64::NAN).is_err());
    }

    #[test]
    fn cg20_a_span_stays_one_the_firmware_takes() {
        let (cs, st) = fixture();
        let c = cx(&st);
        let n = valid_nouns(&cs, ty::POT)
            .into_iter()
            .find(|n| operand_kind(&cs, *n) == kind::CONTINUOUS)
            .unwrap();
        let mut b = make_binding(&c, &cs, &cs.bindings, 0, ty::POT);
        b = set_noun_in(&cs, &b, n, Some(0));
        b.action = act::ADJUST;
        let mut d = Draft::Binding(b);
        d = edit(&c, &cs, 0, &d, "spanOn", 1.0).unwrap();
        let (lo, hi) = noun_range(&cs, n);
        // Push the bottom through the top, then the top through the bottom.
        d = edit(&c, &cs, 0, &d, "spanMin", hi).unwrap();
        let Draft::Binding(b) = &d else { panic!() };
        assert!(b.range_min < b.range_max, "{} {}", b.range_min, b.range_max);
        d = edit(&c, &cs, 0, &d, "spanMax", lo).unwrap();
        let Draft::Binding(b) = &d else { panic!() };
        assert!(b.range_min < b.range_max, "{} {}", b.range_min, b.range_max);
    }

    #[test]
    fn cg20_an_aux_noun_points_at_an_aux_output_never_its_own_slot() {
        let (mut cs, st) = fixture();
        let c = cx(&st);
        let aux = CsBinding {
            component: ty::AUX_OUT,
            gpio: [3, GPIO_UNUSED],
            ..Default::default()
        };
        if let Some(b) = cs.bindings.get_mut(5) {
            *b = aux;
        }
        let d = describe(&c, &cs, 5, &Draft::Binding(cs.binding(5)));
        assert!(row(&d, "noun").is_none(), "an aux output has no function");
        let b = make_binding(&c, &cs, &cs.bindings, 0, ty::BUTTON);
        let Draft::Binding(after) =
            edit(&c, &cs, 0, &Draft::Binding(b), "noun", f64::from(noun::AUX)).unwrap()
        else {
            panic!();
        };
        assert_eq!(after.target, 5);
    }

    #[test]
    fn cg20_a_remote_button_keeps_its_learned_code() {
        let (cs, st) = fixture();
        let c = cx(&st);
        let cmd = IrCommand {
            protocol: 1,
            code: 0x1234_5678,
            ..new_ir(&cs)
        };
        let n = valid_nouns(&cs, ty::IR)
            .into_iter()
            .find(|n| *n != cmd.noun)
            .unwrap();
        let Draft::Ir(after) = edit(&c, &cs, 0, &Draft::Ir(cmd), "noun", f64::from(n)).unwrap()
        else {
            panic!();
        };
        assert_eq!((after.protocol, after.code), (1, 0x1234_5678));
        assert_eq!(after.noun, n);
    }

    #[test]
    fn cg20_rows_serialise_with_their_kind_beside_their_id() {
        let (cs, st) = fixture();
        let c = cx(&st);
        let b = make_binding(&c, &cs, &cs.bindings, 0, ty::BUTTON);
        let d = describe(&c, &cs, 0, &Draft::Binding(b));
        let v = serde_json::to_value(&d).unwrap();
        let noun = v["rows"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["id"] == "noun")
            .unwrap();
        assert_eq!(noun["kind"], "pick");
        assert!(noun["options"][0]["section"].is_string());
    }
}
