//! WP CG11: the flow through `DspiCore`, against `FakeDevice` with simulated
//! hangs. Every test reads the requests that really went out from a log kept
//! at the transport, so "never sends" is proven at the wire.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use dspi_proto::generated::ext as g;
use dspi_transport::{
    DeviceDescriptor, FakeDevice, FakeRemote, NotificationSource, Transport, TransportCaps,
    TransportError,
};
use proptest::prelude::*;

use super::*;
use crate::ext::tests_wp_c8::{Faults, image_as, k340_options, probed};

const SET: u8 = 0;
const TRIAL: u8 = 1;
const TRIAL_END: u8 = 2;
const AUTO: u8 = 3;
const CLEAR: u8 = 4;
const WORD: &str = overvolt::WORD;
const SOAK: u32 = cal::DEFAULT_SOAK_S;
/// Well above any real time a test takes, well below a soak.
const MARGIN: u32 = 300;

#[derive(Default)]
struct Wire {
    /// Unplugged: every transfer fails.
    unplugged: AtomicBool,
    /// A hub refuses this client's writes.
    deny_writes: AtomicBool,
    /// Every `EXT_CORE_VREG` OUT that reached the device.
    log: Mutex<Vec<(u8, u8)>>,
    /// What the device answers to a status read, instead of its own.
    status: Mutex<Option<Vec<u8>>>,
}

/// One connection to a device that outlives it, as a USB cable is to a board.
struct Plug {
    dev: Arc<Mutex<FakeDevice>>,
    wire: Arc<Wire>,
    descriptor: DeviceDescriptor,
    caps: TransportCaps,
}

impl Plug {
    fn check(&self) -> dspi_transport::Result<()> {
        if self.wire.unplugged.load(Ordering::SeqCst) {
            return Err(TransportError::Disconnected);
        }
        Ok(())
    }
}

impl Transport for Plug {
    fn control_in(&mut self, opcode: u8, value: u16, len: u16) -> dspi_transport::Result<Vec<u8>> {
        self.check()?;
        if opcode == op::REQ_EXTENSION
            && value == g::EXT_CORE_VREG
            && let Some(b) = self.wire.status.lock().unwrap().clone()
        {
            return Ok(b);
        }
        self.dev.lock().unwrap().control_in(opcode, value, len)
    }
    fn control_in_upto(
        &mut self,
        opcode: u8,
        value: u16,
        max_len: u16,
    ) -> dspi_transport::Result<Vec<u8>> {
        self.check()?;
        self.dev
            .lock()
            .unwrap()
            .control_in_upto(opcode, value, max_len)
    }
    fn control_out(&mut self, opcode: u8, value: u16, data: &[u8]) -> dspi_transport::Result<()> {
        self.check()?;
        if opcode == op::REQ_EXTENSION && value == g::EXT_CORE_VREG {
            if self.wire.deny_writes.load(Ordering::SeqCst) {
                return Err(TransportError::Denied("config".into()));
            }
            self.wire.log.lock().unwrap().push((data[0], data[1]));
        }
        self.dev.lock().unwrap().control_out(opcode, value, data)
    }
    fn descriptor(&self) -> &DeviceDescriptor {
        &self.descriptor
    }
    fn max_transfer(&self) -> usize {
        self.dev.lock().unwrap().max_transfer()
    }
    fn notifications(&self) -> Option<Box<dyn NotificationSource>> {
        None
    }
    fn caps(&self) -> TransportCaps {
        self.caps
    }
}

/// A K340-like board, up for an hour, on a core.
struct Rig {
    core: Arc<DspiCore>,
    dev: Arc<Mutex<FakeDevice>>,
    remote: FakeRemote,
    wire: Arc<Wire>,
}

impl Rig {
    fn new() -> Self {
        let dev = FakeDevice::new(dspi_transport::Capture::demo(), k340_options());
        let remote = dev.remote();
        remote.vreg_sim(VregSim::Advance(3600));
        let rig = Self {
            core: DspiCore::new(),
            dev: Arc::new(Mutex::new(dev)),
            remote,
            wire: Arc::default(),
        };
        rig.attach("CG11");
        rig
    }

    /// A stock-voltage board: build default 1.15 V.
    fn stock() -> Self {
        let rig = Self::new();
        rig.remote.vreg_set_build_default(0x0C);
        rig
    }

    /// A new connection to the same device.
    fn attach(&self, serial: &str) {
        self.wire.unplugged.store(false, Ordering::SeqCst);
        let dev = self.dev.lock().unwrap();
        let plug = Plug {
            descriptor: dev.descriptor().clone(),
            caps: dev.caps(),
            dev: self.dev.clone(),
            wire: self.wire.clone(),
        };
        drop(dev);
        self.core
            .attach_probed(Box::new(plug), serial.into(), 0)
            .unwrap();
    }

    fn unplug(&self) {
        self.wire.unplugged.store(true, Ordering::SeqCst);
    }

    /// The app restarted: a new core, the same device.
    fn restart_app(&self) -> Rig {
        let rig = Rig {
            core: DspiCore::new(),
            dev: self.dev.clone(),
            remote: self.remote.clone(),
            wire: self.wire.clone(),
        };
        rig.attach("CG11");
        rig
    }

    fn poll(&self) -> VregCalView {
        self.core.core_vreg_cal_poll()
    }

    fn phase(&self) -> VregCalPhase {
        self.poll().phase
    }

    fn sent(&self) -> Vec<(u8, u8)> {
        self.wire.log.lock().unwrap().clone()
    }

    fn sent_ops(&self, code: u8) -> usize {
        self.sent().iter().filter(|(c, _)| *c == code).count()
    }

