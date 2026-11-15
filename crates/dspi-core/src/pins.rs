//! The pin map (WP CG-SWS-b): which GPIOs a function may take on this
//! platform, which function holds each pin, and whether a move would be
//! accepted, from the mirror (`DspState`) and the live control-surface
//! bindings ([`DspiCore::cs_pin_uses`]).
//!
//! Before this, every client had its own copy of these rules (the Mac's
//! `DSPViewModel.pinAssignment` and `HardwareSettingsTab.validPins` /
//! `i2sAllowedFor` / `mckValidPins`, the Qt `PinMap` and `heldBy`, Android's
//! `PinAssignments.kt`), and the copies had drifted. The rules here are the
//! firmware's (Bofr5555/DSPi `vendor_commands.c`: `is_valid_gpio_pin`,
//! `is_pin_in_use`, `check_i2s_rx_pin`, `ctrl_iface_check_pin` and the
//! `REQ_SET_*_PIN` handlers), with these choices where the copies differed:
//!
//! - **Valid GPIOs per platform**: 0-22 and 26-28 on an RP2040, 26-29 on an
//!   RP2350, as `is_valid_gpio_pin`. Every copy left 29 out on both parts.
//!   The table is `dspi_session::pins::valid_pins`, one list for the TUI and
//!   the core.
//! - **The I2S clock pair is always held** (BCK and LRCLK = BCK + 1, and the
//!   slave pair in split clock-pin mode), by every copy and by the firmware's
//!   checks for the long-lived claims (`check_i2s_rx_pin`,
//!   `ctrl_iface_check_pin`). The firmware's `is_pin_in_use` lets an output,
//!   S/PDIF input, ADAT or MCK pin onto a dormant pair (no I2S output, input
//!   not I2S); switching an output slot to I2S does not re-check, so the two
//!   would then drive one pad. The core refuses that, as the copies do. The
//!   one place the dormant pair is told apart is moving the pair itself: its
//!   own pins block it only while the clocks run, as in the firmware.
//! - **A disabled feature holds nothing**: an optional S/PDIF input, ADAT
//!   out or in, MCK, the DAC mute, a control interface that is not live, a
//!   control-surface binding that is not live; I2S RX holds only the pairs the
//!   channel count uses.
//! - **The ADAT input may share the ADAT output's pin** (the loopback
//!   self-test); the exception is one-directional, as in the firmware.
//! - **MCK** only on the clock-output pins: 21 on an RP2040, 13, 15 or 21 on
//!   an RP2350; not movable while enabled (`OUTPUT_ACTIVE`).
//! - **UART and I2C** pins on their mux positions (TX `pin % 4 == 0`, RX
//!   `% 4 == 1`, SDA even, SCL odd); the pair on one instance is checked by
//!   the setters, which send both pins at once.
//! - **Labels**: the Console's owner names ("Output 1", "Subwoofer",
//!   "I2S LRCLK", "S/PDIF 2 RX", ...). An input number appears only when
//!   more than one of its kind is in play (S/PDIF: an optional input enabled;
//!   I2S RX: more than one active pair), as the session and Qt copies do;
//!   the Mac numbered I2S RX by the platform's pair count.
//! - **A no-op is always accepted**: the pin a function already has.
//! - **The STM32H723** (platform 2) has no assignable pins: its SAI clocks
//!   and data pins are fixed. Its slot rule is in [`PinFunctionInfo::i2s_allowed`]:
//!   slot 2 or 3 (index 1, 2) may be I2S only while slot 1 (index 0) is,
//!   slot 4 (index 3) only while slot 3 (index 2) is. A platform this build
//!   does not know is treated the same way (nothing assignable).
//!
//! Not modelled (the device still decides): the PDM output's
//! `OUTPUT_ACTIVE` while PDM runs, a pending I2S clock-mode change, the
//! control-surface display's I2C instance, the audio link's RX and LOS pins
//! (`alink_input_owns_pin`), and a test build's scope-marker pins.
//!
//! Every pin setter goes through [`pin_check`] first when the core knows the
//! platform's rules and has read the device's configuration: a refusal is
//! [`DspiError::Refused`] with the status the firmware would answer, and
//! nothing is sent. `PIN_RESET_TO_DEFAULT` (0xFF) passes, since the device
//! picks the pin.

use dspi_proto::Platform;
use dspi_proto::generated::status as st;
use dspi_session::pins as tables;

use crate::records::{DspState, PinUse};
use crate::{DspiCore, DspiError, Result};

/// No GPIO (the ADAT input's and the DAC mute's "not set").
pub const PIN_NONE: u8 = 0xFF;

