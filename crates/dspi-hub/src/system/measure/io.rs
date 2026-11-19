//! A measured unit's transfers, each a command on its worker **as the
//! measuring session** (its id, so its device lock lets them through and every
//! write is attributed to it; judged here by the family's role, as
//! `system.scene.apply` is), waited for. Blocking: callers are a connection's
//! reader or the family's own thread, never a hub thread.

use std::sync::mpsc;
use std::time::{Duration, Instant};

use dspi_link::frames::CmdRequest;
use dspi_link::{Dir, status};
use dspi_proto::ext::alink::{
    self, AlinkInfo, ApplyOp, ApplyRequest, EXT_APPLY_AT_FRAME, SuperframeSnapshot,
};
use dspi_proto::ext::timebase::{
    self as tbc, AlinkSpdif, SiggenTimeline, TimeBaseRead, encode_apply_tb,
};
use dspi_proto::generated::{ext as g, limiter, opcodes as op, siggen};
use dspi_proto::packets::SiggenConfig;

use crate::device::DeviceShared;
use crate::session::{Completion, Role, SessionCtx, SessionId};

/// How long one transfer may take, queue included.
const TRANSFER_WAIT: Duration = Duration::from_secs(3);
/// `EXT_LIST`'s longest reply read: 64 sub-commands.
const EXT_LIST_LEN: u16 = 128;
/// `REQ_GET_STATUS` sub-query of the meter block.
const STATUS_METERS: u16 = 9;
/// The meter block's longest reply: 17 channels.
const METERS_LEN: u16 = 41;
/// The superframe's system flags bit 0: the system mute (2.8.9).
const SYSTEM_MUTE_FLAG: u8 = 1;
/// `SiggenStatus`'s length.
pub(crate) const SIGGEN_STATUS_LEN: u16 = 16;

/// One read with when the hub sent it and when the answer came.
#[derive(Debug, Clone)]
pub(crate) struct Timed<T> {
    pub value: T,
    pub raw: Vec<u8>,
    pub sent: Instant,
    pub got: Instant,
}

/// A unit's I/O as session `session`.
pub(crate) struct Io<'a> {
    pub dev: &'a DeviceShared,
    pub session: SessionId,
}

/// A Link status as a word, for answers.
pub(crate) fn status_word(s: u8) -> &'static str {
    match s {
        status::NO_DEVICE => "offline",
        status::ERROR => "refused",
        status::TIMEOUT => "timeout",
        status::DENIED => "denied",
        status::LOCKED => "locked",
        _ => "failed",
    }
}

