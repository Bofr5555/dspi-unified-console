//! Limits and the installer lock on the hub (WP P1HL; PRO1 S1's and S4's hub
//! half; PLAN-CLIENTS section 5.9, owner decisions OD-P1i, OD-P1j, OD-P1l).
//!
//! The hub holds a limits profile per device (by serial) and, beside it, the
//! installer PIN's hash, and judges what network clients send against them, with
//! the check the clients' cores run (`dspi_session::limits`, P1LM):
//!
//! - **No profile:** nothing changes. A device behaves as before this WP.
//! - **A direct write** outside a limit (output gain, output and channel delay,
//!   master volume, a PEQ boost, the high-pass an output must keep, a limiter that
//!   must stay on below a ceiling) is refused before it reaches the device:
//!   `DENIED`, with a payload naming the refusal ([`Refusal::payload`]), the same
//!   words as the core's `limit_exceeded` notice. Admin and Control alike: an Admin
//!   widens a limit by changing the profile, not past it.
//! - **Bulk images** (`0xA1`, `0xA3`) from a Control client are refused while the
//!   device has a profile or a lock: an image carries any value past every limit
//!   (decode-and-clamp is a later option). So on such a device an import, a
//!   snapshot recall, an A/B toggle or a guarded apply that sends an image is
//!   Admin-only over Link. An Admin's image is accepted, then held to the profile
//!   as an import is (below).
//! - **The lock** (P1LK's, S4): a device is locked when a PIN is set for it here
//!   and its profile names what the lock covers (`lockedPages`). A Control client
//!   of a locked device gets the lock's scope only (volume, mute, preset:
//!   [`crate::policy::in_lock_scope`]) until it unlocks with the PIN, which lasts
//!   for its session. An Admin is never locked out: it unlocks without the PIN
//!   (OD-P1i: a venue phone is Control within the limits and the lock's scope, or
//!   a Viewer that only watches).
//! - **Held after the fact** (OD-P1j): when the device's whole state changes by
//!   something no write could be judged for (a preset slot loaded by anyone, a
//!   scene's frame-timed preset load, an Admin's image, a factory reset: a
//!   `BULK_INVALIDATED` not from the app's own session or the hub's revert), the
//!   hub reads the device back, writes every value outside the profile to the
//!   limit (a high-pass raised or added, a limiter switched on), and reports what
//!   it held: `limits.held` to every session, and in `limits.get`. A narrowed
//!   profile is held the same way at once.
//! - **The hub's own writes** obey it too: its system scenes (SW2) are refused
//!   before anything is sent when an op crosses a unit's limit (OD-P1l), and the
//!   `hub-system` thread's writes (AUSB2's keep-aligned delays, direct ops to units
//!   without the audio link) are judged like a client's.
//! - **Reverts are never refused:** a guarded apply's revert (P1HA, the hub's own
//!   `0xA1`) and its write-back (`GUARD_SESSION_ID`) pass, and the hold leaves
//!   exactly the revert's own changes alone, by their origin (the hub's session
//!   attributed the revert's `BULK_INVALIDATED`): anyone else's slot load or
//!   image right after a revert is still held.
//! - **The app's own session** (the console this hub runs in) is not judged here:
//!   its core keeps its own copy of the profile and judges itself (P1LM).
//!
//! **The PIN never leaves the hub, and the hub keeps no plain PIN**: salted per
//! device (a random salt with the serial), stretched (PBKDF2-HMAC-SHA256,
//! `dspi_session::limits::PinRecord`), stored under `device_pins` in
//! `hub-store.json`, which a diagnostics bundle removes. No message answers it.
//!
//! **A hub may pin a mode per role** (`docs/UI-MODES.md` section 4): `mode.pinned`
//! tells a client the mode its role is pinned to, if any, and whether a device is
//! locked for it (P1WB2's `FleetBackend` reads it).
//!
//! Additive to Link 1.2: `hello.caps` lists [`CAP`], answers carry `v` ([`VERSION`]).
//! The messages are in `docs/integrator/hub.md`.
#![warn(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

use std::collections::{BTreeMap, BTreeSet};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use dspi_link::frames::CmdRequest;
use dspi_link::{Dir, status};
use dspi_proto::FilterType;
use dspi_proto::generated::notify as nfy;
use dspi_proto::generated::wire::WIRE_MAX_XOVER_BANDS;
use dspi_proto::generated::{limiter, opcodes as op};
use dspi_proto::registry::ALL_OUTPUTS;
use dspi_proto::value::EqParamPacket;
use dspi_proto::wire::BulkPacket;
use dspi_session::limits::{self as shared, BandView, LimitsProfile, PinRecord, Violation};
use dspi_session::preset_file::OutOfRange;
use serde_json::{Value, json};

use crate::device::DeviceShared;
use crate::error::HubError;
use crate::hub::HubHandle;
use crate::registry::DeviceState;
use crate::router::MAX_PAYLOAD;
use crate::session::{
    Completion, GUARD_SESSION_ID, INTERNAL_SESSION_ID, LOCAL_SESSION_ID, Role, Session, SessionCtx,
    SessionId,
};

/// The `hello` cap that says this hub answers `limits.*` and `mode.*`.
pub const CAP: &str = "limits";
/// The messages' version, `v` in every answer.
pub const VERSION: u32 = 1;
/// The modes a hub may pin (`docs/UI-MODES.md`).
pub const MODES: [&str; 3] = ["simple", "advanced", "installer"];
/// The shortest and longest PIN taken.
pub const PIN_LEN: std::ops::RangeInclusive<usize> = 4..=64;
// -- WP P1DKFIX -- F4: the core's rule, one set of constants
// (`dspi_session::limits`): this many wrong PINs are free; each further one
// opens a refusal window of [`shared::LOCK_FIRST_WAIT_MS`], doubling to at
// most [`shared::LOCK_MAX_WAIT_MS`].
pub use dspi_session::limits::{LOCK_FIRST_WAIT_MS, LOCK_FREE_TRIES, LOCK_MAX_WAIT_MS};

/// PEQ bands per channel, and the first crossover band on the wire.
const PEQ_BANDS: u8 = 10;
const XOVER_BASE: u8 = 20;
/// How long a network client's write waits for the device's image when a rule
/// needs it (the high-pass, a channel delay, a linked limiter).
const IMAGE_WAIT: Duration = Duration::from_secs(2);
/// How long after a whole-state change the hold reads the device back: the
/// device settles, and a burst of changes is held once.
const HOLD_DELAY: Duration = Duration::from_millis(150);
/// How long the hold's read and each of its writes may take, queue included.
const HOLD_WAIT: Duration = Duration::from_secs(5);
/// Rows of the last hold kept per device.
const MAX_ROWS: usize = 64;
/// Devices whose profile a hub keeps.
const MAX_DEVICES: usize = 256;

fn guard<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    // Every update under these locks is whole before anything that could panic.
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

// ── What the hub keeps ─────────────────────────────────────────────────────

/// What `hub-store.json` keeps for this module: profiles and PIN hashes by
/// serial, pinned modes by role name.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct StoredLimits {
    /// The limits profiles, by serial.
    pub profiles: BTreeMap<String, LimitsProfile>,
    /// The PIN hashes, by serial. Never a PIN.
    pub pins: BTreeMap<String, PinRecord>,
    /// The mode pinned per role (`viewer`, `control`, `admin`).
    pub modes: BTreeMap<String, String>,
    // -- WP P1DKFIX -- F6
    /// The wrong-PIN count and refusal window, by serial: a restart keeps
    /// them (the window's end on the wall clock).
    pub tries: BTreeMap<String, shared::StoredTries>,
    // -- WP HUBADMIN --
    /// The recovery codes' hashes, by serial (OD-P1c: made when an Admin sets
    /// the PIN, shown that once). Never a code.
    pub recovery: BTreeMap<String, PinRecord>,
}

/// Where [`StoredLimits`] persists (`auth::StoredLimitsFile`, the hub's store).
pub trait LimitsPersist: Send + Sync {
    /// What was kept, or nothing.
    fn load(&self) -> StoredLimits;
    /// Keep this, replacing what was kept.
    fn save(&self, data: &StoredLimits) -> Result<(), HubError>;
}

#[derive(Debug, Default, Clone, Copy)]
struct PinTries {
    wrong: u32,
    until: Option<Instant>,
}

// -- WP P1DKFIX -- F4, F6
impl PinTries {
    /// The window left at `now`, if one is open.
    fn wait_left(&self, now: Instant) -> Option<Duration> {
        let until = self.until?;
        (until > now).then(|| until.saturating_duration_since(now))
    }

