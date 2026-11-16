//! Sessions and their outbound queues (docs/core-merge/HUB.md section 6).
//!
//! An [`Outbox`] is what lets a slow client be slow alone: a publisher pushes and
//! never blocks, and the consumer (a connection's writer thread, or the app's
//! notification reader) pops at its own pace. Lock level L4: nothing is locked and
//! nothing blocks while an outbox's mutex is held.

use std::collections::{BTreeSet, VecDeque};
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use dspi_link::frames::CmdResponse;
use dspi_link::status;

/// A session id, as `origin` in NOTIFY frames (spec 8.3).
pub type SessionId = u16;

/// `origin` of a change no session is known to have made.
pub const NO_ORIGIN: SessionId = 0;
/// The app's in-process session.
pub const LOCAL_SESSION_ID: SessionId = 1;
/// The first id a network session may get.
pub const FIRST_REMOTE_SESSION_ID: SessionId = 2;
/// The hub's own reads and writes (snapshot warm-up, board name adoption).
pub const INTERNAL_SESSION_ID: SessionId = 0xFFFF;
// -- WP P1CFIX -- F6 (coordinator, decision 4)
/// A guarded apply's write-back of the settings outside the image
/// (`apply_extras.rs`): its own id, so only it passes locks in the router's revert
/// window, never the hub's other internal writes (the hub-system thread's
/// keep-aligned and lock-error actions run as [`INTERNAL_SESSION_ID`]).
pub const GUARD_SESSION_ID: SessionId = 0xFFFE;

/// NOTIFY frames held per subscriber before the oldest are dropped.
pub const NOTIFY_CAPACITY: usize = 256;
/// Responses and events held before the consumer counts as gone. Responses are
/// bounded by `max_inflight`, events by how often the device list changes, so
/// only a consumer that stopped reading gets here.
pub const RELIABLE_CAPACITY: usize = 1024;
/// Close code for a consumer that stopped reading (spec 5).
pub const CLOSE_STOPPED_READING: u16 = 1011;
/// Close code for a hub shutting down (spec 5).
pub const CLOSE_SHUTTING_DOWN: u16 = 4003;
/// Close code for a revoked token (spec 5).
pub const CLOSE_REVOKED: u16 = 4002;

/// What a session may do (spec 6.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Role {
    /// Reads, polls, notifications.
    Viewer = 0,
    /// Also commands classed `control`.
    Control = 1,
    /// Everything.
    Admin = 2,
}

impl Role {
    fn from_raw(raw: u8) -> Self {
        match raw {
            0 => Role::Viewer,
            1 => Role::Control,
            _ => Role::Admin,
        }
    }
}

/// What a device worker needs to know about the session behind a command, taken
/// at submission. The role can change between commands, never within one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionCtx {
    /// The session.
    pub id: SessionId,
    /// Its role now.
    pub role: Role,
    /// Exempt from `max_inflight`: a caller that blocks on every command cannot
    /// exceed it, and has no way to retry `RATE_LIMITED` (the app).
    pub exempt_inflight: bool,
}

/// One session on the hub.
#[derive(Debug)]
pub struct Session {
    id: SessionId,
    client_id: Option<u32>,
    role: AtomicU8,
    outbox: Arc<Outbox>,
    exempt_inflight: bool,
    // -- WP P3RO --
    /// Its connection is TLS (set once, where the server opens it).
    encrypted: std::sync::atomic::AtomicBool,
}

impl Session {
    pub(crate) fn new(
        id: SessionId,
        role: Role,
        client_id: Option<u32>,
        exempt_inflight: bool,
    ) -> Arc<Self> {
        Arc::new(Self {
            id,
            client_id,
            role: AtomicU8::new(role as u8),
            outbox: Arc::new(Outbox::new()),
            exempt_inflight,
            // -- WP P3RO --
            encrypted: std::sync::atomic::AtomicBool::new(false),
        })
    }

