//! The installer's PIN and lock (WP P1LK, PRO1 S4, `docs/UI-MODES.md`
//! sections 1 and 3).
//!
//! - **No PIN, no lock.** With no installer PIN set for a device, nothing is
//!   locked, nothing is asked, every mode is reachable and every write passes
//!   as before. A console never invents a lock.
//! - **The PIN** is set in the Installer mode ([`DspiCore::installer_pin_set`]).
//!   Its hash (PBKDF2, a random salt with the serial; P1LM's [`PinRecord`]) is
//!   kept host-side beside the device's limits profile, never in a document.
//!   Setting it also makes a **recovery code**, shown once, for the hand-over
//!   note (OD-P1c); its hash is kept as the PIN's. The last resort for a
//!   forgotten PIN and a lost code is deleting the device's folder of profile
//!   files (`docs/integrator/limits-profiles.md`). There is no "Remove lock"
//!   button.
//! - **The locked-user state:** a PIN is set, the installer has chosen what
//!   the lock covers (the profile's `lockedPages`, by the mode manifest's
//!   screen ids) and this console is not unlocked for the device. Then only
//!   the writes of [`LOCKED_USER_WRITES`] (volume, mute, preset) pass, within
//!   the limits; every other write is refused before anything is sent, with
//!   [`DspiError::InstallerLock`] `locked`. The check sits where every write
//!   to the device starts (`Inner::writable_conn`), so a new write is refused
//!   by default; the allowed setters open a [`LockScope`].
//! - **Unlocking** ([`DspiCore::installer_unlock`], with the PIN or the
//!   recovery code) lasts for this core's session, per device, until
//!   [`DspiCore::installer_lock`]. With a PIN set, the Installer pages, the
//!   profile and the PIN itself change only while unlocked
//!   (`pin_required`); a profile without a PIN is editable by anyone.
//! - **Wrong PINs:** [`LOCK_FREE_TRIES`] are free; each further one opens a
//!   refusal window ([`LOCK_FIRST_WAIT_MS`], doubling, at most
//!   [`LOCK_MAX_WAIT_MS`]) kept as the next allowed time on the clock seam
//!   (`dspi_transport::clock`), never a sleep: an attempt inside it is refused
//!   at once (`wait`), and [`LockState::retry_after_ms`] says how long.
//! - **The lock is on this console, not on the DSPi** (OD-P1d): the firmware
//!   has no store for it, so the same DSPi on another console is open. The
//!   unlock dialog says so (notice `installer_lock.scope`). It is a UI lock,
//!   not a security boundary; on a hub the hub's roles are (P1HL).
//! - A PIN check takes 0.1 to 1 s (PBKDF2): call these off the UI thread. The
//!   hash runs with the core's lock released.
//! - **On the web** there are no files and no random source in the core: no
//!   PIN is set there ([`DspiError::Unsupported`]); the hub keeps the lock of a
//!   device it manages (P1HL).
//! - **The wrong-PIN count survives a restart** (WP P1DKFIX, F6): with a
//!   limits folder named it is kept beside the PIN (`limits.tries.json`, the
//!   window's end on the wall clock, never trusted past
//!   [`LOCK_MAX_WAIT_MS`]); an unreadable count file leaves no free tries.
//! - **Naming another limits folder keeps the lock** (WP P1DKFIX, F2):
//!   [`DspiCore::limits_set_dir`] is refused (`pin_required`) while the open
//!   device's PIN is set and this console is not unlocked, and every PIN this
//!   core knows is carried to the new folder (or kept in memory with none).
//! - **A hub-managed device has one lock, the hub's** (WP P1DKFIX): on a DSPi
//!   Link session whose hub holds a PIN for the bound device
//!   ([`DspiCore::installer_lock_refresh`] reads `limits.get` and
//!   `mode.pinned`), [`LockState`] reports the hub's lock (`held_by` `hub`),
//!   [`DspiCore::installer_unlock`] and [`DspiCore::installer_lock`] go to
//!   the hub, and the console's own PIN asks nothing and refuses nothing for
//!   that device (the hub refuses over Link). A console PIN still applies to
//!   a USB-attached device, and to a hub device whose hub holds no PIN.
#![warn(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]
// The web sets no PIN: the hashing, the files and the codes are unused there.
#![cfg_attr(all(target_arch = "wasm32", target_os = "unknown"), allow(dead_code))]

use std::cell::Cell;
use std::time::Duration;

use dspi_session::limits::PinRecord;
use dspi_transport::clock::Instant;

use crate::{DspiCore, DspiError, Inner, Result};

// -- WP P1DKFIX -- one rule for the core and the hub (`dspi_session::limits`).
pub use dspi_session::limits::{LOCK_FIRST_WAIT_MS, LOCK_FREE_TRIES, LOCK_MAX_WAIT_MS};

/// The writes the locked-user state lets through (by registry path; the
/// manifest's `lockedUser.writes`).
pub const LOCKED_USER_WRITES: [&str; 5] = [
    "vol.master",
    "vol.user",
    "vol.mute",
    "out.mute",
    "preset.load",
];

/// The notice a client shows in the unlock dialog: the lock is on this
/// console, the DSPi itself is open (OD-P1d).
pub const LOCK_SCOPE_NOTICE: &str = "installer_lock.scope";

// -- WP P1DKFIX --
/// The notice the unlock dialog shows when the hub holds the lock: the hub's
/// PIN unlocks the device for this console.
pub const LOCK_HUB_NOTICE: &str = "installer_lock.hub";

// -- WP P1EFIX --
/// The locked-user banner's row, the same line in every client: its `reason`
/// is the banner's sentence ("Locked by the installer: only volume, mute and
/// presets can be changed."), its `title` a short name for it.
pub const LOCK_BANNER_NOTICE: &str = "installer_lock.banner";

/// A PIN's shortest and longest length, in characters.
const PIN_MIN_CHARS: usize = 4;
const PIN_MAX_CHARS: usize = 64;

/// The recovery code's length (-- WP HUBADMIN -- the shared rule's).
const CODE_CHARS: usize = dspi_session::limits::RECOVERY_CODE_CHARS;

thread_local! {
    /// Depth of [`LockScope`] on this thread.
    static EXEMPT: Cell<u32> = const { Cell::new(0) };
}

