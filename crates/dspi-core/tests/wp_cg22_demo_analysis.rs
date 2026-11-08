//! WP CG22: the core reads analyser frames and drives the signal generator on
//! its demo device (`open_demo`, `dspi_transport::fake`), through the same
//! calls the apps make. Real time: the analyser worker polls on its own thread.

use std::sync::Arc;
use std::time::{Duration, Instant};

use dspi_core::consts::{SIGGEN_CTL_START, SIGGEN_CTL_STOP_NOW};
use dspi_core::{DspiCore, RtaRequest, RtaSnapshot, SiggenConfig};

/// `RTA_TAP_OUTPUT`.
const TAP_OUTPUT: u8 = 1;
/// `SIGGEN_SINE`, `SIGGEN_STATE_IDLE`, `SIGGEN_STOP_HOST`.
const SINE: u8 = 0;
const IDLE: u8 = 0;
const STOP_HOST: u8 = 1;

fn wait_for(what: &str, mut ok: impl FnMut() -> bool) {
    let end = Instant::now() + Duration::from_secs(10);
    while !ok() {
        assert!(Instant::now() < end, "timed out: {what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn demo() -> Arc<DspiCore> {
    let core = DspiCore::new();
    core.open_demo().expect("the demo opens");
    core
}

fn request(mask: u16, wants_bins: bool) -> RtaRequest {
    RtaRequest {
        tap: TAP_OUTPUT,
        mask,
        wants_bins,
        fft_order: 10,
        avg_ms: 300,
        peak_decay_db_s: 12,
        watching: false,
    }
}

/// A snapshot with a frame for every channel in `mask`.
fn frames_for(core: &DspiCore, mask: u16) -> RtaSnapshot {
    wait_for("frames", || {
        let s = core.rta_get_snapshot();
        u32::from(s.count) == mask.count_ones()
            && s.frames
                .iter()
                .take(usize::from(s.count))
                .all(|f| !f.silent)
    });
    core.rta_get_snapshot()
}

#[test]
fn wp_cg22_the_core_reads_analyser_frames_from_the_demo() {
    let core = demo();
    core.rta_set_request(request(0b11, false));
    core.rta_start();
    wait_for("caps", || core.rta_get_caps().supported);
    let caps = core.rta_get_caps();
    assert_eq!((caps.bass_bands, caps.max_bands), (14, 37));
    assert_eq!(caps.centres[13], 200, "the bass bank ends at 200 Hz");
    let s = frames_for(&core, 0b11);
    assert_eq!((s.frames[0].channel, s.frames[1].channel), (0, 1));
    assert_eq!(s.frames[0].n_bands, 34);
    // Energy in the bass bank: 40-200 Hz near -18 dBFS, the top well below.
    let avg = &s.frames[0].avg_db;
    assert!(
        avg[6..14].iter().all(|d| (-30.0..-8.0).contains(d)),
        "{avg:?}"
    );
    assert!(avg[30] < avg[10] - 10.0, "{avg:?}");
    wait_for("telemetry", || core.rta_get_snapshot().running);
    let s = core.rta_get_snapshot();
    assert_eq!(
        (s.first_band, s.sample_rate, s.live_mask),
        (0, 48_000, 0b11)
    );
    assert!(s.frames_per_s > 0);
    assert!(!s.config_rejected);

    // One channel with bins.
    core.rta_set_request(request(0b01, true));
    wait_for("bins", || {
        let s = core.rta_get_snapshot();
        s.bins_count == 512 && s.bins_channel == 0
    });
    assert_eq!(core.rta_get_snapshot().bins_sample_rate, 48_000);

    // Released: the core stops the device's analyser.
    core.rta_set_request(request(0, false));
    wait_for("stopped", || !core.rta_get_snapshot().running);
}

#[test]
fn wp_cg22_the_core_drives_the_demo_signal_generator() {
    let core = demo();
    let caps = core.siggen_fetch_caps().expect("caps");
    assert!(caps.supported);
    assert_eq!((caps.type_count, caps.valid_mask), (15, 0x1FF));
    assert_eq!(caps.types[0].name, "sine");
    assert_eq!(core.siggen_get_status().expect("status").state, IDLE);

    let tone = SiggenConfig {
        signal_type: SINE,
        channel_mask: 0b01,
        level_db: -20.0,
        p: vec![1000.0, 0.0, 0.0, 0.0],
        ..SiggenConfig::default()
    };
    core.siggen_set_config(tone.clone()).expect("set");
    let back = core.siggen_get_config().expect("get");
    assert_eq!(
        (back.signal_type, back.channel_mask, back.p[0]),
        (SINE, 0b01, 1000.0)
    );
    std::thread::sleep(Duration::from_millis(50));
    assert_eq!(
        core.siggen_get_status().expect("status").state,
        IDLE,
        "a config alone never starts it"
    );

    core.siggen_control(SIGGEN_CTL_START).expect("start");
    let s = core.siggen_get_status().expect("status");
    assert_ne!(s.state, IDLE);
    assert_eq!(s.signal_type, SINE);

    // The analyser hears it on output 0: the 1 kHz band stands out.
    core.rta_set_request(request(0b11, false));
    core.rta_start();
    wait_for("the tone", || {
        let s = core.rta_get_snapshot();
        let f = &s.frames[0];
        s.count == 2 && !f.silent && f.avg_db[20] > -24.0 && f.avg_db[10] < -60.0
    });
    let s = core.rta_get_snapshot();
    assert!(
        s.frames[1].avg_db[10] > -40.0,
        "output 1 still plays the programme"
    );

    core.siggen_control(SIGGEN_CTL_STOP_NOW).expect("stop");
    let s = core.siggen_get_status().expect("status");
    assert_eq!((s.state, s.stop_reason), (IDLE, STOP_HOST));
}
