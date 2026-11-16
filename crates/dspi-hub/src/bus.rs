//! The hub's internal event bus (WP P2SV, PLAN-P2 section 2): what the hub sees, as
//! typed messages, for its services.
//!
//! A publisher never waits. Each subscriber has its own bounded queue; when it is full
//! the oldest message goes and the subscriber's drop count rises, which it reads with
//! [`Subscription::dropped`] (and may record, as an `events_dropped` history entry). So
//! a slow service costs only itself: a device's reader or the relay publishing here is
//! never delayed by it.
//!
//! Lock order: the subscriber list, then one subscriber's queue, both leaves. Nothing
//! else is locked or called under either, so publishing is safe from any hub thread
//! with any hub lock held or not.
//!
//! The message shapes are fixed here so wave-B services build against them, not
//! against each other: a service that produces `Facts` or `Health` publishes them; one
//! that consumes them subscribes.

use std::collections::VecDeque;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError, Weak};
use std::time::{Duration, Instant};

use dspi_proto::ext::{IdentifyStatus, ResetInfo};
use dspi_session::events::EventRecord;
use serde::Serialize;

use crate::session::SessionId;
use crate::system::alarms::AlarmState;

/// Messages a subscriber holds unless it asks for another capacity.
pub const DEFAULT_CAPACITY: usize = 256;
/// The largest queue a subscriber may ask for.
pub const MAX_CAPACITY: usize = 4096;

/// One target's outcome in a multi-target result (`results: [{serial, ok, code?, msg?}]`,
/// `docs/integrator/hub-api.md`), which phase 3 reuses.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TargetResult {
    /// The device.
    pub serial: String,
    // -- fixup p2a F3 --
    /// Which action of a schedule's row this is, `"<index>:<type>"` (`"0:preset.load"`,
    /// hub-api.md section 12).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action: Option<String>,
    /// Whether the action went through on it.
    pub ok: bool,
    /// The error code when it did not (`offline`, `locked`, `limit_exceeded`, ...).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    /// A readable reason.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub msg: Option<String>,
}

// -- WP P3HB --
/// A device's stream counters, as the health poll reads them (`REQ_GET_STATUS` 4, 6,
/// 8 and 17, totals the diagnostics bundle's `device/stats.json` has). Each is `None`
/// until it was read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
pub struct StreamCounters {
    /// Output underruns: PDM ring, PDM DMA and S/PDIF (`REQ_GET_STATUS` 4, 6, 8) added up.
    pub underruns: Option<u32>,
    /// Output DMA starvations, every instance (`REQ_GET_STATUS` 17).
    pub starvation: Option<u32>,
}

/// The newest history record of a change to a device (WP P3HB). A coalesced record
/// counts once: it is written, and so published, once.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LastChange {
    /// When, ms since the Unix epoch (the record's `t`).
    pub at_ms: i64,
    /// Who made it (the record's `by`), when it names someone.
    pub by: Option<String>,
    /// The record's line.
    pub text: String,
}

