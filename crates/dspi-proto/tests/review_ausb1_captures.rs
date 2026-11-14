//! Review ausb1: the `AlinkUsbStatus` codec against the K340's real
//! `EXT_ALINK_USB_STATUS` (0x8011) replies.
//!
//! `data/ausb1_k340_0x8011_hour_raw.log` is the bench's hour capture (K340 on
//! bofr-main, `flash-tools/alink-usb-test/hour-raw.log`: a timestamp, then the
//! 64 reply bytes as hex, one read a minute). `data/ausb1_k340_0x8011_expected.json`
//! holds what the firmware's own decoder (`tools/alink_usb/alink_usb_status.py`,
//! `struct.unpack("<BBBBfffQHHHHHHIiIIIII")`) makes of each line, the f32 fields
//! as their bit patterns, plus that tool's `offset(A, B)` for each pair of
//! consecutive captures (its 2.9.3 start-offset formula). Every field must agree
//! byte for byte, and `content_lead` must give the tool's offsets.
#![allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

use dspi_proto::ext::alink_usb::{AlinkUsbStatus, NO_STREAM, UsbState, content_lead};
use serde_json::Value;

fn u(v: &Value, k: &str) -> u64 {
    v[k].as_u64().unwrap_or_else(|| panic!("{k}"))
}

#[test]
fn review_ausb1_the_status_codec_matches_the_firmware_tool_on_real_captures() {
    let raw = include_str!("data/ausb1_k340_0x8011_hour_raw.log");
    let expected: Value =
        serde_json::from_str(include_str!("data/ausb1_k340_0x8011_expected.json")).unwrap();
    let rows = expected["rows"].as_array().unwrap();
    let lines: Vec<(&str, Vec<u8>)> = raw
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| {
            let (t, hex) = l.split_once(' ').unwrap();
            let bytes = (0..hex.len())
                .step_by(2)
                .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
                .collect();
            (t, bytes)
        })
        .collect();
    assert_eq!(lines.len(), rows.len());
    assert_eq!(lines.len(), 59, "the hour's captures");
    let mut decoded = Vec::new();
    for ((t, bytes), e) in lines.iter().zip(rows) {
        assert_eq!(bytes.len(), 64, "{t}");
        assert_eq!(e["t"].as_str().unwrap(), *t);
        let s = AlinkUsbStatus::decode(bytes).unwrap_or_else(|| panic!("{t}: no decode"));
        assert_eq!(u64::from(s.state.raw()), u(e, "state"), "{t} state");
        assert_eq!(s.state, UsbState::PhaseLocked, "{t}");
        assert_eq!(u64::from(s.phase_state), u(e, "phase_state"), "{t}");
        assert_eq!(u64::from(s.rate_code), u(e, "rate_code"), "{t}");
        assert_eq!(u64::from(s.flags), u(e, "flags"), "{t}");
        assert_eq!(
            u64::from(s.host_ppm.to_bits()),
            u(e, "host_ppm"),
            "{t} host_ppm"
        );
        assert_eq!(
            u64::from(s.vcxo_ppm.to_bits()),
            u(e, "vcxo_ppm"),
            "{t} vcxo_ppm"
        );
        assert_eq!(
            u64::from(s.phase_error.to_bits()),
            u(e, "phase_error"),
            "{t} phase_error"
        );
        assert_eq!(s.sample, u(e, "sample"), "{t} sample");
        assert_eq!(u64::from(s.bus_frame), u(e, "bus_frame"), "{t}");
        assert_eq!(u64::from(s.d_min), u(e, "d_min"), "{t}");
        assert_eq!(u64::from(s.d_max), u(e, "d_max"), "{t}");
        assert_eq!(u64::from(s.fill), u(e, "fill"), "{t}");
        assert_eq!(
            u64::from(s.stream_first_bus_frame),
            u(e, "stream_first_bus_frame"),
            "{t}"
        );
        assert_eq!(u64::from(s.gap_count), u(e, "gap_count"), "{t}");
        assert_eq!(
            u64::from(s.stream_age_frames),
            u(e, "stream_age_frames"),
            "{t}"
        );
        assert_eq!(
            i64::from(s.stream_shift),
            e["stream_shift"].as_i64().unwrap(),
            "{t}"
        );
        assert_eq!(u64::from(s.stream_samples), u(e, "stream_samples"), "{t}");
        assert_eq!(u64::from(s.anchors), u(e, "anchors"), "{t}");
        assert_eq!(u64::from(s.reanchors), u(e, "reanchors"), "{t}");
        assert_eq!(u64::from(s.missed_samples), u(e, "missed_samples"), "{t}");
        assert_eq!(u64::from(s.late_samples), u(e, "late_samples"), "{t}");
        // The hub's stream start (2.9.3) is the tool's `(bus - age) mod 2048`;
        // on these captures it is also what the firmware reports directly.
        let start = s.stream_start_bus_frame();
        if s.stream_first_bus_frame == NO_STREAM {
            assert_eq!(start, None, "{t}");
        } else {
            assert_eq!(u64::from(start.unwrap()), u(e, "first_start"), "{t}");
            assert_eq!(start, Some(s.stream_first_bus_frame), "{t}");
        }
        // Encode is the inverse on real bytes.
        assert_eq!(&s.encode()[..], &bytes[..], "{t} round trip");
        decoded.push(s);
    }
    // The real K340's figures are what the bench reported (hour.log, RESULTS.md).
    let last = decoded.last().unwrap();
    assert_eq!((last.d_min, last.d_max, last.rate_code), (170, 399, 1));
    // No gap, no missed or late sample in the hour; the unit anchored again
    // several times (anchors 1 -> 9, one re-anchor), which is the firmware's
    // business, not the codec's.
    assert!(
        decoded
            .iter()
            .all(|s| s.gap_count == 0 && s.missed_samples == 0 && s.late_samples == 0)
    );
    assert!(decoded.windows(2).all(|w| w[0].anchors <= w[1].anchors));
    assert_eq!(
        (decoded[0].anchors, last.anchors, last.reanchors),
        (1, 9, 1)
    );
    assert!(
        decoded
            .iter()
            .all(|s| s.host_ppm < -9.0 && s.host_ppm > -13.0),
        "the host about 11 ppm slow"
    );
    // `content_lead` is the tool's `offset(A, B)` for consecutive captures
    // (A the earlier): the same stream gives 0, a restarted one its new start.
    let offsets = expected["pair_offsets"].as_array().unwrap();
    assert_eq!(offsets.len(), decoded.len() - 1);
    let mut nonzero = 0;
    for (i, (a, b)) in decoded.iter().zip(decoded.iter().skip(1)).enumerate() {
        let (Some(fa), Some(fb)) = (a.stream_start_bus_frame(), b.stream_start_bus_frame()) else {
            continue;
        };
        let lead = content_lead((fa, a.stream_shift), (fb, b.stream_shift), a.spf().unwrap());
        assert_eq!(lead, offsets[i].as_i64().unwrap(), "pair {i}");
        nonzero += u32::from(lead != 0);
    }
    assert!(
        nonzero >= 5,
        "the player looped the stream during the hour: {nonzero}"
    );
}
