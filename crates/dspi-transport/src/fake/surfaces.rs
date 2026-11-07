//! Control surfaces on a simulated device (WP CG16): the record store, the
//! refusal rules and the demo's own configuration.
//!
//! On with [`FakeOptions::surfaces`](super::FakeOptions::surfaces), which
//! [`FakeDevice::demo`](super::FakeDevice::demo) sets, so every client's demo has
//! a device to edit. Off, the `REQ_*_CS_*` requests stall as on firmware without
//! the feature, and a host sees no surfaces at all.
//!
//! The rules are `control_surfaces.c`'s, in its order, and the replies are
//! `vendor_commands.c`'s: a SET is acknowledged whatever becomes of it and the
//! verdict is in `REQ_GET_CS_STATUS`, tagged with the slot; `REQ_CS_SAVE`,
//! `REQ_CS_REVERT`, `REQ_CS_MACRO_FIRE` and `REQ_CS_IR_LEARN` answer `1` and stall
//! a refusal; a read of a slot that does not exist stalls.
//!
//! Where it differs from the firmware:
//!
//! - **Nothing is deferred.** A SET is applied before the transfer returns, so a
//!   host never reads `CS_STATUS_PENDING`, and the single-deep handoff's `BUSY`
//!   does not arise on its own. [`EXT_FAKE_CS_SIM`](super::EXT_FAKE_CS_SIM) holds the device in the state
//!   that does refuse, a revert running. There a SET settles `BUSY` at once, where
//!   the firmware leaves it `PENDING` until the revert ends (main.c) and answers
//!   `BUSY` to a second one of its kind.
//! - **Pins.** A pin is checked against the other bindings and for being a GPIO
//!   at all, as an RP2350 has them. The audio and control-interface claims are not
//!   modelled (`ctrl_iface_check_pin`), as the rest of the fake accepts any pin. The
//!   soft-VCXO PWM slice is not refused; no bondable RP2350 GPIO reaches it.
//! - **Nothing runs.** A binding moves no parameter and a macro step dispatches
//!   nothing. A fired macro advances one step per `REQ_GET_CS_EXT_STATUS` read and
//!   ignores its delays, so a host's poll sees it run and end. A display's page
//!   moves on one per `REQ_GET_CS_DISPLAY_STATUS` read in a cycling mode, and
//!   `CS_DMODE_CYCLE_ALL` cycles the configured pages only. IR learning stays armed
//!   until [`FakeRemote::cs_learned`] or a cancel; there is no 10 s timeout.
//! - **Tables.** [`surface_seed`]'s noun table is caps v20's and fills in the
//!   nouns the demo shows. The others read as unavailable on this platform (no
//!   actions), so a binding on one is `CS_STATUS_INVALID_ACTION`. Every PEQ channel
//!   has [`LIVE_BANDS`] bands.
#![warn(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

use dspi_proto::generated::cs::{self as g};
use dspi_proto::generated::{notify as nfy, opcodes as op, status as st};
use dspi_proto::packets::{
    CsAuxStates, CsBinding, CsCapsHeader, CsDisplayCfg, CsDisplayCfgReply, CsDisplayPage,
    CsDisplayStatus, CsExtStatusPacket, CsGroup, CsMacro, CsMacroHeaderWire, CsMacroStep,
    CsNounDesc, CsStatusPacket, CsTypeDesc, GPIO_UNUSED, IrCommand, aux_extras, cs_unit,
    display_flags, page_flags,
};

use super::{FakeRemote, LIVE_BANDS, State};

const SUCCESS: u8 = st::PIN_CONFIG_SUCCESS as u8;
const INVALID_PIN: u8 = st::PIN_CONFIG_INVALID_PIN as u8;
const PIN_IN_USE: u8 = st::PIN_CONFIG_PIN_IN_USE as u8;
const INVALID_SLOT: u8 = g::CS_STATUS_INVALID_SLOT as u8;
const INVALID_TYPE: u8 = g::CS_STATUS_INVALID_TYPE as u8;
const INVALID_NOUN: u8 = g::CS_STATUS_INVALID_NOUN as u8;
const INVALID_ACTION: u8 = g::CS_STATUS_INVALID_ACTION as u8;
const INVALID_VALUE: u8 = g::CS_STATUS_INVALID_VALUE as u8;
const PIN_NOT_ADC: u8 = g::CS_STATUS_PIN_NOT_ADC as u8;
const INVALID_TARGET: u8 = g::CS_STATUS_INVALID_TARGET as u8;
const INVALID_EVENT: u8 = g::CS_STATUS_INVALID_EVENT as u8;
const PWM_CONFLICT: u8 = g::CS_STATUS_PWM_CONFLICT as u8;
const EVENT_IN_USE: u8 = g::CS_STATUS_EVENT_IN_USE as u8;
const BUSY: u8 = g::CS_STATUS_BUSY as u8;
const IR_IN_USE: u8 = g::CS_STATUS_IR_IN_USE as u8;
const NO_IR: u8 = g::CS_STATUS_NO_IR as u8;
const INVALID_GROUP: u8 = g::CS_STATUS_INVALID_GROUP as u8;
const INVALID_MACRO: u8 = g::CS_STATUS_INVALID_MACRO as u8;
const INVALID_STEP: u8 = g::CS_STATUS_INVALID_STEP as u8;
const DISPLAY_IN_USE: u8 = g::CS_STATUS_DISPLAY_IN_USE as u8;
const PIN_NOT_I2C: u8 = g::CS_STATUS_PIN_NOT_I2C as u8;
const I2C_IN_USE: u8 = g::CS_STATUS_I2C_IN_USE as u8;
const INVALID_PAGE: u8 = g::CS_STATUS_INVALID_PAGE as u8;
const INVALID_AUX: u8 = g::CS_STATUS_INVALID_AUX as u8;

/// `cs_last_slot`'s tags (vendor_commands.c).
const TAG_GROUP: u8 = 0x40;
const TAG_DISPLAY: u8 = 0x50;
const TAG_MACRO: u8 = 0x60;
const TAG_IR: u8 = 0x80;
const TAG_SAVE: u8 = 0xFF;

/// `CS_NAME_LEN`.
const NAME_LEN: usize = g::CS_NAME_LEN as usize;
/// `s_macro_run` when the sequencer is idle.
const MACRO_NONE: u8 = 0xFF;

const TY_NONE: u8 = g::CS_TYPE_NONE as u8;
const TY_BUTTON: u8 = g::CS_TYPE_BUTTON as u8;
const TY_POT: u8 = g::CS_TYPE_POT as u8;
const TY_ENCODER: u8 = g::CS_TYPE_ENCODER as u8;
const TY_LED: u8 = g::CS_TYPE_LED as u8;
const TY_LED_PWM: u8 = g::CS_TYPE_LED_PWM as u8;
const TY_IR: u8 = g::CS_TYPE_IR as u8;
const TY_DISPLAY: u8 = g::CS_TYPE_DISPLAY as u8;
const TY_AUX_OUT: u8 = g::CS_TYPE_AUX_OUT as u8;
const TY_AUX_PWM: u8 = g::CS_TYPE_AUX_PWM as u8;

/// `CsAction` (control_surfaces.h), which is not generated.
const ACT_ADJUST: u8 = 0;
const ACT_STEP: u8 = 1;
const ACT_INC: u8 = 2;
const ACT_DEC: u8 = 3;
const ACT_TOGGLE: u8 = 4;
const ACT_SET: u8 = 5;
const ACT_FOLLOW: u8 = 6;
const ACT_TRIGGER: u8 = 7;
const ACT_IND_EQUALS: u8 = 8;
const ACT_MOMENTARY: u8 = 9;
const ACT_IND_ABOVE: u8 = 10;
const ACT_IND_LEVEL: u8 = 11;
const ACT_COUNT: u8 = 12;
/// `CS_EVT_COUNT`, and `CS_EVT_PRESS`.
const EVT_COUNT: u8 = 3;
const EVT_PRESS: u8 = 0;

const FLAG_INVERT: u8 = g::CS_FLAG_INVERT as u8;
const FLAG_WRAP: u8 = g::CS_FLAG_WRAP as u8;
const FLAG_ACCEL: u8 = g::CS_FLAG_ACCEL as u8;
const FLAG_REPEAT: u8 = g::CS_FLAG_REPEAT as u8;
const FLAG_GROUP: u8 = g::CS_FLAG_GROUP as u8;
const FLAG_LINK_ABS: u8 = g::CS_FLAG_LINK_ABS as u8;
const FLAG_GROUP_ALL: u8 = g::CS_FLAG_GROUP_ALL as u8;

const KIND_CONTINUOUS: u8 = g::CS_KIND_CONTINUOUS as u8;
const KIND_BOOL: u8 = g::CS_KIND_BOOL as u8;
const KIND_ENUM: u8 = g::CS_KIND_ENUM as u8;

const T_NONE: u8 = g::CS_TARGET_NONE as u8;
const T_INPUT: u8 = g::CS_TARGET_INPUT_CH as u8;
const T_OUTPUT: u8 = g::CS_TARGET_OUTPUT_CH as u8;
const T_DSP: u8 = g::CS_TARGET_DSP_CH as u8;
const T_BAND: u8 = g::CS_TARGET_DSP_BAND as u8;
const T_AUX: u8 = g::CS_TARGET_AUX as u8;

const N_AUX: u8 = g::CS_NOUN_AUX as u8;
const N_AUX_LEVEL: u8 = g::CS_NOUN_AUX_LEVEL as u8;
const N_MACRO: u8 = g::CS_NOUN_MACRO as u8;
const N_PAGE_VALUE: u8 = g::CS_NOUN_PAGE_VALUE as u8;
const N_DISPLAY_PAGE: u8 = g::CS_NOUN_DISPLAY_PAGE as u8;
const N_DISPLAY_EDIT: u8 = g::CS_NOUN_DISPLAY_EDIT as u8;
const N_FILTER_FREQ: u8 = g::CS_NOUN_FILTER_FREQ as u8;
const N_FILTER_BYPASS: u8 = g::CS_NOUN_FILTER_BYPASS as u8;

/// `CS_ADC_PIN_FIRST`, `_LAST` (control_surfaces.c).
const ADC_PINS: std::ops::RangeInclusive<u8> = 26..=28;
/// `XOVER_BAND_BASE`, `MAX_XOVER_BANDS` (config.h), and the first output channel,
/// on which the crossover bands exist.
const XOVER_BANDS: std::ops::Range<u8> = 20..24;
const FIRST_OUTPUT_CH: u8 = 8;
/// `CS_DMODE_*`.
const DMODE_FIXED: u8 = g::CS_DMODE_FIXED as u8;
const DMODE_CYCLE_ALL: u8 = g::CS_DMODE_CYCLE_ALL as u8;
const DALIGN_RIGHT: u8 = g::CS_DALIGN_RIGHT as u8;
/// `init_state`: a display is up.
const DISPLAY_LIVE: u8 = 2;

const fn bit(action: u8) -> u16 {
    1u16 << action
}

fn is_aux(ty: u8) -> bool {
    ty == TY_AUX_OUT || ty == TY_AUX_PWM
}

fn is_pwm(ty: u8) -> bool {
    ty == TY_LED_PWM || ty == TY_AUX_PWM
}

/// `is_valid_gpio_pin`, on an RP2350 (vendor_commands.c:372).
fn valid_gpio(pin: u8) -> bool {
    pin <= 29 && !(23..=25).contains(&pin)
}

/// What `pwm_gpio_to_slice_num` and `pwm_gpio_to_channel` say together: the
/// GPIOs below 32 repeat every 16.
fn pwm_output(pin: u8) -> u8 {
    pin & 0x0F
}

fn all_zero(b: &[u8]) -> bool {
    b.iter().all(|x| *x == 0)
}

/// The demo device's configuration and capability tables, as a device reports
/// them. `dspi_session::cs_model::demo` shows the same one, so a fixture and the
/// device it stands for cannot drift.
#[derive(Debug, Clone)]
pub struct SurfaceSeed {
    /// The capability header and its type table.
    pub header: CsCapsHeader,
    /// The noun table, by noun.
    pub nouns: Vec<CsNounDesc>,
    /// The bindings, by slot.
    pub bindings: Vec<CsBinding>,
    /// The binding names, by slot.
    pub names: Vec<String>,
    /// The learned IR commands, by sub-slot.
    pub ir: Vec<IrCommand>,
    /// The channel groups.
    pub groups: Vec<CsGroup>,
    /// The macros, each with every step.
    pub macros: Vec<CsMacro>,
    /// The display configuration.
    pub display_cfg: CsDisplayCfg,
    /// `CS_MAX_DISPLAY_PAGES`.
    pub max_pages: u8,
    /// `CS_DISP_MODEL_COUNT`.
    pub model_count: u8,
    /// The display pages.
    pub pages: Vec<CsDisplayPage>,
}

/// Store `value` at `i`, which a table of this module's own size has.
fn put<T>(v: &mut [T], i: usize, value: T) {
    if let Some(x) = v.get_mut(i) {
        *x = value;
    }
}

