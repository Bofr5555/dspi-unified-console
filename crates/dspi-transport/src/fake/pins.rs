//! The pin setters (WP CG-SWS-b): the write-as-read `REQ_SET_*_PIN`
//! requests and the I2S clock-pin mode, so a client on the demo can move a
//! pin and see it moved (before this they stalled).
//!
//! Modelled on the firmware's handlers (Bofr5555/DSPi `vendor_commands.c`):
//! the same status bytes in the same order, `PIN_RESET_TO_DEFAULT` (0xFF) to
//! the platform's default, `PARAM_CHANGED` at the field's bulk offset, and
//! the firmware's own `is_pin_in_use`: the I2S clock pair is held only while
//! the clocks run (an I2S output slot, or I2S the input), which is more
//! lenient than the core's pin map (`dspi-core` `pins.rs`), so whatever the
//! core lets through the demo takes.
//!
//! Not modelled: the control interfaces' and the control surfaces' claims
//! (the fake has no live UART, I2C or binding status), the PDM output's
//! `OUTPUT_ACTIVE`, a pending I2S clock-mode change, and the deferred apply
//! (the image changes at once).
#![warn(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

use dspi_proto::generated::{notify as nfy, opcodes as op, status as st};
use dspi_proto::wire::INPUT_CONFIG_FIELDS;

use super::{Field, State};

const OK: u8 = st::PIN_CONFIG_SUCCESS as u8;
const INVALID_PIN: u8 = st::PIN_CONFIG_INVALID_PIN as u8;
const IN_USE: u8 = st::PIN_CONFIG_PIN_IN_USE as u8;
const INVALID_OUTPUT: u8 = st::PIN_CONFIG_INVALID_OUTPUT as u8;
const OUTPUT_ACTIVE: u8 = st::PIN_CONFIG_OUTPUT_ACTIVE as u8;
const INVALID_PARAM: u8 = st::PIN_CONFIG_INVALID_PARAM as u8;

/// `PIN_RESET_TO_DEFAULT` (config.h).
const RESET: u8 = 0xFF;

// `WireI2SConfig` (bulk_params.h:157-170).
const I_TYPES: Field = ("output_types", 0, 4);
const I_BCK: Field = ("bck_pin", 4, 1);
const I_MCK_PIN: Field = ("mck_pin", 5, 1);
const I_MCK_ENABLED: Field = ("mck_enabled", 6, 1);
const I_PIN_MODE_P1: Field = ("clock_pin_mode_p1", 8, 1);
const I_BCK_SLAVE: Field = ("bck_pin_slave", 9, 1);
// `WireAdatConfig`.
const A_ENABLED: Field = ("enabled", 0, 1);
const A_PIN: Field = ("pin", 1, 1);
// `WireDacHwMute`: enabled, active_low, pin.
const D_ENABLED: Field = ("enabled", 0, 1);
const D_PIN: Field = ("pin", 2, 1);
// `WireInputConfig`.
const IC_SOURCE: Field = INPUT_CONFIG_FIELDS[0];
const IC_SPDIF: Field = INPUT_CONFIG_FIELDS[1];
const IC_I2S_RX: Field = INPUT_CONFIG_FIELDS[2];
const IC_I2S_CHANNELS: Field = INPUT_CONFIG_FIELDS[4];
const IC_I2S_RX_EXT: Field = INPUT_CONFIG_FIELDS[5];
const IC_SPDIF_EXT: Field = INPUT_CONFIG_FIELDS[6];
const IC_SPDIF_ENABLED_P1: Field = INPUT_CONFIG_FIELDS[7];
const IC_I2S_CLOCK_MODE: Field = INPUT_CONFIG_FIELDS[8];
const IC_ADAT_PIN: Field = INPUT_CONFIG_FIELDS[9];
const IC_ADAT_ENABLED_P1: Field = INPUT_CONFIG_FIELDS[10];

const SPDIF_INPUTS: u8 = 4;
const INPUT_SOURCE_I2S: u8 = 2;
const SLAVE: u8 = 1;
const SPLIT: u8 = 1;
const OUTPUT_TYPE_I2S: u8 = 1;

impl State {
    fn rp2350(&self) -> bool {
        self.header_byte(1) == 1
    }

    fn read(&self, section: &str, i: usize, f: Field) -> Option<u8> {
        self.bytes(self.locate(section, 0, 0, f)?.checked_add(i)?, 1)?
            .first()
            .copied()
    }

    fn store(&mut self, section: &str, i: usize, f: Field, v: u8) {
        if let Some(off) = self.locate(section, 0, 0, f).and_then(|o| o.checked_add(i)) {
            self.write(off, &[v], nfy::PARAM_SRC_HOST_SET as u8);
        }
    }