    // -- WP P3RO -- (doc: WP P3FRZ)
    /// Whether what this session sends and receives is protected from the network: what
    /// a `tls_only` family (the rollout, `hub-api.md` 21.1) checks. True for the session
    /// of a TLS connection (set where the server opens it), for an API key's REST session
    /// while it serves a request that came over TLS (set per request: the key's one
    /// session serves its plain and its TLS requests, each judged by its own connection,
    /// WP P3RS), and for the console principal of the hub's own machine
    /// ([`HubHandle::open_console_principal`](crate::HubHandle::open_console_principal),
    /// WP P3TM: nothing it sends crosses a network). False for a plaintext connection and
    /// for every other session the hub opens itself (the app's, a principal's, a
    /// child's).
    pub fn encrypted(&self) -> bool {
        self.encrypted.load(Ordering::SeqCst)
    }

    // -- WP P3RO -- (doc: WP P3FRZ)
    /// Set [`Session::encrypted`]. Called where the server opens a connection's session
    /// (from the connection's TLS state), by the REST path before each request of a
    /// key's session (that request's connection), and by
    /// [`HubHandle::open_console_principal`](crate::HubHandle::open_console_principal)
    /// (`true`). Never for any other session the hub opens itself.
    pub(crate) fn set_encrypted(&self, on: bool) {
        self.encrypted.store(on, Ordering::SeqCst);
    }

    /// The session id.
    pub fn id(&self) -> SessionId {
        self.id
    }

    /// The paired client it authenticated as (`cid`), if any.
    pub fn client_id(&self) -> Option<u32> {
        self.client_id
    }

    /// Its role now.
    pub fn role(&self) -> Role {
        Role::from_raw(self.role.load(Ordering::Relaxed))
    }

    /// Change the role. The next command is judged by it (spec 6.2).
    pub fn set_role(&self, role: Role) {
        self.role.store(role as u8, Ordering::Relaxed);
    }

    /// What the session's connection has to send.
    pub fn outbox(&self) -> &Arc<Outbox> {
        &self.outbox
    }

    /// The command-time view of this session.
    pub fn ctx(&self) -> SessionCtx {
        SessionCtx {
            id: self.id,
            role: self.role(),
            exempt_inflight: self.exempt_inflight,
        }
    }
}

/// One thing to send to a session's consumer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outbound {
    /// A device notification. `seq` is this subscriber's Link sequence (spec
    /// 8.3), taken when it is queued, so a dropped frame shows as a gap.
    Notify {
        /// Link sequence number, per subscriber.
        seq: u16,
        /// The device.
        handle: u8,
        /// The session the change is attributed to.
        origin: SessionId,
        /// The verbatim v2 packet.
        packet: Arc<[u8]>,
    },
    /// Re-read state (spec 8.6).
    Resync {
        /// The device.
        handle: u8,
        /// 0 dropped notifications, 1 device reattached, 2 hub cache rebuilt.
        reason: u8,
    },
    /// A poll result; a newer one for the same slot replaces it while queued.
    Poll {
        /// The device.
        handle: u8,
        /// The session's slot.
        slot: u8,
        /// The verbatim GET reply.
        payload: Vec<u8>,
    },
    /// A CMD response. Never dropped.
    Response(CmdResponse),
    /// A JSON control-plane message. Never dropped.
    Event(String),
    /// The last thing a connection sends: close the WebSocket with this code.
    Close {
        /// The WebSocket close code (spec 5).
        code: u16,
        /// The reason text.
        reason: String,
    },
}

/// What [`Outbox::pop`] found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Pop {
    /// The next thing to send.
    Item(Outbound),
    /// Nothing arrived within the timeout.
    Idle,
    /// The outbox is closed and empty.
    Closed,
}

#[derive(Debug, Default)]
struct OutboxState {
    queue: VecDeque<Outbound>,
    /// How many of `queue` are [`Outbound::Notify`].
    notifies: usize,
    /// How many of `queue` are responses and events.
    reliable: usize,
    /// Handles that lost notifications and are owed one RESYNC.
    behind: BTreeSet<u8>,
    next_seq: u16,
    closed: bool,
}

/// A session's bounded outbound queue. Push never blocks and never fails: what
/// cannot be queued is dropped by the rules in docs/core-merge/HUB.md section 6.
#[derive(Debug)]
pub struct Outbox {
    state: Mutex<OutboxState>,
    ready: Condvar,
    notify_capacity: usize,
    reliable_capacity: usize,
}

impl Default for Outbox {
    fn default() -> Self {
        Self::new()
    }
}

