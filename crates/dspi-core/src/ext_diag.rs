//! WP FWX (PRO1 FW1, FW4, FW5): identify (`EXT_IDENTIFY` 0x0004), the reset
//! readout (`EXT_RESET_INFO` 0x0005) and the clip event (0xE0, behind
//! `EXT_NOTIFY_FEATURES` bit 1), from the fork's
//! `extension_channel_spec.md` 2.4.1, 2.5 and 2.6.
//!
//! Everything here is gated on what the connect probe found (`ExtState`):
//! a sub-command `EXT_LIST` did not name is [`DspiError::Unsupported`] with no
//! traffic, and a 0xE0 packet from a device without the feature bit is never
//! read as a clip, so bofr-main 04122d3 and stock firmware behave as before.
//!
//! Clip events: `process_notifications` hands every 0xE0 to [`note_clip`],
//! which collects the channels on the connection until
//! [`DspiCore::take_clip_mask`] takes them. The event history (WP P0M,
//! `dspi_session::events::decode_ext_event`) reads the same packets through
//! `dspi_proto::ext::clip_event` with the device's `notify_features`: one
//! event can carry several channels, so it is one clip per bit of the mask.

use dspi_proto::generated::ext as gext;
use dspi_session::ext::{self as sx, IdentifyRequest, IdentifyStatus, ResetInfo};

use crate::ext::{read_error, write_error};
use crate::{Connection, DspiCore, DspiError, Result};

/// What `EXT_IDENTIFY` reports: the blink state and what the board is.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IdentifyInfo {
    /// Blink time left in seconds, rounded up; 0 when not blinking.
    pub remaining_s: u8,
    /// The output of the last identify tone since the device started.
    pub last_tone_output: Option<u8>,
    /// `RP2040` or `RP2350` (`chip N` for a value this build does not know).
    pub chip: String,
    /// The SDK's chip revision number.
    pub chip_rev: u8,
    /// The raw `SYSINFO.CHIP_ID` register.
    pub chip_id: u32,
    /// The board the firmware was built for (`pico`, `pico2`).
    pub board: String,
    /// The hardware revision the firmware build was given; empty when the
    /// build set none (the firmware cannot sense it).
    pub hw_rev: String,
}

/// What `EXT_RESET_INFO` reports: why the device last restarted.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceResetInfo {
    /// A stable name: `power_on`, `brownout`, `run_pin`, `watchdog`,
    /// `software`, `debugger`, `glitch` or `unknown`.
    pub reason: String,
    /// The firmware's reason code (reset_info.h), for a code this build does
    /// not know too.
    pub reason_code: u8,
    /// The reason in words.
    pub description: String,
    /// Seconds since that restart.
    pub uptime_s: u32,
    /// How long the run before it lasted, in seconds; `None` after a power-on
    /// or a brown-out, which clear the record.
    pub prev_uptime_s: Option<u32>,
    /// Watchdog hangs this power session not yet acted on (the core voltage
    /// extension's count).
    pub hang_count: u8,
    /// `RP2040` or `RP2350`.
    pub chip: String,
    /// The raw reset-cause register at boot, for a support report.
    pub chip_reset: u32,
    /// The raw `WATCHDOG.REASON` at boot (bit 8: the firmware's own timeout).
    pub watchdog_reason: u32,
}

/// The firmware's tone level, for a client to say how loud "identify with a
/// tone" is: a 1 kHz sine at this peak level for 400 ms, through the output's
/// crossover, EQ, gain, master volume and limiter.
pub const IDENTIFY_TONE_DBFS: f32 = -40.0;

#[cfg_attr(feature = "uniffi", uniffi::export)]
impl DspiCore {
    /// `EXT_IDENTIFY`: blink the device's LED for `seconds` (0 stops it; over
    /// 120 the firmware shortens to 120) and, with `tone_output`, play its
    /// quiet identify tone ([`IDENTIFY_TONE_DBFS`], 400 ms) on that output.
    /// Leave `tone_output` empty unless the user asked for sound.
    ///
    /// Errors, with no traffic: `Disconnected`; `Unsupported` when the device
    /// did not list it; `InvalidArgument` for a tone output past the
    /// device's outputs; `ReadOnly` on a read-only connection. From the
    /// device: `Refused { status: EXT_WRITE_REFUSED }` when it stalls (a tone
    /// while its signal generator runs); `Denied`, `Locked` or `Transfer`
    /// from the link.
    pub fn identify(&self, seconds: u8, tone_output: Option<u8>) -> Result<()> {
        self.ensure_probed(); // -- WP LK4 --
        let mut inner = self.lock();
        if !inner.conn()?.ext.has(gext::EXT_IDENTIFY) {
            return Err(DspiError::Unsupported);
        }
        let outputs = inner.state.num_output_channels;
        if let Some(o) = tone_output
            && o >= outputs
        {
            return Err(DspiError::InvalidArgument {
                msg: format!("identify tone: output {o} is past this device's {outputs} outputs"),
            });
        }
        let req = IdentifyRequest {
            seconds,
            tone_output,
        };
        let mut w = inner.writable_conn()?;
        sx::identify(w.conn().transport(), req).map_err(write_error)
    }

