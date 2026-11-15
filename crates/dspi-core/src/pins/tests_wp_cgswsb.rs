//! WP CG-SWS-b: the core's pin map, per platform, with its conflicts, and the
//! setters going through it on the demo device.

use super::*;
use crate::state::default_state;

const RP2040: u8 = 0;
const RP2350: u8 = 1;
const STM32: u8 = 2;

fn f(kind: PinFunctionKind, index: u8) -> PinFunction {
    PinFunction::new(kind, index)
}

use PinFunctionKind as K;

/// A device as the firmware boots: the platform's defaults.
fn device(platform: u8) -> DspState {
    let mut s = default_state();
    s.platform_id = platform;
    s.bulk_valid = true;
    if platform == RP2040 {
        s.num_pin_outputs = 3;
        s.output_pins = vec![6, 7, 10, 0, 0];
        s.mck_pin = 21;
        s.i2s_bck_pin_slave = 12;
    } else {
        s.num_pin_outputs = 5;
        s.output_pins = vec![6, 7, 8, 9, 10];
    }
    s.i2s_clock_pin_mode_present = true;
    s.spdif_inputs_enabled_present = true;
    s
}

fn check(s: &DspState, func: PinFunction, pin: u8) -> std::result::Result<(), PinRefusal> {
    pin_check(s, &[], func, pin)
}

fn refused(s: &DspState, func: PinFunction, pin: u8) -> (PinRefusalReason, u8, Option<String>) {
    let r = check(s, func, pin).expect_err("refused");
    (r.reason, r.status, r.holder.map(|h| h.label))
}

fn info(map: &PinMap, func: PinFunction) -> &PinFunctionInfo {
    map.functions
        .iter()
        .find(|i| i.function == func)
        .expect("function listed")
}

// --------------------------------------------------------------- platforms

#[test]
fn cgswsb_valid_pins_per_platform() {
    let m40 = pin_map(&device(RP2040), &[]);
    assert!(m40.assignable);
    assert_eq!(m40.valid_pins.len(), 26);
    assert!(!m40.valid_pins.contains(&29));
    for p in 23..=25 {
        assert!(!m40.valid_pins.contains(&p));
    }

    let m50 = pin_map(&device(RP2350), &[]);
    assert_eq!(m50.valid_pins.len(), 27);
    assert!(
        m50.valid_pins.contains(&29),
        "is_valid_gpio_pin takes 29 on an RP2350"
    );

    // GPIO 29 as an output: fine on the RP2350, not a GPIO on the RP2040.
    assert!(check(&device(RP2350), f(K::Output, 0), 29).is_ok());
    assert_eq!(
        refused(&device(RP2040), f(K::Output, 0), 29).0,
        PinRefusalReason::InvalidPin
    );
    assert_eq!(refused(&device(RP2350), f(K::Output, 0), 24).1, 1);
}

#[test]
fn cgswsb_the_functions_each_platform_has() {
    let m40 = pin_map(&device(RP2040), &[]);
    let outs: Vec<String> = m40
        .functions
        .iter()
        .filter(|i| i.function.kind == K::Output)
        .map(|i| i.label.clone())
        .collect();
    assert_eq!(outs, ["Output 1", "Output 2", "Subwoofer"]);
    // One I2S RX pair, no ADAT on an RP2040.
    assert_eq!(
        m40.functions
            .iter()
            .filter(|i| i.function.kind == K::I2sRx)
            .count(),
        1
    );
    assert!(!m40.functions.iter().any(|i| i.function.kind == K::AdatOut));
    assert_eq!(
        refused(&device(RP2040), f(K::AdatOut, 0), 12).1,
        3,
        "INVALID_OUTPUT, as the RP2040 firmware answers"
    );
    assert_eq!(refused(&device(RP2040), f(K::I2sRx, 1), 2).1, 3);

    let m50 = pin_map(&device(RP2350), &[]);
    assert_eq!(
        m50.functions
            .iter()
            .filter(|i| i.function.kind == K::Output)
            .count(),
        5
    );
    assert_eq!(info(&m50, f(K::Output, 4)).label, "Subwoofer");
    assert_eq!(
        m50.functions
            .iter()
            .filter(|i| i.function.kind == K::I2sRx)
            .count(),
        4
    );
    assert!(m50.functions.iter().any(|i| i.function.kind == K::AdatIn));
    assert_eq!(refused(&device(RP2350), f(K::Output, 5), 20).1, 3);
}