/// While alive, writes on this thread pass the locked-user state: the
/// setters of [`LOCKED_USER_WRITES`], and the limits hold after a slot load
/// (it only narrows). Thread-local, as `limits::unlimited`.
pub(crate) struct LockScope(());

impl LockScope {
    /// Let this thread's writes through until the scope is dropped.
    pub(crate) fn allow() -> Self {
        EXEMPT.with(|d| d.set(d.get().saturating_add(1)));
        Self(())
    }

    /// A scope for a write of registry `path` when the locked-user state
    /// allows it ([`LOCKED_USER_WRITES`]); `None` otherwise.
    pub(crate) fn for_path(path: &str) -> Option<Self> {
        LOCKED_USER_WRITES.contains(&path).then(Self::allow)
    }
}

impl Drop for LockScope {
    fn drop(&mut self) {
        EXEMPT.with(|d| d.set(d.get().saturating_sub(1)));
    }
}

fn exempt() -> bool {
    EXEMPT.with(|d| d.get() > 0)
}

/// PBKDF2 iterations for a new hash (fewer in this crate's tests, whose
/// debug builds hash slowly; a record keeps its own count).
fn iterations() -> u32 {
    if cfg!(test) {
        1_000
    } else {
        dspi_session::limits::PIN_ITERATIONS
    }
}

/// What is known of a device's PIN.
#[derive(Debug, Clone)]
enum Known {
    /// No PIN set.
    None,
    /// A PIN, and its recovery code when one is kept.
    Set {
        pin: PinRecord,
        recovery: Option<PinRecord>,
    },
    /// The PIN's file is unreadable: treated as set (fails closed); only the
    /// recovery code, if readable, opens it.
    Unreadable {
        why: String,
        recovery: Option<PinRecord>,
    },
}

impl Known {
    fn is_set(&self) -> bool {
        !matches!(self, Known::None)
    }
}

#[derive(Debug, Clone)]
struct Pins {
    serial: String,
    known: Known,
}

/// Wrong PINs for one device, and the end of its refusal window.
#[derive(Debug, Clone)]
struct Tries {
    serial: String,
    wrong: u32,
    until: Option<Instant>,
}

/// The window a wrong PIN number `wrong` opens (none for the free ones).
pub(crate) fn window_for(wrong: u32) -> Option<Duration> {
    dspi_session::limits::lock_window_for(wrong)
}

impl Tries {
    /// How long until an attempt is allowed at `now`; `None` when it is.
    fn wait_left(&self, now: Instant) -> Option<Duration> {
        let until = self.until?;
        (until > now).then(|| until.duration_since(now))
    }

    /// Count a wrong PIN at `now`, opening the window it earns.
    fn count(&mut self, now: Instant) {
        self.wrong = self.wrong.saturating_add(1);
        if let Some(w) = window_for(self.wrong) {
            self.until = now.checked_add(w).or(self.until);
        }
    }

    // -- WP P1DKFIX -- F6
    /// As kept in a file: the window's end on the wall clock.
    fn stored(&self, now: Instant) -> dspi_session::limits::StoredTries {
        let left = self.wait_left(now).unwrap_or_default();
        let until_unix_ms = if left.is_zero() {
            0
        } else {
            unix_ms().saturating_add(u64::try_from(left.as_millis()).unwrap_or(u64::MAX))
        };
        dspi_session::limits::StoredTries {
            wrong: self.wrong,
            until_unix_ms,
        }
    }

    // -- WP P1DKFIX -- F6
    /// From a file, at `now`: the window left on the wall clock, never more
    /// than the longest.
    fn from_stored(serial: &str, st: dspi_session::limits::StoredTries, now: Instant) -> Self {
        let left = Duration::from_millis(st.left_ms(unix_ms()));
        Self {
            serial: serial.to_owned(),
            wrong: st.wrong,
            until: (!left.is_zero()).then(|| now.checked_add(left)).flatten(),
        }
    }
}

