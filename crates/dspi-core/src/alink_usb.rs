//! WP AUSB1: the alink's USB-direct units (the fork's
//! `extension_channel_spec.md` 2.9, `dspi_proto::ext::alink_usb`): a unit on
//! its USB input locks its clock to the host's Start of Frame and, in phase
//! mode, plays every USB sample a fixed D samples after it arrived, so every
//! unit on one host plays sample-aligned with no audio link cable.
//!
//! The calls read and write `EXT_ALINK_USB` (mode, flags, D), read
//! `EXT_ALINK_USB_STATUS` (state, host rate, VCXO, phase error, the stream's
//! start, gaps, missed and late samples) and `EXT_ALINK_USB_DIAG` (why a
//! timeline ended), read `EXT_FRAME_COUNTER` with its time base, and queue
//! `EXT_APPLY_AT_FRAME` on a chosen time base (the alink's frame, the USB
//! sample index, or a bus frame).
//!
//! Gated on the connect probe (`ExtState`): a unit whose `EXT_LIST` does not
//! name `EXT_ALINK_USB` and `EXT_ALINK_USB_STATUS` (stock firmware, the fork
//! before WP USB1) answers [`DspiError::Unsupported`] with no traffic. The
//! frame counter and the apply also serve the alink's units. Event 0xE3 goes
//! into the event log from a device that announces it (`events.rs`).
//!
//! History (P0W): a mode or D change re-anchors (a short silence) and is
//! recorded with an audible hazard, so undo steps over it; a DIAG clear and an
//! apply are actions.

use dspi_session::ext::alink;
use dspi_session::ext::alink_usb::{self as usb};
use dspi_session::ext::{self as sx, ApplyOp, ApplyRequest, TimeBase};

use dspi_proto::value::Value;
use dspi_transport::Transport;

use crate::ext::{read_error, write_error};
use crate::journal;
use crate::{Connection, DspiCore, DspiError, Inner, Result};

/// `EXT_ALINK_USB`'s mode 0: stock (asynchronous feedback, the crystal).
pub const USB_MODE_OFF: u8 = usb::MODE_OFF;
/// Mode 1: the clock locked to the host's SOF rate.
pub const USB_MODE_RATE_LOCK: u8 = usb::MODE_RATE_LOCK;
/// Mode 2: rate lock and phase lock (D samples from arrival to output).
pub const USB_MODE_PHASE_LOCK: u8 = usb::MODE_PHASE_LOCK;
/// The firmware's default D, samples (4.0 ms at 48 kHz).
pub const USB_DEFAULT_D: u16 = usb::DEFAULT_D;
/// [`DspiCore::apply_at_time_base`]'s time base 0: the alink frame F.
pub const TIME_BASE_ALINK: u8 = 0;
/// Time base 1: the USB sample index S.
pub const TIME_BASE_USB_SAMPLE: u8 = 1;
/// Time base 2: a bus frame (bits 0-10) and the sample in it (bits 16-23).
pub const TIME_BASE_BUS_FRAME: u8 = 2;

/// What `EXT_ALINK_USB` reports.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AlinkUsbConfig {
    /// 0 off, 1 rate lock, 2 phase lock.
    pub mode: u8,
    /// `off`, `rate_lock`, `phase_lock` or `unknown`.
    pub mode_name: String,
    /// A phase error beyond 2 samples re-anchors (else it is pulled in while
    /// playing).
    pub auto_reanchor: bool,
    /// Samples from a USB sample's arrival to its output.
    pub d: u16,
    /// Mode 2 with D below the unit's minimum now: it runs as mode 1.
    pub d_below_min: bool,
    /// Mode 2 with D above its maximum: it runs as mode 1.
    pub d_above_max: bool,
    /// Mode 2 at 44.1 kHz: it runs as mode 1.
    pub rate_unsupported: bool,
    // -- WP AUSB2 --
    /// What mode 2 does in lock error (the fork's WP USB3b): `mute` (the
    /// default, and what firmware before WP USB3b reads as) or
    /// `play_unaligned`; `unknown` for another code.
    pub on_lock_error: String,
    /// The state, as [`AlinkUsbStatus::state`].
    pub state: String,
    /// Its code.
    pub state_code: u8,
}

