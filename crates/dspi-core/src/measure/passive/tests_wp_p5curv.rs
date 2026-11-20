//! WP P5CURV: the passive microphone RTA (no device touched, read-only mode,
//! a Viewer through the fake hub) and a measurement document's curves.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
// -- INT-R1 -- used by the link-only test below; ios-check builds without it
#[cfg(feature = "link")]
use std::time::Duration;

use dspi_measure::doc::{
    CaptureInfo, Curve, DocKind, Measurement, MeasurementDoc, Meta, OutputMeasurement, RunKind,
    Stimulus,
};
use dspi_measure::response::smooth;
use dspi_measure::types::{
    BandSpec, ChannelChain, Coupler, LogGrid, Placement, ProgrammeFeed, Response, WayRole,
};
use dspi_transport::{DeviceDescriptor, FakeDevice, NotificationSource, Transport, TransportCaps};
use serde_json::Map;

use crate::{
    DspiCore, DspiError, MeasureDocCurvesOptions, MeasureRtaAveraging, MeasureRtaPassiveSetup,
    measure_doc_curves,
};

const RATE: u32 = 48_000;

// ── The passive RTA ─────────────────────────────────────────────────────

/// The demo device, counting every request the core sends after `armed`.
struct Counting {
    dev: FakeDevice,
    armed: Arc<Mutex<bool>>,
    requests: Arc<AtomicUsize>,
    writes: Arc<AtomicUsize>,
}

impl Counting {
    fn seen(&self, write: bool) {
        if *self.armed.lock().unwrap() {
            self.requests.fetch_add(1, Ordering::SeqCst);
            if write {
                self.writes.fetch_add(1, Ordering::SeqCst);
            }
        }
    }
}

impl Transport for Counting {
    fn control_in(&mut self, o: u8, v: u16, l: u16) -> dspi_transport::Result<Vec<u8>> {
        // The generator's control is a write-as-read.
        self.seen(o == dspi_proto::generated::opcodes::REQ_SIGGEN_CONTROL);
        self.dev.control_in(o, v, l)
    }
    fn control_in_upto(&mut self, o: u8, v: u16, l: u16) -> dspi_transport::Result<Vec<u8>> {
        self.seen(false);
        self.dev.control_in_upto(o, v, l)
    }
    fn control_out(&mut self, o: u8, v: u16, d: &[u8]) -> dspi_transport::Result<()> {
        self.seen(true);
        self.dev.control_out(o, v, d)
    }
    fn descriptor(&self) -> &DeviceDescriptor {
        self.dev.descriptor()
    }
    fn max_transfer(&self) -> usize {
        self.dev.max_transfer()
    }
    fn notifications(&self) -> Option<Box<dyn NotificationSource>> {
        self.dev.notifications()
    }
    fn caps(&self) -> TransportCaps {
        self.dev.caps()
    }
}

/// White noise, full scale `amp` (xorshift: deterministic).
fn noise(n: usize, amp: f32, seed: &mut u64) -> Vec<f32> {
    (0..n)
        .map(|_| {
            *seed ^= *seed << 13;
            *seed ^= *seed >> 7;
            *seed ^= *seed << 17;
            let u = ((*seed >> 40) as f64 / (1u64 << 24) as f64) * 2.0 - 1.0;
            (f64::from(amp) * u) as f32
        })
        .collect()
}

/// A flat microphone with a known dB SPL scale.
fn mic_doc(spl_offset_db: f64) -> String {
    use dspi_measure::doc::{MicCal, MicDoc, SCHEMA_VERSION};
    let grid = LogGrid::STANDARD;
    let n = grid.freqs().unwrap().len();
    MicDoc {
        schema_version: SCHEMA_VERSION,
        meta: Meta::new(DocKind::Mic, "Test mic", "2026-11-19T00:00:00Z", "test"),
        mic: MicCal {
            id: String::new(),
            name: "Test mic".into(),
            serial: "T1".into(),
            vendor: String::new(),
            format: dspi_measure::cal::CalFormat::Rew,
            angle: 0,
            sensitivity_db: None,
            gain_db: None,
            spl_offset_db: Some(spl_offset_db),
            approximate: false,
            response: Curve::from_response(&Response {
                grid,
                mag_db: vec![0.0; n],
                phase_deg: None,
                valid: None,
                spl: false,
            }),
            source_name: String::new(),
            source_hash: String::new(),
            note: String::new(),
            extra: Default::default(),
        },
        extra: Default::default(),
    }
    .to_text()
    .unwrap()
}

