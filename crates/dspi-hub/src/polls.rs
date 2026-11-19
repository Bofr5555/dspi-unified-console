//! Poll subscriptions (spec 7.6, 8.4). Owned by WP HB3.
//!
//! The scheduler is pure and lives with the device: the caller passes the clock. The
//! device worker alone runs the reads ([`Polls::next_due`], [`Polls::take_due`],
//! [`Polls::finished`]), so a poll takes its turn with the commands and no second
//! thread touches a transport.
//!
//! **Shared.** Polls of the same `req`, `val` and `len` form one group, whoever asked:
//! one read at the fastest rate granted, fanned out to each subscriber at its own rate.
//! `idx` is not part of it: `wIndex` is not passed on (HUB.md section 8), so polls that
//! differ only there are one read. -- WP PGHDEV -- except its page (the high byte):
//! the group is keyed by `(page, req)`, so the same command number on two pages is two
//! reads (PLAN-PG section 1.5); the low byte (0 from the Mac, 2 elsewhere) still is not.
//!
//! **Bounded**, per device, across all sessions, as the bridge does (its spec 7.7): the
//! group is what costs, once.
//! - [`POLL_BUDGET_BPS`] of answers, as `hello.limits.poll_budget_bps`.
//! - [`MAX_TRANSFERS_PER_SEC`] of reads: small polls could otherwise fill the budget
//!   with transfers. A grant stays inside it, and [`Polls::take_due`] spaces reads by
//!   its inverse, so neither a burst nor a client that keeps subscribing can exceed it.
//!
//! **Stops** with its last subscriber: no group, no read, and the worker sleeps. A
//! subscriber whose outbox has closed counts as gone at the next read, without waiting
//! out the dead-peer limit. A device's `Polls` go with its attachment, which ends every
//! subscription when it goes offline, as the spec asks.
//!
//! **Beside commands:** one read per pass of the worker, so a command waits for at most
//! one read, and a device that has stopped answering costs commands one transfer's
//! timeout between each, not one per group.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use dspi_link::Dir;
use dspi_link::messages::{PollGrant, PollSpec};
use dspi_transport::TransportError;
use serde_json::json;

use crate::policy::{Class, Policy};
// -- WP PGHDEV --
use crate::session::{Outbound, Outbox, SessionId};
use dspi_link::messages::LINK_13_MINOR;
use dspi_proto::paged::{CmdId, PagedDir, legacy_equivalent};

/// `hello.limits.poll_max_hz`: the fastest rate granted.
pub const POLL_MAX_HZ: u32 = 20;
/// `hello.limits.poll_budget_bps`: the bytes per second a device's polls may return.
pub const POLL_BUDGET_BPS: u32 = 200_000;
/// The reads per second a device's polls may make: every slot of one session at the
/// fastest rate. C15's full analyser and meters are 240.
pub const MAX_TRANSFERS_PER_SEC: u32 = 16 * POLL_MAX_HZ;
/// Slots per session are `0..SLOTS` (spec 7.6).
const SLOTS: u8 = 16;
/// The longest answer a poll may ask for: the least `limits.max_payload` (spec 7.2).
const MAX_LEN: u16 = 8192;
/// What a poll's answer costs against the budget beyond `len`: its POLL frame's header.
const FRAME_HEADER_BYTES: u64 = 8;
/// A read fails this many times in a row: `poll.error`, and the slower rate (spec 8.4).
const FAILURES_BEFORE_ERROR: u8 = 3;
/// The period of a group that keeps failing. A read can cost a transport's timeout.
const FAILED_PERIOD: Duration = Duration::from_secs(1);
/// How long polls wait after a client's own `0xA2`/`0xA3`. Any other vendor request
/// tears down the device's chunk session (`bulk_params_chunking.md`), so meters would
/// otherwise keep a client from ever finishing one.
const CHUNK_HOLD: Duration = Duration::from_millis(500);
const SECOND: Duration = Duration::from_secs(1);

/// Why a `poll.subscribe` was refused. Nothing changed for the session.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PollRefused {
    /// No such device online (`no_device`).
    #[error("no such device")]
    NoDevice,
    /// A poll is malformed (`bad_request`).
    #[error("{0}")]
    BadRequest(String),
    /// A command not classed *read* (`denied`).
    #[error("only commands classed read may be polled")]
    Denied,
    /// The budget cannot fit these polls even at 1 Hz (`busy`).
    #[error("the poll budget cannot fit these polls even at 1 Hz")]
    Busy,
    // -- WP PGHUB0 --
    /// A poll names a page (`idx` high byte not 0) toward a 1.1.6-generation
    /// device, which would run a different command (`unsupported`, spec 7.4).
    #[error("this device does not take paged requests")]
    Unsupported,
    // -- WP PGHDEV --
    /// A page-0 poll from a DSPi Link 1.3 session toward a paged (1.1.7) device,
    /// which refuses page 0 (`unsupported`, as the `0x10` a CMD gets).
    #[error("this device speaks the paged protocol: poll it by page")]
    PageZero,
}