    fn ic_byte(&self, f: Field) -> u8 {
        self.read("input_config", 0, f).unwrap_or(0)
    }

    fn i2s(&self, f: Field) -> u8 {
        self.read("i2s_config", 0, f).unwrap_or(0)
    }

    /// `is_valid_gpio_pin`.
    fn valid_gpio(&self, pin: u8) -> bool {
        !(23..=25).contains(&pin) && pin <= if self.rp2350() { 29 } else { 28 }
    }

    /// The physical outputs (`pins.count`), slots then the sub.
    fn pin_count(&self) -> u8 {
        self.read("pins", 0, ("count", 0, 1)).unwrap_or(0)
    }

    fn output_pin(&self, i: u8) -> Option<u8> {
        (i < self.pin_count())
            .then(|| self.read("pins", usize::from(i), ("pins", 1, 1)))
            .flatten()
    }

    fn slots(&self) -> u8 {
        self.pin_count().saturating_sub(1).min(4)
    }

    fn any_i2s(&self) -> bool {
        (0..self.slots())
            .any(|i| self.read("i2s_config", usize::from(i), I_TYPES) == Some(OUTPUT_TYPE_I2S))
    }

    fn pin_mode(&self) -> u8 {
        self.i2s(I_PIN_MODE_P1).saturating_sub(1)
    }

    fn spdif_rx(&self, i: u8) -> Option<u8> {
        match i {
            0 => self.read("input_config", 0, IC_SPDIF),
            1..SPDIF_INPUTS => self.read(
                "input_config",
                usize::from(i.saturating_sub(1)),
                IC_SPDIF_EXT,
            ),
            _ => None,
        }
    }

    fn spdif_on(&self, i: u8) -> bool {
        let ext = self.ic_byte(IC_SPDIF_ENABLED_P1).saturating_sub(1);
        i == 0
            || (1..SPDIF_INPUTS).contains(&i)
                && ext.checked_shr(u32::from(i.saturating_sub(1))).unwrap_or(0) & 1 != 0
    }

    fn pairs_max(&self) -> u8 {
        if self.rp2350() { 4 } else { 1 }
    }

    fn i2s_rx(&self, pair: u8) -> Option<u8> {
        match pair {
            0 => self.read("input_config", 0, IC_I2S_RX),
            1..4 => self.read(
                "input_config",
                usize::from(pair.saturating_sub(1)),
                IC_I2S_RX_EXT,
            ),
            _ => None,
        }
    }

    fn active_pairs(&self) -> u8 {
        (self.ic_byte(IC_I2S_CHANNELS).max(2) / 2).min(self.pairs_max())
    }

    /// The ADAT input's pin, `RESET` when unset (wire 0 = absent).
    fn adat_in_pin(&self) -> u8 {
        match self.ic_byte(IC_ADAT_PIN) {
            0 => RESET,
            p => p,
        }
    }

    fn adat_in_on(&self) -> bool {
        self.ic_byte(IC_ADAT_ENABLED_P1) == 2
    }

    fn adat_out_on(&self) -> bool {
        self.rp2350()
            && self
                .read("adat_config", 0, A_ENABLED)
                .is_some_and(|v| v != 0)
    }

    fn clock_claimed(&self, pin: u8) -> bool {
        let bck = self.i2s(I_BCK);
        let slave = self.i2s(I_BCK_SLAVE);
        pin == bck
            || pin == bck.wrapping_add(1)
            || (self.pin_mode() == SPLIT && (pin == slave || pin == slave.wrapping_add(1)))
    }

    /// `pin_used_by_fixed_peripheral`, MCK included.
    fn fixed(&self, pin: u8, exclude_output: u8) -> bool {
        if self.i2s(I_MCK_ENABLED) != 0 && pin == self.i2s(I_MCK_PIN) {
            return true;
        }
        if (0..self.pin_count()).any(|i| i != exclude_output && self.output_pin(i) == Some(pin)) {
            return true;
        }
        if self.adat_out_on() && self.read("adat_config", 0, A_PIN) == Some(pin) {
            return true;
        }
        if self.rp2350() && self.adat_in_on() && self.adat_in_pin() == pin {
            return true;
        }
        if (0..SPDIF_INPUTS).any(|i| self.spdif_on(i) && self.spdif_rx(i) == Some(pin)) {
            return true;
        }
        self.read("dac_hw_mute", 0, D_ENABLED)
            .is_some_and(|v| v != 0)
            && self.read("dac_hw_mute", 0, D_PIN) == Some(pin)
    }

