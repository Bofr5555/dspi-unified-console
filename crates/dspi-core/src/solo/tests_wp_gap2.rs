//! WP GAP2: solo one output, a listening state the mirror and the saves
//! never see.

use std::sync::Arc;
use std::time::Duration;

use dspi_transport::fake::{FakeDevice, FakeRemote};

use crate::records::DspState;
use crate::records::NotifyResult;
use crate::{DspiCore, DspiError, state};

fn demo() -> (Arc<DspiCore>, FakeRemote) {
    let dev = FakeDevice::demo();
    let remote = dev.remote();
    let core = DspiCore::new();
    core.attach_probed(Box::new(dev), "DEMO".into(), 0).unwrap();
    settle(&core);
    (core, remote)
}

/// Apply the device's notifications (the solo's echoes) until quiet.
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

/// The device's output mutes, from its image.
fn device_mutes(core: &DspiCore, remote: &FakeRemote) -> Vec<bool> {
    let mut s: DspState = core.lock().state.clone();
    state::decode_bulk(&remote.image(), &mut s).unwrap();
    s.output_muted
}

fn outputs(core: &DspiCore) -> usize {
    usize::from(core.get_state().num_output_channels)
}

/// Only `o` unmuted, of `n`.
fn only(o: usize, n: usize) -> Vec<bool> {
    (0..n).map(|i| i != o).collect()
}

#[test]
fn gap2_solo_mutes_the_others_on_the_device_only() {
    let (core, remote) = demo();
    let n = outputs(&core);
    assert_eq!(core.solo_state(), None);
    core.solo_output(2).unwrap();
    assert_eq!(core.solo_state(), Some(2));
    assert_eq!(device_mutes(&core, &remote)[..n], only(2, n)[..]);
    // The solo's own echoes come back: the mirror keeps the user's mutes
    settle(&core);
    assert!(core.get_state().output_muted[..n].iter().all(|m| !m));
    assert!(!core.preset_dirty(), "{:?}", core.preset_changes());
    // A re-read too
    core.refresh_params().unwrap();
    assert!(core.get_state().output_muted[..n].iter().all(|m| !m));
    core.fetch_all().unwrap();
    assert!(core.get_state().output_muted[..n].iter().all(|m| !m));
    assert_eq!(core.solo_state(), Some(2));

    core.solo_end().unwrap();
    assert_eq!(core.solo_state(), None);
    assert!(device_mutes(&core, &remote)[..n].iter().all(|m| !m));
    // Ending twice, or with no solo, is nothing
    core.solo_end().unwrap();
}

#[test]
fn gap2_solo_never_unmutes_and_moves() {
    let (core, remote) = demo();
    let n = outputs(&core);
    core.set_output_mute(4, true).unwrap();
    core.set_output_mute(1, true).unwrap();
    core.solo_output(1).unwrap();
    // Out 2 (index 1) is the user's muted one: the solo leaves it muted
    assert!(device_mutes(&core, &remote)[..n].iter().all(|&m| m));
    core.solo_output(3).unwrap();
    assert_eq!(core.solo_state(), Some(3));
    assert_eq!(device_mutes(&core, &remote)[..n], only(3, n)[..]);
    core.solo_output(3).unwrap();
    core.solo_end().unwrap();
    let back = device_mutes(&core, &remote);
    assert!(back[1] && back[4], "the user's mutes stay: {back:?}");
    assert_eq!(back.iter().take(n).filter(|&&m| m).count(), 2, "{back:?}");
    assert_eq!(core.get_state().output_muted, back);
}

#[test]
fn gap2_a_mute_change_while_soloed_waits_for_the_end() {
    let (core, remote) = demo();
    let n = outputs(&core);
    core.set_output_mute(5, true).unwrap();
    core.solo_output(0).unwrap();
    // The user unmutes Out 6 and mutes Out 2: the mirror takes both...
    core.set_output_mute(5, false).unwrap();
    core.set_output_mute(1, true).unwrap();
    let s = core.get_state();
    assert!(!s.output_muted[5] && s.output_muted[1]);
    // ...the device stays soloed
    assert_eq!(device_mutes(&core, &remote)[..n], only(0, n)[..]);
    // Undo is the user's as ever: Out 2 unmuted again in the mirror
    core.undo().unwrap();
    assert!(!core.get_state().output_muted[1]);
    assert_eq!(device_mutes(&core, &remote)[..n], only(0, n)[..]);
    core.set_output_mute(1, true).unwrap();
    // The soloed output's own mute goes to the device
    core.set_output_mute(0, true).unwrap();
    assert!(device_mutes(&core, &remote)[0]);
    core.set_output_mute(0, false).unwrap();
    core.solo_end().unwrap();
    let back = device_mutes(&core, &remote);
    assert!(!back[5], "unmuted while soloed: plays now");
    assert!(back[1], "muted while soloed: stays muted");
    assert_eq!(core.get_state().output_muted, back);
}

