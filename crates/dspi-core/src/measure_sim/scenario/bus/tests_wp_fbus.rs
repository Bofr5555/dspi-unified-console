//! WP FBUS: the host audio bus, without a room (pure looks).

use serde_json::json;

use super::*;
use crate::measure_sim::scenario::DeviceSpec;

fn device() -> DeviceSpec {
    let drv = |sens: f64| {
        json!({"sensitivityDb": sens, "highpassHz": 40.0, "highpassQ": 0.7, "rolloffHz": 0.0,
               "rolloffQ": 0.7, "breakup": null, "offsetMs": 0.0, "inverted": false})
    };
    serde_json::from_value(json!({
        "serial": "FBUS-FRONTS", "name": "Fronts", "userVolumeDb": -20.0,
        "outputs": [
            {"output": 0, "name": "Left woofer", "role": "low", "speaker": "Left", "group": 0,
             "driver": drv(-13.0), "delayMs": 0.0, "gainDb": 0.0,
             "safe": {"belowHz": 0.0, "belowDbfs": 0.0, "capDbfs": -6.0}},
            {"output": 1, "name": "Left tweeter", "role": "high", "speaker": "Left", "group": 0,
             "driver": drv(-8.5), "delayMs": 0.0, "gainDb": 0.0,
             "safe": {"belowHz": 1000.0, "belowDbfs": -50.0, "capDbfs": -12.0}}
        ]
    }))
    .unwrap()
}

/// Blank, inputs 0 and 1 routed to output 0 (woofer) at 0 dB, user volume -20.
fn state() -> DspState {
    let mut s = crate::state::default_state();
    s.preamp_db = 0.0;
    s.bypass = false;
    s.input_preamp_db.iter_mut().for_each(|v| *v = 0.0);
    s.user_volume_db = -20.0;
    s.master_volume_db = 0.0;
    s.user_mute = false;
    s.loudness_enabled = false;
    s.psybass_enabled = false;
    for b in s.filters.iter_mut().chain(s.xover.iter_mut()).flatten() {
        b.filter_type = 0;
    }
    for r in &mut s.matrix_routing {
        r.iter_mut().for_each(|v| *v = false);
    }
    for r in &mut s.matrix_gain {
        r.iter_mut().for_each(|v| *v = 0.0);
    }
    for r in &mut s.matrix_invert {
        r.iter_mut().for_each(|v| *v = false);
    }
    s.channel_delays.iter_mut().for_each(|v| *v = 0.0);
    s.output_gain_db.iter_mut().for_each(|v| *v = 0.0);
    s.output_delay_ms.iter_mut().for_each(|v| *v = 0.0);
    s.output_muted.iter_mut().for_each(|v| *v = false);
    s.output_enabled.iter_mut().for_each(|v| *v = true);
    s.limiter_enabled.iter_mut().for_each(|v| *v = false);
    s.matrix_routing[0][0] = true;
    s
}

fn spec(stream: serde_json::Value) -> BusSpec {
    serde_json::from_value(json!({
        "os": "linux",
        "outputs": [
            {"name": "DSPi Fronts (USB)", "serial": "Raspberry_Pi_DSPi_FBUS-FRONTS", "channels": 8,
             "default": true, "hardwareVolume": true,
             "map": (0..8).map(|c| json!({"channel": c, "serial": "FBUS-FRONTS", "input": c})).collect::<Vec<_>>()},
            {"name": "Laptop speakers", "channels": 2},
            {"name": "Cinema 5.1", "channels": 6,
             "map": [{"channel": 0, "serial": "FBUS-FRONTS", "input": 0},
                     {"channel": 1, "serial": "FBUS-FRONTS", "input": 1},
                     {"channel": 4, "serial": "FBUS-FRONTS", "input": 4}]}
        ],
        "streams": [stream]
    }))
    .unwrap()
}

fn jazz(output: &str) -> serde_json::Value {
    json!({"id": "t1", "app": "Tidal", "title": "Easy Jazz", "channels": 2, "lufs": -14.0,
           "truePeakDbtp": -1.0, "content": "jazz", "state": "play", "output": output})
}

/// A stereo channel at -14 LUFS: -14 + 0.691 - 10 log10(2).
const CHANNEL_DB: f64 = -14.0 + 0.691 - 3.0103;

