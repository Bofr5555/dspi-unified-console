//! WP P1AQ tests: AutoEQ preview and apply to a channel pair, on the demo
//! device. Snapshot folders live under `std::env::temp_dir()`, which
//! `ci/run.sh` points at the home volume (never tmpfs).

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use dspi_session::limits::{LimitsProfile, OutputLimits};
use dspi_transport::{DeviceDescriptor, FakeDevice, NotificationSource, Transport, TransportCaps};

use super::*;
use crate::CURVE_POINTS;

/// The demo device, counting every transfer that reaches it (IN and OUT).
struct Counted {
    dev: FakeDevice,
    transfers: Arc<AtomicUsize>,
}

impl Counted {
    fn count(&self) {
        self.transfers.fetch_add(1, Ordering::SeqCst);
    }
}

impl Transport for Counted {
    fn control_in(&mut self, op: u8, value: u16, len: u16) -> dspi_transport::Result<Vec<u8>> {
        self.count();
        self.dev.control_in(op, value, len)
    }
    fn control_in_upto(&mut self, op: u8, value: u16, len: u16) -> dspi_transport::Result<Vec<u8>> {
        self.count();
        self.dev.control_in_upto(op, value, len)
    }
    fn control_out(&mut self, op: u8, value: u16, data: &[u8]) -> dspi_transport::Result<()> {
        self.count();
        self.dev.control_out(op, value, data)
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

struct Rig {
    core: Arc<DspiCore>,
    transfers: Arc<AtomicUsize>,
}

impl Rig {
    fn sent(&self) -> usize {
        self.transfers.load(Ordering::SeqCst)
    }
    /// Wire channel of output `o` (0-based).
    fn out(&self, o: u8) -> u8 {
        self.core.get_state().num_input_channels + o
    }
}

fn rig() -> Rig {
    let transfers = Arc::new(AtomicUsize::new(0));
    let dev = Counted {
        dev: FakeDevice::demo(),
        transfers: Arc::clone(&transfers),
    };
    let core = DspiCore::new();
    core.attach_probed(Box::new(dev), "P1AQ-DEMO".into(), 0)
        .unwrap();
    core.fetch_all().unwrap();
    Rig { core, transfers }
}

fn store_dir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join("wp-p1aq-core").join(format!(
        "{name}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&d);
    d
}

fn filter(kind: &str, freq: f32, q: f32, gain_db: f32) -> AutoEqFilter {
    AutoEqFilter {
        kind: kind.into(),
        freq,
        q,
        gain_db,
    }
}

/// A low shelf of `boost` dB, a cut and a high shelf; preamp -6.5 dB.
fn profile(boost: f32) -> AutoEqProfile {
    AutoEqProfile {
        id: "oratory1990/harman/Test HP".into(),
        name: "Test HP".into(),
        preamp_db: -6.5,
        filters: vec![
            filter("LSC", 105.0, 0.7, boost),
            filter("PK", 1000.0, 1.4, -3.0),
            filter("HSC", 10000.0, 0.7, 4.0),
        ],
    }
}

/// The device's state with the pair's PEQ banks and levels as `pv` says.
fn expected(before: &DspState, pv: &AutoEqPreview) -> DspState {
    let mut s = before.clone();
    for c in &pv.channels {
        let i = usize::from(c.channel);
        s.filters[i] = c.bands.clone();
        if c.is_output {
            s.xover[i] = c.crossover.clone();
            s.output_gain_db[i - usize::from(s.num_input_channels)] = c.level_db;
        } else {
            s.input_preamp_db[i] = c.level_db;
            if c.channel == 0 {
                s.preamp_db = c.level_db;
            }
        }
    }
    s
}

/// `s` without the raw image bytes the mirror also keeps (the typed fields
/// are what the tests compare).
fn typed(s: &DspState) -> DspState {
    DspState {
        bulk_raw: Vec::new(),
        ..s.clone()
    }
}

#[test]
fn wp_p1aq_preview_writes_nothing() {
    let r = rig();
    let (a, b) = (r.out(2), r.out(3));
    let before = r.core.get_state();
    let sent = r.sent();
    let pv = r.core.autoeq_preview(profile(5.0), a, b).unwrap();
    assert_eq!(r.sent(), sent, "a preview makes no transfer at all");
    assert_eq!(r.core.get_state(), before);

    assert_eq!(pv.profile_id, "oratory1990/harman/Test HP");
    assert_eq!(pv.level_target, AUTOEQ_LEVEL_OUTPUT_GAIN);
    assert_eq!(pv.channels.len(), 2);
    assert_eq!(
        pv.channels.iter().map(|c| c.channel).collect::<Vec<_>>(),
        [a, b]
    );
    for c in &pv.channels {
        assert!(c.is_output);
        assert_eq!(c.level_db, -6.5);
        assert_eq!(c.bands.len(), before.filters[usize::from(a)].len());
        assert_eq!(c.bands[0].filter_type, FilterType::LowShelf.to_raw());
        assert_eq!(c.bands[0].gain, 5.0);
        assert_eq!(c.bands[1].filter_type, FilterType::Peaking.to_raw());
        assert_eq!(c.bands[2].filter_type, FilterType::HighShelf.to_raw());
        assert!(c.bands[3..].iter().all(|f| f.filter_type == 0), "{c:?}");
        assert_eq!(c.curve.len(), CURVE_POINTS);
        // 10 Hz sits on the low shelf's plateau; 1 kHz on the cut.
        assert!((c.curve[0] - 5.0).abs() < 0.3, "{}", c.curve[0]);
        let min = c.curve.iter().copied().fold(f64::MAX, f64::min);
        assert!(min < -2.0, "the cut shows: {min}");
    }
    assert!(pv.held.is_empty() && pv.notes.is_empty(), "{pv:?}");

    // Inputs: the preamp is the inputs' preamp.
    let pv = r.core.autoeq_preview(profile(5.0), 0, 1).unwrap();
    assert_eq!(pv.level_target, AUTOEQ_LEVEL_INPUT_PREAMP);
    assert!(
        pv.channels
            .iter()
            .all(|c| !c.is_output && c.crossover.is_empty())
    );
    assert_eq!(r.sent(), sent);
}

#[test]
fn wp_p1aq_apply_to_outputs_3_and_4_changes_only_them() {
    let r = rig();
    let (a, b) = (r.out(2), r.out(3));
    let before = r.core.get_state();
    let pv = r.core.autoeq_preview(profile(5.0), a, b).unwrap();
    let done = r.core.autoeq_apply(profile(5.0), a, b).unwrap();
    assert_eq!(done.preview, pv, "the apply is the preview");
    let after = r.core.get_state();
    assert_eq!(typed(&after), typed(&expected(&before, &pv)));
    assert_ne!(typed(&after), typed(&before));

    // One undo step, labelled, takes all of it back; one redo brings it.
    let history = r.core.history(u32::MAX);
    assert!(!history.is_empty());
    assert!(
        history
            .iter()
            .all(|h| h.label.as_deref() == Some("AutoEQ: Test HP")),
        "{history:?}"
    );
    r.core.undo().unwrap();
    assert_eq!(typed(&r.core.get_state()), typed(&before));
    r.core.redo().unwrap();
    assert_eq!(typed(&r.core.get_state()), typed(&after));
}

#[test]
fn wp_p1aq_any_pair_inputs_too() {
    let r = rig();
    let before = r.core.get_state();
    let done = r.core.autoeq_apply(profile(3.0), 1, 0).unwrap();
    let after = r.core.get_state();
    assert_eq!(typed(&after), typed(&expected(&before, &done.preview)));
    assert_eq!(after.input_preamp_db[0], -6.5);
    assert_eq!(after.input_preamp_db[1], -6.5);
    assert_eq!(after.output_gain_db, before.output_gain_db);
}

#[test]
fn wp_p1aq_a_boost_above_a_limit_is_clamped_and_reported() {
    let r = rig();
    let (a, b) = (r.out(2), r.out(3));
    let limits = LimitsProfile {
        outputs: vec![OutputLimits {
            output: 2,
            max_gain_db: Some(-10.0),
            ..Default::default()
        }],
        max_peq_boost_db: Some(6.0),
        ..Default::default()
    };
    // The demo's 0 dB on output 3 is held at once; then up to -6.5 (the
    // profile's preamp) would cross the limit again.
    let applied = r.core.limits_apply_profile(limits.to_document()).unwrap();
    assert_eq!(applied.held, ["out.gain[2] 0 → -10 dB"], "{applied:?}");

    let pv = r.core.autoeq_preview(profile(9.0), a, b).unwrap();
    for c in &pv.channels {
        assert_eq!(c.bands[0].gain, 6.0, "the boost held: {c:?}");
        assert_eq!(c.bands[2].gain, 4.0, "under the limit, as given");
    }
    assert_eq!(pv.channels[0].level_db, -10.0, "output 3's gain held");
    assert_eq!(pv.channels[1].level_db, -6.5, "output 4 has no gain limit");
    assert_eq!(
        pv.held,
        [
            "out.gain[2] -6.5 → -10 dB".to_owned(),
            format!("eq.gain[{a},0] 9 → 6 dB"),
            format!("eq.gain[{b},0] 9 → 6 dB"),
        ],
        "{pv:?}"
    );
    assert!(
        pv.notes[0].starts_with("Held to the limits profile: "),
        "{pv:?}"
    );

    let done = r.core.autoeq_apply(profile(9.0), a, b).unwrap();
    assert_eq!(done.preview.held, pv.held);
    let s = r.core.get_state();
    assert_eq!(s.filters[usize::from(a)][0].gain, 6.0);
    assert_eq!(s.filters[usize::from(b)][0].gain, 6.0);
    assert_eq!(s.output_gain_db[2], -10.0);
    assert_eq!(r.core.limits_status().last_held, pv.held);
}

/// An output whose high-pass is a PEQ band the profile overwrites gets the
/// high-pass on its crossover instead, and the apply goes through.
#[test]
fn wp_p1aq_a_highpass_the_limits_ask_for_moves_to_the_crossover() {
    let r = rig();
    let (a, b) = (r.out(2), r.out(3));
    let hp = FilterParams {
        filter_type: FilterType::HighPass.to_raw(),
        bypass: false,
        freq: 2000.0,
        q: 0.707,
        gain: 0.0,
        qp: 0.707,
    };
    r.core.set_filter(a, 0, hp).unwrap();
    let limits = LimitsProfile {
        outputs: vec![OutputLimits {
            output: 2,
            highpass_min_hz: Some(1500.0),
            highpass_min_slope_db: Some(12),
            ..Default::default()
        }],
        ..Default::default()
    };
    let applied = r.core.limits_apply_profile(limits.to_document()).unwrap();
    assert!(applied.held.is_empty(), "{applied:?}");

    let done = r.core.autoeq_apply(profile(5.0), a, b).unwrap();
    assert_eq!(done.preview.held.len(), 1, "{done:?}");
    assert!(
        done.preview.held[0].starts_with("output 3: a 12 dB/oct high-pass at 1500 Hz"),
        "{done:?}"
    );
    let s = r.core.get_state();
    let xo = &s.xover[usize::from(a)];
    assert!(
        xo.iter()
            .any(|f| f.filter_type == 33 && f.freq == 1500.0 && !f.bypass),
        "{xo:?}"
    );
    assert_eq!(
        s.filters[usize::from(a)][0].filter_type,
        FilterType::LowShelf.to_raw()
    );
    assert_eq!(s.xover[usize::from(b)], r_before_xover(&done, b));
}

/// Output 4's crossover as the preview says (unchanged by the profile).
fn r_before_xover(done: &AutoEqApply, ch: u8) -> Vec<FilterParams> {
    done.preview
        .channels
        .iter()
        .find(|c| c.channel == ch)
        .unwrap()
        .crossover
        .clone()
}

#[test]
fn wp_p1aq_the_snapshot_recalls_the_state_before() {
    let r = rig();
    let (a, b) = (r.out(2), r.out(3));
    let before = r.core.get_state();

    // No folder: the document alone.
    let done = r.core.autoeq_apply(profile(5.0), a, b).unwrap();
    assert_eq!(done.snapshot_id, "");
    assert!(done.notes.is_empty(), "{done:?}");
    assert!(done.snapshot_document.contains("\"snapshot\""));
    r.core
        .snapshot_recall_document(done.snapshot_document.clone())
        .unwrap();
    let back = r.core.get_state();
    assert_eq!(back.filters, before.filters);
    assert_eq!(back.xover, before.xover);
    assert_eq!(back.output_gain_db, before.output_gain_db);
    assert_eq!(back.input_preamp_db, before.input_preamp_db);

    // A folder: filed too, and recalled from the store.
    let dir = store_dir("store");
    r.core
        .snapshot_set_dir(dir.to_string_lossy().into_owned())
        .unwrap();
    let done = r.core.autoeq_apply(profile(5.0), 0, 1).unwrap();
    assert_ne!(done.snapshot_id, "");
    let listed = r.core.snapshot_list(String::new());
    let info = listed.iter().find(|s| s.id == done.snapshot_id).unwrap();
    assert_eq!(info.name, "Before AutoEQ: Test HP");
    assert_ne!(r.core.get_state().input_preamp_db, before.input_preamp_db);
    r.core.snapshot_recall(done.snapshot_id.clone()).unwrap();
    assert_eq!(r.core.get_state().input_preamp_db, before.input_preamp_db);
    assert_eq!(r.core.get_state().filters, before.filters);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn wp_p1aq_refusals_send_nothing() {
    let r = rig();
    let n = r.core.get_state().num_channels;
    let out0 = r.out(0);
    let sent = r.sent();
    let bad_q = AutoEqProfile {
        filters: vec![filter("PK", 1000.0, 0.0, 3.0)],
        ..profile(3.0)
    };
    let refused = [
        r.core.autoeq_apply(profile(3.0), 0, 0),
        r.core.autoeq_apply(profile(3.0), 1, out0),
        r.core.autoeq_apply(profile(3.0), out0, n),
        r.core.autoeq_apply(bad_q.clone(), 0, 1),
    ];
    for e in refused {
        assert!(matches!(e, Err(DspiError::InvalidArgument { .. })), "{e:?}");
    }
    assert!(matches!(
        r.core.autoeq_preview(bad_q, 0, 1),
        Err(DspiError::InvalidArgument { .. })
    ));
    assert_eq!(r.sent(), sent, "nothing reached the device");
    assert!(r.core.history(u32::MAX).is_empty());

    // An unknown shape is no refusal: it stays flat and is named.
    let odd = AutoEqProfile {
        filters: vec![filter("XYZ", 0.0, 0.0, 0.0), filter("PK", 500.0, 1.0, 2.0)],
        ..profile(3.0)
    };
    let pv = r.core.autoeq_preview(odd, 0, 1).unwrap();
    assert_eq!(pv.unsupported, ["XYZ"]);
    assert_eq!(pv.channels[0].bands[0].filter_type, 0);
    assert_eq!(pv.channels[0].bands[1].gain, 2.0);
    assert!(pv.notes.iter().any(|l| l.contains("XYZ")), "{pv:?}");

    r.core.disconnect();
    assert_eq!(
        r.core.autoeq_preview(profile(3.0), 0, 1).unwrap_err(),
        DspiError::Disconnected
    );
    assert_eq!(
        r.core.autoeq_apply(profile(3.0), 0, 1).unwrap_err(),
        DspiError::Disconnected
    );
}

#[test]
fn wp_p1aq_a_profile_from_the_databases_json() {
    let p = AutoEqProfile::from_entry_json(
        r#"{"id":"o/Sony A","manufacturer":"Sony","model":"A","preamp":-4.2,
            "filters":[{"type":"PK","freq":105,"q":0.7,"gain":3}]}"#,
    )
    .unwrap();
    assert_eq!(p.name, "Sony A");
    assert_eq!(p.preamp_db, -4.2);
    assert_eq!(p.filters, [filter("PK", 105.0, 0.7, 3.0)]);
    assert!(matches!(
        AutoEqProfile::from_entry_json("[1]"),
        Err(DspiError::InvalidArgument { .. })
    ));
}