impl PollRefused {
    /// The `code` of the `err` reply (spec 7.1).
    pub fn code(&self) -> &'static str {
        match self {
            PollRefused::NoDevice => "no_device",
            PollRefused::BadRequest(_) => "bad_request",
            PollRefused::Denied => "denied",
            PollRefused::Busy => "busy",
            PollRefused::Unsupported => "unsupported", // -- WP PGHUB0 --
            PollRefused::PageZero => "unsupported",    // -- WP PGHDEV --
        }
    }
}

/// One read for the worker to run, for every subscriber of its group.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PollJob {
    // -- WP PGHDEV --
    /// The page (the `wIndex` high byte): 0 is a 1.1.6-shaped read, anything else
    /// a paged GET of `(page, req)` on a paged device.
    pub page: u8,
    /// The GET's `bRequest`.
    pub req: u8,
    /// The GET's `wValue`.
    pub val: u16,
    /// The GET's `wLength`.
    pub len: u16,
}

/// What makes two polls one read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct Key {
    // -- WP PGHDEV --
    page: u8,
    req: u8,
    val: u16,
    len: u16,
}

// -- WP PGHDEV --
impl Key {
    fn of(r: &PollSpec) -> Self {
        Self {
            page: CmdId::from_request(r.req, r.idx).page(),
            req: r.req,
            val: r.val,
            len: r.len,
        }
    }

    /// The 1.1.6 opcode this read is, for the analyser's STOP arbitration: itself on
    /// page 0, a paged read's equivalent otherwise.
    fn legacy_opcode(self) -> Option<u8> {
        let id = CmdId::new(self.page, self.req);
        if id.is_legacy() {
            return Some(self.req);
        }
        legacy_equivalent(id, PagedDir::Get, self.val, &[]).map(|e| e.opcode)
    }
}

// -- WP PGHDEV --
/// What decides how a poll is judged and read (PLAN-PG section 1.5's table, as for
/// a CMD): the device's generation and the session's `hello` minor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PollRoute {
    /// The device answered its attach probe as paged firmware (1.1.7).
    pub paged: bool,
    /// The DSPi Link minor of the session's client (`Session::link_minor`).
    pub link_minor: u32,
}

impl PollRoute {
    /// Every device before PGHDEV, and a 1.2 session.
    pub const LEGACY: Self = Self {
        paged: false,
        link_minor: crate::session::LEGACY_LINK_MINOR,
    };
}

impl Key {
    /// The bytes per second one read per second costs.
    fn cost(self) -> u64 {
        FRAME_HEADER_BYTES.saturating_add(u64::from(self.len))
    }
}

#[derive(Debug)]
struct Group {
    period: Duration,
    next_due: Instant,
    failures: u8,
}

#[derive(Debug)]
struct Subscriber {
    handle: u8,
    key: Key,
    hz: u32,
    outbox: Arc<Outbox>,
    last_sent: Option<Instant>,
}

#[derive(Debug, Default)]
struct State {
    subscribers: BTreeMap<(SessionId, u8), Subscriber>,
    groups: BTreeMap<Key, Group>,
    last_read: Option<Instant>,
    hold_until: Option<Instant>,
    /// The worker has been told to look again, and has not yet.
    wake_sent: bool,
}

/// One device's subscriptions.
#[derive(Debug, Default)]
pub struct Polls {
    state: Mutex<State>,
}

fn period_of(hz: u32) -> Duration {
    SECOND.checked_div(hz).unwrap_or(SECOND)
}

impl State {
    /// The groups from the subscribers, keeping each survivor's schedule.
    fn rebuild(&mut self, now: Instant) {
        let mut fastest: BTreeMap<Key, u32> = BTreeMap::new();
        for s in self.subscribers.values() {
            let hz = fastest.entry(s.key).or_default();
            *hz = (*hz).max(s.hz);
        }
        self.groups.retain(|key, _| fastest.contains_key(key));
        for (key, hz) in fastest {
            let period = period_of(hz);
            match self.groups.get_mut(&key) {
                Some(g) => {
                    g.period = period;
                    // A faster subscriber need not wait out a slower rate's period.
                    if let Some(sooner) = now.checked_add(period) {
                        g.next_due = g.next_due.min(sooner);
                    }
                }
                None => {
                    self.groups.insert(
                        key,
                        Group {
                            period,
                            next_due: now,
                            failures: 0,
                        },
                    );
                }
            }
        }
    }

    /// The earliest a read may start: not before the hold, nor sooner than the rate
    /// limit after the last one.
    fn gate(&self) -> Option<Instant> {
        let spaced = self
            .last_read
            .and_then(|at| at.checked_add(period_of(MAX_TRANSFERS_PER_SEC)));
        spaced.max(self.hold_until)
    }

