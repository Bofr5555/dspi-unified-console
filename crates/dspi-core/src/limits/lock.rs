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

/// Wrong PINs allowed before the refusal window opens.
pub const LOCK_FREE_TRIES: u32 = 3;

/// The first refusal window, ms; each further wrong PIN doubles it.
pub const LOCK_FIRST_WAIT_MS: u64 = 30_000;

/// The longest refusal window, ms.
pub const LOCK_MAX_WAIT_MS: u64 = 15 * 60_000;

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

/// A PIN's shortest and longest length, in characters.
const PIN_MIN_CHARS: usize = 4;
const PIN_MAX_CHARS: usize = 64;

/// The recovery code's alphabet (no I, O, 0 or 1) and length.
const CODE_ALPHABET: &[u8; 32] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
const CODE_CHARS: usize = 20;

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
    let past = wrong.checked_sub(LOCK_FREE_TRIES)?.checked_sub(1)?;
    let factor = 1u64.checked_shl(past.min(20)).unwrap_or(u64::MAX);
    let ms = LOCK_FIRST_WAIT_MS
        .saturating_mul(factor)
        .min(LOCK_MAX_WAIT_MS);
    Some(Duration::from_millis(ms))
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
}

impl LockBook {
    /// Forget what was read from files (another folder was named): PINs kept
    /// in memory before a folder was named go with them.
    pub(crate) fn forget_files(&mut self) {
        self.pins.clear();
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
            return k;
        }
        Known::None
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
    pub(crate) fn installer_locked(&mut self) -> bool {
        let Some(serial) = self.conn.as_ref().map(|c| c.serial.clone()) else {
            return false;
        };
        if !self.pin_known(&serial).is_set() || self.limits.lock.is_unlocked(&serial) {
            return false;
        }
        !self.lock_covers().is_empty()
    }

    /// Whether the Installer pages are open for `serial`: no PIN, or unlocked.
    fn installer_open(&mut self, serial: &str) -> bool {
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
            serial,
        }
    }
}

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
    code.chars()
        .filter(char::is_ascii_alphanumeric)
        .map(|c| c.to_ascii_uppercase())
        .collect()
}

/// A recovery code from `CODE_CHARS` random bytes, in groups of five.
fn code_from(bytes: &[u8]) -> String {
    let chars: Vec<char> = bytes
        .iter()
        .take(CODE_CHARS)
        .map(|b| {
            let i = usize::from(*b & 0x1F); // 32 letters: unbiased
            CODE_ALPHABET.get(i).map_or('A', |c| char::from(*c))
        })
        .collect();
    chars
        .chunks(5)
        .map(|c| c.iter().collect::<String>())
        .collect::<Vec<_>>()
        .join("-")
}

fn check_pin_text(pin: &str) -> Result<()> {
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
    /// console, the DSPi itself is open (`installer_lock.scope`).
    pub scope_notice_key: String,
    /// Why the PIN's file is unreadable, when it is.
    pub detail: String,
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
                Ok(inner.lock_state())
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
            Ok(InstallerPin {
                recovery_code: code,
                state: inner.lock_state(),
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
            let mut inner = self.lock();
            if inner.limits.store().as_ref() == Some(&store)
                && inner.limits.lock.known(&serial).is_none()
            {
                inner.limits.lock.put(&serial, known);
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
