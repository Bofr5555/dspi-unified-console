//! WP SW1: a receiving unit of the audio link, answered when
//! [`super::FakeOptions::audio_link`] is on, as the fork's
//! `extension_channel_spec.md` 2.8 (WP LK7) specifies a receiver build, so
//! clients and the hub can be built without a sender, a fibre or LK5/LK6
//! firmware.
//!
//! The model: the spec's example receiver (GP0, L1-L3, 64 channels,
//! `PHASE_LOCK | APPLY_AT_FRAME | LOS_PIN`, `D_local` 112..1024), hearing L1
//! at 48 kHz, 8 channels, from sender 0x42, locked from the start. The frame
//! counter runs with the fake's clock (48 frames per ms), extrapolated in
//! holdover as the firmware does. Every receiver sub-command is listed and
//! answered with the spec's refusals (a full apply queue, an apply without a
//! lock, an out-of-range delay, a bad pin or flag); `EXT_LINK_TX` is not.
//! `EXT_NOTIFY_FEATURES` announces bits 2 and 3; 0xE1 goes out on a loss, a
//! lock, a re-anchor (a new phase setting while locked too) and a disable,
//! 0xE2 for every completed op. Input source 7 is selectable.
//!
//! [`EXT_FAKE_LINK_SIM`] plays the link's side: lose it, lock it, re-anchor,
//! count a bank error. Apply-at-frame ops are queued (four at most) and run
//! when the counter passes their frame, checked at every link request; one
//! whose frame has passed runs at once and reports "late". Their effect on
//! the audio (a mute, a preset load) is not modelled, only their results.
//! Not modelled either: the 250 ms rate limit on 0xE1, the phase servo (the
//! phase error is 0 when locked; an anchor completes at once), the pin table
//! check against the live configuration, `D_local` limits that follow the
//! output latency option; the link input runs whatever the input source (the
//! firmware runs it only under source 7).
//!
//! WP SW2: [`AudioLinkSetup`] makes the fake another place in a system: a
//! receiver behind re-emitters (`hops`), a repeater or fan-out with its
//! `hop_delay_frames` (STATUS offset 50), a unit that hears another sender,
//! or the sender build itself (role 2: [`alink::SENDER_IDS`], the counter
//! running from its own start, `EXT_LINK_CTRL_MSG` OUT with 2.8.9's lead and
//! one-at-a-time rules, its IN showing the pending apply until the counter
//! passes it, `EXT_LINK_TX` kept as written). An id the role does not list
//! stalls. The superframe does not reach other fakes: a test checks what the
//! sender was asked.
#![warn(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

use dspi_proto::ext::audio_link::{
    self as alink, ApplyOp, ApplyRequest, ApplyResult, ApplyResultEvent, ApplyStatus, FrameCounter,
    LinkConfig, LinkEventReason, LinkInfo, LinkPhase, LinkRole, LinkSlots, LinkState,
    LinkStateEvent, LinkStatus, LinkTx, SuperframeSnapshot, feature,
};
use dspi_proto::generated::notify as nfy;

use super::State;
use crate::clock::Instant;

/// `REQ_EXTENSION` OUT sub-command only a [`super::FakeDevice`] answers, as
/// [`super::EXT_FAKE_VREG_SIM`] (WP SW1): one byte, what happens to the
/// audio link. [`LINK_SIM_LOSE`], [`LINK_SIM_LOCK`], [`LINK_SIM_REANCHOR`] or
/// [`LINK_SIM_BANK_ERROR`]; anything else, or the fake without
/// [`super::FakeOptions::audio_link`], stalls.
pub const EXT_FAKE_LINK_SIM: u16 = 0xFFF7;
/// [`EXT_FAKE_LINK_SIM`]: the fibre is pulled (LOS). Holdover, event 0xE1.
pub const LINK_SIM_LOSE: u8 = 0;
/// [`EXT_FAKE_LINK_SIM`]: the link is back (or locks). Event 0xE1 `LOCKED`.
pub const LINK_SIM_LOCK: u8 = 1;
/// [`EXT_FAKE_LINK_SIM`]: back with too large a phase error: event 0xE1
/// `REANCHOR`.
pub const LINK_SIM_REANCHOR: u8 = 2;
/// [`EXT_FAKE_LINK_SIM`]: one audio bank failed its CRC.
pub const LINK_SIM_BANK_ERROR: u8 = 3;