impl Outbox {
    /// An outbox with the standard limits.
    pub fn new() -> Self {
        Self::with_capacity(NOTIFY_CAPACITY, RELIABLE_CAPACITY)
    }

    /// An outbox with these limits (tests shorten them).
    pub fn with_capacity(notify_capacity: usize, reliable_capacity: usize) -> Self {
        Self {
            state: Mutex::new(OutboxState::default()),
            ready: Condvar::new(),
            notify_capacity: notify_capacity.max(1),
            reliable_capacity: reliable_capacity.max(1),
        }
    }

    fn lock(&self) -> MutexGuard<'_, OutboxState> {
        // Every update below is complete before anything that could panic.
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Queue a notification. When the queue is full the oldest notification goes,
    /// and that device is owed a RESYNC once the queue has drained.
    ///
    /// -- WP P2HS -- Returns the handle whose notification was dropped, if one was.
    pub fn push_notify(&self, handle: u8, origin: SessionId, packet: Arc<[u8]>) -> Option<u8> {
        let mut s = self.lock();
        if s.closed {
            return None;
        }
        let mut dropped = None;
        let seq = s.next_seq;
        s.next_seq = seq.wrapping_add(1);
        if s.notifies >= self.notify_capacity {
            let oldest = s
                .queue
                .iter()
                .position(|o| matches!(o, Outbound::Notify { .. }));
            if let Some(Outbound::Notify { handle: lost, .. }) =
                oldest.and_then(|i| s.queue.remove(i))
            {
                s.behind.insert(lost);
                s.notifies = s.notifies.saturating_sub(1);
                dropped = Some(lost);
            }
        }
        s.queue.push_back(Outbound::Notify {
            seq,
            handle,
            origin,
            packet,
        });
        s.notifies = s.notifies.saturating_add(1);
        drop(s);
        self.ready.notify_all();
        dropped
    }

    /// Queue a poll result, replacing one still queued for the same slot.
    pub fn push_poll(&self, handle: u8, slot: u8, payload: Vec<u8>) {
        let mut s = self.lock();
        if s.closed {
            return;
        }
        let queued = s.queue.iter_mut().find_map(|o| match o {
            Outbound::Poll {
                handle: h,
                slot: sl,
                payload,
            } if *h == handle && *sl == slot => Some(payload),
            _ => None,
        });
        match queued {
            Some(old) => *old = payload,
            None => s.queue.push_back(Outbound::Poll {
                handle,
                slot,
                payload,
            }),
        }
        drop(s);
        self.ready.notify_all();
    }

    /// Queue a RESYNC, behind what is already queued.
    pub fn push_resync(&self, handle: u8, reason: u8) {
        let mut s = self.lock();
        if s.closed {
            return;
        }
        s.queue.push_back(Outbound::Resync { handle, reason });
        drop(s);
        self.ready.notify_all();
    }

    /// Queue a response or an event. A consumer so far behind that this would pass
    /// the limit has stopped reading: everything queued goes and it gets a close.
    pub fn push_reliable(&self, item: Outbound) {
        let mut s = self.lock();
        if s.closed {
            return;
        }
        if s.reliable >= self.reliable_capacity {
            s.queue.clear();
            s.notifies = 0;
            s.reliable = 0;
            s.behind.clear();
            s.queue.push_back(Outbound::Close {
                code: CLOSE_STOPPED_READING,
                reason: "client stopped reading".into(),
            });
            s.closed = true;
        } else {
            s.queue.push_back(item);
            s.reliable = s.reliable.saturating_add(1);
        }
        drop(s);
        self.ready.notify_all();
    }

    /// Send this close after what is queued, and accept nothing more.
    pub fn close_with(&self, code: u16, reason: &str) {
        let mut s = self.lock();
        if !s.closed {
            s.queue.push_back(Outbound::Close {
                code,
                reason: reason.to_owned(),
            });
            s.closed = true;
        }
        drop(s);
        self.ready.notify_all();
    }

    /// Accept nothing more; what is queued can still be popped.
    pub fn close(&self) {
        self.lock().closed = true;
        self.ready.notify_all();
    }

    /// Whether the outbox accepts no more.
    pub fn is_closed(&self) -> bool {
        self.lock().closed
    }