    /// Count a wrong PIN at `now`: the window it earns opens.
    fn count(&mut self, now: Instant) {
        self.wrong = self.wrong.saturating_add(1);
        if let Some(w) = shared::lock_window_for(self.wrong) {
            self.until = now.checked_add(w).or(self.until);
        }
    }

    /// As the store keeps it: the window's end on the wall clock.
    fn stored(&self, now: Instant, wall: std::time::SystemTime) -> shared::StoredTries {
        let left = self.wait_left(now).unwrap_or_default();
        let until_unix_ms = if left.is_zero() {
            0
        } else {
            unix_ms(wall).saturating_add(u64::try_from(left.as_millis()).unwrap_or(u64::MAX))
        };
        shared::StoredTries {
            wrong: self.wrong,
            until_unix_ms,
        }
    }

    /// From the store, at `now`: never a window longer than the longest.
    fn from_stored(t: shared::StoredTries, now: Instant, wall: std::time::SystemTime) -> Self {
        let left = Duration::from_millis(t.left_ms(unix_ms(wall)));
        Self {
            wrong: t.wrong,
            until: (!left.is_zero()).then(|| now.checked_add(left)).flatten(),
        }
    }
}

/// `t` as ms since the epoch; 0 before it.
fn unix_ms(t: std::time::SystemTime) -> u64 {
    t.duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

#[derive(Debug, Default)]
struct State {
    stored: StoredLimits,
    /// Sessions that unlocked a device, by serial.
    unlocked: BTreeMap<String, BTreeSet<SessionId>>,
    tries: BTreeMap<String, PinTries>,
    /// Devices to hold, and when.
    due: BTreeMap<String, Instant>,
    /// The last hold's rows, by serial.
    held: BTreeMap<String, Vec<String>>,
    // -- WP P2PA --
    /// Child sessions (the parameter API's) and the session each acts for: a PIN
    /// unlock of the parent is the child's too, while the parent is open.
    acts_for: BTreeMap<SessionId, SessionId>,
}

/// The hub's limits, PINs and pinned modes. A leaf lock: never held across a
/// transfer, a save or another hub lock.
pub struct HubLimits {
    state: Mutex<State>,
    persist: Option<Arc<dyn LimitsPersist>>,
    /// Keeps saves in the order of the changes; above `state`, never under it.
    saving: Mutex<()>,
    /// Wakes the `hub-system` thread to hold a device.
    wake: Mutex<Option<Sender<()>>>,
    /// -- WP FLAKE2 -- the monotonic and the wall clock together, read when the
    /// wrong-PIN windows are loaded and saved (tests set it).
    clock: Clock,
}

// -- WP FLAKE2 --
/// The time now on both clocks.
pub(crate) type Clock = Arc<dyn Fn() -> (Instant, std::time::SystemTime) + Send + Sync>;

fn real_clock() -> Clock {
    Arc::new(|| (Instant::now(), std::time::SystemTime::now()))
}

impl std::fmt::Debug for HubLimits {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HubLimits").finish_non_exhaustive()
    }
}

/// One device's profile and lock, as a check needs them.
#[derive(Debug, Clone, Default)]
pub(crate) struct DeviceLimits {
    pub(crate) profile: Option<LimitsProfile>,
    pub(crate) locked: bool,
}

fn is_locked(profile: Option<&LimitsProfile>, pin: bool) -> bool {
    pin && profile.is_some_and(|p| !p.locked_pages.is_empty())
}

impl HubLimits {
    /// Over this store (none: kept for this run only).
    pub fn new(persist: Option<Arc<dyn LimitsPersist>>) -> Self {
        Self::with_clock(persist, real_clock())
    }

    // -- WP FLAKE2 --
    /// [`Self::new`], reading the time from `clock`.
    pub(crate) fn with_clock(persist: Option<Arc<dyn LimitsPersist>>, clock: Clock) -> Self {
        let mut stored = persist.as_ref().map(|p| p.load()).unwrap_or_default();
        // -- WP P1DKFIX -- F6: the wrong-PIN windows as they were.
        let (now, wall) = clock();
        let tries: BTreeMap<String, PinTries> = std::mem::take(&mut stored.tries)
            .into_iter()
            .filter(|(s, _)| stored.pins.contains_key(s))
            .map(|(s, t)| (s, PinTries::from_stored(t, now, wall)))
            .collect();
        // -- WP HUBADMIN -- a code without its PIN unlocks nothing: dropped
        let pins = stored.pins.clone();
        stored.recovery.retain(|s, _| pins.contains_key(s));
        let stored = StoredLimits {
            profiles: stored
                .profiles
                .into_iter()
                .map(|(s, p)| {
                    let mut p = p.normalised();
                    p.serial.clone_from(&s);
                    (s, p)
                })
                .collect(),
            ..stored
        };
        Self {
            state: Mutex::new(State {
                stored,
                tries,
                ..State::default()
            }),
            persist,
            saving: Mutex::new(()),
            wake: Mutex::new(None),
            clock,
        }
    }

    /// The `hub-system` thread's wake.
    pub(crate) fn set_wake(&self, wake: Option<Sender<()>>) {
        *guard(&self.wake) = wake;
    }

    fn wake(&self) -> bool {
        guard(&self.wake)
            .as_ref()
            .is_some_and(|w| w.send(()).is_ok())
    }

    fn save(&self) -> Result<(), HubError> {
        let Some(persist) = &self.persist else {
            return Ok(());
        };
        let _in_order = guard(&self.saving);
        let data = {
            let st = guard(&self.state);
            let mut data = st.stored.clone();
            // -- WP P1DKFIX -- F6 (-- WP FLAKE2 -- on the limits' clock)
            let (now, wall) = (self.clock)();
            data.tries = st
                .tries
                .iter()
                .map(|(s, t)| (s.clone(), t.stored(now, wall)))
                .collect();
            data
        };
        persist.save(&data)
    }

    /// The profile and lock of `serial`.
    pub(crate) fn device(&self, serial: &str) -> DeviceLimits {
        let st = guard(&self.state);
        let profile = st.stored.profiles.get(serial).cloned();
        let locked = is_locked(profile.as_ref(), st.stored.pins.contains_key(serial));
        DeviceLimits { profile, locked }
    }

    /// Whether `serial` has a profile.
    pub fn has_profile(&self, serial: &str) -> bool {
        guard(&self.state).stored.profiles.contains_key(serial)
    }

    /// The profile of `serial`.
    pub fn profile(&self, serial: &str) -> Option<LimitsProfile> {
        guard(&self.state).stored.profiles.get(serial).cloned()
    }

    /// Whether `session` has unlocked `serial` (an Admin always has). -- WP P2PA -- a
    /// child session has when the session it acts for has.
    pub fn unlocked(&self, serial: &str, session: SessionId, role: Role) -> bool {
        if role == Role::Admin {
            return true;
        }
        let st = guard(&self.state);
        let parent = st.acts_for.get(&session).copied();
        st.unlocked
            .get(serial)
            .is_some_and(|s| s.contains(&session) || parent.is_some_and(|p| s.contains(&p)))
    }

    // -- WP P2PA --
    /// `child` acts for `parent` (a child session of the parameter API): `parent`'s PIN
    /// unlocks are `child`'s, until either closes ([`Self::session_closed`]).
    pub(crate) fn set_acts_for(&self, child: SessionId, parent: SessionId) {
        guard(&self.state).acts_for.insert(child, parent);
    }

    // -- WP P2PA --
    /// The session `child` acts for, while both are open.
    pub(crate) fn acts_for(&self, child: SessionId) -> Option<SessionId> {
        guard(&self.state).acts_for.get(&child).copied()
    }

    /// Whether `serial` is locked for `session`: locked, and not unlocked by it.
    pub fn locked_for(&self, serial: &str, session: SessionId, role: Role) -> bool {
        self.device(serial).locked && !self.unlocked(serial, session, role)
    }

    /// Whether a PIN is set for `serial`.
    pub fn pin_set(&self, serial: &str) -> bool {
        guard(&self.state).stored.pins.contains_key(serial)
    }

    /// Set (`Some`) or clear `serial`'s profile. Its `serial` is the device's.
    pub fn set_profile(
        &self,
        serial: &str,
        profile: Option<LimitsProfile>,
    ) -> Result<(), HubError> {
        {
            let mut st = guard(&self.state);
            match profile {
                Some(p) => {
                    if !st.stored.profiles.contains_key(serial)
                        && st.stored.profiles.len() >= MAX_DEVICES
                    {
                        return Err(HubError::Io("too many devices with a profile".into()));
                    }
                    let mut p = p.normalised();
                    p.serial = serial.to_owned();
                    st.stored.profiles.insert(serial.to_owned(), p);
                }
                None => {
                    st.stored.profiles.remove(serial);
                    st.held.remove(serial);
                }
            }
        }
        self.save()
    }

