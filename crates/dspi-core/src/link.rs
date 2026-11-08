//! DSPi Link sessions: the session the `link_*` methods drive, its reconnect
//! supervisor, and the way to a Link device that needs no core lock.
//!
//! Owned by WP C15 (docs/core-merge/PLAN.md). Input: the link part of the
//! Android `dspi-ffi/src/core.rs`, and the Link findings of 2026-11-06.
//!
//! # The supervisor
//!
//! Every session has one supervisor thread, which owns all waiting:
//! - The client's events are only wake-ups on a channel. A client can report
//!   its end from any thread, even one in a setter holding the `Inner` lock
//!   (a failed send), so the forwarder neither locks nor calls a listener.
//! - It calls the [`LinkListener`] with the `Inner` lock released (PLAN §1.2).
//! - A lost session is dialled again with backoff (1, 2, 4 ... 30 s), with the
//!   same token. Not after 4000 or 4002, and not while a PIN is awaited: that
//!   keeps the prompt, and `link_pair` dials when the socket has gone.
//! - The device bound with `link_select_device` is bound again, by serial,
//!   and read: after a reconnect, and when the hub lists it online again.
//! - On a reconnect the token is looked up after the hello
//!   (`SessionConfig::token_for`), so another hub taking over the address
//!   never sees it, and that hub is not accepted as this session's.
//! - What to do about a failure is the client's [`LinkError::retry_advice`]:
//!   `rate_limited` waits a minute and keeps the token, and a refused
//!   handshake ends the session.
//!
//! It is never joined, because a dial may take half a minute: closing sets
//! `cancel`, and it stops at its next step. It holds the core only weakly,
//! and never across a dial.
//!
//! # A lost session
//!
//! A lost session is a state the user sees ([`LinkPhase::Reconnecting`], with
//! a banner and a way out), not a teardown:
//! - the listener hears of it at once;
//! - the connection and the mirror stay, last known, until the device is read
//!   again. The core does not report the device as departed;
//! - calls that need the device fail at once with
//!   [`DspiError::Disconnected`]: its client answers NO_DEVICE without a round
//!   trip. Nothing is queued: a write made now is never sent later;
//! - afterwards the device is read and bound again, the polls subscribed anew,
//!   and the next `process_notifications` reports `NOTIFY_REFRESHED`;
//! - it ends by itself when a person is needed: [`LinkPhase::NeedsPairing`]
//!   (token refused or revoked), [`LinkPhase::Refused`] (a 4xx handshake);
//! - `link_close` leaves it from any thread, at once: see there.
//!
//! # No core lock on the network
//!
//! [`LinkSide`] is the connection's [`SideChannel`]. It reaches the device
//! through the thread-safe [`LinkClient`], beside the connection's own
//! `LinkTransport`, which stays behind the core lock for commands.
//!
//! # Meters and the analyser
//!
//! A bridge's UART is shared by everything, and a tunnelled GET queues with
//! the user's commands. The hub's `poll.subscribe` (spec 7.6, bridge spec
//! 7.7) reads once for all sessions, inside a budget, only after any pending
//! client command, and drops frames for a slow session. So:
//! - `fetch_status` returns the latest frame of slot [`METER_SLOT`]. The
//!   supervisor subscribes on demand and unsubscribes after [`METER_IDLE`]
//!   without a call: a hub must stop polling once nobody subscribes.
//! - The analyser's band frames come from slots 1..=15, one per channel
//!   (`0x0B`; a bridge has no `0x0F`), at a rate sized to half the hub's
//!   budget ([`band_hz`]). Its status and configuration stay tunnelled, at
//!   their 2 Hz. Bins cannot be polled (a 132-byte GET limit on a bridge, and
//!   a poll has no offset): on a bridged hub they are read at most every
//!   [`BRIDGED_BINS_EVERY`].
//! - A refused subscription (`busy`, no `poll` capability) falls back to
//!   tunnelled reads at a low rate, and is asked again every
//!   [`POLL_RETRY_AFTER`]. A reduced grant is simply taken.
//! - A subscription ends with its session or when its device goes offline.
//!   Both give a new connection here, with a new [`LinkSide`], which
//!   subscribes afresh; a feed that goes silent is subscribed again too.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};
use std::time::Duration;

use dspi_transport::clock::Instant;

use dspi_link::clock::{Clock, SystemClock};
use dspi_link::frames::PollFrame;
use dspi_link::messages::{DeviceInfo, PollSpec};
use dspi_link::socket::{TcpConnector, WsConnector};
use dspi_link::{
    AuthState, ConnectOptions, Dir, LinkClient, LinkError, LinkEvents, RetryAdvice, SessionConfig,
    SessionState, Timing,
};
use dspi_proto::generated::opcodes as op;
use dspi_proto::packets::RtaBandFrame;
use dspi_transport::link::map_status;
use dspi_transport::{
    DeviceDescriptor, LinkTransport, Transport, TransportCaps, TransportError, TransportKind,
};

use crate::callbacks::LinkListener;
use crate::records::*;
use crate::{DspiCore, DspiError, MeterRead, Result, SideChannel};

/// The role asked for when pairing, as the macOS Console does.
const PAIR_ROLE: &str = "control";
const APP_NAME: &str = "DSPi Console";
/// `GET /dspi/v1/info`: connect, send and read, each.
const INFO_TIMEOUT: Duration = Duration::from_secs(3);
/// The capability that offers `poll.subscribe` (spec 7.6).
const CAP_POLL: &str = "poll";
/// The capability that offers `lock.acquire` (spec 7.7).
const CAP_LOCK: &str = "lock";
/// Longest lock a client may ask for (spec 7.7).
const LOCK_MAX_MS: u32 = 60_000;
/// `REQ_GET_STATUS` sub-query of the meter block.
const STATUS_METERS: u16 = 9;

/// The poll slot of the meter block.
const METER_SLOT: u8 = 0;
/// Requested meter rate. A bridge shares a UART with everything else.
const METER_HZ: u32 = 15;
const METER_HZ_BRIDGED: u32 = 10;
/// No `fetch_status` for this long: the UI stopped asking, so unsubscribe.
const METER_IDLE: Duration = Duration::from_secs(2);
/// How often the supervisor looks at a bound device's meter feed.
const FEED_CHECK: Duration = Duration::from_secs(1);
/// Requested band-frame rate per channel, before the budget.
const BAND_HZ: u32 = 15;
/// Bytes a band frame costs against the hub's budget: the payload and the
/// POLL frame's header.
const BAND_FRAME_COST: u32 = 90;
/// No frame for this long means the feed is gone: subscribe again.
const FEED_LOST: Duration = Duration::from_secs(3);
/// A frame older than this is not shown (and no bands older than this are
/// fed to the analyser), at the lowest rates too.
const FRAME_STALE_MIN: Duration = Duration::from_millis(1500);
/// After a refused subscription: when to ask again.
const POLL_RETRY_AFTER: Duration = Duration::from_secs(15);
/// Tunnelled meter reads when the hub refuses polls: at most one per this.
const METER_TUNNEL_EVERY: Duration = Duration::from_millis(100);
const METER_TUNNEL_EVERY_BRIDGED: Duration = Duration::from_millis(250);
/// Tunnelled band reads on a bridged hub that refuses polls.
const BRIDGED_BANDS_EVERY: Duration = Duration::from_millis(250);
/// Bin frames on a bridged hub.
const BRIDGED_BINS_EVERY: Duration = Duration::from_millis(250);

/// How long `link_close` waits for its teardown; a stalled link's goes on behind.
const CLOSE_WAIT: Duration = Duration::from_millis(100);
/// How long `link_connect` waits for the teardown of the session it replaces.
const REPLACE_WAIT: Duration = Duration::from_secs(20);

/// The session's timers: production's, or shortened by a test.
#[derive(Clone)]
pub(crate) struct Tuning {
    /// The client's timers.
    pub(crate) timing: Timing,
    /// Backoff delays are divided by this.
    pub(crate) backoff_divisor: u32,
    /// The client's liveness clock.
    pub(crate) clock: Arc<dyn Clock>,
    /// How to open a socket.
    pub(crate) connector: Arc<dyn WsConnector>,
}

impl Default for Tuning {
    fn default() -> Self {
        Self {
            timing: Timing::default(),
            backoff_divisor: 1,
            clock: Arc::new(SystemClock),
            connector: Arc::new(TcpConnector::default()),
        }
    }
}

impl Tuning {
    /// The wait after `failures` failed tries, by the advice for `error`
    /// (none: a lost session). `None`: do not try again.
    fn delay(&self, error: Option<&LinkError>, failures: u32) -> Option<Duration> {
        let advice = error.map_or(
            RetryAdvice::Retry {
                min_delay: Duration::ZERO,
            },
            LinkError::retry_advice,
        );
        let delay = advice.delay(failures)?;
        Some(delay.checked_div(self.backoff_divisor).unwrap_or(delay))
    }
}

/// What wakes the supervisor.
enum Msg {
    /// The device list of the client with this epoch changed.
    Devices(u64),
    /// The hub issued a token, perhaps after `link_pair` timed out.
    Token { hub_id: String, token: String },
    /// Something else changed: look again.
    Kick,
}

/// The client's events, as messages. Safe on any thread, under any lock, and
/// never blocking: some run on the client's reader thread.
struct Forward {
    tx: Sender<Msg>,
}

impl LinkEvents for Forward {
    fn session_devices_changed(&self, epoch: u64) {
        let _ = self.tx.send(Msg::Devices(epoch));
    }

    fn session_closed(&self, _epoch: u64, _code: Option<u16>, _reason: &str) {
        let _ = self.tx.send(Msg::Kick);
    }

    fn token_issued(&self, hub_id: &str, token: &str) {
        let _ = self.tx.send(Msg::Token {
            hub_id: hub_id.to_lowercase(),
            token: token.to_owned(),
        });
    }
}

/// How to reach the hub again.
#[derive(Clone)]
struct Target {
    url: String,
    client_name: String,
    app_version: String,
    /// The token for this hub, kept until a pairing replaces it, or the hub
    /// refuses or revokes it.
    token: String,
    /// The `hid` of the hub this session belongs to.
    hub_id: String,
    connector: Arc<dyn WsConnector>,
    timing: Timing,
    clock: Arc<dyn Clock>,
}