#[test]
fn cgswsb_mck_only_on_clock_output_pins() {
    let m40 = pin_map(&device(RP2040), &[]);
    assert_eq!(info(&m40, f(K::Mck, 0)).choices, vec![21]);

    // RP2350: 13, 15, 21; 15 is the LRCLK of BCK 14, held.
    let s = device(RP2350);
    assert_eq!(info(&pin_map(&s, &[]), f(K::Mck, 0)).choices, vec![13, 21]);
    assert_eq!(
        refused(&s, f(K::Mck, 0), 15).2.as_deref(),
        Some("I2S LRCLK")
    );
    assert_eq!(refused(&s, f(K::Mck, 0), 12).0, PinRefusalReason::WrongMux);

    let mut moved = device(RP2350);
    moved.i2s_bck_pin = 2;
    assert_eq!(
        info(&pin_map(&moved, &[]), f(K::Mck, 0)).choices,
        vec![13, 15, 21]
    );

    // Not movable while enabled; the pin is held only while enabled.
    let mut on = device(RP2350);
    on.mck_enabled = true;
    assert_eq!(
        refused(&on, f(K::Mck, 0), 21).0,
        PinRefusalReason::OutputActive
    );
    assert_eq!(
        refused(&on, f(K::Output, 0), 13).2.as_deref(),
        Some("I2S MCK")
    );
    assert!(check(&device(RP2350), f(K::Output, 0), 13).is_ok());
}

#[test]
fn cgswsb_stm32_has_no_assignable_pins_but_its_slot_rule() {
    let mut s = device(STM32);
    s.output_types = vec![0, 0, 0, 0];
    let m = pin_map(&s, &[]);
    assert!(!m.assignable);
    assert!(m.valid_pins.is_empty() && m.holders.is_empty());
    assert!(m.functions.iter().all(|i| i.choices.is_empty()));
    assert_eq!(
        refused(&s, f(K::Output, 0), 6).0,
        PinRefusalReason::NotAssignable
    );
    let allowed = |m: &PinMap| -> Vec<bool> {
        (0..4)
            .map(|i| info(m, f(K::Output, i)).i2s_allowed)
            .collect()
    };
    assert_eq!(allowed(&m), [true, false, false, false]);
    s.output_types = vec![1, 0, 0, 0];
    assert_eq!(allowed(&pin_map(&s, &[])), [true, true, true, false]);
    s.output_types = vec![1, 0, 1, 0];
    assert_eq!(allowed(&pin_map(&s, &[])), [true, true, true, true]);
    assert!(
        !info(&pin_map(&s, &[]), f(K::Output, 4)).i2s_allowed,
        "the sub"
    );

    // The RP parts have no inter-slot rule.
    let m = pin_map(&device(RP2350), &[]);
    assert_eq!(allowed(&m), [true, true, true, true]);
    // A platform this build does not know: nothing assignable.
    assert!(!pin_map(&device(7), &[]).assignable);
}

// --------------------------------------------------------------- conflicts