/// The sender id the fake hears.
const SENDER_ID: u8 = 0x42;
/// Frames per millisecond at 48 kHz.
const FRAMES_PER_MS: u64 = 48;
/// The link frame at the fake's start, so the counter is plainly not zero.
const FRAME_BASE: u64 = 1_000_000;
/// The apply queue's depth (2.8.7).
const APPLY_QUEUE: usize = 4;
/// `EXT_LINK_DIAG` layout version.
const DIAG_VERSION: u8 = 1;

// -- WP SW2 --
/// Where the fake sits on the link ([`super::FakeDevice::demo_audio_link_as`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AudioLinkSetup {
    /// `Receiver` (the default), `Repeater`, `Sender` or `FanOut`.
    pub role: LinkRole,
    /// Re-emitters between the sender and this unit (STATUS offset 4).
    pub hops: u8,
    /// A re-emitter's hop delay (STATUS offset 50), reported while locked.
    pub hop_delay_frames: u8,
    /// The sender the unit hears, or a sender's own id.
    pub sender_id: u8,
    /// `EXT_LINK_INFO.d_local_min`.
    pub d_local_min: u16,
}

impl Default for AudioLinkSetup {
    fn default() -> Self {
        Self {
            role: LinkRole::Receiver,
            hops: 0,
            hop_delay_frames: 0,
            sender_id: SENDER_ID,
            d_local_min: 112,
        }
    }
}

impl AudioLinkSetup {
    fn is_sender_build(&self) -> bool {
        matches!(self.role, LinkRole::Sender | LinkRole::FanOut)
    }
}

/// The link's state on the fake.
#[derive(Debug)]
pub(super) struct AudioLink {
    state: LinkState,
    config: LinkConfig,
    slots: LinkSlots,
    phase: LinkPhase,
    lock_count: u32,
    loss_count: u32,
    crc16_errors: u32,
    queue: Vec<ApplyRequest>,
    last: ApplyStatus,
    // -- WP SW2 --
    pub(super) setup: AudioLinkSetup,
    /// A sender's pending superframe apply.
    ctrl: Option<ApplyRequest>,
    /// A sender's `EXT_LINK_TX`. Its flags bit 0 is the system mute, the one
    /// byte `EXT_LINK_CTRL_MSG`'s `system_flags` writes too (fixup sw2x: the
    /// firmware's `snd.cfg.flags`).
    tx: LinkTx,
}

