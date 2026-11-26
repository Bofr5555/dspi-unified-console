//! FIX016 F3 (REVG F3, reviewer A's A3): a solo's mutes left on the device
//! by a connection lost without the solo's end are lifted on the next
//! connect of that serial, before the baseline is taken: never the user's
//! mutes, never unsaved, never saved.

use std::sync::Arc;
use std::time::{Duration, Instant};

use dspi_transport::fake::{FakeDevice, FakeRemote};

use crate::confirm::ConfirmOp;
use crate::records::{DspState, NotifyResult};
use crate::{DspiCore, state};

const SERIAL: &str = "FIX016-F3";

fn settle(core: &DspiCore) {
    let mut quiet: u32 = 0;
    while quiet < 10 {
        std::thread::sleep(Duration::from_millis(10));
        if core.process_notifications() == NotifyResult::default() {
            quiet = quiet.saturating_add(1);
        } else {
            quiet = 0;
        }
    }
}

fn open(core: &DspiCore, dev: FakeDevice) -> FakeRemote {
    let remote = dev.remote();
    core.attach_probed(Box::new(dev), SERIAL.into(), 0).unwrap();
    core.fetch_all().unwrap();
    settle(core);
    remote
}

/// The device's output mutes, from its image.
fn device_mutes(core: &DspiCore, remote: &FakeRemote) -> Vec<bool> {
    let mut s: DspState = core.lock().state.clone();
    state::decode_bulk(&remote.image(), &mut s).unwrap();
    s.output_muted
}

#[test]
fn fix016_f3_a_lost_solos_mutes_are_lifted_on_the_next_connect() {
    let core: Arc<DspiCore> = DspiCore::new();
    let remote = open(&core, FakeDevice::demo());
    let n = usize::from(core.get_state().num_output_channels);
    assert!(core.get_state().output_muted[..n].iter().all(|m| !m));
    core.solo_output(1).unwrap();
    let held = device_mutes(&core, &remote);
    assert_eq!(held[..n].iter().filter(|m| **m).count(), n - 1, "{held:?}");

    // The connection goes without a close: the device stays as the solo left it
    remote.disconnect();
    let end = Instant::now() + Duration::from_secs(10);
    while !crate::notify::stream_ended_locked(&core.lock()) {
        assert!(Instant::now() < end, "the reader never saw the end");
        std::thread::sleep(Duration::from_millis(10));
    }
    core.poll_with(Vec::new, |_: &str| Ok(()));
    assert!(!core.is_connected());

    // The same DSPi again, as it plays: the solo's mutes in its live image
    let again = open(&core, FakeDevice::demo_running(remote.image()));
    assert!(
        device_mutes(&core, &again)[..n].iter().all(|m| !m),
        "the device plays every output again"
    );
    assert!(
        core.get_state().output_muted[..n].iter().all(|m| !m),
        "{:?}",
        core.get_state().output_muted
    );
    let changes = core.preset_changes();
    assert!(
        !changes.iter().any(|c| c.to_lowercase().contains("mute")),
        "{changes:?}"
    );
    // The baseline has none of them
    let mut base = core.lock().state.clone();
    state::decode_bulk(core.lock().baseline.as_deref().unwrap(), &mut base).unwrap();
    assert!(base.output_muted[..n].iter().all(|m| !m), "{base:?}");
    // And the next save writes none of them
    let token = core.confirm_token(ConfirmOp::SaveParams);
    core.save_params_confirmed(Some(token)).unwrap();
    core.load_params().unwrap();
    assert!(core.get_state().output_muted[..n].iter().all(|m| !m));
    assert!(core.lock().lost_solo.is_empty());
}