/// What the hub tells its services. Cheap to share: a subscriber gets an `Arc` of it.
#[derive(Debug, Clone, PartialEq)]
pub enum BusMessage {
    /// A device came online (attached, new or back within its grace).
    DeviceOnline {
        /// Its serial.
        serial: String,
        /// Its handle.
        handle: u8,
    },
    /// A device went offline; it keeps its handle for the grace.
    DeviceOffline {
        /// Its serial.
        serial: String,
        /// Its handle.
        handle: u8,
    },
    /// A device's grace ended: the hub forgot its handle.
    DeviceRemoved {
        /// Its serial.
        serial: String,
        /// The handle it had.
        handle: u8,
    },
    // -- WP P2DFIXA --
    /// An Admin forgot a device (`inventory.forget`, WP P2IN): it is gone from the
    /// inventory, not only from the registry's grace ([`BusMessage::DeviceRemoved`]).
    DeviceForgotten {
        /// Its serial.
        serial: String,
    },
    /// A device notification, as relayed (every v2 packet that is not idle).
    Notification {
        /// The device.
        serial: String,
        /// The session the change is attributed to (`origin`; 0 for none).
        session: SessionId,
        /// That session's label ([`crate::service::SessionLabel::by`]), if it has one.
        by: Option<String>,
        /// The verbatim v2 packet.
        packet: Arc<[u8]>,
    },
    /// What the hub learnt of a device on attach or later (WP P2HP).
    Facts {
        /// The device.
        serial: String,
        /// `EXT_IDENTIFY` IN, where the firmware lists it.
        identify: Option<IdentifyStatus>,
        /// `EXT_RESET_INFO` IN, where the firmware lists it.
        reset_info: Option<ResetInfo>,
        /// The firmware version.
        fw: String,
    },
    /// One health reading of a device (WP P2HP).
    Health {
        /// The device.
        serial: String,
        /// CPU load, percent, where read.
        cpu: Option<u8>,
        /// The sticky clip latch, one bit per channel ("clipped since cleared").
        clip_mask: u32,
        /// Temperature, °C, where read.
        temp_c: Option<f32>,
        /// Uptime, seconds, where read. -- WP P2EVFIX -- From the last `EXT_RESET_INFO`
        /// read, which P2HP makes every 60 s while `Health` goes out every 2 s: readings
        /// in between repeat the same value, which is no restart.
        uptime_s: Option<u32>,
        /// The firmware's hang count, where read; refreshed with `uptime_s`, every 60 s.
        hang_count: Option<u8>,
        // -- WP P3HB --
        /// The stream's underrun and starvation totals, once either is known (one
        /// single read every 30 s, the four counters in turn).
        counters: Option<StreamCounters>,
        /// Extension clip events (0xE0) in the last minute; `None` on firmware that
        /// only has the latch (no `NOTIFY_FEAT_CLIP_EVENT`).
        clip_per_min: Option<u32>,
        /// The device's newest history record of a change, and who made it.
        last_change: Option<LastChange>,
    },
    /// A device stopped answering while attached.
    Unresponsive {
        /// The device.
        serial: String,
        /// Since when, Unix seconds.
        since: u64,
    },
    /// An alarm changed (WP P2AL): hub-api.md section 7's alarm object, with `at` the
    /// time of this change.
    Alarm {
        // -- fixup p2a F3 --
        /// `"device:<n>"` or `"alink:<n>"`, unique for the hub's run (what `alarm.ack` takes).
        id: String,
        /// `device` or `alink`.
        source: String,
        /// Its code (`device_offline`, `clip`, ...).
        code: String,
        /// What it is about (a serial, a rule).
        subject: String,
        // -- fixup p2a F3 --
        /// The device, for a device alarm.
        serial: Option<String>,
        /// `warning` or `critical`.
        severity: String,
        /// Raised, acknowledged or cleared.
        state: AlarmState,
        /// For people.
        message: String,
        /// When this change happened, Unix seconds.
        at: u64,
        // -- fixup p2a F3 --
        /// When it was raised, Unix seconds.
        raised_at: u64,
        /// When it was cleared, Unix seconds.
        cleared_at: Option<u64>,
        /// Who acknowledged it (a session's `by`).
        acknowledged_by: Option<String>,
        /// How often it was raised while on.
        count: u32,
    },
    /// A schedule rule ran (WP P2SC): `schedule.ran {rule, name, at, by?, results}`.
    ScheduleRan {
        /// The rule's id (`rule` in the event; `id` is only ever a request's).
        rule: String,
        // -- fixup p2a F3 --
        /// The rule's name.
        name: String,
        /// When it ran, Unix seconds.
        at: u64,
        /// The admin who ran it now (`schedule.run_now`); `None` on its schedule.
        by: Option<String>,
        /// Per action and target.
        results: Vec<TargetResult>,
    },
    /// A history record was written (WP P2EV).
    Event {
        /// The device.
        serial: String,
        /// The record as stored.
        record: EventRecord,
    },
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    // Every section below leaves its data whole before anything that could panic.
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

#[derive(Debug, Default)]
struct Queue {
    messages: VecDeque<Arc<BusMessage>>,
    dropped: u64,
}

#[derive(Debug)]
struct Subscriber {
    name: String,
    capacity: usize,
    queue: Mutex<Queue>,
    ready: Condvar,
    // -- WP P2EVFIX --
    /// What it takes; `None` for everything.
    wants: Option<fn(&BusMessage) -> bool>,
}

impl Subscriber {
    /// Never blocks: a full queue loses its oldest message.
    fn push(&self, message: &Arc<BusMessage>) {
        let mut q = lock(&self.queue);
        while q.messages.len() >= self.capacity {
            q.messages.pop_front();
            q.dropped = q.dropped.saturating_add(1);
        }
        q.messages.push_back(Arc::clone(message));
        drop(q);
        self.ready.notify_one();
    }
}

/// The bus. One per hub ([`crate::HubHandle::bus`]).
#[derive(Debug, Default)]
pub struct Bus {
    subscribers: Mutex<Vec<Weak<Subscriber>>>,
}

impl Bus {
    /// A new bus with nobody listening.
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Listen from now on, holding up to `capacity` messages (clamped to
    /// 1..=[`MAX_CAPACITY`]). Dropping the [`Subscription`] stops it.
    pub fn subscribe(&self, name: &str, capacity: usize) -> Subscription {
        let sub = Arc::new(Subscriber {
            name: name.to_owned(),
            capacity: capacity.clamp(1, MAX_CAPACITY),
            queue: Mutex::new(Queue::default()),
            ready: Condvar::new(),
            // -- WP P2EVFIX --
            wants: None,
        });
        let mut list = lock(&self.subscribers);
        list.retain(|s| s.strong_count() > 0);
        list.push(Arc::downgrade(&sub));
        Subscription { sub }
    }

    // -- WP P2EVFIX --
    /// [`Bus::subscribe`], taking only the messages `wants` accepts: the others never
    /// reach its queue, so they cannot push out a message it wants or count as dropped
    /// (the history skips its own `Event` echoes). `wants` runs on the publisher's thread
    /// with no lock held: it must be quick and must not block.
    pub fn subscribe_filtered(
        &self,
        name: &str,
        capacity: usize,
        wants: fn(&BusMessage) -> bool,
    ) -> Subscription {
        let sub = Arc::new(Subscriber {
            name: name.to_owned(),
            capacity: capacity.clamp(1, MAX_CAPACITY),
            queue: Mutex::new(Queue::default()),
            ready: Condvar::new(),
            wants: Some(wants),
        });
        let mut list = lock(&self.subscribers);
        list.retain(|s| s.strong_count() > 0);
        list.push(Arc::downgrade(&sub));
        Subscription { sub }
    }

    /// Whether anyone listens: a publisher may skip building a message nobody reads.
    pub fn has_subscribers(&self) -> bool {
        lock(&self.subscribers).iter().any(|s| s.strong_count() > 0)
    }

    /// Give `message` to every subscriber. Never blocks.
    pub fn publish(&self, message: BusMessage) {
        let live: Vec<Arc<Subscriber>> = {
            let mut list = lock(&self.subscribers);
            list.retain(|s| s.strong_count() > 0);
            list.iter().filter_map(Weak::upgrade).collect()
        };
        if live.is_empty() {
            return;
        }
        let message = Arc::new(message);
        for s in live {
            // -- WP P2EVFIX --
            if s.wants.is_some_and(|wants| !wants(&message)) {
                continue;
            }
            s.push(&message);
        }
    }
}

/// One subscriber's end of the bus. Read it on the service's own thread or timer.
#[derive(Debug)]
pub struct Subscription {
    sub: Arc<Subscriber>,
}

impl Subscription {
    /// The name it was opened with.
    pub fn name(&self) -> &str {
        &self.sub.name
    }