/// What `link_close` needs without the `Inner` lock, which a command on a
/// stalled link can hold for seconds. Its mutex is never held on the network
/// or while taking `Inner`.
pub(crate) struct LinkControl {
    id: u64,
    cancel: Arc<AtomicBool>,
    tx: Sender<Msg>,
    client: Option<LinkClient>,
    // -- WP WB4b --
    /// No supervisor thread: the caller turns it with [`DspiCore::link_tick`].
    pumped: bool,
    /// The supervisor's messages, while no turn has them.
    driver: Option<Receiver<Msg>>,
}

/// The device bound with `link_select_device`.
struct Bound {
    handle: u8,
    serial: String,
    /// The connection last attached for it.
    generation: u64,
    /// `link_select_device` is opening it right now.
    opening: bool,
    /// That connection is still good: false after a reconnect, or when the
    /// device came back online.
    attached: bool,
    /// The hub listed the device as not online since it was attached.
    seen_offline: bool,
    /// The client's `offline_count` when it was attached. A change means a
    /// flap, however quick: the hub ended this session's polls.
    offline_base: u64,
    side: Option<Arc<LinkSide>>,
    // -- WP LK4 --
    /// The probe of its last attach, to describe it again on a re-attach
    /// when it is the same device and firmware (no feature reads).
    probe: Option<dspi_session::probe::Reusable>,
}

/// A planned try: how many have failed, and when the next one is due.
#[derive(Clone, Copy)]
struct Retry {
    failures: u32,
    next_at: Instant,
}

/// The current DSPi Link session (WP C15). Dropping it stops the supervisor
/// and closes the socket, which may block briefly: drop it with the `Inner`
/// lock released, or when the core itself goes.
pub(crate) struct LinkSession {
    id: u64,
    target: Target,
    tuning: Tuning,
    client: Option<LinkClient>,
    /// The epoch of the client whose events count.
    instance: u64,
    pin_source: String,
    /// The hub wants a PIN. Survives the socket: the prompt stays up.
    needs_pin: bool,
    /// The hub refused or revoked the token: the app should forget it.
    token_refused: bool,
    /// The current client's end has been handled.
    loss_seen: bool,
    retry: Option<Retry>,
    last_close: Option<(Option<u16>, String)>,
    /// Over, and not retried.
    ended: bool,
    /// Ended because the hub refuses this client (a 4xx handshake).
    refused: bool,
    /// The app closed the bound device with the session up. If it now opens
    /// another one, it has left the hub.
    released: bool,
    /// The side of a binding given up, to be dropped outside the lock.
    retired: Option<Arc<LinkSide>>,
    /// The next re-attach's read fails.
    #[cfg(test)]
    fail_next_read: bool,
    /// An end the listener has not heard of yet.
    unreported_end: Option<Closed>,
    bound: Option<Bound>,
    /// The phase last reported to the listener.
    reported: LinkPhase,
    cancel: Arc<AtomicBool>,
    tx: Sender<Msg>,
}

impl Drop for LinkSession {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::SeqCst);
        let _ = self.tx.send(Msg::Kick);
        if let Some(c) = self.client.take() {
            c.set_events(None);
            c.close();
        }
    }
}

/// Whether the client exists and its socket is up.
fn alive(client: Option<&LinkClient>) -> bool {
    client.is_some_and(|c| !c.is_closed())
}

/// A board the core can talk to, or soon can.
fn board_present(state: &str) -> bool {
    matches!(state, "online" | "updating")
}

impl LinkSession {
    /// Whether the bound device has a live connection `conn_gen`.
    fn attached(&self, conn_gen: Option<u64>) -> bool {
        self.bound
            .as_ref()
            .is_some_and(|b| b.attached && conn_gen == Some(b.generation))
    }

    fn phase(&self, conn_gen: Option<u64>) -> LinkPhase {
        if self.refused {
            return LinkPhase::Refused;
        }
        if self.ended {
            return LinkPhase::Closed;
        }
        if self.needs_pin {
            return LinkPhase::NeedsPairing;
        }
        let Some(client) = self.client.as_ref() else {
            return LinkPhase::Reconnecting;
        };
        match client.session_state() {
            SessionState::Online => {}
            SessionState::NoBoard => return LinkPhase::NoDevice,
            SessionState::NeedsAuth | SessionState::DeviceListUnknown | SessionState::Closed => {
                return LinkPhase::Reconnecting;
            }
        }
        let devices = client.devices();
        match &self.bound {
            None => LinkPhase::ChooseDevice,
            Some(b) if b.opening => LinkPhase::ChooseDevice,
            Some(b) => match devices.iter().find(|d| d.serial == b.serial) {
                Some(d) if !board_present(&d.state) => LinkPhase::NoDevice,
                None => LinkPhase::NoDevice,
                Some(_) if self.attached(conn_gen) => LinkPhase::Bound,
                Some(_) => LinkPhase::Reconnecting,
            },
        }
    }

    /// Plan the next try after a failure, or end the session if the advice
    /// is not to try again.
    fn schedule_retry(&mut self, error: Option<&LinkError>, now: Instant) {
        let failures = self.retry.map_or(0, |r| r.failures.saturating_add(1));
        match self.tuning.delay(error, failures) {
            Some(delay) => {
                self.retry = Some(Retry {
                    failures,
                    next_at: now.checked_add(delay).unwrap_or(now),
                });
            }
            None => {
                let reason = error.map_or_else(String::new, ToString::to_string);
                self.retry = None;
                self.ended = true;
                self.refused = matches!(error, Some(LinkError::HandshakeRefused { .. }));
                self.unreported_end = Some((None, reason));
            }
        }
    }

    /// The current client's end, once: decide what follows.
    fn note_loss(&mut self, conn_gen: Option<u64>, now: Instant) {
        let Some((code, reason)) = self.client.as_ref().and_then(LinkClient::closed_reason) else {
            return;
        };
        if std::mem::replace(&mut self.loss_seen, true) {
            return;
        }
        // Closed before the loss, by the app (`disconnect`, another device).
        if self
            .bound
            .as_ref()
            .is_some_and(|b| b.attached && conn_gen != Some(b.generation))
        {
            self.bound = None;
            self.released = true;
        }
        if let Some(b) = &mut self.bound {
            b.attached = false;
        }
        self.last_close = Some((code, reason.clone()));
        if code == Some(4002) {
            // Revoked: a person has to pair again; `link_pair` dials.
            self.target.token.clear();
            self.token_refused = true;
            self.needs_pin = true;
            self.retry = None;
        }
        if self.needs_pin {
            return;
        }
        if dspi_link::should_reconnect(code) {
            self.retry = None;
            self.schedule_retry(None, now);
        } else {
            self.ended = true;
            self.unreported_end = Some((code, reason));
        }
    }
}

/// A closing to report: the code and the reason.
type Closed = (Option<u16>, String);

/// One step of the supervisor, decided under the lock.
enum Step {
    /// The session is gone.
    Stop,
    Wait(Option<Duration>, Option<Arc<LinkSide>>),
    Dial(Box<Target>),
    Rebind(u8, String),
    /// Read a device list that failed after a pairing.
    ReadDevices(LinkClient),
}

/// A session number; never reused.
static NEXT_SESSION: AtomicU64 = AtomicU64::new(1);
#[cfg_attr(feature = "uniffi", uniffi::export)]
impl DspiCore {
    /// Open a DSPi Link session to `url` (`link_ws_url`), replacing any
    /// previous one. `token` = the token stored for this hub's id ("" =
    /// none). `expected_hub_id` = that id. With it, a token goes out only after
    /// the hello has named that hub, and another hub at the address fails the
    /// call with `wrong_hub`, having seen no token. "" = not known: the token
    /// goes to whichever hub answers (a hub added by address, no stored id).
    /// Blocks for the hello and authentication (up to ~15 s). When
    /// `needs_pairing`, ask for the PIN and call `link_pair`; otherwise the
    /// device list is ready (`link_devices`).
    ///
    /// From here on the core keeps the session up by itself: see
    /// [`LinkListener`] and `link_status`. An app does not redial.
    ///
    /// Errors: [`DspiError::Link`], its `code` being `wrong_hub`, `io`, `handshake`,
    /// `hub_busy` (all the hub's sessions are in use; try again in a few
    /// seconds), `handshake_refused` (trying again will not help),
    /// `rate_limited` (the token stays good; wait a minute), `protocol`,
    /// `timeout`, `closed`, or the hub's own.
    pub fn link_connect(
        &self,
        url: String,
        client_name: String,
        app_version: String,
        token: String,
        expected_hub_id: String,
    ) -> Result<LinkHello> {
        self.link_connect_with(
            url,
            client_name,
            app_version,
            token,
            expected_hub_id,
            Tuning::default(),
        )
    }

    /// Pair with the PIN the user typed (role `control`, as the macOS
    /// Console asks). Returns the token to store against the hub id ("" when
    /// the hub issued none). If the hub dropped the socket while the prompt
    /// was up (its 30 s auth deadline), it is dialled again first. A pairing
    /// succeeds even if the device list then fails; the core reads it again.
    ///
    /// Errors, as [`DspiError::Link`] with `msg` ready to show:
    /// - `wrong_pin`: a wrong PIN, or, on a hub showing its PIN in a window,
    ///   one that is wrong or expired, or a closed window (the hub does not
    ///   say which). Ask again;
    /// - `rate_limited`: too many wrong tries; the hub refuses for a minute;
    /// - `timeout`: no answer. The hub may still have accepted the PIN:
    ///   [`LinkListener::on_link_token_issued`] then still delivers the token;
    /// - anything else [`DspiCore::link_connect`] reports;
    /// - [`DspiError::Disconnected`] with no session.
    pub fn link_pair(&self, pin: String) -> Result<String> {
        let (client, instance) = self.pairing_client()?;
        let token = client.pair(pin.trim(), PAIR_ROLE).map_err(map_error)?;
        let mut inner = self.lock();
        if let Some(s) = inner.link.as_mut().filter(|s| s.instance == instance) {
            s.needs_pin = false;
            s.last_close = None;
            if !token.is_empty() {
                s.token_refused = false;
                s.target.token.clone_from(&token);
            }
            let _ = s.tx.send(Msg::Kick);
        }
        Ok(token)
    }

    /// The hub's devices (empty before authentication).
    pub fn link_devices(&self) -> Vec<LinkDevice> {
        let client = self.lock().link.as_ref().and_then(|s| s.client.clone());
        client
            .filter(|c| !c.is_closed())
            .map(|c| c.devices().into_iter().map(device_record).collect())
            .unwrap_or_default()
    }

