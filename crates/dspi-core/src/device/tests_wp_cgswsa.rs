//! WP CG-SWS-a: the feature names with no probe, and the list of every name.

use super::*;

/// The probe's names on the open device, read as `feature_present` does.
fn probed_names(core: &DspiCore) -> Vec<String> {
    core.ensure_probed();
    let mut inner = core.lock();
    let conn = inner.conn().unwrap();
    conn.session()
        .unwrap()
        .capabilities()
        .features
        .iter()
        .map(|f| f.name.clone())
        .collect()
}

#[test]
fn cgswsa_known_feature_names_cover_the_probe() {
    let core = DspiCore::new();
    core.open_demo().unwrap();
    let probed = probed_names(&core);
    assert_eq!(probed, PROBED_FEATURES, "the probe's names changed");
    let known = known_feature_names();
    assert_eq!(known.len(), PROBED_FEATURES.len() + DERIVED_FEATURES.len());
    let mut unique = known.clone();
    unique.sort();
    unique.dedup();
    assert_eq!(unique.len(), known.len(), "a name twice");
    for name in &known {
        // Every known name answers on the current demo firmware.
        assert!(core.feature_present(name), "{name}");
    }
}

#[test]
fn cgswsa_derived_features_need_a_device() {
    let core = DspiCore::new();
    for name in DERIVED_FEATURES {
        assert!(!core.feature_present(name), "{name}");
    }
}

/// Each derived name against the Mac's version test it replaces
/// (`apps/apple/DSPi Console/DSPViewModel.swift`): `>= 13` :2121, `>= 14`
/// :2125, `>= 28` :2134, `>= 22` :2197, `>= 30` :2214, `>= 26` with RP2350
/// :2231-2233, firmware 1.1.4 :2288-2310; the crossover gate is set on every
/// image the core decodes (`Core/CoreMirror.swift:941`).
#[test]
fn cgswsa_derived_features_match_the_swift_version_gates() {
    let mut seen = 0;
    for wire in 16..=40u8 {
        let core = DspiCore::new();
        if core.open_demo_wire(wire).is_err() {
            continue;
        }
        seen += 1;
        let s = core.get_state();
        let v = s.format_version;
        let fw = (s.fw_major, s.fw_minor, s.fw_patch) >= (1, 1, 4);
        // The core knows no version past its own: a newer one has every feature.
        let v = v.min(dspi_proto::generated::wire::WIRE_FORMAT_VERSION as u8);
        for (name, want) in [
            ("crossover_filters", v >= 11),
            ("first_order_allpass", v >= 13),
            ("first_order_shelves", v >= 14),
            ("first_order_pass", v >= 28),
            ("linkwitz_transform", v >= 22),
            ("upmixer_presence", v >= 26),
            ("stereo_upmixer", v >= 26 && s.platform_id == 1),
            ("subharmonic_extended", v >= 30),
            ("notch_filter", fw),
            ("allpass_filter", fw),
            ("band_bypass", fw),
        ] {
            assert_eq!(core.feature_present(name), want, "V{wire}: {name}");
        }
    }
    assert!(seen >= 3, "fixtures: {seen}");
    // The current demo has all of them.
    let core = DspiCore::new();
    core.open_demo().unwrap();
    for name in DERIVED_FEATURES {
        assert!(core.feature_present(name), "{name}");
    }
}

#[test]
fn cgswsa_stereo_upmixer_needs_an_rp2350() {
    let mut s = crate::state::default_state();
    s.format_version = 32;
    s.platform_id = 1;
    assert_eq!(derived_feature(&s, "stereo_upmixer"), Some(true));
    s.platform_id = 0;
    assert_eq!(derived_feature(&s, "stereo_upmixer"), Some(false));
    assert_eq!(derived_feature(&s, "upmixer_presence"), Some(true));
    s.format_version = 29;
    assert_eq!(derived_feature(&s, "subharmonic_extended"), Some(false));
    assert_eq!(derived_feature(&s, "input_source"), None);
}
