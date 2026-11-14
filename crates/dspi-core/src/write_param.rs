//! Write any parameter by its registry path (WP CG32).
//!
//! [`DspiCore::write_param`] takes what `dspi_session::Session::write` takes:
//! a path of the parameter registry (`dspi_proto::registry`), its indices and
//! a value. It answers the value read back. One call for a generic screen, a
//! binding or a script, with the same effect as the typed setter for that
//! path:
//!
//! 1. the registry checks the path, the number of indices, each index against
//!    the device's layout, the kind and range of the value, the feature and
//!    the platform's limits (`Session::check`), under
//!    `DspiCore::writing`: [`DspiError::Disconnected`] with no device,
//!    [`DspiError::ReadOnly`] for another wire format;
//! 2. a path the core has a typed setter for goes through that setter, so its
//!    guards (the Core 1 interlock, clamps and quantizing, the Link device
//!    lock, status bytes), its mirror update and the device's echo are those
//!    of the setter; a path no setter covers goes through `Session::write`
//!    (confirmed by reading it back), then the mirror is read again;
//! 3. the answer is the mirror's value for the path once the setter is done
//!    (the device's own read-back for a session write; the value sent for an
//!    action or a value the mirror does not hold).
//!
//! Some paths are refused here ([`write_refusal`]) because a flow of their
//! own owns them: the irreversible ones (factory reset, the bootloader), whose
//! typed call a client confirms first; the whole image; the control surfaces
//! editor; the analyser's worker; the record-shaped configurations. The
//! core-voltage extension (`EXT_CORE_VREG`) has no registry path at all, so
//! nothing here can raise the voltage: that stays with
//! [`DspiCore::core_vreg_op`] and the calibration flow, behind their word.
#![warn(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

use dspi_proto::FilterType;
use dspi_proto::generated::{filter, limiter};
use dspi_proto::packets::MatrixRoutePacket;
use dspi_proto::registry::{
    ALL_OUTPUTS, Kind, ParamDesc, by_path, eq_field_fits, eq_type_fits, retyped,
};
use dspi_proto::value::Value;
use dspi_session::Outcome;

use crate::consts::*;
use crate::params_fx::{Fx, fx_param};
use crate::records::{DspState, FilterParams};
use crate::{DspiCore, DspiError, Result};

/// A parameter value: `dspi_proto::value::Value` across the bindings. In
/// JSON (the C ABI, the web binding) it is tagged by kind in lower case:
/// `{"float": -12.5}`, `{"bool": true}`, `{"bytes": [1, 2]}`, `"trigger"`.
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ParamValue {
    /// A yes/no value.
    Bool(bool),
    /// A number.
    Float(f32),
    /// A whole number.
    Int(i64),
    /// A choice, as its wire number (the registry has the labels).
    Choice(u8),
    /// A bit mask.
    Mask(u32),
    /// Text.
    Text(String),
    /// A structured payload, as the registry's packet encodes it.
    Bytes(Vec<u8>),
    /// An action with no value.
    Trigger,
}

impl From<ParamValue> for Value {
    fn from(v: ParamValue) -> Self {
        match v {
            ParamValue::Bool(b) => Value::Bool(b),
            ParamValue::Float(f) => Value::Float(f),
            ParamValue::Int(i) => Value::Int(i),
            ParamValue::Choice(c) => Value::Choice(c),
            ParamValue::Mask(m) => Value::Mask(m),
            ParamValue::Text(t) => Value::Text(t),
            ParamValue::Bytes(b) => Value::Bytes(b),
            ParamValue::Trigger => Value::Trigger,
        }
    }
}

impl From<Value> for ParamValue {
    fn from(v: Value) -> Self {
        match v {
            Value::Bool(b) => ParamValue::Bool(b),
            Value::Float(f) => ParamValue::Float(f),
            Value::Int(i) => ParamValue::Int(i),
            Value::Choice(c) => ParamValue::Choice(c),
            Value::Mask(m) => ParamValue::Mask(m),
            Value::Text(t) => ParamValue::Text(t),
            Value::Bytes(b) => ParamValue::Bytes(b),
            Value::Trigger => ParamValue::Trigger,
        }
    }
}

/// The paths [`DspiCore::write_param`] refuses, each with the call that
/// writes it instead.
const REFUSED: &[(&str, &str)] = &[
    ("dev.reset", "factory_reset, after the user confirmed it"),
    (
        "dev.bootloader",
        "enter_bootloader, after the user confirmed it",
    ),
    ("bulk", "apply_all_params"),
    ("bulk.chunk", "apply_all_params"),
    ("rta.config", "rta_set_request"),
    ("rta.control", "rta_set_request"),
    ("sig.config", "siggen_set_config"),
    ("dev.dacmute", "set_dac_mute"),
    ("dev.uart", "set_uart"),
    ("dev.i2c", "set_i2c"),
];

/// The paths no typed setter covers: `Session::write`, then a re-read.
const BY_SESSION: &[&str] = &["diag.usb.reset", "up.config"];

/// Why [`DspiCore::write_param`] does not write `path`, naming the call
/// that does; `None` when it does (or `path` is not a writable registry
/// path, which `write_param` refuses on its own terms).
pub fn write_refusal(path: &str) -> Option<String> {
    if path.starts_with("cs.") {
        return Some(format!(
            "{path} belongs to the control surfaces editor: use cs_apply"
        ));
    }
    REFUSED
        .iter()
        .find(|(p, _)| *p == path)
        .map(|(p, how)| format!("{p} is not written by path: use {how}"))
}

/// What a typed setter's write came back as.
type Answer = Result<ParamValue>;