impl Io<'_> {
    fn ctx(&self) -> SessionCtx {
        SessionCtx {
            id: self.session,
            role: Role::Admin,
            exempt_inflight: true,
        }
    }

    /// One transfer, waited for: its payload or its Link status.
    pub(crate) fn exec(
        &self,
        dir: Dir,
        opcode: u8,
        value: u16,
        payload: Vec<u8>,
        len: u16,
    ) -> Result<Vec<u8>, u8> {
        let (tx, rx) = mpsc::sync_channel(1);
        let req = CmdRequest {
            tag: 0,
            handle: self.dev.handle,
            dir,
            b_request: opcode,
            w_value: value,
            w_index: dspi_proto::VENDOR_INTERFACE,
            w_length: match dir {
                Dir::Get => len,
                Dir::Set => u16::try_from(payload.len()).unwrap_or(u16::MAX),
            },
            payload,
        };
        let done = Completion::new(0, move |r| {
            let _ = tx.send(r);
        });
        self.dev.submit(
            self.ctx(),
            req,
            done,
            Instant::now().checked_add(TRANSFER_WAIT),
        );
        match rx.recv_timeout(TRANSFER_WAIT.saturating_add(Duration::from_secs(1))) {
            Ok(r) if r.status == status::OK => Ok(r.payload),
            Ok(r) => Err(r.status),
            Err(_) => Err(status::TIMEOUT),
        }
    }

    fn get_ext(&self, id: u16, len: usize) -> Result<Vec<u8>, u8> {
        let len = u16::try_from(len).unwrap_or(u16::MAX);
        self.exec(Dir::Get, op::REQ_EXTENSION, id, Vec::new(), len)
    }

    fn set_ext(&self, id: u16, data: Vec<u8>) -> Result<(), u8> {
        self.exec(Dir::Set, op::REQ_EXTENSION, id, data, 0)
            .map(|_| ())
    }

    fn timed<T>(
        &self,
        id: u16,
        len: usize,
        decode: impl FnOnce(&[u8]) -> Option<T>,
    ) -> Result<Timed<T>, u8> {
        let sent = Instant::now();
        let raw = self.get_ext(id, len)?;
        let got = Instant::now();
        let value = decode(&raw).ok_or(status::ERROR)?;
        Ok(Timed {
            value,
            raw,
            sent,
            got,
        })
    }

    /// `EXT_LIST`.
    pub(crate) fn ext_list(&self) -> Result<Vec<u16>, u8> {
        self.get_ext(g::EXT_LIST, usize::from(EXT_LIST_LEN))
            .map(|b| dspi_proto::ext::decode_list(&b))
    }

    /// `EXT_NOTIFY_FEATURES`, 0 when refused.
    pub(crate) fn notify_features(&self) -> u32 {
        self.get_ext(g::EXT_NOTIFY_FEATURES, 4)
            .ok()
            .and_then(|b| b.get(..4)?.try_into().ok().map(u32::from_le_bytes))
            .unwrap_or(0)
    }

    /// `EXT_TIME_BASE`, timed.
    pub(crate) fn time_base(&self) -> Result<Timed<TimeBaseRead>, u8> {
        self.timed(tbc::EXT_TIME_BASE, TimeBaseRead::SIZE, TimeBaseRead::decode)
    }

    /// `EXT_SIGGEN_TIMELINE`.
    pub(crate) fn timeline(&self) -> Result<Timed<SiggenTimeline>, u8> {
        self.timed(
            tbc::EXT_SIGGEN_TIMELINE,
            SiggenTimeline::SIZE,
            SiggenTimeline::decode,
        )
    }

    /// `EXT_ALINK_INFO`.
    pub(crate) fn alink_info(&self) -> Option<AlinkInfo> {
        self.get_ext(alink::EXT_ALINK_INFO, AlinkInfo::SIZE)
            .ok()
            .and_then(|b| AlinkInfo::decode(&b))
    }

    /// `EXT_ALINK_SPDIF`.
    pub(crate) fn alink_spdif(&self) -> Option<AlinkSpdif> {
        self.get_ext(tbc::EXT_ALINK_SPDIF, AlinkSpdif::SIZE)
            .ok()
            .and_then(|b| AlinkSpdif::decode(&b))
    }

    /// `EXT_ALINK_CTRL_MSG` IN (a sender's last superframe).
    pub(crate) fn ctrl_msg(&self) -> Option<SuperframeSnapshot> {
        self.get_ext(alink::EXT_ALINK_CTRL_MSG, SuperframeSnapshot::SIZE)
            .ok()
            .and_then(|b| SuperframeSnapshot::decode(&b))
    }

    /// `REQ_SIGGEN_GET_STATUS`, raw.
    pub(crate) fn siggen_status(&self) -> Result<Vec<u8>, u8> {
        self.exec(
            Dir::Get,
            op::REQ_SIGGEN_GET_STATUS,
            0,
            Vec::new(),
            SIGGEN_STATUS_LEN,
        )
    }

    /// `REQ_SIGGEN_SET_CONFIG`.
    pub(crate) fn set_config(&self, cfg: &SiggenConfig) -> Result<(), u8> {
        self.exec(
            Dir::Set,
            op::REQ_SIGGEN_SET_CONFIG,
            0,
            cfg.encode().to_vec(),
            0,
        )
        .map(|_| ())
    }

    /// `SIGGEN_CTL_STOP_NOW`.
    pub(crate) fn stop_now(&self) -> Result<(), u8> {
        self.exec(
            Dir::Get,
            op::REQ_SIGGEN_CONTROL,
            siggen::SIGGEN_CTL_STOP_NOW,
            Vec::new(),
            1,
        )
        .map(|_| ())
    }

    /// `EXT_APPLY_AT_FRAME` OUT of `req` on time base `tb` (byte 12).
    pub(crate) fn apply(&self, req: &ApplyRequest, tb: u8) -> Result<(), u8> {
        if !tbc::apply_form_valid(req, tb) {
            return Err(status::ERROR);
        }
        self.set_ext(EXT_APPLY_AT_FRAME, encode_apply_tb(req, tb).to_vec())
    }

    /// `CANCEL_ALL` on the queue of time base `tb`.
    pub(crate) fn cancel_all(&self, tb: u8) -> Result<(), u8> {
        self.apply(
            &ApplyRequest {
                frame: 0,
                op: ApplyOp::CancelAll,
                arg: 0,
                value: 0,
            },
            tb,
        )
    }

    /// Op 9 `SIGGEN_START` at `t` of time base `tb` (never 2: refused here).
    pub(crate) fn siggen_start_at(&self, t: u64, tb: u8) -> Result<(), u8> {
        if tb == tbc::TB_USB_BUS_FRAME {
            return Err(status::ERROR);
        }
        self.apply(
            &ApplyRequest {
                frame: t,
                op: ApplyOp::SiggenStart,
                arg: 0,
                value: 0,
            },
            tb,
        )
    }

    /// The broadcast `SIGGEN_STOP` now (arg 1) at `t` through a line sender,
    /// keeping its system flags (`flags`: the superframe's, bit 0 the system
    /// mute).
    pub(crate) fn sender_stop(&self, t: u64, flags: u8) -> Result<(), u8> {
        let req = ApplyRequest {
            frame: t,
            op: ApplyOp::SiggenStop,
            arg: tbc::SIGGEN_STOP_NOW,
            value: 0,
        };
        self.set_ext(
            alink::EXT_ALINK_CTRL_MSG,
            req.encode_ctrl_msg(flags & SYSTEM_MUTE_FLAG).to_vec(),
        )
    }

    /// The broadcast `SIGGEN_STOP` now at `t` through an S/PDIF source
    /// (`EXT_ALINK_SPDIF_CTRL`, time base 3).
    pub(crate) fn spdif_stop(&self, t: u64) -> Result<(), u8> {
        let req = ApplyRequest {
            frame: t,
            op: ApplyOp::SiggenStop,
            arg: tbc::SIGGEN_STOP_NOW,
            value: 0,
        };
        self.set_ext(
            tbc::EXT_ALINK_SPDIF_CTRL,
            encode_apply_tb(&req, tbc::TB_SPDIF).to_vec(),
        )
    }

    /// The meter block, raw.
    pub(crate) fn meters(&self) -> Result<Vec<u8>, u8> {
        self.exec(
            Dir::Get,
            op::REQ_GET_STATUS,
            STATUS_METERS,
            Vec::new(),
            METERS_LEN,
        )
    }

    /// The limiter's gain reduction per output, dB; empty without a limiter.
    pub(crate) fn limiter_gr(&self, outputs: u8) -> Vec<f32> {
        let len = u16::from(outputs).saturating_mul(2);
        self.exec(
            Dir::Get,
            op::REQ_LIMITER,
            limiter::LIMITER_GET_METER,
            Vec::new(),
            len,
        )
        .map(|b| {
            b.as_chunks::<2>()
                .0
                .iter()
                .take(usize::from(outputs))
                .map(|c| f32::from(u16::from_le_bytes(*c)) / 100.0)
                .collect()
        })
        .unwrap_or_default()
    }

    /// `REQ_GET_OUTPUT_TYPE` of output slot `slot`: 0 S/PDIF, 1 I2S.
    pub(crate) fn output_type(&self, slot: u8) -> Option<u8> {
        self.exec(
            Dir::Get,
            op::REQ_GET_OUTPUT_TYPE,
            u16::from(slot),
            Vec::new(),
            1,
        )
        .ok()
        .and_then(|b| b.first().copied())
    }
}

