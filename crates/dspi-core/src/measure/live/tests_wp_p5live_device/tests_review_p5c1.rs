//! Review P5-C1: the live view's safety cases the WP's tests left out, on its
//! own rig: the read-only mode switched on while the live view plays (review
//! P5-B F2 for the live view), a lease that ran out (the dead-man), and a
//! change whose second write the device refuses (the rollback).

use dspi_transport::TransportError;

use super::*;

fn playing(mic: &mut Mic) {
    let st = mic.run(20.0, |s| {
        s.session.phase == MeasurePhase::Run && s.session.level_db.is_some()
    });
    assert_eq!(st.session.phase, MeasurePhase::Run, "{st:#?}");
}

fn stop_now() -> Req {
    Req {
        op: op::REQ_SIGGEN_CONTROL,
        value: crate::consts::SIGGEN_CTL_STOP_NOW,
        data: Vec::new(),
    }
}

#[test]
fn review_p5c1_read_only_switched_on_mid_live_stops_the_generator_and_ends_the_live_view() {
    let r = rig();
    let mut mic = Mic::new(&r, free_world());
    r.core.measure_live_start(sweep_setup()).unwrap();
    playing(&mut mic);
    assert_eq!(r.core.held_lock_holder(), Some(Holder::Measure));
    let n = r.log.lock().unwrap().len();
    r.core.set_client_read_only(true).unwrap();
    let after: Vec<Req> = r.log.lock().unwrap().iter().skip(n).cloned().collect();
    assert_eq!(after, vec![stop_now()], "one STOP_NOW and nothing else");
    assert_eq!(r.core.siggen_get_status().unwrap().state, 0, "idle");
    let st = r.core.measure_live_state();
    assert!(!st.active, "{st:#?}");
    assert_eq!(r.core.held_lock_holder(), None, "the lock given back");
    // Ticking on in read-only mode starts nothing and sends no write.
    let n = r.log.lock().unwrap().len();
    mic.run(1.5, |_| false);
    assert_eq!(r.log.lock().unwrap().len(), n);
    // And it cannot be started again while read-only.
    assert!(matches!(
        r.core.measure_live_start(sweep_setup()),
        Err(DspiError::ReadOnly)
    ));
    assert_eq!(r.log.lock().unwrap().len(), n);
}

#[test]
fn review_p5c1_a_lease_that_ran_out_ends_the_live_view_and_stops_the_generator() {
    let r = rig();
    let mut mic = Mic::new(&r, free_world());
    r.core.measure_live_start(sweep_setup()).unwrap();
    playing(&mut mic);
    // The client did not tick for the whole lease.
    r.core.expire_held_lock();
    let n = r.log.lock().unwrap().len();
    let st = r.core.measure_live_tick();
    assert!(!st.active, "{st:#?}");
    assert_eq!(st.issue.unwrap().key, "measure.abort.lock");
    let after: Vec<Req> = r.log.lock().unwrap().iter().skip(n).cloned().collect();
    assert_eq!(after, vec![stop_now()], "one STOP_NOW and nothing else");
    assert_eq!(r.core.siggen_get_status().unwrap().state, 0, "idle");
    assert_eq!(r.core.held_lock_holder(), None);
    // No further run.
    let n = r.log.lock().unwrap().len();
    mic.run(1.5, |_| false);
    assert_eq!(r.log.lock().unwrap().len(), n);
}

/// The rig's recorder, refusing every write whose data holds `needle`.
struct Flaky {
    rec: Rec,
    needle: Arc<Mutex<Option<Vec<u8>>>>,
}

impl Transport for Flaky {
    fn control_in(&mut self, o: u8, v: u16, l: u16) -> dspi_transport::Result<Vec<u8>> {
        self.rec.control_in(o, v, l)
    }
    fn control_in_upto(&mut self, o: u8, v: u16, l: u16) -> dspi_transport::Result<Vec<u8>> {
        self.rec.control_in_upto(o, v, l)
    }
    fn control_out(&mut self, o: u8, v: u16, d: &[u8]) -> dspi_transport::Result<()> {
        if let Some(n) = self.needle.lock().unwrap().as_ref()
            && d.windows(n.len()).any(|w| w == n.as_slice())
        {
            return Err(TransportError::Stalled { opcode: o });
        }
        self.rec.control_out(o, v, d)
    }
    fn descriptor(&self) -> &DeviceDescriptor {
        self.rec.descriptor()
    }
    fn max_transfer(&self) -> usize {
        self.rec.max_transfer()
    }
    fn notifications(&self) -> Option<Box<dyn NotificationSource>> {
        self.rec.notifications()
    }
    fn caps(&self) -> TransportCaps {
        self.rec.caps()
    }
}