    /// Set (`Some`) or clear `serial`'s PIN: only its salted, slow hash is kept.
    /// Clearing it removes the lock and every session's unlock.
    pub fn set_pin(&self, serial: &str, pin: Option<&str>) -> Result<(), PinRefused> {
        self.set_pin_with_recovery(serial, pin).map(|_| ())
    }

    // -- WP HUBADMIN --
    /// [`Self::set_pin`], and with a new PIN a new recovery code (OD-P1c):
    /// returned this once, kept only as a salted, slow hash, and good for
    /// [`Self::unlock`] in place of the PIN until the PIN is set or cleared
    /// again. `None` when the PIN was cleared.
    pub fn set_pin_with_recovery(
        &self,
        serial: &str,
        pin: Option<&str>,
    ) -> Result<Option<String>, PinRefused> {
        let mut code = None;
        let record = match pin {
            Some(pin) => {
                if !PIN_LEN.contains(&pin.chars().count()) {
                    return Err(PinRefused::BadPin);
                }
                let salt: [u8; 16] = crate::store::random_bytes().map_err(PinRefused::Store)?;
                let code_salt: [u8; 16] =
                    crate::store::random_bytes().map_err(PinRefused::Store)?;
                let code_bytes: [u8; shared::RECOVERY_CODE_CHARS] =
                    crate::store::random_bytes().map_err(PinRefused::Store)?;
                let text = shared::recovery_code_from(&code_bytes);
                let code_rec = PinRecord::make(
                    serial,
                    &shared::normalise_recovery_code(&text),
                    &code_salt,
                    shared::PIN_ITERATIONS,
                );
                code = Some((text, code_rec));
                Some(PinRecord::make(serial, pin, &salt, shared::PIN_ITERATIONS))
            }
            None => None,
        };
        {
            let mut st = guard(&self.state);
            match record {
                Some(r) => {
                    if !st.stored.pins.contains_key(serial) && st.stored.pins.len() >= MAX_DEVICES {
                        return Err(PinRefused::Store(HubError::Io(
                            "too many devices with a PIN".into(),
                        )));
                    }
                    st.stored.pins.insert(serial.to_owned(), r);
                }
                None => {
                    st.stored.pins.remove(serial);
                }
            }
            // -- WP HUBADMIN -- the code goes with the PIN it was made for
            match &code {
                Some((_, rec)) => st.stored.recovery.insert(serial.to_owned(), rec.clone()),
                None => st.stored.recovery.remove(serial),
            };
            st.unlocked.remove(serial);
            st.tries.remove(serial);
        }
        self.save().map_err(PinRefused::Store)?;
        Ok(code.map(|(text, _)| text))
    }

    /// Unlock `serial` for `session` with `pin`, at `now`. The core's rule
    /// (WP P1DKFIX, F3/F4): every attempt is counted as wrong **before** the
    /// slow hash runs, under the state's lock, so attempts made at once all see
    /// the count; a right PIN clears it. [`LOCK_FREE_TRIES`] are free, each
    /// further one opens a refusal window ([`LOCK_FIRST_WAIT_MS`], doubling to
    /// [`LOCK_MAX_WAIT_MS`]) kept as the next allowed time, never a sleep; an
    /// attempt inside it is refused at once, nothing hashed. The count is per
    /// device, whichever session sent the PIN (F8, decided: security over
    /// availability; an Admin never needs the PIN), and it is kept in the
    /// store, so a restart does not reset it (F6).
    pub fn unlock(
        &self,
        serial: &str,
        session: SessionId,
        pin: &str,
        now: Instant,
    ) -> Result<(), PinRefused> {
        let record = {
            let mut st = guard(&self.state);
            let Some(record) = st.stored.pins.get(serial).cloned() else {
                // No PIN: nothing to unlock, every session is in.
                return Ok(());
            };
            let t = st.tries.entry(serial.to_owned()).or_default();
            if let Some(left) = t.wait_left(now) {
                return Err(PinRefused::Later(left));
            }
            t.count(now);
            // -- WP HUBADMIN -- and the recovery code, checked the same way
            (record, st.stored.recovery.get(serial).cloned())
        };
        let (record, recovery) = record;
        // Kept before the hash: a restart mid-guess keeps the count.
        let _ = self.save();
        // The slow hash, with the lock released.
        if !record.matches(serial, pin) {
            let code = shared::normalise_recovery_code(pin);
            let by_code = code.len() == shared::RECOVERY_CODE_CHARS
                && recovery.is_some_and(|r| r.matches(serial, &code));
            if !by_code {
                return Err(PinRefused::Wrong);
            }
        }
        {
            let mut st = guard(&self.state);
            st.tries.remove(serial);
            st.unlocked
                .entry(serial.to_owned())
                .or_default()
                .insert(session);
        }
        let _ = self.save();
        Ok(())
    }

    // -- WP P1DKFIX --
    /// `serial`'s wrong PINs and the refusal window left at `now`, for
    /// `limits.get`.
    pub fn tries(&self, serial: &str, now: Instant) -> (u32, Option<Duration>) {
        guard(&self.state)
            .tries
            .get(serial)
            .map_or((0, None), |t| (t.wrong, t.wait_left(now)))
    }

    /// `session` locks `serial` again.
    pub fn lock(&self, serial: &str, session: SessionId) {
        if let Some(s) = guard(&self.state).unlocked.get_mut(serial) {
            s.remove(&session);
        }
    }

    /// A session ended: its unlocks go (ids are reused). -- WP P2PA -- and so does
    /// every child's link to it, and its own to its parent.
    pub fn session_closed(&self, session: SessionId) {
        let mut st = guard(&self.state);
        for s in st.unlocked.values_mut() {
            s.remove(&session);
        }
        st.acts_for
            .retain(|child, parent| *child != session && *parent != session);
    }

    /// The mode pinned for `role`, if any.
    pub fn pinned_mode(&self, role: Role) -> Option<String> {
        guard(&self.state)
            .stored
            .modes
            .get(crate::store::role_name(role))
            .cloned()
    }

    // -- WP HUBADMIN --
    /// The mode pinned for each role (`viewer`, `control`, `admin`), `None` for
    /// the user's choice: what an Admin's Installer page shows.
    pub fn pinned_modes(&self) -> Vec<(&'static str, Option<String>)> {
        let st = guard(&self.state);
        [Role::Viewer, Role::Control, Role::Admin]
            .into_iter()
            .map(|r| {
                let name = crate::store::role_name(r);
                (name, st.stored.modes.get(name).cloned())
            })
            .collect()
    }

    /// Pin (`Some`) or unpin `role`'s mode. `false`: not a mode.
    pub fn pin_mode(&self, role: Role, mode: Option<&str>) -> Result<bool, HubError> {
        if mode.is_some_and(|m| !MODES.contains(&m)) {
            return Ok(false);
        }
        {
            let mut st = guard(&self.state);
            let key = crate::store::role_name(role).to_owned();
            match mode {
                Some(m) => st.stored.modes.insert(key, m.to_owned()),
                None => st.stored.modes.remove(&key),
            };
        }
        self.save().map(|()| true)
    }

    /// The last hold's rows for `serial`.
    pub fn held(&self, serial: &str) -> Vec<String> {
        guard(&self.state)
            .held
            .get(serial)
            .cloned()
            .unwrap_or_default()
    }

    /// Hold `serial` soon, on the `hub-system` thread. False when there is no
    /// thread to do it (the next `limits.*` message does).
    fn hold_soon(&self, serial: &str, now: Instant) -> bool {
        let at = now.checked_add(HOLD_DELAY).unwrap_or(now);
        guard(&self.state)
            .due
            .entry(serial.to_owned())
            .or_insert(at);
        self.wake()
    }

    /// The hub's revert is about to write `serial`'s captured state: a hold due
    /// for the state it replaces is dropped (the revert's own change is never
    /// held: its origin is the hub's session, [`note_packet`]).
    fn revert_begins(&self, serial: &str) {
        guard(&self.state).due.remove(serial);
    }

    /// When the `hub-system` thread must hold a device next.
    pub(crate) fn next_due(&self) -> Option<Instant> {
        guard(&self.state).due.values().min().copied()
    }

