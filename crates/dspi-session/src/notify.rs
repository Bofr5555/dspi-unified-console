//! The device's notification stream, protocol v2.
//!
//! The firmware pushes state changes on bulk endpoint `0x83`
//! (`notification_protocol_v2_spec.md`, `notify.h`). Most of them are
//! `PARAM_CHANGED`: a byte offset into `WireBulkParams`, a size, a source tag
//! and the new bytes, which the host copies straight into its bulk shadow. A
//! few are discrete events with their own small layouts. Every packet carries
//! a wrapping sequence number; a gap means the device dropped something and
//! the only safe recovery is a full re-read.
//!
//! [`Notifications`] runs the reader on its own thread and hands decoded
//! events to the application through a channel, so the interface never
//! blocks on the endpoint.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use dspi_transport::NotificationSource;

use dspi_proto::generated::notify as n;

/// `NOTIFY_V2_VERSION` (notify.h:86): the first byte of every v2 packet.
pub const PROTOCOL_VERSION: u8 = n::NOTIFY_V2_VERSION as u8;

/// The largest field a `PARAM_CHANGED` can carry (notify.h:132); anything
/// bigger arrives as `BULK_INVALIDATED` instead.
pub const MAX_NOTIFIED_FIELD: usize = 52;

/// Event identifiers, `NOTIFY_EVT_*` (notify.h:25-83), generated from the
/// vendored header rather than typed here.
pub mod evt {
    use super::n;
    pub const IDLE: u8 = n::NOTIFY_EVT_IDLE as u8;
    pub const MASTER_VOLUME: u8 = n::NOTIFY_EVT_MASTER_VOLUME as u8;
    pub const PARAM_CHANGED: u8 = n::NOTIFY_EVT_PARAM_CHANGED as u8;
    pub const BULK_INVALIDATED: u8 = n::NOTIFY_EVT_BULK_INVALIDATED as u8;
    pub const PRESET_LOADED: u8 = n::NOTIFY_EVT_PRESET_LOADED as u8;
    pub const INPUT_FORMAT: u8 = n::NOTIFY_EVT_INPUT_FORMAT as u8;
    pub const SIGGEN_STATE: u8 = n::NOTIFY_EVT_SIGGEN_STATE as u8;
    pub const ADAT_STATE: u8 = n::NOTIFY_EVT_ADAT_STATE as u8;
    pub const I2S_SLAVE_STATE: u8 = n::NOTIFY_EVT_I2S_SLAVE_STATE as u8;
    pub const CS_IR_LEARN: u8 = n::NOTIFY_EVT_CS_IR_LEARN as u8;
    pub const ADAT_INPUT_STATE: u8 = n::NOTIFY_EVT_ADAT_INPUT_STATE as u8;
    pub const CS_AUX: u8 = n::NOTIFY_EVT_CS_AUX as u8;
}

/// Who made the change, `ParamSource` (notify.h:92-103). Unknown values are
/// kept rather than mapped, because the range grows with new transports and
/// "someone else changed it" is the right reading for all of them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Unknown,
    HostSet,
    BulkSet,
    Preset,
    Factory,
    /// A control surface: every GPIO dispatch.
    Gpio,
    Internal,
    /// The operating system's volume slider, over UAC1.
    Uac1,
    Uart,
    I2c,
    /// The USB MIDI control channel (`PARAM_SRC_MIDI`, DSPI_USB_MIDI firmware).
    Midi,
    Other(u8),
}

impl Source {
    pub fn from_raw(b: u8) -> Self {
        match b as u16 {
            n::PARAM_SRC_UNKNOWN => Self::Unknown,
            n::PARAM_SRC_HOST_SET => Self::HostSet,
            n::PARAM_SRC_BULK_SET => Self::BulkSet,
            n::PARAM_SRC_PRESET => Self::Preset,
            n::PARAM_SRC_FACTORY => Self::Factory,
            n::PARAM_SRC_GPIO => Self::Gpio,
            n::PARAM_SRC_INTERNAL => Self::Internal,
            n::PARAM_SRC_UAC1 => Self::Uac1,
            n::PARAM_SRC_UART => Self::Uart,
            n::PARAM_SRC_I2C => Self::I2c,
            n::PARAM_SRC_MIDI => Self::Midi,
            _ => Self::Other(b),
        }
    }

