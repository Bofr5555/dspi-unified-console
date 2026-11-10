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
//! blocks on the endpoint. Where there are no threads (`wasm32-unknown-unknown`)
//! it is caller-driven instead: drains read the source, and the caller can push
//! packets it read itself (WP WB2, [`Notifications::caller_driven`]).

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::thread::JoinHandle;
use std::time::Duration;

use dspi_transport::clock::Instant;

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

    /// What the source byte itself says, in words for the echo line. It does
    /// not say whose the change was: that depends on the link, so ask
    /// [`Notification::is_own_echo`] first.
    pub fn describe(self) -> &'static str {
        match self {
            Self::Unknown | Self::Other(_) => "changed elsewhere",
            Self::HostSet => "changed over USB",
            Self::BulkSet => "changed by a bulk write",
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
    /// When the reader thread received the packet. Echo classification
    /// ([`EchoFilter::classify_notification`]) uses this, not the time the
    /// client drains the queue, so a late drain cannot push our own bulk
    /// write's `BULK_SET` outside its window and misread it as foreign.
    pub received: Instant,
}

impl Notification {
    /// Whether this is the echo of a write this client made, by the rule of
    /// the link it came over ([`dspi_transport::is_own_write`]): `kind` is
    /// the transport's [`dspi_transport::TransportCaps::kind`], `session` its
    /// [`dspi_transport::Transport::session_id`]. A `BULK_INVALIDATED` off Link
    /// follows `echo`'s rule, at the packet's `received` stamp: the source that
    /// answers a bulk write in the filter's mode, with one of ours pending; a
    /// preset load, a factory reset or a merged `UNKNOWN` never is ours. This
    /// only asks: [`EchoFilter::classify_notification`] also consumes the
    /// write it answers. Such a change is already in the client's mirror.
    pub fn is_own_echo(
        &self,
        kind: dspi_transport::TransportKind,
        session: Option<u16>,
        echo: &EchoFilter,
    ) -> bool {
        match &self.event {
            Event::BulkInvalidated { source } if kind != dspi_transport::TransportKind::Link => {
                echo.answers_our_bulk(self.received, kind, source.raw())
            }
            Event::ParamChanged { source, .. } | Event::BulkInvalidated { source } => {
                dspi_transport::is_own_write(kind, source.raw(), self.origin, session, false)
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

/// Recognises the `BULK_INVALIDATED` that answers our own bulk write. Over
/// Link the origin decides and this state is not consulted; on USB, MIDI and
/// the fake device there are two modes, chosen once per connection with
/// [`EchoFilter::set_bulk_source`].
///
/// **Tagged** (firmware reporting `NOTIFY_FEAT_BULK_SOURCE`, sub-command
/// `EXT_NOTIFY_FEATURES`; `notification_protocol_v2_spec.md` 3.5.1): an
/// invalidation is ours only when its source is the path we write on
/// (`HOST_SET` over USB, `MIDI` over MIDI) and a bulk SET of ours on that path
/// is outstanding; a match settles every outstanding SET, because a merged
/// invalidation can stand for several. Any other source (`UNKNOWN`, `BULK_SET`,
/// another path) is foreign. Several hosts can share a path, so the
/// outstanding-SET condition is what keeps another app's write from reading
/// as ours. An outstanding SET whose invalidation never comes (the device
/// refused the image on its main loop) expires after
/// [`EchoFilter::SOURCE_EXPIRY`], so it cannot hide a later foreign write.
///
/// **Time window** (older or stock firmware, or the feature read was refused):
/// the firmware tags every bulk apply `BULK_SET` whatever path it came in on
/// (bulk_params.c:371/408), so the source cannot tell our `0xA1`/`0xA3` from
/// another host's. The device's bulk lock keeps other hosts' bulk writes out
/// while ours is in flight and for a moment after, so: a `BULK_SET`
/// invalidation that arrives while our bulk write is in flight, or within
/// [`EchoFilter::WINDOW`] of its successful reply, is ours, **once**.
///
/// A hole no client can close: another host's SET on our path, merged with ours
/// into one `HOST_SET` entry, cannot be told apart from ours alone, so its
/// change goes unread; [`EchoFilter::SOURCE_EXPIRY`] bounds how long a pending
/// SET can be taken for such a write.
///
/// In both modes an invalidation received before our write started is foreign,
/// and everything unclaimed is foreign: it only costs the client a re-read.
///
/// Pure (the caller passes the clock); reference for the window: the iOS
/// console's `MIDITransport` (`dspi-ios` patch console-116/0005).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EchoFilter {
    /// The device tags bulk applies with their path.
    bulk_source: bool,
    /// Time-window mode's state.
    state: BulkEcho,
    /// Tagged mode's state: our bulk SETs whose invalidation has not come.
    outstanding: u32,
    /// When the first outstanding SET started.
    since: Option<Instant>,
    /// When the outstanding SETs expire; `None` while one is in flight.
    expires: Option<Instant>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum BulkEcho {
    /// No bulk write of ours to account for.
    #[default]
    Idle,
    /// Our bulk write is on the wire; `claimed` once its invalidation was seen.
    InFlight { since: Instant, claimed: bool },
    /// Our bulk write was accepted; its invalidation may still arrive until
    /// `until`.
    Window { since: Instant, until: Instant },
}

impl EchoFilter {
    /// How long after our bulk write's reply its invalidation is still
    /// expected, on firmware that does not tag bulk applies.
    pub const WINDOW: Duration = Duration::from_secs(1);

    /// How long an outstanding SET waits for its tagged invalidation after
    /// the reply. Generous: the apply runs on the device's main loop, and
    /// expiring only costs a re-read.
    pub const SOURCE_EXPIRY: Duration = Duration::from_secs(5);

    /// Choose the mode: `true` when the device reports
    /// `NOTIFY_FEAT_BULK_SOURCE` ([`crate::ext_state::ExtState::bulk_source`]).
    /// Absent or refused means the time window.
    pub fn set_bulk_source(&mut self, tagged: bool) {
        self.bulk_source = tagged;
    }

    /// Whether this filter runs in tagged mode.
    pub fn bulk_source(&self) -> bool {
        self.bulk_source
    }

    /// Our `0xA1` (or the first `0xA3` chunk) is about to be sent.
    pub fn bulk_write_started(&mut self, now: Instant) {
        self.state = BulkEcho::InFlight {
            since: now,
            claimed: false,
        };
        if self.outstanding == 0 {
            self.since = Some(now);
        }
        self.outstanding = self.outstanding.saturating_add(1);
        self.expires = None;
    }

    /// Our bulk write finished (`ok`: the device accepted it). Opens the
    /// window unless the invalidation already came, or the write failed; a
    /// failed write is no longer outstanding, whether or not it reached the
    /// device (if it did, the re-read it causes is safe).
    pub fn bulk_write_finished(&mut self, now: Instant, ok: bool) {
        self.state = match self.state {
            BulkEcho::InFlight {
                since,
                claimed: false,
            } if ok => match now.checked_add(Self::WINDOW) {
                Some(until) => BulkEcho::Window { since, until },
                None => BulkEcho::Idle,
            },
            _ => BulkEcho::Idle,
        };
        if !ok {
            self.outstanding = self.outstanding.saturating_sub(1);
        }
        if self.outstanding == 0 {
            self.since = None;
            self.expires = None;
        } else {
            self.expires = now.checked_add(Self::SOURCE_EXPIRY);
        }
    }

    /// Whether a bulk invalidation received at `now` could be claimed as ours.
    pub fn own_bulk_pending(&self, now: Instant) -> bool {
        if self.bulk_source {
            return self.outstanding > 0
                && self.since.is_some_and(|since| now >= since)
                && self.expires.is_none_or(|until| now < until);
        }
        match self.state {
            BulkEcho::Idle => false,
            BulkEcho::InFlight { since, claimed } => !claimed && now >= since,
            BulkEcho::Window { since, until } => now >= since && now < until,
        }
    }

    /// Whether a `BULK_INVALIDATED` with this raw source, received at `now`,
    /// answers a bulk write of ours (off Link): the mode's source, with one
    /// pending.
    fn answers_our_bulk(
        &self,
        now: Instant,
        kind: dspi_transport::TransportKind,
        source: u8,
    ) -> bool {
        use dspi_transport::TransportKind as K;
        let tag = match (self.bulk_source, kind) {
            (false, _) => n::PARAM_SRC_BULK_SET,
            (true, K::Midi) => n::PARAM_SRC_MIDI,
            (true, _) => n::PARAM_SRC_HOST_SET,
        };
        u16::from(source) == tag && self.own_bulk_pending(now)
    }

    /// Nothing of ours is outstanding any more (tagged mode).
    fn settle_outstanding(&mut self) {
        self.outstanding = 0;
        self.since = None;
        self.expires = None;
    }

    /// Classify a `BULK_INVALIDATED` received at `now` by its raw source:
    /// true if it is our own echo, and the client skips the re-read. A claim
    /// consumes what it answers (see the type's docs).
    pub fn classify_invalidation(
        &mut self,
        now: Instant,
        kind: dspi_transport::TransportKind,
        source: u8,
        origin: Option<u16>,
        session: Option<u16>,
    ) -> bool {
        if kind == dspi_transport::TransportKind::Link {
            return dspi_transport::is_own_write(kind, source, origin, session, false);
        }
        // A mixed-source merge stands for every rewrite in it, ours included,
        // and is re-read. Left outstanding, our SET would let another app's
        // later HOST_SET read as ours.
        if self.bulk_source
            && u16::from(source) == n::PARAM_SRC_UNKNOWN
            && self.own_bulk_pending(now)
        {
            self.settle_outstanding();
            return false;
        }
        if !self.answers_our_bulk(now, kind, source) {
            return false;
        }
        if self.bulk_source {
            self.settle_outstanding();
        }
        self.state = match self.state {
            BulkEcho::InFlight { since, .. } => BulkEcho::InFlight {
                since,
                claimed: true,
            },
            _ => BulkEcho::Idle,
        };
        true
    }

    /// Classify any other event's source: true if it is our own echo. Takes
    /// no bulk state: `PARAM_CHANGED` never answers a bulk write.
    pub fn classify(
        &self,
        kind: dspi_transport::TransportKind,
        source: u8,
        origin: Option<u16>,
        session: Option<u16>,
    ) -> bool {
        dspi_transport::is_own_write(kind, source, origin, session, false)
    }

    /// Classify a decoded notification at the time the reader received it
    /// ([`Notification::received`], not the time of the drain); false for
    /// events without a source.
    pub fn classify_notification(
        &mut self,
        kind: dspi_transport::TransportKind,
        session: Option<u16>,
        note: &Notification,
    ) -> bool {
        match &note.event {
            Event::BulkInvalidated { source } => {
                self.classify_invalidation(note.received, kind, source.raw(), note.origin, session)
            }
            Event::ParamChanged { source, .. } => {
                self.classify(kind, source.raw(), note.origin, session)
            }
            _ => false,
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

/// What the reader does with each packet, on its thread or for a caller-driven
/// reader (WP WB2): drop idle packets, decode, track the sequence, queue.
struct Reader {
    tx: mpsc::SyncSender<Notification>,
    seqs: SeqTracker,
    /// Set when the queue was full and a packet was dropped.
    overflowed: bool,
    waker: Option<Waker>,
}

impl Reader {
    /// Queue one notification; false when the receiver is gone.
    fn deliver(&mut self, mut n: Notification) -> bool {
        n.lost |= self.overflowed;
        match self.tx.try_send(n) {
            Ok(()) => {
                self.overflowed = false;
                if let Some(w) = &self.waker {
                    w();
                }
                true
            }
            Err(mpsc::TrySendError::Full(_)) => {
                self.overflowed = true;
                true
            }
            Err(mpsc::TrySendError::Disconnected(_)) => false,
        }
    }

    /// One packet as it arrived; false when the receiver is gone.
    fn packet(&mut self, packet: Vec<u8>, origin: Option<u16>) -> bool {
        if packet.is_empty() {
            return true;
        }
        let received = Instant::now();
        let n = match decode(&packet) {
            Ok((_, Event::Idle)) => return true,
            Ok((seq, event)) => {
                // The v1 volume packet carries no sequence number and does
                // not consume one (notify.c:156-159, 178), so it says
                // nothing about loss.
                let lost = !matches!(event, Event::MasterVolume(_)) && self.seqs.observe(seq);
                Notification {
                    seq,
                    event,
                    lost,
                    origin,
                    received,
                }
            }
            // A packet we cannot read is still a packet we missed; say so
            // through the loss flag.
            Err(_) => Notification {
                seq: 0,
                event: Event::Unknown {
                    id: 0xFF,
                    bytes: packet,
                },
                lost: true,
                origin,
                received,
            },
        };
        self.deliver(n)
    }
}

/// A reader without a thread (WP WB2): the caller's own calls move it.
struct CallerDriven {
    reader: Reader,
    /// Read with a zero timeout on each drain, until it is quiet.
    source: Option<Box<dyn NotificationSource>>,
    // -- WP UPW2 --
    failures: dspi_transport::ReadFailures,
}

/// The reader thread and its channel.
pub struct Notifications {
    rx: Receiver<Notification>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
    /// Set by the reader when the device went away.
    disconnected: Arc<AtomicBool>,
    /// `Some` for a reader without a thread ([`Notifications::caller_driven`]).
    caller: Option<std::sync::Mutex<CallerDriven>>,
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
    ///
    /// On `wasm32-unknown-unknown`, which has no threads, this is
    /// [`Notifications::caller_driven`] and `waker` is never called (WP WB2).
    #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
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
                let mut reader = Reader {
                    tx,
                    seqs: SeqTracker::default(),
                    overflowed: false,
                    waker,
                };
                // -- WP UPW2 --
                let mut failures = dspi_transport::ReadFailures::default();
                while !stop_flag.load(Ordering::Relaxed) {
                    let (packet, origin) = match source.read_attributed(Self::READ_TIMEOUT) {
                        Ok(a) => {
                            failures.worked();
                            (a.packet, a.origin)
                        }
                        Err(e) => {
                            // A stall or a transient error: give the bus a
                            // moment rather than spinning. A run of them is
                            // a dead handle, given up as an unplug is (WP UPW2).
                            let gone = matches!(e, dspi_transport::TransportError::Disconnected);
                            if !gone && !failures.failed() {
                                dspi_transport::clock::sleep(Duration::from_millis(50));
                                continue;
                            }
                            disc_flag.store(true, Ordering::Relaxed);
                            // Wake the client once so it notices the end
                            // (MIDI port closed, Link session over) instead
                            // of waiting for a packet that never comes.
                            if let Some(w) = &reader.waker {
                                w();
                            }
                            break;
                        }
                    };
                    if !reader.packet(packet, origin) {
                        break;
                    }
                }
            })?;
        Ok(Self {
            rx,
            stop,
            thread: Some(thread),
            disconnected,
            caller: None,
        })
    }

    /// The web's [`Notifications::start_with_waker`]: no thread (WP WB2).
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    pub fn start_with_waker(
        source: Box<dyn NotificationSource>,
        _waker: Option<Waker>,
    ) -> std::io::Result<Self> {
        Ok(Self::caller_driven(Some(source)))
    }

    /// A reader with no thread (WP WB2), for a target without threads. Packets
    /// come in two ways, both on the caller's own calls:
    /// - [`Notifications::drain`] and [`Notifications::next`] first read
    ///   `source` with a zero timeout until it is quiet (at most
    ///   [`Self::QUEUE`] packets per call);
    /// - [`Notifications::push`] hands in a packet the caller read itself
    ///   (a page's own endpoint or socket loop).
    ///
    /// No waker is called: whoever reads or pushes knows to drain. Decoding,
    /// the sequence check, the queue limit and the disconnect flag are the
    /// reader thread's.
    pub fn caller_driven(source: Option<Box<dyn NotificationSource>>) -> Self {
        let (tx, rx) = mpsc::sync_channel(Self::QUEUE);
        Self {
            rx,
            stop: Arc::new(AtomicBool::new(false)),
            thread: None,
            disconnected: Arc::new(AtomicBool::new(false)),
            caller: Some(std::sync::Mutex::new(CallerDriven {
                reader: Reader {
                    tx,
                    seqs: SeqTracker::default(),
                    overflowed: false,
                    waker: None,
                },
                source,
                failures: dspi_transport::ReadFailures::default(),
            })),
        }
    }

    /// Hand in one packet as the device sent it, with the link's origin, for
    /// a [`Notifications::caller_driven`] reader. False, and nothing queued,
    /// for a reader with a thread (it reads its own source).
    pub fn push(&self, packet: Vec<u8>, origin: Option<u16>) -> bool {
        let Some(caller) = &self.caller else {
            return false;
        };
        caller
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .reader
            .packet(packet, origin);
        true
    }

    /// A caller-driven reader's turn at its source: read what is waiting.
    fn pump(&self) {
        let Some(caller) = &self.caller else {
            return;
        };
        let mut c = caller
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let CallerDriven {
            reader,
            source,
            failures,
        } = &mut *c;
        let Some(src) = source.as_mut() else {
            return;
        };
        for _ in 0..Self::QUEUE {
            let read = src.read_attributed(Duration::ZERO);
            if read.is_ok() {
                failures.worked();
            }
            match read {
                Ok(a) if a.packet.is_empty() => return,
                Ok(a) => {
                    if !reader.packet(a.packet, a.origin) {
                        return;
                    }
                }
                Err(dspi_transport::TransportError::Disconnected) => {
                    self.disconnected.store(true, Ordering::Relaxed);
                    *source = None;
                    return;
                }
                // A stall or a transient error: try again on the next call,
                // unless a run of them says the handle is dead (WP UPW2).
                Err(_) => {
                    if failures.failed() {
                        self.disconnected.store(true, Ordering::Relaxed);
                        *source = None;
                    }
                    return;
                }
            }
        }
    }

    /// Everything that has arrived since the last drain, oldest first.
    pub fn drain(&self) -> Vec<Notification> {
        self.pump();
        let mut out = Vec::new();
        while let Ok(n) = self.rx.try_recv() {
            out.push(n);
        }
        out
    }

    /// Block for the next notification, up to `timeout`. A caller-driven
    /// reader does not wait: it reads what is there.
    pub fn next(&self, timeout: Duration) -> Option<Notification> {
        if self.caller.is_some() {
            self.pump();
            return self.rx.try_recv().ok();
        }
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
        let c =
            |f: &mut EchoFilter, at, kind, src| f.classify_invalidation(at, kind, src, None, None);
        assert!(!c(&mut f, t0, K::Usb, bulk), "idle: foreign");

        // Invalidation after the reply, inside the window: ours, once.
        f.bulk_write_started(t0);
        f.bulk_write_finished(t0, true);
        assert!(f.own_bulk_pending(t0 + Duration::from_millis(900)));
        assert!(c(&mut f, t0 + Duration::from_millis(10), K::Midi, bulk));
        assert!(!c(&mut f, t0 + Duration::from_millis(20), K::Midi, bulk));

        // Invalidation while in flight: ours; the reply then opens no window.
        f.bulk_write_started(t0);
        assert!(c(&mut f, t0, K::Usb, bulk));
        f.bulk_write_finished(t0, true);
        assert!(!c(&mut f, t0, K::Usb, bulk));

        // Past the window, or after a failed write: foreign.
        f.bulk_write_started(t0);
        f.bulk_write_finished(t0, true);
        assert!(!c(&mut f, t0 + EchoFilter::WINDOW, K::Usb, bulk));
        f.bulk_write_started(t0);
        f.bulk_write_finished(t0, false);
        assert!(!c(&mut f, t0, K::Usb, bulk));

        // Received before the write started (drained late): foreign.
        let t1 = t0 + Duration::from_secs(10);
        f.bulk_write_started(t1);
        assert!(!c(&mut f, t0, K::Usb, bulk));
        assert!(c(&mut f, t1, K::Usb, bulk));

        // Without the feature no other source answers a bulk write.
        f.bulk_write_started(t1);
        assert!(!c(&mut f, t1, K::Usb, n::PARAM_SRC_HOST_SET as u8));
        assert!(!c(&mut f, t1, K::Midi, n::PARAM_SRC_MIDI as u8));
        assert!(f.own_bulk_pending(t1), "and nothing was consumed");
    }

    /// `notification_protocol_v2_spec.md` 3.5.1, the host rule.
    #[test]
    fn echo_filter_with_tagged_firmware_follows_the_host_rule() {
        use dspi_transport::TransportKind as K;
        let (host, midi, bulk, unknown, preset) = (
            n::PARAM_SRC_HOST_SET as u8,
            n::PARAM_SRC_MIDI as u8,
            n::PARAM_SRC_BULK_SET as u8,
            n::PARAM_SRC_UNKNOWN as u8,
            n::PARAM_SRC_PRESET as u8,
        );
        let t0 = Instant::now();
        let tagged = || {
            let mut f = EchoFilter::default();
            f.set_bulk_source(true);
            f
        };
        let c =
            |f: &mut EchoFilter, at, kind, src| f.classify_invalidation(at, kind, src, None, None);

        // Our path with a SET outstanding: ours; nothing outstanding: another
        // app on the same path.
        let mut f = tagged();
        assert!(!c(&mut f, t0, K::Usb, host), "no SET of ours");
        f.bulk_write_started(t0);
        f.bulk_write_finished(t0, true);
        assert!(c(&mut f, t0, K::Usb, host));
        assert!(!c(&mut f, t0, K::Usb, host), "settled");

        // Any other source is foreign and settles nothing.
        f.bulk_write_started(t0);
        for src in [midi, bulk, preset] {
            assert!(!c(&mut f, t0, K::Usb, src), "source {src}");
        }
        assert!(f.own_bulk_pending(t0));

        // A merged entry stands for our SET too: it re-reads and settles it, so
        // another app's later HOST_SET is not taken for ours.
        let mut f = tagged();
        f.bulk_write_started(t0);
        f.bulk_write_finished(t0, true);
        assert!(!c(&mut f, t0, K::Usb, unknown));
        assert!(!f.own_bulk_pending(t0));
        assert!(!c(&mut f, t0, K::Usb, host));
        // With nothing pending it settles nothing and a later SET is unaffected.
        let mut f = tagged();
        assert!(!c(&mut f, t0, K::Usb, unknown));
        f.bulk_write_started(t0);
        assert!(c(&mut f, t0, K::Usb, host));

        // Over MIDI the path is MIDI.
        let mut f = tagged();
        f.bulk_write_started(t0);
        assert!(!c(&mut f, t0, K::Midi, host));
        assert!(c(&mut f, t0, K::Midi, midi));

        // A match settles every outstanding SET; a later one is foreign.
        let mut f = tagged();
        f.bulk_write_started(t0);
        f.bulk_write_finished(t0, true);
        f.bulk_write_started(t0);
        f.bulk_write_finished(t0, true);
        assert!(c(&mut f, t0, K::Usb, host));
        assert!(!c(&mut f, t0, K::Usb, host));

        // A failed SET is not outstanding; the other one still is.
        let mut f = tagged();
        f.bulk_write_started(t0);
        f.bulk_write_finished(t0, true);
        f.bulk_write_started(t0);
        f.bulk_write_finished(t0, false);
        assert!(c(&mut f, t0, K::Usb, host));
        let mut f = tagged();
        f.bulk_write_started(t0);
        f.bulk_write_finished(t0, false);
        assert!(!c(&mut f, t0, K::Usb, host));

        // Invalidation before the reply: the reply does not re-arm anything.
        let mut f = tagged();
        f.bulk_write_started(t0);
        assert!(c(&mut f, t0, K::Usb, host));
        f.bulk_write_finished(t0, true);
        assert!(!c(&mut f, t0, K::Usb, host));

        // No time window in this mode, but a device that never answered
        // cannot hide a foreign write for ever.
        let mut f = tagged();
        f.bulk_write_started(t0);
        f.bulk_write_finished(t0, true);
        assert!(f.own_bulk_pending(t0 + EchoFilter::WINDOW + Duration::from_secs(1)));
        assert!(!c(&mut f, t0 + EchoFilter::SOURCE_EXPIRY, K::Usb, host));

        // Received before the SET started: foreign.
        let mut f = tagged();
        let t1 = t0 + Duration::from_secs(10);
        f.bulk_write_started(t1);
        assert!(!c(&mut f, t0, K::Usb, host));
    }

    /// `HOST_SET` on a `PARAM_CHANGED` is ours over USB, but a tagged
    /// `BULK_INVALIDATED` with `HOST_SET` is another app's unless we have a
    /// SET out: the event type decides which rule applies.
    #[test]
    fn echo_filter_tells_param_changes_from_invalidations() {
        use dspi_transport::TransportKind as K;
        let mut f = EchoFilter::default();
        f.set_bulk_source(true);
        let note = |event| Notification {
            seq: 0,
            event,
            lost: false,
            origin: None,
            received: Instant::now(),
        };
        let inval = note(Event::BulkInvalidated {
            source: Source::HostSet,
        });
        let param = note(Event::ParamChanged {
            offset: 16,
            source: Source::HostSet,
            bytes: vec![0],
        });
        assert!(!f.classify_notification(K::Usb, None, &inval));
        assert!(f.classify_notification(K::Usb, None, &param));
        assert!(!inval.is_own_echo(K::Usb, None, &f));
        assert!(param.is_own_echo(K::Usb, None, &f));
        let mut pending = f.clone();
        pending.bulk_write_started(inval.received);
        assert!(inval.is_own_echo(K::Usb, None, &pending));
        assert!(
            pending.own_bulk_pending(inval.received),
            "asking consumes nothing"
        );

        // With a bulk write of ours pending, only the mode's source is ours: not
        // a preset load, a factory reset or a merged entry.
        for source in [
            Source::Preset,
            Source::Factory,
            Source::Unknown,
            Source::BulkSet,
        ] {
            let other = note(Event::BulkInvalidated { source });
            assert!(!other.is_own_echo(K::Usb, None, &pending), "{source:?}");
        }
        let mut window = EchoFilter::default();
        window.bulk_write_started(inval.received);
        assert!(
            !inval.is_own_echo(K::Usb, None, &window),
            "HOST_SET, untagged"
        );
        for source in [Source::Preset, Source::Factory, Source::Unknown] {
            let other = note(Event::BulkInvalidated { source });
            assert!(!other.is_own_echo(K::Usb, None, &window), "{source:?}");
        }
        let bulk = note(Event::BulkInvalidated {
            source: Source::BulkSet,
        });
        assert!(bulk.is_own_echo(K::Usb, None, &window));
    }

    /// The end of the stream (a MIDI port closed, a Link session over)
    /// wakes the client once, so it notices without waiting for a packet.
    #[test]
    fn a_disconnect_wakes_the_client_once() {
        use std::sync::atomic::AtomicUsize;
        let mock = MockTransport::new();
        mock.push_notification(dspi_transport::mock::MockNotifications::DISCONNECT.to_vec());
        let woken = Arc::new(AtomicUsize::new(0));
        let w = Arc::clone(&woken);
        let waker: Waker = Arc::new(move || {
            w.fetch_add(1, Ordering::SeqCst);
        });
        let n =
            Notifications::start_with_waker(mock.notifications().unwrap(), Some(waker)).unwrap();
        let t0 = std::time::Instant::now();
        while !n.is_disconnected() {
            assert!(t0.elapsed() < Duration::from_secs(5), "never disconnected");
            std::thread::sleep(Duration::from_millis(5));
        }
        std::thread::sleep(Duration::from_millis(20));
        assert_eq!(woken.load(Ordering::SeqCst), 1);
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
        for b in 0..=255u8 {
            assert_eq!(Source::from_raw(b).raw(), b, "raw round-trips");
        }
        assert_eq!(Source::from_raw(10), Source::Midi);
        assert_eq!(Source::Gpio.describe(), "changed by control surface");
        // Whose a change is depends on the link, so the byte never claims it.
        for b in 0..=255u8 {
            assert!(!Source::from_raw(b).describe().contains("this host"));
        }
    }
}

// -- WP WB2 --
#[cfg(test)]
mod tests_wp_wb2 {
    use super::*;
    use dspi_transport::MockTransport;
    use dspi_transport::Transport;
    use dspi_transport::mock::MockNotifications;

    fn param(seq: u8) -> Vec<u8> {
        let mut p = vec![2, evt::PARAM_CHANGED, 0, seq];
        p.extend(4748u16.to_le_bytes());
        p.extend(4u16.to_le_bytes());
        p.extend([5, 0, 0, 0, 0, 0, 0, 0]);
        p
    }

    #[test]
    fn wb2_a_drain_reads_the_source_without_a_thread() {
        let mock = MockTransport::new();
        mock.push_notification(param(1));
        mock.push_notification(vec![0]);
        mock.push_notification(param(3));
        let n = Notifications::caller_driven(mock.notifications());
        let got = n.drain();
        assert_eq!(got.iter().map(|g| g.seq).collect::<Vec<_>>(), [1, 3]);
        assert!(!got[0].lost);
        assert!(got[1].lost, "seq 2 never arrived");
        assert!(n.drain().is_empty());
        mock.push_notification(param(4));
        assert_eq!(n.next(Duration::from_secs(5)).map(|g| g.seq), Some(4));
        assert!(n.next(Duration::from_secs(5)).is_none(), "does not wait");
    }

    #[test]
    fn wb2_pushed_packets_are_decoded_like_read_ones() {
        let n = Notifications::caller_driven(None);
        assert!(n.push(param(7), Some(3)));
        assert!(n.push(vec![0], None));
        assert!(
            n.push(vec![9, 9, 9, 9], None),
            "a version this build cannot read"
        );
        let got = n.drain();
        assert_eq!(got.len(), 2, "the idle packet is dropped");
        assert_eq!((got[0].seq, got[0].origin), (7, Some(3)));
        assert!(matches!(got[1].event, Event::Unknown { id: 0xFF, .. }));
        assert!(got[1].lost, "an unreadable packet counts as lost");
    }

    #[test]
    fn wb2_a_full_queue_marks_the_next_delivery_lost() {
        let n = Notifications::caller_driven(None);
        let mut seq = 0u8;
        for _ in 0..=Notifications::QUEUE {
            n.push(param(seq), None);
            seq = seq.wrapping_add(1);
        }
        assert_eq!(n.drain().len(), Notifications::QUEUE);
        n.push(param(seq), None);
        let got = n.drain();
        assert!(got[0].lost, "one was dropped while the queue was full");
    }

    #[test]
    fn wb2_a_disconnect_is_seen_on_the_next_drain() {
        let mock = MockTransport::new();
        mock.push_notification(param(1));
        mock.push_notification(MockNotifications::DISCONNECT.to_vec());
        let n = Notifications::caller_driven(mock.notifications());
        assert!(!n.is_disconnected());
        assert_eq!(n.drain().len(), 1);
        assert!(n.is_disconnected());
        // The source is gone; pushes still work.
        mock.push_notification(param(2));
        assert!(n.drain().is_empty());
        assert!(n.push(param(2), None));
        assert_eq!(n.drain().len(), 1);
    }

    #[test]
    fn wb2_a_threaded_reader_refuses_pushes() {
        let mock = MockTransport::new();
        let n = Notifications::start(mock.notifications().unwrap()).unwrap();
        assert!(!n.push(param(1), None));
    }
}

// -- WP UPW2 --
#[cfg(test)]
mod tests_wp_upw2 {
    use super::*;
    use dspi_transport::{FakeDevice, ReadFailures, Transport};

    fn wait_until(what: &str, mut done: impl FnMut() -> bool) {
        let start = std::time::Instant::now();
        while !done() {
            assert!(start.elapsed() < Duration::from_secs(10), "{what}");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn wp_upw2_a_handle_that_keeps_failing_is_given_up_as_an_unplug() {
        let dev = FakeDevice::demo();
        let remote = dev.remote();
        let woken = Arc::new(AtomicBool::new(false));
        let w = Arc::clone(&woken);
        let waker: Waker = Arc::new(move || w.store(true, Ordering::Relaxed));
        let n = Notifications::start_with_waker(dev.notifications().unwrap(), Some(waker)).unwrap();
        remote.fail_notifications(u32::MAX);
        wait_until("the reader never gave the handle up", || {
            n.is_disconnected()
        });
        assert!(woken.load(Ordering::Relaxed), "the client is woken once");
        // The device itself still answers: only the reader gave up.
        let mut dev = dev;
        dev.control_in(dspi_proto::generated::opcodes::REQ_GET_SERIAL, 0, 16)
            .unwrap();
    }

    #[test]
    fn wp_upw2_a_shorter_run_of_errors_is_ridden_out() {
        let dev = FakeDevice::demo();
        let remote = dev.remote();
        let n = Notifications::start(dev.notifications().unwrap()).unwrap();
        remote.fail_notifications(ReadFailures::LIMIT - 1);
        // A packet behind the errors still arrives, and the reader stays.
        remote.push(vec![2, 0x04, 0, 0, 3, 0, 0, 0]);
        let got = n.next(Duration::from_secs(10)).expect("the preset event");
        assert!(matches!(got.event, Event::PresetLoaded { slot: 3 }));
        assert!(!n.is_disconnected());
        // A working read reset the count: another short run is ridden out too.
        remote.fail_notifications(ReadFailures::LIMIT - 1);
        remote.push(vec![2, 0x04, 0, 0, 4, 0, 0, 0]);
        assert!(n.next(Duration::from_secs(10)).is_some());
        assert!(!n.is_disconnected());
        drop(dev);
    }

    #[test]
    fn wp_upw2_a_caller_driven_reader_gives_a_dead_handle_up_too() {
        let dev = FakeDevice::demo();
        let remote = dev.remote();
        let n = Notifications::caller_driven(dev.notifications());
        remote.fail_notifications(u32::MAX);
        for _ in 1..ReadFailures::LIMIT {
            assert!(n.drain().is_empty());
            assert!(!n.is_disconnected());
        }
        assert!(n.drain().is_empty());
        assert!(n.is_disconnected());
    }

    #[test]
    fn wp_upw2_the_count_is_of_failures_in_a_row() {
        let mut f = ReadFailures::default();
        for _ in 1..ReadFailures::LIMIT {
            assert!(!f.failed());
        }
        f.worked();
        for _ in 1..ReadFailures::LIMIT {
            assert!(!f.failed());
        }
        assert!(f.failed());
    }
}