/// What `EXT_ALINK_USB_STATUS` reports.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AlinkUsbStatus {
    /// `off` (mode 0, or the source is not USB), `no_sof`, `pull_in`,
    /// `rate_locked`, `anchoring`, `phase_locked`, `rejected` (the host is
    /// beyond 295 ppm) or `unknown`.
    pub state: String,
    /// Its code.
    pub state_code: u8,
    /// 0 free, 1 anchoring, 2 locked, 3 re-anchor pending.
    pub phase_state: u8,
    /// 0 = 44.1, 1 = 48, 2 = 96 kHz, 0xFF other.
    pub rate_code: u8,
    /// The rate, Hz; 0 when unknown.
    pub rate_hz: u32,
    /// RP2350 hardware SOF timestamps.
    pub hw_timestamp: bool,
    /// The host plays a stream to the unit.
    pub stream: bool,
    /// The host follows the SOF-locked clock.
    pub nominal_feedback: bool,
    /// A usable time base.
    pub time_base: bool,
    /// No SOF for over 50 ms: the time base is extrapolated.
    pub holdover: bool,
    // -- WP AUSB2 --
    /// The VCXO started at the host rate the unit stored (WP USB3).
    pub seeded: bool,
    /// Mode 2 cannot be in step (host rejected, a configuration that cannot
    /// phase-lock, or a phase error pending): the unit is muted or plays
    /// unaligned, as [`AlinkUsbConfig::on_lock_error`] says (WP USB3b).
    pub lock_error: bool,
    /// The host's SOF rate against the unit's crystal, ppm (+ = host fast).
    pub host_ppm: f32,
    /// The soft VCXO's correction, ppm.
    pub vcxo_ppm: f32,
    /// The phase error, samples (phase-locked only).
    pub phase_error_samples: f32,
    /// The USB sample index S arriving now; 0 without a time base.
    pub sample: u64,
    /// Its bus frame (0-2047).
    pub bus_frame: u16,
    /// The smallest D phase lock takes now.
    pub d_min: u16,
    /// The largest.
    pub d_max: u16,
    /// The output ring's fill, samples.
    pub fill: u16,
    /// The bus frame the current stream's content began in (2.9.3); none
    /// without a stream on the timeline.
    pub stream_start_bus_frame: Option<u16>,
    /// Frames since then.
    pub stream_age_frames: u32,
    /// Silence inserted minus late samples dropped since then.
    pub stream_shift: i32,
    /// Faded gaps since boot (wraps at 65536).
    pub gap_count: u16,
    /// Anchors since boot.
    pub anchors: u32,
    /// Re-anchors since boot.
    pub reanchors: u32,
    /// Silence inserted for missing packets, samples since boot.
    pub missed_samples: u32,
    /// Samples dropped because their packet came too late, since boot.
    pub late_samples: u32,
}

/// One cause of timeline ends with its count (`EXT_ALINK_USB_DIAG`).
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AlinkUsbEnd {
    /// `sof_lost`, `long_gap`, `discontinuity`, `host_rejected`,
    /// `rate_or_config`, `reanchor_phase_error`, `reanchor_output_reset`,
    /// `reanchor_other`, `stop`.
    pub cause: String,
    /// How often.
    pub count: u16,
}