    fn info(&self) -> CoreVregInfo {
        self.core.fetch_core_vreg().unwrap()
    }

    /// Open and start, into the soak.
    fn soak_at(&self, target: u8) {
        self.core.core_vreg_cal_open().unwrap();
        let typed = cal::needs_word(target, &status_of(&self.info())).then(|| WORD.to_owned());
        self.core.core_vreg_cal_start(target, SOAK, typed).unwrap();
        assert_eq!(self.phase(), VregCalPhase::StartingTrial);
        assert_eq!(self.phase(), VregCalPhase::Soaking);
    }

    fn pass_soak(&self) {
        self.remote.vreg_sim(VregSim::Advance(SOAK));
        assert_eq!(self.phase(), VregCalPhase::Passed);
    }
}

fn invalid<T: std::fmt::Debug>(r: Result<T>) -> bool {
    matches!(r, Err(DspiError::InvalidArgument { .. }))
}

fn no_word<T: std::fmt::Debug>(r: Result<T>) -> bool {
    matches!(r, Err(DspiError::OvervoltWordRequired { .. }))
}

fn word() -> Option<String> {
    Some(WORD.to_owned())
}

// ------------------------------------------------------------ the results

#[test]
fn wp_cg11_pass_then_set_is_verified_and_only_on_request() {
    let r = Rig::new();
    r.soak_at(0x0D);
    assert_eq!(r.sent(), vec![(CLEAR, 0), (TRIAL, 0x0D)]);
    assert_eq!(r.info().live_vreg, 0x0D);

    // An hour of uptime is not a soak: not passed until the soak has run.
    r.remote.vreg_sim(VregSim::Advance(SOAK - MARGIN));
    let v = r.poll();
    assert_eq!(v.phase, VregCalPhase::Soaking);
    assert!(v.elapsed_s >= SOAK - MARGIN && v.elapsed_s < SOAK);
    assert!(!v.can_commit);

    r.remote.vreg_sim(VregSim::Advance(MARGIN));
    let v = r.poll();
    assert_eq!(v.phase, VregCalPhase::Passed);
    assert!(v.can_commit);
    assert!(
        v.status.unwrap().trial_active,
        "stated: the trial still runs"
    );

    // Waiting on a pass commits nothing.
    for _ in 0..10 {
        assert_eq!(r.phase(), VregCalPhase::Passed);
    }
    assert_eq!(r.sent_ops(SET), 0);
    assert_eq!(r.info().stored_sel, 0);

    r.core.core_vreg_cal_commit(None).unwrap();
    assert_eq!(r.sent_ops(SET), 1);
    assert_eq!(*r.sent().last().unwrap(), (SET, 0x0D));
    assert_eq!(r.phase(), VregCalPhase::Committed);
    let i = r.info();
    assert_eq!(
        (i.stored_sel, i.live_vreg, i.trial_active),
        (0x0D, 0x0D, false)
    );
}

#[test]
fn wp_cg11_pass_then_end_returns_to_the_persisted_voltage_and_saves_nothing() {
    let r = Rig::stock();
    r.soak_at(0x0D);
    r.pass_soak();
    r.core.core_vreg_trial_end().unwrap();
    assert_eq!(r.phase(), VregCalPhase::Ended);
    let i = r.info();
    assert_eq!(
        (i.stored_sel, i.live_vreg, i.trial_active),
        (0, 0x0C, false)
    );
    assert_eq!(r.sent_ops(SET), 0);
}

#[test]
fn wp_cg11_fail_step_up_and_stop_at_the_cap() {
    let r = Rig::stock();
    r.soak_at(0x0C);
    r.remote.vreg_sim(VregSim::Advance(100));
    r.poll();
    r.remote.vreg_sim(VregSim::Hang);
    let v = r.poll();
    assert_eq!(v.phase, VregCalPhase::Failed);
    assert_eq!(v.failed_vreg, 0x0C);
    assert!(
        v.ran_s.unwrap() >= 100 && v.ran_s.unwrap() < 100 + MARGIN,
        "the trial's own time, not the {} s of uptime the device reports",
        v.uptime_at_hang_s
    );
    assert!(v.uptime_at_hang_s > 3600);
    assert_eq!(v.next_vreg, 0x0D);
    assert!(!v.hardware_problem);
    assert!(v.can_start);
    // The device is back on its persisted voltage; the hang did not count.
    let i = r.info();
    assert_eq!((i.live_vreg, i.hang_count, i.auto_raised), (0x0C, 0, false));

    // A step up is the user's call: nothing goes out by itself.
    let before = r.sent();
    for _ in 0..5 {
        assert_eq!(r.phase(), VregCalPhase::Failed);
    }
    assert_eq!(r.sent(), before);

    // Next step: 1.20 V, no typing.
    r.core.core_vreg_cal_start(0x0D, SOAK, None).unwrap();
    assert_eq!(r.phase(), VregCalPhase::StartingTrial);
    assert_eq!(r.phase(), VregCalPhase::Soaking);
    r.remote.vreg_sim(VregSim::Hang);
    let v = r.poll();
    assert_eq!((v.failed_vreg, v.next_vreg), (0x0D, 0x0E));

    // 1.25 V: typed.
    assert!(no_word(r.core.core_vreg_cal_start(0x0E, SOAK, None)));
    r.core
        .core_vreg_cal_start(0x0E, SOAK, Some(WORD.into()))
        .unwrap();
    r.poll();
    assert_eq!(r.phase(), VregCalPhase::Soaking);
    r.remote.vreg_sim(VregSim::Hang);
    let v = r.poll();
    assert_eq!(v.phase, VregCalPhase::Failed);
    assert_eq!((v.failed_vreg, v.next_vreg), (0x0E, 0));
    assert!(v.hardware_problem);
    assert!(v.message.contains("hardware problem"));

    // There is no way further up.
    assert!(invalid(r.core.core_vreg_cal_start(0x0F, SOAK, word())));
    let trials: Vec<u8> = r
        .sent()
        .iter()
        .filter(|(c, _)| *c == TRIAL)
        .map(|(_, v)| *v)
        .collect();
    assert_eq!(trials, vec![0x0C, 0x0D, 0x0E]);
}