#[test]
fn cgswsb_output_conflicts_name_the_holder() {
    let s = device(RP2350);
    assert_eq!(
        refused(&s, f(K::Output, 0), 7),
        (PinRefusalReason::InUse, 2, Some("Output 2".into()))
    );
    assert_eq!(
        refused(&s, f(K::Output, 0), 10).2.as_deref(),
        Some("Subwoofer")
    );
    assert_eq!(
        refused(&s, f(K::Output, 0), 5).2.as_deref(),
        Some("S/PDIF RX")
    );
    assert_eq!(refused(&s, f(K::Output, 0), 1).2.as_deref(), Some("I2S RX"));
    // Its own pin is a no-op; a free pin is fine.
    assert!(check(&s, f(K::Output, 0), 6).is_ok());
    assert!(check(&s, f(K::Output, 0), 0).is_ok());
    let m = pin_map(&s, &[]);
    let out1 = info(&m, f(K::Output, 0));
    assert!(out1.holds && out1.pin == 6);
    assert!(out1.choices.contains(&6) && out1.choices.contains(&0));
    assert!(!out1.choices.contains(&7) && !out1.choices.contains(&14));
}

/// The clock pair is held even while dormant, as every copy has it and as
/// the firmware's long-lived claims (I2S RX, UART, I2C) see it.
#[test]
fn cgswsb_the_clock_pair_is_always_held() {
    let s = device(RP2350);
    assert_eq!(
        refused(&s, f(K::Output, 0), 14).2.as_deref(),
        Some("I2S BCK")
    );
    assert_eq!(
        refused(&s, f(K::Output, 0), 15).2.as_deref(),
        Some("I2S LRCLK")
    );
    assert_eq!(
        refused(&s, f(K::I2sRx, 0), 15).2.as_deref(),
        Some("I2S LRCLK")
    );
    let mut ctrl = device(RP2350);
    ctrl.ctrl_iface_supported = true;
    assert_eq!(
        refused(&ctrl, f(K::I2cSda, 0), 14).2.as_deref(),
        Some("I2S BCK")
    );
    let m = pin_map(&s, &[]);
    let h: Vec<(u8, &str)> = m
        .holders
        .iter()
        .filter(|h| h.role == PinHolderRole::Clock)
        .map(|h| (h.pin, h.label.as_str()))
        .collect();
    assert_eq!(h, [(14, "I2S BCK"), (15, "I2S LRCLK")]);
}

#[test]
fn cgswsb_the_slave_pair_only_in_split_mode() {
    let mut s = device(RP2350);
    assert!(
        check(&s, f(K::Output, 0), 26).is_ok(),
        "dormant in unified mode"
    );
    s.i2s_clock_pin_mode = 1;
    assert_eq!(
        refused(&s, f(K::Output, 0), 27).2.as_deref(),
        Some("I2S Slave LRCLK")
    );
    // The pairs may not overlap: the slave always checks against the master,
    // the master against the slave only in split mode.
    assert_eq!(
        refused(&s, f(K::I2sBckSlave, 0), 13).2.as_deref(),
        Some("I2S BCK")
    );
    assert_eq!(
        refused(&s, f(K::I2sBck, 0), 25).0,
        PinRefusalReason::InvalidPin
    );
    assert_eq!(
        refused(&s, f(K::I2sBck, 0), 26).2.as_deref(),
        Some("I2S Slave BCK")
    );
    s.i2s_clock_pin_mode = 0;
    assert!(check(&s, f(K::I2sBck, 0), 26).is_ok());
}

#[test]
fn cgswsb_moving_the_clock_pair() {
    let mut s = device(RP2350);
    // BCK needs BCK + 1 valid too.
    assert_eq!(
        refused(&s, f(K::I2sBck, 0), 22).0,
        PinRefusalReason::InvalidPin
    );
    assert!(check(&s, f(K::I2sBck, 0), 2).is_ok());
    assert_eq!(refused(&s, f(K::I2sBck, 0), 0).2.as_deref(), Some("I2S RX"));
    // Its own pins do not block it while the clocks are idle...
    assert!(check(&s, f(K::I2sBck, 0), 13).is_ok());
    // ...but do while they run, as the firmware's is_pin_in_use has it.
    s.input_source = 2;
    assert_eq!(
        refused(&s, f(K::I2sBck, 0), 13).2.as_deref(),
        Some("I2S BCK")
    );
    // Under I2S outputs the master pair cannot move.
    s.input_source = 0;
    s.output_types = vec![1, 0, 0, 0];
    assert_eq!(
        refused(&s, f(K::I2sBck, 0), 2).0,
        PinRefusalReason::OutputActive
    );
    assert_eq!(refused(&s, f(K::I2sBck, 0), 2).1, 4);
    // ...but the slave pair can, unless split + slave clocking uses it.
    assert!(check(&s, f(K::I2sBckSlave, 0), 2).is_ok());
    s.i2s_clock_pin_mode = 1;
    s.i2s_clock_mode = 1;
    assert_eq!(
        refused(&s, f(K::I2sBckSlave, 0), 2).0,
        PinRefusalReason::OutputActive
    );
    assert!(check(&s, f(K::I2sBck, 0), 2).is_ok());
}