    /// The raw `ParamSource` byte, for [`dspi_transport::is_own_write`].
    pub fn raw(self) -> u8 {
        let v = match self {
            Self::Unknown => n::PARAM_SRC_UNKNOWN,
            Self::HostSet => n::PARAM_SRC_HOST_SET,
            Self::BulkSet => n::PARAM_SRC_BULK_SET,
            Self::Preset => n::PARAM_SRC_PRESET,
            Self::Factory => n::PARAM_SRC_FACTORY,
            Self::Gpio => n::PARAM_SRC_GPIO,
            Self::Internal => n::PARAM_SRC_INTERNAL,
            Self::Uac1 => n::PARAM_SRC_UAC1,
            Self::Uart => n::PARAM_SRC_UART,
            Self::I2c => n::PARAM_SRC_I2C,
            Self::Midi => n::PARAM_SRC_MIDI,
            Self::Other(b) => return b,
        };
        v as u8
    }

    /// Our own writes over USB come back as `HostSet`; everything else is
    /// news. Over other links use [`Notification::is_own_echo`], which knows
    /// the link's rule.
    pub fn is_ours(self) -> bool {
        matches!(self, Self::HostSet | Self::BulkSet)
    }

    /// The Console's wording for the echo line.
    pub fn describe(self) -> &'static str {
        match self {
            Self::Unknown | Self::Other(_) => "changed elsewhere",
            Self::HostSet | Self::BulkSet => "changed by this host",
            Self::Preset => "changed by a preset load",
            Self::Factory => "reset to factory defaults",
            Self::Gpio => "changed by control surface",
            Self::Internal => "changed by the device",
            Self::Uac1 => "changed by the system volume",
            Self::Uart => "changed over UART",
            Self::I2c => "changed over I2C",
            Self::Midi => "changed over MIDI",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// The keep-alive; never delivered to the application.
    Idle,
    /// The v1 legacy volume event, emitted alongside a `PARAM_CHANGED`.
    MasterVolume(f32),
    ParamChanged {
        /// `offsetof(WireBulkParams, field)`, plus `index * size` for arrays.
        offset: u16,
        source: Source,
        bytes: Vec<u8>,
    },
    BulkInvalidated {
        source: Source,
    },
    PresetLoaded {
        slot: u8,
    },
    /// Active input channels changed: 2, 4, 6 or 8.
    InputFormat {
        channels: u8,
    },
    SiggenState {
        state: u8,
        reason: u8,
        signal_type: u8,
        /// `0xFF` when not walking outputs.
        channel: u8,
    },
    AdatState {
        enabled: bool,
        active: bool,
        pin: u8,
    },
    I2sSlaveState {
        state: u8,
        /// Zero until locked.
        rate_hz: u32,
    },
    IrLearn {
        state: u8,
        protocol: u8,
        code: u32,
    },
    AdatInputState {
        state: u8,
        rate_hz: u32,
        clock_mode: u8,
    },
    /// An auxiliary output changed, from any source (notify.h:78-83). Sent
    /// only on a real change, with both values so a host refreshes the slot
    /// in one step.
    CsAux {
        /// The binding slot holding the aux component.
        slot: u8,
        /// Non-zero is on.
        state: u8,
        /// 8.8 percent; 0 on an on/off output.
        level_q8: u16,
        source: Source,
    },
    /// An event id this build does not know; kept so it can be shown in the
    /// monitor rather than silently dropped.
    Unknown {
        id: u8,
        bytes: Vec<u8>,
    },
}

/// One decoded packet.
#[derive(Debug, Clone, PartialEq)]
pub struct Notification {
    pub seq: u8,
    pub event: Event,
    /// The sequence number skipped: something was dropped before this one,
    /// and the bulk shadow can no longer be trusted.
    pub lost: bool,
    /// The link's attribution (DSPi Link session id of the writer, 0 for the
    /// device); `None` over USB and MIDI. See [`Notification::is_own_echo`].
    pub origin: Option<u16>,
}

impl Notification {
    /// Whether this is the echo of a write this client made, by the rule of
    /// the link it came over ([`dspi_transport::is_own_write`]): `kind` is
    /// the transport's [`dspi_transport::TransportCaps::kind`], `session` its
    /// [`dspi_transport::Transport::session_id`], and `own_bulk_pending`
    /// whether one of our own bulk writes may explain a `BULK_SET`
    /// invalidation (see [`EchoFilter`], which also consumes it). Such a
    /// change is already in the client's mirror.
    pub fn is_own_echo(
        &self,
        kind: dspi_transport::TransportKind,
        session: Option<u16>,
        own_bulk_pending: bool,
    ) -> bool {
        match &self.event {
            Event::ParamChanged { source, .. } | Event::BulkInvalidated { source } => {
                dspi_transport::is_own_write(
                    kind,
                    source.raw(),
                    self.origin,
                    session,
                    own_bulk_pending,
                )
            }
            _ => false,
        }
    }