    /// Where the session stands; [`LinkStatus::phase`] is what to tell the
    /// user. No blocking beyond the core lock.
    pub fn link_status(&self) -> LinkStatus {
        let inner = self.lock();
        let Some(s) = inner
            .link
            .as_ref()
            .filter(|s| !s.cancel.load(Ordering::SeqCst))
        else {
            return LinkStatus {
                close_code: -1,
                ..LinkStatus::default()
            };
        };
        let phase = s.phase(inner.conn.as_ref().map(|c| c.generation));
        let client = s.client.as_ref().filter(|c| !c.is_closed());
        // The client's own word until the supervisor has looked: the banner
        // has a reason from its first moment.
        let (code, reason) = s
            .last_close
            .clone()
            .or_else(|| s.client.as_ref().and_then(LinkClient::closed_reason))
            .unwrap_or((None, String::new()));
        let now = Instant::now();
        LinkStatus {
            connected: client.is_some(),
            ready: client.is_some_and(LinkClient::is_ready),
            session: client.and_then(LinkClient::session_id).map_or(0, u32::from),
            role: client.map(LinkClient::role).unwrap_or_default(),
            denied: client
                .map(LinkClient::denied)
                .unwrap_or_default()
                .into_iter()
                .map(|(request, dir)| LinkDenied { request, dir })
                .collect(),
            closed: matches!(phase, LinkPhase::Closed | LinkPhase::Refused),
            close_code: code.map_or(-1, i32::from),
            close_reason: reason,
            phase,
            reconnect_attempt: s.retry.map_or(0, |r| r.failures),
            reconnect_in_ms: s.retry.map_or(0, |r| {
                let left = r.next_at.saturating_duration_since(now).as_millis();
                u32::try_from(left).unwrap_or(u32::MAX)
            }),
            pin_source: s.pin_source.clone(),
            token_refused: s.token_refused,
        }
    }

    /// Bind the core to the hub's device `handle` (as `open_fd` does for
    /// USB): starts the notify listener over Link, resets the state. Then
    /// `fetch_all` as usual (single-shot bulk, the hub's snapshot first).
    /// Selecting again uses the same hub session and slot.
    ///
    /// The core binds it again by itself after a reconnect, and when the
    /// hub lists it online again, unless the app called `disconnect` while
    /// the session was up. `disconnect` while reconnecting does not stop
    /// that: `link_close` does.
    ///
    /// Opening another device over the bound one closes the hub session. After `disconnect` the session stays, ready
    /// for another `link_select_device`, and keeps its slot: call
    /// `link_close` when leaving the hub.
    ///
    /// Errors: [`DspiError::InvalidArgument`] for a handle over 255 (they
    /// are one byte on the wire); [`DspiError::Disconnected`] when the
    /// session is not ready; as for the connect sequence.
    pub fn link_select_device(&self, handle: u32, serial: String) -> Result<()> {
        let handle = u8::try_from(handle).map_err(|_| DspiError::InvalidArgument {
            msg: format!("Link device handle {handle} is over 255"),
        })?;
        // Dropped with the lock released: a side's drop writes to the socket.
        let _replaced = {
            let mut inner = self.lock();
            let s = inner.link.as_mut().ok_or(DspiError::Disconnected)?;
            if !s.client.as_ref().is_some_and(LinkClient::is_ready) {
                return Err(DspiError::Disconnected);
            }
            s.released = false;
            s.bound.replace(Bound {
                handle,
                serial: serial.clone(),
                generation: 0,
                opening: true,
                attached: false,
                seen_offline: false,
                offline_base: 0,
                side: None,
                probe: None,
            })
        };
        // -- WP LK4 -- before the open: the new side subscribes the meter
        // poll during it, and an old side of the same handle unsubscribes
        // its slot when dropped
        drop(_replaced);
        let opened = self.open_bound(handle, serial, false);
        if opened.is_err() {
            {
                let mut inner = self.lock();
                if let Some(s) = inner.link.as_mut()
                    && s.bound.as_ref().is_some_and(|b| b.opening)
                {
                    s.bound = None;
                }
            }
            // The failure may have come after the attach.
            self.close_link_device();
        }
        opened
    }

    /// Close the Link session (normal close) and drop a device bound to it,
    /// and stop any reconnecting. This is how an app leaves a hub, giving back
    /// its session slot (`disconnect` keeps the session, for another
    /// `link_select_device`), and it is the banner's abort button.
    ///
    /// Safe on a UI thread: it returns within about 100 ms whatever the link
    /// does, and `link_status` then says [`LinkPhase::Idle`]. It does not need
    /// the core lock, and ends a command stuck on a stalled link. On a healthy
    /// link the device is closed when it returns; on a stalled one that
    /// finishes behind it. A dial in progress is thrown away. A USB device open
    /// meanwhile stays open. Quiet: no `on_link_closed`.
    pub fn link_close(&self) {
        self.close_session(CLOSE_WAIT);
    }

    /// Register (`Some`) or clear (`None`) the session events listener. Same
    /// rules as the other listeners: called with the core lock released, and
    /// clearing waits for a call in progress.
    pub fn set_link_listener(&self, listener: Option<Arc<dyn LinkListener>>) {
        self.listeners.link.set(listener);
    }

    /// Try to reconnect now instead of waiting out the backoff (the platform
    /// says the network is back). A no-op unless reconnecting.
    pub fn link_retry_now(&self) {
        let mut inner = self.lock();
        if let Some(s) = inner.link.as_mut()
            && let Some(r) = s.retry.as_mut()
        {
            r.next_at = Instant::now();
            let _ = s.tx.send(Msg::Kick);
        }
    }

    /// The hub answers at a new address (the app's browser found its `hid`
    /// again after a DHCP change): reconnect there from now on, at once if
    /// reconnecting. The core does not browse by itself.
    ///
    /// An address that cannot be dialled is logged and ignored: reconnecting
    /// to it would fail for ever.
    pub fn link_set_url(&self, url: String) {
        if !valid_ws_url(&url) {
            log::warn!("ignoring an invalid Link hub address: {url}");
            return;
        }
        let mut inner = self.lock();
        if let Some(s) = inner.link.as_mut() {
            s.target.url = url;
            if let Some(r) = s.retry.as_mut() {
                r.next_at = Instant::now();
            }
            let _ = s.tx.send(Msg::Kick);
        }
    }
}

impl DspiCore {
    /// Stop the session now, finish its teardown on a thread of its own, and
    /// wait for that up to `wait`.
    fn close_session(&self, wait: Duration) {
        let Some(control) = guard(&self.link_control).take() else {
            return;
        };
        control.cancel.store(true, Ordering::SeqCst);
        let _ = control.tx.send(Msg::Kick);
        let Some(core) = self.me.upgrade() else {
            return;
        };
        let (done, finished) = mpsc::sync_channel::<()>(1);
        let id = control.id;
        let client = control.client;
        // -- WP WB4b -- no threads, and no other call can hold the lock: here, now
        if control.pumped {
            core.finish_close(id, client);
            return;
        }
        let spawned = std::thread::Builder::new()
            .name("dspi-link-close".into())
            .spawn(move || {
                core.finish_close(id, client);
                let _ = done.send(());
            });
        match spawned {
            Ok(_) => {
                let _ = finished.recv_timeout(wait);
            }
            Err(e) => log::warn!("cannot close the Link session in the background: {e}"),
        }
    }

    /// The rest of a close. The socket goes first: that ends whatever is stuck
    /// on it, a command holding the core lock or the analyser's poll included.
    fn finish_close(&self, id: u64, client: Option<LinkClient>) {
        if let Some(c) = &client {
            c.set_events(None);
            c.close();
        }
        let (session, ours) = {
            let mut inner = self.lock();
            let link = inner
                .conn
                .as_ref()
                .is_some_and(|c| c.caps.kind == TransportKind::Link);
            if inner.link.as_ref().is_some_and(|s| s.id == id) {
                (inner.link.take(), link)
            } else {
                (None, false)
            }
        };
        if ours {
            self.close_connection();
        }
        drop(session);
    }

    /// Close the open connection if it is a Link one.
    fn close_link_device(&self) {
        let link = self
            .lock()
            .conn
            .as_ref()
            .is_some_and(|c| c.caps.kind == TransportKind::Link);
        if link {
            self.close_connection();
        }
    }

    /// Note the session's current client for `link_close`.
    fn control_client(&self, id: u64, client: &LinkClient) {
        if let Some(c) = guard(&self.link_control).as_mut().filter(|c| c.id == id) {
            c.client = Some(client.clone());
        }
    }

    /// Close the connection for a re-attach: as `close_connection`, but the
    /// mirror stays last known.
    fn close_keeping_mirror(&self) {
        let (old, worker) = {
            let mut inner = self.lock();
            let worker = crate::rta::take_worker(&mut inner);
            (inner.conn.take(), worker)
        };
        drop(self.set_meter_path(None)); // -- WP LK2 --
        if let Some(worker) = worker {
            worker.stop();
        }
        if let Some(conn) = old {
            crate::device::shut(conn);
        }
    }