    /// The rates for `requests`, or `None` if one cannot have 1 Hz. Rates already
    /// running cost nothing again; `session`'s slots being replaced no longer count.
    fn grant(&self, session: SessionId, requests: &[PollSpec]) -> Option<Vec<u32>> {
        let mut rates: BTreeMap<Key, u32> = BTreeMap::new();
        for ((owner, slot), s) in &self.subscribers {
            if *owner == session && requests.iter().any(|r| r.slot == *slot) {
                continue;
            }
            let hz = rates.entry(s.key).or_default();
            *hz = (*hz).max(s.hz);
        }
        let mut bytes: u64 = rates
            .iter()
            .map(|(k, hz)| k.cost().saturating_mul(u64::from(*hz)))
            .fold(0, u64::saturating_add);
        let mut reads: u64 = rates
            .values()
            .map(|hz| u64::from(*hz))
            .fold(0, u64::saturating_add);
        let mut granted = Vec::with_capacity(requests.len());
        for r in requests {
            let key = Key::of(r);
            let running = rates.get(&key).copied().unwrap_or(0);
            let mut hz = r.hz.min(POLL_MAX_HZ);
            if hz > running {
                let by_bytes = u64::from(POLL_BUDGET_BPS)
                    .saturating_sub(bytes)
                    .checked_div(key.cost())
                    .unwrap_or(0);
                let by_reads = u64::from(MAX_TRANSFERS_PER_SEC).saturating_sub(reads);
                let affordable = u32::try_from(by_bytes.min(by_reads)).unwrap_or(u32::MAX);
                hz = hz.min(running.saturating_add(affordable));
            }
            if hz < 1 {
                return None;
            }
            if hz > running {
                let added = u64::from(hz.saturating_sub(running));
                bytes = bytes.saturating_add(key.cost().saturating_mul(added));
                reads = reads.saturating_add(added);
                rates.insert(key, hz);
            }
            granted.push(hz);
        }
        Some(granted)
    }
}

/// Checks that come before anything changes.
fn check(requests: &[PollSpec], policy: &Policy, route: PollRoute) -> Result<(), PollRefused> {
    let bad = |msg: &str| Err(PollRefused::BadRequest(msg.to_owned()));
    if requests.is_empty() || requests.len() > usize::from(SLOTS) {
        return bad("polls must list 1 to 16 polls");
    }
    let mut seen = 0u32;
    for r in requests {
        if r.slot >= SLOTS {
            return bad("slot must be 0 to 15");
        }
        let bit = 1u32.checked_shl(u32::from(r.slot)).unwrap_or(0);
        if seen & bit != 0 {
            return bad("a slot is listed twice");
        }
        seen |= bit;
        if r.len == 0 || r.len > MAX_LEN || r.hz == 0 {
            return bad("len or hz is out of range");
        }
        // -- WP PGHUB0 -- a page is never polled on a legacy device
        // -- WP PGHDEV -- and page 0 only by a 1.2 session on a paged one, through
        // the adapter; each judged as its CMD is (`device::Route::class`)
        let id = CmdId::from_request(r.req, r.idx);
        let class = match (route.paged, id.is_legacy()) {
            (false, true) => policy.classify(r.req, Dir::Get),
            (false, false) => return Err(PollRefused::Unsupported),
            (true, false) => policy.classify_id(id, Dir::Get),
            (true, true) if route.link_minor >= LINK_13_MINOR => {
                return Err(PollRefused::PageZero);
            }
            (true, true) => policy.classify_translated(r.req, r.val, Dir::Get),
        };
        if class != Class::Read {
            return Err(PollRefused::Denied);
        }
    }
    Ok(())
}

impl Polls {
    fn lock(&self) -> MutexGuard<'_, State> {
        // Each update is complete before anything that could panic.
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// When the worker should next wake for a poll; `None` if nothing is subscribed.
    pub fn next_due(&self) -> Option<Instant> {
        let mut s = self.lock();
        s.wake_sent = false;
        let earliest = s.groups.values().map(|g| g.next_due).min()?;
        Some(earliest.max(s.gate().unwrap_or(earliest)))
    }

    /// The read due now, oldest due first: at most one, so that commands go between
    /// reads and they stay spaced.
    pub fn take_due(&self, now: Instant) -> Vec<PollJob> {
        let mut s = self.lock();
        if s.gate().is_some_and(|gate| now < gate) {
            return Vec::new();
        }
        let due = s
            .groups
            .iter_mut()
            .filter(|(_, g)| g.next_due <= now)
            .min_by_key(|(_, g)| g.next_due);
        let Some((key, group)) = due else {
            return Vec::new();
        };
        let job = PollJob {
            page: key.page,
            req: key.req,
            val: key.val,
            len: key.len,
        };
        let period = if group.failures >= FAILURES_BEFORE_ERROR {
            FAILED_PERIOD
        } else {
            group.period
        };
        // Fallen a period behind: skip ahead rather than burst.
        group.next_due = group
            .next_due
            .checked_add(period)
            .filter(|next| *next > now)
            .or_else(|| now.checked_add(period))
            .unwrap_or(now);
        s.last_read = Some(now);
        vec![job]
    }

