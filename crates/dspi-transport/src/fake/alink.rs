//! WP SW1: a receiving unit of the audio link, answered when
//! [`super::FakeOptions::alink`] is on, as the fork's
//! `extension_channel_spec.md` 2.8 (WP LK7) specifies a receiver build, so
//! clients and the hub can be built without a sender, a fibre or LK5/LK6
//! firmware.
//!
//! The model: the spec's example receiver (GP0, L1-L3, 64 channels,
//! `PHASE_LOCK | APPLY_AT_FRAME | LOS_PIN`), hearing L1
//! at 48 kHz, 8 channels, from sender 0x42, locked from the start. The frame
//! counter runs with the fake's clock (48 frames per ms), extrapolated in
//! holdover as the firmware does. Every receiver sub-command is listed and
//! answered with the spec's refusals (a full apply queue, an apply without a
//! lock, an out-of-range delay, a bad pin or flag); `EXT_ALINK_TX` is not.
//! `EXT_NOTIFY_FEATURES` announces bits 2 and 3; 0xE1 goes out on a loss, a
//! lock, a re-anchor (a new phase setting while locked too) and a disable,
//! 0xE2 for every completed op. Input source 7 is selectable.
//!
//! [`EXT_FAKE_ALINK_SIM`] plays the alink's side: lose it, lock it, re-anchor,
//! count a bank error. Apply-at-frame ops are queued (four at most) and run
//! when the counter passes their frame, checked at every alink request; one
//! whose frame has passed runs at once and reports "late". Their effect on
//! the audio (a mute, a preset load) is not modelled, only their results.
//! Not modelled either: the 250 ms rate limit on 0xE1, the phase servo (an
//! anchor completes at once), the pin table check against the live
//! configuration; the alink input runs whatever the input source (the
//! firmware runs it only under source 7).
//!
//! WP FIXS1: `d_local_min..d_local_max` follow the output latency option at
//! 48 kHz as the firmware works them out (spec 2.8.1: 81..431 for the 1 ms
//! option, 257..399 for the default 4 ms one), and the unit starts at an alink
//! delay inside them. The phase error is a small one that drifts slowly
//! while locked (0.01..0.05 frames, a function of the fake's clock) and
//! grows at the VCXO's offset in holdover.
//!
//! WP SW2: [`AlinkSetup`] makes the fake another place in a system: a
//! receiver behind re-emitters (`hops`), a repeater or fan-out with its
//! `hop_delay_frames` (STATUS offset 50), a unit that hears another sender,
//! or the sender build itself (role 2: [`alink::SENDER_IDS`], the counter
//! running from its own start, `EXT_ALINK_CTRL_MSG` OUT with 2.8.9's lead and
//! one-at-a-time rules, its IN showing the pending apply until the counter
//! passes it, `EXT_ALINK_TX` kept as written). An id the role does not list
//! stalls. The superframe does not reach other fakes: a test checks what the
//! sender was asked.
#![warn(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

use dspi_proto::ext::alink::{
    self, AlinkConfig, AlinkEventReason, AlinkInfo, AlinkPhase, AlinkRole, AlinkSlots, AlinkState,
    AlinkStateEvent, AlinkStatus, AlinkTx, ApplyOp, ApplyRequest, ApplyResult, ApplyResultEvent,
    ApplyStatus, FrameCounter, SuperframeSnapshot, feature,
};
use dspi_proto::generated::notify as nfy;

use super::State;
use crate::clock::Instant;