#[test]
fn wp_cg11_a_power_cycle_or_reboot_is_unknown_and_can_run_again() {
    for sim in [VregSim::PowerCycle, VregSim::Reboot] {
        let r = Rig::new();
        r.soak_at(0x0D);
        r.remote.vreg_sim(sim);
        let v = r.poll();
        assert_eq!(v.phase, VregCalPhase::Unknown, "{sim:?}");
        assert_eq!(v.unknown_reason.as_deref(), Some("record_gone"));
        assert!(v.can_start);
        assert_eq!(r.info().live_vreg, 0x0D);
        r.core.core_vreg_cal_start(0x0D, SOAK, None).unwrap();
        r.poll();
        assert_eq!(r.phase(), VregCalPhase::Soaking);
    }
}

#[test]
fn wp_cg11_abort_sends_trial_end_and_returns() {
    let r = Rig::stock();
    r.soak_at(0x0D);
    r.core.core_vreg_trial_end().unwrap();
    assert_eq!(*r.sent().last().unwrap(), (TRIAL_END, 0));
    assert_eq!(r.phase(), VregCalPhase::Ended);
    let i = r.info();
    assert_eq!((i.live_vreg, i.trial_active), (0x0C, false));
    assert!(r.core.core_vreg_cal_close().is_ok());
    assert_eq!(r.phase(), VregCalPhase::Idle);
}

#[test]
fn wp_cg11_an_abort_with_no_device_changes_nothing() {
    let r = Rig::new();
    r.soak_at(0x0D);
    let before = r.sent();
    r.core.lock().conn = None;
    assert_eq!(r.core.core_vreg_trial_end(), Err(DspiError::Disconnected));
    assert_eq!(r.core.core_vreg_cal_view().phase, VregCalPhase::Soaking);
    assert_eq!(r.sent(), before);
    r.attach("CG11");
    assert_eq!(r.phase(), VregCalPhase::Soaking);
}

// ------------------------------------------------------- going away

#[test]
fn wp_cg11_a_drop_mid_trial_keeps_the_soak_and_sends_nothing() {
    let r = Rig::new();
    r.soak_at(0x0D);
    r.remote.vreg_sim(VregSim::Advance(SOAK - MARGIN));
    r.poll();
    let before = r.sent();

    r.unplug();
    let v = r.poll();
    assert_eq!(v.phase, VregCalPhase::Soaking);
    assert!(v.link_lost);
    assert!(!v.available);

    r.attach("CG11");
    let v = r.poll();
    assert_eq!(v.phase, VregCalPhase::Soaking);
    assert!(!v.link_lost);
    assert!(v.elapsed_s >= SOAK - MARGIN, "the soak carried on");
    assert_eq!(r.sent(), before, "no write is replayed");

    r.remote.vreg_sim(VregSim::Advance(MARGIN));
    assert_eq!(r.phase(), VregCalPhase::Passed);
    assert_eq!(r.sent(), before);
}

#[test]
fn wp_cg11_a_hang_during_a_drop_is_reported_on_return() {
    let r = Rig::new();
    r.soak_at(0x0D);
    r.unplug();
    assert_eq!(r.phase(), VregCalPhase::Soaking);
    r.remote.vreg_sim(VregSim::Hang);
    r.attach("CG11");
    let v = r.poll();
    assert_eq!(v.phase, VregCalPhase::Failed);
    assert_eq!(v.failed_vreg, 0x0D);
}

#[test]
fn wp_cg11_a_reconnect_before_the_trial_went_out_does_not_send_it() {
    // Without noticing the unplug in between: a new connection is enough.
    let r = Rig::new();
    r.core.core_vreg_cal_open().unwrap();
    r.core.core_vreg_cal_start(0x0D, SOAK, None).unwrap();
    r.attach("CG11");
    let v = r.poll();
    assert_eq!(v.phase, VregCalPhase::Unknown);
    assert_eq!(v.unknown_reason.as_deref(), Some("not_started"));
    assert_eq!(r.sent(), vec![(CLEAR, 0)]);
}

#[test]
fn wp_cg11_an_app_restart_finds_the_trial_and_offers_end_or_keep() {
    let r = Rig::new();
    r.soak_at(0x0D);
    r.remote.vreg_sim(VregSim::Advance(2 * SOAK));
    let before = r.sent();

    let r2 = r.restart_app();
    let v = r2.poll();
    assert_eq!(v.phase, VregCalPhase::TrialFound);
    assert_eq!(v.target_vreg, 0x0D);
    assert!(v.can_keep && !v.can_commit, "uptime alone is no pass");
    for _ in 0..3 {
        r2.poll();
    }
    assert_eq!(r.sent(), before, "finding it writes nothing");

    // Keep: the soak counts from now.
    r2.core.core_vreg_cal_keep(SOAK).unwrap();
    assert_eq!(r2.phase(), VregCalPhase::Soaking);
    r2.remote.vreg_sim(VregSim::Advance(SOAK - MARGIN));
    assert_eq!(r2.phase(), VregCalPhase::Soaking);
    r2.remote.vreg_sim(VregSim::Advance(MARGIN));
    assert_eq!(r2.phase(), VregCalPhase::Passed);
    assert_eq!(r.sent(), before);

    // End, from another restart.
    let r3 = r.restart_app();
    assert_eq!(r3.phase(), VregCalPhase::TrialFound);
    r3.core.core_vreg_trial_end().unwrap();
    assert_eq!(r3.phase(), VregCalPhase::Ended);
    assert!(!r3.info().trial_active);
}