    /// A read finished. An error sends nothing (spec 8.4); the third in a row gives
    /// `poll.error`, once, and the group slows until a read succeeds.
    pub fn finished(&self, job: &PollJob, reply: Result<Vec<u8>, TransportError>, now: Instant) {
        let key = Key {
            page: job.page,
            req: job.req,
            val: job.val,
            len: job.len,
        };
        let mut guard = self.lock();
        let s = &mut *guard;
        let before = s.subscribers.len();
        s.subscribers.retain(|_, sub| !sub.outbox.is_closed());
        if s.subscribers.len() != before {
            s.rebuild(now);
        }
        let Some(group) = s.groups.get_mut(&key) else {
            return; // Everyone left while it ran.
        };
        let group_period = group.period;
        match reply {
            Ok(payload) => {
                group.failures = 0;
                let half = group_period.checked_div(2).unwrap_or(group_period);
                for ((_, slot), sub) in s.subscribers.iter_mut().filter(|(_, sub)| sub.key == key) {
                    // A slower subscriber takes an answer when its own period is due,
                    // give or take half the group's.
                    let due = sub.last_sent.is_none_or(|at| {
                        now.saturating_duration_since(at).saturating_add(half) >= period_of(sub.hz)
                    });
                    if due {
                        sub.last_sent = Some(now);
                        sub.outbox.push_poll(sub.handle, *slot, payload.clone());
                    }
                }
            }
            Err(e) => {
                let reported = group.failures >= FAILURES_BEFORE_ERROR;
                group.failures = group.failures.saturating_add(1).min(FAILURES_BEFORE_ERROR);
                if !reported && group.failures == FAILURES_BEFORE_ERROR {
                    let code = match e {
                        TransportError::Disconnected | TransportError::NotFound => "no_device",
                        _ => "busy",
                    };
                    for ((_, slot), sub) in s.subscribers.iter().filter(|(_, sub)| sub.key == key) {
                        let event = json!({
                            "t": "poll.error",
                            "handle": sub.handle,
                            "slot": slot,
                            "code": code,
                            "msg": e.to_string(),
                        });
                        sub.outbox.push_reliable(Outbound::Event(event.to_string()));
                    }
                }
            }
        }
    }

    /// `poll.subscribe`: what each slot was granted, in request order, inside the
    /// budget. A slot the session already uses is replaced. `Err` changes nothing.
    /// `handle` and `outbox` are where its frames go. Every role may poll: what may be
    /// polled is classed *read* (spec 6.3).
    pub fn subscribe(
        &self,
        handle: u8,
        session: SessionId,
        outbox: &Arc<Outbox>,
        policy: &Policy,
        specs: &[PollSpec],
        now: Instant,
    ) -> Result<Vec<PollGrant>, PollRefused> {
        // -- WP PGHDEV --
        self.subscribe_routed(
            PollRoute::LEGACY,
            handle,
            session,
            outbox,
            policy,
            specs,
            now,
        )
    }

    // -- WP PGHDEV --
    /// [`Polls::subscribe`] on a device and for a session as `route` says.
    #[allow(clippy::too_many_arguments)]
    pub fn subscribe_routed(
        &self,
        route: PollRoute,
        handle: u8,
        session: SessionId,
        outbox: &Arc<Outbox>,
        policy: &Policy,
        specs: &[PollSpec],
        now: Instant,
    ) -> Result<Vec<PollGrant>, PollRefused> {
        check(specs, policy, route)?;
        let mut s = self.lock();
        let rates = s.grant(session, specs).ok_or(PollRefused::Busy)?;
        let mut granted = Vec::with_capacity(specs.len());
        for (r, hz) in specs.iter().zip(rates) {
            s.subscribers.insert(
                (session, r.slot),
                Subscriber {
                    handle,
                    key: Key::of(r),
                    hz,
                    outbox: Arc::clone(outbox),
                    last_sent: None,
                },
            );
            granted.push(PollGrant { slot: r.slot, hz });
        }
        s.rebuild(now);
        Ok(granted)
    }

    /// Whether the worker needs a wake-up for a change just made: true once, until it
    /// next asks [`Polls::next_due`], so a flood of subscribes queues one wake.
    pub(crate) fn take_wake(&self) -> bool {
        !std::mem::replace(&mut self.lock().wake_sent, true)
    }

    /// A client sent a chunk of its own bulk session: no read for a moment.
    pub(crate) fn hold_for_chunks(&self, now: Instant) {
        self.lock().hold_until = now.checked_add(CHUNK_HOLD);
    }

    /// `poll.unsubscribe`. A slot it does not use is ignored.
    pub fn unsubscribe(&self, session: SessionId, slots: &[u8], now: Instant) {
        let mut s = self.lock();
        for slot in slots {
            s.subscribers.remove(&(session, *slot));
        }
        s.rebuild(now);
    }

    /// A session ended.
    pub fn session_closed(&self, session: SessionId) {
        let mut s = self.lock();
        s.subscribers.retain(|(owner, _), _| *owner != session);
        s.rebuild(Instant::now());
    }