/// Drive the passive RTA as a client does: start, about 2 s of programme
/// noise in 20 ms blocks, the frame, hold, compare, a change of averaging,
/// stop. Returns nothing; panics on what is wrong.
fn drive(core: &DspiCore, mic: &str) {
    let st = core
        .measure_rta_passive_start(MeasureRtaPassiveSetup {
            mic_document: mic.into(),
            capture_rate_hz: RATE,
            averaging: MeasureRtaAveraging::Infinite,
            bands_per_octave: 3,
        })
        .unwrap();
    assert!(st.active);
    assert_eq!(st.spl, !mic.is_empty());
    // Nothing to hold before a frame.
    assert!(matches!(
        core.measure_rta_passive_hold(true),
        Err(DspiError::InvalidArgument { .. })
    ));
    let mut seed = 0x2545_F491_4F6C_DD1Du64;
    let mut st = st;
    for _ in 0..100 {
        st = core
            .measure_rta_passive_push(noise(960, 0.1, &mut seed), RATE)
            .unwrap();
    }
    assert!(st.frames > 0, "{st:?}");
    let f = core.measure_rta_passive_frame();
    let rta = f.rta.unwrap();
    let cur = rta.current.unwrap();
    assert!(cur.mag_db.len() > 100, "the live display grid");
    assert!(cur.mag_db.iter().all(|m| m.is_finite()));
    assert_eq!(cur.spl, !mic.is_empty());
    assert!(rta.held.is_none() && rta.compare.is_none());
    // Hold, then the same noise: the comparison is about 0 dB in band.
    assert!(core.measure_rta_passive_hold(true).unwrap().held);
    for _ in 0..50 {
        core.measure_rta_passive_push(noise(960, 0.1, &mut seed), RATE)
            .unwrap();
    }
    let rta = core.measure_rta_passive_frame().rta.unwrap();
    let cmp = rta.compare.unwrap();
    let mid: Vec<f64> = cmp
        .freqs_hz
        .iter()
        .zip(&cmp.mag_db)
        .filter(|(f, _)| **f > 200.0 && **f < 10_000.0)
        .map(|(_, m)| *m)
        .collect();
    assert!(!mid.is_empty());
    assert!(mid.iter().all(|m| m.abs() < 1.5), "{mid:?}");
    // A change of averaging starts the average again and keeps the held copy.
    let st = core
        .measure_rta_passive_averaging(MeasureRtaAveraging::Fast)
        .unwrap();
    assert_eq!(st.frames, 0);
    assert!(st.held);
    assert_eq!(st.averaging, MeasureRtaAveraging::Fast);
    // Another rate than the started one is refused; nothing analysed.
    assert!(matches!(
        core.measure_rta_passive_push(vec![0.0; 960], 44_100),
        Err(DspiError::InvalidArgument { .. })
    ));
    assert!(!core.measure_rta_passive_stop().active);
    assert!(!core.measure_rta_passive_state().active);
    assert!(core.measure_rta_passive_frame().rta.is_none());
    // A push after the stop is ignored.
    assert!(
        !core
            .measure_rta_passive_push(vec![0.0; 960], RATE)
            .unwrap()
            .active
    );
}

#[test]
fn wp_p5curv_the_passive_rta_works_in_read_only_mode_and_touches_no_device() {
    let armed = Arc::new(Mutex::new(false));
    let requests = Arc::new(AtomicUsize::new(0));
    let writes = Arc::new(AtomicUsize::new(0));
    let core = DspiCore::new();
    core.attach_probed(
        Box::new(Counting {
            dev: FakeDevice::demo(),
            armed: Arc::clone(&armed),
            requests: Arc::clone(&requests),
            writes: Arc::clone(&writes),
        }),
        "P5CURV-DEMO".into(),
        0,
    )
    .unwrap();
    core.fetch_all().unwrap();
    core.set_client_read_only(true).unwrap();
    // Read-only refuses a write before it is sent ...
    assert!(matches!(
        core.set_user_volume(-30.0),
        Err(DspiError::ReadOnly)
    ));
    *armed.lock().unwrap() = true;
    // ... and the passive RTA runs, with and without a calibrated microphone.
    drive(&core, "");
    drive(&core, &mic_doc(120.0));
    assert_eq!(writes.load(Ordering::SeqCst), 0, "no write");
    assert_eq!(requests.load(Ordering::SeqCst), 0, "no request at all");
}