#[test]
fn wp_cg11_a_trial_that_hung_while_the_app_was_gone_is_on_record() {
    let r = Rig::new();
    r.soak_at(0x0D);
    r.remote.vreg_sim(VregSim::Hang);
    let r2 = r.restart_app();
    let v = r2.poll();
    assert_eq!(v.phase, VregCalPhase::Idle, "nothing starts by itself");
    assert_eq!(v.failed_vreg, 0x0D);
    assert_eq!(v.next_vreg, 0x0E);
    assert_eq!(v.ran_s, None, "this app did not see it start");
    assert!(v.uptime_at_hang_s >= 3600);
}

#[test]
fn wp_cg11_another_device_starts_the_flow_over() {
    let r = Rig::new();
    r.soak_at(0x0D);
    r.remote.vreg_sim(VregSim::PowerCycle);
    r.attach("OTHER");
    assert_eq!(r.phase(), VregCalPhase::Idle);
}

// ------------------------------------------------------- what is refused

#[test]
fn wp_cg11_read_only_and_absent_hide_it_with_no_writes() {
    let read_only = probed(image_as(99, 256), k340_options(), Faults::default());
    let stock = probed(
        dspi_transport::Capture::demo().bulk,
        dspi_transport::FakeOptions::default(),
        Faults::default(),
    );
    let viewer = probed(
        dspi_transport::Capture::demo().bulk,
        k340_options(),
        Faults {
            deny_ext_reads_from: Some(0),
            ..Faults::default()
        },
    );
    for (p, want) in [
        (&read_only, DspiError::ReadOnly),
        (&stock, DspiError::Unsupported),
        (&viewer, DspiError::Unsupported),
    ] {
        let c = &p.core;
        let outs = p.outs.load(Ordering::SeqCst);
        let v = c.core_vreg_cal_poll();
        assert!(!v.available, "{want:?}");
        assert_eq!(v.phase, VregCalPhase::Idle);
        assert_eq!(c.core_vreg_cal_open(), Err(want.clone()));
        assert_eq!(c.core_vreg_cal_start(0x0D, SOAK, None), Err(want.clone()));
        assert_eq!(c.core_vreg_manual_set(0, None), Err(want.clone()));
        assert_eq!(c.core_vreg_manual_trial(0x0D, None), Err(want.clone()));
        assert_eq!(c.core_vreg_manual_auto(false), Err(want.clone()));
        assert_eq!(c.core_vreg_manual_clear(), Err(want.clone()));
        assert_eq!(p.outs.load(Ordering::SeqCst), outs, "no write for {want:?}");
    }
}

#[test]
fn wp_cg11_nothing_connected_is_disconnected() {
    let c = DspiCore::new();
    assert!(!c.core_vreg_cal_poll().available);
    assert_eq!(c.core_vreg_cal_open(), Err(DspiError::Disconnected));
    assert_eq!(c.core_vreg_trial_end(), Err(DspiError::Disconnected));
    assert_eq!(
        c.core_vreg_demo_event(VregDemoEvent::Hang, 0),
        Err(DspiError::Disconnected)
    );
}

#[test]
fn wp_cg11_a_hub_refusing_the_write_hides_the_flow_until_the_next_connection() {
    let r = Rig::new();
    r.core.core_vreg_cal_open().unwrap();
    r.wire.deny_writes.store(true, Ordering::SeqCst);
    assert_eq!(
        r.core.core_vreg_cal_start(0x0D, SOAK, None),
        Err(DspiError::Denied)
    );
    let v = r.poll();
    assert!(!v.available);
    assert_eq!(v.phase, VregCalPhase::Idle);
    assert_eq!(r.core.core_vreg_cal_open(), Err(DspiError::Denied));
    assert!(r.sent().is_empty());
    r.wire.deny_writes.store(false, Ordering::SeqCst);
    r.attach("CG11");
    assert!(r.poll().available);
}

#[test]
fn wp_cg11_it_never_trials_below_the_build_default_or_above_1v25() {
    let r = Rig::new(); // default 1.20 V
    r.core.core_vreg_cal_open().unwrap();
    for (code, typed) in [
        (0x0C, None),
        (0x0B, None),
        (0x00, None),
        (0x0F, Some(WORD)),
        (0x10, None),
        (0xFF, None),
    ] {
        let t = typed.map(str::to_owned);
        assert!(
            invalid(r.core.core_vreg_cal_start(code, SOAK, t.clone())),
            "{code:#x}"
        );
        assert!(invalid(r.core.core_vreg_manual_trial(code, t)), "{code:#x}");
    }
    assert!(r.sent().is_empty(), "the K340 never tries 1.15 V");
    assert_eq!(r.info().live_vreg, 0x0D);

    // Manual SET: the same limits, and 0 still resets.
    for (code, typed) in [(0x0C, None), (0x0F, Some(WORD))] {
        assert!(invalid(
            r.core.core_vreg_manual_set(code, typed.map(str::to_owned))
        ));
    }
    assert!(no_word(r.core.core_vreg_manual_set(0x0E, None)));
    assert!(r.sent().is_empty());

    // 1.30 V is not on the menu anywhere.
    let v = r.poll();
    assert_eq!(
        v.choices.iter().map(|c| c.code).collect::<Vec<_>>(),
        vec![0x0D, 0x0E]
    );
    assert!(!v.choices[0].needs_word);
    assert!(v.choices[1].needs_word);
    assert_eq!(v.confirmation_word, "OVERVOLT");

    let s = Rig::stock(); // default 1.15 V: its own default is fair
    assert_eq!(
        s.poll().choices.iter().map(|c| c.code).collect::<Vec<_>>(),
        vec![0x0C, 0x0D, 0x0E]
    );
    s.soak_at(0x0C);
    assert_eq!(s.sent(), vec![(CLEAR, 0), (TRIAL, 0x0C)]);
}

