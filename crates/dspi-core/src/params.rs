//! DSP parameters: EQ, volume, loudness, crossfeed, delays, leveller, matrix, outputs, names.
//!
//! Every setter follows the same three steps (PLAN §1.4):
//!
//! 1. take the lock and check the connection is writable
//!    ([`Inner::writable_conn`]: [`DspiError::Disconnected`] with no device,
//!    [`DspiError::ReadOnly`] for another wire format), then validate the
//!    arguments against the mirror's channel layout;
//! 2. send through the session's fast path ([`dspi_session::Session::send`],
//!    validated and encoded by the parameter registry), or a raw opcode on
//!    the [`crate::Writable`] where the registry has no whole-value path (an
//!    EQ band);
//! 3. only once the device has accepted the transfer, update the record
//!    field and the image (`encode_bulk`, then `decode_bulk` of the result,
//!    so the mirror reads exactly as the device's image would).
//!
//! Values are shaped as the old desktop core shaped them (`commands.rs`):
//! gains clamped, then quantized to 0.1 dB ([`quantize_gain`]); delays
//! quantized to 0.0001 ms ([`quantize_delay`]), then clamped to the delay
//! line; the master volume in 0.5 dB steps with −128 as mute. Values the
//! firmware clamps (loudness, crossfeed, leveller) are clamped to the same
//! range here, the registry's, so the mirror holds what the device stores.
//! A NaN is refused ([`DspiError::InvalidArgument`]) rather than sent.
//!
//! Owned by WP C3 (docs/core-merge/PLAN.md).

use dspi_proto::FilterType;
use dspi_proto::generated::{filter, opcodes as op};
use dspi_proto::packets::MatrixRoutePacket;
use dspi_proto::registry::{Kind, by_path};
use dspi_proto::value::{EqParamPacket, Value};

use crate::consts::*;
use crate::journal::journal;
use crate::records::*;
use crate::{DspiCore, DspiError, Inner, Result, quantize_delay, quantize_gain, state};

/// -- WP FIXM1 -- The device lock's reason for the master-volume save.
const SAVE_MASTER_VOLUME_REASON: &str = "Saving master volume";

/// First crossover band index on the wire (`XOVER_BAND_BASE`, 20).
const XOVER_BASE: u8 = {
    assert!(filter::XOVER_BAND_BASE <= u8::MAX as u16);
    filter::XOVER_BAND_BASE as u8
};

/// Crossover bands per output channel.
const XOVER_BANDS: u8 = {
    assert!(MAX_XOVER_BANDS <= u8::MAX as usize);
    MAX_XOVER_BANDS as u8
};

/// PEQ bands the API addresses per channel.
const PEQ_BANDS: u8 = {
    assert!(BANDS_PER_CHANNEL <= u8::MAX as usize);
    BANDS_PER_CHANNEL as u8
};

/// `REQ_GET_CHANNEL_NAME` reply size (`WIRE_NAME_LEN`).
const NAME_LEN_U16: u16 = {
    assert!(CHANNEL_NAME_LEN <= u16::MAX as usize);
    CHANNEL_NAME_LEN as u16
};

/// Highest `InputSource` the firmware selects (`INPUT_SOURCE_MAX`,
/// audio_input.h:26: S/PDIF input 4; WP SW1: 7, the audio link, on a unit
/// that has it, SYS2 5.1).
const INPUT_SOURCE_MAX: u8 = crate::alink::INPUT_SOURCE_ALINK;

/// The master volume value that mutes (`REQ_SET_MASTER_VOLUME`).
const MASTER_VOL_MUTE_DB: f32 = -128.0;

/// Lowest preamp the old core allowed, on both platforms.
const PREAMP_MIN_DB: f32 = -60.0;

#[cfg_attr(feature = "uniffi", uniffi::export)]
impl DspiCore {
    /// PEQ band 0..9 of wire channel `ch`, sent as one whole band
    /// (`REQ_SET_EQ_PARAM`).
    ///
    /// The gain is quantized to 0.1 dB, except on a Linkwitz Transform, where
    /// it carries the target frequency fp in Hz. A Linkwitz Transform's
    /// target Q `qp` is clamped to 0.1..20; other types ignore `qp` (the
    /// mirror reads it back as 0.707).
    ///
    /// Errors: [`DspiError::InvalidArgument`] for a channel or band out of
    /// range, a non-finite frequency, Q or gain, or a NaN `qp` on a
    /// Linkwitz Transform; the connection and transfer errors of every
    /// setter ([`DspiError::Disconnected`], [`DspiError::ReadOnly`],
    /// [`DspiError::Transfer`]).
    pub fn set_filter(&self, ch: u8, band: u8, params: FilterParams) -> Result<()> {
        let mut inner = self.writing()?;
        let c = channel_index(&inner.state, ch)?;
        if band >= PEQ_BANDS {
            return Err(invalid(format!("PEQ band {band} (0..{PEQ_BANDS})")));
        }
        let mut p = checked_band(params)?;
        if !FilterType::from_raw(p.filter_type).is_linkwitz() {
            p.gain = quantize_gain(p.gain);
        }
        let change = band_journal(&inner, ch, band)?;
        send_band(&mut inner, ch, band, &p)?;
        if let Some(slot) = inner
            .state
            .filters
            .get_mut(c)
            .and_then(|r| r.get_mut(usize::from(band)))
        {
            *slot = p;
        }
        commit(&mut inner);
        change.done(&mut inner);
        Ok(())
    }

    /// Crossover band 0..3 of an output's wire channel `ch` (wire band
    /// `20 + xband`). The type must be flat (0) or a crossover type (32..63);
    /// the gain is sent as given.
    ///
    /// Errors: [`DspiError::InvalidArgument`] for an input channel, a band
    /// out of range, another filter type or a non-finite value; the
    /// connection and transfer errors of every setter.
    pub fn set_crossover(&self, ch: u8, xband: u8, params: FilterParams) -> Result<()> {
        let mut inner = self.writing()?;
        let c = output_channel_index(&inner.state, ch)?;
        if xband >= XOVER_BANDS {
            return Err(invalid(format!(
                "crossover band {xband} (0..{XOVER_BANDS})"
            )));
        }
        let t = FilterType::from_raw(params.filter_type);
        if t != FilterType::Flat && !t.is_crossover() {
            return Err(invalid(format!(
                "filter type {} is not a crossover type",
                params.filter_type
            )));
        }
        let p = checked_band(params)?;
        let band = XOVER_BASE
            .checked_add(xband)
            .ok_or_else(|| invalid(format!("crossover band {xband}")))?;
        let change = band_journal(&inner, ch, band)?;
        send_band(&mut inner, ch, band, &p)?;
        if let Some(slot) = inner
            .state
            .xover
            .get_mut(c)
            .and_then(|r| r.get_mut(usize::from(xband)))
        {
            *slot = p;
        }
        commit(&mut inner);
        change.done(&mut inner);
        Ok(())
    }

    /// Bypass (`true`) or re-enable one band of wire channel `ch`, keeping
    /// its settings (`REQ_SET_BAND_BYPASS`). `band`: PEQ 0..9, or crossover
    /// 20..23 on an output channel.
    ///
    /// Errors: [`DspiError::InvalidArgument`] for a channel or band out of
    /// range; the connection and transfer errors of every setter.
    pub fn set_band_bypass(&self, ch: u8, band: u8, bypass: bool) -> Result<()> {
        let mut inner = self.writing()?;
        let c = channel_index(&inner.state, ch)?;
        let xband = band.checked_sub(XOVER_BASE).filter(|x| *x < XOVER_BANDS);
        if band >= PEQ_BANDS && xband.is_none() {
            return Err(invalid(format!(
                "band {band} (PEQ 0..9 or crossover 20..23)"
            )));
        }
        if xband.is_some() {
            output_channel_index(&inner.state, ch)?;
        }
        let change = inner.journal_begin("eq.bypass", &[ch, band])?;
        send(&mut inner, "eq.bypass", &[ch, band], Value::Bool(bypass))?;
        let slot = match xband {
            Some(x) => inner
                .state
                .xover
                .get_mut(c)
                .and_then(|r| r.get_mut(usize::from(x))),
            None => inner
                .state
                .filters
                .get_mut(c)
                .and_then(|r| r.get_mut(usize::from(band))),
        };
        if let Some(slot) = slot {
            slot.bypass = bypass;
        }
        commit(&mut inner);
        change.done(&mut inner, Value::Bool(bypass));
        Ok(())
    }

    /// Legacy preamp: every input to the same value (`REQ_SET_PREAMP`), in
    /// dB, clamped to −60..[`DspiCore::preamp_max_db`] and quantized to
    /// 0.1 dB.
    ///
    /// Errors: [`DspiError::InvalidArgument`] for a NaN; the connection and
    /// transfer errors of every setter.
    pub fn set_preamp(&self, db: f32) -> Result<()> {
        let mut inner = self.writing()?;
        let val = preamp_db(&inner, db)?;
        let change = inner.journal_begin("pre.all", &[])?;
        send(&mut inner, "pre.all", &[], Value::Float(val))?;
        inner.state.preamp_db = val;
        inner
            .state
            .input_preamp_db
            .iter_mut()
            .for_each(|v| *v = val);
        commit(&mut inner);
        change.done(&mut inner, Value::Float(val));
        Ok(())
    }

    /// Preamp of one input (`REQ_SET_PREAMP_CH`), in dB, clamped to
    /// −60..[`DspiCore::preamp_max_db`] and quantized to 0.1 dB. Input 0 is
    /// also the legacy `preamp_db`.
    ///
    /// Errors: [`DspiError::InvalidArgument`] for an input out of range or a
    /// NaN; the connection and transfer errors of every setter.
    pub fn set_input_preamp(&self, input: u8, db: f32) -> Result<()> {
        let mut inner = self.writing()?;
        let i = input_index(&inner.state, input)?;
        let val = preamp_db(&inner, db)?;
        let change = inner.journal_begin("pre", &[input])?;
        send(&mut inner, "pre", &[input], Value::Float(val))?;
        if let Some(v) = inner.state.input_preamp_db.get_mut(i) {
            *v = val;
        }
        if input == 0 {
            inner.state.preamp_db = val;
        }
        commit(&mut inner);
        change.done(&mut inner, Value::Float(val));
        Ok(())
    }