    /// [`DspiCore::link_connect`] over `connector` (tests inject faults).
    pub(crate) fn link_connect_with(
        &self,
        url: String,
        client_name: String,
        app_version: String,
        token: String,
        expected_hub_id: String,
        tuning: Tuning,
    ) -> Result<LinkHello> {
        self.close_session(REPLACE_WAIT);
        let (tx, rx) = mpsc::channel();
        let mut target = Target {
            url,
            client_name,
            app_version,
            token,
            hub_id: expected_hub_id.to_lowercase(),
            connector: Arc::clone(&tuning.connector),
            timing: tuning.timing,
            clock: Arc::clone(&tuning.clock),
        };
        // -- WP WB4b -- decided on the caller's thread, as the client's own mode is
        let pumped = dspi_link::client::caller_driven();
        let (client, state) = dial(&target, true, &tx).map_err(map_error)?;
        if !target.hub_id.is_empty() && !client.hello().hub.id.eq_ignore_ascii_case(&target.hub_id)
        {
            client.set_events(None);
            client.close();
            return Err(DspiError::Link {
                code: "wrong_hub".into(),
                msg: "Another hub answers at this address.".into(),
            });
        }
        let hello = hello_record(&client, &state);
        target.hub_id.clone_from(&hello.hub_id);
        if hello.token_refused {
            target.token.clear();
        }
        let instance = client.epoch();
        let cancel = Arc::new(AtomicBool::new(false));
        let session = LinkSession {
            id: NEXT_SESSION.fetch_add(1, Ordering::Relaxed),
            target,
            tuning,
            client: Some(client),
            instance,
            pin_source: hello.pin_source.clone(),
            needs_pin: hello.needs_pairing,
            token_refused: hello.token_refused,
            loss_seen: false,
            retry: None,
            last_close: None,
            ended: false,
            refused: false,
            released: false,
            retired: None,
            #[cfg(test)]
            fail_next_read: false,
            unreported_end: None,
            bound: None,
            reported: LinkPhase::Idle,
            cancel: Arc::clone(&cancel),
            tx,
        };
        let (me, id) = (self.me.clone(), session.id);
        let kick = session.tx.clone();
        *guard(&self.link_control) = Some(LinkControl {
            id,
            cancel: Arc::clone(&cancel),
            tx: kick.clone(),
            client: session.client.clone(),
            pumped,
            driver: None,
        });
        // Installed first: the supervisor ends when it finds no session.
        let displaced = self.lock().link.replace(session);
        drop(displaced);
        // -- WP WB4b -- the caller turns the supervisor (`link_tick`)
        if pumped {
            if let Some(c) = guard(&self.link_control).as_mut().filter(|c| c.id == id) {
                c.driver = Some(rx);
            }
            let _ = kick.send(Msg::Kick);
            return Ok(hello);
        }
        let spawned = std::thread::Builder::new()
            .name("dspi-link".into())
            .spawn(move || supervise(&me, id, &rx, &cancel));
        if let Err(e) = spawned {
            *guard(&self.link_control) = None;
            let session = self.lock().link.take();
            drop(session);
            return Err(DspiError::Io {
                msg: format!("cannot start the Link session thread: {e}"),
            });
        }
        let _ = kick.send(Msg::Kick);
        Ok(hello)
    }

    /// The client to pair on: the current one, or a new one if the hub
    /// dropped the socket during the prompt.
    fn pairing_client(&self) -> Result<(LinkClient, u64)> {
        let (target, tx) = {
            let inner = self.lock();
            let s = inner.link.as_ref().ok_or(DspiError::Disconnected)?;
            if let Some(c) = s.client.as_ref().filter(|c| !c.is_closed()) {
                return Ok((c.clone(), s.instance));
            }
            if !s.needs_pin {
                return Err(DspiError::Disconnected);
            }
            (s.target.clone(), s.tx.clone())
        };
        let (client, state) = dial(&target, false, &tx).map_err(map_error)?;
        let instance = client.epoch();
        let mut inner = self.lock();
        let Some(s) = inner.link.as_mut() else {
            drop(inner);
            client.close();
            return Err(DspiError::Disconnected);
        };
        s.pin_source = client.hello().pin_source().to_owned();
        s.client = Some(client.clone());
        s.instance = instance;
        s.loss_seen = false;
        s.needs_pin = state.needs_pin();
        let id = s.id;
        drop(inner);
        self.control_client(id, &client);
        Ok((client, instance))
    }

    /// Open device `handle` on the current client and record it as bound.
    ///
    /// `keep_mirror`: a re-attach. The mirror stays last known until the new
    /// image replaces it, and a failure leaves it.
    fn open_bound(&self, handle: u8, serial: String, keep_mirror: bool) -> Result<()> {
        let (client, instance, tx, reuse, old_side) = {
            let mut inner = self.lock();
            let s = inner.link.as_mut().ok_or(DspiError::Disconnected)?;
            let client = s.client.clone().filter(LinkClient::is_ready);
            // -- WP LK4 -- the last probe of this device, for a re-attach
            let reuse = s
                .bound
                .as_ref()
                .filter(|b| keep_mirror && b.serial == serial)
                .and_then(|b| b.probe.clone());
            let client = client.ok_or(DspiError::Disconnected)?;
            // -- WP LK4 -- the old side goes before the new one subscribes:
            // its drop unsubscribes the meter slot on this handle
            let old_side = s.bound.as_mut().and_then(|b| b.side.take());
            (client, s.instance, s.tx.clone(), reuse, old_side)
        };
        // Read before the attach: a flap during it counts as one after it.
        let offline_base = client.offline_count(u32::from(handle));
        let transport = LinkTransport::new(client.clone(), handle, &serial);
        // -- WP LK4 -- the meter feed starts as soon as the image says how
        // many channels there are, before the feature reads: `fetch_status`
        // serves it from then on. On the supervisor's own thread (a
        // re-attach) it subscribes here; otherwise the supervisor does,
        // alongside the reads.
        if keep_mirror {
            self.close_keeping_mirror();
        } else {
            self.close_connection();
        }
        drop(old_side);
        let side = Arc::new(LinkSide::new(client.clone(), handle, &serial, tx.clone()));
        let mut on_channels = |n: u8| {
            side.want_meters(crate::connect::meter_len(n), keep_mirror);
            drop(self.set_meter_path(Some(crate::MeterPath {
                num_channels: n,
                side: Arc::clone(&side) as Arc<dyn SideChannel>,
            })));
        };
        let attached = self.attach_opened_with(
            Box::new(transport),
            serial.clone(),
            0,
            &mut on_channels,
            reuse.as_ref(),
        );
        let probe = match attached {
            Ok(probe) => probe,
            Err(e) => {
                self.drop_meter_path_of(&side);
                return Err(e);
            }
        };

        // Dropped with the lock released: a side's drop writes to the socket.
        let mut old_side = None;
        let mut guard = self.lock();
        let inner = &mut *guard;
        let Some(current) = inner.link.as_mut().filter(|s| s.instance == instance) else {
            drop(guard);
            self.drop_meter_path_of(&side);
            return Err(DspiError::Disconnected);
        };
        let Some(conn) = inner
            .conn
            .as_mut()
            .filter(|c| c.caps.kind == TransportKind::Link && c.serial == serial)
        else {
            drop(guard);
            self.drop_meter_path_of(&side);
            return Err(DspiError::Disconnected);
        };
        conn.side = Some(Arc::clone(&side) as Arc<dyn SideChannel>);
        // -- WP LK2 -- meters from now on, whatever holds the core lock
        let old_path = self.set_meter_path(Some(crate::MeterPath {
            num_channels: inner.state.num_channels,
            side: Arc::clone(&side) as Arc<dyn SideChannel>,
        }));
        if let Some(b) = current.bound.as_mut() {
            b.handle = handle;
            b.generation = conn.generation;
            b.opening = false;
            b.attached = true;
            b.seen_offline = false;
            b.offline_base = offline_base;
            b.probe = probe;
            old_side = b.side.replace(side);
        }
        current.retry = None;
        let _ = current.tx.send(Msg::Kick);
        drop(guard);
        drop(old_side);
        drop(old_path);
        Ok(())
    }

    // -- WP LK4 --
    /// Clear the meter path if it is `side`'s (a failed open set it early).
    fn drop_meter_path_of(&self, side: &Arc<LinkSide>) {
        let ours = self.meter_path().is_some_and(|p| {
            std::ptr::addr_eq(Arc::as_ptr(&p.side), Arc::as_ptr(side))
        });
        if ours {
            drop(self.set_meter_path(None));
        }
    }

    /// Decide the next supervisor step, under the lock.
    fn plan_step(&self, id: u64, now: Instant) -> (Step, Option<Closed>, bool) {
        let mut locked = self.lock();
        let inner = &mut *locked;
        let conn_gen = inner.conn.as_ref().map(|c| c.generation);
        let Some(s) = inner
            .link
            .as_mut()
            .filter(|s| s.id == id && !s.cancel.load(Ordering::SeqCst))
        else {
            return (Step::Stop, None, false);
        };
        // The app opened another device over the bound one: it has left the
        // hub, so give back the slot (a bridge has only 4 or 8) and stop
        // reconnecting. `link_select_device` on this session is not this.
        let displaced = match &s.bound {
            Some(b) => !b.opening && conn_gen.is_some_and(|g| g != b.generation),
            None => s.released && conn_gen.is_some(),
        };
        if displaced {
            log::info!("the Link device was displaced: closing its hub session");
            let session = inner.link.take();
            drop(locked);
            *guard(&self.link_control) = None;
            drop(session);
            return (Step::Stop, None, true);
        }
        s.note_loss(conn_gen, now);
        let step = next_step(s, conn_gen, now);
        let retired = s.retired.take();
        let ended = s.unreported_end.take();
        let phase = s.phase(conn_gen);
        let changed = std::mem::replace(&mut s.reported, phase) != phase;
        // A side's drop writes to the socket: not under the lock.
        drop(locked);
        drop(retired);
        (step, ended, changed)
    }

    /// Install the result of a dial. Returns false when the session has gone
    /// (the new client is then closed).
    fn finish_dial(
        &self,
        id: u64,
        dialled: std::result::Result<(LinkClient, AuthState), LinkError>,
    ) -> bool {
        let now = Instant::now();
        let mut inner = self.lock();
        let Some(s) = inner
            .link
            .as_mut()
            .filter(|s| s.id == id && !s.cancel.load(Ordering::SeqCst))
        else {
            drop(inner);
            if let Ok((client, _)) = dialled {
                client.close();
            }
            return false;
        };
        match dialled {
            Ok((client, state)) => {
                if matches!(state, AuthState::TokenRefused { .. }) {
                    s.target.token.clear();
                    s.token_refused = true;
                }
                s.pin_source = client.hello().pin_source().to_owned();
                s.instance = client.epoch();
                self.control_client(id, &client);
                s.client = Some(client);
                s.loss_seen = false;
                s.retry = None;
                s.needs_pin = state.needs_pin();
                if !s.needs_pin {
                    s.last_close = None;
                }
                if let Some(b) = &mut s.bound {
                    b.attached = false;
                }
            }
            Err(e) => {
                log::info!("DSPi Link reconnect failed: {e}");
                s.last_close = Some((None, e.to_string()));
                s.schedule_retry(Some(&e), now);
            }
        }
        true
    }

    /// The read after a re-attach.
    fn read_rebound(&self) -> Result<()> {
        #[cfg(test)]
        if let Some(s) = self.lock().link.as_mut()
            && std::mem::take(&mut s.fail_next_read)
        {
            return Err(DspiError::Transfer { msg: "lost".into() });
        }
        self.fetch_all()
    }

