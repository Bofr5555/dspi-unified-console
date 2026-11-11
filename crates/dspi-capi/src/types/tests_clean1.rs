//! The records' defaults, the fill rules of the conversions, and the shape of
//! the generated constants file that the Kotlin and Swift generators read.

use super::*;

#[test]
fn clean1_empty_analyser_records_read_as_the_floor() {
    let frame = RtaChannelFrame::default();
    assert_eq!(frame.n_bands, 0);
    assert!(
        frame
            .avg_db
            .iter()
            .chain(&frame.peak_db)
            .all(|&v| v == RTA_FLOOR_DB)
    );

    let snap = RtaSnapshot::default();
    assert_eq!(snap.count, 0);
    assert!(snap.bins_db.iter().all(|&v| v == RTA_FLOOR_DB));
    assert!(snap.frames.iter().all(|f| *f == RtaChannelFrame::default()));
    assert_eq!(RtaCapsInfo::default().centres, [0; RTA_MAX_BANDS]);
}

#[test]
fn clean1_a_short_frame_is_padded_at_the_floor() {
    let rec = r::RtaChannelFrame {
        channel: 3,
        n_bands: 2,
        silent: false,
        avg_db: vec![-10.0, -20.0],
        peak_db: vec![-5.0; 50],
    };
    let c = RtaChannelFrame::from(&rec);
    assert_eq!(c.channel, 3);
    assert_eq!(c.avg_db[..3], [-10.0, -20.0, RTA_FLOOR_DB]);
    assert_eq!(c.peak_db, [-5.0; RTA_MAX_BANDS], "a long vector is cut");

    let snap = r::RtaSnapshot {
        count: 1,
        frames: vec![rec],
        bins_db: vec![-1.0; 3],
        ..Default::default()
    };
    let c = RtaSnapshot::from(&snap);
    assert_eq!(c.frames[0].channel, 3);
    assert_eq!(c.frames[1], RtaChannelFrame::default());
    assert_eq!(c.bins_db[..4], [-1.0, -1.0, -1.0, RTA_FLOOR_DB]);
}

#[test]
fn clean1_a_device_info_round_trips_through_the_c_record() {
    assert_eq!(DeviceInfo::default().serial, [0; 64]);
    let rec = r::DeviceInfo {
        serial: "ÅÄÖ-1".into(),
        location_id: 0x1234,
    };
    let c = DeviceInfo::from(&rec);
    assert_eq!(c.serial_len as usize, "ÅÄÖ-1".len());
    assert_eq!(r::DeviceInfo::from(&c), rec);

    // A length short of the NUL cuts the text there.
    let short = DeviceInfo { serial_len: 2, ..c };
    assert_eq!(r::DeviceInfo::from(&short).serial, "Å");
}

#[test]
fn clean1_names_fill_with_nul_and_never_split_a_character() {
    let b: [u8; 5] = name_buf("ab€");
    // "ab" + 3-byte "€" is 5 bytes; only 4 fit before the NUL.
    assert_eq!(b, *b"ab\0\0\0");
    let b: [u8; 1] = name_buf("x");
    assert_eq!(b, [0]);
    let b: [u8; 6] = name_buf("");
    assert_eq!(b, [0; 6]);
}

#[test]
fn clean1_small_records_copy_every_field() {
    let s = r::SpdifRxStatus {
        state: 1,
        input_source: 2,
        lock_count: 3,
        loss_count: 4,
        sample_rate: 48_000,
        parity_errors: 6,
        fifo_fill_pct: 7,
        lib_state: 8,
        stable_callbacks: 9,
        lost_callbacks: 10,
    };
    let c = SpdifRxStatus::from(&s);
    assert_eq!(
        (
            c.state,
            c.lock_count,
            c.sample_rate,
            c.fifo_fill_pct,
            c.lost_callbacks
        ),
        (1, 3, 48_000, 7, 10)
    );

    let o = r::RtaOwnership {
        following: true,
        origin: 7,
        tap: RTA_TAP_OUTPUT,
        mask: 0x3,
        fft_order: 10,
    };
    let c = RtaOwnership::from(&o);
    assert_eq!(
        (c.following, c.origin, c.tap, c.mask, c.fft_order),
        (true, 7, 1, 3, 10)
    );

    let b = r::BufferStats {
        num_spdif: 1,
        spdif: vec![r::SpdifBufferStats {
            fill_pct: 50,
            ..Default::default()
        }],
        pdm: r::PdmBufferStats {
            ring_max_pct: 90,
            ..Default::default()
        },
        ..Default::default()
    };
    let c = BufferStats::from(&b);
    assert_eq!(c.spdif[0].fill_pct, 50);
    assert_eq!(c.spdif[1], SpdifBufferStats::default());
    assert_eq!(c.pdm.ring_max_pct, 90);

    let status = r::SystemStatus {
        peaks: vec![0.5; 3],
        cpu0: 40,
        stale: true,
        age_ms: 9,
        ..Default::default()
    };
    let c = SystemStatus::from(&status);
    assert_eq!(c.peaks[..4], [0.5, 0.5, 0.5, 0.0]);
    assert_eq!(c.cpu0, 40);
}

#[test]
fn clean1_a_siggen_config_keeps_four_parameters() {
    let rec = r::SiggenConfig {
        p: vec![1.0, 2.0],
        ..Default::default()
    };
    let c = SiggenConfig::from(&rec);
    assert_eq!(c.p, [1.0, 2.0, 0.0, 0.0]);
    assert_eq!(r::SiggenConfig::from(&c).p, vec![1.0, 2.0, 0.0, 0.0]);
}

/// The generated file keeps the form its readers rely on: the generated
/// banner first, one `pub const NAME: TYPE = LITERAL;` per line, literals
/// only (cbindgen, `gen_kotlin_contract.py`, `gen_swift_constants.py`).
#[test]
fn clean1_the_constants_file_holds_one_literal_per_line() {
    let text = include_str!("consts.rs");
    let first = text.lines().next().unwrap_or_default();
    assert!(first.starts_with("// Generated, do not edit"), "{first}");
    assert!(
        first.contains("cargo run -p xtask -- capi-consts"),
        "{first}"
    );

    let mut count = 0;
    for line in text.lines().filter(|l| l.starts_with("pub const ")) {
        count += 1;
        let (_, value) = line.split_once(" = ").unwrap_or_else(|| panic!("{line}"));
        let value = value.strip_suffix(';').unwrap_or_else(|| panic!("{line}"));
        let literal = value.strip_prefix('-').unwrap_or(value);
        let ok = literal.parse::<f64>().is_ok()
            || literal
                .strip_prefix("0x")
                .is_some_and(|h| u64::from_str_radix(h, 16).is_ok())
            || literal
                .split_once(" << ")
                .is_some_and(|(a, b)| a == "1" && b.parse::<u32>().is_ok());
        assert!(ok, "not a literal: {line}");
    }
    assert!(count > 400, "{count} constants");
}
