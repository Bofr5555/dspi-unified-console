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
// -- WP SECCORE --
use dspi_link::TlsOptions;
use dspi_link::credential::{Credential, Pin};
use dspi_proto::generated::opcodes as op;
use dspi_proto::packets::RtaBandFrame;
use dspi_transport::link::map_status;
// -- WP P1HL --
use dspi_transport::link::map_response;
use dspi_transport::{
    DeviceDescriptor, LinkTransport, Transport, TransportCaps, TransportError, TransportKind,
};
// -- WP PGCORE --
use dspi_proto::paged::legacy::{VC_ERR_PIN_IN_USE as VC_ERR_LAST, VC_ERR_UNKNOWN};
use dspi_proto::paged::{Answer, LegacyBody, Next, Outcome, PageSet};
use dspi_transport::link::{PAGED_REFUSES_PAGE0, firmware_refusal, is_retryable_13, paged_poll};
use dspi_transport::paged::Discovery;
use dspi_transport::{CmdId, PagedTransport};

use crate::callbacks::LinkListener;
// -- WP LKUI1 --
use crate::records::*;
use crate::{DspiCore, DspiError, MeterRead, Result, SideChannel};
pub use alink_system::{
    AlinkRoomLipSync, AlinkSceneRecall, AlinkSystemAlarm, AlinkSystemAlign, AlinkSystemEdits,
    AlinkSystemRoom, AlinkSystemScene, AlinkSystemUnit, AlinkSystemView, AlinkSystemWrite,
    AlinkUnitLatency,
};
// -- WP AUSB1 --
pub use alink_system::AlinkUnitUsbMode;
// -- WP P1QT1 --
pub use alink_system::AlinkSystemSaved;
// -- WP ATQT --
pub use alink_system_spdif::{AlinkSpdifEdit, AlinkSpdifUnit};
// -- WP INST0 --
pub use alink_installation::{
    InstallationDraft, InstallationEdit, InstallationIssue, InstallationRole, InstallationSpeaker,
    InstallationUnseen, InstallationView, installation_apply, installation_changes,
};
// -- WP FEAT1 --
pub use alink_installation::{InstallationProgrammeKind, InstallationSceneAction};
use dspi_session::ext_state::ExtState;
// -- WP FEAT2 --
pub use system_volume::{
    SYSTEM_VOLUME_MASTER_MIN_DB, SYSTEM_VOLUME_STEP_MAX_DB, SYSTEM_VOLUME_USER_MIN_DB,
    SystemVolumeAsk, SystemVolumeKind, SystemVolumePlan, SystemVolumeState, SystemVolumeUnit,
};

/// The role asked for when pairing, as the macOS Console does.
const PAIR_ROLE: &str = "control";
const APP_NAME: &str = "uConsole for DSPi";
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
// -- WP LK4 --
/// The meter rate on a bridge after its feed went silent (a stall).
const METER_HZ_STALLED: u32 = 2;
/// How long frames must flow again before the full rate comes back.
const METER_SLOW_FOR: Duration = Duration::from_secs(10);
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
    // -- WP HUBADMINF -- F1
    /// The client with this epoch ended: what it learnt of the hub goes
    /// (the cached role), then as [`Msg::Kick`].
    Closed(u64),
    // -- WP HUBADMIN --
    /// The hub's limits, lock or pinned modes changed (`limits.changed`,
    /// `limits.held`, `mode.changed`) on the client with this epoch.
    Hub {
        epoch: u64,
        event: Box<crate::hub_admin::HubAdminEvent>,
    },
    // -- WP P2CC1 --
    /// The hub's `event.added`, `inventory.changed` or `snapshot.changed` on
    /// the client with this epoch.
    HubServer {
        epoch: u64,
        event: Box<crate::hub_server::HubServerEvent>,
    },
    // -- WP ATCORE --
    /// The hub aborted a system measurement on its own
    /// (`system.measure.aborted`) on the client with this epoch.
    MeasureAborted {
        epoch: u64,
        event: dspi_measure::system::messages::AbortedEvent,
    },
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

    fn session_closed(&self, epoch: u64, _code: Option<u16>, _reason: &str) {
        // -- WP HUBADMINF -- F1: the role the hub last answered is that session's
        let _ = self.tx.send(Msg::Closed(epoch));
    }

    fn token_issued(&self, hub_id: &str, token: &str) {
        let _ = self.tx.send(Msg::Token {
            hub_id: hub_id.to_lowercase(),
            token: token.to_owned(),
        });
    }

    // -- WP HUBADMIN -- the hub's admin events, to the supervisor (never here:
    // this is the reader, and the lock's refresh is a round trip)
    fn hub_event(&self, epoch: u64, t: &str, body: &serde_json::Value) {
        if let Some(event) = crate::hub_admin::HubAdminEvent::from_hub(t, body) {
            let _ = self.tx.send(Msg::Hub {
                epoch,
                event: Box::new(event),
            });
        // -- WP P2CC1 --
        } else if let Some(event) = crate::hub_server::HubServerEvent::from_hub(t, body) {
            let _ = self.tx.send(Msg::HubServer {
                epoch,
                event: Box::new(event),
            });
        // -- WP ATCORE --
        } else if t == dspi_measure::system::messages::t::ABORTED
            && let Ok(event) = serde_json::from_value(body.clone())
        {
            let _ = self.tx.send(Msg::MeasureAborted { epoch, event });
        }
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
    // -- WP SECCORE --
    /// The key the hub must present (PLAN-SEC 4.3 rule 1), and the successor it
    /// announced; `None`: no pin yet (a v1 credential, or first contact).
    pin: Option<Pin>,
    pin_next: Option<Pin>,
    /// The hub's advert said TLS (ratchet (a)).
    tls_advertised: bool,
    /// An unpinned hub that refused TLS earlier in this session: reconnects go
    /// straight to plaintext.
    plaintext_known: bool,
    // -- WP PINTLS --
    /// The stored record says the hub was pinned before (Pair Again): TLS
    /// only, `tls_required_pinned` when it fails; the pairing pins the new key.
    pinned_before: bool,
    // -- WP LKFAST --
    /// Where a hub met without TLS is remembered across sessions.
    memory: Arc<hub_memory::Store>,
}

// -- WP SECCORE --
impl Target {
    /// The credential to store: `v2` once pinned, else the bare token (`v1`).
    /// Empty without a token.
    fn credential(&self) -> String {
        if self.token.is_empty() {
            return String::new();
        }
        Credential {
            token: self.token.clone(),
            pin: self.pin,
            next: self.pin_next,
            ..Credential::default()
        }
        .encode()
    }

    /// What the client is told about TLS.
    fn tls_options(&self) -> TlsOptions {
        // -- WP LKFAST3 -- (review LKREV 1) a hub that had TLS is TLS only, as
        // one whose advert says so: no plaintext shortcut, no fallback
        let had_tls = self.memory.tls_seen(&self.hub_id);
        TlsOptions {
            pin: self.pin,
            pin_next: self.pin_next,
            advertised: self.tls_advertised || had_tls,
            plaintext_known: self.plaintext_known && !had_tls,
            pinned_before: self.pinned_before, // -- WP PINTLS --
        }
    }

    /// Takes in what `client`'s connection met: a key is pinned only with a
    /// token or a pairing (rule 2), a pinned key moves only as the client
    /// judged it (a proven rotation). Returns whether the credential changed.
    fn learn(&mut self, client: &LinkClient, state: &AuthState) -> bool {
        let Some(seen) = client.tls() else {
            if self.pin.is_none() && !self.tls_advertised && !self.pinned_before {
                // -- WP LKFAST -- and across sessions: no TLS try first next time
                // (the count says when to try again); `tried`: it was refused now
                self.memory
                    .met_plaintext(&client.hello().hub.id, !self.plaintext_known);
                self.plaintext_known = true;
            }
            return false;
        };
        self.plaintext_known = false;
        self.memory.saw_tls(&client.hello().hub.id); // -- WP LKFAST3 -- met encrypted: TLS only from now on
        let keep = self.pin.is_some() || !self.token.is_empty() || state.needs_pin();
        if !keep {
            return false;
        }
        let before = (self.pin, self.pin_next);
        self.pin = Some(seen.key);
        self.pin_next = seen.next;
        before != (self.pin, self.pin_next)
    }
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
    // -- WP LK4 --
    /// The last probe dial past a silent bridge session ([`PROBE_AFTER`]).
    probed_at: Option<Instant>,
    /// The last try to complete the bound device's probe.
    completed_at: Option<Instant>,
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
    // -- WP LKUI1 --
    /// The audio link installation as last read (`system.rs`).
    system: alink_system::Cache,
    // -- WP SECCORE --
    /// The last connection set a new pin (first use or a rotation): show the
    /// fingerprint once.
    key_new: bool,
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
    // -- WP MDQT1 --
    /// The hub this session is with: the id it named, else the one expected.
    fn hub_id(&self) -> &str {
        match &self.client {
            Some(c) if !c.hello().hub.id.is_empty() => &c.hello().hub.id,
            _ => &self.target.hub_id,
        }
    }

    /// [`DspiCore::link_status`]'s phase is [`LinkPhase::Reconnecting`].
    #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
    pub(crate) fn reconnecting(&self, conn_gen: Option<u64>) -> bool {
        !self.cancel.load(Ordering::SeqCst) && self.phase(conn_gen) == LinkPhase::Reconnecting
    }

