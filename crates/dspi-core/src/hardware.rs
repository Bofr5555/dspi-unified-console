//! Hardware: pins, outputs, I2S/S/PDIF/ADAT, clocks, DAC mute, LG, control interfaces, Core 1, bootloader.
//!
//! Three kinds of request, as the firmware has them (`vendor_commands.c`):
//!
//! - **Write-as-read** pin and clock setters: an IN transfer with the
//!   parameters in `wValue` that answers one `PIN_CONFIG_*` status byte
//!   (`dspi_proto::generated::status`). A non-zero status is
//!   [`DspiError::Refused`] with that byte (`INVALID_PIN` 1, `PIN_IN_USE` 2,
//!   `INVALID_OUTPUT` 3, `OUTPUT_ACTIVE` 4, `INVALID_PARAM` 5) and leaves the
//!   mirror alone; status 0 updates the mirror field and `bulk_raw`.
//!   `PIN_RESET_TO_DEFAULT` (0xFF) asks for the platform default; the setter
//!   then reads back which pin that is, under the DSPi Link device lock.
//! - **Payload SETs** (OUT, no status byte): `Ok` means only that the
//!   transfer completed. Several are applied later in the firmware's main
//!   loop (I2S clock mode, the DAC mute, UART and I2C configs) and some are
//!   dropped silently when invalid, so the caller reads the outcome back
//!   (`fetch_dac_mute`, `fetch_ctrl_ifaces`).
//! - **Reads** of status packets and of the values the bulk image does not
//!   carry (Core 1 mode, DAC mute, control interfaces), through WP C2's
//!   per-item readers where they exist. Short replies are typed errors,
//!   never panics.
//!
//! The opcodes are sent raw rather than through the registry's fast path
//! (`Session::send`): the firmware's `wValue` packing and status bytes are
//! what this module maps, and the registry's rows for two of them
//! (`in.adat.enable`, `in.adat.clock`) describe OUT transfers where the
//! firmware has write-as-read ones.
//!
//! Every write goes through `Inner::writable_conn`, so a read-only
//! connection (another wire format, PLAN §1.5) refuses it with
//! [`DspiError::ReadOnly`]. `enter_bootloader` is a plain read on purpose:
//! a device too old or too new for this build must still be updatable.
//!
//! Owned by WP C6 (docs/core-merge/PLAN.md).

use std::time::Duration;

use dspi_proto::generated::opcodes as op;
use dspi_transport::TransportError;

use crate::consts::FW_BETA_EARLY;
use crate::error::status;
use crate::records::*;
use crate::{Connection, DspiCore, DspiError, Result, Writable, connect, state};

/// `PIN_RESET_TO_DEFAULT` (config.h): a pin setter's "the platform default".
const PIN_RESET_TO_DEFAULT: u8 = 0xFF;

/// How long a multi-request flow waits for the DSPi Link device lock.
const LOCK_TIMEOUT: Duration = Duration::from_secs(60);

/// `REQ_SET_DAC_HW_MUTE_CONFIG` payload: the 16-byte `DacHwMuteConfig`.
const DAC_MUTE_CONFIG_LEN: usize = 16;

/// `REQ_SET_UART_CONFIG` / `REQ_SET_I2C_CONFIG` payloads (`UartCtrlConfig`,
/// `I2cCtrlConfig`): 8 bytes each.
const CTRL_CONFIG_LEN: usize = 8;

/// `wValue` = `(hi << 8) | lo`.
fn wv(hi: u8, lo: u8) -> u16 {
    u16::from_be_bytes([hi, lo])
}

/// After a setter changed decoded fields: write them into `bulk_raw` too, so
/// a later bulk write carries them. A display-only image is left alone.
fn sync_image(s: &mut DspState) {
    if s.bulk_valid {
        s.bulk_raw = state::encode_bulk(s);
    }
}

/// An IN transfer of up to `len` bytes that must bring at least `min`.
fn read_min(
    conn: &mut Connection,
    opcode: u8,
    value: u16,
    len: u16,
    min: usize,
) -> Result<Vec<u8>> {
    let d = conn.get_upto(opcode, value, len)?;
    if d.len() < min {
        return Err(TransportError::ShortRead {
            opcode,
            wanted: min,
            got: d.len(),
        }
        .into());
    }
    Ok(d)
}

/// A one-byte read.
fn read_u8(conn: &mut Connection, opcode: u8, value: u16) -> Result<u8> {
    let d = read_min(conn, opcode, value, 1, 1)?;
    Ok(byte(&d, 0))
}

/// The byte at `at`, 0 past the end (callers check the length first).
fn byte(d: &[u8], at: usize) -> u8 {
    d.get(at).copied().unwrap_or(0)
}

fn u16_at(d: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([byte(d, at), byte(d, at.saturating_add(1))])
}

fn u32_at(d: &[u8], at: usize) -> u32 {
    let b = |k: usize| byte(d, at.saturating_add(k));
    u32::from_le_bytes([b(0), b(1), b(2), b(3)])
}