    /// The next item, waiting at most `timeout`. Once the queue has drained, a
    /// device that lost notifications gets one [`Outbound::Resync`] (reason 0).
    pub fn pop(&self, timeout: Duration) -> Pop {
        let start = Instant::now();
        let mut s = self.lock();
        loop {
            if let Some(item) = s.queue.pop_front() {
                match item {
                    Outbound::Notify { .. } => s.notifies = s.notifies.saturating_sub(1),
                    Outbound::Response(_) | Outbound::Event(_) => {
                        s.reliable = s.reliable.saturating_sub(1);
                    }
                    _ => {}
                }
                return Pop::Item(item);
            }
            if let Some(handle) = s.behind.pop_first() {
                return Pop::Item(Outbound::Resync { handle, reason: 0 });
            }
            if s.closed {
                return Pop::Closed;
            }
            let left = timeout.saturating_sub(start.elapsed());
            if left.is_zero() {
                return Pop::Idle;
            }
            s = self
                .ready
                .wait_timeout(s, left)
                .map(|(g, _)| g)
                .unwrap_or_else(|e| e.into_inner().0);
        }
    }
}

/// How a command is answered: exactly once. Dropped unanswered (the device worker
/// ended with the command still queued, a transport panicked) it answers
/// `NO_DEVICE`, so a caller never waits for a command nobody will run.
pub struct Completion {
    tag: u16,
    reply: Option<Box<dyn FnOnce(CmdResponse) + Send>>,
    // -- fixup hwlk1f (F1) --
    /// Set when the transfer failed because the device stalled it: the Link
    /// status `ERROR` also stands for a bus fault, a short or malformed reply.
    stalled: Option<Arc<std::sync::atomic::AtomicBool>>,
}

impl Completion {
    /// A completion for the request with this tag. `reply` runs on the device's
    /// worker, or on the submitting thread if the command is refused at once, so it
    /// must not block: it would hold up every session on that device.
    pub fn new(tag: u16, reply: impl FnOnce(CmdResponse) + Send + 'static) -> Self {
        Self {
            tag,
            reply: Some(Box::new(reply)),
            stalled: None,
        }
    }

    // -- fixup hwlk1f (F1) --
    /// Also tell `flag` whether the device stalled the transfer (a firmware's
    /// answer), as opposed to any other failure the status `ERROR` covers.
    pub(crate) fn noting_stall(mut self, flag: Arc<std::sync::atomic::AtomicBool>) -> Self {
        self.stalled = Some(flag);
        self
    }

    /// The transfer failed with `e`: noted for [`Self::noting_stall`].
    pub(crate) fn note_failure(&self, e: &dspi_transport::TransportError) {
        if let (Some(flag), dspi_transport::TransportError::Stalled { .. }) = (&self.stalled, e) {
            flag.store(true, Ordering::SeqCst);
        }
    }

    /// A completion that queues the response on `outbox` (a network session).
    pub fn to_outbox(tag: u16, outbox: Arc<Outbox>) -> Self {
        Self::new(tag, move |r| outbox.push_reliable(Outbound::Response(r)))
    }

    /// Answer the command.
    pub fn answer(mut self, status: u8, payload: Vec<u8>) {
        if let Some(reply) = self.reply.take() {
            reply(CmdResponse {
                tag: self.tag,
                status,
                payload,
            });
        }
    }
}

impl Drop for Completion {
    fn drop(&mut self) {
        if let Some(reply) = self.reply.take() {
            reply(CmdResponse {
                tag: self.tag,
                status: status::NO_DEVICE,
                payload: Vec::new(),
            });
        }
    }
}