const PLATFORM_STM32H723: u8 = 2;
const OUTPUT_TYPE_I2S: u8 = 1;
const INPUT_SOURCE_I2S: u8 = 2;
const I2S_CLOCK_MODE_SLAVE: u8 = 1;
const CLOCK_PIN_MODE_SPLIT: u8 = 1;
/// `SPDIF_RX_NUM_INPUTS` (audio_input.h), both platforms.
const SPDIF_INPUTS: u8 = 4;

const INVALID_PIN: u8 = st::PIN_CONFIG_INVALID_PIN as u8;
const IN_USE: u8 = st::PIN_CONFIG_PIN_IN_USE as u8;
const INVALID_OUTPUT: u8 = st::PIN_CONFIG_INVALID_OUTPUT as u8;
const OUTPUT_ACTIVE: u8 = st::PIN_CONFIG_OUTPUT_ACTIVE as u8;

// ------------------------------------------------------------------ types

/// A function that can hold a GPIO. The C ABI passes it as its code (the
/// number in brackets).
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PinFunctionKind {
    /// (0) A physical output, `index` 0..`num_pin_outputs`: the S/PDIF or
    /// I2S slots, then the PDM subwoofer last.
    Output,
    /// (1) The I2S bit clock of the master (or unified) pair; also holds
    /// LRCLK, BCK + 1.
    I2sBck,
    /// (2) The slave pair's bit clock (split clock-pin mode); also its LRCLK.
    I2sBckSlave,
    /// (3) The I2S master clock.
    Mck,
    /// (4) An S/PDIF receiver, `index` 0..3.
    SpdifRx,
    /// (5) An I2S input data pin, `index` = stereo pair 0..3.
    I2sRx,
    /// (6) The ADAT output (RP2350).
    AdatOut,
    /// (7) The ADAT input (RP2350).
    AdatIn,
    /// (8) The DAC hardware mute line.
    DacMute,
    /// (9) The UART control interface's TX.
    UartTx,
    /// (10) Its RX.
    UartRx,
    /// (11) The I2C control interface's SDA.
    I2cSda,
    /// (12) Its SCL.
    I2cScl,
    /// (13) A control-surface binding, `index` = its slot.
    ControlSurface,
}

impl PinFunctionKind {
    const ALL: [PinFunctionKind; 14] = [
        Self::Output,
        Self::I2sBck,
        Self::I2sBckSlave,
        Self::Mck,
        Self::SpdifRx,
        Self::I2sRx,
        Self::AdatOut,
        Self::AdatIn,
        Self::DacMute,
        Self::UartTx,
        Self::UartRx,
        Self::I2cSda,
        Self::I2cScl,
        Self::ControlSurface,
    ];

    /// The C ABI's code (the order above).
    pub fn code(self) -> u8 {
        Self::ALL
            .iter()
            .position(|k| *k == self)
            .and_then(|i| u8::try_from(i).ok())
            .unwrap_or(u8::MAX)
    }

    /// The kind with that code, `None` for another number.
    pub fn from_code(code: u8) -> Option<Self> {
        Self::ALL.get(usize::from(code)).copied()
    }
}

/// One function: its kind and, for the indexed kinds, which one.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PinFunction {
    /// What it is.
    pub kind: PinFunctionKind,
    /// Which one (output, input, pair or slot); 0 for the single ones.
    #[serde(default)]
    pub index: u8,
}

impl PinFunction {
    /// `kind` number `index`.
    pub fn new(kind: PinFunctionKind, index: u8) -> Self {
        Self { kind, index }
    }

    fn one(kind: PinFunctionKind) -> Self {
        Self { kind, index: 0 }
    }

    /// Whether `self` and `other` are the same owner: the same function, or
    /// the two pins of one control interface.
    fn same_owner(self, other: PinFunction) -> bool {
        use PinFunctionKind as K;
        self == other
            || matches!(
                (self.kind, other.kind),
                (K::UartTx | K::UartRx, K::UartTx | K::UartRx)
                    | (K::I2cSda | K::I2cScl, K::I2cSda | K::I2cScl)
            )
    }
}

/// What a held GPIO is doing, for grouping (the Console's five roles).
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum PinHolderRole {
    /// A physical or ADAT output.
    Output,
    /// An I2S clock.
    Clock,
    /// An audio input.
    Input,
    /// A control interface or control surface.
    Control,
    /// The DAC mute.
    Utility,
}

/// One GPIO and the function holding it.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PinHolder {
    /// The GPIO.
    pub pin: u8,
    /// The function holding it (BCK's also holds LRCLK, BCK + 1).
    pub function: PinFunction,
    /// For grouping.
    pub role: PinHolderRole,
    /// The Console's owner name, e.g. "Output 1", "I2S LRCLK", "UART
    /// Control" (English; `function` is there to translate from).
    pub label: String,
}

