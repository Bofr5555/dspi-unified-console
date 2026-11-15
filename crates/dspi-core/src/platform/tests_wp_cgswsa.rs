//! WP CG-SWS-a: the platform table, the core-voltage codes, the default
//! channel names, the choice labels and the image's presence bytes, each
//! against the Mac's rule it replaces (cited by line) and the core's own.

use super::*;
use crate::DspiCore;
use crate::records::DspState;

/// The Mac's platform rules, `platformName == "RP2040" ? a : b`, with its id
/// to name mapping (`Commands.swift:160-162`: an unknown id reads `RP2040`).
fn swift_is_rp2040(id: u8) -> bool {
    !matches!(id, 1 | 2)
}

#[test]
fn cgswsa_platform_shape_matches_the_swift_rules() {
    for id in 0..=255u8 {
        let s = platform_shape(id);
        let small = swift_is_rp2040(id);
        let p = |a, b| if small { a } else { b };
        // DSPViewModel.swift:1970 (chOut1), :2346 (numOutputChannels),
        // :2350 (pdmPinIndex), :2352 (eqWorkerRange), :2353 (numOutputSlots),
        // :1540 (i2sMaxPairs).
        assert_eq!(s.num_input_channels, p(2, 8), "{id}");
        assert_eq!(s.num_output_channels, p(5, 9), "{id}");
        assert_eq!(s.pdm_pin_index, p(2, 4), "{id}");
        assert_eq!((s.eq_worker_first, s.eq_worker_last), (2, p(3, 7)), "{id}");
        assert_eq!(s.num_output_slots, p(2, 4), "{id}");
        assert_eq!(s.i2s_max_pairs, p(1, 4), "{id}");
        assert_eq!(s.pdm_output, s.num_output_channels - 1, "{id}");
        // DSPViewModel.swift:1543 (multichannel I2S), Commands.swift:2376
        // and :2445 (ADAT), DSPViewModel.swift:2232 (upmixer),
        // DSPi_ConsoleApp.swift:427 (the I2S page).
        assert_eq!(s.multichannel_i2s, id == 1, "{id}");
        assert_eq!(s.adat, id == 1, "{id}");
        assert_eq!(s.upmixer, id == 1, "{id}");
        assert_eq!(s.i2s_config, id != 2, "{id}");
        assert_eq!(s.known, id <= 2, "{id}");
    }
    assert_eq!(platform_shape(0).name, "RP2040");
    assert_eq!(platform_shape(1).name, "RP2350");
    assert_eq!(platform_shape(2).name, "STM32H723");
    assert_eq!(platform_shape(9).name, "unknown platform 9");
}

#[test]
fn cgswsa_platform_shape_agrees_with_the_core() {
    for id in 0..=255u8 {
        let s = platform_shape(id);
        let mut st = crate::state::default_state();
        st.platform_id = id;
        assert_eq!(s.crossfeed_pairs, crate::ext::crossfeed_pairs(&st), "{id}");
        assert_eq!(s.max_delay_ms, crate::state::max_delay_ms_for(&st), "{id}");
    }
}

/// The Mac's `VregVoltage.volts` (`ExtensionChannel.swift:272-292`).
fn swift_volts(code: u8, platform: &str) -> Option<f64> {
    const HIGH: [f64; 16] = [
        1.35, 1.40, 1.50, 1.60, 1.65, 1.70, 1.80, 1.90, 2.00, 2.35, 2.50, 2.65, 2.80, 3.00, 3.15,
        3.30,
    ];
    match code {
        0x00..=0x0F => {
            if platform == "RP2040" && code < 0x06 {
                return Some(0.80);
            }
            Some(0.55 + 0.05 * f64::from(code))
        }
        0x10..=0x1F => (platform != "RP2040").then(|| HIGH[usize::from(code - 0x10)]),
        _ => None,
    }
}

#[test]
fn cgswsa_vreg_codes_match_the_swift_table() {
    for (id, name) in [(0u8, "RP2040"), (1, "RP2350")] {
        for code in 0..=255u8 {
            let want = swift_volts(code, name);
            let got = vreg_code_mv(code, id);
            assert_eq!(
                got,
                want.map(|v| (v * 1000.0).round() as u16),
                "{name} 0x{code:02X}"
            );
            // ExtensionChannel.swift:294-298: "%.2f V", else "code 0x%02X".
            let label = match want {
                Some(v) => format!("{v:.2} V"),
                None => format!("code 0x{code:02X}"),
            };
            assert_eq!(vreg_code_label(code, id), label, "{name} 0x{code:02X}");
        }
    }
    assert_eq!(vreg_code_label(0x0D, 1), "1.20 V");
    assert_eq!(vreg_code_label(0x02, 0), "0.80 V");
    assert_eq!(vreg_code_label(0x02, 1), "0.65 V");
    assert_eq!(vreg_code_label(0x1E, 1), "3.15 V");
    assert_eq!(vreg_code_label(0x1E, 0), "code 0x1E");
}