    /// `is_pin_in_use`.
    fn in_use(&self, pin: u8, exclude_output: u8) -> bool {
        if self.fixed(pin, exclude_output) {
            return true;
        }
        let clocks = self.any_i2s() || self.ic_byte(IC_SOURCE) == INPUT_SOURCE_I2S;
        if clocks && self.clock_claimed(pin) {
            return true;
        }
        (0..self.active_pairs()).any(|p| self.i2s_rx(p) == Some(pin))
    }

    /// The pin requests: `None` for another opcode, `Some(None)` to stall.
    pub(super) fn get_pin(&mut self, opcode: u8, value: u16) -> Option<Option<Vec<u8>>> {
        let [hi, lo] = value.to_be_bytes();
        let status = match opcode {
            op::REQ_SET_OUTPUT_PIN => self.set_output_pin(lo, hi),
            op::REQ_GET_OUTPUT_PIN => return Some(self.output_pin(lo).map(|p| vec![p])),
            op::REQ_SET_I2S_BCK_PIN => self.set_bck(hi, lo),
            op::REQ_SET_I2S_CLOCK_PIN_MODE => self.set_pin_mode(lo),
            op::REQ_GET_I2S_CLOCK_PIN_MODE => return Some(Some(vec![self.pin_mode()])),
            op::REQ_SET_MCK_PIN => self.set_mck(lo),
            op::REQ_SET_ADAT_PIN => self.set_adat_out(lo),
            op::REQ_SET_SPDIF_RX_PIN => self.set_spdif(hi, lo),
            op::REQ_SET_I2S_RX_PIN => self.set_i2s_rx(hi, lo),
            op::REQ_SET_ADAT_INPUT_PIN => self.set_adat_in(lo),
            _ => return None,
        };
        Some(Some(vec![status]))
    }

    fn set_output_pin(&mut self, idx: u8, pin: u8) -> u8 {
        if idx >= self.pin_count() {
            return INVALID_OUTPUT;
        }
        let pin = if pin == RESET {
            let sub = idx == self.pin_count().saturating_sub(1);
            if sub { 10 } else { 6u8.saturating_add(idx) }
        } else {
            pin
        };
        if !self.valid_gpio(pin) {
            INVALID_PIN
        } else if self.in_use(pin, idx) {
            IN_USE
        } else {
            self.store("pins", usize::from(idx), ("pins", 1, 1), pin);
            OK
        }
    }

    fn set_bck(&mut self, role: u8, pin: u8) -> u8 {
        if role > 1 {
            return INVALID_OUTPUT;
        }
        let pin = match (pin, role) {
            (RESET, 0) => 14,
            (RESET, _) if self.rp2350() => 26,
            (RESET, _) => 12,
            (p, _) => p,
        };
        let lrck = pin.wrapping_add(1);
        let field = if role == 0 { I_BCK } else { I_BCK_SLAVE };
        if !self.valid_gpio(pin) || !self.valid_gpio(lrck) {
            return INVALID_PIN;
        }
        if pin == self.i2s(field) {
            return OK;
        }
        let other = self.i2s(if role == 0 { I_BCK_SLAVE } else { I_BCK });
        let split = self.pin_mode() == SPLIT;
        let overlaps =
            (role == 1 || split) && (pin == other || pin == other.wrapping_add(1) || lrck == other);
        let split_slave = split && self.ic_byte(IC_I2S_CLOCK_MODE) == SLAVE;
        if self.any_i2s() && (role == 1) == split_slave {
            OUTPUT_ACTIVE
        } else if overlaps || self.in_use(pin, RESET) || self.in_use(lrck, RESET) {
            IN_USE
        } else {
            self.store("i2s_config", 0, field, pin);
            OK
        }
    }

    fn set_pin_mode(&mut self, mode: u8) -> u8 {
        if mode > SPLIT {
            return INVALID_PARAM;
        }
        if mode == self.pin_mode() {
            return OK;
        }
        let bck = self.i2s(I_BCK);
        let slave = self.i2s(I_BCK_SLAVE);
        let bad = mode == SPLIT
            && (slave == bck
                || slave == bck.wrapping_add(1)
                || slave.wrapping_add(1) == bck
                || self.in_use(slave, RESET)
                || self.in_use(slave.wrapping_add(1), RESET));
        if self.ic_byte(IC_I2S_CLOCK_MODE) == SLAVE && self.any_i2s() {
            OUTPUT_ACTIVE
        } else if bad {
            IN_USE
        } else {
            self.store("i2s_config", 0, I_PIN_MODE_P1, mode.saturating_add(1));
            OK
        }
    }