/// One function of this device: its pin, whether it holds it, and the pins
/// it may move to.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PinFunctionInfo {
    /// Which function.
    pub function: PinFunction,
    /// Its name, e.g. "Output 1", "S/PDIF 2 RX", "UART TX".
    pub label: String,
    /// Its GPIO, [`PIN_NONE`] when it has none.
    pub pin: u8,
    /// Whether it holds that pin now (a disabled feature holds nothing).
    pub holds: bool,
    /// The GPIOs [`pin_check`] accepts for it, its own included, in order.
    /// For a disabled optional S/PDIF input, without the pins another
    /// function holds: it could store one, but never be enabled on it.
    /// Empty where nothing is assignable.
    pub choices: Vec<u8>,
    /// Output slots: whether the slot may be switched to I2S (the STM32H723's
    /// SAI rule; always on an RP2040/RP2350). False for everything else.
    pub i2s_allowed: bool,
}

/// The whole map.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PinMap {
    /// The platform the rules are for (`DspState.platform_id`).
    pub platform_id: u8,
    /// Whether this platform has assignable pins (RP2040 and RP2350).
    pub assignable: bool,
    /// The platform's valid GPIOs, in order; empty where nothing is
    /// assignable.
    pub valid_pins: Vec<u8>,
    /// The functions this device has, in the Console's order.
    pub functions: Vec<PinFunctionInfo>,
    /// Who holds each held GPIO, in pin order; the first claim wins where
    /// two share one (the ADAT loopback shows as "ADAT Output").
    pub holders: Vec<PinHolder>,
}

/// Why a pin was refused.
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum PinRefusalReason {
    /// This platform's pins are fixed (STM32H723, or a platform this build
    /// does not know).
    NotAssignable,
    /// The device has no such function (an output, input or pair past its
    /// count; ADAT on an RP2040; a feature its firmware lacks).
    NoSuchFunction,
    /// Not a GPIO this platform allows (23-25 are power and LED; BCK also
    /// needs BCK + 1).
    InvalidPin,
    /// A GPIO, but not one this function's peripheral can use: MCK off a
    /// clock-output pin, UART or I2C off its mux position or instance.
    WrongMux,
    /// Another function holds it: see `holder`.
    InUse,
    /// The function cannot move now: MCK while enabled, the I2S clock pair
    /// while I2S outputs run on it.
    OutputActive,
    /// The ADAT input's pin cannot be cleared while the input is enabled.
    ClearWhileEnabled,
}

/// A refused pin.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PinRefusal {
    /// Why.
    pub reason: PinRefusalReason,
    /// The `PIN_CONFIG_*` status the firmware answers for it (1 invalid
    /// pin, 2 in use, 3 no such output, 4 output active).
    pub status: u8,
    /// For [`PinRefusalReason::InUse`], the function holding the pin.
    pub holder: Option<PinHolder>,
}

impl PinRefusal {
    fn new(reason: PinRefusalReason, status: u8) -> Self {
        Self {
            reason,
            status,
            holder: None,
        }
    }

    fn in_use(holder: PinHolder) -> Self {
        Self {
            reason: PinRefusalReason::InUse,
            status: IN_USE,
            holder: Some(holder),
        }
    }
}

// ------------------------------------------------------------- the rules

/// The view of the mirror the rules read.
struct Ctx<'a> {
    s: &'a DspState,
    platform: Platform,
    valid: Vec<u8>,
    /// Every claim, duplicates kept (the ADAT loopback holds one pin twice),
    /// in the Console's order.
    claims: Vec<PinHolder>,
}

fn assignable(platform_id: u8) -> bool {
    matches!(
        Platform::from_id(platform_id),
        Platform::Rp2040 | Platform::Rp2350
    )
}

/// `I2S_RX_MAX_PAIRS`: 4 on an RP2350, 1 on an RP2040.
fn max_pairs(platform: Platform) -> u8 {
    if platform == Platform::Rp2350 { 4 } else { 1 }
}

fn at(v: &[u8], i: u8) -> Option<u8> {
    v.get(usize::from(i)).copied()
}

impl<'a> Ctx<'a> {
    fn new(s: &'a DspState, uses: &[PinUse]) -> Self {
        let platform = Platform::from_id(s.platform_id);
        let mut cx = Ctx {
            s,
            platform,
            valid: if assignable(s.platform_id) {
                tables::valid_pins(platform)
            } else {
                Vec::new()
            },
            claims: Vec::new(),
        };
        if !cx.valid.is_empty() {
            cx.claims = cx.build_claims(uses);
        }
        cx
    }

    fn valid(&self, pin: u8) -> bool {
        self.valid.contains(&pin)
    }