#[test]
fn wp_p5curv_the_passive_rta_needs_no_device() {
    let core = DspiCore::new();
    drive(&core, "");
}

#[test]
fn wp_p5curv_a_calibrated_microphone_reads_db_spl() {
    // A full-scale sine reads 0 dBFS in its band (the crate's convention):
    // with a scale of 120 dB SPL at 0 dBFS the 1 kHz band reads 120 dB SPL.
    let core = DspiCore::new();
    core.measure_rta_passive_start(MeasureRtaPassiveSetup {
        mic_document: mic_doc(120.0),
        capture_rate_hz: RATE,
        averaging: MeasureRtaAveraging::Infinite,
        bands_per_octave: 3,
    })
    .unwrap();
    let block: Vec<f32> = (0..RATE as usize)
        .map(|i| (2.0 * std::f64::consts::PI * 1000.0 * i as f64 / f64::from(RATE)).sin() as f32)
        .collect();
    core.measure_rta_passive_push(block, RATE).unwrap();
    let cur = core
        .measure_rta_passive_frame()
        .rta
        .unwrap()
        .current
        .unwrap();
    let (_, peak) = cur
        .freqs_hz
        .iter()
        .zip(&cur.mag_db)
        .filter(|(f, _)| (900.0..1100.0).contains(*f))
        .fold(
            (0.0, f64::MIN),
            |a, (f, m)| if *m > a.1 { (*f, *m) } else { a },
        );
    assert!((peak - 120.0).abs() < 1.5, "{peak}");
}

#[test]
fn wp_p5curv_a_bad_passive_setup_is_refused() {
    let core = DspiCore::new();
    for setup in [
        MeasureRtaPassiveSetup {
            capture_rate_hz: 1_000,
            ..Default::default()
        },
        MeasureRtaPassiveSetup {
            bands_per_octave: 49,
            ..Default::default()
        },
        MeasureRtaPassiveSetup {
            mic_document: "not a document".into(),
            ..Default::default()
        },
    ] {
        assert!(matches!(
            core.measure_rta_passive_start(setup),
            Err(DspiError::InvalidArgument { .. })
        ));
    }
    assert!(!core.measure_rta_passive_state().active);
    assert!(matches!(
        core.measure_rta_passive_hold(true),
        Err(DspiError::InvalidArgument { .. })
    ));
    assert!(matches!(
        core.measure_rta_passive_averaging(MeasureRtaAveraging::Slow),
        Err(DspiError::InvalidArgument { .. })
    ));
    // Defaults: 48 kHz, 1/3 octave.
    let st = core
        .measure_rta_passive_start(MeasureRtaPassiveSetup::default())
        .unwrap();
    assert_eq!((st.capture_rate_hz, st.bands_per_octave), (48_000, 3));
    // A block past the bound is refused.
    assert!(matches!(
        core.measure_rta_passive_push(vec![0.0; dspi_measure::rta::MAX_BLOCK + 1], 48_000),
        Err(DspiError::InvalidArgument { .. })
    ));
    // Non-finite samples are silence, counted.
    let st = core
        .measure_rta_passive_push(vec![f32::NAN; 100], 48_000)
        .unwrap();
    assert_eq!(st.nonfinite_samples, 100);
}

#[cfg(feature = "link")]
#[test]
fn wp_p5curv_a_viewer_through_the_hub_gets_the_passive_rta_and_sends_nothing() {
    use dspi_link::fakehub::{Auth, FakeHub, HubConfig};
    use dspi_proto::generated::opcodes as op;
    let hub = FakeHub::start(HubConfig {
        auth: Auth::Fixed("123456".into()),
        pin_role: "viewer".into(),
        ..HubConfig::bridge()
    })
    .unwrap();
    let core = DspiCore::new();
    core.link_connect(
        hub.url(),
        "p5curv".into(),
        "test".into(),
        String::new(),
        String::new(),
    )
    .unwrap();
    core.link_pair("123456".into()).unwrap();
    let until = std::time::Instant::now() + Duration::from_secs(8);
    while core.link_devices().is_empty() {
        assert!(std::time::Instant::now() < until, "no device");
        std::thread::sleep(Duration::from_millis(5));
    }
    let d = core.link_devices().remove(0);
    core.link_select_device(d.handle, d.serial).unwrap();
    core.ensure_probed();
    // A Viewer may not play the generator.
    assert!(matches!(
        core.measure_start(crate::MeasureSetup {
            outputs: vec![crate::MeasureOutput::default()],
            ..Default::default()
        }),
        Err(DspiError::Denied | DspiError::InvalidArgument { .. })
    ));
    let before = hub.log().len();
    drive(&core, "");
    let after = hub.log();
    let new = after.get(before..).unwrap_or_default();
    // `Dir::Set` is 0: nothing written; no generator request of any kind.
    assert!(new.iter().all(|e| e.1 != 0), "a write: {new:?}");
    assert!(
        after
            .iter()
            .all(|e| e.2 != op::REQ_SIGGEN_SET_CONFIG && e.2 != op::REQ_SIGGEN_CONTROL),
        "the generator: {after:?}"
    );
}

