//! Notification fan-out (spec 8.3, 8.6). Owned by WP HB2.
//!
//! Lock level L3. Holding it while pushing to an outbox (L4) is safe because a push
//! never blocks, so a slow consumer cannot stall the device's reader or another
//! session.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use dspi_proto::generated::notify as nfy;
use dspi_transport::TransportKind;

use crate::session::{Outbound, Outbox, SessionId};

/// The firmware's v2 notification version byte.
const PROTOCOL_V2: u8 = 2;
const EVT_IDLE: u8 = nfy::NOTIFY_EVT_IDLE as u8;
const EVT_PARAM_CHANGED: u8 = nfy::NOTIFY_EVT_PARAM_CHANGED as u8;
const EVT_BULK_INVALIDATED: u8 = nfy::NOTIFY_EVT_BULK_INVALIDATED as u8;
const EVT_RTA_CONFIG: u8 = nfy::NOTIFY_EVT_RTA_CONFIG as u8;

/// Which writes a notification can answer (`Router::consume_attribution`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PacketKind {
    /// `PARAM_CHANGED`: a scalar write.
    Scalar,
    /// `BULK_INVALIDATED`: a bulk write.
    Bulk,
    /// `RTA_CONFIG`: a `REQ_RTA_SET_CONFIG` (spec 7.15).
    RtaConfig,
}

/// Whether a device packet is relayed: "every non-idle packet", and never a v1
/// legacy one, since every v1 event has a v2 twin (spec 8.3).
pub fn is_relayable(packet: &[u8]) -> bool {
    matches!(packet, [PROTOCOL_V2, event, _, _, ..] if *event != EVT_IDLE)
}

/// What a packet says about its origin: its raw `ParamSource`, and which writes it
/// can answer. `None` for events that carry no source.
pub fn packet_source(packet: &[u8]) -> Option<(u8, PacketKind)> {
    match *packet.get(1)? {
        EVT_PARAM_CHANGED => Some((*packet.get(8)?, PacketKind::Scalar)),
        EVT_BULK_INVALIDATED => Some((*packet.get(4)?, PacketKind::Bulk)),
        EVT_RTA_CONFIG => Some((*packet.get(16)?, PacketKind::RtaConfig)),
        _ => None,
    }
}

/// Whether a packet with this source came in on the path the hub writes on, so it
/// can be a session's write. Anything else (a knob, a preset, another host on
/// another path) has no session behind it. The firmware tags a bulk apply
/// `BULK_SET` whatever the path, unless it reports the path (PLAN section 1.2).
pub fn is_hub_path(kind: TransportKind, source: u8) -> bool {
    let src = u16::from(source);
    if src == nfy::PARAM_SRC_BULK_SET {
        return true;
    }
    match kind {
        TransportKind::Usb | TransportKind::Fake => src == nfy::PARAM_SRC_HOST_SET,
        TransportKind::Midi => src == nfy::PARAM_SRC_MIDI,
        TransportKind::Link => src == nfy::PARAM_SRC_UART || src == nfy::PARAM_SRC_HOST_SET,
    }
}

/// Which notifications a subscriber gets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    /// Every device: a network session, multiplexed (spec 4).
    AllDevices,
    /// One attachment of one device: the app's transport. When the device goes,
    /// its outbox closes, which its reader sees as the end of the stream.
    Attachment(u8),
}

#[derive(Debug)]
struct Subscriber {
    id: u64,
    session: SessionId,
    scope: Scope,
    outbox: Arc<Outbox>,
}

/// The hub's fan-out.
#[derive(Debug, Default)]
pub struct Relay {
    subscribers: Mutex<Vec<Subscriber>>,
    next_id: AtomicU64,
    // -- WP SW2 -- (-- WP P2SV -- generalised: the system model and the bus)
    /// What watches every device packet: the system model, the hub's bus.
    taps: Taps,
    // -- WP P2HS --
    /// Notifications dropped from a full outbox, by the handle of the device they were
    /// from, since the hub started (`hub.stats`). A leaf lock, taken only after a drop.
    dropped: Mutex<std::collections::BTreeMap<u8, u64>>,
}

