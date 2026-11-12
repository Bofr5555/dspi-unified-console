//! A [`FakeDevice`] that plays gains the way released firmware does (WP BULKFIX).
//!
//! The fake itself only stores dB. [`BulkDbDevice`] wraps one and keeps what
//! each preamp, crosspoint and output gain *plays* (the linear factor the
//! firmware computed), as firmware v1.1.6 does:
//! - a bulk SET (`0xA1`, or the last `0xA3` chunk) converts with the Taylor
//!   series of `bulk_params.c` ([`dspi_proto::bulk_gain::firmware_bulk_linear`]),
//!   unless [`BulkDbOptions::fixed`] (FWX's FW2): then with `powf`;
//! - `REQ_SET_PREAMP_CH`, `REQ_SET_PREAMP`, `REQ_SET_MATRIX_ROUTE`,
//!   `REQ_SET_OUTPUT_GAIN`, a preset load and a factory reset convert with
//!   `powf`;
//! - a bulk SET is applied on the main loop, later: the next
//!   [`BulkDbOptions::apply_delay`] requests are served before it, a bulk read
//!   among them stalls (`vendor_bulk_try_acquire` refuses while
//!   `bulk_params_pending`), and a single SET among them is overwritten by the
//!   apply.
//!
//! [`GainProbe`] reports what plays: the worst excess over the stored dB on an
//! audible path since the first bulk apply, and the largest mismatch of any
//! gain now. With `fixed`, `EXT_LIST` also names
//! [`dspi_proto::bulk_gain::EXT_FIRMWARE_FIXES`] and that read answers
//! [`dspi_proto::bulk_gain::FIX_BULK_DB_POWF`].
//!
//! Firmware details not modelled: the soft mute around the apply (taken as no
//! mute at all, the stricter case), NaN refusals and FW2's clamps.

use std::sync::{Arc, Mutex, PoisonError};

use dspi_proto::bulk_gain::{
    EXT_FIRMWARE_FIXES, FIX_BULK_DB_POWF, firmware_bulk_linear, output_enabled,
};
use dspi_proto::generated::{self, ext as gext, notify as nfy, opcodes as op, wire as w};
use dspi_proto::wire::BulkPacket;

use super::{FakeDevice, FakeRemote};
use crate::{
    DeviceDescriptor, NotificationSource, Result, Transport, TransportCaps, TransportError,
};

const INS: usize = w::WIRE_MAX_INPUT_CHANNELS as usize;
const OUTS: usize = w::WIRE_MAX_OUTPUT_CHANNELS as usize;

/// How the wrapped firmware behaves.
#[derive(Debug, Clone, Copy, Default)]
pub struct BulkDbOptions {
    /// FWX's FW2: the bulk SET converts with `powf`, and the fix is reported.
    pub fixed: bool,
    /// Requests served after a bulk SET before it is applied.
    pub apply_delay: usize,
}

/// A [`FakeDevice`] whose gains play as the firmware's do. See the module docs.
pub struct BulkDbDevice {
    inner: FakeDevice,
    model: Arc<Mutex<Model>>,
}

/// A handle on what a [`BulkDbDevice`] plays, usable after the device moved
/// into a core or a hub.
#[derive(Clone)]
pub struct GainProbe {
    model: Arc<Mutex<Model>>,
}

/// Linear factors, as the firmware holds them.
#[derive(Debug, Clone)]
struct Played {
    preamp: [f32; INS],
    xp: [[f32; OUTS]; INS],
    out: [f32; OUTS],
}

struct Model {
    options: BulkDbOptions,
    remote: FakeRemote,
    played: Played,
    /// A received bulk SET and the requests still served before its apply.
    pending: Option<(Vec<u8>, usize)>,
    /// A chunked bulk SET being received.
    chunks: Option<Vec<u8>>,
    bulk_applies: usize,
    /// Every OUT request's opcode, in order.
    sets: Vec<u8>,
    /// The largest dB by which an audible gain played above its stored value
    /// since the first bulk apply.
    worst_excess_db: f32,
}

fn powf_linear(db: f32) -> f32 {
    10f32.powf(db / 20.0)
}

fn f32_at(b: &[u8], at: usize) -> f32 {
    b.get(at..at + 4)
        .and_then(|s| <[u8; 4]>::try_from(s).ok())
        .map_or(0.0, f32::from_le_bytes)
}

fn preamp_at(i: usize) -> usize {
    generated::OFF_PREAMP + i * 4
}
fn xp_at(i: usize, o: usize) -> usize {
    generated::OFF_CROSSPOINTS + (i * OUTS + o) * 8
}
fn out_at(o: usize) -> usize {
    generated::OFF_OUTPUTS + o * 12
}