/// `REQ_EXTENSION` OUT sub-command only a [`super::FakeDevice`] answers, as
/// [`super::EXT_FAKE_VREG_SIM`] (WP SW1): one byte, what happens to the
/// audio link. [`ALINK_SIM_LOSE`], [`ALINK_SIM_LOCK`], [`ALINK_SIM_REANCHOR`] or
/// [`ALINK_SIM_BANK_ERROR`]; anything else, or the fake without
/// [`super::FakeOptions::alink`], stalls.
pub const EXT_FAKE_ALINK_SIM: u16 = 0xFFF7;
/// [`EXT_FAKE_ALINK_SIM`]: the fibre is pulled (LOS). Holdover, event 0xE1.
pub const ALINK_SIM_LOSE: u8 = 0;
/// [`EXT_FAKE_ALINK_SIM`]: the alink is back (or locks). Event 0xE1 `LOCKED`.
pub const ALINK_SIM_LOCK: u8 = 1;
/// [`EXT_FAKE_ALINK_SIM`]: back with too large a phase error: event 0xE1
/// `REANCHOR`.
pub const ALINK_SIM_REANCHOR: u8 = 2;
/// [`EXT_FAKE_ALINK_SIM`]: one audio bank failed its CRC.
pub const ALINK_SIM_BANK_ERROR: u8 = 3;

/// The sender id the fake hears.
const SENDER_ID: u8 = 0x42;
/// Frames per millisecond at 48 kHz.
const FRAMES_PER_MS: u64 = 48;
/// The alink frame at the fake's start, so the counter is plainly not zero.
const FRAME_BASE: u64 = 1_000_000;
/// The apply queue's depth (2.8.7).
const APPLY_QUEUE: usize = 4;
/// `EXT_ALINK_DIAG` layout version.
const DIAG_VERSION: u8 = 1;

// -- WP SW2 --
/// Where the fake sits on the alink ([`super::FakeDevice::demo_alink_as`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AlinkSetup {
    /// `Receiver` (the default), `Repeater`, `Sender` or `FanOut`.
    pub role: AlinkRole,
    /// Re-emitters between the sender and this unit (STATUS offset 4).
    pub hops: u8,
    /// A re-emitter's hop delay (STATUS offset 50), reported while locked.
    pub hop_delay_frames: u8,
    /// The sender the unit hears, or a sender's own id.
    pub sender_id: u8,
    /// `EXT_ALINK_INFO.d_local_min` if set; `None` (the default): from the
    /// output latency option, as the firmware (WP FIXS1).
    pub d_local_min: Option<u16>,
}

impl Default for AlinkSetup {
    fn default() -> Self {
        Self {
            role: AlinkRole::Receiver,
            hops: 0,
            hop_delay_frames: 0,
            sender_id: SENDER_ID,
            d_local_min: None,
        }
    }
}

impl AlinkSetup {
    fn is_sender_build(&self) -> bool {
        matches!(self.role, AlinkRole::Sender | AlinkRole::FanOut)
    }
}

// -- WP FIXS1 --
/// The output rings' writable span (`output_latency_ring_span`: 512 - 64).
const RING_SPAN: u32 = 448;
/// The firmware's `ALINK_PHASE_DECODE_MARGIN` (alink/alink_phase.h).
const DECODE_MARGIN: u32 = 16;
/// The fake's start audio link delay: inside the default 4 ms option's 257..399, as
/// a unit an installer has aligned.
const D_LOCAL_START: u16 = 300;
/// The fake's VCXO correction, ppm (STATUS `vcxo_ppm`); a holdover's phase
/// error grows at it.
const VCXO_PPM: f32 = 1.5;
/// The rate the fake's alink runs at.
const ALINK_RATE_HZ: u32 = 48_000;

/// `d_local_min`, `d_local_max` for an option's `(latency_us, batch_us)` at
/// 48 kHz: `alink_phase_limits` with output_latency.c's rounding.
fn d_local_limits((latency_us, batch_us): (u16, u16)) -> (u16, u16) {
    let rate = ALINK_RATE_HZ;
    let frames = |us: u16, round: u32| {
        (u32::from(us).saturating_mul(rate).saturating_add(round)) / 1_000_000
    };
    let cap = frames(batch_us, 999_999).saturating_add(1);
    let ceiling = RING_SPAN.saturating_sub(cap);
    let target = frames(latency_us, 500_000).min(ceiling);
    let lo = target.saturating_add(cap).saturating_add(DECODE_MARGIN);
    let to16 = |v: u32| u16::try_from(v).unwrap_or(u16::MAX);
    (to16(lo), to16(ceiling))
}

