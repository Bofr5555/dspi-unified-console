//! Per-device admission and write attribution, over a device worker.
//!
//! Owned by WP HB2. The order is the worker's queue (`device.rs`): strict arrival
//! order across sessions (spec 8.2). This file decides what enters the queue and
//! who a notification belongs to. Lock level L2, never held across a transfer.
//!
//! HB0 has what the app's path needs; HB2 adds the analyser's STOP arbitration
//! (spec 7.15, in [`Router::before_exec`]), the payload limit, and the tests of its
//! done criteria.

use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use dspi_link::frames::CmdRequest;
use dspi_link::status;
use dspi_proto::generated::opcodes as op;

use crate::lock::LockTable;
use crate::policy::Policy;
use crate::session::{NO_ORIGIN, SessionCtx, SessionId};

/// How long after a write a notification is attributed to it (spec 8.3).
pub const ATTRIBUTION_WINDOW: Duration = Duration::from_millis(500);
/// Writes remembered for attribution.
const MAX_WRITERS: usize = 64;

/// One write waiting for its notification.
#[derive(Debug, Clone)]
struct Writer {
    token: u64,
    session: SessionId,
    code: u8,
    param: u32,
    bulk: bool,
    at: Instant,
}

/// What [`Router::before_exec`] hands to [`Router::after_exec`].
#[derive(Debug)]
pub struct ExecTicket {
    token: Option<u64>,
    /// The entry this write merged into, to restore if it fails.
    previous: Option<Instant>,
}

impl ExecTicket {
    /// The ticket of a command that changes nothing, or is not going to run.
    pub fn none() -> Self {
        Self {
            token: None,
            previous: None,
        }
    }
}

#[derive(Debug, Default)]
struct State {
    inflight: BTreeMap<SessionId, u32>,
    lock: LockTable,
    writers: VecDeque<Writer>,
    next_token: u64,
    shut: bool,
}

/// One device's router.
#[derive(Debug)]
pub struct Router {
    policy: Arc<Policy>,
    max_inflight: u32,
    state: Mutex<State>,
}

/// Whether this opcode writes the whole image.
fn is_bulk_write(code: u8) -> bool {
    matches!(code, op::REQ_SET_ALL_PARAMS | op::REQ_SET_ALL_PARAMS_CHUNK)
}

/// Which parameter a write targets, so repeats merge where the firmware coalesces
/// them. `REQ_SET_EQ_PARAM` names its channel and band in the payload; all of a
/// bulk write's chunks are one write.
fn parameter_key(req: &CmdRequest) -> u32 {
    if is_bulk_write(req.b_request) {
        return 0;
    }
    match (req.b_request, req.payload.first(), req.payload.get(1)) {
        (op::REQ_SET_EQ_PARAM, Some(channel), Some(band)) => {
            u32::from(req.w_value) | u32::from(*channel) << 16 | u32::from(*band) << 24
        }
        _ => u32::from(req.w_value),
    }
}