#[cfg_attr(feature = "uniffi", uniffi::export)]
impl DspiCore {
    /// Write the parameter at registry `path` (`vol.user`, `out.gain`,
    /// `eq.freq`, ...) with `indices` (an output, a channel and a band, ...)
    /// and return the value read back: the mirror's value once the typed
    /// setter for the path has run (clamped and quantized as that setter
    /// does), the device's read-back for a path without one, the value sent
    /// for an action or a value the mirror does not hold.
    ///
    /// The checks, guards, mirror update and echo are those of the typed
    /// setter (module docs). Over DSPi Link the setters' device lock applies.
    ///
    /// Errors: [`DspiError::InvalidArgument`] for an unknown or read-only
    /// path, the wrong number of indices, an index the device does not have,
    /// a value of the wrong kind, out of range or NaN, or a path a flow of
    /// its own writes ([`write_refusal`]); [`DspiError::Unsupported`] for a
    /// feature the firmware lacks; [`DspiError::Disconnected`],
    /// [`DspiError::ReadOnly`]; then the setter's own errors
    /// ([`DspiError::Refused`] with a status byte, [`DspiError::Locked`],
    /// [`DspiError::Transfer`], ...).
    pub fn write_param(
        &self,
        path: String,
        indices: Vec<u8>,
        value: ParamValue,
    ) -> Result<ParamValue> {
        let d = by_path(&path).ok_or_else(|| invalid(format!("there is no parameter {path}")))?;
        if !d.is_writable() {
            return Err(invalid(format!(
                "{path} cannot be changed; it is read-only"
            )));
        }
        if let Some(why) = write_refusal(d.path) {
            return Err(invalid(why));
        }
        if !kind_fits(d.kind, &value) {
            return Err(invalid(format!(
                "{} ({path}) takes {}, not {}",
                d.label,
                kind_name(d.kind),
                Value::from(value).type_name()
            )));
        }
        let value = Value::from(value);
        // -- WP P1LK -- the locked-user state's writes pass its check
        let _lock = crate::limits::LockScope::for_path(d.path);
        // -- WP LK4 -- a write that depends on the probe (a feature, a band
        // index, a control-surface slot) waits for a DSPi Link bind's probe
        if dspi_session::write::depends_on_probe(d) {
            self.ensure_probed();
        }
        // Diagnostics housekeeping: allowed on a read-only connection too,
        // as its typed call is.
        match d.path {
            "meters.clear" => return self.clear_clips().map(|()| ParamValue::Trigger),
            "diag.buffers.reset" => {
                return self.reset_buffer_stats().map(|()| ParamValue::Trigger);
            }
            _ => {}
        }
        let value = {
            let mut inner = self.writing()?;
            // -- WP CG36 -- the line's length is the connected platform's,
            // which the registry's 85 ms does not know. -- WP CG37 -- checked
            // on the quantized value, as the typed setter checks it, and
            // below 0 refused with the line's range (the setter makes it 0).
            let value = match value {
                Value::Float(_) | Value::Int(_) if matches!(d.path, "out.delay" | "ch.delay") => {
                    let ms = crate::quantize_delay(value.as_f32().unwrap_or(f32::NAN));
                    if !ms.is_nan() {
                        crate::params::check_delay(&inner.state, d.path, ms)?;
                    }
                    Value::Float(ms)
                }
                v => v,
            };
            let mut w = inner.writable_conn()?;
            let session = w.session()?;
            if d.path == "eq.type" {
                // -- WP CG36 -- the choices depend on the band: the registry's
                // list is the PEQ bands' (indices and feature checked with Off,
                // which every band takes).
                let code = byte(&value)?;
                session.check(d.path, &indices, &Value::Choice(0))?;
                let b = indices.get(1).copied().unwrap_or(0);
                eq_type_fits(b, code)
                    .map_err(|why| invalid(format!("eq.type {indices:?}: {why}")))?;
                Value::Choice(code)
            } else {
                session.check(d.path, &indices, &value)?;
                d.kind
                    .validate(&value)
                    .map_err(|e| invalid(e.to_string()))?
            }
        };
        if value.as_f32().is_some_and(f32::is_nan) {
            return Err(invalid(format!("{path} is NaN")));
        }
        if BY_SESSION.contains(&d.path) {
            return self.write_by_session(d, &indices, value);
        }
        self.write_typed(d, &indices, &value)
    }

    // -- WP CG36 --
    /// Whether enabling output `output` would collide on Core 1, and if so
    /// why, in the words [`DspiCore::write_param`] refuses it with: `None`
    /// when it would not. The device decides
    /// ([`DspiCore::check_core1_conflict`]); the words come from the layout.
    ///
    /// Errors: those of [`DspiCore::check_core1_conflict`].
    pub fn core1_conflict_reason(&self, output: u8) -> Result<Option<String>> {
        Ok(self
            .check_core1_conflict(output)?
            .then(|| self.seen(|s| core1_conflict_reason(s, output))))
    }
}

impl DspiCore {
    /// The mirror's view of something, under the lock.
    fn seen<T>(&self, f: impl FnOnce(&DspState) -> T) -> T {
        f(&self.lock().state)
    }

    /// A path without a typed setter: `Session::write` on the writable
    /// connection, then the image read again for the mirror (an action
    /// changes nothing in it). The answer is the device's read-back.
    fn write_by_session(&self, d: &ParamDesc, ix: &[u8], value: Value) -> Answer {
        let mut inner = self.writing()?;
        // -- WP P0W -- `Session::write` journals it; an undo's replay as one
        crate::journal::check_replay(&inner)?;
        let outcome = {
            let mut w = inner.writable_conn()?;
            w.conn().fresh_image = false;
            let session = w.session()?;
            session.replaying_as(crate::journal::replay_source());
            let r = session.write(d.path, ix, value.clone());
            session.replaying_as(None);
            r?
        };
        let back = match outcome {
            Outcome::Confirmed(v) | Outcome::Rejected { actual: v, .. } => v,
            Outcome::Accepted => value,
            Outcome::Triggered => Value::Trigger,
        };
        if !matches!(d.kind, Kind::Trigger)
            && let Err(e) = crate::connect::refresh_params_locked(&mut inner)
        {
            log::warn!(
                "{}: written, but the image could not be read again: {e}",
                d.path
            );
        }
        Ok(back.into())
    }