// -- WP P1DKFIX -- F6
/// The wall clock, ms since the epoch (the clock seam's; 0 before it).
fn unix_ms() -> u64 {
    dspi_transport::clock::system_time()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

// -- WP P1DKFIX --
/// The hub's lock of the device bound on this console's DSPi Link session, as
/// last read ([`DspiCore::installer_lock_refresh`]).
#[derive(Debug, Clone, Default)]
pub(crate) struct HubLock {
    /// The bound device's serial.
    pub(crate) serial: String,
    /// The hub holds a PIN for it.
    pub(crate) pin_set: bool,
    /// The device is locked on the hub (a PIN, and pages it covers).
    pub(crate) locked: bool,
    /// This session may write past the lock: unlocked with the PIN, an
    /// Admin, or nothing locked.
    pub(crate) unlocked: bool,
    /// What the hub's profile locks (`lockedPages`).
    pub(crate) covers: Vec<String>,
    /// The hub's wrong PINs for the device, and the end of its window.
    pub(crate) wrong: u32,
    pub(crate) until: Option<Instant>,
    /// The installer's note and contact from the hub's profile.
    pub(crate) note: String,
    pub(crate) contact: String,
}

/// The installer's lock as this core holds it, per device serial.
#[derive(Debug, Default)]
pub(crate) struct LockBook {
    /// What is known of each device's PIN: a cache of its files, or (with no
    /// folder named) the PIN itself, for this core's lifetime.
    pins: Vec<Pins>,
    /// The devices this console is unlocked for, this session.
    unlocked: Vec<String>,
    /// The devices unlocked with the recovery code (a new PIN is due).
    recovered: Vec<String>,
    /// Wrong PINs per device.
    tries: Vec<Tries>,
    /// A mode the hub pins for a device: (serial, mode id) (P1HL).
    pub(crate) pinned: Option<(String, String)>,
    // -- WP P1DKFIX --
    /// The hub's lock of the device bound over DSPi Link, as last read.
    pub(crate) hub: Option<HubLock>,
}

impl LockBook {
    // -- WP P1DKFIX -- F2
    /// Another folder was named: what was read as "no PIN" is read again from
    /// it, but every PIN known (set, or unreadable: fails closed) is kept, so
    /// naming a folder never drops a lock. The PINs kept, for the new folder.
    pub(crate) fn carry_over(&mut self) -> Vec<(String, PinRecord, Option<PinRecord>)> {
        self.pins.retain(|p| p.known.is_set());
        self.pins
            .iter()
            .filter_map(|p| match &p.known {
                Known::Set { pin, recovery } => {
                    Some((p.serial.clone(), pin.clone(), recovery.clone()))
                }
                _ => None,
            })
            .collect()
    }

    // -- WP P1DKFIX -- F2, F6
    /// The wrong tries of the PINs `carried`, as files keep them.
    pub(crate) fn tries_of(
        &self,
        carried: &[(String, PinRecord, Option<PinRecord>)],
    ) -> Vec<dspi_session::limits::StoredTries> {
        let now = Instant::now();
        carried
            .iter()
            .map(|(serial, _, _)| self.stored_tries(serial, now))
            .collect()
    }

    // -- WP P1DKFIX -- F6
    /// `serial`'s wrong tries, as a file keeps them, at `now` (none: none).
    fn stored_tries(&self, serial: &str, now: Instant) -> dspi_session::limits::StoredTries {
        self.tries
            .iter()
            .find(|t| t.serial == serial)
            .map(|t| t.stored(now))
            .unwrap_or_default()
    }

    // -- WP P1DKFIX -- F6
    /// Take `serial`'s wrong tries from a file unless counted here already.
    fn adopt_tries(&mut self, t: Option<Tries>) {
        if let Some(t) = t
            && !self.tries.iter().any(|x| x.serial == t.serial)
        {
            self.tries.push(t);
        }
    }

    fn known(&self, serial: &str) -> Option<&Known> {
        self.pins
            .iter()
            .find(|p| p.serial == serial)
            .map(|p| &p.known)
    }

    fn put(&mut self, serial: &str, known: Known) {
        self.pins.retain(|p| p.serial != serial);
        self.pins.push(Pins {
            serial: serial.to_owned(),
            known,
        });
    }

    fn is_unlocked(&self, serial: &str) -> bool {
        self.unlocked.iter().any(|s| s == serial)
    }

    fn set_unlocked(&mut self, serial: &str, on: bool) {
        self.unlocked.retain(|s| s != serial);
        if on {
            self.unlocked.push(serial.to_owned());
        }
    }

    fn set_recovered(&mut self, serial: &str, on: bool) {
        self.recovered.retain(|s| s != serial);
        if on {
            self.recovered.push(serial.to_owned());
        }
    }

    /// Run `f` on `serial`'s wrong tries (made when missing).
    fn with_tries<R>(&mut self, serial: &str, f: impl FnOnce(&mut Tries) -> R) -> R {
        if let Some(t) = self.tries.iter_mut().find(|t| t.serial == serial) {
            return f(t);
        }
        let mut t = Tries {
            serial: serial.to_owned(),
            wrong: 0,
            until: None,
        };
        let r = f(&mut t);
        self.tries.push(t);
        r
    }

    fn reset_tries(&mut self, serial: &str) {
        self.tries.retain(|t| t.serial != serial);
    }

    /// Move every refusal window `by` into the past, for tests of its end.
    #[cfg(test)]
    pub(crate) fn age_for_test(&mut self, by: Duration) {
        for t in &mut self.tries {
            t.until = t.until.and_then(|u| u.checked_sub(by));
        }
    }
}

// -- WP P1DKFIX -- F6
/// `serial`'s wrong tries as its file holds them; an unreadable file leaves
/// no free try (fails closed).
#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
fn load_tries(store: &dspi_session::limits::LimitsStore, serial: &str) -> Option<Tries> {
    let now = Instant::now();
    match store.tries(serial) {
        Ok(None) => None,
        Ok(Some(st)) => Some(Tries::from_stored(serial, st, now)),
        Err(e) => {
            log::warn!("the installer PIN's try count is unreadable ({e}): no free try left");
            Some(Tries {
                serial: serial.to_owned(),
                wrong: LOCK_FREE_TRIES,
                until: None,
            })
        }
    }
}

// -- WP P1DKFIX -- F2
/// Write the PINs carried from another folder into `store`, each where the
/// store has none for that device (best effort: they stand in memory either
/// way), with their wrong tries.
#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
pub(crate) fn write_carried(
    store: &dspi_session::limits::LimitsStore,
    carried: &[(String, PinRecord, Option<PinRecord>)],
    tries: &[dspi_session::limits::StoredTries],
) {
    for ((serial, pin, recovery), t) in carried.iter().zip(tries) {
        if !matches!(store.pin(serial), Ok(None)) {
            continue;
        }
        let kept = store.write_pin(pin, serial).and_then(|()| match recovery {
            Some(r) => store.write_recovery(r, serial),
            None => Ok(()),
        });
        if let Err(e) = kept {
            log::warn!("an installer PIN could not be written to the new limits folder: {e}");
        }
        save_tries(Some(store), serial, t);
    }
}

// -- WP P1DKFIX -- F6
/// Keep `serial`'s wrong tries in its file (best effort: the count in memory
/// stands for this session either way).
#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
fn save_tries(
    store: Option<&dspi_session::limits::LimitsStore>,
    serial: &str,
    tries: &dspi_session::limits::StoredTries,
) {
    if let Some(store) = store
        && let Err(e) = store.write_tries(tries, serial)
    {
        log::warn!("the installer PIN's try count could not be kept: {e}");
    }
}

/// `serial`'s PIN as its files hold it (file I/O: preloaded with the core's
/// lock released, see `limits_preload`).
#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
fn load_known(store: &dspi_session::limits::LimitsStore, serial: &str) -> Known {
    let recovery = store.recovery(serial).ok().flatten();
    match store.pin(serial) {
        Ok(None) => Known::None,
        Ok(Some(pin)) => Known::Set { pin, recovery },
        Err(e) => Known::Unreadable {
            why: format!("the installer PIN's file is unreadable: {e}"),
            recovery,
        },
    }
}

impl Inner {
    /// What is known of `serial`'s PIN, read from its files once (the last
    /// resort when no preload did).
    fn pin_known(&mut self, serial: &str) -> Known {
        if let Some(k) = self.limits.lock.known(serial) {
            return k.clone();
        }
        #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
        if let Some(store) = self.limits.store() {
            let k = load_known(&store, serial);
            self.limits.lock.put(serial, k.clone());
            self.limits.lock.adopt_tries(load_tries(&store, serial)); // -- WP P1DKFIX --
            return k;
        }
        Known::None
    }

    // -- WP P1DKFIX --
    /// The hub's lock of the open device, when this console reaches it over a
    /// live DSPi Link session and the hub holds a PIN for it: the one lock
    /// that device has here.
    fn hub_lock(&self, serial: &str) -> Option<HubLock> {
        let h = self.limits.lock.hub.as_ref()?;
        if h.serial != serial || !h.pin_set || !self.hub_bound(serial) {
            return None;
        }
        Some(h.clone())
    }

    // -- WP P1DKFIX --
    /// Whether `serial` is the device bound on a live DSPi Link session.
    #[cfg(feature = "link")]
    fn hub_bound(&self, serial: &str) -> bool {
        self.link_bound_serial().is_some_and(|s| s == serial)
    }

    #[cfg(not(feature = "link"))]
    fn hub_bound(&self, _serial: &str) -> bool {
        false
    }

    /// The open device's serial, or [`DspiError::Disconnected`].
    fn lock_serial(&mut self) -> Result<String> {
        Ok(self.conn()?.serial.clone())
    }

    /// The pages the open device's lock covers (its profile's).
    fn lock_covers(&mut self) -> Vec<String> {
        self.limits_profile()
            .map(|p| p.locked_pages.clone())
            .unwrap_or_default()
    }

    /// Whether the open device is in the locked-user state on this console.
    /// On a device whose hub holds the lock: never here (the hub refuses).
    pub(crate) fn installer_locked(&mut self) -> bool {
        let Some(serial) = self.conn.as_ref().map(|c| c.serial.clone()) else {
            return false;
        };
        if self.hub_lock(&serial).is_some() {
            return false; // -- WP P1DKFIX -- the hub's lock is the one
        }
        if !self.pin_known(&serial).is_set() || self.limits.lock.is_unlocked(&serial) {
            return false;
        }
        !self.lock_covers().is_empty()
    }

    /// Whether the Installer pages are open for `serial`: no PIN, or unlocked
    /// (on the hub, when the hub holds the lock).
    fn installer_open(&mut self, serial: &str) -> bool {
        if let Some(h) = self.hub_lock(serial) {
            return h.unlocked; // -- WP P1DKFIX --
        }
        !self.pin_known(serial).is_set() || self.limits.lock.is_unlocked(serial)
    }

    /// The open device's lock as the bindings show it.
    pub(crate) fn lock_state(&mut self) -> LockState {
        let Some(serial) = self.conn.as_ref().map(|c| c.serial.clone()) else {
            return LockState {
                allowed_writes: allowed(),
                scope_notice_key: LOCK_SCOPE_NOTICE.into(),
                installer_open: true,
                ..LockState::default()
            };
        };
        // -- WP P1DKFIX -- a hub that holds the device's PIN: its lock alone.
        if let Some(h) = self.hub_lock(&serial) {
            let now = Instant::now();
            let wait = h
                .until
                .and_then(|u| (u > now).then(|| u.duration_since(now)));
            let pinned_mode = self.hub_pinned_mode(&serial);
            return LockState {
                pin_set: true,
                unlocked: h.unlocked,
                locked: h.locked && !h.unlocked,
                installer_open: h.unlocked,
                covers: h.covers,
                allowed_writes: allowed(),
                wrong_tries: h.wrong,
                retry_after_ms: wait
                    .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX)),
                recovered: false,
                note: h.note,
                contact: h.contact,
                scope_notice_key: LOCK_HUB_NOTICE.into(),
                detail: String::new(),
                held_by: HELD_BY_HUB.into(),
                hub_pinned_mode: pinned_mode,
                serial,
            };
        }
        let known = self.pin_known(&serial);
        let pin_set = known.is_set();
        let unlocked = pin_set && self.limits.lock.is_unlocked(&serial);
        let covers = self.lock_covers();
        let (note, contact) = self
            .limits_profile()
            .map(|p| (p.note.clone(), p.contact.clone()))
            .unwrap_or_default();
        let now = Instant::now();
        let (wrong, wait) = self
            .limits
            .lock
            .tries
            .iter()
            .find(|t| t.serial == serial)
            .map_or((0, None), |t| (t.wrong, t.wait_left(now)));
        LockState {
            pin_set,
            unlocked,
            locked: pin_set && !unlocked && !covers.is_empty(),
            installer_open: !pin_set || unlocked,
            covers,
            allowed_writes: allowed(),
            wrong_tries: wrong,
            retry_after_ms: wait.map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX)),
            recovered: unlocked && self.limits.lock.recovered.contains(&serial),
            note,
            contact,
            scope_notice_key: LOCK_SCOPE_NOTICE.into(),
            detail: match known {
                Known::Unreadable { why, .. } => why,
                _ => String::new(),
            },
            held_by: if pin_set { HELD_BY_CONSOLE } else { "" }.into(), // -- WP P1DKFIX --
            hub_pinned_mode: self.hub_pinned_mode(&serial),             // -- WP P1DKFIX --
            serial,
        }
    }

    // -- WP P1DKFIX --
    /// The mode the hub pins for this session's role, for `serial` (P1HL's
    /// `mode.pinned`, as last read or set); "" for none.
    fn hub_pinned_mode(&self, serial: &str) -> String {
        self.limits
            .lock
            .pinned
            .as_ref()
            .filter(|(s, _)| s == serial)
            .map(|(_, m)| m.clone())
            .unwrap_or_default()
    }
}