#[test]
fn cgswsa_vreg_codes_elsewhere_are_the_shared_range() {
    for id in [2u8, 3, 255] {
        for code in 0..=255u8 {
            assert_eq!(
                vreg_code_mv(code, id),
                dspi_proto::ext::vreg_mv(code),
                "{id} 0x{code:02X}"
            );
        }
    }
}

/// The Mac's `defaultChannelNames(for:slotTypes:)`
/// (`DSPViewModel.swift:3445-3471`) for both platforms, written out.
#[test]
fn cgswsa_default_channel_names_match_the_swift_lists() {
    let rp2040 = platform_shape(0);
    assert_eq!(
        default_channel_names(
            rp2040.num_input_channels,
            rp2040.num_output_channels,
            vec![0, 1]
        ),
        [
            "USB L",
            "USB R",
            "SPDIF 1 L",
            "SPDIF 1 R",
            "I2S 2 L",
            "I2S 2 R",
            "PDM"
        ]
    );
    let rp2350 = platform_shape(1);
    assert_eq!(
        default_channel_names(
            rp2350.num_input_channels,
            rp2350.num_output_channels,
            vec![0, 1, 0, 1]
        ),
        [
            "USB L",
            "USB R",
            "USB 3",
            "USB 4",
            "USB 5",
            "USB 6",
            "USB 7",
            "USB 8",
            "SPDIF 1 L",
            "SPDIF 1 R",
            "I2S 2 L",
            "I2S 2 R",
            "SPDIF 3 L",
            "SPDIF 3 R",
            "I2S 4 L",
            "I2S 4 R",
            "PDM",
        ]
    );
    // Missing slot types read as S/PDIF, as the Mac's `slotName`.
    assert_eq!(
        default_channel_names(0, 3, vec![]),
        ["SPDIF 1 L", "SPDIF 1 R", "PDM"]
    );
    assert!(default_channel_names(0, 0, vec![]).is_empty());
    assert_eq!(default_channel_names(255, 255, vec![1; 300]).len(), 510);
}

/// The core's own preset rows name an unnamed channel by the same rule
/// (`presets.rs` `channel_label`).
#[test]
fn cgswsa_default_channel_names_match_the_preset_rows() {
    for (inputs, outputs, types) in [
        (2u8, 5u8, vec![0u8, 1]),
        (8, 9, vec![1, 0, 1, 0]),
        (4, 3, vec![1]),
        (1, 1, vec![]),
    ] {
        let mut s: DspState = crate::state::default_state();
        s.num_input_channels = inputs;
        s.num_output_channels = outputs;
        s.output_types = types.clone();
        s.channel_names = Vec::new();
        let rows: Vec<String> = (0..usize::from(inputs) + usize::from(outputs))
            .map(|ch| crate::presets::channel_label(&s, ch).unwrap())
            .collect();
        assert_eq!(default_channel_names(inputs, outputs, types), rows);
    }
}

fn labels(path: &str) -> Vec<(u8, String, String)> {
    enum_value_labels(path.to_owned())
        .unwrap()
        .into_iter()
        .map(|l| (l.value, l.key, l.label))
        .collect()
}

fn words(path: &str) -> Vec<String> {
    labels(path).into_iter().map(|(_, _, w)| w).collect()
}

#[test]
fn cgswsa_enum_value_labels_are_the_shared_words() {
    // The words dspi-wasm's `effectChoices` (WP WB6H) and the TUI copy.
    assert_eq!(words("up.center_mode"), ["Sinner", "Logician", "Off"]);
    assert_eq!(words("up.surround_mode"), ["Off", "Sinner", "Logician"]);
    assert_eq!(
        words("sub.select"),
        ["All material", "Percussive", "Sustained"]
    );
    assert_eq!(words("lev.speed"), ["Slow", "Medium", "Fast"]);
    assert_eq!(
        words("cf.preset"),
        ["Default", "Chu Moy", "Meier", "Custom"]
    );
    assert_eq!(words("in.source")[0], "USB");
    assert_eq!(
        labels("up.center_mode")[0],
        (0, "passive".to_owned(), "Sinner".to_owned())
    );
    let tube = labels("tube.type");
    assert_eq!(tube.len(), 17);
    assert_eq!(tube[0], (0, "0".to_owned(), "Custom".to_owned()));
    assert_eq!(tube[1].2, dspi_session::tube::type_name(1));
    for (path, _) in LABELLED {
        assert!(!words(path).is_empty(), "{path}");
    }
    for bad in ["", "up.enabled", "no.such", "sub.low"] {
        assert!(
            matches!(
                enum_value_labels(bad.to_owned()),
                Err(DspiError::InvalidArgument { .. })
            ),
            "{bad}"
        );
    }
}