#[test]
fn wp_cg11_a_build_default_above_the_flow_offers_nothing() {
    let r = Rig::new();
    r.remote.vreg_set_build_default(0x0F);
    let v = r.poll();
    assert!(v.choices.is_empty());
    r.core.core_vreg_cal_open().unwrap();
    assert!(invalid(r.core.core_vreg_cal_start(0x0F, SOAK, word())));
    assert!(r.sent().is_empty());
}

#[test]
fn wp_cg14_the_flow_asks_only_when_it_raises() {
    let r = Rig::new();
    r.core.core_vreg_cal_open().unwrap();
    for typed in [
        None,
        Some("yes"),
        Some("overvolt"),
        Some("OVERVOLT "),
        Some(""),
    ] {
        let t = typed.map(str::to_owned);
        assert!(no_word(r.core.core_vreg_cal_start(0x0E, SOAK, t.clone())));
        assert!(no_word(r.core.core_vreg_manual_trial(0x0E, t.clone())));
        assert!(no_word(r.core.core_vreg_manual_set(0x0E, t)));
    }
    assert!(r.sent().is_empty());

    r.soak_at(0x0E);
    r.pass_soak();
    // The word was typed for the trial; saving it asks again.
    let v = r.poll();
    assert!(v.commit_needs_word);
    assert_eq!(v.confirmation_word, "OVERVOLT");
    let choice = v.choices.iter().find(|c| c.code == 0x0E).unwrap();
    assert!(!choice.needs_word && choice.save_needs_word);
    let before = r.sent();
    assert!(no_word(r.core.core_vreg_cal_commit(None)));
    assert!(no_word(
        r.core.core_vreg_cal_commit(Some("overvolt".into()))
    ));
    assert_eq!(r.sent(), before, "refused at the transport too");
    let v = r.poll();
    assert_eq!(v.phase, VregCalPhase::Passed, "still there to answer");
    r.core.core_vreg_cal_commit(word()).unwrap();
    assert_eq!(*r.sent().last().unwrap(), (SET, 0x0E));
    assert_eq!(r.phase(), VregCalPhase::Committed);
    assert!(!r.poll().commit_needs_word);
    assert_eq!(r.info().stored_sel, 0x0E);

    // Now at 1.25 V: trial and save it again with no word.
    assert!(!r.poll().choices.iter().any(|c| c.needs_word));
    r.core.core_vreg_manual_trial(0x0E, None).unwrap();
    assert_eq!(r.phase(), VregCalPhase::TrialFound);
    r.core.core_vreg_manual_set(0x0E, None).unwrap();
    assert_eq!(r.phase(), VregCalPhase::Committed);
}

// ------------------------------------------------- CG14: the raw request

/// A unit at `code`: stored there, on a build default of 1.15 V.
fn unit_at(code: u8) -> Rig {
    let r = Rig::stock();
    if code != 0x0C {
        r.core.core_vreg_op(SET, code, word()).unwrap();
    }
    r.wire.log.lock().unwrap().clear();
    r
}

#[test]
fn wp_cg14_the_owners_example_1v25_to_1v30_at_the_transport() {
    for op in [SET, TRIAL] {
        let r = unit_at(0x0E);
        assert!(no_word(r.core.core_vreg_op(op, 0x0F, None)));
        assert!(r.sent().is_empty(), "refused: nothing written");
        r.core.core_vreg_op(op, 0x0F, word()).unwrap();
        assert_eq!(r.sent(), vec![(op, 0x0F)]);
        assert_eq!(r.info().live_vreg, 0x0F);
    }
}

#[test]
fn wp_cg14_each_case_at_the_transport() {
    for (from, to, needs) in [
        (0x0C, 0x0D, false), // 1.15 to 1.20
        (0x0D, 0x0E, true),  // 1.20 to 1.25
        (0x0C, 0x0E, true),  // 1.15 to 1.25
        (0x0E, 0x0E, false), // 1.25 to 1.25
        (0x0F, 0x0E, false), // 1.30 to 1.25
        (0x0F, 0x0F, false), // 1.30 to 1.30
    ] {
        for op in [SET, TRIAL] {
            let r = unit_at(from);
            let without = r.core.core_vreg_op(op, to, None);
            if needs {
                assert!(no_word(without), "{from:#x} to {to:#x}");
                assert!(r.sent().is_empty());
                r.core.core_vreg_op(op, to, word()).unwrap();
            } else {
                without.unwrap();
            }
            assert_eq!(r.sent(), vec![(op, to)], "{from:#x} to {to:#x}");
        }
    }
}