#[test]
fn gap2_a_save_puts_the_mutes_back_first() {
    let (core, remote) = demo();
    let n = outputs(&core);
    core.solo_output(2).unwrap();
    core.save_params_tok().unwrap();
    assert_eq!(core.solo_state(), None, "saving ends the solo");
    assert!(device_mutes(&core, &remote)[..n].iter().all(|m| !m));
    // What was saved has no solo in it
    core.solo_output(2).unwrap();
    let slot = core.get_state().active_preset_slot;
    core.load_preset(slot).unwrap();
    assert_eq!(core.solo_state(), None, "a load ends the solo");
    assert!(device_mutes(&core, &remote)[..n].iter().all(|m| !m));
    assert!(core.get_state().output_muted[..n].iter().all(|m| !m));

    core.solo_output(1).unwrap();
    core.save_preset_tok(3).unwrap();
    assert_eq!(core.solo_state(), None);
    core.load_preset(3).unwrap();
    assert!(device_mutes(&core, &remote)[..n].iter().all(|m| !m));
}

#[test]
fn gap2_a_load_shows_the_presets_mutes_not_the_solos() {
    let (core, remote) = demo();
    let n = outputs(&core);
    core.set_output_mute(3, true).unwrap();
    core.save_preset_tok(2).unwrap();
    core.set_output_mute(3, false).unwrap();
    core.solo_output(0).unwrap();
    core.load_preset(2).unwrap();
    assert_eq!(core.solo_state(), None);
    let mut want = vec![false; n];
    want[3] = true;
    assert_eq!(device_mutes(&core, &remote)[..n], want[..]);
    assert_eq!(core.get_state().output_muted[..n], want[..]);
}

#[test]
fn gap2_a_whole_image_ends_the_solo() {
    let (core, remote) = demo();
    let n = outputs(&core);
    core.solo_output(4).unwrap();
    // Our own bulk write (a recall, A/B, an import) carries the user's mutes
    core.apply_all_params().unwrap();
    assert_eq!(core.solo_state(), None);
    assert!(device_mutes(&core, &remote)[..n].iter().all(|m| !m));

    // Another writer's image: the solo is over, its mutes are the device's
    core.solo_output(4).unwrap();
    let mut s = core.get_state();
    s.output_muted[7] = true;
    remote.load_image(state::encode_bulk(&s), 0);
    settle(&core);
    assert_eq!(core.solo_state(), None);
    assert!(core.get_state().output_muted[7]);
}

#[test]
fn gap2_disconnect_puts_the_mutes_back() {
    let (core, remote) = demo();
    let n = outputs(&core);
    core.solo_output(1).unwrap();
    core.disconnect();
    let mut s = state::default_state();
    state::decode_bulk(&remote.image(), &mut s).unwrap();
    assert!(
        s.output_muted[..n].iter().all(|m| !m),
        "{:?}",
        s.output_muted
    );
    assert_eq!(core.solo_state(), None);
}

#[test]
fn gap2_refusals() {
    let (core, remote) = demo();
    let n = core.get_state().num_output_channels;
    assert!(matches!(
        core.solo_output(n),
        Err(DspiError::InvalidArgument { .. })
    ));
    assert_eq!(core.solo_state(), None);
    assert!(device_mutes(&core, &remote).iter().all(|m| !m));
    let lone = DspiCore::new();
    assert!(lone.solo_output(0).is_err());
    lone.solo_end().unwrap();
    assert_eq!(lone.solo_state(), None);
}

#[test]
fn gap2_a_load_of_an_empty_slot_still_ends_the_solo() {
    let (core, remote) = demo();
    let n = outputs(&core);
    core.solo_output(0).unwrap();
    // An empty slot changes nothing on the device: the solo's mutes must
    // not stay behind as if they were the user's
    let _ = core.load_preset(9);
    assert_eq!(core.solo_state(), None);
    assert!(device_mutes(&core, &remote)[..n].iter().all(|m| !m));
    assert!(core.get_state().output_muted[..n].iter().all(|m| !m));
}

#[test]
fn gap2_a_copy_to_a_slot_puts_the_mutes_back_first() {
    let (core, remote) = demo();
    let n = outputs(&core);
    let active = core.get_state().active_preset_slot;
    core.solo_output(2).unwrap();
    core.copy_preset_tok(active, 4).unwrap();
    assert_eq!(core.solo_state(), None);
    core.load_preset(4).unwrap();
    assert!(device_mutes(&core, &remote)[..n].iter().all(|m| !m));
}
