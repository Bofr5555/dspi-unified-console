//! WP CG13: the input source name cache behind `preset_changes`, against
//! `FakeDevice`: filled on connect, by a set and by notifications.

use std::time::{Duration, Instant};

use dspi_proto::generated::notify as nfy;
use dspi_proto::generated::opcodes as op;
use dspi_transport::{Capture, FakeDevice, FakeOptions, FakeRemote, Transport};

use super::*;
use crate::consts::{NOTIFY_NAMES, NOTIFY_REFRESHED};
use crate::ext::tests_wp_c8::{Faults, k340, probed};
use crate::notify::{EVT_DEVICE_NAME, EVT_INPUT_SOURCE_NAME};

const I2S: u8 = 2;
const SRC_HOST: u8 = nfy::PARAM_SRC_HOST_SET as u8;
const SRC_GPIO: u8 = nfy::PARAM_SRC_GPIO as u8;

fn name_packet(evt: u8, id: u8, src: u8, name: &[u8]) -> Vec<u8> {
    let mut p = vec![2, evt, 0, 0, id, src, 0, 0];
    let mut field = [0u8; 32];
    field[..name.len()].copy_from_slice(name);
    p.extend(field);
    p
}

/// Run `process_notifications` until the flags hold `want`.
fn process_until(core: &DspiCore, want: u32) {
    let start = Instant::now();
    let mut flags = 0;
    while flags & want != want {
        flags |= core.process_notifications().flags;
        assert!(start.elapsed() < Duration::from_secs(5), "flags {flags:#x}");
        std::thread::sleep(Duration::from_millis(2));
    }
}

/// Run `process_notifications` until the input source row reads `want`.
fn row_until(core: &DspiCore, want: &str) {
    let start = Instant::now();
    loop {
        core.process_notifications();
        let changes = core.preset_changes();
        if changes == [want] {
            return;
        }
        assert!(start.elapsed() < Duration::from_secs(5), "{changes:?}");
        std::thread::sleep(Duration::from_millis(2));
    }
}

/// The demo device, fetched, with the input source moved from USB to I2S.
fn on_i2s() -> crate::ext::tests_wp_c8::Probed {
    let p = k340();
    p.core.fetch_all().unwrap();
    p.core.set_input_source(I2S).unwrap();
    assert_eq!(p.core.preset_changes(), ["Input source: USB → I2S"]);
    p
}

fn push_name(remote: &FakeRemote, source: u8, src: u8, name: &[u8]) {
    remote.push(name_packet(EVT_INPUT_SOURCE_NAME, source, src, name));
}

#[test]
fn wp_cg13_connect_reads_the_names_the_device_holds() {
    let mut dev = FakeDevice::demo();
    dev.control_out(op::REQ_SET_INPUT_SOURCE_NAME, u16::from(I2S), b"Turntable")
        .unwrap();
    let core = DspiCore::new();
    core.attach_probed(Box::new(dev), "CG13".into(), 0).unwrap();
    core.fetch_all().unwrap();
    core.set_input_source(I2S).unwrap();
    assert_eq!(core.preset_changes(), ["Input source: USB → Turntable"]);
}

#[test]
fn wp_cg13_another_clients_rename_updates_the_row_without_io() {
    let p = on_i2s();
    push_name(&p.remote, I2S, SRC_GPIO, b"Turntable");
    process_until(&p.core, NOTIFY_NAMES);

    let before = p.traffic();
    assert_eq!(p.core.preset_changes(), ["Input source: USB → Turntable"]);
    assert_eq!(p.traffic(), before, "preset_changes does no I/O");

    // The other side of the row, a second rename, and clearing one.
    push_name(&p.remote, 0, SRC_GPIO, b"Mac");
    row_until(&p.core, "Input source: Mac → Turntable");
    push_name(&p.remote, I2S, SRC_GPIO, b"Tape");
    row_until(&p.core, "Input source: Mac → Tape");
    push_name(&p.remote, I2S, SRC_GPIO, b"");
    row_until(&p.core, "Input source: Mac → I2S");
}

#[test]
fn wp_cg13_a_rename_from_another_usb_app_counts_though_it_looks_like_our_own() {
    // HOST_SET on USB is "our own" to the echo rule, so it raises no flag; the
    // cache still follows it, since the device says what it stored.
    let p = on_i2s();
    push_name(&p.remote, I2S, SRC_HOST, b"Turntable");
    row_until(&p.core, "Input source: USB → Turntable");
}

#[test]
fn wp_cg13_a_set_shows_at_once_and_its_echo_changes_nothing() {
    let p = on_i2s();
    p.core.set_input_source_name(I2S, "Tape".into()).unwrap();
    assert_eq!(p.core.preset_changes(), ["Input source: USB → Tape"]);
    p.core.process_notifications();
    assert_eq!(p.core.preset_changes(), ["Input source: USB → Tape"]);

    p.core.set_input_source_name(I2S, String::new()).unwrap();
    assert_eq!(p.core.preset_changes(), ["Input source: USB → I2S"]);
}

