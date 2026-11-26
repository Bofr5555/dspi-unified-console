//! WP P0W: every typed setter journals its change; undo and redo put it back
//! and forth through the setter, and the device holds what the mirror says.

use std::sync::Arc;

use dspi_proto::registry::REGISTRY;

use super::*;
use crate::consts::*;
use crate::records::{I2cConfig, SiggenConfig, UartConfig};

fn demo() -> Arc<DspiCore> {
    let core = DspiCore::new();
    core.open_demo_blank().unwrap(); // -- WP UB12 --
    core
}

/// The demo device, with the pin and clock-pin requests it does not model
/// answered as the firmware does (status 0) and applied to its image, so a
/// read-back sees them.
struct PinRig {
    inner: dspi_transport::FakeDevice,
    remote: dspi_transport::FakeRemote,
}

impl PinRig {
    /// The image edit for a write-as-read request the fake does not model.
    fn apply(&self, opcode: u8, value: u16) -> bool {
        use dspi_proto::generated::opcodes as op;
        let [hi, lo] = value.to_be_bytes();
        let set = |v: &mut Vec<u8>, i: u8, x: u8| {
            if let Some(slot) = v.get_mut(usize::from(i)) {
                *slot = x;
            }
        };
        let edit: Box<dyn Fn(&mut DspState)> = match opcode {
            op::REQ_SET_OUTPUT_PIN => Box::new(move |s| set(&mut s.output_pins, lo, hi)),
            op::REQ_SET_I2S_BCK_PIN if hi == 0 => Box::new(move |s| s.i2s_bck_pin = lo),
            op::REQ_SET_I2S_BCK_PIN => Box::new(move |s| s.i2s_bck_pin_slave = lo),
            op::REQ_SET_MCK_PIN => Box::new(move |s| s.mck_pin = lo),
            op::REQ_SET_ADAT_PIN => Box::new(move |s| s.adat_out_pin = lo),
            op::REQ_SET_SPDIF_RX_PIN => Box::new(move |s| set(&mut s.spdif_rx_pins, hi, lo)),
            op::REQ_SET_I2S_RX_PIN => Box::new(move |s| set(&mut s.i2s_rx_pins, hi, lo)),
            op::REQ_SET_ADAT_INPUT_PIN => Box::new(move |s| s.adat_input_pin = lo),
            op::REQ_SET_I2S_CLOCK_PIN_MODE => Box::new(move |s| s.i2s_clock_pin_mode = lo),
            op::REQ_TEST_DAC_HW_MUTE => Box::new(|_| {}),
            _ => return false,
        };
        let mut s = crate::state::default_state();
        crate::state::decode_bulk(&self.remote.image(), &mut s).unwrap();
        edit(&mut s);
        self.remote.load_image(crate::state::encode_bulk(&s), 0);
        true
    }
}

impl dspi_transport::Transport for PinRig {
    fn control_in(&mut self, op: u8, value: u16, len: u16) -> dspi_transport::Result<Vec<u8>> {
        if self.apply(op, value) {
            return Ok(vec![0]);
        }
        self.inner.control_in(op, value, len)
    }
    fn control_in_upto(&mut self, op: u8, value: u16, len: u16) -> dspi_transport::Result<Vec<u8>> {
        if self.apply(op, value) {
            return Ok(vec![0]);
        }
        self.inner.control_in_upto(op, value, len)
    }
    fn control_out(&mut self, op: u8, value: u16, data: &[u8]) -> dspi_transport::Result<()> {
        self.inner.control_out(op, value, data)
    }
    fn descriptor(&self) -> &dspi_transport::DeviceDescriptor {
        self.inner.descriptor()
    }
    fn max_transfer(&self) -> usize {
        self.inner.max_transfer()
    }
    fn caps(&self) -> dspi_transport::TransportCaps {
        self.inner.caps()
    }
}

/// [`demo`] behind a [`PinRig`].
fn pin_demo() -> Arc<DspiCore> {
    let inner = dspi_transport::FakeDevice::demo();
    let remote = inner.remote();
    let core = DspiCore::new();
    core.attach_probed(Box::new(PinRig { inner, remote }), "DEMO".into(), 0)
        .unwrap();
    core
}

/// One test per pin setter, on the [`PinRig`].
macro_rules! pin_setter {
    ($name:ident, $path:literal, [$($ix:expr),*], $write:expr) => {
        #[test]
        fn $name() {
            let core = pin_demo();
            check(&core, $path, &[$($ix),*], $write);
        }
    };
}

fn mirror(core: &DspiCore, path: &str, ix: &[u8]) -> Option<Value> {
    mirror_value(&core.lock(), path, ix)
}

fn newest(core: &DspiCore) -> HistoryEntry {
    core.history(1).pop().expect("a journal entry")
}

/// A value setter: `write` moves the mirror's `path` at `ix`, which the
/// journal records with the value before and after. An undoable one goes
/// back with undo (the device too: the image is read again) and forth with
/// redo; any other stays where it is, named by undo.
fn check(core: &DspiCore, path: &str, ix: &[u8], write: impl FnOnce(&DspiCore) -> Result<()>) {
    let before = mirror(core, path, ix);
    let len = core.history(u32::MAX).len();
    write(core).unwrap_or_else(|e| panic!("{path} {ix:?}: {e}"));
    let after = mirror(core, path, ix);
    assert_ne!(before, after, "{path} {ix:?}: the test value must move it");

    assert_eq!(core.history(u32::MAX).len(), len + 1, "{path}: one entry");
    let e = newest(core);
    assert_eq!((e.path.as_str(), e.indices.as_slice()), (path, ix));
    assert_eq!(e.before, before.clone().map(Into::into), "{path}: before");
    assert_eq!(Some(e.after.clone().into()), after, "{path}: after");
    assert_eq!(e.source, HistorySource::Local);
    assert!(e.at_ms > 0);

    let undoable = super::hazard(path) == Hazard::None;
    assert_eq!(e.undoable, undoable, "{path}: the registry decides");
    let mut f = core.process_notifications().flags;
    assert_ne!(f & NOTIFY_HISTORY, 0, "{path}: the history moved");

    let undone = core.undo().unwrap();
    if !undoable {
        assert!(undone.entry.is_none(), "{path}: {undone:?}");
        assert_eq!(undone.skipped.first().map(|s| s.path.as_str()), Some(path));
        assert_eq!(mirror(core, path, ix), after, "{path}: not undone");
        return;
    }
    assert_eq!(undone.entry.map(|e| e.path), Some(path.to_owned()));
    assert_eq!(mirror(core, path, ix), before, "{path}: undo put it back");
    assert!(!core.can_undo() || core.history(u32::MAX).len() > len + 1);
    assert!(core.can_redo(), "{path}");
    let e = newest(core);
    assert_eq!(
        e.source,
        HistorySource::Undo,
        "{path}: the reversal is journalled"
    );
    // The device holds it: read everything again.
    core.fetch_all().unwrap();
    assert_eq!(
        mirror(core, path, ix),
        before,
        "{path}: the device's read-back"
    );
    f = core.process_notifications().flags;
    assert_ne!(f & NOTIFY_HISTORY, 0);

    let redone = core.redo().unwrap();
    assert_eq!(redone.entry.map(|e| e.path), Some(path.to_owned()));
    assert_eq!(
        mirror(core, path, ix),
        after,
        "{path}: redo applied it again"
    );
    assert_eq!(newest(core).source, HistorySource::Redo);
    core.fetch_all().unwrap();
    assert_eq!(
        mirror(core, path, ix),
        after,
        "{path}: the device's read-back"
    );
    assert!(!core.can_redo());
}

