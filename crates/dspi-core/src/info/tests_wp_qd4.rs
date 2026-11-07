//! WP QD4: the cached name getters and the demo device's rename from
//! elsewhere, against `FakeDevice`.

use std::time::{Duration, Instant};

use dspi_proto::generated::opcodes as op;
use dspi_transport::{Capture, FakeOptions};

use super::*;
use crate::consts::NOTIFY_NAMES;
use crate::ext::tests_wp_c8::{Faults, k340, probed};

const I2S: u8 = 2;
const DEVICE: u8 = 0xFF;

fn process_until(core: &DspiCore, done: impl Fn(&DspiCore, u32) -> bool) {
    let start = Instant::now();
    let mut flags = 0;
    while !done(core, flags) {
        flags |= core.process_notifications().flags;
        assert!(start.elapsed() < Duration::from_secs(5), "flags {flags:#x}");
        std::thread::sleep(Duration::from_millis(2));
    }
}

#[test]
fn wp_qd4_the_getters_send_nothing_and_follow_a_set() {
    let p = k340();
    p.core.fetch_all().unwrap();
    let before = p.traffic();
    assert_eq!(p.core.cached_input_source_name(I2S).unwrap(), "");
    assert_eq!(p.core.cached_device_name().unwrap(), "DSPi Demo");
    assert_eq!(p.traffic(), before, "the getters do no I/O");

    p.core.set_input_source_name(I2S, "Tape".into()).unwrap();
    p.core.set_device_name("Den".into()).unwrap();
    let before = p.traffic();
    assert_eq!(p.core.cached_input_source_name(I2S).unwrap(), "Tape");
    assert_eq!(p.core.cached_device_name().unwrap(), "Den");
    assert_eq!(p.traffic(), before);
    assert_eq!(p.core.get_device_name().unwrap(), "Den");
}

#[test]
fn wp_qd4_connect_reads_the_device_name_it_holds() {
    let p = k340();
    p.core.set_device_name("Den".into()).unwrap();
    p.core.lock().conn().unwrap().device_name.clear();
    p.core.fetch_all().unwrap();
    assert_eq!(p.core.cached_device_name().unwrap(), "Den");
}

#[test]
fn wp_qd4_a_rename_elsewhere_reaches_the_cache_and_raises_names() {
    let p = k340();
    p.core.fetch_all().unwrap();
    p.core.demo_rename_elsewhere(I2S, "Tape".into()).unwrap();
    process_until(&p.core, |_, f| f & NOTIFY_NAMES != 0);
    assert_eq!(p.core.cached_input_source_name(I2S).unwrap(), "Tape");
    assert_eq!(p.core.get_input_source_name(I2S).unwrap(), "Tape");

    p.core.demo_rename_elsewhere(DEVICE, "Den".into()).unwrap();
    process_until(&p.core, |c, f| {
        f & NOTIFY_NAMES != 0 && c.cached_device_name().is_ok_and(|n| n == "Den")
    });
    assert_eq!(p.core.get_device_name().unwrap(), "Den");

    p.core.demo_rename_elsewhere(DEVICE, String::new()).unwrap();
    process_until(&p.core, |c, _| {
        c.cached_device_name().is_ok_and(|n| n.is_empty())
    });
    assert_eq!(p.core.get_device_name().unwrap(), "");
}

#[test]
fn wp_qd4_a_lost_packet_reads_the_device_name_again() {
    let p = k340();
    p.core.fetch_all().unwrap();
    p.core.set_device_name("Den".into()).unwrap();
    p.core.lock().conn().unwrap().device_name = "Old".into();
    // A version the decoder refuses arrives as a lost packet.
    p.remote.push(vec![3, 2, 0, 0, 0, 0, 0, 0]);
    process_until(&p.core, |c, _| {
        c.cached_device_name().is_ok_and(|n| n == "Den")
    });
}

#[test]
fn wp_qd4_callers_past_the_table_and_firmware_without_names_are_refused() {
    let p = k340();
    p.core.fetch_all().unwrap();
    assert!(matches!(
        p.core.cached_input_source_name(7),
        Err(DspiError::InvalidArgument { .. })
    ));
    assert!(p.core.demo_rename_elsewhere(I2S, "x".repeat(32)).is_err());

    let p = probed(
        Capture::demo().bulk,
        FakeOptions {
            ack_unknown_sets: true,
            ..FakeOptions::default()
        },
        Faults {
            stall_reads: &[op::REQ_GET_INPUT_SOURCE_NAME, op::REQ_GET_DEVICE_NAME],
            ..Faults::default()
        },
    );
    p.core.fetch_all().unwrap();
    assert!(matches!(
        p.core.cached_input_source_name(I2S),
        Err(DspiError::Unsupported)
    ));
    assert!(matches!(
        p.core.cached_device_name(),
        Err(DspiError::Unsupported)
    ));
}

#[test]
fn wp_qd4_without_a_device_the_getter_and_a_rename_fail() {
    let core = DspiCore::new();
    assert!(core.demo_rename_elsewhere(I2S, "x".into()).is_err());
    assert!(core.cached_device_name().is_err());
}
