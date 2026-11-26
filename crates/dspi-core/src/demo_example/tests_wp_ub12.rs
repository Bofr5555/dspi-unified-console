//! -- WP UB12 -- The demo opens on a tuned example; "Reset demo" puts it back.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use dspi_proto::enums::FilterType;
use dspi_transport::{DeviceDescriptor, FakeDevice, Transport, TransportCaps, TransportKind};

use super::*;
use crate::actor::{ActorScope, register_actor};
use crate::measure::MeasureRole;
use crate::speaker_layout::LayoutState;

fn demo() -> Arc<DspiCore> {
    let core = DspiCore::new();
    core.open_demo().unwrap();
    core.fetch_all().unwrap();
    core
}

fn out(s: &DspState, o: usize) -> usize {
    usize::from(s.num_input_channels) + o
}

/// What a test reads of the example: routing, crossovers, names, levels.
fn assert_example(s: &DspState) {
    let lr4 = |hp| {
        Meta {
            family: Family::LinkwitzRiley,
            order: 4,
            high_pass: hp,
        }
        .to_type()
    };
    for o in 0..4 {
        let ch = out(s, o);
        assert_eq!(s.channel_names[ch], OUTPUT_NAMES[o]);
        assert!(s.output_enabled[o], "output {} on", o + 1);
        assert_eq!(s.xover[ch][0].filter_type, lr4(o >= 2), "output {}", o + 1);
        assert_eq!(s.xover[ch][0].freq, CROSSOVER_HZ);
        assert_eq!(s.filters[ch][0].filter_type, FilterType::Peaking.to_raw());
        assert!(s.filters[ch][0].gain < 0.0, "a cut on output {}", o + 1);
        for i in 0..usize::from(s.num_input_channels) {
            assert_eq!(
                s.matrix_routing[i][o],
                i == o % 2,
                "USB {} to output {}",
                i + 1,
                o + 1
            );
        }
    }
    assert_eq!(s.output_gain_db[2], -3.0);
    assert_eq!(s.output_delay_ms[3], 0.1);
    for i in 0..2 {
        assert_eq!(s.filters[i][0].filter_type, FilterType::Peaking.to_raw());
        assert_eq!(s.filters[i][0].freq, 63.0);
    }
    // Outputs 5-9 play nothing
    for o in 4..usize::from(s.num_output_channels) {
        assert!(
            s.matrix_routing.iter().all(|r| !r[o]),
            "output {} plays",
            o + 1
        );
    }
    assert_eq!(s.master_volume_db, -20.0, "the capture's");
}

fn assert_example_roles(core: &DspiCore) {
    let l = core.speaker_layout().unwrap().expect("the demo's roles");
    assert!(l.confirmed_by_user);
    assert_eq!(l.state(), LayoutState::Confirmed);
    let rows: Vec<_> = l
        .outputs
        .iter()
        .map(|o| (o.output, o.role, o.speaker.as_str(), o.source))
        .collect();
    assert_eq!(rows.len(), 4, "{rows:?}");
    for (o, role) in [
        (0, MeasureRole::Low),
        (1, MeasureRole::Low),
        (2, MeasureRole::High),
        (3, MeasureRole::High),
    ] {
        let r = rows.iter().find(|r| r.0 == o).expect("a row per output");
        assert_eq!(r.1, role, "output {}", o + 1);
        assert_eq!(r.3, RoleSource::User);
    }
    // One speaker per side: the left woofer and tweeter share theirs
    let speaker = |o: u8| rows.iter().find(|r| r.0 == o).map(|r| r.2.to_owned());
    assert_eq!(speaker(0), speaker(2));
    assert_eq!(speaker(1), speaker(3));
    assert_ne!(speaker(0), speaker(1));
}

#[test]
fn wp_ub12_the_demo_opens_on_the_tuned_example() {
    let core = demo();
    let s = core.get_state();
    assert_example(&s);
    assert_example_roles(&core);
    // Its labels are the names; outputs 5-9 are unused
    let labels = core.output_labels();
    for (o, n) in OUTPUT_NAMES.iter().enumerate() {
        assert_eq!(labels[o].label, *n);
        assert!(!labels[o].unused);
    }
    assert!(labels[4..].iter().all(|l| l.unused), "{labels:?}");
    // Saved as preset slot 1, so nothing is unsaved
    assert_eq!(s.active_preset_slot, 0);
    assert!(!core.preset_dirty());
}

#[test]
fn wp_ub12_the_blank_demo_is_the_capture() {
    let core = DspiCore::new();
    core.open_demo_blank().unwrap();
    core.fetch_all().unwrap();
    let s = core.get_state();
    for o in 0..usize::from(s.num_output_channels) {
        assert_eq!(s.xover[out(&s, o)][0].filter_type, 0);
        assert_ne!(s.channel_names[out(&s, o)], OUTPUT_NAMES[o.min(3)]);
    }
    assert!(core.speaker_layout().unwrap().is_none());
}

#[test]
fn wp_ub12_the_tweeters_pass_the_tweeter_floor() {
    // The example never trips the tweeter rule (ROLE0): its high ways are
    // crossed over at or above 1 kHz with at least 12 dB/octave
    let s = demo().get_state();
    for o in 2..4 {
        let x = s.xover[out(&s, o)][0];
        let meta = dspi_proto::xover::meta(x.filter_type).expect("a crossover");
        assert!(
            meta.high_pass && meta.order >= 2,
            "output {}: 12 dB/octave or more",
            o + 1
        );
        assert!(x.freq >= 1000.0, "output {}", o + 1);
    }
}