/// An action or a write the mirror holds no value for (a save, a test, a
/// record-shaped configuration): journalled with no value before, never
/// undone.
fn check_action(core: &DspiCore, path: &str, write: impl FnOnce(&DspiCore) -> Result<()>) {
    write(core).unwrap_or_else(|e| panic!("{path}: {e}"));
    let e = newest(core);
    assert_eq!(e.path, path);
    assert_eq!(e.before, None, "{path}");
    assert!(!e.undoable, "{path}");
    let undone = core.undo().unwrap();
    assert!(undone.entry.is_none(), "{path}: {undone:?}");
    assert_eq!(undone.skipped.first().map(|s| s.path.as_str()), Some(path));
}

/// One test per value setter.
macro_rules! setter {
    ($name:ident, $path:literal, [$($ix:expr),*], $write:expr) => {
        #[test]
        fn $name() {
            let core = demo();
            check(&core, $path, &[$($ix),*], $write);
        }
    };
}

/// One test per action setter.
macro_rules! action {
    ($name:ident, $path:literal, $write:expr) => {
        #[test]
        fn $name() {
            let core = demo();
            check_action(&core, $path, $write);
        }
    };
}

fn out_ch(core: &DspiCore, output: u8) -> u8 {
    core.get_state().num_input_channels + output
}

/// A free GPIO for a pin move on the demo device.
const SPARE_PIN: u8 = 22;
// -- WP CG-SWS-b --
const SPARE_BCK_PIN: u8 = 18;
const SPARE_MCK_PIN: u8 = 21;

// ── params.rs ───────────────────────────────────────────────────────

setter!(wp_p0w_set_filter_one_field, "eq.freq", [0, 1], |c| {
    let mut p = c.get_state().filters[0][1];
    p.freq = if p.freq == 1234.0 { 2345.0 } else { 1234.0 };
    c.set_filter(0, 1, p)
});
setter!(wp_p0w_set_filter_whole_band, "eq.band", [0, 2], |c| {
    let mut p = c.get_state().filters[0][2];
    p.filter_type = if p.filter_type == 1 { 2 } else { 1 };
    p.freq = 3000.0;
    p.gain = 4.5;
    c.set_filter(0, 2, p)
});
setter!(wp_p0w_set_crossover, "eq.band", [8, 20], |c| {
    let ch = out_ch(c, 0);
    assert_eq!(ch, 8, "the demo's first output channel");
    let p = FilterParams {
        filter_type: 33,
        bypass: false,
        freq: 2500.0,
        q: 0.707,
        gain: 0.0,
        qp: 0.707,
    };
    c.set_crossover(ch, 0, p)
});
setter!(wp_p0w_set_band_bypass, "eq.bypass", [0, 3], |c| {
    let on = c.get_state().filters[0][3].bypass;
    c.set_band_bypass(0, 3, !on)
});
setter!(wp_p0w_set_preamp, "pre.all", [], |c| c.set_preamp(-3.5));
setter!(wp_p0w_set_input_preamp, "pre", [1], |c| c
    .set_input_preamp(1, -4.5));
setter!(wp_p0w_set_delay, "ch.delay", [0], |c| c.set_delay(0, 1.25));
setter!(wp_p0w_set_input_source, "in.source", [], |c| {
    let s = c.get_state().input_source;
    c.set_input_source(if s == 1 { 0 } else { 1 })
});
setter!(wp_p0w_set_matrix_route, "mix", [0, 1], |c| {
    c.set_matrix_route(0, 1, true, -3.3, true)
});
setter!(wp_p0w_set_output_enable, "out.enable", [1], |c| {
    let on = c.get_state().output_enabled[1];
    c.set_output_enable(1, !on).map(drop)
});
setter!(wp_p0w_set_output_gain, "out.gain", [0], |c| c
    .set_output_gain(0, -6.5));
setter!(wp_p0w_set_output_mute, "out.mute", [0], |c| {
    let m = c.get_state().output_muted[0];
    c.set_output_mute(0, !m)
});
setter!(wp_p0w_set_output_delay, "out.delay", [0], |c| c
    .set_output_delay(0, 2.5));
setter!(wp_p0w_set_channel_name, "ch.name", [0], |c| {
    c.set_channel_name(0, "Turntable".into())
});
setter!(wp_p0w_set_user_volume, "vol.user", [], |c| c
    .set_user_volume(-21.0));
setter!(wp_p0w_set_user_mute, "vol.mute", [], |c| {
    let m = c.get_state().user_mute;
    c.set_user_mute(!m)
});
setter!(wp_p0w_set_master_volume, "vol.master", [], |c| c
    .set_master_volume(-10.5));
setter!(wp_p0w_set_master_volume_mode, "vol.master.mode", [], |c| {
    let m = c.get_state().master_volume_mode;
    c.set_master_volume_mode(1 - m.min(1))
});
setter!(wp_p0w_set_bypass, "bypass", [], |c| {
    let b = c.get_state().bypass;
    c.set_bypass(!b)
});
setter!(wp_p0w_set_loudness, "loud.on", [], |c| {
    let b = c.get_state().loudness_enabled;
    c.set_loudness(!b)
});
setter!(wp_p0w_set_loudness_ref, "loud.ref", [], |c| c
    .set_loudness_ref(71.0));
