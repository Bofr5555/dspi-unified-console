//! WP C8: names, output latency and clock diagnostics through `DspiCore`,
//! against `FakeDevice`. Refusals are proven by reading the device back.

use dspi_proto::generated::opcodes as op;
use dspi_transport::{Capture, FakeOptions};
use proptest::prelude::*;

use super::*;
use crate::ext::tests_wp_c8::{Faults, image_as, k340, k340_options, probed};

#[test]
fn names_round_trip_through_the_demo_device() {
    let p = k340();
    assert_eq!(p.core.get_input_source_name(2).unwrap(), "");
    p.core.set_input_source_name(2, "Turntable".into()).unwrap();
    assert_eq!(p.core.get_input_source_name(2).unwrap(), "Turntable");
    assert_eq!(p.core.get_input_source_name(1).unwrap(), "");

    p.core.set_device_name("Rack left".into()).unwrap();
    assert_eq!(p.core.get_device_name().unwrap(), "Rack left");

    // The longest name the field holds, and UTF-8.
    let long = "x".repeat(31);
    p.core.set_device_name(long.clone()).unwrap();
    assert_eq!(p.core.get_device_name().unwrap(), long);
    p.core.set_input_source_name(0, "Café".into()).unwrap();
    assert_eq!(p.core.get_input_source_name(0).unwrap(), "Café");

    // "" clears (a leading NUL on the wire).
    p.core.set_device_name(String::new()).unwrap();
    assert_eq!(p.core.get_device_name().unwrap(), "");
    p.core.set_input_source_name(2, String::new()).unwrap();
    assert_eq!(p.core.get_input_source_name(2).unwrap(), "");
}

#[test]
#[ignore = "proto Text trim fix"]
fn wp_c8_names_keep_their_spaces() {
    // `Repr::Text::decode` trims whitespace today; it should trim only NULs.
    let p = k340();
    p.core.set_device_name(" TV ".into()).unwrap();
    assert_eq!(p.core.get_device_name().unwrap(), " TV ");
    p.core.set_input_source_name(3, " TV ".into()).unwrap();
    assert_eq!(p.core.get_input_source_name(3).unwrap(), " TV ");
}

#[test]
fn wp_c8_bad_names_and_sources_are_refused_before_any_traffic() {
    let p = k340();
    p.core.set_device_name("Den".into()).unwrap();
    let before = p.traffic();
    let invalid = |r: Result<()>| matches!(r, Err(DspiError::InvalidArgument { .. }));
    assert!(invalid(p.core.set_device_name("y".repeat(32))));
    assert!(invalid(p.core.set_device_name("a\0b".into())));
    assert!(invalid(p.core.set_input_source_name(0, "é".repeat(16))));
    assert!(invalid(p.core.set_input_source_name(7, "Nope".into())));
    assert!(matches!(
        p.core.get_input_source_name(200),
        Err(DspiError::InvalidArgument { .. })
    ));
    assert_eq!(p.traffic(), before, "nothing sent");
    assert_eq!(p.core.get_device_name().unwrap(), "Den");
}

#[test]
fn wp_c8_firmware_without_names_or_latency_is_unsupported_and_never_written() {
    // Older firmware answers 0x55/0x57 with a legacy gain/mute: the probe
    // finds no names, and nothing may be sent to those opcodes.
    let p = probed(
        Capture::demo().bulk,
        FakeOptions {
            ack_unknown_sets: true,
            ..FakeOptions::default()
        },
        Faults {
            stall_reads: &[
                op::REQ_GET_INPUT_SOURCE_NAME,
                op::REQ_GET_DEVICE_NAME,
                op::REQ_OUTPUT_LATENCY,
            ],
            ..Faults::default()
        },
    );
    let before = p.traffic();
    assert_eq!(p.core.get_input_source_name(0), Err(DspiError::Unsupported));
    assert_eq!(
        p.core.set_input_source_name(0, "x".into()),
        Err(DspiError::Unsupported)
    );
    assert_eq!(p.core.get_device_name(), Err(DspiError::Unsupported));
    assert_eq!(
        p.core.set_device_name("x".into()),
        Err(DspiError::Unsupported)
    );
    assert_eq!(p.core.fetch_output_latency(), Err(DspiError::Unsupported));
    assert_eq!(
        p.core.fetch_output_latency_options(),
        Err(DspiError::Unsupported)
    );
    assert_eq!(p.core.set_output_latency(3), Err(DspiError::Unsupported));
    assert_eq!(p.traffic(), before, "nothing sent");
}