fn flat(_: &OutputSpec, _: f64) -> f64 {
    // -13 dB from the amplifier to the microphone, at every frequency
    10f64.powf(-1.3)
}

fn look1(spec: &BusSpec, s: &DspState, generator: u16) -> Look {
    let dev = device();
    let units = [Unit {
        dev: &dev,
        state: s,
        generator,
    }];
    look(spec, &units, 0.0, &flat)
}

#[test]
fn wp_fbus_a_stream_reaches_the_input_and_the_output_at_its_level() {
    let s = state();
    let l = look1(&spec(jazz("DSPi Fronts (USB)")), &s, 0);
    assert!(l.playing());
    assert_eq!(l.streams[0].reaches, ["Fronts in 1", "Fronts in 2"]);
    let i0 = l.inputs.iter().find(|x| x.input == 0).unwrap();
    assert!(
        (i0.sig.rms_db() - CHANNEL_DB).abs() < 0.01,
        "{}",
        i0.sig.rms_db()
    );
    assert!((i0.sig.peak_db() - -1.0).abs() < 0.01);
    assert!(!i0.clip);
    // Through a flat path and -20 dB of user volume, to the woofer only
    let o = &l.outputs;
    assert_eq!(o.len(), 1, "{o:?}");
    assert_eq!(o[0].output, 0);
    assert!((o[0].sig.rms_db() - (CHANNEL_DB - 20.0)).abs() < 0.01);
    assert!((o[0].sig.peak_db() - -21.0).abs() < 0.01);
    // dB SPL at the seat: -13 dB to the microphone, 0 dBFS there is 115 dB SPL
    let spl = o[0].spl_db.unwrap();
    assert!(
        (spl - (CHANNEL_DB - 20.0 - 13.0 + 115.0)).abs() < 0.01,
        "{spl}"
    );
    assert!((l.seat_spl_db.unwrap() - spl).abs() < 1e-9);
    assert!(o[0].below.is_none() && !o[0].clip);
}

#[test]
fn wp_fbus_volumes_and_mutes_on_the_way() {
    let mut s = state();
    // App volume 50 % on Linux: the cube, -18.1 dB
    let mut st = jazz("DSPi Fronts (USB)");
    st["volumePct"] = json!(50.0);
    let l = look1(&spec(st), &s, 0);
    let i0 = l.inputs.iter().find(|x| x.input == 0).unwrap();
    assert!((i0.sig.rms_db() - (CHANNEL_DB - 18.062)).abs() < 0.01);
    // Windows: the scalar, -6 dB
    let mut sp = spec(jazz("DSPi Fronts (USB)"));
    sp.os = "windows".into();
    sp.streams[0].volume_pct = 50.0;
    let i0 = look1(&sp, &s, 0).inputs[0].clone();
    assert!((i0.sig.rms_db() - (CHANNEL_DB - 6.0206)).abs() < 0.01);
    // A hardware-volume output's OS volume does not scale the samples
    let mut sp = spec(jazz("DSPi Fronts (USB)"));
    sp.outputs[0].volume_pct = 10.0;
    assert!((look1(&sp, &s, 0).inputs[0].sig.rms_db() - CHANNEL_DB).abs() < 0.01);
    // ... a software one does
    sp.outputs[0].hardware_volume = false;
    assert!((look1(&sp, &s, 0).inputs[0].sig.rms_db() - (CHANNEL_DB - 60.0)).abs() < 0.01);
    // Paused: nothing anywhere
    let mut st = jazz("DSPi Fronts (USB)");
    st["state"] = json!("pause");
    let l = look1(&spec(st), &s, 0);
    assert!(!l.playing() && l.inputs.is_empty() && l.outputs.is_empty());
    // The output muted, then the user mute, then volume at the bottom
    s.output_muted[0] = true;
    assert!(
        look1(&spec(jazz("DSPi Fronts (USB)")), &s, 0)
            .outputs
            .is_empty()
    );
    s.output_muted[0] = false;
    s.user_mute = true;
    assert!(
        look1(&spec(jazz("DSPi Fronts (USB)")), &s, 0)
            .outputs
            .is_empty()
    );
    s.user_mute = false;
    s.user_volume_db = -60.0;
    let l = look1(&spec(jazz("DSPi Fronts (USB)")), &s, 0);
    assert!(l.outputs.is_empty() && !l.inputs.is_empty());
}