setter!(wp_p0w_set_loudness_intensity, "loud.intensity", [], |c| {
    c.set_loudness_intensity(55.0)
});
setter!(wp_p0w_set_loudness_mask, "loud.mask", [], |c| {
    let m = c.get_state().loudness_output_mask;
    c.set_loudness_mask(m ^ 0b10)
});
setter!(wp_p0w_set_crossfeed, "cf.on", [], |c| {
    let b = c.get_state().crossfeed_enabled;
    c.set_crossfeed(!b)
});
setter!(wp_p0w_set_crossfeed_preset, "cf.preset", [], |c| {
    let p = c.get_state().crossfeed_preset;
    c.set_crossfeed_preset(if p == 2 { 1 } else { 2 })
});
setter!(wp_p0w_set_crossfeed_freq, "cf.freq", [], |c| c
    .set_crossfeed_freq(910.0));
setter!(wp_p0w_set_crossfeed_feed, "cf.feed", [], |c| c
    .set_crossfeed_feed(6.5));
setter!(wp_p0w_set_crossfeed_itd, "cf.itd", [], |c| {
    let b = c.get_state().crossfeed_itd;
    c.set_crossfeed_itd(!b)
});
setter!(wp_p0w_set_crossfeed_outputs, "cf.outputs", [], |c| {
    let m = c.get_state().crossfeed_output_pair_mask;
    c.set_crossfeed_outputs(m ^ 0b10)
});
setter!(wp_p0w_set_leveller_enabled, "lev.on", [], |c| {
    let b = c.get_state().leveller_enabled;
    c.set_leveller_enabled(!b)
});
setter!(wp_p0w_set_leveller_amount, "lev.amount", [], |c| c
    .set_leveller_amount(41.0));
setter!(wp_p0w_set_leveller_speed, "lev.speed", [], |c| {
    let s = c.get_state().leveller_speed;
    c.set_leveller_speed(if s == 2 { 0 } else { 2 })
});
setter!(wp_p0w_set_leveller_max_gain, "lev.maxgain", [], |c| {
    c.set_leveller_max_gain(11.0)
});
setter!(wp_p0w_set_leveller_lookahead, "lev.lookahead", [], |c| {
    let b = c.get_state().leveller_lookahead;
    c.set_leveller_lookahead(!b)
});
setter!(wp_p0w_set_leveller_gate, "lev.gate", [], |c| c
    .set_leveller_gate(-51.0));
setter!(wp_p0w_set_leveller_masks, "lev.masks", [], |c| {
    let s = c.get_state();
    c.set_leveller_masks(s.leveller_detector_mask ^ 1, s.leveller_apply_mask ^ 2)
});

// ── params_fx.rs ────────────────────────────────────────────────────

setter!(wp_p0w_set_psybass_param, "bass.cutoff", [], |c| {
    c.set_psybass_param(PSYBASS_PARAM_CUTOFF, 123.0)
});
setter!(wp_p0w_set_psybass_mask, "bass.mask", [], |c| {
    let m = c.get_state().psybass_output_mask;
    c.set_psybass_mask(m ^ 0b100)
});
setter!(wp_p0w_set_psybass_enabled, "bass.on", [], |c| {
    let b = c.get_state().psybass_enabled;
    c.set_psybass_enabled(!b)
});
setter!(wp_p0w_set_limiter_param, "limit.threshold", [0], |c| {
    c.set_limiter_param(0, 1, -7.5)
});
setter!(wp_p0w_set_upmix_param, "up.strength", [], |c| {
    c.set_upmix_param(UPMIX_PARAM_STRENGTH, 0.42)
});
setter!(wp_p0w_set_subharm_param, "sub.boost", [], |c| {
    c.set_subharm_param(SUBHARM_PARAM_BOOST, 3.5)
});
setter!(wp_p0w_set_subharm_solo, "sub.solo", [], |c| {
    let b = c.get_state().subharm_solo;
    c.set_subharm_solo(!b)
});
setter!(wp_p0w_set_tube_param, "tube.drive", [], |c| {
    c.set_tube_param(dspi_proto::generated::tube::TUBE_PARAM_DRIVE_DB as u8, 7.5)
});

// ── hardware.rs ─────────────────────────────────────────────────────

pin_setter!(wp_p0w_set_output_pin, "out.pin", [0], |c| c
    .set_output_pin(0, SPARE_PIN));
setter!(wp_p0w_set_output_config_mode, "preset.iomode", [], |c| {
    let m = c.get_state().output_config_mode;
    c.set_output_config_mode(1 - m.min(1))
});
setter!(wp_p0w_set_output_type, "out.type", [1], |c| {
    let t = c.get_state().output_types[1];
    c.set_output_type(1, if t == 1 { 0 } else { 1 })
});
// -- WP CG-SWS-b -- a pin the pin map accepts: BCK 22 needs LRCLK 23 (power),
// and MCK only takes a clock-output pin; the setters now check first
pin_setter!(wp_p0w_set_i2s_bck_pin, "i2s.bck", [], |c| c
    .set_i2s_bck_pin(0, SPARE_BCK_PIN));
pin_setter!(wp_p0w_set_i2s_clock_pin_mode, "i2s.clockpins", [], |c| {
    let m = c.get_state().i2s_clock_pin_mode;
    c.set_i2s_clock_pin_mode(1 - m.min(1))
});
setter!(wp_p0w_set_mck_enabled, "i2s.mck", [], |c| {
    let b = c.get_state().mck_enabled;
    c.set_mck_enabled(!b)
});
pin_setter!(wp_p0w_set_mck_pin, "i2s.mck.pin", [], |c| c
    .set_mck_pin(SPARE_MCK_PIN));
setter!(wp_p0w_set_mck_multiplier, "i2s.mck.mult", [], |c| {
    let m = c.get_state().mck_multiplier;
    c.set_mck_multiplier(1 - m.min(1))
});
setter!(wp_p0w_set_adat_out_enabled, "adat.enable", [], |c| {
    let b = c.get_state().adat_out_enabled;
    c.set_adat_out_enabled(!b)
});
pin_setter!(wp_p0w_set_adat_out_pin, "adat.pin", [], |c| c
    .set_adat_out_pin(SPARE_PIN));