/// The header's channel counts, capped at the wire's maximum.
fn counts(image: &[u8]) -> (usize, usize) {
    let n = |k: usize| usize::from(image.get(k).copied().unwrap_or(0));
    (n(4).min(INS), n(3).min(OUTS))
}

impl Played {
    /// Every gain of `image` through `f`.
    fn from(image: &[u8], f: fn(f32) -> f32) -> Self {
        let mut p = Played {
            preamp: [1.0; INS],
            xp: [[1.0; OUTS]; INS],
            out: [1.0; OUTS],
        };
        for i in 0..INS {
            p.preamp[i] = f(f32_at(image, preamp_at(i)));
            for o in 0..OUTS {
                p.xp[i][o] = f(f32_at(image, xp_at(i, o) + 4));
            }
        }
        for o in 0..OUTS {
            p.out[o] = f(f32_at(image, out_at(o) + 4));
        }
        p
    }
}

fn ratio_db(played: f32, stored_db: f32) -> f32 {
    20.0 * (played / powf_linear(stored_db)).log10()
}

impl Model {
    fn image(&self) -> Vec<u8> {
        self.remote.image()
    }

    /// Run the pending apply if its time has come (before this request).
    fn tick(&mut self) {
        let Some((image, left)) = self.pending.take() else {
            return;
        };
        if left > 0 {
            self.pending = Some((image, left - 1));
            return;
        }
        // `bulk_params_apply` refuses another version or size and applies
        // nothing; else the image is stored and BULK_INVALIDATED sent.
        if BulkPacket::decode(image.clone()).is_ok() {
            self.remote
                .load_image(image.clone(), nfy::PARAM_SRC_BULK_SET as u8);
            let f = if self.options.fixed {
                powf_linear
            } else {
                firmware_bulk_linear
            };
            self.played = Played::from(&image, f);
            self.bulk_applies += 1;
        }
    }

    /// Track the worst excess on an audible path.
    fn audit(&mut self) {
        if self.bulk_applies == 0 {
            return;
        }
        let image = self.image();
        let (ins, outs) = counts(&image);
        let mut worst = self.worst_excess_db;
        for o in 0..outs {
            let muted = image.get(out_at(o) + 1).is_some_and(|m| *m != 0);
            if !output_enabled(&image, o) || muted {
                continue;
            }
            worst = worst.max(ratio_db(self.played.out[o], f32_at(&image, out_at(o) + 4)));
            for i in 0..ins {
                if image.get(xp_at(i, o)).is_some_and(|e| *e != 0) {
                    worst = worst.max(ratio_db(
                        self.played.xp[i][o],
                        f32_at(&image, xp_at(i, o) + 4),
                    ));
                    worst = worst.max(ratio_db(
                        self.played.preamp[i],
                        f32_at(&image, preamp_at(i)),
                    ));
                }
            }
        }
        self.worst_excess_db = worst;
    }

    /// A single SET the inner fake accepted: what it converts with `powf`.
    fn note_set(&mut self, opcode: u8, value: u16, data: &[u8]) {
        let image = self.image();
        let (ins, outs) = counts(&image);
        let db = || f32_at(data, 0);
        match opcode {
            op::REQ_SET_PREAMP_CH if usize::from(value) < ins => {
                self.played.preamp[usize::from(value)] = powf_linear(db());
            }
            op::REQ_SET_PREAMP => {
                for i in 0..ins {
                    self.played.preamp[i] = powf_linear(db());
                }
            }
            op::REQ_SET_MATRIX_ROUTE => {
                if let [i, o, _, _, ..] = *data
                    && usize::from(i) < ins
                    && usize::from(o) < outs
                {
                    self.played.xp[usize::from(i)][usize::from(o)] = powf_linear(f32_at(data, 4));
                }
            }
            op::REQ_SET_OUTPUT_GAIN if usize::from(value) < outs => {
                self.played.out[usize::from(value)] = powf_linear(db());
            }
            _ => self.note_reload(opcode),
        }
    }

    /// An OUT request, as the firmware takes it.
    fn set(&mut self, inner: &mut FakeDevice, opcode: u8, value: u16, data: &[u8]) -> Result<()> {
        match opcode {
            op::REQ_SET_ALL_PARAMS => {
                self.pending = Some((data.to_vec(), self.options.apply_delay));
                Ok(())
            }
            op::REQ_SET_ALL_PARAMS_CHUNK => {
                let offset = usize::from(value);
                if offset == 0 {
                    self.chunks = Some(Vec::new());
                }
                match self.chunks.as_mut() {
                    Some(c) if c.len() == offset => c.extend_from_slice(data),
                    _ => {
                        self.chunks = None;
                        return Err(TransportError::Stalled { opcode });
                    }
                }
                if self
                    .chunks
                    .as_ref()
                    .is_some_and(|c| c.len() >= generated::BULK_SIZE)
                {
                    let image = self.chunks.take().unwrap_or_default();
                    self.pending = Some((image, self.options.apply_delay));
                }
                Ok(())
            }
            _ => {
                self.chunks = None;
                inner.control_out(opcode, value, data)?;
                self.note_set(opcode, value, data);
                Ok(())
            }
        }
    }