    /// A token from the hub: keep it for reconnects, and tell the app.
    fn note_token(&self, hub_id: String, token: String) {
        {
            let mut inner = self.lock();
            if let Some(s) = inner.link.as_mut() {
                // This can come after the session's end was handled, and a
                // revoked token must not come back with it.
                if alive(s.client.as_ref()) {
                    s.needs_pin = false;
                    s.token_refused = false;
                }
                if !s.token_refused {
                    s.target.token.clone_from(&token);
                }
            }
        }
        self.listeners
            .link
            .call(|l| l.on_link_token_issued(hub_id, token));
    }

    /// The device list that failed after a pairing: read it again.
    fn read_devices(&self, id: u64, client: &LinkClient) {
        let result = client.refresh_devices();
        let mut inner = self.lock();
        if let Some(s) = inner.link.as_mut().filter(|s| s.id == id) {
            match result {
                Ok(_) => s.retry = None,
                Err(e) => {
                    log::info!("reading the hub's device list failed: {e}");
                    s.schedule_retry(Some(&e), Instant::now());
                }
            }
        }
    }

    /// Bind the device again and read it. On failure the connection is closed,
    /// the mirror goes back to what it was, and the supervisor tries again later.
    fn rebind(&self, id: u64, handle: u8, serial: String) {
        // `install` replaces the mirror before the per-item reads: a read that
        // then fails must not leave a half-read one where the last known was.
        let last_known = self.lock().state.clone();
        let result = self
            .open_bound(handle, serial, true)
            .and_then(|()| self.read_rebound());
        let ended_meanwhile = {
            let mut inner = self.lock();
            let current = inner.link.as_mut().filter(|s| s.id == id);
            let ended = current.is_none();
            if let Some(s) = current {
                match &result {
                    Ok(()) => s.retry = None,
                    Err(e) => {
                        log::info!("re-attaching the Link device failed: {e}");
                        s.last_close = Some((None, format!("re-attaching the device failed: {e}")));
                        if let Some(b) = &mut s.bound {
                            b.attached = false;
                        }
                        s.schedule_retry(None, Instant::now());
                    }
                }
                if result.is_ok()
                    && let Some(conn) = inner.conn.as_mut()
                {
                    conn.announce_refresh = true;
                }
            }
            ended
        };
        if ended_meanwhile {
            self.close_link_device();
        } else if result.is_err() {
            self.close_keeping_mirror();
            self.lock().state = last_known;
        } else {
            self.listeners.notify.call(|l| l.on_notifications());
        }
    }
}

/// Whether the open connection is bound to a session still working on it: a
/// stream that ended there is a lost session, not a departed device.
pub(crate) fn session_recovers(inner: &crate::Inner) -> bool {
    let generation = inner.conn.as_ref().map(|c| c.generation);
    inner.link.as_ref().is_some_and(|s| {
        !s.ended
            && !s.cancel.load(Ordering::SeqCst)
            && s.bound
                .as_ref()
                .is_some_and(|b| generation == Some(b.generation))
    })
}

/// The next step for session `s`, under the lock.
fn next_step(s: &mut LinkSession, conn_gen: Option<u64>, now: Instant) -> Step {
    let feed = s.bound.as_ref().and_then(|b| b.side.clone());
    if s.ended {
        return Step::Wait(None, None);
    }
    if !alive(s.client.as_ref()) {
        if s.needs_pin {
            return Step::Wait(None, None);
        }
        return match s.retry {
            Some(r) if now >= r.next_at => Step::Dial(Box::new(s.target.clone())),
            Some(r) => Step::Wait(Some(r.next_at.saturating_duration_since(now)), None),
            // Should not happen (`note_loss` runs before this), but a session
            // must never sit here for ever with nothing planned.
            None => {
                s.schedule_retry(None, now);
                Step::Wait(
                    Some(FEED_CHECK.min(s.tuning.delay(None, 0).unwrap_or(FEED_CHECK))),
                    None,
                )
            }
        };
    }
    let Some(client) = s.client.clone().filter(LinkClient::is_ready) else {
        return Step::Wait(None, None);
    };
    if client.session_state() == SessionState::DeviceListUnknown {
        return match s.retry {
            Some(r) if now < r.next_at => {
                Step::Wait(Some(r.next_at.saturating_duration_since(now)), None)
            }
            _ => Step::ReadDevices(client),
        };
    }
    let devices = client.devices();
    // Closed by the app with the session up (`disconnect`).
    if s.bound
        .as_ref()
        .is_some_and(|b| !b.opening && b.attached && conn_gen != Some(b.generation))
    {
        s.retired = s.bound.take().and_then(|b| b.side);
        s.released = true;
    }
    let Some(b) = s.bound.as_mut().filter(|b| !b.opening) else {
        return Step::Wait(None, None);
    };
    if client.offline_count(u32::from(b.handle)) != b.offline_base {
        b.seen_offline = true;
    }
    let Some(d) = devices.iter().find(|d| d.serial == b.serial) else {
        b.seen_offline = true;
        return Step::Wait(Some(FEED_CHECK), feed);
    };
    if !board_present(&d.state) {
        b.seen_offline = true;
        return Step::Wait(Some(FEED_CHECK), feed);
    }
    if b.attached && std::mem::take(&mut b.seen_offline) {
        // The hub ended the feed, and the device may have changed.
        b.attached = false;
    }
    if b.attached {
        return Step::Wait(Some(FEED_CHECK), feed);
    }
    let handle = u8::try_from(d.handle).unwrap_or(b.handle);
    let serial = b.serial.clone();
    match s.retry {
        Some(r) if now < r.next_at => {
            Step::Wait(Some(r.next_at.saturating_duration_since(now)), None)
        }
        _ => Step::Rebind(handle, serial),
    }
}

/// The supervisor thread. See the module's docs.
fn supervise(me: &Weak<DspiCore>, id: u64, rx: &Receiver<Msg>, cancel: &AtomicBool) {
    let mut wait: Option<Duration> = None;
    loop {
        let msg = match wait {
            Some(d) => match rx.recv_timeout(d) {
                Ok(m) => Some(m),
                Err(RecvTimeoutError::Timeout) => None,
                Err(RecvTimeoutError::Disconnected) => return,
            },
            None => match rx.recv() {
                Ok(m) => Some(m),
                Err(_) => return,
            },
        };
        if cancel.load(Ordering::SeqCst) {
            return;
        }
        let Some(core) = me.upgrade() else {
            return;
        };
        if let Some(msg) = msg {
            on_msg(&core, id, msg);
        }
        drop(core);
        match turn(me, id, cancel) {
            Some(next) => wait = next,
            None => return,
        }
    }
}

/// A message to the supervisor.
fn on_msg(core: &DspiCore, id: u64, msg: Msg) {
    match msg {
        Msg::Devices(epoch) => {
            let current = core
                .lock()
                .link
                .as_ref()
                .is_some_and(|s| s.id == id && s.instance == epoch);
            if current {
                core.listeners.link.call(|l| l.on_link_devices_changed());
            }
        }
        Msg::Token { hub_id, token } => core.note_token(hub_id, token),
        Msg::Kick => {}
    }
}

/// The supervisor's work after its messages: the meter feed, then the steps.
/// The time to sleep, or `None` when the session is over.
fn turn(me: &Weak<DspiCore>, id: u64, cancel: &AtomicBool) -> Option<Option<Duration>> {
    let core = me.upgrade()?;
    // -- WP LK2 -- the meter feed first, without the core lock: a start-up
    // read or a re-read may hold it for many seconds
    let meters = (!cancel.load(Ordering::SeqCst))
        .then(|| core.meter_path())
        .flatten();
    drop(core);
    if let Some(path) = meters {
        path.side.maintain_meters();
    }
    run_steps(me, id, cancel)
}

// -- WP WB4b --
/// Turns of the supervisor in one [`DspiCore::link_tick`] at most: each turn's
/// steps may wake it again (a bind kicks it), as the thread would loop.
const TICK_TURNS: usize = 8;

/// The caller-driven side of Link sessions (WP WB4b), for a build without
/// threads (`wasm32-unknown-unknown`): the same supervisor, client and rules,
/// turned by the caller's timer instead of a supervisor thread and the
/// client's reader and keepalive threads. Not exported over UniFFI or the C
/// ABI; the web binding calls these.
impl DspiCore {
    /// [`DspiCore::link_connect`] over `connector`, the platform's WebSocket
    /// (the browser's, on the web).
    pub fn link_connect_over(
        &self,
        connector: Arc<dyn WsConnector>,
        url: String,
        client_name: String,
        app_version: String,
        token: String,
        expected_hub_id: String,
    ) -> Result<LinkHello> {
        self.link_connect_with(
            url,
            client_name,
            app_version,
            token,
            expected_hub_id,
            Tuning {
                connector,
                ..Tuning::default()
            },
        )
    }

    /// Read what the hub sent the session: replies nobody waits for, device
    /// list changes, notifications and meter frames for the bound device, a
    /// ping when due, the end of a silent link. Cheap; call it before
    /// draining notifications or reading meters. A no-op with threads.
    pub fn link_service(&self) {
        let client = guard(&self.link_control)
            .as_ref()
            .filter(|c| c.pumped)
            .and_then(|c| c.client.clone());
        if let Some(c) = client {
            c.service();
        }
    }

    /// One turn of a caller-driven session: [`DspiCore::link_service`], then
    /// the supervisor's messages and steps (reconnect, re-bind, meter
    /// subscriptions). Returns when to call again: the supervisor's own wait,
    /// at most the client's keepalive tick. `None` when there is no
    /// caller-driven session (none, or one with threads).
    pub fn link_tick(&self) -> Option<Duration> {
        let (id, rx, cancel) = {
            let mut control = guard(&self.link_control);
            let c = control.as_mut().filter(|c| c.pumped)?;
            (c.id, c.driver.take()?, Arc::clone(&c.cancel))
        };
        self.link_service();
        let me = self.me.clone();
        let mut wait = None;
        let mut over = false;
        for i in 0..TICK_TURNS {
            let mut woken = false;
            while let Ok(msg) = rx.try_recv() {
                woken = true;
                if cancel.load(Ordering::SeqCst) {
                    break;
                }
                on_msg(self, id, msg);
            }
            if cancel.load(Ordering::SeqCst) {
                over = true;
                break;
            }
            // After the first turn, only what the thread would wake for: a
            // message, or a wait of zero
            if i > 0 && !woken && wait.is_none_or(|w: Duration| !w.is_zero()) {
                break;
            }
            match turn(&me, id, &cancel) {
                Some(next) => wait = next,
                None => {
                    over = true;
                    break;
                }
            }
        }
        let tick = {
            let inner = self.lock();
            inner
                .link
                .as_ref()
                .map_or(Timing::default().keepalive_tick, |s| {
                    s.tuning.timing.keepalive_tick
                })
        };
        let mut control = guard(&self.link_control);
        let mine = control.as_mut().filter(|c| c.id == id);
        match mine {
            Some(c) if !over => {
                c.driver = Some(rx);
                Some(wait.map_or(tick, |w| w.min(tick)))
            }
            // Over: what is left of it goes with the next connect or close.
            Some(c) => {
                c.driver = Some(rx);
                None
            }
            None => None,
        }
    }
}