    // -- WP P1PR --
    /// Preamp of a linked input pair: `input` and its partner (`input ^ 1`,
    /// inputs 2p and 2p+1) both to `db`, shaped as
    /// [`DspiCore::set_input_preamp`] shapes it, in one undo group
    /// ([`DspiCore::begin_group`]): one undo step labelled "Input trim,
    /// linked pair". `input`, the side the user moved, is written first.
    /// The link itself is the client's (a document's `inputPairLinked`):
    /// call this while the pair is linked, `set_input_preamp` otherwise.
    ///
    /// Both inputs and the value are checked before anything is sent. A
    /// failure on the second write leaves the first one written (one undo
    /// step takes it back).
    ///
    /// Errors: [`DspiError::InvalidArgument`] for an input or its partner
    /// out of range, or a NaN; the connection and transfer errors of every
    /// setter.
    pub fn set_input_pair_preamp(&self, input: u8, db: f32) -> Result<()> {
        let partner = input ^ 1;
        {
            let inner = self.lock();
            input_index(&inner.state, input)?;
            input_index(&inner.state, partner)?;
            preamp_db(&inner, db)?;
        }
        self.begin_group(PAIR_PREAMP_LABEL.into());
        let r = self
            .set_input_preamp(input, db)
            .and_then(|()| self.set_input_preamp(partner, db));
        self.end_group();
        r
    }

    // -- WP PAIRBAND --
    /// PEQ band `band` (0..9) of a linked input pair: `input` and its
    /// partner (`input ^ 1`, inputs 2p and 2p+1, which are wire channels
    /// 2p and 2p+1) both to `params`, as [`DspiCore::set_filter`] writes
    /// it, in one undo group ([`DspiCore::begin_group`]): one undo step
    /// labelled "EQ band, linked pair", and undo and redo move both sides
    /// together. `input`, the side the person edited, is written first.
    /// The link itself is the client's (a document's `inputPairLinked`):
    /// call this for every band edit while the pair is linked (the band
    /// list, a menu, a typed value, each tick of a graph drag), and
    /// `set_filter` otherwise.
    ///
    /// In a drag ([`DspiCore::set_dragging`]) the group joins the drag's
    /// step, so a whole drag of a pair's bands is one step that holds each
    /// band of each side once (WP P0W2) and undo puts both sides back as
    /// they were when the drag began. A value someone else changes on one
    /// side is judged on its own, as in every step: undo leaves theirs and
    /// names them, and puts the rest back.
    ///
    /// The inputs, the band and the values are checked before anything is
    /// sent. A failure on the second write leaves the first one written
    /// (one undo step takes it back).
    ///
    /// Errors: [`DspiError::InvalidArgument`] for an input or its partner
    /// out of range, a band out of range, or the values `set_filter`
    /// refuses; the connection and transfer errors of every setter.
    pub fn set_input_pair_filter(&self, input: u8, band: u8, params: FilterParams) -> Result<()> {
        let partner = pair_band_check(self, input, band)?;
        checked_band(params)?;
        self.begin_group(PAIR_BAND_LABEL.into());
        let r = self
            .set_filter(input, band, params)
            .and_then(|()| self.set_filter(partner, band, params));
        self.end_group();
        r
    }

    // -- WP PAIRBAND --
    /// Bypass (`true`) or re-enable PEQ band `band` (0..9) of a linked
    /// input pair, both sides, keeping their settings: `input` and then
    /// `input ^ 1` through [`DspiCore::set_band_bypass`], in one undo group
    /// with the label of [`DspiCore::set_input_pair_filter`]. One undo
    /// step; checks and errors as there.
    pub fn set_input_pair_band_bypass(&self, input: u8, band: u8, bypass: bool) -> Result<()> {
        let partner = pair_band_check(self, input, band)?;
        self.begin_group(PAIR_BAND_LABEL.into());
        let r = self
            .set_band_bypass(input, band, bypass)
            .and_then(|()| self.set_band_bypass(partner, band, bypass));
        self.end_group();
        r
    }

    /// Bypass the whole EQ (`true`) or run it.
    ///
    /// Errors: the connection and transfer errors of every setter.
    pub fn set_bypass(&self, enabled: bool) -> Result<()> {
        self.set_scalar("bypass", Value::Bool(enabled), |s| s.bypass = enabled)
    }

    /// Device master volume in dB: −128 (or anything below) mutes, otherwise
    /// −127..0 in 0.5 dB steps (rounded, then clamped).
    ///
    /// The firmware answers with the legacy v1 volume notification and then
    /// a `PARAM_CHANGED`; both are the echo of this write.
    ///
    /// Errors: [`DspiError::InvalidArgument`] for a NaN; the connection and
    /// transfer errors of every setter.
    pub fn set_master_volume(&self, db: f32) -> Result<()> {
        let _lock = crate::limits::LockScope::allow(); // -- WP P1LK -- allowed while locked
        let db = not_nan(db, "master volume")?;
        let val = if db <= MASTER_VOL_MUTE_DB {
            MASTER_VOL_MUTE_DB
        } else {
            ((db * 2.0).round() / 2.0).clamp(-127.0, 0.0)
        };
        self.set_scalar("vol.master", Value::Float(val), |s| {
            s.master_volume_db = val;
        })
    }

    /// 0 = master volume independent of presets, 1 = saved with presets.
    ///
    /// Errors: [`DspiError::InvalidArgument`] for another mode; the
    /// connection and transfer errors of every setter.
    pub fn set_master_volume_mode(&self, mode: u8) -> Result<()> {
        self.set_scalar("vol.master.mode", Value::Choice(mode), |s| {
            s.master_volume_mode = mode;
        })
    }

    /// FLASH WRITE: store the live master volume for the independent mode
    /// (`REQ_SAVE_MASTER_VOLUME`), under the device lock like the other
    /// flash writes. Changes nothing in the mirror.
    ///
    /// Errors: [`DspiError::Refused`] with the device's status byte
    /// (`PRESET_*`) when the save fails; [`DspiError::Locked`] /
    /// [`DspiError::Denied`] (Link); the connection and transfer errors of
    /// every setter.
    pub fn save_master_volume(&self) -> Result<()> {
        // -- WP FIXM1 -- (review sw13 N1) the device lock, as save_preset.
        self.with_device_lock(SAVE_MASTER_VOLUME_REASON, |inner| {
            send(inner, "vol.master.save", &[], Value::Trigger)?;
            let code = inner
                .writable_conn()?
                .session()?
                .last_write_status()
                .ok_or_else(|| DspiError::Transfer {
                    msg: "no status reply to the master-volume save".into(),
                })?;
            crate::error::status(code)?;
            journal(inner, "vol.master.save", &[], Value::Trigger);
            Ok(())
        })
    }

    /// User volume (what the host's volume slider drives) in dB, clamped to
    /// −60..0.
    ///
    /// Errors: [`DspiError::InvalidArgument`] for a NaN; the connection and
    /// transfer errors of every setter.
    pub fn set_user_volume(&self, db: f32) -> Result<()> {
        let _lock = crate::limits::LockScope::allow(); // -- WP P1LK -- allowed while locked
        let val = not_nan(db, "user volume")?.clamp(-60.0, 0.0);
        self.set_scalar("vol.user", Value::Float(val), |s| s.user_volume_db = val)
    }

    /// Mute (`true`) or unmute the user volume.
    ///
    /// Errors: the connection and transfer errors of every setter.
    pub fn set_user_mute(&self, muted: bool) -> Result<()> {
        let _lock = crate::limits::LockScope::allow(); // -- WP P1LK -- allowed while locked
        self.set_scalar("vol.mute", Value::Bool(muted), |s| s.user_mute = muted)
    }

    /// Delay of wire channel `ch` in ms, quantized to 0.0001 ms; below 0 it
    /// is 0. An output channel's delay is its output delay, so it goes
    /// through [`DspiCore::set_output_delay`].
    ///
    /// Errors: [`DspiError::InvalidArgument`] for a channel out of range, a
    /// NaN, or a delay above the platform's [`DspiCore::max_delay_ms`] (the
    /// message names it; nothing is sent); the connection and transfer
    /// errors of every setter.
    pub fn set_delay(&self, ch: u8, ms: f32) -> Result<()> {
        let mut inner = self.writing()?;
        let c = channel_index(&inner.state, ch)?;
        if let Some(out) = ch.checked_sub(inner.state.num_input_channels) {
            drop(inner);
            return self.set_output_delay(out, ms);
        }
        let val = delay_ms(&inner, "ch.delay", ms)?;
        let change = inner.journal_begin("ch.delay", &[ch])?;
        send(&mut inner, "ch.delay", &[ch], Value::Float(val))?;
        if let Some(d) = inner.state.channel_delays.get_mut(c) {
            *d = val;
        }
        commit(&mut inner);
        change.done(&mut inner, Value::Float(val));
        Ok(())
    }

    /// Loudness compensation on (`true`) or off.
    ///
    /// Errors: the connection and transfer errors of every setter.
    pub fn set_loudness(&self, enabled: bool) -> Result<()> {
        self.set_scalar("loud.on", Value::Bool(enabled), |s| {
            s.loudness_enabled = enabled;
        })
    }

    /// Loudness reference level in dB SPL, clamped to 40..100 as the
    /// firmware does.
    ///
    /// Errors: [`DspiError::InvalidArgument`] for a NaN; the connection and
    /// transfer errors of every setter.
    pub fn set_loudness_ref(&self, spl: f32) -> Result<()> {
        let val = clamped("loud.ref", spl)?;
        self.set_scalar("loud.ref", Value::Float(val), |s| s.loudness_ref_spl = val)
    }

    /// Loudness intensity in percent, clamped to 0..200 as the firmware does.
    ///
    /// Errors: [`DspiError::InvalidArgument`] for a NaN; the connection and
    /// transfer errors of every setter.
    pub fn set_loudness_intensity(&self, pct: f32) -> Result<()> {
        let val = clamped("loud.intensity", pct)?;
        self.set_scalar("loud.intensity", Value::Float(val), |s| {
            s.loudness_intensity = val;
        })
    }

    /// Bit k: loudness compensates output k.
    ///
    /// Errors: [`DspiError::Unsupported`] on firmware without the per-output
    /// mask; the connection and transfer errors of every setter.
    pub fn set_loudness_mask(&self, mask: u16) -> Result<()> {
        self.set_scalar("loud.mask", Value::Mask(u32::from(mask)), |s| {
            s.loudness_output_mask = mask;
        })
    }

    /// Crossfeed on (`true`) or off.
    ///
    /// Errors: the connection and transfer errors of every setter.
    pub fn set_crossfeed(&self, enabled: bool) -> Result<()> {
        self.set_scalar("cf.on", Value::Bool(enabled), |s| {
            s.crossfeed_enabled = enabled;
        })
    }

    /// Crossfeed preset: 0 default, 1 Chu Moy, 2 Meier, 3 custom. Only the
    /// preset is sent: the device keeps its custom frequency and feed whatever
    /// the preset. While a named preset (0 to 2) is chosen, `get_state` gives
    /// that preset's frequency and feed (700 Hz / 4.5 dB, 700 Hz / 6 dB,
    /// 650 Hz / 9.5 dB), which the device runs; with Custom (3) it gives the
    /// custom pair again. A preset export, the bulk image and undo keep the
    /// custom pair.
    ///
    /// Errors: [`DspiError::InvalidArgument`] for another preset; the
    /// connection and transfer errors of every setter.
    pub fn set_crossfeed_preset(&self, preset: u8) -> Result<()> {
        self.set_scalar("cf.preset", Value::Choice(preset), |s| {
            s.crossfeed_preset = preset;
        })
    }

