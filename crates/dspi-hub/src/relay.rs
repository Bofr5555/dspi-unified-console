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

/// Whether a device packet is relayed: "every non-idle packet", and never a v1
/// legacy one, since every v1 event has a v2 twin (spec 8.3).
pub fn is_relayable(packet: &[u8]) -> bool {
    matches!(packet, [PROTOCOL_V2, event, _, _, ..] if *event != EVT_IDLE)
}

/// What a packet says about its origin: its raw `ParamSource`, and whether it is a
/// `BULK_INVALIDATED`. `None` for events that carry no source.
pub fn packet_source(packet: &[u8]) -> Option<(u8, bool)> {
    match *packet.get(1)? {
        EVT_PARAM_CHANGED => Some((*packet.get(8)?, false)),
        EVT_BULK_INVALIDATED => Some((*packet.get(4)?, true)),
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
}

impl Relay {
    fn lock(&self) -> MutexGuard<'_, Vec<Subscriber>> {
        // A push cannot leave the list half-updated.
        self.subscribers
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
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
            s.outbox.push_notify(handle, origin, Arc::clone(&packet));
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
        assert_eq!(packet_source(&p), Some((7, false)));
        p.truncate(8);
        assert_eq!(packet_source(&p), None, "short");
        assert_eq!(
            packet_source(&[2, EVT_BULK_INVALIDATED, 0, 1, 3]),
            Some((3, true))
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