// -- WP P2SV --
/// A read-only watcher of every relayed device packet, with the device's serial and
/// the session it is attributed to. Called on the device's reader with no relay lock
/// held, so it must not block; it may broadcast through the relay it is given.
pub(crate) trait PacketTap: Send + Sync {
    /// Whether it looks at this packet: `serial` is worked out only if one does.
    fn wants(&self, packet: &[u8]) -> bool;
    /// The packet, from the device `serial`, attributed to `origin`.
    fn on_packet(&self, relay: &Relay, serial: &str, origin: SessionId, packet: &[u8]);
}

#[derive(Default)]
struct Taps(Mutex<Vec<Arc<dyn PacketTap>>>);

impl std::fmt::Debug for Taps {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let n = self.0.lock().unwrap_or_else(PoisonError::into_inner).len();
        write!(f, "Taps({n})")
    }
}

// -- WP SW2 -- the system model follows the audio link's events (0xE1, 0xE3).
impl PacketTap for crate::system::SystemShared {
    fn wants(&self, packet: &[u8]) -> bool {
        crate::system::SystemShared::wants(packet)
    }

    fn on_packet(&self, relay: &Relay, serial: &str, _origin: SessionId, packet: &[u8]) {
        crate::system::SystemShared::on_packet(self, relay, serial, packet);
    }
}

impl Relay {
    fn lock(&self) -> MutexGuard<'_, Vec<Subscriber>> {
        // A push cannot leave the list half-updated.
        self.subscribers
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    // -- WP HUBFLAKE4 --
    /// Hold every push back while the guard lives: a test's way to stop a broadcast
    /// partway.
    #[cfg(test)]
    pub(crate) fn hold(&self) -> impl Sized + '_ {
        self.lock()
    }

    /// Add a subscriber. Returns its id for [`Relay::remove`].
    pub fn add(&self, session: SessionId, scope: Scope, outbox: Arc<Outbox>) -> u64 {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        self.lock().push(Subscriber {
            id,
            session,
            scope,
            outbox,
        });
        id
    }

    /// Remove a subscriber.
    pub fn remove(&self, id: u64) {
        self.lock().retain(|s| s.id != id);
    }

    /// Remove every subscriber of a session.
    pub fn remove_session(&self, session: SessionId) {
        self.lock().retain(|s| s.session != session);
    }

    /// Queue a notification for everyone who follows `handle`. Never blocks.
    pub fn publish_notify(&self, handle: u8, origin: SessionId, packet: &[u8]) {
        let packet: Arc<[u8]> = Arc::from(packet);
        for s in self.lock().iter().filter(|s| follows(s, handle)) {
            // -- WP P2HS --
            if let Some(lost) = s.outbox.push_notify(handle, origin, Arc::clone(&packet)) {
                let mut dropped = self.dropped.lock().unwrap_or_else(PoisonError::into_inner);
                let n = dropped.entry(lost).or_insert(0);
                *n = n.saturating_add(1);
            }
        }
    }

    // -- WP P2HS --
    /// How many notifications of the device at `handle` full outboxes have dropped
    /// since the hub started, over every session.
    pub(crate) fn notify_dropped(&self, handle: u8) -> u64 {
        self.dropped
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&handle)
            .copied()
            .unwrap_or(0)
    }

    // -- WP SW2 --
    /// Watch every device packet with the system model.
    pub(crate) fn set_tap(&self, tap: Arc<crate::system::SystemShared>) {
        self.add_tap(tap);
    }

    // -- WP P2SV --
    /// Add a watcher of every device packet, after those already there.
    pub(crate) fn add_tap(&self, tap: Arc<dyn PacketTap>) {
        self.taps
            .0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(tap);
    }

    // -- fixup sw2x (F5) -- (-- WP P2SV -- every tap, with the packet's session)
    /// Show a device's packet to every tap that wants it, with the device's serial,
    /// which `serial` gives once and only if one does, and the session `origin` it is
    /// attributed to. The taps may broadcast through `self`, so they are called
    /// without the subscriber lock, and with their own list's lock released.
    pub(crate) fn tap_packet(
        &self,
        packet: &[u8],
        origin: SessionId,
        serial: impl FnOnce() -> String,
    ) {
        let taps: Vec<Arc<dyn PacketTap>> = self
            .taps
            .0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .filter(|t| t.wants(packet))
            .cloned()
            .collect();
        if taps.is_empty() {
            return;
        }
        let serial = serial();
        for tap in taps {
            tap.on_packet(self, &serial, origin, packet);
        }
    }

    /// Tell everyone who follows `handle` to re-read (spec 8.6).
    pub fn resync_all(&self, handle: u8, reason: u8) {
        for s in self.lock().iter().filter(|s| follows(s, handle)) {
            s.outbox.push_resync(handle, reason);
        }
    }

    /// Queue a control-plane message for every network session.
    pub fn broadcast_event(&self, json: &str) {
        for s in self.lock().iter().filter(|s| s.scope == Scope::AllDevices) {
            s.outbox.push_reliable(Outbound::Event(json.to_owned()));
        }
    }

    /// The device at `handle` went: the app's transports on it see their stream
    /// end, after whatever is still queued.
    pub fn device_gone(&self, handle: u8) {
        self.lock().retain(|s| {
            let bound = s.scope == Scope::Attachment(handle);
            if bound {
                s.outbox.close();
            }
            !bound
        });
    }

    /// Close every subscriber, with this WebSocket close code for the network ones.
    pub fn close_all(&self, code: u16, reason: &str) {
        for s in self.lock().drain(..) {
            match s.scope {
                Scope::AllDevices => s.outbox.close_with(code, reason),
                Scope::Attachment(_) => s.outbox.close(),
            }
        }
    }
}