// ── A measurement document's curves ─────────────────────────────────────

fn band(t: u8, freq: f32) -> BandSpec {
    BandSpec {
        filter_type: t,
        bypass: false,
        freq,
        q: 0.707,
        gain: 0.0,
        qp: 0.0,
    }
}

fn chain() -> ChannelChain {
    ChannelChain {
        peq: (0..12).map(|i| band(0, 100.0 * (i + 1) as f32)).collect(),
        crossover: (0..8).map(|_| band(0, 1000.0)).collect(),
        gain_db: 0.0,
        delay_ms: 0.0,
        feed: ProgrammeFeed {
            gain_db: 0.0,
            inverted: false,
        },
    }
}

/// A rippled response with an untrusted bottom and top, and a spread.
fn response(seed: f64) -> Curve {
    let f = LogGrid::STANDARD.freqs().unwrap();
    Curve {
        grid: LogGrid::STANDARD,
        mag_db: f
            .iter()
            .map(|x| 75.0 + 6.0 * (x.ln() * seed * 3.0).sin())
            .collect(),
        phase_deg: None,
        valid: Some(f.iter().map(|x| *x > 30.0 && *x < 16_000.0).collect()),
        spread_db: Some(
            f.iter()
                .map(|x| (x.ln() * 0.37).sin().abs() * 3.0)
                .collect(),
        ),
        spl: true,
        extra: Map::new(),
    }
}

fn measurement(placement: Placement) -> String {
    MeasurementDoc {
        schema_version: 1,
        meta: Meta::new(
            DocKind::Measurement,
            "P5CURV",
            "2026-11-19T10:00:00Z",
            "0.5.0",
        ),
        measurement: Measurement {
            id: String::new(),
            serial: "E66164084B3C2E2B".into(),
            platform: "RP2350".into(),
            saved_ms: 1_763_287_200_000,
            placement,
            mic: None,
            stimulus: Stimulus {
                kind: RunKind::Walked,
                f1_hz: 20.0,
                f2_hz: 20_000.0,
                duration_ms: 5_500,
                repeat: 1,
                gap_ms: 1_000,
                level_db: -24.0,
                device_rate_hz: 48_000,
                extra: Map::new(),
            },
            capture: CaptureInfo {
                rate_hz: 48_000,
                clock_ratio: 1.0,
                timing_reliable: true,
                max_residual_us: Some(3.0),
                extra: Map::new(),
            },
            outputs: [(0u8, WayRole::Low, 1.0), (1, WayRole::High, 2.0)]
                .iter()
                .map(|(o, role, seed)| OutputMeasurement {
                    output: *o,
                    role: *role,
                    speaker: "L".into(),
                    chain: chain().into(),
                    arrival_ms: None,
                    response: response(*seed),
                    extra: Map::new(),
                })
                .collect(),
            positions: 1,
            note: String::new(),
            tunes: Vec::new(),
            extra: Map::new(),
        },
        extra: Map::new(),
    }
    .to_text()
    .unwrap()
}

/// The document's stored response of output `i`, as it reads back.
fn stored(text: &str, i: usize) -> Response {
    MeasurementDoc::from_text(text).unwrap().measurement.outputs[i]
        .response
        .to_response()
}

#[test]
fn wp_p5curv_doc_curves_are_the_stored_curves_on_the_standard_grid() {
    let text = measurement(Placement::ListeningPosition);
    let c = measure_doc_curves(text.clone(), MeasureDocCurvesOptions::default()).unwrap();
    let n = LogGrid::STANDARD.freqs().unwrap().len();
    assert_eq!(n, 1066);
    assert_eq!(c.outputs.len(), 2);
    assert!(c.timing_reliable);
    assert_eq!(c.outputs[1].role, crate::MeasureRole::High);
    for (i, o) in c.outputs.iter().enumerate() {
        let r = stored(&text, i);
        assert_eq!(o.measured.freqs_hz, LogGrid::STANDARD.freqs().unwrap());
        assert_eq!(o.measured.mag_db, r.mag_db, "unsmoothed: as stored");
        assert_eq!(Some(o.measured.valid.clone()), r.valid, "the mask");
        assert!(o.measured.valid.iter().any(|v| !v) && o.measured.valid.iter().any(|v| *v));
        assert!(o.measured.spl);
        assert!(o.target.is_none());
        assert_eq!(o.spread_db.len(), n);
    }
}