/// What `EXT_ALINK_USB_DIAG` reports: why a unit lost its timeline.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AlinkUsbDiag {
    // -- WP AUSB2 --
    /// The layout read: 1, or 2 since the fork's WP USB3b (with
    /// [`Self::bad_sof_readings`] and [`Self::lock_errors`]).
    pub version: u8,
    /// The latest end's cause ([`AlinkUsbEnd::cause`]); none yet.
    pub last_cause: Option<String>,
    /// SOF absences ridden through.
    pub rides: u16,
    /// SOF absences of 50 ms or more.
    pub holdovers: u16,
    /// Timeline ends by cause, in the firmware's order.
    pub ends: Vec<AlinkUsbEnd>,
    /// USB suspends.
    pub suspends: u16,
    /// USB resumes.
    pub resumes: u16,
    /// USB bus resets.
    pub bus_resets: u16,
    /// Disconnects.
    pub unplugs: u16,
    /// The latest absence: frames counted on the bus.
    pub last_frames: u32,
    // -- WP AUSB2 --
    /// RP2350 SOF timestamps the unit repaired (layout 2; 0 in layout 1).
    pub bad_sof_readings: u16,
    /// Entries into lock error (layout 2; 0 in layout 1).
    pub lock_errors: u16,
    /// Its duration by the unit's clock, µs.
    pub last_us: u32,
    /// Clock minus frames at the host rate, µs.
    pub last_dev_us: i32,
    /// The longest absence ridden through, µs.
    pub longest_ride_us: u32,
    /// The longest main-loop pass, µs.
    pub max_pass_us: u32,
    /// The longest time without a SOF interrupt, µs.
    pub max_isr_quiet_us: u32,
    /// The latest SOF interrupt after its SOF, µs.
    pub max_isr_late_us: u32,
    /// The running holdover's age, µs.
    pub holdover_us: u32,
}

/// What `EXT_FRAME_COUNTER` reports, with its time base.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AlinkTimeBaseCounter {
    /// The alink frame F (time base `alink`) or the USB sample S
    /// (`usb_sample`); 0 with none.
    pub frame: u64,
    /// The unit's clock at the read, µs (wraps).
    pub time_us: u32,
    /// Output samples played, low 32 bits.
    pub output_frames: u32,
    /// `alink`, `usb_sample` or `none`.
    pub time_base: String,
    /// Its code (0, 1, 0xFF).
    pub time_base_code: u8,
    /// The bus frame of the latest SOF (time base `usb_sample` only).
    pub bus_frame: u16,
}

#[cfg_attr(feature = "uniffi", uniffi::export)]
impl DspiCore {
    /// Whether the unit has the alink's USB-direct block, as the connect
    /// probe found it, without traffic. `Disconnected` without a device.
    pub fn usb_direct_supported(&self) -> Result<bool> {
        self.ensure_probed();
        let mut inner = self.lock();
        Ok(inner.conn()?.ext.usb_direct())
    }

    /// Read `EXT_ALINK_USB`: the mode, automatic re-anchor, D, why mode 2
    /// runs as mode 1, and the state.
    ///
    /// `Unsupported`, with no traffic, without the block; also when the read
    /// stalls or a DSPi Link hub refuses it. `Disconnected` without a device;
    /// `Transfer` for a short or failed read.
    pub fn fetch_alink_usb(&self) -> Result<AlinkUsbConfig> {
        self.ensure_probed();
        let mut inner = self.lock();
        let conn = usb_conn(&mut inner, usb::EXT_ALINK_USB)?;
        let c = sx::read_alink_usb(conn.transport()).map_err(read_error)?;
        Ok(config(&c))
    }