    /// The `ParamSource` byte of an event that has one.
    pub fn source(&self) -> Option<Source> {
        match &self.event {
            Event::ParamChanged { source, .. } | Event::BulkInvalidated { source } => Some(*source),
            _ => None,
        }
    }
}

/// Recognises the `BULK_INVALIDATED` that answers our own bulk write.
///
/// The firmware tags every bulk apply `BULK_SET`, whatever path it came in
/// on (bulk_params.c:371/408), so on USB, MIDI and the fake device the source
/// cannot tell our `0xA1`/`0xA3` from another host's. The device's bulk lock
/// keeps other hosts' bulk writes out while ours is in flight and for a
/// moment after, so: a `BULK_SET` invalidation that arrives while our bulk
/// write is in flight, or within [`EchoFilter::WINDOW`] of its successful
/// reply, is ours, **once**. Everything else is foreign; an unclaimed
/// invalidation only costs the client a re-read. Over Link the origin
/// decides and this filter is not consulted.
///
/// Pure (the caller passes the clock); reference behaviour: the iOS
/// console's `MIDITransport` (`dspi-ios` patch console-116/0005).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EchoFilter {
    state: BulkEcho,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum BulkEcho {
    /// No bulk write of ours to account for.
    #[default]
    Idle,
    /// Our bulk write is on the wire; `claimed` once its invalidation was seen.
    InFlight { claimed: bool },
    /// Our bulk write was accepted; its invalidation may still arrive until
    /// `until`.
    Window { until: Instant },
}

impl EchoFilter {
    /// How long after our bulk write's reply its invalidation is still
    /// expected.
    pub const WINDOW: Duration = Duration::from_secs(1);

    /// Our `0xA1` (or the first `0xA3` chunk) is about to be sent.
    pub fn bulk_write_started(&mut self) {
        self.state = BulkEcho::InFlight { claimed: false };
    }

    /// Our bulk write finished (`ok`: the device accepted it). Opens the
    /// window unless the invalidation already came, or the write failed.
    pub fn bulk_write_finished(&mut self, now: Instant, ok: bool) {
        self.state = match self.state {
            BulkEcho::InFlight { claimed: false } if ok => match now.checked_add(Self::WINDOW) {
                Some(until) => BulkEcho::Window { until },
                None => BulkEcho::Idle,
            },
            _ => BulkEcho::Idle,
        };
    }

    /// Whether a `BULK_SET` arriving `now` would be claimed as ours.
    pub fn own_bulk_pending(&self, now: Instant) -> bool {
        match self.state {
            BulkEcho::Idle => false,
            BulkEcho::InFlight { claimed } => !claimed,
            BulkEcho::Window { until } => now < until,
        }
    }

    /// Classify one notification's source: true if it is our own echo. A
    /// `BULK_SET` claimed here consumes the pending bulk write, so a second
    /// one is foreign.
    pub fn classify(
        &mut self,
        now: Instant,
        kind: dspi_transport::TransportKind,
        source: u8,
        origin: Option<u16>,
        session: Option<u16>,
    ) -> bool {
        let pending = self.own_bulk_pending(now);
        let own = dspi_transport::is_own_write(kind, source, origin, session, pending);
        let bulk = u16::from(source) == n::PARAM_SRC_BULK_SET;
        if own && bulk && kind != dspi_transport::TransportKind::Link {
            self.state = match self.state {
                BulkEcho::InFlight { .. } => BulkEcho::InFlight { claimed: true },
                _ => BulkEcho::Idle,
            };
        }
        own
    }