/// Run `f` under the transport's device lock (DSPi Link; a no-op on other
/// links). A refused lock is [`DspiError::Locked`] and nothing is sent; the
/// lock is released whatever `f` returns.
fn locked<T>(
    w: &mut Writable<'_>,
    reason: &str,
    f: impl FnOnce(&mut Writable<'_>) -> Result<T>,
) -> Result<T> {
    w.conn().transport().lock(reason, LOCK_TIMEOUT)?;
    let r = f(w);
    w.conn().transport().unlock();
    r
}

/// The mirror's entry `i` of a per-index vector, if it exists.
fn entry(v: &mut [u8], i: u8) -> Option<&mut u8> {
    v.get_mut(usize::from(i))
}

/// `major.minor.patch` and the beta ordinal (0 = release) at the start of a
/// UF2 program-version string (`1.1.6`, `1.1.6-beta3`,
/// `1.1.6-beta3-16-g557bce7`, a leading `v` allowed). `None` when it does not
/// start with three dotted numbers that fit a byte.
fn parse_version(s: &str) -> Option<(u8, u8, u8, u8)> {
    let s = s.trim();
    let s = s.strip_prefix(['v', 'V']).unwrap_or(s);
    let end = s
        .find(|c: char| !(c.is_ascii_digit() || c == '.'))
        .unwrap_or(s.len());
    let (core, rest) = s.split_at_checked(end)?;
    let mut parts = core.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = parts.next()?.parse().ok()?;
    if parts.next().is_some() {
        return None;
    }
    let beta = match rest.strip_prefix("-beta") {
        Some(b) => b
            .split(|c: char| !c.is_ascii_digit())
            .next()?
            .parse()
            .ok()?,
        None => 0,
    };
    Some((major, minor, patch, beta))
}

/// Whether the firmware the mirror describes is `version` (see
/// [`DspiCore::firmware_version_matches`]).
fn version_matches(s: &DspState, version: &str) -> bool {
    let Some((major, minor, patch, beta)) = parse_version(version) else {
        return false;
    };
    if (s.fw_major, s.fw_minor, s.fw_patch) != (major, minor, patch) {
        return false;
    }
    if s.fw_beta == FW_BETA_EARLY {
        // The 1.1.6 betas 1 and 2 do not say which one they are.
        matches!(beta, 1 | 2)
    } else {
        s.fw_beta == beta
    }
}

impl DspiCore {
    /// A write-as-read setter: send `opcode` with `value`, map the status
    /// byte, and on success apply `update` to the mirror and `bulk_raw`.
    fn set_with_status(
        &self,
        opcode: u8,
        value: u16,
        update: impl FnOnce(&mut DspState),
    ) -> Result<()> {
        let mut guard = self.lock();
        let inner = &mut *guard;
        {
            let mut w = inner.writable_conn()?;
            w.conn().fresh_image = false;
            status(w.set_as_read(opcode, value)?)?;
        }
        update(&mut inner.state);
        sync_image(&mut inner.state);
        Ok(())
    }

    /// A write-as-read pin setter. With `pin` = `PIN_RESET_TO_DEFAULT` the
    /// device picks the platform default; `readback` (opcode, `wValue`) then
    /// reads which pin that is, the set and the read under the Link device
    /// lock. `update` gets the pin now in force.
    fn set_pin_with_status(
        &self,
        opcode: u8,
        value: u16,
        pin: u8,
        readback: (u8, u16),
        update: impl FnOnce(&mut DspState, u8),
    ) -> Result<()> {
        let mut guard = self.lock();
        let inner = &mut *guard;
        let actual = {
            let mut w = inner.writable_conn()?;
            w.conn().fresh_image = false;
            if pin == PIN_RESET_TO_DEFAULT {
                locked(&mut w, "Resetting a pin to its default", |w| {
                    status(w.set_as_read(opcode, value)?)?;
                    read_u8(w.conn(), readback.0, readback.1)
                })?
            } else {
                status(w.set_as_read(opcode, value)?)?;
                pin
            }
        };
        update(&mut inner.state, actual);
        sync_image(&mut inner.state);
        Ok(())
    }

    /// A payload SET (OUT, no status byte): `Ok` means the transfer
    /// completed. `update` then applies to the mirror and `bulk_raw`.
    fn set_payload(
        &self,
        opcode: u8,
        data: &[u8],
        update: impl FnOnce(&mut DspState),
    ) -> Result<()> {
        let mut guard = self.lock();
        let inner = &mut *guard;
        {
            let mut w = inner.writable_conn()?;
            w.conn().fresh_image = false;
            w.set(opcode, 0, data)?;
        }
        update(&mut inner.state);
        sync_image(&mut inner.state);
        Ok(())
    }

    /// A read on the open connection (read-only connections too).
    fn read_with<T>(
        &self,
        f: impl FnOnce(&mut Connection, &mut DspState) -> Result<T>,
    ) -> Result<T> {
        let mut guard = self.lock();
        let inner = &mut *guard;
        let conn = inner.conn.as_mut().ok_or(DspiError::Disconnected)?;
        f(conn, &mut inner.state)
    }

    // ── Physical outputs ──────────────────────────────────────────────

    /// Move physical output `slot` (0..`num_pin_outputs`) to GPIO `pin`;
    /// `PIN_RESET_TO_DEFAULT` (0xFF) restores the platform default, which is
    /// then read back. The firmware applies the move later through a muted
    /// pipeline reset; it is live only until a preset or output-config save.
    ///
    /// Errors: [`DspiError::Refused`] with `PIN_CONFIG_INVALID_OUTPUT` (no
    /// such slot), `INVALID_PIN`, `PIN_IN_USE`, or `OUTPUT_ACTIVE` (the PDM
    /// slot while PDM runs), the mirror unchanged; [`DspiError::ReadOnly`],
    /// [`DspiError::Disconnected`], [`DspiError::Locked`] (Link, reset only)
    /// or a transfer error.
    pub fn set_output_pin(&self, slot: u8, pin: u8) -> Result<()> {
        self.set_pin_with_status(
            op::REQ_SET_OUTPUT_PIN,
            wv(pin, slot),
            pin,
            (op::REQ_GET_OUTPUT_PIN, u16::from(slot)),
            |s, p| {
                if let Some(v) = entry(&mut s.output_pins, slot) {
                    *v = p;
                }
            },
        )
    }

    /// Read the GPIO of physical output `slot` into `DspState.output_pins`
    /// and return it.
    ///
    /// Errors: a transfer error (the device stalls on a slot it does not
    /// have); [`DspiError::Disconnected`].
    pub fn fetch_output_pin(&self, slot: u8) -> Result<u8> {
        self.read_with(|conn, s| {
            let pin = read_u8(conn, op::REQ_GET_OUTPUT_PIN, u16::from(slot))?;
            if let Some(v) = entry(&mut s.output_pins, slot) {
                *v = pin;
                sync_image(s);
            }
            Ok(pin)
        })
    }

    /// Where the physical I/O configuration is stored: 0 = independently of
    /// the presets (saved with [`DspiCore::save_output_config`]), 1 = with
    /// each preset. A device-global flash write the firmware runs later.
    /// Sets `DspState.output_config_mode`.
    ///
    /// Errors: [`DspiError::InvalidArgument`] for a mode above 1 (the
    /// firmware would silently take it as 0); [`DspiError::ReadOnly`],
    /// [`DspiError::Disconnected`] or a transfer error.
    pub fn set_output_config_mode(&self, mode: u8) -> Result<()> {
        if mode > 1 {
            return Err(DspiError::InvalidArgument {
                msg: format!("output config mode {mode} is not 0 or 1"),
            });
        }
        self.set_payload(op::REQ_SET_OUTPUT_CONFIG_MODE, &[mode], |s| {
            s.output_config_mode = mode;
        })
    }

    /// FLASH WRITE: persist the live physical I/O configuration (the
    /// independent mode's explicit save). The firmware writes it later in
    /// its main loop. Runs under the DSPi Link device lock.
    ///
    /// Errors: [`DspiError::Refused`] with a `PRESET_*` status;
    /// [`DspiError::Locked`] (Link); [`DspiError::ReadOnly`],
    /// [`DspiError::Disconnected`] or a transfer error.
    pub fn save_output_config(&self) -> Result<()> {
        let mut guard = self.lock();
        let mut w = guard.writable_conn()?;
        locked(&mut w, "Saving output configuration", |w| {
            status(w.set_as_read(op::REQ_SAVE_OUTPUT_CONFIG, 0)?)
        })
    }

    /// The bus of output slot `slot` (a stereo pair, 0..4): 0 = S/PDIF,
    /// 1 = I2S. The firmware switches later through a muted pipeline reset.
    ///
    /// Errors: [`DspiError::Refused`] with `INVALID_OUTPUT` (no such slot)
    /// or `INVALID_PIN` (a type above 1); [`DspiError::ReadOnly`],
    /// [`DspiError::Disconnected`] or a transfer error.
    pub fn set_output_type(&self, slot: u8, kind: u8) -> Result<()> {
        self.set_with_status(op::REQ_SET_OUTPUT_TYPE, wv(kind, slot), |s| {
            if let Some(v) = entry(&mut s.output_types, slot) {
                *v = kind;
            }
        })
    }

    // ── I2S clocks ────────────────────────────────────────────────────

    /// The I2S bit-clock GPIO (the word clock is `pin + 1`). `role` 0 = the
    /// master/shared pair, 1 = the slave pair used in split clock-pin mode.
    /// `PIN_RESET_TO_DEFAULT` restores the role's default, read back.
    ///
    /// Errors: [`DspiError::Refused`] with `INVALID_OUTPUT` (role above 1),
    /// `INVALID_PIN`, `PIN_IN_USE` or `OUTPUT_ACTIVE` (I2S outputs run on
    /// that pair); [`DspiError::ReadOnly`], [`DspiError::Disconnected`],
    /// [`DspiError::Locked`] (Link, reset only) or a transfer error.
    pub fn set_i2s_bck_pin(&self, role: u8, pin: u8) -> Result<()> {
        self.set_pin_with_status(
            op::REQ_SET_I2S_BCK_PIN,
            wv(role, pin),
            pin,
            (op::REQ_GET_I2S_BCK_PIN, u16::from(role)),
            |s, p| {
                if role == 0 {
                    s.i2s_bck_pin = p;
                } else {
                    s.i2s_bck_pin_slave = p;
                }
            },
        )
    }

    /// 0 = master and slave share the clock pins (unified), 1 = separate
    /// pairs (split).
    ///
    /// Errors: [`DspiError::Refused`] with `INVALID_PARAM` (mode above 1),
    /// `OUTPUT_ACTIVE` (a live swap under running I2S outputs) or
    /// `PIN_IN_USE` (the slave pair is taken); [`DspiError::ReadOnly`],
    /// [`DspiError::Disconnected`] or a transfer error.
    pub fn set_i2s_clock_pin_mode(&self, mode: u8) -> Result<()> {
        self.set_with_status(op::REQ_SET_I2S_CLOCK_PIN_MODE, u16::from(mode), |s| {
            s.i2s_clock_pin_mode = mode;
        })
    }

    /// The I2S master clock output on or off.
    ///
    /// Errors: [`DspiError::Refused`] with `PIN_IN_USE` (its pin was taken
    /// meanwhile); [`DspiError::ReadOnly`], [`DspiError::Disconnected`] or a
    /// transfer error.
    pub fn set_mck_enabled(&self, enabled: bool) -> Result<()> {
        self.set_with_status(op::REQ_SET_MCK_ENABLE, u16::from(enabled), |s| {
            s.mck_enabled = enabled;
        })
    }

    /// The master clock GPIO (a clock-output-capable pin);
    /// `PIN_RESET_TO_DEFAULT` restores the default, read back.
    ///
    /// Errors: [`DspiError::Refused`] with `INVALID_PIN` (not a GPOUT pin),
    /// `OUTPUT_ACTIVE` (MCK is enabled) or `PIN_IN_USE`;
    /// [`DspiError::ReadOnly`], [`DspiError::Disconnected`],
    /// [`DspiError::Locked`] (Link, reset only) or a transfer error.
    pub fn set_mck_pin(&self, pin: u8) -> Result<()> {
        self.set_pin_with_status(
            op::REQ_SET_MCK_PIN,
            u16::from(pin),
            pin,
            (op::REQ_GET_MCK_PIN, 0),
            |s, p| s.mck_pin = p,
        )
    }

    /// Master clock rate: 0 = 128 × fs, 1 = 256 × fs.
    ///
    /// Errors: [`DspiError::Refused`] with `INVALID_PIN` for anything else;
    /// [`DspiError::ReadOnly`], [`DspiError::Disconnected`] or a transfer
    /// error.
    pub fn set_mck_multiplier(&self, mult: u8) -> Result<()> {
        self.set_with_status(op::REQ_SET_MCK_MULTIPLIER, u16::from(mult), |s| {
            s.mck_multiplier = mult;
        })
    }

    // ── ADAT output ───────────────────────────────────────────────────

    /// The ADAT lightpipe output (RP2350) on or off.
    ///
    /// Errors: [`DspiError::Refused`] with `INVALID_OUTPUT` (RP2040),
    /// `INVALID_PARAM`, or `INVALID_PIN` / `PIN_IN_USE` (its pin, on
    /// enabling); [`DspiError::ReadOnly`], [`DspiError::Disconnected`] or a
    /// transfer error.
    pub fn set_adat_out_enabled(&self, enabled: bool) -> Result<()> {
        self.set_with_status(op::REQ_SET_ADAT_ENABLE, u16::from(enabled), |s| {
            s.adat_out_enabled = enabled;
        })
    }

    /// The ADAT output GPIO; `PIN_RESET_TO_DEFAULT` restores the default,
    /// read back. Allowed while enabled (the stream moves under mute).
    ///
    /// Errors: [`DspiError::Refused`] with `INVALID_OUTPUT` (RP2040),
    /// `INVALID_PIN` or `PIN_IN_USE`; [`DspiError::ReadOnly`],
    /// [`DspiError::Disconnected`], [`DspiError::Locked`] (Link, reset
    /// only) or a transfer error.
    pub fn set_adat_out_pin(&self, pin: u8) -> Result<()> {
        self.set_pin_with_status(
            op::REQ_SET_ADAT_PIN,
            u16::from(pin),
            pin,
            (op::REQ_GET_ADAT_PIN, 0),
            |s, p| s.adat_out_pin = p,
        )
    }

    // ── Inputs ────────────────────────────────────────────────────────

    /// The RX GPIO of S/PDIF input `index` (0..3); `PIN_RESET_TO_DEFAULT`
    /// restores the default, read back. A disabled optional input stores
    /// the pin as a preference; the live input is re-routed.
    ///
    /// Errors: [`DspiError::Refused`] with `INVALID_OUTPUT` (no such input),
    /// `INVALID_PIN` or `PIN_IN_USE`; [`DspiError::ReadOnly`],
    /// [`DspiError::Disconnected`], [`DspiError::Locked`] (Link, reset
    /// only) or a transfer error.
    pub fn set_spdif_rx_pin(&self, index: u8, pin: u8) -> Result<()> {
        self.set_pin_with_status(
            op::REQ_SET_SPDIF_RX_PIN,
            wv(index, pin),
            pin,
            (op::REQ_GET_SPDIF_RX_PIN, u16::from(index)),
            |s, p| {
                if let Some(v) = entry(&mut s.spdif_rx_pins, index) {
                    *v = p;
                }
            },
        )
    }

    /// Enable or disable optional S/PDIF input `index` (1..3). Input 0 is
    /// always on: enabling it is a no-op, disabling it is refused. Bit 0 of
    /// `DspState.spdif_inputs_enabled` therefore always stays set.
    ///
    /// Errors: [`DspiError::Refused`] with `INVALID_OUTPUT` (index 0 off, or
    /// no such input) or `PIN_IN_USE` (enabling: its pin is invalid or
    /// taken; disabling: it is the live source or a pending switch targets
    /// it, so switch the source away first); [`DspiError::ReadOnly`],
    /// [`DspiError::Disconnected`] or a transfer error.
    pub fn set_spdif_input_enabled(&self, index: u8, enabled: bool) -> Result<()> {
        self.set_with_status(
            op::REQ_SET_SPDIF_INPUT_ENABLE,
            wv(index, u8::from(enabled)),
            |s| {
                if (1..=3).contains(&index) {
                    let bit = 1u8.wrapping_shl(u32::from(index));
                    if enabled {
                        s.spdif_inputs_enabled |= bit;
                    } else {
                        s.spdif_inputs_enabled &= !bit;
                    }
                }
                s.spdif_inputs_enabled |= 1;
            },
        )
    }

    /// The data GPIO of I2S input stereo pair `pair` (0..3);
    /// `PIN_RESET_TO_DEFAULT` restores the default, read back.
    ///
    /// Errors: [`DspiError::Refused`] with `INVALID_OUTPUT` (no such pair),
    /// `INVALID_PIN` or `PIN_IN_USE`; [`DspiError::ReadOnly`],
    /// [`DspiError::Disconnected`], [`DspiError::Locked`] (Link, reset
    /// only) or a transfer error.
    pub fn set_i2s_rx_pin(&self, pair: u8, pin: u8) -> Result<()> {
        self.set_pin_with_status(
            op::REQ_SET_I2S_RX_PIN,
            wv(pair, pin),
            pin,
            (op::REQ_GET_I2S_RX_PIN, u16::from(pair)),
            |s, p| {
                if let Some(v) = entry(&mut s.i2s_rx_pins, pair) {
                    *v = p;
                }
            },
        )
    }

    /// I2S input channels: 2, 4, 6 or 8 (four pairs on RP2350, one on
    /// RP2040).
    ///
    /// Errors: [`DspiError::Refused`] with `INVALID_PIN` (not 2/4/6/8),
    /// `INVALID_OUTPUT` (more pairs than the part has) or a pin status for a
    /// newly active pair; [`DspiError::ReadOnly`],
    /// [`DspiError::Disconnected`] or a transfer error.
    pub fn set_i2s_input_channels(&self, channels: u8) -> Result<()> {
        self.set_with_status(op::REQ_SET_I2S_INPUT_CHANNELS, u16::from(channels), |s| {
            s.i2s_input_channels = channels;
        })
    }

    /// The ADAT input (RP2350) on or off. Enabling needs a data pin
    /// (`DspState.adat_input_pin` not 0xFF); only an enabled ADAT input can
    /// be selected as the source.
    ///
    /// Errors: [`DspiError::Refused`] with `INVALID_OUTPUT` (RP2040),
    /// `INVALID_PIN` (no pin set), or `PIN_IN_USE` (its pin is taken; or,
    /// disabling, it is the live source: switch away first);
    /// [`DspiError::ReadOnly`], [`DspiError::Disconnected`] or a transfer
    /// error.
    pub fn set_adat_input_enabled(&self, enabled: bool) -> Result<()> {
        self.set_with_status(op::REQ_SET_ADAT_INPUT_ENABLE, u16::from(enabled), |s| {
            s.adat_input_enabled = enabled;
        })
    }

    /// The ADAT input GPIO; 0xFF clears it (allowed only while the input is
    /// disabled). The ADAT output's pin is allowed (loopback self-test).
    ///
    /// Errors: [`DspiError::Refused`] with `INVALID_OUTPUT` (RP2040),
    /// `INVALID_PIN` or `PIN_IN_USE` (taken, or clearing while enabled);
    /// [`DspiError::ReadOnly`], [`DspiError::Disconnected`] or a transfer
    /// error.
    pub fn set_adat_input_pin(&self, pin: u8) -> Result<()> {
        self.set_with_status(op::REQ_SET_ADAT_INPUT_PIN, u16::from(pin), |s| {
            s.adat_input_pin = pin;
        })
    }

    /// ADAT input clock: 0 = master (the device sets the rate), 1 = slave
    /// (follows the lightpipe). Applied later in the firmware's main loop;
    /// read the live mode back with [`DspiCore::fetch_input_lock`] (1).
    ///
    /// Errors: [`DspiError::Refused`] with `INVALID_PARAM` (above 1);
    /// [`DspiError::ReadOnly`], [`DspiError::Disconnected`] or a transfer
    /// error.
    pub fn set_adat_input_clock_mode(&self, mode: u8) -> Result<()> {
        self.set_with_status(op::REQ_SET_ADAT_INPUT_CLOCK_MODE, u16::from(mode), |s| {
            s.adat_input_clock_mode = mode;
        })
    }

    /// I2S/ADAT-master input rate: 0 = 44.1k, 1 = 48k, 2 = 96k (others clamp
    /// to 48k). REQ_SET_INPUT_RATE 0xED is a payload SET with no status byte:
    /// Ok means the transfer completed. The firmware stores the rate and
    /// applies it live only while it is the rate authority (I2S or ADAT input
    /// in master clock mode). Sets `DspState.i2s_input_rate`.
    ///
    /// Errors: [`DspiError::ReadOnly`], [`DspiError::Disconnected`] or a
    /// transfer error.
    pub fn set_input_rate(&self, index: u8) -> Result<()> {
        let index = if index > 2 { 1 } else { index };
        let hz: u32 = match index {
            0 => 44_100,
            2 => 96_000,
            _ => 48_000,
        };
        self.set_payload(op::REQ_SET_INPUT_RATE, &hz.to_le_bytes(), |s| {
            s.i2s_input_rate = index;
        })
    }

    /// Hz the pipeline runs at now (0 if unknown): the LIVE rate (firmware
    /// `audio_state.freq`, REQ_GET_INPUT_RATE 0xEE word 0, same as status
    /// wValue 15) for every source, not the configured I2S rate (word 1,
    /// ignored; that one is `DspState.i2s_input_rate`).
    ///
    /// Errors: a reply shorter than 4 bytes or another transfer error;
    /// [`DspiError::Disconnected`].
    pub fn fetch_input_rate(&self) -> Result<u32> {
        self.read_with(|conn, _| {
            let d = read_min(conn, op::REQ_GET_INPUT_RATE, 0, 8, 4)?;
            Ok(u32_at(&d, 0))
        })
    }

    /// I2S clock mode, 0 master / 1 slave. REQ_SET_I2S_CLOCK_MODE 0x88 is a
    /// payload SET with no status byte (Ok = transfer completed); the
    /// firmware applies it deferred in its main loop, and notifies then.
    /// Sets `DspState.i2s_clock_mode`.
    ///
    /// Errors: [`DspiError::InvalidArgument`] for a mode above 1 (the
    /// firmware would ignore it silently); [`DspiError::ReadOnly`],
    /// [`DspiError::Disconnected`] or a transfer error.
    pub fn set_i2s_clock_mode(&self, mode: u8) -> Result<()> {
        if mode > 1 {
            return Err(DspiError::InvalidArgument {
                msg: format!("I2S clock mode {mode} is not 0 (master) or 1 (slave)"),
            });
        }
        self.set_payload(op::REQ_SET_I2S_CLOCK_MODE, &[mode], |s| {
            s.i2s_clock_mode = mode;
        })
    }

    /// Lock status of a clocked input. `which`: 0 = I2S slave
    /// (`REQ_GET_I2S_SLAVE_STATUS`, 16 bytes; `rate_ok` is always true),
    /// 1 = ADAT input (`REQ_GET_ADAT_INPUT_STATUS`, 20 bytes; all zero on
    /// RP2040).
    ///
    /// Errors: [`DspiError::InvalidArgument`] for another `which`; a short
    /// reply or other transfer error; [`DspiError::Disconnected`].
    pub fn fetch_input_lock(&self, which: u8) -> Result<InputLockStatus> {
        match which {
            0 => self.read_with(|conn, _| {
                let d = read_min(conn, op::REQ_GET_I2S_SLAVE_STATUS, 0, 16, 12)?;
                Ok(InputLockStatus {
                    state: byte(&d, 0),
                    clock_mode: byte(&d, 1),
                    detected_rate: u32_at(&d, 4),
                    measured_hz: u32_at(&d, 8),
                    rate_ok: true,
                    lock_count: byte(&d, 2),
                    loss_count: byte(&d, 3),
                })
            }),
            1 => self.read_with(|conn, _| {
                let d = read_min(conn, op::REQ_GET_ADAT_INPUT_STATUS, 0, 20, 20)?;
                Ok(InputLockStatus {
                    state: byte(&d, 0),
                    clock_mode: byte(&d, 1),
                    detected_rate: u32_at(&d, 12),
                    measured_hz: u32_at(&d, 16),
                    rate_ok: byte(&d, 4) != 0,
                    lock_count: byte(&d, 5),
                    loss_count: byte(&d, 6),
                })
            }),
            _ => Err(DspiError::InvalidArgument {
                msg: format!("input lock {which} is not 0 (I2S slave) or 1 (ADAT)"),
            }),
        }
    }

    /// The ADAT output's status (8 bytes; all zero on RP2040).
    ///
    /// Errors: a short reply or other transfer error;
    /// [`DspiError::Disconnected`].
    pub fn fetch_adat_out_status(&self) -> Result<AdatOutStatus> {
        self.read_with(|conn, _| {
            let d = read_min(conn, op::REQ_GET_ADAT_STATUS, 0, 8, 8)?;
            Ok(AdatOutStatus {
                enabled: byte(&d, 0) != 0,
                active: byte(&d, 1) != 0,
                pin: byte(&d, 2),
                rate_ok: byte(&d, 3) != 0,
                resync_count: u16_at(&d, 4),
                slip_count: u16_at(&d, 6),
            })
        })
    }

    // ── DAC hardware mute ─────────────────────────────────────────────

    /// Send a DAC hardware mute config: on/off, polarity (`active_low`:
    /// pull the pin low to mute), GPIO, and the hold times in ms around a
    /// clock stop. The firmware validates and applies it later in its main
    /// loop (with a flash write) and drops an invalid one silently: `Ok`
    /// means only that the transfer completed. Read the outcome back with
    /// [`DspiCore::fetch_dac_mute`] about 150 ms later; the mirror is not
    /// changed here.
    ///
    /// Errors: [`DspiError::ReadOnly`], [`DspiError::Disconnected`] or a
    /// transfer error.
    pub fn set_dac_mute(
        &self,
        enabled: bool,
        active_low: bool,
        pin: u8,
        hold_ms: u16,
        release_ms: u16,
    ) -> Result<()> {
        let [h0, h1] = hold_ms.to_le_bytes();
        let [r0, r1] = release_ms.to_le_bytes();
        let mut d = [0u8; DAC_MUTE_CONFIG_LEN];
        let fields = [
            u8::from(enabled),
            u8::from(active_low),
            pin,
            0,
            h0,
            h1,
            r0,
            r1,
        ];
        for (dst, src) in d.iter_mut().zip(fields) {
            *dst = src;
        }
        self.set_payload(op::REQ_SET_DAC_HW_MUTE_CONFIG, &d, |_| {})
    }

    /// Re-read the DAC mute config into the state (`dac_mute_*`; also the
    /// support probe: `dac_mute_supported` becomes true when it answers).
    ///
    /// Errors: a stall (firmware without the feature), a short reply or
    /// another transfer error; [`DspiError::Disconnected`]. The mirror is
    /// unchanged on error.
    pub fn fetch_dac_mute(&self) -> Result<()> {
        self.read_with(connect::read_dac_mute)
    }

    /// Pulse the mute output for about a second, so an installer can hear
    /// that the pin and polarity are right.
    ///
    /// Errors: [`DspiError::Refused`] { 3 } (`INVALID_OUTPUT`) when the DAC
    /// mute is disabled; [`DspiError::ReadOnly`],
    /// [`DspiError::Disconnected`] or a transfer error.
    pub fn test_dac_mute(&self) -> Result<()> {
        let mut guard = self.lock();
        let mut w = guard.writable_conn()?;
        status(w.set_as_read(op::REQ_TEST_DAC_HW_MUTE, 0)?)
    }

    // ── LG Sound Sync ─────────────────────────────────────────────────

    /// LG Sound Sync on or off: live, saved with the preset. Sets
    /// `DspState.lg_sound_sync_enabled`.
    ///
    /// Errors: [`DspiError::ReadOnly`], [`DspiError::Disconnected`] or a
    /// transfer error.
    pub fn set_lg_sound_sync(&self, enabled: bool) -> Result<()> {
        self.set_payload(
            op::REQ_SET_LG_SOUND_SYNC_ENABLE,
            &[u8::from(enabled)],
            |s| {
                s.lg_sound_sync_enabled = enabled;
            },
        )
    }

    /// LG Sound Sync status: enabled, a TV detected, its last volume
    /// (0..100, 0xFF before one is decoded) and mute.
    ///
    /// Errors: a short reply or other transfer error;
    /// [`DspiError::Disconnected`].
    pub fn fetch_lg_status(&self) -> Result<LgStatus> {
        self.read_with(|conn, _| {
            let d = read_min(conn, op::REQ_GET_LG_SOUND_SYNC_STATUS, 0, 16, 4)?;
            Ok(LgStatus {
                enabled: byte(&d, 0) != 0,
                present: byte(&d, 1) != 0,
                volume: byte(&d, 2),
                muted: byte(&d, 3) != 0,
            })
        })
    }

    // ── Control interfaces ────────────────────────────────────────────

    /// Re-read UART/I2C configs + status into the state (`uart`, `i2c`,
    /// `ctrl_status`, `ctrl_iface_supported`). A device that does not
    /// answer the status read has no control interfaces:
    /// `ctrl_iface_supported` becomes false and the error is returned.
    ///
    /// Errors: a stall, a short reply or another transfer error;
    /// [`DspiError::Disconnected`].
    pub fn fetch_ctrl_ifaces(&self) -> Result<()> {
        self.read_with(connect::read_ctrl_ifaces)
    }

    /// Send a UART control-interface config (pins, baud, notifications).
    /// The firmware applies and persists it later; read the outcome
    /// (`ctrl_status.uart_last_status`, a `PIN_CONFIG_*` code) with
    /// [`DspiCore::fetch_ctrl_ifaces`] about 250 ms later. The mirror is not
    /// changed here.
    ///
    /// Errors: [`DspiError::ReadOnly`], [`DspiError::Disconnected`] or a
    /// transfer error.
    pub fn set_uart(&self, config: UartConfig) -> Result<()> {
        let [b0, b1, b2, b3] = config.baud.to_le_bytes();
        let d: [u8; CTRL_CONFIG_LEN] = [
            u8::from(config.enabled),
            config.tx_pin,
            config.rx_pin,
            u8::from(config.notify),
            b0,
            b1,
            b2,
            b3,
        ];
        self.set_payload(op::REQ_SET_UART_CONFIG, &d, |_| {})
    }

    /// Send an I2C target control-interface config (pins, 7-bit address).
    /// Applied and persisted later, as [`DspiCore::set_uart`]; read the
    /// outcome with [`DspiCore::fetch_ctrl_ifaces`].
    ///
    /// Errors: [`DspiError::ReadOnly`], [`DspiError::Disconnected`] or a
    /// transfer error.
    pub fn set_i2c(&self, config: I2cConfig) -> Result<()> {
        let d: [u8; CTRL_CONFIG_LEN] = [
            u8::from(config.enabled),
            config.sda_pin,
            config.scl_pin,
            config.address,
            0,
            0,
            0,
            0,
        ];
        self.set_payload(op::REQ_SET_I2C_CONFIG, &d, |_| {})
    }

    // ── Bootloader, Core 1 ────────────────────────────────────────────

    /// Reboots the device into its USB bootloader (it disappears). Always Ok,
    /// as in the core: the transfer result is ignored because the device
    /// drops off the bus (also Ok without a device). Allowed on a read-only
    /// connection, so a firmware too old or too new for this build can be
    /// updated.
    ///
    /// The connection stays open until the departure is noticed (hotplug or
    /// notifications). A firmware update then goes: this call; the
    /// bootloader drive appears (`bootloader_drives`); `flash_to_drive`
    /// writes the image; the DSPi re-enumerates. Only that last step
    /// confirms success: open it again and check the new version with
    /// [`DspiCore::firmware_version_matches`]; a timeout there is the
    /// failure.
    pub fn enter_bootloader(&self) -> Result<()> {
        let mut guard = self.lock();
        if let Some(conn) = guard.conn.as_mut()
            && let Err(e) = conn.get_upto(op::REQ_ENTER_BOOTLOADER, 0, 1)
        {
            log::debug!("enter_bootloader: {e} (expected while the device reboots)");
        }
        Ok(())
    }

    /// Whether the open device runs firmware `version`: the leading
    /// `major.minor.patch[-betaN]` of a UF2's `Uf2Info::firmware_version`
    /// (`1.1.6-beta3-16-g557bce7` compares as 1.1.6 beta 3), against the
    /// mirror's `fw_*` from `GET_PLATFORM`. The last step of a firmware
    /// update, after the device re-enumerated and was opened again. False
    /// for a string that does not start with a version; an early 1.1.6 beta
    /// that does not report its number matches betas 1 and 2.
    ///
    /// Errors: [`DspiError::Disconnected`] with no device open.
    pub fn firmware_version_matches(&self, version: &str) -> Result<bool> {
        let mut guard = self.lock();
        guard.conn()?;
        Ok(version_matches(&guard.state, version))
    }

    /// 0 idle, 1 PDM, 2 EQ worker. Also stored in `DspState.core1_mode`.
    ///
    /// Errors: a stall or other transfer error;
    /// [`DspiError::Disconnected`].
    pub fn fetch_core1_mode(&self) -> Result<u8> {
        self.read_with(connect::read_core1_mode)
    }

    /// Whether enabling `output` would conflict with Core 1's mode (PDM on
    /// the last output and the Core 1 EQ worker exclude each other, and the
    /// firmware skips a blocked enable silently). Asked through the
    /// session's `core1_conflict`, which reports a failed read as no
    /// conflict.
    ///
    /// Errors: [`DspiError::InvalidArgument`] for an output the device does
    /// not have; [`DspiError::Disconnected`] with no device, or one whose
    /// image could not be mapped (no session).
    pub fn check_core1_conflict(&self, output: u8) -> Result<bool> {
        let mut guard = self.lock();
        let session = guard.conn()?.session()?;
        let outputs = session.channel_map().num_outputs();
        if output >= outputs {
            return Err(DspiError::InvalidArgument {
                msg: format!("output {output} is out of range (0..{outputs})"),
            });
        }
        Ok(session.core1_conflict(output))
    }
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_c6 {
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use dspi_proto::generated::status as st;
    use dspi_proto::wire_compat::WireRead;
    use dspi_transport::{
        Capture, DeviceDescriptor, FakeDevice, FakeOptions, FakeRemote, NotificationSource,
        Transport, TransportCaps,
    };
    use proptest::prelude::*;

    use super::*;

    const INVALID_PIN: u8 = st::PIN_CONFIG_INVALID_PIN as u8;
    const PIN_IN_USE: u8 = st::PIN_CONFIG_PIN_IN_USE as u8;
    const INVALID_OUTPUT: u8 = st::PIN_CONFIG_INVALID_OUTPUT as u8;
    const OUTPUT_ACTIVE: u8 = st::PIN_CONFIG_OUTPUT_ACTIVE as u8;

    type Log = Arc<Mutex<Vec<String>>>;

    /// A `FakeDevice` with scripted IN replies for the opcodes it does not
    /// model (`None`: stall), a log of every transfer
    /// (`"in|out <opcode> <wValue> <payload hex>"`) and of the lock calls,
    /// optionally refusing the Link device lock.
    struct Rig {
        inner: FakeDevice,
        script: HashMap<u8, Option<Vec<u8>>>,
        log: Log,
        refuse_lock: bool,
    }

    impl Rig {
        fn note(&self, s: String) {
            self.log.lock().unwrap().push(s);
        }
        fn scripted(&self, op: u8) -> Option<dspi_transport::Result<Vec<u8>>> {
            self.script.get(&op).map(|r| match r {
                Some(b) => Ok(b.clone()),
                None => Err(TransportError::Stalled { opcode: op }),
            })
        }
    }

    impl Transport for Rig {
        fn control_in(&mut self, op: u8, value: u16, len: u16) -> dspi_transport::Result<Vec<u8>> {
            self.note(format!("in {op:02X} {value}"));
            match self.scripted(op) {
                Some(r) => r,
                None => self.inner.control_in(op, value, len),
            }
        }
        fn control_in_upto(
            &mut self,
            op: u8,
            value: u16,
            len: u16,
        ) -> dspi_transport::Result<Vec<u8>> {
            self.note(format!("in {op:02X} {value}"));
            match self.scripted(op) {
                Some(r) => r.map(|mut b| {
                    b.truncate(len.into());
                    b
                }),
                None => self.inner.control_in_upto(op, value, len),
            }
        }
        fn control_out(&mut self, op: u8, value: u16, data: &[u8]) -> dspi_transport::Result<()> {
            let hex: String = data.iter().map(|b| format!("{b:02x}")).collect();
            self.note(format!("out {op:02X} {value} {hex}"));
            self.inner.control_out(op, value, data)
        }
        fn descriptor(&self) -> &DeviceDescriptor {
            self.inner.descriptor()
        }
        fn max_transfer(&self) -> usize {
            self.inner.max_transfer()
        }
        fn notifications(&self) -> Option<Box<dyn NotificationSource>> {
            None
        }
        fn caps(&self) -> TransportCaps {
            self.inner.caps()
        }
        fn lock(&mut self, reason: &str, _timeout: Duration) -> dspi_transport::Result<()> {
            self.note(format!("lock {reason}"));
            if self.refuse_lock {
                Err(TransportError::Locked)
            } else {
                Ok(())
            }
        }
        fn unlock(&mut self) {
            self.note("unlock".into());
        }
    }

    fn decoded(image: &[u8]) -> DspState {
        let mut s = state::default_state();
        state::decode_bulk(image, &mut s).unwrap();
        s
    }

    /// The demo capture with `edit` applied to its decoded state.
    fn capture_with(edit: impl FnOnce(&mut DspState)) -> Capture {
        let demo = Capture::demo();
        let mut s = decoded(&demo.bulk);
        edit(&mut s);
        Capture {
            platform: demo.platform,
            bulk: state::encode_bulk(&s),
        }
    }

    /// A probed core on `dev`, behind a `Rig` with `script`.
    fn rig_on(
        dev: FakeDevice,
        script: &[(u8, Option<Vec<u8>>)],
        refuse_lock: bool,
    ) -> (Arc<DspiCore>, FakeRemote, Log) {
        let remote = dev.remote();
        let log: Log = Arc::default();
        let rig = Rig {
            inner: dev,
            script: script.iter().cloned().collect(),
            log: Arc::clone(&log),
            refuse_lock,
        };
        let core = DspiCore::new();
        core.attach_probed(Box::new(rig), "DEMO".into(), 0).unwrap();
        log.lock().unwrap().clear();
        (core, remote, log)
    }

    fn rig(script: &[(u8, Option<Vec<u8>>)]) -> (Arc<DspiCore>, FakeRemote, Log) {
        rig_on(FakeDevice::demo(), script, false)
    }

    fn entries(log: &Log) -> Vec<String> {
        log.lock().unwrap().clone()
    }

    /// The mirror's own image agrees with its decoded fields.
    fn assert_image_in_step(core: &DspiCore) {
        let s = core.get_state();
        assert!(s.bulk_valid);
        let again = decoded(&s.bulk_raw);
        assert_eq!(state::encode_bulk(&again), s.bulk_raw);
        assert_eq!(state::encode_bulk(&s), s.bulk_raw);
    }

    // ── Write-as-read setters on the fake ─────────────────────────────

    #[test]
    fn wp_c6_output_type_round_trips() {
        let (core, remote, _) = rig(&[]);
        core.set_output_type(1, 1).unwrap();
        assert_eq!(core.get_state().output_types[1], 1);
        assert_eq!(decoded(&remote.image()).output_types[1], 1);
        assert_eq!(decoded(&core.get_state().bulk_raw).output_types[1], 1);
        assert_image_in_step(&core);
    }

    #[test]
    fn wp_c6_output_type_refusals_are_typed_and_leave_the_mirror() {
        let (core, remote, _) = rig(&[]);
        let before = core.get_state();
        assert_eq!(
            core.set_output_type(9, 1),
            Err(DspiError::Refused {
                status: INVALID_OUTPUT
            })
        );
        assert_eq!(
            core.set_output_type(0, 2),
            Err(DspiError::Refused {
                status: INVALID_PIN
            })
        );
        assert_eq!(core.get_state().output_types, before.output_types);
        assert_eq!(core.get_state().bulk_raw, before.bulk_raw);
        assert_eq!(decoded(&remote.image()).output_types, before.output_types);
    }

    #[test]
    fn wp_c6_mck_and_adat_out_follow_the_device() {
        let (core, remote, _) = rig(&[]);
        core.set_mck_enabled(true).unwrap();
        core.set_mck_multiplier(1).unwrap();
        core.set_adat_out_enabled(true).unwrap();
        let dev = decoded(&remote.image());
        let s = core.get_state();
        assert!(s.mck_enabled && dev.mck_enabled);
        assert_eq!((s.mck_multiplier, dev.mck_multiplier), (1, 1));
        assert!(s.adat_out_enabled && dev.adat_out_enabled);
        assert_image_in_step(&core);
        // The fake refuses a multiplier above 1 as the firmware does.
        assert_eq!(
            core.set_mck_multiplier(2),
            Err(DspiError::Refused {
                status: INVALID_PIN
            })
        );
        assert_eq!(core.get_state().mck_multiplier, 1);
    }

    #[test]
    fn wp_c6_i2s_input_channels_validate_on_the_device() {
        let (core, remote, _) = rig(&[]);
        core.set_i2s_input_channels(4).unwrap();
        assert_eq!(core.get_state().i2s_input_channels, 4);
        assert_eq!(decoded(&remote.image()).i2s_input_channels, 4);
        assert_eq!(
            core.set_i2s_input_channels(3),
            Err(DspiError::Refused {
                status: INVALID_PIN
            })
        );
        assert_eq!(core.get_state().i2s_input_channels, 4);
    }

    #[test]
    fn wp_c6_spdif_enable_keeps_input_one_on() {
        let (core, remote, _) = rig(&[]);
        assert_eq!(core.get_state().spdif_inputs_enabled & 1, 1);
        core.set_spdif_input_enabled(2, true).unwrap();
        let s = core.get_state();
        assert_eq!(s.spdif_inputs_enabled & 0b101, 0b101);
        assert_eq!(
            decoded(&remote.image()).spdif_inputs_enabled,
            s.spdif_inputs_enabled
        );
        // Input 0 cannot be turned off; bit 0 stays set everywhere.
        assert_eq!(
            core.set_spdif_input_enabled(0, false),
            Err(DspiError::Refused {
                status: INVALID_OUTPUT
            })
        );
        core.set_spdif_input_enabled(2, false).unwrap();
        let s = core.get_state();
        assert_eq!(s.spdif_inputs_enabled & 0b101, 0b001);
        assert_eq!(
            decoded(&s.bulk_raw).spdif_inputs_enabled,
            s.spdif_inputs_enabled
        );
        assert_eq!(
            decoded(&remote.image()).spdif_inputs_enabled,
            s.spdif_inputs_enabled
        );
    }

    #[test]
    fn wp_c6_spdif_enable_cannot_disable_the_live_source() {
        let mut dev = FakeDevice::demo();
        // Enable S/PDIF input 3 (index 2) and make it the live source.
        assert_eq!(
            dev.control_in(op::REQ_SET_SPDIF_INPUT_ENABLE, wv(2, 1), 1)
                .unwrap(),
            [0]
        );
        dev.control_out(op::REQ_SET_INPUT_SOURCE, 0, &[5]).unwrap();
        let (core, remote, _) = rig_on(dev, &[], false);
        assert_eq!(core.get_state().input_source, 5);
        assert_eq!(
            core.set_spdif_input_enabled(2, false),
            Err(DspiError::Refused { status: PIN_IN_USE })
        );
        // Proven by read-back: still enabled on the device and the mirror.
        assert_ne!(decoded(&remote.image()).spdif_inputs_enabled & 0b100, 0);
        assert_ne!(core.get_state().spdif_inputs_enabled & 0b100, 0);
    }

    #[test]
    fn wp_c6_adat_input_needs_a_pin() {
        let dev = FakeDevice::new(
            capture_with(|s| {
                s.adat_input_pin = 0xFF;
                s.adat_input_enabled = false;
            }),
            FakeOptions::default(),
        );
        let (core, remote, _) = rig_on(dev, &[], false);
        assert_eq!(
            core.set_adat_input_enabled(true),
            Err(DspiError::Refused {
                status: INVALID_PIN
            })
        );
        assert!(!core.get_state().adat_input_enabled);
        assert!(!decoded(&remote.image()).adat_input_enabled);

        let dev = FakeDevice::new(
            capture_with(|s| {
                s.adat_input_pin = 20;
                s.adat_input_enabled = false;
            }),
            FakeOptions::default(),
        );
        let (core, remote, _) = rig_on(dev, &[], false);
        core.set_adat_input_enabled(true).unwrap();
        assert!(core.get_state().adat_input_enabled);
        assert!(decoded(&remote.image()).adat_input_enabled);
        assert_image_in_step(&core);
        core.set_adat_input_clock_mode(1).unwrap();
        assert_eq!(core.get_state().adat_input_clock_mode, 1);
        assert_eq!(decoded(&remote.image()).adat_input_clock_mode, 1);
    }

    #[test]
    fn wp_c6_adat_input_pin_clears_to_unset() {
        let (core, _, log) = rig(&[(op::REQ_SET_ADAT_INPUT_PIN, Some(vec![0]))]);
        core.set_adat_input_pin(20).unwrap();
        assert_eq!(core.get_state().adat_input_pin, 20);
        core.set_adat_input_pin(0xFF).unwrap();
        let s = core.get_state();
        assert_eq!(s.adat_input_pin, 0xFF);
        // 0xFF goes into the image as the firmware's "unset" 0 and decodes
        // back as 0xFF.
        assert_eq!(decoded(&s.bulk_raw).adat_input_pin, 0xFF);
        // Clearing is a plain set: no read-back, no lock.
        assert_eq!(
            entries(&log),
            vec![
                format!("in {:02X} 20", op::REQ_SET_ADAT_INPUT_PIN),
                format!("in {:02X} 255", op::REQ_SET_ADAT_INPUT_PIN),
            ]
        );
    }

    // ── Pin setters (not modelled by the fake: scripted) ──────────────

    #[test]
    fn wp_c6_output_pin_packs_wvalue_and_updates_the_mirror() {
        let (core, _, log) = rig(&[(op::REQ_SET_OUTPUT_PIN, Some(vec![0]))]);
        core.set_output_pin(1, 12).unwrap();
        let s = core.get_state();
        assert_eq!(s.output_pins[1], 12);
        assert_eq!(decoded(&s.bulk_raw).output_pins[1], 12);
        assert_eq!(
            entries(&log),
            vec![format!(
                "in {:02X} {}",
                op::REQ_SET_OUTPUT_PIN,
                (12 << 8) | 1
            )]
        );
    }

    #[test]
    fn wp_c6_pin_status_codes_map_to_refused() {
        for code in [INVALID_PIN, PIN_IN_USE, INVALID_OUTPUT, OUTPUT_ACTIVE] {
            let (core, _, _) = rig(&[(op::REQ_SET_OUTPUT_PIN, Some(vec![code]))]);
            let before = core.get_state();
            assert_eq!(
                core.set_output_pin(4, 3),
                Err(DspiError::Refused { status: code })
            );
            let after = core.get_state();
            assert_eq!(after.output_pins, before.output_pins);
            assert_eq!(after.bulk_raw, before.bulk_raw);
        }
    }

    #[test]
    fn wp_c6_empty_status_reply_is_an_error_not_a_panic() {
        let (core, _, _) = rig(&[(op::REQ_SET_MCK_PIN, Some(vec![]))]);
        assert!(matches!(
            core.set_mck_pin(21),
            Err(DspiError::Transfer { .. })
        ));
    }

    #[test]
    fn wp_c6_pin_reset_reads_the_default_back_under_the_link_lock() {
        let (core, _, log) = rig(&[
            (op::REQ_SET_OUTPUT_PIN, Some(vec![0])),
            (op::REQ_GET_OUTPUT_PIN, Some(vec![7])),
        ]);
        core.set_output_pin(2, PIN_RESET_TO_DEFAULT).unwrap();
        assert_eq!(core.get_state().output_pins[2], 7);
        assert_eq!(
            entries(&log),
            vec![
                "lock Resetting a pin to its default".to_string(),
                format!("in {:02X} {}", op::REQ_SET_OUTPUT_PIN, (0xFF << 8) | 2),
                format!("in {:02X} 2", op::REQ_GET_OUTPUT_PIN),
                "unlock".to_string(),
            ]
        );
    }

    #[test]
    fn wp_c6_pin_reset_with_a_refused_lock_sends_nothing() {
        let (core, _, log) = rig_on(
            FakeDevice::demo(),
            &[(op::REQ_SET_SPDIF_RX_PIN, Some(vec![0]))],
            true,
        );
        let before = core.get_state();
        assert_eq!(
            core.set_spdif_rx_pin(1, PIN_RESET_TO_DEFAULT),
            Err(DspiError::Locked)
        );
        assert_eq!(
            entries(&log),
            vec!["lock Resetting a pin to its default".to_string()]
        );
        assert_eq!(core.get_state().spdif_rx_pins, before.spdif_rx_pins);
    }

    #[test]
    fn wp_c6_refused_reset_unlocks_and_skips_the_read_back() {
        let (core, _, log) = rig(&[(op::REQ_SET_I2S_RX_PIN, Some(vec![PIN_IN_USE]))]);
        assert_eq!(
            core.set_i2s_rx_pin(1, PIN_RESET_TO_DEFAULT),
            Err(DspiError::Refused { status: PIN_IN_USE })
        );
        let log = entries(&log);
        assert_eq!(log.len(), 3, "{log:?}");
        assert_eq!(log.last().map(String::as_str), Some("unlock"));
    }

    #[test]
    fn wp_c6_reset_defaults_read_back_from_the_fake() {
        // SETs scripted; the fake answers the GETs from its image.
        let (core, remote, _) = rig(&[
            (op::REQ_SET_SPDIF_RX_PIN, Some(vec![0])),
            (op::REQ_SET_I2S_RX_PIN, Some(vec![0])),
            (op::REQ_SET_MCK_PIN, Some(vec![0])),
            (op::REQ_SET_ADAT_PIN, Some(vec![0])),
            (op::REQ_SET_I2S_BCK_PIN, Some(vec![0])),
        ]);
        let dev = decoded(&remote.image());
        // Scramble the mirror so the read-back is what puts it right.
        {
            let mut inner = core.lock();
            let s = &mut inner.state;
            s.spdif_rx_pins[1] = 99;
            s.i2s_rx_pins[0] = 99;
            s.mck_pin = 99;
            s.adat_out_pin = 99;
            s.i2s_bck_pin_slave = 99;
        }
        core.set_spdif_rx_pin(1, PIN_RESET_TO_DEFAULT).unwrap();
        core.set_i2s_rx_pin(0, PIN_RESET_TO_DEFAULT).unwrap();
        core.set_mck_pin(PIN_RESET_TO_DEFAULT).unwrap();
        core.set_adat_out_pin(PIN_RESET_TO_DEFAULT).unwrap();
        core.set_i2s_bck_pin(1, PIN_RESET_TO_DEFAULT).unwrap();
        let s = core.get_state();
        assert_eq!(s.spdif_rx_pins[1], dev.spdif_rx_pins[1]);
        assert_eq!(s.i2s_rx_pins[0], dev.i2s_rx_pins[0]);
        assert_eq!(s.mck_pin, dev.mck_pin);
        assert_eq!(s.adat_out_pin, dev.adat_out_pin);
        assert_eq!(s.i2s_bck_pin_slave, dev.i2s_bck_pin_slave);
    }

    #[test]
    fn wp_c6_bck_roles_and_clock_pin_mode() {
        let (core, _, log) = rig(&[
            (op::REQ_SET_I2S_BCK_PIN, Some(vec![0])),
            (op::REQ_SET_I2S_CLOCK_PIN_MODE, Some(vec![0])),
        ]);
        core.set_i2s_bck_pin(0, 14).unwrap();
        core.set_i2s_bck_pin(1, 26).unwrap();
        core.set_i2s_clock_pin_mode(1).unwrap();
        let s = core.get_state();
        assert_eq!((s.i2s_bck_pin, s.i2s_bck_pin_slave), (14, 26));
        assert_eq!(s.i2s_clock_pin_mode, 1);
        assert_eq!(
            entries(&log)[..2],
            [
                format!("in {:02X} 14", op::REQ_SET_I2S_BCK_PIN),
                format!("in {:02X} {}", op::REQ_SET_I2S_BCK_PIN, (1 << 8) | 26),
            ]
        );
    }

    #[test]
    fn wp_c6_fetch_output_pin_updates_the_mirror() {
        let (core, _, _) = rig(&[(op::REQ_GET_OUTPUT_PIN, Some(vec![17]))]);
        assert_eq!(core.fetch_output_pin(3), Ok(17));
        assert_eq!(core.get_state().output_pins[3], 17);
        assert_image_in_step(&core);
    }

    // ── Payload SETs: proven by read-back ─────────────────────────────

    #[test]
    fn wp_c6_input_rate_clamps_and_reaches_the_device() {
        let (core, remote, log) = rig(&[]);
        core.set_input_rate(2).unwrap();
        assert_eq!(core.get_state().i2s_input_rate, 2);
        assert_eq!(decoded(&remote.image()).i2s_input_rate, 2);
        core.set_input_rate(7).unwrap();
        assert_eq!(core.get_state().i2s_input_rate, 1);
        assert_eq!(decoded(&remote.image()).i2s_input_rate, 1);
        assert!(entries(&log).contains(&format!("out {:02X} 0 80bb0000", op::REQ_SET_INPUT_RATE)));
        // The live rate (word 0), not the configured one.
        assert_eq!(core.fetch_input_rate(), Ok(48_000));
    }

    #[test]
    fn wp_c6_i2s_clock_mode_validates_and_reaches_the_device() {
        let (core, remote, _) = rig(&[]);
        core.set_i2s_clock_mode(1).unwrap();
        assert_eq!(core.get_state().i2s_clock_mode, 1);
        assert_eq!(decoded(&remote.image()).i2s_clock_mode, 1);
        assert!(matches!(
            core.set_i2s_clock_mode(2),
            Err(DspiError::InvalidArgument { .. })
        ));
        assert_eq!(decoded(&remote.image()).i2s_clock_mode, 1);
    }

    #[test]
    fn wp_c6_output_config_mode() {
        let (core, _, log) = rig(&[]);
        core.set_output_config_mode(1).unwrap();
        assert_eq!(core.get_state().output_config_mode, 1);
        assert!(matches!(
            core.set_output_config_mode(2),
            Err(DspiError::InvalidArgument { .. })
        ));
        assert_eq!(
            entries(&log),
            vec![format!("out {:02X} 0 01", op::REQ_SET_OUTPUT_CONFIG_MODE)]
        );
    }

    #[test]
    fn wp_c6_dac_mute_set_then_read_back() {
        let (core, remote, log) = rig(&[]);
        core.set_dac_mute(true, false, 13, 20, 300).unwrap();
        assert_eq!(
            entries(&log),
            vec![format!(
                "out {:02X} 0 01000d0014002c010000000000000000",
                op::REQ_SET_DAC_HW_MUTE_CONFIG
            )]
        );
        core.fetch_dac_mute().unwrap();
        let s = core.get_state();
        assert!(s.dac_mute_supported && s.dac_mute_enabled && !s.dac_mute_active_low);
        assert_eq!(
            (s.dac_mute_pin, s.dac_mute_hold_ms, s.dac_mute_release_ms),
            (13, 20, 300)
        );
        let dev = decoded(&remote.image());
        assert_eq!((dev.dac_mute_pin, dev.dac_mute_release_ms), (13, 300));
        assert_eq!(decoded(&s.bulk_raw).dac_mute_pin, 13);
    }

    #[test]
    fn wp_c6_lg_sound_sync() {
        let (core, remote, _) = rig(&[]);
        core.set_lg_sound_sync(true).unwrap();
        assert!(core.get_state().lg_sound_sync_enabled);
        assert!(decoded(&remote.image()).lg_sound_sync_enabled);
        assert_image_in_step(&core);
        let st = core.fetch_lg_status().unwrap();
        assert!(st.enabled);
    }

    #[test]
    fn wp_c6_control_interfaces_set_then_read_back() {
        let (core, _, log) = rig(&[(
            op::REQ_GET_CTRL_IFACE_STATUS,
            Some(vec![0, 1, 0, 0, 1, 0, 0, 0]),
        )]);
        let uart = UartConfig {
            enabled: true,
            tx_pin: 4,
            rx_pin: 5,
            notify: true,
            baud: 115_200,
        };
        let i2c = I2cConfig {
            enabled: true,
            sda_pin: 2,
            scl_pin: 3,
            address: 0x42,
        };
        core.set_uart(uart).unwrap();
        core.set_i2c(i2c).unwrap();
        assert_eq!(
            entries(&log),
            vec![
                format!("out {:02X} 0 0104050100c20100", op::REQ_SET_UART_CONFIG),
                format!("out {:02X} 0 0102034200000000", op::REQ_SET_I2C_CONFIG),
            ]
        );
        core.fetch_ctrl_ifaces().unwrap();
        let s = core.get_state();
        assert!(s.ctrl_iface_supported);
        assert_eq!(s.uart, uart);
        assert_eq!(s.i2c, i2c);
        assert!(s.ctrl_status.uart_live);
        assert_eq!(s.ctrl_status.protocol_version, 1);
    }

    #[test]
    fn wp_c6_control_interfaces_absent() {
        // The fake stalls the status read: no control interfaces.
        let (core, _, _) = rig(&[]);
        core.lock().state.ctrl_iface_supported = true;
        assert!(core.fetch_ctrl_ifaces().is_err());
        assert!(!core.get_state().ctrl_iface_supported);
    }

    // ── Status-only commands ──────────────────────────────────────────

    #[test]
    fn wp_c6_save_output_config_runs_under_the_lock() {
        let (core, _, log) = rig(&[(op::REQ_SAVE_OUTPUT_CONFIG, Some(vec![0]))]);
        core.save_output_config().unwrap();
        assert_eq!(
            entries(&log),
            vec![
                "lock Saving output configuration".to_string(),
                format!("in {:02X} 0", op::REQ_SAVE_OUTPUT_CONFIG),
                "unlock".to_string(),
            ]
        );
        let (core, _, _) = rig(&[(op::REQ_SAVE_OUTPUT_CONFIG, Some(vec![4]))]);
        assert_eq!(
            core.save_output_config(),
            Err(DspiError::Refused { status: 4 })
        );
        let (core, _, _) = rig_on(FakeDevice::demo(), &[], true);
        assert_eq!(core.save_output_config(), Err(DspiError::Locked));
    }

    #[test]
    fn wp_c6_test_dac_mute_status() {
        let (core, _, _) = rig(&[(op::REQ_TEST_DAC_HW_MUTE, Some(vec![INVALID_OUTPUT]))]);
        assert_eq!(
            core.test_dac_mute(),
            Err(DspiError::Refused {
                status: INVALID_OUTPUT
            })
        );
        let (core, _, _) = rig(&[(op::REQ_TEST_DAC_HW_MUTE, Some(vec![0]))]);
        assert_eq!(core.test_dac_mute(), Ok(()));
    }

    // ── Status reads ──────────────────────────────────────────────────

    #[test]
    fn wp_c6_input_lock_packets() {
        let mut i2s = vec![3, 1, 5, 2];
        i2s.extend(48_000u32.to_le_bytes());
        i2s.extend(48_013u32.to_le_bytes());
        i2s.extend([0; 4]);
        let mut adat = vec![2, 1, 1, 20, 0, 9, 8, 0, 0, 0, 0, 0];
        adat.extend(44_100u32.to_le_bytes());
        adat.extend(44_099u32.to_le_bytes());
        let (core, _, _) = rig(&[
            (op::REQ_GET_I2S_SLAVE_STATUS, Some(i2s)),
            (op::REQ_GET_ADAT_INPUT_STATUS, Some(adat)),
        ]);
        assert_eq!(
            core.fetch_input_lock(0),
            Ok(InputLockStatus {
                state: 3,
                clock_mode: 1,
                detected_rate: 48_000,
                measured_hz: 48_013,
                rate_ok: true,
                lock_count: 5,
                loss_count: 2,
            })
        );
        assert_eq!(
            core.fetch_input_lock(1),
            Ok(InputLockStatus {
                state: 2,
                clock_mode: 1,
                detected_rate: 44_100,
                measured_hz: 44_099,
                rate_ok: false,
                lock_count: 9,
                loss_count: 8,
            })
        );
        assert!(matches!(
            core.fetch_input_lock(2),
            Err(DspiError::InvalidArgument { .. })
        ));
    }

    #[test]
    fn wp_c6_short_status_replies_are_errors() {
        let (core, _, _) = rig(&[
            (op::REQ_GET_I2S_SLAVE_STATUS, Some(vec![3; 11])),
            (op::REQ_GET_ADAT_INPUT_STATUS, Some(vec![3; 19])),
            (op::REQ_GET_ADAT_STATUS, Some(vec![1; 7])),
            (op::REQ_GET_LG_SOUND_SYNC_STATUS, Some(vec![1; 3])),
            (op::REQ_GET_INPUT_RATE, Some(vec![1; 3])),
            (op::REQ_GET_CORE1_MODE, Some(vec![])),
            (op::REQ_GET_OUTPUT_PIN, Some(vec![])),
        ]);
        let transfer = |r: std::result::Result<(), DspiError>| {
            assert!(matches!(r, Err(DspiError::Transfer { .. })), "{r:?}");
        };
        transfer(core.fetch_input_lock(0).map(|_| ()));
        transfer(core.fetch_input_lock(1).map(|_| ()));
        transfer(core.fetch_adat_out_status().map(|_| ()));
        transfer(core.fetch_lg_status().map(|_| ()));
        transfer(core.fetch_input_rate().map(|_| ()));
        transfer(core.fetch_core1_mode().map(|_| ()));
        transfer(core.fetch_output_pin(0).map(|_| ()));
    }

    #[test]
    fn wp_c6_adat_out_status() {
        let (core, _, _) = rig(&[(op::REQ_GET_ADAT_STATUS, Some(vec![1, 1, 22, 1, 3, 0, 0, 1]))]);
        assert_eq!(
            core.fetch_adat_out_status(),
            Ok(AdatOutStatus {
                enabled: true,
                active: true,
                pin: 22,
                rate_ok: true,
                resync_count: 3,
                slip_count: 256,
            })
        );
    }

    #[test]
    fn wp_c6_core1_mode_and_conflict() {
        let (core, _, log) = rig(&[
            (op::REQ_GET_CORE1_MODE, Some(vec![2])),
            (op::REQ_GET_CORE1_CONFLICT, Some(vec![1])),
        ]);
        assert_eq!(core.fetch_core1_mode(), Ok(2));
        assert_eq!(core.get_state().core1_mode, 2);
        let outputs = core.get_state().num_output_channels;
        assert_eq!(core.check_core1_conflict(outputs - 1), Ok(true));
        assert!(entries(&log).contains(&format!(
            "in {:02X} {}",
            op::REQ_GET_CORE1_CONFLICT,
            outputs - 1
        )));
        assert!(matches!(
            core.check_core1_conflict(outputs),
            Err(DspiError::InvalidArgument { .. })
        ));
        let (core, _, _) = rig(&[(op::REQ_GET_CORE1_CONFLICT, Some(vec![0]))]);
        assert_eq!(core.check_core1_conflict(0), Ok(false));
    }

    // ── Connection states ─────────────────────────────────────────────

    const NO_UART: UartConfig = UartConfig {
        enabled: false,
        tx_pin: 0,
        rx_pin: 1,
        notify: false,
        baud: 115_200,
    };
    const NO_I2C: I2cConfig = I2cConfig {
        enabled: false,
        sda_pin: 0,
        scl_pin: 1,
        address: 0x42,
    };

    /// Every setter, as `(name, result)`.
    fn every_setter(core: &DspiCore) -> Vec<(&'static str, Result<()>)> {
        vec![
            ("set_output_pin", core.set_output_pin(0, 6)),
            ("set_output_config_mode", core.set_output_config_mode(0)),
            ("save_output_config", core.save_output_config()),
            ("set_output_type", core.set_output_type(0, 0)),
            ("set_i2s_bck_pin", core.set_i2s_bck_pin(0, 14)),
            ("set_i2s_clock_pin_mode", core.set_i2s_clock_pin_mode(0)),
            ("set_mck_enabled", core.set_mck_enabled(false)),
            ("set_mck_pin", core.set_mck_pin(21)),
            ("set_mck_multiplier", core.set_mck_multiplier(0)),
            ("set_adat_out_enabled", core.set_adat_out_enabled(false)),
            ("set_adat_out_pin", core.set_adat_out_pin(22)),
            ("set_spdif_rx_pin", core.set_spdif_rx_pin(0, 20)),
            (
                "set_spdif_input_enabled",
                core.set_spdif_input_enabled(1, false),
            ),
            ("set_i2s_rx_pin", core.set_i2s_rx_pin(0, 18)),
            ("set_i2s_input_channels", core.set_i2s_input_channels(2)),
            ("set_adat_input_enabled", core.set_adat_input_enabled(false)),
            ("set_adat_input_pin", core.set_adat_input_pin(0xFF)),
            (
                "set_adat_input_clock_mode",
                core.set_adat_input_clock_mode(0),
            ),
            ("set_input_rate", core.set_input_rate(1)),
            ("set_i2s_clock_mode", core.set_i2s_clock_mode(0)),
            ("set_dac_mute", core.set_dac_mute(false, true, 11, 5, 0)),
            ("test_dac_mute", core.test_dac_mute()),
            ("set_lg_sound_sync", core.set_lg_sound_sync(false)),
            ("set_uart", core.set_uart(NO_UART)),
            ("set_i2c", core.set_i2c(NO_I2C)),
        ]
    }

    #[test]
    fn wp_c6_read_only_connection_refuses_every_setter() {
        let (core, remote, log) = rig(&[]);
        core.lock().conn.as_mut().unwrap().wire = Some(WireRead::Older);
        let image = remote.image();
        let state = core.get_state();
        for (name, r) in every_setter(&core) {
            assert_eq!(r, Err(DspiError::ReadOnly), "{name}");
        }
        assert!(entries(&log).is_empty(), "{:?}", entries(&log));
        assert_eq!(remote.image(), image);
        assert_eq!(core.get_state(), state);
        // Entering the bootloader still works: an incompatible firmware
        // must stay updatable.
        core.enter_bootloader().unwrap();
        assert_eq!(
            entries(&log),
            vec![format!("in {:02X} 0", op::REQ_ENTER_BOOTLOADER)]
        );
    }

    #[test]
    fn wp_c6_disconnected() {
        let core = DspiCore::new();
        for (name, r) in every_setter(&core) {
            assert_eq!(r, Err(DspiError::Disconnected), "{name}");
        }
        assert_eq!(core.fetch_output_pin(0), Err(DspiError::Disconnected));
        assert_eq!(core.fetch_core1_mode(), Err(DspiError::Disconnected));
        assert_eq!(core.fetch_dac_mute(), Err(DspiError::Disconnected));
        assert_eq!(core.fetch_ctrl_ifaces(), Err(DspiError::Disconnected));
        assert_eq!(core.check_core1_conflict(0), Err(DspiError::Disconnected));
        assert_eq!(
            core.firmware_version_matches("1.1.6"),
            Err(DspiError::Disconnected)
        );
        // Always Ok, as in the old core.
        assert_eq!(core.enter_bootloader(), Ok(()));
    }

    #[test]
    fn wp_c6_enter_bootloader_ignores_the_vanishing_device() {
        // The fake stalls the opcode; a real device answers and reboots.
        let (core, _, log) = rig(&[]);
        assert_eq!(core.enter_bootloader(), Ok(()));
        assert_eq!(
            entries(&log),
            vec![format!("in {:02X} 0", op::REQ_ENTER_BOOTLOADER)]
        );
        let (core, _, _) = rig(&[(op::REQ_ENTER_BOOTLOADER, Some(vec![1]))]);
        assert_eq!(core.enter_bootloader(), Ok(()));
    }

    #[test]
    fn wp_c6_setters_clear_fresh_image() {
        let (core, _, _) = rig(&[]);
        assert!(core.lock().conn.as_ref().unwrap().fresh_image);
        core.set_lg_sound_sync(true).unwrap();
        assert!(!core.lock().conn.as_ref().unwrap().fresh_image);
    }

    // ── Firmware version check ────────────────────────────────────────

    #[test]
    fn wp_c6_parse_version() {
        assert_eq!(parse_version("1.1.6"), Some((1, 1, 6, 0)));
        assert_eq!(parse_version("v1.1.6-beta3"), Some((1, 1, 6, 3)));
        assert_eq!(parse_version("1.1.6-beta3-16-g557bce7"), Some((1, 1, 6, 3)));
        assert_eq!(parse_version("1.1.6-16-g557bce7"), Some((1, 1, 6, 0)));
        assert_eq!(parse_version(" 1.2.10 "), Some((1, 2, 10, 0)));
        assert_eq!(parse_version(""), None);
        assert_eq!(parse_version("1.1"), None);
        assert_eq!(parse_version("1.1.6.2"), None);
        assert_eq!(parse_version("1.1.300"), None);
        assert_eq!(parse_version("1.1.6-beta"), None);
        assert_eq!(parse_version("é1.1.6"), None);
    }

    #[test]
    fn wp_c6_firmware_version_matches_the_platform_reply() {
        let (core, _, _) = rig(&[]);
        let s = core.get_state();
        let here = format!("{}.{}.{}", s.fw_major, s.fw_minor, s.fw_patch);
        let full = if s.fw_beta == 0 {
            here.clone()
        } else {
            format!("{here}-beta{}-3-gdeadbee", s.fw_beta)
        };
        assert_eq!(core.firmware_version_matches(&full), Ok(true));
        assert_eq!(
            core.firmware_version_matches(&format!("{here}-beta200")),
            Ok(s.fw_beta == 200)
        );
        assert_eq!(core.firmware_version_matches("0.0.1"), Ok(false));
        assert_eq!(core.firmware_version_matches("garbage"), Ok(false));

        let mut early = s.clone();
        early.fw_beta = FW_BETA_EARLY;
        assert!(version_matches(&early, &format!("{here}-beta2")));
        assert!(!version_matches(&early, &format!("{here}-beta3")));
        assert!(!version_matches(&early, &here));
    }

    proptest! {
        #[test]
        fn proptest_wp_c6_parse_version_never_panics(s in ".{0,40}") {
            let _ = parse_version(&s);
        }

        #[test]
        fn proptest_wp_c6_version_round_trips(
            v in (any::<u8>(), any::<u8>(), any::<u8>(), any::<u8>())
        ) {
            let text = if v.3 == 0 {
                format!("{}.{}.{}", v.0, v.1, v.2)
            } else {
                format!("{}.{}.{}-beta{}-1-gabc", v.0, v.1, v.2, v.3)
            };
            prop_assert_eq!(parse_version(&text), Some(v));
        }
    }
}