    /// Write `EXT_ALINK_USB`: `mode` 0 off, 1 rate lock, 2 phase lock;
    /// -- WP AUSB2 -- the unit's lock-error action (byte 4) is read first and
    /// kept (mute when it cannot be read);
    /// `auto_reanchor` (normally true); `d` samples from arrival to output,
    /// the same on every unit that should play aligned (170-399 at 48 kHz,
    /// 324-351 at 96 kHz in mode 2). Takes effect at once; a change of mode
    /// or D while phase-locked re-anchors (about D + 20 ms of silence). The
    /// core never saves it: the firmware stores it in flash only on
    /// `REQ_SAVE_OUTPUT_CONFIG`, which this call does not send, so the unit
    /// keeps it across a restart only after the user saves.
    ///
    /// Errors, with no traffic: `Disconnected`; `Unsupported` without the
    /// block; `InvalidArgument` for a mode above 2 or D 0; `ReadOnly`. From the
    /// device: `Refused` when it stalls (mode 2 with D outside its limits now);
    /// `Denied`, `Locked` or `Transfer` from a hub.
    pub fn set_alink_usb(&self, mode: u8, auto_reanchor: bool, d: u16) -> Result<()> {
        self.ensure_probed();
        let c = usb::AlinkUsbConfig {
            mode,
            flags: if auto_reanchor {
                usb::FLAG_AUTO_REANCHOR
            } else {
                0
            },
            d,
            // -- WP AUSB2 -- replaced by the unit's own below
            on_lock_error: usb::ON_LOCK_ERROR_MUTE,
            status_flags: 0,
            state: 0,
        };
        if !c.is_valid() {
            return Err(DspiError::InvalidArgument {
                msg: format!(
                    "audio link USB-direct: mode {mode} with D {d} (modes 0 off, 1 rate lock, 2 phase lock; D at least 1)"
                ),
            });
        }
        let mut inner = self.lock();
        usb_conn(&mut inner, usb::EXT_ALINK_USB)?;
        let mut w = inner.writable_conn()?;
        let t = w.sending(); // -- WP ECHO1 --
        let read = sx::read_alink_usb(t).ok();
        let before = read.as_ref().map(config_value);
        // -- WP AUSB2 -- never reset the lock-error action by writing the mode
        let c = usb::AlinkUsbConfig {
            on_lock_error: read.map_or(usb::ON_LOCK_ERROR_MUTE, |r| r.on_lock_error),
            ..c
        };
        sx::write_alink_usb(t, c).map_err(write_error)?;
        journal::Pending::of(journal::ALINK_USB, &[], before)
            .done_with(&mut inner, config_value(&c));
        Ok(())
    }

    // -- WP AUSB2 --
    /// Set what the unit does in mode 2 when it cannot phase-lock (*lock
    /// error*: the host's rate out of range, a configuration that cannot
    /// phase-lock, a phase error pending; the fork's WP USB3b,
    /// `extension_channel_spec.md` 2.9.2): `play_unaligned` false mutes it
    /// (the default: it was asked to be in step), true lets it play
    /// unaligned. Writes `EXT_ALINK_USB` with its mode, flags and D as read,
    /// only byte 4 changed, so nothing re-anchors. The core never saves it
    /// (`REQ_SAVE_OUTPUT_CONFIG` stores it).
    ///
    /// Errors, with no traffic: `Disconnected`; `Unsupported` without the
    /// block; `ReadOnly`. `Transfer` when the setting cannot be read first.
    /// `Unsupported` when the unit refuses `play_unaligned` (firmware before
    /// WP USB3b, where the byte is reserved: the test for the setting) while it
    /// takes its settings as they are; `Refused` when it refuses those too
    /// (mode 2 with D outside its limits at this rate, said by its status
    /// flags); `Denied`, `Locked` or `Transfer` from a hub.
    ///
    /// -- fixup ausb2 (F5) -- A unit in a DSPi Link hub's installation takes
    /// the action the installation asks: the hub writes it back within a poll
    /// (about 5 s) and records it (alarm `usb_setting_restored`); change it in
    /// the installation (the Installer's "When it can't lock") instead.
    pub fn set_alink_usb_on_lock_error(&self, play_unaligned: bool) -> Result<()> {
        self.ensure_probed();
        let mut inner = self.lock();
        usb_conn(&mut inner, usb::EXT_ALINK_USB)?;
        let mut w = inner.writable_conn()?;
        let t = w.sending(); // -- WP ECHO1 --
        let read = sx::read_alink_usb(t).map_err(read_error)?;
        let c = usb::AlinkUsbConfig {
            on_lock_error: if play_unaligned {
                usb::ON_LOCK_ERROR_PLAY
            } else {
                usb::ON_LOCK_ERROR_MUTE
            },
            status_flags: 0,
            state: 0,
            ..read
        };
        match sx::write_alink_usb(t, c).map_err(write_error) {
            // -- fixup ausb2 (F2) -- a refusal is the firmware's lack of the
            // setting only when the same settings with the unit's own action are
            // taken; else they are refused as they are (D outside its range at
            // this rate: `status_flags`), a configuration to fix, not old firmware
            Err(e @ DspiError::Refused { .. }) if play_unaligned => {
                let explained = read.status_flags
                    & (usb::STATUS_D_BELOW_MIN
                        | usb::STATUS_D_ABOVE_MAX
                        | usb::STATUS_RATE_UNSUPPORTED)
                    != 0;
                let own = usb::AlinkUsbConfig {
                    status_flags: 0,
                    state: 0,
                    ..read
                };
                if explained || sx::write_alink_usb(t, own).is_err() {
                    return Err(e);
                }
                return Err(DspiError::Unsupported);
            }
            r => r?,
        }
        journal::Pending::of(journal::ALINK_USB, &[], Some(config_value(&read)))
            .done_with(&mut inner, config_value(&c));
        Ok(())
    }