    /// The typed setter for `d`, then the read-back. A path this router does
    /// not know (a registry row newer than it) is an invalid argument.
    #[allow(clippy::too_many_lines)] // one arm per path, nothing to factor
    fn write_typed(&self, d: &ParamDesc, ix: &[u8], v: &Value) -> Answer {
        let i0 = ix.first().copied().unwrap_or(0);
        let i1 = ix.get(1).copied().unwrap_or(0);
        let k = d.kind;
        // A setter's result, then the mirror's value.
        let then = |r: Result<()>, read: &dyn Fn(&DspState) -> Option<ParamValue>| -> Answer {
            r?;
            self.seen(read)
                .ok_or_else(|| invalid(format!("{} has no index {ix:?}", d.path)))
        };
        let echo = |r: Result<()>| -> Answer { r.map(|()| v.clone().into()) };
        if let Some((fx, id)) = fx_param(d.path) {
            return self.write_fx(fx, id, i0, v, k);
        }
        match d.path {
            "vol.user" => then(self.set_user_volume(num(v)?), &|s| some_f(s.user_volume_db)),
            "vol.mute" => then(self.set_user_mute(flag(v)?), &|s| some_b(s.user_mute)),
            "vol.master" => then(self.set_master_volume(num(v)?), &|s| {
                some_f(s.master_volume_db)
            }),
            "vol.master.mode" => then(self.set_master_volume_mode(byte(v)?), &|s| {
                Some(ParamValue::Choice(s.master_volume_mode))
            }),
            "vol.master.save" => echo(self.save_master_volume()),
            "pre" => then(self.set_input_preamp(i0, num(v)?), &|s| {
                at(&s.input_preamp_db, i0).map(ParamValue::Float)
            }),
            "pre.all" => then(self.set_preamp(num(v)?), &|s| some_f(s.preamp_db)),
            "bypass" => then(self.set_bypass(flag(v)?), &|s| some_b(s.bypass)),
            "eq.type" | "eq.freq" | "eq.q" | "eq.gain" | "eq.fp" | "eq.qp" => {
                self.write_band_field(d.path, i0, i1, v)
            }
            "eq.bypass" => then(self.set_band_bypass(i0, i1, flag(v)?), &|s| {
                band(s, i0, i1).map(|b| ParamValue::Bool(b.bypass))
            }),
            "ch.delay" => then(self.set_delay(i0, num(v)?), &|s| {
                at(&s.channel_delays, i0).map(ParamValue::Float)
            }),
            "ch.name" => then(self.set_channel_name(i0, text(v)?), &|s| {
                at(&s.channel_names, i0).map(ParamValue::Text)
            }),
            "mix" => self.write_crosspoint(i0, i1, v),
            // -- WP CG36 -- a Core 1 conflict is refused in words, not read back as false.
            "out.enable" => match self.output_enable(i0, flag(v)?)? {
                (_, true) => Err(invalid(self.seen(|s| core1_conflict_reason(s, i0)))),
                (on, false) => Ok(ParamValue::Bool(on)),
            },
            "out.gain" => then(self.set_output_gain(i0, num(v)?), &|s| {
                at(&s.output_gain_db, i0).map(ParamValue::Float)
            }),
            "out.mute" => then(self.set_output_mute(i0, flag(v)?), &|s| {
                at(&s.output_muted, i0).map(ParamValue::Bool)
            }),
            "out.delay" => then(self.set_output_delay(i0, num(v)?), &|s| {
                at(&s.output_delay_ms, i0).map(ParamValue::Float)
            }),
            "out.pin" => then(self.set_output_pin(i0, byte(v)?), &|s| {
                at(&s.output_pins, i0).map(|p| ParamValue::Int(p.into()))
            }),
            "out.type" => then(self.set_output_type(i0, byte(v)?), &|s| {
                at(&s.output_types, i0).map(ParamValue::Choice)
            }),
            "out.latency" => echo(self.set_output_latency(byte(v)?)),
            "loud.on" => then(self.set_loudness(flag(v)?), &|s| some_b(s.loudness_enabled)),
            "loud.ref" => then(self.set_loudness_ref(num(v)?), &|s| {
                some_f(s.loudness_ref_spl)
            }),
            "loud.intensity" => then(self.set_loudness_intensity(num(v)?), &|s| {
                some_f(s.loudness_intensity)
            }),
            "loud.mask" => then(self.set_loudness_mask(mask16(v)?), &|s| {
                Some(ParamValue::Mask(s.loudness_output_mask.into()))
            }),
            "lev.on" => then(self.set_leveller_enabled(flag(v)?), &|s| {
                some_b(s.leveller_enabled)
            }),
            "lev.amount" => then(self.set_leveller_amount(num(v)?), &|s| {
                some_f(s.leveller_amount)
            }),
            "lev.speed" => then(self.set_leveller_speed(byte(v)?), &|s| {
                Some(ParamValue::Choice(s.leveller_speed))
            }),
            "lev.maxgain" => then(self.set_leveller_max_gain(num(v)?), &|s| {
                some_f(s.leveller_max_gain_db)
            }),
            "lev.lookahead" => then(self.set_leveller_lookahead(flag(v)?), &|s| {
                some_b(s.leveller_lookahead)
            }),
            "lev.gate" => then(self.set_leveller_gate(num(v)?), &|s| {
                some_f(s.leveller_gate_db)
            }),
            "lev.masks" => {
                // Detector in the low byte, apply in the high one (the wire's order).
                let [det, app, ..] = mask16(v)?.to_le_bytes();
                then(self.set_leveller_masks(det, app), &|s| {
                    Some(ParamValue::Mask(
                        u32::from(s.leveller_detector_mask)
                            | u32::from(s.leveller_apply_mask).wrapping_shl(8),
                    ))
                })
            }
            "cf.on" => then(self.set_crossfeed(flag(v)?), &|s| {
                some_b(s.crossfeed_enabled)
            }),
            "cf.preset" => then(self.set_crossfeed_preset(byte(v)?), &|s| {
                Some(ParamValue::Choice(s.crossfeed_preset))
            }),
            "cf.freq" => then(self.set_crossfeed_freq(num(v)?), &|s| {
                some_f(s.crossfeed_freq)
            }),
            "cf.feed" => then(self.set_crossfeed_feed(num(v)?), &|s| {
                some_f(s.crossfeed_feed)
            }),
            "cf.itd" => then(self.set_crossfeed_itd(flag(v)?), &|s| {
                some_b(s.crossfeed_itd)
            }),
            "cf.outputs" => then(self.set_crossfeed_outputs(mask8(v)?), &|s| {
                Some(ParamValue::Mask(s.crossfeed_output_pair_mask.into()))
            }),
            "bass.on" => then(self.set_psybass_enabled(flag(v)?), &|s| {
                some_b(s.psybass_enabled)
            }),
            "bass.mask" => then(self.set_psybass_mask(mask16(v)?), &|s| {
                Some(ParamValue::Mask(s.psybass_output_mask.into()))
            }),
            "sub.solo" => then(self.set_subharm_solo(flag(v)?), &|s| some_b(s.subharm_solo)),
            "in.source" => then(self.set_input_source(byte(v)?), &|s| {
                Some(ParamValue::Choice(s.input_source))
            }),
            "in.name" => {
                let name = text(v)?;
                self.set_input_source_name(i0, name.clone())
                    .map(|()| ParamValue::Text(self.cached_input_source_name(i0).unwrap_or(name)))
            }
            "in.rate" => {
                let index = match num(v)? as u32 {
                    44_100 => 0,
                    48_000 => 1,
                    96_000 => 2,
                    hz => {
                        return Err(invalid(format!(
                            "input rate {hz} Hz (44100, 48000 or 96000)"
                        )));
                    }
                };
                then(self.set_input_rate(index), &|s| {
                    let hz: i64 = match s.i2s_input_rate {
                        0 => 44_100,
                        2 => 96_000,
                        _ => 48_000,
                    };
                    Some(ParamValue::Int(hz))
                })
            }
            "in.spdif.pin" => then(self.set_spdif_rx_pin(i0, byte(v)?), &|s| {
                at(&s.spdif_rx_pins, i0).map(|p| ParamValue::Int(p.into()))
            }),
            "in.spdif.enable" => then(self.set_spdif_input_enabled(i0, flag(v)?), &|s| {
                let bit = 1u8.checked_shl(u32::from(i0))?;
                Some(ParamValue::Bool(s.spdif_inputs_enabled & bit != 0))
            }),
            "in.i2s.pin" => then(self.set_i2s_rx_pin(i0, byte(v)?), &|s| {
                at(&s.i2s_rx_pins, i0).map(|p| ParamValue::Int(p.into()))
            }),
            "in.i2s.channels" => then(self.set_i2s_input_channels(byte(v)?), &|s| {
                Some(ParamValue::Int(s.i2s_input_channels.into()))
            }),
            "in.i2s.clock" => then(self.set_i2s_clock_mode(byte(v)?), &|s| {
                Some(ParamValue::Choice(s.i2s_clock_mode))
            }),
            "in.adat.enable" => then(self.set_adat_input_enabled(flag(v)?), &|s| {
                some_b(s.adat_input_enabled)
            }),
            "in.adat.pin" => then(self.set_adat_input_pin(byte(v)?), &|s| {
                Some(ParamValue::Int(s.adat_input_pin.into()))
            }),
            "in.adat.clock" => then(self.set_adat_input_clock_mode(byte(v)?), &|s| {
                Some(ParamValue::Choice(s.adat_input_clock_mode))
            }),
            "in.lg" => then(self.set_lg_sound_sync(flag(v)?), &|s| {
                some_b(s.lg_sound_sync_enabled)
            }),
            "i2s.bck" => then(self.set_i2s_bck_pin(0, byte(v)?), &|s| {
                Some(ParamValue::Int(s.i2s_bck_pin.into()))
            }),
            "i2s.bck.slave" => then(self.set_i2s_bck_pin(1, byte(v)?), &|s| {
                Some(ParamValue::Int(s.i2s_bck_pin_slave.into()))
            }),
            "i2s.clockpins" => then(self.set_i2s_clock_pin_mode(byte(v)?), &|s| {
                Some(ParamValue::Choice(s.i2s_clock_pin_mode))
            }),
            "i2s.mck" => then(self.set_mck_enabled(flag(v)?), &|s| some_b(s.mck_enabled)),
            "i2s.mck.pin" => then(self.set_mck_pin(byte(v)?), &|s| {
                Some(ParamValue::Int(s.mck_pin.into()))
            }),
            "i2s.mck.mult" => then(self.set_mck_multiplier(byte(v)?), &|s| {
                Some(ParamValue::Choice(s.mck_multiplier))
            }),
            "adat.enable" => then(self.set_adat_out_enabled(flag(v)?), &|s| {
                some_b(s.adat_out_enabled)
            }),
            "adat.pin" => then(self.set_adat_out_pin(byte(v)?), &|s| {
                Some(ParamValue::Int(s.adat_out_pin.into()))
            }),
            "dev.dacmute.test" => echo(self.test_dac_mute()),
            "dev.save" => echo(self.save_params()),
            "dev.save.io" => echo(self.save_output_config()),
            "dev.name" => {
                let name = text(v)?;
                self.set_device_name(name.clone())
                    .map(|()| ParamValue::Text(self.cached_device_name().unwrap_or(name)))
            }
            "preset.save" => echo(self.save_preset(i0)),
            "preset.load" => echo(self.load_preset(i0)),
            "preset.delete" => echo(self.delete_preset(i0)),
            "preset.name" => then(self.set_preset_name(i0, text(v)?), &|s| {
                at(&s.preset_names, i0).map(ParamValue::Text)
            }),
            "preset.startup" => {
                let Some(&[mode, slot]) = bytes(v)?.get(..) else {
                    return Err(invalid(
                        "preset.startup takes two bytes: the mode and the default slot".into(),
                    ));
                };
                then(self.set_preset_startup(mode, slot), &|s| {
                    Some(ParamValue::Bytes(vec![
                        s.preset_startup_mode,
                        s.preset_default_slot,
                    ]))
                })
            }
            "preset.iomode" => then(self.set_output_config_mode(byte(v)?), &|s| {
                Some(ParamValue::Choice(s.output_config_mode))
            }),
            "sig.control" => echo(self.siggen_control(byte(v)?.into())),
            _ => Err(invalid(format!("{} has no writer in this core", d.path))),
        }
    }

