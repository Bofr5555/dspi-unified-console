//! Per-device admission and write attribution, over a device worker.
//!
//! Owned by WP HB2. The order is the worker's queue (`device.rs`): strict arrival
//! order across sessions (spec 8.2). This file decides what enters the queue and
//! who a notification belongs to. Lock level L2, never held across a transfer.
//!
//! Three things are decided on the worker, in queue order, in [`Router::before_exec`]:
//! the voltage rule, the analyser's STOP (spec 7.15), and a closed session's leftovers.
//!
//! **A write is never replayed.** Nothing here or in the worker runs a command twice:
//! `run_command` makes one call and answers what it got, whatever it got, and the only
//! repeats are parameter reads inside a busy window (HUB.md section 4), which
//! [`Policy::retries_stall`] keeps to commands that change nothing.
//!
//! **A hostile client.** Everyone shares one queue, in arrival order. A session has at
//! most `max_inflight` commands in it, so another's command waits for at most that many
//! per session, and past that it is `RATE_LIMITED` without touching the device. Closing
//! a session drops what it left queued, so reconnecting gets no more of the queue. The
//! lock has its own limit (`lock.rs`). HUB.md section 6.

use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use dspi_link::frames::CmdRequest;
use dspi_link::{Dir, status};
use dspi_proto::ext::CoreVregStatus;
use dspi_proto::generated::ext;
use dspi_proto::generated::opcodes as op;
use dspi_proto::generated::rta;
use dspi_session::overvolt;

use crate::lock::LockTable;
use crate::policy::{self, Policy};
use crate::relay::PacketKind;
use crate::session::{LOCAL_SESSION_ID, NO_ORIGIN, SessionCtx, SessionId};
// -- WP P1HA --
use crate::session::INTERNAL_SESSION_ID;

/// How long after a write a notification is attributed to it (spec 8.3).
pub const ATTRIBUTION_WINDOW: Duration = Duration::from_millis(500);
/// Writes remembered for attribution.
const MAX_WRITERS: usize = 64;
/// `limits.max_payload` (spec 7.2): the largest payload either way a network session
/// may ask for. The hub's `hello` must say the same.
pub const MAX_PAYLOAD: usize = 8192;
/// How long a read of the analyser counts as watching: the firmware's own idle timeout,
/// so a swallowed STOP never keeps it running longer than an unread one (spec 7.15).
pub const RTA_WATCH_WINDOW: Duration = Duration::from_millis(5000);
/// The reads that make a session a watcher, by command or by poll.
const RTA_WATCH: [u8; 3] = [
    op::REQ_RTA_GET_BANDS,
    op::REQ_RTA_GET_BINS,
    op::REQ_RTA_GET_BANDS_ALL,
];
/// What the device answers an `REQ_RTA_CONTROL` with.
const RTA_ACK: u8 = 1;

/// One write waiting for its notification.
#[derive(Debug, Clone)]
struct Writer {
    token: u64,
    session: SessionId,
    code: u8,
    param: u32,
    kind: PacketKind,
    at: Instant,
}

/// What [`Router::before_exec`] needs of the device. The worker's, so nothing here takes
/// a lock of the router's across them: a transfer is what the second one makes.
pub struct ExecUnit<'a> {
    /// Whether a session other than the second argument polls one of these opcodes
    /// (`Polls::others_poll`).
    pub others_poll: &'a dyn Fn(&[u8], SessionId) -> bool,
    /// Read the unit's voltage status now. `None` if it cannot be read.
    pub read_vreg: &'a mut dyn FnMut() -> Option<CoreVregStatus>,
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
    /// Sessions that closed with commands queued, and how many. An id reused while its
    /// old commands are queued is refused until they have gone; the ids wrap at 65 536.
    closing: BTreeMap<SessionId, u32>,
    /// When each session last read the analyser or started it (spec 7.15).
    rta_watchers: BTreeMap<SessionId, Instant>,
    lock: LockTable,
    writers: VecDeque<Writer>,
    next_token: u64,
    shut: bool,
    // -- WP P1HA --
    /// The hub's next own bulk write is a guarded apply's revert: it passes the lock
    /// ([`Router::pass_revert`]).
    revert_pass: bool,
    // -- WP P1CFIX -- F6
    /// The hub is writing a guarded apply's settings outside the image back: its
    /// own commands pass the lock until it closes ([`Router::revert_window`]).
    revert_window: bool,
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