/// A locked unit's phase error, frames: 0.03 ± 0.02, one period every 30 s
/// of the fake's clock.
fn locked_phase_error(since_start_s: f32) -> f32 {
    0.03 + 0.02 * (since_start_s * std::f32::consts::TAU / 30.0).sin()
}

/// The alink's state on the fake.
#[derive(Debug)]
pub(super) struct Alink {
    state: AlinkState,
    config: AlinkConfig,
    slots: AlinkSlots,
    phase: AlinkPhase,
    lock_count: u32,
    loss_count: u32,
    crc16_errors: u32,
    queue: Vec<ApplyRequest>,
    last: ApplyStatus,
    // -- WP SW2 --
    pub(super) setup: AlinkSetup,
    /// A sender's pending superframe apply.
    ctrl: Option<ApplyRequest>,
    /// A sender's `EXT_ALINK_TX`. Its flags bit 0 is the system mute, the one
    /// byte `EXT_ALINK_CTRL_MSG`'s `system_flags` writes too (fixup sw2x: the
    /// firmware's `snd.cfg.flags`).
    tx: AlinkTx,
    // -- WP FIXS1 --
    /// When a holdover began, and the phase error then.
    held: Option<(Instant, f32)>,
}

impl Default for Alink {
    fn default() -> Self {
        Self {
            state: AlinkState::Locked,
            config: AlinkConfig {
                enable: true,
                rx_pin: 0xFF,
                class: 0,
                tx_pin: 0xFF,
                los_pin: 0xFF,
                txdis_pin: 0xFF,
                flags: 0b11,
            },
            slots: AlinkSlots::IDENTITY,
            phase: AlinkPhase {
                mode: alink::PHASE_MODE_LOCK,
                d_local: D_LOCAL_START,
                hops_offset: 0,
                flags: 0,
                state: 2,
            },
            lock_count: 1,
            loss_count: 0,
            crc16_errors: 0,
            queue: Vec::new(),
            last: ApplyStatus {
                pending: 0,
                last_op: ApplyOp::CancelAll,
                last_result: ApplyResult::Applied,
                last_frame: 0,
            },
            // -- WP SW2 --
            setup: AlinkSetup::default(),
            ctrl: None,
            tx: AlinkTx {
                alink_mask: 0,
                class: 1,
                channels: 8,
                rate_code: 1,
                test_pattern: 0,
                slot_map_id: 0,
                sync_pin: 0xFF,
                txdis_mask: 0x00FF,
                los_mask: 0,
                usb_alt: 1,
                flags: 0,
            },
            held: None,
        }
    }
}

/// The alink input's build default, GP0 (2.8.3).
const RX_PIN_DEFAULT: u8 = 0;

/// The receiver's `EXT_ALINK_INFO`: 2.8.1's example, `rx_pin` following
/// `config` (its `0xFF` resolved to the build default, 2.8.1).
///
/// WP SW2: for the fake's place on the alink; a sender build reports its alinks
/// and no slots or delay range (2.8.1).
fn info_as(config: &AlinkConfig, setup: &AlinkSetup, latency: (u16, u16)) -> AlinkInfo {
    if setup.is_sender_build() {
        return AlinkInfo {
            role: setup.role,
            protocol_version: alink::ALINK_PROTOCOL_VERSION,
            class_mask: 0b111,
            max_alinks: 8,
            max_channels: alink::ALINK_MAX_CHANNELS,
            slots_per_unit: 0,
            flags: 0,
            rx_pin: 20,
            tx_pin: 0xFF,
            feature_bits: feature::APPLY_AT_FRAME | feature::FAN_OUT | feature::TEST_PATTERNS,
            d_local_min: 0,
            d_local_max: 0,
        };
    }
    let repeater = setup.role == AlinkRole::Repeater;
    // -- WP FIXS1 --
    let limits = d_local_limits(latency);
    AlinkInfo {
        role: setup.role,
        protocol_version: alink::ALINK_PROTOCOL_VERSION,
        class_mask: 0b111,
        max_alinks: 1,
        max_channels: alink::ALINK_MAX_CHANNELS,
        slots_per_unit: 8,
        flags: 0,
        rx_pin: if config.rx_pin == 0xFF {
            RX_PIN_DEFAULT
        } else {
            config.rx_pin
        },
        tx_pin: if repeater { 22 } else { 0xFF },
        feature_bits: feature::PHASE_LOCK
            | feature::APPLY_AT_FRAME
            | feature::LOS_PIN
            | if repeater { feature::REPEATER } else { 0 },
        d_local_min: setup.d_local_min.unwrap_or(limits.0),
        d_local_max: limits.1,
    }
}