    fn take_due(&self, now: Instant) -> Vec<String> {
        let mut st = guard(&self.state);
        let due: Vec<String> = st
            .due
            .iter()
            .filter(|(_, at)| **at <= now)
            .map(|(s, _)| s.clone())
            .collect();
        for s in &due {
            st.due.remove(s);
        }
        due
    }

    fn keep_held(&self, serial: &str, rows: &[String]) {
        let rows: Vec<String> = rows.iter().take(MAX_ROWS).cloned().collect();
        guard(&self.state).held.insert(serial.to_owned(), rows);
    }
}

/// Why a PIN call was refused.
#[derive(Debug)]
pub enum PinRefused {
    /// Not a PIN this hub takes ([`PIN_LEN`]).
    BadPin,
    /// The wrong PIN.
    Wrong,
    /// Too many wrong PINs: try again after this long.
    Later(Duration),
    /// The store could not keep it.
    Store(HubError),
}

impl PinRefused {
    fn code(&self) -> &'static str {
        match self {
            PinRefused::BadPin => "bad_request",
            PinRefused::Wrong => "bad_pin",
            PinRefused::Later(_) => "retry_later",
            PinRefused::Store(_) => "internal",
        }
    }

    fn msg(&self) -> String {
        match self {
            PinRefused::BadPin => format!(
                "a PIN is {} to {} characters",
                PIN_LEN.start(),
                PIN_LEN.end()
            ),
            PinRefused::Wrong => "wrong PIN".into(),
            PinRefused::Later(d) => format!(
                "too many wrong PINs; try again in {} s",
                d.as_secs().saturating_add(1)
            ),
            PinRefused::Store(e) => e.to_string(),
        }
    }
}

// ── The device's image ─────────────────────────────────────────────────────

/// A device's bulk image, as the rules read it.
pub(crate) struct Image(BulkPacket);

fn le_f32(b: &[u8], at: usize) -> Option<f32> {
    let end = at.checked_add(4)?;
    Some(f32::from_le_bytes(b.get(at..end)?.try_into().ok()?))
}

/// `o`'s record of `stride` bytes, `off` into it.
fn rec(o: u8, stride: usize, off: usize) -> usize {
    usize::from(o).saturating_mul(stride).saturating_add(off)
}

fn view(p: &EqParamPacket) -> BandView {
    BandView {
        filter_type: p.filter_type.to_raw(),
        bypass: p.bypass,
        freq: p.freq,
    }
}

impl Image {
    /// A whole image of this build's wire format; `None` for anything else.
    pub(crate) fn decode(raw: Vec<u8>) -> Option<Self> {
        BulkPacket::decode(raw).ok().map(Self)
    }

    fn n_in(&self) -> u8 {
        self.0.header().num_input_channels
    }

    fn n_out(&self) -> u8 {
        self.0.header().num_output_channels
    }

    fn f32_in(&self, section: &str, at: usize) -> Option<f32> {
        le_f32(self.0.section(section)?, at)
    }

    fn out_gain(&self, o: u8) -> Option<f32> {
        self.f32_in("outputs", rec(o, 12, 4))
    }

    fn out_delay(&self, o: u8) -> Option<f32> {
        self.f32_in("outputs", rec(o, 12, 8))
    }

    fn master_volume(&self) -> Option<f32> {
        self.f32_in("master_volume", 0)
    }

    /// `o`'s limiter: on, link group, threshold.
    fn limiter(&self, o: u8) -> Option<(bool, u8, f32)> {
        let s = self.0.section("limiter")?;
        let r = s.get(rec(o, 12, 0)..rec(o, 12, 12))?;
        Some((*r.first()? != 0, *r.get(1)?, le_f32(r, 4)?))
    }

    /// The output whose wire channel is `ch`.
    fn output_of(&self, ch: u8) -> Option<u8> {
        ch.checked_sub(self.n_in()).filter(|o| *o < self.n_out())
    }

    fn band(&self, ch: u8, b: u8) -> Option<EqParamPacket> {
        match b.checked_sub(XOVER_BASE) {
            Some(x) => self.0.xover_band(ch, x),
            None => self.0.band(ch, b),
        }
    }

    /// Every band of `ch`, PEQ then crossover, by wire band number.
    fn bands(&self, ch: u8) -> Vec<(u8, EqParamPacket)> {
        let xo = u8::try_from(WIRE_MAX_XOVER_BANDS).unwrap_or(4);
        (0..PEQ_BANDS)
            .chain(XOVER_BASE..XOVER_BASE.saturating_add(xo))
            .filter_map(|b| Some((b, self.band(ch, b)?)))
            .collect()
    }

    fn views(&self, ch: u8, replace: Option<(u8, BandView)>) -> Vec<BandView> {
        self.bands(ch)
            .iter()
            .map(|(b, p)| match replace {
                Some((rb, with)) if rb == *b => with,
                _ => view(p),
            })
            .collect()
    }
}

// ── Judging a write ────────────────────────────────────────────────────────

/// Why the hub refused a command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// A value outside the profile.
    Limit(Violation),
    /// A Control client's command outside the lock's scope on a locked device.
    Locked,
    /// A Control client's bulk image on a device with a profile or a lock.
    Image,
    /// The device's state, which this rule needs, could not be read.
    Unknown,
}

impl Refusal {
    /// The `DENIED` answer's payload: `{"refused", "limit"?, "msg"}`. `refused` is
    /// a notice key: `limit_exceeded` (P1NT's), `installer_lock` (P1LK's; not
    /// `locked`, which is another console's device lock), `image_refused` or
    /// `limits_unknown` (`dspi_transport::HUB_REFUSALS`).
    pub fn payload(&self) -> Vec<u8> {
        let v = match self {
            Refusal::Limit(v) => json!({
                "refused": "limit_exceeded",
                "limit": v.code,
                "msg": v.msg,
            }),
            Refusal::Locked => json!({
                "refused": "installer_lock",
                "msg": "this device is locked by its installer: volume, mute and presets only",
            }),
            Refusal::Image => json!({
                "refused": "image_refused",
                "msg": "this device has a limits profile or a lock: only an Admin may write its whole state",
            }),
            Refusal::Unknown => json!({
                "refused": "limits_unknown",
                "msg": "the device's state could not be read to judge this against its limits",
            }),
        };
        v.to_string().into_bytes()
    }
}

/// What a rule needs that the caller has not given.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Need {
    Image,
}

fn f32_payload(req: &CmdRequest) -> Option<f32> {
    le_f32(&req.payload, 0)
}

fn is_bulk_write(req: &CmdRequest) -> bool {
    req.dir == Dir::Set
        && matches!(
            req.b_request,
            op::REQ_SET_ALL_PARAMS | op::REQ_SET_ALL_PARAMS_CHUNK
        )
}

/// A `REQ_SET_EQ_PARAM` payload: channel, band, the band as the rules see it,
/// its gain. `None` for one shorter than the firmware's packet, or with a
/// frequency, Q or gain that is not a number (WP P1DKFIX: refused).
fn eq_write(payload: &[u8]) -> Option<(u8, u8, BandView, f32)> {
    if payload.len() < 16 {
        return None;
    }
    if [4, 8, 12]
        .iter()
        .any(|at| le_f32(payload, *at).is_none_or(|v| !v.is_finite()))
    {
        return None;
    }
    let filter_type = *payload.get(2)?;
    let band = BandView {
        filter_type,
        bypass: *payload.get(3)? == 1,
        freq: le_f32(payload, 4)?,
    };
    Some((
        *payload.first()?,
        *payload.get(1)?,
        band,
        le_f32(payload, 12)?,
    ))
}

fn any_highpass(p: &LimitsProfile) -> bool {
    p.outputs.iter().any(|o| o.highpass_min_hz.is_some())
}

fn any_ceiling(p: &LimitsProfile) -> bool {
    p.outputs.iter().any(|o| o.limiter_ceiling_db.is_some())
}

/// A write the judge cannot read as the firmware would (a short payload, a
/// value that is not a number): refused, never passed unjudged (WP P1DKFIX,
/// F1: fail closed). The core's setters never send one.
fn malformed(code: &'static str, what: &str) -> Violation {
    Violation {
        code,
        msg: format!("a {what} write the hub cannot judge against the limits is refused"),
    }
}

/// The payload's first `f32`, finite, or the malformed refusal.
fn finite_value(req: &CmdRequest, code: &'static str, what: &str) -> Result<f32, Violation> {
    f32_payload(req)
        .filter(|v| v.is_finite())
        .ok_or_else(|| malformed(code, what))
}