#[test]
fn wp_fbus_wrong_device_channel_maps_and_the_generator() {
    let s = state();
    // Laptop speakers: plays, reaches no DSPi
    let l = look1(&spec(jazz("Laptop speakers")), &s, 0);
    assert!(l.playing() && l.streams[0].reaches.is_empty() && l.inputs.is_empty());
    assert_eq!(l.streams[0].output, "Laptop speakers");
    // Stereo into the 5.1 device: L and R only, the surround stays silent
    let l = look1(&spec(jazz("Cinema 5.1")), &s, 0);
    assert_eq!(l.streams[0].reaches, ["Fronts in 1", "Fronts in 2"]);
    // 5.1 into the 8-channel device: the LFE carries bass only
    let mut st = jazz("DSPi Fronts (USB)");
    st["channels"] = json!(6);
    st["content"] = json!("film");
    st["lufs"] = json!(-27.0);
    let l = look1(&spec(st.clone()), &s, 0);
    let lfe = l.inputs.iter().find(|x| x.input == 3).unwrap();
    assert!(lfe.sig.bands[0] > lfe.sig.bands[5] * 1e6);
    // Full channels: -27 + 0.691 - 10 log10(5)
    let c = l.inputs.iter().find(|x| x.input == 2).unwrap();
    assert!((c.sig.rms_db() - (-27.0 + 0.691 - 6.9897)).abs() < 0.01);
    // 5.1 into stereo: L gets L, half of C and half of Ls
    let mut sp = spec(st);
    sp.outputs[1].map = vec![MapEntry {
        channel: 0,
        serial: "FBUS-FRONTS".into(),
        input: 0,
    }];
    sp.streams[0].output = "Laptop speakers".into();
    let l = look1(&sp, &s, 0);
    let full = -27.0 + 0.691 - 6.9897;
    let want = full + 10.0 * 2f64.log10();
    assert!((l.inputs[0].sig.rms_db() - want).abs() < 0.01);
    // The generator replaces programme on its outputs
    let l = look1(&spec(jazz("DSPi Fronts (USB)")), &s, 0b1);
    assert!(l.outputs[0].generator && l.outputs[0].sig.rms_db() < -150.0);
}

#[test]
fn wp_fbus_clipping_and_the_limiter() {
    let mut s = state();
    s.user_volume_db = 0.0;
    let mut st = jazz("DSPi Fronts (USB)");
    st["truePeakDbtp"] = json!(-0.05);
    st["lufs"] = json!(-8.0);
    st["content"] = json!("edm");
    let l = look1(&spec(st.clone()), &s, 0);
    assert!(l.inputs[0].clip, "a hot master clips the input");
    assert!(l.outputs[0].clip, "and the output at full volume");
    s.limiter_enabled[0] = true;
    s.limiter_threshold_db[0] = -3.0;
    let l = look1(&spec(st), &s, 0);
    let o = &l.outputs[0];
    assert!(
        !o.clip && (o.sig.peak_db() - -3.0).abs() < 0.01 && o.limiter_db > 2.0,
        "{o:?}"
    );
}