    /// Physical outputs: the slots, then the PDM sub.
    fn num_outputs(&self) -> u8 {
        let n = u8::try_from(self.s.output_pins.len()).unwrap_or(u8::MAX);
        self.s.num_pin_outputs.min(n)
    }

    fn num_slots(&self) -> u8 {
        self.num_outputs().saturating_sub(1)
    }

    fn any_slot_i2s(&self) -> bool {
        self.s
            .output_types
            .iter()
            .take(usize::from(self.num_slots()))
            .any(|t| *t == OUTPUT_TYPE_I2S)
    }

    /// The firmware's `i2s_clocks_in_use`: an I2S output, or I2S the input.
    fn clocks_running(&self) -> bool {
        self.any_slot_i2s() || self.s.input_source == INPUT_SOURCE_I2S
    }

    fn split(&self) -> bool {
        self.s.i2s_clock_pin_mode == CLOCK_PIN_MODE_SPLIT
    }

    fn spdif_inputs(&self) -> u8 {
        let n = u8::try_from(self.s.spdif_rx_pins.len()).unwrap_or(u8::MAX);
        if self.s.spdif_inputs_enabled_present {
            n.min(SPDIF_INPUTS)
        } else {
            n.min(1)
        }
    }

    /// Input 0 is always on; bit `i` of `spdif_inputs_enabled` is input `i`.
    fn spdif_enabled(&self, i: u8) -> bool {
        i == 0
            || self
                .s
                .spdif_inputs_enabled
                .checked_shr(u32::from(i))
                .unwrap_or(0)
                & 1
                != 0
    }

    fn active_pairs(&self) -> u8 {
        (self.s.i2s_input_channels.max(2) / 2).clamp(1, max_pairs(self.platform))
    }

    fn is_rp2350(&self) -> bool {
        self.platform == Platform::Rp2350
    }

    fn build_claims(&self, uses: &[PinUse]) -> Vec<PinHolder> {
        use PinFunctionKind as K;
        use PinHolderRole as R;
        let s = self.s;
        let mut out = Vec::new();
        let mut claim = |pin: u8, f: PinFunction, role: R, label: String| {
            if self.valid(pin) {
                out.push(PinHolder {
                    pin,
                    function: f,
                    role,
                    label,
                });
            }
        };
        let slots = self.num_slots();
        for i in 0..self.num_outputs() {
            if let Some(pin) = at(&s.output_pins, i) {
                claim(
                    pin,
                    PinFunction::new(K::Output, i),
                    R::Output,
                    output_label(i, slots),
                );
            }
        }
        let bck = PinFunction::one(K::I2sBck);
        claim(s.i2s_bck_pin, bck, R::Clock, "I2S BCK".into());
        claim(
            s.i2s_bck_pin.wrapping_add(1),
            bck,
            R::Clock,
            "I2S LRCLK".into(),
        );
        if self.split() {
            let slave = PinFunction::one(K::I2sBckSlave);
            claim(s.i2s_bck_pin_slave, slave, R::Clock, "I2S Slave BCK".into());
            claim(
                s.i2s_bck_pin_slave.wrapping_add(1),
                slave,
                R::Clock,
                "I2S Slave LRCLK".into(),
            );
        }
        if s.mck_enabled {
            claim(
                s.mck_pin,
                PinFunction::one(K::Mck),
                R::Clock,
                "I2S MCK".into(),
            );
        }
        for i in 0..self.spdif_inputs() {
            if self.spdif_enabled(i)
                && let Some(pin) = at(&s.spdif_rx_pins, i)
            {
                claim(
                    pin,
                    PinFunction::new(K::SpdifRx, i),
                    R::Input,
                    self.spdif_label(i),
                );
            }
        }
        for p in 0..self.active_pairs() {
            if let Some(pin) = at(&s.i2s_rx_pins, p) {
                claim(
                    pin,
                    PinFunction::new(K::I2sRx, p),
                    R::Input,
                    self.i2s_rx_label(p),
                );
            }
        }
        if s.dac_mute_enabled && s.dac_mute_pin != PIN_NONE {
            claim(
                s.dac_mute_pin,
                PinFunction::one(K::DacMute),
                R::Utility,
                "DAC Mute".into(),
            );
        }
        if self.is_rp2350() && s.adat_out_enabled {
            claim(
                s.adat_out_pin,
                PinFunction::one(K::AdatOut),
                R::Output,
                "ADAT Output".into(),
            );
        }
        if self.is_rp2350() && s.adat_input_enabled && s.adat_input_pin != PIN_NONE {
            claim(
                s.adat_input_pin,
                PinFunction::one(K::AdatIn),
                R::Input,
                "ADAT Input".into(),
            );
        }
        if s.ctrl_status.uart_live {
            claim(
                s.uart.tx_pin,
                PinFunction::one(K::UartTx),
                R::Control,
                "UART Control".into(),
            );
            claim(
                s.uart.rx_pin,
                PinFunction::one(K::UartRx),
                R::Control,
                "UART Control".into(),
            );
        }
        if s.ctrl_status.i2c_live {
            claim(
                s.i2c.sda_pin,
                PinFunction::one(K::I2cSda),
                R::Control,
                "I2C Control".into(),
            );
            claim(
                s.i2c.scl_pin,
                PinFunction::one(K::I2cScl),
                R::Control,
                "I2C Control".into(),
            );
        }
        for u in uses {
            claim(
                u.pin,
                PinFunction::new(K::ControlSurface, u.slot),
                R::Control,
                format!("Control Surface {}", u.slot.saturating_add(1)),
            );
        }
        out
    }

