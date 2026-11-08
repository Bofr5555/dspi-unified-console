//! WP CG31: another client's write reaches this client's state, for every
//! per-channel field a `PARAM_CHANGED` can patch. Each case pokes the bytes
//! the firmware notifies for that SET (vendor_commands.c, main.c), as a
//! second host would make it, and reads the field the clients show.

use std::sync::Arc;
use std::time::{Duration, Instant};

use dspi_proto::generated::{self as g, notify as nfy};
use dspi_transport::{FakeDevice, FakeRemote};

use super::*;

const WAIT: Duration = Duration::from_secs(8);
/// The hw-win repro wrote over USB MIDI: not this USB client's own source.
const MIDI: u8 = nfy::PARAM_SRC_MIDI as u8;

fn demo() -> (Arc<DspiCore>, FakeRemote) {
    let dev = FakeDevice::demo();
    let remote = dev.remote();
    let core = DspiCore::new();
    core.attach_probed(Box::new(dev), "DEMO".into(), 0).unwrap();
    core.process_notifications();
    (core, remote)
}

fn wait_for(core: &DspiCore, what: &str, mut f: impl FnMut(&crate::DspState) -> bool) {
    let start = Instant::now();
    loop {
        core.process_notifications();
        if f(&core.get_state()) {
            return;
        }
        assert!(start.elapsed() < WAIT, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn f32le(v: f32) -> [u8; 4] {
    v.to_le_bytes()
}

/// `REQ_SET_DELAY` on an output's wire channel (`dspi --midi set ch.delay
/// i2s.1.l 1`, the hw-win F1 repro) notifies only `delays[]`: the output's
/// delay, which the clients show, follows it.
#[test]
fn wp_cg31_a_channel_delay_from_another_client_moves_the_output_delay() {
    let (core, remote) = demo();
    let inputs = usize::from(core.get_state().num_input_channels);
    for o in [0usize, 3] {
        let ch = inputs + o;
        let ms = 1.0 + o as f32;
        assert!(remote.poke(g::OFF_DELAYS + ch * 4, &f32le(ms), MIDI));
        wait_for(&core, "the channel delay", |s| s.channel_delays[ch] == ms);
        let s = core.get_state();
        assert_eq!(s.output_delay_ms[o], ms, "output {o}");
        assert_eq!(
            &s.bulk_raw[g::OFF_OUTPUTS + o * 12 + 8..][..4],
            &f32le(ms),
            "the image's twin, so a later bulk write sends one value"
        );
    }
    // An input channel's delay has no twin.
    let before = core.get_state().output_delay_ms;
    assert!(remote.poke(g::OFF_DELAYS, &f32le(4.0), MIDI));
    wait_for(&core, "the input delay", |s| s.channel_delays[0] == 4.0);
    assert_eq!(core.get_state().output_delay_ms, before);
}

/// The other direction: an output-delay patch alone moves the channel delay
/// (the firmware sends both; either order ends in one value).
#[test]
fn wp_cg31_an_output_delay_patch_moves_the_channel_delay() {
    let (core, remote) = demo();
    let inputs = usize::from(core.get_state().num_input_channels);
    assert!(remote.poke(g::OFF_OUTPUTS + 2 * 12 + 8, &f32le(2.5), MIDI));
    wait_for(&core, "the output delay", |s| s.output_delay_ms[2] == 2.5);
    assert_eq!(core.get_state().channel_delays[inputs + 2], 2.5);
}

/// Every other per-channel field: one copy in the image, patched as is.
#[test]
fn wp_cg31_every_per_channel_field_follows_another_client() {
    let (core, remote) = demo();
    let s0 = core.get_state();
    let inputs = usize::from(s0.num_input_channels);
    let out = |o: usize, at: usize| g::OFF_OUTPUTS + o * 12 + at;

    // Output gain and mute (REQ_SET_OUTPUT_GAIN, _MUTE).
    assert!(remote.poke(out(1, 4), &f32le(-6.5), MIDI));
    wait_for(&core, "the gain", |s| s.output_gain_db[1] == -6.5);
    let muted = !s0.output_muted[1];
    assert!(remote.poke(out(1, 1), &[u8::from(muted)], MIDI));
    wait_for(&core, "the mute", |s| s.output_muted[1] == muted);

    // Polarity and routing (REQ_SET_MATRIX_ROUTE: a whole crosspoint).
    let xp = g::OFF_CROSSPOINTS + (MAX_OUTPUTS + 2) * 8;
    let mut cell = [0u8; 8];
    cell[0] = 1;
    cell[1] = 1;
    cell[4..].copy_from_slice(&f32le(-3.0));
    assert!(remote.poke(xp, &cell, MIDI));
    wait_for(&core, "the crosspoint", |s| {
        s.matrix_invert[1][2] && s.matrix_routing[1][2] && s.matrix_gain[1][2] == -3.0
    });

    // A PEQ band and a crossover band (main.c, the EQ apply).
    let mut band = [0u8; 16];
    band[0] = 1;
    band[4..8].copy_from_slice(&f32le(1234.0));
    band[8..12].copy_from_slice(&f32le(0.9));
    band[12..].copy_from_slice(&f32le(-2.0));
    let ch = inputs + 1;
    assert!(remote.poke(g::OFF_EQ + (ch * EQ_BANDS + 4) * BAND_SIZE, &band, MIDI));
    wait_for(&core, "the band", |s| s.filters[ch][4].freq == 1234.0);
    assert_eq!(core.get_state().filters[ch][4].gain, -2.0);
    band[4..8].copy_from_slice(&f32le(2500.0));
    assert!(remote.poke(
        g::OFF_CROSSOVERS + (ch * XOVER_BANDS + 1) * BAND_SIZE,
        &band,
        MIDI
    ));
    wait_for(&core, "the crossover", |s| s.xover[ch][1].freq == 2500.0);

    // A channel name (REQ_SET_CHANNEL_NAME: the whole 32-byte field).
    let mut name = [0u8; 32];
    name[..5].copy_from_slice(b"Woofs");
    assert!(remote.poke(g::OFF_CHANNEL_NAMES + ch * 32, &name, MIDI));
    wait_for(&core, "the name", |s| s.channel_names[ch] == "Woofs");

    // An input preamp (update_preamp): input 0's is also the legacy preamp.
    assert!(remote.poke(g::OFF_PREAMP + 4, &f32le(-4.0), MIDI));
    wait_for(&core, "preamp 1", |s| s.input_preamp_db[1] == -4.0);
    assert_eq!(
        core.get_state().preamp_db,
        s0.preamp_db,
        "input 1 is not the legacy one"
    );
    assert!(remote.poke(g::OFF_PREAMP, &f32le(-5.0), MIDI));
    wait_for(&core, "preamp 0", |s| s.input_preamp_db[0] == -5.0);
    assert_eq!(core.get_state().preamp_db, -5.0);
}

#[test]
fn wp_cg31_sync_twins_leaves_a_partial_or_foreign_range_alone() {
    let (core, _remote) = demo();
    let mut img = core.get_state().bulk_raw;
    let inputs = usize::from(img[g::OFF_HEADER + 4]);
    let before = img.clone();
    // Half a delay, the gain next to the output delay, and the global preamp
    // (which the device never notifies) change no twin.
    assert!(!dspi_proto::wire::sync_twins(
        &mut img,
        g::OFF_DELAYS + inputs * 4,
        2
    ));
    assert!(!dspi_proto::wire::sync_twins(
        &mut img,
        g::OFF_OUTPUTS + 4,
        4
    ));
    assert!(!dspi_proto::wire::sync_twins(&mut img, g::OFF_GLOBAL, 4));
    assert_eq!(img, before);
    // A range wider than one value carries every value it covers.
    let all = g::OFF_DELAYS..g::OFF_DELAYS + g::LEN_DELAYS;
    img[all.clone()]
        .chunks_mut(4)
        .for_each(|c| c.copy_from_slice(&f32le(7.0)));
    assert!(dspi_proto::wire::sync_twins(&mut img, all.start, all.len()));
    let mut s = core.get_state();
    state::decode_bulk(&img, &mut s).unwrap();
    let outputs = usize::from(s.num_output_channels);
    assert!(s.output_delay_ms[..outputs].iter().all(|d| *d == 7.0));
}

/// Two clients on one hub: one writes a channel delay, the other's state
/// changes (the hw-win F1 repro over DSPi Link).
#[cfg(feature = "link")]
mod link {
    use dspi_link::fakehub::{FakeHub, HubConfig};
    use dspi_proto::generated::opcodes as op;

    use super::*;
    use crate::link::Tuning;

    fn bound(hub: &FakeHub, name: &str) -> Arc<DspiCore> {
        let core = DspiCore::new();
        core.link_connect_with(
            hub.url(),
            name.into(),
            "test".into(),
            String::new(),
            String::new(),
            Tuning {
                backoff_divisor: 50,
                ..Tuning::default()
            },
        )
        .unwrap();
        let start = Instant::now();
        while core.link_devices().is_empty() {
            assert!(start.elapsed() < WAIT, "no device list");
            std::thread::sleep(Duration::from_millis(5));
        }
        let d = core.link_devices().remove(0);
        core.link_select_device(d.handle, d.serial).unwrap();
        core.fetch_all().unwrap();
        core
    }

    #[test]
    fn wp_cg31_another_link_clients_channel_delay_reaches_this_one() {
        let mut cfg = HubConfig::default();
        let inputs = usize::from(cfg.image[g::OFF_HEADER + 4]);
        let ch = inputs; // the first output's wire channel
        let off = u16::try_from(g::OFF_DELAYS + ch * 4).unwrap();
        cfg.set_offsets.insert(op::REQ_SET_DELAY, (off, 4));
        let hub = FakeHub::start(cfg).unwrap();
        let writer = bound(&hub, "wp-cg31-writer");
        let viewer = bound(&hub, "wp-cg31-viewer");
        assert_eq!(viewer.get_state().output_delay_ms[0], 0.0);

        writer
            .raw_set(
                op::REQ_SET_DELAY,
                u16::try_from(ch).unwrap(),
                f32le(1.0).to_vec(),
            )
            .unwrap();
        wait_for(&viewer, "the viewer's output delay", |s| {
            s.output_delay_ms[0] == 1.0
        });
        assert_eq!(viewer.get_state().channel_delays[ch], 1.0);
    }
}