// -- WP P1DKFIX --
/// [`LockState::held_by`]: the console's own PIN holds the lock.
pub const HELD_BY_CONSOLE: &str = "console";
// -- WP P1DKFIX --
/// [`LockState::held_by`]: the hub this console reaches the device through
/// holds the lock (its `limits.*`).
pub const HELD_BY_HUB: &str = "hub";

fn allowed() -> Vec<String> {
    LOCKED_USER_WRITES.iter().map(|s| (*s).to_owned()).collect()
}

fn lock_error(code: &str, msg: impl Into<String>) -> DspiError {
    DspiError::InstallerLock {
        code: code.to_owned(),
        msg: msg.into(),
    }
}

/// The lock's check where every write to the device starts
/// (`Inner::writable_conn`): refused in the locked-user state, unless a
/// [`LockScope`] is open on this thread or a revert runs (`unlimited`).
pub(crate) fn lock_check(inner: &mut Inner) -> Result<()> {
    if exempt() || super::suspended() || !inner.installer_locked() {
        return Ok(());
    }
    Err(lock_error(
        "locked",
        "The installer has locked this DSPi on this console: only volume, mute and presets can be changed.",
    ))
}

// -- WP TMCOREF -- (review tmcore F2)
/// [`lock_check`] for a request that a read-only connection may still send
/// (the bootloader, so an old firmware can be updated) and so does not go
/// through `writable_conn`; it also refuses while the hub holds the device's
/// lock and this session is not unlocked, since the hub cannot be relied on
/// to answer a request that reboots the device.
pub(crate) fn lock_check_any(inner: &mut Inner) -> Result<()> {
    lock_check(inner)?;
    if exempt() || super::suspended() {
        return Ok(());
    }
    let Some(serial) = inner.conn.as_ref().map(|c| c.serial.clone()) else {
        return Ok(());
    };
    match inner.hub_lock(&serial) {
        Some(h) if h.locked && !h.unlocked => Err(lock_error(
            "hub",
            "The hub has locked this DSPi: only volume, mute and presets can be changed.",
        )),
        _ => Ok(()),
    }
}