/// Run steps until there is nothing to do: the time to sleep, or `None` when
/// the session is over.
fn run_steps(me: &Weak<DspiCore>, id: u64, cancel: &AtomicBool) -> Option<Option<Duration>> {
    loop {
        if cancel.load(Ordering::SeqCst) {
            return None;
        }
        let core = me.upgrade()?;
        let (step, ended, changed) = core.plan_step(id, Instant::now());
        if let Some((code, reason)) = ended {
            let code = code.map_or(-1, i32::from);
            core.listeners.link.call(|l| l.on_link_closed(code, reason));
        }
        if changed {
            core.listeners.link.call(|l| l.on_link_status_changed());
        }
        match step {
            Step::Stop => return None,
            Step::Wait(wait, feed) => {
                // A round trip may follow: not with the core in hand.
                drop(core);
                if let Some(feed) = feed {
                    feed.maintain(Instant::now());
                }
                return Some(wait);
            }
            Step::Dial(target) => {
                let tx = core.lock().link.as_ref().map(|s| s.tx.clone())?;
                drop(core);
                let dialled = redial(&target, &tx);
                let Some(core) = me.upgrade() else {
                    if let Ok((client, _)) = dialled {
                        client.close();
                    }
                    return None;
                };
                let ok = dialled.is_ok();
                if !core.finish_dial(id, dialled) {
                    return None;
                }
                core.listeners.link.call(|l| l.on_link_status_changed());
                if ok {
                    core.listeners.link.call(|l| l.on_link_devices_changed());
                }
            }
            Step::Rebind(handle, serial) => core.rebind(id, handle, serial),
            Step::ReadDevices(client) => core.read_devices(id, &client),
        }
    }
}

/// Connect and, if `with_token`, authenticate with the session's token. Once
/// the hub's id is known, the token is looked up after the hello, so another
/// hub at the address never sees it.
fn dial(
    target: &Target,
    with_token: bool,
    tx: &Sender<Msg>,
) -> std::result::Result<(LinkClient, AuthState), LinkError> {
    let token = Some(&target.token).filter(|t| with_token && !t.is_empty());
    let known_hub = !target.hub_id.is_empty();
    let opts = ConnectOptions {
        client_name: target.client_name.clone(),
        app: APP_NAME.into(),
        app_version: target.app_version.clone(),
        token: token.filter(|_| !known_hub).cloned(),
    };
    let lookup = token.filter(|_| known_hub).map(|token| {
        let (hub_id, token) = (target.hub_id.clone(), token.clone());
        let lookup: dspi_link::TokenLookup =
            Arc::new(move |hub: &str| hub.eq_ignore_ascii_case(&hub_id).then(|| token.clone()));
        lookup
    });
    let cfg = SessionConfig {
        timing: target.timing,
        clock: Arc::clone(&target.clock),
        token_for: lookup,
    };
    LinkClient::connect_with(
        target.connector.as_ref(),
        &target.url,
        &opts,
        Some(Arc::new(Forward { tx: tx.clone() })),
        &cfg,
    )
}

/// Whether `url` is `ws://host[:port][/path]`.
fn valid_ws_url(url: &str) -> bool {
    url.strip_prefix("ws://")
        .and_then(|rest| rest.split('/').next())
        .is_some_and(|authority| dspi_link::address::parse_address(authority).is_some())
}

/// Tell the current session's supervisor to look again.
pub(crate) fn kick(core: &DspiCore) {
    if let Some(c) = guard(&core.link_control).as_ref() {
        let _ = c.tx.send(Msg::Kick);
    }
}

/// A dial on a reconnect: the same hub only.
fn redial(
    target: &Target,
    tx: &Sender<Msg>,
) -> std::result::Result<(LinkClient, AuthState), LinkError> {
    let (client, state) = dial(target, true, tx)?;
    if !target.hub_id.is_empty() && !client.hello().hub.id.eq_ignore_ascii_case(&target.hub_id) {
        client.set_events(None);
        client.close();
        return Err(LinkError::Io("another hub answers at this address".into()));
    }
    Ok((client, state))
}

/// A client failure as a typed error, its text ready to show. Spec 7.1 codes
/// pass through.
fn map_error(e: LinkError) -> DspiError {
    let msg = e.to_string();
    let code: &str = match &e {
        LinkError::Io(_) => "io",
        LinkError::Handshake(_) => "handshake",
        LinkError::HubBusy => "hub_busy",
        LinkError::HandshakeRefused { .. } => "handshake_refused",
        LinkError::Protocol(_) => "protocol",
        LinkError::Timeout => "timeout",
        LinkError::Closed { .. } => "closed",
        LinkError::WrongPin { .. } => "wrong_pin",
        LinkError::RateLimited { .. } => "rate_limited",
        LinkError::Rejected { code, .. } => code,
    };
    DspiError::Link {
        code: code.to_owned(),
        msg,
    }
}

fn hello_record(client: &LinkClient, state: &AuthState) -> LinkHello {
    let h = client.hello();
    LinkHello {
        hub_id: h.hub.id.to_lowercase(),
        hub_name: h.hub.name.clone(),
        hub_kind: h.hub.kind.clone(),
        hub_version: h.hub.version.clone().unwrap_or_default(),
        proto_minor: h.proto.minor,
        auth: h.auth.clone(),
        pin_source: h.pin_source().to_owned(),
        needs_pairing: state.needs_pin(),
        token_refused: matches!(state, AuthState::TokenRefused { .. }),
        caps: h.caps.clone(),
        max_payload: h.limits.max_payload,
        max_inflight: h.limits.max_inflight,
    }
}

fn device_record(d: DeviceInfo) -> LinkDevice {
    LinkDevice {
        handle: d.handle,
        serial: d.serial,
        name: d.name,
        fw: d.fw.unwrap_or_default(),
        fw_beta: d.fw_beta.map_or(-1, |b| i32::try_from(b).unwrap_or(-1)),
        platform: d.platform.map_or(-1, |p| i32::try_from(p).unwrap_or(-1)),
        state: d.state,
        link: d.link,
        locked_by: d.locked_by.unwrap_or(0),
    }
}

// ── The side channel ───────────────────────────────────────────────────

/// What a poll feed knows; behind a short lock, never held on the network.
#[derive(Default)]
struct Feed {
    /// The latest frame of each slot and when it came.
    latest: HashMap<u8, (Vec<u8>, Instant)>,
    /// The last `fetch_status`.
    meter_asked: Option<Instant>,
    /// The block size asked for.
    meter_len: u16,
    /// The granted meter rate, while subscribed.
    meter_hz: Option<u32>,
    meter_refused_until: Option<Instant>,
    meter_subscribed_at: Option<Instant>,
    /// A wake-up to the supervisor is on its way.
    kicked: bool,
    /// The channels the analyser wants.
    band_mask: u16,
    /// Channels subscribed, bit per channel.
    band_granted: u16,
    /// The lowest granted band rate, while subscribed.
    band_hz: Option<u32>,
    band_refused_until: Option<Instant>,
    band_subscribed_at: Option<Instant>,
    /// Throttles on a bridged hub.
    last_tunnel_meter: Option<Instant>,
    tunnel_meter_data: Option<Vec<u8>>,
    last_tunnel_bands: Option<Instant>,
    last_bins: Option<Instant>,
}

impl Feed {
    /// The last tunnelled read again: not a fresh reading.
    fn repeat_tunnelled(&self, now: Instant) -> MeterRead {
        MeterRead {
            data: self.tunnel_meter_data.clone(),
            stale: true,
            age_ms: self
                .last_tunnel_meter
                .map_or(0, |t| millis(now.saturating_duration_since(t))),
        }
    }
}

/// A duration in whole ms, saturating.
fn millis(d: Duration) -> u32 {
    u32::try_from(d.as_millis()).unwrap_or(u32::MAX)
}

/// One Link device, reached without the core lock (see the module's docs).
pub(crate) struct LinkSide {
    client: LinkClient,
    handle: u8,
    descriptor: DeviceDescriptor,
    /// The hub has the `poll` capability.
    polls: bool,
    /// The hub is a bridge: every read crosses a UART.
    bridged: bool,
    /// Frames from the client's sink.
    rx: Mutex<Receiver<PollFrame>>,
    feed: Mutex<Feed>,
    /// How many flows hold the hub's lock on this side: it is taken for the
    /// first and released with the last, so two flows cannot release each
    /// other's. Held across the request, which only flows wait for.
    lock_holds: Mutex<u32>,
    /// One tunnelled meter read at a time.
    meter_tunnel: Mutex<()>,
    tx: Sender<Msg>,
}

fn guard<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// How long a frame counts at `hz`: four periods, at least
/// [`FRAME_STALE_MIN`].
fn stale_after(hz: u32) -> Duration {
    Duration::from_millis(4000u64.checked_div(u64::from(hz)).unwrap_or(4000)).max(FRAME_STALE_MIN)
}

/// The rate to ask for `n` band channels: [`BAND_HZ`], within the hub's
/// `poll_max_hz` and half its budget (the budget is shared by all sessions,
/// and the meters want some). A hub that states no budget gets the plain ask;
/// it grants what it can.
fn band_hz(n: u32, poll_max_hz: u32, budget_bps: u32) -> u32 {
    let mut hz = BAND_HZ;
    if poll_max_hz != 0 {
        hz = hz.min(poll_max_hz);
    }
    let cost = n.max(1).saturating_mul(BAND_FRAME_COST);
    if budget_bps != 0 {
        hz = hz.min((budget_bps / 2).checked_div(cost).unwrap_or(hz));
    }
    hz.max(1)
}