    /// The next message, if one is queued.
    pub fn try_recv(&self) -> Option<Arc<BusMessage>> {
        lock(&self.sub.queue).messages.pop_front()
    }

    /// The next message, waiting up to `timeout` (real time) for one. Blocks: only
    /// on the subscriber's own thread.
    pub fn recv_timeout(&self, timeout: Duration) -> Option<Arc<BusMessage>> {
        let deadline = Instant::now().checked_add(timeout);
        let mut q = lock(&self.sub.queue);
        loop {
            if let Some(m) = q.messages.pop_front() {
                return Some(m);
            }
            let left = deadline?.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return None;
            }
            q = self
                .sub
                .ready
                .wait_timeout(q, left)
                .map(|(q, _)| q)
                .unwrap_or_else(|p| p.into_inner().0);
        }
    }

    /// Everything queued, oldest first.
    pub fn drain(&self) -> Vec<Arc<BusMessage>> {
        lock(&self.sub.queue).messages.drain(..).collect()
    }

    /// How many messages are queued.
    pub fn len(&self) -> usize {
        lock(&self.sub.queue).messages.len()
    }

    /// Whether nothing is queued.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// How many messages this subscriber has lost to a full queue, since it opened.
    pub fn dropped(&self) -> u64 {
        lock(&self.sub.queue).dropped
    }