/// A header GPIO on a Pico 2 (2.8.3), or 0xFF.
fn pin_ok(p: u8) -> bool {
    p == 0xFF || p <= 22 || (26..=28).contains(&p)
}

/// 2.8.3's refusals, but the live pin table's.
fn config_ok(c: &AlinkConfig, raw: &[u8]) -> bool {
    let pins = [c.rx_pin, c.tx_pin, c.los_pin, c.txdis_pin];
    let set: Vec<u8> = pins.iter().copied().filter(|p| *p != 0xFF).collect();
    let distinct = set
        .iter()
        .enumerate()
        .all(|(i, p)| !set.iter().skip(i.saturating_add(1)).any(|q| q == p));
    raw.len() == AlinkConfig::SIZE
        && raw.first().is_some_and(|e| *e <= 1)
        && raw.get(7) == Some(&0)
        && c.class <= 3
        && c.flags & !0b11 == 0
        && pins.iter().all(|p| pin_ok(*p))
        && distinct
        // No REPEATER in this build.
        && c.tx_pin == 0xFF
}

fn low32(f: u64) -> u32 {
    (f & u64::from(u32::MAX)) as u32
}

/// An alink packet: the v2 header, then `payload`.
fn packet(id: u8, payload: &[u8]) -> Vec<u8> {
    let mut p = vec![nfy::NOTIFY_V2_VERSION as u8, id, 0, 0];
    p.extend_from_slice(payload);
    p
}

impl State {
    /// The alink frame now: from the fake's start, while locked or in
    /// holdover (extrapolated); 0 otherwise.
    fn alink_frame(&self) -> u64 {
        // -- WP SW2 -- the master sender's counter always runs.
        let master = self.alink.setup.role == AlinkRole::Sender;
        if !master && !matches!(self.alink.state, AlinkState::Locked | AlinkState::Holdover) {
            return 0;
        }
        let ms = Instant::now()
            .saturating_duration_since(self.started)
            .as_millis();
        let frames = u64::try_from(ms)
            .unwrap_or(u64::MAX)
            .saturating_mul(FRAMES_PER_MS);
        FRAME_BASE.saturating_add(frames)
    }

    fn alink_status(&self) -> AlinkStatus {
        let l = &self.alink;
        let locked = l.state == AlinkState::Locked;
        // -- WP SW2 -- a master sender has no input: state 0 (2.8.2).
        let master = l.setup.role == AlinkRole::Sender;
        let reemits = matches!(l.setup.role, AlinkRole::Repeater | AlinkRole::FanOut);
        AlinkStatus {
            state: if master { AlinkState::Off } else { l.state },
            class: u8::from(locked),
            rate_code: if locked { 1 } else { 0xFF },
            channels: if locked { 8 } else { 0 },
            hops: l.setup.hops,
            los_pin: if l.config.los_pin == 0xFF {
                0xFF
            } else {
                u8::from(!locked)
            },
            sender_id: l.setup.sender_id,
            flags: 0b1000, // bit 3: the sender has a live USB stream
            frame: self.alink_frame(),
            lock_count: l.lock_count,
            loss_count: l.loss_count,
            crc8_errors: 0,
            crc16_errors: l.crc16_errors,
            bit_slips: 0,
            superframe_errors: 0,
            vcxo_ppm: VCXO_PPM,
            phase_error_frames: self.alink_phase_error(),
            fill_frames: l.phase.d_local,
            // -- WP SW2 --
            hop_delay_frames: if reemits && locked {
                l.setup.hop_delay_frames
            } else {
                0
            },
        }
    }