#[test]
fn cgswsb_spdif_inputs_hold_only_while_enabled() {
    let mut s = device(RP2350);
    s.spdif_rx_pins = vec![5, 20, 21, 22];
    s.spdif_inputs_enabled = 0b0101; // inputs 1 and 3
    let m = pin_map(&s, &[]);
    let held: Vec<(u8, &str)> = m
        .holders
        .iter()
        .filter(|h| h.function.kind == K::SpdifRx)
        .map(|h| (h.pin, h.label.as_str()))
        .collect();
    assert_eq!(held, [(5, "S/PDIF 1 RX"), (21, "S/PDIF 3 RX")]);
    assert!(
        check(&s, f(K::Output, 0), 20).is_ok(),
        "input 2 is disabled"
    );
    assert_eq!(
        refused(&s, f(K::Output, 0), 21).2.as_deref(),
        Some("S/PDIF 3 RX")
    );
    // An enabled input cannot move onto a held pin; a disabled one stores any
    // GPIO, but is not offered the held ones.
    assert_eq!(
        refused(&s, f(K::SpdifRx, 2), 7).2.as_deref(),
        Some("Output 2")
    );
    assert!(check(&s, f(K::SpdifRx, 1), 7).is_ok());
    let off = info(&m, f(K::SpdifRx, 1));
    assert!(!off.holds && !off.choices.contains(&7) && off.choices.contains(&20));
}

#[test]
fn cgswsb_i2s_rx_holds_only_the_active_pairs() {
    let mut s = device(RP2350);
    s.i2s_rx_pins = vec![1, 2, 3, 4];
    assert!(
        check(&s, f(K::Output, 0), 2).is_ok(),
        "pair 2 inactive in stereo"
    );
    assert_eq!(
        pin_map(&s, &[])
            .holders
            .iter()
            .find(|h| h.pin == 1)
            .map(|h| h.label.as_str()),
        Some("I2S RX")
    );
    s.i2s_input_channels = 8;
    assert_eq!(
        refused(&s, f(K::Output, 0), 4).2.as_deref(),
        Some("I2S RX 4")
    );
    // Pairs may not share; a pair keeps its own.
    assert_eq!(
        refused(&s, f(K::I2sRx, 0), 2).2.as_deref(),
        Some("I2S RX 2")
    );
    assert!(check(&s, f(K::I2sRx, 0), 1).is_ok());
}

#[test]
fn cgswsb_adat_loopback_is_one_directional() {
    let mut s = device(RP2350);
    s.adat_out_enabled = true;
    s.adat_out_pin = 12;
    s.adat_input_enabled = true;
    s.adat_input_pin = 11;
    // The input may take the output's pin...
    assert!(check(&s, f(K::AdatIn, 0), 12).is_ok());
    // ...the output may not take the input's.
    assert_eq!(
        refused(&s, f(K::AdatOut, 0), 11).2.as_deref(),
        Some("ADAT Input")
    );
    // Clearing the input's pin while enabled.
    assert_eq!(
        refused(&s, f(K::AdatIn, 0), PIN_NONE).0,
        PinRefusalReason::ClearWhileEnabled
    );
    s.adat_input_enabled = false;
    assert!(check(&s, f(K::AdatIn, 0), PIN_NONE).is_ok());
    assert!(
        check(&s, f(K::Output, 0), 11).is_ok(),
        "a disabled input holds nothing"
    );
    // Sharing the pin, the holder shown is the output.
    s.adat_input_enabled = true;
    s.adat_input_pin = 12;
    let m = pin_map(&s, &[]);
    assert_eq!(m.holders.iter().filter(|h| h.pin == 12).count(), 1);
    assert_eq!(
        m.holders
            .iter()
            .find(|h| h.pin == 12)
            .map(|h| h.label.as_str()),
        Some("ADAT Output")
    );
    assert!(info(&m, f(K::AdatIn, 0)).holds);
    // And the output, moving, still sees the input there.
    assert!(
        pin_holder(&s, &[], 12, Some(f(K::AdatOut, 0))).is_some_and(|h| h.label == "ADAT Input")
    );
}