/// [`rig`] with a [`Flaky`] transport.
fn flaky_rig() -> (Rig, Arc<Mutex<Option<Vec<u8>>>>) {
    let dev = FakeDevice::demo();
    let remote = dev.remote();
    let log = Arc::new(Mutex::new(Vec::new()));
    let needle = Arc::new(Mutex::new(None));
    let t = Flaky {
        rec: Rec {
            dev,
            log: Arc::clone(&log),
        },
        needle: Arc::clone(&needle),
    };
    let core = DspiCore::new();
    core.attach_probed(Box::new(t), "P5C1-FLAKY".into(), 0)
        .unwrap();
    core.fetch_all().unwrap();
    core.set_user_volume(-20.0).unwrap();
    let s = core.get_state();
    if s.loudness_enabled {
        core.set_loudness(false).unwrap();
    }
    if s.crossfeed_enabled {
        core.set_crossfeed(false).unwrap();
    }
    if s.psybass_enabled {
        core.set_psybass_enabled(false).unwrap();
    }
    for o in 0..s.num_output_channels.min(4) {
        if !s.output_enabled[usize::from(o)] {
            core.set_output_enable(o, true).unwrap();
        }
        if s.output_muted[usize::from(o)] {
            core.set_output_mute(o, false).unwrap();
        }
    }
    for o in 0..s.num_output_channels {
        if s.limiter_enabled
            .get(usize::from(o))
            .copied()
            .unwrap_or(false)
        {
            core.set_limiter_param(o, 0, 0.0).unwrap();
        }
    }
    remote.measure_set_peaks(-200.0, &[], 2);
    settle(&core);
    log.lock().unwrap().clear();
    (Rig { core, remote, log }, needle)
}

#[test]
fn review_p5c1_a_refused_second_write_puts_the_first_back() {
    let (r, needle) = flaky_rig();
    let mut mic = Mic::new(&r, free_world());
    r.core.measure_live_start(sweep_setup()).unwrap();
    first_frame(&mut mic, &r.core);
    let s = r.core.get_state();
    let ch = usize::from(s.num_input_channels);
    let free: Vec<u8> = s.filters[ch]
        .iter()
        .enumerate()
        .filter(|(_, b)| b.filter_type == 0 || b.bypass)
        .map(|(i, _)| i as u8)
        .take(2)
        .collect();
    assert_eq!(free.len(), 2, "two free bands on the demo's output 1");
    let before = s.filters[ch].clone();
    let edit = |band: u8, freq: f32| MeasureLiveBand {
        band,
        filter_type: 1,
        freq,
        q: 1.0,
        gain: -3.0,
        ..MeasureLiveBand::default()
    };
    // The second band's frequency is what the device refuses.
    let odd = 777.25f32;
    let p = r
        .core
        .measure_live_predict(0, vec![edit(free[0], 300.0), edit(free[1], odd)])
        .unwrap();
    assert_eq!(p.changes.len(), 2);
    *needle.lock().unwrap() = Some(odd.to_le_bytes().to_vec());
    let e = r.core.measure_live_apply(p.id).unwrap_err();
    *needle.lock().unwrap() = None;
    eprintln!("P5C1 refused apply: {e}");
    settle(&r.core);
    let s = r.core.get_state();
    assert_eq!(
        s.filters[ch], before,
        "the first band is back, the second untouched"
    );
    let st = r.core.measure_live_state();
    assert!(st.pending.is_none());
    // The live view goes on (or ended with its reason): never a half change.
    assert_configs_safe(&r);
    r.core.measure_live_stop();
    assert_eq!(r.core.held_lock_holder(), None);
}

/// S3 for the live view: a later run starts at the level the last ramp
/// settled on, lowered by any boost the output's chain gained since, the
/// user's own EQ edit included (not only a change applied through the live
/// view).
#[test]
fn review_p5c1_a_boost_since_the_last_ramp_lowers_the_next_start() {
    let r = rig();
    let mut mic = Mic::new(&r, free_world());
    r.core.measure_live_start(sweep_setup()).unwrap();
    let st = mic.run(40.0, |s| s.seq >= 1);
    assert!(st.active && st.seq >= 1, "{st:#?}");
    let level = st.level_db.unwrap();
    // The user's own edit while the second run plays: +6 dB at 1 kHz.
    let band = free_band(&r);
    let n_in = r.core.get_state().num_input_channels;
    r.core
        .set_filter(
            n_in,
            band,
            FilterParams {
                filter_type: 1,
                bypass: false,
                freq: 1_000.0,
                q: 1.0,
                gain: 6.0,
                qp: 0.0,
            },
        )
        .unwrap();
    // The run after it (armed with the boost in the chain).
    let runs = st.runs;
    let st = mic.run(40.0, |s| s.runs > runs);
    assert!(st.active && st.runs > runs, "{st:#?}");
    let lowered = st.level_db.unwrap();
    assert!(
        lowered <= level - 5.5,
        "the next ramp starts at {lowered} dBFS after a +6 dB boost; the last settled on {level}"
    );
    let n = configs(&r).len();
    let st = mic.run(40.0, |s| s.session.generator_starts >= 1);
    assert!(st.active, "{st:#?}");
    let first = configs(&r)[n].clone();
    assert!(
        first.level_db <= level - 5.5,
        "the first sweep after the boost plays at {} dBFS",
        first.level_db
    );
    r.core.measure_live_stop();
    assert_configs_safe(&r);
}