fn follows(s: &Subscriber, handle: u8) -> bool {
    match s.scope {
        Scope::AllDevices => true,
        Scope::Attachment(h) => h == handle,
    }
}

#[cfg(test)]
mod tests_wp_hb0 {
    use std::time::Duration;

    use super::*;
    use crate::session::Pop;

    const NOW: Duration = Duration::ZERO;

    #[test]
    fn wp_hb0_only_v2_non_idle_packets_are_relayed() {
        assert!(is_relayable(&[2, EVT_PARAM_CHANGED, 0, 1, 0]));
        assert!(!is_relayable(&[2, EVT_IDLE, 0, 1]), "idle");
        assert!(!is_relayable(&[1, 0, 0, 0, 0, 0, 0, 0]), "v1 legacy");
        assert!(!is_relayable(&[0]), "the one-byte keepalive");
        assert!(!is_relayable(&[]));
    }

    #[test]
    fn wp_hb0_the_source_is_read_safely() {
        let mut p = vec![2, EVT_PARAM_CHANGED, 0, 1, 0, 0, 4, 0, 7];
        assert_eq!(packet_source(&p), Some((7, PacketKind::Scalar)));
        p.truncate(8);
        assert_eq!(packet_source(&p), None, "short");
        assert_eq!(
            packet_source(&[2, EVT_BULK_INVALIDATED, 0, 1, 3]),
            Some((3, PacketKind::Bulk))
        );
        assert_eq!(packet_source(&[2]), None);
        assert_eq!(packet_source(&[2, EVT_IDLE, 0, 1, 3]), None);
    }

    #[test]
    fn wp_hb0_only_the_hub_path_can_be_a_session() {
        let (host, midi, gpio) = (
            nfy::PARAM_SRC_HOST_SET as u8,
            nfy::PARAM_SRC_MIDI as u8,
            nfy::PARAM_SRC_GPIO as u8,
        );
        assert!(is_hub_path(TransportKind::Usb, host));
        assert!(!is_hub_path(TransportKind::Usb, midi), "a MIDI host");
        assert!(!is_hub_path(TransportKind::Usb, gpio), "a knob");
        assert!(is_hub_path(TransportKind::Midi, midi));
        assert!(!is_hub_path(TransportKind::Midi, host), "a USB host");
        assert!(is_hub_path(
            TransportKind::Midi,
            nfy::PARAM_SRC_BULK_SET as u8
        ));
    }