    /// A preset load or factory reset (write-as-read opcodes): every gain is
    /// converted again with `powf` (`flash_storage.c`).
    fn note_reload(&mut self, opcode: u8) {
        if matches!(opcode, op::REQ_PRESET_LOAD | op::REQ_FACTORY_RESET) {
            self.played = Played::from(&self.image(), powf_linear);
        }
    }
}

impl BulkDbDevice {
    /// Wrap `inner`; what it holds now plays right (as after a boot).
    pub fn new(inner: FakeDevice, options: BulkDbOptions) -> Self {
        let remote = inner.remote();
        let played = Played::from(&remote.image(), powf_linear);
        let model = Model {
            options,
            remote,
            played,
            pending: None,
            chunks: None,
            bulk_applies: 0,
            sets: Vec::new(),
            worst_excess_db: f32::NEG_INFINITY,
        };
        Self {
            inner,
            model: Arc::new(Mutex::new(model)),
        }
    }

    /// The handle on what plays.
    pub fn probe(&self) -> GainProbe {
        GainProbe {
            model: Arc::clone(&self.model),
        }
    }

    /// The wrapped fake's remote.
    pub fn remote(&self) -> FakeRemote {
        self.inner.remote()
    }

    fn with<T>(&mut self, f: impl FnOnce(&mut Model, &mut FakeDevice) -> T) -> T {
        let mut m = self.model.lock().unwrap_or_else(PoisonError::into_inner);
        m.tick();
        let r = f(&mut m, &mut self.inner);
        m.audit();
        r
    }

    fn get(&mut self, opcode: u8, value: u16, len: u16, upto: bool) -> Result<Vec<u8>> {
        self.with(|m, inner| {
            let bulk_read = matches!(
                opcode,
                op::REQ_GET_ALL_PARAMS | op::REQ_GET_ALL_PARAMS_CHUNK
            );
            if bulk_read && m.pending.is_some() {
                return Err(TransportError::Stalled { opcode });
            }
            let read = |inner: &mut FakeDevice| {
                if upto {
                    inner.control_in_upto(opcode, value, len)
                } else {
                    inner.control_in(opcode, value, len)
                }
            };
            if opcode == op::REQ_EXTENSION && m.options.fixed {
                if value == gext::EXT_LIST {
                    let mut ids = read(inner).unwrap_or_default();
                    ids.extend_from_slice(&EXT_FIRMWARE_FIXES.to_le_bytes());
                    ids.truncate(usize::from(len));
                    return Ok(ids);
                }
                if value == EXT_FIRMWARE_FIXES {
                    let mut b = FIX_BULK_DB_POWF.to_le_bytes().to_vec();
                    b.truncate(usize::from(len));
                    return Ok(b);
                }
            }
            let r = read(inner);
            if r.is_ok() {
                m.note_reload(opcode);
            }
            r
        })
    }
}

impl GainProbe {
    fn model(&self) -> std::sync::MutexGuard<'_, Model> {
        self.model.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// How many bulk SETs have been applied.
    pub fn bulk_applies(&self) -> usize {
        self.model().bulk_applies
    }

    /// Every OUT request's opcode so far, in order.
    pub fn sets(&self) -> Vec<u8> {
        self.model().sets.clone()
    }

    /// The most an audible gain has played above its stored dB since the
    /// first bulk apply; `-inf` before one, or with nothing audible.
    pub fn worst_excess_db(&self) -> f32 {
        self.model().worst_excess_db
    }

    /// The largest `|played − stored|` in dB over every preamp, crosspoint
    /// and output gain of the device's channels, muted or not.
    pub fn mismatch_db(&self) -> f32 {
        let m = self.model();
        let image = m.image();
        let (ins, outs) = counts(&image);
        let mut worst = 0f32;
        for i in 0..ins {
            worst = worst.max(ratio_db(m.played.preamp[i], f32_at(&image, preamp_at(i))).abs());
            for o in 0..outs {
                worst =
                    worst.max(ratio_db(m.played.xp[i][o], f32_at(&image, xp_at(i, o) + 4)).abs());
            }
        }
        for o in 0..outs {
            worst = worst.max(ratio_db(m.played.out[o], f32_at(&image, out_at(o) + 4)).abs());
        }
        worst
    }

    /// What crosspoint `input` → `output` plays, in dB.
    pub fn crosspoint_db(&self, input: usize, output: usize) -> f32 {
        let m = self.model();
        20.0 * m.played.xp[input.min(INS - 1)][output.min(OUTS - 1)].log10()
    }
}