    /// Read `EXT_IDENTIFY`: the blink time left and the board description.
    ///
    /// `Unsupported`, with no traffic, when the device did not list it, and
    /// when the read stalls or a DSPi Link hub refuses it; `Disconnected`
    /// without a device; `Transfer` for a short or failed read.
    pub fn fetch_identify(&self) -> Result<IdentifyInfo> {
        self.ensure_probed(); // -- WP LK4 --
        let mut inner = self.lock();
        let conn = inner.conn()?;
        if !conn.ext.has(gext::EXT_IDENTIFY) {
            return Err(DspiError::Unsupported);
        }
        let s = sx::read_identify(conn.transport()).map_err(read_error)?;
        Ok(identify_info(&s))
    }

    /// Read `EXT_RESET_INFO`: why the device last restarted, its uptime and
    /// the run before. Also refreshes the connection's copy from the probe.
    ///
    /// `Unsupported`, with no traffic, when the device did not list it, and
    /// when the read stalls or a DSPi Link hub refuses it; `Disconnected`
    /// without a device; `Transfer` for a short or failed read.
    pub fn fetch_reset_info(&self) -> Result<DeviceResetInfo> {
        self.ensure_probed(); // -- WP LK4 --
        let mut inner = self.lock();
        let conn = inner.conn()?;
        if !conn.ext.has(gext::EXT_RESET_INFO) {
            return Err(DspiError::Unsupported);
        }
        let r = sx::read_reset_info(conn.transport()).map_err(read_error)?;
        conn.ext.reset_info = Some(r);
        Ok(reset_info(&r))
    }

    /// The reset readout as the connect probe found it, without traffic:
    /// `None` when the device has no `EXT_RESET_INFO` or the read failed.
    /// Its uptime is the one at connect. `Disconnected` without a device.
    pub fn reset_info_at_connect(&self) -> Result<Option<DeviceResetInfo>> {
        self.ensure_probed(); // -- WP LK4 --
        let mut inner = self.lock();
        Ok(inner.conn()?.ext.reset_info.as_ref().map(reset_info))
    }

    /// Whether the device sends clip events (`EXT_NOTIFY_FEATURES` bit 1):
    /// then a clip shows without polling the meters fast. No traffic;
    /// `Disconnected` without a device.
    pub fn has_clip_events(&self) -> Result<bool> {
        self.ensure_probed(); // -- WP LK4 --
        let mut inner = self.lock();
        Ok(inner.conn()?.ext.clip_events())
    }
}

impl DspiCore {
    /// The channels the device's clip events named since the last call, bit
    /// `c` for wire channel `c` (inputs first, then outputs, as the meters'
    /// clip flags), and clears them. 0 without a device, without the feature
    /// or without a clip. The polled clip flags are separate and unchanged.
    ///
    /// The hook for WP P0M's clip hold and history (not bound for the
    /// clients yet): the event history can also read the packets itself.
    pub fn take_clip_mask(&self) -> u32 {
        let mut inner = self.lock();
        inner
            .conn
            .as_mut()
            .map_or(0, |c| std::mem::take(&mut c.clip_mask))
    }
}

/// A notification the session left as `Event::Unknown { id, bytes }`: when
/// it is a clip event of a device that announces them, collect its channels
/// on the connection and say so.
pub(crate) fn note_clip(conn: &mut Connection, id: u8, bytes: &[u8]) -> bool {
    match sx::clip_event(conn.ext.notify_features, id, bytes) {
        Some(c) => {
            conn.clip_mask |= c.mask;
            true
        }
        None => false,
    }
}

fn identify_info(s: &IdentifyStatus) -> IdentifyInfo {
    IdentifyInfo {
        remaining_s: s.remaining_s,
        last_tone_output: s.last_tone,
        chip: s.chip.label(),
        chip_rev: s.chip_rev,
        chip_id: s.chip_id,
        board: s.board.clone(),
        hw_rev: s.hw_rev.clone(),
    }
}

fn reset_info(r: &ResetInfo) -> DeviceResetInfo {
    DeviceResetInfo {
        reason: r.reason.as_str().into(),
        reason_code: r.reason.raw(),
        description: r.reason.describe().into(),
        uptime_s: r.uptime_s,
        prev_uptime_s: r.prev_uptime_s,
        hang_count: r.hang_count,
        chip: r.chip.label(),
        chip_reset: r.chip_reset,
        watchdog_reason: r.watchdog_reason,
    }
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_fwx;