/// Judge `req` against `p` with the device's image `img`. `Err(Ok(..))`: refused;
/// `Err(Err(Need::Image))`: this write needs the image, which was not given.
///
/// Every index is read as the firmware reads it (WP P1DKFIX, F1): the output or
/// channel of `SET_OUTPUT_GAIN`, `SET_OUTPUT_DELAY` and `SET_DELAY` is
/// `wValue & 0xFF` (`vendor_commands.c`), so `wValue = 0x0100` is output 0;
/// `SET_BAND_BYPASS` and `LIMITER` take `(wValue >> 8, wValue & 0xFF)`; an EQ
/// write names its channel and band in its payload. A payload the firmware
/// would act on but the judge cannot read is refused ([`malformed`]).
pub(crate) fn judge(
    p: &LimitsProfile,
    req: &CmdRequest,
    img: Option<&Image>,
) -> Result<(), Result<Violation, Need>> {
    if req.dir != Dir::Set && req.b_request != op::REQ_LIMITER {
        return Ok(());
    }
    // `vendor_last_wValue & 0xFF` and `>> 8`, as the firmware reads them.
    let [lo, hi] = req.w_value.to_le_bytes();
    let r = match (req.b_request, req.dir) {
        (op::REQ_SET_OUTPUT_GAIN, Dir::Set) => {
            finite_value(req, "out_gain", "output gain").and_then(|v| p.check_output_gain(lo, v))
        }
        (op::REQ_SET_OUTPUT_DELAY, Dir::Set) => {
            finite_value(req, "delay", "output delay").and_then(|v| p.check_delay(lo, v))
        }
        (op::REQ_SET_DELAY, Dir::Set) => {
            let v = finite_value(req, "delay", "channel delay").map_err(Ok)?;
            if p.outputs.is_empty() {
                return Ok(());
            }
            let img = img.ok_or(Err(Need::Image))?;
            match img.output_of(lo) {
                Some(o) => p.check_delay(o, v),
                None => Ok(()),
            }
        }
        (op::REQ_SET_MASTER_VOLUME, Dir::Set) => {
            finite_value(req, "master_volume", "master volume")
                .and_then(|v| p.check_master_volume(v))
        }
        (op::REQ_SET_EQ_PARAM, Dir::Set) => {
            let Some((ch, band, after, gain)) = eq_write(&req.payload) else {
                return Err(Ok(malformed("eq_boost", "EQ band")));
            };
            p.check_boost(after.filter_type, gain).map_err(Ok)?;
            if !any_highpass(p) {
                return Ok(());
            }
            let img = img.ok_or(Err(Need::Image))?;
            match img.output_of(ch) {
                Some(o) => {
                    p.check_highpass(o, &img.views(ch, None), &img.views(ch, Some((band, after))))
                }
                None => Ok(()),
            }
        }
        (op::REQ_SET_BAND_BYPASS, Dir::Set) => {
            let Some(&b) = req.payload.first() else {
                return Err(Ok(malformed("highpass", "band bypass")));
            };
            if !any_highpass(p) {
                return Ok(());
            }
            let img = img.ok_or(Err(Need::Image))?;
            let (ch, band) = (hi, lo);
            let (Some(o), Some(now)) = (img.output_of(ch), img.band(ch, band)) else {
                return Ok(());
            };
            let mut after = view(&now);
            after.bypass = b == 1;
            p.check_highpass(o, &img.views(ch, None), &img.views(ch, Some((band, after))))
        }
        (op::REQ_LIMITER, Dir::Set) => {
            let (out, idx) = (hi, u16::from(lo));
            // The firmware ignores a short payload and a NaN; any other value
            // is judged, whatever the index (a link group moves settings too).
            let Some(v) = f32_payload(req) else {
                return Err(Ok(malformed("limiter", "limiter")));
            };
            if v.is_nan() || !any_ceiling(p) {
                return Ok(());
            }
            let img = img.ok_or(Err(Need::Image))?;
            judge_limiter(p, img, out, idx, v)
        }
        _ => Ok(()),
    };
    r.map_err(Ok)
}

/// Each output's limiter (on, link group, threshold) after the firmware's
/// `limiter_set_param(out, idx, v)` on the image's state (`limiter.c`): a
/// setting moves its link group with it, `0xFF` moves every output, joining a
/// group adopts the group's settings, a link group set on every output gangs
/// each group to its first member. `None`: the image has no limiter section.
fn limiter_after(img: &Image, out: u8, idx: u16, v: f32) -> Option<Vec<(bool, u8, f32)>> {
    let n = img.n_out();
    let mut st: Vec<(bool, u8, f32)> = (0..n).map(|o| img.limiter(o)).collect::<Option<_>>()?;
    let max_group = f32::from(u8::try_from(limiter::LIMITER_LINK_GROUP_MAX).unwrap_or(u8::MAX));
    let set_one = |st: &mut Vec<(bool, u8, f32)>, k: usize| {
        let Some(l) = st.get_mut(k) else {
            return;
        };
        match idx {
            limiter::LIMITER_PARAM_ENABLED => l.0 = v != 0.0,
            limiter::LIMITER_PARAM_THRESHOLD_DB => l.2 = v,
            limiter::LIMITER_PARAM_LINK_GROUP => {
                // `(uint8_t)(clamp(v, 0, MAX) + 0.5)`
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                let g = (v.clamp(0.0, max_group) + 0.5) as u8;
                l.1 = g;
            }
            _ => {}
        }
    };
    let peer = |st: &[(bool, u8, f32)], k: usize, g: u8| {
        (0..st.len()).find(|m| *m != k && st.get(*m).is_some_and(|l| l.1 == g))
    };
    let copy = |st: &mut Vec<(bool, u8, f32)>, dst: usize, src: usize| {
        if let Some(&(on, _, thr)) = st.get(src)
            && let Some(d) = st.get_mut(dst)
        {
            d.0 = on;
            d.2 = thr;
        }
    };
    let o = usize::from(out);
    if out == ALL_OUTPUTS {
        for k in 0..st.len() {
            set_one(&mut st, k);
        }
        if idx == limiter::LIMITER_PARAM_LINK_GROUP {
            for k in 0..st.len() {
                let g = st.get(k).map_or(0, |l| l.1);
                if g == 0 {
                    continue;
                }
                if let Some(lead) = peer(&st, k, g).filter(|lead| *lead < k) {
                    copy(&mut st, k, lead);
                }
            }
        }
    } else if o >= st.len() {
        // The firmware refuses it: nothing moves.
    } else if idx == limiter::LIMITER_PARAM_LINK_GROUP {
        set_one(&mut st, o);
        let g = st.get(o).map_or(0, |l| l.1);
        if g != 0
            && let Some(p) = peer(&st, o, g)
        {
            copy(&mut st, o, p);
        }
    } else {
        let g = st.get(o).map_or(0, |l| l.1);
        for k in 0..st.len() {
            if k == o || (g != 0 && st.get(k).is_some_and(|l| l.1 == g)) {
                set_one(&mut st, k);
            }
        }
    }
    Some(st)
}

/// A limiter write of `v` to `out`'s parameter `idx`: judged on every output it
/// moves ([`limiter_after`]), as the core's `judge_limiters`: refused when it
/// takes a limiter outside its ceiling, or further outside; one that brings it
/// closer passes. An image without a limiter section fails closed.
fn judge_limiter(
    p: &LimitsProfile,
    img: &Image,
    out: u8,
    idx: u16,
    v: f32,
) -> Result<(), Violation> {
    let Some(after) = limiter_after(img, out, idx, v) else {
        return Err(malformed("limiter", "limiter"));
    };
    for (o, now) in (0..img.n_out()).zip(after) {
        let Some((on, _, thr)) = img.limiter(o) else {
            continue;
        };
        let was = (on, thr);
        let now = (now.0, now.2);
        if was == now {
            continue;
        }
        let Err(violation) = p.check_limiter(o, now.0, now.1) else {
            continue;
        };
        let worse = p.check_limiter(o, was.0, was.1).is_ok() || (was.0 && !now.0) || now.1 > was.1;
        if worse {
            return Err(violation);
        }
    }
    Ok(())
}

