//! WP P1TK: confirmation tokens through the worker calls. The refusals are the
//! core's (`confirmation_required`, with its notice key), as through C.

use super::*;

fn run(h: u32, method: &str, params: Value) -> Value {
    let out = call(
        h,
        json!({ "method": method, "params": params })
            .to_string()
            .as_bytes(),
    );
    serde_json::from_slice(&out).unwrap()
}

fn ok(h: u32, method: &str, params: Value) -> Value {
    let r = run(h, method, params);
    assert_eq!(r["ok"], true, "{method}: {r}");
    r["value"].clone()
}

/// A refusal for want of a confirmation, in the core's words for `op`/`code`.
fn assert_refused(r: &Value, op: dspi_core::ConfirmOp, code: &str) {
    assert_eq!(r["ok"], false, "{r}");
    assert_eq!(r["code"], "confirmation_required", "{r}");
    assert_eq!(r["noticeKey"], "confirmation_required", "{r}");
    let text = r["error"].as_str().unwrap();
    assert!(text.starts_with(op.key()), "{r}");
    assert!(text.contains(words(code)), "{code}: {r}");
}

/// The core's words for each reason (`dspi_core::confirm`).
pub(super) fn words(code: &str) -> &'static str {
    match code {
        "missing" => "no confirmation token",
        "used" => "already used",
        "wrong_op" => "for another operation",
        "expired" => "ran out",
        "connection_changed" => "connection changed",
        _ => "not one this console gave",
    }
}

#[test]
fn wp_p1tk_tokens_through_the_worker_calls() {
    use dspi_core::ConfirmOp as Op;
    use dspi_core::confirm::refusal;
    guard::reset_for_test();
    let h = session_new().unwrap();
    ok(h, "open", json!({ "kind": "demo" }));
    // -- WP P1TZ -- required from the start; the switch is gone.
    for gone in ["requireConfirmation", "confirmationRequired"] {
        let r = run(h, gone, json!({ "on": false }));
        assert_eq!(r["ok"], false, "{gone}: {r}");
    }

    let save = json!({ "path": "preset.save", "indices": [1], "value": "trigger" });
    let token = |op: &str| {
        ok(h, "confirmToken", json!({ "op": op }))
            .as_str()
            .unwrap()
            .to_owned()
    };
    let with = |v: &Value, t: &str| {
        let mut v = v.clone();
        v["token"] = json!(t);
        v
    };

    // No token, another op's, an expired one is the core's own test; a good
    // one; the same again.
    assert_refused(
        &run(h, "write", save.clone()),
        Op::SavePreset,
        refusal::MISSING,
    );
    let reset = token("factory_reset");
    assert_refused(
        &run(h, "write", with(&save, &reset)),
        Op::SavePreset,
        refusal::WRONG_OP,
    );
    assert_refused(
        &run(h, "write", with(&save, "save_preset-00")),
        Op::SavePreset,
        refusal::UNKNOWN,
    );
    let t = token("save_preset");
    ok(h, "write", with(&save, &t));
    assert_refused(
        &run(h, "write", with(&save, &t)),
        Op::SavePreset,
        refusal::USED,
    );

    // The dedicated calls take `token` too.
    assert_refused(
        &run(h, "factoryReset", Value::Null),
        Op::FactoryReset,
        refusal::MISSING,
    );
    ok(h, "factoryReset", json!({ "token": reset }));
    let copy = json!({ "source": 1, "dest": 2 });
    assert_refused(
        &run(h, "presetCopy", copy.clone()),
        Op::CopyPreset,
        refusal::MISSING,
    );
    ok(h, "presetCopy", with(&copy, &token("copy_preset")));
    // -- WP P1TZF -- (review p1tz F4) the import refuses the same way
    let doc = ok(h, "configExport", json!({ "name": "P1TZF" }))["text"].clone();
    let import = json!({ "text": doc });
    assert_refused(
        &run(h, "configImport", import.clone()),
        Op::ConfigImport,
        refusal::MISSING,
    );
    let wrong = token("save_params");
    assert_refused(
        &run(h, "configImport", with(&import, &wrong)),
        Op::ConfigImport,
        refusal::WRONG_OP,
    );
    ok(h, "configImport", with(&import, &token("config_import")));

    // A token does not cross a reconnect.
    let stale = token("save_params");
    ok(h, "close", Value::Null);
    ok(h, "open", json!({ "kind": "demo" }));
    let dev_save = json!({ "path": "dev.save", "value": "trigger" });
    assert_refused(
        &run(h, "write", with(&dev_save, &stale)),
        Op::SaveParams,
        refusal::CONNECTION_CHANGED,
    );

    // An operation the core does not have.
    assert_eq!(
        run(h, "confirmToken", json!({ "op": "format" }))["code"],
        "invalid"
    );
    assert!(session_free(h).unwrap());
}