    /// Custom crossfeed frequency in Hz, clamped to 500..2000 as the firmware
    /// does.
    ///
    /// Errors: [`DspiError::InvalidArgument`] for a NaN; the connection and
    /// transfer errors of every setter.
    pub fn set_crossfeed_freq(&self, freq: f32) -> Result<()> {
        let val = clamped("cf.freq", freq)?;
        self.set_scalar("cf.freq", Value::Float(val), |s| s.crossfeed_freq = val)
    }

    /// Custom crossfeed feed level in dB, clamped to 0..15 as the firmware
    /// does.
    ///
    /// Errors: [`DspiError::InvalidArgument`] for a NaN; the connection and
    /// transfer errors of every setter.
    pub fn set_crossfeed_feed(&self, feed: f32) -> Result<()> {
        let val = clamped("cf.feed", feed)?;
        self.set_scalar("cf.feed", Value::Float(val), |s| s.crossfeed_feed = val)
    }

    // -- WP P1AFIX -- (review p1a N1)
    /// The crossfeed's custom pair (frequency and feed) as the device keeps
    /// it: what Custom runs. Under a named preset `get_state` gives that
    /// preset's values instead; this gives the custom pair whatever the
    /// preset, from the mirror (no transfer). It is what
    /// `set_crossfeed_freq`/`set_crossfeed_feed` write, an export saves, and
    /// `config_diff`/`config_preview` compare.
    ///
    /// Errors: [`DspiError::Disconnected`] with no device.
    pub fn crossfeed_custom_pair(&self) -> Result<CrossfeedPair> {
        let inner = self.lock();
        if inner.conn.is_none() {
            return Err(DspiError::Disconnected);
        }
        Ok(CrossfeedPair {
            freq_hz: inner.state.crossfeed_freq,
            feed_db: inner.state.crossfeed_feed,
        })
    }

    /// Crossfeed interaural time delay on (`true`) or off.
    ///
    /// Errors: the connection and transfer errors of every setter.
    pub fn set_crossfeed_itd(&self, enabled: bool) -> Result<()> {
        self.set_scalar("cf.itd", Value::Bool(enabled), |s| {
            s.crossfeed_itd = enabled;
        })
    }

    /// Bit p: crossfeed runs on output pair p. The firmware keeps only the
    /// bits of pairs it has; its echo corrects the mirror.
    ///
    /// Errors: [`DspiError::Unsupported`] on firmware without the mask; the
    /// connection and transfer errors of every setter.
    pub fn set_crossfeed_outputs(&self, pair_mask: u8) -> Result<()> {
        self.set_scalar("cf.outputs", Value::Mask(u32::from(pair_mask)), |s| {
            s.crossfeed_output_pair_mask = pair_mask;
        })
    }

    /// Volume leveller on (`true`) or off.
    ///
    /// Errors: the connection and transfer errors of every setter.
    pub fn set_leveller_enabled(&self, enabled: bool) -> Result<()> {
        self.set_scalar("lev.on", Value::Bool(enabled), |s| {
            s.leveller_enabled = enabled;
        })
    }

    /// Leveller amount in percent, clamped to 0..100 as the firmware does.
    ///
    /// Errors: [`DspiError::InvalidArgument`] for a NaN; the connection and
    /// transfer errors of every setter.
    pub fn set_leveller_amount(&self, pct: f32) -> Result<()> {
        let val = clamped("lev.amount", pct)?;
        self.set_scalar("lev.amount", Value::Float(val), |s| s.leveller_amount = val)
    }

    /// 0 slow, 1 medium, 2 fast.
    ///
    /// Errors: [`DspiError::InvalidArgument`] for another speed; the
    /// connection and transfer errors of every setter.
    pub fn set_leveller_speed(&self, speed: u8) -> Result<()> {
        self.set_scalar("lev.speed", Value::Choice(speed), |s| {
            s.leveller_speed = speed;
        })
    }

    /// Leveller maximum gain in dB, clamped to 0..35 as the firmware does.
    ///
    /// Errors: [`DspiError::InvalidArgument`] for a NaN; the connection and
    /// transfer errors of every setter.
    pub fn set_leveller_max_gain(&self, db: f32) -> Result<()> {
        let val = clamped("lev.maxgain", db)?;
        self.set_scalar("lev.maxgain", Value::Float(val), |s| {
            s.leveller_max_gain_db = val;
        })
    }

    /// Leveller look-ahead on (`true`) or off.
    ///
    /// Errors: the connection and transfer errors of every setter.
    pub fn set_leveller_lookahead(&self, enabled: bool) -> Result<()> {
        self.set_scalar("lev.lookahead", Value::Bool(enabled), |s| {
            s.leveller_lookahead = enabled;
        })
    }

    /// Leveller gate threshold in dB, clamped to −96..0 as the firmware does.
    ///
    /// Errors: [`DspiError::InvalidArgument`] for a NaN; the connection and
    /// transfer errors of every setter.
    pub fn set_leveller_gate(&self, db: f32) -> Result<()> {
        let val = clamped("lev.gate", db)?;
        self.set_scalar("lev.gate", Value::Float(val), |s| s.leveller_gate_db = val)
    }

    /// Leveller detector and apply masks (bit k: output k).
    ///
    /// Errors: [`DspiError::Unsupported`] on firmware without the masks; the
    /// connection and transfer errors of every setter.
    pub fn set_leveller_masks(&self, detector: u8, apply: u8) -> Result<()> {
        // `lev.masks` is a 16-bit mask on the wire: detector, then apply.
        let both = u32::from(detector) | (u32::from(apply) << 8);
        self.set_scalar("lev.masks", Value::Mask(both), |s| {
            s.leveller_detector_mask = detector;
            s.leveller_apply_mask = apply;
        })
    }

    /// Input source: 0 USB, 1 S/PDIF, 2 I2S, 3 ADAT, 4..6 S/PDIF inputs 2..4,
    /// 7 the audio link (WP SW1). The firmware switches on its main loop and
    /// ignores a source that is not selectable (a disabled S/PDIF input); its
    /// notification at the switch, or a re-read, corrects the mirror.
    ///
    /// Errors: [`DspiError::InvalidArgument`] above 7;
    /// [`DspiError::Unsupported`], with no traffic, for 7 on a unit without
    /// the link input (`alink_info`'s `alink_input`); the connection and
    /// transfer errors of every setter.
    pub fn set_input_source(&self, source: u8) -> Result<()> {
        // -- WP SW1 --
        self.ensure_probed();
        let mut inner = self.writing()?;
        if source > INPUT_SOURCE_MAX {
            return Err(invalid(format!(
                "input source {source} (0..={INPUT_SOURCE_MAX})"
            )));
        }
        // -- WP SW1 --
        if !crate::alink::source_selectable(inner.conn()?, source) {
            return Err(DspiError::Unsupported);
        }
        let change = inner.journal_begin("in.source", &[])?;
        send(&mut inner, "in.source", &[], Value::Choice(source))?;
        inner.state.input_source = source;
        commit(&mut inner);
        change.done(&mut inner, Value::Choice(source));
        Ok(())
    }

    /// One matrix crosspoint: route `input` to `output`, with a gain in dB
    /// (quantized to 0.1 dB) and polarity.
    ///
    /// Errors: [`DspiError::InvalidArgument`] for an input or output out of
    /// range or a non-finite gain; the connection and transfer errors of
    /// every setter.
    pub fn set_matrix_route(
        &self,
        input: u8,
        output: u8,
        enabled: bool,
        gain: f32,
        invert: bool,
    ) -> Result<()> {
        let mut inner = self.writing()?;
        let i = input_index(&inner.state, input)?;
        let o = output_index(&inner.state, output)?;
        let gain = quantize_gain(finite(gain, "matrix gain")?);
        let packet = MatrixRoutePacket {
            input,
            output,
            enabled,
            phase_invert: invert,
            gain_db: gain,
        };
        let change = inner.journal_begin("mix", &[input, output])?;
        send(
            &mut inner,
            "mix",
            &[input, output],
            Value::Bytes(packet.encode().to_vec()),
        )?;
        let s = &mut inner.state;
        if let Some(v) = s.matrix_routing.get_mut(i).and_then(|r| r.get_mut(o)) {
            *v = enabled;
        }
        if let Some(v) = s.matrix_gain.get_mut(i).and_then(|r| r.get_mut(o)) {
            *v = gain;
        }
        if let Some(v) = s.matrix_invert.get_mut(i).and_then(|r| r.get_mut(o)) {
            *v = invert;
        }
        commit(&mut inner);
        change.done(&mut inner, Value::Bytes(packet.encode().to_vec()));
        Ok(())
    }

    /// Enable or disable an output. Returns the resulting state, read back
    /// from the device: the firmware silently refuses an enable that would
    /// collide on Core 1 (PDM against the EQ-worker outputs). An enable is
    /// checked first (`REQ_GET_CORE1_CONFLICT`) and not sent when it would
    /// collide. The Core 1 mode is re-read afterwards when the device
    /// answers it.
    ///
    /// Errors: [`DspiError::InvalidArgument`] for an output out of range; the
    /// connection and transfer errors of every setter. A failed read-back is
    /// not an error: the requested state (or, after a refused enable, the
    /// previous one) is assumed, as the old core did.
    pub fn set_output_enable(&self, output: u8, enabled: bool) -> Result<bool> {
        self.output_enable(output, enabled)
            .map(|(actual, _)| actual)
    }

    /// Output gain in dB, clamped to −60..+20 and quantized to 0.1 dB.
    ///
    /// Errors: [`DspiError::InvalidArgument`] for an output out of range or
    /// a NaN; the connection and transfer errors of every setter.
    pub fn set_output_gain(&self, output: u8, db: f32) -> Result<()> {
        let mut inner = self.writing()?;
        let o = output_index(&inner.state, output)?;
        let val = quantize_gain(clamped("out.gain", db)?);
        let change = inner.journal_begin("out.gain", &[output])?;
        send(&mut inner, "out.gain", &[output], Value::Float(val))?;
        if let Some(v) = inner.state.output_gain_db.get_mut(o) {
            *v = val;
        }
        commit(&mut inner);
        change.done(&mut inner, Value::Float(val));
        Ok(())
    }

    /// Mute (`true`) or unmute one output.
    ///
    /// Errors: [`DspiError::InvalidArgument`] for an output out of range; the
    /// connection and transfer errors of every setter.
    pub fn set_output_mute(&self, output: u8, muted: bool) -> Result<()> {
        let _lock = crate::limits::LockScope::allow(); // -- WP P1LK -- allowed while locked
        let mut inner = self.writing()?;
        let o = output_index(&inner.state, output)?;
        let change = inner.journal_begin("out.mute", &[output])?;
        send(&mut inner, "out.mute", &[output], Value::Bool(muted))?;
        if let Some(v) = inner.state.output_muted.get_mut(o) {
            *v = muted;
        }
        commit(&mut inner);
        change.done(&mut inner, Value::Bool(muted));
        Ok(())
    }