#[test]
fn wp_cg13_a_refused_set_leaves_the_cache_alone() {
    let p = on_i2s();
    p.core.set_input_source_name(I2S, "Tape".into()).unwrap();
    let long = "x".repeat(NAME_MAX + 1);
    assert!(p.core.set_input_source_name(I2S, long).is_err());
    assert!(p.core.set_input_source_name(7, "Nope".into()).is_err());
    assert_eq!(p.core.preset_changes(), ["Input source: USB → Tape"]);
}

#[test]
fn wp_cg13_bad_packets_and_other_events_never_touch_the_names() {
    let p = on_i2s();
    p.core.set_input_source_name(I2S, "Tape".into()).unwrap();
    // A source past the table, a packet cut before the end of the name, and
    // the device name, whose id is 0 in the same layout.
    push_name(&p.remote, 200, SRC_GPIO, b"Far");
    let mut short = name_packet(EVT_INPUT_SOURCE_NAME, I2S, SRC_GPIO, b"Cut");
    short.truncate(20);
    p.remote.push(short);
    p.remote
        .push(name_packet(EVT_DEVICE_NAME, I2S, SRC_GPIO, b"Rack"));
    // This last one proves the others have been read.
    push_name(&p.remote, 0, SRC_GPIO, b"Mac");
    row_until(&p.core, "Input source: Mac → Tape");
}

#[test]
fn wp_cg13_a_name_from_a_packet_is_decoded_lossily_and_cut() {
    let p = on_i2s();
    push_name(&p.remote, I2S, SRC_GPIO, &[b'A', 0xFF, b'B', 0, b'x']);
    row_until(&p.core, "Input source: USB → A\u{FFFD}B");
    push_name(&p.remote, I2S, SRC_GPIO, &[b'y'; 32]);
    row_until(
        &p.core,
        &format!("Input source: USB → {}", "y".repeat(NAME_MAX)),
    );
}

#[test]
fn wp_cg13_a_lost_packet_reads_the_names_again() {
    let p = on_i2s();
    let cached = |p: &crate::ext::tests_wp_c8::Probed| {
        p.core.lock().conn().unwrap().source_names[usize::from(I2S)].clone()
    };
    // As if a rename back to "" had been among the lost packets.
    p.core.lock().conn().unwrap().source_names[usize::from(I2S)] = "Old".into();
    // A version the decoder refuses arrives as a lost packet.
    p.remote.push(vec![3, 2, 0, 0, 0, 0, 0, 0]);
    process_until(&p.core, NOTIFY_REFRESHED);
    assert_eq!(cached(&p), "");
    assert!(!p.core.lock().conn().unwrap().source_names_stale);
}

#[test]
fn wp_cg13_a_re_read_for_any_other_reason_sends_no_name_request() {
    // Preset load and factory reset leave names alone
    // (input_source_names_spec.md), so their re-reads do not ask.
    let p = on_i2s();
    p.core.lock().conn().unwrap().source_names[usize::from(I2S)] = "Kept".into();
    p.remote
        .load_image(p.remote.image(), nfy::PARAM_SRC_PRESET as u8);
    process_until(&p.core, NOTIFY_REFRESHED);
    assert_eq!(
        p.core.lock().conn().unwrap().source_names[usize::from(I2S)],
        "Kept"
    );
}

#[test]
fn wp_cg13_firmware_without_names_is_never_asked_and_keeps_the_built_in_titles() {
    let p = probed(
        Capture::demo().bulk,
        FakeOptions {
            ack_unknown_sets: true,
            ..FakeOptions::default()
        },
        Faults {
            stall_reads: &[op::REQ_GET_INPUT_SOURCE_NAME],
            ..Faults::default()
        },
    );
    p.core.fetch_all().unwrap();
    let before = p.traffic();
    read_source_names(p.core.lock().conn().unwrap());
    assert_eq!(p.traffic(), before, "no request on such firmware");
    p.core.set_input_source(I2S).unwrap();
    assert_eq!(p.core.preset_changes(), ["Input source: USB → I2S"]);
}

#[test]
fn wp_cg13_a_new_connection_starts_without_the_old_ones_names() {
    let p = on_i2s();
    p.core.set_input_source_name(I2S, "Tape".into()).unwrap();
    p.core.disconnect();
    assert!(p.core.preset_changes().is_empty());

    p.core
        .attach_probed(Box::new(FakeDevice::demo()), "OTHER".into(), 0)
        .unwrap();
    p.core.fetch_all().unwrap();
    p.core.set_input_source(I2S).unwrap();
    assert_eq!(p.core.preset_changes(), ["Input source: USB → I2S"]);
}