#[test]
fn wp_c8_a_read_only_connection_never_renames_or_changes_the_latency() {
    let p = probed(image_as(33, 6200), k340_options(), Faults::default());
    assert!(!p.core.lock().conn().unwrap().writable());
    let status = p.core.fetch_output_latency().unwrap();
    let outs = p.traffic().1;
    assert_eq!(
        p.core.set_input_source_name(1, "Phono".into()),
        Err(DspiError::ReadOnly)
    );
    assert_eq!(
        p.core.set_device_name("Den".into()),
        Err(DspiError::ReadOnly)
    );
    assert_eq!(p.core.set_output_latency(3), Err(DspiError::ReadOnly));
    assert_eq!(p.traffic().1, outs, "no OUT transfer");
    // Proven by read-back.
    assert_eq!(p.core.get_input_source_name(1).unwrap(), "");
    assert_eq!(p.core.get_device_name().unwrap(), "DSPi Demo");
    assert_eq!(p.core.fetch_output_latency().unwrap(), status);
}

fn field(l: &OutputLatency, name: &str) -> u32 {
    l.fields
        .iter()
        .find(|f| f.name == name)
        .unwrap_or_else(|| panic!("no {name}"))
        .value
        .parse()
        .unwrap()
}

#[test]
fn wp_c8_output_latency_status_options_and_set() {
    let p = k340();
    let s = p.core.fetch_output_latency().unwrap();
    assert_eq!(s.raw.len(), 12);
    let names: Vec<_> = s.fields.iter().map(|f| f.name.as_str()).collect();
    assert_eq!(
        names,
        [
            "requested_id",
            "effective_id",
            "effective_us",
            "target_frames",
            "batch_frames",
            "pending",
            "option_count",
        ]
    );
    // The 4 ms default at 48 kHz.
    assert_eq!(field(&s, "requested_id"), 5);
    assert_eq!(field(&s, "effective_us"), 4000);
    assert_eq!(field(&s, "target_frames"), 192);
    assert_eq!(field(&s, "option_count"), 7);

    let opts = p.core.fetch_output_latency_options().unwrap();
    assert_eq!(opts.len(), 7);
    assert_eq!(
        opts.iter().map(|o| o.id).collect::<Vec<_>>(),
        [1, 2, 3, 4, 5, 6, 7]
    );
    assert!(opts[0].stream_only && opts[1].stream_only && !opts[2].stream_only);
    assert_eq!((opts[0].latency_us, opts[0].batch_us), (667, 333));
    assert!(opts[2].available);

    p.core.set_output_latency(3).unwrap();
    let s = p.core.fetch_output_latency().unwrap();
    assert_eq!(
        (field(&s, "requested_id"), field(&s, "effective_id")),
        (3, 3)
    );
    assert_eq!(field(&s, "effective_us"), 2000);

    // A stream-only option is stored as asked; on USB input the first
    // option without the flag runs instead.
    p.core.set_output_latency(1).unwrap();
    let s = p.core.fetch_output_latency().unwrap();
    assert_eq!(field(&s, "requested_id"), 1);
    let runs_as_itself = opts[0].available;
    assert_eq!(
        field(&s, "effective_id"),
        if runs_as_itself { 1 } else { 3 }
    );
    if !runs_as_itself {
        assert_eq!(opts[0].effective_us, opts[2].latency_us, "the fallback");
    }

    // Not an option: refused before any traffic, proven by read-back.
    let before = p.traffic();
    for id in [0, 8, 0xFF] {
        assert!(matches!(
            p.core.set_output_latency(id),
            Err(DspiError::InvalidArgument { .. })
        ));
    }
    assert_eq!(p.traffic(), before);
    assert_eq!(
        field(&p.core.fetch_output_latency().unwrap(), "requested_id"),
        1
    );
}

#[test]
fn wp_c8_clock_diag_is_unavailable_on_a_build_without_it() {
    // The fake, like a build without DSPI_CLOCK_DIAG's opcode, stalls.
    let p = k340();
    assert_eq!(p.core.fetch_clock_diag().unwrap(), ClockDiag::default());
    let core = DspiCore::new();
    assert_eq!(core.fetch_clock_diag(), Err(DspiError::Disconnected));
    assert_eq!(core.get_device_name(), Err(DspiError::Disconnected));
    assert_eq!(core.fetch_output_latency(), Err(DspiError::Disconnected));
}

#[test]
fn wp_c8_clock_diag_reads_a_zero_length_reply_as_unavailable() {
    use dspi_transport::MockTransport;
    let t = MockTransport::new().data(op::REQ_GET_CLOCK_DIAG, vec![]);
    let core = DspiCore::new();
    core.attach(Box::new(t), "X".into(), 0);
    assert_eq!(core.fetch_clock_diag().unwrap(), ClockDiag::default());

    let packet = diag_packet(4, 2);
    let t = MockTransport::new().data(op::REQ_GET_CLOCK_DIAG, packet.clone());
    let core = DspiCore::new();
    core.attach(Box::new(t), "X".into(), 0);
    let d = core.fetch_clock_diag().unwrap();
    assert!(d.available);
    assert_eq!(d.raw, packet);
    assert_eq!(d.fields.len(), CLOCK_DIAG.len() - 1 - 5 + 5 * 4);
}