#[test]
fn cgswsb_control_interfaces_and_surfaces() {
    let mut s = device(RP2350);
    s.ctrl_iface_supported = true;
    // Mux positions.
    assert!(check(&s, f(K::UartTx, 0), 0).is_ok());
    assert_eq!(
        refused(&s, f(K::UartTx, 0), 17).0,
        PinRefusalReason::WrongMux
    );
    assert_eq!(
        refused(&s, f(K::I2cScl, 0), 18).0,
        PinRefusalReason::WrongMux
    );
    // Pins held only while live; both pins are one owner.
    assert!(check(&s, f(K::Output, 0), 16).is_ok());
    s.ctrl_status.uart_live = true;
    assert_eq!(
        refused(&s, f(K::Output, 0), 17).2.as_deref(),
        Some("UART Control")
    );
    assert!(check(&s, f(K::UartTx, 0), 16).is_ok());
    assert!(check(&s, f(K::UartRx, 0), 17).is_ok());
    // A live binding holds its pins.
    let uses = [PinUse { pin: 20, slot: 2 }];
    let r = pin_check(&s, &uses, f(K::Output, 0), 20).expect_err("held");
    assert_eq!(
        r.holder.map(|h| (h.label, h.role)),
        Some(("Control Surface 3".into(), PinHolderRole::Control))
    );
    // The pair gate: one instance.
    let pair = |tx, rx| check_ctrl_pair(&s, &[], [(K::UartTx, tx), (K::UartRx, rx)], uart_instance);
    assert!(pair(16, 17).is_ok());
    assert!(pair(12, 13).is_ok());
    assert_eq!(
        pair(4, 13).expect_err("instances").reason,
        PinRefusalReason::WrongMux
    );
    // A pin another function holds: I2S RX pair 1 is on GPIO 1.
    assert_eq!(
        pair(0, 1).expect_err("held").reason,
        PinRefusalReason::InUse
    );
    let i2c =
        |sda, scl| check_ctrl_pair(&s, &[], [(K::I2cSda, sda), (K::I2cScl, scl)], i2c_instance);
    assert!(i2c(18, 19).is_ok());
    assert_eq!(
        i2c(18, 21).expect_err("instances").reason,
        PinRefusalReason::WrongMux
    );
}

#[test]
fn cgswsb_dac_mute() {
    let mut s = device(RP2350);
    s.dac_mute_supported = true;
    assert_eq!(
        refused(&s, f(K::DacMute, 0), 6).2.as_deref(),
        Some("Output 1")
    );
    assert!(check(&s, f(K::DacMute, 0), 11).is_ok());
    s.dac_mute_enabled = true;
    s.dac_mute_pin = 11;
    let h = pin_holder(&s, &[], 11, None).expect("held");
    assert_eq!(
        (h.label.as_str(), h.role),
        ("DAC Mute", PinHolderRole::Utility)
    );
    assert!(pin_holder(&s, &[], 11, Some(f(K::DacMute, 0))).is_none());
    s.dac_mute_supported = false;
    assert_eq!(
        refused(&s, f(K::DacMute, 0), 11).0,
        PinRefusalReason::NoSuchFunction
    );
}