    fn spdif_label(&self, i: u8) -> String {
        let any_optional = (1..self.spdif_inputs()).any(|k| self.spdif_enabled(k));
        if i == 0 && !any_optional {
            "S/PDIF RX".into()
        } else {
            format!("S/PDIF {} RX", i.saturating_add(1))
        }
    }

    fn i2s_rx_label(&self, pair: u8) -> String {
        if self.active_pairs() > 1 {
            format!("I2S RX {}", pair.saturating_add(1))
        } else {
            "I2S RX".into()
        }
    }

    /// The first claim on `pin` by anyone but `excluding`'s owner.
    fn held_by(&self, pin: u8, excluding: Option<PinFunction>) -> Option<&PinHolder> {
        self.claims
            .iter()
            .find(|c| c.pin == pin && excluding.is_none_or(|e| !e.same_owner(c.function)))
    }

    /// The function's pin now; `None` when the device has no such function.
    fn current(&self, f: PinFunction) -> Option<u8> {
        use PinFunctionKind as K;
        let s = self.s;
        match f.kind {
            K::Output => (f.index < self.num_outputs())
                .then(|| at(&s.output_pins, f.index))
                .flatten(),
            K::I2sBck => (f.index == 0).then_some(s.i2s_bck_pin),
            K::I2sBckSlave => (f.index == 0).then_some(s.i2s_bck_pin_slave),
            K::Mck => (f.index == 0).then_some(s.mck_pin),
            K::SpdifRx => (f.index < SPDIF_INPUTS)
                .then(|| at(&s.spdif_rx_pins, f.index))
                .flatten(),
            K::I2sRx => (f.index < max_pairs(self.platform))
                .then(|| at(&s.i2s_rx_pins, f.index))
                .flatten(),
            K::AdatOut => (f.index == 0 && self.is_rp2350()).then_some(s.adat_out_pin),
            K::AdatIn => (f.index == 0 && self.is_rp2350()).then_some(s.adat_input_pin),
            K::DacMute => (f.index == 0 && s.dac_mute_supported).then_some(s.dac_mute_pin),
            K::UartTx => (f.index == 0 && s.ctrl_iface_supported).then_some(s.uart.tx_pin),
            K::UartRx => (f.index == 0 && s.ctrl_iface_supported).then_some(s.uart.rx_pin),
            K::I2cSda => (f.index == 0 && s.ctrl_iface_supported).then_some(s.i2c.sda_pin),
            K::I2cScl => (f.index == 0 && s.ctrl_iface_supported).then_some(s.i2c.scl_pin),
            // A binding's pins are the control-surface rules' (cs_rules); here
            // only "is it free", with no pin of its own to keep.
            K::ControlSurface => Some(PIN_NONE),
        }
    }