    /// [`EchoFilter::classify`] for a decoded notification; false for events
    /// without a source.
    pub fn classify_notification(
        &mut self,
        now: Instant,
        kind: dspi_transport::TransportKind,
        session: Option<u16>,
        note: &Notification,
    ) -> bool {
        match note.source() {
            Some(src) => self.classify(now, kind, src.raw(), note.origin, session),
            None => false,
        }
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum DecodeError {
    #[error("notification protocol v{0}; this build speaks v2")]
    Version(u8),
    #[error("event 0x{id:02X} is {got} bytes, needs {need}")]
    Short { id: u8, got: usize, need: usize },
}

/// Decode one packet. A packet shorter than the common header is the idle
/// keep-alive (the firmware sends exactly one byte), never an error.
pub fn decode(p: &[u8]) -> Result<(u8, Event), DecodeError> {
    if p.len() < 4 {
        return Ok((0, Event::Idle));
    }
    // The v1 legacy volume packet is `[0x01, 0, 0, 0, f32]` with no version
    // byte at all; it is recognised by shape before the version check.
    if p[0] == evt::MASTER_VOLUME && p[1] == 0 && p[2] == 0 && p[3] == 0 && p.len() >= 8 {
        return Ok((0, Event::MasterVolume(f32_at(p, 4))));
    }
    if p[0] != PROTOCOL_VERSION {
        return Err(DecodeError::Version(p[0]));
    }
    let id = p[1];
    let seq = p[3];
    let need = |n: usize| {
        if p.len() < n {
            Err(DecodeError::Short {
                id,
                got: p.len(),
                need: n,
            })
        } else {
            Ok(())
        }
    };
    let event = match id {
        evt::IDLE => Event::Idle,
        evt::PARAM_CHANGED => {
            need(12)?;
            let offset = u16::from_le_bytes([p[4], p[5]]);
            let size = u16::from_le_bytes([p[6], p[7]]) as usize;
            let source = Source::from_raw(p[8]);
            need(12 + size)?;
            Event::ParamChanged {
                offset,
                source,
                bytes: p[12..12 + size].to_vec(),
            }
        }
        evt::BULK_INVALIDATED => {
            need(5)?;
            Event::BulkInvalidated {
                source: Source::from_raw(p[4]),
            }
        }
        evt::PRESET_LOADED => {
            need(5)?;
            Event::PresetLoaded { slot: p[4] }
        }
        evt::INPUT_FORMAT => {
            need(5)?;
            Event::InputFormat { channels: p[4] }
        }
        evt::SIGGEN_STATE => {
            need(8)?;
            Event::SiggenState {
                state: p[4],
                reason: p[5],
                signal_type: p[6],
                channel: p[7],
            }
        }
        evt::ADAT_STATE => {
            need(7)?;
            Event::AdatState {
                enabled: p[4] != 0,
                active: p[5] != 0,
                pin: p[6],
            }
        }
        evt::I2S_SLAVE_STATE => {
            need(9)?;
            Event::I2sSlaveState {
                state: p[4],
                rate_hz: u32::from_le_bytes([p[5], p[6], p[7], p[8]]),
            }
        }
        evt::CS_IR_LEARN => {
            need(12)?;
            Event::IrLearn {
                state: p[4],
                protocol: p[5],
                code: u32::from_le_bytes([p[8], p[9], p[10], p[11]]),
            }
        }
        evt::ADAT_INPUT_STATE => {
            need(10)?;
            Event::AdatInputState {
                state: p[4],
                rate_hz: u32::from_le_bytes([p[5], p[6], p[7], p[8]]),
                clock_mode: p[9],
            }
        }
        // `[ver, 0x0C, flags, seq, slot, state, level_q8 LE, src]`, 9 bytes.
        evt::CS_AUX => {
            need(9)?;
            Event::CsAux {
                slot: p[4],
                state: p[5],
                level_q8: u16::from_le_bytes([p[6], p[7]]),
                source: Source::from_raw(p[8]),
            }
        }
        other => Event::Unknown {
            id: other,
            bytes: p[4..].to_vec(),
        },
    };
    Ok((seq, event))
}

fn f32_at(p: &[u8], o: usize) -> f32 {
    f32::from_le_bytes([p[o], p[o + 1], p[o + 2], p[o + 3]])
}

/// Tracks the wrapping sequence counter and reports gaps.
#[derive(Debug, Default, Clone)]
pub struct SeqTracker {
    last: Option<u8>,
}

impl SeqTracker {
    /// Record `seq`; returns true when at least one packet was skipped.
    pub fn observe(&mut self, seq: u8) -> bool {
        let lost = match self.last {
            Some(prev) => seq != prev.wrapping_add(1),
            None => false,
        };
        self.last = Some(seq);
        lost
    }

    /// Forget the counter, after a full re-read has restored the shadow.
    pub fn reset(&mut self) {
        self.last = None;
    }
}

/// Called by the reader thread when notifications are waiting.
pub type Waker = Arc<dyn Fn() + Send + Sync>;

/// The reader thread and its channel.
pub struct Notifications {
    rx: Receiver<Notification>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
    /// Set by the reader when the device went away.
    disconnected: Arc<AtomicBool>,
}

impl Notifications {
    /// How long one read waits before checking whether to stop.
    ///
    /// From beta4 the endpoint NAKs while idle and sends a keep-alive only
    /// after [`Self::KEEP_ALIVE`] of quiet (usb_audio.c:1064-1080), so an idle
    /// read blocks for about that long. The timeout is longer than the
    /// keep-alive, so a quiet read normally ends on the idle packet rather than
    /// on a cancelled transfer, and short enough that dropping the reader stops
    /// its thread within a quarter of a second.
    pub const READ_TIMEOUT: Duration = Duration::from_millis(250);

    /// `NOTIFY_IDLE_KEEPALIVE_US` (usb_audio.c:1066): the longest a quiet
    /// endpoint holds a read before it answers the idle packet.
    pub const KEEP_ALIVE: Duration = Duration::from_millis(100);

    /// The most notifications kept waiting for [`Notifications::drain`]. A
    /// reader that falls this far behind loses the oldest news anyway; the
    /// queue drops new packets instead of growing without bound (a device or
    /// a hub can send faster than a stalled UI drains), and marks the next
    /// packet it does deliver `lost`, so the client re-reads the bulk image.
    pub const QUEUE: usize = 1024;

    /// Start reading. Idle packets are dropped here; everything else is
    /// delivered with its sequence number, the loss flag and the link's
    /// origin. Fails only if the reader thread cannot be spawned.
    pub fn start(source: Box<dyn NotificationSource>) -> std::io::Result<Self> {
        Self::start_with_waker(source, None)
    }

    /// [`Notifications::start`], calling `waker` (on the reader thread) each
    /// time a notification has been queued. A GUI's waker only schedules a
    /// drain; it must not block or call back into the reader's owner.
    pub fn start_with_waker(
        mut source: Box<dyn NotificationSource>,
        waker: Option<Waker>,
    ) -> std::io::Result<Self> {
        let (tx, rx) = mpsc::sync_channel(Self::QUEUE);
        let stop = Arc::new(AtomicBool::new(false));
        let disconnected = Arc::new(AtomicBool::new(false));
        let stop_flag = Arc::clone(&stop);
        let disc_flag = Arc::clone(&disconnected);
        let thread = std::thread::Builder::new()
            .name("dspi-notify".into())
            .spawn(move || {
                let mut seqs = SeqTracker::default();
                // Set when the queue was full and a packet was dropped.
                let mut overflowed = false;
                // Queue one notification; false when the receiver is gone.
                let mut deliver = |mut n: Notification| -> bool {
                    n.lost |= overflowed;
                    match tx.try_send(n) {
                        Ok(()) => {
                            overflowed = false;
                            if let Some(w) = &waker {
                                w();
                            }
                            true
                        }
                        Err(mpsc::TrySendError::Full(_)) => {
                            overflowed = true;
                            true
                        }
                        Err(mpsc::TrySendError::Disconnected(_)) => false,
                    }
                };
                while !stop_flag.load(Ordering::Relaxed) {
                    let (packet, origin) = match source.read_attributed(Self::READ_TIMEOUT) {
                        Ok(a) => (a.packet, a.origin),
                        Err(dspi_transport::TransportError::Disconnected) => {
                            disc_flag.store(true, Ordering::Relaxed);
                            break;
                        }
                        // A stall or a transient error: give the bus a
                        // moment rather than spinning.
                        Err(_) => {
                            std::thread::sleep(Duration::from_millis(50));
                            continue;
                        }
                    };
                    if packet.is_empty() {
                        continue;
                    }
                    let n = match decode(&packet) {
                        Ok((_, Event::Idle)) => continue,
                        Ok((seq, event)) => {
                            // The v1 volume packet carries no sequence number
                            // and does not consume one (notify.c:156-159,
                            // 178), so it says nothing about loss.
                            let lost =
                                !matches!(event, Event::MasterVolume(_)) && seqs.observe(seq);
                            Notification {
                                seq,
                                event,
                                lost,
                                origin,
                            }
                        }
                        // A packet we cannot read is still a packet we
                        // missed; say so through the loss flag.
                        Err(_) => Notification {
                            seq: 0,
                            event: Event::Unknown {
                                id: 0xFF,
                                bytes: packet,
                            },
                            lost: true,
                            origin,
                        },
                    };
                    if !deliver(n) {
                        break;
                    }
                }
            })?;
        Ok(Self {
            rx,
            stop,
            thread: Some(thread),
            disconnected,
        })
    }

    /// Everything that has arrived since the last drain, oldest first.
    pub fn drain(&self) -> Vec<Notification> {
        let mut out = Vec::new();
        while let Ok(n) = self.rx.try_recv() {
            out.push(n);
        }
        out
    }

    /// Block for the next notification, up to `timeout`.
    pub fn next(&self, timeout: Duration) -> Option<Notification> {
        self.rx.recv_timeout(timeout).ok()
    }

    pub fn is_disconnected(&self) -> bool {
        self.disconnected.load(Ordering::Relaxed)
    }
}

impl Drop for Notifications {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dspi_transport::MockTransport;
    use dspi_transport::Transport;

    fn param(seq: u8, offset: u16, source: u8, bytes: &[u8]) -> Vec<u8> {
        let mut p = vec![2, evt::PARAM_CHANGED, 0, seq];
        p.extend(offset.to_le_bytes());
        p.extend((bytes.len() as u16).to_le_bytes());
        p.extend([source, 0, 0, 0]);
        p.extend(bytes);
        p
    }

    #[test]
    fn a_param_change_carries_offset_source_and_bytes() {
        // user_volume.user_volume_db at 4748 (bulk_params.h), from UAC1.
        let p = param(7, 4748, 7, &(-12.0f32).to_le_bytes());
        let (seq, e) = decode(&p).unwrap();
        assert_eq!(seq, 7);
        assert_eq!(
            e,
            Event::ParamChanged {
                offset: 4748,
                source: Source::Uac1,
                bytes: (-12.0f32).to_le_bytes().to_vec()
            }
        );
    }

    #[test]
    fn discrete_events_decode_by_id() {
        assert_eq!(
            decode(&[2, 3, 0, 9, 3, 0, 0, 0]).unwrap().1,
            Event::BulkInvalidated {
                source: Source::Preset
            }
        );
        assert_eq!(
            decode(&[2, 4, 0, 10, 5, 0, 0, 0]).unwrap().1,
            Event::PresetLoaded { slot: 5 }
        );
        assert_eq!(
            decode(&[2, 5, 0, 11, 8, 0, 0, 0]).unwrap().1,
            Event::InputFormat { channels: 8 }
        );
        assert_eq!(
            decode(&[2, 7, 0, 12, 1, 0, 4, 0xFF]).unwrap().1,
            Event::SiggenState {
                state: 1,
                reason: 0,
                signal_type: 4,
                channel: 0xFF
            }
        );
        assert_eq!(
            decode(&[2, 8, 0, 13, 1, 1, 12, 0]).unwrap().1,
            Event::AdatState {
                enabled: true,
                active: true,
                pin: 12
            }
        );
        assert_eq!(
            decode(&[2, 9, 0, 14, 2, 0x80, 0xBB, 0, 0]).unwrap().1,
            Event::I2sSlaveState {
                state: 2,
                rate_hz: 48_000
            }
        );
        assert_eq!(
            decode(&[2, 0x0A, 0, 15, 2, 1, 0, 0, 0x78, 0x56, 0x34, 0x12])
                .unwrap()
                .1,
            Event::IrLearn {
                state: 2,
                protocol: 1,
                code: 0x1234_5678
            }
        );
        assert_eq!(
            decode(&[2, 0x0B, 0, 16, 2, 0x44, 0xAC, 0, 0, 1]).unwrap().1,
            Event::AdatInputState {
                state: 2,
                rate_hz: 44_100,
                clock_mode: 1
            }
        );
        assert_eq!(
            decode(&[2, 0x42, 0, 17, 9]).unwrap().1,
            Event::Unknown {
                id: 0x42,
                bytes: vec![9]
            }
        );
    }

    /// `NOTIFY_EVT_CS_AUX`, 9 bytes: slot, state, the level as 8.8 percent
    /// and the source (notify.h:78-83).
    #[test]
    fn an_aux_output_change_decodes_slot_state_level_and_source() {
        let p = [2, 0x0C, 0, 21, 5, 1, 0x00, 0x4B, 5];
        assert_eq!(
            decode(&p).unwrap(),
            (
                21,
                Event::CsAux {
                    slot: 5,
                    state: 1,
                    level_q8: 75 * 256,
                    source: Source::Gpio
                }
            )
        );
        assert!(matches!(
            decode(&p[..8]).unwrap_err(),
            DecodeError::Short {
                id: 0x0C,
                need: 9,
                ..
            }
        ));
    }

    /// The ids and source tags are the generated ones: `notify.h` is vendored
    /// and read by the generator, so these are the header's numbers.
    #[test]
    fn event_ids_and_sources_are_the_headers() {
        assert_eq!(evt::CS_AUX, 0x0C);
        assert_eq!(evt::ADAT_INPUT_STATE, 0x0B);
        assert_eq!(evt::SIGGEN_STATE, 0x07);
        assert_eq!(PROTOCOL_VERSION, 2);
        assert_eq!(Source::from_raw(9), Source::I2c);
        assert_eq!(Source::from_raw(10), Source::Midi);
        assert_eq!(Source::from_raw(11), Source::Other(11));
    }

    #[test]
    fn idle_and_legacy_packets_are_recognised_by_shape() {
        assert_eq!(decode(&[0]).unwrap().1, Event::Idle);
        assert_eq!(decode(&[]).unwrap().1, Event::Idle);
        let mut legacy = vec![1, 0, 0, 0];
        legacy.extend((-20.0f32).to_le_bytes());
        assert_eq!(decode(&legacy).unwrap().1, Event::MasterVolume(-20.0));
        assert_eq!(decode(&[3, 2, 0, 0]).unwrap_err(), DecodeError::Version(3));
        assert!(matches!(
            decode(&[2, 2, 0, 0, 1, 2]).unwrap_err(),
            DecodeError::Short { .. }
        ));
    }

    #[test]
    fn sequence_gaps_are_reported_and_wrap_is_not_a_gap() {
        let mut t = SeqTracker::default();
        assert!(!t.observe(254));
        assert!(!t.observe(255));
        assert!(!t.observe(0), "255 to 0 is the wrap, not a loss");
        assert!(t.observe(2), "1 was skipped");
        t.reset();
        assert!(!t.observe(9));
    }

    #[test]
    fn the_reader_delivers_events_and_flags_a_loss() {
        let mock = MockTransport::new();
        mock.push_notification(param(1, 4748, 5, &[0, 0, 0, 0]));
        mock.push_notification(vec![0]);
        mock.push_notification(vec![2, 3, 0, 3, 1, 0, 0, 0]);
        let n = Notifications::start(mock.notifications().unwrap()).unwrap();
        let first = n.next(Duration::from_secs(2)).expect("first");
        assert_eq!(first.seq, 1);
        assert!(!first.lost);
        assert!(matches!(
            first.event,
            Event::ParamChanged {
                source: Source::Gpio,
                ..
            }
        ));
        let second = n.next(Duration::from_secs(2)).expect("second");
        assert_eq!(second.seq, 3);
        assert!(second.lost, "seq 2 never arrived");
        assert!(matches!(second.event, Event::BulkInvalidated { .. }));
        assert!(n.next(Duration::from_millis(50)).is_none());
        assert!(!n.is_disconnected());
    }

    /// A master-volume change pushes the v1 packet, which has no sequence
    /// number, just before its v2 `PARAM_CHANGED` (usb_audio.c:367-375,
    /// notify.c:156-159). It must not read as a loss, before or after.
    #[test]
    fn the_legacy_volume_packet_is_not_a_loss() {
        let mock = MockTransport::new();
        let mut legacy = vec![1, 0, 0, 0];
        legacy.extend((-20.0f32).to_le_bytes());
        mock.push_notification(param(7, 4748, 7, &[0, 0, 0, 0]));
        mock.push_notification(legacy.clone());
        mock.push_notification(param(8, 4748, 7, &[0, 0, 0, 0]));
        mock.push_notification(legacy);
        mock.push_notification(param(9, 4748, 7, &[0, 0, 0, 0]));
        let n = Notifications::start(mock.notifications().unwrap()).unwrap();
        for _ in 0..5 {
            let got = n.next(Duration::from_secs(2)).expect("a packet");
            assert!(!got.lost, "{got:?}");
        }
    }

    #[test]
    fn echo_filter_claims_our_bulk_invalidation_once() {
        use dspi_transport::TransportKind as K;
        let bulk = n::PARAM_SRC_BULK_SET as u8;
        let t0 = Instant::now();
        let mut f = EchoFilter::default();
        assert!(!f.classify(t0, K::Usb, bulk, None, None), "idle: foreign");

        // Invalidation after the reply, inside the window: ours, once.
        f.bulk_write_started();
        f.bulk_write_finished(t0, true);
        assert!(f.own_bulk_pending(t0 + Duration::from_millis(900)));
        assert!(f.classify(t0 + Duration::from_millis(10), K::Midi, bulk, None, None));
        assert!(!f.classify(t0 + Duration::from_millis(20), K::Midi, bulk, None, None));

        // Invalidation while in flight: ours; the reply then opens no window.
        f.bulk_write_started();
        assert!(f.classify(t0, K::Usb, bulk, None, None));
        f.bulk_write_finished(t0, true);
        assert!(!f.classify(t0, K::Usb, bulk, None, None));

        // Past the window, or after a failed write: foreign.
        f.bulk_write_started();
        f.bulk_write_finished(t0, true);
        assert!(!f.classify(t0 + EchoFilter::WINDOW, K::Usb, bulk, None, None));
        f.bulk_write_started();
        f.bulk_write_finished(t0, false);
        assert!(!f.classify(t0, K::Usb, bulk, None, None));

        // A HOST_SET over USB is ours regardless, and does not consume.
        f.bulk_write_started();
        assert!(f.classify(t0, K::Usb, n::PARAM_SRC_HOST_SET as u8, None, None));
        assert!(f.own_bulk_pending(t0));
    }

    /// A reader that is not drained must not grow without bound: past
    /// [`Notifications::QUEUE`] packets are dropped, and the next one
    /// delivered says so, which makes the client re-read the image.
    #[test]
    fn a_full_queue_drops_and_marks_the_next_packet_lost() {
        let mock = MockTransport::new();
        let total = Notifications::QUEUE + 50;
        for i in 0..total {
            mock.push_notification(param(i as u8, 4748, 7, &[0, 0, 0, 0]));
        }
        let queue = mock.notify_queue();
        let n = Notifications::start(mock.notifications().unwrap()).unwrap();
        let t0 = std::time::Instant::now();
        while !queue.lock().unwrap().is_empty() {
            assert!(t0.elapsed() < Duration::from_secs(10), "the reader stalled");
            std::thread::sleep(Duration::from_millis(5));
        }
        std::thread::sleep(Duration::from_millis(50));
        assert_eq!(n.drain().len(), Notifications::QUEUE);
        mock.push_notification(param(total as u8, 4748, 7, &[0, 0, 0, 0]));
        let next = n
            .next(Duration::from_secs(2))
            .expect("a packet after the overflow");
        assert!(next.lost, "the overflow is reported as a loss");
    }

    /// Beta4 paces the endpoint: an idle read blocks for the 100 ms
    /// keep-alive, then answers the idle packet. The reader must deliver
    /// events that arrive between keep-alives, drop the idles, and still stop
    /// promptly when it is dropped.
    #[test]
    fn a_paced_endpoint_delivers_events_and_stops_promptly() {
        assert!(Notifications::READ_TIMEOUT > Notifications::KEEP_ALIVE);
        let mock = MockTransport::new().with_notification_pace(Notifications::KEEP_ALIVE);
        let n = Notifications::start(mock.notifications().unwrap()).unwrap();

        // Nothing but keep-alives: nothing is delivered.
        assert!(n.next(Duration::from_millis(250)).is_none());

        mock.push_notification(param(1, 5980, 5, &[1]));
        let got = n.next(Duration::from_secs(2)).expect("the tube change");
        assert!(matches!(
            got.event,
            Event::ParamChanged { offset: 5980, .. }
        ));
        assert!(!got.lost);

        mock.push_notification(vec![2, 0x0C, 0, 2, 3, 0, 0, 0, 1]);
        let got = n.next(Duration::from_secs(2)).expect("the aux change");
        assert!(matches!(got.event, Event::CsAux { slot: 3, .. }));
        assert!(!got.lost, "idle packets carry no sequence number");
        assert!(!n.is_disconnected());

        // Dropping joins the thread; a blocked read must not hold it for long.
        let start = std::time::Instant::now();
        drop(n);
        assert!(
            start.elapsed() < Notifications::READ_TIMEOUT + Duration::from_millis(250),
            "shutdown took {:?}",
            start.elapsed()
        );
    }

    #[test]
    fn a_disconnect_stops_the_reader_and_is_visible() {
        let mock = MockTransport::new();
        mock.push_notification(dspi_transport::mock::MockNotifications::DISCONNECT.to_vec());
        let n = Notifications::start(mock.notifications().unwrap()).unwrap();
        std::thread::sleep(Duration::from_millis(50));
        assert!(n.is_disconnected());
    }

    #[test]
    fn sources_describe_themselves_in_the_consoles_words() {
        assert_eq!(Source::from_raw(7), Source::Uac1);
        assert_eq!(Source::from_raw(42), Source::Other(42));
        assert!(Source::HostSet.is_ours());
        for b in 0..=255u8 {
            assert_eq!(Source::from_raw(b).raw(), b, "raw round-trips");
        }
        assert_eq!(Source::from_raw(10), Source::Midi);
        assert!(!Source::Gpio.is_ours());
        assert_eq!(Source::Gpio.describe(), "changed by control surface");
    }
}