impl Default for AudioLink {
    fn default() -> Self {
        Self {
            state: LinkState::Locked,
            config: LinkConfig {
                enable: true,
                rx_pin: 0xFF,
                class: 0,
                tx_pin: 0xFF,
                los_pin: 0xFF,
                txdis_pin: 0xFF,
                flags: 0b11,
            },
            slots: LinkSlots::IDENTITY,
            phase: LinkPhase {
                mode: alink::PHASE_MODE_LOCK,
                d_local: 128,
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
            setup: AudioLinkSetup::default(),
            ctrl: None,
            tx: LinkTx {
                link_mask: 0,
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
        }
    }
}

/// The link input's build default, GP0 (2.8.3).
const RX_PIN_DEFAULT: u8 = 0;

/// The receiver's `EXT_LINK_INFO`: 2.8.1's example, `rx_pin` following
/// `config` (its `0xFF` resolved to the build default, 2.8.1).
///
/// WP SW2: for the fake's place on the link; a sender build reports its links
/// and no slots or delay range (2.8.1).
fn info_as(config: &LinkConfig, setup: &AudioLinkSetup) -> LinkInfo {
    if setup.is_sender_build() {
        return LinkInfo {
            role: setup.role,
            protocol_version: alink::LINK_PROTOCOL_VERSION,
            class_mask: 0b111,
            max_links: 8,
            max_channels: alink::LINK_MAX_CHANNELS,
            slots_per_unit: 0,
            flags: 0,
            rx_pin: 20,
            tx_pin: 0xFF,
            feature_bits: feature::APPLY_AT_FRAME | feature::FAN_OUT | feature::TEST_PATTERNS,
            d_local_min: 0,
            d_local_max: 0,
        };
    }
    let repeater = setup.role == LinkRole::Repeater;
    LinkInfo {
        role: setup.role,
        protocol_version: alink::LINK_PROTOCOL_VERSION,
        class_mask: 0b111,
        max_links: 1,
        max_channels: alink::LINK_MAX_CHANNELS,
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
        d_local_min: setup.d_local_min,
        d_local_max: 1024,
    }
}

/// A header GPIO on a Pico 2 (2.8.3), or 0xFF.
fn pin_ok(p: u8) -> bool {
    p == 0xFF || p <= 22 || (26..=28).contains(&p)
}

/// 2.8.3's refusals, but the live pin table's.
fn config_ok(c: &LinkConfig, raw: &[u8]) -> bool {
    let pins = [c.rx_pin, c.tx_pin, c.los_pin, c.txdis_pin];
    let set: Vec<u8> = pins.iter().copied().filter(|p| *p != 0xFF).collect();
    let distinct = set
        .iter()
        .enumerate()
        .all(|(i, p)| !set.iter().skip(i.saturating_add(1)).any(|q| q == p));
    raw.len() == LinkConfig::SIZE
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

/// A link packet: the v2 header, then `payload`.
fn packet(id: u8, payload: &[u8]) -> Vec<u8> {
    let mut p = vec![nfy::NOTIFY_V2_VERSION as u8, id, 0, 0];
    p.extend_from_slice(payload);
    p
}

impl State {
    /// The link frame now: from the fake's start, while locked or in
    /// holdover (extrapolated); 0 otherwise.
    fn link_frame(&self) -> u64 {
        // -- WP SW2 -- the master sender's counter always runs.
        let master = self.audio_link.setup.role == LinkRole::Sender;
        if !master
            && !matches!(
                self.audio_link.state,
                LinkState::Locked | LinkState::Holdover
            )
        {
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

    fn link_status(&self) -> LinkStatus {
        let l = &self.audio_link;
        let locked = l.state == LinkState::Locked;
        // -- WP SW2 -- a master sender has no input: state 0 (2.8.2).
        let master = l.setup.role == LinkRole::Sender;
        let reemits = matches!(l.setup.role, LinkRole::Repeater | LinkRole::FanOut);
        LinkStatus {
            state: if master { LinkState::Off } else { l.state },
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
            frame: self.link_frame(),
            lock_count: l.lock_count,
            loss_count: l.loss_count,
            crc8_errors: 0,
            crc16_errors: l.crc16_errors,
            bit_slips: 0,
            superframe_errors: 0,
            vcxo_ppm: 1.5,
            phase_error_frames: 0.0,
            fill_frames: l.phase.d_local,
            // -- WP SW2 --
            hop_delay_frames: if reemits && locked {
                l.setup.hop_delay_frames
            } else {
                0
            },
        }
    }

    /// The link sub-commands the fake lists, with the option on.
    pub(super) fn link_ids(&self) -> Vec<u16> {
        if !self.options.audio_link {
            return Vec::new();
        }
        // -- WP SW2 --
        if self.audio_link.setup.is_sender_build() {
            return alink::SENDER_IDS.to_vec();
        }
        alink::RECEIVER_IDS.to_vec()
    }

    /// Whether input source 7 is selectable.
    pub(super) fn link_input(&self) -> bool {
        self.options.audio_link && !self.audio_link.setup.is_sender_build()
    }

    /// IN: `Some(reply)` for a link sub-command (`Some(None)` stalls), `None`
    /// for anything else.
    pub(super) fn get_link(&mut self, value: u16) -> Option<Option<Vec<u8>>> {
        if !self.options.audio_link
            || !(alink::EXT_LINK_INFO..=alink::EXT_LINK_DIAG).contains(&value)
        {
            return None;
        }
        self.run_due_ops();
        // -- WP SW2 -- an id this build does not list stalls (2.8).
        if !self.link_ids().contains(&value) {
            return Some(None);
        }
        if self.audio_link.setup.is_sender_build()
            && let Some(reply) = self.get_sender(value)
        {
            return Some(Some(reply));
        }
        let l = &self.audio_link;
        Some(match value {
            alink::EXT_LINK_INFO => Some(info_as(&l.config, &l.setup).encode().to_vec()),
            alink::EXT_LINK_STATUS => Some(self.link_status().encode().to_vec()),
            alink::EXT_LINK_CONFIG => Some(l.config.encode().to_vec()),
            alink::EXT_LINK_SLOTS => Some(l.slots.0.to_vec()),
            alink::EXT_LINK_PHASE => Some(l.phase.encode().to_vec()),
            alink::EXT_FRAME_COUNTER => {
                let us = Instant::now()
                    .saturating_duration_since(self.started)
                    .as_micros();
                let frame = self.link_frame();
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
            alink::EXT_LINK_CTRL_MSG => Some(
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
                    frame_low: low32(self.link_frame() & !31),
                }
                .encode()
                .to_vec(),
            ),
            alink::EXT_LINK_DIAG => {
                let mut d = vec![0u8; alink::LINK_DIAG_SIZE];
                if let Some(v) = d.first_mut() {
                    *v = DIAG_VERSION;
                }
                Some(d)
            }
            // EXT_LINK_TX: a sender's.
            _ => None,
        })
    }

    /// OUT: `Some(accepted)` for a link sub-command or [`EXT_FAKE_LINK_SIM`],
    /// `None` for anything else.
    pub(super) fn set_link(&mut self, value: u16, data: &[u8]) -> Option<bool> {
        if value == EXT_FAKE_LINK_SIM {
            return Some(match *data {
                [op] if self.options.audio_link => self.link_sim(op),
                _ => false,
            });
        }
        if !self.options.audio_link
            || !(alink::EXT_LINK_INFO..=alink::EXT_LINK_DIAG).contains(&value)
        {
            return None;
        }
        self.run_due_ops();
        // -- WP SW2 --
        if !self.link_ids().contains(&value) {
            return Some(false);
        }
        if self.audio_link.setup.is_sender_build() {
            match value {
                alink::EXT_LINK_CTRL_MSG => return Some(self.ctrl_msg(data)),
                alink::EXT_LINK_TX => {
                    return Some(match LinkTx::decode(data) {
                        Some(t)
                            if data.len() == LinkTx::SIZE
                                && t.slot_map_id == 0
                                && t.flags & !1 == 0
                                && data.get(14..) == Some(&[0, 0]) =>
                        {
                            self.audio_link.tx = LinkTx {
                                los_mask: 0,
                                usb_alt: self.audio_link.tx.usb_alt,
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
            alink::EXT_LINK_CONFIG => match LinkConfig::decode(data) {
                Some(c) if config_ok(&c, data) => {
                    let was = self.audio_link.config.enable;
                    self.audio_link.config = c;
                    // -- fixup sw1 (F5) -- with `enable` 0 the state reads 0,
                    // from holdover too (2.8.3); the change sends 0xE1.
                    if was && !c.enable && self.audio_link.state != LinkState::Off {
                        self.audio_link.state = LinkState::Off;
                        self.link_event(LinkEventReason::Disabled);
                    } else if !was && c.enable && self.audio_link.state == LinkState::Off {
                        self.lock(LinkEventReason::Locked);
                    }
                    true
                }
                _ => false,
            },
            alink::EXT_LINK_SLOTS => match LinkSlots::decode(data) {
                Some(s) if data.len() == LinkSlots::SIZE && s.is_valid() => {
                    self.audio_link.slots = s;
                    true
                }
                _ => false,
            },
            alink::EXT_LINK_PHASE => self.set_phase(data),
            alink::EXT_APPLY_AT_FRAME => match ApplyRequest::decode(data) {
                Some(r) if data.len() == ApplyRequest::SIZE && data.get(12..) == Some(&[0; 4]) => {
                    self.queue_op(r)
                }
                _ => false,
            },
            alink::EXT_LINK_STATS_RESET => match *data {
                [1] => {
                    let l = &mut self.audio_link;
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
        let Some(p) = LinkPhase::decode(data) else {
            return false;
        };
        if data.len() != LinkPhase::SIZE
            || data.get(1) != Some(&0)
            || !p.is_valid_for(&info_as(&self.audio_link.config, &self.audio_link.setup))
        {
            return false;
        }
        let old = self.audio_link.phase;
        let changed = old.mode != p.mode || old.effective() != p.effective();
        let locked = self.audio_link.state == LinkState::Locked;
        self.audio_link.phase = LinkPhase {
            flags: 0,
            state: match (p.mode, locked) {
                (alink::PHASE_MODE_LOCK, true) => 2,
                _ => 0,
            },
            ..p
        };
        if changed && locked && old.mode == alink::PHASE_MODE_LOCK {
            self.link_event(LinkEventReason::Reanchor);
        }
        true
    }

    /// Queue an op, or run it now when its frame has passed; `CANCEL_ALL`
    /// cancels the queue. Refused: arguments out of range, a full queue, no
    /// lock.
    fn queue_op(&mut self, r: ApplyRequest) -> bool {
        if !r.args_valid() || self.audio_link.state != LinkState::Locked {
            return false;
        }
        if r.op == ApplyOp::CancelAll {
            for q in std::mem::take(&mut self.audio_link.queue) {
                self.op_done(q, ApplyResult::Cancelled);
            }
            return true;
        }
        if r.frame <= self.link_frame() {
            self.op_done(r, ApplyResult::Late);
            return true;
        }
        if self.audio_link.queue.len() >= APPLY_QUEUE {
            return false;
        }
        self.audio_link.queue.push(r);
        true
    }

    /// Run the queued ops whose frame has come, in frame order.
    fn run_due_ops(&mut self) {
        let now = self.link_frame();
        let mut due: Vec<ApplyRequest> = Vec::new();
        self.audio_link.queue.retain(|r| {
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
        self.audio_link.last = ApplyStatus {
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

    fn link_event(&mut self, reason: LinkEventReason) {
        let state = self.audio_link.state;
        let e = LinkStateEvent {
            state,
            class: u8::from(state == LinkState::Locked),
            reason,
            frame_low: low32(self.link_frame()),
        };
        self.push(packet(alink::NOTIFY_EVT_LINK_STATE, &e.encode()));
    }

    fn lock(&mut self, reason: LinkEventReason) {
        let l = &mut self.audio_link;
        if l.state != LinkState::Locked {
            l.lock_count = l.lock_count.saturating_add(1);
        }
        l.state = LinkState::Locked;
        l.phase.state = if l.phase.mode == alink::PHASE_MODE_LOCK {
            2
        } else {
            0
        };
        self.link_event(reason);
    }

    // -- WP SW2 --
    /// A sender build's own replies: `EXT_LINK_CTRL_MSG` IN (the superframe it
    /// sends, with the pending apply until the counter reaches it) and
    /// `EXT_LINK_TX`. `None`: answered as on a receiver.
    fn get_sender(&mut self, value: u16) -> Option<Vec<u8>> {
        let now = self.link_frame();
        let l = &mut self.audio_link;
        if l.ctrl.is_some_and(|c| c.frame <= now) {
            l.ctrl = None;
        }
        match value {
            alink::EXT_LINK_CTRL_MSG => Some(
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
            alink::EXT_LINK_TX => Some(l.tx.encode().to_vec()),
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
        let now = self.link_frame();
        if self.audio_link.ctrl.is_some_and(|c| c.frame <= now) {
            self.audio_link.ctrl = None;
        }
        if r.op == ApplyOp::CancelAll {
            self.set_system_mute(flags);
            return true;
        }
        let lead_ok = alink::ctrl_msg_min_frame(now).is_some_and(|min| r.frame >= min);
        let rate_ok = r.op != ApplyOp::Rate || r.frame % alink::SUPERFRAME_FRAMES == 0;
        if !r.args_valid() || !lead_ok || !rate_ok || self.audio_link.ctrl.is_some() {
            return false;
        }
        self.audio_link.ctrl = Some(r);
        // As the firmware's `set_ctrl`: every accepted write sets the bit.
        self.set_system_mute(flags);
        true
    }

    /// `system_flags` bit 0 into the one flags byte.
    fn set_system_mute(&mut self, flags: u8) {
        let tx = &mut self.audio_link.tx;
        tx.flags = (tx.flags & !1) | (flags & 1);
    }

    fn link_sim(&mut self, op: u8) -> bool {
        match op {
            LINK_SIM_LOSE => {
                let l = &mut self.audio_link;
                if l.state == LinkState::Locked {
                    l.state = LinkState::Holdover;
                    l.loss_count = l.loss_count.saturating_add(1);
                    l.phase.state = if l.phase.mode == alink::PHASE_MODE_LOCK {
                        3
                    } else {
                        0
                    };
                    self.link_event(LinkEventReason::Los);
                }
            }
            LINK_SIM_LOCK => self.lock(LinkEventReason::Locked),
            LINK_SIM_REANCHOR => self.lock(LinkEventReason::Reanchor),
            LINK_SIM_BANK_ERROR => {
                let l = &mut self.audio_link;
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