    /// An effects parameter through its `set_*_param`, read back from the
    /// effect's table in the mirror.
    fn write_fx(&self, fx: Fx, id: u8, output: u8, v: &Value, k: Kind) -> Answer {
        // -- WP TMCOREF -- a bit-mask parameter (`sub.mask`, `tube.mask`) comes
        // as the registry's kind, a mask; it fits an f32 exactly (16 bits)
        #[allow(clippy::cast_precision_loss)]
        let x = match v {
            Value::Mask(m) if *m <= 0x00FF_FFFF => *m as f32,
            v => num(v)?,
        };
        match fx {
            Fx::Psybass => self.set_psybass_param(id, x)?,
            Fx::Upmix => self.set_upmix_param(id, x)?,
            Fx::Subharm => self.set_subharm_param(id, x)?,
            Fx::Tube => self.set_tube_param(id, x)?,
            Fx::Limiter => self.set_limiter_param(output, id, x)?,
        }
        let back = self.seen(|s| fx_mirror(s, fx, id, output));
        back.map(|x| as_kind(k, x))
            .ok_or_else(|| invalid(format!("output {output} has no limiter")))
    }

    /// One field of a PEQ band (0..9) or a crossover band (20..23): the
    /// mirror's band with that field changed, sent whole through
    /// `set_filter` or `set_crossover`.
    ///
    /// A Linkwitz Transform band takes `eq.fp` and `eq.qp` and refuses
    /// `eq.gain` (its gain field is fp, in Hz); every other band the
    /// reverse (`registry::eq_field_fits`). A type change into or out of a
    /// Linkwitz Transform rewrites the gain field as the Console does
    /// (`registry::retyped`).
    fn write_band_field(&self, path: &str, ch: u8, b: u8, v: &Value) -> Answer {
        let mut p = self
            .seen(|s| band(s, ch, b))
            .ok_or_else(|| invalid(format!("channel {ch} has no band {b}")))?;
        // -- WP CG34 --
        let from = FilterType::from_raw(p.filter_type);
        eq_field_fits(path, b, from)
            .map_err(|why| invalid(format!("{path} [{ch}, {b}]: {why}")))?;
        match path {
            "eq.type" => {
                let to = byte(v)?;
                [p.freq, p.q, p.gain, p.qp] =
                    retyped(from, FilterType::from_raw(to), [p.freq, p.q, p.gain, p.qp]);
                p.filter_type = to;
            }
            "eq.freq" => p.freq = num(v)?,
            "eq.q" => p.q = num(v)?,
            "eq.qp" => p.qp = num(v)?,
            // eq.gain, or eq.fp on a Linkwitz Transform: the same field.
            _ => p.gain = num(v)?,
        }
        match b.checked_sub(XOVER_BASE) {
            Some(x) => self.set_crossover(ch, x, p)?,
            None => self.set_filter(ch, b, p)?,
        }
        let p = self
            .seen(|s| band(s, ch, b))
            .ok_or_else(|| invalid(format!("channel {ch} has no band {b}")))?;
        Ok(match path {
            "eq.type" => ParamValue::Choice(p.filter_type),
            "eq.freq" => ParamValue::Float(p.freq),
            "eq.q" => ParamValue::Float(p.q),
            "eq.qp" => ParamValue::Float(p.qp),
            _ => ParamValue::Float(p.gain),
        })
    }