impl Transport for BulkDbDevice {
    fn control_in(&mut self, opcode: u8, value: u16, len: u16) -> Result<Vec<u8>> {
        self.get(opcode, value, len, false)
    }

    fn control_in_upto(&mut self, opcode: u8, value: u16, max_len: u16) -> Result<Vec<u8>> {
        self.get(opcode, value, max_len, true)
    }

    fn control_out(&mut self, opcode: u8, value: u16, data: &[u8]) -> Result<()> {
        self.with(|m, inner| {
            m.sets.push(opcode);
            m.set(inner, opcode, value, data)
        })
    }

    fn descriptor(&self) -> &DeviceDescriptor {
        self.inner.descriptor()
    }

    fn max_transfer(&self) -> usize {
        self.inner.max_transfer()
    }

    fn notifications(&self) -> Option<Box<dyn NotificationSource>> {
        self.inner.notifications()
    }

    fn caps(&self) -> TransportCaps {
        self.inner.caps()
    }
}

#[cfg(test)]
mod tests_wp_bulkfix {
    use super::*;

    fn put(b: &mut [u8], at: usize, v: f32) {
        b[at..at + 4].copy_from_slice(&v.to_le_bytes());
    }

    #[test]
    fn wp_bulkfix_the_model_plays_a_bulk_cut_loud_and_a_single_set_right() {
        let mut d = BulkDbDevice::new(FakeDevice::demo(), BulkDbOptions::default());
        let probe = d.probe();
        let mut img = d.remote().image();
        put(&mut img, xp_at(0, 0) + 4, -60.0);
        d.control_out(op::REQ_SET_ALL_PARAMS, 0, &img).unwrap();
        // Applied on the next request.
        let _ = d.control_in(op::REQ_GET_PLATFORM, 0, 7);
        assert_eq!(probe.bulk_applies(), 1);
        assert!((probe.crosspoint_db(0, 0) - 35.25).abs() < 0.1);
        assert!(probe.mismatch_db() > 90.0);
        let pkt = [0u8, 0, 1, 0]
            .into_iter()
            .chain((-60.0f32).to_le_bytes())
            .collect::<Vec<_>>();
        d.control_out(op::REQ_SET_MATRIX_ROUTE, 0, &pkt).unwrap();
        assert!((probe.crosspoint_db(0, 0) + 60.0).abs() < 0.01);
    }

    #[test]
    fn wp_bulkfix_a_pending_apply_stalls_bulk_reads_and_overwrites_single_sets() {
        let opts = BulkDbOptions {
            apply_delay: 2,
            ..BulkDbOptions::default()
        };
        let mut d = BulkDbDevice::new(FakeDevice::demo(), opts);
        let probe = d.probe();
        let mut img = d.remote().image();
        put(&mut img, out_at(0) + 4, -40.0);
        d.control_out(op::REQ_SET_ALL_PARAMS, 0, &img).unwrap();
        // Too early: this single SET lands before the apply...
        d.control_out(op::REQ_SET_OUTPUT_GAIN, 0, &(-40.0f32).to_le_bytes())
            .unwrap();
        assert!(matches!(
            d.control_in_upto(op::REQ_GET_ALL_PARAMS_CHUNK, 0, 64),
            Err(TransportError::Stalled { .. })
        ));
        // ...and the apply overwrites it with the series' value.
        assert!(
            d.control_in_upto(op::REQ_GET_ALL_PARAMS_CHUNK, 0, 64)
                .is_ok()
        );
        assert_eq!(probe.bulk_applies(), 1);
        assert!(probe.mismatch_db() > 50.0);
        assert!(probe.worst_excess_db() > 50.0, "output 0 is audible");
    }

    #[test]
    fn wp_bulkfix_fixed_firmware_plays_a_bulk_set_right_and_says_so() {
        let opts = BulkDbOptions {
            fixed: true,
            ..BulkDbOptions::default()
        };
        let mut d = BulkDbDevice::new(FakeDevice::demo(), opts);
        let probe = d.probe();
        let list = d
            .control_in_upto(op::REQ_EXTENSION, gext::EXT_LIST, 64)
            .unwrap();
        assert!(
            list.chunks(2)
                .any(|c| c == EXT_FIRMWARE_FIXES.to_le_bytes())
        );
        let mut img = d.remote().image();
        put(&mut img, preamp_at(0), -20.0);
        d.control_out(op::REQ_SET_ALL_PARAMS, 0, &img).unwrap();
        let fixes = d
            .control_in(op::REQ_EXTENSION, EXT_FIRMWARE_FIXES, 4)
            .unwrap();
        assert_eq!(
            u32::from_le_bytes(fixes.try_into().unwrap()),
            FIX_BULK_DB_POWF
        );
        assert_eq!(probe.bulk_applies(), 1);
        assert!(probe.mismatch_db() < 0.01);
    }
}