/// Refuse an installer action (the profile, the PIN, what the lock covers)
/// while a PIN is set and this console is not unlocked for the open device.
pub(crate) fn installer_gate(inner: &mut Inner) -> Result<()> {
    let serial = inner.lock_serial()?;
    if inner.installer_open(&serial) {
        return Ok(());
    }
    Err(lock_error(
        "pin_required",
        "An installer PIN is set for this DSPi on this console: unlock first.",
    ))
}

/// `code` as it is compared: letters and digits only, upper case.
fn normalise_code(code: &str) -> String {
    // -- WP HUBADMIN -- one rule with the hub's codes
    dspi_session::limits::normalise_recovery_code(code)
}

/// A recovery code from `CODE_CHARS` random bytes, in groups of five.
fn code_from(bytes: &[u8]) -> String {
    // -- WP HUBADMIN -- one rule with the hub's codes
    dspi_session::limits::recovery_code_from(bytes)
}

pub(crate) fn check_pin_text(pin: &str) -> Result<()> {
    let n = pin.chars().count();
    if pin.trim() != pin || !(PIN_MIN_CHARS..=PIN_MAX_CHARS).contains(&n) {
        return Err(DspiError::InvalidArgument {
            msg: format!(
                "an installer PIN has {PIN_MIN_CHARS} to {PIN_MAX_CHARS} characters, with no space at either end"
            ),
        });
    }
    Ok(())
}

/// How an unlock matched.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Via {
    Pin,
    Recovery,
}

/// Check `secret` against `known` for `serial` (slow: no lock held).
fn verify(known: &Known, serial: &str, secret: &str) -> Option<Via> {
    let (pin, recovery) = match known {
        Known::None => return None,
        Known::Set { pin, recovery } => (Some(pin), recovery.as_ref()),
        Known::Unreadable { recovery, .. } => (None, recovery.as_ref()),
    };
    if pin.is_some_and(|p| p.matches(serial, secret)) {
        return Some(Via::Pin);
    }
    let code = normalise_code(secret);
    (code.len() == CODE_CHARS && recovery.is_some_and(|r| r.matches(serial, &code)))
        .then_some(Via::Recovery)
}

// ── The bindings' view ──────────────────────────────────────────────────

/// The open device's installer lock on this console
/// ([`DspiCore::installer_lock_state`]).
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LockState {
    /// The open device's serial ("" with none).
    pub serial: String,
    /// An installer PIN is set for it on this console (an unreadable PIN
    /// file counts as set; `detail` says why).
    pub pin_set: bool,
    /// A PIN is set and this console is unlocked for the device, this
    /// session.
    pub unlocked: bool,
    /// The locked-user state: a PIN is set, the lock covers something and
    /// this console is not unlocked. Only `allowed_writes` pass.
    pub locked: bool,
    /// The Installer pages, the profile and the PIN may change without a
    /// PIN prompt: no PIN is set, or the console is unlocked.
    pub installer_open: bool,
    /// What the lock covers: the mode manifest's screen ids (the profile's
    /// `lockedPages`); empty: a PIN alone locks nothing.
    pub covers: Vec<String>,
    /// The writes the locked-user state allows (registry paths).
    pub allowed_writes: Vec<String>,
    /// Wrong PINs since the last good one.
    pub wrong_tries: u32,
    /// How long until the next attempt is allowed, ms; 0: now.
    pub retry_after_ms: u64,
    /// Unlocked with the recovery code: the installer should set a new PIN.
    pub recovered: bool,
    /// The installer's note, for the locked front page.
    pub note: String,
    /// How to reach the installer.
    pub contact: String,
    /// The notice the unlock dialog shows in one line: the lock is on this
    /// console, the DSPi itself is open (`installer_lock.scope`); or, when
    /// the hub holds it, the hub's PIN unlocks it (`installer_lock.hub`).
    pub scope_notice_key: String,
    /// Why the PIN's file is unreadable, when it is.
    pub detail: String,
    // -- WP P1DKFIX --
    /// Which lock this is: `console` (this console's PIN), `hub` (the DSPi
    /// Link hub the device is reached through holds the PIN; unlocking asks
    /// the hub), or "" with no PIN. One lock, one prompt.
    pub held_by: String,
    // -- WP P1DKFIX --
    /// The mode the hub pins for this console's role on this device
    /// (`mode.pinned`), "" for none.
    pub hub_pinned_mode: String,
}

/// A PIN just set ([`DspiCore::installer_pin_set`]).
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallerPin {
    /// The recovery code (`XXXXX-XXXXX-XXXXX-XXXXX`): shown this once, for
    /// the hand-over note, never kept but as a hash (notice
    /// `installer_lock.recovery`).
    pub recovery_code: String,
    /// The lock now.
    pub state: LockState,
}

