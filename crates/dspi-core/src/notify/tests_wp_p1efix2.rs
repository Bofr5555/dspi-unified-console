//! WP P1EFIX2: the settle before a whole image (review p1efix). Adopted from
//! the reviewer's probes (`tests_review_p1efix.rs`): every flag a settle took
//! still reaches the client (F1), a settle with nothing queued claims no
//! re-read, and the guard's before-image and A/B's captured A hold what the
//! device held when the apply began (F3).
#![allow(clippy::unwrap_used)]

use std::sync::Arc;
use std::time::Duration;

use dspi_transport::{FakeDevice, FakeRemote};

use super::*;

const SRC_GPIO: u8 = nfy::PARAM_SRC_GPIO as u8;

/// Pump until ten quiet rounds in a row, every result ORed.
fn quiet(core: &DspiCore) -> NotifyResult {
    let mut acc = NotifyResult::default();
    let mut q = 0;
    while q < 10 {
        std::thread::sleep(Duration::from_millis(10));
        let r = core.process_notifications();
        if r == NotifyResult::default() {
            q += 1;
        } else {
            q = 0;
        }
        acc.flags |= r.flags;
        acc.filter_channels |= r.filter_channels;
    }
    acc
}

fn demo() -> (Arc<DspiCore>, FakeRemote) {
    let dev = FakeDevice::demo();
    let remote = dev.remote();
    let core = DspiCore::new();
    core.attach_probed(Box::new(dev), "DEMO".into(), 0).unwrap();
    quiet(&core);
    (core, remote)
}

fn name_packet(evt: u8, id: u8, src: u8, name: &[u8]) -> Vec<u8> {
    let mut p = vec![2, evt, 0, 0, id, src, 0, 0];
    let mut field = [0u8; 32];
    field[..name.len()].copy_from_slice(name);
    p.extend(field);
    p
}

/// A knob (GPIO) moves the master volume to `db`, queued, not pumped.
fn knob(remote: &FakeRemote, db: f32) {
    assert!(remote.poke(g::OFF_MASTER_VOLUME, &db.to_le_bytes(), SRC_GPIO));
    std::thread::sleep(Duration::from_millis(100));
}

/// F1 (the reviewer's probe 1): a foreign change and a name event queued
/// before a recall. The settle applies both; the next pump still says NAMES.
#[test]
fn wp_p1efix2_a_settle_keeps_every_flag_for_the_next_pump() {
    let (core, remote) = demo();
    let v0 = core.get_state().master_volume_db;
    let b = core
        .snapshot_capture_document("B".into(), String::new())
        .unwrap();
    assert!(remote.poke(g::OFF_MASTER_VOLUME, &(v0 - 7.0).to_le_bytes(), SRC_GPIO));
    remote.push(name_packet(
        EVT_INPUT_SOURCE_NAME,
        0,
        SRC_GPIO,
        b"Turntable",
    ));
    std::thread::sleep(Duration::from_millis(100));
    core.snapshot_recall_document(b).unwrap();
    let r1 = core.process_notifications();
    assert_ne!(
        r1.flags & NOTIFY_NAMES,
        0,
        "NAMES reaches the next pump: {r1:?}"
    );
    assert_ne!(r1.flags & NOTIFY_STATE, 0, "the knob's change too: {r1:?}");
    quiet(&core);
    assert_eq!(core.get_state().master_volume_db, v0, "B's volume holds");
}

/// The reviewer's probe 3: a settle with nothing queued claims no re-read.
#[test]
fn wp_p1efix2_a_settle_with_nothing_queued_claims_no_reread() {
    let (core, _remote) = demo();
    let b = core
        .snapshot_capture_document("B".into(), String::new())
        .unwrap();
    core.snapshot_recall_document(b).unwrap();
    let r = core.process_notifications();
    assert_eq!(r.flags & NOTIFY_REFRESHED, 0, "no re-read claimed: {r:?}");
}

/// F3 (the reviewer's probe 2, the knob at -27): a foreign change queued
/// before a guarded recall is in the before-image, so Revert puts the
/// device back as it was when the apply began.
#[test]
fn wp_p1efix2_revert_puts_back_a_change_queued_before_the_apply() {
    let (core, remote) = demo();
    let v0 = core.get_state().master_volume_db;
    core.set_master_volume(v0 - 3.0).unwrap();
    quiet(&core);
    let b = core
        .snapshot_capture_document("B".into(), String::new())
        .unwrap();
    core.set_master_volume(v0).unwrap();
    quiet(&core);
    knob(&remote, v0 - 7.0); // -27 with the demo's -20
    core.apply_guarded_snapshot_document(b, 30).unwrap();
    assert_eq!(core.get_state().master_volume_db, v0 - 3.0, "B applied");
    core.apply_revert().unwrap();
    quiet(&core);
    core.refresh_params().unwrap();
    assert_eq!(
        core.get_state().master_volume_db,
        v0 - 7.0,
        "Revert puts back the device's state from before the apply"
    );
}

/// F3 for A/B: "the device now" as A includes a change queued before the
/// compare began, so keeping A after B keeps the knob's -27.
#[test]
fn wp_p1efix2_the_captured_a_holds_a_change_queued_before_the_compare() {
    let (core, remote) = demo();
    let v0 = core.get_state().master_volume_db;
    core.set_master_volume(v0 - 3.0).unwrap();
    quiet(&core);
    let b = core
        .snapshot_capture_document("B".into(), String::new())
        .unwrap();
    core.set_master_volume(v0).unwrap();
    quiet(&core);
    knob(&remote, v0 - 7.0);
    core.ab_start_documents(String::new(), b).unwrap();
    core.ab_toggle().unwrap();
    assert_eq!(core.get_state().master_volume_db, v0 - 3.0, "on B");
    core.ab_keep(crate::AbSide::A).unwrap();
    quiet(&core);
    core.refresh_params().unwrap();
    assert_eq!(
        core.get_state().master_volume_db,
        v0 - 7.0,
        "A is the device as it was"
    );
}

/// F2 (the reviewer's probe 4), not fixed here: the echo of this console's
/// own write arriving only after the image is still applied over it. The
/// complete fix is in the echo classifier, a later core WP.
#[test]
#[ignore = "review p1efix F2: a later core WP (the echo classifier)"]
fn wp_p1efix2_f2_a_late_own_echo_after_the_image() {
    let (core, remote) = demo();
    let v0 = core.get_state().master_volume_db;
    core.set_master_volume(v0 - 3.0).unwrap();
    quiet(&core);
    let b = core
        .snapshot_capture_document("B".into(), String::new())
        .unwrap();
    core.set_master_volume(v0).unwrap();
    quiet(&core);
    core.snapshot_recall_document(b).unwrap();
    let mut p = vec![2, nfy::NOTIFY_EVT_PARAM_CHANGED as u8, 0, 0];
    p.extend((g::OFF_MASTER_VOLUME as u16).to_le_bytes());
    p.extend(4u16.to_le_bytes());
    p.extend([nfy::PARAM_SRC_HOST_SET as u8, 0, 0, 0]);
    p.extend(v0.to_le_bytes());
    remote.push(p);
    std::thread::sleep(Duration::from_millis(100));
    let _ = core.process_notifications();
    assert_eq!(core.get_state().master_volume_db, v0 - 3.0);
}
