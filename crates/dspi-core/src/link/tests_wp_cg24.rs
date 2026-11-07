//! WP CG24: the device lock held across a flow, on the fake hub. Two cores are
//! two sessions of one hub: while one holds the lock, the other's commands
//! answer `Locked`.

use std::time::{Duration, Instant};

use dspi_link::fakehub::{FakeHub, HubConfig};
use dspi_proto::generated::opcodes::REQ_ENTER_BOOTLOADER;

use super::*;
use crate::presets::Holder;

fn hub() -> FakeHub {
    FakeHub::start(HubConfig::default()).unwrap()
}

/// A core bound to the hub's device: one session.
fn bound(hub: &FakeHub) -> Arc<DspiCore> {
    let core = DspiCore::new();
    core.link_connect(
        hub.url(),
        "wp-cg24".into(),
        "test".into(),
        String::new(),
        String::new(),
    )
    .unwrap();
    wait_until("the device list", || !core.link_devices().is_empty());
    select(&core);
    core
}

fn select(core: &DspiCore) {
    let d = core.link_devices().remove(0);
    core.link_select_device(d.handle, d.serial).unwrap();
    core.fetch_all().unwrap();
}

fn wait_until(what: &str, mut f: impl FnMut() -> bool) {
    let start = Instant::now();
    while !f() {
        assert!(
            start.elapsed() < Duration::from_secs(8),
            "timed out waiting for {what}"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// The release is fire and forget: wait for the hub to have it.
fn wait_unlocked(hub: &FakeHub) {
    wait_until("the release", || hub.lock_holder().is_none());
}

/// Another session can write again.
fn writable(other: &DspiCore) {
    other.set_master_volume(-20.0).unwrap();
}

#[test]
fn wp_cg24_another_session_is_locked_out_until_the_release() {
    let hub = hub();
    let (a, b) = (bound(&hub), bound(&hub));
    writable(&b);
    assert_eq!(
        a.device_lock_acquire("Flow".into(), 0),
        Err(DspiError::InvalidArgument {
            msg: "a device lock needs a timeout".into()
        })
    );
    a.device_lock_acquire("Flow".into(), 10_000).unwrap();
    assert!(hub.lock_holder().is_some());
    assert_eq!(a.held_lock_holder(), Some(Holder::Caller));
    assert_eq!(b.set_master_volume(-12.0), Err(DspiError::Locked));
    assert_eq!(
        b.device_lock_acquire("Other".into(), 1000),
        Err(DspiError::Locked)
    );
    // The holder's own writes go through, and a second acquire asks nothing.
    a.set_master_volume(-14.0).unwrap();
    a.device_lock_acquire("Again".into(), 10_000).unwrap();
    a.device_lock_release();
    wait_unlocked(&hub);
    assert_eq!(a.held_lock_holder(), None);
    writable(&b);
    // Idempotent.
    a.device_lock_release();
    writable(&b);
}

#[test]
fn wp_cg24_a_nested_flow_keeps_the_held_lock() {
    let hub = hub();
    let (a, b) = (bound(&hub), bound(&hub));
    a.device_lock_acquire("Install".into(), 10_000).unwrap();
    let holder = hub.lock_holder();
    // The core's own locked flows run inside it: no deadlock, and their end
    // does not give it back.
    a.apply_all_params().unwrap();
    assert_eq!(hub.lock_holder(), holder);
    // The fake hub has no presets (the save stalls): it returns, all the same.
    let _ = a.save_preset(1);
    assert_eq!(hub.lock_holder(), holder);
    assert_eq!(b.set_master_volume(-1.0), Err(DspiError::Locked));
    a.device_lock_release();
    wait_unlocked(&hub);
    writable(&b);
}

#[test]
fn wp_cg24_the_bootloader_restart_holds_the_lock() {
    let hub = hub();
    let (a, b) = (bound(&hub), bound(&hub));
    a.enter_bootloader().unwrap();
    assert_eq!(a.held_lock_holder(), Some(Holder::Bootloader));
    assert!(hub.lock_holder().is_some());
    assert_eq!(b.set_master_volume(-3.0), Err(DspiError::Locked));
    // A refused lock sends nothing.
    let sent = |h: &FakeHub| {
        h.log()
            .iter()
            .filter(|c| c.2 == REQ_ENTER_BOOTLOADER)
            .count()
    };
    let before = sent(&hub);
    assert_eq!(b.enter_bootloader(), Err(DspiError::Locked));
    assert_eq!(sent(&hub), before);
    // The caller's release is not the install's: still locked.
    a.device_lock_release();
    assert_eq!(a.held_lock_holder(), Some(Holder::Bootloader));
    assert_eq!(b.set_master_volume(-3.0), Err(DspiError::Locked));
    // The device leaves: the lost connection gives it back.
    a.disconnect();
    wait_unlocked(&hub);
    writable(&b);
}

#[test]
fn wp_cg24_an_install_inside_the_callers_lock_ends_with_its_release() {
    let hub = hub();
    let (a, b) = (bound(&hub), bound(&hub));
    a.device_lock_acquire("Installing firmware".into(), 60_000)
        .unwrap();
    a.enter_bootloader().unwrap();
    assert_eq!(b.set_master_volume(-3.0), Err(DspiError::Locked));
    // The restart never happened.
    a.device_lock_release();
    wait_unlocked(&hub);
    writable(&b);
}

#[test]
fn wp_cg24_a_closed_connection_gives_the_lock_back() {
    let hub = hub();
    let (a, b) = (bound(&hub), bound(&hub));
    a.enter_bootloader().unwrap();
    // The device left: the connection closes.
    a.disconnect();
    wait_unlocked(&hub);
    writable(&b);
}

#[test]
fn wp_cg24_the_lock_never_outlives_its_connection() {
    let hub = hub();
    let (a, b) = (bound(&hub), bound(&hub));
    a.device_lock_acquire("Flow".into(), 10_000).unwrap();
    let generation = a.lock().conn().unwrap().generation;
    // A re-bind is a new connection.
    select(&a);
    assert_ne!(a.lock().conn().unwrap().generation, generation);
    assert_eq!(a.held_lock_holder(), None);
    wait_unlocked(&hub);
    writable(&b);
    // The new connection locks afresh.
    a.device_lock_acquire("Flow".into(), 10_000).unwrap();
    assert_eq!(b.set_master_volume(-2.0), Err(DspiError::Locked));
    a.device_lock_release();
    wait_unlocked(&hub);
}

#[test]
fn wp_cg24_the_timeout_ends_it_on_the_hub_and_in_the_core() {
    let hub = hub();
    let (a, b) = (bound(&hub), bound(&hub));
    a.device_lock_acquire("Short".into(), 150).unwrap();
    assert_eq!(b.set_master_volume(-2.0), Err(DspiError::Locked));
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(a.held_lock_holder(), None);
    writable(&b);
    // The core's next flow asks the hub again, then gives it back.
    a.apply_all_params().unwrap();
    wait_unlocked(&hub);
    writable(&b);
}