pin_setter!(wp_p0w_set_spdif_rx_pin, "in.spdif.pin", [1], |c| {
    c.set_spdif_rx_pin(1, SPARE_PIN)
});
setter!(
    wp_p0w_set_spdif_input_enabled,
    "in.spdif.enable",
    [1],
    |c| {
        let on = c.get_state().spdif_inputs_enabled & 0b10 != 0;
        c.set_spdif_input_enabled(1, !on)
    }
);
pin_setter!(wp_p0w_set_i2s_rx_pin, "in.i2s.pin", [0], |c| c
    .set_i2s_rx_pin(0, SPARE_PIN));
setter!(wp_p0w_set_i2s_input_channels, "in.i2s.channels", [], |c| {
    let n = c.get_state().i2s_input_channels;
    c.set_i2s_input_channels(if n == 4 { 2 } else { 4 })
});
#[test]
fn wp_p0w_set_adat_input_enabled() {
    // The fake refuses an enable without a pin.
    let core = pin_demo();
    core.set_adat_input_pin(SPARE_PIN).unwrap();
    check(&core, "in.adat.enable", &[], |c| {
        let b = c.get_state().adat_input_enabled;
        c.set_adat_input_enabled(!b)
    });
}
pin_setter!(wp_p0w_set_adat_input_pin, "in.adat.pin", [], |c| {
    c.set_adat_input_pin(SPARE_PIN)
});
setter!(wp_p0w_set_adat_input_clock_mode, "in.adat.clock", [], |c| {
    let m = c.get_state().adat_input_clock_mode;
    c.set_adat_input_clock_mode(1 - m.min(1))
});
setter!(wp_p0w_set_input_rate, "in.rate", [], |c| {
    let r = c.get_state().i2s_input_rate;
    c.set_input_rate(if r == 2 { 0 } else { 2 })
});
setter!(wp_p0w_set_i2s_clock_mode, "in.i2s.clock", [], |c| {
    let m = c.get_state().i2s_clock_mode;
    c.set_i2s_clock_mode(1 - m.min(1))
});
setter!(wp_p0w_set_lg_sound_sync, "in.lg", [], |c| {
    let b = c.get_state().lg_sound_sync_enabled;
    c.set_lg_sound_sync(!b)
});
action!(wp_p0w_set_dac_mute, "dev.dacmute", |c| {
    c.set_dac_mute(true, true, SPARE_PIN, 20, 30)
});
#[test]
fn wp_p0w_test_dac_mute() {
    check_action(&pin_demo(), "dev.dacmute.test", |c| c.test_dac_mute());
}
action!(wp_p0w_set_uart, "dev.uart", |c| {
    c.set_uart(UartConfig {
        enabled: false,
        tx_pin: 0,
        rx_pin: 1,
        notify: false,
        baud: 115_200,
    })
});
action!(wp_p0w_set_i2c, "dev.i2c", |c| {
    c.set_i2c(I2cConfig {
        enabled: false,
        sda_pin: 2,
        scl_pin: 3,
        address: 0x42,
    })
});
action!(wp_p0w_save_output_config, "dev.save.io", |c| c
    .save_output_config_tok());

// ── presets.rs ──────────────────────────────────────────────────────

setter!(wp_p0w_set_preset_name, "preset.name", [1], |c| {
    c.set_preset_name(1, "Evening".into())
});
setter!(wp_p0w_set_preset_startup, "preset.startup", [], |c| {
    let s = c.get_state();
    c.set_preset_startup(1 - s.preset_startup_mode.min(1), 2)
});
action!(wp_p0w_save_preset, "preset.save", |c| c.save_preset_tok(1));
action!(wp_p0w_save_params, "dev.save", |c| c.save_params_tok());
action!(wp_p0w_save_master_volume, "vol.master.save", |c| c
    .save_master_volume());
action!(wp_p0w_delete_preset, "preset.delete", |c| {
    c.save_preset_tok(3)?;
    c.delete_preset_tok(3)
});

// ── info.rs, stats.rs, raw.rs, ext.rs ───────────────────────────────

setter!(wp_p0w_set_input_source_name, "in.name", [1], |c| {
    c.set_input_source_name(1, "Television".into())
});
setter!(wp_p0w_set_device_name, "dev.name", [], |c| c
    .set_device_name("Rack".into()));
action!(wp_p0w_set_output_latency, "out.latency", |c| c
    .set_output_latency(1));
action!(wp_p0w_siggen_control, "sig.control", |c| {
    c.siggen_control(dspi_proto::generated::siggen::SIGGEN_CTL_STOP_NOW)
});
action!(wp_p0w_raw_set, "raw", |c| {
    c.raw_set(
        dspi_proto::generated::opcodes::REQ_SET_USER_VOLUME,
        0,
        (-30.0f32).to_le_bytes().to_vec(),
    )
});

#[test]
fn wp_p0w_siggen_set_config() {
    let core = demo();
    let caps = core.siggen_fetch_caps().unwrap();
    let t = caps.types.first().expect("a signal type");
    check_action(&core, "sig.config", |c| {
        c.siggen_set_config(SiggenConfig {
            signal_type: t.id,
            channel_mask: 1,
            invert_mask: 0,
            flags: 0,
            level_db: -30.0,
            duration_ms: 0,
            repeat: 1,
            gap_ms: 0,
            p: t.params.iter().map(|p| p.def).collect(),
        })
    });
}

#[test]
fn wp_p0w_set_crossfeed_pair_map() {
    // The extension channel's pair map: the demo device in K340 form has it.
    let core = demo();
    let Ok(map) = core.fetch_crossfeed_pair_map() else {
        // A demo without the extension channel: nothing to journal; say so.
        assert!(matches!(
            core.set_crossfeed_pair_map(vec![0x10, 0x32, 0, 0]),
            Err(DspiError::Unsupported)
        ));
        assert!(core.history(10).is_empty());
        return;
    };
    // The K340's map, or back to the default.
    let other = if map == [0x02, 0x13, 0, 0] {
        vec![0; 4]
    } else {
        vec![0x02, 0x13, 0, 0]
    };
    check(&core, EXT_PAIRMAP, &[], |c| c.set_crossfeed_pair_map(other));
}