    /// Whether a session other than `excluding` polls one of these opcodes, for the
    /// analyser's STOP arbitration (spec 7.15).
    /// -- WP PGHDEV -- A paged poll counts as its 1.1.6 equivalent (`MEAS.RTA_BANDS`
    /// as `RTA_GET_BANDS`): analyser sharing by id on a paged device (spec 7.15).
    pub fn others_poll(&self, opcodes: &[u8], excluding: SessionId) -> bool {
        self.lock().subscribers.iter().any(|((owner, _), sub)| {
            *owner != excluding
                && sub
                    .key
                    .legacy_opcode()
                    .is_some_and(|o| opcodes.contains(&o))
                && !sub.outbox.is_closed()
        })
    }
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_hb3 {
    use dspi_proto::generated::opcodes as op;

    use super::*;
    use crate::session::Pop;

    const WAIT: Duration = Duration::ZERO;

    fn meter(slot: u8, hz: u32) -> PollSpec {
        PollSpec {
            slot,
            req: op::REQ_GET_STATUS,
            val: 9,
            idx: 2,
            len: 41,
            hz,
        }
    }

    fn sub(p: &Polls, session: SessionId, out: &Arc<Outbox>, specs: &[PollSpec], now: Instant) {
        p.subscribe(0, session, out, &Policy::embedded(), specs, now)
            .unwrap();
    }

    /// Run the worker's part in virtual time, each read answering at once. Returns
    /// how many reads each opcode and value got.
    fn run(p: &Polls, from: Instant, for_: Duration) -> (usize, Instant) {
        let (mut reads, mut now) = (0, from);
        let end = from + for_;
        while let Some(due) = p.next_due() {
            now = now.max(due);
            if now >= end {
                return (reads, end);
            }
            for job in p.take_due(now) {
                reads += 1;
                p.finished(&job, Ok(vec![1; usize::from(job.len)]), now);
            }
        }
        (reads, end)
    }

    fn polls_in(o: &Outbox) -> usize {
        let mut n = 0;
        while let Pop::Item(item) = o.pop(WAIT) {
            n += usize::from(matches!(item, Outbound::Poll { .. }));
        }
        n
    }

    /// A run that lets each poll out of the outbox as it comes, as a connection's writer
    /// does, since a queued frame is replaced by the next.
    fn deliveries(p: &Polls, from: Instant, for_: Duration, outs: &[&Outbox]) -> Vec<usize> {
        let mut seen = vec![0; outs.len()];
        let mut now = from;
        let end = from + for_;
        while let Some(due) = p.next_due() {
            now = now.max(due);
            if now >= end {
                break;
            }
            for job in p.take_due(now) {
                p.finished(&job, Ok(vec![1]), now);
            }
            for (o, n) in outs.iter().zip(&mut seen) {
                *n += polls_in(o);
            }
        }
        seen
    }

    #[test]
    fn wp_hb3_identical_polls_are_read_once_at_the_fastest_rate() {
        let t = Instant::now();
        let p = Polls::default();
        let (a, b) = (Arc::new(Outbox::new()), Arc::new(Outbox::new()));
        sub(&p, 2, &a, &[meter(0, 15)], t);
        // Another session, another slot, the same read at a third of the rate. The
        // Swift client sends wIndex 0: the firmware ignores it, so it is the same read.
        let other = PollSpec {
            idx: 0,
            ..meter(5, 5)
        };
        sub(&p, 3, &b, &[other], t);
        let seen = deliveries(&p, t, Duration::from_secs(10), &[&a, &b]);
        assert!((149..=151).contains(&seen[0]), "{seen:?}");
        assert!((49..=51).contains(&seen[1]), "own rate: {seen:?}");

        let t = Instant::now();
        let p = Polls::default();
        sub(&p, 2, &a, &[meter(0, 15)], t);
        sub(&p, 3, &b, &[other], t);
        let (reads, _) = run(&p, t, Duration::from_secs(10));
        assert!(
            (149..=151).contains(&reads),
            "one read, fanned out: {reads}"
        );
    }

    #[test]
    fn wp_hb3_a_lower_rate_returns_when_the_fast_subscriber_leaves() {
        let t = Instant::now();
        let p = Polls::default();
        let o = Arc::new(Outbox::new());
        sub(&p, 2, &o, &[meter(0, 20)], t);
        sub(&p, 3, &o, &[meter(0, 2)], t);
        let (_, t) = run(&p, t, Duration::from_secs(1));
        p.session_closed(2);
        let (reads, _) = run(&p, t, Duration::from_secs(10));
        assert!((19..=21).contains(&reads), "2 Hz: {reads}");
    }

    #[test]
    fn wp_hb3_polling_stops_when_nobody_subscribes() {
        let t = Instant::now();
        let p = Polls::default();
        let o = Arc::new(Outbox::new());
        assert_eq!(p.next_due(), None);
        sub(&p, 2, &o, &[meter(0, 10), meter(1, 10)], t);
        sub(&p, 3, &o, &[meter(0, 10)], t);
        assert!(p.next_due().is_some());
        p.unsubscribe(2, &[0], t);
        p.unsubscribe(2, &[1], t);
        assert!(p.next_due().is_some(), "session 3 still polls it");
        p.session_closed(3);
        assert_eq!(p.next_due(), None);
        assert!(p.take_due(t + Duration::from_secs(5)).is_empty());
    }

    #[test]
    fn wp_hb3_a_closed_outbox_stops_its_poll_at_the_next_read() {
        let t = Instant::now();
        let p = Polls::default();
        let o = Arc::new(Outbox::new());
        sub(&p, 2, &o, &[meter(0, 10)], t);
        o.close();
        assert!(!p.others_poll(&[op::REQ_GET_STATUS], 9));
        let job = p.take_due(t).remove(0);
        p.finished(&job, Ok(vec![1]), t);
        assert_eq!(p.next_due(), None);
    }

    #[test]
    fn wp_hb3_a_slot_is_replaced() {
        let t = Instant::now();
        let p = Polls::default();
        let o = Arc::new(Outbox::new());
        sub(&p, 2, &o, &[meter(0, 10)], t);
        let next = PollSpec {
            len: 21,
            ..meter(0, 10)
        };
        sub(&p, 2, &o, &[next], t);
        let job = p.take_due(t).remove(0);
        assert_eq!(job.len, 21);
        p.finished(&job, Ok(vec![1]), t);
        assert!(
            p.take_due(t + Duration::from_millis(5)).is_empty(),
            "the old read went"
        );
    }

    #[test]
    fn wp_hb3_the_budget_is_shared_and_counts_a_shared_poll_once() {
        let t = Instant::now();
        let p = Polls::default();
        let policy = Policy::embedded();
        let o = Arc::new(Outbox::new());
        let bulk = |slot, val, hz| PollSpec {
            slot,
            req: op::REQ_GET_ALL_PARAMS,
            val,
            idx: 2,
            len: 8192,
            hz,
        };
        // 8200 bytes a read: 24 reads a second.
        let g = p
            .subscribe(0, 2, &o, &policy, &[bulk(0, 0, 20), bulk(1, 1, 20)], t)
            .unwrap();
        assert_eq!((g[0].hz, g[1].hz), (20, 4), "a reduced grant");
        // Full. The same read costs nothing, at up to its running rate.
        let g = p
            .subscribe(0, 3, &o, &policy, &[bulk(0, 0, 20)], t)
            .unwrap();
        assert_eq!(g[0].hz, 20);
        // Anything new gets nothing, and changes nothing.
        let e = p.subscribe(0, 3, &o, &policy, &[bulk(1, 2, 1), meter(2, 5)], t);
        assert_eq!(e, Err(PollRefused::Busy));
        assert_eq!(PollRefused::Busy.code(), "busy");
        assert!(!p.lock().subscribers.contains_key(&(3, 1)));
        // Replacing a slot gives back what it held.
        let g = p
            .subscribe(0, 2, &o, &policy, &[meter(0, 15), meter(1, 15)], t)
            .unwrap();
        assert_eq!((g[0].hz, g[1].hz), (15, 15));
    }

    #[test]
    fn wp_hb3_rates_are_capped_at_the_advertised_maximum() {
        let t = Instant::now();
        let g = Polls::default()
            .subscribe(
                0,
                2,
                &Arc::new(Outbox::new()),
                &Policy::embedded(),
                &[meter(0, 1000)],
                t,
            )
            .unwrap();
        assert_eq!(g[0], PollGrant { slot: 0, hz: 20 });
    }

    /// Small polls fit any byte budget; the reads are what the device feels.
    #[test]
    fn wp_hb3_grants_stay_inside_the_transfer_limit() {
        let t = Instant::now();
        let p = Polls::default();
        let o = Arc::new(Outbox::new());
        let tiny = |slot, val| PollSpec {
            slot,
            req: op::REQ_GET_STATUS,
            val,
            idx: 2,
            len: 1,
            hz: 20,
        };
        let mut granted = 0;
        for session in 2..40u16 {
            let specs: Vec<_> = (0..16u8)
                .map(|s| tiny(s, session * 16 + u16::from(s)))
                .collect();
            if let Ok(g) = p.subscribe(0, session, &o, &Policy::embedded(), &specs, t) {
                granted += g.iter().map(|g| g.hz).sum::<u32>();
            }
        }
        assert_eq!(granted, MAX_TRANSFERS_PER_SEC);
        let (reads, _) = run(&p, t, Duration::from_secs(10));
        let per_second = reads / 10;
        assert!(per_second <= MAX_TRANSFERS_PER_SEC as usize, "{per_second}");
    }

    /// A new group is due at once, so without the spacing a client could read the device
    /// as fast as it can subscribe.
    #[test]
    fn wp_hb3_subscribing_in_a_loop_cannot_outrun_the_limit() {
        let start = Instant::now();
        let p = Polls::default();
        let o = Arc::new(Outbox::new());
        let policy = Policy::embedded();
        let (mut reads, mut now) = (0u32, start);
        for n in 0..100_000u32 {
            now += Duration::from_micros(100);
            let spec = PollSpec {
                val: n as u16,
                ..meter(0, 20)
            };
            p.subscribe(0, 2, &o, &policy, &[spec], now).unwrap();
            for job in p.take_due(now) {
                reads += 1;
                p.finished(&job, Ok(vec![1]), now);
            }
        }
        let limit = MAX_TRANSFERS_PER_SEC * 10;
        assert!(reads <= limit + 1, "{reads} in 10 s");
    }

    #[test]
    fn wp_hb3_a_burst_of_due_polls_comes_one_at_a_time_oldest_first() {
        let t = Instant::now();
        let p = Polls::default();
        let o = Arc::new(Outbox::new());
        sub(&p, 2, &o, &[meter(0, 10)], t);
        let later = t + Duration::from_millis(1);
        sub(
            &p,
            2,
            &o,
            &[PollSpec {
                val: 7,
                ..meter(1, 10)
            }],
            later,
        );
        let now = t + Duration::from_millis(50);
        let jobs = p.take_due(now);
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].val, 9, "the older");
        assert!(p.take_due(now).is_empty(), "spaced");
        let next = p.next_due().unwrap();
        assert_eq!(next, now + period_of(MAX_TRANSFERS_PER_SEC));
        assert_eq!(p.take_due(next)[0].val, 7);
    }

    #[test]
    fn wp_hb3_a_late_worker_skips_ahead_rather_than_bursts() {
        let t = Instant::now();
        let p = Polls::default();
        let o = Arc::new(Outbox::new());
        sub(&p, 2, &o, &[meter(0, 10)], t);
        let late = t + Duration::from_secs(5);
        assert_eq!(p.take_due(late).len(), 1);
        assert_eq!(p.next_due(), Some(late + Duration::from_millis(100)));
    }

    #[test]
    fn wp_hb3_three_failures_report_once_and_slow_until_recovery() {
        let t = Instant::now();
        let p = Polls::default();
        let o = Arc::new(Outbox::new());
        sub(&p, 2, &o, &[meter(0, 10)], t);
        let job = PollJob {
            page: 0,
            req: op::REQ_GET_STATUS,
            val: 9,
            len: 41,
        };
        let fail = || Err(TransportError::Stalled { opcode: job.req });
        p.finished(&job, fail(), t);
        p.finished(&job, fail(), t);
        assert_eq!(o.pop(WAIT), Pop::Idle, "nothing yet");
        p.finished(&job, fail(), t);
        let Pop::Item(Outbound::Event(event)) = o.pop(WAIT) else {
            panic!("no poll.error");
        };
        let v: serde_json::Value = serde_json::from_str(&event).unwrap();
        assert_eq!(
            (&v["t"], &v["handle"], &v["slot"], &v["code"]),
            (&json!("poll.error"), &json!(0), &json!(0), &json!("busy"))
        );
        p.finished(&job, fail(), t);
        assert_eq!(o.pop(WAIT), Pop::Idle, "once an episode");

        assert_eq!(p.take_due(t).len(), 1);
        assert_eq!(p.next_due(), Some(t + FAILED_PERIOD), "the slow rate");

        p.finished(&job, Ok(vec![1]), t);
        assert!(matches!(o.pop(WAIT), Pop::Item(Outbound::Poll { .. })));
        let now = t + FAILED_PERIOD;
        assert_eq!(p.take_due(now).len(), 1);
        assert_eq!(
            p.next_due(),
            Some(now + Duration::from_millis(100)),
            "recovered"
        );
        p.finished(&job, fail(), now);
        p.finished(&job, fail(), now);
        p.finished(&job, Err(TransportError::Disconnected), now);
        let Pop::Item(Outbound::Event(event)) = o.pop(WAIT) else {
            panic!("a new episode reports again");
        };
        assert!(event.contains("no_device"), "{event}");
    }

    #[test]
    fn wp_hb3_a_read_that_lands_after_everyone_left_goes_nowhere() {
        let t = Instant::now();
        let p = Polls::default();
        let o = Arc::new(Outbox::new());
        sub(&p, 2, &o, &[meter(0, 10)], t);
        let job = p.take_due(t).remove(0);
        p.unsubscribe(2, &[0], t);
        p.finished(&job, Ok(vec![1]), t);
        assert_eq!(o.pop(WAIT), Pop::Idle);
    }

    #[test]
    fn wp_hb3_malformed_and_writing_polls_are_refused_and_change_nothing() {
        let t = Instant::now();
        let p = Polls::default();
        let policy = Policy::embedded();
        let o = Arc::new(Outbox::new());
        let bad = |specs: &[PollSpec]| p.subscribe(0, 2, &o, &policy, specs, t).unwrap_err();
        assert_eq!(bad(&[]).code(), "bad_request");
        assert_eq!(bad(&[meter(16, 1)]).code(), "bad_request");
        assert_eq!(bad(&[meter(0, 0)]).code(), "bad_request");
        assert_eq!(bad(&[meter(1, 1), meter(1, 2)]).code(), "bad_request");
        let many: Vec<_> = (0..17).map(|n| meter(n % 16, 1)).collect();
        assert_eq!(bad(&many).code(), "bad_request");
        for len in [0, MAX_LEN + 1] {
            assert_eq!(
                bad(&[PollSpec { len, ..meter(0, 1) }]).code(),
                "bad_request"
            );
        }
        // A save, and a command the table does not list: not reads, so not polled.
        for req in [op::REQ_SAVE_PARAMS, op::REQ_EXTENSION] {
            let e = bad(&[PollSpec { req, ..meter(0, 1) }]);
            assert_eq!(e, PollRefused::Denied);
            assert_eq!(e.code(), "denied");
        }
        assert_eq!(p.next_due(), None);
    }

    #[test]
    fn wp_hb3_others_poll_is_about_others() {
        let t = Instant::now();
        let p = Polls::default();
        let o = Arc::new(Outbox::new());
        let bands = PollSpec {
            req: op::REQ_RTA_GET_BANDS,
            ..meter(0, 10)
        };
        sub(&p, 2, &o, &[bands], t);
        assert!(p.others_poll(&[op::REQ_RTA_GET_BANDS], 3));
        assert!(!p.others_poll(&[op::REQ_RTA_GET_BANDS], 2), "only itself");
        assert!(!p.others_poll(&[op::REQ_RTA_GET_BINS], 3), "another opcode");
    }

    #[test]
    fn wp_hb3_a_clients_chunk_session_holds_polls_back() {
        let t = Instant::now();
        let p = Polls::default();
        let o = Arc::new(Outbox::new());
        sub(&p, 2, &o, &[meter(0, 10)], t);
        p.hold_for_chunks(t);
        assert!(p.take_due(t + CHUNK_HOLD / 2).is_empty());
        assert_eq!(p.next_due(), Some(t + CHUNK_HOLD));
        assert_eq!(p.take_due(t + CHUNK_HOLD).len(), 1);
    }

    #[test]
    fn wp_hb3_one_wake_until_the_worker_looks() {
        let p = Polls::default();
        assert!(p.take_wake());
        assert!(!p.take_wake());
        let _ = p.next_due();
        assert!(p.take_wake());
    }
}