#[test]
fn wp_cg14_a_running_trial_counts_as_current() {
    let r = unit_at(0x0C);
    r.core.core_vreg_op(TRIAL, 0x0E, word()).unwrap();
    // A TRIAL counts it: no raise.
    r.core.core_vreg_op(TRIAL, 0x0E, None).unwrap();
    r.core.core_vreg_op(TRIAL, 0x0D, None).unwrap();
    r.core.core_vreg_op(TRIAL, 0x0E, None).unwrap_err();
    r.core.core_vreg_op(TRIAL, 0x0E, word()).unwrap();
    // A SET does not: RAM to flash is a raise, at the transport too.
    let before = r.sent();
    assert!(no_word(r.core.core_vreg_op(SET, 0x0E, None)));
    assert_eq!(r.sent(), before);
    assert_eq!(r.info().stored_sel, 0);
    r.core.core_vreg_op(SET, 0x0E, word()).unwrap();
    assert_eq!(r.info().stored_sel, 0x0E);
    // Stored now: another trial at it, then a save, is level.
    r.core.core_vreg_op(TRIAL, 0x0E, None).unwrap();
    r.core.core_vreg_op(SET, 0x0E, None).unwrap();
}

#[test]
fn wp_cg14_a_kept_trial_nobody_here_confirmed_asks_before_it_is_saved() {
    let r = Rig::new();
    r.core.core_vreg_op(TRIAL, 0x0E, word()).unwrap();
    let r2 = r.restart_app();
    assert_eq!(r2.phase(), VregCalPhase::TrialFound);
    r2.core.core_vreg_cal_keep(SOAK).unwrap();
    r2.pass_soak();
    assert!(r2.poll().commit_needs_word);
    assert!(no_word(r2.core.core_vreg_cal_commit(None)));
    assert_eq!(r2.info().stored_sel, 0);
}

#[test]
fn wp_cg14_a_wrong_word_is_refused_at_the_transport() {
    for bad in [
        "overvolt",
        "Overvolt",
        "OVERVOLT ",
        " OVERVOLT",
        "",
        "--force",
        "yes",
    ] {
        let r = unit_at(0x0E);
        assert!(
            no_word(r.core.core_vreg_op(SET, 0x0F, Some(bad.into()))),
            "{bad:?}"
        );
        assert!(r.sent().is_empty(), "{bad:?}");
    }
}

#[test]
fn wp_cg14_an_unreadable_unit_needs_the_word_above_1v20() {
    // A short reply, then one whose fields are not voltages.
    for reply in [vec![1u8, 2], vec![0xFF; 16]] {
        let r = Rig::new();
        *r.wire.status.lock().unwrap() = Some(reply);
        assert!(no_word(r.core.core_vreg_op(SET, 0x0E, None)));
        assert!(no_word(r.core.core_vreg_op(TRIAL, 0x0F, None)));
        assert!(
            no_word(r.core.core_vreg_op(SET, 0, None)),
            "no known target"
        );
        assert!(r.sent().is_empty());
        r.core.core_vreg_op(TRIAL, 0x0D, None).unwrap();
        r.core.core_vreg_op(TRIAL, 0x0E, word()).unwrap();
        assert_eq!(r.sent(), vec![(TRIAL, 0x0D), (TRIAL, 0x0E)]);
    }
}

#[test]
fn wp_cg14_a_refusal_costs_only_the_status_read_and_no_host_record_helps() {
    let r = unit_at(0x0E);
    // Typed once for this unit, and it comes back down: asked again.
    r.core.core_vreg_op(SET, 0x0F, word()).unwrap();
    r.core.core_vreg_op(SET, 0x0E, None).unwrap();
    assert!(no_word(r.core.core_vreg_op(SET, 0x0F, None)));
    // A new app, the same unit: judged the same, from the unit.
    let again = r.restart_app();
    assert!(no_word(again.core.core_vreg_op(SET, 0x0F, None)));
    // Others need no read and no word.
    r.core.core_vreg_op(TRIAL_END, 0, None).unwrap();
    r.core.core_vreg_op(AUTO, 1, None).unwrap();
    r.core.core_vreg_op(CLEAR, 0, None).unwrap();
}

#[test]
fn wp_cg14_a_raw_write_cannot_go_round_the_word() {
    let r = unit_at(0x0E);
    assert!(invalid(r.core.raw_set(
        op::REQ_EXTENSION,
        g::EXT_CORE_VREG,
        vec![SET, 0x0F]
    )));
    assert!(invalid(r.core.raw_set(
        op::REQ_EXTENSION,
        g::EXT_CORE_VREG,
        vec![TRIAL, 0x0F]
    )));
    assert!(r.sent().is_empty());
    // The status can still be read raw.
    assert_eq!(
        r.core
            .raw_get(op::REQ_EXTENSION, g::EXT_CORE_VREG, 16)
            .unwrap()
            .len(),
        16
    );
}

#[test]
fn wp_cg14_1v30_stays_out_of_the_flow_even_with_the_word() {
    let r = unit_at(0x0E);
    assert!(invalid(r.core.core_vreg_manual_set(0x0F, word())));
    assert!(invalid(r.core.core_vreg_manual_trial(0x0F, word())));
    r.core.core_vreg_cal_open().unwrap();
    assert!(invalid(r.core.core_vreg_cal_start(0x0F, SOAK, word())));
    assert!(r.sent().is_empty());
    assert!(r.poll().choices.iter().all(|c| c.code <= 0x0E));
}

#[test]
fn wp_cg11_the_soak_has_a_floor() {
    let r = Rig::new();
    r.core.core_vreg_cal_open().unwrap();
    assert!(invalid(r.core.core_vreg_cal_start(
        0x0D,
        cal::MIN_SOAK_S - 1,
        None
    )));
    r.core
        .core_vreg_cal_start(0x0D, cal::MIN_SOAK_S, None)
        .unwrap();
    assert_eq!(r.poll().soak_s, cal::MIN_SOAK_S);
}