/// A noun table shaped like the firmware's: available nouns with their kinds,
/// units, ranges and target spaces, and one deliberately absent.
fn seed_nouns() -> Vec<CsNounDesc> {
    let cont = bit(ACT_ADJUST) | bit(ACT_STEP) | bit(ACT_INC) | bit(ACT_DEC);
    let ind = bit(ACT_IND_ABOVE) | bit(ACT_IND_LEVEL);
    let boolean = bit(ACT_TOGGLE) | bit(ACT_SET) | bit(ACT_FOLLOW) | bit(ACT_IND_EQUALS);
    let enums = bit(ACT_STEP) | bit(ACT_INC) | bit(ACT_DEC) | bit(ACT_SET);
    // The firmware's CS_CONT_RW, CS_BOOL_RW and CS_ENUM_RW masks.
    let cont_rw = cont | bit(ACT_SET) | ind;
    let bool_rw = boolean | bit(ACT_MOMENTARY);
    let enum_rw = enums | bit(ACT_IND_EQUALS);
    let continuous = |actions, min_q, max_q, unit, target_kind, target_count| CsNounDesc {
        kind: KIND_CONTINUOUS,
        actions,
        min_q,
        max_q,
        unit,
        target_kind,
        target_count,
        ..Default::default()
    };
    let flag = |actions, target_kind, target_count| CsNounDesc {
        kind: KIND_BOOL,
        actions,
        target_kind,
        target_count,
        ..Default::default()
    };
    let choice = |enum_count: u8, max_q, actions, target_kind, target_count| CsNounDesc {
        kind: KIND_ENUM,
        enum_count,
        actions,
        max_q,
        target_kind,
        target_count,
        ..Default::default()
    };
    let db = cs_unit::DB;
    let pct = cs_unit::PERCENT;
    let table: [(u16, CsNounDesc); 26] = [
        (
            g::CS_NOUN_USER_VOLUME,
            continuous(cont | ind, -15360, 0, db, T_NONE, 0),
        ),
        (
            g::CS_NOUN_MASTER_VOLUME,
            continuous(cont | ind, -20480, 0, db, T_NONE, 0),
        ),
        (g::CS_NOUN_USER_MUTE, flag(boolean, T_NONE, 0)),
        (
            g::CS_NOUN_PRESET,
            choice(10, 9, enums | bit(ACT_IND_EQUALS), T_NONE, 0),
        ),
        (g::CS_NOUN_INPUT_SOURCE, choice(4, 3, enums, T_NONE, 0)),
        (
            g::CS_NOUN_CLIP,
            flag(bit(ACT_TRIGGER) | bit(ACT_IND_EQUALS), T_NONE, 0),
        ),
        (
            g::CS_NOUN_PREAMP,
            continuous(cont, -10240, 5120, db, T_INPUT, 8),
        ),
        (
            g::CS_NOUN_OUTPUT_GAIN,
            continuous(cont | ind, -15360, 5120, db, T_OUTPUT, 9),
        ),
        (g::CS_NOUN_OUTPUT_MUTE, flag(boolean, T_OUTPUT, 9)),
        (
            g::CS_NOUN_FILTER_FREQ,
            continuous(cont, 20, 20000, cs_unit::HZ, T_BAND, 17),
        ),
        (g::CS_NOUN_LEVEL, continuous(ind, -15360, 0, db, T_DSP, 17)),
        (
            g::CS_NOUN_MACRO,
            choice(8, 7, bit(ACT_SET) | bit(ACT_IND_EQUALS), T_NONE, 0),
        ),
        (
            g::CS_NOUN_CPU_LOAD,
            continuous(ind, 0, 25600, pct, T_NONE, 0),
        ),
        (
            g::CS_NOUN_DISPLAY_PAGE,
            choice(16, 15, enums | bit(ACT_IND_EQUALS), T_NONE, 0),
        ),
        (g::CS_NOUN_DISPLAY_EDIT, flag(boolean, T_NONE, 0)),
        (
            g::CS_NOUN_PAGE_VALUE,
            CsNounDesc {
                kind: KIND_ENUM,
                enum_count: 1,
                actions: bit(ACT_STEP) | bit(ACT_INC) | bit(ACT_DEC) | bit(ACT_TOGGLE),
                ..Default::default()
            },
        ),
        // Caps v14 to v20, as control_surfaces_nouns.c publishes them: the
        // subharm, tube and limiter families, the two aux nouns addressing
        // binding slots, and the limiter's read-only gain reduction.
        (g::CS_NOUN_SUBHARM, flag(bool_rw, T_NONE, 0)),
        (g::CS_NOUN_SUBHARM_SELECT, choice(3, 0, enum_rw, T_NONE, 0)),
        (g::CS_NOUN_TUBE_TYPE, choice(17, 0, enum_rw, T_NONE, 0)),
        (g::CS_NOUN_AUX, flag(bool_rw, T_AUX, 16)),
        (
            g::CS_NOUN_AUX_LEVEL,
            continuous(cont_rw, 0, 25600, pct, T_AUX, 16),
        ),
        (g::CS_NOUN_LIMITER, flag(bool_rw, T_OUTPUT, 9)),
        (
            g::CS_NOUN_LIMITER_THRESHOLD,
            continuous(cont_rw, -7680, 0, db, T_OUTPUT, 9),
        ),
        (
            g::CS_NOUN_LIMITER_RELEASE,
            continuous(cont_rw, 10, 1000, cs_unit::MS_LOG, T_OUTPUT, 9),
        ),
        (g::CS_NOUN_LIMITER_LINK, choice(5, 0, enum_rw, T_OUTPUT, 9)),
        (
            g::CS_NOUN_LIMITER_GR,
            continuous(ind, 0, 7680, db, T_OUTPUT, 9),
        ),
    ];
    let mut v = vec![CsNounDesc::default(); 79];
    for (noun, desc) in table {
        put(&mut v, usize::from(noun), desc);
    }
    // ADAT_ACTIVE is left with a zero action mask: the firmware's way of saying
    // the platform does not have it.
    v
}

/// The demo's configuration: an encoder, a button, a pot, an IR receiver with a
/// learned key, a display with two pages, two aux outputs and a button on one of
/// them, a group and a macro.
pub fn surface_seed() -> SurfaceSeed {
    let button = bit(ACT_INC)
        | bit(ACT_DEC)
        | bit(ACT_TOGGLE)
        | bit(ACT_SET)
        | bit(ACT_TRIGGER)
        | bit(ACT_MOMENTARY);
    let t = |actions: u16, pin_count: u8, pin_class: u8| CsTypeDesc {
        actions,
        pin_count,
        pin_class,
    };
    let header = CsCapsHeader {
        caps_version: 20,
        max_bindings: 16,
        type_count: 11,
        noun_count: 79,
        // The v1.1.6 type table (control_surfaces.h), so every picker is exercised.
        types: vec![
            t(0, 0, 0),
            t(button, 1, 0),
            t(bit(ACT_FOLLOW), 1, 0),
            t(bit(ACT_ADJUST), 1, CsTypeDesc::PINCLASS_ADC),
            t(bit(ACT_STEP), 2, 0),
            t(bit(ACT_IND_EQUALS) | bit(ACT_IND_ABOVE), 1, 0),
            t(
                bit(ACT_IND_EQUALS) | bit(ACT_IND_ABOVE) | bit(ACT_IND_LEVEL),
                1,
                0,
            ),
            t(button, 1, 0),
            t(0, 2, 0),
            // The two aux containers drive nothing themselves.
            t(0, 1, 0),
            t(0, 1, 0),
        ],
        max_ir_commands: 16,
        max_groups: 8,
        max_macros: 8,
        max_macro_steps: 8,
    };
    let mut bindings = vec![CsBinding::default(); 16];
    put(
        &mut bindings,
        0,
        CsBinding {
            component: TY_ENCODER,
            noun: g::CS_NOUN_USER_VOLUME as u8,
            action: ACT_STEP,
            flags: FLAG_ACCEL,
            gpio: [10, 11],
            ..Default::default()
        },
    );
    put(
        &mut bindings,
        1,
        CsBinding {
            component: TY_BUTTON,
            noun: g::CS_NOUN_USER_MUTE as u8,
            action: ACT_TOGGLE,
            gpio: [16, GPIO_UNUSED],
            value: 1,
            ..Default::default()
        },
    );
    put(
        &mut bindings,
        2,
        CsBinding {
            component: TY_POT,
            noun: g::CS_NOUN_OUTPUT_GAIN as u8,
            action: ACT_ADJUST,
            gpio: [26, GPIO_UNUSED],
            target: 1,
            ..Default::default()
        },
    );
    put(
        &mut bindings,
        3,
        CsBinding {
            component: TY_IR,
            gpio: [15, GPIO_UNUSED],
            ..Default::default()
        },
    );
    put(
        &mut bindings,
        4,
        CsBinding {
            component: TY_DISPLAY,
            gpio: [2, 3],
            index: g::CS_DISP_MODEL_SSD1306_128X64 as u8,
            ..Default::default()
        },
    );
    // Two auxiliary outputs and a button that switches the first. They sit high
    // so the first free slot stays where it was.
    put(
        &mut bindings,
        10,
        CsBinding {
            component: TY_AUX_OUT,
            gpio: [27, GPIO_UNUSED],
            // 60 s, in 0.1 s units.
            off_delay: 600,
            ..Default::default()
        },
    );
    put(
        &mut bindings,
        11,
        CsBinding {
            component: TY_AUX_PWM,
            gpio: [28, GPIO_UNUSED],
            base_bright: 80,
            // 50 %, 8.8.
            value: 12800,
            extras: aux_extras::BOOT_ON,
            ..Default::default()
        },
    );
    put(
        &mut bindings,
        12,
        CsBinding {
            component: TY_BUTTON,
            noun: N_AUX,
            action: ACT_TOGGLE,
            gpio: [19, GPIO_UNUSED],
            target: 10,
            value: 1,
            ..Default::default()
        },
    );
    let mut names = vec![String::new(); 16];
    put(&mut names, 0, "Volume Knob".into());
    put(&mut names, 10, "Amp Trigger".into());
    let mut ir = vec![IrCommand::default(); 16];
    put(
        &mut ir,
        0,
        IrCommand {
            noun: g::CS_NOUN_USER_VOLUME as u8,
            action: ACT_INC,
            protocol: g::CS_IR_PROTO_NEC as u8,
            code: 0x20DF_40BF,
            ..Default::default()
        },
    );
    let mut groups = vec![CsGroup::default(); 8];
    put(
        &mut groups,
        0,
        CsGroup {
            target_kind: T_OUTPUT,
            member_mask: 0b0011,
            name: "Front Pair".into(),
        },
    );
    let mut macros = vec![CsMacro::default(); 8];
    let mut steps = vec![CsMacroStep::default(); 8];
    put(
        &mut steps,
        0,
        CsMacroStep {
            noun: g::CS_NOUN_INPUT_SOURCE as u8,
            action: ACT_SET,
            value: 1,
            ..Default::default()
        },
    );
    put(
        &mut steps,
        1,
        CsMacroStep {
            noun: g::CS_NOUN_PRESET as u8,
            action: ACT_SET,
            value: 2,
            pre_delay: 50,
            ..Default::default()
        },
    );
    put(
        &mut macros,
        0,
        CsMacro {
            name: "Night".into(),
            step_count: 2,
            steps,
        },
    );
    let mut pages = vec![CsDisplayPage::default(); 16];
    put(
        &mut pages,
        0,
        CsDisplayPage {
            noun: g::CS_NOUN_USER_VOLUME as u8,
            target: 0,
            index: 0,
            flags: page_flags::ACTIVE | page_flags::LARGE,
        },
    );
    put(
        &mut pages,
        1,
        CsDisplayPage {
            noun: g::CS_NOUN_PRESET as u8,
            target: 0,
            index: 0,
            flags: page_flags::ACTIVE,
        },
    );
    SurfaceSeed {
        header,
        nouns: seed_nouns(),
        bindings,
        names,
        ir,
        groups,
        macros,
        display_cfg: CsDisplayCfg {
            mode: g::CS_DMODE_CYCLE_SELECTED as u8,
            home_page: 0,
            dwell: 30,
            overlay_hold: 20,
            brightness: 0,
            flags: display_flags::EDIT_GATED,
            edit_timeout: 100,
        },
        max_pages: 16,
        model_count: g::CS_DISP_MODEL_COUNT as u8,
        pages,
    }
}

/// What `REQ_CS_SAVE` stores and `REQ_CS_REVERT` brings back.
#[derive(Debug, Clone)]
struct Records {
    bindings: Vec<CsBinding>,
    names: Vec<String>,
    ir: Vec<IrCommand>,
    groups: Vec<CsGroup>,
    macros: Vec<CsMacro>,
    display: CsDisplayCfg,
    pages: Vec<CsDisplayPage>,
}

/// The surfaces of one simulated device.
#[derive(Debug, Clone)]
pub(super) struct Surfaces {
    header: CsCapsHeader,
    nouns: Vec<CsNounDesc>,
    max_pages: u8,
    model_count: u8,
    live: Records,
    stored: Records,
    /// `s_rt[slot].active`, as a mask.
    active: u16,
    slot_status: Vec<u8>,
    ir_status: Vec<u8>,
    group_status: Vec<u8>,
    macro_status: Vec<u8>,
    aux: CsAuxStates,
    last_status: u8,
    last_slot: u8,
    dirty: bool,
    learn_state: u8,
    learn_protocol: u8,
    learn_code: u32,
    macro_run: u8,
    macro_step: u8,
    shown_page: u8,
    /// A running revert (`control_surfaces_busy`): [`EXT_FAKE_CS_SIM`](super::EXT_FAKE_CS_SIM).
    busy: bool,
    /// The live control-interface I2C instance, set before each request: a
    /// display checks it on a SET and on a revert.
    i2c_instance: Option<u8>,
}

impl Surfaces {
    pub(super) fn new() -> Self {
        let mut seed = surface_seed();
        // Storage is whole on the device, however few steps a record names.
        for m in &mut seed.macros {
            m.steps.resize(
                usize::from(seed.header.max_macro_steps),
                CsMacroStep::default(),
            );
        }
        let live = Records {
            bindings: seed.bindings,
            names: seed.names,
            ir: seed.ir,
            groups: seed.groups,
            macros: seed.macros,
            display: seed.display_cfg,
            pages: seed.pages,
        };
        let mut s = Self {
            stored: live.clone(),
            live,
            nouns: seed.nouns,
            max_pages: seed.max_pages,
            model_count: seed.model_count,
            active: 0,
            slot_status: vec![SUCCESS; usize::from(seed.header.max_bindings)],
            ir_status: vec![SUCCESS; usize::from(seed.header.max_ir_commands)],
            group_status: vec![SUCCESS; usize::from(seed.header.max_groups)],
            macro_status: vec![SUCCESS; usize::from(seed.header.max_macros)],
            header: seed.header,
            aux: CsAuxStates::default(),
            last_status: SUCCESS,
            last_slot: 0,
            dirty: false,
            learn_state: g::CS_IR_LEARN_IDLE as u8,
            learn_protocol: 0,
            learn_code: 0,
            macro_run: MACRO_NONE,
            macro_step: 0,
            shown_page: 0,
            busy: false,
            i2c_instance: None,
        };
        s.reload();
        for slot in 0..s.header.max_bindings {
            s.seed_aux(slot);
        }
        s
    }

    /// `cs_display_live_instance`: the I2C block a live display holds.
    pub(super) fn display_instance(&self) -> Option<u8> {
        self.slots()
            .find(|(s, b)| b.component == TY_DISPLAY && self.is_active(*s))
            .map(|(_, b)| (b.gpio[0] >> 1) & 1)
    }

    pub(super) fn set_busy(&mut self, on: bool) {
        self.busy = on;
    }

    fn is_active(&self, slot: u8) -> bool {
        u32::from(self.active) & 1u32.checked_shl(u32::from(slot)).unwrap_or(0) != 0
    }

    fn set_active(&mut self, slot: u8, on: bool) {
        let m = 1u16.checked_shl(u32::from(slot)).unwrap_or(0);
        if on {
            self.active |= m;
        } else {
            self.active &= !m;
        }
    }