    /// The unit bound through it (its serial), with the hub's id and address:
    /// what a [`crate::CoreSet`] keeps the device under.
    #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
    pub(crate) fn bound_unit(&self) -> Option<(&str, &str, &str)> {
        let b = self.bound.as_ref().filter(|b| !b.opening)?;
        Some((self.hub_id(), &self.target.url, &b.serial))
    }

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
                // -- WP SECCORE -- a changed key ends as a refusal: pair again
                self.refused = matches!(
                    error,
                    Some(
                        LinkError::HandshakeRefused { .. }
                            | LinkError::HubKeyChanged
                            | LinkError::HubKeyUnverified
                            | LinkError::TlsRequired
                    )
                );
                self.unreported_end = Some((None, reason));
            }
        }
    }

    // -- WP HUBFLAKE3 --
    /// `link_pair` got `token` from this session's client. The hub can revoke between
    /// its answer and this: [`Self::note_loss`] has then cleared the token and asked
    /// for a person, which this must not undo (it dialled again with the revoked
    /// token). A loss not judged yet is judged after this, by its close code.
    fn paired(&mut self, token: &str) {
        let revoked = self.loss_seen
            && self
                .last_close
                .as_ref()
                .is_some_and(|(code, _)| *code == Some(4002));
        if revoked {
            return;
        }
        self.needs_pin = false;
        self.last_close = None;
        if !token.is_empty() {
            self.token_refused = false;
            token.clone_into(&mut self.target.token);
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
        // -- WP SECBFIXA -- not after 4003 `tls_required` either
        if dspi_link::should_reconnect_after(code, &reason) {
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
    /// -- WP LK4 -- dial past a silent bridge session ([`PROBE_AFTER`]).
    Probe(Box<Target>),
    /// -- WP LK4 -- complete the bound device's probe ([`DspiCore::ensure_probed`]).
    Complete,
    /// Read a device list that failed after a pairing.
    ReadDevices(LinkClient),
}

// -- WP LK4 --
/// A bridge closes a session whose queued frames it could not write for 10 s
/// (bridge spec 8.2, `LINK_UNWRITABLE_TIMEOUT_MS`); with a poll or its own
/// pings running it always has some. Silence past this on a bridge means that
/// close is likely, and the reset that would say so reaches us only when our
/// own TCP retries again, which a long outage backs off by many seconds. So
/// from then on a new session is dialled every [`PROBE_EVERY`]: if one
/// connects while the old one is still silent, the network works and the old
/// session is dead; the new one replaces it at once. Accepted by the owner
/// (2026-11-08) as the one exception to the 30 s silence rule
/// (`dspi_link::client::Timing::silence_limit`).
const PROBE_AFTER: Duration = Duration::from_secs(12);
const PROBE_EVERY: Duration = Duration::from_secs(2);
/// How long a select that lost its session waits for the next one.
const SELECT_AGAIN_WITHIN: Duration = Duration::from_secs(10);
/// After a failed try, the bound device's probe is completed again this much
/// later.
const COMPLETE_AGAIN: Duration = Duration::from_secs(2);

/// Whether a probe dial past session `s`'s silent client is due.
fn probe_due(s: &LinkSession, client: &LinkClient, now: Instant) -> bool {
    client.hello().hub.kind == "bridge"
        && client.silent_for() >= PROBE_AFTER
        && s.probed_at
            .is_none_or(|t| now.saturating_duration_since(t) >= PROBE_EVERY)
}

/// A session number; never reused.
static NEXT_SESSION: AtomicU64 = AtomicU64::new(1);
#[cfg_attr(feature = "uniffi", uniffi::export)]
impl DspiCore {
    /// Open a DSPi Link session to `url` (`link_ws_url`), replacing any
    /// previous one. `token` = the token stored for this hub's id ("" =
    /// none). `expected_hub_id` = that id. With it, a token goes out only after
    /// the hello has named that hub, and another hub at the address fails the
    /// call with `wrong_hub`, having seen no token. "" = not known (a hub
    /// added by address, no stored id): dial with no token, read the hub's id
    /// from the hello, and dial again with that hub's token and id. -- WP
    /// LINKFIX -- A bare (`v1`) token with no expected id is never sent: the
    /// call fails with `token_without_hub_id` before anything is dialled
    /// (pair again). A `v2` credential's key pin still guards it.
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
    /// `timeout`, `closed`, `token_without_hub_id` (above), or the hub's own.
    ///
    /// -- WP SECCORE -- `token` is the stored **credential** (PLAN-SEC 4.3): a
    /// bare token (`v1`) or a `v2` string with the hub's key pin, as
    /// `link_pair`, [`LinkListener::on_link_token_issued`] and
    /// `link_security().credential` give it; store those whenever they change.
    /// TLS is tried first. A pinned hub is reached over TLS only, with its key
    /// checked before the token goes out: `hub_key_changed` when it is another
    /// key (pair again), `hub_key_unverified` for a pairing link's pin,
    /// `tls_refused` when TLS fails; never plaintext. -- WP PINTLS -- A record
    /// from [`link_credential_unpaired`] (a hub pinned before, after Pair
    /// Again) is TLS only too, any key, pinned by the pairing:
    /// `tls_required_pinned` when TLS fails, before any PIN goes out. An unpinned hub without
    /// TLS (an upstream hub) connects unencrypted, as before
    /// (`link_security().encrypted` false). Same as
    /// [`DspiCore::link_connect_advertised`] with no advert.
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

    // -- WP SECCORE --
    /// [`DspiCore::link_connect`] with what the hub's DNS-SD advert said:
    /// `tls_advertised` (the browsed record's `tls_advertised`, spec 3.1
    /// `tls=1`): TLS is then required even without a pin, and its failure is
    /// `tls_refused`, never a plaintext retry (ratchet (a)). A pinned hub needs
    /// TLS whatever the advert says.
    pub fn link_connect_advertised(
        &self,
        url: String,
        client_name: String,
        app_version: String,
        credential: String,
        expected_hub_id: String,
        tls_advertised: bool,
    ) -> Result<LinkHello> {
        self.link_connect_hinted(
            url,
            client_name,
            app_version,
            credential,
            expected_hub_id,
            Advert {
                tls: tls_advertised,
                ..Advert::default()
            },
            Tuning::default(),
        )
    }

    // -- WP LKFAST3 -- (review LKREV 6)
    /// [`DspiCore::link_connect_advertised`] for the dial at app start to the
    /// hub used last ([`DspiCore::link_start_hub`]): the same rules, but its
    /// first dial gives up after [`START_DIAL_BUDGET`] instead of a minute, so
    /// a hub that is not at its stored address hands over to the client's
    /// search quickly. A dial the user asked for keeps the long budget.
    pub fn link_connect_at_start(
        &self,
        url: String,
        client_name: String,
        app_version: String,
        credential: String,
        expected_hub_id: String,
        tls_advertised: bool,
    ) -> Result<LinkHello> {
        self.link_connect_hinted(
            url,
            client_name,
            app_version,
            credential,
            expected_hub_id,
            Advert {
                tls: tls_advertised,
                first_dial: Some(START_DIAL_BUDGET),
            },
            Tuning::default(),
        )
    }

    // -- WP SECCORE --
    /// The current session's encryption, for the hub's status and settings.
    pub fn link_security(&self) -> LinkSecurity {
        let inner = self.lock();
        let Some(s) = inner.link.as_ref() else {
            return LinkSecurity::default();
        };
        let seen = s.client.as_ref().and_then(LinkClient::tls);
        LinkSecurity {
            encrypted: seen.is_some(),
            // -- WP SECFIX --
            local: seen.is_none() && dspi_link::address::is_loopback_url(&s.target.url),
            hub_key: seen
                .map(|t| t.key)
                .or(s.target.pin)
                .map(|k| dspi_link::credential::pin_display(&k))
                .unwrap_or_default(),
            hub_key_new: s.key_new,
            credential: s.target.credential(),
        }
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
            s.paired(&token);
            let _ = s.tx.send(Msg::Kick);
            // -- WP SECCORE -- the credential: pinned when the pairing ran over TLS
            if !token.is_empty() {
                return Ok(Credential {
                    token,
                    pin: s.target.pin,
                    next: s.target.pin_next,
                    ..Credential::default()
                }
                .encode());
            }
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
        // -- WP MDQT1 -- (review RA) one session per serial: not a unit another
        // core of this core's set holds
        let hub_id = self
            .lock()
            .link
            .as_ref()
            .map(|s| s.hub_id().to_owned())
            .unwrap_or_default();
        self.claim(&format!("hub:{hub_id}/{serial}"), &serial)?;
        // Dropped with the lock released: a side's drop writes to the socket.
        let _replaced = {
            let mut inner = self.lock();
            let ready = inner
                .link
                .as_mut()
                .filter(|s| s.client.as_ref().is_some_and(LinkClient::is_ready));
            let Some(s) = ready else {
                drop(inner);
                self.unclaim(&serial);
                return Err(DspiError::Disconnected);
            };
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
        let bound_serial = serial.clone(); // -- WP MDQT1 --
        let mut opened = self.open_bound(handle, serial.clone(), false);
        // -- WP LK4 -- the session was replaced meanwhile (the hub closed it on
        // a lossy link, or a probe dial found it dead): bind once more on the
        // new one
        if matches!(opened, Err(DspiError::Disconnected))
            && let Some(handle) = self.ready_again(&serial)
        {
            opened = self.open_bound(handle, serial, false);
        }
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
            self.unclaim(&bound_serial); // -- WP MDQT1 --
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
        if let Some(conn) = &old {
            self.events.disconnected(&conn.serial, "link lost"); // -- WP P0M --
        }
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
        self.link_connect_hinted(
            url,
            client_name,
            app_version,
            token,
            expected_hub_id,
            Advert::default(),
            tuning,
        )
    }

    // -- WP SECCORE --
    /// [`DspiCore::link_connect_with`] with the hub's advert.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn link_connect_hinted(
        &self,
        url: String,
        client_name: String,
        app_version: String,
        credential: String,
        expected_hub_id: String,
        advert: Advert,
        tuning: Tuning,
    ) -> Result<LinkHello> {
        let credential = Credential::parse(&credential);
        let tls_only = credential.tls_only(); // -- WP LKFAST --
        // -- WP LINKFIX -- fail closed: a bare token goes only to the hub it
        // was issued by, so never without that hub's id (nothing else checks
        // who answers before it goes out in plaintext)
        if bare_token_without_hub_id(&credential, &expected_hub_id) {
            return Err(token_without_hub_id());
        }
        self.close_session(REPLACE_WAIT);
        let (tx, rx) = mpsc::channel();
        let mut target = Target {
            url,
            client_name,
            app_version,
            token: credential.token,
            hub_id: expected_hub_id.to_lowercase(),
            connector: Arc::clone(&tuning.connector),
            timing: tuning.timing,
            clock: Arc::clone(&tuning.clock),
            pin: credential.pin,
            pin_next: credential.next,
            tls_advertised: advert.tls,
            // -- WP LKFAST -- a hub met without TLS lately: no TLS try first. A
            // pinned hub, one pinned before, or one whose advert says TLS never
            // skips it (and the advert makes the next plaintext meeting count anew)
            plaintext_known: !tls_only && !advert.tls && self.hub_memory.skip_tls(&expected_hub_id),
            pinned_before: credential.pinned_before, // -- WP PINTLS --
            memory: Arc::clone(&self.hub_memory),
        };
        if advert.tls {
            self.hub_memory.saw_tls(&expected_hub_id); // -- WP LKFAST3 -- the ratchet
        }
        // -- WP WB4b -- decided on the caller's thread, as the client's own mode is
        let pumped = dspi_link::client::caller_driven();
        // -- WP LK4 -- the first dial retries as the reference client does, and
        // `link_close` (the app's Disconnect) stops it
        let id = NEXT_SESSION.fetch_add(1, Ordering::Relaxed);
        let cancel = Arc::new(AtomicBool::new(false));
        if !pumped {
            *guard(&self.link_control) = Some(LinkControl {
                id,
                cancel: Arc::clone(&cancel),
                tx: tx.clone(),
                client: None,
                pumped,
                driver: None,
            });
        }
        // -- WP LKFAST3 -- the start's dial has a short budget (never longer than the override)
        let budget = advert
            .first_dial
            .map_or_else(first_dial_budget, |d| d.min(first_dial_budget()));
        let dialled = dial_first(
            &target,
            &tx,
            &cancel,
            (!pumped).then_some(tuning.backoff_divisor),
            budget,
        );
        let dialled = match dialled {
            Ok(d) if !cancel.load(Ordering::SeqCst) => Ok(d),
            Ok((client, _)) => {
                client.set_events(None);
                client.close();
                Err(closed_by_client())
            }
            Err(e) => Err(e),
        };
        let (client, state) = match dialled {
            Ok(d) => d,
            Err(e) => {
                let mut control = guard(&self.link_control);
                if control.as_ref().is_some_and(|c| c.id == id) {
                    *control = None;
                }
                return Err(map_error(e));
            }
        };
        if !target.hub_id.is_empty() && !client.hello().hub.id.eq_ignore_ascii_case(&target.hub_id)
        {
            client.set_events(None);
            client.close();
            let mut control = guard(&self.link_control);
            if control.as_ref().is_some_and(|c| c.id == id) {
                *control = None;
            }
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
        // -- WP SECCORE -- the app reads the (possibly upgraded) credential from
        // `link_security`, which the listener also delivers when it changes
        let key_new = target.learn(&client, &state);
        let instance = client.epoch();
        let session = LinkSession {
            id,
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
            probed_at: None,
            completed_at: None,
            #[cfg(test)]
            fail_next_read: false,
            unreported_end: None,
            bound: None,
            reported: LinkPhase::Idle,
            cancel: Arc::clone(&cancel),
            tx,
            // -- WP LKUI1 --
            system: alink_system::Cache::default(),
            key_new, // -- WP SECCORE --
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
        // -- WP LK4 -- a `link_close` that came while this was installed
        if cancel.load(Ordering::SeqCst) {
            let gone = {
                let mut inner = self.lock();
                if inner.link.as_ref().is_some_and(|s| s.id == id) {
                    inner.link.take()
                } else {
                    None
                }
            };
            if let Some(c) = gone.as_ref().and_then(|s| s.client.clone()) {
                c.set_events(None);
                c.close();
            }
            drop(gone);
            let mut control = guard(&self.link_control);
            if control.as_ref().is_some_and(|c| c.id == id) {
                *control = None;
            }
            return Err(map_error(closed_by_client()));
        }
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
        s.key_new |= s.target.learn(&client, &state); // -- WP SECCORE --
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
        // A subscription sent ahead is settled by the supervisor, never here
        let ahead = std::cell::Cell::new(false);
        let mut on_channels = |n: u8| {
            side.want_meters(crate::connect::meter_len(n), keep_mirror && !ahead.get());
            drop(self.set_meter_path(Some(crate::MeterPath {
                num_channels: n,
                side: Arc::clone(&side) as Arc<dyn SideChannel>,
            })));
        };
        // The hub's device list says how many channels there are (as the
        // reference client sizes its poll), so the feed starts before even
        // the identity read; the image corrects it if it says otherwise.
        let device = client
            .devices()
            .into_iter()
            .find(|d| d.handle == u32::from(handle) && d.serial == serial);
        let channels = device
            .as_ref()
            .and_then(|d| d.inputs.zip(d.outputs))
            .and_then(|(i, o)| u8::try_from(i.checked_add(o)?).ok())
            .filter(|n| (1..=crate::consts::MAX_CHANNELS as u8).contains(n));
        // -- WP PGCORE -- a paged device's meter poll is planned with its
        // discovery, which `paged::open` reads below: none sent ahead
        let paged = matches!(
            dspi_transport::link::LinkRoute::of(client.hello(), device.as_ref()),
            dspi_transport::link::LinkRoute::Paged { .. }
        );
        let mut transport = LinkTransport::new(client.clone(), handle, &serial);
        // -- WP PGFIX -- (PGMAT F2) on the legacy route nothing but
        // `REQ_GET_PLATFORM` goes to the device until it says which
        // generation it is: a paged device behind a 1.2 hub refuses it and the
        // bind ends in `link_needs_13_hub` with no page-0 meter poll
        // subscribed. The poll and the snapshot go out once the platform
        // answered as a legacy device with outputs (not the alink sender,
        // which has no meters).
        if paged {
            // -- WP LK4 -- the snapshot is on its way while the identity is read
            transport.prefetch_snapshot();
        }
        // -- WP LKFAST -- a device an ESP32 bridge lists as legacy with meters
        // ([`listed_legacy_with_meters`]): its meter poll and snapshot go out
        // ahead of the identity read, in the same round trip, instead of one
        // after it. Other hubs keep PGFIX's order.
        let bridge = client.hello().hub.kind == "bridge";
        if !paged
            && bridge
            && let Some(n) =
                channels.filter(|_| device.as_ref().is_some_and(listed_legacy_with_meters))
        {
            side.subscribe_ahead(crate::connect::meter_len(n));
            ahead.set(true);
            on_channels(n);
            transport.prefetch_snapshot();
        }
        // The identity too: the serial is what the hub lists. -- WP PGCORE --
        // The platform is the device's own 8 bytes (`read_platform`), not the
        // hub's 7, so the board id in byte 7 is seen (PLAN-PG R7c); on a
        // paged device the discovery `paged::open` reads holds them.
        let own_platform = if paged {
            None
        } else {
            match transport.read_platform() {
                Ok(p) if !p.is_empty() => Some(p),
                Err(e)
                    if matches!(e, TransportError::Disconnected)
                        || dspi_transport::link::LinkNotice::of(&e).is_some() =>
                {
                    self.drop_meter_path_of(&side);
                    return Err(e.into());
                }
                // The bind's identity read asks again
                _ => None,
            }
        };
        // -- WP PGFIX -- the generation is known (legacy) from here
        if !paged {
            let has_meters = own_platform
                .as_deref()
                .is_some_and(|p| !crate::device::is_alink_sender_platform(p, None));
            if let Some(n) = channels.filter(|_| has_meters && !ahead.get()) {
                // Sent ahead of the snapshot: the hub answers in order, so its
                // POLL frames start before the snapshot's bytes, not behind them
                side.subscribe_ahead(crate::connect::meter_len(n));
                ahead.set(true);
                on_channels(n);
            }
            // -- WP LK4 -- the snapshot is on its way while the rest is read
            transport.prefetch_snapshot();
        }
        // -- WP PGWIRE -- the generation wrapper (no probe over DSPi Link)
        // -- WP PGCORE -- on a watched wire: a generation change re-attaches
        let watched = Watched {
            inner: transport,
            regen: Arc::clone(&side.regen),
            tx: tx.clone(),
        };
        let transport = match dspi_transport::paged::open(Box::new(watched)) {
            Ok(t) => t,
            Err(e) => {
                self.drop_meter_path_of(&side);
                return Err(e.into());
            }
        };
        // -- WP PGCORE -- the side plans with the connection's discovery
        let disc = transport.paged_discovery();
        let own_platform = own_platform.or_else(|| disc.as_ref().map(|d| d.platform.clone()));
        // -- WP PGSENDER -- the dual-stack alink sender has no meters: no
        // poll is subscribed for it (its kind is settled at the attach below)
        let paged_sender = disc
            .as_ref()
            .is_some_and(|d| crate::device::is_paged_alink_sender(d, None));
        side.set_discovery(disc);
        if paged
            && !paged_sender
            && let Some(n) = channels
        {
            on_channels(n);
        }
        let bind_platform = own_platform.clone(); // -- WP LKFAST --
        let listed = device
            .as_ref()
            .zip(own_platform)
            .and_then(|(d, p)| listed_identity(d, p));
        let attached = self.attach_opened_with(
            transport,
            serial.clone(),
            0,
            &mut on_channels,
            Some(crate::LinkBind {
                reuse: reuse.as_ref(),
                listed: listed.as_ref(),
            }),
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
        // -- WP LKFAST -- the board from the bind's own platform read
        if let Some(p) = bind_platform {
            conn.board_seen = crate::hardware::board_of_platform(&p, conn.transport());
        }
        // -- WP LK2 -- meters from now on, whatever holds the core lock
        // -- WP PGFIX -- none on the alink sender (it has no meters)
        let old_path =
            self.set_meter_path((conn.kind != crate::DeviceKind::AlinkSender).then(|| {
                crate::MeterPath {
                    num_channels: inner.state.num_channels,
                    side: Arc::clone(&side) as Arc<dyn SideChannel>,
                }
            }));
        if let Some(b) = current.bound.as_mut() {
            b.handle = handle;
            b.generation = conn.generation;
            b.opening = false;
            b.attached = true;
            b.seen_offline = false;
            b.offline_base = offline_base;
            b.probe = probe;
            old_side = b.side.replace(Arc::clone(&side));
        }
        current.retry = None;
        let _ = current.tx.send(Msg::Kick);
        side.bind_done(); // -- WP LK4 --
        // -- WP LKFAST -- the hub to dial again at start
        let h = client.hello();
        let last = LinkLastHub {
            hub_id: h.hub.id.to_lowercase(),
            name: h.hub.name.clone(),
            kind: h.hub.kind.clone(),
            auth: h.auth.clone(),
            tls_advertised: current.target.tls_advertised,
            ..LinkLastHub::default()
        };
        let url = current.target.url.clone();
        drop(guard);
        drop(old_side);
        drop(old_path);
        self.hub_memory.bound(&last, &url);
        Ok(())
    }

    // -- WP LK4 --
    /// After a select lost its session: the device's handle once the session
    /// is ready again (within [`SELECT_AGAIN_WITHIN`]), if the select is still
    /// wanted; `None` at once for a session that ended or a caller-driven one
    /// (it cannot wait here).
    fn ready_again(&self, serial: &str) -> Option<u8> {
        if guard(&self.link_control).as_ref().is_none_or(|c| c.pumped) {
            return None;
        }
        let start = Instant::now();
        while start.elapsed() < SELECT_AGAIN_WITHIN {
            {
                let inner = self.lock();
                let s = inner.link.as_ref()?;
                let wanted = s
                    .bound
                    .as_ref()
                    .is_some_and(|b| b.opening && b.serial == serial);
                if s.ended || s.cancel.load(Ordering::SeqCst) || !wanted {
                    return None;
                }
                if let Some(c) = s.client.as_ref().filter(|c| c.is_ready())
                    && let Some(d) = c.devices().into_iter().find(|d| d.serial == serial)
                {
                    return u8::try_from(d.handle).ok();
                }
            }
            dspi_transport::clock::sleep(Duration::from_millis(50));
        }
        None
    }

    /// Clear the meter path if it is `side`'s (a failed open set it early).
    fn drop_meter_path_of(&self, side: &Arc<LinkSide>) {
        let ours = self
            .meter_path()
            .is_some_and(|p| std::ptr::addr_eq(Arc::as_ptr(&p.side), Arc::as_ptr(side)));
        if ours {
            drop(self.set_meter_path(None));
        }
    }

    /// Decide the next supervisor step, under the lock.
    fn plan_step(&self, id: u64, now: Instant) -> (Step, Option<Closed>, bool) {
        let mut locked = self.lock();
        let inner = &mut *locked;
        let conn_gen = inner.conn.as_ref().map(|c| c.generation);
        // -- WP LK4 -- a bound device whose probe is still to complete
        let pending = inner
            .conn
            .as_ref()
            .is_some_and(|c| c.pending_probe.is_some() && c.side.is_some());
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
        let step = next_step(s, conn_gen, pending, now);
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
        let mut changed = None; // -- WP SECCORE --
        match dialled {
            Ok((client, state)) => {
                if matches!(state, AuthState::TokenRefused { .. }) {
                    s.target.token.clear();
                    s.token_refused = true;
                }
                // -- WP SECCORE -- a v1 credential upgraded, or a rotation: the app stores it
                let new_key = s.target.learn(&client, &state);
                s.key_new = new_key;
                if new_key && !s.target.token.is_empty() {
                    changed = Some((s.target.hub_id.clone(), s.target.credential()));
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
        // -- WP SECCORE --
        drop(inner);
        if let Some((hub_id, credential)) = changed {
            self.listeners
                .link
                .call(|l| l.on_link_token_issued(hub_id, credential));
        }
        true
    }

    // -- WP LK4 --
    /// A probe dial connected: if the old client is still silent, it is dead
    /// (the network works); close it and take the new one, as after a lost
    /// session. Otherwise the old one spoke meanwhile and the new one goes.
    /// Returns false when the session has gone.
    fn finish_probe(&self, id: u64, dialled: (LinkClient, AuthState)) -> bool {
        let old = {
            let mut inner = self.lock();
            let Some(s) = inner
                .link
                .as_mut()
                .filter(|s| s.id == id && !s.cancel.load(Ordering::SeqCst))
            else {
                drop(inner);
                dialled.0.close();
                return false;
            };
            let silent = s
                .client
                .as_ref()
                .is_some_and(|c| !c.is_closed() && c.silent_for() >= PROBE_AFTER);
            if !silent {
                drop(inner);
                dialled.0.set_events(None);
                dialled.0.close();
                return true;
            }
            s.client.take()
        };
        if let Some(old) = old {
            log::info!(
                "the Link hub answers a new session while this one has been silent for {:?}: replacing it",
                old.silent_for()
            );
            old.set_events(None);
            old.close();
        }
        self.finish_dial(id, Ok(dialled))
    }

    /// Complete the open Link device's pending probe (see
    /// [`DspiCore::ensure_probed`]): the features, caps, band count and
    /// extension channel, then the per-item reads, all through the side
    /// channel's background transport without the core lock (never ahead of a
    /// user's command: it leaves one of the hub's slots free); then, under the
    /// lock and only for the same connection, the results go in and the next
    /// `process_notifications` reports a full refresh. A failure leaves it
    /// pending, for the next try.
    pub(crate) fn complete_probe(&self) {
        let _one = guard(&self.probe_lock);
        self.complete_probe_held();
    }

    /// [`DspiCore::ensure_probed`] for a Link build: waits for a completion
    /// in progress, then completes what is still pending.
    pub(crate) fn ensure_probed_link(&self) {
        let _one = guard(&self.probe_lock);
        let pending = self
            .lock()
            .conn
            .as_ref()
            .is_some_and(|c| c.pending_probe.is_some());
        if pending {
            self.complete_probe_held();
        }
    }

    /// [`DspiCore::complete_probe`] with `probe_lock` held.
    fn complete_probe_held(&self) {
        let (quick, side, generation, sources, mut scratch, names_before) = {
            let mut guard = self.lock();
            let inner = &mut *guard;
            let Some(conn) = inner.conn.as_mut() else {
                return;
            };
            // The side comes with the bind's end (`open_bound`)
            let Some(side) = conn.side.clone().filter(|_| conn.pending_probe.is_some()) else {
                return;
            };
            // Left in place until the results go in: meanwhile `fetch_all`
            // still knows the per-item reads are this completion's
            let Some(quick) = conn.pending_probe.clone() else {
                return;
            };
            (
                quick,
                side,
                conn.generation,
                conn.source_names.len(),
                inner.state.clone(),
                (conn.source_names.clone(), conn.device_name.clone()),
            )
        };
        // What the mirror held as the reads went out: a value that differs at
        // the end was written (or notified) meanwhile, and stays (review c82 F1)
        let before = scratch.clone();
        let Some(mut t) = side.background() else {
            return;
        };
        // -- WP PGCORE -- the bind's identity is already the device's own
        // word (`read_platform`, 8 bytes): no second platform read here
        let done = match dspi_session::probe::probe_rest(&mut *t, &quick) {
            Ok(done) => done,
            Err(TransportError::Disconnected) => return,
            Err(e) => {
                log::info!("completing the Link device's probe failed, trying again: {e}");
                return;
            }
        };
        let names = crate::info::name_reads_for(&done.caps, sources);
        // From the mirror, not from defaults: a read that fails (firmware
        // without it, a stall) is no value, and leaves the field as it is
        let answers = crate::connect::read_per_item_on(&mut *t, &names, &mut scratch);
        let reusable = done.reusable();
        let ext = match done.ext {
            Ok(ext) => ext,
            Err(TransportError::Disconnected) => return,
            // As at a whole probe: the device stays, without the channel
            Err(e) => {
                log::warn!("extension probe failed, continuing without it: {e}");
                ExtState::default()
            }
        };
        {
            let mut guard = self.lock();
            let inner = &mut *guard;
            let Some(conn) = inner.conn.as_mut().filter(|c| c.generation == generation) else {
                return;
            };
            let fits = conn
                .session()
                .is_ok_and(|session| session.complete_capabilities(done.caps));
            if !fits {
                log::warn!(
                    "the Link device's completed probe does not fit its image; kept as it was"
                );
                conn.pending_probe = None;
                return;
            }
            conn.pending_probe = None;
            conn.echo.set_bulk_source(ext.bulk_source());
            conn.ext = ext;
            // The names likewise: one renamed meanwhile keeps its new name
            let names_now = (conn.source_names.clone(), conn.device_name.clone());
            crate::info::apply_names(conn, &names, answers);
            for (i, (now, was)) in names_now.0.iter().zip(names_before.0.iter()).enumerate() {
                if now != was
                    && let Some(slot) = conn.source_names.get_mut(i)
                {
                    slot.clone_from(now);
                }
            }
            if names_now.1 != names_before.1 {
                conn.device_name = names_now.1;
            }
            conn.announce_refresh = true;
            crate::connect::merge_per_item(&before, &scratch, &mut inner.state);
            if let Some(b) = inner.link.as_mut().and_then(|s| s.bound.as_mut()) {
                b.probe = reusable;
            }
        }
        self.listeners.notify.call(|l| l.on_notifications());
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
        // -- WP SECCORE -- the credential, pinned when the pairing ran over TLS
        let token = {
            let inner = self.lock();
            match inner.link.as_ref() {
                Some(s) if s.target.token == token => s.target.credential(),
                _ => token,
            }
        };
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
                // -- WP HUBFLAKE3 -- The session ended under the read: that is no
                // failed list, and `note_loss` decides by the hub's close code
                // (4002 asks for a person; as a list failure it ended the session).
                Err(_) if client.is_closed() => {}
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
fn next_step(s: &mut LinkSession, conn_gen: Option<u64>, pending: bool, now: Instant) -> Step {
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
    // -- WP PGCORE -- a generation change (the transport's 0x86 after an
    // answer, or another `protocol` in the hub's list): a new transport and
    // a new `paged::open`, never a retry on the old one (section 1.1 step 8)
    if b.attached
        && b.side
            .as_ref()
            .is_some_and(|side| side.generation_changed(d.protocol))
    {
        b.attached = false;
    }
    if b.attached {
        // -- WP LK4 -- the rest of the bind's probe, in the background
        if pending
            && client.silent_for() < PROBE_AFTER
            && s.completed_at
                .is_none_or(|t| now.saturating_duration_since(t) >= COMPLETE_AGAIN)
        {
            s.completed_at = Some(now);
            return Step::Complete;
        }
        if probe_due(s, &client, now) {
            s.probed_at = Some(now);
            return Step::Probe(Box::new(s.target.clone()));
        }
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
        // -- WP HUBADMINF -- F1
        Msg::Closed(epoch) => core.hub_role_forget(id, epoch),
        // -- WP HUBADMIN --
        Msg::Hub { epoch, event } => {
            let current = core
                .lock()
                .link
                .as_ref()
                .is_some_and(|s| s.id == id && s.instance == epoch);
            if current {
                core.hub_admin_event(*event);
            }
        }
        // -- WP P2CC1 --
        Msg::HubServer { epoch, event } => {
            let current = core
                .lock()
                .link
                .as_ref()
                .is_some_and(|s| s.id == id && s.instance == epoch);
            if current {
                core.hub_server_event(*event);
            }
        }
        // -- WP ATCORE --
        Msg::MeasureAborted { epoch, event } => {
            let current = core
                .lock()
                .link
                .as_ref()
                .is_some_and(|s| s.id == id && s.instance == epoch);
            if current {
                core.measure_system_on_aborted(event);
            }
        }
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
            // -- WP LK4 --
            Step::Probe(target) => {
                let tx = core.lock().link.as_ref().map(|s| s.tx.clone())?;
                drop(core);
                let dialled = redial(&target, &tx);
                let Some(core) = me.upgrade() else {
                    if let Ok((client, _)) = dialled {
                        client.close();
                    }
                    return None;
                };
                match dialled {
                    Ok(dialled) => {
                        if !core.finish_probe(id, dialled) {
                            return None;
                        }
                        core.listeners.link.call(|l| l.on_link_status_changed());
                    }
                    Err(e) => log::debug!("probe dial past a silent session failed: {e}"),
                }
            }
            Step::ReadDevices(client) => core.read_devices(id, &client),
            // -- WP LK4 --
            Step::Complete => core.complete_probe(),
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
    // -- WP SECCORE -- TLS first (PLAN-SEC 4.3)
    LinkClient::connect_tls(
        target.connector.as_ref(),
        &target.url,
        &opts,
        Some(Arc::new(Forward { tx: tx.clone() })),
        &cfg,
        &target.tls_options(),
    )
    .map_err(|e| match e {
        // A pin with no token is a pairing link's: its key was not met
        LinkError::HubKeyChanged if target.token.is_empty() => LinkError::HubKeyUnverified,
        e => e,
    })
}

// -- WP LINKFIX --
/// Whether `credential` is a bare (`v1`) token, no key pin, with no expected
/// hub id: [`DspiCore::link_connect`] refuses to send it to whoever answers.
fn bare_token_without_hub_id(credential: &Credential, expected_hub_id: &str) -> bool {
    !credential.token.trim().is_empty()
        && credential.pin.is_none()
        && expected_hub_id.trim().is_empty()
}

// -- WP LINKFIX --
/// The error for [`bare_token_without_hub_id`]: nothing was dialled; the
/// client pairs again (or dials with the hub's id).
fn token_without_hub_id() -> DspiError {
    DspiError::Link {
        code: "token_without_hub_id".into(),
        msg: "This pairing has no hub id to check the hub against, so its token was not sent. \
              Pair with the hub again."
            .into(),
    }
}

// -- WP SECCORE --
/// What a hub's DNS-SD advert said about TLS (SECDISC's result).
#[derive(Debug, Clone, Default)]
pub(crate) struct Advert {
    /// The browsed record's `tls_advertised` (spec 3.1 `tls=1`).
    pub(crate) tls: bool,
    // -- WP LKFAST3 --
    /// The first dial's budget when not [`FIRST_DIAL_BUDGET`] (the app start's dial).
    pub(crate) first_dial: Option<Duration>,
}

// -- WP LKFAST3 -- (review LKREV 6)
/// The first dial's budget at app start ([`DspiCore::link_connect_at_start`]):
/// 8 s is the third SYN retransmission of a TCP connect (1 + 2 + 4 s) plus
/// the 0.5 s and 1 s first-dial retries, so a hub at its address on a lossy
/// Wi-Fi still gets through, while a hub that moved hands over to the search
/// in seconds instead of a minute.
pub(crate) const START_DIAL_BUDGET: Duration = Duration::from_secs(8);

// -- WP LK4 --
/// How long the first dial keeps trying (each try bounded by the connector's
/// and the client's own connect timeouts): longer than the reference
/// client's 15 s handshake plus its prober's 1 s retries.
pub(crate) const FIRST_DIAL_BUDGET: Duration = Duration::from_secs(60);
/// The environment variable that overrides [`FIRST_DIAL_BUDGET`], in ms, for
/// tests and integration rigs that drive the core through its bindings (the
/// Android suite's fake-hub tests): unset, empty or unparsable means the
/// default. Read at each first dial, in debug builds only (review c83 N1: a
/// release build ignores it).
pub const FIRST_DIAL_BUDGET_ENV: &str = "DSPI_LINK_FIRST_DIAL_MS";

/// [`FIRST_DIAL_BUDGET`], or its override from [`FIRST_DIAL_BUDGET_ENV`].
fn first_dial_budget() -> Duration {
    #[cfg(all(
        debug_assertions,
        not(all(target_arch = "wasm32", target_os = "unknown"))
    ))]
    if let Some(ms) = std::env::var(FIRST_DIAL_BUDGET_ENV)
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
    {
        return Duration::from_millis(ms.min(600_000));
    }
    FIRST_DIAL_BUDGET
}
/// The wait before the first dial's second try, doubled each time up to
/// [`FIRST_DIAL_RETRY_MAX`] (or the hub's own advice when longer).
const FIRST_DIAL_RETRY: Duration = Duration::from_millis(500);
const FIRST_DIAL_RETRY_MAX: Duration = Duration::from_secs(2);

/// The first dial of a session: as [`dial`], tried again with a short
/// backoff while the error's advice is to retry (a timeout, a lost socket, a
/// busy hub; never a refusal, a protocol error or a pairing problem) and
/// within [`FIRST_DIAL_BUDGET`], until `cancel`. Nothing is sent before the
/// hello, so nothing is replayed. `retry`: the backoff divisor (tests speed
/// it up; 1 in production), `None` for one try (a caller-driven session).
fn dial_first(
    target: &Target,
    tx: &Sender<Msg>,
    cancel: &AtomicBool,
    retry: Option<u32>,
    budget: Duration,
) -> std::result::Result<(LinkClient, AuthState), LinkError> {
    let divided = |d: Duration, by: u32| d.checked_div(by.max(1)).unwrap_or(d);
    let start = Instant::now();
    let mut failures = 0u32;
    let mut told_tls = false;
    loop {
        let e = match dial(target, true, tx) {
            Ok(d) => return Ok(d),
            Err(e) => e,
        };
        // -- WP LKFAST3 -- (review LKREV 4) the hub refused plaintext: it has TLS.
        // TLS only from now on (the ratchet), and dialled again at once, over TLS
        if matches!(e, LinkError::TlsRequired)
            && !told_tls
            && !target.hub_id.is_empty()
            && !cancel.load(Ordering::SeqCst)
        {
            told_tls = true;
            target.memory.saw_tls(&target.hub_id);
            log::info!("DSPi Link: the hub asks for TLS; dialling it again over TLS");
            continue;
        }
        let Some(by) = retry.filter(|_| !cancel.load(Ordering::SeqCst)) else {
            return Err(e);
        };
        // -- WP SECCORE -- a TLS the hub must have and refused: say so now
        if matches!(
            e,
            LinkError::TlsRefused { .. } | LinkError::TlsRequiredPinned
        ) {
            return Err(e); // -- WP PINTLS -- the same for a hub pinned before
        }
        let RetryAdvice::Retry { min_delay } = e.retry_advice() else {
            return Err(e);
        };
        let wait = FIRST_DIAL_RETRY
            .saturating_mul(1u32.checked_shl(failures).unwrap_or(u32::MAX))
            .min(FIRST_DIAL_RETRY_MAX)
            .max(min_delay);
        let wait = divided(wait, by);
        if start.elapsed().saturating_add(wait) >= divided(budget, by) {
            return Err(e);
        }
        log::info!("DSPi Link: the first dial failed ({e}); trying again in {wait:?}");
        let waited = Instant::now();
        while waited.elapsed() < wait {
            if cancel.load(Ordering::SeqCst) {
                return Err(e);
            }
            dspi_transport::clock::sleep(Duration::from_millis(50));
        }
        failures = failures.saturating_add(1);
    }
}

/// The error a dial stopped by `link_close` ends with.
fn closed_by_client() -> LinkError {
    LinkError::Closed {
        code: Some(1000),
        reason: dspi_link::close_reason(Some(1000)),
    }
}

/// Whether `url` is `ws://host[:port][/path]` (-- WP SECCORE -- or `wss://`).
fn valid_ws_url(url: &str) -> bool {
    url.strip_prefix("ws://")
        .or_else(|| url.strip_prefix("wss://"))
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
// -- WP SW3 --
/// How long a hub's own JSON request may take: the audio link's `system.status`
/// and `system.align` read and write every unit of the model, a few transfers each.
const HUB_REQUEST_TIMEOUT: Duration = Duration::from_secs(15);

// -- WP SW3 --
/// The hub's own JSON requests on this session's connection, for the web
/// console's System view (the audio link's `system.*`,
/// `docs/integrator/audio-link.md`). Not exported over UniFFI or the C ABI;
/// the web binding calls these.
impl DspiCore {
    /// Send `t` with `fields` to the hub this session is on, and wait for its
    /// answer (the `ok` message whole). The hub judges it by this session's
    /// role. Errors: [`DspiError::Disconnected`] with no hub session,
    /// [`DspiError::Link`] with the hub's code (`denied`, `rate_limited`,
    /// `bad_request`, ...) or the client's (`timeout`, `closed`).
    pub fn link_hub_request(
        &self,
        t: &str,
        fields: serde_json::Map<String, serde_json::Value>,
    ) -> Result<serde_json::Value> {
        self.link_hub_request_within(t, fields, HUB_REQUEST_TIMEOUT)
    }

    // -- WP P3CC -- a group request answers after its last member
    /// [`Self::link_hub_request`] waiting up to `timeout` for the answer.
    pub(crate) fn link_hub_request_within(
        &self,
        t: &str,
        fields: serde_json::Map<String, serde_json::Value>,
        timeout: Duration,
    ) -> Result<serde_json::Value> {
        let client = {
            let mut inner = self.lock();
            let client = inner
                .link
                .as_ref()
                .and_then(|s| s.client.clone())
                .filter(|c| !c.is_closed())
                .ok_or(DspiError::Disconnected)?;
            // -- WP ROMODE -- the client read-only mode: the hub's reads only
            crate::read_only::check_hub_request(&mut inner, t)?;
            client
        };
        client.request_json(t, fields, timeout).map_err(map_error)
    }

    /// The role the hub gave this session (`viewer`, `control`, `admin`); ""
    /// with no hub session.
    pub fn link_hub_role(&self) -> String {
        self.lock()
            .link
            .as_ref()
            .and_then(|s| s.client.clone())
            .filter(|c| !c.is_closed())
            .map(|c| c.role())
            .unwrap_or_default()
    }
}

// -- WP P1DKFIX --
impl crate::Inner {
    /// The serial of the device bound on a live DSPi Link session, if any.
    pub(crate) fn link_bound_serial(&self) -> Option<&str> {
        let s = self.link.as_ref()?;
        s.client.as_ref().filter(|c| !c.is_closed())?;
        s.bound.as_ref().map(|b| b.serial.as_str())
    }
}

// -- WP P1DKFIX --
impl DspiCore {
    /// The device bound on this session's live hub, when the hub answers
    /// `limits.*` (its `hello` lists `limits`) and that device is the open
    /// one: its handle and serial. No I/O.
    pub(crate) fn link_limits_target(&self) -> Option<(u8, String)> {
        let inner = self.lock();
        let s = inner.link.as_ref()?;
        let client = s.client.as_ref().filter(|c| !c.is_closed())?;
        let bound = s.bound.as_ref()?;
        if !client.hello().has_cap("limits") {
            return None;
        }
        let open = inner.conn.as_ref().map(|c| c.serial.as_str());
        // -- WP HUBADMINF -- F3: and the open connection is the bound one
        let same = inner
            .conn
            .as_ref()
            .is_some_and(|c| bound.opening || c.generation == bound.generation);
        (same && open == Some(bound.serial.as_str())).then(|| (bound.handle, bound.serial.clone()))
    }
}

// -- WP HUBADMIN --
/// The open device as the bound device of this console's DSPi Link session,
/// for the Installer page's choice of copy (`hub_admin.rs`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AdminTarget {
    /// Its handle on the hub.
    pub(crate) handle: u8,
    /// Its serial.
    pub(crate) serial: String,
    /// This console's role on the hub (`viewer`, `control`, `admin`; "" while
    /// the session is down).
    pub(crate) role: String,
    /// The hub answers `limits.*` (its `hello` lists `limits`).
    pub(crate) has_limits: bool,
    /// The session's socket is up.
    pub(crate) live: bool,
    // -- WP HUBADMINF -- F1
    /// The Link session and its client's epoch: a role learnt from the hub
    /// holds for this pair only (a reconnect is a new epoch).
    pub(crate) session: (u64, u64),
}

// -- WP HUBADMIN --
impl DspiCore {
    /// The open device, when it is the one bound on this console's DSPi Link
    /// session (live or reconnecting); `None` for USB, MIDI or the demo. No I/O.
    pub(crate) fn link_admin_target(&self) -> Option<AdminTarget> {
        let inner = self.lock();
        let s = inner.link.as_ref()?;
        let bound = s.bound.as_ref()?;
        let conn = inner.conn.as_ref()?;
        if conn.serial != bound.serial {
            return None;
        }
        // -- WP HUBADMINF -- F3: the same serial is not enough (the demo device
        // and a demo hub's share one): the open connection must be the one
        // this session attached for it, or the one it is attaching now.
        if !bound.opening && conn.generation != bound.generation {
            return None;
        }
        let client = s.client.as_ref().filter(|c| !c.is_closed());
        Some(AdminTarget {
            session: (s.id, s.instance),
            handle: bound.handle,
            serial: bound.serial.clone(),
            role: client.map(LinkClient::role).unwrap_or_default(),
            has_limits: client.is_some_and(|c| c.hello().has_cap("limits")),
            live: client.is_some(),
        })
    }
}

// -- WP P2CC1 --
impl DspiCore {
    /// The caps of this console's DSPi Link hub: `None` with no session,
    /// `Some(None)` while its socket is down. No I/O.
    pub(crate) fn link_hub_caps(&self) -> Option<Option<Vec<String>>> {
        let inner = self.lock();
        let s = inner.link.as_ref()?;
        Some(
            s.client
                .as_ref()
                .filter(|c| !c.is_closed())
                .map(|c| c.hello().caps.clone()),
        )
    }
}

// -- WP P1SA --
/// What a guarded apply (`guarded.rs`) needs of this session's hub.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct GuardTarget {
    /// The bound device's handle on the hub.
    pub(crate) handle: u8,
    /// This session's id on the hub (0 before the hub gave one).
    pub(crate) session: u16,
    /// The hub answers `apply.*` (its `hello` lists `apply_guard`).
    pub(crate) has_guard: bool,
}

// -- WP P2FFIX --
impl DspiCore {
    /// This console's session id on its live hub connection, bound to a
    /// device or not (`None` with no live session, or before the hub gave
    /// one). No I/O.
    pub(crate) fn link_client_session(&self) -> Option<u16> {
        let inner = self.lock();
        let s = inner.link.as_ref()?;
        let client = s.client.as_ref().filter(|c| !c.is_closed())?;
        client.session_id()
    }
}

// -- WP P1SA --
impl DspiCore {
    /// The device bound on this session's live hub connection, for a guarded
    /// apply; `None` with no live hub session or no bound device. No I/O.
    pub(crate) fn link_guard_target(&self) -> Option<GuardTarget> {
        let inner = self.lock();
        let s = inner.link.as_ref()?;
        let client = s.client.as_ref().filter(|c| !c.is_closed())?;
        let bound = s.bound.as_ref()?;
        Some(GuardTarget {
            handle: bound.handle,
            session: client.session_id().unwrap_or(0),
            has_guard: client.hello().has_cap("apply_guard"),
        })
    }
}

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
        // -- WP SECCORE --
        LinkError::HubKeyChanged => "hub_key_changed",
        LinkError::HubKeyUnverified => "hub_key_unverified",
        LinkError::TlsRefused { .. } => "tls_refused",
        LinkError::TlsRequired => "tls_required", // -- WP SECBFIXA --
        LinkError::TlsRequiredPinned => "tls_required_pinned", // -- WP PINTLS --
    };
    DspiError::Link {
        code: code.to_owned(),
        msg,
    }
}

// -- WP LKFAST --
#[cfg_attr(feature = "uniffi", uniffi::export)]
impl DspiCore {
    /// Remember DSPi Link hubs between sessions in a small file under `dir`,
    /// the app's own data folder ("" forgets the folder): a hub met without
    /// TLS is dialled without a TLS try next time ([`DspiCore::link_recheck_tls`]),
    /// and the hub bound last is kept for [`DspiCore::link_start_hub`]. Name
    /// it before the first connect.
    ///
    /// Errors: [`DspiError::Unsupported`] on the web (no files).
    pub fn link_set_dir(&self, dir: String) -> Result<()> {
        #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
        {
            let dir = dir.trim();
            self.hub_memory
                .set_dir((!dir.is_empty()).then(|| dir.into()));
            Ok(())
        }
        #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
        {
            let _ = dir;
            Err(DspiError::Unsupported)
        }
    }

    /// The hub to dial at start (owner decision 2026-11-21): the one whose
    /// device was bound last, unless the user left it
    /// ([`DspiCore::link_forget_last_hub`]) or a DSPi is on USB
    /// (`usb_present`: USB wins). `None` without a folder. The client checks
    /// its own settings (DSPi Link on, reconnect at start on) and dials it as
    /// a browsed hub, with that hub's stored credential: a pinned hub stays
    /// TLS only. Needs no device; no traffic.
    pub fn link_start_hub(&self, usb_present: bool) -> Option<LinkLastHub> {
        if usb_present {
            return None;
        }
        self.hub_memory.last()
    }

    /// The user left the hub (Disconnect, or switched to another device): no
    /// dial at start until a device is bound on a hub again.
    pub fn link_forget_last_hub(&self) {
        self.hub_memory.left();
    }

    /// Try TLS again at hub `hub_id`'s next connect as at first contact (Pair
    /// Again): what was met lately, without TLS or with it, is forgotten. No
    /// traffic.
    pub fn link_recheck_tls(&self, hub_id: String) {
        self.hub_memory.recheck(&hub_id);
    }

    // -- WP LKFAST3 -- (review LKREV 2)
    /// The user forgot hub `hub_id` (its pairing deleted): everything the core
    /// remembers of it goes, and it is no longer [`DspiCore::link_start_hub`]'s
    /// hub. No traffic.
    pub fn link_forget_hub(&self, hub_id: String) {
        self.hub_memory.forget(&hub_id);
    }

    /// A hub's encryption before it is dialled, for its line in a list or a
    /// prompt: [`LinkEncryption::Verified`] for a stored credential with a key
    /// pin, `Offered` for one pinned before or an advert that says TLS, `Off`
    /// for a hub met without TLS lately or a browsed hub that does not
    /// advertise it, `Unknown` for a `typed` address with nothing known.
    /// The words: [`link_encryption_notice_key`]. No traffic.
    pub fn link_hub_encryption(
        &self,
        credential: String,
        tls_advertised: bool,
        hub_id: String,
        typed: bool,
    ) -> LinkEncryption {
        let c = Credential::parse(&credential);
        if c.pin.is_some() {
            LinkEncryption::Verified
        } else if c.pinned_before || tls_advertised || self.hub_memory.tls_seen(&hub_id) {
            LinkEncryption::Offered
        } else if self.hub_memory.met_without_tls(&hub_id) || !typed {
            LinkEncryption::Off
        } else {
            LinkEncryption::Unknown
        }
    }

    /// The current session's encryption ([`DspiCore::link_security`] as one
    /// state): `Unknown` without a session.
    pub fn link_session_encryption(&self) -> LinkEncryption {
        let live = self
            .lock()
            .link
            .as_ref()
            .is_some_and(|s| s.client.is_some());
        if !live {
            return LinkEncryption::Unknown;
        }
        let s = self.link_security();
        if s.local {
            LinkEncryption::Local
        } else if !s.encrypted {
            LinkEncryption::Off
        } else if Credential::parse(&s.credential).pin.is_some() {
            LinkEncryption::Verified
        } else {
            LinkEncryption::Offered
        }
    }
}

// -- WP LKFAST --
/// The notice row with `encryption`'s words (`notice_for_key`): one rule for
/// every client's hub lines, prompts and status.
#[cfg_attr(feature = "uniffi", uniffi::export)]
pub fn link_encryption_notice_key(encryption: LinkEncryption) -> String {
    encryption.notice_key().to_owned()
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

// -- WP LK4 --
// -- WP LKFAST --
/// An ESP32 bridge lists `d` as a legacy DSPi with meters: firmware before
/// the paged generation (1.1.7), outputs (the alink sender has none), and the
/// image header's `wire_version`, which the bridge lists only once it has read
/// the device's legacy settings image (upstream `hub_device.c`
/// `learn_bulk_header`): a paged device or the sender never gets one.
fn listed_legacy_with_meters(d: &DeviceInfo) -> bool {
    use dspi_proto::packets::PlatformInfo;
    if d.wire_version.is_none() {
        return false;
    }
    hub_listed_identity(d)
        .and_then(|l| PlatformInfo::decode(&l.platform).ok())
        .is_some_and(|p| {
            let v = p.version;
            p.num_output_channels > 0 && (v.major, v.minor, v.patch) < (1, 1, 7)
        })
}

/// The GET_PLATFORM reply and serial the hub lists for `d`, when it says all
/// of it (platform, firmware `major.minor.patch` with its beta, outputs);
/// `None` otherwise.
fn hub_listed_identity(d: &DeviceInfo) -> Option<dspi_session::probe::Listed> {
    use dspi_proto::packets::{FirmwareVersion, PlatformInfo};
    let mut parts =
        d.fw.as_deref()?
            .split('.')
            .map(|p| p.trim().parse::<u8>().ok());
    let (major, minor, patch) = (parts.next()??, parts.next()??, parts.next()??);
    if parts.next().is_some() || d.serial.is_empty() {
        return None;
    }
    let info = PlatformInfo {
        platform_id: u8::try_from(d.platform?).ok()?,
        num_output_channels: u8::try_from(d.outputs?).ok()?,
        version: FirmwareVersion::new(major, minor, patch, u8::try_from(d.fw_beta?).ok()?),
        reply_len: PlatformInfo::REQUEST_LEN,
        // -- WP PGFAKE2 -- a hub lists 7 platform bytes (spec 7.4): no board id
        // here; PGLINKC reads the 8 bytes itself (PLAN-PG R7c).
        board_id: 0,
    };
    Some(dspi_session::probe::Listed {
        platform: info.encode().to_vec(),
        serial: d.serial.clone(),
    })
}

// -- WP PGCORE --
/// The bind's identity: the device's own platform reply `platform` (8 bytes
/// asked, so byte 7, the board id, is seen: PLAN-PG R7c) with the serial the
/// hub lists for `d`; `None` when the serial is not listed or the reply does
/// not decode, and the bind reads them. A hub listing that differs from the
/// device is logged: the device's word wins.
fn listed_identity(d: &DeviceInfo, platform: Vec<u8>) -> Option<dspi_session::probe::Listed> {
    use dspi_proto::packets::PlatformInfo;
    if d.serial.is_empty() || PlatformInfo::decode(&platform).is_err() {
        return None;
    }
    if let Some(hub) = hub_listed_identity(d)
        && platform.get(..hub.platform.len()) != Some(hub.platform.as_slice())
    {
        log::info!("the hub's listing of the Link device differs from its own GET_PLATFORM");
    }
    Some(dspi_session::probe::Listed {
        platform,
        serial: d.serial.clone(),
    })
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
    // -- WP LK4 --
    /// The meter feed last went silent on a bridge then: it runs at
    /// [`METER_HZ_STALLED`] until frames have flowed for [`METER_SLOW_FOR`].
    meter_slow_since: Option<Instant>,
    /// A subscription sent ahead of the bind's snapshot, its grant not read
    /// yet ([`LinkSide::subscribe_ahead`]), with the rate asked.
    meter_pending: Option<(dspi_link::client::PendingPolls, u32)>,
    /// The rate of a subscription sent ahead whose grant is not read yet
    /// (also while the supervisor waits for it): its frames count meanwhile.
    meter_ahead_hz: Option<u32>,
    /// A bind is reading the snapshot: on a bridge the feed runs at
    /// [`METER_HZ_STALLED`] meanwhile, so its frames take little from the
    /// snapshot on a lossy link ([`LinkSide::bind_done`] lifts it).
    meter_binding: bool,
    /// The current subscription was asked at the slow rate.
    meter_asked_slow: bool,
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
    // -- WP PGCORE --
    /// The bound connection's discovery when the device is paged (set right
    /// after `paged::open`): every request of this side is then planned with
    /// it, and nothing on page 0 is sent (PLAN-PG section 1.1).
    disc: Mutex<Option<Discovery>>,
    /// The `protocol` the hub listed for the device when this side was made:
    /// another one later is a re-flash under the handle (section 1.1 step 8).
    listed_protocol: Option<dspi_link::messages::Proto>,
    /// The bound transport saw a generation change
    /// (`LinkTransport::needs_reattach`): the supervisor re-attaches.
    regen: Arc<AtomicBool>,
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
            ..Default::default()
        };
        Self {
            handle,
            descriptor,
            polls,
            bridged,
            rx: Mutex::new(rx),
            feed: Mutex::new(Feed::default()),
            lock_holds: Mutex::new(0),
            meter_tunnel: Mutex::new(()),
            // -- WP PGCORE --
            disc: Mutex::new(None),
            listed_protocol: client.device(u32::from(handle)).and_then(|d| d.protocol),
            regen: Arc::new(AtomicBool::new(false)),
            client,
            tx,
        }
    }

    /// Move what arrived into `feed`.
    fn drain(&self, feed: &mut Feed, now: Instant) {
        let pages = self.pages(); // -- WP PGCORE --
        for f in guard(&self.rx).try_iter() {
            // An empty frame is a failed read on the hub, not a reading.
            if f.payload.is_empty() {
                continue;
            }
            // -- WP PGCORE -- a paged device's frame is the paged GET's
            // reply: fed through the same plan into the 1.1.6 shape
            let payload = match &pages {
                Some(pages) => {
                    let Some(shaped) = slot_read(feed, f.slot)
                        .and_then(|(op, val, len)| reshape(pages, op, val, len, &f.payload))
                    else {
                        continue;
                    };
                    shaped
                }
                None => f.payload,
            };
            feed.latest.insert(f.slot, (payload, now));
        }
    }

    /// One tunnelled GET; its reply may be shorter than `len`.
    fn tunnel_get(&self, opcode: u8, value: u16, len: u16) -> dspi_transport::Result<Vec<u8>> {
        // -- WP PGCORE -- planned on a paged device
        if let Some(mut paged) = self.paged_view(false) {
            return paged.control_in_upto(opcode, value, len);
        }
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
        self.settle_pending(now); // -- WP LK4 --
        // -- review c84 F1 -- settle_pending can block for the ahead grant; frames
        // drained after it are stamped with the time they arrive, not the call's.
        let now = Instant::now();
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
            // -- WP LK4 -- a bridge closes a session whose queued frames it
            // could not write for 10 s (bridge spec 8.2; a stall fills its
            // send buffer sooner the faster it polls): a feed gone silent is
            // asked again at a low rate, and back at the full rate once
            // frames have flowed again for a while
            if self.bridged && silent {
                feed.meter_slow_since = Some(now);
            }
            let flowing = feed.latest.get(&METER_SLOT).is_some_and(|f| {
                now.saturating_duration_since(f.1) < stale_after(METER_HZ_STALLED)
            });
            let recovered = !silent
                && flowing
                && feed
                    .meter_slow_since
                    .is_some_and(|t| now.saturating_duration_since(t) >= METER_SLOW_FOR);
            if recovered {
                feed.meter_slow_since = None;
            }
            // The bind's slow rate is over: up to the full one
            let slow = feed.meter_slow_since.is_some() || feed.meter_binding;
            let upgrade = !slow && feed.meter_asked_slow && feed.meter_hz.is_some();
            if feed.meter_hz.is_some() && !silent && !recovered && !upgrade {
                return;
            }
            feed.meter_subscribed_at = Some(now);
            feed.meter_asked_slow = slow;
            // -- WP PGCORE -- a read the paged device cannot poll: tunnelled
            match self.meter_spec(feed.meter_len, slow) {
                Some(spec) => spec,
                None => {
                    feed.meter_hz = None;
                    feed.meter_refused_until = now.checked_add(POLL_RETRY_AFTER);
                    return;
                }
            }
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
    /// Subscribe the meter block of `len` bytes now without waiting for the
    /// grant (the supervisor reads it), so the request goes out ahead of
    /// whatever the caller sends next: the bind's snapshot.
    fn subscribe_ahead(&self, len: u16) {
        if !self.polls || self.client.is_closed() {
            return;
        }
        let now = Instant::now();
        // -- WP PGCORE --
        let Some(spec) = self.meter_spec(len, self.bridged) else {
            return;
        };
        let Ok(pending) = self.client.poll_subscribe_begin(self.handle, &[spec]) else {
            return;
        };
        let mut feed = guard(&self.feed);
        feed.meter_binding = self.bridged;
        feed.meter_asked_slow = self.bridged;
        feed.meter_len = len;
        feed.meter_asked = Some(now);
        feed.meter_subscribed_at = Some(now);
        feed.meter_pending = Some((pending, spec.hz));
        feed.meter_ahead_hz = Some(spec.hz);
        if !std::mem::replace(&mut feed.kicked, true) {
            let _ = self.tx.send(Msg::Kick);
        }
    }

    /// The bind has its snapshot: the feed goes to its full rate.
    fn bind_done(&self) {
        let mut feed = guard(&self.feed);
        if std::mem::take(&mut feed.meter_binding) && !std::mem::replace(&mut feed.kicked, true) {
            let _ = self.tx.send(Msg::Kick);
        }
    }

    /// Read the grant of [`LinkSide::subscribe_ahead`]'s request, if one is
    /// pending: the supervisor's, may wait for the answer.
    fn settle_pending(&self, now: Instant) {
        let Some((pending, _)) = guard(&self.feed).meter_pending.take() else {
            return;
        };
        let granted = self.client.poll_subscribe_finish(pending);
        let mut feed = guard(&self.feed);
        feed.meter_ahead_hz = None;
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

    /// Ask for the meter block of `len` bytes now, as a first `fetch_status`
    /// would, so the poll is subscribed before the app's first look: here
    /// with `now` (one round trip), else by a word to the supervisor.
    fn want_meters(&self, len: u16, now: bool) {
        {
            let mut feed = guard(&self.feed);
            if feed.meter_len != len {
                // Another size than asked before (the image corrects the
                // device list): subscribe again
                feed.meter_len = len;
                feed.meter_hz = None;
                feed.latest.remove(&METER_SLOT);
            }
            feed.meter_asked = Some(Instant::now());
            if !now && !std::mem::replace(&mut feed.kicked, true) {
                let _ = self.tx.send(Msg::Kick);
            }
        }
        if now {
            self.maintain(Instant::now());
        }
    }

    /// -- WP PGCORE -- `None` when a paged device's plan for the meter read
    /// is not one GET (OD-PGo): the meters are then tunnelled.
    fn meter_spec(&self, len: u16, slow: bool) -> Option<PollSpec> {
        let wanted = if slow {
            METER_HZ_STALLED
        } else if self.bridged {
            METER_HZ_BRIDGED
        } else {
            METER_HZ
        };
        let max = self.client.hello().limits.poll_max_hz;
        let hz = if max == 0 { wanted } else { wanted.min(max) };
        // -- WP PGCORE -- the paged GET of the read's single-step plan
        if let Some(pages) = self.pages() {
            return paged_poll(
                METER_SLOT,
                op::REQ_GET_STATUS,
                STATUS_METERS,
                len,
                hz,
                &pages,
            )
            .map_err(|e| log::info!("the meters are not polled on this device: {e}"))
            .ok();
        }
        Some(PollSpec {
            slot: METER_SLOT,
            req: op::REQ_GET_STATUS,
            val: STATUS_METERS,
            idx: dspi_proto::VENDOR_INTERFACE,
            len,
            hz,
        })
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
            let pages = self.pages(); // -- WP PGCORE --
            let specs: Vec<PollSpec> = band_channels(mask)
                .filter_map(|ch| {
                    let slot = band_slot(ch)?;
                    // -- WP PGCORE -- planned on a paged device; a channel
                    // it cannot poll is tunnelled (not granted)
                    if let Some(pages) = &pages {
                        return paged_poll(
                            slot,
                            op::REQ_RTA_GET_BANDS,
                            u16::from(ch),
                            len,
                            hz,
                            pages,
                        )
                        .ok();
                    }
                    Some(PollSpec {
                        slot,
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

// -- WP PGCORE --
impl LinkSide {
    /// Plan every request of this side with `disc`, the bound connection's
    /// discovery (`None`: a legacy device, as before).
    fn set_discovery(&self, disc: Option<Discovery>) {
        *guard(&self.disc) = disc;
    }

    /// The paged device's pages, `None` on a legacy one.
    fn pages(&self) -> Option<PageSet> {
        guard(&self.disc).as_ref().map(|d| d.pages.clone())
    }

    /// This side as the adapter sees a paged device: 1.1.6-shaped calls
    /// planned into paged requests sent by the side's own client; `None` on a
    /// legacy device. `reads_only`: every SET refused here.
    fn paged_view(&self, reads_only: bool) -> Option<PagedTransport> {
        let disc = guard(&self.disc).clone()?;
        let wire = SideWire {
            client: self.client.clone(),
            handle: self.handle,
            descriptor: self.descriptor.clone(),
            reads_only,
        };
        Some(PagedTransport::with_discovery(Box::new(wire), disc))
    }

    /// The band frames held: the analyser's tap or channels changed.
    fn forget_bands(&self) {
        let mut feed = guard(&self.feed);
        for ch in 0u8..15 {
            if let Some(slot) = band_slot(ch) {
                feed.latest.remove(&slot);
            }
        }
    }

    /// Whether the device changed generation under this side: its transport
    /// said so, or the hub lists another `protocol` for it now.
    fn generation_changed(&self, listed: Option<dspi_link::messages::Proto>) -> bool {
        self.regen.load(Ordering::SeqCst) || listed != self.listed_protocol
    }
}

// -- WP PGCORE --
/// The 1.1.6 read a poll slot carries: the meter block or one band channel.
fn slot_read(feed: &Feed, slot: u8) -> Option<(u8, u16, u16)> {
    if slot == METER_SLOT {
        return Some((op::REQ_GET_STATUS, STATUS_METERS, feed.meter_len));
    }
    let ch = (0u8..15).find(|ch| band_slot(*ch) == Some(slot))?;
    let len = u16::try_from(RtaBandFrame::SIZE).ok()?;
    Some((op::REQ_RTA_GET_BANDS, u16::from(ch), len))
}

// -- WP PGCORE --
/// A paged poll frame of the 1.1.6 read `opcode`/`value`/`len` in the 1.1.6
/// shape: the frame fed through the read's own single-step plan (what
/// `paged_poll` subscribed). `None` for a frame the plan does not take.
fn reshape(pages: &PageSet, opcode: u8, value: u16, len: u16, frame: &[u8]) -> Option<Vec<u8>> {
    let mut plan = dspi_proto::paged::plan(opcode, value, LegacyBody::In { len }, pages).ok()?;
    match plan.start() {
        Next::Send(_) => match plan.feed(Answer::Data(frame)) {
            Next::Done(Outcome::Reply(b)) => Some(b),
            _ => None,
        },
        Next::Done(_) => None,
    }
}

// -- WP PGCORE --
/// The side channel's raw wire to a paged device: paged requests on the
/// session's client, `wIndex` `(page << 8) | 2`, a GET never with `wLength`
/// 0. Only [`PagedTransport::with_discovery`] uses it, so a 1.1.6-shaped
/// call never reaches it (and would be refused).
struct SideWire {
    client: LinkClient,
    handle: u8,
    descriptor: DeviceDescriptor,
    reads_only: bool,
}

impl SideWire {
    /// One paged request, a retryable answer (spec 1.3 section 8.1) tried
    /// again through the busy window.
    fn exchange(
        &self,
        dir: Dir,
        id: CmdId,
        value: u16,
        len: u16,
        data: &[u8],
    ) -> dspi_transport::Result<Vec<u8>> {
        let mut tries = 0;
        loop {
            let r = self
                .client
                .command(self.handle, dir, id.cmd(), value, id.w_index(), len, data);
            if is_retryable_13(r.status) && tries < dspi_transport::BUSY_RETRIES {
                tries = tries.saturating_add(1);
                dspi_transport::clock::sleep(dspi_transport::BUSY_BACKOFF);
                continue;
            }
            return settle_paged(id, len, r.status, r.payload);
        }
    }
}

/// A paged answer as the adapter takes it: a firmware result code is
/// `Refused` (no `SYS.LAST_RESULT` read after it), a hub's status its error.
fn settle_paged(
    id: CmdId,
    len: u16,
    status: u8,
    payload: Vec<u8>,
) -> dspi_transport::Result<Vec<u8>> {
    if (VC_ERR_UNKNOWN..=VC_ERR_LAST).contains(&status) {
        return Err(firmware_refusal(id, status));
    }
    match map_response(status, id.cmd(), &payload) {
        Some(e) => Err(e),
        None => {
            let mut payload = payload;
            payload.truncate(usize::from(len));
            Ok(payload)
        }
    }
}

impl Transport for SideWire {
    fn control_in(&mut self, _: u8, _: u16, _: u16) -> dspi_transport::Result<Vec<u8>> {
        Err(TransportError::Incompatible(PAGED_REFUSES_PAGE0.into()))
    }

    fn control_out(&mut self, _: u8, _: u16, _: &[u8]) -> dspi_transport::Result<()> {
        Err(TransportError::Incompatible(PAGED_REFUSES_PAGE0.into()))
    }

    fn descriptor(&self) -> &DeviceDescriptor {
        &self.descriptor
    }

    fn max_transfer(&self) -> usize {
        usize::try_from(self.client.hello().limits.max_payload)
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
        self.client.session_id()
    }

    fn carries_pages(&self) -> bool {
        true
    }

    fn control_in_paged(
        &mut self,
        id: CmdId,
        value: u16,
        max_len: u16,
    ) -> dspi_transport::Result<Vec<u8>> {
        if max_len == 0 || id.is_legacy() {
            return Err(TransportError::Stalled { opcode: id.cmd() });
        }
        self.exchange(Dir::Get, id, value, max_len, &[])
    }

    fn control_out_paged(
        &mut self,
        id: CmdId,
        value: u16,
        data: &[u8],
    ) -> dspi_transport::Result<()> {
        if self.reads_only {
            return Err(TransportError::Remote(format!(
                "0x{:04X}: background reads only",
                id.0
            )));
        }
        let Ok(len) = u16::try_from(data.len()) else {
            return Err(TransportError::Remote(format!(
                "{} bytes for 0x{:04X} exceed a vendor request",
                data.len(),
                id.0
            )));
        };
        if id.is_legacy() || id == dspi_proto::paged::cmds::STORE_ALL_PARAMS_CHUNK {
            return Err(TransportError::Stalled { opcode: id.cmd() });
        }
        self.exchange(Dir::Set, id, value, len, data).map(drop)
    }

    /// Up to the hub's `max_inflight` less one at once (one slot stays free
    /// for what a user waits for, as [`BackgroundTransport`]'s).
    fn control_in_batch_paged(
        &mut self,
        reads: &[dspi_transport::PagedRead],
    ) -> Vec<dspi_transport::Result<Vec<u8>>> {
        if reads.iter().any(|r| r.len == 0 || r.id.is_legacy()) {
            return reads
                .iter()
                .map(|r| self.control_in_paged(r.id, r.value, r.len))
                .collect();
        }
        let specs: Vec<dspi_link::CmdSpec> = reads
            .iter()
            .map(|r| dspi_link::CmdSpec {
                handle: self.handle,
                dir: Dir::Get,
                b_request: r.id.cmd(),
                w_value: r.value,
                w_index: r.id.w_index(),
                w_length: r.len,
                payload: Vec::new(),
            })
            .collect();
        let slots = self.client.max_inflight().saturating_sub(1).max(1);
        let answers = self.client.commands_within(&specs, slots);
        reads
            .iter()
            .zip(answers)
            .map(|(r, a)| {
                if is_retryable_13(a.status) {
                    return self.control_in_paged(r.id, r.value, r.len);
                }
                settle_paged(r.id, r.len, a.status, a.payload)
            })
            .collect()
    }
}

// -- WP PGCORE --
/// The bound device's raw Link wire as `paged::open` gets it: the
/// [`LinkTransport`] itself, which also tells the supervisor at once when it
/// finds that the device changed generation (`needs_reattach`), so the core
/// re-attaches with a new transport and a new `paged::open` instead of
/// retrying on this one (PLAN-PG section 1.1 step 8).
struct Watched {
    inner: LinkTransport,
    regen: Arc<AtomicBool>,
    tx: Sender<Msg>,
}

impl Watched {
    fn seen<T>(&self, r: dspi_transport::Result<T>) -> dspi_transport::Result<T> {
        if self.inner.needs_reattach() && !self.regen.swap(true, Ordering::SeqCst) {
            log::info!("the Link device changed generation: re-attaching");
            let _ = self.tx.send(Msg::Kick);
        }
        r
    }
}

impl Transport for Watched {
    fn control_in(&mut self, opcode: u8, value: u16, len: u16) -> dspi_transport::Result<Vec<u8>> {
        let r = self.inner.control_in(opcode, value, len);
        self.seen(r)
    }

    fn control_in_upto(
        &mut self,
        opcode: u8,
        value: u16,
        max_len: u16,
    ) -> dspi_transport::Result<Vec<u8>> {
        let r = self.inner.control_in_upto(opcode, value, max_len);
        self.seen(r)
    }

    fn control_out(&mut self, opcode: u8, value: u16, data: &[u8]) -> dspi_transport::Result<()> {
        let r = self.inner.control_out(opcode, value, data);
        self.seen(r)
    }

    fn descriptor(&self) -> &DeviceDescriptor {
        self.inner.descriptor()
    }

    fn max_transfer(&self) -> usize {
        self.inner.max_transfer()
    }

    fn notifications(&self) -> Option<Box<dyn dspi_transport::NotificationSource>> {
        self.inner.notifications()
    }

    fn caps(&self) -> TransportCaps {
        self.inner.caps()
    }

    fn session_id(&self) -> Option<u16> {
        self.inner.session_id()
    }

    fn lock(&mut self, reason: &str, timeout: Duration) -> dspi_transport::Result<()> {
        let r = self.inner.lock(reason, timeout);
        self.seen(r)
    }

    fn unlock(&mut self) {
        self.inner.unlock();
    }

    fn control_in_batch(
        &mut self,
        reads: &[dspi_transport::InRead],
    ) -> Vec<dspi_transport::Result<Vec<u8>>> {
        let r = self.inner.control_in_batch(reads);
        let _ = self.seen(Ok(()));
        r
    }

    fn carries_pages(&self) -> bool {
        self.inner.carries_pages()
    }

    fn control_in_paged(
        &mut self,
        id: CmdId,
        value: u16,
        max_len: u16,
    ) -> dspi_transport::Result<Vec<u8>> {
        let r = self.inner.control_in_paged(id, value, max_len);
        self.seen(r)
    }

    fn control_out_paged(
        &mut self,
        id: CmdId,
        value: u16,
        data: &[u8],
    ) -> dspi_transport::Result<()> {
        let r = self.inner.control_out_paged(id, value, data);
        self.seen(r)
    }

    fn control_in_batch_paged(
        &mut self,
        reads: &[dspi_transport::PagedRead],
    ) -> Vec<dspi_transport::Result<Vec<u8>>> {
        let r = self.inner.control_in_batch_paged(reads);
        let _ = self.seen(Ok(()));
        r
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

    // -- WP LK4 --
    fn background(self: Arc<Self>) -> Option<Box<dyn Transport>> {
        // -- WP PGCORE -- a paged device's reads go through the adapter
        if let Some(paged) = self.paged_view(true) {
            return Some(Box::new(paged));
        }
        Some(Box::new(BackgroundTransport { side: self }))
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
        // -- WP LK4 -- frames of a subscription sent ahead count before its
        // grant is read (the supervisor may be busy with the re-attach)
        let ahead = feed
            .meter_ahead_hz
            .filter(|_| feed.latest.contains_key(&METER_SLOT));
        let Some(hz) = feed.meter_hz.or(ahead) else {
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
        // -- WP LK4 -- (review c83 N5) also a subscription sent ahead whose
        // grant was never read (a bind that failed before it)
        if feed.meter_hz.is_some() || feed.meter_pending.is_some() || feed.meter_ahead_hz.is_some()
        {
            slots.push(METER_SLOT);
        }
        if !slots.is_empty() {
            self.client.poll_unsubscribe(self.handle, &slots);
        }
    }
}

// -- WP LK4 --
/// Background reads of a Link device without the core lock (a bind's probe
/// completing): tunnelled GETs, batches with one of the hub's
/// `max_inflight` slots left free for the commands a user waits for. Reads
/// only.
struct BackgroundTransport {
    side: Arc<LinkSide>,
}

impl BackgroundTransport {
    /// `reply` as [`Transport::control_in`] (`exact`) or
    /// [`Transport::control_in_upto`] answers it.
    fn shaped(
        opcode: u8,
        len: u16,
        exact: bool,
        reply: Vec<u8>,
    ) -> dspi_transport::Result<Vec<u8>> {
        let mut reply = reply;
        reply.truncate(usize::from(len));
        if exact && reply.len() < usize::from(len) {
            return Err(TransportError::ShortRead {
                opcode,
                wanted: usize::from(len),
                got: reply.len(),
            });
        }
        Ok(reply)
    }

    /// One GET, a busy answer (spec 8.1, refused before it ran) tried again.
    fn get(&self, opcode: u8, value: u16, len: u16) -> dspi_transport::Result<Vec<u8>> {
        let mut tries = 0;
        loop {
            let r = self.side.client.command(
                self.side.handle,
                Dir::Get,
                opcode,
                value,
                dspi_proto::VENDOR_INTERFACE,
                len,
                &[],
            );
            if busy(r.status) && tries < dspi_transport::BUSY_RETRIES {
                tries = tries.saturating_add(1);
                dspi_transport::clock::sleep(dspi_transport::BUSY_BACKOFF);
                continue;
            }
            // -- WP P1HL -- a hub's named refusal keeps its key
            return match map_response(r.status, opcode, &r.payload) {
                Some(e) => Err(e),
                None => Ok(r.payload),
            };
        }
    }
}

/// A status that says the command did not run and may go again (spec 8.1;
/// RATE_LIMITED: a slot the hub still counts for one of ours).
fn busy(status: u8) -> bool {
    use dspi_link::status as st;
    matches!(
        status,
        st::BUSY | st::BULK_LOCKED | st::CRC_ERROR | st::RATE_LIMITED
    )
}

impl Transport for BackgroundTransport {
    fn control_in(&mut self, opcode: u8, value: u16, len: u16) -> dspi_transport::Result<Vec<u8>> {
        let reply = self.get(opcode, value, len)?;
        Self::shaped(opcode, len, true, reply)
    }

    fn control_in_upto(
        &mut self,
        opcode: u8,
        value: u16,
        len: u16,
    ) -> dspi_transport::Result<Vec<u8>> {
        let reply = self.get(opcode, value, len)?;
        Self::shaped(opcode, len, false, reply)
    }

    fn control_out(&mut self, opcode: u8, _value: u16, _data: &[u8]) -> dspi_transport::Result<()> {
        Err(TransportError::Remote(format!(
            "opcode 0x{opcode:02X}: background reads only"
        )))
    }

    fn control_in_batch(
        &mut self,
        reads: &[dspi_transport::InRead],
    ) -> Vec<dspi_transport::Result<Vec<u8>>> {
        let client = &self.side.client;
        let specs: Vec<dspi_link::CmdSpec> = reads
            .iter()
            .map(|r| dspi_link::CmdSpec {
                handle: self.side.handle,
                dir: Dir::Get,
                b_request: r.opcode,
                w_value: r.value,
                w_index: dspi_proto::VENDOR_INTERFACE,
                w_length: r.len,
                payload: Vec::new(),
            })
            .collect();
        let slots = client.max_inflight().saturating_sub(1).max(1);
        let answers = client.commands_within(&specs, slots);
        reads
            .iter()
            .zip(answers)
            .map(|(r, a)| {
                let reply = if busy(a.status) {
                    self.get(r.opcode, r.value, r.len)?
                } else if let Some(e) = map_status(a.status, r.opcode) {
                    return Err(e);
                } else {
                    a.payload
                };
                Self::shaped(r.opcode, r.len, r.exact, reply)
            })
            .collect()
    }

    fn descriptor(&self) -> &DeviceDescriptor {
        &self.side.descriptor
    }

    fn caps(&self) -> TransportCaps {
        TransportCaps {
            kind: TransportKind::Link,
            chunked_bulk: false,
            remote: true,
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
        // -- WP PGCORE -- planned on a paged device
        if let Some(mut paged) = self.side.paged_view(false) {
            let r = paged.control_out(opcode, value, data);
            if opcode == op::REQ_RTA_SET_CONFIG {
                self.side.forget_bands();
            }
            return r;
        }
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
        // -- WP P1HL -- a hub's named refusal keeps its key
        match map_response(r.status, opcode, &r.payload) {
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

// -- WP SECCORE --
/// Parse a pairing link, `dspi://pair?h=<hid>&a=<host:port>&k=<key>&p=<pin>`
/// (the hub's QR code; PLAN-SEC 4.3). A hint, never a bypass: connect with
/// [`LinkPairLink::credential`] (pinned to `k`) and `hub_id`, then `link_pair`
/// with `pin` after the user has confirmed the hub name and fingerprint. A hub
/// presenting another key fails the connect with `hub_key_unverified`,
/// before any PIN goes out.
///
/// Errors: [`DspiError::InvalidArgument`] when it is not a valid pairing link.
#[cfg_attr(feature = "uniffi", uniffi::export)]
pub fn link_parse_pair_link(text: String) -> Result<LinkPairLink> {
    let link = dspi_link::credential::parse_pair_link(&text)
        .map_err(|e| DspiError::InvalidArgument { msg: e.to_string() })?;
    Ok(LinkPairLink {
        url: dspi_link::address::ws_url(&link.host, link.port, None),
        hub_key: dspi_link::credential::pin_display(&link.key),
        credential: link.credential().encode(),
        hub_id: link.hub_id,
        host: link.host,
        port: link.port,
        pin: link.pin_code,
    })
}

// -- WP SECCORE --
/// The hub key fingerprint a stored credential is pinned to
/// (`SHA256:K7Q2-M9XD-4TNB-0ZRA`), "" for a `v1` credential (no TLS yet).
#[cfg_attr(feature = "uniffi", uniffi::export)]
pub fn link_credential_hub_key(credential: String) -> String {
    Credential::parse(&credential)
        .pin
        .map(|k| dspi_link::credential::pin_display(&k))
        .unwrap_or_default()
}

// -- WP PINTLS --
/// What to store in place of `credential` when the pairing with its hub is
/// dropped: **Pair Again** (after `hub_key_changed`, `tls_refused`, ...), or a
/// refused or revoked token. For a hub that was ever pinned this is a record
/// with no token that still requires TLS: [`DspiCore::link_connect`] with it
/// reaches the hub over TLS only (a failure is `tls_required_pinned`, before
/// any PIN or token goes out), and the pairing then pins the new key. "" for a
/// `v1` token or none (such a hub never had TLS): remove the record then.
/// Owner decision 2026-11-17: a hub once pinned stays TLS-only; only
/// **Forget hub** (removing its record) allows an unencrypted connection again.
#[cfg_attr(feature = "uniffi", uniffi::export)]
pub fn link_credential_unpaired(credential: String) -> String {
    dspi_link::credential::unpaired_record(&credential)
}

// -- WP PINTLS --
/// Whether a stored credential reaches its hub over TLS only: pinned, or
/// pinned before ([`link_credential_unpaired`]'s record). For the paired-hubs
/// list: such a hub is never connected or paired unencrypted.
#[cfg_attr(feature = "uniffi", uniffi::export)]
pub fn link_credential_tls_only(credential: String) -> bool {
    Credential::parse(&credential).tls_only()
}

// -- WP PINX -- (review-fu1 N1)
/// Whether a stored credential carries a token: the hub is paired. True for a
/// `v1` token and a `v2` credential with one; false for none, the
/// [`link_credential_unpaired`] record (a hub pinned before: it asks for its
/// PIN, over TLS) and a damaged `v2` record. For the paired-hubs list and the
/// choice of a hub at launch.
#[cfg_attr(feature = "uniffi", uniffi::export)]
pub fn link_credential_has_token(credential: String) -> bool {
    !Credential::parse(&credential).token.is_empty()
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

// -- WP LK4 --
#[cfg(test)]
#[path = "link/tests_wp_lk4.rs"]
mod tests_wp_lk4;
// -- WP LKFAST --
#[cfg(test)]
#[path = "link/tests_wp_lkfast.rs"]
mod tests_wp_lkfast;
// -- WP LKFAST --
#[path = "link/hub_memory.rs"]
pub(crate) mod hub_memory;

// -- WP WB4b --
#[cfg(test)]
#[path = "link/tests_wp_wb4b.rs"]
mod tests_wp_wb4b;

// -- WP SECCORE --
#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
#[path = "link/tests_wp_seccore.rs"]
mod tests_wp_seccore;

// -- WP PINTLS --
#[cfg(test)]
#[path = "link/tests_wp_pintls.rs"]
mod tests_wp_pintls;

// -- WP PINX --
#[cfg(test)]
#[path = "link/tests_wp_pinx.rs"]
mod tests_wp_pinx;

// -- WP LKCLOSE --
#[cfg(test)]
#[path = "link/tests_wp_lkclose.rs"]
mod tests_wp_lkclose;

// -- WP LKUI1 --
#[path = "alink/system.rs"]
pub mod alink_system;

// -- WP ATQT -- the S/PDIF transport in the native System editor
#[path = "alink/system_spdif.rs"]
pub mod alink_system_spdif;

// -- WP INST0 -- Installation setup: drafts of the hub's site (MPLAN 1.4)
#[path = "alink/installation.rs"]
pub mod alink_installation;

// -- WP FEAT2 -- a room's or zone's volume
#[path = "alink/volume.rs"]
pub mod system_volume;
#[cfg(test)]
#[allow(clippy::indexing_slicing)]
#[path = "alink/tests_wp_feat2.rs"]
mod tests_wp_feat2;

// -- WP ATQT --
#[cfg(test)]
#[allow(clippy::indexing_slicing)]
#[path = "alink/tests_wp_atqt.rs"]
mod tests_wp_atqt;

// -- WP LKUI1 --
#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
#[path = "alink/tests_wp_lkui1.rs"]
mod tests_wp_lkui1;

// -- WP HWLK1F --
#[cfg(test)]
#[allow(clippy::arithmetic_side_effects, clippy::indexing_slicing)]
#[path = "alink/tests_wp_hwlk1f.rs"]
mod tests_wp_hwlk1f;

// -- WP PGCORE --
#[cfg(test)]
#[allow(clippy::arithmetic_side_effects, clippy::indexing_slicing)]
#[path = "link/tests_wp_pgcore.rs"]
mod tests_wp_pgcore;

// -- WP PGFIX --
#[cfg(test)]
#[allow(clippy::indexing_slicing)]
#[path = "link/tests_wp_pgfix.rs"]
mod tests_wp_pgfix;

// -- WP PGSENDER --
#[cfg(test)]
#[allow(clippy::indexing_slicing)]
#[path = "link/tests_wp_pgsender.rs"]
mod tests_wp_pgsender;

// -- WP LINKFIX --
#[cfg(test)]
#[path = "link/tests_wp_linkfix.rs"]
mod tests_wp_linkfix;