    /// Output delay in ms, quantized to 0.0001 ms; below 0 it is 0. It is
    /// also the delay of the output's wire channel.
    ///
    /// Errors: [`DspiError::InvalidArgument`] for an output out of range, a
    /// NaN, or a delay above the platform's [`DspiCore::max_delay_ms`] (the
    /// message names it; nothing is sent); the connection and transfer
    /// errors of every setter.
    pub fn set_output_delay(&self, output: u8, ms: f32) -> Result<()> {
        let mut inner = self.writing()?;
        let o = output_index(&inner.state, output)?;
        let val = delay_ms(&inner, "out.delay", ms)?;
        let change = inner.journal_begin("out.delay", &[output])?;
        send(&mut inner, "out.delay", &[output], Value::Float(val))?;
        let s = &mut inner.state;
        if let Some(v) = s.output_delay_ms.get_mut(o) {
            *v = val;
        }
        let ch = usize::from(s.num_input_channels).checked_add(o);
        if let Some(v) = ch.and_then(|c| s.channel_delays.get_mut(c)) {
            *v = val;
        }
        commit(&mut inner);
        change.done(&mut inner, Value::Float(val));
        Ok(())
    }

    /// Name of wire channel `ch`, cut to 31 bytes on a character boundary
    /// (the firmware keeps 31 and a NUL).
    ///
    /// Errors: [`DspiError::InvalidArgument`] for a channel out of range; the
    /// connection and transfer errors of every setter.
    pub fn set_channel_name(&self, ch: u8, name: String) -> Result<()> {
        let mut inner = self.writing()?;
        let c = channel_index(&inner.state, ch)?;
        let name = cut_name(&name).to_owned();
        let change = inner.journal_begin("ch.name", &[ch])?;
        send(&mut inner, "ch.name", &[ch], Value::Text(name.clone()))?;
        if let Some(v) = inner.state.channel_names.get_mut(c) {
            v.clone_from(&name);
        }
        commit(&mut inner);
        change.done(&mut inner, Value::Text(name));
        Ok(())
    }

    /// Reads the name of wire channel `ch` from the device
    /// (`REQ_GET_CHANNEL_NAME`) and keeps it in the mirror. Works on a
    /// read-only connection too (its image is then left alone).
    ///
    /// Errors: [`DspiError::Disconnected`] with no device;
    /// [`DspiError::InvalidArgument`] for a channel out of range; the
    /// transfer's error.
    pub fn get_channel_name(&self, ch: u8) -> Result<String> {
        let mut inner = self.lock();
        let writable = inner.conn()?.writable();
        let c = channel_index(&inner.state, ch)?;
        let data = inner
            .conn()?
            .get_upto(op::REQ_GET_CHANNEL_NAME, u16::from(ch), NAME_LEN_U16)?;
        let bytes = data
            .get(..CHANNEL_NAME_LEN.saturating_sub(1))
            .unwrap_or(&data);
        let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
        let name = String::from_utf8_lossy(bytes.get(..end).unwrap_or_default()).into_owned();
        if let Some(v) = inner.state.channel_names.get_mut(c) {
            v.clone_from(&name);
        }
        if writable {
            commit(&mut inner);
        }
        Ok(name)
    }
}

impl DspiCore {
    // -- WP CG36 --
    /// [`DspiCore::set_output_enable`], also answering whether the enable
    /// was not sent for a Core 1 conflict (`write_param` refuses it in
    /// words).
    pub(crate) fn output_enable(&self, output: u8, enabled: bool) -> Result<(bool, bool)> {
        let mut inner = self.writing()?;
        let o = output_index(&inner.state, output)?;
        let before = inner.state.output_enabled.get(o).copied().unwrap_or(false);
        let change = inner.journal_begin("out.enable", &[output])?;
        // `Session::send` runs no interlock, and the firmware would skip a
        // colliding enable without a word.
        let conflict = enabled
            && inner
                .writable_conn()?
                .session()?
                .try_core1_conflict(output)?;
        if conflict {
            log::info!("output {output}: not enabled, Core 1 runs the other side");
        } else {
            send(&mut inner, "out.enable", &[output], Value::Bool(enabled))?;
        }
        let conn = inner.conn()?;
        let fallback = if conflict { before } else { enabled };
        let actual = conn
            .get(op::REQ_GET_OUTPUT_ENABLE, u16::from(output), 1)
            .ok()
            .and_then(|b| b.first().map(|v| *v != 0))
            .unwrap_or(fallback);
        let core1 = conn
            .get(op::REQ_GET_CORE1_MODE, 0, 1)
            .ok()
            .and_then(|b| b.first().copied());
        if let Some(v) = inner.state.output_enabled.get_mut(o) {
            *v = actual;
        }
        if let Some(mode) = core1 {
            inner.state.core1_mode = mode;
        }
        commit(&mut inner);
        if !conflict {
            change.done(&mut inner, Value::Bool(actual));
        }
        Ok((actual, conflict))
    }

    // ── helpers ──────────────────────────────────────────────────────

    /// The lock, once the connection is known to be writable.
    pub(crate) fn writing(&self) -> Result<std::sync::MutexGuard<'_, Inner>> {
        let mut inner = self.lock();
        inner.writable_conn()?;
        Ok(inner)
    }

    /// A setter without indices: send `value` to `path`, then apply `update`
    /// to the mirror.
    fn set_scalar(
        &self,
        path: &'static str,
        value: Value,
        update: impl FnOnce(&mut DspState),
    ) -> Result<()> {
        let mut inner = self.writing()?;
        let change = inner.journal_begin(path, &[])?;
        send(&mut inner, path, &[], value.clone())?;
        update(&mut inner.state);
        commit(&mut inner);
        change.done(&mut inner, value);
        Ok(())
    }
}

// ── Sending ────────────────────────────────────────────────────────────

// -- WP P0W --
/// Begin the journal entry of a whole-band write: journalled under the one
/// field it moves (`eq.freq`, ...) or as a whole band (`journal::EQ_BAND`).
fn band_journal(inner: &Inner, ch: u8, band: u8) -> Result<BandChange> {
    crate::journal::check_replay(inner)?;
    Ok(BandChange {
        ch,
        band,
        was: crate::write_param::band(&inner.state, ch, band),
    })
}

/// A whole-band write in progress: the band as it was.
struct BandChange {
    ch: u8,
    band: u8,
    was: Option<FilterParams>,
}

impl BandChange {
    /// Record the write, the band now in the mirror.
    fn done(self, inner: &mut Inner) {
        let Some(now) = crate::write_param::band(&inner.state, self.ch, self.band) else {
            return;
        };
        let was = self.was.unwrap_or(now);
        let path = crate::journal::band_change(&was, &now);
        let ix = [self.ch, self.band];
        let pick = |p: &FilterParams| -> Value {
            match path {
                "eq.bypass" => Value::Bool(p.bypass),
                "eq.freq" => Value::Float(p.freq),
                "eq.q" => Value::Float(p.q),
                "eq.gain" | "eq.fp" => Value::Float(p.gain),
                "eq.qp" => Value::Float(p.qp),
                _ => Value::Bytes(crate::journal::band_bytes(p)),
            }
        };
        let change = crate::journal::Pending::of(path, &ix, self.was.as_ref().map(pick));
        change.done_with(inner, pick(&now));
    }
}

/// Send through the session's fast path (registry-validated). The image
/// `attach_probed` read no longer describes the device after a write, so
/// `fetch_all` must read it again (`fresh_image`).
fn send(inner: &mut Inner, path: &str, indices: &[u8], value: Value) -> Result<()> {
    // -- WP P1LM -- a limits profile refuses a value past a limit, unsent
    crate::limits::check_write(inner, path, indices, &value)?;
    let mut w = inner.writable_conn()?;
    w.conn().fresh_image = false;
    w.session()?.send(path, indices, value)?;
    Ok(())
}

/// Send a raw OUT request, for the values the registry cannot carry.
fn send_raw(inner: &mut Inner, opcode: u8, value: u16, data: &[u8]) -> Result<()> {
    let mut w = inner.writable_conn()?;
    w.conn().fresh_image = false;
    w.set(opcode, value, data)
}

/// Send one whole band (`REQ_SET_EQ_PARAM`): the session's field path would
/// read the band back first, and the caller holds all of it. The Linkwitz
/// `qp` sidecar goes only with a Linkwitz Transform.
fn send_band(inner: &mut Inner, ch: u8, band: u8, p: &FilterParams) -> Result<()> {
    // -- WP P1LM -- a limits profile refuses a boost or a lost high-pass
    crate::limits::check_band(inner, ch, band, p)?;
    let filter_type = FilterType::from_raw(p.filter_type);
    let packet = EqParamPacket {
        channel: ch,
        band,
        filter_type,
        bypass: p.bypass,
        freq: p.freq,
        q: p.q,
        gain_db: p.gain,
        qp: filter_type.is_linkwitz().then_some(p.qp),
    };
    send_raw(inner, op::REQ_SET_EQ_PARAM, 0, &packet.encode())
}

/// Write the record back into the image and decode it again, so every field
/// reads as the device's image would (a cut name, a Linkwitz `qp` rounded to
/// 1/512, an unused `qp` as 0.707). Only for a writable connection, whose
/// image is V32.
fn commit(inner: &mut Inner) {
    let image = state::encode_bulk(&inner.state);
    if let Err(e) = state::decode_bulk(&image, &mut inner.state) {
        // Unreachable: `encode_bulk` always yields a V32 image of the right
        // size. Keep the image the record encodes to anyway.
        log::warn!("the mirror's own image did not decode: {e}");
        inner.state.bulk_raw = image;
    }
}

// -- WP P1XF --
/// The named crossfeed presets' (frequency Hz, feed dB) by preset number:
/// Default, Chu Moy, Meier, as the firmware runs them (`crossfeed.c`
/// `presets`). Preset 3 is Custom, which runs the custom pair.
pub(crate) const CROSSFEED_PRESETS: [(f32, f32); 3] = [(700.0, 4.5), (700.0, 6.0), (650.0, 9.5)];

/// The mirror as clients read it: under a named crossfeed preset,
/// `crossfeed_freq` and `crossfeed_feed` are that preset's values, so every
/// client's graph draws what the device runs. The core's own record and
/// `bulk_raw` keep the device's custom pair, which the export, the journal and
/// Custom use.
pub(crate) fn crossfeed_shown(mut s: DspState) -> DspState {
    if let Some(&(freq, feed)) = CROSSFEED_PRESETS.get(usize::from(s.crossfeed_preset)) {
        s.crossfeed_freq = freq;
        s.crossfeed_feed = feed;
    }
    s
}

// ── Validation ─────────────────────────────────────────────────────────

fn invalid(msg: String) -> DspiError {
    DspiError::InvalidArgument { msg }
}