    /// Read `EXT_ALINK_USB_STATUS`. Errors as [`DspiCore::fetch_alink_usb`].
    pub fn fetch_alink_usb_status(&self) -> Result<AlinkUsbStatus> {
        self.ensure_probed();
        let mut inner = self.lock();
        let conn = usb_conn(&mut inner, usb::EXT_ALINK_USB_STATUS)?;
        let s = sx::read_alink_usb_status(conn.transport()).map_err(read_error)?;
        Ok(status(&s))
    }

    /// Read `EXT_ALINK_USB_DIAG` (layout version 1 or 2 -- WP AUSB2 --;
    /// another reads as `Transfer`). Errors as [`DspiCore::fetch_alink_usb`].
    pub fn fetch_alink_usb_diag(&self) -> Result<AlinkUsbDiag> {
        self.ensure_probed();
        let mut inner = self.lock();
        let conn = usb_conn(&mut inner, usb::EXT_ALINK_USB_DIAG)?;
        let d = sx::read_alink_usb_diag(conn.transport()).map_err(read_error)?;
        Ok(diag(&d))
    }

    /// Clear `EXT_ALINK_USB_DIAG`. Errors as [`DspiCore::set_alink_usb`].
    pub fn clear_alink_usb_diag(&self) -> Result<()> {
        self.ensure_probed();
        let mut inner = self.lock();
        usb_conn(&mut inner, usb::EXT_ALINK_USB_DIAG)?;
        let mut w = inner.writable_conn()?;
        sx::clear_alink_usb_diag(w.sending()).map_err(write_error)?;
        journal::journal(
            &mut inner,
            journal::ALINK_USB_DIAG_CLEAR,
            &[],
            Value::Trigger,
        );
        Ok(())
    }

    /// Read `EXT_FRAME_COUNTER` with its time base: the alink's frame, or the
    /// USB sample index and bus frame on a USB-direct unit not in mode 0.
    /// `Unsupported`, with no traffic, on a unit with neither the alink nor
    /// the USB-direct block; otherwise errors as [`DspiCore::fetch_alink_usb`].
    pub fn fetch_frame_counter_time_base(&self) -> Result<AlinkTimeBaseCounter> {
        self.ensure_probed();
        let mut inner = self.lock();
        let conn = time_base_conn(&mut inner, alink::EXT_FRAME_COUNTER)?;
        let c = sx::read_frame_counter_tb(conn.transport()).map_err(read_error)?;
        Ok(AlinkTimeBaseCounter {
            frame: c.counter.frame,
            time_us: c.counter.time_us,
            output_frames: c.counter.output_frames,
            time_base: c.time_base.as_str().into(),
            time_base_code: c.time_base.raw(),
            bus_frame: c.bus_frame,
        })
    }