    fn check(&self, f: PinFunction, pin: u8) -> std::result::Result<(), PinRefusal> {
        use PinFunctionKind as K;
        use PinRefusalReason as Why;
        if self.valid.is_empty() {
            return Err(PinRefusal::new(Why::NotAssignable, INVALID_PIN));
        }
        let Some(current) = self.current(f) else {
            return Err(PinRefusal::new(Why::NoSuchFunction, INVALID_OUTPUT));
        };
        let s = self.s;
        // The ADAT input's "clear".
        if f.kind == K::AdatIn && pin == PIN_NONE {
            return if s.adat_input_enabled {
                Err(PinRefusal::new(Why::ClearWhileEnabled, IN_USE))
            } else {
                Ok(())
            };
        }
        if f.kind == K::DacMute && pin == PIN_NONE {
            return Ok(());
        }
        if pin == current && f.kind != K::ControlSurface {
            return Ok(());
        }
        let in_use = |p: u8, excl: Option<PinFunction>| match self.held_by(p, excl) {
            Some(h) => Err(PinRefusal::in_use(h.clone())),
            None => Ok(()),
        };
        if !self.valid(pin) {
            return Err(PinRefusal::new(Why::InvalidPin, INVALID_PIN));
        }
        match f.kind {
            K::Output | K::AdatOut | K::DacMute | K::ControlSurface | K::I2sRx => {
                in_use(pin, Some(f))
            }
            K::SpdifRx => {
                // A disabled optional input stores its pin as a preference;
                // conflicts are checked when it is enabled.
                if self.spdif_enabled(f.index) {
                    in_use(pin, Some(f))
                } else {
                    Ok(())
                }
            }
            K::AdatIn => {
                if pin == s.adat_out_pin {
                    Ok(()) // the loopback self-test
                } else {
                    in_use(pin, Some(f))
                }
            }
            K::Mck => {
                if !tables::mck_pins(self.platform).contains(&pin) {
                    Err(PinRefusal::new(Why::WrongMux, INVALID_PIN))
                } else if s.mck_enabled {
                    Err(PinRefusal::new(Why::OutputActive, OUTPUT_ACTIVE))
                } else {
                    in_use(pin, Some(f))
                }
            }
            K::UartTx | K::UartRx | K::I2cSda | K::I2cScl => {
                let mux = match f.kind {
                    K::UartTx => pin.is_multiple_of(4),
                    K::UartRx => pin % 4 == 1,
                    K::I2cSda => pin.is_multiple_of(2),
                    _ => pin % 2 == 1,
                };
                if mux {
                    in_use(pin, Some(f))
                } else {
                    Err(PinRefusal::new(Why::WrongMux, INVALID_PIN))
                }
            }
            K::I2sBck | K::I2sBckSlave => self.check_bck(f, pin),
        }
    }

    /// `REQ_SET_I2S_BCK_PIN`: role 0 the master/unified pair, 1 the slave.
    fn check_bck(&self, f: PinFunction, pin: u8) -> std::result::Result<(), PinRefusal> {
        use PinRefusalReason as Why;
        let s = self.s;
        let slave = f.kind == PinFunctionKind::I2sBckSlave;
        let lrck = pin.wrapping_add(1);
        if !self.valid(lrck) {
            return Err(PinRefusal::new(Why::InvalidPin, INVALID_PIN));
        }
        // Moving the pair under I2S outputs that run on it.
        let split_slave = self.split() && s.i2s_clock_mode == I2S_CLOCK_MODE_SLAVE;
        if self.any_slot_i2s() && slave == split_slave {
            return Err(PinRefusal::new(Why::OutputActive, OUTPUT_ACTIVE));
        }
        // The two pairs may not overlap; the master pair checks against the
        // slave pair only in split mode.
        let other = if slave {
            s.i2s_bck_pin
        } else {
            s.i2s_bck_pin_slave
        };
        let overlaps = pin == other || pin == other.wrapping_add(1) || lrck == other;
        if overlaps && (slave || self.split()) {
            let (kind, label) = if slave {
                (PinFunctionKind::I2sBck, "I2S BCK")
            } else {
                (PinFunctionKind::I2sBckSlave, "I2S Slave BCK")
            };
            return Err(PinRefusal::in_use(PinHolder {
                pin: other,
                function: PinFunction::one(kind),
                role: PinHolderRole::Clock,
                label: label.into(),
            }));
        }
        // Its own pins block it only while the clocks run on them (the
        // firmware's `is_pin_in_use` without exclusion).
        let own_live = self.clocks_running() && (!slave || self.split());
        let excl = if own_live { None } else { Some(f) };
        for p in [pin, lrck] {
            if let Some(h) = self.held_by(p, excl) {
                return Err(PinRefusal::in_use(h.clone()));
            }
        }
        Ok(())
    }

    /// The STM32H723's SAI rule for switching output slot `slot` to I2S.
    fn i2s_allowed(&self, slot: u8) -> bool {
        if self.s.platform_id != PLATFORM_STM32H723 {
            return true;
        }
        let is_i2s = |k: u8| at(&self.s.output_types, k) == Some(OUTPUT_TYPE_I2S);
        match slot {
            1 | 2 => is_i2s(0),
            3 => is_i2s(2),
            _ => true,
        }
    }

    fn info(&self, f: PinFunction, label: String) -> PinFunctionInfo {
        let pin = self.current(f).unwrap_or(PIN_NONE);
        let holds = self.claims.iter().any(|c| c.function == f && c.pin == pin);
        let optional_off = f.kind == PinFunctionKind::SpdifRx && !self.spdif_enabled(f.index);
        let choices = self
            .valid
            .iter()
            .copied()
            .filter(|p| self.check(f, *p).is_ok())
            .filter(|p| !optional_off || *p == pin || self.held_by(*p, Some(f)).is_none())
            .collect();
        let i2s_allowed = f.kind == PinFunctionKind::Output
            && f.index < self.num_slots()
            && self.i2s_allowed(f.index);
        PinFunctionInfo {
            function: f,
            label,
            pin,
            holds,
            choices,
            i2s_allowed,
        }
    }

