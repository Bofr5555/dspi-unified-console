//! WP P1DKFIX: review p1d-core's fixups of the installer lock (F2, F6, F7,
//! F10), its probes adopted as tests of the fixed behaviour. Files live under
//! `std::env::temp_dir()`, which `ci/run.sh` points at the home volume (never
//! tmpfs).

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use dspi_session::limits::{LimitsStore, PIN_FILE, StoredTries, TRIES_FILE};

use super::*;

const PIN: &str = "pin-Zq7-p1dkfix";

fn demo() -> Arc<DspiCore> {
    let core = DspiCore::new();
    core.open_demo().unwrap();
    core
}

fn dir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join("wp-p1dkfix-core").join(format!(
        "{name}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&d);
    d
}

fn code_of(e: &DspiError) -> &str {
    match e {
        DspiError::InstallerLock { code, .. } => code,
        _ => "",
    }
}

fn locked(core: &DspiCore) {
    core.installer_pin_set(PIN.into()).unwrap();
    core.installer_lock_cover(vec!["front".into()]).unwrap();
    let st = core.installer_lock().unwrap();
    assert!(st.locked && st.held_by == HELD_BY_CONSOLE, "{st:?}");
}

fn path(d: &std::path::Path) -> String {
    d.to_string_lossy().into_owned()
}

/// F2: a locked user cannot name the lock away (PIN kept in memory).
#[test]
fn wp_p1dkfix_limits_set_dir_is_refused_while_locked_pin_in_memory() {
    let core = demo();
    locked(&core);
    let d = dir("fresh");
    let e = core.limits_set_dir(path(&d)).unwrap_err();
    assert_eq!(code_of(&e), "pin_required", "{e:?}");
    let st = core.installer_lock_state();
    assert!(st.locked && st.pin_set, "{st:?}");
    assert_eq!(
        code_of(&core.set_output_gain(1, 1.0).unwrap_err()),
        "locked"
    );
    assert!(!d.exists(), "nothing changed");
    // Unlocked, the installer may name a folder: the PIN goes with it.
    core.installer_unlock(PIN.into()).unwrap();
    core.limits_set_dir(path(&d)).unwrap();
    let serial = core.installer_lock_state().serial;
    assert!(
        LimitsStore::new(&d).pin(&serial).unwrap().is_some(),
        "written there"
    );
    let st = core.installer_lock().unwrap();
    assert!(st.locked && st.pin_set, "{st:?}");
}

/// F2: with the PIN in files, neither "" nor another folder drops it.
#[test]
fn wp_p1dkfix_limits_set_dir_elsewhere_keeps_a_pin_kept_in_files() {
    let core = demo();
    let d = dir("files");
    core.limits_set_dir(path(&d)).unwrap();
    locked(&core);
    for other in [String::new(), path(&dir("other"))] {
        let e = core.limits_set_dir(other.clone()).unwrap_err();
        assert_eq!(code_of(&e), "pin_required", "{other:?}: {e:?}");
        assert!(core.installer_lock_state().locked, "{other:?}");
    }
    // Named by the installer (unlocked): the PIN stays in force, in memory
    // with no folder, and in the next folder's files.
    core.installer_unlock(PIN.into()).unwrap();
    core.limits_set_dir(String::new()).unwrap();
    assert!(core.installer_lock().unwrap().locked, "kept in memory");
    assert_eq!(
        code_of(&core.set_output_gain(1, 1.0).unwrap_err()),
        "locked"
    );
    // Not unlocked: refused again, the PIN unchanged.
    let next = dir("next");
    assert!(core.limits_set_dir(path(&next)).is_err());
    core.installer_unlock(PIN.into()).unwrap();
    core.limits_set_dir(path(&next)).unwrap();
    core.installer_lock().unwrap();
    // A new core on the next folder finds the PIN there.
    let fresh = DspiCore::new();
    fresh.limits_set_dir(path(&next)).unwrap();
    fresh.open_demo().unwrap();
    let st = fresh.installer_lock_state();
    assert!(st.pin_set, "{st:?}");
    assert!(fresh.installer_unlock(PIN.into()).unwrap().unlocked);
}

/// F2: with no device open, naming a folder is never refused (an app names
/// it at start-up), and a PIN known before stays known.
#[test]
fn wp_p1dkfix_limits_set_dir_without_a_device_keeps_known_pins() {
    let core = demo();
    locked(&core);
    core.disconnect();
    core.limits_set_dir(path(&dir("nodev"))).unwrap();
    core.open_demo().unwrap();
    let st = core.installer_lock_state();
    assert!(st.locked && st.pin_set, "{st:?}");
}

/// F6: the wrong-PIN count and the window survive a restart, beside the PIN.
#[test]
fn wp_p1dkfix_the_wrong_pin_window_survives_a_restart() {
    let d = dir("tries");
    let core = demo();
    core.limits_set_dir(path(&d)).unwrap();
    locked(&core);
    for _ in 0..=LOCK_FREE_TRIES {
        assert_eq!(
            code_of(&core.installer_unlock("0000".into()).unwrap_err()),
            "wrong_pin"
        );
    }
    let serial = core.installer_lock_state().serial;
    let kept = LimitsStore::new(&d).tries(&serial).unwrap().unwrap();
    assert_eq!(kept.wrong, LOCK_FREE_TRIES + 1);
    assert!(kept.until_unix_ms > 0);
    drop(core);
    let again = DspiCore::new();
    again.limits_set_dir(path(&d)).unwrap();
    again.open_demo().unwrap();
    let st = again.installer_lock_state();
    assert_eq!(st.wrong_tries, LOCK_FREE_TRIES + 1, "{st:?}");
    assert!(
        st.retry_after_ms > 0 && st.retry_after_ms <= LOCK_FIRST_WAIT_MS,
        "{st:?}"
    );
    let e = again.installer_unlock(PIN.into()).unwrap_err();
    assert_eq!(code_of(&e), "wait", "even the right PIN waits");
    // Past the window the right PIN unlocks, and the file goes.
    again
        .lock()
        .limits
        .lock
        .age_for_test(Duration::from_millis(LOCK_FIRST_WAIT_MS + 1));
    assert!(again.installer_unlock(PIN.into()).unwrap().unlocked);
    let dev_dir = std::fs::read_dir(&d)
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    assert!(dev_dir.join(PIN_FILE).exists());
    assert!(!dev_dir.join(TRIES_FILE).exists(), "cleared on a right PIN");
}

/// F6: a window kept far ahead (a clock set back) is never longer than the
/// longest; an unreadable count file leaves no free try.
#[test]
fn wp_p1dkfix_a_kept_window_is_bounded_and_an_unreadable_one_fails_closed() {
    let d = dir("bounded");
    let core = demo();
    core.limits_set_dir(path(&d)).unwrap();
    locked(&core);
    let serial = core.installer_lock_state().serial;
    drop(core);
    let store = LimitsStore::new(&d);
    store
        .write_tries(
            &StoredTries {
                wrong: 9,
                until_unix_ms: u64::MAX,
            },
            &serial,
        )
        .unwrap();
    let again = DspiCore::new();
    again.limits_set_dir(path(&d)).unwrap();
    again.open_demo().unwrap();
    let st = again.installer_lock_state();
    assert!(st.retry_after_ms <= LOCK_MAX_WAIT_MS, "{st:?}");
    assert!(st.retry_after_ms > LOCK_MAX_WAIT_MS - 60_000, "{st:?}");
    drop(again);
    let dev_dir = std::fs::read_dir(&d)
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    std::fs::write(dev_dir.join(TRIES_FILE), "{ torn").unwrap();
    let third = DspiCore::new();
    third.limits_set_dir(path(&d)).unwrap();
    third.open_demo().unwrap();
    assert_eq!(third.installer_lock_state().wrong_tries, LOCK_FREE_TRIES);
    // The very next wrong PIN opens the window.
    let _ = third.installer_unlock("0000".into());
    assert!(third.installer_lock_state().retry_after_ms > 0);
}

/// The demo device counting every transfer (as P1AQ's rig).
struct Counted {
    dev: dspi_transport::FakeDevice,
    transfers: Arc<AtomicUsize>,
}

impl dspi_transport::Transport for Counted {
    fn control_in(&mut self, op: u8, value: u16, len: u16) -> dspi_transport::Result<Vec<u8>> {
        self.transfers.fetch_add(1, Ordering::SeqCst);
        self.dev.control_in(op, value, len)
    }
    fn control_in_upto(&mut self, op: u8, value: u16, len: u16) -> dspi_transport::Result<Vec<u8>> {
        self.transfers.fetch_add(1, Ordering::SeqCst);
        self.dev.control_in_upto(op, value, len)
    }
    fn control_out(&mut self, op: u8, value: u16, data: &[u8]) -> dspi_transport::Result<()> {
        self.transfers.fetch_add(1, Ordering::SeqCst);
        self.dev.control_out(op, value, data)
    }
    fn descriptor(&self) -> &dspi_transport::DeviceDescriptor {
        self.dev.descriptor()
    }
    fn max_transfer(&self) -> usize {
        self.dev.max_transfer()
    }
    fn notifications(&self) -> Option<Box<dyn dspi_transport::NotificationSource>> {
        self.dev.notifications()
    }
    fn caps(&self) -> dspi_transport::TransportCaps {
        self.dev.caps()
    }
}

/// F7 (decided): the analyser runs while the installer's lock is on.
#[test]
fn wp_p1dkfix_the_analyser_runs_while_locked() {
    let transfers = Arc::new(AtomicUsize::new(0));
    let dev = Counted {
        dev: dspi_transport::FakeDevice::demo(),
        transfers: Arc::clone(&transfers),
    };
    let core = DspiCore::new();
    core.attach_probed(Box::new(dev), "DK-RTA".into(), 0)
        .unwrap();
    core.fetch_all().unwrap();
    locked(&core);
    let sent = transfers.load(Ordering::SeqCst);
    core.rta_start();
    let start = std::time::Instant::now();
    while transfers.load(Ordering::SeqCst) == sent {
        assert!(
            start.elapsed() < Duration::from_secs(5),
            "the analyser does not start while locked"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    // Other writes stay refused.
    assert_eq!(
        code_of(&core.set_output_gain(1, 1.0).unwrap_err()),
        "locked"
    );
    core.disconnect();
}

/// F10: a redo of a write the lock refuses is refused, nothing written.
#[test]
fn wp_p1dkfix_redo_is_refused_while_locked() {
    let core = demo();
    core.set_output_gain(1, 1.5).unwrap();
    core.undo().unwrap();
    let before = core.get_state();
    locked(&core);
    let e = core.redo().unwrap_err();
    assert_eq!(code_of(&e), "locked", "{e:?}");
    assert_eq!(core.get_state().output_gain_db, before.output_gain_db);
    // Unlocked, the same redo passes.
    core.installer_unlock(PIN.into()).unwrap();
    assert!(core.redo().is_ok());
    assert_eq!(core.get_state().output_gain_db[1], 1.5);
}

/// F10: an undo of a group mixing an allowed write (the master volume) with a
/// refused one (an output gain) is whole or nothing, never half: refused, and
/// nothing of it undone.
#[test]
fn wp_p1dkfix_undo_of_a_mixed_group_while_locked_is_whole_or_nothing() {
    let core = demo();
    core.begin_group("mixed".into());
    core.set_master_volume(-30.0).unwrap();
    core.set_output_gain(1, 1.5).unwrap();
    core.end_group();
    let after_group = core.get_state();
    locked(&core);
    let e = core.undo().unwrap_err();
    assert_eq!(code_of(&e), "locked", "{e:?}");
    let now = core.get_state();
    assert_eq!(now.output_gain_db, after_group.output_gain_db);
    assert_eq!(now.master_volume_db, after_group.master_volume_db);
}

/// Without a hub session the refresh asks nothing: the console's own lock.
#[test]
fn wp_p1dkfix_the_lock_refresh_without_a_hub_is_the_consoles_lock() {
    let core = demo();
    let st = core.installer_lock_refresh().unwrap();
    assert_eq!((st.held_by.as_str(), st.pin_set), ("", false));
    locked(&core);
    let st = core.installer_lock_refresh().unwrap();
    assert_eq!(st.held_by, HELD_BY_CONSOLE);
    assert!(st.locked && st.hub_pinned_mode.is_empty(), "{st:?}");
}