/// The poll slot of analyser channel `ch`: 1..=15. The bit of a 16th channel
/// has no slot and is not polled.
fn band_slot(ch: u8) -> Option<u8> {
    (ch < 15).then(|| ch.saturating_add(1))
}

/// The mask bit of channel `ch`, which is below 15.
fn bit(ch: u8) -> u16 {
    1u16.checked_shl(u32::from(ch)).unwrap_or(0)
}

/// The channels of `mask` that have a slot.
fn band_channels(mask: u16) -> impl Iterator<Item = u8> {
    (0u8..15).filter(move |ch| mask & bit(*ch) != 0)
}

impl LinkSide {
    fn new(client: LinkClient, handle: u8, serial: &str, tx: Sender<Msg>) -> Self {
        let rx = client.polls(handle);
        let hello = client.hello();
        let polls = hello.has_cap(CAP_POLL);
        let bridged = hello.hub.kind == "bridge";
        let descriptor = DeviceDescriptor {
            serial: serial.to_owned(),
            bus_id: format!("link:{}", hello.hub.id),
            address: handle,
        };
        Self {
            client,
            handle,
            descriptor,
            polls,
            bridged,
            rx: Mutex::new(rx),
            feed: Mutex::new(Feed::default()),
            lock_holds: Mutex::new(0),
            meter_tunnel: Mutex::new(()),
            tx,
        }
    }

    /// Move what arrived into `feed`.
    fn drain(&self, feed: &mut Feed, now: Instant) {
        for f in guard(&self.rx).try_iter() {
            // An empty frame is a failed read on the hub, not a reading.
            if !f.payload.is_empty() {
                feed.latest.insert(f.slot, (f.payload, now));
            }
        }
    }

    /// One tunnelled GET; its reply may be shorter than `len`.
    fn tunnel_get(&self, opcode: u8, value: u16, len: u16) -> dspi_transport::Result<Vec<u8>> {
        let r = self.client.command(
            self.handle,
            Dir::Get,
            opcode,
            value,
            dspi_proto::VENDOR_INTERFACE,
            len,
            &[],
        );
        match map_status(r.status, opcode) {
            Some(e) => Err(e),
            None => Ok(r.payload),
        }
    }

    /// The supervisor's look at the meter feed: subscribe when asked for,
    /// unsubscribe when nobody asks any more, subscribe again when it went
    /// silent or a refusal expired. May do a round trip.
    fn maintain(&self, now: Instant) {
        if !self.polls || self.client.is_closed() {
            return;
        }
        let spec = {
            let mut feed = guard(&self.feed);
            feed.kicked = false;
            self.drain(&mut feed, now);
            let asked = feed
                .meter_asked
                .is_some_and(|t| now.saturating_duration_since(t) < METER_IDLE);
            if !asked {
                feed.meter_asked = None;
                if feed.meter_hz.take().is_some() {
                    feed.latest.remove(&METER_SLOT);
                    drop(feed);
                    self.client.poll_unsubscribe(self.handle, &[METER_SLOT]);
                }
                return;
            }
            if feed.meter_refused_until.is_some_and(|t| now < t) {
                return;
            }
            let silent = feed.meter_hz.is_some()
                && feed
                    .latest
                    .get(&METER_SLOT)
                    .map(|f| f.1)
                    .max(feed.meter_subscribed_at)
                    .is_none_or(|t| now.saturating_duration_since(t) >= FEED_LOST);
            if feed.meter_hz.is_some() && !silent {
                return;
            }
            feed.meter_subscribed_at = Some(now);
            self.meter_spec(feed.meter_len)
        };
        let granted = self.client.poll_subscribe(self.handle, &[spec]);
        let mut feed = guard(&self.feed);
        match granted {
            Ok(g) => match g.iter().find(|g| g.slot == METER_SLOT && g.hz > 0) {
                Some(g) => {
                    feed.meter_hz = Some(g.hz);
                    feed.meter_refused_until = None;
                }
                None => {
                    feed.meter_hz = None;
                    feed.meter_refused_until = now.checked_add(POLL_RETRY_AFTER);
                }
            },
            Err(e) => {
                log::info!("the hub did not grant the meter poll: {e}");
                feed.meter_hz = None;
                feed.meter_refused_until = now.checked_add(POLL_RETRY_AFTER);
            }
        }
    }

    // -- WP LK4 --
    /// Ask for the meter block of `len` bytes now, as a first `fetch_status`
    /// would, so the poll is subscribed before the app's first look: here
    /// with `now` (one round trip), else by a word to the supervisor.
    fn want_meters(&self, len: u16, now: bool) {
        {
            let mut feed = guard(&self.feed);
            feed.meter_len = len;
            feed.meter_asked = Some(Instant::now());
            if !now && !std::mem::replace(&mut feed.kicked, true) {
                let _ = self.tx.send(Msg::Kick);
            }
        }
        if now {
            self.maintain(Instant::now());
        }
    }

    fn meter_spec(&self, len: u16) -> PollSpec {
        let wanted = if self.bridged {
            METER_HZ_BRIDGED
        } else {
            METER_HZ
        };
        let max = self.client.hello().limits.poll_max_hz;
        PollSpec {
            slot: METER_SLOT,
            req: op::REQ_GET_STATUS,
            val: STATUS_METERS,
            idx: dspi_proto::VENDOR_INTERFACE,
            len,
            hz: if max == 0 { wanted } else { wanted.min(max) },
        }
    }

    /// The meter block by a tunnelled read, when the hub refuses polls: at a
    /// low rate, one at a time, the last one otherwise.
    fn meters_tunnel(&self, len: u16, now: Instant) -> Result<MeterRead> {
        let every = if self.bridged {
            METER_TUNNEL_EVERY_BRIDGED
        } else {
            METER_TUNNEL_EVERY
        };
        {
            let feed = guard(&self.feed);
            if feed
                .last_tunnel_meter
                .is_some_and(|t| now.saturating_duration_since(t) < every)
            {
                return Ok(feed.repeat_tunnelled(now));
            }
        }
        let Ok(_one) = self.meter_tunnel.try_lock() else {
            return Ok(guard(&self.feed).repeat_tunnelled(now));
        };
        guard(&self.feed).last_tunnel_meter = Some(now);
        let data = self.tunnel_get(op::REQ_GET_STATUS, STATUS_METERS, len)?;
        guard(&self.feed).tunnel_meter_data = Some(data.clone());
        Ok(MeterRead {
            data: Some(data),
            stale: false,
            age_ms: 0,
        })
    }

    /// Bring the band subscription in line with `mask`, on the analyser's
    /// thread. A round trip only when something changed.
    fn sync_bands(&self, mask: u16, now: Instant) {
        if !self.polls || self.client.is_closed() {
            return;
        }
        let mask = band_channels(mask).fold(0u16, |m, ch| m | (bit(ch)));
        let (stale_slots, specs) = {
            let mut feed = guard(&self.feed);
            self.drain(&mut feed, now);
            let silent = feed.band_granted != 0
                && band_channels(feed.band_granted)
                    .filter_map(band_slot)
                    .filter_map(|s| feed.latest.get(&s).map(|f| f.1))
                    .max()
                    .max(feed.band_subscribed_at)
                    .is_none_or(|t| now.saturating_duration_since(t) >= FEED_LOST);
            let unchanged = feed.band_mask == mask && !silent;
            let waiting = feed.band_refused_until.is_some_and(|t| now < t);
            if (unchanged && (feed.band_granted == mask || waiting)) || (mask != 0 && waiting) {
                return;
            }
            let stale_slots: Vec<u8> = band_channels(feed.band_granted & !mask)
                .filter_map(band_slot)
                .collect();
            for slot in &stale_slots {
                feed.latest.remove(slot);
            }
            feed.band_mask = mask;
            feed.band_granted &= mask;
            feed.band_subscribed_at = Some(now);
            let limits = self.client.hello().limits;
            let hz = band_hz(
                mask.count_ones(),
                limits.poll_max_hz,
                limits.poll_budget_bps,
            );
            let len = u16::try_from(RtaBandFrame::SIZE).unwrap_or(u16::MAX);
            let specs: Vec<PollSpec> = band_channels(mask)
                .filter_map(|ch| {
                    Some(PollSpec {
                        slot: band_slot(ch)?,
                        req: op::REQ_RTA_GET_BANDS,
                        val: u16::from(ch),
                        idx: dspi_proto::VENDOR_INTERFACE,
                        len,
                        hz,
                    })
                })
                .collect();
            (stale_slots, specs)
        };
        if !stale_slots.is_empty() {
            self.client.poll_unsubscribe(self.handle, &stale_slots);
        }
        if specs.is_empty() {
            let mut feed = guard(&self.feed);
            feed.band_hz = None;
            feed.band_refused_until = None;
            return;
        }
        let granted = self.client.poll_subscribe(self.handle, &specs);
        let mut feed = guard(&self.feed);
        match granted {
            Ok(grants) => {
                let mut bits = 0u16;
                let mut lowest = None;
                for ch in band_channels(mask) {
                    let hit = band_slot(ch)
                        .and_then(|slot| grants.iter().find(|g| g.slot == slot && g.hz > 0));
                    if let Some(g) = hit {
                        bits |= bit(ch);
                        lowest = Some(lowest.map_or(g.hz, |l: u32| l.min(g.hz)));
                    }
                }
                feed.band_granted = bits;
                feed.band_hz = lowest;
                feed.band_refused_until = (bits != mask)
                    .then(|| now.checked_add(POLL_RETRY_AFTER))
                    .flatten();
            }
            Err(e) => {
                log::info!("the hub did not grant the band polls: {e}");
                feed.band_hz = None;
                feed.band_refused_until = now.checked_add(POLL_RETRY_AFTER);
            }
        }
    }
}

impl SideChannel for LinkSide {
    fn lock(&self, reason: &str, timeout: Duration) -> Result<()> {
        if !self.client.hello().has_cap(CAP_LOCK) {
            return Ok(());
        }
        let mut holds = guard(&self.lock_holds);
        if *holds == 0 {
            let ms = u32::try_from(timeout.as_millis())
                .unwrap_or(LOCK_MAX_MS)
                .min(LOCK_MAX_MS);
            self.client
                .lock_acquire(self.handle, reason, ms)
                .map_err(map_lock_error)?;
        }
        *holds = holds.saturating_add(1);
        Ok(())
    }