/// From the meter block of a unit with `outputs` outputs: each output's peak,
/// dBFS (-120 for silence), and the clip flags. `None` for a short block.
pub(crate) fn decode_meters(b: &[u8], outputs: u8) -> Option<(Vec<f32>, u32)> {
    // Levels (u16 per channel, inputs then outputs), cpu0, cpu1, clip flags
    // (u32), active inputs.
    let n = b.len().checked_sub(7)?.checked_div(2)?;
    let outputs = usize::from(outputs).min(n);
    let first = n.checked_sub(outputs)?;
    let level = |i: usize| -> Option<f32> {
        let at = i.checked_mul(2)?;
        let raw = u16::from_le_bytes(b.get(at..at.checked_add(2)?)?.try_into().ok()?);
        Some(if raw == 0 {
            -120.0
        } else {
            (20.0 * (f32::from(raw) / 32767.0).log10()).max(-120.0)
        })
    };
    let peaks = (first..n).map(level).collect::<Option<Vec<f32>>>()?;
    let at = n.checked_mul(2)?.checked_add(2)?;
    let flags = u32::from_le_bytes(b.get(at..at.checked_add(4)?)?.try_into().ok()?);
    Some((peaks, flags))
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects, clippy::indexing_slicing)]
mod tests_wp_athub {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn wp_athub_meters_take_the_outputs_at_the_end() {
        // Two inputs, two outputs: silence, full scale; flags 5.
        let mut b = Vec::new();
        for v in [100u16, 200, 0, 32767] {
            b.extend_from_slice(&v.to_le_bytes());
        }
        b.extend_from_slice(&[10, 10, 5, 0, 0, 0, 2]);
        let (peaks, flags) = decode_meters(&b, 2).unwrap();
        assert_eq!(peaks[0], -120.0);
        assert!(peaks[1].abs() < 1e-3);
        assert_eq!(flags, 5);
        assert!(decode_meters(&b[..6], 2).is_none());
    }

    proptest! {
        #[test]
        fn proptest_wp_athub_meters_never_panic(b in proptest::collection::vec(any::<u8>(), 0..60), o in any::<u8>()) {
            if let Some((p, _)) = decode_meters(&b, o) {
                prop_assert!(p.len() <= usize::from(o));
                prop_assert!(p.iter().all(|v| v.is_finite()));
            }
        }
    }
}