    /// A matrix crosspoint from its packet (`MatrixRoutePacket`), which must
    /// name the input and output the indices do.
    fn write_crosspoint(&self, input: u8, output: u8, v: &Value) -> Answer {
        let p = MatrixRoutePacket::decode(bytes(v)?).map_err(|e| invalid(format!("mix: {e}")))?;
        if (p.input, p.output) != (input, output) {
            return Err(invalid(format!(
                "mix [{input}, {output}]: the packet is for input {} and output {}",
                p.input, p.output
            )));
        }
        self.set_matrix_route(input, output, p.enabled, p.gain_db, p.phase_invert)?;
        self.seen(|s| {
            let cell =
                |m: &Vec<Vec<bool>>| m.get(usize::from(input))?.get(usize::from(output)).copied();
            let gain = s
                .matrix_gain
                .get(usize::from(input))?
                .get(usize::from(output))
                .copied()?;
            Some(MatrixRoutePacket {
                input,
                output,
                enabled: cell(&s.matrix_routing)?,
                phase_invert: cell(&s.matrix_invert)?,
                gain_db: gain,
            })
        })
        .map(|p| ParamValue::Bytes(p.encode().to_vec()))
        .ok_or_else(|| invalid(format!("mix has no crosspoint [{input}, {output}]")))
    }
}

// -- WP P0W -- shared with the journal's mirror reads
/// An effects parameter's value in the mirror's table. "Every output" reads
/// as the first output, which it set too.
pub(crate) fn fx_mirror(s: &DspState, fx: Fx, id: u8, output: u8) -> Option<f32> {
    let o = if output == ALL_OUTPUTS { 0 } else { output };
    match fx {
        Fx::Psybass => match id {
            PSYBASS_PARAM_CUTOFF => Some(s.psybass_cutoff_hz),
            PSYBASS_PARAM_HARMONICS => Some(s.psybass_harmonics_db),
            PSYBASS_PARAM_DRIVE => Some(s.psybass_drive_db),
            PSYBASS_PARAM_CHARACTER => Some(s.psybass_character_pct),
            _ => Some(s.psybass_original_db),
        },
        Fx::Upmix => at(&s.upmix, id),
        Fx::Subharm => at(&s.subharm, id),
        Fx::Tube => at(&s.tube, id),
        Fx::Limiter => match u16::from(id) {
            limiter::LIMITER_PARAM_ENABLED => {
                at(&s.limiter_enabled, o).map(|b| f32::from(u8::from(b)))
            }
            limiter::LIMITER_PARAM_THRESHOLD_DB => at(&s.limiter_threshold_db, o),
            limiter::LIMITER_PARAM_RELEASE_MS => at(&s.limiter_release_ms, o),
            _ => at(&s.limiter_link_group, o).map(f32::from),
        },
    }
}

// -- WP CG36 --
/// Why output `output` (0-based) cannot be enabled while Core 1 runs the
/// other side, numbered from 1 as the Console shows outputs. Core 1 runs
/// either the PDM output (the last) or the EQ worker for outputs 3 up to the
/// one before it (`eqWorkerRange`, `pdmOutputIndex` in DSPViewModel.swift).
fn core1_conflict_reason(s: &DspState, output: u8) -> String {
    let n = u16::from(output).saturating_add(1);
    let pdm = s.num_output_channels;
    let workers = format!("outputs 3-{}", pdm.saturating_sub(1));
    if output.saturating_add(1) == pdm {
        format!(
            "Output {n} is the PDM output, which shares Core 1 with {workers}, and one of \
             them is enabled: disable {workers} first"
        )
    } else if (2..pdm.saturating_sub(1)).contains(&output) {
        format!(
            "Output {n} shares Core 1 with the PDM output (output {pdm}), which is enabled: \
             disable the PDM output first"
        )
    } else {
        format!("Output {n} cannot be enabled now: Core 1 is busy with another output")
    }
}

// ── Values ─────────────────────────────────────────────────────────────

/// The first crossover band on the wire (`XOVER_BAND_BASE`, 20).
const XOVER_BASE: u8 = {
    assert!(filter::XOVER_BAND_BASE <= u8::MAX as u16);
    filter::XOVER_BAND_BASE as u8
};

fn invalid(msg: String) -> DspiError {
    DspiError::InvalidArgument { msg }
}

fn wrong(v: &Value, want: &str) -> DspiError {
    invalid(format!("expected {want}, got {}", v.type_name()))
}

fn num(v: &Value) -> Result<f32> {
    v.as_f32().ok_or_else(|| wrong(v, "a number"))
}

fn flag(v: &Value) -> Result<bool> {
    v.as_bool().ok_or_else(|| wrong(v, "a yes/no value"))
}

/// A choice or a whole number that fits a byte (a pin, a mode, a count).
fn byte(v: &Value) -> Result<u8> {
    match v {
        Value::Choice(c) => Ok(*c),
        Value::Int(i) => u8::try_from(*i).map_err(|_| invalid(format!("{i} does not fit a byte"))),
        _ => Err(wrong(v, "a choice or a whole number")),
    }
}

fn mask(v: &Value) -> Result<u32> {
    match v {
        Value::Mask(m) => Ok(*m),
        Value::Int(i) => u32::try_from(*i).map_err(|_| invalid(format!("{i} is not a mask"))),
        _ => Err(wrong(v, "a mask")),
    }
}

fn mask16(v: &Value) -> Result<u16> {
    let m = mask(v)?;
    u16::try_from(m).map_err(|_| invalid(format!("mask 0x{m:X} is wider than 16 bits")))
}

fn mask8(v: &Value) -> Result<u8> {
    let m = mask(v)?;
    u8::try_from(m).map_err(|_| invalid(format!("mask 0x{m:X} is wider than 8 bits")))
}

