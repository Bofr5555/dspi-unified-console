//! WP P1: property tests for the notification decoder and the mirror it
//! drives (PLAN §1.9). Notify packets come from the device or a hub (§1.7):
//! no packet may panic the decoder, and no decoded packet may panic
//! `DeviceState::apply`.

use dspi_proto::Platform;
use dspi_proto::generated::BULK_SIZE;
use dspi_proto::packets::FirmwareVersion;
use dspi_proto::wire::BulkPacket;
use dspi_session::notify::{self, DecodeError, Event, PROTOCOL_VERSION, evt};
use dspi_session::{Applied, Capabilities, DeviceState, Notification};
use proptest::prelude::*;

fn caps() -> Capabilities {
    Capabilities {
        serial: "WP-P1".into(),
        platform: Platform::Rp2350,
        firmware: "1.1.6".into(),
        firmware_version: FirmwareVersion::new(1, 1, 6, 0),
        build_info: None,
        wire_format: 32,
        num_channels: 17,
        num_inputs: 8,
        num_outputs: 9,
        max_bands: 10,
        band_storage: 12,
        channels: Vec::new(),
        features: Vec::new(),
        cs: None,
        siggen: None,
        active_preset: None,
    }
}

/// A V32 image with a valid header and random contents.
fn image(seed: u64) -> Vec<u8> {
    let mut x = seed | 1;
    let mut d: Vec<u8> = (0..BULK_SIZE)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x as u8
        })
        .collect();
    d[..8].copy_from_slice(&[32, 1, 17, 9, 8, 12, 0, 0]);
    d[6..8].copy_from_slice(&(BULK_SIZE as u16).to_le_bytes());
    d
}

/// Packets shaped like v2 notifications: the right version byte, a known
/// (or unknown) event id, and a body of any length. `PARAM_CHANGED` gets a
/// size field that sometimes agrees with the body.
fn packet() -> impl Strategy<Value = Vec<u8>> {
    let ids = vec![
        evt::IDLE,
        evt::MASTER_VOLUME,
        evt::PARAM_CHANGED,
        evt::BULK_INVALIDATED,
        evt::PRESET_LOADED,
        evt::INPUT_FORMAT,
        evt::SIGGEN_STATE,
        evt::ADAT_STATE,
        evt::I2S_SLAVE_STATE,
        evt::CS_IR_LEARN,
        evt::ADAT_INPUT_STATE,
        evt::CS_AUX,
    ];
    let shaped = (
        prop_oneof![prop::sample::select(ids), any::<u8>()],
        any::<u8>(),
        any::<u8>(),
        prop::collection::vec(any::<u8>(), 0..80),
        any::<bool>(),
        any::<u16>(),
    )
        .prop_map(|(id, flags, seq, body, agree, offset)| {
            let mut p = vec![PROTOCOL_VERSION, id, flags, seq];
            p.extend(body);
            if id == evt::PARAM_CHANGED && agree && p.len() >= 12 {
                let size = (p.len() - 12) as u16;
                p[4..6].copy_from_slice(&offset.to_le_bytes());
                p[6..8].copy_from_slice(&size.to_le_bytes());
            }
            p
        });
    prop_oneof![
        3 => shaped,
        1 => prop::collection::vec(any::<u8>(), 0..80),
        // The v1 legacy volume shape.
        1 => prop::collection::vec(any::<u8>(), 0..12).prop_map(|t| {
            let mut p = vec![evt::MASTER_VOLUME, 0, 0, 0];
            p.extend(t);
            p
        }),
    ]
}

proptest! {
    #[test]
    fn proptest_wp_p1_notify_decode_never_panics(p in prop::collection::vec(any::<u8>(), 0..120)) {
        let _ = notify::decode(&p);
    }

    #[test]
    fn proptest_wp_p1_notify_decode_never_panics_on_v2_shaped_packets(p in packet()) {
        match notify::decode(&p) {
            Ok((_, Event::ParamChanged { bytes, .. })) => {
                prop_assert!(bytes.len() + 12 <= p.len(), "the field fits the packet");
            }
            Ok(_) => {}
            Err(DecodeError::Short { got, need, .. }) => prop_assert!(got < need),
            Err(DecodeError::Version(v)) => prop_assert_ne!(v, PROTOCOL_VERSION),
        }
    }

    /// Whatever decodes can be applied to a mirror, which stays a V32 image
    /// with its header untouched.
    #[test]
    fn proptest_wp_p1_device_state_apply_never_panics(
        seed in any::<u64>(),
        packets in prop::collection::vec((packet(), any::<bool>(), any::<Option<u16>>()), 1..12),
    ) {
        let bulk = BulkPacket::decode(image(seed)).map_err(|e| TestCaseError::fail(e.to_string()))?;
        let header = bulk.as_bytes()[..16].to_vec();
        let mut s = DeviceState::new(caps(), bulk);
        for (p, lost, origin) in packets {
            let Ok((seq, event)) = notify::decode(&p) else { continue };
            let n = Notification {
                seq,
                event,
                lost,
                origin,
                received: std::time::Instant::now(),
            };
            let applied = s.apply(&n);
            if lost {
                prop_assert!(s.stale, "a lost packet marks the mirror stale");
                prop_assert!(
                    matches!(applied, Applied::NeedsReread { .. } | Applied::PresetLoaded { .. }),
                    "a lost packet asks for a re-read"
                );
            }
            prop_assert_eq!(s.bulk.as_bytes().len(), BULK_SIZE);
            prop_assert_eq!(&s.bulk.as_bytes()[..16], &header[..]);
        }
        let _ = s.has_unsaved_changes();
    }

    /// A `PARAM_CHANGED` inside the image lands exactly where it says.
    #[test]
    fn proptest_wp_p1_param_changed_patches_the_mirror(
        seed in any::<u64>(),
        offset in 16u16..BULK_SIZE as u16,
        field in prop::collection::vec(any::<u8>(), 1..=notify::MAX_NOTIFIED_FIELD),
    ) {
        let mut p = vec![PROTOCOL_VERSION, evt::PARAM_CHANGED, 0, 7];
        p.extend(offset.to_le_bytes());
        p.extend((field.len() as u16).to_le_bytes());
        p.extend([1, 0, 0, 0]);
        p.extend(&field);
        let (seq, event) = notify::decode(&p).map_err(|e| TestCaseError::fail(e.to_string()))?;
        prop_assert_eq!(seq, 7);
        let bulk = BulkPacket::decode(image(seed)).map_err(|e| TestCaseError::fail(e.to_string()))?;
        let mut s = DeviceState::new(caps(), bulk);
        let applied = s.apply(&Notification { seq, event, lost: false, origin: None, received: std::time::Instant::now() });
        let at = offset as usize;
        if at + field.len() <= BULK_SIZE {
            prop_assert!(matches!(applied, Applied::Section { .. }), "{applied:?}");
            prop_assert_eq!(&s.bulk.as_bytes()[at..at + field.len()], &field[..]);
            prop_assert!(!s.stale);
        } else {
            prop_assert!(matches!(applied, Applied::NeedsReread { .. }), "{applied:?}");
            prop_assert!(s.stale);
        }
    }
}