    fn pin_count(&self, ty: u8) -> usize {
        self.header
            .types
            .get(usize::from(ty))
            .map_or(0, |t| usize::from(t.pin_count))
    }

    /// The binding's pins, as many as its type has.
    fn pins<'b>(&self, b: &'b CsBinding) -> &'b [u8] {
        b.gpio.get(..self.pin_count(b.component)).unwrap_or(&[])
    }

    fn slots(&self) -> impl Iterator<Item = (u8, &CsBinding)> {
        (0u8..).zip(self.live.bindings.iter())
    }

    /// Other live bindings: this slot's own pins are released before a check.
    fn others(&self, slot: u8) -> impl Iterator<Item = (u8, &CsBinding)> {
        self.slots()
            .filter(move |(s, _)| *s != slot && self.is_active(*s))
    }

    fn slot_type(&self, slot: u8) -> u8 {
        self.live
            .bindings
            .get(usize::from(slot))
            .map_or(TY_NONE, |b| b.component)
    }

    // ----------------------------------------------------------- validation

    /// `cs_noun_validate_target_ch`.
    fn check_target_ch(&self, noun: u8, ch: u8, index: u8) -> u8 {
        let Some(nd) = self.nouns.get(usize::from(noun)) else {
            return INVALID_TARGET;
        };
        match nd.target_kind {
            T_INPUT | T_OUTPUT | T_DSP => {
                if ch >= nd.target_count || index != 0 {
                    INVALID_TARGET
                } else {
                    SUCCESS
                }
            }
            T_AUX => {
                if ch >= nd.target_count || index != 0 {
                    return INVALID_TARGET;
                }
                let t = self.slot_type(ch);
                let ok = if noun == N_AUX_LEVEL {
                    t == TY_AUX_PWM
                } else {
                    is_aux(t)
                };
                if ok { SUCCESS } else { INVALID_AUX }
            }
            T_BAND => {
                if ch >= nd.target_count {
                    return INVALID_TARGET;
                }
                let peq = usize::from(index) < LIVE_BANDS;
                let xover = XOVER_BANDS.contains(&index) && ch >= FIRST_OUTPUT_CH;
                let ok = if noun == N_FILTER_FREQ || noun == N_FILTER_BYPASS {
                    peq || xover
                } else {
                    peq
                };
                if ok { SUCCESS } else { INVALID_TARGET }
            }
            _ => INVALID_TARGET,
        }
    }

    /// `cs_noun_validate_target`.
    fn check_target(&self, noun: u8, target: u8, index: u8) -> u8 {
        let none = self
            .nouns
            .get(usize::from(noun))
            .is_none_or(|nd| nd.target_kind == T_NONE);
        if !none {
            return self.check_target_ch(noun, target, index);
        }
        if target != 0 || index != 0 {
            INVALID_TARGET
        } else {
            SUCCESS
        }
    }

    /// `cs_validate_group_ref`.
    fn check_group_ref(&self, noun: u8, target: u8, index: u8) -> u8 {
        let (Some(nd), Some(group)) = (
            self.nouns.get(usize::from(noun)),
            self.live.groups.get(usize::from(target)),
        ) else {
            return INVALID_GROUP;
        };
        if self.group_status.get(usize::from(target)) != Some(&SUCCESS) {
            return INVALID_GROUP;
        }
        let want = match nd.target_kind {
            T_INPUT => T_INPUT,
            T_OUTPUT => T_OUTPUT,
            T_DSP | T_BAND => T_DSP,
            _ => return INVALID_GROUP,
        };
        if group.target_kind != want {
            return INVALID_GROUP;
        }
        let lim = 1u32
            .checked_shl(u32::from(nd.target_count))
            .map_or(u32::MAX, |x| x.wrapping_sub(1));
        let members = group.member_mask & lim;
        if members == 0 {
            return INVALID_GROUP;
        }
        for ch in 0u8..32 {
            if members & 1u32.checked_shl(u32::from(ch)).unwrap_or(0) != 0 {
                let st = self.check_target_ch(noun, ch, index);
                if st != SUCCESS {
                    return st;
                }
            }
        }
        SUCCESS
    }

    /// `cs_validate_group_flags`.
    fn check_group_flags(flags: u8, action: u8, nd: &CsNounDesc) -> u8 {
        if flags & FLAG_GROUP == 0 {
            return if flags & (FLAG_LINK_ABS | FLAG_GROUP_ALL) != 0 {
                INVALID_VALUE
            } else {
                SUCCESS
            };
        }
        if flags & FLAG_LINK_ABS != 0 && (action != ACT_ADJUST || nd.kind != KIND_CONTINUOUS) {
            return INVALID_VALUE;
        }
        if flags & FLAG_GROUP_ALL != 0 && action != ACT_IND_EQUALS && action != ACT_IND_ABOVE {
            return INVALID_VALUE;
        }
        SUCCESS
    }

    /// `cs_validate_values`.
    fn check_values(
        nd: &CsNounDesc,
        action: u8,
        value: i16,
        step: i16,
        range: Option<(i16, i16)>,
    ) -> u8 {
        let bad = match nd.kind {
            KIND_CONTINUOUS => {
                let (lo, hi) = range.unwrap_or((0, 0));
                (matches!(action, ACT_SET | ACT_IND_ABOVE)
                    && (value < nd.min_q || value > nd.max_q))
                    || step < 0
                    || (matches!(action, ACT_ADJUST | ACT_IND_LEVEL)
                        && (lo != 0 || hi != 0)
                        && (lo >= hi || lo < nd.min_q || hi > nd.max_q))
            }
            KIND_BOOL => {
                matches!(action, ACT_SET | ACT_IND_EQUALS | ACT_MOMENTARY)
                    && !(0..=1).contains(&value)
            }
            KIND_ENUM => {
                matches!(action, ACT_SET | ACT_IND_EQUALS)
                    && (value < 0 || value >= i16::from(nd.enum_count))
            }
            _ => false,
        };
        if bad { INVALID_VALUE } else { SUCCESS }
    }

    /// `cs_validate_ir_container`.
    fn check_ir_container(&self, b: &CsBinding, slot: u8) -> u8 {
        if b.noun != 0
            || b.action != 0
            || b.event != 0
            || b.target != 0
            || b.index != 0
            || b.value != 0
            || b.step != 0
            || b.range_min != 0
            || b.range_max != 0
            || b.flags & !FLAG_INVERT != 0
        {
            return INVALID_VALUE;
        }
        if self.others(slot).any(|(_, o)| o.component == TY_IR) {
            return IR_IN_USE;
        }
        SUCCESS
    }

    /// `cs_validate_display_container`.
    fn check_display_container(&self, b: &CsBinding, slot: u8) -> u8 {
        if b.noun != 0
            || b.action != 0
            || b.event != 0
            || b.target != 0
            || b.step != 0
            || b.range_min != 0
            || b.range_max != 0
            || b.flags != 0
            || b.index == g::CS_DISP_MODEL_NONE as u8
            || b.index >= self.model_count
            || (b.value != 0 && !(0x08..=0x77).contains(&b.value))
        {
            return INVALID_VALUE;
        }
        let [sda, scl] = b.gpio;
        if sda & 1 != 0 || scl & 1 != 1 || (sda >> 1) & 1 != (scl >> 1) & 1 {
            return PIN_NOT_I2C;
        }
        if self.i2c_instance == Some((sda >> 1) & 1) {
            return I2C_IN_USE;
        }
        if self.others(slot).any(|(_, o)| o.component == TY_DISPLAY) {
            return DISPLAY_IN_USE;
        }
        SUCCESS
    }

    /// `cs_validate_aux_container`.
    fn check_aux_container(&self, b: &CsBinding) -> u8 {
        if b.noun != 0
            || b.action != 0
            || b.event != 0
            || b.target != 0
            || b.index != 0
            || b.step != 0
            || b.range_min != 0
            || b.range_max != 0
            || b.flags & !FLAG_INVERT != 0
            || b.extras & !(g::CS_AUX_X_MASK as u8) != 0
        {
            return INVALID_VALUE;
        }
        let bad = if b.component == TY_AUX_OUT {
            b.value != 0 || b.base_bright != 0 || b.extras & aux_extras::LINEAR != 0
        } else {
            let max = self
                .nouns
                .get(usize::from(N_AUX_LEVEL))
                .map_or(0, |n| n.max_q);
            b.value < 0 || b.value > max
        };
        if bad { INVALID_VALUE } else { SUCCESS }
    }

    /// `cs_check_button_share`.
    fn check_button_share(&self, nb: &CsBinding, slot: u8) -> u8 {
        for (_, b) in self.others(slot) {
            if !self.pins(b).contains(&nb.gpio[0]) {
                continue;
            }
            if b.component != TY_BUTTON {
                return PIN_IN_USE;
            }
            if (b.flags ^ nb.flags) & FLAG_INVERT != 0 {
                return INVALID_VALUE;
            }
            if b.event == nb.event {
                return EVENT_IN_USE;
            }
        }
        SUCCESS
    }

    /// The half of `cs_validate` a control has and a container does not.
    fn check_control(&self, b: &CsBinding, td: &CsTypeDesc, nd: &CsNounDesc, slot: u8) -> u8 {
        let action = bit(b.action);
        if td.actions & action == 0 || nd.actions & action == 0 {
            return INVALID_ACTION;
        }
        if b.component == TY_BUTTON {
            if b.event >= EVT_COUNT
                || ((b.action == ACT_MOMENTARY || b.flags & FLAG_REPEAT != 0)
                    && b.event != EVT_PRESS)
            {
                return INVALID_EVENT;
            }
        } else if b.event != 0 {
            return INVALID_EVENT;
        }
        if b.flags & FLAG_REPEAT != 0
            && (b.component != TY_BUTTON || !matches!(b.action, ACT_INC | ACT_DEC))
        {
            return INVALID_VALUE;
        }
        if b.flags & FLAG_ACCEL != 0 && b.component != TY_ENCODER {
            return INVALID_VALUE;
        }
        let grouped = b.flags & FLAG_GROUP != 0;
        let st = if grouped {
            self.check_group_ref(b.noun, b.target, b.index)
        } else {
            self.check_target(b.noun, b.target, b.index)
        };
        if st != SUCCESS {
            return st;
        }
        if nd.target_kind == T_AUX && !grouped && b.target == slot {
            return INVALID_TARGET;
        }
        let st = Self::check_values(
            nd,
            b.action,
            b.value,
            b.step,
            Some((b.range_min, b.range_max)),
        );
        if st != SUCCESS {
            return st;
        }
        if b.noun == N_PAGE_VALUE
            && (b.value != 0 || b.step != 0 || b.range_min != 0 || b.range_max != 0)
        {
            return INVALID_VALUE;
        }
        SUCCESS
    }

    /// `cs_validate`: why this binding cannot go in `slot`.
    fn check_binding(&self, b: &CsBinding, reserved: u8, slot: u8) -> u8 {
        let Some(td) = self.header.types.get(usize::from(b.component)) else {
            return INVALID_TYPE;
        };
        let Some(nd) = self.nouns.get(usize::from(b.noun)) else {
            return INVALID_NOUN;
        };
        if b.action >= ACT_COUNT {
            return INVALID_ACTION;
        }
        let st = Self::check_group_flags(b.flags, b.action, nd);
        if st != SUCCESS {
            return st;
        }
        let ty = b.component;
        let aux = is_aux(ty);
        let delays_allowed = aux
            || (matches!(ty, TY_LED | TY_LED_PWM)
                && matches!(b.action, ACT_IND_EQUALS | ACT_IND_ABOVE));
        if reserved != 0
            || (b.extras != 0 && !aux)
            || b.base_bright > 100
            || (b.base_bright != 0 && !is_pwm(ty))
            || ((b.on_delay != 0 || b.off_delay != 0) && !delays_allowed)
        {
            return INVALID_VALUE;
        }
        let st = match ty {
            TY_IR => self.check_ir_container(b, slot),
            TY_DISPLAY => self.check_display_container(b, slot),
            _ if aux => self.check_aux_container(b),
            _ => self.check_control(b, td, nd, slot),
        };
        if st != SUCCESS {
            return st;
        }
        self.check_pins(b, td, slot)
    }

    /// The pin half of `cs_validate`: distinct, valid, unclaimed; the one overlap
    /// allowed is button bindings on one GPIO with different events.
    fn check_pins(&self, b: &CsBinding, td: &CsTypeDesc, slot: u8) -> u8 {
        if td.pin_count == 2 && b.gpio[0] == b.gpio[1] {
            return INVALID_PIN;
        }
        for pin in self.pins(b) {
            if td.pin_class == CsTypeDesc::PINCLASS_ADC && !ADC_PINS.contains(pin) {
                return PIN_NOT_ADC;
            }
            if self.others(slot).any(|(_, o)| self.pins(o).contains(pin)) {
                if b.component != TY_BUTTON {
                    return PIN_IN_USE;
                }
                let st = self.check_button_share(b, slot);
                if st != SUCCESS {
                    return st;
                }
            } else if !valid_gpio(*pin) {
                return INVALID_PIN;
            }
        }
        if is_pwm(b.component) {
            let out = pwm_output(b.gpio[0]);
            let clash = self
                .others(slot)
                .any(|(_, o)| is_pwm(o.component) && pwm_output(o.gpio[0]) == out);
            if clash {
                return PWM_CONFLICT;
            }
        }
        SUCCESS
    }

    /// `cs_validate_ir_cmd`.
    fn check_ir(&self, c: &IrCommand, raw: &[u8]) -> u8 {
        let Some(nd) = self.nouns.get(usize::from(c.noun)) else {
            return INVALID_NOUN;
        };
        if c.action >= ACT_COUNT {
            return INVALID_ACTION;
        }
        if c.protocol >= g::CS_IR_PROTO_COUNT as u8
            || c.code == 0
            || !all_zero(raw.get(10..12).unwrap_or(&[]))
            || c.flags & !(FLAG_WRAP | FLAG_REPEAT | FLAG_GROUP) != 0
            || (c.flags & FLAG_REPEAT != 0 && !matches!(c.action, ACT_INC | ACT_DEC))
            || (c.noun == N_PAGE_VALUE && (c.value != 0 || c.step != 0))
        {
            return INVALID_VALUE;
        }
        let type_actions = self
            .header
            .types
            .get(usize::from(TY_IR))
            .map_or(0, |t| t.actions);
        if type_actions & bit(c.action) == 0 || nd.actions & bit(c.action) == 0 {
            return INVALID_ACTION;
        }
        let st = if c.flags & FLAG_GROUP != 0 {
            self.check_group_ref(c.noun, c.target, c.index)
        } else {
            self.check_target(c.noun, c.target, c.index)
        };
        if st != SUCCESS {
            return st;
        }
        Self::check_values(nd, c.action, c.value, c.step, None)
    }

    /// `cs_validate_group`.
    fn check_group(g: &CsGroup, raw: &[u8]) -> u8 {
        if g.target_kind == T_NONE {
            return if all_zero(raw) {
                SUCCESS
            } else {
                INVALID_VALUE
            };
        }
        let count: u32 = match g.target_kind {
            T_INPUT => 8,
            T_OUTPUT => 9,
            T_DSP => 17,
            _ => return INVALID_GROUP,
        };
        if !all_zero(raw.get(1..4).unwrap_or(&[])) {
            return INVALID_VALUE;
        }
        let lim = 1u32
            .checked_shl(count)
            .map_or(u32::MAX, |x| x.wrapping_sub(1));
        if g.member_mask == 0 || g.member_mask & !lim != 0 {
            return INVALID_GROUP;
        }
        SUCCESS
    }

    /// `cs_validate_macro_step`.
    fn check_step(&self, s: &CsMacroStep, reserved: u8) -> u8 {
        if s.is_empty() && reserved == 0 {
            return SUCCESS;
        }
        if s.noun == N_MACRO || s.noun == N_PAGE_VALUE {
            return INVALID_STEP;
        }
        let Some(nd) = self.nouns.get(usize::from(s.noun)) else {
            return INVALID_STEP;
        };
        let allowed =
            bit(ACT_SET) | bit(ACT_TOGGLE) | bit(ACT_INC) | bit(ACT_DEC) | bit(ACT_TRIGGER);
        if s.action >= ACT_COUNT
            || allowed & bit(s.action) == 0
            || s.flags & !(FLAG_WRAP | FLAG_GROUP) != 0
            || reserved != 0
        {
            return INVALID_STEP;
        }
        if nd.actions & bit(s.action) == 0 {
            return INVALID_ACTION;
        }
        let st = if s.flags & FLAG_GROUP != 0 {
            self.check_group_ref(s.noun, s.target, s.index)
        } else {
            self.check_target(s.noun, s.target, s.index)
        };
        if st != SUCCESS {
            return st;
        }
        Self::check_values(nd, s.action, s.value, s.step, None)
    }

    /// `disp_validate_page`.
    fn check_page(&self, p: &CsDisplayPage, raw: &[u8]) -> u8 {
        if p.flags & page_flags::ACTIVE == 0 {
            return if all_zero(raw) { SUCCESS } else { INVALID_PAGE };
        }
        let known = page_flags::ACTIVE | page_flags::GROUP | page_flags::LARGE | page_flags::BAR;
        if p.flags & !known != 0 {
            return INVALID_PAGE;
        }
        let Some(nd) = self.nouns.get(usize::from(p.noun)) else {
            return INVALID_NOUN;
        };
        if p.flags & page_flags::BAR != 0 && nd.kind != KIND_CONTINUOUS {
            return INVALID_PAGE;
        }
        if matches!(p.noun, N_DISPLAY_PAGE | N_DISPLAY_EDIT | N_PAGE_VALUE) {
            return INVALID_PAGE;
        }
        if nd.actions == 0 {
            return INVALID_NOUN;
        }
        if p.flags & page_flags::GROUP != 0 {
            return self.check_group_ref(p.noun, p.target, p.index);
        }
        self.check_target(p.noun, p.target, p.index)
    }

    /// `control_surfaces_apply_display_cfg`.
    fn check_display_cfg(&self, c: &CsDisplayCfg, raw: &[u8]) -> u8 {
        let known = display_flags::OVERLAY_ANY
            | display_flags::EDIT_GATED
            | display_flags::LABEL_ALIGN
            | display_flags::VALUE_ALIGN;
        if c.mode > DMODE_CYCLE_ALL {
            return INVALID_VALUE;
        }
        if c.home_page >= self.max_pages {
            return INVALID_PAGE;
        }
        let label = (c.flags & display_flags::LABEL_ALIGN) >> display_flags::LABEL_ALIGN_SHIFT;
        let value = (c.flags & display_flags::VALUE_ALIGN) >> display_flags::VALUE_ALIGN_SHIFT;
        if c.flags & !known != 0
            || label > DALIGN_RIGHT
            || value > DALIGN_RIGHT
            || !all_zero(raw.get(10..12).unwrap_or(&[]))
            || (c.mode != DMODE_FIXED && c.dwell < 10)
        {
            return INVALID_VALUE;
        }
        SUCCESS
    }

    // -------------------------------------------------------------- applies

    /// What an aux slot starts at: `cs_aux_initial_from`, without its carry.
    fn seed_aux(&mut self, slot: u8) {
        let i = usize::from(slot);
        let (on, level) = match self.live.bindings.get(i) {
            Some(b) if b.component == TY_AUX_PWM => {
                (b.extras & aux_extras::BOOT_ON != 0, b.value.max(0) as u16)
            }
            Some(b) if b.component == TY_AUX_OUT => (b.extras & aux_extras::BOOT_ON != 0, 0),
            _ => (false, 0),
        };
        put(&mut self.aux.state, i, u8::from(on));
        put(&mut self.aux.level_q8, i, level);
    }

    /// `cs_apply_binding_slot`, with `cs_revalidate_aux_dependents`.
    fn apply_binding(&mut self, slot: u8, b: CsBinding, reserved: u8) -> u8 {
        if self.busy {
            return BUSY;
        }
        let i = usize::from(slot);
        let old = self.slot_type(slot);
        if b.component != TY_NONE {
            let st = self.check_binding(&b, reserved, slot);
            if st != SUCCESS {
                return st;
            }
        }
        let ty = b.component;
        put(&mut self.live.bindings, i, b);
        self.set_active(slot, ty != TY_NONE);
        put(&mut self.slot_status, i, SUCCESS);
        // A live aux slot keeps its value across a same-type re-apply.
        if old != ty {
            self.seed_aux(slot);
        }
        if old != ty && (is_aux(old) || is_aux(ty)) {
            self.revalidate_aux(slot);
        }
        SUCCESS
    }

    /// One binding pointing at what was just edited: a live one goes down if its
    /// target stopped being valid, keeping its status otherwise only for an aux
    /// target; one that is down gets the full check, pins included, and comes
    /// back only if it passes (`cs_revalidate_grouped_slot`).
    fn recheck(&mut self, slot: u8, b: &CsBinding, target_status: u8, keep_status: bool) {
        let i = usize::from(slot);
        if self.is_active(slot) {
            if target_status != SUCCESS {
                self.set_active(slot, false);
                put(&mut self.slot_status, i, target_status);
            } else if !keep_status {
                put(&mut self.slot_status, i, SUCCESS);
            }
            return;
        }
        let st = self.check_binding(b, 0, slot);
        self.set_active(slot, st == SUCCESS);
        put(&mut self.slot_status, i, st);
    }

    /// After a group edit: only what names this group
    /// (`control_surfaces_apply_group`).
    fn revalidate_group(&mut self, group: u8) {
        for slot in 0..self.header.max_bindings {
            let Some(b) = self.live.bindings.get(usize::from(slot)).cloned() else {
                continue;
            };
            if matches!(b.component, TY_NONE | TY_IR)
                || b.flags & FLAG_GROUP == 0
                || b.target != group
            {
                continue;
            }
            let st = self.check_group_ref(b.noun, b.target, b.index);
            self.recheck(slot, &b, st, false);
        }
        self.recheck_ir(|c| c.flags & FLAG_GROUP != 0 && c.target == group);
        self.recount_macros();
    }

    /// After an aux slot appeared, vanished or changed kind: only what addresses
    /// this slot (`cs_revalidate_aux_dependents`).
    fn revalidate_aux(&mut self, aux_slot: u8) {
        for slot in 0..self.header.max_bindings {
            let Some(b) = self.live.bindings.get(usize::from(slot)).cloned() else {
                continue;
            };
            if slot == aux_slot
                || matches!(b.component, TY_NONE | TY_IR | TY_DISPLAY)
                || is_aux(b.component)
                || !matches!(b.noun, N_AUX | N_AUX_LEVEL)
                || b.flags & FLAG_GROUP != 0
                || b.target != aux_slot
            {
                continue;
            }
            let st = self.check_target(b.noun, b.target, b.index);
            self.recheck(slot, &b, st, true);
        }
        self.recheck_ir(|c| {
            matches!(c.noun, N_AUX | N_AUX_LEVEL)
                && c.flags & FLAG_GROUP == 0
                && c.target == aux_slot
        });
        self.recount_macros();
    }

    fn recheck_ir(&mut self, names_it: impl Fn(&IrCommand) -> bool) {
        for sub in 0..self.live.ir.len() {
            let st = match self.live.ir.get(sub) {
                Some(c) if !c.is_empty() && names_it(c) => self.check_ir(c, &c.encode()),
                _ => continue,
            };
            put(&mut self.ir_status, sub, st);
        }
    }

    fn recount_ir(&mut self) {
        for sub in 0..self.live.ir.len() {
            let st = match self.live.ir.get(sub) {
                Some(c) if !c.is_empty() => self.check_ir(c, &c.encode()),
                _ => SUCCESS,
            };
            put(&mut self.ir_status, sub, st);
        }
    }

    /// `cs_macro_recount_status`: the worst of each macro's executed steps.
    fn recount_macros(&mut self) {
        for i in 0..self.live.macros.len() {
            let Some(m) = self.live.macros.get(i) else {
                continue;
            };
            let count = usize::from(m.step_count);
            let worst = if count > usize::from(self.header.max_macro_steps) {
                INVALID_MACRO
            } else {
                m.steps
                    .iter()
                    .take(count)
                    .map(|s| self.check_step(s, 0))
                    .rfind(|st| *st != SUCCESS)
                    .unwrap_or(SUCCESS)
            };
            put(&mut self.macro_status, i, worst);
        }
    }

    /// Bring everything up from the live records, aux slots first as at boot
    /// (`cs_load_stored_slot`). A binding that no longer validates stays in its
    /// slot, down, with the reason in `slot_status`.
    fn reload(&mut self) {
        self.active = 0;
        for i in 0..self.live.groups.len() {
            let st = match self.live.groups.get(i) {
                Some(group) => Self::check_group(group, &group.encode()),
                None => SUCCESS,
            };
            put(&mut self.group_status, i, st);
        }
        self.slot_status.fill(SUCCESS);
        for aux_pass in [true, false] {
            for slot in 0..self.header.max_bindings {
                let Some(b) = self.live.bindings.get(usize::from(slot)).cloned() else {
                    continue;
                };
                if is_aux(b.component) != aux_pass || b.is_empty() {
                    continue;
                }
                let st = self.check_binding(&b, 0, slot);
                self.set_active(slot, st == SUCCESS);
                put(&mut self.slot_status, usize::from(slot), st);
            }
        }
        self.recount_ir();
        self.recount_macros();
    }

    fn settle(&mut self, slot: u8, code: u8) {
        self.last_slot = slot;
        self.last_status = code;
        self.dirty |= code == SUCCESS;
    }

    /// A `REQ_SET_CS_*` request. `None` for any other opcode; the transfer itself
    /// is always acknowledged, as on USB, and the outcome is the status packet's.
    pub(super) fn set(
        &mut self,
        opcode: u8,
        value: u16,
        d: &[u8],
        i2c_instance: Option<u8>,
    ) -> Option<Vec<u8>> {
        self.i2c_instance = i2c_instance;
        let slot = (value & 0xFF) as u8;
        let i = usize::from(slot);
        let mut events = Vec::new();
        match opcode {
            op::REQ_SET_CS_BINDING => {
                let code = if i >= self.live.bindings.len() {
                    INVALID_SLOT
                } else {
                    match CsBinding::decode(d) {
                        Ok(b) => self.apply_binding(slot, b, d.get(23).copied().unwrap_or(0)),
                        Err(_) => INVALID_VALUE,
                    }
                };
                self.settle(slot, code);
            }
            op::REQ_SET_CS_NAME => {
                let code = if i >= self.live.names.len() {
                    INVALID_SLOT
                } else if d.is_empty() {
                    INVALID_VALUE
                } else if self.busy {
                    BUSY
                } else {
                    let d = d.get(..d.len().min(NAME_LEN - 1)).unwrap_or(d);
                    let end = d.iter().position(|b| *b == 0).unwrap_or(d.len());
                    let text = String::from_utf8_lossy(d.get(..end).unwrap_or(d));
                    put(&mut self.live.names, i, text.into_owned());
                    SUCCESS
                };
                self.settle(slot, code);
            }
            op::REQ_SET_CS_IR_CMD => {
                let code = if i >= self.live.ir.len() {
                    INVALID_SLOT
                } else {
                    match IrCommand::decode(d) {
                        Ok(c) => self.apply_ir(i, c, d),
                        Err(_) => INVALID_VALUE,
                    }
                };
                self.settle(TAG_IR | slot, code);
            }
            op::REQ_SET_CS_AUX_STATE | op::REQ_SET_CS_AUX_LEVEL => {
                self.set_aux(opcode == op::REQ_SET_CS_AUX_LEVEL, slot, d, &mut events);
            }
            op::REQ_SET_CS_GROUP => {
                let code = if i >= self.live.groups.len() {
                    INVALID_GROUP
                } else {
                    match CsGroup::decode(d) {
                        Ok(group) => self.apply_group(i, group, d),
                        Err(_) => INVALID_VALUE,
                    }
                };
                self.settle(TAG_GROUP | slot, code);
            }
            op::REQ_SET_CS_MACRO => {
                let code = if i >= self.live.macros.len() {
                    INVALID_MACRO
                } else {
                    match CsMacroHeaderWire::decode(d) {
                        Ok(h) => self.apply_macro_header(i, h, d),
                        Err(_) => INVALID_VALUE,
                    }
                };
                self.settle(TAG_MACRO | slot, code);
            }
            op::REQ_SET_CS_MACRO_STEP => {
                let at = usize::from(value >> 8);
                let code = if i >= self.live.macros.len() {
                    INVALID_MACRO
                } else if at >= usize::from(self.header.max_macro_steps) {
                    INVALID_STEP
                } else {
                    match CsMacroStep::decode(d) {
                        Ok(s) => self.apply_step(i, at, s, d),
                        Err(_) => INVALID_VALUE,
                    }
                };
                self.settle(TAG_MACRO | slot, code);
            }
            op::REQ_SET_CS_DISPLAY_CFG => {
                let code = match CsDisplayCfg::decode(d) {
                    Ok(_) if self.busy => BUSY,
                    Ok(c) => {
                        let st = self.check_display_cfg(&c, d);
                        if st == SUCCESS {
                            self.live.display = c;
                        }
                        st
                    }
                    Err(_) => INVALID_VALUE,
                };
                self.settle(TAG_DISPLAY, code);
            }
            op::REQ_SET_CS_DISPLAY_PAGE => {
                let code = if i >= usize::from(self.max_pages) {
                    INVALID_PAGE
                } else {
                    match CsDisplayPage::decode(d) {
                        Ok(_) if self.busy => BUSY,
                        Ok(p) => {
                            let st = self.check_page(&p, d);
                            if st == SUCCESS {
                                put(&mut self.live.pages, i, p);
                            }
                            st
                        }
                        Err(_) => INVALID_VALUE,
                    }
                };
                self.settle(TAG_DISPLAY | slot, code);
            }
            _ => return None,
        }
        Some(events)
    }

    /// `control_surfaces_apply_ir_cmd`.
    fn apply_ir(&mut self, i: usize, c: IrCommand, raw: &[u8]) -> u8 {
        if self.busy {
            return BUSY;
        }
        let st = if c.is_empty() {
            if all_zero(raw.get(..IrCommand::SIZE).unwrap_or(raw)) {
                SUCCESS
            } else {
                INVALID_VALUE
            }
        } else {
            self.check_ir(&c, raw)
        };
        if st == SUCCESS {
            put(&mut self.live.ir, i, c);
            put(&mut self.ir_status, i, SUCCESS);
        }
        st
    }

    /// `control_surfaces_apply_group`.
    fn apply_group(&mut self, i: usize, group: CsGroup, raw: &[u8]) -> u8 {
        if self.busy {
            return BUSY;
        }
        let st = Self::check_group(&group, raw.get(..CsGroup::SIZE).unwrap_or(raw));
        if st != SUCCESS {
            return st;
        }
        put(&mut self.live.groups, i, group);
        put(&mut self.group_status, i, SUCCESS);
        self.revalidate_group(u8::try_from(i).unwrap_or(u8::MAX));
        SUCCESS
    }

    /// `control_surfaces_apply_macro_header`.
    fn apply_macro_header(&mut self, i: usize, h: CsMacroHeaderWire, raw: &[u8]) -> u8 {
        if self.busy {
            return BUSY;
        }
        if h.step_count > self.header.max_macro_steps {
            return INVALID_MACRO;
        }
        if !all_zero(raw.get(33..36).unwrap_or(&[])) {
            return INVALID_VALUE;
        }
        if let Some(m) = self.live.macros.get_mut(i) {
            m.name = h.name;
            m.step_count = h.step_count;
        }
        self.recount_macros();
        SUCCESS
    }

    /// `control_surfaces_apply_macro_step`.
    fn apply_step(&mut self, i: usize, at: usize, s: CsMacroStep, raw: &[u8]) -> u8 {
        if self.busy {
            return BUSY;
        }
        let st = self.check_step(&s, raw.get(5).copied().unwrap_or(0));
        if st != SUCCESS {
            return st;
        }
        if let Some(step) = self
            .live
            .macros
            .get_mut(i)
            .and_then(|m| m.steps.get_mut(at))
        {
            *step = s;
        }
        self.recount_macros();
        SUCCESS
    }

    /// `REQ_SET_CS_AUX_STATE`, `_LEVEL`: applied at once, no dirty flag, and a
    /// success leaves the status alone.
    fn set_aux(&mut self, is_level: bool, slot: u8, d: &[u8], events: &mut Vec<u8>) {
        let i = usize::from(slot);
        if i >= self.live.bindings.len() {
            (self.last_status, self.last_slot) = (INVALID_AUX, slot);
            return;
        }
        if d.len() < if is_level { 2 } else { 1 } {
            (self.last_status, self.last_slot) = (INVALID_VALUE, slot);
            return;
        }
        if self.busy {
            return;
        }
        let ty = self.slot_type(slot);
        let live = is_aux(ty) && self.is_active(slot);
        if !live || (is_level && ty != TY_AUX_PWM) {
            (self.last_status, self.last_slot) = (INVALID_AUX, slot);
            return;
        }
        let was = (
            self.aux.state.get(i).copied(),
            self.aux.level_q8.get(i).copied(),
        );
        if let [lo, hi, ..] = *d
            && is_level
        {
            let max = self
                .nouns
                .get(usize::from(N_AUX_LEVEL))
                .map_or(0, |n| n.max_q.max(0) as u16);
            put(
                &mut self.aux.level_q8,
                i,
                u16::from_le_bytes([lo, hi]).min(max),
            );
        } else if let Some(on) = d.first()
            && !is_level
        {
            put(&mut self.aux.state, i, u8::from(*on != 0));
        }
        let (state, level) = (
            self.aux.state.get(i).copied(),
            self.aux.level_q8.get(i).copied(),
        );
        if (state, level) != was {
            events.push(slot);
            events.push(state.unwrap_or(0));
            events.extend_from_slice(&level.unwrap_or(0).to_le_bytes());
            events.push(nfy::PARAM_SRC_HOST_SET as u8);
        }
    }

    // ---------------------------------------------------------------- reads

    fn status_packet(&self) -> Vec<u8> {
        let ir_mask = self
            .live
            .ir
            .iter()
            .zip(0u32..16)
            .filter(|(c, sub)| !c.is_empty() && self.ir_status.get(*sub as usize) == Some(&SUCCESS))
            .fold(0u16, |m, (_, sub)| m | 1u16.checked_shl(sub).unwrap_or(0));
        let ir_up = self
            .slots()
            .any(|(s, b)| b.component == TY_IR && self.is_active(s));
        CsStatusPacket {
            last_status: self.last_status,
            last_slot: self.last_slot,
            max_bindings: self.header.max_bindings,
            dirty: self.dirty,
            active_mask: self.active,
            slot_status: self.slot_status.clone(),
            ir_active_mask: if ir_up { ir_mask } else { 0 },
            ir_learn_state: self.learn_state,
            ir_cmd_status: self.ir_status.clone(),
        }
        .encode()
    }

    fn ext_status(&mut self) -> Vec<u8> {
        let mut group_status = [0u8; 8];
        for (x, st) in group_status.iter_mut().zip(&self.group_status) {
            *x = *st;
        }
        let mut macro_status = [0u8; 8];
        for (x, st) in macro_status.iter_mut().zip(&self.macro_status) {
            *x = *st;
        }
        let packet = CsExtStatusPacket {
            max_groups: self.header.max_groups,
            max_macros: self.header.max_macros,
            max_macro_steps: self.header.max_macro_steps,
            macro_running: self.macro_run,
            macro_step: if self.macro_run == MACRO_NONE {
                0
            } else {
                self.macro_step
            },
            group_status,
            macro_status,
        }
        .encode()
        .to_vec();
        // One step per look: see the module docs.
        if self.macro_run != MACRO_NONE {
            let count = self
                .live
                .macros
                .get(usize::from(self.macro_run))
                .map_or(0, |m| m.step_count);
            self.macro_step = self.macro_step.saturating_add(1);
            if self.macro_step >= count {
                self.macro_run = MACRO_NONE;
                self.macro_step = 0;
            }
        }
        packet
    }

    fn display_status(&mut self) -> Vec<u8> {
        let display = self
            .slots()
            .find(|(s, b)| b.component == TY_DISPLAY && self.is_active(*s))
            .map(|(_, b)| b.index);
        let cfg = &self.live.display;
        let current = if cfg.mode == DMODE_FIXED {
            cfg.home_page
        } else {
            self.shown_page
        };
        let reply = CsDisplayStatus {
            init_state: if display.is_some() { DISPLAY_LIVE } else { 0 },
            current_page: current,
            flags: 0,
            model: display.unwrap_or(0),
            nak_count: 0,
        }
        .encode()
        .to_vec();
        if cfg.mode != DMODE_FIXED {
            // The next active page after this one, this one if there is no other.
            let n = usize::from(self.max_pages);
            let from = usize::from(self.shown_page);
            self.shown_page = (1..=n)
                .map(|k| from.saturating_add(k).checked_rem(n).unwrap_or(0))
                .find(|p| {
                    self.live
                        .pages
                        .get(*p)
                        .is_some_and(|x| x.flags & page_flags::ACTIVE != 0)
                })
                .and_then(|p| u8::try_from(p).ok())
                .unwrap_or(self.shown_page);
        }
        reply
    }

    fn macro_reply(&self, i: usize) -> Option<Vec<u8>> {
        let m = self.live.macros.get(i)?;
        let mut b = m.header().encode().to_vec();
        for s in m
            .steps
            .iter()
            .take(usize::from(self.header.max_macro_steps))
        {
            b.extend_from_slice(&s.encode());
        }
        b.resize(CsMacro::SIZE, 0);
        Some(b)
    }

    /// `REQ_CS_SAVE`: `control_surfaces_aux_prepare_save`, then the store. The
    /// firmware's refusal is a request still queued, which reads as a stall here
    /// while [`EXT_FAKE_CS_SIM`](super::EXT_FAKE_CS_SIM) holds the device.
    fn save_or_revert(&mut self, save: bool) -> Option<Vec<u8>> {
        if self.busy {
            (self.last_status, self.last_slot) = (BUSY, TAG_SAVE);
            return None;
        }
        if save {
            for (i, b) in self.live.bindings.iter_mut().enumerate() {
                if !is_aux(b.component) || b.extras & aux_extras::BOOT_SAVED == 0 {
                    continue;
                }
                let up = u32::try_from(i).ok().is_some_and(|s| {
                    u32::from(self.active) & 1u32.checked_shl(s).unwrap_or(0) != 0
                });
                if !up {
                    continue;
                }
                if self.aux.state.get(i).is_some_and(|s| *s != 0) {
                    b.extras |= aux_extras::BOOT_ON;
                } else {
                    b.extras &= !aux_extras::BOOT_ON;
                }
                if b.component == TY_AUX_PWM {
                    b.value = self.aux.level_q8.get(i).copied().unwrap_or(0) as i16;
                }
            }
            self.stored = self.live.clone();
        } else {
            let before: Vec<u8> = self.live.bindings.iter().map(|b| b.component).collect();
            self.live = self.stored.clone();
            self.reload();
            // An output of the same kind keeps its value (`s_aux_carry`).
            for slot in 0..self.header.max_bindings {
                if before.get(usize::from(slot)).copied() != Some(self.slot_type(slot)) {
                    self.seed_aux(slot);
                }
            }
        }
        self.dirty = false;
        (self.last_status, self.last_slot) = (SUCCESS, TAG_SAVE);
        Some(vec![1])
    }

    fn fire(&mut self, value: u16) -> Option<Vec<u8>> {
        if value == 0xFFFF {
            self.macro_run = MACRO_NONE;
            self.macro_step = 0;
            return Some(vec![1]);
        }
        let i = usize::from(value);
        if self.busy || i >= self.live.macros.len() {
            return None;
        }
        let count = self.live.macros.get(i).map_or(0, |m| m.step_count);
        self.macro_step = 0;
        self.macro_run = if count == 0 || count > self.header.max_macro_steps {
            MACRO_NONE
        } else {
            u8::try_from(i).unwrap_or(MACRO_NONE)
        };
        Some(vec![1])
    }

    fn learn(&mut self, value: u16) -> Option<Vec<u8>> {
        match value {
            2 => {
                let mut b = vec![self.learn_state, self.learn_protocol, 0, 0];
                b.extend_from_slice(&self.learn_code.to_le_bytes());
                Some(b)
            }
            1 | 0 if self.busy => None,
            1 => {
                let receiver = self
                    .slots()
                    .any(|(s, b)| b.component == TY_IR && self.is_active(s));
                if !receiver {
                    (self.last_status, self.last_slot) = (NO_IR, TAG_SAVE);
                    return None;
                }
                self.learn_state = g::CS_IR_LEARN_ARMED as u8;
                self.learn_protocol = 0;
                self.learn_code = 0;
                Some(vec![1])
            }
            0 => {
                self.learn_state = g::CS_IR_LEARN_IDLE as u8;
                Some(vec![1])
            }
            _ => None,
        }
    }

    /// A `REQ_*_CS_*` IN request: `None` for any other opcode, `Some(None)` for a
    /// stall.
    pub(super) fn get(&mut self, opcode: u8, value: u16) -> Option<Option<Vec<u8>>> {
        let i = usize::from(value);
        let reply = match opcode {
            op::REQ_GET_CS_CAPS if value == 0xFFFF => Some(self.header.encode()),
            op::REQ_GET_CS_CAPS => self.nouns.get(i).map(|n| n.encode().to_vec()),
            op::REQ_GET_CS_BINDING => self.live.bindings.get(i).map(|b| b.encode().to_vec()),
            op::REQ_GET_CS_NAME => self.live.names.get(usize::from(value as u8)).map(|n| {
                let mut b = n.as_bytes().to_vec();
                b.resize(NAME_LEN, 0);
                b
            }),
            op::REQ_GET_CS_IR_CMD => self.live.ir.get(i).map(|c| c.encode().to_vec()),
            op::REQ_GET_CS_GROUP => self.live.groups.get(i).map(|x| x.encode().to_vec()),
            op::REQ_GET_CS_MACRO => self.macro_reply(i),
            op::REQ_GET_CS_STATUS => Some(self.status_packet()),
            op::REQ_GET_CS_EXT_STATUS => Some(self.ext_status()),
            op::REQ_GET_CS_DISPLAY_CFG => Some(
                CsDisplayCfgReply {
                    max_pages: self.max_pages,
                    model_count: self.model_count,
                    cfg: self.live.display.clone(),
                }
                .encode()
                .to_vec(),
            ),
            op::REQ_GET_CS_DISPLAY_PAGE => self.live.pages.get(i).map(|p| p.encode().to_vec()),
            op::REQ_GET_CS_DISPLAY_STATUS => Some(self.display_status()),
            op::REQ_GET_CS_AUX_STATE if value == 0xFFFF => Some(self.aux.encode().to_vec()),
            op::REQ_GET_CS_AUX_STATE | op::REQ_GET_CS_AUX_LEVEL => {
                let up = value < 16
                    && is_aux(self.slot_type(value as u8))
                    && self.is_active(value as u8);
                if !up {
                    None
                } else if opcode == op::REQ_GET_CS_AUX_STATE {
                    self.aux.state.get(i).map(|s| vec![*s])
                } else {
                    self.aux.level_q8.get(i).map(|l| l.to_le_bytes().to_vec())
                }
            }
            op::REQ_CS_SAVE => self.save_or_revert(true),
            op::REQ_CS_REVERT => self.save_or_revert(false),
            op::REQ_CS_MACRO_FIRE => self.fire(value),
            op::REQ_CS_IR_LEARN => self.learn(value),
            _ => return None,
        };
        Some(reply)
    }

    /// A learn's result; false when nothing was armed.
    fn learned(&mut self, protocol: u8, code: u32) -> bool {
        if self.learn_state != g::CS_IR_LEARN_ARMED as u8 {
            return false;
        }
        self.learn_state = g::CS_IR_LEARN_DONE as u8;
        self.learn_protocol = protocol;
        self.learn_code = code;
        true
    }
}