    // -- WP FIXS1 --
    /// STATUS `phase_error_frames`: small and slowly varying while locked,
    /// growing at the VCXO's offset in holdover, 0 otherwise.
    fn alink_phase_error(&self) -> f32 {
        let l = &self.alink;
        if l.setup.is_sender_build() || l.phase.mode != alink::PHASE_MODE_LOCK {
            return 0.0;
        }
        let now = Instant::now();
        match (l.state, l.held) {
            (AlinkState::Locked, _) => {
                locked_phase_error(now.saturating_duration_since(self.started).as_secs_f32())
            }
            (AlinkState::Holdover, Some((at, e0))) => {
                let s = now.saturating_duration_since(at).as_secs_f32();
                e0 + s * VCXO_PPM * 1e-6 * ALINK_RATE_HZ as f32
            }
            _ => 0.0,
        }
    }

    /// The alink's `EXT_ALINK_INFO` now.
    fn alink_info(&self) -> AlinkInfo {
        let l = &self.alink;
        info_as(&l.config, &l.setup, self.latency_for_alink())
    }

    /// `EXT_ALINK_PHASE` as read: `D_BELOW_MIN` and a free state while the
    /// delay is below the option's minimum (2.8.5).
    fn alink_phase_read(&self) -> AlinkPhase {
        let p = self.alink.phase;
        let info = self.alink_info();
        let below = p.effective() < i32::from(info.d_local_min);
        if p.mode == alink::PHASE_MODE_LOCK && below {
            return AlinkPhase {
                flags: alink::PHASE_FLAG_D_BELOW_MIN,
                state: 0,
                ..p
            };
        }
        p
    }

    /// The alink sub-commands the fake lists, with the option on.
    pub(super) fn alink_ids(&self) -> Vec<u16> {
        if !self.options.alink {
            return Vec::new();
        }
        // -- WP SW2 --
        if self.alink.setup.is_sender_build() {
            return alink::SENDER_IDS.to_vec();
        }
        alink::RECEIVER_IDS.to_vec()
    }

    /// Whether input source 7 is selectable.
    pub(super) fn alink_input(&self) -> bool {
        self.options.alink && !self.alink.setup.is_sender_build()
    }

    /// IN: `Some(reply)` for an alink sub-command (`Some(None)` stalls), `None`
    /// for anything else.
    pub(super) fn get_alink(&mut self, value: u16) -> Option<Option<Vec<u8>>> {
        if !self.options.alink || !(alink::EXT_ALINK_INFO..=alink::EXT_ALINK_DIAG).contains(&value)
        {
            return None;
        }
        self.run_due_ops();
        // -- WP SW2 -- an id this build does not list stalls (2.8).
        if !self.alink_ids().contains(&value) {
            return Some(None);
        }
        if self.alink.setup.is_sender_build()
            && let Some(reply) = self.get_sender(value)
        {
            return Some(Some(reply));
        }
        let l = &self.alink;
        Some(match value {
            alink::EXT_ALINK_INFO => Some(self.alink_info().encode().to_vec()),
            alink::EXT_ALINK_STATUS => Some(self.alink_status().encode().to_vec()),
            alink::EXT_ALINK_CONFIG => Some(l.config.encode().to_vec()),
            alink::EXT_ALINK_SLOTS => Some(l.slots.0.to_vec()),
            alink::EXT_ALINK_PHASE => Some(self.alink_phase_read().encode().to_vec()),
            alink::EXT_FRAME_COUNTER => {
                let us = Instant::now()
                    .saturating_duration_since(self.started)
                    .as_micros();
                let frame = self.alink_frame();
                Some(
                    FrameCounter {
                        frame,
                        // Both wrap, as the firmware's do.
                        time_us: (us & u128::from(u32::MAX)) as u32,
                        output_frames: low32(frame.saturating_sub(FRAME_BASE)),
                    }
                    .encode()
                    .to_vec(),
                )
            }
            alink::EXT_APPLY_AT_FRAME => {
                let pending = u8::try_from(l.queue.len()).unwrap_or(u8::MAX);
                Some(ApplyStatus { pending, ..l.last }.encode().to_vec())
            }
            alink::EXT_ALINK_CTRL_MSG => Some(
                SuperframeSnapshot {
                    flags: 0b1000,
                    rate_code: 1,
                    channels: 8,
                    class: 1,
                    hops: l.setup.hops,
                    sender_id: l.setup.sender_id,
                    apply: ApplyRequest {
                        frame: 0,
                        op: ApplyOp::CancelAll,
                        arg: 0,
                        value: 0,
                    },
                    frame_low: low32(self.alink_frame() & !31),
                }
                .encode()
                .to_vec(),
            ),
            alink::EXT_ALINK_DIAG => {
                let mut d = vec![0u8; alink::ALINK_DIAG_SIZE];
                if let Some(v) = d.first_mut() {
                    *v = DIAG_VERSION;
                }
                // -- WP SW3 -- a receiver's trim scan at alink lock (offsets 32-39):
                // done, clean from -3 to +4, running at 0, as a healthy alink.
                if l.setup.role.receives()
                    && l.state != AlinkState::Off
                    && let Some(scan) = d.get_mut(32..40)
                {
                    scan.copy_from_slice(&[0xFD, 4, 0, 2, 0xC0, 0x3F, 0, 0]);
                }
                Some(d)
            }
            // EXT_ALINK_TX: a sender's.
            _ => None,
        })
    }

