//! WP Q1 tests: the records' layout against the old header (C compiler),
//! the generated header against the old one (text), the constants against
//! the firmware-generated ones, and the conversions.

use std::collections::BTreeMap;
use std::io::Write;
use std::mem::{align_of, offset_of, size_of};
use std::path::PathBuf;
use std::process::{Command, Stdio};

use super::*;

fn manifest() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// The old desktop core's header: a committed snapshot
/// (`tests/data/dspi_core_old.h`); the crate itself is gone (WP Q3).
fn old_header() -> String {
    std::fs::read_to_string(manifest().join("tests/data/dspi_core_old.h")).unwrap()
}

/// The header build.rs wrote.
fn new_header() -> String {
    std::fs::read_to_string(manifest().join("include/dspi_core.h")).unwrap()
}

/// What a header defines, comments stripped and whitespace collapsed:
/// `#define NAME` -> value, `struct NAME` -> body, `typedef NAME` -> the
/// whole typedef (callbacks), `opaque NAME` for forward-declared structs.
fn items(h: &str) -> BTreeMap<String, String> {
    let mut s = String::new();
    let mut rest = h;
    while let Some(i) = rest.find("/*") {
        s.push_str(&rest[..i]);
        rest = rest[i..].find("*/").map_or("", |j| &rest[i + j + 2..]);
    }
    s.push_str(rest);
    let norm = |t: &str| t.split_whitespace().collect::<Vec<_>>().join(" ");

    let mut out = BTreeMap::new();
    let mut lines = s.lines();
    while let Some(line) = lines.next() {
        let line = line.trim();
        if let Some(d) = line.strip_prefix("#define ") {
            let (name, value) = d.split_once(' ').unwrap_or((d, ""));
            if name != "DSPI_CORE_H" {
                out.insert(format!("#define {name}"), norm(value));
            }
        } else if let Some(t) = line.strip_prefix("typedef struct ") {
            if let Some(name) = t.strip_suffix(" {") {
                let mut body = Vec::new();
                for l in lines.by_ref() {
                    if l.trim_start().starts_with('}') {
                        break;
                    }
                    body.push(l);
                }
                out.insert(format!("struct {name}"), norm(&body.join(" ")));
            } else {
                let name = t.split_whitespace().next().unwrap_or("");
                out.insert(format!("opaque {name}"), norm(line));
            }
        } else if line.starts_with("typedef ") {
            let mut whole = line.to_string();
            while !whole.ends_with(';') {
                match lines.next() {
                    Some(l) => whole.push_str(l),
                    None => break,
                }
            }
            let name = whole
                .split("(*")
                .nth(1)
                .and_then(|t| t.split(')').next())
                .unwrap_or(&whole)
                .to_string();
            out.insert(format!("typedef {name}"), norm(&whole));
        }
    }
    out
}

#[test]
fn wp_q1_header_matches_the_old_one() {
    let old = items(&old_header());
    let new = items(&new_header());
    // Sanity: the parser found what the old header has.
    assert!(old.keys().filter(|k| k.starts_with("#define")).count() > 400);
    assert!(old.contains_key("struct DspState"));
    assert!(old.contains_key("typedef NotifyCallback"));

    let mut problems = Vec::new();
    for (key, value) in &old {
        // Forward declarations come with the functions that take them
        // (the core handle, WP Q2a); they have no layout to compare.
        if key.starts_with("opaque ") {
            continue;
        }
        match new.get(key) {
            None => problems.push(format!("missing: {key}")),
            Some(v) if v != value => {
                problems.push(format!("differs: {key}\n  old: {value}\n  new: {v}"))
            }
            Some(_) => {}
        }
    }
    assert!(
        problems.is_empty(),
        "include/dspi_core.h vs the old header:\n{}",
        problems.join("\n")
    );
}

/// Name, size, alignment and field offsets of the records, from Rust.
type Layout = (&'static str, usize, usize, Vec<(&'static str, usize)>);

macro_rules! layouts {
    ($($s:ident { $($f:ident),* $(,)? })*) => {
        vec![$((
            stringify!($s),
            size_of::<$s>(),
            align_of::<$s>(),
            vec![$((stringify!($f), offset_of!($s, $f))),*],
        )),*]
    };
}

fn rust_layouts() -> Vec<Layout> {
    layouts! {
        DeviceInfo { serial, serial_len, location_id }
        SystemStatus { peaks, cpu0, cpu1, clip_flags, active_input_channels, num_channels }
        FilterParams { filter_type, bypass, freq, q, gain, qp }
        UartConfig { enabled, tx_pin, rx_pin, notify, baud }
        I2cConfig { enabled, sda_pin, scl_pin, address }
        CtrlIfaceStatus { uart_last_status, uart_live, i2c_last_status, i2c_live, protocol_version }
        DspState { platform_id, num_channels, num_input_channels, num_output_channels, format_version, fw_major, fw_minor, fw_patch, fw_beta, compat, preamp_db, bypass, input_preamp_db, master_volume_db, master_volume_mode, user_volume_db, user_mute, loudness_enabled, loudness_ref_spl, loudness_intensity, loudness_output_mask, crossfeed_enabled, crossfeed_preset, crossfeed_freq, crossfeed_feed, crossfeed_itd, crossfeed_output_pair_mask, channel_delays, matrix_routing, matrix_gain, matrix_invert, output_enabled, output_muted, output_gain_db, output_delay_ms, limiter_enabled, limiter_link_group, limiter_threshold_db, limiter_release_ms, num_pin_outputs, output_pins, output_config_mode, filters, xover, channel_names, leveller_enabled, leveller_speed, leveller_lookahead, leveller_amount, leveller_max_gain_db, leveller_gate_db, leveller_detector_mask, leveller_apply_mask, input_source, lg_sound_sync_enabled, spdif_inputs_enabled, adat_input_enabled, spdif_rx_pins, i2s_rx_pins, i2s_input_channels, i2s_input_rate, i2s_clock_mode, adat_input_pin, adat_input_clock_mode, psybass_enabled, psybass_output_mask, psybass_cutoff_hz, psybass_harmonics_db, psybass_drive_db, psybass_character_pct, psybass_original_db, upmix, subharm, subharm_solo, tube, output_types, i2s_bck_pin, i2s_bck_pin_slave, i2s_clock_pin_mode, mck_enabled, mck_pin, mck_multiplier, adat_out_enabled, adat_out_pin, dac_mute_supported, dac_mute_enabled, dac_mute_active_low, dac_mute_pin, dac_mute_hold_ms, dac_mute_release_ms, ctrl_iface_supported, uart, i2c, ctrl_status, core1_mode, preset_occupied, preset_names, active_preset_slot, preset_startup_mode, preset_default_slot, bulk_valid, bulk_raw }
        PresetDirectory { occupied_mask, startup_mode, default_slot, last_active, output_config_mode, master_volume_mode }
        UpmixStatus { active, parked_reason, correlation, balance, center_gain, ls_gain, rs_gain }
        InputLockStatus { state, clock_mode, detected_rate, measured_hz, rate_ok, lock_count, loss_count }
        AdatOutStatus { enabled, active, pin, rate_ok, resync_count, slip_count }
        LgStatus { enabled, present, volume, muted }
        NotifyResult { flags, filter_channels }
        RtaRequest { tap, mask, wants_bins, fft_order, avg_ms, peak_decay_db_s }
        RtaCapsInfo { supported, input_channels, output_channels, order_min, order_max, order_default, bass_bands, max_bands, dynamic_range_db, bass_dynamic_range_db, centres }
        RtaChannelFrame { channel, n_bands, silent, avg_db, peak_db }
        RtaSnapshot { version, telemetry_version, tap, count, frames, populated, bins_count, bins_channel, bins_sample_rate, bins_db, running, live_count, live_mask, frames_per_s, busy_us_per_s, last_frame_us, bass_busy_us_per_s, sample_rate, first_band, fft_order, config_rejected, refresh_interval_s }
        SiggenParam { semantic, min, max, def }
        SiggenTypeInfo { id, name, timing, params }
        SiggenCaps { supported, type_count, output_channels, multitone_max, valid_mask, types }
        SiggenConfig { signal_type, channel_mask, invert_mask, flags, level_db, duration_ms, repeat, gap_ms, p }
        SiggenStatus { state, signal_type, active_channel, elapsed_ms, cycles_done, stop_reason, current_freq }
        SpdifBufferStats { free, prepared, playing, fill_pct, min_pct, max_pct }
        PdmBufferStats { dma_fill_pct, dma_min_pct, dma_max_pct, ring_fill_pct, ring_min_pct, ring_max_pct }
        BufferStats { num_spdif, pdm_active, streaming, sequence, spdif, pdm }
        SpdifRxStatus { state, input_source, lock_count, loss_count, sample_rate, parity_errors, fifo_fill_pct, lib_state, stable_callbacks, lost_callbacks }
        PlatformInfo { platform_id, num_output_channels, fw_major, fw_minor, fw_patch, fw_beta }
    }
}

/// Field names of each struct in a header, in order.
fn header_fields(h: &str) -> BTreeMap<String, Vec<String>> {
    items(h)
        .into_iter()
        .filter_map(|(k, body)| {
            let name = k.strip_prefix("struct ")?.to_string();
            let fields = body
                .split(';')
                .filter(|d| !d.trim().is_empty())
                .map(|d| {
                    let d = d.split('[').next().unwrap_or(d);
                    d.split_whitespace().last().unwrap_or("").to_string()
                })
                .collect();
            Some((name, fields))
        })
        .collect()
}