/// Judge `ctx`'s command for the device `dev` before it is admitted. `Err` is
/// the status and payload to answer at once. May wait for the device's image, up
/// to [`IMAGE_WAIT`], for a network session only (on its connection's reader).
pub(crate) fn admit(
    dev: &DeviceShared,
    policy: &crate::policy::Policy,
    ctx: &SessionCtx,
    req: &CmdRequest,
) -> Result<(), (u8, Vec<u8>)> {
    // The app judges itself; a guarded apply's write-back is never refused.
    if ctx.id == LOCAL_SESSION_ID || ctx.id == GUARD_SESSION_ID {
        return Ok(());
    }
    let Some(limits) = dev.router.limits() else {
        return Ok(());
    };
    // The role's own refusal comes from the router, as before.
    if !policy.mutates(req.b_request, req.dir) || !policy.allows(ctx.role, req.b_request, req.dir) {
        return Ok(());
    }
    let serial = dev.serial();
    let internal = ctx.id == INTERNAL_SESSION_ID;
    if internal && is_bulk_write(req) {
        // Nothing else of the hub's writes an image: a guarded apply's revert. A
        // hold still due is for a state the revert writes over: it goes.
        limits.revert_begins(&serial);
        return Ok(());
    }
    let d = limits.device(&serial);
    let Some(profile) = d.profile else {
        return Ok(());
    };
    let refuse = |r: Refusal| Err((status::DENIED, r.payload()));
    if !internal && ctx.role == Role::Control {
        if is_bulk_write(req) {
            return refuse(Refusal::Image);
        }
        if d.locked
            && !limits.unlocked(&serial, ctx.id, ctx.role)
            && !crate::policy::in_lock_scope(req.b_request, req.dir)
        {
            return refuse(Refusal::Locked);
        }
    }
    let image = || dev.snapshot.bulk().and_then(Image::decode);
    let first = image();
    match judge(&profile, req, first.as_ref()) {
        Ok(()) => Ok(()),
        Err(Ok(v)) => refuse(Refusal::Limit(v)),
        // The hub's own writes never wait (they may run on a device's worker).
        Err(Err(Need::Image)) if internal => Ok(()),
        Err(Err(Need::Image)) => {
            let waited = dev.snapshot_wait(IMAGE_WAIT).and_then(|_| image());
            match waited.map(|img| judge(&profile, req, Some(&img))) {
                Some(Ok(())) => Ok(()),
                Some(Err(Ok(v))) => refuse(Refusal::Limit(v)),
                Some(Err(Err(_))) | None => refuse(Refusal::Unknown),
            }
        }
    }
}

/// A device packet went past (its reader; must not block or take the hub's
/// state): a whole-state change not made by the app or the hub's revert is held
/// to the profile soon ([`HubHandle::limits_run_due`]).
pub(crate) fn note_packet(dev: &DeviceShared, origin: SessionId, packet: &[u8]) {
    if packet.get(1).copied().map(u16::from) != Some(nfy::NOTIFY_EVT_BULK_INVALIDATED) {
        return;
    }
    if matches!(
        origin,
        LOCAL_SESSION_ID | INTERNAL_SESSION_ID | GUARD_SESSION_ID
    ) {
        return;
    }
    let Some(limits) = dev.router.limits() else {
        return;
    };
    let serial = dev.serial();
    if limits.profile(&serial).is_some_and(|p| p.limits_values()) {
        limits.hold_soon(&serial, Instant::now());
    }
}

// ── Holding a device to its profile ────────────────────────────────────────

fn row(path: String, value: f32, applied: f32) -> String {
    OutOfRange {
        path,
        value: f64::from(value),
        applied: Some(f64::from(applied)),
    }
    .line()
}

fn write(handle: u8, code: u8, w_value: u16, payload: Vec<u8>) -> CmdRequest {
    CmdRequest {
        tag: 0,
        handle,
        dir: Dir::Set,
        b_request: code,
        w_value,
        w_index: dspi_proto::VENDOR_INTERFACE,
        w_length: u16::try_from(payload.len()).unwrap_or(u16::MAX),
        payload,
    }
}

fn band_write(handle: u8, ch: u8, b: u8, p: &EqParamPacket) -> CmdRequest {
    let mut p = *p;
    p.channel = ch;
    p.band = b;
    write(handle, op::REQ_SET_EQ_PARAM, 0, p.encode())
}

fn limiter_write(handle: u8, o: u8, idx: u16, v: f32) -> CmdRequest {
    let idx = idx & 0xFF;
    write(
        handle,
        op::REQ_LIMITER,
        (u16::from(o) << 8) | idx,
        v.to_le_bytes().to_vec(),
    )
}

/// The writes that hold the device's state `img` to `p`, and a row for each, in
/// the core's words (`dspi_core::limits::hold_state`): every number past a limit
/// to the limit, a limiter the profile wants on switched on, a high-pass it wants
/// raised or added.
pub(crate) fn hold_writes(
    p: &LimitsProfile,
    img: &Image,
    handle: u8,
) -> (Vec<CmdRequest>, Vec<String>) {
    let mut writes = Vec::new();
    let mut rows = Vec::new();
    for o in 0..img.n_out() {
        let Some(lim) = p.output(o) else {
            continue;
        };
        if let Some(g) = img.out_gain(o) {
            let to = p.clamp_output_gain(o, g);
            if to != g {
                rows.push(row(format!("out.gain[{o}]"), g, to));
                writes.push(write(
                    handle,
                    op::REQ_SET_OUTPUT_GAIN,
                    u16::from(o),
                    to.to_le_bytes().to_vec(),
                ));
            }
        }
        if let Some(d) = img.out_delay(o) {
            let to = p.clamp_delay(o, d);
            if to != d {
                rows.push(row(format!("out.delay[{o}]"), d, to));
                writes.push(write(
                    handle,
                    op::REQ_SET_OUTPUT_DELAY,
                    u16::from(o),
                    to.to_le_bytes().to_vec(),
                ));
            }
        }
        if let (Some(ceiling), Some((on, _, thr))) = (lim.limiter_ceiling_db, img.limiter(o)) {
            if !on {
                rows.push(format!("limit.on[{o}] off → on"));
                writes.push(limiter_write(
                    handle,
                    o,
                    limiter::LIMITER_PARAM_ENABLED,
                    1.0,
                ));
            }
            if thr > ceiling {
                rows.push(row(format!("limit.threshold[{o}]"), thr, ceiling));
                writes.push(limiter_write(
                    handle,
                    o,
                    limiter::LIMITER_PARAM_THRESHOLD_DB,
                    ceiling,
                ));
            }
        }
        if let Some(ch) = img.n_in().checked_add(o) {
            hold_highpass(p, img, o, ch, handle, &mut writes, &mut rows);
        }
    }
    if p.max_peq_boost_db.is_some() {
        let channels = img.n_in().saturating_add(img.n_out());
        for ch in 0..channels {
            for b in 0..PEQ_BANDS {
                let Some(band) = img.band(ch, b) else {
                    continue;
                };
                let to = p.clamp_boost(band.filter_type.to_raw(), band.gain_db);
                if to != band.gain_db {
                    rows.push(row(format!("eq.gain[{ch},{b}]"), band.gain_db, to));
                    let held = EqParamPacket {
                        gain_db: to,
                        ..band
                    };
                    writes.push(band_write(handle, ch, b, &held));
                }
            }
        }
    }
    if let Some(m) = img.master_volume() {
        let to = p.clamp_master_volume(m);
        if to != m && m > -128.0 {
            rows.push(row("vol.master".into(), m, to));
            writes.push(write(
                handle,
                op::REQ_SET_MASTER_VOLUME,
                0,
                to.to_le_bytes().to_vec(),
            ));
        }
    }
    (writes, rows)
}

/// Give output `o` (wire channel `ch`) the high-pass `p` asks of it, as the
/// core's `hold_highpass`: raise an active one that is steep enough, else make
/// a crossover high-pass (one already there, else a flat slot) steep enough at
/// the corner, else say it could not.
fn hold_highpass(
    p: &LimitsProfile,
    img: &Image,
    o: u8,
    ch: u8,
    handle: u8,
    writes: &mut Vec<CmdRequest>,
    rows: &mut Vec<String>,
) {
    if p.highpass_kept(o, &img.views(ch, None)) {
        return;
    }
    let Some(min_hz) = p.output(o).and_then(|l| l.highpass_min_hz) else {
        return;
    };
    let slope = p
        .output(o)
        .and_then(|l| l.highpass_min_slope_db)
        .unwrap_or(6);
    let steep = |t: u8| shared::highpass_slope(t).is_some_and(|x| x >= slope);
    let name = |b: u8| -> String {
        match b.checked_sub(XOVER_BASE) {
            Some(x) => format!("crossover {}", u16::from(x).saturating_add(1)),
            None => format!("band {}", u16::from(b).saturating_add(1)),
        }
    };
    let bands = img.bands(ch);
    if let Some((b, f)) = bands
        .iter()
        .find(|(_, f)| !f.bypass && steep(f.filter_type.to_raw()))
    {
        rows.push(row(format!("eq.freq[{ch},{b}]"), f.freq, min_hz));
        let raised = EqParamPacket { freq: min_hz, ..*f };
        writes.push(band_write(handle, ch, *b, &raised));
        return;
    }
    let xover = || bands.iter().filter(|(b, _)| *b >= XOVER_BASE);
    let slot = xover()
        .find(|(_, f)| shared::highpass_slope(f.filter_type.to_raw()).is_some())
        .or_else(|| xover().find(|(_, f)| f.filter_type.to_raw() == 0));
    let out = u16::from(o).saturating_add(1);
    match slot {
        Some((b, f)) => {
            let raw = if steep(f.filter_type.to_raw()) {
                f.filter_type.to_raw()
            } else {
                shared::lr_highpass_for(slope)
            };
            let freq = f.freq.max(min_hz);
            let hp = EqParamPacket {
                channel: ch,
                band: *b,
                filter_type: FilterType::from_raw(raw),
                bypass: false,
                freq,
                q: 0.707,
                gain_db: 0.0,
                qp: None,
            };
            writes.push(band_write(handle, ch, *b, &hp));
            rows.push(format!(
                "output {out}: a {} dB/oct high-pass at {} Hz in {}",
                shared::highpass_slope(raw).unwrap_or(slope),
                freq.round(),
                name(*b)
            ));
        }
        None => rows.push(format!(
            "output {out}: no free crossover slot for the high-pass the profile asks for; not changed"
        )),
    }
}

