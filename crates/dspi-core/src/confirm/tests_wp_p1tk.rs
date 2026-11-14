//! WP P1TK: confirmation tokens. Every gated call with no token, an expired
//! one, one for another operation, a reused one and a good one; a token never
//! crosses a reconnect. "Nothing sent" is counted at the transport.

use std::time::Duration;

use super::*;
use crate::ext::tests_wp_c8::{Probed, k340};

/// A gated call: `Err(code)` for a confirmation refusal, `Err("other: ..")`
/// for any other error.
type Gated = Box<dyn Fn(&DspiCore, Option<String>) -> std::result::Result<(), String>>;

fn code_of(r: Result<()>) -> std::result::Result<(), String> {
    match r {
        Ok(()) => Ok(()),
        Err(DspiError::ConfirmationRequired { code, .. }) => Err(code),
        Err(e) => Err(format!("other: {e:?}")),
    }
}

/// `config_import`'s JSON reply, read back into a code by its words.
fn import_code(reply: &str) -> std::result::Result<(), String> {
    let v: serde_json::Value = serde_json::from_str(reply).unwrap();
    if v["ok"] == true {
        return Ok(());
    }
    if v["noticeKey"] != "confirmation_required" {
        return Err(format!("other: {v}"));
    }
    let text = v["error"].as_str().unwrap();
    for code in [
        refusal::MISSING,
        refusal::UNKNOWN,
        refusal::USED,
        refusal::WRONG_OP,
        refusal::EXPIRED,
        refusal::CONNECTION_CHANGED,
    ] {
        if refused(ConfirmOp::ConfigImport, code).to_string() == text {
            return Err(code.to_owned());
        }
    }
    panic!("not a refusal's words: {text}");
}

/// The K340-like demo behind a counting transport, read once.
fn rig() -> Probed {
    let p = k340();
    p.core.fetch_all().unwrap();
    p
}

/// Every gated call on a connected device, with the operation it is.
fn gated_calls(doc: String) -> Vec<(ConfirmOp, Gated)> {
    vec![
        (
            ConfirmOp::SavePreset,
            Box::new(|c: &DspiCore, t| code_of(c.save_preset_confirmed(1, t))),
        ),
        (
            ConfirmOp::CopyPreset,
            Box::new(|c: &DspiCore, t| code_of(c.copy_preset_confirmed(1, 2, t))),
        ),
        (
            ConfirmOp::DeletePreset,
            Box::new(|c: &DspiCore, t| code_of(c.delete_preset_confirmed(2, t))),
        ),
        (
            ConfirmOp::SaveParams,
            Box::new(|c: &DspiCore, t| code_of(c.save_params_confirmed(t))),
        ),
        (
            ConfirmOp::FactoryReset,
            Box::new(|c: &DspiCore, t| code_of(c.factory_reset_confirmed(t))),
        ),
        (
            ConfirmOp::SaveOutputConfig,
            Box::new(|c: &DspiCore, t| code_of(c.save_output_config_confirmed(t))),
        ),
        (
            ConfirmOp::ConfigImport,
            Box::new(move |c: &DspiCore, t| {
                import_code(&c.config_import_confirmed(doc.clone(), 0, u32::MAX, t))
            }),
        ),
        (
            ConfirmOp::VregSave,
            // 0: back to the build default; no word needed.
            Box::new(|c: &DspiCore, t| code_of(c.core_vreg_manual_set_confirmed(0, None, t))),
        ),
    ]
}

/// The refusals for `call`, all before any traffic, then a good token, then
/// that token again.
fn check_gate(p: &Probed, op: ConfirmOp, call: &Gated) {
    let core = &p.core;
    let other = ConfirmOp::ALL.into_iter().find(|o| *o != op).unwrap();
    let before = p.traffic();

    assert_eq!(call(core, None), Err(refusal::MISSING.into()), "{op:?}");
    assert_eq!(
        call(core, Some("save_preset-0000".into())),
        Err(refusal::UNKNOWN.into()),
        "{op:?}"
    );
    let wrong = core.confirm_token(other);
    assert_eq!(
        call(core, Some(wrong.clone())),
        Err(refusal::WRONG_OP.into())
    );
    let old = core.confirm_token(op);
    core.lock()
        .confirm
        .age_for_test(Duration::from_millis(u64::from(CONFIRM_TOKEN_TTL_MS) + 1));
    assert_eq!(
        call(core, Some(old)),
        Err(refusal::EXPIRED.into()),
        "{op:?}"
    );
    assert_eq!(p.traffic(), before, "{op:?}: a refusal sent something");

    let good = core.confirm_token(op);
    assert_eq!(call(core, Some(good.clone())), Ok(()), "{op:?}");
    assert_ne!(p.traffic(), before, "{op:?}: the good call sent nothing");
    assert_eq!(call(core, Some(good)), Err(refusal::USED.into()), "{op:?}");
    // The wrong-op token was not spent by the refusal: still good for its own.
    assert_ne!(call(core, Some(wrong)), Err(refusal::USED.into()));
}