#[test]
fn cgswsa_enum_value_labels_values_are_the_registry_choices() {
    for (path, _) in LABELLED {
        let got: Vec<u8> = labels(path).into_iter().map(|(v, _, _)| v).collect();
        match registry::by_path(path).unwrap().kind {
            Kind::Choice(variants) => {
                assert_eq!(got, variants.iter().map(|(v, _)| *v).collect::<Vec<_>>())
            }
            Kind::Int { min, max, .. } => {
                assert_eq!(got, (min as u8..=max as u8).collect::<Vec<_>>())
            }
            k => panic!("{path}: {k:?}"),
        }
    }
}

/// The Mac's literal offsets (`CoreMirror.swift:661-720`): the input config
/// section's +4 (I2S channels), +4+pair (I2S pins 2-4), +8+i (S/PDIF pins
/// 2-4) and +11, `BULK_I2S_BCK_PIN_SLAVE_OFFSET` and `BULK_ADAT_OFFSET + 1`.
#[test]
fn cgswsa_bulk_image_extras_read_the_swift_offsets() {
    use dspi_proto::generated as g;
    let base = crate::state::encode_bulk(&crate::state::default_state());
    let mut img = base.clone();
    for i in [4, 5, 6, 7, 8, 9, 10] {
        img[g::OFF_INPUT_CONFIG + i] = 0;
    }
    img[g::OFF_I2S_CONFIG + 9] = 0;
    img[g::OFF_ADAT_CONFIG + 1] = 0;
    let none = bulk_image_extras(img.clone());
    assert!(none.decoded);
    assert_eq!(
        none,
        BulkImageExtras {
            decoded: true,
            i2s_rx_pin_ext_present: vec![false; 3],
            spdif_rx_pin_ext_present: vec![false; 3],
            ..BulkImageExtras::default()
        }
    );
    let set = |img: &mut Vec<u8>, at: usize| img[at] = 7;
    let mut one = img.clone();
    set(&mut one, g::OFF_INPUT_CONFIG + 4);
    assert!(bulk_image_extras(one).i2s_input_channels_present);
    for pair in 1..4usize {
        let mut one = img.clone();
        set(&mut one, g::OFF_INPUT_CONFIG + 4 + pair);
        let mut want = vec![false; 3];
        want[pair - 1] = true;
        assert_eq!(bulk_image_extras(one).i2s_rx_pin_ext_present, want);
    }
    for i in 0..3usize {
        let mut one = img.clone();
        set(&mut one, g::OFF_INPUT_CONFIG + 8 + i);
        let mut want = vec![false; 3];
        want[i] = true;
        assert_eq!(bulk_image_extras(one).spdif_rx_pin_ext_present, want);
    }
    let mut one = img.clone();
    set(&mut one, g::OFF_I2S_CONFIG + 9);
    assert!(bulk_image_extras(one).i2s_bck_pin_slave_present);
    let mut one = img.clone();
    set(&mut one, g::OFF_ADAT_CONFIG + 1);
    assert!(bulk_image_extras(one).adat_out_pin_present);
    // CoreMirror.swift:803-805 (`decoded`): a whole image of this version.
    let mut old = img.clone();
    old[0] = 31;
    assert!(!bulk_image_extras(old).decoded);
    assert!(!bulk_image_extras(img[..100].to_vec()).decoded);
    let empty = bulk_image_extras(Vec::new());
    assert!(!empty.decoded && !empty.adat_out_pin_present);
    assert_eq!(empty.spdif_rx_pin_ext_present, vec![false; 3]);
}

#[test]
fn cgswsa_image_extras_follow_the_mirror() {
    let core = DspiCore::new();
    assert_eq!(core.image_extras(), bulk_image_extras(Vec::new()));
    core.open_demo().unwrap();
    let raw = core.get_state().bulk_raw;
    assert!(!raw.is_empty());
    let extras = core.image_extras();
    assert_eq!(extras, bulk_image_extras(raw));
    assert!(extras.decoded);
}
