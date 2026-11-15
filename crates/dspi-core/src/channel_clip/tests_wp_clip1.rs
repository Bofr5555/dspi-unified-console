//! WP CLIP1: the channel clipboard in the core, on the demo device. Limits
//! and PIN files live under `std::env::temp_dir()`, which `ci/run.sh` points
//! at the home volume (never tmpfs).

use std::path::PathBuf;
use std::sync::Arc;

use dspi_session::limits::{LimitsProfile, OutputLimits};

use super::*;

fn demo() -> Arc<DspiCore> {
    let core = DspiCore::new();
    core.open_demo().unwrap();
    core
}

fn dir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join("wp-clip1-core").join(format!(
        "{name}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&d);
    d
}

fn band(filter_type: u8, freq: f32, q: f32, gain: f32) -> FilterParams {
    FilterParams {
        filter_type,
        bypass: false,
        freq,
        q,
        gain,
        qp: 0.707,
    }
}

/// The wire channel of output `o`.
fn out_ch(core: &DspiCore, o: u8) -> u8 {
    core.get_state().num_input_channels + o
}

/// Output `o` made distinctive: three PEQ bands, an LR4 high-pass in its
/// first crossover slot, gain -7 dB, delay 1.5 ms, muted.
fn distinctive_output(core: &DspiCore, o: u8) {
    let ch = out_ch(core, o);
    core.set_filter(ch, 0, band(1, 1234.0, 2.0, -4.5)).unwrap();
    core.set_filter(ch, 3, band(1, 80.0, 0.9, 3.0)).unwrap();
    let mut by = band(1, 5000.0, 1.0, -2.0);
    by.bypass = true;
    core.set_filter(ch, 9, by).unwrap();
    core.set_crossover(ch, 0, band(35, 120.0, 0.707, 0.0))
        .unwrap();
    core.set_output_gain(o, -7.0).unwrap();
    core.set_output_delay(o, 1.5).unwrap();
    core.set_output_mute(o, true).unwrap();
}

/// What a channel carries, as the clip sees it.
fn bank(core: &DspiCore, ch: u8) -> Vec<FilterParams> {
    core.get_state().filters[usize::from(ch)][..BANDS_PER_CHANNEL].to_vec()
}

#[test]
fn wp_clip1_output_to_output_takes_peq_crossover_gain_delay_and_mute() {
    let core = demo();
    distinctive_output(&core, 0);
    let (src, dst) = (out_ch(&core, 0), out_ch(&core, 1));
    let clip = core.copy_channel(src).unwrap();
    assert_eq!(clip.version, CHANNEL_CLIP_VERSION);
    assert_eq!(clip.source_channel, src);
    assert_eq!(clip.filters.len(), BANDS_PER_CHANNEL);
    assert_eq!(clip.crossover.as_ref().unwrap().len(), MAX_XOVER_BANDS);
    let o = clip.output.unwrap();
    assert_eq!((o.gain_db, o.delay_ms, o.muted), (-7.0, 1.5, true));

    core.paste_channel(clip, dst, ChannelPasteOptions::default())
        .unwrap();
    let s = core.get_state();
    assert_eq!(bank(&core, dst), bank(&core, src));
    assert!(s.filters[usize::from(dst)][9].bypass, "bypass travels");
    assert_eq!(s.xover[usize::from(dst)], s.xover[usize::from(src)]);
    assert_eq!(s.output_gain_db[1], -7.0);
    assert_eq!(s.output_delay_ms[1], 1.5);
    assert!(s.output_muted[1]);
}

#[test]
fn wp_clip1_input_to_input_takes_the_peq_only() {
    let core = demo();
    core.set_filter(0, 2, band(1, 300.0, 1.2, 4.0)).unwrap();
    core.set_input_preamp(0, -6.0).unwrap();
    // Something on input 1 the paste must clear.
    core.set_filter(1, 7, band(1, 9000.0, 1.0, -3.0)).unwrap();
    let before = core.get_state();
    let clip = core.copy_channel(0).unwrap();
    assert!(clip.crossover.is_none() && clip.output.is_none());

    core.paste_channel(clip, 1, ChannelPasteOptions::default())
        .unwrap();
    let s = core.get_state();
    assert_eq!(bank(&core, 1), bank(&core, 0));
    assert_eq!(
        s.filters[1][7].filter_type, 0,
        "bands past the clip go flat"
    );
    assert_eq!(
        s.input_preamp_db, before.input_preamp_db,
        "the trim does not travel"
    );
    assert_eq!(s.output_gain_db, before.output_gain_db);
}

/// Between an input and an output only the PEQ crosses: an output keeps its
/// crossover bank and its strip when the clip came from an input, and an
/// input takes only an output clip's PEQ.
#[test]
fn wp_clip1_input_and_output_exchange_only_the_peq() {
    let core = demo();
    distinctive_output(&core, 0);
    let out0 = out_ch(&core, 0);
    core.set_filter(0, 1, band(1, 500.0, 1.0, 2.0)).unwrap();
    let before = core.get_state();

    core.paste_channel(core.copy_channel(0).unwrap(), out0, Default::default())
        .unwrap();
    let s = core.get_state();
    assert_eq!(bank(&core, out0), bank(&core, 0));
    assert_eq!(s.xover[usize::from(out0)], before.xover[usize::from(out0)]);
    assert_eq!((s.output_gain_db[0], s.output_delay_ms[0]), (-7.0, 1.5));
    assert!(s.output_muted[0]);

    let out1 = out_ch(&core, 1);
    core.set_filter(out1, 4, band(1, 2000.0, 3.0, 5.0)).unwrap();
    core.set_output_gain(1, -3.0).unwrap();
    let before = core.get_state();
    core.paste_channel(core.copy_channel(out1).unwrap(), 2, Default::default())
        .unwrap();
    let s = core.get_state();
    assert_eq!(bank(&core, 2), bank(&core, out1));
    assert_eq!(s.input_preamp_db, before.input_preamp_db);
    assert_eq!(s.output_gain_db, before.output_gain_db);
    assert_eq!(s.xover, before.xover);
}

#[test]
fn wp_clip1_a_linked_partner_gets_the_same_peq_and_nothing_else() {
    let core = demo();
    distinctive_output(&core, 0);
    let (src, dst, partner) = (out_ch(&core, 0), out_ch(&core, 2), out_ch(&core, 3));
    let before = core.get_state();
    let clip = core.copy_channel(src).unwrap();
    core.paste_channel(
        clip.clone(),
        dst,
        ChannelPasteOptions {
            mirror: Some(partner),
        },
    )
    .unwrap();
    let s = core.get_state();
    assert_eq!(bank(&core, dst), bank(&core, src));
    assert_eq!(bank(&core, partner), bank(&core, src));
    assert_eq!(s.xover[usize::from(dst)], s.xover[usize::from(src)]);
    assert_eq!(
        s.xover[usize::from(partner)],
        before.xover[usize::from(partner)],
        "the crossover goes to the destination alone"
    );
    assert_eq!(s.output_gain_db[3], before.output_gain_db[3]);
    assert_eq!(s.output_muted[3], before.output_muted[3]);

    // An input pair.
    core.paste_channel(clip.clone(), 0, ChannelPasteOptions { mirror: Some(1) })
        .unwrap();
    assert_eq!(bank(&core, 0), bank(&core, src));
    assert_eq!(bank(&core, 1), bank(&core, src));

    // A partner that is the channel itself, or out of range, is refused.
    for m in [dst, 200] {
        let e = core
            .paste_channel(clip.clone(), dst, ChannelPasteOptions { mirror: Some(m) })
            .unwrap_err();
        assert!(matches!(e, DspiError::InvalidArgument { .. }), "{e:?}");
    }
}

#[test]
fn wp_clip1_one_undo_restores_everything_and_one_redo_pastes_again() {
    let core = demo();
    distinctive_output(&core, 0);
    let (src, dst, partner) = (out_ch(&core, 0), out_ch(&core, 1), out_ch(&core, 3));
    // The destination different from the source in every part.
    core.set_filter(dst, 5, band(1, 700.0, 1.0, 6.0)).unwrap();
    core.set_crossover(dst, 1, band(34, 3000.0, 0.707, 0.0))
        .unwrap();
    core.set_output_gain(1, 2.0).unwrap();
    core.set_output_delay(1, 0.25).unwrap();
    let before = core.get_state();
    let steps = core.history(1000).len();

    let clip = core.copy_channel(src).unwrap();
    core.paste_channel(
        clip,
        dst,
        ChannelPasteOptions {
            mirror: Some(partner),
        },
    )
    .unwrap();
    let h = core.history(1000);
    assert_eq!(h.len(), steps + 1, "one step: {:?}", h.first());
    assert_eq!(h[0].label.as_deref(), Some(PASTE_LABEL));
    let pasted = core.get_state();
    assert_ne!(pasted.filters, before.filters);

    let m = core.undo().unwrap();
    assert_eq!(m.entry.unwrap().label.as_deref(), Some(PASTE_LABEL));
    let s = core.get_state();
    assert_eq!(s.filters, before.filters);
    assert_eq!(s.xover, before.xover);
    assert_eq!(s.output_gain_db, before.output_gain_db);
    assert_eq!(s.output_delay_ms, before.output_delay_ms);
    assert_eq!(s.output_muted, before.output_muted);

    core.redo().unwrap();
    let s = core.get_state();
    assert_eq!(s.filters, pasted.filters);
    assert_eq!(s.xover, pasted.xover);
    assert_eq!(s.output_gain_db, pasted.output_gain_db);
    assert_eq!(s.output_delay_ms, pasted.output_delay_ms);
    assert_eq!(s.output_muted, pasted.output_muted);
}

/// The parts a refused paste must leave alone, and the history length.
fn untouched(core: &DspiCore, before: &DspState, steps: usize) {
    let s = core.get_state();
    assert_eq!(s.filters, before.filters);
    assert_eq!(s.xover, before.xover);
    assert_eq!(s.output_gain_db, before.output_gain_db);
    assert_eq!(s.output_delay_ms, before.output_delay_ms);
    assert_eq!(s.output_muted, before.output_muted);
    assert_eq!(core.history(1000).len(), steps, "nothing journalled");
}

#[test]
fn wp_clip1_refused_in_read_only_mode_with_nothing_written() {
    let core = demo();
    distinctive_output(&core, 0);
    let clip = core.copy_channel(out_ch(&core, 0)).unwrap();
    let before = core.get_state();
    let steps = core.history(1000).len();
    core.set_client_read_only(true).unwrap();
    // Copying is a read: it goes on.
    assert_eq!(core.copy_channel(out_ch(&core, 0)).unwrap(), clip);
    let e = core
        .paste_channel(clip, out_ch(&core, 1), Default::default())
        .unwrap_err();
    assert!(matches!(e, DspiError::ReadOnly), "{e:?}");
    untouched(&core, &before, steps);
}

#[test]
fn wp_clip1_refused_under_the_installer_lock_with_nothing_written() {
    let core = demo();
    distinctive_output(&core, 0);
    let clip = core.copy_channel(out_ch(&core, 0)).unwrap();
    core.limits_set_dir(dir("lock").to_string_lossy().into())
        .unwrap();
    core.installer_pin_set("2468".into()).unwrap();
    core.installer_lock_cover(vec!["front".into()]).unwrap();
    assert!(core.installer_lock().unwrap().locked);
    let before = core.get_state();
    let steps = core.history(1000).len();
    // The mute alone would pass the lock; the paste as a whole does not.
    let e = core
        .paste_channel(clip, out_ch(&core, 1), Default::default())
        .unwrap_err();
    assert!(matches!(e, DspiError::InstallerLock { .. }), "{e:?}");
    untouched(&core, &before, steps);
}

#[test]
fn wp_clip1_a_limit_anywhere_in_the_paste_refuses_it_with_nothing_written() {
    let core = demo();
    core.limits_set_dir(dir("limit").to_string_lossy().into())
        .unwrap();
    distinctive_output(&core, 0);
    core.set_output_gain(0, 6.0).unwrap();
    let mut clip = core.copy_channel(out_ch(&core, 0)).unwrap();
    let gain_only = LimitsProfile {
        outputs: vec![OutputLimits {
            output: 1,
            max_gain_db: Some(3.0),
            ..Default::default()
        }],
        max_peq_boost_db: Some(6.0),
        ..Default::default()
    };
    core.limits_apply_profile(gain_only.to_document()).unwrap();
    let before = core.get_state();
    let steps = core.history(1000).len();
    // The gain, last but two in the paste, is past the limit.
    let e = core
        .paste_channel(clip.clone(), out_ch(&core, 1), Default::default())
        .unwrap_err();
    assert!(
        matches!(&e, DspiError::LimitExceeded { code, .. } if code == "out_gain"),
        "{e:?}"
    );
    untouched(&core, &before, steps);
    // A boost past the limit, on an input, likewise.
    clip.output = None;
    clip.filters[6] = band(1, 1000.0, 1.0, 9.0);
    let e = core.paste_channel(clip, 1, Default::default()).unwrap_err();
    assert!(
        matches!(&e, DspiError::LimitExceeded { code, .. } if code == "eq_boost"),
        "{e:?}"
    );
    untouched(&core, &before, steps);
}

/// An output the profile holds to a high-pass keeps one throughout: the
/// destination's high-pass in crossover slot 0, the clip's in slot 1 goes
/// in first, so clearing slot 0 is never judged a lost high-pass.
#[test]
fn wp_clip1_a_high_pass_moving_slots_pastes_under_a_profile() {
    let core = demo();
    core.limits_set_dir(dir("highpass").to_string_lossy().into())
        .unwrap();
    let (src, dst) = (out_ch(&core, 0), out_ch(&core, 1));
    core.set_crossover(src, 1, band(35, 2000.0, 0.707, 0.0))
        .unwrap();
    core.set_crossover(dst, 0, band(35, 2000.0, 0.707, 0.0))
        .unwrap();
    let hp = LimitsProfile {
        outputs: vec![OutputLimits {
            output: 1,
            highpass_min_hz: Some(1500.0),
            highpass_min_slope_db: Some(24),
            ..Default::default()
        }],
        ..Default::default()
    };
    core.limits_apply_profile(hp.to_document()).unwrap();
    let clip = core.copy_channel(src).unwrap();
    core.paste_channel(clip.clone(), dst, Default::default())
        .unwrap();
    let s = core.get_state();
    assert_eq!(s.xover[usize::from(dst)], s.xover[usize::from(src)]);
    assert_eq!(s.xover[usize::from(dst)][1].filter_type, 35);

    // A clip without a high-pass is refused before anything is written.
    let mut bare = clip;
    bare.crossover = Some(vec![band(0, 1000.0, 0.707, 0.0); MAX_XOVER_BANDS]);
    let before = core.get_state();
    let steps = core.history(1000).len();
    let e = core
        .paste_channel(bare, dst, Default::default())
        .unwrap_err();
    assert!(
        matches!(&e, DspiError::LimitExceeded { code, .. } if code == "highpass"),
        "{e:?}"
    );
    untouched(&core, &before, steps);
}

#[test]
fn wp_clip1_the_delay_is_clamped_to_this_devices_line() {
    let core = demo();
    let max = core.max_delay_ms();
    let mut clip = core.copy_channel(out_ch(&core, 0)).unwrap();
    clip.output = Some(ChannelClipOutput {
        gain_db: 0.0,
        delay_ms: max + 40.0,
        muted: false,
    });
    core.paste_channel(clip, out_ch(&core, 1), Default::default())
        .unwrap();
    assert_eq!(core.get_state().output_delay_ms[1], max);
}

#[test]
fn wp_clip1_a_clip_round_trips_through_its_document() {
    let core = demo();
    distinctive_output(&core, 0);
    core.set_filter(0, 0, {
        let mut lt = band(11, 40.0, 0.5, 25.0);
        lt.qp = 0.6;
        lt
    })
    .unwrap();
    for ch in [0, out_ch(&core, 0)] {
        let mut clip = core.copy_channel(ch).unwrap();
        clip.source_name = "Sub \"left\"".into();
        let text = channel_clip_to_json(clip.clone()).unwrap();
        let v: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(v["kind"], CHANNEL_CLIP_KIND);
        assert_eq!(v["version"], CHANNEL_CLIP_VERSION);
        assert_eq!(channel_clip_from_json(text).unwrap(), clip);
    }
    // A kept document pastes on another core (another session).
    let text = channel_clip_to_json(core.copy_channel(out_ch(&core, 0)).unwrap()).unwrap();
    let other = demo();
    other
        .paste_channel(
            channel_clip_from_json(text).unwrap(),
            out_ch(&other, 1),
            Default::default(),
        )
        .unwrap();
    assert_eq!(
        bank(&other, out_ch(&other, 1)),
        bank(&core, out_ch(&core, 0))
    );
    // A NaN has no JSON: refused rather than written as null.
    let mut nan = core.copy_channel(0).unwrap();
    nan.filters[0].freq = f32::NAN;
    assert!(channel_clip_to_json(nan).is_err());
}

#[test]
fn wp_clip1_an_old_or_unknown_version_is_refused() {
    let core = demo();
    let clip = core.copy_channel(out_ch(&core, 0)).unwrap();
    let text = channel_clip_to_json(clip.clone()).unwrap();
    for version in [0u64, 2, 99, u64::from(u32::MAX) + 1] {
        let mut v: serde_json::Value = serde_json::from_str(&text).unwrap();
        v["version"] = version.into();
        let e = channel_clip_from_json(v.to_string()).unwrap_err();
        assert!(
            matches!(&e, DspiError::InvalidArgument { msg } if msg.contains("version")),
            "{version}: {e:?}"
        );
    }
    for bad in [
        String::new(),
        "{}".to_owned(),
        r#"{"kind":"dspi.preset","version":1}"#.to_owned(),
        r#"{"kind":"dspi.channelClip"}"#.to_owned(),
        r#"{"kind":"dspi.channelClip","version":1}"#.to_owned(),
        "x".repeat(MAX_DOCUMENT_LEN + 1),
    ] {
        assert!(channel_clip_from_json(bad.clone()).is_err(), "{bad:.40}");
    }
    // A record of another version is refused by the paste, nothing written.
    let before = core.get_state();
    let steps = core.history(1000).len();
    for version in [0, 2] {
        let mut old = clip.clone();
        old.version = version;
        let e = core
            .paste_channel(old.clone(), out_ch(&core, 1), Default::default())
            .unwrap_err();
        assert!(matches!(e, DspiError::InvalidArgument { .. }), "{e:?}");
        assert!(channel_clip_to_json(old).is_err());
    }
    untouched(&core, &before, steps);
}

#[test]
fn wp_clip1_bad_channels_and_values_are_refused_up_front() {
    let core = demo();
    let n = core.get_state().num_channels;
    assert!(matches!(
        core.copy_channel(n),
        Err(DspiError::InvalidArgument { .. })
    ));
    assert!(matches!(
        DspiCore::new().copy_channel(0),
        Err(DspiError::Disconnected)
    ));
    let clip = core.copy_channel(out_ch(&core, 0)).unwrap();
    assert!(matches!(
        DspiCore::new().paste_channel(clip.clone(), 0, Default::default()),
        Err(DspiError::Disconnected)
    ));
    let before = core.get_state();
    let steps = core.history(1000).len();
    let mut bad = clip.clone();
    bad.crossover.as_mut().unwrap()[3] = band(1, 1000.0, 1.0, 0.0);
    let mut nan = clip.clone();
    nan.filters[9].gain = f32::NAN;
    let mut inf = clip;
    inf.output.as_mut().unwrap().delay_ms = f32::INFINITY;
    for (what, c, ch) in [
        ("channel", nan.clone(), n),
        ("xover type", bad, out_ch(&core, 1)),
        ("nan", nan, 0),
        ("inf", inf, out_ch(&core, 1)),
    ] {
        let e = core.paste_channel(c, ch, Default::default()).unwrap_err();
        assert!(
            matches!(e, DspiError::InvalidArgument { .. }),
            "{what}: {e:?}"
        );
    }
    untouched(&core, &before, steps);
}