// ── Scenes (OD-P1l) ────────────────────────────────────────────────────────

/// The sample rate a scene's output delay (in samples) is judged at: the
/// units' usual rate, and the strictest for a longest delay.
const SCENE_RATE_HZ: f64 = 48_000.0;

/// Judge a system scene's ops against the profile and lock of each unit it
/// reaches (`units`: the serials per action, in order), for `role`. Refused
/// before anything is sent: a frame-timed scene must not play half.
pub(crate) fn judge_scene(
    limits: &HubLimits,
    scene: &crate::system::model::Scene,
    units: &[Vec<String>],
    role: Role,
) -> Result<(), String> {
    use dspi_proto::ext::alink::ApplyOp;
    for (a, serials) in scene.actions.iter().zip(units) {
        // -- WP FEAT1 -- a source switch is not volume, mute or a preset: a
        // locked unit refuses it to a control role
        if a.op == crate::system::model::SCENE_OP_SOURCE {
            if role == Role::Control
                && let Some(serial) = serials.iter().find(|s| limits.device(s).locked)
            {
                return Err(format!(
                    "op {} on {}: unit {serial} is locked by its installer (volume, mute and presets only)",
                    a.op, a.target
                ));
            }
            continue;
        }
        let Some(kind) = crate::system::model::scene_op(&a.op) else {
            continue;
        };
        for serial in serials {
            let d = limits.device(serial);
            let Some(p) = d.profile else {
                continue;
            };
            let in_scope = matches!(
                kind,
                ApplyOp::Mute | ApplyOp::Unmute | ApplyOp::PresetLoad | ApplyOp::MasterVolume
            );
            if role == Role::Control && d.locked && !in_scope {
                return Err(format!(
                    "op {} on {}: unit {serial} is locked by its installer (volume, mute and presets only)",
                    a.op, a.target
                ));
            }
            let r = match kind {
                ApplyOp::MasterVolume => p.check_master_volume(f32::from(a.value) * 0.01),
                ApplyOp::OutDelay => {
                    let ms = ((f64::from(a.value) + 0.5) * 1000.0 / SCENE_RATE_HZ) as f32;
                    p.check_delay(a.arg, ms)
                }
                _ => Ok(()),
            };
            if let Err(v) = r {
                return Err(format!(
                    "op {} on {}: unit {serial}: {} (limit_exceeded)",
                    a.op, a.target, v.msg
                ));
            }
        }
    }
    Ok(())
}

// ── On the hub ─────────────────────────────────────────────────────────────

fn internal() -> SessionCtx {
    SessionCtx {
        id: INTERNAL_SESSION_ID,
        role: Role::Admin,
        exempt_inflight: true,
    }
}

/// One command as the hub's own session, waited for.
fn exec(dev: &DeviceShared, req: CmdRequest) -> Result<Vec<u8>, u8> {
    let (tx, rx) = mpsc::channel();
    let deadline = Instant::now().checked_add(HOLD_WAIT);
    dev.submit(
        internal(),
        req,
        Completion::new(0, move |r| {
            let _ = tx.send(r);
        }),
        deadline,
    );
    match rx.recv_timeout(HOLD_WAIT.saturating_add(Duration::from_secs(5))) {
        Ok(r) if r.status == status::OK => Ok(r.payload),
        Ok(r) => Err(r.status),
        Err(_) => Err(status::TIMEOUT),
    }
}

impl HubHandle {
    fn limits(&self) -> &Arc<HubLimits> {
        &self.inner.limits
    }

    /// The serial of the online device at `handle`.
    fn limits_serial(&self, handle: u8) -> Option<String> {
        self.devices()
            .into_iter()
            .find(|d| d.handle == handle && d.state == DeviceState::Online)
            .map(|d| d.identity.serial)
    }

    /// When the `hub-system` thread must hold a device next.
    pub(crate) fn limits_next_due(&self) -> Option<Instant> {
        self.limits().next_due()
    }

    /// Hold every device that is due (the `hub-system` thread; or the caller's
    /// thread when that thread could not start).
    pub(crate) fn limits_run_due(&self) {
        for serial in self.limits().take_due(Instant::now()) {
            self.limits_hold(&serial);
        }
    }

    /// Read `serial` back and write every value outside its profile to the limit.
    /// The rows, also kept and sent as `limits.held`. Blocks: never on a device's
    /// threads.
    pub fn limits_hold(&self, serial: &str) -> Vec<String> {
        let Some(p) = self
            .limits()
            .profile(serial)
            .filter(LimitsProfile::limits_values)
        else {
            return Vec::new();
        };
        let Some(dev) = self.device_by_serial(serial) else {
            return Vec::new();
        };
        let read = CmdRequest {
            tag: 0,
            handle: dev.handle,
            dir: Dir::Get,
            b_request: op::REQ_GET_ALL_PARAMS,
            w_value: 0,
            w_index: dspi_proto::VENDOR_INTERFACE,
            w_length: u16::try_from(MAX_PAYLOAD).unwrap_or(u16::MAX),
            payload: Vec::new(),
        };
        let Some(img) = exec(&dev, read).ok().and_then(Image::decode) else {
            log::warn!("hub: {serial}'s state could not be read to hold it to its limits");
            return Vec::new();
        };
        let (writes, mut rows) = hold_writes(&p, &img, dev.handle);
        let mut failed = 0usize;
        for w in writes {
            // Past a session's lock: the hold is the hub's, as a revert is.
            dev.router.pass_hold(&w);
            if exec(&dev, w).is_err() {
                failed = failed.saturating_add(1);
            }
        }
        if failed > 0 {
            rows.push(format!("{failed} of these writes failed"));
        }
        if !rows.is_empty() {
            log::info!("hub: {serial} held to its limits: {}", rows.join(", "));
            self.limits().keep_held(serial, &rows);
            let event = json!({
                "t": "limits.held",
                "v": VERSION,
                "serial": serial,
                "handle": dev.handle,
                "rows": rows,
            });
            self.inner.relay.broadcast_event(&event.to_string());
        }
        rows
    }

    /// Tell every session that `serial`'s limits or lock changed.
    fn limits_changed(&self, serial: &str, handle: u8) {
        let event =
            json!({"t": "limits.changed", "v": VERSION, "serial": serial, "handle": handle});
        self.inner.relay.broadcast_event(&event.to_string());
    }

    /// `limits.get`'s body for `session`.
    pub fn limits_status(&self, session: &Session, handle: u8) -> Option<Value> {
        let serial = self.limits_serial(handle)?;
        let l = self.limits();
        let d = l.device(&serial);
        Some(json!({
            "serial": serial,
            "handle": handle,
            "profile": d.profile.as_ref().map(LimitsProfile::to_value),
            "pin_set": l.pin_set(&serial),
            "locked": d.locked,
            "unlocked": !l.locked_for(&serial, session.id(), session.role()),
            "held": l.held(&serial),
            // -- WP P1DKFIX -- the refusal window, for the client's prompt
            "wrong_tries": l.tries(&serial, Instant::now()).0,
            "retry_in_ms": l
                .tries(&serial, Instant::now())
                .1
                .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX)),
        }))
    }

    /// `limits.set`: an Admin sets (or, with `None`, clears) the profile of the
    /// device at `handle`. A narrowed profile is held at once (on this thread).
    pub fn limits_set(
        &self,
        session: &Session,
        handle: u8,
        profile: Option<LimitsProfile>,
    ) -> Result<Vec<String>, (&'static str, String)> {
        if !crate::policy::limits_allows(session.role(), "limits.set") {
            return Err(("denied", "admin role required".into()));
        }
        let serial = self
            .limits_serial(handle)
            .ok_or(("no_device", "no such device online".to_owned()))?;
        let set = profile.is_some();
        self.limits()
            .set_profile(&serial, profile)
            .map_err(|e| ("internal", e.to_string()))?;
        if let Some(dev) = self.device_by_serial(&serial) {
            dev.warm_snapshot();
        }
        self.limits_changed(&serial, handle);
        Ok(if set {
            self.limits_hold(&serial)
        } else {
            Vec::new()
        })
    }
}