// -- WP UPW1 --
/// Writes the firmware never answers with a notification `relay::is_hub_path` takes
/// (DSPi release/v1.1.6 and bofr-main, `vendor_commands.c`). Each would sit in the
/// queue as a scalar writer and take the next console's change for its own echo
/// within [`ATTRIBUTION_WINDOW`], which a client that drops its own echoes by
/// `origin` never shows (upstream's Windows hub, f1e18ed1, for `RTA_CONTROL`). A
/// newer firmware that tags one of these after all only gets origin 0 for it, which
/// costs a re-read (HUB.md section 4).
const SILENT: [u8; 45] = [
    // No event at all.
    op::REQ_OUTPUT_LATENCY,
    op::REQ_RTA_CONTROL,
    op::REQ_SET_CS_GROUP,
    op::REQ_SET_CS_MACRO,
    op::REQ_SET_CS_MACRO_STEP,
    op::REQ_SET_CS_DISPLAY_CFG,
    op::REQ_SET_CS_DISPLAY_PAGE,
    op::REQ_SET_SUBHARM_SOLO,
    op::REQ_SAVE_PARAMS,
    op::REQ_SAVE_OUTPUT_CONFIG,
    op::REQ_CLEAR_CLIPS,
    op::REQ_SET_CS_BINDING,
    op::REQ_SET_CS_NAME,
    op::REQ_SET_CS_IR_CMD,
    op::REQ_PRESET_SAVE,
    op::REQ_PRESET_SET_NAME,
    op::REQ_PRESET_SET_STARTUP,
    op::REQ_SET_OUTPUT_CONFIG_MODE,
    op::REQ_CS_SAVE,
    op::REQ_CS_REVERT,
    op::REQ_RESET_BUFFER_STATS,
    op::REQ_RESET_USB_ERROR_STATS,
    op::REQ_SET_MASTER_VOLUME_MODE,
    op::REQ_SAVE_MASTER_VOLUME,
    op::REQ_TEST_DAC_HW_MUTE,
    op::REQ_ENTER_BOOTLOADER,
    op::REQ_SET_UART_CONFIG,
    op::REQ_SET_I2C_CONFIG,
    // Only events that carry no source a writer can answer: CS_AUX, the two name
    // events, CS_IR_LEARN, SIGGEN_STATE. A macro's steps run later, tagged GPIO.
    op::REQ_SET_CS_AUX_STATE,
    op::REQ_SET_CS_AUX_LEVEL,
    op::REQ_SET_INPUT_SOURCE_NAME,
    op::REQ_SET_DEVICE_NAME,
    op::REQ_CS_IR_LEARN,
    op::REQ_SIGGEN_SET_CONFIG,
    op::REQ_SIGGEN_CONTROL,
    op::REQ_CS_MACRO_FIRE,
    // A notification, but never on the hub's path: BULK_INVALIDATED tagged PRESET or
    // FACTORY, and changes the main loop applies after the request's source is
    // cleared, so they are tagged UNKNOWN.
    op::REQ_PRESET_LOAD,
    op::REQ_FACTORY_RESET,
    op::REQ_PRESET_DELETE,
    op::REQ_SET_EQ_PARAM,
    op::REQ_SET_BAND_BYPASS,
    op::REQ_SET_I2S_CLOCK_MODE,
    op::REQ_SET_ADAT_INPUT_CLOCK_MODE,
    op::REQ_SET_INPUT_SOURCE,
    op::REQ_SET_DAC_HW_MUTE_CONFIG,
];

/// Whether the firmware answers this write with nothing to attribute ([`SILENT`]).
/// The voltage extension notifies nothing either (bofr-main `ext_core_vreg`).
fn is_silent(req: &CmdRequest) -> bool {
    SILENT.contains(&req.b_request)
        || (req.b_request == op::REQ_EXTENSION && req.w_value == ext::EXT_CORE_VREG)
        // -- WP SW2 -- the audio link's block sends no `PARAM_CHANGED` for any write
        // (`extension_channel_spec.md` 2.8): a waiting writer would take the next
        // console's change.
        || (req.b_request == op::REQ_EXTENSION && is_alink_id(req.w_value))
}

// -- WP SW2 --
/// The fork's audio link block, 0x8000-0x80FF (2.8).
fn is_alink_id(id: u16) -> bool {
    id & 0xFF00 == 0x8000
}

