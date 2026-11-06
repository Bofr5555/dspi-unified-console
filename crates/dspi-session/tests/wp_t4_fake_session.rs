//! WP T4 done criteria: the Terminal's session layer against the demo device
//! (`dspi_transport::FakeDevice`). Lives here, not in dspi-transport, so the
//! transport needs no dev-dependency on the session.

use std::time::Duration;

use dspi_proto::generated;
use dspi_proto::value::Value;
use dspi_session::notify::{Event, Source, decode};
use dspi_session::{Outcome, Session, probe, read_bulk};
use dspi_transport::{FakeDevice, Transport};

const BULK: &[u8] = include_bytes!("../../../tests/fixtures/link/device/bulk_v32.bin");

#[test]
fn wp_t4_the_terminal_probes_and_reads_the_demo() {
    let mut d = FakeDevice::demo();
    let caps = probe(&mut d).unwrap();
    assert_eq!(caps.wire_format, 32);
    assert_eq!(
        (caps.num_channels, caps.num_inputs, caps.num_outputs),
        (17, 8, 9)
    );
    assert_eq!(caps.firmware, "1.1.6 beta 5");
    assert_eq!(caps.serial, "DEMO0000DSPI0001");
    assert_eq!((caps.max_bands, caps.band_storage), (10, 12));
    assert_eq!(caps.active_preset, Some(0));
    assert!(caps.build_info.is_some());
    assert_eq!(caps.channels.len(), 17);

    let bulk = read_bulk(&mut d).unwrap();
    assert_eq!(bulk.as_bytes(), BULK);
}

/// T4b: the demo answers every feature probe but the signal generator and
/// the spectrum analyser, which it does not model.
#[test]
fn wp_t4b_the_demo_has_every_feature_but_siggen_and_rta() {
    let mut d = FakeDevice::demo();
    let caps = probe(&mut d).unwrap();
    let absent: Vec<_> = caps
        .features
        .iter()
        .filter(|f| !f.present)
        .map(|f| (f.name.as_str(), f.evidence.as_str()))
        .collect();
    let names: Vec<_> = absent.iter().map(|(n, _)| *n).collect();
    assert_eq!(names, ["test_signals", "spectrum_analyser"], "{absent:?}");
    assert!(caps.features.len() >= 20);
}

#[test]
fn wp_t4_a_master_volume_write_reads_back_and_notifies() {
    let mut d = FakeDevice::demo();
    let mut notes = d.notifications().unwrap();
    let remote = d.remote();
    let caps = probe(&mut d).unwrap();
    let mut s = Session::new(Box::new(d), caps).unwrap();

    let out = s.write("vol.master", &[], Value::Float(-12.0)).unwrap();
    assert_eq!(out, Outcome::Confirmed(Value::Float(-12.0)));
    assert_eq!(s.read("vol.master", &[]).unwrap().as_f32(), Some(-12.0));

    let off = generated::OFF_MASTER_VOLUME;
    // The legacy v1 volume packet comes first, as from the firmware.
    let v1 = notes.read(Duration::from_millis(500)).unwrap();
    assert_eq!(decode(&v1).unwrap().1, Event::MasterVolume(-12.0));
    let p = notes.read(Duration::from_millis(500)).unwrap();
    let (_, ev) = decode(&p).unwrap();
    match ev {
        Event::ParamChanged {
            offset,
            source,
            bytes,
        } => {
            assert_eq!(usize::from(offset), off);
            assert_eq!(
                source,
                Source::from_raw(generated::notify::PARAM_SRC_HOST_SET as u8)
            );
            assert_eq!(bytes, (-12.0f32).to_le_bytes());
        }
        other => panic!("expected PARAM_CHANGED, got {other:?}"),
    }
    assert_eq!(remote.image()[off..off + 4], (-12.0f32).to_le_bytes());
    // The bulk image, read the Terminal's way, agrees.
    let bulk = s.with_transport(|t| read_bulk(t)).unwrap();
    assert_eq!(bulk.as_bytes()[off..off + 4], (-12.0f32).to_le_bytes());
}