#[test]
fn cgswsb_function_codes_round_trip() {
    for code in 0..14 {
        let k = PinFunctionKind::from_code(code).expect("kind");
        assert_eq!(k.code(), code);
    }
    assert!(PinFunctionKind::from_code(14).is_none());
    let json = serde_json::to_value(f(K::I2sBckSlave, 0)).expect("json");
    assert_eq!(json, serde_json::json!({"kind": "i2sBckSlave", "index": 0}));
}

// ----------------------------------------------- the setters, on the demo

#[test]
fn cgswsb_demo_setters_go_through_the_map() {
    let core = DspiCore::new();
    core.open_demo().expect("demo");
    let m = core.pin_map();
    assert!(m.assignable, "the demo is an RP2350");
    let out1 = m
        .functions
        .iter()
        .find(|i| i.function == f(K::Output, 0))
        .expect("output 1")
        .clone();
    // A free pin: the demo answers the move, and the map follows.
    let to = *out1
        .choices
        .iter()
        .find(|p| **p != out1.pin)
        .expect("a free pin");
    core.set_output_pin(0, to).expect("moved");
    assert_eq!(core.get_state().output_pins[0], to);
    assert_eq!(core.fetch_output_pin(0).expect("read back"), to);
    assert_eq!(
        core.pin_holder(to, None).map(|h| h.label),
        Some("Output 1".into())
    );
    // A held pin is refused before anything is sent: the dormant BCK, which
    // the firmware's own check would let through.
    let bck = core.get_state().i2s_bck_pin;
    assert_eq!(
        core.set_output_pin(0, bck),
        Err(DspiError::Refused { status: 2 })
    );
    assert_eq!(core.get_state().output_pins[0], to);
    let r = core.pin_check(f(K::Output, 0), bck).expect("refused");
    assert_eq!(r.holder.map(|h| h.label), Some("I2S BCK".into()));
    // Every pin setter answers on the demo: each function moves to a pin
    // the map offers it, and the map follows.
    type Setter = fn(&DspiCore, u8) -> Result<()>;
    let setters: [(PinFunction, Setter); 7] = [
        (f(K::Mck, 0), |c, p| c.set_mck_pin(p)),
        (f(K::I2sBck, 0), |c, p| c.set_i2s_bck_pin(0, p)),
        (f(K::I2sBckSlave, 0), |c, p| c.set_i2s_bck_pin(1, p)),
        (f(K::SpdifRx, 0), |c, p| c.set_spdif_rx_pin(0, p)),
        (f(K::I2sRx, 0), |c, p| c.set_i2s_rx_pin(0, p)),
        (f(K::AdatOut, 0), |c, p| c.set_adat_out_pin(p)),
        (f(K::AdatIn, 0), |c, p| c.set_adat_input_pin(p)),
    ];
    let mut moved = 0;
    for (func, set) in setters {
        let Some(now) = core
            .pin_map()
            .functions
            .into_iter()
            .find(|i| i.function == func)
        else {
            continue;
        };
        let Some(&to) = now.choices.iter().find(|p| **p != now.pin) else {
            continue;
        };
        set(&core, to).unwrap_or_else(|e| panic!("{func:?} to {to}: {e}"));
        let after = core
            .pin_map()
            .functions
            .into_iter()
            .find(|i| i.function == func);
        assert_eq!(after.map(|i| i.pin), Some(to), "{func:?}");
        moved += 1;
    }
    assert!(moved >= 5, "only {moved} pin setters moved on the demo");
    // Back to the defaults through the device.
    core.set_output_pin(0, PIN_NONE).expect("default");
    assert_eq!(core.get_state().output_pins[0], 6);
}

#[test]
fn cgswsb_no_device_no_map() {
    let core = DspiCore::new();
    assert_eq!(core.pin_map(), PinMap::default());
    // Without a device the setters say so; the gate does not get in the way.
    assert_eq!(core.set_output_pin(0, 6), Err(DspiError::Disconnected));
}