    /// OUT: `Some(accepted)` for an alink sub-command or [`EXT_FAKE_ALINK_SIM`],
    /// `None` for anything else.
    pub(super) fn set_alink(&mut self, value: u16, data: &[u8]) -> Option<bool> {
        if value == EXT_FAKE_ALINK_SIM {
            return Some(match *data {
                [op] if self.options.alink => self.alink_sim(op),
                _ => false,
            });
        }
        if !self.options.alink || !(alink::EXT_ALINK_INFO..=alink::EXT_ALINK_DIAG).contains(&value)
        {
            return None;
        }
        self.run_due_ops();
        // -- WP SW2 --
        if !self.alink_ids().contains(&value) {
            return Some(false);
        }
        if self.alink.setup.is_sender_build() {
            match value {
                alink::EXT_ALINK_CTRL_MSG => return Some(self.ctrl_msg(data)),
                alink::EXT_ALINK_TX => {
                    return Some(match AlinkTx::decode(data) {
                        Some(t)
                            if data.len() == AlinkTx::SIZE
                                && t.slot_map_id == 0
                                && t.flags & !1 == 0
                                && data.get(14..) == Some(&[0, 0]) =>
                        {
                            self.alink.tx = AlinkTx {
                                los_mask: 0,
                                usb_alt: self.alink.tx.usb_alt,
                                ..t
                            };
                            true
                        }
                        _ => false,
                    });
                }
                _ => {}
            }
        }
        Some(match value {
            alink::EXT_ALINK_CONFIG => match AlinkConfig::decode(data) {
                Some(c) if config_ok(&c, data) => {
                    let was = self.alink.config.enable;
                    self.alink.config = c;
                    // -- fixup sw1 (F5) -- with `enable` 0 the state reads 0,
                    // from holdover too (2.8.3); the change sends 0xE1.
                    if was && !c.enable && self.alink.state != AlinkState::Off {
                        self.alink.state = AlinkState::Off;
                        self.alink_event(AlinkEventReason::Disabled);
                    } else if !was && c.enable && self.alink.state == AlinkState::Off {
                        self.lock(AlinkEventReason::Locked);
                    }
                    true
                }
                _ => false,
            },
            alink::EXT_ALINK_SLOTS => match AlinkSlots::decode(data) {
                Some(s) if data.len() == AlinkSlots::SIZE && s.is_valid() => {
                    self.alink.slots = s;
                    true
                }
                _ => false,
            },
            alink::EXT_ALINK_PHASE => self.set_phase(data),
            alink::EXT_APPLY_AT_FRAME => match ApplyRequest::decode(data) {
                Some(r) if data.len() == ApplyRequest::SIZE && data.get(12..) == Some(&[0; 4]) => {
                    self.queue_op(r)
                }
                _ => false,
            },
            alink::EXT_ALINK_STATS_RESET => match *data {
                [1] => {
                    let l = &mut self.alink;
                    l.lock_count = 0;
                    l.loss_count = 0;
                    l.crc16_errors = 0;
                    true
                }
                _ => false,
            },
            // INFO, STATUS, FRAME_COUNTER, DIAG are IN only; CTRL_MSG OUT
            // and TX are a sender's.
            _ => false,
        })
    }