fn err(id: Option<&Value>, code: &str, msg: &str) -> String {
    json!({"t": "err", "id": id, "code": code, "msg": msg}).to_string()
}

fn ok(id: &Value, body: Value) -> String {
    let mut out = json!({"t": "ok", "id": id, "v": VERSION});
    if let (Some(map), Value::Object(body)) = (out.as_object_mut(), body) {
        map.extend(body);
    }
    out.to_string()
}

/// A profile from a message: a limits document, a document carrying one in
/// `meta.limits`, or the profile object itself.
fn profile_of(v: &Value) -> Result<LimitsProfile, String> {
    let doc = match v {
        Value::String(text) => serde_json::from_str::<Value>(text).map_err(|e| e.to_string())?,
        other => other.clone(),
    };
    match shared::from_value(&doc) {
        Ok(p) => Ok(p),
        Err(shared::LimitsError::NoProfile) => serde_json::from_value::<LimitsProfile>(doc)
            .map(LimitsProfile::normalised)
            .map_err(|e| e.to_string()),
        Err(e) => Err(e.to_string()),
    }
}

const MESSAGES: [&str; 7] = [
    "limits.get",
    "limits.set",
    "limits.pin",
    "limits.unlock",
    "limits.lock",
    "mode.pinned",
    "mode.pin",
];

/// Answer a `limits.*` or `mode.*` message (`docs/integrator/hub.md`). `None` for
/// any other. For the connection's reader: `limits.set` holds the device, and
/// `limits.pin` and `limits.unlock` run the slow hash, on this connection alone.
pub fn handle_message(hub: &HubHandle, session: &Session, msg: &Value) -> Option<String> {
    let t = msg.get("t")?.as_str()?;
    if !MESSAGES.contains(&t) {
        return None;
    }
    // Holds the `hub-system` thread could not run (it did not start).
    hub.limits_run_due();
    let id = msg.get("id").filter(|id| id.is_i64() || id.is_u64());
    let Some(id) = id else {
        return Some(err(None, "bad_request", "id is required"));
    };
    if !crate::policy::limits_allows(session.role(), t) {
        return Some(err(Some(id), "denied", "this role may not"));
    }
    if t.starts_with("mode.") {
        return Some(mode_message(hub, session, id, t, msg));
    }
    let handle = msg
        .get("handle")
        .and_then(Value::as_u64)
        .and_then(|h| u8::try_from(h).ok());
    let Some(handle) = handle else {
        return Some(err(Some(id), "bad_request", "handle is required"));
    };
    let Some(serial) = hub.limits_serial(handle) else {
        return Some(err(Some(id), "no_device", "no such device online"));
    };
    let l = hub.limits();
    Some(match t {
        "limits.get" => match hub.limits_status(session, handle) {
            Some(body) => ok(id, body),
            None => err(Some(id), "no_device", "no such device online"),
        },
        "limits.set" => {
            let profile = match msg.get("profile") {
                None => {
                    return Some(err(
                        Some(id),
                        "bad_request",
                        "profile is required (null clears it)",
                    ));
                }
                Some(Value::Null) => None,
                Some(v) => match profile_of(v) {
                    Ok(p) => Some(p),
                    Err(e) => {
                        return Some(err(
                            Some(id),
                            "bad_request",
                            &format!("not a limits profile: {e}"),
                        ));
                    }
                },
            };
            match hub.limits_set(session, handle, profile) {
                Ok(held) => ok(id, json!({"serial": serial, "held": held})),
                Err((code, m)) => err(Some(id), code, &m),
            }
        }
        "limits.pin" => {
            let pin = match msg.get("pin") {
                Some(Value::Null) => None,
                Some(Value::String(p)) => Some(p.as_str()),
                _ => {
                    return Some(err(
                        Some(id),
                        "bad_request",
                        "pin is a string, or null to clear it",
                    ));
                }
            };
            // -- WP HUBADMIN -- a new PIN's recovery code, in this reply only
            match l.set_pin_with_recovery(&serial, pin) {
                Ok(code) => {
                    hub.limits_changed(&serial, handle);
                    let mut body = json!({"serial": serial, "pin_set": pin.is_some()});
                    if let (Some(code), Some(map)) = (code, body.as_object_mut()) {
                        map.insert("recovery_code".into(), code.into());
                    }
                    ok(id, body)
                }
                Err(e) => err(Some(id), e.code(), &e.msg()),
            }
        }
        "limits.unlock" => {
            if session.role() == Role::Admin {
                return Some(ok(id, json!({"serial": serial, "unlocked": true})));
            }
            let Some(pin) = msg.get("pin").and_then(Value::as_str) else {
                return Some(err(Some(id), "bad_request", "pin is required"));
            };
            match l.unlock(&serial, session.id(), pin, Instant::now()) {
                Ok(()) => ok(id, json!({"serial": serial, "unlocked": true})),
                Err(e) => {
                    let mut e_msg = err(Some(id), e.code(), &e.msg());
                    if let PinRefused::Later(d) = e
                        && let Ok(mut v) = serde_json::from_str::<Value>(&e_msg)
                        && let Some(map) = v.as_object_mut()
                    {
                        map.insert(
                            "retry_in_ms".into(),
                            u64::try_from(d.as_millis()).unwrap_or(u64::MAX).into(),
                        );
                        e_msg = v.to_string();
                    }
                    e_msg
                }
            }
        }
        _ => {
            l.lock(&serial, session.id());
            ok(
                id,
                json!({"serial": serial, "unlocked": !l.locked_for(&serial, session.id(), session.role())}),
            )
        }
    })
}

fn mode_message(hub: &HubHandle, session: &Session, id: &Value, t: &str, msg: &Value) -> String {
    let l = hub.limits();
    if t == "mode.pin" {
        let Some(role) = msg
            .get("role")
            .and_then(Value::as_str)
            .and_then(crate::store::parse_role)
        else {
            return err(Some(id), "bad_request", "role is viewer, control or admin");
        };
        let mode = match msg.get("mode") {
            Some(Value::Null) | None => None,
            Some(Value::String(m)) => Some(m.as_str()),
            _ => return err(Some(id), "bad_request", "mode is a string, or null"),
        };
        return match l.pin_mode(role, mode) {
            Ok(true) => {
                let event = json!({"t": "mode.changed", "v": VERSION, "role": crate::store::role_name(role), "mode": mode});
                hub.inner.relay.broadcast_event(&event.to_string());
                ok(
                    id,
                    json!({"role": crate::store::role_name(role), "mode": mode}),
                )
            }
            Ok(false) => err(
                Some(id),
                "bad_request",
                "mode is simple, advanced or installer",
            ),
            Err(e) => err(Some(id), "internal", &e.to_string()),
        };
    }
    let role = session.role();
    let mut body = json!({
        "role": crate::store::role_name(role),
        "mode": l.pinned_mode(role),
    });
    // -- WP HUBADMIN -- an Admin sees every role's pin (its Installer page)
    if role == Role::Admin
        && let Some(map) = body.as_object_mut()
    {
        let pins: serde_json::Map<String, Value> = l
            .pinned_modes()
            .into_iter()
            .map(|(r, m)| (r.to_owned(), m.map_or(Value::Null, Value::from)))
            .collect();
        map.insert("pins".into(), Value::Object(pins));
    }
    let handle = msg
        .get("handle")
        .and_then(Value::as_u64)
        .and_then(|h| u8::try_from(h).ok());
    if let (Some(handle), Some(map)) = (handle, body.as_object_mut()) {
        let locked = hub
            .limits_serial(handle)
            .is_some_and(|s| l.locked_for(&s, session.id(), role));
        map.insert("locked".into(), locked.into());
    }
    ok(id, body)
}

#[cfg(test)]
#[path = "limits/tests_wp_p1hl.rs"]
mod tests_wp_p1hl;