#[test]
fn wp_p5curv_doc_curves_smooth_and_level_as_the_live_view() {
    let text = measurement(Placement::ListeningPosition);
    let c = measure_doc_curves(
        text.clone(),
        MeasureDocCurvesOptions {
            smoothing: 3,
            target: true,
            target_document: String::new(),
        },
    )
    .unwrap();
    assert_eq!(c.smoothing, 3);
    for (i, o) in c.outputs.iter().enumerate() {
        let r = stored(&text, i);
        let s = smooth(&r, 3).unwrap();
        assert_eq!(o.measured.mag_db, s.mag_db, "P5RESP's smoothing");
        assert_eq!(Some(o.measured.valid.clone()), r.valid, "the mask kept");
        // The target levelled as the live view levels it (to the raw curve).
        let t = crate::measure::live::levelled_target(&live_default_target(false), &r);
        let got = o.target.as_ref().unwrap();
        assert_eq!(got.freqs_hz.len(), 1066);
        assert_eq!(got.mag_db, t.mag_db);
        assert!(got.spl);
    }
    // Headphones: the coupler's default target, not the speakers'.
    let hp = measure_doc_curves(
        measurement(Placement::Headphones {
            coupler: Coupler::Iec60318_4,
            reseats: 1,
        }),
        MeasureDocCurvesOptions {
            smoothing: 0,
            target: true,
            target_document: String::new(),
        },
    )
    .unwrap();
    let sp = measure_doc_curves(
        measurement(Placement::ListeningPosition),
        MeasureDocCurvesOptions {
            smoothing: 0,
            target: true,
            target_document: String::new(),
        },
    )
    .unwrap();
    assert_ne!(
        hp.outputs[0].target.as_ref().unwrap().mag_db,
        sp.outputs[0].target.as_ref().unwrap().mag_db
    );
}

/// The default target the live view evaluates (speakers or headphones on an
/// IEC 60318-4 coupler), on the standard grid.
fn live_default_target(headphones: bool) -> Response {
    use dspi_measure::target::{
        DEFAULT_REFERENCE_BAND, Target, TargetDefinition, TargetUse, default_target,
    };
    let (u, c) = if headphones {
        (TargetUse::Headphones, Some(Coupler::Iec60318_4))
    } else {
        (TargetUse::Speakers, None)
    };
    Target {
        target_use: u,
        coupler: c,
        definition: TargetDefinition::Builtin {
            builtin: default_target(u, c),
        },
        reference: DEFAULT_REFERENCE_BAND,
    }
    .evaluate(&LogGrid::STANDARD)
    .unwrap()
}

#[test]
fn wp_p5curv_doc_curves_refuse_what_does_not_read() {
    let text = measurement(Placement::ListeningPosition);
    let bad = [
        (text.clone(), 49u8, true, String::new()),
        ("{}".to_owned(), 0, false, String::new()),
        ("not json".to_owned(), 0, false, String::new()),
        (mic_doc(100.0), 0, false, String::new()),
        (text.clone(), 0, true, "not a target".to_owned()),
        (text[..text.len() / 2].to_owned(), 0, false, String::new()),
    ];
    for (t, smoothing, target, target_document) in bad {
        assert!(matches!(
            measure_doc_curves(
                t,
                MeasureDocCurvesOptions {
                    smoothing,
                    target,
                    target_document,
                }
            ),
            Err(DspiError::InvalidArgument { .. })
        ));
    }
    // A target document is not read when no target is asked for.
    assert!(
        measure_doc_curves(
            text,
            MeasureDocCurvesOptions {
                smoothing: 0,
                target: false,
                target_document: "not a target".into(),
            }
        )
        .is_ok()
    );
}

proptest::proptest! {
    #[test]
    fn wp_p5curv_doc_curves_never_panic(s in ".{0,400}", smoothing in 0u8..60) {
        let _ = measure_doc_curves(s, MeasureDocCurvesOptions {
            smoothing,
            target: true,
            target_document: String::new(),
        });
    }
}