/// Which notification answers a write of this opcode.
fn write_kind(code: u8) -> PacketKind {
    if is_bulk_write(code) {
        PacketKind::Bulk
    } else if code == op::REQ_RTA_SET_CONFIG {
        PacketKind::RtaConfig
    } else {
        PacketKind::Scalar
    }
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
    /// once, without touching the device: size, role, lock, then `max_inflight`. The
    /// app has no wire, so no size limit.
    pub fn admit(&self, s: &SessionCtx, req: &CmdRequest, now: Instant) -> Result<(), u8> {
        let size = req.payload.len().max(usize::from(req.w_length));
        if !s.exempt_inflight && size > MAX_PAYLOAD {
            return Err(status::TOO_LARGE);
        }
        if !self.policy.allows(s.role, req.b_request, req.dir) {
            return Err(status::DENIED);
        }
        let mut st = self.lock();
        // -- WP P1HA -- taken by the hub's next bulk write, whatever its fate
        let revert = s.id == INTERNAL_SESSION_ID
            && (st.revert_window // -- WP P1CFIX -- F6
                || (is_bulk_write(req.b_request) && std::mem::take(&mut st.revert_pass)));
        if st.shut {
            return Err(status::NO_DEVICE);
        }
        if !revert && !st.lock.permits(s.id, now) {
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

    /// Whether this request is a STOP that must not reach the device, because another
    /// session is watching (spec 7.15). Notes what makes this session a watcher. Reads
    /// and START count even if the device then refuses them.
    fn stop_is_swallowed(
        &self,
        s: &SessionCtx,
        req: &CmdRequest,
        now: Instant,
        others_poll: &dyn Fn(&[u8], SessionId) -> bool,
    ) -> bool {
        let code = req.b_request;
        let control = code == op::REQ_RTA_CONTROL;
        if req.dir != Dir::Get || !(control || RTA_WATCH.contains(&code)) {
            return false;
        }
        let mut st = self.lock();
        if !control || req.w_value == rta::RTA_CTL_START {
            st.rta_watchers.insert(s.id, now);
            return false;
        }
        if req.w_value != rta::RTA_CTL_STOP {
            return false;
        }
        st.rta_watchers.remove(&s.id);
        st.rta_watchers
            .retain(|_, at| now.saturating_duration_since(*at) <= RTA_WATCH_WINDOW);
        let others_watch = !st.rta_watchers.is_empty();
        drop(st);
        others_watch || others_poll(&RTA_WATCH, s.id)
    }

    /// On the worker, in queue order, right before the device call. `unit` may touch
    /// the device; no lock of the router's is held while it does.
    ///
    /// `Err` answers the command with this status and payload without running it:
    /// - **A session that has closed**: nobody is waiting, and it no longer holds a
    ///   place in the queue.
    /// - **An analyser STOP others still depend on**: `OK` and the device's `01` (spec
    ///   7.15). Decided here, so a band read queued ahead of a STOP counts as watching.
    /// - **A voltage raise the owner's rule would ask a word for** (`overvolt`, WP CG14),
    ///   from a network session of any role: `DENIED`. The rule is judged in a client's
    ///   core, so a raw `EXT_CORE_VREG` from a remote client would skip it, and a hub cannot
    ///   type a word for anyone. The app's session is not judged here: its core already
    ///   has. The unit's voltage is read here, at that moment, in the same
    ///   turn of the queue as the write, so nothing lands between. `DENIED` rather than
    ///   `BLOCKED`: a client reads `BLOCKED` as a stall, that is, "this build lacks it"
    ///   (`map_status`). HUB.md section 13.
    ///
    /// `Ok` notes a write for attribution. The notification cannot arrive first, since
    /// the call has not been made.
    pub fn before_exec(
        &self,
        s: &SessionCtx,
        req: &CmdRequest,
        now: Instant,
        unit: ExecUnit<'_>,
    ) -> Result<ExecTicket, (u8, Vec<u8>)> {
        if self.lock().closing.contains_key(&s.id) {
            return Err((status::NO_DEVICE, Vec::new()));
        }
        if self.stop_is_swallowed(s, req, now, unit.others_poll) {
            return Err((status::OK, vec![RTA_ACK]));
        }
        // The app's session is exempt, and this is safe: it is this process's own core, and
        // that core's `core_vreg_op` is the only path that encodes these requests. It
        // judges the word itself, on a status read through this same queue, and `raw_set`
        // refuses them. Someone at the machine who typed the word is not refused because
        // sharing happens to be on, and with no word the request never gets this far. No
        // network session can have this id (`next_session_id`).
        let judged = if s.id == LOCAL_SESSION_ID {
            None
        } else {
            policy::core_vreg_write(req)
        };
        if let Some(request) = judged {
            let refused = match request {
                Some(request) => overvolt::op_needs_word(request, (unit.read_vreg)().as_ref()),
                None => true,
            };
            if refused {
                log::warn!(
                    "hub: session {} asked for a core voltage raise, {:?}: refused",
                    s.id,
                    req.payload
                );
                return Err((status::DENIED, Vec::new()));
            }
        }
        if !self.policy.mutates(req.b_request, req.dir) || is_silent(req) {
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
            kind: write_kind(req.b_request),
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
        } else if let Some(count) = st.closing.get_mut(&s.id) {
            *count = count.saturating_sub(1);
            if *count == 0 {
                st.closing.remove(&s.id);
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
    /// A `BULK_INVALIDATED` only takes a bulk write, a `PARAM_CHANGED` only a scalar
    /// one and an `RTA_CONFIG` only a `REQ_RTA_SET_CONFIG`, which is narrower than the
    /// spec's single queue. It has to be: a client skips the re-read for its own bulk
    /// echo, so one client's stale scalar entry must never claim another client's bulk
    /// write, and an analyser follower must not take a change for its own.
    pub fn consume_attribution(&self, kind: PacketKind, now: Instant) -> SessionId {
        let mut st = self.lock();
        st.writers
            .retain(|w| now.saturating_duration_since(w.at) <= ATTRIBUTION_WINDOW);
        match st.writers.iter().position(|w| w.kind == kind) {
            Some(i) => st.writers.remove(i).map_or(NO_ORIGIN, |w| w.session),
            None => NO_ORIGIN,
        }
    }

    /// Take or renew the device lock. False if it is refused ([`LockTable::acquire`]).
    pub fn lock_acquire(&self, session: SessionId, timeout: Duration, now: Instant) -> bool {
        self.lock_grant(session, timeout, now).is_some()
    }

    /// Take or renew the device lock, and how long for ([`LockTable::acquire`]).
    pub fn lock_grant(
        &self,
        session: SessionId,
        timeout: Duration,
        now: Instant,
    ) -> Option<Duration> {
        self.lock().lock.acquire(session, timeout, now)
    }

    // -- WP P1HA --
    /// Let the hub's next own bulk write (`0xA1` as [`INTERNAL_SESSION_ID`]) past any
    /// session's lock, once: a guarded apply's revert is never refused by a lock
    /// (`apply_guard.rs`). Call it right before submitting that write; nothing else
    /// of the hub's writes a bulk image.
    pub fn pass_revert(&self) {
        self.lock().revert_pass = true;
    }

    // -- WP P1CFIX -- F6
    /// Open (or close) the window in which every command of the hub's own
    /// ([`INTERNAL_SESSION_ID`]) passes any session's lock: a guarded apply's revert
    /// writing back the settings outside the image (`apply_extras.rs`), never refused
    /// by a lock like its bulk write. Closed again before the bulk write.
    pub fn revert_window(&self, open: bool) {
        self.lock().revert_window = open;
    }

    /// Release the device lock if `session` holds it.
    pub fn lock_release(&self, session: SessionId) {
        self.lock().lock.release(session, Instant::now());
    }

    /// The live lock holder.
    pub fn lock_holder(&self, now: Instant) -> Option<SessionId> {
        self.lock().lock.holder(now)
    }

    /// A session ended: its lock goes at once (a vanished client's are released when
    /// its connection is declared dead or by their timeout, HUB.md section 6), it
    /// stops watching the analyser with nothing sent to the device (spec 7.15), and what
    /// it left queued will not run.
    pub fn session_closed(&self, session: SessionId) {
        let mut st = self.lock();
        st.lock.release(session, Instant::now());
        st.rta_watchers.remove(&session);
        if let Some(queued) = st.inflight.remove(&session) {
            let left = st.closing.entry(session).or_insert(0);
            *left = left.saturating_add(queued);
        }
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

    /// A device nobody polls and whose voltage cannot be read.
    fn exec(
        r: &Router,
        s: SessionId,
        req: &CmdRequest,
        now: Instant,
    ) -> Result<ExecTicket, (u8, Vec<u8>)> {
        r.before_exec(
            &ctx(s),
            req,
            now,
            ExecUnit {
                others_poll: &|_, _| false,
                read_vreg: &mut || None,
            },
        )
    }

    fn wrote(r: &Router, session: SessionId, req: &CmdRequest, now: Instant) {
        r.admit(&ctx(session), req, now).unwrap();
        let t = exec(r, session, req, now).unwrap();
        r.after_exec(&ctx(session), t, true);
    }

    #[test]
    fn wp_hb0_attribution_goes_to_the_oldest_writer() {
        let (r, t) = (router(8), Instant::now());
        wrote(&r, 2, &set(VOL, 0), t);
        wrote(&r, 3, &set(op::REQ_SET_PREAMP, 1), t);
        assert_eq!(r.consume_attribution(PacketKind::Scalar, t), 2);
        assert_eq!(r.consume_attribution(PacketKind::Scalar, t), 3);
        assert_eq!(r.consume_attribution(PacketKind::Scalar, t), NO_ORIGIN);
    }

    // -- WP UPW1 --
    /// A write the firmware answers with nothing the hub can attribute leaves no
    /// writer behind to claim the next console's change.
    #[test]
    fn wp_upw1_a_silent_write_leaves_nothing_to_attribute() {
        let (r, t) = (router(8), Instant::now());
        let start = CmdRequest {
            dir: Dir::Get,
            ..set(op::REQ_RTA_CONTROL, rta::RTA_CTL_START)
        };
        for silent in [
            start,
            set(op::REQ_SET_EQ_PARAM, 0),
            set(op::REQ_SET_DEVICE_NAME, 0),
        ] {
            wrote(&r, 2, &silent, t);
        }
        wrote(&r, 3, &set(VOL, 0), t);
        assert_eq!(r.consume_attribution(PacketKind::Scalar, t), 3);
        assert_eq!(r.consume_attribution(PacketKind::Scalar, t), NO_ORIGIN);
        // A voltage write notifies nothing; the crossfeed pair map does (bofr-main,
        // HOST_SET).
        assert!(is_silent(&set(op::REQ_EXTENSION, ext::EXT_CORE_VREG)));
        wrote(
            &r,
            2,
            &set(op::REQ_EXTENSION, ext::EXT_CROSSFEED_PAIR_MAP),
            t,
        );
        assert_eq!(r.consume_attribution(PacketKind::Scalar, t), 2);
    }

    // -- WP SW2 --
    /// SW1's note: an alink write (slots, phase, apply at frame) leaves no writer
    /// waiting, so the next console's scalar change is attributed to it alone.
    #[test]
    fn wp_sw2_audio_link_writes_are_silent() {
        use dspi_proto::ext::alink;
        let (r, t) = (router(8), Instant::now());
        for id in alink::ALINK_IDS {
            assert!(is_silent(&set(op::REQ_EXTENSION, id)), "{id:#06x}");
        }
        for id in [
            alink::EXT_ALINK_SLOTS,
            alink::EXT_ALINK_PHASE,
            alink::EXT_APPLY_AT_FRAME,
        ] {
            wrote(&r, 2, &set(op::REQ_EXTENSION, id), t);
        }
        wrote(&r, 3, &set(VOL, 0), t);
        assert_eq!(r.consume_attribution(PacketKind::Scalar, t), 3);
        assert_eq!(r.consume_attribution(PacketKind::Scalar, t), NO_ORIGIN);
        assert!(!is_silent(&set(op::REQ_EXTENSION, 0x7FFF)));
        assert!(!is_silent(&set(op::REQ_EXTENSION, 0x8100)));
    }

    #[test]
    fn wp_hb0_a_repeat_merges_as_the_firmware_coalesces() {
        let (r, t) = (router(8), Instant::now());
        wrote(&r, 2, &set(VOL, 0), t);
        wrote(&r, 2, &set(VOL, 0), t);
        assert_eq!(r.consume_attribution(PacketKind::Scalar, t), 2);
        assert_eq!(r.consume_attribution(PacketKind::Scalar, t), NO_ORIGIN);
    }

    #[test]
    fn wp_hb0_a_stale_write_attributes_nothing() {
        let (r, t) = (router(8), Instant::now());
        wrote(&r, 2, &set(VOL, 0), t);
        let later = t + ATTRIBUTION_WINDOW + Duration::from_millis(1);
        assert_eq!(r.consume_attribution(PacketKind::Scalar, later), NO_ORIGIN);
    }

    /// Without the split, session 2's scalar entry would claim session 3's bulk
    /// write, and session 2 would skip the re-read it needs.
    #[test]
    fn wp_hb0_a_scalar_write_never_claims_a_bulk_invalidation() {
        let (r, t) = (router(8), Instant::now());
        wrote(&r, 2, &set(VOL, 0), t);
        assert_eq!(r.consume_attribution(PacketKind::Bulk, t), NO_ORIGIN);
        wrote(&r, 3, &set(op::REQ_SET_ALL_PARAMS, 0), t);
        assert_eq!(r.consume_attribution(PacketKind::Bulk, t), 3);
        assert_eq!(r.consume_attribution(PacketKind::Scalar, t), 2);
    }

    #[test]
    fn wp_hb0_all_chunks_of_a_bulk_write_are_one_write() {
        let (r, t) = (router(8), Instant::now());
        wrote(&r, 2, &set(op::REQ_SET_ALL_PARAMS_CHUNK, 0), t);
        wrote(&r, 2, &set(op::REQ_SET_ALL_PARAMS_CHUNK, 2048), t);
        assert_eq!(r.consume_attribution(PacketKind::Bulk, t), 2);
        assert_eq!(r.consume_attribution(PacketKind::Bulk, t), NO_ORIGIN);
    }

    #[test]
    fn wp_hb0_a_failed_write_gives_back_its_attribution() {
        let (r, t) = (router(8), Instant::now());
        let req = set(VOL, 0);
        r.admit(&ctx(2), &req, t).unwrap();
        let ticket = exec(&r, 2, &req, t).unwrap();
        r.after_exec(&ctx(2), ticket, false);
        assert_eq!(r.consume_attribution(PacketKind::Scalar, t), NO_ORIGIN);
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

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_hb2 {
    use std::cell::Cell;

    use dspi_proto::ext::VregOp;
    use dspi_proto::generated::ext as g;

    use super::*;
    use crate::session::Role;

    fn router() -> Router {
        Router::new(Arc::new(Policy::embedded()), 8)
    }

    fn ctx(id: SessionId, role: Role) -> SessionCtx {
        SessionCtx {
            id,
            role,
            exempt_inflight: false,
        }
    }

    fn cmd(dir: Dir, code: u8, value: u16, payload: Vec<u8>) -> CmdRequest {
        CmdRequest {
            tag: 0,
            handle: 0,
            dir,
            b_request: code,
            w_value: value,
            w_index: 2,
            w_length: u16::try_from(payload.len()).unwrap_or(u16::MAX),
            payload,
        }
    }

    fn rta_control(what: u16) -> CmdRequest {
        cmd(Dir::Get, op::REQ_RTA_CONTROL, what, Vec::new())
    }

    fn bands() -> CmdRequest {
        cmd(Dir::Get, op::REQ_RTA_GET_BANDS, 0, Vec::new())
    }

    /// `Ok`: the command runs. `Err`: answered without the device.
    fn run_with_polls(
        r: &Router,
        s: SessionId,
        req: &CmdRequest,
        now: Instant,
        polled: bool,
    ) -> Result<(), (u8, Vec<u8>)> {
        let ctx = ctx(s, Role::Control);
        r.admit(&ctx, req, now).map_err(|e| (e, Vec::new()))?;
        let unit = ExecUnit {
            others_poll: &|codes, me| {
                assert_eq!(codes, RTA_WATCH);
                assert_eq!(me, s);
                polled
            },
            read_vreg: &mut || None,
        };
        let result = r.before_exec(&ctx, req, now, unit);
        match result {
            Ok(ticket) => {
                r.after_exec(&ctx, ticket, true);
                Ok(())
            }
            Err(answer) => {
                r.after_exec(&ctx, ExecTicket::none(), true);
                Err(answer)
            }
        }
    }

    fn run(r: &Router, s: SessionId, req: &CmdRequest, now: Instant) -> Result<(), (u8, Vec<u8>)> {
        run_with_polls(r, s, req, now, false)
    }

    fn is_swallowed(result: Result<(), (u8, Vec<u8>)>) -> bool {
        result == Err((status::OK, vec![RTA_ACK]))
    }

    #[test]
    fn wp_hb2_a_stop_with_nobody_else_watching_reaches_the_device() {
        let (r, t) = (router(), Instant::now());
        assert_eq!(run(&r, 2, &bands(), t), Ok(()));
        assert_eq!(run(&r, 2, &rta_control(rta::RTA_CTL_STOP), t), Ok(()));
    }

    #[test]
    fn wp_hb2_a_stop_is_per_session() {
        let (r, t) = (router(), Instant::now());
        let stop = rta_control(rta::RTA_CTL_STOP);
        assert_eq!(run(&r, 3, &bands(), t), Ok(()));
        assert_eq!(run(&r, 2, &rta_control(rta::RTA_CTL_START), t), Ok(()));
        assert!(is_swallowed(run(&r, 2, &stop, t)), "3 still reads bands");
        // The other stopped too, and with that nobody watches: the device hears it.
        assert_eq!(run(&r, 3, &stop, t), Ok(()));
    }

    #[test]
    fn wp_hb2_watching_ends_with_the_window_and_with_the_session() {
        let (r, t) = (router(), Instant::now());
        let stop = rta_control(rta::RTA_CTL_STOP);
        assert_eq!(run(&r, 3, &bands(), t), Ok(()));
        let inside = t + RTA_WATCH_WINDOW;
        assert!(is_swallowed(run(&r, 2, &stop, inside)));
        let past = inside + Duration::from_millis(1);
        assert_eq!(run(&r, 2, &stop, past), Ok(()), "3 stopped reading");

        assert_eq!(run(&r, 3, &bands(), past), Ok(()));
        r.session_closed(3);
        assert_eq!(run(&r, 2, &stop, past), Ok(()), "3 left");
    }

    #[test]
    fn wp_hb2_start_and_reset_always_go_through_and_only_start_watches() {
        let (r, t) = (router(), Instant::now());
        let stop = rta_control(rta::RTA_CTL_STOP);
        assert_eq!(run(&r, 3, &rta_control(rta::RTA_CTL_RESET_AVG), t), Ok(()));
        assert_eq!(run(&r, 2, &stop, t), Ok(()), "a reset is not watching");
        assert_eq!(run(&r, 3, &rta_control(rta::RTA_CTL_START), t), Ok(()));
        assert!(is_swallowed(run(&r, 2, &stop, t)));
        assert_eq!(run(&r, 2, &rta_control(rta::RTA_CTL_START), t), Ok(()));
        assert_eq!(run(&r, 2, &rta_control(rta::RTA_CTL_RESET_AVG), t), Ok(()));
    }

    #[test]
    fn wp_hb2_another_sessions_poll_counts_as_watching() {
        let (r, t) = (router(), Instant::now());
        let stop = rta_control(rta::RTA_CTL_STOP);
        assert!(is_swallowed(run_with_polls(&r, 2, &stop, t, true)));
        assert_eq!(run_with_polls(&r, 2, &stop, t, false), Ok(()));
    }

    /// A swallowed STOP changed nothing, so it must not wait for a notification.
    #[test]
    fn wp_hb2_a_swallowed_stop_is_not_a_write() {
        let (r, t) = (router(), Instant::now());
        assert_eq!(run(&r, 3, &bands(), t), Ok(()));
        assert!(is_swallowed(run(&r, 2, &rta_control(rta::RTA_CTL_STOP), t)));
        assert_eq!(r.consume_attribution(PacketKind::Scalar, t), NO_ORIGIN);
    }

    #[test]
    fn wp_hb2_an_analyser_configuration_goes_to_the_session_that_chose_it() {
        let (r, t) = (router(), Instant::now());
        let volume = cmd(Dir::Set, op::REQ_SET_MASTER_VOLUME, 0, vec![0; 4]);
        let config = cmd(Dir::Set, op::REQ_RTA_SET_CONFIG, 0, vec![0; 12]);
        assert_eq!(run(&r, 2, &volume, t), Ok(()));
        assert_eq!(run(&r, 3, &config, t), Ok(()));
        assert_eq!(r.consume_attribution(PacketKind::RtaConfig, t), 3);
        assert_eq!(r.consume_attribution(PacketKind::RtaConfig, t), NO_ORIGIN);
        assert_eq!(r.consume_attribution(PacketKind::Scalar, t), 2);
    }

    #[test]
    fn wp_hb2_too_large_goes_by_size_not_the_app() {
        let (r, t) = (router(), Instant::now());
        let admin = ctx(2, Role::Admin);
        let at = cmd(Dir::Set, op::REQ_SET_ALL_PARAMS, 0, vec![0; MAX_PAYLOAD]);
        let over = cmd(
            Dir::Set,
            op::REQ_SET_ALL_PARAMS,
            0,
            vec![0; MAX_PAYLOAD + 1],
        );
        let mut read = cmd(Dir::Get, op::REQ_GET_ALL_PARAMS, 0, Vec::new());
        read.w_length = MAX_PAYLOAD as u16 + 1;
        assert_eq!(r.admit(&admin, &at, t), Ok(()));
        assert_eq!(r.admit(&admin, &over, t), Err(status::TOO_LARGE));
        assert_eq!(r.admit(&admin, &read, t), Err(status::TOO_LARGE));
        let app = SessionCtx {
            exempt_inflight: true,
            ..ctx(1, Role::Admin)
        };
        assert_eq!(r.admit(&app, &over, t), Ok(()));
        assert_eq!(r.admit(&app, &read, t), Ok(()));
    }

    /// The fair share the queue promises holds: a session that left gets no more of it.
    #[test]
    fn wp_hb2_what_a_closed_session_left_queued_does_not_run() {
        let (r, t) = (router(), Instant::now());
        let volume = cmd(Dir::Set, op::REQ_SET_MASTER_VOLUME, 0, vec![0; 4]);
        let s = ctx(2, Role::Admin);
        for _ in 0..3 {
            r.admit(&s, &volume, t).unwrap();
        }
        r.session_closed(2);
        let exec = || {
            r.before_exec(
                &s,
                &volume,
                t,
                ExecUnit {
                    others_poll: &|_, _| false,
                    read_vreg: &mut || None,
                },
            )
            .map(|_| ())
        };
        for _ in 0..3 {
            assert_eq!(exec(), Err((status::NO_DEVICE, Vec::new())));
            r.after_exec(&s, ExecTicket::none(), true);
        }
        assert_eq!(r.consume_attribution(PacketKind::Scalar, t), NO_ORIGIN);
        // Drained: the table forgets it, and an id reused later is an ordinary session.
        assert!(r.lock().closing.is_empty());
        assert_eq!(exec(), Ok(()));
    }

    fn status_at(live: u8, stored: u8, default_vreg: u8) -> CoreVregStatus {
        CoreVregStatus {
            live_vreg: live,
            stored_sel: stored,
            default_vreg,
            flags: 0,
            hang_count: 0,
            trial_active: false,
            trial_vreg: 0,
            failed_trial_vreg: 0,
            failed_trial_uptime_s: 0,
            uptime_s: 0,
        }
    }

    /// `(answer, how many status reads it took)`.
    fn judge(
        role: Role,
        payload: &[u8],
        unit: Option<CoreVregStatus>,
    ) -> (Result<(), (u8, Vec<u8>)>, u32) {
        let (r, t) = (router(), Instant::now());
        let req = cmd(
            Dir::Set,
            op::REQ_EXTENSION,
            g::EXT_CORE_VREG,
            payload.to_vec(),
        );
        let reads = Cell::new(0);
        let answer = r
            .before_exec(
                &ctx(2, role),
                &req,
                t,
                ExecUnit {
                    others_poll: &|_, _| false,
                    read_vreg: &mut || {
                        reads.set(reads.get() + 1);
                        unit.clone()
                    },
                },
            )
            .map(|_| ());
        (answer, reads.get())
    }

    const DENIED: Result<(), (u8, Vec<u8>)> = Err((status::DENIED, Vec::new()));

    /// CG14's cases, at the hub: the owner's "already 1.25 and changes to 1.3" and the
    /// rest of the rule. The role does not come into it: `before_exec` is past `admit`.
    #[test]
    fn wp_hb2_the_voltage_rule_at_the_hub() {
        const V115: u8 = 0x0C;
        const V120: u8 = 0x0D;
        const V125: u8 = 0x0E;
        const V130: u8 = 0x0F;
        for (name, unit, request, refused) in [
            (
                "owner's example",
                status_at(V125, 0, V125),
                VregOp::Set(V130),
                true,
            ),
            (
                "owner's example, trial",
                status_at(V125, 0, V125),
                VregOp::Trial(V130),
                true,
            ),
            (
                "past 1.20",
                status_at(V120, 0, V120),
                VregOp::Set(V125),
                true,
            ),
            (
                "past 1.20, trial",
                status_at(V120, 0, V120),
                VregOp::Trial(V125),
                true,
            ),
            (
                "stock past 1.20",
                status_at(V115, 0, V115),
                VregOp::Set(V125),
                true,
            ),
            (
                "not a voltage",
                status_at(V120, 0, V120),
                VregOp::Trial(0x40),
                true,
            ),
            (
                "up to 1.20",
                status_at(V115, 0, V115),
                VregOp::Set(V120),
                false,
            ),
            (
                "level",
                status_at(V125, V125, V120),
                VregOp::Set(V125),
                false,
            ),
            (
                "down",
                status_at(V130, V130, V120),
                VregOp::Set(V125),
                false,
            ),
            (
                "build default",
                status_at(V125, V125, V120),
                VregOp::Set(0),
                false,
            ),
            (
                "trial end",
                status_at(V125, 0, V120),
                VregOp::TrialEnd,
                false,
            ),
            ("auto", status_at(V120, 0, V120), VregOp::Auto(true), false),
            ("clear", status_at(V120, 0, V120), VregOp::Clear, false),
        ] {
            for role in [Role::Viewer, Role::Control, Role::Admin] {
                let (answer, _) = judge(role, &request.encode(), Some(unit.clone()));
                assert_eq!(answer == DENIED, refused, "{name}, {role:?}");
                assert_eq!(answer.is_ok(), !refused, "{name}, {role:?}");
            }
        }
    }

    #[test]
    fn wp_hb2_an_unreadable_voltage_counts_as_needing_the_word() {
        for (request, refused) in [
            (VregOp::Set(0x0E), true),
            (VregOp::Trial(0x0F), true),
            (VregOp::Set(0), true),
            (VregOp::Set(0x0D), false),
            (VregOp::Trial(0x0C), false),
            (VregOp::TrialEnd, false),
        ] {
            let (answer, reads) = judge(Role::Admin, &request.encode(), None);
            assert_eq!(answer == DENIED, refused, "{request:?}");
            assert!(reads <= 1, "{request:?}");
        }
    }

    #[test]
    fn wp_hb2_a_voltage_write_the_rule_cannot_read_is_refused_unread() {
        for bad in [&[0u8][..], &[0, 0x0E, 0], &[], &[9, 0x0E], &[3, 2]] {
            let (answer, reads) = judge(Role::Admin, bad, Some(status_at(0x0F, 0, 0x0F)));
            assert_eq!(answer, DENIED, "{bad:?}");
            assert_eq!(reads, 0, "{bad:?}");
        }
    }

    /// The app's core has judged the word by the time its request gets here. Only its
    /// id is exempt, whatever a network session's role, and it costs no second read.
    #[test]
    fn wp_hb2_only_the_apps_session_skips_the_voltage_rule() {
        let (r, t) = (router(), Instant::now());
        let req = cmd(
            Dir::Set,
            op::REQ_EXTENSION,
            g::EXT_CORE_VREG,
            VregOp::Set(0x0F).encode().to_vec(),
        );
        let answer = |id: SessionId, role: Role| {
            r.before_exec(
                &ctx(id, role),
                &req,
                t,
                ExecUnit {
                    others_poll: &|_, _| false,
                    read_vreg: &mut || {
                        assert_ne!(id, LOCAL_SESSION_ID, "no second read for the app");
                        Some(status_at(0x0D, 0, 0x0D))
                    },
                },
            )
            .map(|_| ())
        };
        assert_eq!(answer(LOCAL_SESSION_ID, Role::Admin), Ok(()));
        for id in [crate::session::FIRST_REMOTE_SESSION_ID, 3, 0xFFFE] {
            for role in [Role::Viewer, Role::Control, Role::Admin] {
                assert_eq!(answer(id, role), DENIED, "{id} {role:?}");
            }
        }
    }

    /// A refused raise leaves nothing behind that could claim a later notification.
    #[test]
    fn wp_hb2_a_refused_raise_is_not_a_write() {
        let (r, t) = (router(), Instant::now());
        let req = cmd(
            Dir::Set,
            op::REQ_EXTENSION,
            g::EXT_CORE_VREG,
            VregOp::Set(0x0E).encode().to_vec(),
        );
        let answer = r
            .before_exec(
                &ctx(2, Role::Admin),
                &req,
                t,
                ExecUnit {
                    others_poll: &|_, _| false,
                    read_vreg: &mut || None,
                },
            )
            .map(|_| ());
        assert_eq!(answer, DENIED);
        assert_eq!(r.consume_attribution(PacketKind::Scalar, t), NO_ORIGIN);
    }

    /// Neither the read of 0x00 nor another sub-command costs a status read.
    #[test]
    fn wp_hb2_other_extension_commands_are_not_judged() {
        let (r, t) = (router(), Instant::now());
        for req in [
            cmd(Dir::Get, op::REQ_EXTENSION, g::EXT_CORE_VREG, Vec::new()),
            cmd(
                Dir::Set,
                op::REQ_EXTENSION,
                g::EXT_CROSSFEED_PAIR_MAP,
                vec![0; 4],
            ),
        ] {
            let unit = ExecUnit {
                others_poll: &|_, _| false,
                read_vreg: &mut || panic!("no status read for this"),
            };
            assert!(r.before_exec(&ctx(2, Role::Admin), &req, t, unit).is_ok());
        }
    }

    /// Nothing the router says makes a write run again. A repeat is a parameter read only.
    #[test]
    fn wp_hb2_only_a_parameter_read_can_be_retried() {
        let p = Policy::embedded();
        for code in 0..=u8::MAX {
            assert!(!p.retries_stall(code, Dir::Set), "{code:#04x}");
            if p.retries_stall(code, Dir::Get) {
                assert!(!p.mutates(code, Dir::Get), "{code:#04x}");
            }
        }
    }
}