#[test]
fn wp_cg11_nothing_runs_without_being_opened_or_during_a_run() {
    let r = Rig::new();
    assert!(invalid(r.core.core_vreg_cal_start(0x0D, SOAK, None)));
    assert!(invalid(r.core.core_vreg_cal_commit(None)));
    assert!(invalid(r.core.core_vreg_cal_keep(SOAK)));
    assert!(r.sent().is_empty());

    r.soak_at(0x0D);
    let before = r.sent();
    assert!(invalid(r.core.core_vreg_cal_open()));
    assert!(invalid(r.core.core_vreg_cal_close()));
    assert!(invalid(r.core.core_vreg_cal_start(0x0D, SOAK, None)));
    assert!(invalid(r.core.core_vreg_manual_set(0, None)));
    assert!(invalid(r.core.core_vreg_manual_trial(0x0D, None)));
    assert!(invalid(r.core.core_vreg_manual_clear()));
    assert_eq!(r.sent(), before);
    assert_eq!(r.phase(), VregCalPhase::Soaking);
}

#[test]
fn wp_cg11_polling_never_sends_a_set_whatever_happens() {
    let r = Rig::stock();
    r.core.core_vreg_cal_open().unwrap();
    for sim in [
        VregSim::Advance(5000),
        VregSim::Hang,
        VregSim::Hang,
        VregSim::Hang,
        VregSim::Reboot,
        VregSim::PowerCycle,
    ] {
        r.remote.vreg_sim(sim);
        for _ in 0..3 {
            r.poll();
        }
    }
    r.soak_at(0x0D);
    r.remote.vreg_sim(VregSim::Advance(5 * SOAK));
    for _ in 0..5 {
        r.poll();
    }
    for sim in [VregSim::Hang, VregSim::Advance(SOAK), VregSim::PowerCycle] {
        r.remote.vreg_sim(sim);
        for _ in 0..3 {
            r.poll();
        }
    }
    assert_eq!(r.sent_ops(SET), 0);
    assert_eq!(r.sent_ops(AUTO), 0);
    assert_eq!(r.sent_ops(TRIAL), 1);
}

// ------------------------------------------------------- read back

#[test]
fn wp_cg11_each_step_waits_for_the_main_loop() {
    let r = Rig::new();
    // One hang on record, shown before anything is cleared.
    r.remote.vreg_sim(VregSim::Hang);
    r.core.core_vreg_cal_open().unwrap();
    let v = r.poll();
    assert_eq!(v.phase, VregCalPhase::Review);
    assert_eq!(v.status.unwrap().hang_count, 1);
    assert!(r.sent().is_empty(), "shown first; cleared at the start");

    r.remote.vreg_stage_requests(true);
    r.core.core_vreg_cal_start(0x0D, SOAK, None).unwrap();
    assert_eq!(r.phase(), VregCalPhase::Clearing);
    assert_eq!(r.phase(), VregCalPhase::Clearing);
    assert_eq!(r.sent(), vec![(CLEAR, 0)], "the slot still holds the CLEAR");

    r.remote.vreg_run_main_loop();
    assert_eq!(r.phase(), VregCalPhase::StartingTrial);
    assert_eq!(r.phase(), VregCalPhase::StartingTrial);
    r.remote.vreg_run_main_loop();
    assert_eq!(r.phase(), VregCalPhase::Soaking);

    r.pass_soak();
    r.core.core_vreg_cal_commit(None).unwrap();
    assert_eq!(r.phase(), VregCalPhase::Committing);
    r.remote.vreg_run_main_loop();
    assert_eq!(r.phase(), VregCalPhase::Committed);

    // Reset is SET 0, read back the same way.
    r.core.core_vreg_manual_set(0, None).unwrap();
    assert_eq!(*r.sent().last().unwrap(), (SET, 0));
    assert_eq!(r.phase(), VregCalPhase::Committing);
    r.remote.vreg_run_main_loop();
    assert_eq!(r.phase(), VregCalPhase::Committed);
    assert_eq!(r.info().stored_sel, 0);
}

#[test]
fn wp_cg11_a_failed_write_is_not_retried_and_says_why() {
    let c = probed(
        dspi_transport::Capture::demo().bulk,
        k340_options(),
        Faults {
            stall_ext_writes: true,
            ..Faults::default()
        },
    );
    assert!(matches!(
        c.core
            .core_vreg_cal_open()
            .and_then(|()| c.core.core_vreg_cal_start(0x0D, SOAK, None)),
        Err(DspiError::Refused { .. })
    ));
    let outs = c.outs.load(Ordering::SeqCst);
    let v = c.core.core_vreg_cal_poll();
    assert_eq!(v.phase, VregCalPhase::Unknown);
    assert_eq!(v.unknown_reason.as_deref(), Some("not_started"));
    assert_eq!(c.outs.load(Ordering::SeqCst), outs);
}

// ------------------------------------------------------- manual, and view

#[test]
fn wp_cg11_manual_requests_reach_the_device_with_a_view_of_them() {
    let r = Rig::new();
    r.core.core_vreg_manual_auto(false).unwrap();
    assert!(r.info().auto_disabled);
    r.core.core_vreg_manual_auto(true).unwrap();
    assert!(!r.info().auto_disabled);

    r.core.core_vreg_manual_trial(0x0E, word()).unwrap();
    let v = r.poll();
    assert_eq!(v.phase, VregCalPhase::TrialFound);
    r.core.core_vreg_trial_end().unwrap();
    assert_eq!(r.phase(), VregCalPhase::Ended);

    r.remote.vreg_sim(VregSim::Hang);
    assert_eq!(r.poll().status.unwrap().hang_count, 1);
    r.core.core_vreg_manual_clear().unwrap();
    assert_eq!(r.poll().status.unwrap().hang_count, 0);
}