    /// The drop count, reset to 0: for a service that records each loss once.
    pub fn take_dropped(&self) -> u64 {
        std::mem::take(&mut lock(&self.sub.queue).dropped)
    }
}

#[cfg(test)]
mod tests_wp_p2sv {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing)]
    use super::*;

    fn online(n: u8) -> BusMessage {
        BusMessage::DeviceOnline {
            serial: format!("S{n}"),
            handle: n,
        }
    }

    #[test]
    fn wp_p2sv_every_subscriber_gets_every_message_in_order() {
        let bus = Bus::new();
        let a = bus.subscribe("a", 8);
        let b = bus.subscribe("b", 8);
        bus.publish(online(0));
        bus.publish(online(1));
        for s in [&a, &b] {
            let got: Vec<_> = s.drain().iter().map(|m| (**m).clone()).collect();
            assert_eq!(got, vec![online(0), online(1)], "{}", s.name());
            assert_eq!(s.dropped(), 0);
        }
    }

    #[test]
    fn wp_p2sv_a_full_queue_drops_its_oldest_and_counts() {
        let bus = Bus::new();
        let slow = bus.subscribe("slow", 3);
        let fast = bus.subscribe("fast", 64);
        for n in 0..10 {
            bus.publish(online(n));
        }
        assert_eq!(slow.dropped(), 7);
        let kept: Vec<_> = slow.drain().iter().map(|m| (**m).clone()).collect();
        assert_eq!(
            kept,
            vec![online(7), online(8), online(9)],
            "the newest stay"
        );
        assert_eq!(fast.len(), 10, "another subscriber lost nothing");
        assert_eq!(fast.dropped(), 0);
        assert_eq!(slow.take_dropped(), 7);
        assert_eq!(slow.dropped(), 0);
    }

    #[test]
    fn wp_p2sv_a_dropped_subscription_leaves_the_bus() {
        let bus = Bus::new();
        assert!(!bus.has_subscribers());
        let s = bus.subscribe("s", 0);
        assert!(bus.has_subscribers());
        bus.publish(online(1));
        bus.publish(online(2));
        assert_eq!(s.len(), 1, "a capacity of 0 is 1");
        drop(s);
        assert!(!bus.has_subscribers());
        bus.publish(online(3));
        assert!(bus.subscribers.lock().unwrap().is_empty(), "pruned");
    }

    #[test]
    fn wp_p2sv_recv_timeout_wakes_on_a_publish() {
        let bus = Bus::new();
        let s = bus.subscribe("s", 4);
        assert!(s.recv_timeout(Duration::from_millis(10)).is_none());
        let publisher = {
            let bus = Arc::clone(&bus);
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(20));
                bus.publish(online(5));
            })
        };
        let got = s.recv_timeout(Duration::from_secs(10)).unwrap();
        assert_eq!(*got, online(5));
        publisher.join().unwrap();
    }

    #[test]
    fn wp_p2sv_a_target_result_has_the_documented_shape() {
        let mut r = TargetResult {
            serial: "S1".into(),
            action: None,
            ok: false,
            code: Some("offline".into()),
            msg: None,
        };
        assert_eq!(
            serde_json::to_value(&r).unwrap(),
            serde_json::json!({"serial": "S1", "ok": false, "code": "offline"})
        );
        // -- fixup p2a F3 -- a schedule's row names its action
        r.action = Some("0:preset.load".into());
        assert_eq!(
            serde_json::to_value(&r).unwrap(),
            serde_json::json!({"serial": "S1", "action": "0:preset.load", "ok": false, "code": "offline"})
        );
    }
}