// -- WP P2PA --
/// A child session of a principal (PLAN-P2 P2PA, the session-id rule M2): the same role
/// and paired client as its parent, its parent's name for attribution, and **its own id
/// from the remote range** (`next_session_id`, through
/// [`crate::HubHandle::open_principal`]), so the policy, the limits, the lock and the
/// voltage rule judge what it sends as any client's, and other sessions see its writes
/// as foreign ones. Never `LOCAL_SESSION_ID`, `INTERNAL_SESSION_ID` or
/// `GUARD_SESSION_ID`.
///
/// It hears nothing as a session: what it needs of a device comes through the transport
/// opened for it ([`crate::api::transport::PrincipalTransport`]). Close it with
/// [`crate::HubHandle::close_session`] when done; it is also done once its parent is
/// ([`ChildSession::parent_gone`]).
///
/// **It acts for its parent** in two things only: a device lock (spec 7.7) the parent
/// holds lets the child's commands through (the router), and a PIN unlock the parent
/// made is the child's too (the limits). Both links are the hub's, set when the child
/// opens ([`crate::HubHandle::open_child`]) and on each device it reaches
/// ([`crate::HubHandle::open_as`]), and dropped when either session closes, so a child
/// never keeps them after its parent has gone, even if the parent's id is reused. Its
/// role is its own (the parent's, followed on each use), and the voltage rule judges it
/// as any network session.
#[derive(Debug)]
pub struct ChildSession {
    session: Arc<Session>,
    parent: std::sync::Weak<Session>,
    parent_id: SessionId,
}

impl ChildSession {
    pub(crate) fn new(session: Arc<Session>, parent: &Arc<Session>) -> Self {
        Self {
            session,
            parent: Arc::downgrade(parent),
            parent_id: parent.id(),
        }
    }

    /// The child's own session.
    pub fn session(&self) -> &Arc<Session> {
        &self.session
    }

    /// The child's id: from the remote range, never the parent's.
    pub fn id(&self) -> SessionId {
        self.session.id()
    }

    /// The parent's id.
    pub fn parent_id(&self) -> SessionId {
        self.parent_id
    }

    /// Whether the parent has ended: dropped, or closed by the hub (which drops the
    /// child's link to it). An outbox the parent stopped reading does not count: a
    /// principal that nobody reads (a schedule's) is still open.
    pub fn parent_gone(&self, hub: &crate::hub::HubHandle) -> bool {
        self.parent.upgrade().is_none()
            || hub.inner.limits.acts_for(self.id()) != Some(self.parent_id)
    }

    /// Take the parent's role now (an Admin may have changed it since): the next
    /// command is judged by it. False once the parent has gone.
    pub fn follow_parent(&self, hub: &crate::hub::HubHandle) -> bool {
        if self.parent_gone(hub) {
            return false;
        }
        match self.parent.upgrade() {
            Some(p) => {
                self.session.set_role(p.role());
                true
            }
            None => false,
        }
    }
}