#[test]
fn wp_cg11_the_warning_states_the_limits() {
    let r = Rig::new();
    let v = r.poll();
    for needed in [
        "1.16 V",
        "1.21 V",
        "3 %",
        "1.20 V",
        "1.25 V",
        "Audio may cut out",
        "drop off USB",
        "lower the volume",
        "OVERVOLT",
        "1.24 V",
    ] {
        assert!(v.warning.contains(needed), "{needed}");
    }
    assert!(v.soak_note.contains("short soak proves little"));
    assert_eq!((v.default_soak_s, v.min_soak_s), (1800, 600));
}

#[test]
fn wp_cg11_the_fake_models_core_vreg_c() {
    let r = Rig::stock();
    // A SET ends a trial.
    r.core.core_vreg_op(TRIAL, 0x0E, word()).unwrap();
    r.core.core_vreg_op(SET, 0x0D, None).unwrap();
    let i = r.info();
    assert_eq!((i.trial_active, i.live_vreg), (false, 0x0D));
    // Two hangs raise the stored voltage one notch.
    r.remote.vreg_sim(VregSim::Hang);
    assert_eq!(r.info().hang_count, 1);
    r.remote.vreg_sim(VregSim::Hang);
    let i = r.info();
    assert_eq!(
        (i.stored_sel, i.live_vreg, i.auto_raised),
        (0x0E, 0x0E, true)
    );
    assert_eq!(i.hang_count, 0);
    // A further warm reset keeps a failed trial's code but not its uptime.
    r.core.core_vreg_op(TRIAL, 0x0D, None).unwrap();
    r.remote.vreg_sim(VregSim::Advance(50));
    r.remote.vreg_sim(VregSim::Hang);
    assert_ne!(r.info().failed_trial_uptime_s, 0);
    r.remote.vreg_sim(VregSim::Reboot);
    let i = r.info();
    assert_eq!((i.failed_trial_vreg, i.failed_trial_uptime_s), (0x0D, 0));
    // A power cycle takes it all.
    r.remote.vreg_sim(VregSim::PowerCycle);
    assert_eq!(r.info().failed_trial_vreg, 0);
}

#[test]
fn wp_cg11_the_demo_device_plays_a_whole_flow_with_events() {
    let c = DspiCore::new();
    c.open_demo().unwrap();
    assert!(c.core_vreg_cal_poll().available);
    c.core_vreg_cal_open().unwrap();
    c.core_vreg_cal_start(0x0D, SOAK, None).unwrap();
    c.core_vreg_cal_poll();
    assert_eq!(c.core_vreg_cal_poll().phase, VregCalPhase::Soaking);
    c.core_vreg_demo_event(VregDemoEvent::AdvanceUptime, SOAK)
        .unwrap();
    assert_eq!(c.core_vreg_cal_poll().phase, VregCalPhase::Passed);
    c.core_vreg_demo_event(VregDemoEvent::Hang, 0).unwrap();
    let v = c.core_vreg_cal_poll();
    assert_eq!((v.phase, v.failed_vreg), (VregCalPhase::Failed, 0x0D));
    c.core_vreg_cal_start(0x0D, SOAK, None).unwrap();
    c.core_vreg_cal_poll();
    c.core_vreg_cal_poll();
    c.core_vreg_demo_event(VregDemoEvent::PowerCycle, 0)
        .unwrap();
    assert_eq!(c.core_vreg_cal_poll().phase, VregCalPhase::Unknown);
    c.core_vreg_demo_event(VregDemoEvent::Reboot, 0).unwrap();
}

#[test]
fn wp_cg11_demo_events_go_only_to_a_simulated_device() {
    let dev = FakeDevice::new(dspi_transport::Capture::demo(), k340_options());
    let wire: Arc<Wire> = Arc::default();
    let plug = Plug {
        descriptor: dev.descriptor().clone(),
        caps: TransportCaps::USB,
        dev: Arc::new(Mutex::new(dev)),
        wire: wire.clone(),
    };
    let c = DspiCore::new();
    c.attach_probed(Box::new(plug), "REAL".into(), 0).unwrap();
    assert_eq!(
        c.core_vreg_demo_event(VregDemoEvent::Hang, 0),
        Err(DspiError::Unsupported)
    );
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(128))]

    /// Whatever the device reports, nothing panics, and every write that goes
    /// out stays inside the limits for what it reported.
    #[test]
    fn proptest_wp_cg11_any_status_is_survived_and_stays_in_the_limits(
        replies in proptest::collection::vec(any::<[u8; 16]>(), 1..12),
        targets in proptest::collection::vec(any::<u8>(), 1..12),
    ) {
        let r = Rig::new();
        for (b, target) in replies.iter().zip(targets.iter().cycle()) {
            *r.wire.status.lock().unwrap() = Some(b.to_vec());
            let before = r.sent().len();
            let _ = r.core.core_vreg_cal_open();
            let typed = word();
            let _ = r.core.core_vreg_cal_start(*target, SOAK, typed.clone());
            r.poll();
            let _ = r.core.core_vreg_cal_commit(typed.clone());
            let _ = r.core.core_vreg_manual_set(*target, typed.clone());
            let _ = r.core.core_vreg_manual_trial(*target, typed);
            let _ = r.core.core_vreg_cal_keep(SOAK);
            let _ = r.core.core_vreg_trial_end();
            let _ = r.core.core_vreg_cal_view();
            r.poll();
            // Each write was decided on this iteration's status.
            for (code, v) in &r.sent()[before..] {
                if *code == TRIAL {
                    prop_assert!(cal::in_flow_range(*v, b[2]), "trial {v:#x} on {b:?}");
                }
                if *code == SET {
                    prop_assert!(*v == 0 || cal::in_flow_range(*v, b[2]), "set {v:#x} on {b:?}");
                }
            }
        }
    }
}