/// A `ClockDiagPacket` with `slots` slots: every byte its own offset (mod
/// 256), version `version`, `num_slots` = `slots`.
fn diag_packet(slots: u8, version: u8) -> Vec<u8> {
    let n = 4 + 16 + 38 + 22 + 36 + 11 * usize::from(slots) + 12 + 24 + 4;
    let n = if version >= 2 { n + 48 } else { n };
    let mut b: Vec<u8> = (0..n).map(|i| i as u8).collect();
    b[0] = version;
    b[1] = 1;
    b[2] = slots;
    b
}

fn value_of<'a>(f: &'a [NamedValue], name: &str) -> Option<&'a str> {
    f.iter().find(|v| v.name == name).map(|v| v.value.as_str())
}

#[test]
fn wp_c8_the_clock_diag_packet_decodes_in_order() {
    let b = diag_packet(4, 2);
    assert_eq!(b.len(), 248, "sizeof(ClockDiagPacket) on an RP2350");
    let f = clock_diag_fields(&b);
    assert_eq!(f[0].name, "version");
    assert_eq!(value_of(&f, "num_slots"), Some("4"));
    // sysclk_last_hz at offset 4: bytes 4..8.
    assert_eq!(
        value_of(&f, "sysclk_last_hz"),
        Some(u32::from_le_bytes([4, 5, 6, 7]).to_string().as_str())
    );
    // vcxo_last_cppm is signed, at offset 20.
    assert_eq!(
        value_of(&f, "vcxo_last_cppm"),
        Some(i32::from_le_bytes([20, 21, 22, 23]).to_string().as_str())
    );
    // Slot arrays start at 116: fill[4], min_fill[4], is_i2s[4], starv[4].
    assert_eq!(value_of(&f, "slot_fill[0]"), Some("116"));
    assert_eq!(value_of(&f, "slot_fill[3]"), Some("119"));
    assert_eq!(value_of(&f, "slot_is_i2s[3]"), Some("127"));
    assert_eq!(
        value_of(&f, "slot_starv[0]"),
        Some(
            u32::from_le_bytes([128, 129, 130, 131])
                .to_string()
                .as_str()
        )
    );
    // The servo before them ends at 116.
    assert_eq!(
        value_of(&f, "servo_ticks"),
        Some(
            u32::from_le_bytes([112, 113, 114, 115])
                .to_string()
                .as_str()
        )
    );
    assert!(value_of(&f, "slot_dwc[3]").is_some());
    assert!(value_of(&f, "_rsv").is_none(), "padding is skipped");
    assert_eq!(f.last().map(|v| v.name.as_str()), Some("fb_reanchors"));
    assert_eq!(
        value_of(&f, "fb_reanchors"),
        Some(
            u32::from_le_bytes([244, 245, 246, 247])
                .to_string()
                .as_str()
        )
    );

    // Version 1 ends at loop_gapmax_us; an RP2040 has two slots.
    let f = clock_diag_fields(&diag_packet(2, 1));
    assert_eq!(f.last().map(|v| v.name.as_str()), Some("loop_gapmax_us"));
    assert!(value_of(&f, "slot_fill[1]").is_some());
    assert!(value_of(&f, "slot_fill[2]").is_none());

    // A cut reply decodes as far as it goes; an absurd slot count stops
    // before the arrays.
    let f = clock_diag_fields(&b[..10]);
    assert_eq!(f.len(), 5, "the header and sysclk_last_hz");
    let mut bad = b.clone();
    bad[2] = 200;
    let f = clock_diag_fields(&bad);
    assert_eq!(f.last().map(|v| v.name.as_str()), Some("servo_ticks"));
    assert!(clock_diag_fields(&[]).is_empty());
}

#[test]
fn wp_c8_device_names_are_cut_at_nul_and_31_bytes() {
    assert_eq!(decode_name(b"Movie\0junk"), "Movie");
    assert_eq!(decode_name(&[0; 32]), "");
    assert_eq!(decode_name(&[b'a'; 40]), "a".repeat(31));
    assert_eq!(decode_name(&[0xFF, b'b', 0]), "\u{FFFD}b");
    assert_eq!(decode_name(&[]), "");
}

#[test]
fn wp_c8_short_latency_replies_are_errors_not_panics() {
    assert!(latency_fields(&[5; 11]).is_none());
    assert_eq!(latency_fields(&[5; 12]).map(|f| f.len()), Some(7));
    // A trailing partial option entry is ignored.
    assert_eq!(latency_options(&[1; 8 * 2 + 5]).len(), 2);
    assert!(latency_options(&[]).is_empty());
}

proptest! {
    #[test]
    fn proptest_wp_c8_device_replies_never_panic(
        b in proptest::collection::vec(any::<u8>(), 0..400)
    ) {
        let f = clock_diag_fields(&b);
        prop_assert!(f.len() <= CLOCK_DIAG.len() + 5 * usize::from(MAX_DIAG_SLOTS));
        let _ = latency_fields(&b);
        prop_assert_eq!(latency_options(&b).len(), b.len() / 8);
        prop_assert!(decode_name(&b).len() <= 31 * 3);
    }
}
