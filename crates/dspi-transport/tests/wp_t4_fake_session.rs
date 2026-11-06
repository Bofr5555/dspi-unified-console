//! WP T4 done criteria: the Terminal's session layer against the demo device.

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