/// The first C compiler found: `$CC`, then `cc`, `gcc`, `clang`.
fn c_compiler() -> Command {
    let candidates = std::env::var("CC")
        .into_iter()
        .chain(["cc", "gcc", "clang"].map(String::from));
    for cc in candidates {
        let found = Command::new(&cc)
            .arg("--version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|s| s.success());
        if found {
            return Command::new(cc);
        }
    }
    panic!(
        "the layout tests need a C compiler ($CC, cc, gcc or clang); \
         run them in the build container: ci/run.sh 'cargo test -p dspi-capi'"
    );
}

/// Compile `header` plus `_Static_assert`s of the Rust layouts with the C
/// compiler; a mismatch fails the compile and names the field.
fn check_layout_in_c(what: &str, header: &str) {
    let mut c = String::from("#include <stddef.h>\n");
    c.push_str(header);
    c.push('\n');
    for (s, size, align, fields) in rust_layouts() {
        c.push_str(&format!(
            "_Static_assert(sizeof(struct {s}) == {size}, \"{s}: size\");\n\
             _Static_assert(_Alignof(struct {s}) == {align}, \"{s}: align\");\n"
        ));
        for (f, off) in fields {
            c.push_str(&format!(
                "_Static_assert(offsetof(struct {s}, {f}) == {off}, \"{s}.{f}: offset\");\n"
            ));
        }
    }
    let mut child = c_compiler()
        .args(["-std=c11", "-fsyntax-only", "-x", "c", "-"])
        .stdin(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(c.as_bytes()).unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(
        out.status.success(),
        "{what}: the Rust layouts differ:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn wp_q1_every_old_struct_field_is_checked() {
    let old = header_fields(&old_header());
    let rust: BTreeMap<String, Vec<String>> = rust_layouts()
        .into_iter()
        .map(|(s, _, _, f)| {
            (
                s.to_string(),
                f.into_iter().map(|(n, _)| n.to_string()).collect(),
            )
        })
        .collect();
    assert_eq!(
        rust, old,
        "the layout list must name every struct and field of the old header"
    );
}

#[test]
fn wp_q1_layout_matches_the_old_header() {
    check_layout_in_c("old header", &old_header());
}

#[test]
fn wp_q1_layout_matches_the_new_header() {
    check_layout_in_c("include/dspi_core.h", &new_header());
}

#[test]
fn wp_q1_known_sizes() {
    // The sizes the Qt app was built against (x86_64 / aarch64 / MSVC alike:
    // no pointers, no 8-byte fields but RtaSnapshot::populated).
    assert_eq!(size_of::<FilterParams>(), 20);
    assert_eq!(size_of::<DspState>(), 12812);
    assert_eq!(size_of::<BulkCheck>(), BULK_PARAMS_SIZE);
}

/// The bulk image fits the record's buffer exactly.
type BulkCheck = [u8; BULK_PARAMS_SIZE];

#[test]
fn wp_q1_constants_match_the_generated_ones() {
    use dspi_proto::generated as g;
    assert_eq!(
        REQ_SET_CS_AUX_STATE as i128,
        g::opcodes::REQ_SET_CS_AUX_STATE as i128,
        "REQ_SET_CS_AUX_STATE"
    );
    assert_eq!(
        REQ_GET_CS_AUX_STATE as i128,
        g::opcodes::REQ_GET_CS_AUX_STATE as i128,
        "REQ_GET_CS_AUX_STATE"
    );
    assert_eq!(
        REQ_SET_CS_AUX_LEVEL as i128,
        g::opcodes::REQ_SET_CS_AUX_LEVEL as i128,
        "REQ_SET_CS_AUX_LEVEL"
    );
    assert_eq!(
        REQ_GET_CS_AUX_LEVEL as i128,
        g::opcodes::REQ_GET_CS_AUX_LEVEL as i128,
        "REQ_GET_CS_AUX_LEVEL"
    );
    assert_eq!(
        REQ_RTA_SET_CONFIG as i128,
        g::opcodes::REQ_RTA_SET_CONFIG as i128,
        "REQ_RTA_SET_CONFIG"
    );
    assert_eq!(
        REQ_RTA_GET_CONFIG as i128,
        g::opcodes::REQ_RTA_GET_CONFIG as i128,
        "REQ_RTA_GET_CONFIG"
    );
    assert_eq!(
        REQ_RTA_GET_CAPS as i128,
        g::opcodes::REQ_RTA_GET_CAPS as i128,
        "REQ_RTA_GET_CAPS"
    );
    assert_eq!(
        REQ_RTA_GET_BANDS as i128,
        g::opcodes::REQ_RTA_GET_BANDS as i128,
        "REQ_RTA_GET_BANDS"
    );
    assert_eq!(
        REQ_RTA_GET_BINS as i128,
        g::opcodes::REQ_RTA_GET_BINS as i128,
        "REQ_RTA_GET_BINS"
    );
    assert_eq!(
        REQ_RTA_GET_STATUS as i128,
        g::opcodes::REQ_RTA_GET_STATUS as i128,
        "REQ_RTA_GET_STATUS"
    );
    assert_eq!(
        REQ_RTA_CONTROL as i128,
        g::opcodes::REQ_RTA_CONTROL as i128,
        "REQ_RTA_CONTROL"
    );
    assert_eq!(
        REQ_RTA_GET_BANDS_ALL as i128,
        g::opcodes::REQ_RTA_GET_BANDS_ALL as i128,
        "REQ_RTA_GET_BANDS_ALL"
    );
    assert_eq!(
        REQ_SET_SUBHARM as i128,
        g::opcodes::REQ_SET_SUBHARM as i128,
        "REQ_SET_SUBHARM"
    );
    assert_eq!(
        REQ_GET_SUBHARM as i128,
        g::opcodes::REQ_GET_SUBHARM as i128,
        "REQ_GET_SUBHARM"
    );
    assert_eq!(
        REQ_SET_SUBHARM_LOW as i128,
        g::opcodes::REQ_SET_SUBHARM_LOW as i128,
        "REQ_SET_SUBHARM_LOW"
    );
    assert_eq!(
        REQ_GET_SUBHARM_LOW as i128,
        g::opcodes::REQ_GET_SUBHARM_LOW as i128,
        "REQ_GET_SUBHARM_LOW"
    );
    assert_eq!(
        REQ_SET_SUBHARM_HIGH as i128,
        g::opcodes::REQ_SET_SUBHARM_HIGH as i128,
        "REQ_SET_SUBHARM_HIGH"
    );
    assert_eq!(
        REQ_GET_SUBHARM_HIGH as i128,
        g::opcodes::REQ_GET_SUBHARM_HIGH as i128,
        "REQ_GET_SUBHARM_HIGH"
    );
    assert_eq!(
        REQ_SET_SUBHARM_BOOST as i128,
        g::opcodes::REQ_SET_SUBHARM_BOOST as i128,
        "REQ_SET_SUBHARM_BOOST"
    );
    assert_eq!(
        REQ_GET_SUBHARM_BOOST as i128,
        g::opcodes::REQ_GET_SUBHARM_BOOST as i128,
        "REQ_GET_SUBHARM_BOOST"
    );
    assert_eq!(
        REQ_SET_SUBHARM_MASK as i128,
        g::opcodes::REQ_SET_SUBHARM_MASK as i128,
        "REQ_SET_SUBHARM_MASK"
    );
    assert_eq!(
        REQ_GET_SUBHARM_MASK as i128,
        g::opcodes::REQ_GET_SUBHARM_MASK as i128,
        "REQ_GET_SUBHARM_MASK"
    );
    assert_eq!(
        REQ_GET_SUBHARM_HEADROOM as i128,
        g::opcodes::REQ_GET_SUBHARM_HEADROOM as i128,
        "REQ_GET_SUBHARM_HEADROOM"
    );
    assert_eq!(
        REQ_SET_SUBHARM_TOP as i128,
        g::opcodes::REQ_SET_SUBHARM_TOP as i128,
        "REQ_SET_SUBHARM_TOP"
    );
    assert_eq!(
        REQ_GET_SUBHARM_TOP as i128,
        g::opcodes::REQ_GET_SUBHARM_TOP as i128,
        "REQ_GET_SUBHARM_TOP"
    );
    assert_eq!(
        REQ_SET_SUBHARM_SELECT as i128,
        g::opcodes::REQ_SET_SUBHARM_SELECT as i128,
        "REQ_SET_SUBHARM_SELECT"
    );
    assert_eq!(
        REQ_GET_SUBHARM_SELECT as i128,
        g::opcodes::REQ_GET_SUBHARM_SELECT as i128,
        "REQ_GET_SUBHARM_SELECT"
    );
    assert_eq!(
        REQ_GET_SUBHARM_METER as i128,
        g::opcodes::REQ_GET_SUBHARM_METER as i128,
        "REQ_GET_SUBHARM_METER"
    );
    assert_eq!(
        REQ_SET_SUBHARM_SOLO as i128,
        g::opcodes::REQ_SET_SUBHARM_SOLO as i128,
        "REQ_SET_SUBHARM_SOLO"
    );
    assert_eq!(
        REQ_GET_SUBHARM_SOLO as i128,
        g::opcodes::REQ_GET_SUBHARM_SOLO as i128,
        "REQ_GET_SUBHARM_SOLO"
    );
    assert_eq!(
        REQ_SET_SUBHARM_LINK as i128,
        g::opcodes::REQ_SET_SUBHARM_LINK as i128,
        "REQ_SET_SUBHARM_LINK"
    );
    assert_eq!(
        REQ_GET_SUBHARM_LINK as i128,
        g::opcodes::REQ_GET_SUBHARM_LINK as i128,
        "REQ_GET_SUBHARM_LINK"
    );
    assert_eq!(
        REQ_SET_SUBHARM_DEPTH as i128,
        g::opcodes::REQ_SET_SUBHARM_DEPTH as i128,
        "REQ_SET_SUBHARM_DEPTH"
    );
    assert_eq!(
        REQ_GET_SUBHARM_DEPTH as i128,
        g::opcodes::REQ_GET_SUBHARM_DEPTH as i128,
        "REQ_GET_SUBHARM_DEPTH"
    );
    assert_eq!(
        REQ_SET_SUBHARM_HOLD as i128,
        g::opcodes::REQ_SET_SUBHARM_HOLD as i128,
        "REQ_SET_SUBHARM_HOLD"
    );
    assert_eq!(
        REQ_GET_SUBHARM_HOLD as i128,
        g::opcodes::REQ_GET_SUBHARM_HOLD as i128,
        "REQ_GET_SUBHARM_HOLD"
    );
    assert_eq!(
        REQ_SET_SUBHARM_CEILING as i128,
        g::opcodes::REQ_SET_SUBHARM_CEILING as i128,
        "REQ_SET_SUBHARM_CEILING"
    );
    assert_eq!(
        REQ_GET_SUBHARM_CEILING as i128,
        g::opcodes::REQ_GET_SUBHARM_CEILING as i128,
        "REQ_GET_SUBHARM_CEILING"
    );
    assert_eq!(
        REQ_SET_CS_GROUP as i128,
        g::opcodes::REQ_SET_CS_GROUP as i128,
        "REQ_SET_CS_GROUP"
    );
    assert_eq!(
        REQ_GET_CS_GROUP as i128,
        g::opcodes::REQ_GET_CS_GROUP as i128,
        "REQ_GET_CS_GROUP"
    );
    assert_eq!(
        REQ_SET_CS_MACRO as i128,
        g::opcodes::REQ_SET_CS_MACRO as i128,
        "REQ_SET_CS_MACRO"
    );
    assert_eq!(
        REQ_GET_CS_MACRO as i128,
        g::opcodes::REQ_GET_CS_MACRO as i128,
        "REQ_GET_CS_MACRO"
    );
    assert_eq!(
        REQ_SET_CS_MACRO_STEP as i128,
        g::opcodes::REQ_SET_CS_MACRO_STEP as i128,
        "REQ_SET_CS_MACRO_STEP"
    );
    assert_eq!(
        REQ_CS_MACRO_FIRE as i128,
        g::opcodes::REQ_CS_MACRO_FIRE as i128,
        "REQ_CS_MACRO_FIRE"
    );
    assert_eq!(
        REQ_GET_CS_EXT_STATUS as i128,
        g::opcodes::REQ_GET_CS_EXT_STATUS as i128,
        "REQ_GET_CS_EXT_STATUS"
    );
    assert_eq!(
        REQ_SET_CS_DISPLAY_CFG as i128,
        g::opcodes::REQ_SET_CS_DISPLAY_CFG as i128,
        "REQ_SET_CS_DISPLAY_CFG"
    );
    assert_eq!(
        REQ_GET_CS_DISPLAY_CFG as i128,
        g::opcodes::REQ_GET_CS_DISPLAY_CFG as i128,
        "REQ_GET_CS_DISPLAY_CFG"
    );
    assert_eq!(
        REQ_SET_CS_DISPLAY_PAGE as i128,
        g::opcodes::REQ_SET_CS_DISPLAY_PAGE as i128,
        "REQ_SET_CS_DISPLAY_PAGE"
    );
    assert_eq!(
        REQ_GET_CS_DISPLAY_PAGE as i128,
        g::opcodes::REQ_GET_CS_DISPLAY_PAGE as i128,
        "REQ_GET_CS_DISPLAY_PAGE"
    );
    assert_eq!(
        REQ_GET_CS_DISPLAY_STATUS as i128,
        g::opcodes::REQ_GET_CS_DISPLAY_STATUS as i128,
        "REQ_GET_CS_DISPLAY_STATUS"
    );
    assert_eq!(
        REQ_SET_PSYBASS as i128,
        g::opcodes::REQ_SET_PSYBASS as i128,
        "REQ_SET_PSYBASS"
    );
    assert_eq!(
        REQ_GET_PSYBASS as i128,
        g::opcodes::REQ_GET_PSYBASS as i128,
        "REQ_GET_PSYBASS"
    );
    assert_eq!(
        REQ_SET_PSYBASS_CUTOFF as i128,
        g::opcodes::REQ_SET_PSYBASS_CUTOFF as i128,
        "REQ_SET_PSYBASS_CUTOFF"
    );
    assert_eq!(
        REQ_GET_PSYBASS_CUTOFF as i128,
        g::opcodes::REQ_GET_PSYBASS_CUTOFF as i128,
        "REQ_GET_PSYBASS_CUTOFF"
    );
    assert_eq!(
        REQ_SET_PSYBASS_HARMONICS as i128,
        g::opcodes::REQ_SET_PSYBASS_HARMONICS as i128,
        "REQ_SET_PSYBASS_HARMONICS"
    );
    assert_eq!(
        REQ_GET_PSYBASS_HARMONICS as i128,
        g::opcodes::REQ_GET_PSYBASS_HARMONICS as i128,
        "REQ_GET_PSYBASS_HARMONICS"
    );
    assert_eq!(
        REQ_SET_PSYBASS_DRIVE as i128,
        g::opcodes::REQ_SET_PSYBASS_DRIVE as i128,
        "REQ_SET_PSYBASS_DRIVE"
    );
    assert_eq!(
        REQ_GET_PSYBASS_DRIVE as i128,
        g::opcodes::REQ_GET_PSYBASS_DRIVE as i128,
        "REQ_GET_PSYBASS_DRIVE"
    );
    assert_eq!(
        REQ_SET_PSYBASS_CHARACTER as i128,
        g::opcodes::REQ_SET_PSYBASS_CHARACTER as i128,
        "REQ_SET_PSYBASS_CHARACTER"
    );
    assert_eq!(
        REQ_GET_PSYBASS_CHARACTER as i128,
        g::opcodes::REQ_GET_PSYBASS_CHARACTER as i128,
        "REQ_GET_PSYBASS_CHARACTER"
    );
    assert_eq!(
        REQ_SET_PSYBASS_ORIGINAL as i128,
        g::opcodes::REQ_SET_PSYBASS_ORIGINAL as i128,
        "REQ_SET_PSYBASS_ORIGINAL"
    );
    assert_eq!(
        REQ_GET_PSYBASS_ORIGINAL as i128,
        g::opcodes::REQ_GET_PSYBASS_ORIGINAL as i128,
        "REQ_GET_PSYBASS_ORIGINAL"
    );
    assert_eq!(
        REQ_SET_PSYBASS_MASK as i128,
        g::opcodes::REQ_SET_PSYBASS_MASK as i128,
        "REQ_SET_PSYBASS_MASK"
    );
    assert_eq!(
        REQ_GET_PSYBASS_MASK as i128,
        g::opcodes::REQ_GET_PSYBASS_MASK as i128,
        "REQ_GET_PSYBASS_MASK"
    );
    assert_eq!(
        REQ_SET_TUBE_PARAM as i128,
        g::opcodes::REQ_SET_TUBE_PARAM as i128,
        "REQ_SET_TUBE_PARAM"
    );
    assert_eq!(
        REQ_GET_TUBE_PARAM as i128,
        g::opcodes::REQ_GET_TUBE_PARAM as i128,
        "REQ_GET_TUBE_PARAM"
    );
    assert_eq!(
        TUBE_PARAM_ENABLED as i128,
        g::tube::TUBE_PARAM_ENABLED as i128,
        "TUBE_PARAM_ENABLED"
    );
    assert_eq!(
        TUBE_PARAM_RECTIFIER as i128,
        g::tube::TUBE_PARAM_RECTIFIER as i128,
        "TUBE_PARAM_RECTIFIER"
    );
    assert_eq!(
        REQ_SET_EQ_PARAM as i128,
        g::opcodes::REQ_SET_EQ_PARAM as i128,
        "REQ_SET_EQ_PARAM"
    );
    assert_eq!(
        REQ_GET_EQ_PARAM as i128,
        g::opcodes::REQ_GET_EQ_PARAM as i128,
        "REQ_GET_EQ_PARAM"
    );
    assert_eq!(
        REQ_SET_PREAMP as i128,
        g::opcodes::REQ_SET_PREAMP as i128,
        "REQ_SET_PREAMP"
    );
    assert_eq!(
        REQ_GET_PREAMP as i128,
        g::opcodes::REQ_GET_PREAMP as i128,
        "REQ_GET_PREAMP"
    );
    assert_eq!(
        REQ_SET_BYPASS as i128,
        g::opcodes::REQ_SET_BYPASS as i128,
        "REQ_SET_BYPASS"
    );
    assert_eq!(
        REQ_GET_BYPASS as i128,
        g::opcodes::REQ_GET_BYPASS as i128,
        "REQ_GET_BYPASS"
    );
    assert_eq!(
        REQ_SET_DELAY as i128,
        g::opcodes::REQ_SET_DELAY as i128,
        "REQ_SET_DELAY"
    );
    assert_eq!(
        REQ_GET_DELAY as i128,
        g::opcodes::REQ_GET_DELAY as i128,
        "REQ_GET_DELAY"
    );
    assert_eq!(
        REQ_UPMIX_SET_CONFIG as i128,
        g::opcodes::REQ_UPMIX_SET_CONFIG as i128,
        "REQ_UPMIX_SET_CONFIG"
    );
    assert_eq!(
        REQ_UPMIX_GET_CONFIG as i128,
        g::opcodes::REQ_UPMIX_GET_CONFIG as i128,
        "REQ_UPMIX_GET_CONFIG"
    );
    assert_eq!(
        REQ_UPMIX_SET_PARAM as i128,
        g::opcodes::REQ_UPMIX_SET_PARAM as i128,
        "REQ_UPMIX_SET_PARAM"
    );
    assert_eq!(
        REQ_UPMIX_GET_PARAM as i128,
        g::opcodes::REQ_UPMIX_GET_PARAM as i128,
        "REQ_UPMIX_GET_PARAM"
    );
    assert_eq!(
        REQ_UPMIX_GET_STATUS as i128,
        g::opcodes::REQ_UPMIX_GET_STATUS as i128,
        "REQ_UPMIX_GET_STATUS"
    );
    assert_eq!(
        REQ_GET_STATUS as i128,
        g::opcodes::REQ_GET_STATUS as i128,
        "REQ_GET_STATUS"
    );
    assert_eq!(
        REQ_SAVE_PARAMS as i128,
        g::opcodes::REQ_SAVE_PARAMS as i128,
        "REQ_SAVE_PARAMS"
    );
    assert_eq!(
        REQ_SAVE_OUTPUT_CONFIG as i128,
        g::opcodes::REQ_SAVE_OUTPUT_CONFIG as i128,
        "REQ_SAVE_OUTPUT_CONFIG"
    );
    assert_eq!(
        REQ_FACTORY_RESET as i128,
        g::opcodes::REQ_FACTORY_RESET as i128,
        "REQ_FACTORY_RESET"
    );
    assert_eq!(
        REQ_SET_LOUDNESS as i128,
        g::opcodes::REQ_SET_LOUDNESS as i128,
        "REQ_SET_LOUDNESS"
    );
    assert_eq!(
        REQ_GET_LOUDNESS as i128,
        g::opcodes::REQ_GET_LOUDNESS as i128,
        "REQ_GET_LOUDNESS"
    );
    assert_eq!(
        REQ_SET_LOUDNESS_REF as i128,
        g::opcodes::REQ_SET_LOUDNESS_REF as i128,
        "REQ_SET_LOUDNESS_REF"
    );
    assert_eq!(
        REQ_GET_LOUDNESS_REF as i128,
        g::opcodes::REQ_GET_LOUDNESS_REF as i128,
        "REQ_GET_LOUDNESS_REF"
    );
    assert_eq!(
        REQ_SET_LOUDNESS_INTENSITY as i128,
        g::opcodes::REQ_SET_LOUDNESS_INTENSITY as i128,
        "REQ_SET_LOUDNESS_INTENSITY"
    );
    assert_eq!(
        REQ_GET_LOUDNESS_INTENSITY as i128,
        g::opcodes::REQ_GET_LOUDNESS_INTENSITY as i128,
        "REQ_GET_LOUDNESS_INTENSITY"
    );
    assert_eq!(
        REQ_SET_LOUDNESS_MASK as i128,
        g::opcodes::REQ_SET_LOUDNESS_MASK as i128,
        "REQ_SET_LOUDNESS_MASK"
    );
    assert_eq!(
        REQ_GET_LOUDNESS_MASK as i128,
        g::opcodes::REQ_GET_LOUDNESS_MASK as i128,
        "REQ_GET_LOUDNESS_MASK"
    );
    assert_eq!(
        REQ_SET_CROSSFEED as i128,
        g::opcodes::REQ_SET_CROSSFEED as i128,
        "REQ_SET_CROSSFEED"
    );
    assert_eq!(
        REQ_GET_CROSSFEED as i128,
        g::opcodes::REQ_GET_CROSSFEED as i128,
        "REQ_GET_CROSSFEED"
    );
    assert_eq!(
        REQ_SET_CROSSFEED_PRESET as i128,
        g::opcodes::REQ_SET_CROSSFEED_PRESET as i128,
        "REQ_SET_CROSSFEED_PRESET"
    );
    assert_eq!(
        REQ_GET_CROSSFEED_PRESET as i128,
        g::opcodes::REQ_GET_CROSSFEED_PRESET as i128,
        "REQ_GET_CROSSFEED_PRESET"
    );
    assert_eq!(
        REQ_SET_CROSSFEED_FREQ as i128,
        g::opcodes::REQ_SET_CROSSFEED_FREQ as i128,
        "REQ_SET_CROSSFEED_FREQ"
    );
    assert_eq!(
        REQ_GET_CROSSFEED_FREQ as i128,
        g::opcodes::REQ_GET_CROSSFEED_FREQ as i128,
        "REQ_GET_CROSSFEED_FREQ"
    );
    assert_eq!(
        REQ_SET_CROSSFEED_FEED as i128,
        g::opcodes::REQ_SET_CROSSFEED_FEED as i128,
        "REQ_SET_CROSSFEED_FEED"
    );
    assert_eq!(
        REQ_GET_CROSSFEED_FEED as i128,
        g::opcodes::REQ_GET_CROSSFEED_FEED as i128,
        "REQ_GET_CROSSFEED_FEED"
    );
    assert_eq!(
        REQ_SET_CROSSFEED_ITD as i128,
        g::opcodes::REQ_SET_CROSSFEED_ITD as i128,
        "REQ_SET_CROSSFEED_ITD"
    );
    assert_eq!(
        REQ_GET_CROSSFEED_ITD as i128,
        g::opcodes::REQ_GET_CROSSFEED_ITD as i128,
        "REQ_GET_CROSSFEED_ITD"
    );
    assert_eq!(
        REQ_SET_CROSSFEED_OUTPUTS as i128,
        g::opcodes::REQ_SET_CROSSFEED_OUTPUTS as i128,
        "REQ_SET_CROSSFEED_OUTPUTS"
    );
    assert_eq!(
        REQ_GET_CROSSFEED_OUTPUTS as i128,
        g::opcodes::REQ_GET_CROSSFEED_OUTPUTS as i128,
        "REQ_GET_CROSSFEED_OUTPUTS"
    );
    assert_eq!(
        REQ_SET_ADAT_INPUT_ENABLE as i128,
        g::opcodes::REQ_SET_ADAT_INPUT_ENABLE as i128,
        "REQ_SET_ADAT_INPUT_ENABLE"
    );
    assert_eq!(
        REQ_GET_ADAT_INPUT_ENABLE as i128,
        g::opcodes::REQ_GET_ADAT_INPUT_ENABLE as i128,
        "REQ_GET_ADAT_INPUT_ENABLE"
    );
    assert_eq!(
        REQ_SET_ADAT_INPUT_PIN as i128,
        g::opcodes::REQ_SET_ADAT_INPUT_PIN as i128,
        "REQ_SET_ADAT_INPUT_PIN"
    );
    assert_eq!(
        REQ_GET_ADAT_INPUT_PIN as i128,
        g::opcodes::REQ_GET_ADAT_INPUT_PIN as i128,
        "REQ_GET_ADAT_INPUT_PIN"
    );
    assert_eq!(
        REQ_SET_ADAT_INPUT_CLOCK_MODE as i128,
        g::opcodes::REQ_SET_ADAT_INPUT_CLOCK_MODE as i128,
        "REQ_SET_ADAT_INPUT_CLOCK_MODE"
    );
    assert_eq!(
        REQ_GET_ADAT_INPUT_CLOCK_MODE as i128,
        g::opcodes::REQ_GET_ADAT_INPUT_CLOCK_MODE as i128,
        "REQ_GET_ADAT_INPUT_CLOCK_MODE"
    );
    assert_eq!(
        REQ_GET_ADAT_INPUT_STATUS as i128,
        g::opcodes::REQ_GET_ADAT_INPUT_STATUS as i128,
        "REQ_GET_ADAT_INPUT_STATUS"
    );
    assert_eq!(
        REQ_SET_MATRIX_ROUTE as i128,
        g::opcodes::REQ_SET_MATRIX_ROUTE as i128,
        "REQ_SET_MATRIX_ROUTE"
    );
    assert_eq!(
        REQ_GET_MATRIX_ROUTE as i128,
        g::opcodes::REQ_GET_MATRIX_ROUTE as i128,
        "REQ_GET_MATRIX_ROUTE"
    );
    assert_eq!(
        REQ_SET_OUTPUT_ENABLE as i128,
        g::opcodes::REQ_SET_OUTPUT_ENABLE as i128,
        "REQ_SET_OUTPUT_ENABLE"
    );
    assert_eq!(
        REQ_GET_OUTPUT_ENABLE as i128,
        g::opcodes::REQ_GET_OUTPUT_ENABLE as i128,
        "REQ_GET_OUTPUT_ENABLE"
    );
    assert_eq!(
        REQ_SET_OUTPUT_GAIN as i128,
        g::opcodes::REQ_SET_OUTPUT_GAIN as i128,
        "REQ_SET_OUTPUT_GAIN"
    );
    assert_eq!(
        REQ_GET_OUTPUT_GAIN as i128,
        g::opcodes::REQ_GET_OUTPUT_GAIN as i128,
        "REQ_GET_OUTPUT_GAIN"
    );
    assert_eq!(
        REQ_SET_OUTPUT_MUTE as i128,
        g::opcodes::REQ_SET_OUTPUT_MUTE as i128,
        "REQ_SET_OUTPUT_MUTE"
    );
    assert_eq!(
        REQ_GET_OUTPUT_MUTE as i128,
        g::opcodes::REQ_GET_OUTPUT_MUTE as i128,
        "REQ_GET_OUTPUT_MUTE"
    );
    assert_eq!(
        REQ_SET_OUTPUT_DELAY as i128,
        g::opcodes::REQ_SET_OUTPUT_DELAY as i128,
        "REQ_SET_OUTPUT_DELAY"
    );
    assert_eq!(
        REQ_GET_OUTPUT_DELAY as i128,
        g::opcodes::REQ_GET_OUTPUT_DELAY as i128,
        "REQ_GET_OUTPUT_DELAY"
    );
    assert_eq!(
        REQ_GET_CORE1_MODE as i128,
        g::opcodes::REQ_GET_CORE1_MODE as i128,
        "REQ_GET_CORE1_MODE"
    );
    assert_eq!(
        REQ_GET_CORE1_CONFLICT as i128,
        g::opcodes::REQ_GET_CORE1_CONFLICT as i128,
        "REQ_GET_CORE1_CONFLICT"
    );
    assert_eq!(
        REQ_SET_OUTPUT_PIN as i128,
        g::opcodes::REQ_SET_OUTPUT_PIN as i128,
        "REQ_SET_OUTPUT_PIN"
    );
    assert_eq!(
        REQ_GET_OUTPUT_PIN as i128,
        g::opcodes::REQ_GET_OUTPUT_PIN as i128,
        "REQ_GET_OUTPUT_PIN"
    );
    assert_eq!(
        REQ_GET_SERIAL as i128,
        g::opcodes::REQ_GET_SERIAL as i128,
        "REQ_GET_SERIAL"
    );
    assert_eq!(
        REQ_GET_PLATFORM as i128,
        g::opcodes::REQ_GET_PLATFORM as i128,
        "REQ_GET_PLATFORM"
    );
    assert_eq!(
        REQ_GET_BUILD_INFO as i128,
        g::opcodes::REQ_GET_BUILD_INFO as i128,
        "REQ_GET_BUILD_INFO"
    );
    assert_eq!(
        REQ_LIMITER as i128,
        g::opcodes::REQ_LIMITER as i128,
        "REQ_LIMITER"
    );
    assert_eq!(
        LIMITER_PARAM_ENABLED as i128,
        g::limiter::LIMITER_PARAM_ENABLED as i128,
        "LIMITER_PARAM_ENABLED"
    );
    assert_eq!(
        LIMITER_PARAM_THRESHOLD_DB as i128,
        g::limiter::LIMITER_PARAM_THRESHOLD_DB as i128,
        "LIMITER_PARAM_THRESHOLD_DB"
    );
    assert_eq!(
        LIMITER_PARAM_RELEASE_MS as i128,
        g::limiter::LIMITER_PARAM_RELEASE_MS as i128,
        "LIMITER_PARAM_RELEASE_MS"
    );
    assert_eq!(
        LIMITER_PARAM_LINK_GROUP as i128,
        g::limiter::LIMITER_PARAM_LINK_GROUP as i128,
        "LIMITER_PARAM_LINK_GROUP"
    );
    assert_eq!(
        LIMITER_GET_METER as i128,
        g::limiter::LIMITER_GET_METER as i128,
        "LIMITER_GET_METER"
    );
    assert_eq!(
        LIMITER_GET_STATUS as i128,
        g::limiter::LIMITER_GET_STATUS as i128,
        "LIMITER_GET_STATUS"
    );
    assert_eq!(
        LIMITER_ALL_OUTPUTS as i128,
        g::limiter::LIMITER_ALL_OUTPUTS as i128,
        "LIMITER_ALL_OUTPUTS"
    );
    assert_eq!(
        REQ_CLEAR_CLIPS as i128,
        g::opcodes::REQ_CLEAR_CLIPS as i128,
        "REQ_CLEAR_CLIPS"
    );
    assert_eq!(
        REQ_SET_CS_BINDING as i128,
        g::opcodes::REQ_SET_CS_BINDING as i128,
        "REQ_SET_CS_BINDING"
    );
    assert_eq!(
        REQ_GET_CS_BINDING as i128,
        g::opcodes::REQ_GET_CS_BINDING as i128,
        "REQ_GET_CS_BINDING"
    );
    assert_eq!(
        REQ_GET_CS_CAPS as i128,
        g::opcodes::REQ_GET_CS_CAPS as i128,
        "REQ_GET_CS_CAPS"
    );
    assert_eq!(
        REQ_GET_CS_STATUS as i128,
        g::opcodes::REQ_GET_CS_STATUS as i128,
        "REQ_GET_CS_STATUS"
    );
    assert_eq!(
        REQ_SET_CS_NAME as i128,
        g::opcodes::REQ_SET_CS_NAME as i128,
        "REQ_SET_CS_NAME"
    );
    assert_eq!(
        REQ_GET_CS_NAME as i128,
        g::opcodes::REQ_GET_CS_NAME as i128,
        "REQ_GET_CS_NAME"
    );
    assert_eq!(
        REQ_SET_CS_IR_CMD as i128,
        g::opcodes::REQ_SET_CS_IR_CMD as i128,
        "REQ_SET_CS_IR_CMD"
    );
    assert_eq!(
        REQ_GET_CS_IR_CMD as i128,
        g::opcodes::REQ_GET_CS_IR_CMD as i128,
        "REQ_GET_CS_IR_CMD"
    );
    assert_eq!(
        REQ_CS_IR_LEARN as i128,
        g::opcodes::REQ_CS_IR_LEARN as i128,
        "REQ_CS_IR_LEARN"
    );
    assert_eq!(
        REQ_CS_SAVE as i128,
        g::opcodes::REQ_CS_SAVE as i128,
        "REQ_CS_SAVE"
    );
    assert_eq!(
        REQ_CS_REVERT as i128,
        g::opcodes::REQ_CS_REVERT as i128,
        "REQ_CS_REVERT"
    );
    assert_eq!(
        REQ_SET_I2S_CLOCK_MODE as i128,
        g::opcodes::REQ_SET_I2S_CLOCK_MODE as i128,
        "REQ_SET_I2S_CLOCK_MODE"
    );
    assert_eq!(
        REQ_GET_I2S_CLOCK_MODE as i128,
        g::opcodes::REQ_GET_I2S_CLOCK_MODE as i128,
        "REQ_GET_I2S_CLOCK_MODE"
    );
    assert_eq!(
        REQ_GET_I2S_SLAVE_STATUS as i128,
        g::opcodes::REQ_GET_I2S_SLAVE_STATUS as i128,
        "REQ_GET_I2S_SLAVE_STATUS"
    );
    assert_eq!(
        REQ_SET_I2S_CLOCK_PIN_MODE as i128,
        g::opcodes::REQ_SET_I2S_CLOCK_PIN_MODE as i128,
        "REQ_SET_I2S_CLOCK_PIN_MODE"
    );
    assert_eq!(
        REQ_GET_I2S_CLOCK_PIN_MODE as i128,
        g::opcodes::REQ_GET_I2S_CLOCK_PIN_MODE as i128,
        "REQ_GET_I2S_CLOCK_PIN_MODE"
    );
    assert_eq!(
        REQ_PRESET_SAVE as i128,
        g::opcodes::REQ_PRESET_SAVE as i128,
        "REQ_PRESET_SAVE"
    );
    assert_eq!(
        REQ_PRESET_LOAD as i128,
        g::opcodes::REQ_PRESET_LOAD as i128,
        "REQ_PRESET_LOAD"
    );
    assert_eq!(
        REQ_PRESET_DELETE as i128,
        g::opcodes::REQ_PRESET_DELETE as i128,
        "REQ_PRESET_DELETE"
    );
    assert_eq!(
        REQ_PRESET_GET_NAME as i128,
        g::opcodes::REQ_PRESET_GET_NAME as i128,
        "REQ_PRESET_GET_NAME"
    );
    assert_eq!(
        REQ_PRESET_SET_NAME as i128,
        g::opcodes::REQ_PRESET_SET_NAME as i128,
        "REQ_PRESET_SET_NAME"
    );
    assert_eq!(
        REQ_PRESET_GET_DIR as i128,
        g::opcodes::REQ_PRESET_GET_DIR as i128,
        "REQ_PRESET_GET_DIR"
    );
    assert_eq!(
        REQ_PRESET_SET_STARTUP as i128,
        g::opcodes::REQ_PRESET_SET_STARTUP as i128,
        "REQ_PRESET_SET_STARTUP"
    );
    assert_eq!(
        REQ_PRESET_GET_STARTUP as i128,
        g::opcodes::REQ_PRESET_GET_STARTUP as i128,
        "REQ_PRESET_GET_STARTUP"
    );
    assert_eq!(
        REQ_SET_OUTPUT_CONFIG_MODE as i128,
        g::opcodes::REQ_SET_OUTPUT_CONFIG_MODE as i128,
        "REQ_SET_OUTPUT_CONFIG_MODE"
    );
    assert_eq!(
        REQ_GET_OUTPUT_CONFIG_MODE as i128,
        g::opcodes::REQ_GET_OUTPUT_CONFIG_MODE as i128,
        "REQ_GET_OUTPUT_CONFIG_MODE"
    );
    assert_eq!(
        REQ_PRESET_GET_ACTIVE as i128,
        g::opcodes::REQ_PRESET_GET_ACTIVE as i128,
        "REQ_PRESET_GET_ACTIVE"
    );
    assert_eq!(
        REQ_SET_CHANNEL_NAME as i128,
        g::opcodes::REQ_SET_CHANNEL_NAME as i128,
        "REQ_SET_CHANNEL_NAME"
    );
    assert_eq!(
        REQ_GET_CHANNEL_NAME as i128,
        g::opcodes::REQ_GET_CHANNEL_NAME as i128,
        "REQ_GET_CHANNEL_NAME"
    );
    assert_eq!(
        REQ_GET_ALL_PARAMS as i128,
        g::opcodes::REQ_GET_ALL_PARAMS as i128,
        "REQ_GET_ALL_PARAMS"
    );
    assert_eq!(
        REQ_SET_ALL_PARAMS as i128,
        g::opcodes::REQ_SET_ALL_PARAMS as i128,
        "REQ_SET_ALL_PARAMS"
    );
    assert_eq!(
        REQ_GET_ALL_PARAMS_CHUNK as i128,
        g::opcodes::REQ_GET_ALL_PARAMS_CHUNK as i128,
        "REQ_GET_ALL_PARAMS_CHUNK"
    );
    assert_eq!(
        REQ_SET_ALL_PARAMS_CHUNK as i128,
        g::opcodes::REQ_SET_ALL_PARAMS_CHUNK as i128,
        "REQ_SET_ALL_PARAMS_CHUNK"
    );
    assert_eq!(
        REQ_SIGGEN_SET_CONFIG as i128,
        g::opcodes::REQ_SIGGEN_SET_CONFIG as i128,
        "REQ_SIGGEN_SET_CONFIG"
    );
    assert_eq!(
        REQ_SIGGEN_GET_CONFIG as i128,
        g::opcodes::REQ_SIGGEN_GET_CONFIG as i128,
        "REQ_SIGGEN_GET_CONFIG"
    );
    assert_eq!(
        REQ_SIGGEN_CONTROL as i128,
        g::opcodes::REQ_SIGGEN_CONTROL as i128,
        "REQ_SIGGEN_CONTROL"
    );
    assert_eq!(
        REQ_SIGGEN_GET_STATUS as i128,
        g::opcodes::REQ_SIGGEN_GET_STATUS as i128,
        "REQ_SIGGEN_GET_STATUS"
    );
    assert_eq!(
        REQ_SIGGEN_GET_CAPS as i128,
        g::opcodes::REQ_SIGGEN_GET_CAPS as i128,
        "REQ_SIGGEN_GET_CAPS"
    );
    assert_eq!(
        REQ_GET_BUFFER_STATS as i128,
        g::opcodes::REQ_GET_BUFFER_STATS as i128,
        "REQ_GET_BUFFER_STATS"
    );
    assert_eq!(
        REQ_RESET_BUFFER_STATS as i128,
        g::opcodes::REQ_RESET_BUFFER_STATS as i128,
        "REQ_RESET_BUFFER_STATS"
    );
    assert_eq!(
        REQ_GET_USB_ERROR_STATS as i128,
        g::opcodes::REQ_GET_USB_ERROR_STATS as i128,
        "REQ_GET_USB_ERROR_STATS"
    );
    assert_eq!(
        REQ_RESET_USB_ERROR_STATS as i128,
        g::opcodes::REQ_RESET_USB_ERROR_STATS as i128,
        "REQ_RESET_USB_ERROR_STATS"
    );
    assert_eq!(
        REQ_SET_LEVELLER_ENABLE as i128,
        g::opcodes::REQ_SET_LEVELLER_ENABLE as i128,
        "REQ_SET_LEVELLER_ENABLE"
    );
    assert_eq!(
        REQ_GET_LEVELLER_ENABLE as i128,
        g::opcodes::REQ_GET_LEVELLER_ENABLE as i128,
        "REQ_GET_LEVELLER_ENABLE"
    );
    assert_eq!(
        REQ_SET_LEVELLER_AMOUNT as i128,
        g::opcodes::REQ_SET_LEVELLER_AMOUNT as i128,
        "REQ_SET_LEVELLER_AMOUNT"
    );
    assert_eq!(
        REQ_GET_LEVELLER_AMOUNT as i128,
        g::opcodes::REQ_GET_LEVELLER_AMOUNT as i128,
        "REQ_GET_LEVELLER_AMOUNT"
    );
    assert_eq!(
        REQ_SET_LEVELLER_SPEED as i128,
        g::opcodes::REQ_SET_LEVELLER_SPEED as i128,
        "REQ_SET_LEVELLER_SPEED"
    );
    assert_eq!(
        REQ_GET_LEVELLER_SPEED as i128,
        g::opcodes::REQ_GET_LEVELLER_SPEED as i128,
        "REQ_GET_LEVELLER_SPEED"
    );
    assert_eq!(
        REQ_SET_LEVELLER_MAX_GAIN as i128,
        g::opcodes::REQ_SET_LEVELLER_MAX_GAIN as i128,
        "REQ_SET_LEVELLER_MAX_GAIN"
    );
    assert_eq!(
        REQ_GET_LEVELLER_MAX_GAIN as i128,
        g::opcodes::REQ_GET_LEVELLER_MAX_GAIN as i128,
        "REQ_GET_LEVELLER_MAX_GAIN"
    );
    assert_eq!(
        REQ_SET_LEVELLER_LOOKAHEAD as i128,
        g::opcodes::REQ_SET_LEVELLER_LOOKAHEAD as i128,
        "REQ_SET_LEVELLER_LOOKAHEAD"
    );
    assert_eq!(
        REQ_GET_LEVELLER_LOOKAHEAD as i128,
        g::opcodes::REQ_GET_LEVELLER_LOOKAHEAD as i128,
        "REQ_GET_LEVELLER_LOOKAHEAD"
    );
    assert_eq!(
        REQ_SET_LEVELLER_GATE as i128,
        g::opcodes::REQ_SET_LEVELLER_GATE as i128,
        "REQ_SET_LEVELLER_GATE"
    );
    assert_eq!(
        REQ_GET_LEVELLER_GATE as i128,
        g::opcodes::REQ_GET_LEVELLER_GATE as i128,
        "REQ_GET_LEVELLER_GATE"
    );
    assert_eq!(
        REQ_SET_LEVELLER_MASKS as i128,
        g::opcodes::REQ_SET_LEVELLER_MASKS as i128,
        "REQ_SET_LEVELLER_MASKS"
    );
    assert_eq!(
        REQ_GET_LEVELLER_MASKS as i128,
        g::opcodes::REQ_GET_LEVELLER_MASKS as i128,
        "REQ_GET_LEVELLER_MASKS"
    );
    assert_eq!(
        REQ_SET_OUTPUT_TYPE as i128,
        g::opcodes::REQ_SET_OUTPUT_TYPE as i128,
        "REQ_SET_OUTPUT_TYPE"
    );
    assert_eq!(
        REQ_GET_OUTPUT_TYPE as i128,
        g::opcodes::REQ_GET_OUTPUT_TYPE as i128,
        "REQ_GET_OUTPUT_TYPE"
    );
    assert_eq!(
        REQ_SET_I2S_BCK_PIN as i128,
        g::opcodes::REQ_SET_I2S_BCK_PIN as i128,
        "REQ_SET_I2S_BCK_PIN"
    );
    assert_eq!(
        REQ_GET_I2S_BCK_PIN as i128,
        g::opcodes::REQ_GET_I2S_BCK_PIN as i128,
        "REQ_GET_I2S_BCK_PIN"
    );
    assert_eq!(
        REQ_SET_MCK_ENABLE as i128,
        g::opcodes::REQ_SET_MCK_ENABLE as i128,
        "REQ_SET_MCK_ENABLE"
    );
    assert_eq!(
        REQ_GET_MCK_ENABLE as i128,
        g::opcodes::REQ_GET_MCK_ENABLE as i128,
        "REQ_GET_MCK_ENABLE"
    );
    assert_eq!(
        REQ_SET_MCK_PIN as i128,
        g::opcodes::REQ_SET_MCK_PIN as i128,
        "REQ_SET_MCK_PIN"
    );
    assert_eq!(
        REQ_GET_MCK_PIN as i128,
        g::opcodes::REQ_GET_MCK_PIN as i128,
        "REQ_GET_MCK_PIN"
    );
    assert_eq!(
        REQ_SET_MCK_MULTIPLIER as i128,
        g::opcodes::REQ_SET_MCK_MULTIPLIER as i128,
        "REQ_SET_MCK_MULTIPLIER"
    );
    assert_eq!(
        REQ_GET_MCK_MULTIPLIER as i128,
        g::opcodes::REQ_GET_MCK_MULTIPLIER as i128,
        "REQ_GET_MCK_MULTIPLIER"
    );
    assert_eq!(
        REQ_SET_ADAT_ENABLE as i128,
        g::opcodes::REQ_SET_ADAT_ENABLE as i128,
        "REQ_SET_ADAT_ENABLE"
    );
    assert_eq!(
        REQ_GET_ADAT_ENABLE as i128,
        g::opcodes::REQ_GET_ADAT_ENABLE as i128,
        "REQ_GET_ADAT_ENABLE"
    );
    assert_eq!(
        REQ_SET_ADAT_PIN as i128,
        g::opcodes::REQ_SET_ADAT_PIN as i128,
        "REQ_SET_ADAT_PIN"
    );
    assert_eq!(
        REQ_GET_ADAT_PIN as i128,
        g::opcodes::REQ_GET_ADAT_PIN as i128,
        "REQ_GET_ADAT_PIN"
    );
    assert_eq!(
        REQ_GET_ADAT_STATUS as i128,
        g::opcodes::REQ_GET_ADAT_STATUS as i128,
        "REQ_GET_ADAT_STATUS"
    );
    assert_eq!(
        REQ_SET_PREAMP_CH as i128,
        g::opcodes::REQ_SET_PREAMP_CH as i128,
        "REQ_SET_PREAMP_CH"
    );
    assert_eq!(
        REQ_GET_PREAMP_CH as i128,
        g::opcodes::REQ_GET_PREAMP_CH as i128,
        "REQ_GET_PREAMP_CH"
    );
    assert_eq!(
        REQ_SET_MASTER_VOLUME as i128,
        g::opcodes::REQ_SET_MASTER_VOLUME as i128,
        "REQ_SET_MASTER_VOLUME"
    );
    assert_eq!(
        REQ_GET_MASTER_VOLUME as i128,
        g::opcodes::REQ_GET_MASTER_VOLUME as i128,
        "REQ_GET_MASTER_VOLUME"
    );
    assert_eq!(
        REQ_SET_MASTER_VOLUME_MODE as i128,
        g::opcodes::REQ_SET_MASTER_VOLUME_MODE as i128,
        "REQ_SET_MASTER_VOLUME_MODE"
    );
    assert_eq!(
        REQ_GET_MASTER_VOLUME_MODE as i128,
        g::opcodes::REQ_GET_MASTER_VOLUME_MODE as i128,
        "REQ_GET_MASTER_VOLUME_MODE"
    );
    assert_eq!(
        REQ_SAVE_MASTER_VOLUME as i128,
        g::opcodes::REQ_SAVE_MASTER_VOLUME as i128,
        "REQ_SAVE_MASTER_VOLUME"
    );
    assert_eq!(
        REQ_GET_SAVED_MASTER_VOLUME as i128,
        g::opcodes::REQ_GET_SAVED_MASTER_VOLUME as i128,
        "REQ_GET_SAVED_MASTER_VOLUME"
    );
    assert_eq!(
        REQ_SET_BAND_BYPASS as i128,
        g::opcodes::REQ_SET_BAND_BYPASS as i128,
        "REQ_SET_BAND_BYPASS"
    );
    assert_eq!(
        REQ_GET_BAND_BYPASS as i128,
        g::opcodes::REQ_GET_BAND_BYPASS as i128,
        "REQ_GET_BAND_BYPASS"
    );
    assert_eq!(
        REQ_SET_USER_VOLUME as i128,
        g::opcodes::REQ_SET_USER_VOLUME as i128,
        "REQ_SET_USER_VOLUME"
    );
    assert_eq!(
        REQ_GET_USER_VOLUME as i128,
        g::opcodes::REQ_GET_USER_VOLUME as i128,
        "REQ_GET_USER_VOLUME"
    );
    assert_eq!(
        REQ_SET_USER_MUTE as i128,
        g::opcodes::REQ_SET_USER_MUTE as i128,
        "REQ_SET_USER_MUTE"
    );
    assert_eq!(
        REQ_GET_USER_MUTE as i128,
        g::opcodes::REQ_GET_USER_MUTE as i128,
        "REQ_GET_USER_MUTE"
    );
    assert_eq!(
        REQ_SET_INPUT_SOURCE as i128,
        g::opcodes::REQ_SET_INPUT_SOURCE as i128,
        "REQ_SET_INPUT_SOURCE"
    );
    assert_eq!(
        REQ_GET_INPUT_SOURCE as i128,
        g::opcodes::REQ_GET_INPUT_SOURCE as i128,
        "REQ_GET_INPUT_SOURCE"
    );
    assert_eq!(
        REQ_GET_SPDIF_RX_STATUS as i128,
        g::opcodes::REQ_GET_SPDIF_RX_STATUS as i128,
        "REQ_GET_SPDIF_RX_STATUS"
    );
    assert_eq!(
        REQ_GET_SPDIF_RX_CH_STATUS as i128,
        g::opcodes::REQ_GET_SPDIF_RX_CH_STATUS as i128,
        "REQ_GET_SPDIF_RX_CH_STATUS"
    );
    assert_eq!(
        REQ_SET_SPDIF_RX_PIN as i128,
        g::opcodes::REQ_SET_SPDIF_RX_PIN as i128,
        "REQ_SET_SPDIF_RX_PIN"
    );
    assert_eq!(
        REQ_GET_SPDIF_RX_PIN as i128,
        g::opcodes::REQ_GET_SPDIF_RX_PIN as i128,
        "REQ_GET_SPDIF_RX_PIN"
    );
    assert_eq!(
        REQ_SET_SPDIF_INPUT_ENABLE as i128,
        g::opcodes::REQ_SET_SPDIF_INPUT_ENABLE as i128,
        "REQ_SET_SPDIF_INPUT_ENABLE"
    );
    assert_eq!(
        REQ_GET_SPDIF_INPUT_CONFIG as i128,
        g::opcodes::REQ_GET_SPDIF_INPUT_CONFIG as i128,
        "REQ_GET_SPDIF_INPUT_CONFIG"
    );
    assert_eq!(
        REQ_SET_INPUT_RATE as i128,
        g::opcodes::REQ_SET_INPUT_RATE as i128,
        "REQ_SET_INPUT_RATE"
    );
    assert_eq!(
        REQ_GET_INPUT_RATE as i128,
        g::opcodes::REQ_GET_INPUT_RATE as i128,
        "REQ_GET_INPUT_RATE"
    );
    assert_eq!(
        REQ_SET_I2S_RX_PIN as i128,
        g::opcodes::REQ_SET_I2S_RX_PIN as i128,
        "REQ_SET_I2S_RX_PIN"
    );
    assert_eq!(
        REQ_GET_I2S_RX_PIN as i128,
        g::opcodes::REQ_GET_I2S_RX_PIN as i128,
        "REQ_GET_I2S_RX_PIN"
    );
    assert_eq!(
        REQ_SET_I2S_INPUT_CHANNELS as i128,
        g::opcodes::REQ_SET_I2S_INPUT_CHANNELS as i128,
        "REQ_SET_I2S_INPUT_CHANNELS"
    );
    assert_eq!(
        REQ_GET_I2S_INPUT_CHANNELS as i128,
        g::opcodes::REQ_GET_I2S_INPUT_CHANNELS as i128,
        "REQ_GET_I2S_INPUT_CHANNELS"
    );
    assert_eq!(
        REQ_SET_LG_SOUND_SYNC_ENABLE as i128,
        g::opcodes::REQ_SET_LG_SOUND_SYNC_ENABLE as i128,
        "REQ_SET_LG_SOUND_SYNC_ENABLE"
    );
    assert_eq!(
        REQ_GET_LG_SOUND_SYNC_ENABLE as i128,
        g::opcodes::REQ_GET_LG_SOUND_SYNC_ENABLE as i128,
        "REQ_GET_LG_SOUND_SYNC_ENABLE"
    );
    assert_eq!(
        REQ_GET_LG_SOUND_SYNC_STATUS as i128,
        g::opcodes::REQ_GET_LG_SOUND_SYNC_STATUS as i128,
        "REQ_GET_LG_SOUND_SYNC_STATUS"
    );
    assert_eq!(
        REQ_SET_DAC_HW_MUTE_CONFIG as i128,
        g::opcodes::REQ_SET_DAC_HW_MUTE_CONFIG as i128,
        "REQ_SET_DAC_HW_MUTE_CONFIG"
    );
    assert_eq!(
        REQ_GET_DAC_HW_MUTE_CONFIG as i128,
        g::opcodes::REQ_GET_DAC_HW_MUTE_CONFIG as i128,
        "REQ_GET_DAC_HW_MUTE_CONFIG"
    );
    assert_eq!(
        REQ_TEST_DAC_HW_MUTE as i128,
        g::opcodes::REQ_TEST_DAC_HW_MUTE as i128,
        "REQ_TEST_DAC_HW_MUTE"
    );
    assert_eq!(
        REQ_ENTER_BOOTLOADER as i128,
        g::opcodes::REQ_ENTER_BOOTLOADER as i128,
        "REQ_ENTER_BOOTLOADER"
    );
    assert_eq!(
        REQ_SET_UART_CONFIG as i128,
        g::opcodes::REQ_SET_UART_CONFIG as i128,
        "REQ_SET_UART_CONFIG"
    );
    assert_eq!(
        REQ_GET_UART_CONFIG as i128,
        g::opcodes::REQ_GET_UART_CONFIG as i128,
        "REQ_GET_UART_CONFIG"
    );
    assert_eq!(
        REQ_SET_I2C_CONFIG as i128,
        g::opcodes::REQ_SET_I2C_CONFIG as i128,
        "REQ_SET_I2C_CONFIG"
    );
    assert_eq!(
        REQ_GET_I2C_CONFIG as i128,
        g::opcodes::REQ_GET_I2C_CONFIG as i128,
        "REQ_GET_I2C_CONFIG"
    );
    assert_eq!(
        REQ_GET_CTRL_IFACE_STATUS as i128,
        g::opcodes::REQ_GET_CTRL_IFACE_STATUS as i128,
        "REQ_GET_CTRL_IFACE_STATUS"
    );
    assert_eq!(
        PIN_CONFIG_SUCCESS as i128,
        g::status::PIN_CONFIG_SUCCESS as i128,
        "PIN_CONFIG_SUCCESS"
    );
    assert_eq!(
        PIN_CONFIG_INVALID_PIN as i128,
        g::status::PIN_CONFIG_INVALID_PIN as i128,
        "PIN_CONFIG_INVALID_PIN"
    );
    assert_eq!(
        PIN_CONFIG_PIN_IN_USE as i128,
        g::status::PIN_CONFIG_PIN_IN_USE as i128,
        "PIN_CONFIG_PIN_IN_USE"
    );
    assert_eq!(
        PIN_CONFIG_INVALID_OUTPUT as i128,
        g::status::PIN_CONFIG_INVALID_OUTPUT as i128,
        "PIN_CONFIG_INVALID_OUTPUT"
    );
    assert_eq!(
        PIN_CONFIG_OUTPUT_ACTIVE as i128,
        g::status::PIN_CONFIG_OUTPUT_ACTIVE as i128,
        "PIN_CONFIG_OUTPUT_ACTIVE"
    );
    assert_eq!(
        PIN_CONFIG_INVALID_PARAM as i128,
        g::status::PIN_CONFIG_INVALID_PARAM as i128,
        "PIN_CONFIG_INVALID_PARAM"
    );
    assert_eq!(PRESET_OK as i128, g::status::PRESET_OK as i128, "PRESET_OK");
    assert_eq!(
        PRESET_ERR_INVALID_SLOT as i128,
        g::status::PRESET_ERR_INVALID_SLOT as i128,
        "PRESET_ERR_INVALID_SLOT"
    );
    assert_eq!(
        PRESET_ERR_SLOT_EMPTY as i128,
        g::status::PRESET_ERR_SLOT_EMPTY as i128,
        "PRESET_ERR_SLOT_EMPTY"
    );
    assert_eq!(
        PRESET_ERR_CRC as i128,
        g::status::PRESET_ERR_CRC as i128,
        "PRESET_ERR_CRC"
    );
    assert_eq!(
        PRESET_ERR_FLASH_WRITE as i128,
        g::status::PRESET_ERR_FLASH_WRITE as i128,
        "PRESET_ERR_FLASH_WRITE"
    );
    assert_eq!(
        WIRE_FORMAT_VERSION as i128,
        g::wire::WIRE_FORMAT_VERSION as i128,
        "WIRE_FORMAT_VERSION"
    );
    assert_eq!(
        RTA_MAX_BANDS as i128,
        g::rta::RTA_MAX_BANDS as i128,
        "RTA_MAX_BANDS"
    );
    assert_eq!(
        RTA_TAP_INPUT as i128,
        g::rta::RTA_TAP_INPUT as i128,
        "RTA_TAP_INPUT"
    );
    assert_eq!(
        RTA_TAP_OUTPUT as i128,
        g::rta::RTA_TAP_OUTPUT as i128,
        "RTA_TAP_OUTPUT"
    );
    assert_eq!(
        SIGGEN_FLAG_RAW as i128,
        g::siggen::SIGGEN_FLAG_RAW as i128,
        "SIGGEN_FLAG_RAW"
    );
    assert_eq!(
        SIGGEN_FLAG_DECORR as i128,
        g::siggen::SIGGEN_FLAG_DECORR as i128,
        "SIGGEN_FLAG_DECORR"
    );
    assert_eq!(
        SIGGEN_FLAG_WALK as i128,
        g::siggen::SIGGEN_FLAG_WALK as i128,
        "SIGGEN_FLAG_WALK"
    );
    assert_eq!(
        SIGGEN_CTL_STOP as i128,
        g::siggen::SIGGEN_CTL_STOP as i128,
        "SIGGEN_CTL_STOP"
    );
    assert_eq!(
        SIGGEN_CTL_START as i128,
        g::siggen::SIGGEN_CTL_START as i128,
        "SIGGEN_CTL_START"
    );
    assert_eq!(
        SIGGEN_CTL_STOP_NOW as i128,
        g::siggen::SIGGEN_CTL_STOP_NOW as i128,
        "SIGGEN_CTL_STOP_NOW"
    );
    assert_eq!(
        CS_MAX_BINDINGS as i128,
        g::cs::CS_MAX_BINDINGS as i128,
        "CS_MAX_BINDINGS"
    );
    assert_eq!(
        CS_MAX_GROUPS as i128,
        g::cs::CS_MAX_GROUPS as i128,
        "CS_MAX_GROUPS"
    );
    assert_eq!(
        CS_MAX_MACROS as i128,
        g::cs::CS_MAX_MACROS as i128,
        "CS_MAX_MACROS"
    );
    assert_eq!(
        MAX_XOVER_BANDS as i128,
        g::filter::MAX_XOVER_BANDS as i128,
        "MAX_XOVER_BANDS"
    );
    assert_eq!(
        XOVER_BAND_BASE as i128,
        g::filter::XOVER_BAND_BASE as i128,
        "XOVER_BAND_BASE"
    );
}

#[test]
fn wp_q1_constants_match_dspi_core() {
    use dspi_core::consts as c;
    assert_eq!(COMPAT_UNKNOWN, c::COMPAT_UNKNOWN);
    assert_eq!(COMPAT_OK, c::COMPAT_OK);
    assert_eq!(COMPAT_FIRMWARE_TOO_OLD, c::COMPAT_FIRMWARE_TOO_OLD);
    assert_eq!(COMPAT_FIRMWARE_TOO_NEW, c::COMPAT_FIRMWARE_TOO_NEW);
}

fn filt(n: u8) -> r::FilterParams {
    r::FilterParams {
        filter_type: n,
        bypass: true,
        freq: f32::from(n) * 10.0,
        q: 0.5,
        gain: -3.0,
        qp: 0.7,
    }
}

fn sample_state() -> r::DspState {
    r::DspState {
        platform_id: 1,
        num_channels: 2,
        num_input_channels: 3,
        num_output_channels: 4,
        format_version: 5,
        fw_major: 6,
        fw_minor: 7,
        fw_patch: 8,
        fw_beta: 9,
        compat: 10,
        preamp_db: 11.5,
        bypass: true,
        input_preamp_db: (0..MAX_INPUTS).map(|i| 13.0 + i as f32).collect(),
        master_volume_db: 14.5,
        master_volume_mode: 15,
        user_volume_db: 16.5,
        user_mute: true,
        loudness_enabled: true,
        loudness_ref_spl: 19.5,
        loudness_intensity: 20.5,
        loudness_output_mask: 1021,
        crossfeed_enabled: true,
        crossfeed_preset: 23,
        crossfeed_freq: 24.5,
        crossfeed_feed: 25.5,
        crossfeed_itd: true,
        crossfeed_output_pair_mask: 27,
        crossfeed_pair_map: vec![1, 2, 3, 4],
        channel_delays: (0..MAX_CHANNELS).map(|i| 29.0 + i as f32).collect(),
        matrix_routing: vec![vec![true; MAX_OUTPUTS]; MAX_INPUTS],
        matrix_gain: vec![vec![31.25; MAX_OUTPUTS]; MAX_INPUTS],
        matrix_invert: vec![vec![true; MAX_OUTPUTS]; MAX_INPUTS],
        output_enabled: vec![true; MAX_OUTPUTS],
        output_muted: vec![true; MAX_OUTPUTS],
        output_gain_db: (0..MAX_OUTPUTS).map(|i| 35.0 + i as f32).collect(),
        output_delay_ms: (0..MAX_OUTPUTS).map(|i| 36.0 + i as f32).collect(),
        limiter_enabled: vec![true; MAX_OUTPUTS],
        limiter_link_group: (0..MAX_OUTPUTS).map(|i| (38 + i) as u8).collect(),
        limiter_threshold_db: (0..MAX_OUTPUTS).map(|i| 39.0 + i as f32).collect(),
        limiter_release_ms: (0..MAX_OUTPUTS).map(|i| 40.0 + i as f32).collect(),
        num_pin_outputs: 41,
        output_pins: (0..MAX_PHYSICAL_OUTPUTS).map(|i| (42 + i) as u8).collect(),
        output_config_mode: 43,
        filters: vec![vec![filt(44); BANDS_PER_CHANNEL]; MAX_CHANNELS],
        xover: vec![vec![filt(45); MAX_XOVER_BANDS]; MAX_CHANNELS],
        channel_names: (0..MAX_CHANNELS).map(|i| format!("name {i}")).collect(),
        leveller_enabled: true,
        leveller_speed: 48,
        leveller_lookahead: true,
        leveller_amount: 50.5,
        leveller_max_gain_db: 51.5,
        leveller_gate_db: 52.5,
        leveller_detector_mask: 53,
        leveller_apply_mask: 54,
        input_source: 55,
        lg_sound_sync_enabled: true,
        spdif_inputs_enabled: 57,
        adat_input_enabled: true,
        spdif_rx_pins: (0..4).map(|i| (59 + i) as u8).collect(),
        i2s_rx_pins: (0..4).map(|i| (60 + i) as u8).collect(),
        i2s_input_channels: 61,
        i2s_input_rate: 62,
        i2s_clock_mode: 63,
        adat_input_pin: 64,
        adat_input_clock_mode: 65,
        psybass_enabled: true,
        psybass_output_mask: 1067,
        psybass_cutoff_hz: 68.5,
        psybass_harmonics_db: 69.5,
        psybass_drive_db: 70.5,
        psybass_character_pct: 71.5,
        psybass_original_db: 72.5,
        upmix: (0..UPMIX_PARAM_COUNT).map(|i| 73.0 + i as f32).collect(),
        subharm: (0..SUBHARM_PARAM_COUNT).map(|i| 74.0 + i as f32).collect(),
        subharm_solo: true,
        tube: (0..TUBE_PARAM_COUNT).map(|i| 76.0 + i as f32).collect(),
        output_types: (0..4).map(|i| (77 + i) as u8).collect(),
        i2s_bck_pin: 78,
        i2s_bck_pin_slave: 79,
        i2s_clock_pin_mode: 80,
        mck_enabled: true,
        mck_pin: 82,
        mck_multiplier: 83,
        adat_out_enabled: true,
        adat_out_pin: 85,
        dac_mute_supported: true,
        dac_mute_enabled: true,
        dac_mute_active_low: true,
        dac_mute_pin: 89,
        dac_mute_hold_ms: 1090,
        dac_mute_release_ms: 1091,
        ctrl_iface_supported: true,
        uart: r::UartConfig {
            enabled: true,
            tx_pin: 4,
            rx_pin: 5,
            notify: true,
            baud: 57_600,
        },
        i2c: r::I2cConfig {
            enabled: true,
            sda_pin: 6,
            scl_pin: 7,
            address: 0x33,
        },
        ctrl_status: r::CtrlIfaceStatus::default(),
        core1_mode: 96,
        preset_occupied: 1097,
        preset_names: (0..MAX_PRESETS).map(|i| format!("name {i}")).collect(),
        active_preset_slot: 99,
        preset_startup_mode: 100,
        preset_default_slot: 101,
        bulk_valid: true,
        bulk_raw: (0..BULK_PARAMS_SIZE).map(|i| (103 + i) as u8).collect(),
        // -- WP CG32 -- not in the C record (its layout is the old core's)
        i2s_clock_pin_mode_present: true,
        spdif_inputs_enabled_present: true,
        adat_input_enabled_present: true,
        adat_input_clock_mode_present: true,
    }
}

#[test]
fn wp_q1_dsp_state_converts_field_for_field() {
    let rec = sample_state();
    let c = DspState::from(&rec);
    assert_eq!(c.platform_id, 1);
    assert_eq!(c.compat, 10);
    assert_eq!(c.preamp_db, 11.5);
    assert_eq!(c.input_preamp_db[7], 20.0);
    assert_eq!(c.loudness_output_mask, 1021);
    assert_eq!(c.channel_delays[16], 45.0);
    assert!(c.matrix_routing[7][8]);
    assert_eq!(c.matrix_gain[3][4], 31.25);
    assert_eq!(c.limiter_link_group[8], 46);
    assert_eq!(c.output_pins, [42, 43, 44, 45, 46]);
    assert_eq!(c.filters[16][9], FilterParams::from(&filt(44)));
    assert_eq!(c.xover[2][3].filter_type, 45);
    assert_eq!(&c.channel_names[12][..8], b"name 12\0");
    assert_eq!(&c.preset_names[9][..7], b"name 9\0");
    assert_eq!(c.upmix[13], 86.0);
    assert_eq!(c.tube[0], 76.0);
    assert_eq!(c.uart.baud, 57_600);
    assert_eq!(c.i2c.address, 0x33);
    assert_eq!(c.dac_mute_release_ms, 1091);
    assert_eq!(c.preset_default_slot, 101);
    assert!(c.bulk_valid);
    assert_eq!(c.bulk_raw[0], 103);
    assert_eq!(
        c.bulk_raw[BULK_PARAMS_SIZE - 1],
        (103 + BULK_PARAMS_SIZE - 1) as u8
    );
}

#[test]
fn wp_q1_short_and_long_vectors_never_panic() {
    let mut rec = sample_state();
    rec.input_preamp_db = vec![1.0];
    rec.matrix_gain = vec![vec![2.0; 20]; 1];
    rec.filters = vec![];
    rec.channel_names = vec!["x".into(); 40];
    rec.bulk_raw = vec![9; 10];
    let c = DspState::from(&rec);
    assert_eq!(c.input_preamp_db, [1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]);
    assert_eq!(c.matrix_gain[0], [2.0; MAX_OUTPUTS]);
    assert_eq!(c.matrix_gain[1], [0.0; MAX_OUTPUTS]);
    assert_eq!(c.filters[0][0], FilterParams::default());
    assert_eq!(c.channel_names[16][0], b'x');
    assert_eq!(c.bulk_raw[9], 9);
    assert_eq!(c.bulk_raw[10], 0);

    let snap = RtaSnapshot::from(&r::RtaSnapshot::default());
    assert_eq!(snap.frames[0], RtaChannelFrame::default());
    assert_eq!(snap.bins_db[511], RTA_FLOOR_DB);
    let caps = SiggenCaps::from(&r::SiggenCaps::default());
    assert_eq!(caps.types[14], SiggenTypeInfo::default());
    let stats = BufferStats::from(&r::BufferStats::default());
    assert_eq!(stats.spdif[3], SpdifBufferStats::default());
}

#[test]
fn wp_q1_names_are_cut_at_a_char_boundary_and_terminated() {
    // 30 ASCII bytes + a 3-byte character: 33 bytes, cut before the character.
    let long = format!("{}€", "a".repeat(30));
    let buf: [u8; CHANNEL_NAME_LEN] = name_buf(&long);
    assert_eq!(&buf[..30], "a".repeat(30).as_bytes());
    assert_eq!(buf[30..], [0, 0]);
    let exact: [u8; 4] = name_buf("abcdef");
    assert_eq!(exact, *b"abc\0");
    let none: [u8; 0] = name_buf("abc");
    assert_eq!(none, [0u8; 0]);

    let info = r::SiggenTypeInfo {
        name: "multitone-long".into(),
        ..Default::default()
    };
    assert_eq!(SiggenTypeInfo::from(&info).name, *b"multiton\0");
}

#[test]
fn wp_q1_device_info_round_trips_and_clamps() {
    let rec = r::DeviceInfo {
        serial: "E6614C311B7A".into(),
        location_id: 7,
    };
    let c = DeviceInfo::from(&rec);
    assert_eq!(c.serial_len, 12);
    assert_eq!(c.serial[12], 0);
    assert_eq!(r::DeviceInfo::from(&c), rec);

    let long = r::DeviceInfo {
        serial: "s".repeat(100),
        location_id: 0,
    };
    let c = DeviceInfo::from(&long);
    assert_eq!(c.serial_len, 63);
    assert_eq!(c.serial[63], 0);

    // A C caller's lies about the length are clamped, bad UTF-8 replaced.
    let mut bad = DeviceInfo {
        serial_len: u32::MAX,
        ..DeviceInfo::default()
    };
    bad.serial[..3].copy_from_slice(&[b'a', 0xFF, b'b']);
    assert_eq!(r::DeviceInfo::from(&bad).serial, "a\u{FFFD}b");
}

#[test]
fn wp_q1_input_records_round_trip() {
    let f = filt(11);
    assert_eq!(r::FilterParams::from(&FilterParams::from(&f)), f);
    let u = r::UartConfig {
        enabled: true,
        tx_pin: 1,
        rx_pin: 2,
        notify: false,
        baud: 9600,
    };
    assert_eq!(r::UartConfig::from(&UartConfig::from(&u)), u);
    let i = r::I2cConfig {
        enabled: false,
        sda_pin: 3,
        scl_pin: 4,
        address: 0x42,
    };
    assert_eq!(r::I2cConfig::from(&I2cConfig::from(&i)), i);
    let q = r::RtaRequest {
        tap: 1,
        mask: 0x0F,
        wants_bins: true,
        fft_order: 12,
        avg_ms: 300,
        peak_decay_db_s: 20,
        watching: false,
    };
    assert_eq!(r::RtaRequest::from(&RtaRequest::from(&q)), q);
    let s = r::SiggenConfig {
        signal_type: 3,
        channel_mask: 3,
        invert_mask: 2,
        flags: SIGGEN_FLAG_WALK,
        level_db: -20.0,
        duration_ms: 1000,
        repeat: 2,
        gap_ms: 50,
        p: vec![1.0, 2.0, 3.0, 4.0],
    };
    assert_eq!(r::SiggenConfig::from(&SiggenConfig::from(&s)), s);
}