#[test]
fn wp_p0w_cs_apply() {
    let core = demo();
    let r = core.cs_apply(r#"{"op": "auxState", "slot": 0, "on": true}"#.into());
    let v: serde_json::Value = serde_json::from_str(&r).unwrap();
    if v["ok"] != serde_json::Value::Bool(true) {
        // A demo without control surfaces: refused, nothing journalled.
        assert!(
            core.history(10).iter().all(|e| !e.path.starts_with("cs.")),
            "{v}"
        );
        return;
    }
    let e = newest(&core);
    assert_eq!(e.path, "cs.aux");
    assert!(!e.undoable, "the editor's own revert is the way back");
}

// ── Whole-state changes: recorded, and the end of the history ──────

#[test]
fn wp_p0w_load_preset_ends_the_history() {
    let core = demo();
    core.save_preset_tok(2).unwrap();
    core.set_output_gain(0, -9.0).unwrap();
    assert!(core.can_undo());
    core.load_preset(2).unwrap();
    assert!(
        !core.can_undo(),
        "a gain from before the load would land on another state"
    );
    let e = newest(&core);
    assert_eq!(
        (e.path.as_str(), e.indices.as_slice()),
        ("preset.load", &[2u8][..])
    );
    let undone = core.undo().unwrap();
    assert!(undone.entry.is_none());
    assert_eq!(undone.skipped.len(), 1);
    assert_eq!(core.undo().unwrap(), HistoryMove::default());
    assert!(core.history(10).len() >= 3, "the journal keeps it all");
}

#[test]
fn wp_p0w_load_params_ends_the_history() {
    let core = demo();
    core.save_params_tok().unwrap();
    core.set_output_gain(0, -9.0).unwrap();
    core.load_params().unwrap();
    assert!(!core.can_undo());
    assert_eq!(newest(&core).path, "preset.load");
}

#[test]
fn wp_p0w_apply_all_params_ends_the_history() {
    let core = demo();
    core.set_output_gain(0, -9.0).unwrap();
    core.apply_all_params().unwrap();
    assert!(!core.can_undo());
    assert_eq!(newest(&core).path, "bulk");
}

#[test]
fn wp_p0w_factory_reset_ends_the_history() {
    let core = demo();
    core.set_output_gain(0, -9.0).unwrap();
    let len = core.history(100).len();
    core.factory_reset_tok().unwrap();
    // The reset reads the device again on the same connection.
    let h = core.history(100);
    assert!(h.len() > len, "{h:?}");
    assert!(h.iter().any(|e| e.path == "dev.reset" && !e.undoable));
    assert!(!core.can_undo());
}

// ── The rules ───────────────────────────────────────────────────────

/// The hazard table of the report, from the registry itself.
#[test]
fn wp_p0w_only_a_parameter_without_a_hazard_is_undoable() {
    for d in REGISTRY.iter().filter(|d| d.is_writable()) {
        let want = d.hazard == Hazard::None && !d.path.starts_with("cs.");
        assert_eq!(super::hazard(d.path) == Hazard::None, want, "{}", d.path);
    }
    assert_eq!(super::hazard(EQ_BAND), Hazard::None);
    assert_eq!(super::hazard("raw"), Hazard::Irreversible);
    assert_eq!(super::hazard("ext.vreg"), Hazard::Irreversible);
}

/// The mirror reads the journal uses are the read-backs `write_param`
/// answers with, for every path a typed setter writes.
#[test]
fn wp_p0w_the_journal_reads_the_mirror_as_write_param_answers() {
    let core = demo();
    let mut checked = 0;
    for d in REGISTRY.iter().filter(|d| d.is_writable()) {
        let ix: Vec<u8> = match d.path {
            "in.spdif.enable" => vec![1],
            _ if d.target.arity() == 2 => vec![0, 1],
            _ => vec![0; d.target.arity()],
        };
        let Some(v) = mirror(&core, d.path, &ix) else {
            continue;
        };
        let answer = core.write_param(d.path.into(), ix.clone(), v.clone().into());
        if let Ok(a) = answer {
            assert_eq!(
                Value::from(a),
                mirror(&core, d.path, &ix).unwrap(),
                "{}",
                d.path
            );
            checked += 1;
        }
    }
    assert!(checked > 60, "{checked}");
}

#[test]
fn wp_p0w_writing_the_value_already_there_is_not_a_change() {
    let core = demo();
    let g = core.get_state().output_gain_db[0];
    core.set_output_gain(0, g).unwrap();
    assert!(core.history(10).is_empty());
    assert!(!core.can_undo());
}

#[test]
fn wp_p0w_a_drag_is_one_step() {
    let core = demo();
    let start = core.get_state().output_gain_db[0];
    core.set_dragging(true); // -- WP QTGAP1 -- only a drag gathers
    for db in [-1.0, -2.0, -3.0, -4.0] {
        core.set_output_gain(0, db).unwrap();
    }
    core.set_dragging(false);
    assert_eq!(core.history(10).len(), 1);
    core.undo().unwrap();
    assert_eq!(core.get_state().output_gain_db[0], start);
    assert!(!core.can_undo());
    core.redo().unwrap();
    assert_eq!(core.get_state().output_gain_db[0], -4.0);
}

#[test]
fn wp_p0w_a_new_change_ends_what_can_be_redone() {
    let core = demo();
    core.set_output_gain(0, -5.0).unwrap();
    core.undo().unwrap();
    assert!(core.can_redo());
    core.set_output_mute(1, true).unwrap();
    assert!(!core.can_redo());
    assert_eq!(core.redo().unwrap(), HistoryMove::default());
}

#[test]
fn wp_p0w_undo_steps_over_what_it_cannot_undo() {
    let core = demo();
    core.set_output_gain(0, -5.0).unwrap();
    core.save_params_tok().unwrap();
    let undone = core.undo().unwrap();
    assert_eq!(undone.entry.map(|e| e.path).as_deref(), Some("out.gain"));
    assert_eq!(undone.skipped.len(), 1);
    assert_eq!(undone.skipped[0].path, "dev.save");
}

/// The tube's voicing and a limiter group come back with the value that
/// moved them.
#[test]
fn wp_p0w_what_a_write_moved_comes_back_with_it() {
    let core = demo();
    let tube = |c: &DspiCore| c.get_state().tube;
    let type_ix = dspi_proto::generated::tube::TUBE_PARAM_TUBE_TYPE as u8;
    let bias_ix = dspi_proto::generated::tube::TUBE_PARAM_BIAS_PCT as u8;
    // A custom voicing, then a type that loads its own.
    core.set_tube_param(type_ix, 0.0).unwrap();
    core.set_tube_param(bias_ix, 37.0).unwrap();
    let custom = tube(&core);
    core.set_tube_param(type_ix, 3.0).unwrap();
    assert_ne!(tube(&core), custom);
    core.undo().unwrap();
    assert_eq!(tube(&core), custom, "type and voicing back");

    // Every output's threshold at once, then back to each one's own.
    core.set_limiter_param(0, 1, -3.0).unwrap();
    core.set_limiter_param(1, 1, -9.0).unwrap();
    let own = core.get_state().limiter_threshold_db;
    core.set_limiter_param(ALL_OUTPUTS, 1, -20.0).unwrap();
    assert_eq!(
        newest(&core).before,
        None,
        "every output has no single value"
    );
    core.undo().unwrap();
    assert_eq!(core.get_state().limiter_threshold_db, own);
    core.redo().unwrap();
    assert_eq!(core.get_state().limiter_threshold_db[1], -20.0);
}

/// A band field written through `write_param` is journalled under that
/// field, as the typed setter it goes through records it.
#[test]
fn wp_p0w_write_param_journals_through_the_setter() {
    let core = demo();
    core.write_param("eq.gain".into(), vec![0, 1], ParamValue::Float(5.0))
        .unwrap();
    let e = newest(&core);
    assert_eq!(
        (e.path.as_str(), e.after.clone()),
        ("eq.gain", ParamValue::Float(5.0))
    );
    core.undo().unwrap();
    assert_eq!(core.history(10).len(), 2);
}

// ── Reconnects and replays ──────────────────────────────────────────

/// The journal belongs to the connection: a reconnect starts with none, and
/// says so with `NOTIFY_HISTORY`.
#[test]
fn wp_p0w_a_reconnect_starts_a_new_history() {
    let core = demo();
    core.set_output_gain(0, -5.0).unwrap();
    assert_ne!(core.process_notifications().flags & NOTIFY_HISTORY, 0);
    assert_eq!(
        core.process_notifications().flags & NOTIFY_HISTORY,
        0,
        "once"
    );

    core.disconnect();
    assert!(!core.can_undo() && !core.can_redo());
    assert!(core.history(10).is_empty());
    assert!(matches!(core.undo(), Err(DspiError::Disconnected)));
    assert_ne!(core.process_notifications().flags & NOTIFY_HISTORY, 0);

    core.open_demo_blank().unwrap(); // -- WP UB12 --
    assert!(
        core.history(10).is_empty(),
        "no history crosses a reconnect"
    );
    assert!(!core.can_undo());
    assert_eq!(core.undo().unwrap(), HistoryMove::default());
    assert_eq!(core.process_notifications().flags & NOTIFY_HISTORY, 0);
}

/// A replay that finds another connection than its entry's sends nothing.
#[test]
fn wp_p0w_a_replay_never_reaches_another_connection() {
    let core = demo();
    let stale = core.lock().conn().unwrap().generation.wrapping_add(1);
    let gain = core.get_state().output_gain_db[0];
    REPLAY.set(Some(Replay {
        source: ChangeSource::Undo,
        generation: stale,
    }));
    let r = core.set_output_gain(0, -12.0);
    let p = core.write_param("up.config".into(), vec![], ParamValue::Bytes(vec![0; 4]));
    REPLAY.set(None);
    assert!(matches!(r, Err(DspiError::Disconnected)), "{r:?}");
    assert!(
        matches!(
            p,
            Err(DspiError::Disconnected | DspiError::InvalidArgument { .. })
        ),
        "{p:?}"
    );
    assert_eq!(core.get_state().output_gain_db[0], gain, "nothing sent");
    assert!(core.history(10).is_empty());
}

/// A setter's failure leaves no entry, and an undo the device refuses
/// keeps the change to undo.
#[test]
fn wp_p0w_a_failed_write_is_not_a_change() {
    let core = demo();
    assert!(core.set_output_gain(99, -3.0).is_err());
    assert!(core.history(10).is_empty());
}

#[test]
fn wp_p0w_history_is_newest_first_and_bounded_by_n() {
    let core = demo();
    core.set_output_gain(0, -5.0).unwrap();
    core.set_output_mute(1, true).unwrap();
    core.set_user_volume(-30.0).unwrap();
    let h = core.history(2);
    assert_eq!(h.len(), 2);
    assert_eq!(h[0].path, "vol.user");
    assert_eq!(h[1].path, "out.mute");
    let json = serde_json::to_value(&h[0]).unwrap();
    assert_eq!(json["source"], "local");
    assert!(json["atMs"].as_u64().unwrap() > 0);
    assert_eq!(json["after"], serde_json::json!({ "float": -30.0 }));
    assert_eq!(json["undoable"], true);
}

#[test]
fn wp_p0w_no_device_no_history() {
    let core = DspiCore::new();
    assert!(!core.can_undo() && !core.can_redo());
    assert!(core.history(5).is_empty());
    assert!(matches!(core.undo(), Err(DspiError::Disconnected)));
    assert!(matches!(core.redo(), Err(DspiError::Disconnected)));
}

#[test]
fn wp_p0w_a_band_survives_the_journal_encoding() {
    let p = FilterParams {
        filter_type: 7,
        bypass: true,
        freq: 1234.5,
        q: 0.9,
        gain: -3.25,
        qp: 0.5,
    };
    assert_eq!(band_from_bytes(&band_bytes(&p)), Some(p));
    assert_eq!(band_from_bytes(&[1, 2, 3]), None);
}

// ── Fixup p0w ───────────────────────────────────────────────────────

/// Review p0w finding 2 (its probe): the legacy preamp sets every input to
/// one value; undo puts back each input's own.
#[test]
fn wp_p0w_fixup_pre_all_undo_restores_each_input() {
    let core = demo();
    core.set_input_preamp(0, -3.0).unwrap();
    core.set_input_preamp(1, -6.0).unwrap();
    let before = core.get_state().input_preamp_db.clone();
    core.set_preamp(0.0).unwrap();
    assert!(core.get_state().input_preamp_db.iter().all(|v| *v == 0.0));
    core.undo().unwrap();
    assert_eq!(core.get_state().input_preamp_db, before);
    core.fetch_all().unwrap();
    assert_eq!(core.get_state().input_preamp_db, before, "the device too");
    core.redo().unwrap();
    assert!(core.get_state().input_preamp_db.iter().all(|v| *v == 0.0));
}

/// Review p0w finding 3: only a live value drags; two pin moves or two
/// rate changes are two entries.
#[test]
fn wp_p0w_fixup_a_non_undoable_write_never_coalesces() {
    let core = demo();
    core.set_dragging(true); // -- WP QTGAP1 -- even within a drag
    core.set_input_rate(0).unwrap();
    core.set_input_rate(2).unwrap();
    let h = core.history(10);
    assert_eq!(h.len(), 2, "{h:?}");
    assert!(h.iter().all(|e| e.path == "in.rate" && !e.undoable));
}

/// Review p0w finding 4: a panic inside a replayed setter leaves no mark.
#[test]
fn wp_p0w_fixup_the_replay_mark_is_cleared_by_a_panic() {
    let r = std::panic::catch_unwind(|| {
        let _mark = ReplayMark::set(Replay {
            source: ChangeSource::Undo,
            generation: 1,
        });
        assert_eq!(replay_source(), Some(ChangeSource::Undo));
        panic!("a setter bug");
    });
    assert!(r.is_err());
    assert_eq!(replay_source(), None);
}

/// The demo device with its remote, for a change made elsewhere.
fn remote_demo() -> (Arc<DspiCore>, dspi_transport::FakeRemote) {
    let dev = dspi_transport::FakeDevice::demo();
    let remote = dev.remote();
    let core = DspiCore::new();
    core.attach_probed(Box::new(dev), "DEMO".into(), 0).unwrap();
    (core, remote)
}

/// Our master volume, then the USB audio volume control moves it.
fn overwritten_by_uac1() -> (Arc<DspiCore>, f32) {
    let (core, remote) = remote_demo();
    let start = core.get_state().master_volume_db;
    core.set_master_volume(-10.0).unwrap();
    let src = dspi_proto::generated::notify::PARAM_SRC_UAC1 as u8;
    let off = dspi_proto::generated::OFF_MASTER_VOLUME;
    assert!(remote.poke(off, &(-12.0f32).to_le_bytes(), src));
    let t = dspi_transport::clock::Instant::now();
    while core.get_state().master_volume_db != -12.0 {
        assert!(
            t.elapsed() < std::time::Duration::from_secs(5),
            "no notification"
        );
        core.process_notifications();
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    (core, start)
}

/// Owner decision (2026-11-12): a value someone else has overwritten since
/// is left as they set it; undo drops the entry and names them.
#[test]
fn wp_p0w_fixup_a_foreign_change_skip_and_name() {
    let (core, _) = overwritten_by_uac1();
    let m = core.undo().unwrap();
    assert!(m.entry.is_none(), "{m:?}");
    assert_eq!(m.skipped.len(), 1);
    assert_eq!(m.skipped[0].path, "vol.master");
    let note = m.note.unwrap();
    // -- WP QPI4 -- the label and no host-device noun
    assert_eq!(
        note,
        "Master volume was changed by the USB audio volume control since; it was left as they set it"
    );
    assert_eq!(core.get_state().master_volume_db, -12.0, "theirs stays");
    assert!(!core.can_undo() && !core.can_redo(), "the entry is gone");
}

#[test]
fn wp_p0w_fixup_a_foreign_change_after_undo_skips_redo() {
    let (core, remote) = remote_demo();
    core.set_master_volume(-10.0).unwrap();
    core.undo().unwrap();
    let start = core.get_state().master_volume_db;
    let src = dspi_proto::generated::notify::PARAM_SRC_UART as u8;
    let off = dspi_proto::generated::OFF_MASTER_VOLUME;
    let other = if start == -20.0 { -21.0f32 } else { -20.0 };
    assert!(remote.poke(off, &other.to_le_bytes(), src));
    let t = dspi_transport::clock::Instant::now();
    while core.get_state().master_volume_db != other {
        assert!(t.elapsed() < std::time::Duration::from_secs(5));
        core.process_notifications();
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    let m = core.redo().unwrap();
    assert!(m.entry.is_none());
    assert!(m.note.unwrap().contains("the UART"));
    assert_eq!(core.get_state().master_volume_db, other);
    assert!(!core.can_redo());
}

// ── P0WECHO: a late own echo is not another writer ─────────────────

/// Drain the notifications until 50 ms pass without one changing the state.
fn drain(core: &DspiCore) {
    let mut quiet = 0;
    while quiet < 10 {
        let f = core.process_notifications().flags;
        quiet = if f & (NOTIFY_STATE | NOTIFY_REFRESHED) == 0 {
            quiet + 1
        } else {
            0
        };
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

/// A `PARAM_CHANGED` for the master volume from `source`, queued on the
/// endpoint without touching the device's image: a late echo replays a value
/// the device no longer holds.
fn master_echo(remote: &dspi_transport::FakeRemote, db: f32, source: u16) {
    let mut p = vec![
        2,
        dspi_proto::generated::notify::NOTIFY_EVT_PARAM_CHANGED as u8,
        0,
        0,
    ];
    p.extend((dspi_proto::generated::OFF_MASTER_VOLUME as u16).to_le_bytes());
    p.extend(4u16.to_le_bytes());
    p.extend([source as u8, 0, 0, 0]);
    p.extend(db.to_le_bytes());
    remote.push(p);
}

const HOST_SET: u16 = dspi_proto::generated::notify::PARAM_SRC_HOST_SET;
const UAC1: u16 = dspi_proto::generated::notify::PARAM_SRC_UAC1;

/// Our write, undone and read back; then the write's own echo arrives late
/// and puts our old value back in the mirror (HUBFLAKE3 section 4).
fn undone_then_late_own_echo() -> (Arc<DspiCore>, dspi_transport::FakeRemote, f32) {
    let (core, remote) = remote_demo();
    drain(&core);
    let start = core.get_state().master_volume_db;
    core.set_master_volume(-10.0).unwrap();
    drain(&core);
    core.undo().unwrap();
    drain(&core);
    core.fetch_all().unwrap();
    assert_eq!(core.get_state().master_volume_db, start, "read back");
    master_echo(&remote, -10.0, HOST_SET);
    let t = dspi_transport::clock::Instant::now();
    while core.get_state().master_volume_db != -10.0 {
        assert!(
            t.elapsed() < std::time::Duration::from_secs(5),
            "the echo never came"
        );
        core.process_notifications();
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    (core, remote, start)
}

#[test]
fn wp_p0wecho_a_late_own_echo_does_not_cost_the_redo() {
    let (core, _remote, _) = undone_then_late_own_echo();
    let m = core.redo().unwrap();
    assert_eq!(m.note, None, "{m:?}");
    assert_eq!(m.entry.map(|e| e.path).as_deref(), Some("vol.master"));
    // The mirror already showed -10, and the redo is journalled anyway.
    assert_eq!(newest(&core).source, HistorySource::Redo);
    assert!(core.can_undo() && !core.can_redo());
    drain(&core);
    core.fetch_all().unwrap();
    assert_eq!(core.get_state().master_volume_db, -10.0);
}

#[test]
fn wp_p0wecho_a_foreign_write_after_the_late_echo_is_still_named() {
    let (core, remote, _) = undone_then_late_own_echo();
    master_echo(&remote, -14.0, UAC1);
    let t = dspi_transport::clock::Instant::now();
    while core.get_state().master_volume_db != -14.0 {
        assert!(t.elapsed() < std::time::Duration::from_secs(5));
        core.process_notifications();
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    let m = core.redo().unwrap();
    assert!(m.entry.is_none(), "{m:?}");
    let note = m.note.unwrap();
    assert!(note.contains("the USB audio volume control"), "{note}");
}

#[test]
fn wp_p0wecho_a_foreign_write_elsewhere_in_the_window_changes_nothing() {
    let (core, remote, _) = undone_then_late_own_echo();
    let off = dspi_proto::generated::OFF_USER_VOLUME;
    assert!(remote.poke(off, &(-33.0f32).to_le_bytes(), UAC1 as u8));
    let t = dspi_transport::clock::Instant::now();
    while core.get_state().user_volume_db != -33.0 {
        assert!(t.elapsed() < std::time::Duration::from_secs(5));
        core.process_notifications();
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    let m = core.redo().unwrap();
    assert_eq!(m.note, None, "{m:?}");
    assert!(m.entry.is_some());
}

/// The same for undo: a drag's earlier tick echoed late is ours too.
#[test]
fn wp_p0wecho_a_late_echo_of_a_drag_tick_does_not_cost_the_undo() {
    let (core, remote) = remote_demo();
    drain(&core);
    let start = core.get_state().master_volume_db;
    core.set_dragging(true); // -- WP QTGAP1 --
    core.set_master_volume(-10.0).unwrap();
    core.set_master_volume(-12.0).unwrap();
    core.set_dragging(false);
    drain(&core);
    master_echo(&remote, -10.0, HOST_SET);
    let t = dspi_transport::clock::Instant::now();
    while core.get_state().master_volume_db != -10.0 {
        assert!(t.elapsed() < std::time::Duration::from_secs(5));
        core.process_notifications();
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    let m = core.undo().unwrap();
    assert_eq!(m.note, None, "{m:?}");
    assert_eq!(core.get_state().master_volume_db, start);
}

/// Our own value is remembered for a while only: long after, a matching
/// value is judged like any other.
#[test]
fn wp_p0wecho_own_writes_are_bounded_by_age_and_count() {
    let mut w = OwnWrites::default();
    w.note("vol.master", &[], &Value::Float(-10.0), 1_000);
    // -- WP SWAPFIX -- only a value written over since is a late echo
    assert!(!w.explains("vol.master", &[], &Value::Float(-10.0), 1_000));
    w.note("vol.master", &[], &Value::Float(-12.0), 1_000);
    assert!(w.explains("vol.master", &[], &Value::Float(-10.0), 1_000 + OWN_ECHO_MS));
    assert!(!w.explains("vol.master", &[], &Value::Float(-10.0), 1_001 + OWN_ECHO_MS));
    assert!(!w.explains("vol.master", &[], &Value::Float(-11.0), 1_000));
    assert!(!w.explains("out.gain", &[0], &Value::Float(-10.0), 1_000));
    for i in 0..OWN_WRITES_CAP {
        w.note("out.gain", &[0], &Value::Float(i as f32), 2_000);
    }
    assert!(
        !w.explains("vol.master", &[], &Value::Float(-10.0), 2_000),
        "the oldest went"
    );
    assert_eq!(w.0.len(), OWN_WRITES_CAP);
}

// ── DSPi Link ───────────────────────────────────────────────────────

#[cfg(feature = "link")]
mod link {
    use std::time::Duration;

    use dspi_link::client::set_caller_driven_on_this_thread;
    use dspi_link::fakehub::{FakeHub, HubConfig, req};
    use dspi_transport::clock::Instant;

    use super::*;
    use crate::LinkPhase;
    use crate::link::Tuning;

    const WAIT: Duration = Duration::from_secs(8);

    fn tick_until(core: &DspiCore, phase: LinkPhase) {
        let start = Instant::now();
        while core.link_status().phase != phase {
            assert!(start.elapsed() < WAIT, "no {phase:?}");
            core.link_tick();
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn sets(hub: &FakeHub) -> Vec<u16> {
        hub.log()
            .iter()
            .filter(|e| e.2 == req::SET_MASTER_VOLUME)
            .map(|e| e.0)
            .collect()
    }

    /// An undo over Link is a write the hub runs on this client's session,
    /// attributed to it like any other. A lost link replays nothing: the
    /// re-attached session starts with no history, and what was journalled
    /// before is not sent again.
    #[test]
    fn wp_p0w_undo_over_link_is_a_write_and_a_lost_link_replays_nothing() {
        let hub = FakeHub::start(HubConfig::default()).unwrap();
        let core = DspiCore::new();
        set_caller_driven_on_this_thread(true);
        core.link_connect_with(
            hub.url(),
            "wp-p0w".into(),
            "test".into(),
            String::new(),
            String::new(),
            Tuning {
                backoff_divisor: 50,
                ..Tuning::default()
            },
        )
        .unwrap();
        set_caller_driven_on_this_thread(false);
        tick_until(&core, LinkPhase::ChooseDevice);
        let d = core.link_devices().remove(0);
        core.link_select_device(d.handle, d.serial.clone()).unwrap();
        core.fetch_all().unwrap();
        tick_until(&core, LinkPhase::Bound);
        let me = hub.sessions();
        assert_eq!(me.len(), 1);

        let start = core.get_state().master_volume_db;
        core.set_master_volume(-9.0).unwrap();
        core.undo().unwrap();
        assert_eq!(core.get_state().master_volume_db, start);
        assert_eq!(
            sets(&hub),
            vec![me[0], me[0]],
            "the write and its undo, ours"
        );
        core.redo().unwrap();
        assert_eq!(sets(&hub).len(), 3);

        hub.drop_connections();
        tick_until(&core, LinkPhase::Reconnecting);
        assert!(matches!(core.undo(), Err(DspiError::Disconnected)));
        tick_until(&core, LinkPhase::Bound);
        assert!(core.history(10).is_empty(), "a new session, a new history");
        assert!(!core.can_undo());
        assert_eq!(core.undo().unwrap(), HistoryMove::default());
        assert_eq!(sets(&hub).len(), 3, "nothing replayed");
        core.link_close();
    }
}
