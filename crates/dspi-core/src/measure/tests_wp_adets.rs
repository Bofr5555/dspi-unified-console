//! WP ADETS tests: a high way's measuring floor follows the limits profile's
//! high-pass minimum (the K340's 2.5 kHz, AUTODES owner Q3), found by the
//! function test: an agent's measurement of a detected K340 tweeter swept
//! from 1 kHz. The profile lives beside the test binary, never on tmpfs.

use std::path::PathBuf;

use dspi_session::limits::{LimitsProfile, OutputLimits};
use dspi_transport::FakeDevice;

use super::plan::{Plan, RunSpec};
use super::preflight::SIG_LOG_SWEEP;
use super::session::below_floor;
use super::{MeasureOutput, MeasureRole, MeasureSetup};
use crate::DspiCore;
use crate::records::SiggenConfig;

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::current_exe()
        .unwrap()
        .with_file_name("test-scratch")
        .join(format!("adets-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn plan(core: &DspiCore) -> Plan {
    let out = |output, role| MeasureOutput {
        output,
        role,
        speaker: "Left".into(),
        group: 0,
        inverted: false,
    };
    let mut plan = Plan::new(&MeasureSetup {
        outputs: vec![out(2, MeasureRole::Low), out(3, MeasureRole::High)],
        ..MeasureSetup::default()
    })
    .unwrap();
    core.measure_resolve_roles(&mut plan).unwrap();
    plan
}

fn sweep_from(f1: f32, output: u8) -> SiggenConfig {
    SiggenConfig {
        signal_type: SIG_LOG_SWEEP,
        channel_mask: 1 << output,
        level_db: -40.0,
        duration_ms: 1_000,
        p: vec![f1, 20_000.0],
        ..SiggenConfig::default()
    }
}

#[test]
fn wp_adets_a_high_ways_floor_follows_the_profiles_highpass_minimum() {
    let core = DspiCore::new();
    core.attach_probed(Box::new(FakeDevice::demo()), "ADETS-FLOOR".into(), 0)
        .unwrap();
    core.fetch_all().unwrap();
    core.limits_set_dir(scratch("floor").to_string_lossy().into())
        .unwrap();
    // No profile: the tweeter rule's 1 kHz
    let p = plan(&core);
    assert_eq!(p.floor_of(&[3]), 1_000.0);
    assert_eq!(p.floor_of(&[2]), 0.0);
    // The owner's 2.5 kHz on the cup's tweeter output (on the woofer's it
    // would hold a 2.5 kHz high-pass on the woofer's programme too)
    let floor = |output| OutputLimits {
        output,
        highpass_min_hz: Some(2_500.0),
        highpass_min_slope_db: Some(12),
        ..OutputLimits::default()
    };
    let profile = LimitsProfile {
        outputs: vec![floor(3)],
        ..LimitsProfile::default()
    };
    core.limits_apply_profile(profile.to_document()).unwrap();
    let p = plan(&core);
    assert_eq!(p.floor_of(&[3]), 2_500.0, "the tweeter from 2.5 kHz");
    assert_eq!(p.floor_of(&[2]), 0.0, "the woofer's is not raised");
    assert_eq!(p.floor_of(&[2, 3]), 2_500.0, "a run of both");
    let run = RunSpec {
        position: 0,
        outputs: vec![3],
        invert_mask: 0,
        f1_hz: 20.0,
        aligns: true,
    };
    assert!(p.sweep_of(&run).f1_hz >= 2_500.0);
    // The interlock holds it too: a sweep from 1 kHz on the tweeter refused,
    // from 2.5 kHz played; the woofer's from 20 Hz played
    assert!(below_floor(&p, &sweep_from(1_000.0, 3)).is_some());
    assert!(below_floor(&p, &sweep_from(2_500.0, 3)).is_none());
    assert!(below_floor(&p, &sweep_from(20.0, 2)).is_none());
}
