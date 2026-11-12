//! Extension channel (`REQ_EXTENSION` 0x00, Bofr5555/DSPi fork,
//! `Documentation/Features/extension_channel_spec.md`): the crossfeed pair
//! map (`EXT_CROSSFEED_PAIR_MAP`) and the core voltage (`EXT_CORE_VREG`).
//!
//! What the device offers was probed once on connect (`ExtState::probe`,
//! WP S3) and is kept on the connection. A sub-command is usable only when
//! its probe read succeeded: `ExtState::has(id)` can be true while the value
//! is `None` (the read stalled, was short, or a DSPi Link hub refused this
//! client's role), so the `Option` decides, never `has`. An absent
//! sub-command is [`DspiError::Unsupported`] with no traffic.
//!
//! A read that stalls or is refused by a DSPi Link hub (`Denied`) is
//! "absent" too (`Unsupported`): a stall is the device's answer and is never
//! retried.
//!
//! Owned by WP C8 (docs/core-merge/PLAN.md).

use dspi_session::ext::{self as sx, CoreVregStatus, ExtError, PairMap, VregOp};
use dspi_transport::TransportError;

use crate::consts::{CROSSFEED_MAP_PAIRS, EXT_WRITE_REFUSED};
use crate::records::*;
use crate::{DspiCore, DspiError, Inner, Result, state};

#[cfg_attr(feature = "uniffi", uniffi::export)]
impl DspiCore {
    /// `EXT_CROSSFEED_PAIR_MAP` (Bofr5555/DSPi fork, the K340 build): the
    /// outputs of every crossfeed pair at once, 4 bytes
    /// (`DspState.crossfeed_pair_map` encoding).
    ///
    /// The map is checked first: `InvalidArgument`, with no traffic, for a
    /// map the firmware would refuse ([`crate::crossfeed_pair_map_valid`] for
    /// the platform's pair count). It is then sent normalized and read back,
    /// because the firmware ACKs an invalid map and ignores it; the mirror
    /// (`crossfeed_pair_map` and `bulk_raw`) takes what the device reports.
    ///
    /// Errors: `Disconnected`; `ReadOnly` on a read-only connection;
    /// `Unsupported` when the device has no usable pair map (not listed, or
    /// its probe read failed); `InvalidArgument` as above, or when the
    /// read-back shows the device kept another map; `Refused { status:
    /// EXT_WRITE_REFUSED }` when the device stalls the write; `Denied`,
    /// `Locked` or `Transfer` from the link.
    pub fn set_crossfeed_pair_map(&self, map: Vec<u8>) -> Result<()> {
        self.ensure_probed(); // -- WP LK4 --
        set_pair_map_locked(&mut self.lock(), &map)
    }

    /// Read the live pair map (4 bytes) into the mirror: `crossfeed_pair_map`
    /// and `bulk_raw`, which carries it in the crossfeed section.
    ///
    /// `Unsupported`, with no traffic, when the device has no usable pair
    /// map (stock firmware stalls `EXT_LIST`; a DSPi Link hub refuses a
    /// viewer), or when the read stalls or is refused: the mirror then keeps
    /// the value decoded from the bulk image (all zeros, the default
    /// pairing, on stock firmware). `Disconnected` without a device;
    /// `Transfer` for a short or failed read.
    pub fn fetch_crossfeed_pair_map(&self) -> Result<Vec<u8>> {
        self.ensure_probed(); // -- WP LK4 --
        fetch_pair_map_locked(&mut self.lock())
    }

    /// What the extension channel offers on the connected device, as probed
    /// on connect; no traffic. Empty on stock firmware. `has_pair_map` and
    /// `has_core_vreg` are true only when the sub-command was listed *and*
    /// its probe read succeeded. `Disconnected` without a device.
    pub fn ext_status(&self) -> Result<ExtStatus> {
        self.ensure_probed(); // -- WP LK4 --
        let mut inner = self.lock();
        let ext = &inner.conn()?.ext;
        Ok(ExtStatus {
            supported: ext.supported.clone(),
            has_pair_map: ext.pair_map.is_some(),
            has_core_vreg: ext.core_vreg.is_some(),
        })
    }

    /// Read `EXT_CORE_VREG` (16 bytes) and refresh the connection's copy.
    ///
    /// `Unsupported`, with no traffic, when the device has no usable
    /// core-voltage extension, and when the read stalls or a DSPi Link hub
    /// refuses it; `Disconnected` without a device; `Transfer` for a short
    /// or failed read.
    pub fn fetch_core_vreg(&self) -> Result<CoreVregInfo> {
        self.ensure_probed(); // -- WP LK4 --
        let mut inner = self.lock();
        let conn = inner.conn()?;
        if conn.ext.core_vreg.is_none() {
            return Err(DspiError::Unsupported);
        }
        let status = sx::read_core_vreg(conn.transport()).map_err(read_error)?;
        let info = vreg_info(&status);
        conn.ext.core_vreg = Some(status);
        Ok(info)
    }