impl crate::hub::HubHandle {
    /// Open a child session of `parent` ([`ChildSession`]): its role and client, its
    /// label's name, an id of its own from the remote range, acting for `parent`'s device
    /// lock and PIN unlocks. It follows no device: the session's relay subscription goes
    /// at once, and its outbox is closed.
    pub fn open_child(&self, parent: &Arc<Session>) -> Result<ChildSession, crate::HubError> {
        let name = self.session_label(parent.id()).by().unwrap_or_default();
        let session = self.open_principal(parent.role(), parent.client_id(), &name)?;
        self.inner.relay.remove_session(session.id());
        session.outbox().close();
        self.inner.limits.set_acts_for(session.id(), parent.id());
        let child = ChildSession::new(session, parent);
        // A parent closed meanwhile has had its links dropped already: none for it now.
        if parent.outbox().is_closed() {
            self.close_session(child.id());
            return Err(crate::HubError::Stopped);
        }
        Ok(child)
    }
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_hb0 {
    use super::*;

    const NOW: Duration = Duration::ZERO;

    fn packet(n: u8) -> Arc<[u8]> {
        Arc::from(vec![2, 1, 0, n, 0, 0, 0, 0])
    }

    /// The slow client: the publisher never blocks, the oldest frames go, the
    /// sequence shows the gap, and one RESYNC follows the frames that are left.
    #[test]
    fn wp_hb0_a_slow_consumer_loses_the_oldest_and_gets_one_resync() {
        let o = Outbox::with_capacity(4, 16);
        for n in 0..10 {
            o.push_notify(3, 0, packet(n));
        }
        let mut seqs = Vec::new();
        loop {
            match o.pop(NOW) {
                Pop::Item(Outbound::Notify { seq, .. }) => seqs.push(seq),
                Pop::Item(Outbound::Resync { handle, reason }) => {
                    assert_eq!((handle, reason), (3, 0));
                    break;
                }
                other => panic!("{other:?}"),
            }
        }
        assert_eq!(seqs, [6, 7, 8, 9], "the newest stay");
        assert_eq!(o.pop(NOW), Pop::Idle, "one RESYNC only");
    }

    #[test]
    fn wp_hb0_each_device_that_lost_frames_gets_its_own_resync() {
        let o = Outbox::with_capacity(1, 16);
        o.push_notify(1, 0, packet(0));
        o.push_notify(2, 0, packet(1));
        o.push_notify(3, 0, packet(2));
        let mut resyncs = Vec::new();
        while let Pop::Item(item) = o.pop(NOW) {
            if let Outbound::Resync { handle, .. } = item {
                resyncs.push(handle);
            }
        }
        assert_eq!(resyncs, [1, 2]);
    }

    #[test]
    fn wp_hb0_a_notification_flood_never_drops_a_response() {
        let o = Outbox::with_capacity(2, 16);
        let response = Outbound::Response(CmdResponse {
            tag: 7,
            status: status::OK,
            payload: Vec::new(),
        });
        o.push_reliable(response.clone());
        for n in 0..50 {
            o.push_notify(0, 0, packet(n));
        }
        assert_eq!(o.pop(NOW), Pop::Item(response));
    }

    #[test]
    fn wp_hb0_a_consumer_that_stopped_reading_gets_a_close() {
        let o = Outbox::with_capacity(2, 2);
        for tag in 0..3 {
            o.push_reliable(Outbound::Event(format!("{tag}")));
        }
        assert!(matches!(
            o.pop(NOW),
            Pop::Item(Outbound::Close {
                code: CLOSE_STOPPED_READING,
                ..
            })
        ));
        assert_eq!(o.pop(NOW), Pop::Closed);
    }

    #[test]
    fn wp_hb0_a_newer_poll_replaces_the_queued_one() {
        let o = Outbox::new();
        o.push_poll(0, 1, vec![1]);
        o.push_poll(0, 2, vec![2]);
        o.push_poll(0, 1, vec![3]);
        assert_eq!(
            o.pop(NOW),
            Pop::Item(Outbound::Poll {
                handle: 0,
                slot: 1,
                payload: vec![3]
            })
        );
        assert!(matches!(
            o.pop(NOW),
            Pop::Item(Outbound::Poll { slot: 2, .. })
        ));
        assert_eq!(o.pop(NOW), Pop::Idle);
    }

    #[test]
    fn wp_hb0_a_closed_outbox_still_gives_up_what_it_holds() {
        let o = Outbox::new();
        o.push_notify(0, 0, packet(0));
        o.close_with(CLOSE_SHUTTING_DOWN, "bye");
        o.push_notify(0, 0, packet(1));
        assert!(matches!(
            o.pop(NOW),
            Pop::Item(Outbound::Notify { seq: 0, .. })
        ));
        assert!(matches!(
            o.pop(NOW),
            Pop::Item(Outbound::Close {
                code: CLOSE_SHUTTING_DOWN,
                ..
            })
        ));
        assert_eq!(o.pop(NOW), Pop::Closed);
    }

    #[test]
    fn wp_hb0_a_dropped_completion_answers_no_device() {
        let (tx, rx) = std::sync::mpsc::channel();
        drop(Completion::new(9, move |r| tx.send(r).unwrap()));
        let r = rx.try_recv().unwrap();
        assert_eq!((r.tag, r.status), (9, status::NO_DEVICE));
    }

    #[test]
    fn wp_hb0_a_completion_answers_once() {
        let (tx, rx) = std::sync::mpsc::channel();
        Completion::new(9, move |r| tx.send(r).unwrap()).answer(status::OK, vec![1]);
        assert_eq!(rx.try_recv().unwrap().payload, [1]);
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn wp_hb0_the_role_is_read_at_each_command() {
        let s = Session::new(5, Role::Viewer, Some(3), false);
        assert_eq!(s.ctx().role, Role::Viewer);
        s.set_role(Role::Control);
        assert_eq!(s.ctx().role, Role::Control);
        assert_eq!(s.client_id(), Some(3));
    }
}