    /// 2.8.5: refused out of range or with OUT flags; a change of mode or
    /// delay while locked re-anchors.
    fn set_phase(&mut self, data: &[u8]) -> bool {
        let Some(p) = AlinkPhase::decode(data) else {
            return false;
        };
        if data.len() != AlinkPhase::SIZE
            || data.get(1) != Some(&0)
            || !p.is_valid_for(&self.alink_info())
        {
            return false;
        }
        let old = self.alink.phase;
        let changed = old.mode != p.mode || old.effective() != p.effective();
        let locked = self.alink.state == AlinkState::Locked;
        self.alink.phase = AlinkPhase {
            flags: 0,
            state: match (p.mode, locked) {
                (alink::PHASE_MODE_LOCK, true) => 2,
                _ => 0,
            },
            ..p
        };
        if changed && locked && old.mode == alink::PHASE_MODE_LOCK {
            self.alink_event(AlinkEventReason::Reanchor);
        }
        true
    }

    /// Queue an op, or run it now when its frame has passed; `CANCEL_ALL`
    /// cancels the queue. Refused: arguments out of range, a full queue, no
    /// lock.
    fn queue_op(&mut self, r: ApplyRequest) -> bool {
        if !r.args_valid() || self.alink.state != AlinkState::Locked {
            return false;
        }
        if r.op == ApplyOp::CancelAll {
            for q in std::mem::take(&mut self.alink.queue) {
                self.op_done(q, ApplyResult::Cancelled);
            }
            return true;
        }
        if r.frame <= self.alink_frame() {
            self.op_done(r, ApplyResult::Late);
            return true;
        }
        if self.alink.queue.len() >= APPLY_QUEUE {
            return false;
        }
        self.alink.queue.push(r);
        true
    }

    /// Run the queued ops whose frame has come, in frame order.
    fn run_due_ops(&mut self) {
        let now = self.alink_frame();
        let mut due: Vec<ApplyRequest> = Vec::new();
        self.alink.queue.retain(|r| {
            let d = r.frame <= now;
            if d {
                due.push(*r);
            }
            !d
        });
        due.sort_by_key(|r| r.frame);
        for r in due {
            self.op_done(r, ApplyResult::Applied);
        }
    }

    fn op_done(&mut self, r: ApplyRequest, result: ApplyResult) {
        self.alink.last = ApplyStatus {
            pending: 0,
            last_op: r.op,
            last_result: result,
            last_frame: r.frame,
        };
        let e = ApplyResultEvent {
            op: r.op,
            result,
            frame_low: low32(r.frame),
        };
        self.push(packet(alink::NOTIFY_EVT_APPLY_RESULT, &e.encode()));
    }

    fn alink_event(&mut self, reason: AlinkEventReason) {
        let state = self.alink.state;
        let e = AlinkStateEvent {
            state,
            class: u8::from(state == AlinkState::Locked),
            reason,
            frame_low: low32(self.alink_frame()),
        };
        self.push(packet(alink::NOTIFY_EVT_ALINK_STATE, &e.encode()));
    }

    fn lock(&mut self, reason: AlinkEventReason) {
        let l = &mut self.alink;
        if l.state != AlinkState::Locked {
            l.lock_count = l.lock_count.saturating_add(1);
        }
        l.state = AlinkState::Locked;
        l.held = None;
        l.phase.state = if l.phase.mode == alink::PHASE_MODE_LOCK {
            2
        } else {
            0
        };
        self.alink_event(reason);
    }