impl State {
    /// [`Surfaces::get`], when this device has them.
    pub(super) fn get_surface(&mut self, opcode: u8, value: u16) -> Option<Option<Vec<u8>>> {
        let i2c = self.i2c_ctrl_live_instance();
        let surfaces = self.surfaces.as_mut()?;
        surfaces.i2c_instance = i2c;
        surfaces.get(opcode, value)
    }

    /// [`Surfaces::set`], when this device has them, with the aux change's
    /// `NOTIFY_EVT_CS_AUX`.
    pub(super) fn set_surface(&mut self, opcode: u8, value: u16, data: &[u8]) -> Option<bool> {
        let i2c = self.i2c_ctrl_live_instance();
        let event = self.surfaces.as_mut()?.set(opcode, value, data, i2c)?;
        if !event.is_empty() {
            let mut p = vec![
                nfy::NOTIFY_V2_VERSION as u8,
                nfy::NOTIFY_EVT_CS_AUX as u8,
                0,
                0,
            ];
            p.extend_from_slice(&event);
            self.push(p);
        }
        Some(true)
    }
}

impl FakeRemote {
    /// The remote button a test pressed while learning: the learn ends `DONE`
    /// and notifies, as `cs_tick_ir` does. False when nothing was armed.
    pub fn cs_learned(&self, protocol: u8, code: u32) -> bool {
        let mut s = self.shared.lock();
        let done = s
            .surfaces
            .as_mut()
            .is_some_and(|x| x.learned(protocol, code));
        if done {
            let mut p = vec![
                nfy::NOTIFY_V2_VERSION as u8,
                nfy::NOTIFY_EVT_CS_IR_LEARN as u8,
                0,
                0,
                g::CS_IR_LEARN_DONE as u8,
                protocol,
                0,
                0,
            ];
            p.extend_from_slice(&code.to_le_bytes());
            s.push(p);
            drop(s);
            self.shared.wake.notify_all();
        }
        done
    }
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_cg16 {
    use super::super::EXT_FAKE_CS_SIM;
    use super::super::{Capture, FakeDevice, FakeOptions};
    use super::*;
    use crate::{Transport, TransportError};

    fn status(d: &mut FakeDevice) -> CsStatusPacket {
        let b = d
            .control_in(op::REQ_GET_CS_STATUS, 0, CsStatusPacket::SIZE as u16)
            .unwrap();
        CsStatusPacket::decode(&b).unwrap()
    }

    fn binding(d: &mut FakeDevice, slot: u16) -> CsBinding {
        let b = d
            .control_in(op::REQ_GET_CS_BINDING, slot, CsBinding::SIZE as u16)
            .unwrap();
        CsBinding::decode(&b).unwrap()
    }

    /// Set a binding; the transfer is acknowledged whatever became of it.
    fn set(d: &mut FakeDevice, slot: u16, b: &CsBinding) -> u8 {
        d.control_out(op::REQ_SET_CS_BINDING, slot, &b.encode())
            .unwrap();
        let st = status(d);
        assert_eq!(st.last_slot, slot as u8);
        st.last_status
    }

    fn button(pin: u8) -> CsBinding {
        CsBinding {
            component: TY_BUTTON,
            noun: g::CS_NOUN_USER_MUTE as u8,
            action: ACT_TOGGLE,
            gpio: [pin, GPIO_UNUSED],
            value: 1,
            ..Default::default()
        }
    }

    fn led(pin: u8) -> CsBinding {
        CsBinding {
            component: TY_LED,
            noun: g::CS_NOUN_USER_MUTE as u8,
            action: ACT_IND_EQUALS,
            gpio: [pin, GPIO_UNUSED],
            value: 1,
            ..Default::default()
        }
    }

    fn stalled<T>(r: crate::Result<T>) -> bool {
        matches!(r, Err(TransportError::Stalled { .. }))
    }

    #[test]
    fn wp_cg16_the_demo_has_surfaces_and_stock_firmware_stalls_them() {
        let mut d = FakeDevice::demo();
        assert_eq!(binding(&mut d, 0).component, TY_ENCODER);
        for mut stock in [
            FakeDevice::demo_without_surfaces(),
            FakeDevice::new(Capture::demo(), FakeOptions::default()),
        ] {
            assert!(stalled(stock.control_in(op::REQ_GET_CS_CAPS, 0xFFFF, 4)));
            assert!(stalled(stock.control_in(op::REQ_GET_CS_STATUS, 0, 41)));
            assert!(stalled(stock.control_in(op::REQ_CS_SAVE, 0, 1)));
            assert!(stalled(stock.control_out(
                op::REQ_SET_CS_BINDING,
                0,
                &button(5).encode()
            )));
        }
    }

    /// The seed is a device the firmware would have booted with nothing down.
    #[test]
    fn wp_cg16_every_seeded_record_passes_the_firmwares_rules() {
        let s = Surfaces::new();
        assert_eq!(s.active, 0b0001_1100_0001_1111);
        assert!(
            s.slot_status.iter().all(|x| *x == SUCCESS),
            "{:?}",
            s.slot_status
        );
        assert!(s.ir_status.iter().all(|x| *x == SUCCESS));
        assert!(s.group_status.iter().all(|x| *x == SUCCESS));
        assert!(s.macro_status.iter().all(|x| *x == SUCCESS));
        for (i, p) in s.live.pages.iter().enumerate() {
            assert_eq!(s.check_page(p, &p.encode()), SUCCESS, "page {i}");
        }
        assert_eq!(
            s.check_display_cfg(&s.live.display, &s.live.display.encode()),
            SUCCESS
        );
    }

    /// `vendor_commands.c`: a read of a slot that does not exist stalls, and a
    /// SET to one is acknowledged and refused in the status, under its tag.
    #[test]
    fn wp_cg16_bad_indices_as_the_handlers_treat_them() {
        let mut d = FakeDevice::demo();
        assert!(stalled(d.control_in(op::REQ_GET_CS_BINDING, 16, 24)));
        assert!(stalled(d.control_in(op::REQ_GET_CS_CAPS, 79, 12)));
        assert!(stalled(d.control_in(op::REQ_GET_CS_GROUP, 8, 40)));
        assert!(stalled(d.control_in(op::REQ_GET_CS_MACRO, 8, 132)));
        assert!(stalled(d.control_in(op::REQ_GET_CS_IR_CMD, 16, 16)));
        assert!(stalled(d.control_in(op::REQ_GET_CS_DISPLAY_PAGE, 16, 4)));
        assert!(
            stalled(d.control_in(op::REQ_GET_CS_AUX_STATE, 0, 1)),
            "not an aux"
        );

        let mut refused = |opcode, value, data: &[u8], code: u8, tag: u8| {
            d.control_out(opcode, value, data).unwrap();
            let st = status(&mut d);
            assert_eq!(
                (st.last_status, st.last_slot),
                (code, tag),
                "0x{opcode:02X}"
            );
            assert!(!st.dirty, "0x{opcode:02X}");
        };
        let b = button(5).encode();
        refused(op::REQ_SET_CS_BINDING, 16, &b, INVALID_SLOT, 16);
        refused(op::REQ_SET_CS_BINDING, 5, &b[..16], INVALID_VALUE, 5);
        refused(op::REQ_SET_CS_NAME, 16, b"x", INVALID_SLOT, 16);
        refused(op::REQ_SET_CS_NAME, 3, b"", INVALID_VALUE, 3);
        refused(
            op::REQ_SET_CS_IR_CMD,
            16,
            &[0; 16],
            INVALID_SLOT,
            TAG_IR | 16,
        );
        refused(
            op::REQ_SET_CS_GROUP,
            8,
            &[0; 40],
            INVALID_GROUP,
            TAG_GROUP | 8,
        );
        refused(
            op::REQ_SET_CS_MACRO,
            8,
            &[0; 36],
            INVALID_MACRO,
            TAG_MACRO | 8,
        );
        refused(
            op::REQ_SET_CS_MACRO_STEP,
            8,
            &[0; 12],
            INVALID_MACRO,
            TAG_MACRO | 8,
        );
        refused(
            op::REQ_SET_CS_MACRO_STEP,
            8 << 8,
            &[0; 12],
            INVALID_STEP,
            TAG_MACRO,
        );
        refused(
            op::REQ_SET_CS_DISPLAY_PAGE,
            16,
            &[0; 4],
            INVALID_PAGE,
            TAG_DISPLAY | 16,
        );
        refused(op::REQ_SET_CS_AUX_STATE, 16, &[1], INVALID_AUX, 16);
        refused(op::REQ_SET_CS_AUX_LEVEL, 11, &[1], INVALID_VALUE, 11);
    }

    #[test]
    fn wp_cg16_what_a_binding_can_be_refused_for() {
        let mut d = FakeDevice::demo();
        let free = 5;
        let before = binding(&mut d, free);
        let mut cases: Vec<(&str, CsBinding, u8)> = vec![
            (
                "type",
                CsBinding {
                    component: 11,
                    ..button(5)
                },
                INVALID_TYPE,
            ),
            (
                "noun",
                CsBinding {
                    noun: 79,
                    ..button(5)
                },
                INVALID_NOUN,
            ),
            (
                "action",
                CsBinding {
                    action: 12,
                    ..button(5)
                },
                INVALID_ACTION,
            ),
            (
                "type's action",
                CsBinding {
                    action: ACT_STEP,
                    ..button(5)
                },
                INVALID_ACTION,
            ),
            (
                "event",
                CsBinding {
                    event: 3,
                    ..button(5)
                },
                INVALID_EVENT,
            ),
            (
                "event off a button",
                CsBinding { event: 1, ..led(5) },
                INVALID_EVENT,
            ),
            (
                "group flag",
                CsBinding {
                    flags: FLAG_LINK_ABS,
                    ..button(5)
                },
                INVALID_VALUE,
            ),
            (
                "accel off an encoder",
                CsBinding {
                    flags: FLAG_ACCEL,
                    ..button(5)
                },
                INVALID_VALUE,
            ),
            (
                "extras off an aux",
                CsBinding {
                    extras: 1,
                    ..button(5)
                },
                INVALID_VALUE,
            ),
            (
                "bright off a PWM",
                CsBinding {
                    base_bright: 10,
                    ..button(5)
                },
                INVALID_VALUE,
            ),
            (
                "target on an untargeted noun",
                CsBinding {
                    target: 1,
                    ..button(5)
                },
                INVALID_TARGET,
            ),
            (
                "value",
                CsBinding {
                    action: ACT_SET,
                    value: 2,
                    ..button(5)
                },
                INVALID_VALUE,
            ),
            // The pins: 16 is slot 1's button, 10 slot 0's encoder.
            ("same gesture", button(16), EVENT_IN_USE),
            (
                "sense",
                CsBinding {
                    flags: FLAG_INVERT,
                    event: 1,
                    ..button(16)
                },
                INVALID_VALUE,
            ),
            ("button on an encoder's pin", button(10), PIN_IN_USE),
            ("LED on a button's pin", led(16), PIN_IN_USE),
            ("power pin", button(24), INVALID_PIN),
            ("no such GPIO", button(30), INVALID_PIN),
            (
                "equal pins",
                CsBinding {
                    component: TY_ENCODER,
                    noun: g::CS_NOUN_USER_VOLUME as u8,
                    action: ACT_STEP,
                    gpio: [5, 5],
                    value: 0,
                    ..Default::default()
                },
                INVALID_PIN,
            ),
            (
                "pot off the ADC",
                CsBinding {
                    component: TY_POT,
                    noun: g::CS_NOUN_USER_VOLUME as u8,
                    action: ACT_ADJUST,
                    gpio: [5, GPIO_UNUSED],
                    value: 0,
                    ..Default::default()
                },
                PIN_NOT_ADC,
            ),
            (
                "second IR",
                CsBinding {
                    component: TY_IR,
                    gpio: [5, GPIO_UNUSED],
                    ..Default::default()
                },
                IR_IN_USE,
            ),
            (
                "second display",
                CsBinding {
                    component: TY_DISPLAY,
                    gpio: [6, 7],
                    index: 6,
                    ..Default::default()
                },
                DISPLAY_IN_USE,
            ),
            (
                "I2C pair",
                CsBinding {
                    component: TY_DISPLAY,
                    gpio: [6, 9],
                    index: 6,
                    ..Default::default()
                },
                PIN_NOT_I2C,
            ),
            (
                "display model",
                CsBinding {
                    component: TY_DISPLAY,
                    gpio: [6, 7],
                    index: 9,
                    ..Default::default()
                },
                INVALID_VALUE,
            ),
            // Slot 11 is a dimmable output on GPIO 28, which GPIO 12 shares a PWM
            // output with.
            (
                "PWM",
                CsBinding {
                    component: TY_AUX_PWM,
                    gpio: [12, GPIO_UNUSED],
                    ..Default::default()
                },
                PWM_CONFLICT,
            ),
            // Slot 0 is no aux output, and a level needs a dimmable one.
            (
                "aux target",
                CsBinding {
                    noun: N_AUX,
                    target: 0,
                    ..button(5)
                },
                INVALID_AUX,
            ),
            (
                "aux level target",
                CsBinding {
                    noun: N_AUX_LEVEL,
                    action: ACT_INC,
                    target: 10,
                    value: 0,
                    ..button(5)
                },
                INVALID_AUX,
            ),
            (
                "a group of nothing",
                CsBinding {
                    noun: g::CS_NOUN_OUTPUT_MUTE as u8,
                    flags: FLAG_GROUP,
                    target: 7,
                    ..button(5)
                },
                INVALID_GROUP,
            ),
        ];
        for (what, b, code) in cases.drain(..) {
            assert_eq!(set(&mut d, free, &b), code, "{what}");
            assert_eq!(binding(&mut d, free), before, "{what} kept the old binding");
        }
        assert!(!status(&mut d).dirty);

        // A control cannot target the slot it is about to occupy.
        let own = CsBinding {
            noun: N_AUX,
            target: 10,
            ..button(5)
        };
        assert_eq!(set(&mut d, 10, &own), INVALID_TARGET);

        // Byte 23 is reserved.
        let mut raw = button(5).encode();
        raw[23] = 1;
        d.control_out(op::REQ_SET_CS_BINDING, free, &raw).unwrap();
        assert_eq!(status(&mut d).last_status, INVALID_VALUE);

        // Not refused: another gesture on a button's pin, an edit in place, and
        // clearing whatever a cleared record holds.
        assert_eq!(
            set(
                &mut d,
                free,
                &CsBinding {
                    event: 1,
                    ..button(16)
                }
            ),
            SUCCESS
        );
        assert_eq!(
            set(
                &mut d,
                1,
                &CsBinding {
                    event: 2,
                    ..button(16)
                }
            ),
            SUCCESS
        );
        assert_eq!(set(&mut d, free, &CsBinding::default()), SUCCESS);
        assert_eq!(set(&mut d, 0, &binding_on_new_pins()), SUCCESS);
    }

    fn binding_on_new_pins() -> CsBinding {
        CsBinding {
            component: TY_ENCODER,
            noun: g::CS_NOUN_USER_VOLUME as u8,
            action: ACT_STEP,
            gpio: [10, 12],
            ..Default::default()
        }
    }

    /// `cs_validate_display_container` asks which block the I2C control
    /// interface holds.
    #[test]
    fn wp_cg16_a_display_stays_off_the_control_interfaces_i2c() {
        let mut d = FakeDevice::demo();
        assert_eq!(set(&mut d, 4, &CsBinding::default()), SUCCESS);
        let cfg = dspi_proto::packets::I2cCtrlConfig {
            enabled: true,
            sda_pin: 6,
            scl_pin: 7,
            address: 0x42,
        };
        d.control_out(op::REQ_SET_I2C_CONFIG, 0, &cfg.encode())
            .unwrap();
        let display = |gpio| CsBinding {
            component: TY_DISPLAY,
            gpio,
            index: 6,
            ..Default::default()
        };
        assert_eq!(set(&mut d, 4, &display([6, 7])), I2C_IN_USE);
        assert_eq!(set(&mut d, 4, &display([4, 5])), SUCCESS, "I2C0");
    }

    #[test]
    fn wp_cg16_the_other_records_follow_their_rules() {
        let mut d = FakeDevice::demo();
        fn code_on(d: &mut FakeDevice, opcode: u8, value: u16, data: &[u8]) -> u8 {
            d.control_out(opcode, value, data).unwrap();
            status(d).last_status
        }
        macro_rules! code {
            ($opcode:expr, $value:expr, $data:expr) => {
                code_on(&mut d, $opcode, $value, $data)
            };
        }
        let ir = |x: IrCommand| x.encode();
        let good = IrCommand {
            noun: g::CS_NOUN_USER_MUTE as u8,
            action: ACT_TOGGLE,
            protocol: 1,
            code: 7,
            ..Default::default()
        };
        assert_eq!(code!(op::REQ_SET_CS_IR_CMD, 1, &ir(good.clone())), SUCCESS);
        let never = IrCommand {
            code: 0,
            ..good.clone()
        };
        assert_eq!(code!(op::REQ_SET_CS_IR_CMD, 1, &ir(never)), INVALID_VALUE);
        let proto = IrCommand {
            protocol: 5,
            ..good.clone()
        };
        assert_eq!(code!(op::REQ_SET_CS_IR_CMD, 1, &ir(proto)), INVALID_VALUE);
        let no_ir_action = IrCommand {
            action: ACT_ADJUST,
            ..good.clone()
        };
        assert_eq!(
            code!(op::REQ_SET_CS_IR_CMD, 1, &ir(no_ir_action)),
            INVALID_ACTION
        );
        let unclean = IrCommand {
            protocol: 0,
            ..good
        };
        assert_eq!(
            code!(op::REQ_SET_CS_IR_CMD, 1, &ir(unclean)),
            INVALID_VALUE,
            "a clear is all-zero"
        );
        assert_eq!(code!(op::REQ_SET_CS_IR_CMD, 1, &[0; 16]), SUCCESS);

        let group = |target_kind, member_mask| {
            CsGroup {
                target_kind,
                member_mask,
                name: "G".into(),
            }
            .encode()
        };
        assert_eq!(
            code!(op::REQ_SET_CS_GROUP, 1, &group(T_INPUT, 0b11)),
            SUCCESS
        );
        assert_eq!(
            code!(op::REQ_SET_CS_GROUP, 1, &group(T_INPUT, 1 << 8)),
            INVALID_GROUP,
            "outside"
        );
        assert_eq!(
            code!(op::REQ_SET_CS_GROUP, 1, &group(T_BAND, 1)),
            INVALID_GROUP
        );
        assert_eq!(
            code!(op::REQ_SET_CS_GROUP, 1, &group(T_INPUT, 0)),
            INVALID_GROUP
        );
        assert_eq!(
            code!(op::REQ_SET_CS_GROUP, 1, &group(T_NONE, 1)),
            INVALID_VALUE,
            "a clear is all-zero"
        );

        let step = |noun, action, value| {
            CsMacroStep {
                noun,
                action,
                value,
                ..Default::default()
            }
            .encode()
        };
        let preset = g::CS_NOUN_PRESET as u8;
        assert_eq!(
            code!(op::REQ_SET_CS_MACRO_STEP, 1, &step(preset, ACT_SET, 3)),
            SUCCESS
        );
        assert_eq!(
            code!(op::REQ_SET_CS_MACRO_STEP, 1, &step(preset, ACT_SET, 10)),
            INVALID_VALUE
        );
        assert_eq!(
            code!(op::REQ_SET_CS_MACRO_STEP, 1, &step(N_MACRO, ACT_SET, 0)),
            INVALID_STEP,
            "no nesting"
        );
        assert_eq!(
            code!(op::REQ_SET_CS_MACRO_STEP, 1, &step(preset, ACT_STEP, 0)),
            INVALID_STEP
        );
        assert_eq!(
            code!(
                op::REQ_SET_CS_MACRO_STEP,
                1,
                &step(g::CS_NOUN_USER_MUTE as u8, ACT_TRIGGER, 0)
            ),
            INVALID_ACTION
        );
        assert_eq!(
            code!(op::REQ_SET_CS_MACRO_STEP, 1, &[0; 12]),
            SUCCESS,
            "an empty step"
        );
        let header = |step_count| {
            CsMacroHeaderWire {
                name: "M".into(),
                step_count,
            }
            .encode()
        };
        assert_eq!(code!(op::REQ_SET_CS_MACRO, 1, &header(9)), INVALID_MACRO);
        assert_eq!(code!(op::REQ_SET_CS_MACRO, 1, &header(1)), SUCCESS);
        assert_eq!(
            code!(op::REQ_SET_CS_MACRO_STEP, 1, &step(preset, ACT_SET, 3)),
            SUCCESS
        );
        let stored = d.control_in(op::REQ_GET_CS_MACRO, 1, 132).unwrap();
        let stored = CsMacro::decode(&stored).unwrap();
        assert_eq!(
            (
                stored.name.as_str(),
                stored.step_count,
                stored.steps[0].value
            ),
            ("M", 1, 3)
        );

        let page = |noun, flags| {
            CsDisplayPage {
                noun,
                target: 0,
                index: 0,
                flags,
            }
            .encode()
        };
        let volume = g::CS_NOUN_USER_VOLUME as u8;
        assert_eq!(
            code!(
                op::REQ_SET_CS_DISPLAY_PAGE,
                2,
                &page(volume, page_flags::ACTIVE | page_flags::BAR)
            ),
            SUCCESS
        );
        assert_eq!(
            code!(
                op::REQ_SET_CS_DISPLAY_PAGE,
                2,
                &page(
                    g::CS_NOUN_USER_MUTE as u8,
                    page_flags::ACTIVE | page_flags::BAR
                )
            ),
            INVALID_PAGE
        );
        assert_eq!(
            code!(
                op::REQ_SET_CS_DISPLAY_PAGE,
                2,
                &page(N_PAGE_VALUE, page_flags::ACTIVE)
            ),
            INVALID_PAGE
        );
        assert_eq!(
            code!(
                op::REQ_SET_CS_DISPLAY_PAGE,
                2,
                &page(g::CS_NOUN_ADAT_ACTIVE as u8, page_flags::ACTIVE)
            ),
            INVALID_NOUN,
            "unavailable"
        );
        assert_eq!(
            code!(op::REQ_SET_CS_DISPLAY_PAGE, 2, &page(preset, 0)),
            INVALID_PAGE,
            "a clear is all-zero"
        );
        assert_eq!(code!(op::REQ_SET_CS_DISPLAY_PAGE, 2, &[0; 4]), SUCCESS);

        let base = surface_seed().display_cfg;
        let cfg = |c: CsDisplayCfg| c.encode();
        assert_eq!(
            code!(
                op::REQ_SET_CS_DISPLAY_CFG,
                0,
                &cfg(CsDisplayCfg {
                    mode: 3,
                    ..base.clone()
                })
            ),
            INVALID_VALUE
        );
        assert_eq!(
            code!(
                op::REQ_SET_CS_DISPLAY_CFG,
                0,
                &cfg(CsDisplayCfg {
                    home_page: 16,
                    ..base.clone()
                })
            ),
            INVALID_PAGE
        );
        assert_eq!(
            code!(
                op::REQ_SET_CS_DISPLAY_CFG,
                0,
                &cfg(CsDisplayCfg {
                    dwell: 9,
                    ..base.clone()
                })
            ),
            INVALID_VALUE
        );
        assert_eq!(
            code!(
                op::REQ_SET_CS_DISPLAY_CFG,
                0,
                &cfg(CsDisplayCfg {
                    mode: 0,
                    dwell: 9,
                    ..base
                })
            ),
            SUCCESS
        );
    }

    /// Bindings on an aux slot or a group go down with it and come back with it,
    /// as stored records, with no in-use refusal.
    #[test]
    fn wp_cg16_a_removed_target_takes_its_controls_down() {
        let mut d = FakeDevice::demo();
        assert_ne!(status(&mut d).active_mask & (1 << 12), 0);
        assert_eq!(set(&mut d, 10, &CsBinding::default()), SUCCESS);
        let st = status(&mut d);
        assert_eq!(st.active_mask & (1 << 12), 0);
        assert_eq!(st.slot_status[12], INVALID_AUX);
        assert_eq!(binding(&mut d, 12).noun, N_AUX, "kept");

        let aux = CsBinding {
            component: TY_AUX_OUT,
            gpio: [27, GPIO_UNUSED],
            ..Default::default()
        };
        assert_eq!(set(&mut d, 10, &aux), SUCCESS);
        let st = status(&mut d);
        assert_ne!(st.active_mask & (1 << 12), 0);
        assert_eq!(st.slot_status[12], SUCCESS);

        let grouped = CsBinding {
            noun: g::CS_NOUN_OUTPUT_MUTE as u8,
            flags: FLAG_GROUP,
            ..button(5)
        };
        assert_eq!(set(&mut d, 5, &grouped), SUCCESS);
        let inputs = CsGroup {
            target_kind: T_INPUT,
            member_mask: 1,
            name: String::new(),
        };
        d.control_out(op::REQ_SET_CS_GROUP, 0, &inputs.encode())
            .unwrap();
        assert_eq!(status(&mut d).slot_status[5], INVALID_GROUP);
        assert_eq!(status(&mut d).active_mask & (1 << 5), 0);
    }

    /// Review w1e's sequence: `control_surfaces_apply_group` re-checks only what
    /// names the edited group, so an unrelated one leaves slot 5 down.
    #[test]
    fn wp_cg16_only_the_edited_groups_bindings_are_rechecked() {
        let mut d = FakeDevice::demo();
        let group = |d: &mut FakeDevice, i: u16, g: &CsGroup| {
            d.control_out(op::REQ_SET_CS_GROUP, i, &g.encode()).unwrap();
            assert_eq!(status(d).last_status, SUCCESS);
        };
        let outputs = CsGroup {
            target_kind: T_OUTPUT,
            member_mask: 0b11,
            name: String::new(),
        };
        let down = |d: &mut FakeDevice, slot: usize| {
            let st = status(d);
            (st.active_mask & (1 << slot) == 0, st.slot_status[slot])
        };
        group(&mut d, 1, &outputs);
        let grouped = CsBinding {
            noun: g::CS_NOUN_OUTPUT_MUTE as u8,
            flags: FLAG_GROUP,
            target: 1,
            ..button(20)
        };
        assert_eq!(set(&mut d, 5, &grouped), SUCCESS);
        group(&mut d, 1, &CsGroup::default());
        assert_eq!(down(&mut d, 5), (true, INVALID_GROUP));
        assert_eq!(set(&mut d, 6, &led(20)), SUCCESS, "its pin was released");
        group(&mut d, 1, &outputs);
        assert_eq!(down(&mut d, 5), (true, PIN_IN_USE));
        assert_eq!(set(&mut d, 6, &CsBinding::default()), SUCCESS);
        group(&mut d, 2, &outputs);
        assert_eq!(down(&mut d, 5), (true, PIN_IN_USE), "another group's edit");
        group(&mut d, 1, &outputs);
        assert_eq!(down(&mut d, 5), (false, SUCCESS), "its own group's edit");
    }

    /// `cs_revalidate_aux_dependents`, the same for an aux slot.
    #[test]
    fn wp_cg16_only_the_edited_aux_slots_controls_are_rechecked() {
        let mut d = FakeDevice::demo();
        let on_slot_10 = binding(&mut d, 12);
        assert_eq!(set(&mut d, 10, &CsBinding::default()), SUCCESS);
        assert_eq!(status(&mut d).slot_status[12], INVALID_AUX);
        let other = CsBinding {
            component: TY_AUX_OUT,
            gpio: [20, GPIO_UNUSED],
            ..Default::default()
        };
        assert_eq!(set(&mut d, 9, &other), SUCCESS);
        let st = status(&mut d);
        assert_eq!(st.slot_status[12], INVALID_AUX, "another slot's edit");
        assert_eq!(st.active_mask & (1 << 12), 0);
        assert_eq!(binding(&mut d, 12), on_slot_10);
        let aux = CsBinding {
            gpio: [27, GPIO_UNUSED],
            ..other
        };
        assert_eq!(set(&mut d, 10, &aux), SUCCESS);
        assert_eq!(status(&mut d).slot_status[12], SUCCESS);
    }

    /// `NUM_OUTPUT_CHANNELS` is 9 (8 S/PDIF and the PDM): the last is a target, on
    /// its own and as a group's member, and the next is not.
    #[test]
    fn wp_cg16_every_output_noun_reaches_the_ninth_output() {
        let seed = surface_seed();
        for noun in [
            g::CS_NOUN_OUTPUT_GAIN,
            g::CS_NOUN_OUTPUT_MUTE,
            g::CS_NOUN_LIMITER,
            g::CS_NOUN_LIMITER_THRESHOLD,
            g::CS_NOUN_LIMITER_RELEASE,
            g::CS_NOUN_LIMITER_LINK,
            g::CS_NOUN_LIMITER_GR,
        ] {
            assert_eq!(seed.nouns[usize::from(noun)].target_count, 9, "noun {noun}");
        }
        let mut d = FakeDevice::demo();
        let mute = CsBinding {
            noun: g::CS_NOUN_OUTPUT_MUTE as u8,
            ..button(5)
        };
        assert_eq!(
            set(
                &mut d,
                5,
                &CsBinding {
                    target: 8,
                    ..mute.clone()
                }
            ),
            SUCCESS
        );
        assert_eq!(
            set(
                &mut d,
                5,
                &CsBinding {
                    target: 9,
                    ..mute.clone()
                }
            ),
            INVALID_TARGET
        );

        let last_only = CsGroup {
            target_kind: T_OUTPUT,
            member_mask: 1 << 8,
            name: String::new(),
        };
        d.control_out(op::REQ_SET_CS_GROUP, 1, &last_only.encode())
            .unwrap();
        assert_eq!(status(&mut d).last_status, SUCCESS);
        let grouped = CsBinding {
            flags: FLAG_GROUP,
            target: 1,
            ..mute
        };
        assert_eq!(set(&mut d, 5, &grouped), SUCCESS);
    }

    /// A revert checks the stored display against the I2C block as it is now
    /// (`cs_validate_display_container`). The control interface can only have
    /// taken that block while the display was off it (`i2c_ctrl_validate`).
    #[test]
    fn wp_cg16_a_revert_checks_the_display_against_the_live_i2c() {
        let mut d = FakeDevice::demo();
        let i2c1 = dspi_proto::packets::I2cCtrlConfig {
            enabled: true,
            sda_pin: 6,
            scl_pin: 7,
            address: 0x42,
        };
        let i2c = |d: &mut FakeDevice| {
            let b = d.control_in(op::REQ_GET_I2C_CONFIG, 0, 8).unwrap();
            dspi_proto::packets::I2cCtrlConfig::decode(&b)
                .unwrap()
                .enabled
        };
        // The display holds I2C1 (GPIO 2 and 3), so the interface is refused.
        assert!(
            d.control_out(op::REQ_SET_I2C_CONFIG, 0, &i2c1.encode())
                .is_err()
        );
        assert!(!i2c(&mut d));

        // Off the block, the interface gets it, and the stored display comes back
        // down, kept in its slot.
        assert_eq!(set(&mut d, 4, &CsBinding::default()), SUCCESS);
        d.control_out(op::REQ_SET_I2C_CONFIG, 0, &i2c1.encode())
            .unwrap();
        assert!(i2c(&mut d));
        d.control_in(op::REQ_CS_REVERT, 0, 1).unwrap();
        let st = status(&mut d);
        assert_eq!(st.active_mask & (1 << 4), 0);
        assert_eq!(st.slot_status[4], I2C_IN_USE);
        assert_eq!(binding(&mut d, 4).component, TY_DISPLAY);
    }

    #[test]
    fn wp_cg16_save_keeps_and_revert_restores_the_store() {
        let mut d = FakeDevice::demo();
        let original = binding(&mut d, 1);
        assert_eq!(set(&mut d, 1, &button(5)), SUCCESS);
        d.control_out(op::REQ_SET_CS_NAME, 1, b"Mine").unwrap();
        assert!(status(&mut d).dirty);
        assert_eq!(d.control_in(op::REQ_CS_REVERT, 0, 1).unwrap(), [1]);
        let st = status(&mut d);
        assert_eq!(
            (st.last_status, st.last_slot, st.dirty),
            (SUCCESS, TAG_SAVE, false)
        );
        assert_eq!(binding(&mut d, 1), original);
        assert_eq!(d.control_in(op::REQ_GET_CS_NAME, 1, 32).unwrap(), [0; 32]);

        assert_eq!(set(&mut d, 1, &button(5)), SUCCESS);
        assert_eq!(d.control_in(op::REQ_CS_SAVE, 0, 1).unwrap(), [1]);
        let st = status(&mut d);
        assert_eq!(
            (st.last_status, st.last_slot, st.dirty),
            (SUCCESS, TAG_SAVE, false)
        );
        assert_eq!(set(&mut d, 1, &button(6)), SUCCESS);
        d.control_in(op::REQ_CS_REVERT, 0, 1).unwrap();
        assert_eq!(binding(&mut d, 1).gpio[0], 5, "the saved one");
    }

    #[test]
    fn wp_cg16_aux_outputs_switch_live_and_notify_once() {
        let mut d = FakeDevice::demo();
        let all = d.control_in(op::REQ_GET_CS_AUX_STATE, 0xFFFF, 48).unwrap();
        let all = CsAuxStates::decode(&all).unwrap();
        assert_eq!(
            (all.state[10], all.state[11], all.level_q8[11]),
            (0, 1, 12800),
            "boot fields"
        );
        let mut events = d.notifications().unwrap();
        d.control_out(op::REQ_SET_CS_AUX_STATE, 10, &[1]).unwrap();
        d.control_out(op::REQ_SET_CS_AUX_STATE, 10, &[1]).unwrap();
        let p = events.read(std::time::Duration::ZERO).unwrap();
        assert_eq!(
            p,
            [
                2,
                nfy::NOTIFY_EVT_CS_AUX as u8,
                0,
                p[3],
                10,
                1,
                0,
                0,
                nfy::PARAM_SRC_HOST_SET as u8
            ]
        );
        assert!(
            events.read(std::time::Duration::ZERO).unwrap().is_empty(),
            "unchanged is silent"
        );
        assert_eq!(d.control_in(op::REQ_GET_CS_AUX_STATE, 10, 1).unwrap(), [1]);
        assert!(!status(&mut d).dirty, "no preview");

        // A level clamps at 100 %, and takes only on a dimmable output.
        d.control_out(op::REQ_SET_CS_AUX_LEVEL, 11, &u16::MAX.to_le_bytes())
            .unwrap();
        assert_eq!(
            d.control_in(op::REQ_GET_CS_AUX_LEVEL, 11, 2).unwrap(),
            25600u16.to_le_bytes()
        );
        for (opcode, slot, data) in [
            (op::REQ_SET_CS_AUX_LEVEL, 10, &[0u8, 1][..]),
            (op::REQ_SET_CS_AUX_STATE, 0, &[1][..]),
        ] {
            d.control_out(opcode, slot, data).unwrap();
            let st = status(&mut d);
            assert_eq!((st.last_status, st.last_slot), (INVALID_AUX, slot as u8));
        }
    }

    #[test]
    fn wp_cg16_a_fired_macro_runs_and_ends_for_whoever_polls() {
        let mut d = FakeDevice::demo();
        let ext = |d: &mut FakeDevice| {
            let b = d.control_in(op::REQ_GET_CS_EXT_STATUS, 0, 24).unwrap();
            let p = CsExtStatusPacket::decode(&b).unwrap();
            (p.macro_running, p.macro_step)
        };
        assert_eq!(ext(&mut d), (MACRO_NONE, 0));
        assert_eq!(d.control_in(op::REQ_CS_MACRO_FIRE, 0, 1).unwrap(), [1]);
        assert_eq!(ext(&mut d), (0, 0));
        assert_eq!(ext(&mut d), (0, 1));
        assert_eq!(ext(&mut d), (MACRO_NONE, 0));
        // An empty macro is a successful no-op; cancel always answers.
        assert_eq!(d.control_in(op::REQ_CS_MACRO_FIRE, 3, 1).unwrap(), [1]);
        assert_eq!(ext(&mut d), (MACRO_NONE, 0));
        d.control_in(op::REQ_CS_MACRO_FIRE, 0, 1).unwrap();
        assert_eq!(d.control_in(op::REQ_CS_MACRO_FIRE, 0xFFFF, 1).unwrap(), [1]);
        assert_eq!(ext(&mut d), (MACRO_NONE, 0));
        assert!(stalled(d.control_in(op::REQ_CS_MACRO_FIRE, 8, 1)));
    }

    #[test]
    fn wp_cg16_the_display_cycles_for_whoever_polls() {
        let mut d = FakeDevice::demo();
        let shown = |d: &mut FakeDevice| {
            let b = d.control_in(op::REQ_GET_CS_DISPLAY_STATUS, 0, 8).unwrap();
            let s = CsDisplayStatus::decode(&b).unwrap();
            (s.init_state, s.current_page, s.model)
        };
        assert_eq!(shown(&mut d), (DISPLAY_LIVE, 0, 6));
        assert_eq!(shown(&mut d), (DISPLAY_LIVE, 1, 6));
        assert_eq!(shown(&mut d), (DISPLAY_LIVE, 0, 6));
        assert_eq!(set(&mut d, 4, &CsBinding::default()), SUCCESS);
        assert_eq!(shown(&mut d).0, 0);
    }

    #[test]
    fn wp_cg16_learning_needs_a_receiver_and_ends_with_the_press() {
        let mut d = FakeDevice::demo();
        let remote = d.remote();
        assert!(!remote.cs_learned(1, 5), "nothing armed");
        let read = |d: &mut FakeDevice| d.control_in(op::REQ_CS_IR_LEARN, 2, 8).unwrap();
        assert_eq!(d.control_in(op::REQ_CS_IR_LEARN, 1, 1).unwrap(), [1]);
        assert_eq!(read(&mut d)[0], g::CS_IR_LEARN_ARMED as u8);
        assert_eq!(status(&mut d).ir_learn_state, g::CS_IR_LEARN_ARMED as u8);
        let mut events = d.notifications().unwrap();
        assert!(remote.cs_learned(1, 0x0102_0304));
        assert_eq!(
            read(&mut d),
            [g::CS_IR_LEARN_DONE as u8, 1, 0, 0, 4, 3, 2, 1]
        );
        let p = events.read(std::time::Duration::ZERO).unwrap();
        assert_eq!(p[1], nfy::NOTIFY_EVT_CS_IR_LEARN as u8);
        assert_eq!(p[4..], [g::CS_IR_LEARN_DONE as u8, 1, 0, 0, 4, 3, 2, 1]);
        assert_eq!(d.control_in(op::REQ_CS_IR_LEARN, 0, 1).unwrap(), [1]);
        assert_eq!(read(&mut d)[0], g::CS_IR_LEARN_IDLE as u8);

        assert_eq!(set(&mut d, 3, &CsBinding::default()), SUCCESS);
        assert!(stalled(d.control_in(op::REQ_CS_IR_LEARN, 1, 1)));
        let st = status(&mut d);
        assert_eq!((st.last_status, st.last_slot), (NO_IR, TAG_SAVE));
    }

    /// `EXT_FAKE_CS_SIM`: every refusal a revert in progress gives.
    #[test]
    fn wp_cg16_a_held_revert_refuses_as_the_firmware_does() {
        let mut d = FakeDevice::demo();
        let original = binding(&mut d, 1);
        // Stock firmware does not have it.
        assert!(stalled(FakeDevice::demo_without_surfaces().control_out(
            op::REQ_EXTENSION,
            EXT_FAKE_CS_SIM,
            &[1]
        )));
        d.control_out(op::REQ_EXTENSION, EXT_FAKE_CS_SIM, &[1])
            .unwrap();

        assert_eq!(set(&mut d, 1, &button(5)), BUSY);
        assert_eq!(binding(&mut d, 1), original);
        d.control_out(op::REQ_SET_CS_NAME, 1, b"x").unwrap();
        assert_eq!(status(&mut d).last_status, BUSY);
        d.control_out(op::REQ_SET_CS_GROUP, 1, &[0; 40]).unwrap();
        assert_eq!(status(&mut d).last_status, BUSY);
        assert!(!status(&mut d).dirty);
        // Stalls, the status untouched: what it read before is still there.
        assert!(stalled(d.control_in(op::REQ_CS_MACRO_FIRE, 0, 1)));
        assert!(stalled(d.control_in(op::REQ_CS_IR_LEARN, 1, 1)));
        assert!(stalled(d.control_in(op::REQ_CS_IR_LEARN, 0, 1)));
        // An aux SET changes nothing and reports nothing.
        d.control_out(op::REQ_SET_CS_AUX_STATE, 10, &[1]).unwrap();
        assert_eq!(d.control_in(op::REQ_GET_CS_AUX_STATE, 10, 1).unwrap(), [0]);
        // Save and revert stall with the reason.
        assert!(stalled(d.control_in(op::REQ_CS_SAVE, 0, 1)));
        assert_eq!(status(&mut d).last_status, BUSY);
        assert!(stalled(d.control_in(op::REQ_CS_REVERT, 0, 1)));
        // Reads answer, and a cancel still works.
        assert_eq!(d.control_in(op::REQ_CS_MACRO_FIRE, 0xFFFF, 1).unwrap(), [1]);

        d.control_out(op::REQ_EXTENSION, EXT_FAKE_CS_SIM, &[0])
            .unwrap();
        assert_eq!(set(&mut d, 1, &button(5)), SUCCESS);
    }

    /// The firmware's CS opcodes, every one answered or stalled by the model and
    /// none left to the rest of the fake.
    #[test]
    fn wp_cg16_every_cs_read_is_answered() {
        let mut d = FakeDevice::demo();
        for (opcode, value, len) in [
            (op::REQ_GET_CS_CAPS, 0xFFFF, 52),
            (op::REQ_GET_CS_CAPS, 0, 12),
            (op::REQ_GET_CS_BINDING, 0, 24),
            (op::REQ_GET_CS_STATUS, 0, 41),
            (op::REQ_GET_CS_NAME, 0, 32),
            (op::REQ_GET_CS_IR_CMD, 0, 16),
            (op::REQ_GET_CS_AUX_STATE, 0xFFFF, 48),
            (op::REQ_GET_CS_AUX_STATE, 10, 1),
            (op::REQ_GET_CS_AUX_LEVEL, 11, 2),
            (op::REQ_GET_CS_GROUP, 0, 40),
            (op::REQ_GET_CS_MACRO, 0, 132),
            (op::REQ_GET_CS_EXT_STATUS, 0, 24),
            (op::REQ_GET_CS_DISPLAY_CFG, 0, 16),
            (op::REQ_GET_CS_DISPLAY_PAGE, 0, 4),
            (op::REQ_GET_CS_DISPLAY_STATUS, 0, 8),
            (op::REQ_CS_IR_LEARN, 2, 8),
        ] {
            assert_eq!(
                d.control_in(opcode, value, len).unwrap().len(),
                usize::from(len),
                "0x{opcode:02X}"
            );
        }
    }
}
