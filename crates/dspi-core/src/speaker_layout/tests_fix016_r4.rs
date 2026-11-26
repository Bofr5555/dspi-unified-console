//! FIX016 R4 (REVG R4): the speaker layout, whose roles the preset load
//! guard and the measurement floor trust, is not writable in the installer
//! lock's locked-user state, by the user's screens (every client goes
//! through `speaker_layout_set`) nor by an agent's statement of the roles;
//! unlocked with the PIN it is.

use std::sync::Arc;

use dspi_transport::FakeDevice;

use super::*;

const PIN: &str = "~fix016~r4~";

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
        speaker: "L".into(),
        group: 0,
        source: RoleSource::User,
    }
}

fn layout(tweeter: MeasureRole) -> SpeakerLayout {
    SpeakerLayout {
        outputs: vec![row(0, MeasureRole::Low), row(1, tweeter)],
        confirmed_by_user: true,
        ..SpeakerLayout::default()
    }
}

fn lock_code(e: &DspiError) -> &str {
    match e {
        DspiError::InstallerLock { code, .. } => code,
        _ => "",
    }
}

#[test]
fn fix016_r4_the_speaker_layout_is_refused_under_the_lock_and_set_once_unlocked() {
    let core = core_as("FIX016-R4");
    core.speaker_layout_set(layout(MeasureRole::High)).unwrap();
    core.installer_pin_set(PIN.into()).unwrap();
    core.installer_lock_cover(vec!["front".into()]).unwrap();
    assert!(core.installer_lock().unwrap().locked);
    // The reviewer's probe: the tweeter relabelled a woofer while locked
    let e = core
        .speaker_layout_set(layout(MeasureRole::Low))
        .unwrap_err();
    assert_eq!(lock_code(&e), "locked", "{e:?}");
    // Nor an agent's statement of the roles
    let e = core
        .speaker_roles_state(
            vec![row(1, MeasureRole::Low)],
            "out 2 is a woofer".into(),
            "the user rewired".into(),
        )
        .unwrap_err();
    assert_eq!(lock_code(&e), "locked", "{e:?}");
    let kept = core.speaker_layout().unwrap().unwrap();
    assert_eq!(kept.output(1).unwrap().role, MeasureRole::High);
    // Unlocked with the PIN: the installer sets it
    core.installer_unlock(PIN.into()).unwrap();
    let set = core.speaker_layout_set(layout(MeasureRole::Mid)).unwrap();
    assert_eq!(set.output(1).unwrap().role, MeasureRole::Mid);
}