    // -- WP SW2 --
    /// A sender build's own replies: `EXT_ALINK_CTRL_MSG` IN (the superframe it
    /// sends, with the pending apply until the counter reaches it) and
    /// `EXT_ALINK_TX`. `None`: answered as on a receiver.
    fn get_sender(&mut self, value: u16) -> Option<Vec<u8>> {
        let now = self.alink_frame();
        let l = &mut self.alink;
        if l.ctrl.is_some_and(|c| c.frame <= now) {
            l.ctrl = None;
        }
        match value {
            alink::EXT_ALINK_CTRL_MSG => Some(
                SuperframeSnapshot {
                    flags: 0b1000 | (l.tx.flags & 1) | if l.ctrl.is_some() { 0b100 } else { 0 },
                    rate_code: l.tx.rate_code,
                    channels: l.tx.channels,
                    class: l.tx.class,
                    hops: l.setup.hops,
                    sender_id: l.setup.sender_id,
                    apply: l.ctrl.unwrap_or(ApplyRequest {
                        frame: 0,
                        op: ApplyOp::CancelAll,
                        arg: 0,
                        value: 0,
                    }),
                    frame_low: low32(now & !31),
                }
                .encode()
                .to_vec(),
            ),
            alink::EXT_ALINK_TX => Some(l.tx.encode().to_vec()),
            _ => None,
        }
    }

    /// 2.8.9 OUT on a sender: one apply at a time, at least the next
    /// superframe start plus 32 ahead; op 0 sets only the flags; RATE on a
    /// multiple of 32 with value 0.
    fn ctrl_msg(&mut self, data: &[u8]) -> bool {
        let (Some(r), Some(&flags)) = (ApplyRequest::decode(data), data.get(12)) else {
            return false;
        };
        if data.len() != ApplyRequest::SIZE || data.get(13..) != Some(&[0; 3]) || flags & !1 != 0 {
            return false;
        }
        let now = self.alink_frame();
        if self.alink.ctrl.is_some_and(|c| c.frame <= now) {
            self.alink.ctrl = None;
        }
        if r.op == ApplyOp::CancelAll {
            self.set_system_mute(flags);
            return true;
        }
        let lead_ok = alink::ctrl_msg_min_frame(now).is_some_and(|min| r.frame >= min);
        let rate_ok = r.op != ApplyOp::Rate || r.frame % alink::SUPERFRAME_FRAMES == 0;
        if !r.args_valid() || !lead_ok || !rate_ok || self.alink.ctrl.is_some() {
            return false;
        }
        self.alink.ctrl = Some(r);
        // As the firmware's `set_ctrl`: every accepted write sets the bit.
        self.set_system_mute(flags);
        true
    }

    /// `system_flags` bit 0 into the one flags byte.
    fn set_system_mute(&mut self, flags: u8) {
        let tx = &mut self.alink.tx;
        tx.flags = (tx.flags & !1) | (flags & 1);
    }

    fn alink_sim(&mut self, op: u8) -> bool {
        match op {
            ALINK_SIM_LOSE => {
                let l = &mut self.alink;
                if l.state == AlinkState::Locked {
                    // -- WP FIXS1 --
                    let e0 = self.alink_phase_error();
                    let l = &mut self.alink;
                    l.held = Some((Instant::now(), e0));
                    l.state = AlinkState::Holdover;
                    l.loss_count = l.loss_count.saturating_add(1);
                    l.phase.state = if l.phase.mode == alink::PHASE_MODE_LOCK {
                        3
                    } else {
                        0
                    };
                    self.alink_event(AlinkEventReason::Los);
                }
            }
            ALINK_SIM_LOCK => self.lock(AlinkEventReason::Locked),
            ALINK_SIM_REANCHOR => self.lock(AlinkEventReason::Reanchor),
            ALINK_SIM_BANK_ERROR => {
                let l = &mut self.alink;
                l.crc16_errors = l.crc16_errors.saturating_add(1);
            }
            _ => return false,
        }
        true
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests_wp_sw1;

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests_wp_sw2;