#[cfg_attr(feature = "uniffi", uniffi::export)]
impl DspiCore {
    /// The open device's installer lock on this console: whether a PIN is
    /// set, whether the locked-user state is on, what it covers and allows,
    /// the refusal window after wrong PINs, the installer's note and contact.
    /// With no device: no PIN, nothing locked.
    pub fn installer_lock_state(&self) -> LockState {
        self.lock().lock_state()
    }

    /// Unlock the open device on this console for this session with the
    /// installer's PIN or the recovery code. With no PIN set, nothing is
    /// asked and nothing changes. Slow (a PIN hash): off the UI thread.
    ///
    /// Errors: [`DspiError::Disconnected`] with no device;
    /// [`DspiError::InstallerLock`] `wrong_pin`, or `wait` while the refusal
    /// window after repeated wrong PINs is open (nothing checked;
    /// [`LockState::retry_after_ms`]).
    pub fn installer_unlock(&self, pin: String) -> Result<LockState> {
        // -- WP P1DKFIX -- the hub holds this device's lock: ask the hub.
        #[cfg(feature = "link")]
        if let Some(handle) = self.hub_lock_handle() {
            return self.hub_unlock(handle, &pin);
        }
        let (serial, known) = {
            let mut inner = self.lock();
            let serial = inner.lock_serial()?;
            let known = inner.pin_known(&serial);
            if !known.is_set() || inner.limits.lock.is_unlocked(&serial) {
                return Ok(inner.lock_state());
            }
            let now = Instant::now();
            // Counted before the check, so attempts made meanwhile see it.
            let wait = inner.limits.lock.with_tries(&serial, |t| {
                let wait = t.wait_left(now);
                if wait.is_none() {
                    t.count(now);
                }
                wait
            });
            if let Some(w) = wait {
                return Err(lock_error(
                    "wait",
                    format!(
                        "Too many wrong PINs: try again in {} s.",
                        w.as_secs().saturating_add(1)
                    ),
                ));
            }
            // -- WP P1DKFIX -- F6: the count is kept before the check runs.
            #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
            {
                let tries = inner.limits.lock.stored_tries(&serial, now);
                let store = inner.limits.store();
                drop(inner);
                save_tries(store.as_ref(), &serial, &tries);
            }
            (serial, known)
        };
        let via = verify(&known, &serial, &pin);
        let mut inner = self.lock();
        match via {
            Some(via) => {
                inner.limits.lock.reset_tries(&serial);
                inner.limits.lock.set_unlocked(&serial, true);
                inner
                    .limits
                    .lock
                    .set_recovered(&serial, via == Via::Recovery);
                let state = inner.lock_state();
                // -- WP P1DKFIX -- F6
                #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
                {
                    let store = inner.limits.store();
                    drop(inner);
                    save_tries(store.as_ref(), &serial, &Default::default());
                }
                Ok(state)
            }
            None => Err(lock_error(
                "wrong_pin",
                "That is not the installer's PIN or recovery code for this DSPi.",
            )),
        }
    }

    /// Lock the open device again on this console (the end of an unlock).
    ///
    /// Errors: [`DspiError::Disconnected`]; [`DspiError::InstallerLock`]
    /// `no_pin` when no PIN is set (nothing to lock).
    pub fn installer_lock(&self) -> Result<LockState> {
        // -- WP P1DKFIX -- the hub holds this device's lock: lock it there.
        #[cfg(feature = "link")]
        if let Some(handle) = self.hub_lock_handle() {
            let mut f = serde_json::Map::new();
            f.insert("handle".into(), handle.into());
            self.link_hub_request("limits.lock", f)?;
            return self.installer_lock_refresh();
        }
        let mut inner = self.lock();
        let serial = inner.lock_serial()?;
        if !inner.pin_known(&serial).is_set() {
            return Err(lock_error(
                "no_pin",
                "No installer PIN is set for this DSPi on this console.",
            ));
        }
        inner.limits.lock.set_unlocked(&serial, false);
        inner.limits.lock.set_recovered(&serial, false);
        Ok(inner.lock_state())
    }

    /// Set (or change) the open device's installer PIN on this console, and
    /// make a new recovery code, shown this once. The console stays unlocked
    /// for the device. Kept in the limits folder (`limits_set_dir`), else for
    /// this core's lifetime. Slow (two hashes): off the UI thread.
    ///
    /// Errors: [`DspiError::Disconnected`]; [`DspiError::InvalidArgument`]
    /// for a PIN of fewer than 4 or more than 64 characters, or with a space
    /// at either end; [`DspiError::InstallerLock`] `pin_required` while a PIN
    /// is set and the console is not unlocked; [`DspiError::Io`] when a file
    /// could not be written; [`DspiError::Unsupported`] on the web.
    pub fn installer_pin_set(&self, pin: String) -> Result<InstallerPin> {
        check_pin_text(&pin)?;
        #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
        {
            let mut inner = self.lock();
            installer_gate(&mut inner)?;
            Err(DspiError::Unsupported)
        }
        #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
        {
            use dspi_session::limits::random_bytes;
            let (serial, store) = {
                let mut inner = self.lock();
                installer_gate(&mut inner)?;
                (inner.lock_serial()?, inner.limits.store())
            };
            let io = |what: &str, e: std::io::Error| DspiError::Io {
                msg: format!("the installer PIN could not be {what}: {e}"),
            };
            let salt: [u8; 16] = random_bytes().map_err(|e| io("made", e))?;
            let code_salt: [u8; 16] = random_bytes().map_err(|e| io("made", e))?;
            let code_bytes: [u8; CODE_CHARS] = random_bytes().map_err(|e| io("made", e))?;
            let code = code_from(&code_bytes);
            let pin_rec = PinRecord::make(&serial, &pin, &salt, iterations());
            let code_rec =
                PinRecord::make(&serial, &normalise_code(&code), &code_salt, iterations());
            if let Some(store) = &store {
                store
                    .write_pin(&pin_rec, &serial)
                    .map_err(|e| io("written", e))?;
                if let Err(e) = store.write_recovery(&code_rec, &serial) {
                    // The PIN stands; a stale code must not.
                    let _ = store.clear_recovery(&serial);
                    let mut inner = self.lock();
                    inner.limits.lock.put(
                        &serial,
                        Known::Set {
                            pin: pin_rec,
                            recovery: None,
                        },
                    );
                    inner.limits.lock.set_unlocked(&serial, true);
                    return Err(DspiError::Io {
                        msg: format!(
                            "the installer PIN is set, but its recovery code could not be kept: {e}"
                        ),
                    });
                }
            }
            let mut inner = self.lock();
            inner.limits.lock.put(
                &serial,
                Known::Set {
                    pin: pin_rec,
                    recovery: Some(code_rec),
                },
            );
            inner.limits.lock.set_unlocked(&serial, true);
            inner.limits.lock.set_recovered(&serial, false);
            inner.limits.lock.reset_tries(&serial);
            let state = inner.lock_state();
            drop(inner);
            save_tries(store.as_ref(), &serial, &Default::default()); // -- WP P1DKFIX -- F6
            Ok(InstallerPin {
                recovery_code: code,
                state,
            })
        }
    }