fn text(v: &Value) -> Result<String> {
    match v {
        Value::Text(t) => Ok(t.clone()),
        _ => Err(wrong(v, "text")),
    }
}

fn bytes(v: &Value) -> Result<&[u8]> {
    match v {
        Value::Bytes(b) => Ok(b),
        _ => Err(wrong(v, "bytes")),
    }
}

/// Whether `v` is a value of kind `k`. The registry's own check converts
/// between kinds (a yes/no value is a number to it); this one does not,
/// except that a whole number is also a number, a choice or a mask.
fn kind_fits(k: Kind, v: &ParamValue) -> bool {
    use ParamValue as P;
    matches!(
        (k, v),
        (Kind::Bool, P::Bool(_))
            | (Kind::Float { .. }, P::Float(_) | P::Int(_))
            | (Kind::Int { .. }, P::Int(_))
            | (Kind::Choice(_), P::Choice(_) | P::Int(_))
            | (Kind::Mask, P::Mask(_) | P::Int(_))
            | (Kind::Text { .. }, P::Text(_))
            | (Kind::Packet, P::Bytes(_))
            | (Kind::Trigger, P::Trigger)
    )
}

/// What a value of kind `k` is, in words.
fn kind_name(k: Kind) -> &'static str {
    match k {
        Kind::Bool => "a yes/no value",
        Kind::Float { .. } => "a number",
        Kind::Int { .. } => "a whole number",
        Kind::Choice(_) => "a choice",
        Kind::Mask => "a mask",
        Kind::Text { .. } => "text",
        Kind::Packet => "bytes",
        Kind::Trigger => "no value (an action)",
        Kind::Status => "nothing (it is read-only)",
    }
}

fn some_f(x: f32) -> Option<ParamValue> {
    Some(ParamValue::Float(x))
}

fn some_b(x: bool) -> Option<ParamValue> {
    Some(ParamValue::Bool(x))
}

fn at<T: Clone>(v: &[T], i: u8) -> Option<T> {
    v.get(usize::from(i)).cloned()
}

/// Band `b` of wire channel `ch` in the mirror: PEQ 0..9 or crossover 20..23.
pub(crate) fn band(s: &DspState, ch: u8, b: u8) -> Option<FilterParams> {
    let ch = usize::from(ch);
    match b.checked_sub(XOVER_BASE) {
        Some(x) => s.xover.get(ch)?.get(usize::from(x)).copied(),
        None => s.filters.get(ch)?.get(usize::from(b)).copied(),
    }
}