// -- WP PGHDEV --
#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_pghdev {
    use dspi_proto::generated::opcodes as op;
    use dspi_proto::paged::cmds;

    use super::*;

    const PAGED_13: PollRoute = PollRoute {
        paged: true,
        link_minor: LINK_13_MINOR,
    };

    fn spec(slot: u8, id: CmdId) -> PollSpec {
        PollSpec {
            slot,
            req: id.cmd(),
            val: 0,
            idx: id.w_index(),
            len: 4,
            hz: 5,
        }
    }

    /// The same command number on two pages is two groups, two reads, each with its
    /// own page; the low byte of `idx` still is not part of the key.
    #[test]
    fn wp_pghdev_the_same_command_on_two_pages_is_two_reads() {
        let (p, o, t) = (Polls::default(), Arc::new(Outbox::new()), Instant::now());
        let volume = cmds::ROUTE_MASTER_VOLUME;
        let other = CmdId::new(0x03, volume.cmd());
        let policy = Policy::embedded();
        p.subscribe_routed(
            PAGED_13,
            0,
            1,
            &o,
            &policy,
            &[spec(0, volume), spec(1, other)],
            t,
        )
        .unwrap();
        // The Mac's interface 0 in the low byte: the same read as slot 0's.
        let mac = PollSpec {
            idx: volume.w_index() & 0xFF00,
            ..spec(2, volume)
        };
        p.subscribe_routed(PAGED_13, 0, 2, &o, &policy, &[mac], t)
            .unwrap();
        let mut pages = Vec::new();
        let mut now = t;
        for _ in 0..4 {
            now = now.max(p.next_due().unwrap());
            for job in p.take_due(now) {
                pages.push((job.page, job.req));
                p.finished(&job, Ok(vec![0; 4]), now);
            }
        }
        pages.sort_unstable();
        pages.dedup();
        assert_eq!(pages, vec![(0x03, volume.cmd()), (0x05, volume.cmd())]);
    }

    /// Each poll is judged as its CMD would be (PLAN-PG section 1.5).
    #[test]
    fn wp_pghdev_polls_are_routed_by_generation_and_minor() {
        let (p, o, t) = (Polls::default(), Arc::new(Outbox::new()), Instant::now());
        let policy = Policy::embedded();
        let volume = spec(0, cmds::ROUTE_MASTER_VOLUME);
        let page0 = PollSpec {
            req: op::REQ_GET_MASTER_VOLUME,
            idx: 2,
            ..volume
        };
        let legacy13 = PollRoute {
            paged: false,
            link_minor: LINK_13_MINOR,
        };
        let paged12 = PollRoute {
            paged: true,
            link_minor: 2,
        };
        let sub = |route, s: PollSpec| p.subscribe_routed(route, 0, 1, &o, &policy, &[s], t);
        assert_eq!(sub(legacy13, volume), Err(PollRefused::Unsupported));
        assert!(sub(legacy13, page0).is_ok());
        assert!(sub(PAGED_13, volume).is_ok());
        assert_eq!(sub(PAGED_13, page0), Err(PollRefused::PageZero));
        assert_eq!(PollRefused::PageZero.code(), "unsupported");
        assert!(sub(paged12, page0).is_ok(), "translated for a 1.2 session");
        // A paged write is not polled; neither is one by its page-0 form.
        let start = spec(0, cmds::MEAS_RTA_START);
        assert_eq!(sub(PAGED_13, start), Err(PollRefused::Denied));
    }

    /// The analyser's STOP arbitration sees a paged band poll as `RTA_GET_BANDS`.
    #[test]
    fn wp_pghdev_others_poll_counts_a_paged_poll_as_its_equivalent() {
        let (p, o, t) = (Polls::default(), Arc::new(Outbox::new()), Instant::now());
        let bands = PollSpec {
            len: 82,
            ..spec(0, cmds::MEAS_RTA_BANDS)
        };
        p.subscribe_routed(PAGED_13, 0, 7, &o, &Policy::embedded(), &[bands], t)
            .unwrap();
        assert!(p.others_poll(&[op::REQ_RTA_GET_BANDS], 1));
        assert!(!p.others_poll(&[op::REQ_RTA_GET_BANDS], 7), "its own");
        assert!(!p.others_poll(&[op::REQ_GET_STATUS], 1));
    }
}