#[test]
fn wp_ub12_reset_demo_puts_the_example_back() {
    let core = demo();
    let n_in = core.get_state().num_input_channels;
    // The user plays with it: a level, a name, a crossover, a route, a role
    core.set_output_gain(0, -9.0).unwrap();
    core.set_channel_name(n_in, "Mine".into()).unwrap();
    core.set_crossover(n_in + 2, 0, band_raw(0, 1000.0, 0.707, 0.0))
        .unwrap();
    core.set_matrix_route(2, 5, true, 0.0, false).unwrap();
    let mut l = core.speaker_layout().unwrap().unwrap();
    l.outputs[2].role = MeasureRole::Full;
    core.speaker_layout_set(l).unwrap();
    assert!(core.preset_dirty());

    core.reset_demo().unwrap();
    assert_example(&core.get_state());
    assert_example_roles(&core);
    assert!(!core.preset_dirty(), "nothing unsaved after a reset");
    assert_eq!(core.get_state().active_preset_slot, 0);
    // And the device's own idea of its settings is the example too
    core.fetch_all().unwrap();
    assert_example(&core.get_state());
}

#[test]
fn wp_ub12_a_saved_change_is_reset_too() {
    let core = demo();
    core.set_output_gain(1, -6.0).unwrap();
    let t = core.confirm_token(crate::ConfirmOp::SaveParams);
    core.save_params_with(Some(&t)).unwrap();
    core.reset_demo().unwrap();
    assert_example(&core.get_state());
}

#[test]
fn wp_ub12_reset_on_the_blank_demo_gives_the_capture_and_no_roles() {
    let core = DspiCore::new();
    core.open_demo_blank().unwrap();
    core.fetch_all().unwrap();
    let before = core.get_state();
    core.set_output_gain(0, -9.0).unwrap();
    core.reset_demo().unwrap();
    let after = core.get_state();
    assert_eq!(after.output_gain_db, before.output_gain_db);
    assert!(core.speaker_layout().unwrap().is_none());
}

/// The example device reporting itself as a real transport, counting traffic.
struct RealKind {
    dev: FakeDevice,
    calls: Arc<AtomicUsize>,
}

impl Transport for RealKind {
    fn control_in(&mut self, opcode: u8, value: u16, len: u16) -> dspi_transport::Result<Vec<u8>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.dev.control_in(opcode, value, len)
    }
    fn control_in_upto(
        &mut self,
        opcode: u8,
        value: u16,
        max_len: u16,
    ) -> dspi_transport::Result<Vec<u8>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.dev.control_in_upto(opcode, value, max_len)
    }
    fn control_out(&mut self, opcode: u8, value: u16, data: &[u8]) -> dspi_transport::Result<()> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.dev.control_out(opcode, value, data)
    }
    fn descriptor(&self) -> &DeviceDescriptor {
        self.dev.descriptor()
    }
    fn max_transfer(&self) -> usize {
        self.dev.max_transfer()
    }
    fn caps(&self) -> TransportCaps {
        TransportCaps {
            kind: TransportKind::Usb,
            ..self.dev.caps()
        }
    }
}

#[test]
fn wp_ub12_reset_demo_is_for_the_demo_only() {
    let core = DspiCore::new();
    assert_eq!(core.reset_demo(), Err(DspiError::Disconnected));
    // A device that is not simulated: refused with no traffic
    let calls = Arc::<AtomicUsize>::default();
    core.attach_probed(
        Box::new(RealKind {
            dev: device(),
            calls: calls.clone(),
        }),
        "UB12".into(),
        0,
    )
    .unwrap();
    core.set_output_gain(0, -9.0).unwrap();
    let before = calls.load(Ordering::SeqCst);
    assert_eq!(core.reset_demo(), Err(DspiError::Unsupported));
    assert_eq!(calls.load(Ordering::SeqCst), before, "it sent something");
    assert_eq!(core.get_state().output_gain_db[0], -9.0);
}

#[test]
fn wp_ub12_factory_reset_still_gives_the_capture() {
    let core = demo();
    let t = core.confirm_token(crate::ConfirmOp::FactoryReset);
    core.factory_reset_with(Some(&t)).unwrap();
    let s = core.get_state();
    assert_eq!(s.xover[out(&s, 0)][0].filter_type, 0, "the capture's");
}

#[test]
fn wp_ub12_an_agent_may_open_the_demo_and_gets_its_roles() {
    // The example's roles are the demo's, not an agent's word (MPLAN 2.2):
    // opening it inside an agent's scope keeps them all the same
    let core = DspiCore::new();
    let agent = register_actor("AI · UB12");
    {
        let _scope = ActorScope::enter(&core, agent, None);
        core.open_demo().unwrap();
    }
    assert_example_roles(&core);
}

#[test]
fn wp_ub12_reopening_replaces_an_older_layout() {
    // A layout kept for the demo's serial from before the example (the
    // blank demo's "Full" roles) does not contradict the new outputs
    let core = DspiCore::new();
    core.open_demo_blank().unwrap();
    core.fetch_all().unwrap();
    let mut old = core.speaker_layout_propose().unwrap();
    old.confirmed_by_user = true;
    core.speaker_layout_set(old).unwrap();
    core.open_demo().unwrap();
    assert_example_roles(&core);
}