    /// Remove the open device's installer PIN (and its recovery code) on
    /// this console: no lock, no prompt, as before a PIN was set. Nothing
    /// to do with none set.
    ///
    /// Errors: [`DspiError::Disconnected`]; [`DspiError::InstallerLock`]
    /// `pin_required` while the console is not unlocked; [`DspiError::Io`]
    /// when a file could not be removed.
    pub fn installer_pin_clear(&self) -> Result<LockState> {
        let (serial, _store) = {
            let mut inner = self.lock();
            installer_gate(&mut inner)?;
            let serial = inner.lock_serial()?;
            #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
            let store = inner.limits.store();
            #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
            let store: Option<()> = None;
            (serial, store)
        };
        #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
        if let Some(store) = &_store {
            let io = |e: std::io::Error| DspiError::Io {
                msg: format!("the installer PIN could not be removed: {e}"),
            };
            store.clear_pin(&serial).map_err(io)?;
            store.clear_recovery(&serial).map_err(io)?;
            store.clear_tries(&serial).map_err(io)?; // -- WP P1DKFIX -- F6
        }
        let mut inner = self.lock();
        inner.limits.lock.put(&serial, Known::None);
        inner.limits.lock.set_unlocked(&serial, false);
        inner.limits.lock.set_recovered(&serial, false);
        inner.limits.lock.reset_tries(&serial);
        Ok(inner.lock_state())
    }

    /// Choose what the open device's lock covers: screen ids of the mode
    /// manifest (`front` and the rest; empty: the PIN locks nothing). Kept in
    /// the device's limits profile (`lockedPages`), which is created when
    /// there is none; the device is held to the profile as
    /// [`DspiCore::limits_apply_profile`] does.
    ///
    /// Errors: [`DspiError::Disconnected`]; [`DspiError::InvalidArgument`] for
    /// an id the manifest does not have; [`DspiError::InstallerLock`]
    /// `pin_required`; [`DspiError::Io`] as `limits_apply_profile`.
    pub fn installer_lock_cover(&self, pages: Vec<String>) -> Result<LockState> {
        let known = crate::modes::screen_ids();
        if let Some(bad) = pages.iter().find(|p| !known.contains(&p.as_str())) {
            return Err(DspiError::InvalidArgument {
                msg: format!("\"{bad}\" is not a screen of the mode manifest"),
            });
        }
        let mut profile = {
            let mut inner = self.lock();
            installer_gate(&mut inner)?;
            inner.limits_profile().cloned().unwrap_or_default()
        };
        let mut pages = pages;
        pages.dedup();
        profile.locked_pages = pages;
        self.limits_apply_profile(profile.to_document())?;
        Ok(self.lock().lock_state())
    }
}

// -- WP P1DKFIX --
#[cfg_attr(feature = "uniffi", uniffi::export)]
impl DspiCore {
    /// Read the lock of the device bound on this console's DSPi Link session
    /// from its hub (`limits.get`, and the mode the hub pins for this role:
    /// `mode.pinned`), and return the one lock state the console shows: the
    /// hub's when the hub holds a PIN for the device (`held_by` `hub`; then
    /// [`DspiCore::installer_unlock`] asks the hub and this console's own PIN
    /// asks nothing), this console's otherwise. Call it after binding a hub
    /// device and when the hub says its limits changed. With no hub session,
    /// or a hub without `limits`, nothing is asked: the console's own lock.
    /// Slow (a round trip): off the UI thread.
    ///
    /// Errors: [`DspiError::Link`] when the hub could not answer.
    pub fn installer_lock_refresh(&self) -> Result<LockState> {
        #[cfg(feature = "link")]
        {
            let Some((handle, serial)) = self.link_limits_target() else {
                self.lock().limits.lock.hub = None;
                return Ok(self.lock().lock_state());
            };
            let mut f = serde_json::Map::new();
            f.insert("handle".into(), handle.into());
            let got = self.link_hub_request("limits.get", f.clone())?;
            let pinned = self.link_hub_request("mode.pinned", f)?;
            let hub = hub_lock_from(&serial, &got);
            let mode = pinned
                .get("mode")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned);
            let mut inner = self.lock();
            if inner.conn.as_ref().is_some_and(|c| c.serial == serial) {
                inner.limits.lock.hub = Some(hub);
                inner.limits.lock.pinned = mode.map(|m| (serial, m));
            }
            Ok(inner.lock_state())
        }
        #[cfg(not(feature = "link"))]
        {
            Ok(self.lock().lock_state())
        }
    }
}

