//! WP ROLE0 tests: the speaker layout store, the one inference, the
//! effective role and measuring from the layout. Stores live beside the test
//! binary, never on tmpfs.

use std::path::PathBuf;
use std::sync::Arc;

use dspi_session::output_label::{self as ol, Band, LabelFilter};
use dspi_transport::FakeDevice;
use serde_json::Value;

use super::*;
use crate::consts::{IMPORT_BY_BLOCK, IMPORT_CROSSOVER, IMPORT_EQ};
use crate::measure::{
    MeasureLayout, MeasureOutput, MeasurePreflight, MeasureSetup, MeasureTuneOptions,
};
use crate::records::FilterParams;

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::current_exe()
        .unwrap()
        .with_file_name("test-scratch")
        .join(format!("role0-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A demo device opened as `serial`.
fn core_as(serial: &str) -> Arc<DspiCore> {
    let core = DspiCore::new();
    core.attach_probed(Box::new(FakeDevice::demo()), serial.into(), 0)
        .unwrap();
    core.fetch_all().unwrap();
    core
}

fn band(filter_type: u8, freq: f32) -> FilterParams {
    FilterParams {
        filter_type,
        bypass: false,
        freq,
        q: 0.707,
        gain: 0.0,
        qp: 0.0,
    }
}

fn confirmed(outputs: &[(u8, MeasureRole)]) -> SpeakerLayout {
    SpeakerLayout {
        outputs: outputs
            .iter()
            .map(|(o, r)| SpeakerLayoutOutput {
                output: *o,
                role: *r,
                speaker: if *o % 2 == 0 { "L" } else { "R" }.into(),
                group: *o % 2,
            })
            .collect(),
        confirmed_by_user: true,
        ..SpeakerLayout::default()
    }
}

fn keys(p: &MeasurePreflight) -> Vec<String> {
    p.blockers.iter().map(|b| b.key.clone()).collect()
}

const LP12: u8 = 4;
const HP12: u8 = 5;
const LP6: u8 = 12;
const HP6: u8 = 13;
const BELL: u8 = 0;

/// A table of filter chains with the role both the labels and the
/// measurement's layout give them.
/// What, its filters (type, corner), the role.
type Case = (&'static str, Vec<(u8, f32)>, MeasureRole);

fn table() -> Vec<Case> {
    vec![
        ("nothing", vec![], MeasureRole::Full),
        ("a bell", vec![(BELL, 1_000.0)], MeasureRole::Full),
        (
            "a subsonic high-pass",
            vec![(HP12, 25.0)],
            MeasureRole::Full,
        ),
        (
            "a top-octave trim",
            vec![(LP12, 16_000.0)],
            MeasureRole::Full,
        ),
        ("a woofer", vec![(LP12, 3_500.0)], MeasureRole::Low),
        ("a sub", vec![(LP12, 90.0)], MeasureRole::Sub),
        ("a 1st-order sub", vec![(LP6, 120.0)], MeasureRole::Sub),
        ("a tweeter", vec![(HP12, 3_500.0)], MeasureRole::High),
        ("a satellite", vec![(HP12, 200.0)], MeasureRole::High),
        (
            "a mid",
            vec![(HP12, 300.0), (LP12, 3_000.0)],
            MeasureRole::Mid,
        ),
        (
            "a woofer with a subsonic filter",
            vec![(HP6, 30.0), (LP12, 2_000.0)],
            MeasureRole::Low,
        ),
    ]
}

#[test]
fn wp_role0_band_and_layout_agree_on_a_table_of_filter_chains() {
    for (what, chain, want) in table() {
        // The labels' side: dspi-session's band over the same filters.
        let filters: Vec<LabelFilter> = chain
            .iter()
            .map(|&(t, f)| LabelFilter {
                filter_type: t,
                bypass: false,
                freq: f,
            })
            .collect();
        let label = ol::band(&filters);
        assert_eq!(label, MeasureRole::driver(want).band(), "{what}: the label");
        // The measurement's side: the layout proposal over a device state.
        let core = core_as("ROLE0-TABLE");
        let n_in = core.get_state().num_input_channels;
        for (i, &(t, f)) in chain.iter().enumerate() {
            core.set_filter(n_in, u8::try_from(i).unwrap(), band(t, f))
                .unwrap();
        }
        let l = crate::measure::layout_proposal_of(&core.get_state());
        let o = l.outputs.iter().find(|o| o.output == 0).expect(what);
        assert_eq!(o.role, want, "{what}: the layout");
        assert_eq!(
            MeasureRole::driver(o.role).band(),
            label,
            "{what}: the two agree"
        );
        let shown = core.output_labels();
        assert_eq!(shown[0].band, label.word(), "{what}: the core's labels");
    }
    // A name with "sub" is a sub to the layout (and Low to the label).
    let f = [LabelFilter {
        filter_type: LP12,
        bypass: false,
        freq: 300.0,
    }];
    assert_eq!(ol::driver_role(&f, "Rear SUB"), ol::DriverRole::Sub);
    assert_eq!(ol::DriverRole::Sub.band(), Band::Low);
}

#[test]
fn wp_role0_a_full_output_with_a_highpass_at_1khz_is_high_for_s6() {
    let core = core_as("ROLE0-S6");
    core.set_user_volume(-20.0).unwrap();
    let n_in = core.get_state().num_input_channels;
    let setup = |role| MeasureSetup {
        outputs: vec![MeasureOutput {
            output: 0,
            role,
            ..MeasureOutput::default()
        }],
        ..MeasureSetup::default()
    };
    let has_s6 = |role| {
        keys(&core.measure_preflight(setup(role)).unwrap()).contains(&NOTICE_TWEETER.to_owned())
    };
    // No high-pass: Full is not held to the tweeter rule.
    assert!(!has_s6(MeasureRole::Full));
    // A first-order high-pass at 2 kHz: too shallow for a tweeter. Called
    // Full, it is still a tweeter for safety, and refused.
    core.set_filter(n_in, 0, band(HP6, 2_000.0)).unwrap();
    assert!(has_s6(MeasureRole::High));
    assert!(has_s6(MeasureRole::Full), "a Full label does not skip S6");
    assert!(has_s6(MeasureRole::Low));
    let p = core.measure_preflight(setup(MeasureRole::Full)).unwrap();
    let b = p.blockers.iter().find(|b| b.key == NOTICE_TWEETER).unwrap();
    assert!(b.message.contains("1 kHz or above"), "{}", b.message);
    // A steep one at 2 kHz passes, whatever the label.
    core.set_filter(n_in, 0, band(BELL, 1_000.0)).unwrap();
    core.set_crossover(n_in, 0, band(35, 2_000.0)).unwrap(); // LR4 high-pass
    assert!(!has_s6(MeasureRole::Full));
    assert!(!has_s6(MeasureRole::High));
    // A satellite's 200 Hz high-pass leaves a Full label alone.
    core.set_crossover(n_in, 0, band(BELL, 1_000.0)).unwrap();
    core.set_filter(n_in, 0, band(HP6, 200.0)).unwrap();
    assert!(!has_s6(MeasureRole::Full));
    // The pure rule.
    let hp = |f: f32| {
        [LabelFilter {
            filter_type: HP6,
            bypass: false,
            freq: f,
        }]
    };
    use ol::DriverRole as D;
    assert_eq!(ol::effective_role(D::Full, &hp(1_000.0)), D::High);
    assert_eq!(ol::effective_role(D::Mid, &hp(999.6)), D::High);
    assert_eq!(ol::effective_role(D::Full, &hp(999.0)), D::Full);
    assert_eq!(ol::effective_role(D::High, &hp(200.0)), D::High);
    let mut bypassed = hp(2_000.0);
    bypassed[0].bypass = true;
    assert_eq!(ol::effective_role(D::Sub, &bypassed), D::Sub);
}

const NOTICE_TWEETER: &str = "measure.tweeter_highpass";

#[test]
fn wp_role0_roles_from_the_layout_need_a_confirmed_layout() {
    let core = core_as("ROLE0-MODE");
    core.set_user_volume(-20.0).unwrap();
    let from_layout = |outputs: Vec<MeasureOutput>| MeasureSetup {
        outputs,
        roles_from_layout: true,
        ..MeasureSetup::default()
    };
    let refused = |r: Result<MeasurePreflight>| match r {
        Err(DspiError::InvalidArgument { msg }) => {
            assert!(msg.starts_with(NOTICE_LAYOUT_UNCONFIRMED), "{msg}");
            assert!(crate::notice_text::has_row(NOTICE_LAYOUT_UNCONFIRMED));
        }
        other => panic!("not refused: {other:?}"),
    };
    // No layout: refused, and the start too.
    refused(core.measure_preflight(from_layout(vec![])));
    assert!(matches!(
        core.measure_start(from_layout(vec![])),
        Err(DspiError::InvalidArgument { msg }) if msg.starts_with(NOTICE_LAYOUT_UNCONFIRMED)
    ));
    // A layout not confirmed: still refused.
    let mut l = confirmed(&[(0, MeasureRole::High), (1, MeasureRole::Low)]);
    l.confirmed_by_user = false;
    core.speaker_layout_set(l.clone()).unwrap();
    refused(core.measure_preflight(from_layout(vec![])));
    // The tune and the live view take the same mode.
    assert!(matches!(
        core.measure_tune_propose(
            String::new(),
            MeasureLayout::default(),
            MeasureTuneOptions {
                roles_from_layout: true,
                ..MeasureTuneOptions::default()
            }
        ),
        Err(DspiError::InvalidArgument { msg }) if msg.starts_with(NOTICE_LAYOUT_UNCONFIRMED)
    ));
    assert!(matches!(
        core.measure_live_start(crate::measure::MeasureLiveSetup {
            roles_from_layout: true,
            ..Default::default()
        }),
        Err(DspiError::InvalidArgument { msg }) if msg.starts_with(NOTICE_LAYOUT_UNCONFIRMED)
    ));
    // Confirmed: the roles are the layout's, not the caller's. Output 0 is a
    // tweeter there with no high-pass, so S6 refuses it although the caller
    // said Full.
    l.confirmed_by_user = true;
    core.speaker_layout_set(l).unwrap();
    let caller_says_full = vec![MeasureOutput {
        output: 0,
        role: MeasureRole::Full,
        ..MeasureOutput::default()
    }];
    let p = core
        .measure_preflight(from_layout(caller_says_full.clone()))
        .unwrap();
    assert!(keys(&p).contains(&NOTICE_TWEETER.to_owned()), "{p:#?}");
    // -- WP RCFIX -- and with the caller's own roles too: the confirmed
    // layout is the stricter one.
    let p = core
        .measure_preflight(MeasureSetup {
            outputs: caller_says_full,
            ..MeasureSetup::default()
        })
        .unwrap();
    let b = p.blockers.iter().find(|b| b.key == NOTICE_TWEETER).unwrap();
    assert!(b.message.contains("in the speaker layout"), "{}", b.message);
    // No outputs listed: every output of the layout; one it lacks: refused.
    let s = core.setup_roles(from_layout(vec![])).unwrap();
    let got: Vec<(u8, MeasureRole, String)> = s
        .outputs
        .iter()
        .map(|o| (o.output, o.role, o.speaker.clone()))
        .collect();
    assert_eq!(
        got,
        vec![
            (0, MeasureRole::High, "L".into()),
            (1, MeasureRole::Low, "R".into())
        ]
    );
    let lacking = from_layout(vec![MeasureOutput {
        output: 3,
        ..MeasureOutput::default()
    }]);
    refused(core.measure_preflight(lacking));
    // The tune's layout gets the same roles.
    let t = core.layout_roles(MeasureLayout::default(), true).unwrap();
    assert_eq!(t.outputs.len(), 2);
    assert_eq!(t.outputs[0].role, MeasureRole::High);
    assert_eq!(t.outputs[1].speaker, "R");
}

#[test]
fn wp_role0_the_layout_persists_per_serial() {
    let dir = scratch("persist");
    let a = core_as("ROLE0-A");
    a.speaker_layout_set_dir(dir.display().to_string()).unwrap();
    assert_eq!(a.speaker_layout().unwrap(), None);
    let kept = a
        .speaker_layout_set(confirmed(&[(1, MeasureRole::High), (0, MeasureRole::Low)]))
        .unwrap();
    assert_eq!(kept.serial, "ROLE0-A");
    assert!(kept.updated_ms > 0);
    assert_eq!(kept.outputs[0].output, 0, "kept in output order");
    // Another core with the same folder reads it back for that serial only.
    let again = core_as("ROLE0-A");
    again
        .speaker_layout_set_dir(dir.display().to_string())
        .unwrap();
    assert_eq!(again.speaker_layout().unwrap(), Some(kept.clone()));
    let b = core_as("ROLE0-B");
    b.speaker_layout_set_dir(dir.display().to_string()).unwrap();
    assert_eq!(b.speaker_layout().unwrap(), None);
    // A layout set before the folder was named moves into it.
    let c = core_as("ROLE0-C");
    c.speaker_layout_set(confirmed(&[(0, MeasureRole::Full)]))
        .unwrap();
    c.speaker_layout_set_dir(dir.display().to_string()).unwrap();
    assert_eq!(c.speaker_layout().unwrap().unwrap().outputs.len(), 1);
    assert!(
        dir.join(dspi_session::snapshot_store::dir_name("ROLE0-C"))
            .join(FILE)
            .exists()
    );
    // A damaged file is set aside and reads as none; never a panic.
    let file = dir
        .join(dspi_session::snapshot_store::dir_name("ROLE0-A"))
        .join(FILE);
    std::fs::write(&file, "{not json").unwrap();
    assert_eq!(again.speaker_layout().unwrap(), None);
    assert!(file.with_extension("json.bad").exists());
    // A bad layout is refused and nothing kept.
    for bad in [
        confirmed(&[(0, MeasureRole::Low), (0, MeasureRole::High)]),
        confirmed(&[(99, MeasureRole::Low)]),
        SpeakerLayout {
            outputs: vec![SpeakerLayoutOutput {
                speaker: "x".repeat(65),
                ..SpeakerLayoutOutput::default()
            }],
            ..SpeakerLayout::default()
        },
    ] {
        assert!(matches!(
            b.speaker_layout_set(bad),
            Err(DspiError::InvalidArgument { .. })
        ));
    }
    assert_eq!(b.speaker_layout().unwrap(), None);
    // The proposal is never saved by itself, nor confirmed.
    let p = b.speaker_layout_propose().unwrap();
    assert!(!p.confirmed_by_user);
    assert_eq!(p.updated_ms, 0);
    assert_eq!(b.speaker_layout().unwrap(), None);
    // No device: refused.
    let none = DspiCore::new();
    assert!(matches!(
        none.speaker_layout(),
        Err(DspiError::Disconnected)
    ));
    assert!(matches!(
        none.speaker_layout_set(SpeakerLayout::default()),
        Err(DspiError::Disconnected)
    ));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn wp_role0_the_layout_round_trips_through_config_export_and_import() {
    let core = core_as("ROLE0-CFG");
    let layout = confirmed(&[(0, MeasureRole::Low), (1, MeasureRole::High)]);
    core.speaker_layout_set(layout.clone()).unwrap();
    let text = core.config_export("R".into(), "test".into(), 0).unwrap();
    let doc: Value = serde_json::from_str(&text).unwrap();
    let meta = &doc["meta"][META_BLOCK];
    assert_eq!(meta["confirmedByUser"], true, "{meta}");
    assert_eq!(meta["serial"], "", "no serial in an export");
    assert_eq!(meta["outputs"][1]["role"], "high");

    // Another device imports it with the crossover: kept for that device.
    let other = core_as("ROLE0-CFG2");
    let import = |c: &DspiCore, options: u32| -> Value {
        let t = c.confirm_token(crate::ConfirmOp::ConfigImport);
        serde_json::from_str(&c.config_import_confirmed(text.clone(), options, u32::MAX, Some(t)))
            .unwrap()
    };
    // Without the crossover block the layout stays behind.
    let r = import(&other, IMPORT_BY_BLOCK | IMPORT_EQ);
    assert_eq!(
        (r["ok"].clone(), r["speakerLayout"].clone()),
        (true.into(), false.into()),
        "{r}"
    );
    assert_eq!(other.speaker_layout().unwrap(), None);
    let r = import(&other, IMPORT_BY_BLOCK | IMPORT_CROSSOVER);
    assert_eq!(r["speakerLayout"], true, "{r}");
    let got = other.speaker_layout().unwrap().unwrap();
    assert_eq!(got.serial, "ROLE0-CFG2");
    assert_eq!(got.outputs, layout.outputs);
    // Review RA F1: an import is never confirmed, whatever the file says,
    // so measuring refuses until the user confirms it on this device.
    assert!(!got.confirmed_by_user);
    assert!(other.confirmed_speaker_layout().is_err());
    // The old option bits import every audio block, the layout with them.
    let third = core_as("ROLE0-CFG3");
    let r = import(&third, 0);
    assert_eq!(r["speakerLayout"], true, "{r}");
    // A document without one leaves the stored layout alone.
    let bare = third.config_export("B".into(), "t".into(), 0).unwrap();
    let mut bare: Value = serde_json::from_str(&bare).unwrap();
    bare["meta"].as_object_mut().unwrap().remove(META_BLOCK);
    let t = core.confirm_token(crate::ConfirmOp::ConfigImport);
    let r: Value =
        serde_json::from_str(&core.config_import_confirmed(bare.to_string(), 0, u32::MAX, Some(t)))
            .unwrap();
    assert_eq!(r["speakerLayout"], false, "{r}");
    assert_eq!(
        core.speaker_layout().unwrap().unwrap().outputs,
        layout.outputs
    );
}

/// Owner 2026-11-22: a tweeter is never measured below 1 kHz. The floor is
/// hard: a limits profile may raise it, never lower it, and it follows the
/// effective role.
#[test]
fn wp_role0_the_tweeter_floor_of_1khz_is_hard() {
    use crate::measure::preflight_tweeter_floor as floor;
    // The pure rule: a high way's corner and slope never drop below 1 kHz
    // and 12 dB/octave; other ways keep theirs.
    assert_eq!(floor(MeasureRole::High, 500.0, 6), (1_000.0, 12));
    assert_eq!(floor(MeasureRole::High, 2_000.0, 24), (2_000.0, 24));
    assert_eq!(floor(MeasureRole::High, f32::NAN, 0), (1_000.0, 12));
    assert_eq!(floor(MeasureRole::Mid, 300.0, 24), (300.0, 24));
    assert_eq!(floor(MeasureRole::Full, 500.0, 6), (500.0, 6));

    let core = core_as("ROLE0-FLOOR");
    core.set_user_volume(-20.0).unwrap();
    let n_in = core.get_state().num_input_channels;
    // An LR4 high-pass at 600 Hz on output 0, and a profile that would allow
    // a "tweeter" down to 500 Hz.
    core.set_crossover(n_in, 0, band(35, 600.0)).unwrap();
    let profile = dspi_session::limits::LimitsProfile {
        outputs: vec![dspi_session::limits::OutputLimits {
            output: 0,
            highpass_min_hz: Some(500.0),
            highpass_min_slope_db: Some(12),
            ..dspi_session::limits::OutputLimits::default()
        }],
        ..dspi_session::limits::LimitsProfile::default()
    };
    core.limits_apply_profile(profile.to_document()).unwrap();
    let s6 = |role| {
        let p = core
            .measure_preflight(MeasureSetup {
                outputs: vec![MeasureOutput {
                    output: 0,
                    role,
                    ..MeasureOutput::default()
                }],
                ..MeasureSetup::default()
            })
            .unwrap();
        keys(&p).contains(&NOTICE_TWEETER.to_owned())
    };
    assert!(
        s6(MeasureRole::High),
        "the profile's 500 Hz does not lower the floor"
    );
    // From the layout too: the user calls it a tweeter.
    core.speaker_layout_set(confirmed(&[(0, MeasureRole::High)]))
        .unwrap();
    let p = core
        .measure_preflight(MeasureSetup {
            roles_from_layout: true,
            ..MeasureSetup::default()
        })
        .unwrap();
    assert!(keys(&p).contains(&NOTICE_TWEETER.to_owned()), "{p:#?}");
    // At 1.2 kHz it passes as a tweeter, and a Full label is a tweeter there
    // (the effective role) held to the same floor.
    core.set_crossover(n_in, 0, band(35, 1_200.0)).unwrap();
    assert!(!s6(MeasureRole::High));
    assert!(!s6(MeasureRole::Full));
    // A Full output whose only high-pass at 1 kHz or above is 6 dB/octave
    // (the profile keeps the 600 Hz one): a tweeter, below the floor's slope.
    core.set_crossover(n_in, 0, band(35, 600.0)).unwrap();
    core.set_filter(n_in, 0, band(HP6, 1_200.0)).unwrap();
    assert!(s6(MeasureRole::Full), "6 dB/octave is below the floor's 12");
}

// -- WP INT1 -- (ROLE0 x MCP0)
#[test]
fn wp_int1_an_agent_cannot_set_the_speaker_layout() {
    use crate::actor::{ActorScope, USER_ONLY, register_actor};
    let core = core_as("INT1-AGENT");
    let agent = register_actor("AI · INT1");
    let layout = confirmed(&[(0, MeasureRole::Full), (1, MeasureRole::High)]);
    {
        let _scope = ActorScope::enter(&core, agent, None);
        match core.speaker_layout_set(layout.clone()) {
            Err(DspiError::ConfirmationRequired { code, .. }) => assert_eq!(code, USER_ONLY),
            other => panic!("an agent set the layout: {other:?}"),
        }
        // Nor through an import the agent makes: the device part applies,
        // the layout stays behind.
        let source = core_as("INT1-SRC");
        source.speaker_layout_set(layout.clone()).unwrap();
        let text = source.config_export("S".into(), "t".into(), 0).unwrap();
        let t = core.confirm_token(crate::ConfirmOp::ConfigImport);
        let r: Value = serde_json::from_str(&core.config_import_confirmed(
            text,
            IMPORT_BY_BLOCK | IMPORT_CROSSOVER,
            u32::MAX,
            Some(t),
        ))
        .unwrap();
        assert_eq!(
            (r["ok"].clone(), r["speakerLayout"].clone()),
            (true.into(), false.into()),
            "{r}"
        );
    }
    assert_eq!(core.speaker_layout().unwrap(), None);
    // The app itself still sets it.
    assert!(core.speaker_layout_set(layout).unwrap().confirmed_by_user);
}

// -- WP MDQT1 --
/// A layout confirmed before the folder was named, and a folder that cannot be
/// written: said, and still in force (review RA, ROLE0 item 3).
#[test]
fn wp_mdqt1_a_failed_migration_is_said_and_loses_nothing() {
    let core = core_as("MIGRATE1");
    let kept = core
        .speaker_layout_set(confirmed(&[(0, MeasureRole::High)]))
        .unwrap();
    let dir = scratch("migration");
    std::fs::create_dir_all(&dir).unwrap();
    let blocked = dir.join("a-file");
    std::fs::write(&blocked, "x").unwrap();
    assert!(matches!(
        core.speaker_layout_set_dir(blocked.join("layouts").display().to_string()),
        Err(DspiError::Io { .. })
    ));
    assert_eq!(core.speaker_layout().unwrap(), Some(kept.clone()));
    // Naming a folder that works writes it.
    let good = dir.join("layouts").display().to_string();
    core.speaker_layout_set_dir(good.clone()).unwrap();
    let again = core_as("MIGRATE1");
    again.speaker_layout_set_dir(good).unwrap();
    assert_eq!(again.speaker_layout().unwrap(), Some(kept));
}

// -- WP FFIX -- FUNC1 bug 4
#[test]
fn wp_ffix_a_summed_run_plays_each_output_with_its_routes_polarity() {
    use crate::measure::MeasureRunKind;
    let core = core_as("FFIX-SUM");
    core.speaker_layout_set_dir(scratch("sum").to_string_lossy().into_owned())
        .unwrap();
    core.speaker_layout_set(confirmed(&[(0, MeasureRole::Low), (1, MeasureRole::Sub)]))
        .unwrap();
    // The sub's programme comes inverted (on its crosspoint), the woofer's not.
    for i in 0..core.get_state().num_input_channels {
        core.set_matrix_route(i, 1, false, 0.0, false).unwrap();
    }
    core.set_matrix_route(0, 0, true, 0.0, false).unwrap();
    core.set_matrix_route(0, 1, true, 0.0, true).unwrap();
    let setup = |kind| MeasureSetup {
        kind,
        outputs: [0u8, 1]
            .iter()
            .map(|o| MeasureOutput {
                output: *o,
                ..MeasureOutput::default()
            })
            .collect(),
        roles_from_layout: true,
        ..MeasureSetup::default()
    };
    let inverted =
        |s: MeasureSetup| -> Vec<bool> { s.outputs.iter().map(|o| o.inverted).collect() };
    let s = core.setup_roles(setup(MeasureRunKind::Summed)).unwrap();
    assert_eq!(inverted(s), [false, true]);
    // A walked run plays each alone: no polarity to set.
    let s = core.setup_roles(setup(MeasureRunKind::Walked)).unwrap();
    assert_eq!(inverted(s), [false, false]);
}