    fn unlock(&self) {
        let mut holds = guard(&self.lock_holds);
        *holds = holds.saturating_sub(1);
        if *holds == 0 && self.client.hello().has_cap(CAP_LOCK) {
            self.client.lock_release(self.handle);
        }
    }

    fn rta_transport(self: Arc<Self>, rta_mask: u16) -> Box<dyn Transport> {
        let now = Instant::now();
        self.sync_bands(rta_mask, now);
        let mut feed = guard(&self.feed);
        let tunnel_bands = !self.bridged
            || feed
                .last_tunnel_bands
                .is_none_or(|t| now.saturating_duration_since(t) >= BRIDGED_BANDS_EVERY);
        if tunnel_bands && feed.band_granted == 0 {
            feed.last_tunnel_bands = Some(now);
        }
        let bins = !self.bridged
            || feed
                .last_bins
                .is_none_or(|t| now.saturating_duration_since(t) >= BRIDGED_BINS_EVERY);
        drop(feed);
        Box::new(RtaLinkTransport {
            side: self,
            tunnel_bands,
            bins,
        })
    }

    fn maintain_meters(&self) {
        self.maintain(Instant::now());
    }

    fn meters(&self, len: u16) -> Result<MeterRead> {
        if self.client.is_closed() {
            return Err(DspiError::Disconnected);
        }
        let now = Instant::now();
        let mut feed = guard(&self.feed);
        self.drain(&mut feed, now);
        feed.meter_asked = Some(now);
        if feed.meter_len != len {
            feed.meter_len = len;
            feed.meter_hz = None;
            feed.latest.remove(&METER_SLOT);
        }
        let refused = feed.meter_refused_until.is_some_and(|t| now < t);
        if !self.polls || (refused && feed.meter_hz.is_none()) {
            drop(feed);
            return self.meters_tunnel(len, now);
        }
        let Some(hz) = feed.meter_hz else {
            if !std::mem::replace(&mut feed.kicked, true) {
                let _ = self.tx.send(Msg::Kick);
            }
            // Between a refusal's expiry and the next answer: the last read.
            return Ok(feed.repeat_tunnelled(now));
        };
        let Some((data, at)) = feed.latest.get(&METER_SLOT) else {
            return Ok(MeterRead {
                data: None,
                stale: true,
                age_ms: 0,
            });
        };
        let age = now.saturating_duration_since(*at);
        Ok(MeterRead {
            data: (age < stale_after(hz)).then(|| data.clone()),
            stale: age >= stale_after(hz),
            age_ms: millis(age),
        })
    }
}

impl Drop for LinkSide {
    fn drop(&mut self) {
        if self.client.is_closed() {
            return;
        }
        let feed = guard(&self.feed);
        let mut slots: Vec<u8> = band_channels(feed.band_granted)
            .filter_map(band_slot)
            .collect();
        if feed.meter_hz.is_some() {
            slots.push(METER_SLOT);
        }
        if !slots.is_empty() {
            self.client.poll_unsubscribe(self.handle, &slots);
        }
    }
}

/// One analyser poll's view of a Link device (see the module's docs): band
/// frames from the feed, everything else tunnelled, one try each. A failed
/// poll is simply the next one's to redo.
struct RtaLinkTransport {
    side: Arc<LinkSide>,
    /// Tunnelled band reads are allowed in this poll.
    tunnel_bands: bool,
    /// A new bin frame may be started in this poll.
    bins: bool,
}

impl RtaLinkTransport {
    fn band(&self, ch: u16, len: u16) -> dspi_transport::Result<Vec<u8>> {
        let opcode = op::REQ_RTA_GET_BANDS;
        let polled = u8::try_from(ch).ok().filter(|c| {
            guard(&self.side.feed).band_granted & bit(*c) != 0 && band_slot(*c).is_some()
        });
        if let Some(c) = polled {
            let frame = band_slot(c).and_then(|slot| self.side.cached_band(slot));
            // Not yet: the next poll's. Never a tunnelled read behind the feed.
            return frame.ok_or(TransportError::TimedOut { opcode, retries: 0 });
        }
        if !self.tunnel_bands {
            return Err(TransportError::Stalled { opcode });
        }
        self.side.tunnel_get(opcode, ch, len)
    }

    /// All polled channels' frames, back to back as `0x0F` answers.
    fn bands_all(&self) -> Vec<u8> {
        let granted = guard(&self.side.feed).band_granted;
        band_channels(granted)
            .filter_map(band_slot)
            .filter_map(|slot| self.side.cached_band(slot))
            .flatten()
            .collect()
    }
}

impl LinkSide {
    /// The latest frame of band `slot`, if recent.
    fn cached_band(&self, slot: u8) -> Option<Vec<u8>> {
        let now = Instant::now();
        let feed = guard(&self.feed);
        let hz = feed.band_hz.unwrap_or(1);
        feed.latest
            .get(&slot)
            .filter(|(_, at)| now.saturating_duration_since(*at) < stale_after(hz))
            .map(|(data, _)| data.clone())
    }
}

impl Transport for RtaLinkTransport {
    fn control_in(&mut self, opcode: u8, value: u16, len: u16) -> dspi_transport::Result<Vec<u8>> {
        let data = if opcode == op::REQ_RTA_GET_BANDS {
            self.band(value, len)?
        } else {
            self.side.tunnel_get(opcode, value, len)?
        };
        if data.len() < usize::from(len) {
            return Err(TransportError::ShortRead {
                opcode,
                wanted: usize::from(len),
                got: data.len(),
            });
        }
        Ok(data)
    }

    fn control_in_upto(
        &mut self,
        opcode: u8,
        value: u16,
        max_len: u16,
    ) -> dspi_transport::Result<Vec<u8>> {
        if opcode == op::REQ_RTA_GET_BANDS_ALL {
            let polled = guard(&self.side.feed).band_granted != 0;
            if polled {
                return Ok(self.bands_all());
            }
            if !self.tunnel_bands {
                return Err(TransportError::Stalled { opcode });
            }
        }
        if opcode == op::REQ_RTA_GET_BINS && value == 0 {
            if !self.bins {
                return Err(TransportError::Stalled { opcode });
            }
            guard(&self.side.feed).last_bins = Some(Instant::now());
        }
        self.side.tunnel_get(opcode, value, max_len)
    }

    fn control_out(&mut self, opcode: u8, value: u16, data: &[u8]) -> dspi_transport::Result<()> {
        let Ok(len) = u16::try_from(data.len()) else {
            return Err(TransportError::Remote(format!(
                "{} bytes for opcode 0x{opcode:02X} exceed a vendor request",
                data.len()
            )));
        };
        let r = self.side.client.command(
            self.side.handle,
            Dir::Set,
            opcode,
            value,
            dspi_proto::VENDOR_INTERFACE,
            len,
            data,
        );
        if opcode == op::REQ_RTA_SET_CONFIG {
            // Frames of the old tap or channels must not pass for the new.
            let mut feed = guard(&self.side.feed);
            for ch in 0u8..15 {
                if let Some(slot) = band_slot(ch) {
                    feed.latest.remove(&slot);
                }
            }
        }
        match map_status(r.status, opcode) {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }

    fn descriptor(&self) -> &DeviceDescriptor {
        &self.side.descriptor
    }

    fn max_transfer(&self) -> usize {
        usize::try_from(self.side.client.hello().limits.max_payload)
            .map_or(8192, |n| n.clamp(64, 65_535))
    }

    fn caps(&self) -> TransportCaps {
        TransportCaps {
            kind: TransportKind::Link,
            chunked_bulk: false,
            remote: true,
        }
    }

    fn session_id(&self) -> Option<u16> {
        self.side.client.session_id()
    }
}

fn map_lock_error(e: LinkError) -> DspiError {
    match e {
        LinkError::Rejected { code, .. } if code == "locked" => DspiError::Locked,
        LinkError::Rejected { code, .. } if code == "denied" => DspiError::Denied,
        LinkError::Closed { .. } => DspiError::Disconnected,
        other => map_error(other),
    }
}

/// Parse a hub address typed by the user (`host`, `host:port`, `[v6]:port`).
///
/// Errors: [`DspiError::InvalidArgument`] when it cannot be an address.
#[cfg_attr(feature = "uniffi", uniffi::export)]
pub fn link_parse_address(text: String) -> Result<LinkAddress> {
    dspi_link::address::parse_address(&text)
        .map(|(host, port)| LinkAddress { host, port })
        .ok_or_else(|| DspiError::InvalidArgument {
            msg: format!("not a hub address: {text}"),
        })
}

/// The WebSocket URL for a hub. An empty `path` is the default (`/dspi/v1`).
#[cfg_attr(feature = "uniffi", uniffi::export)]
pub fn link_ws_url(host: String, port: u16, path: String) -> String {
    dspi_link::address::ws_url(&host, port, Some(path.as_str()).filter(|p| !p.is_empty()))
}

/// `GET /dspi/v1/info` (spec 3.2). Blocks up to a few seconds.
///
/// Errors: [`DspiError::Link`] with `io` or `protocol`.
#[cfg_attr(feature = "uniffi", uniffi::export)]
pub fn link_fetch_info(host: String, port: u16) -> Result<LinkInfo> {
    let doc = dspi_link::address::fetch_info(&host, port, INFO_TIMEOUT).map_err(map_error)?;
    Ok(LinkInfo {
        hub_id: doc.hub.id.to_lowercase(),
        hub_name: doc.hub.name,
        hub_kind: doc.hub.kind,
        auth: doc.auth,
        proto_minor: doc.proto.minor,
        ws_path: doc.ws.unwrap_or_else(|| dspi_link::DEFAULT_PATH.to_owned()),
        serials: doc.devices.into_iter().map(|d| d.serial).collect(),
    })
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
#[path = "link/tests_wp_c15.rs"]
mod tests_wp_c15;

// -- WP CG21 --
#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
#[path = "link/tests_wp_cg21.rs"]
mod tests_wp_cg21;

// -- WP CG24 --
#[cfg(test)]
#[path = "link/tests_wp_cg24.rs"]
mod tests_wp_cg24;

// -- WP LK2 --
#[cfg(test)]
#[path = "link/tests_wp_lk2.rs"]
mod tests_wp_lk2;

// -- WP WB4b --
#[cfg(test)]
#[path = "link/tests_wp_wb4b.rs"]
mod tests_wp_wb4b;