/// A mirror's `f32` as the registry's kind for the path. The effects
/// tables hold whole numbers for their flags, choices and masks.
pub(crate) fn as_kind(k: Kind, x: f32) -> ParamValue {
    match k {
        Kind::Bool => ParamValue::Bool(x != 0.0),
        Kind::Choice(_) => ParamValue::Choice(x as u8),
        Kind::Int { .. } => ParamValue::Int(x as i64),
        Kind::Mask => ParamValue::Mask(x as u32),
        _ => ParamValue::Float(x),
    }
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_cg32 {
    use super::*;
    use dspi_proto::generated::opcodes as op;
    use dspi_proto::registry::REGISTRY;

    fn demo() -> std::sync::Arc<DspiCore> {
        let core = DspiCore::new();
        core.open_demo().unwrap();
        core
    }

    fn put(core: &DspiCore, path: &str, ix: &[u8], v: ParamValue) -> Result<ParamValue> {
        core.write_param(path.into(), ix.to_vec(), v)
    }

    fn msg(r: Result<ParamValue>) -> String {
        match r {
            Err(DspiError::InvalidArgument { msg }) => msg,
            other => panic!("expected an invalid argument, got {other:?}"),
        }
    }

    /// A value of `d`'s kind, inside its range.
    fn sample(d: &ParamDesc) -> ParamValue {
        match d.kind {
            Kind::Bool => ParamValue::Bool(false),
            Kind::Float { min, max, .. } => ParamValue::Float(min.max(0.0).min(max)),
            Kind::Int { min, .. } => ParamValue::Int(min),
            Kind::Choice(list) => ParamValue::Choice(list.first().map_or(0, |c| c.0)),
            Kind::Mask => ParamValue::Mask(1),
            Kind::Text { .. } => ParamValue::Text("x".into()),
            Kind::Packet => ParamValue::Bytes(vec![0; 8]),
            Kind::Trigger | Kind::Status => ParamValue::Trigger,
        }
    }

    /// Indices `d` takes, all 0 but a band's (PEQ band 1) and an S/PDIF
    /// extra input's (1).
    fn indices(d: &ParamDesc) -> Vec<u8> {
        match d.path {
            "in.spdif.enable" => vec![1],
            _ => vec![0; d.target.arity()],
        }
    }

    #[test]
    fn wp_cg32_every_writable_path_has_a_writer_or_a_named_refusal() {
        let core = demo();
        for d in REGISTRY.iter().filter(|d| d.is_writable()) {
            let r = put(&core, d.path, &indices(d), sample(d));
            if let Some(why) = write_refusal(d.path) {
                assert_eq!(msg(r), why, "{}", d.path);
                continue;
            }
            if let Err(DspiError::InvalidArgument { msg }) = &r {
                assert!(!msg.contains("has no writer"), "{}: {msg}", d.path);
            }
        }
    }

    #[test]
    fn wp_cg32_nothing_reaches_the_core_voltage_extension() {
        // EXT_CORE_VREG rides on REQ_EXTENSION; the registry writes nothing there.
        for d in REGISTRY.iter().filter(|d| d.is_writable()) {
            assert_ne!(d.set, Some(op::REQ_EXTENSION), "{}", d.path);
        }
    }

    /// The same change through `write_param` and the typed setter leaves
    /// two demo devices' mirrors (image included) the same, and the answer
    /// is the mirror's value.
    #[test]
    fn wp_cg32_write_param_does_what_the_typed_setter_does() {
        type Typed = fn(&DspiCore) -> Result<()>;
        let cases: &[(&str, &[u8], ParamValue, Typed, ParamValue)] = &[
            (
                "out.gain",
                &[1],
                ParamValue::Float(-4.05),
                |c| c.set_output_gain(1, -4.05),
                ParamValue::Float(-4.1),
            ),
            (
                "vol.master",
                &[],
                ParamValue::Float(-10.3),
                |c| c.set_master_volume(-10.3),
                ParamValue::Float(-10.5),
            ),
            (
                "pre",
                &[1],
                ParamValue::Float(-4.0),
                |c| c.set_input_preamp(1, -4.0),
                ParamValue::Float(-4.0),
            ),
            (
                "eq.gain",
                &[3, 2],
                ParamValue::Float(3.33),
                |c| {
                    let mut p = c.get_state().filters[3][2];
                    p.gain = 3.33;
                    c.set_filter(3, 2, p)
                },
                ParamValue::Float(3.3),
            ),
            (
                "eq.bypass",
                &[0, 1],
                ParamValue::Bool(true),
                |c| c.set_band_bypass(0, 1, true),
                ParamValue::Bool(true),
            ),
            (
                "limit.threshold",
                &[ALL_OUTPUTS],
                ParamValue::Float(-6.0),
                |c| {
                    c.set_limiter_param(
                        ALL_OUTPUTS,
                        limiter::LIMITER_PARAM_THRESHOLD_DB as u8,
                        -6.0,
                    )
                },
                ParamValue::Float(-6.0),
            ),
            (
                "tube.type",
                &[],
                ParamValue::Int(3),
                |c| c.set_tube_param(dspi_proto::generated::tube::TUBE_PARAM_TUBE_TYPE as u8, 3.0),
                ParamValue::Int(3),
            ),
            (
                "up.on",
                &[],
                ParamValue::Bool(true),
                |c| c.set_upmix_param(UPMIX_PARAM_ENABLED, 1.0),
                ParamValue::Bool(true),
            ),
            (
                "lev.masks",
                &[],
                ParamValue::Mask(0x0201),
                |c| c.set_leveller_masks(1, 2),
                ParamValue::Mask(0x0201),
            ),
            (
                "ch.name",
                &[2],
                ParamValue::Text("a name far longer than thirty-one bytes".into()),
                |c| c.set_channel_name(2, "a name far longer than thirty-one bytes".into()),
                ParamValue::Text("a name far longer than thirty-o".into()),
            ),
            (
                "out.enable",
                &[3],
                ParamValue::Bool(false),
                |c| c.set_output_enable(3, false).map(|_| ()),
                ParamValue::Bool(false),
            ),
            (
                "in.rate",
                &[],
                ParamValue::Int(96_000),
                |c| c.set_input_rate(2),
                ParamValue::Int(96_000),
            ),
            (
                "cf.preset",
                &[],
                ParamValue::Choice(2),
                |c| c.set_crossfeed_preset(2),
                ParamValue::Choice(2),
            ),
        ];
        let (a, b) = (demo(), demo());
        for (path, ix, v, typed, want) in cases {
            let got = put(&a, path, ix, v.clone()).unwrap_or_else(|e| panic!("{path}: {e}"));
            typed(&b).unwrap_or_else(|e| panic!("{path} typed: {e}"));
            assert_eq!(&got, want, "{path}");
            assert_eq!(a.get_state(), b.get_state(), "{path}");
        }
        // Their echoes, too, leave the two the same.
        for _ in 0..20 {
            a.process_notifications();
            b.process_notifications();
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert_eq!(a.get_state(), b.get_state());
    }

    #[test]
    fn wp_cg32_the_registry_checks_come_first_in_its_words() {
        let core = DspiCore::new();
        assert_eq!(
            put(&core, "vol.user", &[], ParamValue::Float(-3.0)),
            Err(DspiError::Disconnected)
        );
        core.open_demo().unwrap();
        let before = core.get_state();
        assert_eq!(
            msg(put(&core, "no.such", &[], ParamValue::Trigger)),
            "there is no parameter no.such"
        );
        assert!(msg(put(&core, "meters", &[], ParamValue::Trigger)).contains("read-only"));
        assert!(msg(put(&core, "out.gain", &[], ParamValue::Float(-3.0))).contains("index"));
        assert!(msg(put(&core, "vol.user", &[], ParamValue::Float(3.0))).contains("outside"));
        assert!(msg(put(&core, "vol.user", &[], ParamValue::Bool(true))).contains("number"));
        assert!(msg(put(&core, "vol.user", &[], ParamValue::Float(f32::NAN))).contains("NaN"));
        assert!(msg(put(&core, "loud.mask", &[], ParamValue::Float(1.0))).contains("mask"));
        // -- c6e W3-1 -- an input the device lacks, refused before any setter.
        assert!(!msg(put(&core, "pre", &[200], ParamValue::Float(-4.0))).is_empty());
        assert!(!msg(put(&core, "out.mute", &[99], ParamValue::Bool(true))).is_empty());
        assert!(
            msg(put(&core, "dev.bootloader", &[], ParamValue::Trigger))
                .contains("enter_bootloader")
        );
        assert!(msg(put(&core, "cs.save", &[], ParamValue::Trigger)).contains("cs_apply"));
        assert_eq!(core.get_state(), before, "nothing refused changed anything");
    }

    #[test]
    fn wp_cg32_a_read_only_connection_refuses_writes_but_not_housekeeping() {
        let core = DspiCore::new();
        core.open_demo_wire(31).unwrap();
        assert_eq!(
            put(&core, "vol.user", &[], ParamValue::Float(-10.0)),
            Err(DspiError::ReadOnly)
        );
        assert_eq!(core.read_only_refusals(), 1);
        assert_eq!(
            put(&core, "meters.clear", &[], ParamValue::Trigger),
            Ok(ParamValue::Trigger)
        );
    }

    #[test]
    fn wp_cg32_a_path_without_a_setter_goes_through_session_write() {
        let core = demo();
        let mut cfg = core
            .lock()
            .conn()
            .unwrap()
            .get(op::REQ_UPMIX_GET_CONFIG, 0, 44)
            .unwrap();
        cfg[0] = 1; // enabled
        assert_eq!(
            core.get_state().upmix[usize::from(UPMIX_PARAM_ENABLED)],
            0.0
        );
        // The device's read-back, and the mirror read again.
        assert_eq!(
            put(&core, "up.config", &[], ParamValue::Bytes(cfg.clone())),
            Ok(ParamValue::Bytes(cfg))
        );
        assert_eq!(
            core.get_state().upmix[usize::from(UPMIX_PARAM_ENABLED)],
            1.0
        );
    }

    #[test]
    fn wp_cg32_values_are_tagged_by_kind_in_lower_case() {
        let v: ParamValue = serde_json::from_str(r#"{"float": -12.5}"#).unwrap();
        assert_eq!(v, ParamValue::Float(-12.5));
        assert_eq!(
            serde_json::to_string(&ParamValue::Trigger).unwrap(),
            r#""trigger""#
        );
        assert_eq!(
            serde_json::to_string(&ParamValue::Bytes(vec![1, 2])).unwrap(),
            r#"{"bytes":[1,2]}"#
        );
        for v in [
            ParamValue::Mask(3),
            ParamValue::Text("t".into()),
            ParamValue::Choice(2),
        ] {
            assert_eq!(ParamValue::from(Value::from(v.clone())), v);
        }
    }

    // -- WP TMCOREF --
    /// A bit-mask effect parameter takes its registry kind, a mask (a
    /// `.dspipreset` import through the Terminal's routed session sends it so).
    #[test]
    fn wp_tmcoref_a_mask_effect_parameter_takes_a_mask() {
        let core = demo();
        for path in ["sub.mask", "tube.mask"] {
            let back = put(&core, path, &[], ParamValue::Mask(0b11)).unwrap();
            assert_eq!(back, ParamValue::Mask(0b11), "{path}");
        }
    }
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_cg34 {
    use super::*;

    fn demo() -> std::sync::Arc<DspiCore> {
        let core = DspiCore::new();
        core.open_demo().unwrap();
        core
    }

    fn put(core: &DspiCore, path: &str, ix: &[u8], v: ParamValue) -> Result<ParamValue> {
        core.write_param(path.into(), ix.to_vec(), v)
    }

    fn refused(r: Result<ParamValue>) -> String {
        match r {
            Err(DspiError::InvalidArgument { msg }) => msg,
            other => panic!("expected an invalid argument, got {other:?}"),
        }
    }

    fn settle(cores: &[&DspiCore]) {
        for _ in 0..20 {
            for c in cores {
                c.process_notifications();
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }

    /// A Linkwitz Transform written field by field reads back, and leaves
    /// the mirror (image included) as one whole-band `set_filter` does.
    #[test]
    fn wp_cg34_a_linkwitz_band_by_path_is_the_typed_setters_band() {
        let (a, b) = (demo(), demo());
        let (ch, band) = (9, 2);
        let f = |p: &str, v: ParamValue| put(&a, p, &[ch, band], v).unwrap();
        assert_eq!(f("eq.type", ParamValue::Choice(11)), ParamValue::Choice(11));
        assert_eq!(
            f("eq.freq", ParamValue::Float(42.0)),
            ParamValue::Float(42.0)
        );
        assert_eq!(f("eq.q", ParamValue::Float(0.95)), ParamValue::Float(0.95));
        assert_eq!(f("eq.fp", ParamValue::Float(25.0)), ParamValue::Float(25.0));
        // Qp travels as Q * 512 and reads back on that step.
        assert_eq!(
            f("eq.qp", ParamValue::Float(0.6)),
            ParamValue::Float(307.0 / 512.0)
        );
        let bypass = a.get_state().filters[usize::from(ch)][usize::from(band)].bypass;
        b.set_filter(
            ch,
            band,
            FilterParams {
                filter_type: 11,
                bypass,
                freq: 42.0,
                q: 0.95,
                gain: 25.0,
                qp: 0.6,
            },
        )
        .unwrap();
        assert_eq!(a.get_state(), b.get_state());
        settle(&[&a, &b]);
        assert_eq!(a.get_state(), b.get_state());
        let p = a.get_state().filters[usize::from(ch)][usize::from(band)];
        assert_eq!((p.filter_type, p.gain, p.qp), (11, 25.0, 307.0 / 512.0));
    }

    #[test]
    fn wp_cg34_out_of_range_and_wrong_band_writes_are_refused_and_change_nothing() {
        let core = demo();
        let (ch, band) = (9, 3);
        put(&core, "eq.type", &[ch, band], ParamValue::Choice(11)).unwrap();
        put(&core, "eq.fp", &[ch, band], ParamValue::Float(30.0)).unwrap();
        let before = core.get_state();
        for (path, v) in [
            ("eq.fp", 9.0),
            ("eq.fp", 14_401.0),
            ("eq.fp", -30.0),
            ("eq.qp", 0.05),
            ("eq.qp", 21.0),
            ("eq.fp", f32::NAN),
            ("eq.qp", f32::INFINITY),
        ] {
            let msg = refused(put(&core, path, &[ch, band], ParamValue::Float(v)));
            assert!(!msg.is_empty(), "{path} {v}");
        }
        // A dB value would land in fp as a few hertz: refused on a Linkwitz band.
        let msg = refused(put(&core, "eq.gain", &[ch, band], ParamValue::Float(6.0)));
        assert!(msg.contains("eq.fp"), "{msg}");
        // fp and Qp mean nothing on another type.
        let msg = refused(put(&core, "eq.fp", &[ch, 4], ParamValue::Float(30.0)));
        assert!(msg.contains("Linkwitz"), "{msg}");
        let msg = refused(put(&core, "eq.qp", &[ch, 4], ParamValue::Float(0.7)));
        assert!(msg.contains("eq.type"), "{msg}");
        // Nor on a crossover band (20..23).
        assert!(refused(put(&core, "eq.qp", &[ch, 20], ParamValue::Float(0.7))).contains("eq.qp"));
        assert_eq!(core.get_state(), before);
    }

    /// Other types write exactly as before; a type change into a Linkwitz
    /// Transform seeds a neutral one (fp = f0, Qp = Q0), and out of one
    /// zeroes the gain, as the Console does.
    #[test]
    fn wp_cg34_other_bands_are_unchanged_and_retyping_is_the_consoles() {
        let (a, b) = (demo(), demo());
        let (ch, band) = (9, 5);
        put(&a, "eq.type", &[ch, band], ParamValue::Choice(1)).unwrap();
        put(&a, "eq.freq", &[ch, band], ParamValue::Float(800.0)).unwrap();
        put(&a, "eq.q", &[ch, band], ParamValue::Float(2.0)).unwrap();
        assert_eq!(
            put(&a, "eq.gain", &[ch, band], ParamValue::Float(4.04)).unwrap(),
            ParamValue::Float(4.0)
        );
        let bypass = a.get_state().filters[usize::from(ch)][usize::from(band)].bypass;
        let bell = FilterParams {
            filter_type: 1,
            bypass,
            freq: 800.0,
            q: 2.0,
            gain: 4.04,
            qp: 0.707,
        };
        b.set_filter(ch, band, bell).unwrap();
        assert_eq!(a.get_state(), b.get_state());

        put(&a, "eq.type", &[ch, band], ParamValue::Choice(11)).unwrap();
        let p = a.get_state().filters[usize::from(ch)][usize::from(band)];
        assert_eq!(
            (p.filter_type, p.freq, p.q, p.gain, p.qp),
            (11, 800.0, 2.0, 800.0, 1024.0 / 512.0)
        );
        put(&a, "eq.type", &[ch, band], ParamValue::Choice(2)).unwrap();
        let p = a.get_state().filters[usize::from(ch)][usize::from(band)];
        assert_eq!((p.filter_type, p.freq, p.q, p.gain), (2, 800.0, 2.0, 0.0));
    }
}