impl Router {
    /// A router over this table.
    pub fn new(policy: Arc<Policy>, max_inflight: u32) -> Self {
        Self {
            policy,
            max_inflight,
            state: Mutex::new(State::default()),
        }
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        // Each update is complete before anything that could panic.
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Whether this command may enter the queue. `Err` is the status to answer at
    /// once, without touching the device: role, then lock, then `max_inflight`.
    pub fn admit(&self, s: &SessionCtx, req: &CmdRequest, now: Instant) -> Result<(), u8> {
        if !self.policy.allows(s.role, req.b_request, req.dir) {
            return Err(status::DENIED);
        }
        let mut st = self.lock();
        if st.shut {
            return Err(status::NO_DEVICE);
        }
        if !st.lock.permits(s.id, now) {
            return Err(status::LOCKED);
        }
        let count = st.inflight.entry(s.id).or_insert(0);
        if !s.exempt_inflight && *count >= self.max_inflight {
            return Err(status::RATE_LIMITED);
        }
        *count = count.saturating_add(1);
        Ok(())
    }

    /// Whether this command is a parameter read ([`Policy::retries_stall`]).
    pub fn retries_stall(&self, req: &CmdRequest) -> bool {
        self.policy.retries_stall(req.b_request, req.dir)
    }

    /// Whether this command can change the device ([`Policy::mutates`]).
    pub fn mutates(&self, req: &CmdRequest) -> bool {
        self.policy.mutates(req.b_request, req.dir)
    }

    /// On the worker, in queue order, right before the device call.
    ///
    /// `Err` answers the command with this status and payload without touching the
    /// device: WP HB2's analyser STOP (spec 7.15) goes here, in order, so a band read
    /// queued ahead of a STOP counts as watching. Nothing does so yet.
    ///
    /// `Ok` notes a write for attribution. The notification cannot arrive first, since
    /// the call has not been made.
    pub fn before_exec(
        &self,
        s: &SessionCtx,
        req: &CmdRequest,
        now: Instant,
    ) -> Result<ExecTicket, (u8, Vec<u8>)> {
        if !self.policy.mutates(req.b_request, req.dir) {
            return Ok(ExecTicket::none());
        }
        let param = parameter_key(req);
        let mut st = self.lock();
        if let Some(w) = st
            .writers
            .iter_mut()
            .rev()
            .find(|w| w.session == s.id && w.code == req.b_request && w.param == param)
        {
            let previous = Some(w.at);
            w.at = now;
            return Ok(ExecTicket {
                token: Some(w.token),
                previous,
            });
        }
        let token = st.next_token;
        st.next_token = token.wrapping_add(1);
        st.writers.push_back(Writer {
            token,
            session: s.id,
            code: req.b_request,
            param,
            bulk: is_bulk_write(req.b_request),
            at: now,
        });
        if st.writers.len() > MAX_WRITERS {
            st.writers.pop_front();
        }
        Ok(ExecTicket {
            token: Some(token),
            previous: None,
        })
    }

    /// On the worker, right after the device call. A failed write gives back its
    /// attribution; one a notification already consumed stays consumed.
    pub fn after_exec(&self, s: &SessionCtx, ticket: ExecTicket, ok: bool) {
        let mut st = self.lock();
        if let Some(count) = st.inflight.get_mut(&s.id) {
            *count = count.saturating_sub(1);
            if *count == 0 {
                st.inflight.remove(&s.id);
            }
        }
        if ok {
            return;
        }
        let Some(token) = ticket.token else {
            return;
        };
        let Some(i) = st.writers.iter().position(|w| w.token == token) else {
            return;
        };
        match (ticket.previous, st.writers.get_mut(i)) {
            (Some(at), Some(w)) => w.at = at,
            _ => {
                st.writers.remove(i);
            }
        }
    }

    /// The session that gets this notification as its `origin`: the oldest write
    /// within [`ATTRIBUTION_WINDOW`] (spec 8.3), [`NO_ORIGIN`] when there is none.
    ///
    /// A `BULK_INVALIDATED` (`bulk`) only takes a bulk write and a `PARAM_CHANGED`
    /// only a scalar one, which is narrower than the spec's single queue. It has to
    /// be: a client skips the re-read for its own bulk echo, so one client's stale
    /// scalar entry must never claim another client's bulk write.
    pub fn consume_attribution(&self, bulk: bool, now: Instant) -> SessionId {
        let mut st = self.lock();
        st.writers
            .retain(|w| now.saturating_duration_since(w.at) <= ATTRIBUTION_WINDOW);
        match st.writers.iter().position(|w| w.bulk == bulk) {
            Some(i) => st.writers.remove(i).map_or(NO_ORIGIN, |w| w.session),
            None => NO_ORIGIN,
        }
    }

    /// Take or renew the device lock. False if another session holds it.
    pub fn lock_acquire(&self, session: SessionId, timeout: Duration, now: Instant) -> bool {
        self.lock().lock.acquire(session, timeout, now)
    }

    /// Release the device lock if `session` holds it.
    pub fn lock_release(&self, session: SessionId) {
        self.lock().lock.release(session);
    }

    /// The live lock holder.
    pub fn lock_holder(&self, now: Instant) -> Option<SessionId> {
        self.lock().lock.holder(now)
    }

    /// A session ended: its lock and its count go at once (a vanished client's are
    /// released when its connection is declared dead, HUB.md section 6).
    pub fn session_closed(&self, session: SessionId) {
        let mut st = self.lock();
        st.lock.release(session);
        st.inflight.remove(&session);
    }

    /// The device went: everything submitted from now on answers `NO_DEVICE`.
    pub fn shutdown(&self) {
        self.lock().shut = true;
    }
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_hb0 {
    use super::*;
    use crate::session::Role;
    use dspi_link::Dir;

    fn router(max: u32) -> Router {
        Router::new(Arc::new(Policy::embedded()), max)
    }

    fn ctx(id: SessionId) -> SessionCtx {
        SessionCtx {
            id,
            role: Role::Admin,
            exempt_inflight: false,
        }
    }

    fn set(code: u8, value: u16) -> CmdRequest {
        CmdRequest {
            tag: 0,
            handle: 0,
            dir: Dir::Set,
            b_request: code,
            w_value: value,
            w_index: 2,
            w_length: 0,
            payload: Vec::new(),
        }
    }

    const VOL: u8 = op::REQ_SET_MASTER_VOLUME;

    fn wrote(r: &Router, session: SessionId, req: &CmdRequest, now: Instant) {
        r.admit(&ctx(session), req, now).unwrap();
        let t = r.before_exec(&ctx(session), req, now).unwrap();
        r.after_exec(&ctx(session), t, true);
    }

    #[test]
    fn wp_hb0_attribution_goes_to_the_oldest_writer() {
        let (r, t) = (router(8), Instant::now());
        wrote(&r, 2, &set(VOL, 0), t);
        wrote(&r, 3, &set(op::REQ_SET_EQ_PARAM, 1), t);
        assert_eq!(r.consume_attribution(false, t), 2);
        assert_eq!(r.consume_attribution(false, t), 3);
        assert_eq!(r.consume_attribution(false, t), NO_ORIGIN);
    }

    #[test]
    fn wp_hb0_a_repeat_merges_as_the_firmware_coalesces() {
        let (r, t) = (router(8), Instant::now());
        wrote(&r, 2, &set(VOL, 0), t);
        wrote(&r, 2, &set(VOL, 0), t);
        assert_eq!(r.consume_attribution(false, t), 2);
        assert_eq!(r.consume_attribution(false, t), NO_ORIGIN);
    }

    #[test]
    fn wp_hb0_a_stale_write_attributes_nothing() {
        let (r, t) = (router(8), Instant::now());
        wrote(&r, 2, &set(VOL, 0), t);
        let later = t + ATTRIBUTION_WINDOW + Duration::from_millis(1);
        assert_eq!(r.consume_attribution(false, later), NO_ORIGIN);
    }

    /// Without the split, session 2's scalar entry would claim session 3's bulk
    /// write, and session 2 would skip the re-read it needs.
    #[test]
    fn wp_hb0_a_scalar_write_never_claims_a_bulk_invalidation() {
        let (r, t) = (router(8), Instant::now());
        wrote(&r, 2, &set(VOL, 0), t);
        assert_eq!(r.consume_attribution(true, t), NO_ORIGIN);
        wrote(&r, 3, &set(op::REQ_SET_ALL_PARAMS, 0), t);
        assert_eq!(r.consume_attribution(true, t), 3);
        assert_eq!(r.consume_attribution(false, t), 2);
    }

    #[test]
    fn wp_hb0_all_chunks_of_a_bulk_write_are_one_write() {
        let (r, t) = (router(8), Instant::now());
        wrote(&r, 2, &set(op::REQ_SET_ALL_PARAMS_CHUNK, 0), t);
        wrote(&r, 2, &set(op::REQ_SET_ALL_PARAMS_CHUNK, 2048), t);
        assert_eq!(r.consume_attribution(true, t), 2);
        assert_eq!(r.consume_attribution(true, t), NO_ORIGIN);
    }

    #[test]
    fn wp_hb0_a_failed_write_gives_back_its_attribution() {
        let (r, t) = (router(8), Instant::now());
        let req = set(VOL, 0);
        r.admit(&ctx(2), &req, t).unwrap();
        let ticket = r.before_exec(&ctx(2), &req, t).unwrap();
        r.after_exec(&ctx(2), ticket, false);
        assert_eq!(r.consume_attribution(false, t), NO_ORIGIN);
    }

    #[test]
    fn wp_hb0_max_inflight_and_exemption() {
        let (r, t) = (router(1), Instant::now());
        let req = set(VOL, 0);
        assert_eq!(r.admit(&ctx(2), &req, t), Ok(()));
        assert_eq!(r.admit(&ctx(2), &req, t), Err(status::RATE_LIMITED));
        assert_eq!(r.admit(&ctx(3), &req, t), Ok(()), "per session");
        let app = SessionCtx {
            exempt_inflight: true,
            ..ctx(1)
        };
        assert_eq!(r.admit(&app, &req, t), Ok(()));
        assert_eq!(r.admit(&app, &req, t), Ok(()));
    }

    #[test]
    fn wp_hb0_a_lock_refuses_others_and_goes_with_its_session() {
        let (r, t) = (router(8), Instant::now());
        let req = set(VOL, 0);
        assert!(r.lock_acquire(2, Duration::from_secs(5), t));
        assert_eq!(r.admit(&ctx(3), &req, t), Err(status::LOCKED));
        assert_eq!(r.admit(&ctx(2), &req, t), Ok(()));
        r.session_closed(2);
        assert_eq!(r.admit(&ctx(3), &req, t), Ok(()));
    }

    #[test]
    fn wp_hb0_a_shut_router_answers_no_device() {
        let (r, t) = (router(8), Instant::now());
        r.shutdown();
        assert_eq!(r.admit(&ctx(2), &set(VOL, 0), t), Err(status::NO_DEVICE));
    }
}