    /// `EXT_CORE_VREG` write: `op` 0 set (value = VREG code, 0 = build
    /// default), 1 trial, 2 end trial, 3 auto (value 0/1), 4 clear
    /// (`dspi_proto::ext::VregOp`).
    ///
    /// Checked before anything is sent (`InvalidArgument`, no traffic): an
    /// unknown op; a SET below the build default (the floor,
    /// `CoreVregInfo::default_vreg`) or above 0x0F (1.30 V); a TRIAL outside
    /// 0x0C..=0x0F (1.15-1.30 V); an AUTO value other than 0 or 1. The value
    /// of TRIAL_END and CLEAR is ignored (sent as 0). Read the status back
    /// with [`DspiCore::fetch_core_vreg`].
    ///
    /// WP CG14: a SET or TRIAL reads the unit's status first and goes through
    /// `dspi_session::overvolt`: a raise above 1.20 V from what it reports now
    /// needs `word`, exactly `OVERVOLT`, or nothing is written. This is the
    /// one path to 1.30 V; the GUIs' flow stops at 1.25 V (`core_vreg_cal_*`).
    ///
    /// Errors: `OvervoltWordRequired` as above; `Disconnected`; `ReadOnly` on
    /// a read-only connection;
    /// `Unsupported` when the device has no usable core-voltage extension;
    /// `Refused { status: EXT_WRITE_REFUSED }` when the device stalls the
    /// request anyway; `Denied`, `Locked` or `Transfer` from the link.
    pub fn core_vreg_op(&self, op: u8, value: u8, word: Option<String>) -> Result<()> {
        self.ensure_probed(); // -- WP LK4 --
        let mut inner = self.lock();
        let floor = match &inner.conn()?.ext.core_vreg {
            Some(s) => s.default_vreg,
            None => return Err(DspiError::Unsupported),
        };
        let request = vreg_request(op, value, floor)?;
        let mut w = inner.writable_conn()?;
        sx::core_vreg_op(w.conn().transport(), request, word.as_deref()).map_err(write_error)
    }
}

// ------------------------------------------------- for WP C4 (crossfeed)

/// [`DspiCore::set_crossfeed_pair_map`] with the lock already held, for
/// WP C4's crossfeed setters or a caller changing several things in one
/// critical section. Same contract and errors.
pub(crate) fn set_pair_map_locked(inner: &mut Inner, map: &[u8]) -> Result<()> {
    let pairs = crossfeed_pairs(&inner.state);
    let raw = <[u8; CROSSFEED_MAP_PAIRS]>::try_from(map)
        .ok()
        .filter(|_| crate::crossfeed_pair_map_valid(map.to_vec(), pairs))
        .ok_or_else(|| DspiError::InvalidArgument {
            msg: format!(
                "not a crossfeed pair map for {pairs} pairs: 4 bytes, every pair \
                 on two distinct stereo outputs, no output in two pairs, unused \
                 pairs 0"
            ),
        })?;
    let wanted = PairMap(raw).normalized();
    if inner.conn()?.ext.pair_map.is_none() {
        return Err(DspiError::Unsupported);
    }

    let mut w = inner.writable_conn()?;
    let written = sx::write_pair_map(w.conn().transport(), wanted);
    let now = match written {
        Ok(now) => now,
        // ACKed and ignored: the read-back disagreed. Read once more for
        // the map the device kept, so the mirror shows it.
        Err(ExtError::NotApplied(why)) => {
            let kept = sx::read_pair_map(w.conn().transport()).map_err(read_error)?;
            store_pair_map(inner, kept);
            return Err(DspiError::InvalidArgument { msg: why });
        }
        Err(e) => return Err(write_error(e)),
    };
    store_pair_map(inner, now);
    Ok(())
}

/// [`DspiCore::fetch_crossfeed_pair_map`] with the lock already held. Same
/// contract and errors.
pub(crate) fn fetch_pair_map_locked(inner: &mut Inner) -> Result<Vec<u8>> {
    let conn = inner.conn()?;
    if conn.ext.pair_map.is_none() {
        return Err(DspiError::Unsupported);
    }
    let map = sx::read_pair_map(conn.transport()).map_err(read_error)?;
    store_pair_map(inner, map);
    Ok(map.0.to_vec())
}

/// Put a map the device reported into the connection's copy and the mirror.
fn store_pair_map(inner: &mut Inner, map: PairMap) {
    if let Some(c) = inner.conn.as_mut() {
        c.ext.pair_map = Some(map);
    }
    inner.state.crossfeed_pair_map = map.0.to_vec();
    // Only this build's own image is re-encoded: a read-only connection
    // (another wire format, or no image) keeps the image it was shown.
    if inner.state.bulk_valid {
        inner.state.bulk_raw = state::encode_bulk(&inner.state);
    }
}