    fn set_mck(&mut self, pin: u8) -> u8 {
        let rp2350 = self.rp2350();
        let pin = match pin {
            RESET if rp2350 => 13,
            RESET => 21,
            p => p,
        };
        let gpout = if rp2350 {
            matches!(pin, 13 | 15 | 21)
        } else {
            pin == 21
        };
        if !self.valid_gpio(pin) || !gpout {
            INVALID_PIN
        } else if self.i2s(I_MCK_ENABLED) != 0 {
            OUTPUT_ACTIVE
        } else if self.in_use(pin, RESET) {
            IN_USE
        } else {
            self.store("i2s_config", 0, I_MCK_PIN, pin);
            OK
        }
    }

    fn set_adat_out(&mut self, pin: u8) -> u8 {
        if !self.rp2350() {
            return INVALID_OUTPUT;
        }
        let pin = if pin == RESET { 12 } else { pin };
        if !self.valid_gpio(pin) {
            INVALID_PIN
        } else if self.read("adat_config", 0, A_PIN) == Some(pin) {
            OK
        } else if self.in_use(pin, RESET) {
            IN_USE
        } else {
            self.store("adat_config", 0, A_PIN, pin);
            OK
        }
    }

    fn set_spdif(&mut self, index: u8, pin: u8) -> u8 {
        if index >= SPDIF_INPUTS {
            return INVALID_OUTPUT;
        }
        let pin = if pin == RESET {
            [5, 20, 21, 22]
                .get(usize::from(index))
                .copied()
                .unwrap_or(5)
        } else {
            pin
        };
        if !self.valid_gpio(pin) {
            INVALID_PIN
        } else if self.spdif_rx(index) == Some(pin) {
            OK
        } else if self.spdif_on(index) && self.in_use(pin, RESET) {
            IN_USE
        } else {
            match index {
                0 => self.store("input_config", 0, IC_SPDIF, pin),
                i => self.store(
                    "input_config",
                    usize::from(i.saturating_sub(1)),
                    IC_SPDIF_EXT,
                    pin,
                ),
            }
            OK
        }
    }

    fn set_i2s_rx(&mut self, pair: u8, pin: u8) -> u8 {
        if pair >= self.pairs_max() {
            return INVALID_OUTPUT;
        }
        let pin = if pin == RESET {
            1u8.saturating_add(pair)
        } else {
            pin
        };
        if self.i2s_rx(pair) == Some(pin) {
            return OK;
        }
        let other_pair = (0..self.active_pairs()).any(|q| q != pair && self.i2s_rx(q) == Some(pin));
        if !self.valid_gpio(pin) {
            INVALID_PIN
        } else if self.clock_claimed(pin) || self.fixed(pin, RESET) || other_pair {
            IN_USE
        } else {
            match pair {
                0 => self.store("input_config", 0, IC_I2S_RX, pin),
                p => self.store(
                    "input_config",
                    usize::from(p.saturating_sub(1)),
                    IC_I2S_RX_EXT,
                    pin,
                ),
            }
            OK
        }
    }

    fn set_adat_in(&mut self, pin: u8) -> u8 {
        if !self.rp2350() {
            return INVALID_OUTPUT;
        }
        let out_pin = self.read("adat_config", 0, A_PIN);
        if pin == RESET {
            if self.adat_in_on() {
                return IN_USE;
            }
            self.store("input_config", 0, IC_ADAT_PIN, 0);
            return OK;
        }
        if !self.valid_gpio(pin) {
            INVALID_PIN
        } else if pin == self.adat_in_pin() {
            OK
        } else if out_pin != Some(pin) && self.in_use(pin, RESET) {
            IN_USE
        } else {
            self.store("input_config", 0, IC_ADAT_PIN, pin);
            OK
        }
    }
}

#[cfg(test)]
mod tests_wp_cgswsb {
    use super::super::FakeDevice;
    use crate::Transport;
    use dspi_proto::generated::{opcodes as op, status as st};

    fn set(d: &mut FakeDevice, opcode: u8, value: u16) -> u8 {
        d.control_in(opcode, value, 1).expect("answered")[0]
    }

    fn get(d: &mut FakeDevice, opcode: u8, value: u16) -> u8 {
        d.control_in(opcode, value, 1).expect("answered")[0]
    }

    fn wv(hi: u8, lo: u8) -> u16 {
        u16::from_be_bytes([hi, lo])
    }