    #[test]
    fn wp_hb0_a_device_going_ends_only_its_own_app_transport() {
        let relay = Relay::default();
        let (a, b, net) = (
            Arc::new(Outbox::new()),
            Arc::new(Outbox::new()),
            Arc::new(Outbox::new()),
        );
        relay.add(1, Scope::Attachment(0), Arc::clone(&a));
        relay.add(1, Scope::Attachment(1), Arc::clone(&b));
        relay.add(2, Scope::AllDevices, Arc::clone(&net));
        relay.publish_notify(0, 2, &[2, 1, 0, 0]);
        relay.device_gone(0);
        assert!(matches!(a.pop(NOW), Pop::Item(Outbound::Notify { .. })));
        assert_eq!(a.pop(NOW), Pop::Closed);
        assert_eq!(b.pop(NOW), Pop::Idle, "another device's");
        assert!(matches!(net.pop(NOW), Pop::Item(Outbound::Notify { .. })));
        assert!(!net.is_closed());
    }
}

#[cfg(test)]
mod tests_wp_hb2 {
    use std::time::Duration;

    use super::*;
    use crate::session::Pop;

    const NOW: Duration = Duration::ZERO;

    /// Spec 7.15: the source byte of `RTA_CONFIG` is at offset 16.
    #[test]
    fn wp_hb2_an_analyser_configuration_carries_its_source() {
        let mut p = vec![0; 17];
        p[0] = PROTOCOL_V2;
        p[1] = EVT_RTA_CONFIG;
        p[16] = nfy::PARAM_SRC_HOST_SET as u8;
        assert_eq!(
            packet_source(&p),
            Some((nfy::PARAM_SRC_HOST_SET as u8, PacketKind::RtaConfig))
        );
        p.truncate(16);
        assert_eq!(packet_source(&p), None, "short");
    }

    /// Everyone gets a packet with the same `origin`, the writer too: dropping an own
    /// echo is the client's business, and only it knows its session.
    #[test]
    fn wp_hb2_the_writer_is_not_skipped_and_each_session_counts_its_own_sequence() {
        let relay = Relay::default();
        let (a, b) = (Arc::new(Outbox::new()), Arc::new(Outbox::new()));
        relay.add(2, Scope::AllDevices, Arc::clone(&a));
        relay.add(3, Scope::AllDevices, Arc::clone(&b));
        relay.publish_notify(0, 2, &[2, EVT_PARAM_CHANGED, 0, 0]);
        relay.remove_session(3);
        relay.publish_notify(0, 3, &[2, EVT_PARAM_CHANGED, 0, 1]);
        let next = |o: &Outbox| match o.pop(NOW) {
            Pop::Item(Outbound::Notify { seq, origin, .. }) => (seq, origin),
            other => panic!("{other:?}"),
        };
        assert_eq!(next(&a), (0, 2));
        assert_eq!(next(&b), (0, 2));
        assert_eq!(next(&a), (1, 3));
        assert_eq!(b.pop(NOW), Pop::Idle, "a closed session hears nothing more");
    }

    /// A client that stops reading loses only its own notifications, and is told to
    /// re-read that device once it has caught up.
    #[test]
    fn wp_hb2_a_slow_client_costs_only_itself_and_gets_a_resync() {
        let relay = Relay::default();
        let (slow, fast) = (
            Arc::new(Outbox::with_capacity(4, 16)),
            Arc::new(Outbox::with_capacity(4, 16)),
        );
        relay.add(2, Scope::AllDevices, Arc::clone(&slow));
        relay.add(3, Scope::AllDevices, Arc::clone(&fast));
        for i in 0u16..100 {
            relay.publish_notify(7, 0, &[2, EVT_PARAM_CHANGED, 0, i as u8]);
            assert!(
                matches!(fast.pop(NOW), Pop::Item(Outbound::Notify { seq, .. }) if seq == i),
                "the reader of its own queue misses nothing"
            );
        }
        let mut items = Vec::new();
        while let Pop::Item(item) = slow.pop(NOW) {
            items.push(item);
        }
        assert_eq!(items.len(), 5, "what fits, and one RESYNC");
        assert!(
            matches!(items[0], Outbound::Notify { seq: 96, .. }),
            "a gap shows"
        );
        assert_eq!(
            items[4],
            Outbound::Resync {
                handle: 7,
                reason: 0
            }
        );
    }