#[test]
fn wp_p1tk_every_gated_call_needs_a_good_token_when_required() {
    let p = rig();
    let doc = p
        .core
        .config_export("P1TK".into(), "test".into(), 0)
        .unwrap();
    p.core.require_confirmation(true);
    assert!(p.core.confirmation_required());
    for (op, call) in gated_calls(doc) {
        check_gate(&p, op, &call);
    }
}

/// The core-voltage commit: the same gate before the flow's own checks.
#[test]
fn wp_p1tk_the_vreg_commit_is_gated_before_the_flow() {
    let p = rig();
    p.core.require_confirmation(true);
    let before = p.traffic();
    assert_eq!(
        code_of(p.core.core_vreg_cal_commit_confirmed(None, None)),
        Err(refusal::MISSING.into())
    );
    assert_eq!(p.traffic(), before);
    // Noted for the view, as the flow's own refusals are.
    assert!(p.core.core_vreg_cal_view().last_error.is_some());
    let t = p.core.confirm_token(ConfirmOp::VregSave);
    // Let through: now the flow's own answer (nothing has passed a soak).
    let r = p.core.core_vreg_cal_commit_confirmed(None, Some(t.clone()));
    assert!(
        !matches!(r, Err(DspiError::ConfirmationRequired { .. })),
        "{r:?}"
    );
    assert_eq!(
        code_of(p.core.core_vreg_cal_commit_confirmed(None, Some(t))),
        Err(refusal::USED.into())
    );
}

/// The bootloader: refused with nothing sent, then let through.
#[test]
fn wp_p1tk_the_bootloader_is_gated() {
    let p = rig();
    p.core.require_confirmation(true);
    let before = p.traffic();
    assert_eq!(
        code_of(p.core.enter_bootloader()),
        Err(refusal::MISSING.into())
    );
    let wrong = p.core.confirm_token(ConfirmOp::FactoryReset);
    assert_eq!(
        code_of(p.core.enter_bootloader_confirmed(Some(wrong))),
        Err(refusal::WRONG_OP.into())
    );
    assert_eq!(p.traffic(), before);
    let t = p.core.confirm_token(ConfirmOp::EnterBootloader);
    assert_eq!(p.core.enter_bootloader_confirmed(Some(t.clone())), Ok(()));
    assert_ne!(p.traffic(), before);
    assert_eq!(
        code_of(p.core.enter_bootloader_confirmed(Some(t))),
        Err(refusal::USED.into())
    );
}

/// Off (the default): the plain calls work as before; a token that is given
/// is still checked.
#[test]
fn wp_p1tk_off_by_default_and_a_given_token_is_still_checked() {
    let p = rig();
    assert!(!p.core.confirmation_required());
    p.core.save_preset(1).unwrap();
    p.core.save_params().unwrap();
    p.core.save_output_config().unwrap();
    assert_eq!(
        code_of(p.core.save_preset_confirmed(1, Some("nonsense".into()))),
        Err(refusal::UNKNOWN.into())
    );
    let t = p.core.confirm_token(ConfirmOp::SavePreset);
    p.core.save_preset_confirmed(1, Some(t)).unwrap();
    // On and off again.
    p.core.require_confirmation(true);
    assert!(p.core.save_preset(1).is_err());
    p.core.require_confirmation(false);
    p.core.save_preset(1).unwrap();
}

/// A token is for the connection it was minted on: a disconnect and a
/// reconnect end it, to the same device or another.
#[test]
fn wp_p1tk_a_token_never_crosses_a_reconnect() {
    let core = DspiCore::new();
    core.open_demo().unwrap();
    core.require_confirmation(true);
    let t = core.confirm_token(ConfirmOp::SavePreset);
    let spare = core.confirm_token(ConfirmOp::SavePreset);
    core.disconnect();
    core.open_demo().unwrap();
    assert!(core.confirmation_required(), "the requirement stays");
    assert_eq!(
        code_of(core.save_preset_confirmed(1, Some(t))),
        Err(refusal::CONNECTION_CHANGED.into())
    );
    // Gone while disconnected too.
    core.disconnect();
    assert_eq!(
        code_of(core.save_preset_confirmed(1, Some(spare))),
        Err(refusal::CONNECTION_CHANGED.into())
    );
    // One minted on the new connection is good.
    core.open_demo().unwrap();
    let fresh = core.confirm_token(ConfirmOp::SavePreset);
    core.save_preset_confirmed(1, Some(fresh)).unwrap();
}