    const OK: u8 = st::PIN_CONFIG_SUCCESS as u8;
    const INVALID_PIN: u8 = st::PIN_CONFIG_INVALID_PIN as u8;
    const IN_USE: u8 = st::PIN_CONFIG_PIN_IN_USE as u8;
    const INVALID_OUTPUT: u8 = st::PIN_CONFIG_INVALID_OUTPUT as u8;

    /// WB6I found `REQ_SET_OUTPUT_PIN` stalling on the demo.
    #[test]
    fn cgswsb_the_demo_moves_an_output_pin() {
        let mut d = FakeDevice::demo();
        assert_eq!(get(&mut d, op::REQ_GET_OUTPUT_PIN, 0), 6);
        let out2 = get(&mut d, op::REQ_GET_OUTPUT_PIN, 1);
        assert_eq!(set(&mut d, op::REQ_SET_OUTPUT_PIN, wv(0, 0)), OK);
        assert_eq!(get(&mut d, op::REQ_GET_OUTPUT_PIN, 0), 0);
        // Output 2's pin is taken; GPIO 24 is power; there is no output 9.
        assert_eq!(set(&mut d, op::REQ_SET_OUTPUT_PIN, wv(out2, 0)), IN_USE);
        assert_eq!(set(&mut d, op::REQ_SET_OUTPUT_PIN, wv(24, 0)), INVALID_PIN);
        assert_eq!(
            set(&mut d, op::REQ_SET_OUTPUT_PIN, wv(0, 9)),
            INVALID_OUTPUT
        );
        // Back to the default.
        assert_eq!(set(&mut d, op::REQ_SET_OUTPUT_PIN, wv(0xFF, 0)), OK);
        assert_eq!(get(&mut d, op::REQ_GET_OUTPUT_PIN, 0), 6);
        assert!(
            d.control_in(op::REQ_GET_OUTPUT_PIN, 9, 1).is_err(),
            "stalls"
        );
    }

    #[test]
    fn cgswsb_the_demo_answers_every_pin_set() {
        let mut d = FakeDevice::demo();
        // MCK: clock-output pins only.
        assert_eq!(set(&mut d, op::REQ_SET_MCK_PIN, 12), INVALID_PIN);
        assert_eq!(set(&mut d, op::REQ_SET_MCK_PIN, 21), OK);
        assert_eq!(get(&mut d, op::REQ_GET_MCK_PIN, 0), 21);
        // The clock pairs, and split mode.
        assert_eq!(set(&mut d, op::REQ_SET_I2S_BCK_PIN, wv(0, 22)), INVALID_PIN);
        assert_eq!(set(&mut d, op::REQ_SET_I2S_BCK_PIN, wv(1, 18)), OK);
        assert_eq!(get(&mut d, op::REQ_GET_I2S_BCK_PIN, 1), 18);
        assert_eq!(set(&mut d, op::REQ_SET_I2S_CLOCK_PIN_MODE, 1), OK);
        assert_eq!(get(&mut d, op::REQ_GET_I2S_CLOCK_PIN_MODE, 0), 1);
        assert_eq!(
            set(&mut d, op::REQ_SET_I2S_CLOCK_PIN_MODE, 2),
            st::PIN_CONFIG_INVALID_PARAM as u8
        );
        // In split mode the master pair may not overlap the slave pair.
        assert_eq!(set(&mut d, op::REQ_SET_I2S_BCK_PIN, wv(0, 17)), IN_USE);
        // The receivers.
        assert_eq!(set(&mut d, op::REQ_SET_SPDIF_RX_PIN, wv(0, 16)), OK);
        assert_eq!(get(&mut d, op::REQ_GET_SPDIF_RX_PIN, 0), 16);
        assert_eq!(
            set(&mut d, op::REQ_SET_SPDIF_RX_PIN, wv(4, 16)),
            INVALID_OUTPUT
        );
        assert_eq!(
            set(&mut d, op::REQ_SET_I2S_RX_PIN, wv(0, 18)),
            IN_USE,
            "the slave BCK"
        );
        assert_eq!(set(&mut d, op::REQ_SET_I2S_RX_PIN, wv(0, 0)), OK);
        assert_eq!(get(&mut d, op::REQ_GET_I2S_RX_PIN, 0), 0);
        // ADAT: the input may share the output's pin.
        assert_eq!(set(&mut d, op::REQ_SET_ADAT_PIN, 28), OK);
        assert_eq!(get(&mut d, op::REQ_GET_ADAT_PIN, 0), 28);
        assert_eq!(set(&mut d, op::REQ_SET_ADAT_INPUT_PIN, 28), OK);
        assert_eq!(get(&mut d, op::REQ_GET_ADAT_INPUT_PIN, 0), 28);
    }
}