    /// `EXT_APPLY_AT_FRAME` on time base `time_base`: [`TIME_BASE_ALINK`]
    /// (`frame` an alink frame, as [`DspiCore::apply_at_frame`]),
    /// [`TIME_BASE_USB_SAMPLE`] (`frame` the unit's USB sample index S) or
    /// [`TIME_BASE_BUS_FRAME`] (`frame` a bus frame 0-2047 in bits 0-10 and
    /// the sample in it in bits 16-23: the same 8 bytes address the same
    /// instant on every unit of one host, up to 1.98 s ahead). Ops and their
    /// arguments are [`DspiCore::apply_at_frame`]'s.
    ///
    /// Errors, with no traffic: `Unsupported` for time base 0 without the
    /// alink, 1 or 2 without the USB-direct block; `InvalidArgument` for
    /// another time base, a bad bus-frame form, or an op or argument out of
    /// range; `ReadOnly`, `Disconnected`. `Refused` when the unit stalls (not
    /// phase-locked on the USB time base, a full queue).
    pub fn apply_at_time_base(
        &self,
        time_base: u8,
        frame: u64,
        op: u8,
        arg: u8,
        value: i16,
    ) -> Result<()> {
        self.ensure_probed();
        let tb = TimeBase::from_raw(time_base);
        let req = ApplyRequest {
            frame,
            op: ApplyOp::from_raw(op),
            arg,
            value,
        };
        let form_ok = match tb {
            TimeBase::Alink | TimeBase::UsbSample => true,
            TimeBase::BusFrame => usb::decode_bus_target(frame).is_some(),
            _ => false,
        };
        if op > ApplyOp::MAX || !req.args_valid() || !form_ok {
            return Err(DspiError::InvalidArgument {
                msg: format!(
                    "apply at frame: time base {time_base}, frame {frame:#x}, op {op} with arg {arg} and value {value} \
                     (time bases 0 alink, 1 USB sample, 2 bus frame; extension_channel_spec.md 2.9.4)"
                ),
            });
        }
        let mut inner = self.lock();
        let conn = inner.conn()?;
        let allowed = match tb {
            TimeBase::Alink => conn.ext.alink(),
            _ => conn.ext.usb_direct(),
        } && conn.ext.has(alink::EXT_APPLY_AT_FRAME);
        if !allowed {
            return Err(DspiError::Unsupported);
        }
        let mut w = inner.writable_conn()?;
        sx::apply_at_frame_tb(w.sending(), req, tb).map_err(write_error)?;
        journal::journal(
            &mut inner,
            journal::ALINK_APPLY,
            &[op, arg],
            Value::Bytes(usb::encode_apply(&req, tb).to_vec()),
        );
        Ok(())
    }

    /// Open the demo device as an alink USB-direct unit (the fake device's
    /// model, `extension_channel_spec.md` 2.9: phase-locked at 48 kHz with D
    /// 192, its host 11.5 ppm slow, a stream playing), for clients without
    /// hardware. Replaces any open device.
    pub fn open_demo_usb_direct(&self) -> Result<()> {
        let demo = dspi_transport::FakeDevice::demo_usb_direct(Default::default());
        let serial = demo.descriptor().serial.clone();
        self.attach_probed(Box::new(demo), serial, 0)
    }
}

/// The open connection when it has the USB-direct block and `EXT_LIST` named
/// `id`; else `Unsupported` (no traffic) or `Disconnected`.
fn usb_conn(inner: &mut Inner, id: u16) -> Result<&mut Connection> {
    let conn = inner.conn()?;
    if !conn.ext.usb_direct() || !conn.ext.has(id) {
        return Err(DspiError::Unsupported);
    }
    Ok(conn)
}

/// The open connection when it has the alink or the USB-direct block and
/// `EXT_LIST` named `id` (the frame counter, the apply).
fn time_base_conn(inner: &mut Inner, id: u16) -> Result<&mut Connection> {
    let conn = inner.conn()?;
    if !conn.ext.time_base_id(id) {
        return Err(DspiError::Unsupported);
    }
    Ok(conn)
}

pub(crate) fn config(c: &usb::AlinkUsbConfig) -> AlinkUsbConfig {
    let state = usb::UsbState::from_raw(c.state);
    AlinkUsbConfig {
        mode: c.mode,
        mode_name: c.usb_mode().as_str().into(),
        auto_reanchor: c.flags & usb::FLAG_AUTO_REANCHOR != 0,
        d: c.d,
        d_below_min: c.status_flags & usb::STATUS_D_BELOW_MIN != 0,
        d_above_max: c.status_flags & usb::STATUS_D_ABOVE_MAX != 0,
        rate_unsupported: c.status_flags & usb::STATUS_RATE_UNSUPPORTED != 0,
        on_lock_error: on_lock_error_name(c.on_lock_error).into(),
        state: state.as_str().into(),
        state_code: c.state,
    }
}