    fn functions(&self) -> Vec<PinFunctionInfo> {
        use PinFunctionKind as K;
        let s = self.s;
        let mut out = Vec::new();
        let slots = self.num_slots();
        for i in 0..self.num_outputs() {
            out.push(self.info(PinFunction::new(K::Output, i), output_label(i, slots)));
        }
        if !assignable(s.platform_id) {
            // The fixed-pin platforms: only the outputs, for their slot rule.
            return out;
        }
        out.push(self.info(PinFunction::one(K::I2sBck), "I2S BCK".into()));
        if s.i2s_clock_pin_mode_present {
            out.push(self.info(PinFunction::one(K::I2sBckSlave), "I2S Slave BCK".into()));
        }
        out.push(self.info(PinFunction::one(K::Mck), "I2S MCK".into()));
        for i in 0..self.spdif_inputs() {
            out.push(self.info(PinFunction::new(K::SpdifRx, i), self.spdif_label(i)));
        }
        for p in 0..max_pairs(self.platform) {
            let label = if max_pairs(self.platform) > 1 {
                format!("I2S RX {}", p.saturating_add(1))
            } else {
                "I2S RX".into()
            };
            out.push(self.info(PinFunction::new(K::I2sRx, p), label));
        }
        if self.is_rp2350() {
            out.push(self.info(PinFunction::one(K::AdatOut), "ADAT Output".into()));
            out.push(self.info(PinFunction::one(K::AdatIn), "ADAT Input".into()));
        }
        if s.dac_mute_supported {
            out.push(self.info(PinFunction::one(K::DacMute), "DAC Mute".into()));
        }
        if s.ctrl_iface_supported {
            out.push(self.info(PinFunction::one(K::UartTx), "UART TX".into()));
            out.push(self.info(PinFunction::one(K::UartRx), "UART RX".into()));
            out.push(self.info(PinFunction::one(K::I2cSda), "I2C SDA".into()));
            out.push(self.info(PinFunction::one(K::I2cScl), "I2C SCL".into()));
        }
        out
    }
}

fn output_label(i: u8, slots: u8) -> String {
    if i < slots {
        format!("Output {}", i.saturating_add(1))
    } else {
        "Subwoofer".into()
    }
}

// ------------------------------------------------------------ pure API

/// The pin map of the device `state` describes, with the live
/// control-surface bindings `uses`.
pub fn pin_map(state: &DspState, uses: &[PinUse]) -> PinMap {
    let cx = Ctx::new(state, uses);
    let mut holders: Vec<PinHolder> = Vec::new();
    for c in &cx.claims {
        if !holders.iter().any(|h| h.pin == c.pin) {
            holders.push(c.clone());
        }
    }
    holders.sort_by_key(|h| h.pin);
    PinMap {
        platform_id: state.platform_id,
        assignable: !cx.valid.is_empty(),
        valid_pins: cx.valid.clone(),
        functions: cx.functions(),
        holders,
    }
}

/// Whether `function` may move to `pin`, by the rules in the module docs:
/// `Ok`, or why not with the status the firmware answers.
pub fn pin_check(
    state: &DspState,
    uses: &[PinUse],
    function: PinFunction,
    pin: u8,
) -> std::result::Result<(), PinRefusal> {
    Ctx::new(state, uses).check(function, pin)
}

/// The function holding `pin`, not counting `excluding` (both pins of a
/// control interface count as one owner).
pub fn pin_holder(
    state: &DspState,
    uses: &[PinUse],
    pin: u8,
    excluding: Option<PinFunction>,
) -> Option<PinHolder> {
    Ctx::new(state, uses).held_by(pin, excluding).cloned()
}

/// A whole UART or I2C config, as its setter sends it: both pins checked,
/// and on one instance. A disabled config is not checked (the firmware
/// stores it as it is).
fn check_ctrl_pair(
    state: &DspState,
    uses: &[PinUse],
    pins: [(PinFunctionKind, u8); 2],
    instance: fn(u8) -> u8,
) -> std::result::Result<(), PinRefusal> {
    if !state.ctrl_iface_supported {
        return Ok(());
    }
    let cx = Ctx::new(state, uses);
    for (kind, pin) in pins {
        cx.check(PinFunction::one(kind), pin)?;
    }
    let [(_, a), (_, b)] = pins;
    if instance(a) != instance(b) {
        return Err(PinRefusal::new(PinRefusalReason::WrongMux, INVALID_PIN));
    }
    Ok(())
}