/// The firmware install: minted with the board in its bootloader (no device
/// open), good until a device is opened; one minted while the device was
/// still connected is not.
#[test]
fn wp_p1tk_a_firmware_token_belongs_to_the_bootloader_wait() {
    let core = DspiCore::new();
    core.require_confirmation(true);
    let drive = || crate::BootloaderDrive {
        path: "/nonexistent/p1tk-drive".into(),
        platform: 1,
        info: String::new(),
    };
    let flash = |t: Option<String>| {
        code_of(core.flash_confirmed(
            vec![0; 16],
            drive(),
            crate::firmware_guard_board_in_bootloader(),
            String::new(),
            t,
        ))
    };
    assert_eq!(flash(None), Err(refusal::MISSING.into()));

    core.open_demo().unwrap();
    let connected = core.confirm_token(ConfirmOp::FirmwareFlash);
    core.disconnect();
    assert_eq!(
        flash(Some(connected)),
        Err(refusal::CONNECTION_CHANGED.into())
    );

    let waiting = core.confirm_token(ConfirmOp::FirmwareFlash);
    let stale = core.confirm_token(ConfirmOp::FirmwareFlash);
    // Let through: the image is refused next, before any drive is touched.
    let r = flash(Some(waiting));
    assert!(
        r.as_ref()
            .is_err_and(|e| e.starts_with("other: InvalidArgument")),
        "{r:?}"
    );
    // A device opened and closed meanwhile ends a token minted with none.
    core.open_demo().unwrap();
    core.disconnect();
    assert_eq!(flash(Some(stale)), Err(refusal::CONNECTION_CHANGED.into()));
}

/// The registry's gated paths refuse without a token when tokens are
/// required, and take one through `write_param_confirmed`.
#[test]
fn wp_p1tk_write_param_paths_are_gated() {
    let p = rig();
    let core = &p.core;
    core.require_confirmation(true);
    for (path, indices, op) in [
        ("preset.save", vec![1u8], ConfirmOp::SavePreset),
        ("dev.save", vec![], ConfirmOp::SaveParams),
        ("dev.save.io", vec![], ConfirmOp::SaveOutputConfig),
        ("preset.delete", vec![1u8], ConfirmOp::DeletePreset),
    ] {
        assert_eq!(gated_path(path), Some(op));
        let r = core.write_param(path.into(), indices.clone(), ParamValue::Trigger);
        assert!(
            matches!(r, Err(DspiError::ConfirmationRequired { ref code, .. }) if code == refusal::MISSING),
            "{path}: {r:?}"
        );
        let t = core.confirm_token(op);
        assert_eq!(
            core.write_param_confirmed(path.into(), indices.clone(), ParamValue::Trigger, Some(t)),
            Ok(ParamValue::Trigger),
            "{path}"
        );
    }
    // A wrong shape is refused before the gate: the token stays unspent.
    let t = core.confirm_token(ConfirmOp::SavePreset);
    assert!(matches!(
        core.write_param_confirmed(
            "preset.save".into(),
            vec![],
            ParamValue::Trigger,
            Some(t.clone())
        ),
        Err(DspiError::InvalidArgument { .. })
    ));
    // A path that is not gated takes no token, and is written without one.
    assert!(matches!(
        core.write_param_confirmed(
            "out.gain".into(),
            vec![0],
            ParamValue::Float(-3.0),
            Some(t.clone())
        ),
        Err(DspiError::InvalidArgument { .. })
    ));
    assert!(
        core.write_param_confirmed("out.gain".into(), vec![0], ParamValue::Float(-3.0), None)
            .is_ok()
    );
    core.save_preset_confirmed(1, Some(t)).unwrap();
}

/// Tokens differ, name their operation, and the core remembers a bounded
/// number of them.
#[test]
fn wp_p1tk_tokens_are_distinct_and_bounded() {
    let core = DspiCore::new();
    let mut seen = std::collections::BTreeSet::new();
    for i in 0..200 {
        let op = ConfirmOp::ALL[i % ConfirmOp::ALL.len()];
        let t = core.confirm_token(op);
        assert!(t.starts_with(op.key()), "{t}");
        assert!(seen.insert(t));
    }
    assert!(core.lock().confirm.minted.len() <= REMEMBER_MAX);
    for op in ConfirmOp::ALL {
        assert_eq!(ConfirmOp::from_key(op.key()), Some(op));
    }
    assert_eq!(ConfirmOp::from_key("format_disk"), None);
}

/// The refusal is a typed error with its own notice row.
#[test]
fn wp_p1tk_the_refusal_has_a_notice() {
    let e = refused(ConfirmOp::FactoryReset, refusal::EXPIRED);
    assert_eq!(e.notice_key(), "confirmation_required");
    let n = crate::notice_for(e.clone());
    assert_eq!(n.key, "confirmation_required");
    assert_eq!(n.kind, crate::NoticeKind::Warning);
    assert_eq!(n.detail, e.to_string());
    assert!(n.detail.contains("factory_reset"), "{}", n.detail);
}