    #[test]
    fn wp_hb2_a_reattached_device_resyncs_those_who_follow_it() {
        let relay = Relay::default();
        let (net, app, other) = (
            Arc::new(Outbox::new()),
            Arc::new(Outbox::new()),
            Arc::new(Outbox::new()),
        );
        relay.add(2, Scope::AllDevices, Arc::clone(&net));
        relay.add(1, Scope::Attachment(5), Arc::clone(&app));
        relay.add(1, Scope::Attachment(6), Arc::clone(&other));
        relay.resync_all(5, 1);
        let resync = Pop::Item(Outbound::Resync {
            handle: 5,
            reason: 1,
        });
        assert_eq!(net.pop(NOW), resync);
        assert_eq!(app.pop(NOW), resync);
        assert_eq!(other.pop(NOW), Pop::Idle);
    }
}

// -- WP P2SV --
#[cfg(test)]
mod tests_wp_p2sv {
    #![allow(clippy::unwrap_used)]
    use std::sync::atomic::AtomicUsize;

    use super::*;

    type Seen = Mutex<Vec<(String, SessionId, Vec<u8>)>>;

    #[derive(Default)]
    struct Recorder {
        only_param_changed: bool,
        seen: Seen,
    }

    impl PacketTap for Recorder {
        fn wants(&self, packet: &[u8]) -> bool {
            !self.only_param_changed || packet.get(1) == Some(&EVT_PARAM_CHANGED)
        }

        fn on_packet(&self, _relay: &Relay, serial: &str, origin: SessionId, packet: &[u8]) {
            self.seen
                .lock()
                .unwrap()
                .push((serial.to_owned(), origin, packet.to_vec()));
        }
    }

    /// Every tap sees what it wants with the serial and the session; the serial is
    /// worked out once, and not at all when no tap wants the packet.
    #[test]
    fn wp_p2sv_every_tap_sees_the_packet_with_its_serial_and_session() {
        let relay = Relay::default();
        let all = Arc::new(Recorder::default());
        let some = Arc::new(Recorder {
            only_param_changed: true,
            ..Recorder::default()
        });
        relay.add_tap(Arc::clone(&all) as Arc<dyn PacketTap>);
        relay.add_tap(Arc::clone(&some) as Arc<dyn PacketTap>);
        let asked = AtomicUsize::new(0);
        let serial = || {
            asked.fetch_add(1, Ordering::SeqCst);
            "P2SV-T".to_owned()
        };
        relay.tap_packet(&[2, EVT_PARAM_CHANGED, 0, 1], 7, serial);
        relay.tap_packet(&[2, EVT_BULK_INVALIDATED, 0, 1], 0, serial);
        assert_eq!(asked.load(Ordering::SeqCst), 2, "once per packet");
        let all = all.seen.lock().unwrap().clone();
        assert_eq!(
            all,
            vec![
                ("P2SV-T".to_owned(), 7, vec![2, EVT_PARAM_CHANGED, 0, 1]),
                ("P2SV-T".to_owned(), 0, vec![2, EVT_BULK_INVALIDATED, 0, 1]),
            ]
        );
        assert_eq!(some.seen.lock().unwrap().len(), 1, "only what it wants");

        let none = Relay::default();
        none.add_tap(Arc::new(Recorder {
            only_param_changed: true,
            ..Recorder::default()
        }));
        none.tap_packet(&[2, EVT_BULK_INVALIDATED, 0, 1], 0, serial);
        assert_eq!(asked.load(Ordering::SeqCst), 2, "nobody wanted it");
    }

    /// The system model is a tap like any other: it still gets the audio link's
    /// events, and not the rest (`wp_ausb2_lock_error` follows one to its alarm).
    #[test]
    fn wp_p2sv_the_system_model_is_still_a_tap() {
        let system = Arc::new(crate::system::SystemShared::new(None));
        let tap: &dyn PacketTap = &*system;
        assert!(!tap.wants(&[2, EVT_PARAM_CHANGED, 0, 1]));
        assert!(tap.wants(&[2, dspi_proto::ext::alink::NOTIFY_EVT_ALINK_STATE, 0, 0]));
        let relay = Relay::default();
        relay.set_tap(Arc::clone(&system));
        assert_eq!(format!("{relay:?}").matches("Taps(1)").count(), 1);
    }
}