/// Crossfeed pairs on the platform: 2 on RP2040, 4 on RP2350 and STM32H723
/// (the stereo outputs, without the PDM sub; `NUM_SPDIF_INSTANCES` in both
/// firmwares' `config.h`). The rule `files.rs` uses too.
// -- WP QPI2 -- platform 2 (STM32H723) has four pairs, as the Mac counts them
pub(crate) fn crossfeed_pairs(s: &DspState) -> u8 {
    if s.platform_id == 0 { 2 } else { 4 }
}

/// Check an `EXT_CORE_VREG` request against the spec's limits
/// (extension_channel_spec.md 2.3); `floor` is the build default.
fn vreg_request(op: u8, value: u8, floor: u8) -> Result<VregOp> {
    const MIN_TRIAL: u8 = 0x0C;
    const MAX: u8 = 0x0F;
    let bad = |msg: String| Err(DspiError::InvalidArgument { msg });
    match op {
        0 if value == 0 || (floor..=MAX).contains(&value) => Ok(VregOp::Set(value)),
        0 => bad(format!(
            "core voltage 0x{value:02X} is outside the build default 0x{floor:02X} to 0x{MAX:02X}"
        )),
        1 if (MIN_TRIAL..=MAX).contains(&value) => Ok(VregOp::Trial(value)),
        1 => bad(format!(
            "trial voltage 0x{value:02X} is outside 0x{MIN_TRIAL:02X} to 0x{MAX:02X}"
        )),
        2 => Ok(VregOp::TrialEnd),
        3 if value <= 1 => Ok(VregOp::Auto(value == 1)),
        3 => bad(format!("auto takes 0 or 1, not {value}")),
        4 => Ok(VregOp::Clear),
        _ => bad(format!("unknown core-voltage op {op}")),
    }
}

/// The record for a decoded status.
fn vreg_info(s: &CoreVregStatus) -> CoreVregInfo {
    CoreVregInfo {
        live_vreg: s.live_vreg,
        millivolts: sx::vreg_mv(s.live_vreg).unwrap_or(0),
        stored_sel: s.stored_sel,
        default_vreg: s.default_vreg,
        auto_raised: s.flags & CoreVregStatus::FLAG_AUTO_RAISED != 0,
        auto_disabled: s.flags & CoreVregStatus::FLAG_AUTO_DISABLED != 0,
        hang_count: s.hang_count,
        trial_active: s.trial_active,
        trial_vreg: s.trial_vreg,
        failed_trial_vreg: s.failed_trial_vreg,
        failed_trial_uptime_s: s.failed_trial_uptime_s,
        uptime_s: s.uptime_s,
    }
}

/// A failed extension read: a stall, or a DSPi Link hub refusing this
/// client's role, is "absent" (`Unsupported`); everything else is the
/// transport's error.
fn read_error(e: ExtError) -> DspiError {
    match e {
        ExtError::Unsupported { .. } | ExtError::Transport(TransportError::Denied(_)) => {
            DspiError::Unsupported
        }
        ExtError::Refused { .. } => refused(),
        ExtError::Transport(t) => t.into(),
        e @ ExtError::WordRequired { .. } => DspiError::OvervoltWordRequired { msg: e.to_string() },
        ExtError::NotApplied(msg) | ExtError::BadArgument(msg) => {
            DspiError::InvalidArgument { msg }
        }
    }
}

/// A failed extension write: a stall is the device refusing the request
/// (extension_channel_spec.md 1.1). A refusal by a DSPi Link hub stays
/// `Denied`: the caller asked to change something it may not.
fn write_error(e: ExtError) -> DspiError {
    match e {
        ExtError::Refused { .. } | ExtError::Unsupported { .. } => refused(),
        ExtError::Transport(t) => t.into(),
        e @ ExtError::WordRequired { .. } => DspiError::OvervoltWordRequired { msg: e.to_string() },
        ExtError::NotApplied(msg) | ExtError::BadArgument(msg) => {
            DspiError::InvalidArgument { msg }
        }
    }
}

fn refused() -> DspiError {
    DspiError::Refused {
        status: EXT_WRITE_REFUSED,
    }
}

/// Tests, and the counting transport `info.rs`'s tests share.
#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
pub(crate) mod tests_wp_c8;

// -- WP QPI2 --
#[cfg(test)]
mod tests_wp_qpi2 {
    use super::*;

    #[test]
    fn wp_qpi2_crossfeed_pairs_follow_the_output_slots() {
        let pairs = |platform_id| {
            crossfeed_pairs(&DspState {
                platform_id,
                ..state::default_state()
            })
        };
        assert_eq!(pairs(0), 2, "RP2040");
        assert_eq!(pairs(1), 4, "RP2350");
        assert_eq!(pairs(2), 4, "STM32H723");
    }
}
