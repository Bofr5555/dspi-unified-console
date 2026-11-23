//! WP ADETS tests: the user correcting one row of a detected layout
//! (AUTODES 3.1). The layouts stay in memory (no folder named).

use std::sync::Arc;

use dspi_transport::FakeDevice;

use super::*;

fn core_as(serial: &str) -> Arc<DspiCore> {
    let core = DspiCore::new();
    core.attach_probed(Box::new(FakeDevice::demo()), serial.into(), 0)
        .unwrap();
    core.fetch_all().unwrap();
    core
}

fn row(output: u8, role: MeasureRole) -> SpeakerLayoutOutput {
    SpeakerLayoutOutput {
        output,
        role,
        speaker: "Centre".into(),
        group: 0,
        source: RoleSource::Detected,
    }
}

/// The user gives a detected row another role (Settings › Speakers, the
/// function test's `layout` action): that row is the user's from then on,
/// the others stay the detection's and the layout stays auto-detected; a
/// row sent back unchanged keeps its source.
#[test]
fn wp_adets_a_row_the_user_corrects_after_a_detection_is_the_users() {
    let core = core_as("ADETS-ROW");
    let outputs = core.lock().state.num_output_channels;
    let detected = SpeakerLayout {
        outputs: vec![row(0, MeasureRole::Low), row(2, MeasureRole::High)],
        detected: Some(Detection {
            device_outputs: outputs,
            summary: "1 woofer, 1 tweeter?.".into(),
            ..Detection::default()
        }),
        ..SpeakerLayout::default()
    };
    core.speaker_layout_keep_detected(&detected).unwrap();
    let kept = core.speaker_layout().unwrap().unwrap();
    assert_eq!(kept.state(), LayoutState::AutoDetected);

    // Out 3 is the midrange, the user says; Out 1 sent back as it was
    let mut corrected = kept.clone();
    corrected.outputs[1].role = MeasureRole::Mid;
    let set = core.speaker_layout_set(corrected).unwrap();
    let by_output = |l: &SpeakerLayout, o: u8| l.output(o).map(|r| (r.role, r.source));
    assert_eq!(
        by_output(&set, 2),
        Some((MeasureRole::Mid, RoleSource::User))
    );
    assert_eq!(
        by_output(&set, 0),
        Some((MeasureRole::Low, RoleSource::Detected))
    );
    let now = core.speaker_layout().unwrap().unwrap();
    assert_eq!(now.state(), LayoutState::AutoDetected, "not confirmed yet");
    assert!(now.detected.is_some(), "the detection's record stays");
    assert_eq!(
        crate::trusted_roles(&now),
        Some(vec![Some(MeasureRole::Low), None, Some(MeasureRole::Mid)])
    );
}