// -- WP P1DKFIX --
/// The hub's lock from its `limits.get` answer (`docs/integrator/hub.md`).
#[cfg(feature = "link")]
fn hub_lock_from(serial: &str, v: &serde_json::Value) -> HubLock {
    use serde_json::Value;
    let flag = |k: &str| v.get(k).and_then(Value::as_bool).unwrap_or(false);
    let profile = v.get("profile").filter(|p| p.is_object());
    let text = |k: &str| {
        profile
            .and_then(|p| p.get(k))
            .and_then(Value::as_str)
            .map(|t| t.chars().take(500).collect::<String>())
            .unwrap_or_default()
    };
    let covers = profile
        .and_then(|p| p.get("lockedPages"))
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .take(64)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default();
    let retry = v.get("retry_in_ms").and_then(Value::as_u64).unwrap_or(0);
    let pin_set = flag("pin_set");
    HubLock {
        serial: serial.to_owned(),
        pin_set,
        locked: flag("locked"),
        // Unknown counts as locked (fails closed) when a PIN is set.
        unlocked: v
            .get("unlocked")
            .and_then(Value::as_bool)
            .unwrap_or(!pin_set),
        covers,
        wrong: v
            .get("wrong_tries")
            .and_then(Value::as_u64)
            .and_then(|n| u32::try_from(n).ok())
            .unwrap_or(0),
        until: (retry > 0)
            .then(|| Instant::now().checked_add(Duration::from_millis(retry.min(LOCK_MAX_WAIT_MS))))
            .flatten(),
        note: text("note"),
        contact: text("contact"),
    }
}

// -- WP P1DKFIX --
#[cfg(feature = "link")]
impl DspiCore {
    /// The hub handle of the open device when its hub holds its lock.
    fn hub_lock_handle(&self) -> Option<u8> {
        let (handle, serial) = self.link_limits_target()?;
        let inner = self.lock();
        inner.hub_lock(&serial).map(|_| handle)
    }

    /// Unlock on the hub with `pin` (`limits.unlock`), then read its lock
    /// again. The hub's refusals in the console's words: `wrong_pin`, `wait`.
    fn hub_unlock(&self, handle: u8, pin: &str) -> Result<LockState> {
        let mut f = serde_json::Map::new();
        f.insert("handle".into(), handle.into());
        f.insert("pin".into(), pin.into());
        let r = self.link_hub_request("limits.unlock", f);
        let state = self.installer_lock_refresh();
        match r {
            Ok(_) => state,
            Err(DspiError::Link { code, msg }) => Err(match code.as_str() {
                "bad_pin" => lock_error(
                    "wrong_pin",
                    "That is not the installer's PIN for this DSPi on its hub.",
                ),
                "retry_later" => lock_error("wait", msg),
                _ => DspiError::Link { code, msg },
            }),
            Err(e) => Err(e),
        }
    }
}

impl DspiCore {
    /// Read the open device's PIN files with the core's lock released (the
    /// profile's preload does the same for the profile).
    pub(crate) fn lock_preload(&self) {
        #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
        {
            let (store, serial) = {
                let inner = self.lock();
                let Some(store) = inner.limits.store() else {
                    return;
                };
                let Some(serial) = inner.conn.as_ref().map(|c| c.serial.clone()) else {
                    return;
                };
                if inner.limits.lock.known(&serial).is_some() {
                    return;
                }
                (store, serial)
            };
            let known = load_known(&store, &serial);
            let tries = load_tries(&store, &serial); // -- WP P1DKFIX -- F6
            let mut inner = self.lock();
            if inner.limits.store().as_ref() == Some(&store)
                && inner.limits.lock.known(&serial).is_none()
            {
                inner.limits.lock.put(&serial, known);
                inner.limits.lock.adopt_tries(tries);
            }
        }
    }
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects, clippy::unwrap_used)]
mod tests_pure {
    use super::*;

    #[test]
    fn wp_p1lk_the_window_opens_after_the_free_tries_and_doubles() {
        for n in 0..=LOCK_FREE_TRIES {
            assert_eq!(window_for(n), None, "{n}");
        }
        let ms = |n| window_for(n).map(|d| d.as_millis() as u64);
        assert_eq!(ms(LOCK_FREE_TRIES + 1), Some(LOCK_FIRST_WAIT_MS));
        assert_eq!(ms(LOCK_FREE_TRIES + 2), Some(2 * LOCK_FIRST_WAIT_MS));
        assert_eq!(ms(LOCK_FREE_TRIES + 3), Some(4 * LOCK_FIRST_WAIT_MS));
        assert_eq!(ms(u32::MAX), Some(LOCK_MAX_WAIT_MS));
        assert_eq!(ms(LOCK_FREE_TRIES + 60), Some(LOCK_MAX_WAIT_MS));
    }

    #[test]
    fn wp_p1lk_tries_keep_the_next_allowed_time_never_sleep() {
        let t0 = Instant::now();
        let mut t = Tries {
            serial: "S".into(),
            wrong: 0,
            until: None,
        };
        for _ in 0..LOCK_FREE_TRIES {
            t.count(t0);
            assert_eq!(t.wait_left(t0), None);
        }
        t.count(t0);
        let first = Duration::from_millis(LOCK_FIRST_WAIT_MS);
        assert_eq!(t.wait_left(t0), Some(first));
        let later = t0 + first - Duration::from_millis(1);
        assert_eq!(t.wait_left(later), Some(Duration::from_millis(1)));
        assert_eq!(t.wait_left(t0 + first), None);
    }

    #[test]
    fn wp_p1lk_recovery_codes_read_back_however_typed() {
        let code = code_from(&[
            0u8, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 255,
        ]);
        assert_eq!(code, "ABCDE-FGHJK-LMNPQ-RSTU9");
        assert_eq!(
            normalise_code("abcde fghjk-lmnpq rstu9"),
            "ABCDEFGHJKLMNPQRSTU9"
        );
        let rec = PinRecord::make("S", "ABCDEFGHJKLMNPQRSTU9", &[1; 16], 10);
        let known = Known::Set {
            pin: PinRecord::make("S", "1234", &[2; 16], 10),
            recovery: Some(rec),
        };
        assert_eq!(verify(&known, "S", "1234"), Some(Via::Pin));
        assert_eq!(
            verify(&known, "S", "abcde-fghjk-lmnpq-rstu9"),
            Some(Via::Recovery)
        );
        assert_eq!(verify(&known, "S", "4321"), None);
        assert_eq!(verify(&known, "OTHER", "1234"), None);
        assert_eq!(verify(&Known::None, "S", "1234"), None);
    }

    #[test]
    fn wp_p1lk_pin_text_rules() {
        assert!(check_pin_text("1234").is_ok());
        assert!(check_pin_text("123").is_err());
        assert!(check_pin_text(" 1234").is_err());
        assert!(check_pin_text(&"9".repeat(65)).is_err());
        assert!(check_pin_text("åäöü").is_ok());
    }
}