// -- WP AUSB2 --
/// `EXT_ALINK_USB.on_lock_error` by name: `mute`, `play_unaligned`, `unknown`.
pub(crate) fn on_lock_error_name(code: u8) -> &'static str {
    match code {
        usb::ON_LOCK_ERROR_MUTE => "mute",
        usb::ON_LOCK_ERROR_PLAY => "play_unaligned",
        _ => "unknown",
    }
}

/// The journal's value of a USB-direct setting: `[mode, flags, d LE,
/// on_lock_error]` (-- WP AUSB2 -- with byte 4), without the device's IN-only
/// bytes.
fn config_value(c: &usb::AlinkUsbConfig) -> Value {
    let mut b = vec![c.mode, c.flags];
    b.extend(c.d.to_le_bytes());
    b.push(c.on_lock_error);
    Value::Bytes(b)
}

pub(crate) fn status(s: &usb::AlinkUsbStatus) -> AlinkUsbStatus {
    AlinkUsbStatus {
        state: s.state.as_str().into(),
        state_code: s.state.raw(),
        phase_state: s.phase_state,
        rate_code: s.rate_code,
        rate_hz: match s.rate_code {
            0 => 44_100,
            1 => 48_000,
            2 => 96_000,
            _ => 0,
        },
        hw_timestamp: s.flags & usb::USB_FLAG_HW_TIMESTAMP != 0,
        stream: s.flags & usb::USB_FLAG_STREAM != 0,
        nominal_feedback: s.flags & usb::USB_FLAG_NOMINAL_FEEDBACK != 0,
        time_base: s.flags & usb::USB_FLAG_TIME_BASE != 0,
        holdover: s.flags & usb::USB_FLAG_HOLDOVER != 0,
        seeded: s.flags & usb::USB_FLAG_SEEDED != 0,
        lock_error: s.flags & usb::USB_FLAG_LOCK_ERROR != 0,
        host_ppm: s.host_ppm,
        vcxo_ppm: s.vcxo_ppm,
        phase_error_samples: s.phase_error,
        sample: s.sample,
        bus_frame: s.bus_frame,
        d_min: s.d_min,
        d_max: s.d_max,
        fill: s.fill,
        stream_start_bus_frame: s.stream_start_bus_frame(),
        stream_age_frames: s.stream_age_frames,
        stream_shift: s.stream_shift,
        gap_count: s.gap_count,
        anchors: s.anchors,
        reanchors: s.reanchors,
        missed_samples: s.missed_samples,
        late_samples: s.late_samples,
    }
}

pub(crate) fn diag(d: &usb::AlinkUsbDiag) -> AlinkUsbDiag {
    AlinkUsbDiag {
        version: d.version,
        last_cause: d.last_cause_name().map(str::to_owned),
        rides: d.rides,
        holdovers: d.holdovers,
        ends: d
            .ends
            .iter()
            .zip(0u8..)
            .map(|(n, i)| AlinkUsbEnd {
                cause: usb::end_cause_name(i).unwrap_or("unknown").into(),
                count: *n,
            })
            .collect(),
        suspends: d.suspends,
        resumes: d.resumes,
        bus_resets: d.bus_resets,
        unplugs: d.unplugs,
        last_frames: d.last_frames,
        bad_sof_readings: d.bad_sof_readings,
        lock_errors: d.lock_errors,
        last_us: d.last_us,
        last_dev_us: d.last_dev_us,
        longest_ride_us: d.longest_ride_us,
        max_pass_us: d.max_pass_us,
        max_isr_quiet_us: d.max_isr_quiet_us,
        max_isr_late_us: d.max_isr_late_us,
        holdover_us: d.holdover_us,
    }
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_ausb1;

// -- WP AUSB2 --
#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_ausb2;