fn not_nan(v: f32, what: &str) -> Result<f32> {
    if v.is_nan() {
        Err(invalid(format!("{what} is NaN")))
    } else {
        Ok(v)
    }
}

fn finite(v: f32, what: &str) -> Result<f32> {
    if v.is_finite() {
        Ok(v)
    } else {
        Err(invalid(format!("{what} is not a finite number ({v})")))
    }
}

/// `v` clamped to the registry's range for `path` (the firmware's own clamp
/// for these parameters). NaN is refused.
fn clamped(path: &str, v: f32) -> Result<f32> {
    let v = not_nan(v, path)?;
    Ok(match by_path(path).map(|d| d.kind) {
        Some(Kind::Float { min, max, .. }) => v.clamp(min, max),
        _ => v,
    })
}

/// A delay as the old core shaped it, quantized, below 0 made 0; above the
/// platform's line it is refused (WP CG36: the old core clamped it there
/// without a word).
fn delay_ms(inner: &Inner, path: &str, ms: f32) -> Result<f32> {
    let ms = quantize_delay(not_nan(ms, "delay")?).max(0.0);
    check_delay(&inner.state, path, ms)?;
    Ok(ms)
}

// -- WP CG36 -- -- WP CG37 --
/// `ms` (quantized, [`quantize_delay`]) against the connected platform's
/// line ([`state::max_delay_ms_for`]): an error naming the range, 0 to that
/// limit, outside it. NaN is the caller's to refuse.
pub(crate) fn check_delay(s: &DspState, path: &str, ms: f32) -> Result<()> {
    let max = state::max_delay_ms_for(s);
    if ms < 0.0 || ms > max {
        let platform = dspi_proto::Platform::from_id(s.platform_id).name();
        return Err(invalid(format!(
            "{path}: {ms} ms is out of range; this device ({platform}) holds 0 to {max} ms"
        )));
    }
    Ok(())
}

/// A preamp as the old core shaped it: clamped to the platform's range,
/// then quantized.
fn preamp_db(inner: &Inner, db: f32) -> Result<f32> {
    let max = if inner.state.platform_id == 1 {
        crate::device::PREAMP_MAX_DB_RP2350
    } else {
        crate::device::PREAMP_MAX_DB_RP2040
    };
    Ok(quantize_gain(
        not_nan(db, "preamp")?.clamp(PREAMP_MIN_DB, max),
    ))
}

/// A band's values, checked: finite frequency, Q and gain; a Linkwitz
/// Transform's `qp` clamped to 0.1..20 and refused when NaN.
fn checked_band(mut p: FilterParams) -> Result<FilterParams> {
    finite(p.freq, "frequency")?;
    finite(p.q, "Q")?;
    finite(p.gain, "gain")?;
    if FilterType::from_raw(p.filter_type).is_linkwitz() {
        p.qp = not_nan(p.qp, "Linkwitz target Q")?.clamp(0.1, 20.0);
    }
    Ok(p)
}

fn channel_index(s: &DspState, ch: u8) -> Result<usize> {
    if ch < s.num_channels && usize::from(ch) < MAX_CHANNELS {
        Ok(usize::from(ch))
    } else {
        Err(invalid(format!("channel {ch} (0..{})", s.num_channels)))
    }
}

/// An output's wire channel, as an index into the channel vectors.
fn output_channel_index(s: &DspState, ch: u8) -> Result<usize> {
    let c = channel_index(s, ch)?;
    if ch >= s.num_input_channels {
        Ok(c)
    } else {
        Err(invalid(format!(
            "channel {ch} is an input; crossovers are on outputs"
        )))
    }
}

fn input_index(s: &DspState, input: u8) -> Result<usize> {
    if input < s.num_input_channels && usize::from(input) < MAX_INPUTS {
        Ok(usize::from(input))
    } else {
        Err(invalid(format!(
            "input {input} (0..{})",
            s.num_input_channels
        )))
    }
}

fn output_index(s: &DspState, output: u8) -> Result<usize> {
    if output < s.num_output_channels && usize::from(output) < MAX_OUTPUTS {
        Ok(usize::from(output))
    } else {
        Err(invalid(format!(
            "output {output} (0..{})",
            s.num_output_channels
        )))
    }
}