/// `uart_instance_index` (uart_control.c).
fn uart_instance(pin: u8) -> u8 {
    (((pin >> 2).wrapping_add(1)) >> 1) & 1
}

/// `i2c_instance_index` (i2c_control.c).
fn i2c_instance(pin: u8) -> u8 {
    (pin >> 1) & 1
}

// ------------------------------------------------------------ the core

#[cfg_attr(feature = "uniffi", uniffi::export)]
impl DspiCore {
    /// The pin map (WP CG-SWS-b): the platform's valid GPIOs, each function
    /// with its pin, whether it holds it and the pins it may move to, and
    /// who holds every held GPIO. From the mirror and the last
    /// control-surface read: no device I/O. Empty without a device.
    pub fn pin_map(&self) -> PinMap {
        let uses = self.cs_pin_uses();
        let inner = self.lock();
        if inner.conn.is_none() {
            return PinMap::default();
        }
        pin_map(&inner.state, &uses)
    }

    /// Whether `function` may move to `pin`: `None` when it may, else why
    /// not (with the status the firmware would answer and, when another
    /// function holds it, which). From the mirror: no device I/O. The pin
    /// setters run this check before they send.
    pub fn pin_check(&self, function: PinFunction, pin: u8) -> Option<PinRefusal> {
        let uses = self.cs_pin_uses();
        let inner = self.lock();
        pin_check(&inner.state, &uses, function, pin).err()
    }

    /// The function holding `pin`, not counting `excluding` (a picker passes
    /// its own function), or `None` when it is free. From the mirror.
    pub fn pin_holder(&self, pin: u8, excluding: Option<PinFunction>) -> Option<PinHolder> {
        let uses = self.cs_pin_uses();
        let inner = self.lock();
        pin_holder(&inner.state, &uses, pin, excluding)
    }
}

impl DspiCore {
    /// The setters' gate: the check, when the core knows this platform's
    /// rules and the device's configuration (an open, writable connection
    /// whose bulk image was read). Otherwise, and for
    /// `PIN_RESET_TO_DEFAULT`, the device decides.
    pub(crate) fn pin_gate(&self, function: PinFunction, pin: u8) -> Result<()> {
        if pin == PIN_NONE && !matches!(function.kind, PinFunctionKind::AdatIn) {
            return Ok(());
        }
        self.gate_with(|s, uses| pin_check(s, uses, function, pin))
    }

    /// The gate for a DAC mute config: its pin, while enabled, on firmware
    /// that has the mute line.
    pub(crate) fn dac_mute_gate(&self, enabled: bool, pin: u8) -> Result<()> {
        if !enabled {
            return Ok(());
        }
        self.gate_with(|s, uses| {
            if !s.dac_mute_supported {
                return Ok(());
            }
            pin_check(s, uses, PinFunction::one(PinFunctionKind::DacMute), pin)
        })
    }

    /// The gate for a UART config.
    pub(crate) fn uart_gate(&self, enabled: bool, tx: u8, rx: u8) -> Result<()> {
        if !enabled {
            return Ok(());
        }
        self.gate_with(|s, uses| {
            check_ctrl_pair(
                s,
                uses,
                [(PinFunctionKind::UartTx, tx), (PinFunctionKind::UartRx, rx)],
                uart_instance,
            )
        })
    }

    /// The gate for an I2C config.
    pub(crate) fn i2c_gate(&self, enabled: bool, sda: u8, scl: u8) -> Result<()> {
        if !enabled {
            return Ok(());
        }
        self.gate_with(|s, uses| {
            check_ctrl_pair(
                s,
                uses,
                [
                    (PinFunctionKind::I2cSda, sda),
                    (PinFunctionKind::I2cScl, scl),
                ],
                i2c_instance,
            )
        })
    }

    fn gate_with(
        &self,
        check: impl FnOnce(&DspState, &[PinUse]) -> std::result::Result<(), PinRefusal>,
    ) -> Result<()> {
        let uses = self.cs_pin_uses();
        let mut inner = self.lock();
        let ready =
            inner.conn.is_some() && inner.state.bulk_valid && assignable(inner.state.platform_id);
        if !ready {
            return Ok(());
        }
        match check(&inner.state, &uses) {
            Ok(()) => Ok(()),
            Err(r) => {
                // The setter's own refusals first (read-only, the installer's
                // lock), as if it had run: nothing is sent either way.
                inner.writable_conn()?;
                Err(DspiError::Refused { status: r.status })
            }
        }
    }
}

#[cfg(test)]
#[path = "pins/tests_wp_cgswsb.rs"]
mod tests_wp_cgswsb;
