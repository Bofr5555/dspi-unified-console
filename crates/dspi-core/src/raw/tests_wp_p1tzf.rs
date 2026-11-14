//! WP P1TZF (review p1tz F1): a raw request never carries a dangerous
//! operation, which takes a confirmation token through its typed
//! `_confirmed` call; everything else still goes out raw.

use std::sync::Arc;

use dspi_transport::FakeDevice;

use super::*;
use crate::ConfirmOp;

/// The demo device behind a core.
fn demo() -> Arc<DspiCore> {
    let core = DspiCore::new();
    core.open_transport(Box::new(FakeDevice::demo()), "P1TZF".into(), 0)
        .unwrap();
    core
}

const GATED: [(u8, &str); 6] = [
    (op::REQ_PRESET_SAVE, "save_preset_confirmed"),
    (op::REQ_PRESET_DELETE, "delete_preset_confirmed"),
    (op::REQ_SAVE_PARAMS, "save_params_confirmed"),
    (op::REQ_SAVE_OUTPUT_CONFIG, "save_output_config_confirmed"),
    (op::REQ_FACTORY_RESET, "factory_reset_confirmed"),
    (op::REQ_ENTER_BOOTLOADER, "enter_bootloader_confirmed"),
];

#[test]
fn wp_p1tzf_the_gated_opcodes_are_refused_raw_naming_the_confirmed_call() {
    let core = demo();
    let before = core.get_state();
    for (code, call) in GATED {
        for r in [
            core.raw_get(code, 1, 1).map(|_| ()),
            core.raw_set(code, 1, vec![0]),
        ] {
            match r {
                Err(DspiError::InvalidArgument { msg }) => {
                    assert!(msg.contains(call), "0x{code:02X}: {msg}");
                    assert!(msg.contains("confirmation token"), "{msg}");
                }
                other => panic!("0x{code:02X} went out raw: {other:?}"),
            }
        }
    }
    // Nothing happened: the device still answers and nothing was saved or reset.
    assert!(core.is_connected());
    assert_eq!(core.get_state(), before);
    // The typed calls still do it, with a token.
    let t = core.confirm_token(ConfirmOp::SavePreset);
    core.save_preset_confirmed(1, Some(t)).unwrap();
}

#[test]
fn wp_p1tzf_everything_else_still_goes_out_raw() {
    let core = demo();
    // A plain read, a write-as-read and an OUT write that take no token.
    assert!(core.raw_get(op::REQ_PRESET_GET_ACTIVE, 0, 1).is_ok());
    assert!(core.raw_get(op::REQ_PRESET_LOAD, 0, 1).is_ok());
    assert!(
        core.raw_set(
            op::REQ_SET_MASTER_VOLUME,
            0,
            (-6.0f32).to_le_bytes().to_vec()
        )
        .is_ok()
    );
    assert!(raw_gated_call(op::REQ_PRESET_LOAD).is_none());
    assert!(raw_gated_call(op::REQ_SAVE_MASTER_VOLUME).is_none());
}