/// `name` cut to 31 bytes on a character boundary.
fn cut_name(name: &str) -> &str {
    let max = CHANNEL_NAME_LEN.saturating_sub(1);
    let len = (0..=name.len().min(max))
        .rev()
        .find(|&l| name.is_char_boundary(l))
        .unwrap_or(0);
    name.get(..len).unwrap_or_default()
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_c3 {
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    use dspi_transport::{
        DeviceDescriptor, FakeDevice, FakeRemote, NotificationSource, Transport, TransportCaps,
    };

    use super::*;

    fn demo() -> (Arc<DspiCore>, FakeRemote) {
        let dev = FakeDevice::demo();
        let remote = dev.remote();
        let core = DspiCore::new();
        core.attach_probed(Box::new(dev), "DEMO".into(), 0).unwrap();
        (core, remote)
    }

    /// The device's image decoded onto a copy of the mirror (so the fields
    /// the image does not carry agree), `bulk_raw` taken from the mirror.
    fn device_view(core: &DspiCore, remote: &FakeRemote) -> (DspState, DspState) {
        // -- WP P1XF -- the core's record: `get_state` shows a named
        // crossfeed preset's values, not the custom pair the image holds
        let mirror = core.lock().state.clone();
        let mut dev = mirror.clone();
        state::decode_bulk(&remote.image(), &mut dev).unwrap();
        dev.bulk_raw.clone_from(&mirror.bulk_raw);
        (mirror, dev)
    }

    /// The mirror's own image decodes to the mirror.
    fn assert_self_consistent(s: &DspState) {
        let mut t = s.clone();
        state::decode_bulk(&s.bulk_raw, &mut t).unwrap();
        assert_eq!(&t, s, "bulk_raw and the record disagree");
    }

    fn pk(freq: f32, gain: f32, q: f32) -> FilterParams {
        FilterParams {
            filter_type: 1,
            bypass: false,
            freq,
            q,
            gain,
            qp: DEFAULT_Q,
        }
    }

    fn lt(qp: f32) -> FilterParams {
        FilterParams {
            filter_type: FilterType::LinkwitzTransform.to_raw(),
            bypass: false,
            freq: 40.0,
            q: 0.9,
            gain: 25.37,
            qp,
        }
    }

    fn invalid_arg<T: std::fmt::Debug>(r: Result<T>) {
        assert!(
            matches!(r, Err(DspiError::InvalidArgument { .. })),
            "expected InvalidArgument, got {r:?}"
        );
    }

    /// One logged request: OUT or IN, opcode, wValue, data stage.
    type Log = Arc<Mutex<Vec<(bool, u8, u16, Vec<u8>)>>>;

    /// The demo device behind a recorder. `answers` replies to chosen IN
    /// opcodes itself; `swallow` ACKs chosen OUT opcodes without applying
    /// them (what the firmware does with a refused or unknown SET on USB).
    struct Spy {
        inner: FakeDevice,
        answers: Arc<Mutex<HashMap<u8, Vec<u8>>>>,
        swallow: Arc<Mutex<Vec<u8>>>,
        log: Log,
    }

    pub(super) struct SpyHandles {
        answers: Arc<Mutex<HashMap<u8, Vec<u8>>>>,
        swallow: Arc<Mutex<Vec<u8>>>,
        log: Log,
        remote: FakeRemote,
    }

    impl SpyHandles {
        fn answer(&self, opcode: u8, reply: &[u8]) {
            self.answers.lock().unwrap().insert(opcode, reply.to_vec());
        }
        fn swallow(&self, opcode: u8) {
            self.swallow.lock().unwrap().push(opcode);
        }
        pub(super) fn clear(&self) {
            self.log.lock().unwrap().clear();
        }
        pub(super) fn outs(&self, opcode: u8) -> Vec<(u16, Vec<u8>)> {
            self.log
                .lock()
                .unwrap()
                .iter()
                .filter(|e| e.0 && e.1 == opcode)
                .map(|e| (e.2, e.3.clone()))
                .collect()
        }
        fn asked(&self, opcode: u8) -> bool {
            self.log
                .lock()
                .unwrap()
                .iter()
                .any(|e| !e.0 && e.1 == opcode)
        }
    }

    pub(super) fn spy_core() -> (Arc<DspiCore>, SpyHandles) {
        let inner = FakeDevice::demo();
        let h = SpyHandles {
            answers: Arc::default(),
            swallow: Arc::default(),
            log: Arc::default(),
            remote: inner.remote(),
        };
        let spy = Spy {
            inner,
            answers: Arc::clone(&h.answers),
            swallow: Arc::clone(&h.swallow),
            log: Arc::clone(&h.log),
        };
        let core = DspiCore::new();
        core.attach_probed(Box::new(spy), "SPY".into(), 0).unwrap();
        (core, h)
    }

    impl Spy {
        fn answered(&self, opcode: u8, value: u16) -> Option<Vec<u8>> {
            self.log
                .lock()
                .unwrap()
                .push((false, opcode, value, Vec::new()));
            self.answers.lock().unwrap().get(&opcode).cloned()
        }
    }

    impl Transport for Spy {
        fn control_in(
            &mut self,
            opcode: u8,
            value: u16,
            len: u16,
        ) -> dspi_transport::Result<Vec<u8>> {
            match self.answered(opcode, value) {
                Some(r) => Ok(r),
                None => self.inner.control_in(opcode, value, len),
            }
        }
        fn control_in_upto(
            &mut self,
            opcode: u8,
            value: u16,
            len: u16,
        ) -> dspi_transport::Result<Vec<u8>> {
            match self.answered(opcode, value) {
                Some(r) => Ok(r),
                None => self.inner.control_in_upto(opcode, value, len),
            }
        }
        fn control_out(
            &mut self,
            opcode: u8,
            value: u16,
            data: &[u8],
        ) -> dspi_transport::Result<()> {
            self.log
                .lock()
                .unwrap()
                .push((true, opcode, value, data.to_vec()));
            if self.swallow.lock().unwrap().contains(&opcode) {
                return Ok(());
            }
            self.inner.control_out(opcode, value, data)
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
    }

    #[test]
    fn wp_c3_setters_reach_the_device_and_the_mirror() {
        let (core, remote) = demo();
        let s0 = core.get_state();
        assert_eq!((s0.num_input_channels, s0.num_output_channels), (8, 9));

        core.set_filter(0, 0, pk(321.0, -4.5, 2.0)).unwrap();
        core.set_filter(3, 9, pk(8000.0, 3.0, 0.7)).unwrap();
        let lr = FilterParams {
            filter_type: 32,
            ..pk(2000.0, 0.0, 0.707)
        };
        core.set_crossover(9, 1, lr).unwrap();
        core.set_preamp(-2.0).unwrap();
        core.set_input_preamp(1, -3.5).unwrap();
        core.set_bypass(true).unwrap();
        core.set_master_volume(-20.0).unwrap();
        core.set_user_volume(-6.0).unwrap();
        core.set_user_mute(true).unwrap();
        core.set_delay(0, 2.0).unwrap();
        core.set_loudness(true).unwrap();
        core.set_loudness_ref(83.0).unwrap();
        core.set_loudness_intensity(50.0).unwrap();
        core.set_loudness_mask(0b11).unwrap();
        core.set_crossfeed(true).unwrap();
        core.set_crossfeed_preset(3).unwrap();
        core.set_crossfeed_freq(800.0).unwrap();
        core.set_crossfeed_feed(5.0).unwrap();
        core.set_crossfeed_itd(true).unwrap();
        core.set_crossfeed_outputs(0b1).unwrap();
        core.set_leveller_enabled(true).unwrap();
        core.set_leveller_amount(40.0).unwrap();
        core.set_leveller_speed(2).unwrap();
        core.set_leveller_max_gain(12.0).unwrap();
        core.set_leveller_lookahead(true).unwrap();
        core.set_leveller_gate(-60.0).unwrap();
        core.set_leveller_masks(0x03, 0x0C).unwrap();
        core.set_matrix_route(1, 2, true, -3.04, true).unwrap();
        core.set_output_gain(3, -4.0).unwrap();
        core.set_output_mute(8, true).unwrap();
        core.set_output_delay(2, 1.5).unwrap();
        core.set_channel_name(2, "Tweeter L".into()).unwrap();

        let (mirror, dev) = device_view(&core, &remote);
        assert_eq!(dev, mirror, "the device and the mirror agree");
        assert_self_consistent(&mirror);

        // Spot checks on the device's own image.
        assert_eq!(dev.filters[0][0], pk(321.0, -4.5, 2.0));
        assert_eq!(dev.xover[9][1], lr);
        assert_eq!(dev.input_preamp_db[0], -2.0);
        assert_eq!(dev.input_preamp_db[1], -3.5);
        assert_eq!(dev.master_volume_db, -20.0);
        assert_eq!(
            (dev.leveller_detector_mask, dev.leveller_apply_mask),
            (0x03, 0x0C)
        );
        assert_eq!(dev.loudness_output_mask, 0b11);
        assert_eq!(dev.crossfeed_output_pair_mask, 0b1);
        assert!(dev.matrix_routing[1][2] && dev.matrix_invert[1][2]);
        assert_eq!(dev.matrix_gain[1][2], -3.0);
        assert_eq!(dev.output_delay_ms[2], 1.5);
        assert_eq!(dev.channel_delays[8 + 2], 1.5, "the output's channel too");
        assert_eq!(dev.channel_names[2], "Tweeter L");
    }

    #[test]
    fn wp_c3_master_volume_mode_reaches_the_device() {
        // The mode is not in the image; the device answers it on its own.
        let (core, _) = demo();
        core.set_master_volume_mode(1).unwrap();
        assert_eq!(core.get_state().master_volume_mode, 1);
        let got = core
            .lock()
            .conn()
            .unwrap()
            .get(op::REQ_GET_MASTER_VOLUME_MODE, 0, 1)
            .unwrap();
        assert_eq!(got, [1]);
    }

    #[test]
    fn wp_c3_gains_are_clamped_then_quantized_delays_quantized_then_clamped() {
        let (core, remote) = demo();
        assert_eq!(core.get_state().platform_id, 1, "an RP2350");

        core.set_input_preamp(0, 30.0).unwrap();
        assert_eq!(core.get_state().input_preamp_db[0], 24.0);
        core.set_input_preamp(0, 23.96).unwrap();
        assert_eq!(core.get_state().input_preamp_db[0], 24.0);
        core.set_input_preamp(0, -3.04).unwrap();
        assert_eq!(core.get_state().input_preamp_db[0], -3.0);
        assert_eq!(
            core.get_state().preamp_db,
            -3.0,
            "input 0 is the legacy preamp"
        );
        core.set_preamp(-100.0).unwrap();
        assert!(
            core.get_state().input_preamp_db[..8]
                .iter()
                .all(|v| *v == -60.0)
        );

        core.set_output_gain(0, 25.0).unwrap();
        assert_eq!(core.get_state().output_gain_db[0], 20.0);
        core.set_output_gain(0, 1.26).unwrap();
        assert_eq!(core.get_state().output_gain_db[0], 1.3);

        // -- WP CG36 -- above the line: refused, no longer clamped to 42.
        invalid_arg(core.set_delay(0, 50.0));
        core.set_delay(0, 42.0).unwrap();
        assert_eq!(core.get_state().channel_delays[0], 42.0);
        core.set_delay(0, 1.23456).unwrap();
        assert_eq!(core.get_state().channel_delays[0], quantize_delay(1.23456));
        core.set_delay(0, -1.0).unwrap();
        assert_eq!(core.get_state().channel_delays[0], 0.0);

        for (asked, got) in [
            (-200.0, -128.0),
            (-128.0, -128.0),
            (-127.9, -127.0),
            (-126.7, -126.5),
            (-10.3, -10.5),
            (-10.2, -10.0),
            (3.0, 0.0),
            (f32::NEG_INFINITY, -128.0),
        ] {
            core.set_master_volume(asked).unwrap();
            assert_eq!(core.get_state().master_volume_db, got, "{asked}");
        }

        core.set_user_volume(-80.0).unwrap();
        assert_eq!(core.get_state().user_volume_db, -60.0);
        core.set_loudness_ref(10.0).unwrap();
        assert_eq!(core.get_state().loudness_ref_spl, 40.0);
        core.set_loudness_intensity(500.0).unwrap();
        assert_eq!(core.get_state().loudness_intensity, 200.0);
        // -- WP P1XF -- the custom pair (the demo runs a named preset)
        core.set_crossfeed_freq(100.0).unwrap();
        assert_eq!(core.lock().state.crossfeed_freq, 500.0);
        core.set_crossfeed_feed(20.0).unwrap();
        assert_eq!(core.lock().state.crossfeed_feed, 15.0);
        core.set_leveller_amount(101.0).unwrap();
        assert_eq!(core.get_state().leveller_amount, 100.0);
        core.set_leveller_max_gain(-1.0).unwrap();
        assert_eq!(core.get_state().leveller_max_gain_db, 0.0);
        core.set_leveller_gate(-200.0).unwrap();
        assert_eq!(core.get_state().leveller_gate_db, -96.0);

        let (mirror, dev) = device_view(&core, &remote);
        assert_eq!(dev, mirror);
    }

    #[test]
    fn wp_c3_filter_gains_are_quantized_except_the_linkwitz_fp() {
        let (core, remote) = demo();
        core.set_filter(1, 0, pk(31.5, -4.24, 3.5)).unwrap();
        assert_eq!(core.get_state().filters[1][0].gain, -4.2);
        core.set_filter(1, 1, lt(0.5)).unwrap();
        assert_eq!(core.get_state().filters[1][1].gain, 25.37, "fp in Hz");
        let (mirror, dev) = device_view(&core, &remote);
        assert_eq!(dev.filters[1], mirror.filters[1]);
    }

    #[test]
    fn wp_c3_linkwitz_qp_is_clamped_and_nan_refused() {
        let (core, remote) = demo();
        core.set_filter(1, 0, lt(50.0)).unwrap();
        let (mirror, dev) = device_view(&core, &remote);
        assert_eq!(dev.filters[1][0].qp, 20.0);
        assert_eq!(mirror.filters[1][0], dev.filters[1][0]);

        core.set_filter(1, 0, lt(0.01)).unwrap();
        let (mirror, dev) = device_view(&core, &remote);
        assert!((dev.filters[1][0].qp - 0.1).abs() <= 1.0 / 512.0);
        assert_eq!(mirror.filters[1][0], dev.filters[1][0]);

        let before = remote.image();
        invalid_arg(core.set_filter(1, 0, lt(f32::NAN)));
        assert_eq!(remote.image(), before, "nothing was sent");
        // A NaN qp on another type is ignored, as the type ignores qp.
        core.set_filter(1, 2, pk(100.0, 1.0, 1.0).with_qp(f32::NAN))
            .unwrap();
        assert_eq!(core.get_state().filters[1][2].qp, DEFAULT_Q);
    }

    trait WithQp {
        fn with_qp(self, qp: f32) -> Self;
    }
    impl WithQp for FilterParams {
        fn with_qp(mut self, qp: f32) -> Self {
            self.qp = qp;
            self
        }
    }

    #[test]
    fn wp_c3_bad_values_and_indices_are_refused_before_the_wire() {
        let (core, remote) = demo();
        let before_img = remote.image();
        let before = core.get_state();

        invalid_arg(core.set_master_volume(f32::NAN));
        invalid_arg(core.set_user_volume(f32::NAN));
        invalid_arg(core.set_preamp(f32::NAN));
        invalid_arg(core.set_delay(0, f32::NAN));
        invalid_arg(core.set_output_gain(0, f32::NAN));
        invalid_arg(core.set_loudness_ref(f32::NAN));
        invalid_arg(core.set_matrix_route(0, 0, true, f32::INFINITY, false));
        invalid_arg(core.set_filter(0, 0, pk(f32::NAN, 0.0, 1.0)));
        invalid_arg(core.set_filter(0, 0, pk(100.0, f32::INFINITY, 1.0)));

        invalid_arg(core.set_filter(17, 0, pk(100.0, 0.0, 1.0)));
        invalid_arg(core.set_filter(0, 10, pk(100.0, 0.0, 1.0)));
        let lr = FilterParams {
            filter_type: 32,
            ..pk(2000.0, 0.0, 0.707)
        };
        invalid_arg(core.set_crossover(0, 0, lr));
        invalid_arg(core.set_crossover(9, 4, lr));
        invalid_arg(core.set_crossover(9, 0, pk(100.0, 0.0, 1.0)));
        invalid_arg(core.set_band_bypass(0, 20, true));
        invalid_arg(core.set_band_bypass(9, 15, true));
        invalid_arg(core.set_band_bypass(9, 24, true));
        invalid_arg(core.set_band_bypass(17, 0, true));
        invalid_arg(core.set_input_preamp(8, 0.0));
        invalid_arg(core.set_delay(17, 1.0));
        invalid_arg(core.set_matrix_route(8, 0, true, 0.0, false));
        invalid_arg(core.set_matrix_route(0, 9, true, 0.0, false));
        invalid_arg(core.set_output_enable(9, true));
        invalid_arg(core.set_output_gain(9, 0.0));
        invalid_arg(core.set_output_mute(9, true));
        invalid_arg(core.set_output_delay(9, 1.0));
        invalid_arg(core.set_channel_name(17, "x".into()));
        invalid_arg(core.get_channel_name(17));
        invalid_arg(core.set_crossfeed_preset(4));
        invalid_arg(core.set_leveller_speed(3));
        invalid_arg(core.set_master_volume_mode(2));
        // -- WP SW1 -- 7 is the audio link: unsupported on a unit without it
        invalid_arg(core.set_input_source(8));
        assert!(matches!(
            core.set_input_source(7),
            Err(DspiError::Unsupported)
        ));

        assert_eq!(remote.image(), before_img, "nothing reached the device");
        assert_eq!(core.get_state(), before, "nor the mirror");
    }

    #[test]
    fn wp_c3_setters_need_a_writable_connection() {
        let core = DspiCore::new();
        assert_eq!(core.set_bypass(true), Err(DspiError::Disconnected));
        assert_eq!(
            core.set_output_enable(0, true),
            Err(DspiError::Disconnected)
        );
        assert_eq!(core.get_channel_name(0), Err(DspiError::Disconnected));

        // A connection whose image was never read in this build's format.
        core.attach(Box::new(FakeDevice::demo()), "RO".into(), 0);
        assert_eq!(core.set_bypass(true), Err(DspiError::ReadOnly));
        assert_eq!(
            core.set_filter(0, 0, pk(100.0, 0.0, 1.0)),
            Err(DspiError::ReadOnly)
        );
        assert_eq!(core.set_output_enable(0, true), Err(DspiError::ReadOnly));
        assert_eq!(core.save_master_volume(), Err(DspiError::ReadOnly));
        assert_eq!(
            core.set_channel_name(0, "x".into()),
            Err(DspiError::ReadOnly)
        );
    }

    #[test]
    fn wp_c3_a_failed_transfer_leaves_the_mirror_alone() {
        // -- WP UPW2 -- The demo models subharm solo now: make it stall.
        let (core, remote) = demo();
        remote.stall(op::REQ_SET_SUBHARM_SOLO);
        let before = core.get_state();
        assert!(matches!(
            core.set_subharm_solo(true),
            Err(DspiError::Transfer { .. })
        ));
        assert_eq!(core.get_state(), before);
        let (mirror, dev) = device_view(&core, &remote);
        assert_eq!(dev, mirror);
    }

    #[test]
    fn wp_c3_band_bypass_addresses_peq_and_crossover_bands() {
        let (core, h) = spy_core();
        core.set_band_bypass(0, 3, true).unwrap();
        core.set_band_bypass(9, 21, true).unwrap();
        assert_eq!(
            h.outs(op::REQ_SET_BAND_BYPASS),
            [(3, vec![1]), ((9 << 8) | 21, vec![1])]
        );
        let s = core.get_state();
        assert!(s.filters[0][3].bypass);
        assert!(s.xover[9][1].bypass);
        assert_self_consistent(&s);
        core.set_band_bypass(9, 21, false).unwrap();
        assert!(!core.get_state().xover[9][1].bypass);
    }

    #[test]
    fn wp_c3_an_enable_asks_core1_first_and_is_not_sent_on_a_conflict() {
        let (core, h) = spy_core();
        let pdm = 8;
        assert!(!core.get_state().output_enabled[pdm as usize]);
        h.answer(op::REQ_GET_CORE1_CONFLICT, &[1]);
        h.answer(op::REQ_GET_CORE1_MODE, &[2]);

        assert_eq!(core.set_output_enable(pdm, true), Ok(false));
        assert!(h.asked(op::REQ_GET_CORE1_CONFLICT));
        assert!(h.outs(op::REQ_SET_OUTPUT_ENABLE).is_empty(), "not sent");
        let s = core.get_state();
        assert!(!s.output_enabled[pdm as usize]);
        assert_eq!(s.core1_mode, 2, "the Core 1 mode is re-read");

        // A disable never asks.
        h.clear();
        assert_eq!(core.set_output_enable(0, false), Ok(false));
        assert!(!h.asked(op::REQ_GET_CORE1_CONFLICT));
        assert_eq!(h.outs(op::REQ_SET_OUTPUT_ENABLE), [(0, vec![0])]);

        // No conflict: sent, and the device has it.
        h.answer(op::REQ_GET_CORE1_CONFLICT, &[0]);
        assert_eq!(core.set_output_enable(pdm, true), Ok(true));
        assert!(core.get_state().output_enabled[pdm as usize]);
        let mut dev = core.get_state();
        state::decode_bulk(&h.remote.image(), &mut dev).unwrap();
        assert!(dev.output_enabled[pdm as usize]);
        assert!(!dev.output_enabled[0]);
    }

    #[test]
    fn wp_c3_a_silently_refused_enable_reads_back_as_off() {
        // The firmware ACKs an enable it skips; only the read-back tells.
        let (core, h) = spy_core();
        h.answer(op::REQ_GET_CORE1_CONFLICT, &[0]);
        h.swallow(op::REQ_SET_OUTPUT_ENABLE);
        assert_eq!(core.set_output_enable(5, true), Ok(false));
        assert_eq!(h.outs(op::REQ_SET_OUTPUT_ENABLE), [(5, vec![1])], "sent");
        assert!(!core.get_state().output_enabled[5], "the mirror says off");
    }

    #[test]
    fn wp_c3_save_master_volume_maps_the_status_byte() {
        let (core, h) = spy_core();
        h.answer(op::REQ_SAVE_MASTER_VOLUME, &[0]);
        assert_eq!(core.save_master_volume(), Ok(()));
        h.answer(op::REQ_SAVE_MASTER_VOLUME, &[3]);
        assert_eq!(
            core.save_master_volume(),
            Err(DspiError::Refused { status: 3 })
        );
        // -- WP UPW2 -- The demo answers PRESET_OK, as the firmware does; a
        // stall is a transfer error.
        let (core, remote) = demo();
        assert_eq!(core.save_master_volume(), Ok(()));
        remote.stall(op::REQ_SAVE_MASTER_VOLUME);
        assert!(matches!(
            core.save_master_volume(),
            Err(DspiError::Transfer { .. })
        ));
    }

    #[test]
    fn wp_c3_channel_names_are_cut_on_a_character_boundary() {
        let (core, remote) = demo();
        core.set_channel_name(2, "é".repeat(16)).unwrap();
        assert_eq!(core.get_state().channel_names[2], "é".repeat(15));
        assert_eq!(core.get_channel_name(2).unwrap(), "é".repeat(15));
        let long = "This is a very long channel name that exceeds 31 bytes";
        core.set_channel_name(3, long.into()).unwrap();
        assert_eq!(core.get_channel_name(3).unwrap(), long[..31]);
        let (mirror, dev) = device_view(&core, &remote);
        assert_eq!(dev, mirror);
    }

    #[test]
    fn wp_c3_get_channel_name_reads_the_device() {
        let (core, h) = spy_core();
        let mut reply = b"From device".to_vec();
        reply.resize(32, 0);
        h.answer(op::REQ_GET_CHANNEL_NAME, &reply);
        assert_eq!(core.get_channel_name(4).unwrap(), "From device");
        let s = core.get_state();
        assert_eq!(s.channel_names[4], "From device");
        assert_self_consistent(&s);
    }

    #[test]
    fn wp_c3_a_crossfeed_preset_keeps_the_custom_values() {
        let (core, remote) = demo();
        core.set_crossfeed_freq(800.0).unwrap();
        core.set_crossfeed_feed(5.0).unwrap();
        core.set_crossfeed_preset(2).unwrap();
        // -- WP P1XF -- the core's record keeps the custom pair, as the
        // device does; `get_state` gives Meier's values
        let s = core.get_state();
        assert_eq!(
            (s.crossfeed_preset, s.crossfeed_freq, s.crossfeed_feed),
            (2, 650.0, 9.5)
        );
        let (mirror, dev) = device_view(&core, &remote);
        assert_eq!((mirror.crossfeed_freq, mirror.crossfeed_feed), (800.0, 5.0));
        assert_eq!(dev, mirror, "the mirror reads as the device's image");
    }

    #[test]
    fn wp_c3_an_output_channel_delay_is_the_output_delay() {
        let (core, h) = spy_core();
        core.set_delay(9, 3.0).unwrap();
        assert_eq!(h.outs(op::REQ_SET_OUTPUT_DELAY).len(), 1);
        assert!(h.outs(op::REQ_SET_DELAY).is_empty());
        let s = core.get_state();
        assert_eq!((s.output_delay_ms[1], s.channel_delays[9]), (3.0, 3.0));
        let mut dev = s.clone();
        state::decode_bulk(&h.remote.image(), &mut dev).unwrap();
        assert_eq!(dev.output_delay_ms[1], 3.0);
    }

    #[test]
    fn wp_c3_input_sources_beyond_adat_go_out() {
        // S/PDIF inputs 2..4 are sources 4..6, past the registry's list.
        let (core, h) = spy_core();
        h.swallow(op::REQ_SET_INPUT_SOURCE);
        core.set_input_source(5).unwrap();
        assert_eq!(h.outs(op::REQ_SET_INPUT_SOURCE), [(0, vec![5])]);
        assert_eq!(core.get_state().input_source, 5);
    }

    #[test]
    fn wp_c3_preamps_above_the_registry_range_go_out() {
        let (core, h) = spy_core();
        core.set_preamp(22.0).unwrap();
        core.set_input_preamp(3, 23.0).unwrap();
        assert_eq!(
            h.outs(op::REQ_SET_PREAMP),
            [(0, 22.0f32.to_le_bytes().to_vec())]
        );
        assert_eq!(
            h.outs(op::REQ_SET_PREAMP_CH),
            [(3, 23.0f32.to_le_bytes().to_vec())]
        );
        let mut dev = core.get_state();
        state::decode_bulk(&h.remote.image(), &mut dev).unwrap();
        assert_eq!(dev.input_preamp_db[..4], [22.0, 22.0, 22.0, 23.0]);
    }

    #[test]
    fn wp_c3_a_write_marks_the_connect_image_stale() {
        let (core, _) = demo();
        assert!(core.lock().conn().unwrap().fresh_image);
        core.set_bypass(true).unwrap();
        assert!(!core.lock().conn().unwrap().fresh_image);
    }

    // -- WP CG36 --
    use crate::ParamValue;

    pub(super) fn refusal(r: Result<impl std::fmt::Debug>) -> String {
        match r {
            Err(DspiError::InvalidArgument { msg }) => msg,
            other => panic!("expected InvalidArgument, got {other:?}"),
        }
    }

    #[test]
    fn wp_cg36_eq_type_on_a_crossover_band_takes_crossover_types_only() {
        let (core, h) = spy_core();
        let put = |ch: u8, b: u8, t: u8| {
            core.write_param("eq.type".into(), vec![ch, b], ParamValue::Choice(t))
        };
        // Every crossover code on band 20..23 of an output channel, read back.
        for (x, b) in (20..=23u8).enumerate() {
            for t in FilterType::XOVER_FIRST..=FilterType::XOVER_LAST {
                assert_eq!(put(9, b, t), Ok(ParamValue::Choice(t)), "band {b} type {t}");
                assert_eq!(core.get_state().xover[9][x].filter_type, t);
            }
        }
        // The device has it too, through set_crossover's packet.
        let mut dev = core.get_state();
        state::decode_bulk(&h.remote.image(), &mut dev).unwrap();
        assert_eq!(dev.xover[9][3].filter_type, FilterType::XOVER_LAST);
        // A family change keeps the band's frequency (no Linkwitz retype).
        let f = core.get_state().xover[9][0].freq;
        assert_eq!(put(9, 20, 40), Ok(ParamValue::Choice(40)));
        assert_eq!(core.get_state().xover[9][0].freq, f);
        // Off is a crossover band's too, as set_crossover takes it.
        assert_eq!(put(9, 20, 0), Ok(ParamValue::Choice(0)));

        // A PEQ type on a crossover band and a crossover type on a PEQ band
        // are refused before the wire, in words.
        h.clear();
        let before = core.get_state();
        let e = refusal(put(9, 21, 5));
        assert!(e.contains("crossover band") && e.contains("32..63"), "{e}");
        let e = refusal(put(9, 3, 40));
        assert!(e.contains("PEQ band") && e.contains("20..23"), "{e}");
        let e = refusal(put(0, 3, 40));
        assert!(e.contains("PEQ band"), "{e}");
        refusal(put(9, 20, 14));
        refusal(put(9, 20, 64));
        assert!(h.outs(op::REQ_SET_EQ_PARAM).is_empty(), "nothing sent");
        assert_eq!(core.get_state(), before);
        // PEQ bands keep 0..13.
        assert_eq!(put(0, 3, 13), Ok(ParamValue::Choice(13)));
        // An input channel has no crossover band: refused as before.
        refusal(put(0, 20, 40));
    }

    #[test]
    fn wp_cg36_a_delay_past_the_platforms_line_is_refused_naming_it() {
        let (core, h) = spy_core();
        let put = |path: &str, i: u8, ms: f32| {
            core.write_param(path.into(), vec![i], ParamValue::Float(ms))
        };
        // RP2350: 42 ms.
        assert_eq!(put("out.delay", 1, 42.0), Ok(ParamValue::Float(42.0)));
        h.clear();
        let before = core.get_state();
        let e = refusal(put("out.delay", 1, 42.5));
        assert!(
            e.contains("out.delay") && e.contains("42 ms") && e.contains("RP2350"),
            "{e}"
        );
        let e = refusal(put("ch.delay", 0, 60.0));
        assert!(e.contains("ch.delay") && e.contains("42 ms"), "{e}");
        // The typed setters refuse the same (they clamped silently before).
        let e = refusal(core.set_output_delay(1, 50.0));
        assert!(e.contains("0 to 42 ms"), "{e}");
        refusal(core.set_delay(0, 50.0));
        // An output's wire channel goes through the output's setter.
        refusal(core.set_delay(9, 50.0));
        assert!(h.outs(op::REQ_SET_OUTPUT_DELAY).is_empty(), "nothing sent");
        assert!(h.outs(op::REQ_SET_DELAY).is_empty(), "nothing sent");
        assert_eq!(core.get_state(), before);

        // RP2040: 21 ms, below the registry's 85 and the session's 42.
        core.lock().state.platform_id = 0;
        let e = refusal(put("out.delay", 1, 30.0));
        assert!(e.contains("0 to 21 ms") && e.contains("RP2040"), "{e}");
        assert!(h.outs(op::REQ_SET_OUTPUT_DELAY).is_empty(), "nothing sent");
        assert_eq!(put("out.delay", 1, 21.0), Ok(ParamValue::Float(21.0)));
        // Below 0 is still 0 for a typed setter.
        core.set_output_delay(1, -0.5).unwrap();
        assert_eq!(core.get_state().output_delay_ms[1], 0.0);
    }

    #[test]
    fn wp_cg36_a_core1_conflict_is_refused_in_words_by_write_param() {
        let (core, h) = spy_core();
        let outputs = core.get_state().num_output_channels;
        let pdm = outputs - 1;
        let enable = |o: u8| core.write_param("out.enable".into(), vec![o], ParamValue::Bool(true));
        h.answer(op::REQ_GET_CORE1_CONFLICT, &[1]);
        let e = refusal(enable(pdm));
        assert_eq!(
            e,
            format!(
                "Output {outputs} is the PDM output, which shares Core 1 with outputs 3-{pdm}, \
                 and one of them is enabled: disable outputs 3-{pdm} first"
            )
        );
        assert!(h.outs(op::REQ_SET_OUTPUT_ENABLE).is_empty(), "not sent");
        assert!(!core.get_state().output_enabled[usize::from(pdm)]);
        let e = refusal(enable(2));
        assert_eq!(
            e,
            format!(
                "Output 3 shares Core 1 with the PDM output (output {outputs}), which is \
                 enabled: disable the PDM output first"
            )
        );
        // The same words to ask before, and the typed setter's answer unchanged.
        assert_eq!(core.core1_conflict_reason(2), Ok(Some(e)));
        assert_eq!(core.set_output_enable(pdm, true), Ok(false));
        // A disable never conflicts.
        assert_eq!(
            core.write_param("out.enable".into(), vec![0], ParamValue::Bool(false)),
            Ok(ParamValue::Bool(false))
        );

        h.answer(op::REQ_GET_CORE1_CONFLICT, &[0]);
        assert_eq!(core.core1_conflict_reason(pdm), Ok(None));
        assert_eq!(enable(pdm), Ok(ParamValue::Bool(true)));
        assert!(core.get_state().output_enabled[usize::from(pdm)]);
        assert!(matches!(
            core.core1_conflict_reason(outputs),
            Err(DspiError::InvalidArgument { .. })
        ));
    }
}

// -- WP CG37 --
#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_cg37 {
    use super::tests_wp_c3::{refusal, spy_core};
    use crate::{ParamValue, state};
    use dspi_proto::generated::opcodes as op;

    /// The core's line is the session's: one source for both.
    #[test]
    fn wp_cg37_the_cores_line_is_the_sessions() {
        let (core, _) = spy_core();
        for id in [0u8, 1, 7] {
            core.lock().state.platform_id = id;
            let p = dspi_proto::Platform::from_id(id);
            assert_eq!(core.max_delay_ms(), p.max_delay_ms(), "{id}");
            assert_eq!(
                state::max_delay_ms_for(&core.get_state()),
                core.max_delay_ms()
            );
        }
    }

    /// write_param checks the quantized value, as the typed setters do: a
    /// value that quantizes onto the line is taken both ways, one that
    /// quantizes past it is refused both ways.
    #[test]
    fn wp_cg37_write_param_and_the_setters_check_the_same_quantized_value() {
        let (core, h) = spy_core();
        for (id, max) in [(1u8, 42.0f32), (0, 21.0)] {
            // A write may read the image again, which brings back the
            // demo's platform: set the one under test before each call.
            let at = || core.lock().state.platform_id = id;
            let put = |path: &str, i: u8, ms: f32| {
                at();
                core.write_param(path.into(), vec![i], ParamValue::Float(ms))
            };
            let on = max + 0.00004;
            assert_eq!(put("out.delay", 1, on), Ok(ParamValue::Float(max)));
            assert_eq!(core.get_state().output_delay_ms[1], max);
            at();
            core.set_output_delay(2, on).unwrap();
            assert_eq!(core.get_state().output_delay_ms[2], max);
            assert_eq!(put("ch.delay", 0, on), Ok(ParamValue::Float(max)));
            at();
            core.set_delay(1, on).unwrap();
            assert_eq!(core.get_state().channel_delays[1], max);

            at();
            h.clear();
            let before = core.get_state();
            let past = max + 0.00006;
            let w = refusal(put("out.delay", 1, past));
            let t = refusal(core.set_output_delay(1, past));
            assert_eq!(w, t, "the same words both ways");
            assert!(w.contains(&format!("0 to {max} ms")), "{w}");
            refusal(put("ch.delay", 0, past));
            refusal(core.set_delay(0, past));
            assert!(h.outs(op::REQ_SET_OUTPUT_DELAY).is_empty(), "nothing sent");
            assert!(h.outs(op::REQ_SET_DELAY).is_empty(), "nothing sent");
            assert_eq!(core.get_state(), before);
        }
    }

    /// Below 0, write_param names the line's range (not the registry's
    /// 85 ms); a typed setter still makes it 0, and a value that quantizes
    /// to 0 is 0 both ways.
    #[test]
    fn wp_cg37_a_negative_delay_names_the_lines_range() {
        let (core, h) = spy_core();
        for (id, platform, max) in [(1u8, "RP2350", 42), (0, "RP2040", 21)] {
            core.lock().state.platform_id = id;
            h.clear();
            for path in ["out.delay", "ch.delay"] {
                let e = refusal(core.write_param(path.into(), vec![1], ParamValue::Float(-1.0)));
                assert_eq!(
                    e,
                    format!(
                        "{path}: -1 ms is out of range; this device ({platform}) holds 0 to {max} ms"
                    )
                );
                let e = refusal(core.write_param(path.into(), vec![1], ParamValue::Int(-3)));
                assert!(e.contains(&format!("0 to {max} ms")), "{e}");
            }
            assert!(h.outs(op::REQ_SET_OUTPUT_DELAY).is_empty(), "nothing sent");
            assert!(h.outs(op::REQ_SET_DELAY).is_empty(), "nothing sent");
            assert_eq!(
                core.write_param("out.delay".into(), vec![1], ParamValue::Float(-0.00002)),
                Ok(ParamValue::Float(0.0))
            );
            core.set_output_delay(1, -1.0).unwrap();
            assert_eq!(core.get_state().output_delay_ms[1], 0.0);
        }
    }
}

// -- WP P1XF --
#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
#[path = "params/tests_wp_p1xf.rs"]
mod tests_wp_p1xf;

// -- WP P1PR --
/// The undo step's label of [`DspiCore::set_input_pair_preamp`].
pub(crate) const PAIR_PREAMP_LABEL: &str = "Input trim, linked pair";

// -- WP PAIRBAND --
/// The undo step's label of [`DspiCore::set_input_pair_filter`] and
/// [`DspiCore::set_input_pair_band_bypass`].
pub(crate) const PAIR_BAND_LABEL: &str = "EQ band, linked pair";

/// A linked pair's band write, checked before anything is sent: a writable
/// device, `input` and its partner are inputs, `band` a PEQ band. The
/// partner.
fn pair_band_check(core: &DspiCore, input: u8, band: u8) -> Result<u8> {
    let partner = input ^ 1;
    let inner = core.writing()?;
    input_index(&inner.state, input)?;
    input_index(&inner.state, partner)?;
    if band >= PEQ_BANDS {
        return Err(invalid(format!("PEQ band {band} (0..{PEQ_BANDS})")));
    }
    Ok(partner)
}

#[cfg(test)]
#[allow(
    clippy::arithmetic_side_effects,
    clippy::unwrap_used,
    clippy::indexing_slicing
)]
#[path = "params/tests_wp_pairband.rs"]
mod tests_wp_pairband;