#[test]
fn wp_fbus_programme_below_a_tweeters_safe_band() {
    let mut s = state();
    s.matrix_routing[0][1] = true;
    let n_in = usize::from(s.num_input_channels);
    // No high-pass at all: caught at once
    let l = look1(&spec(jazz("DSPi Fronts (USB)")), &s, 0);
    let tw = l.outputs.iter().find(|x| x.output == 1).unwrap();
    let (hz, peak, over) = tw.below.unwrap();
    assert!(hz < 750.0 && peak > -50.0 && over > 0.0, "{:?}", tw.below);
    // An LR4 at 2 kHz: fine
    s.xover[n_in + 1][0].filter_type = 35;
    s.xover[n_in + 1][0].freq = 2000.0;
    let l = look1(&spec(jazz("DSPi Fronts (USB)")), &s, 0);
    assert!(
        l.outputs
            .iter()
            .find(|x| x.output == 1)
            .unwrap()
            .below
            .is_none()
    );
    // 6 dB/octave at 1 kHz: not enough
    s.xover[n_in + 1][0].filter_type = 41;
    s.xover[n_in + 1][0].freq = 1000.0;
    let l = look1(&spec(jazz("DSPi Fronts (USB)")), &s, 0);
    assert!(
        l.outputs
            .iter()
            .find(|x| x.output == 1)
            .unwrap()
            .below
            .is_some()
    );
    // 12 dB/octave at 1 kHz (BW2): the rule's own minimum passes
    s.xover[n_in + 1][0].filter_type = 43;
    let l = look1(&spec(jazz("DSPi Fronts (USB)")), &s, 0);
    assert!(
        l.outputs
            .iter()
            .find(|x| x.output == 1)
            .unwrap()
            .below
            .is_none()
    );
}

#[test]
fn wp_fbus_coupling_events_and_the_host_view() {
    assert_eq!(pct_to_user_db(100.0), 0.0);
    assert_eq!(pct_to_user_db(0.0), -60.0);
    assert!((pct_to_user_db(50.0) - -18.06).abs() < 0.01);
    assert!((user_db_to_pct(-18.0618) - 50.0).abs() < 0.1);
    assert_eq!(user_db_to_pct(-60.0), 0.0);
    let old = spec(jazz("DSPi Fronts (USB)"));
    let mut new = old.clone();
    new.streams[0].state = "pause".into();
    new.streams[0].output = "Laptop speakers".into();
    new.outputs[0].volume_pct = 50.0;
    let what: Vec<String> = changes(&old, &new)
        .iter()
        .map(|e| e["what"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(what, ["pause", "move", "output-volume"]);
    assert_eq!(changes(&BusSpec::default(), &old).len(), 4);
    // The host view: the coupled output shows the DSPi's user volume
    let h = host_of(&old, &|serial| {
        (serial == "FBUS-FRONTS").then_some(-18.0618)
    });
    assert_eq!(h.os, "linux");
    assert_eq!(h.audio_system, "simulated host audio bus");
    let d = &h.outputs[0];
    assert!(d.is_default && d.running && d.hardware_volume && d.channels == 8);
    assert!((d.volume_pct - 50.0).abs() < 0.1);
    assert_eq!(d.serial, "Raspberry_Pi_DSPi_FBUS-FRONTS");
    assert_eq!(d.streams[0].app, "Tidal");
    assert!(!h.outputs[1].running && h.outputs[1].streams.is_empty());
    // The look's line names everything a story checks
    let line = look_json(&look1(&old, &state(), 0));
    assert_eq!(line["outputs"][0]["device"], "Fronts");
    assert!(line["outputs"][0]["splDb"].is_number());
    assert_eq!(line["streams"][0]["reaches"][0], "Fronts in 1");
}

#[test]
fn wp_fbus_envelope_and_content() {
    let mut st: BusStream = serde_json::from_value(jazz("x")).unwrap();
    st.started_unix_s = Some(100.0);
    st.envelope = vec![
        EnvPoint {
            at_s: 0.0,
            lufs: -20.0,
            true_peak_dbtp: None,
        },
        EnvPoint {
            at_s: 10.0,
            lufs: -10.0,
            true_peak_dbtp: Some(-0.5),
        },
    ];
    let at = |t: f64| stream_channels(&st, t)[0].rms_db();
    assert!((at(105.0) - (-20.0 + 0.691 - 3.0103)).abs() < 0.01);
    assert!((at(111.0) - (-10.0 + 0.691 - 3.0103)).abs() < 0.01);
    assert!((stream_channels(&st, 111.0)[0].peak_db() - -0.5).abs() < 0.01);
    // A tone sits in its band
    st.content = "tone:1000".into();
    st.started_unix_s = None;
    let c = stream_channels(&st, 0.0);
    assert!(c[0].bands[5] > 0.99 * c[0].bands.iter().sum::<f64>());
    // Speech has next to no deep bass
    st.content = "speech".into();
    let c = stream_channels(&st, 0.0);
    assert!(c[0].bands[0] < c[0].bands[4] * 1e-3);
}
