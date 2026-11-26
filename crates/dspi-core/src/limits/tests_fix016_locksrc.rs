//! FIX016 LOCKSRC (owner: "locked users may change the source", D4's remote
//! home): one core rule, the input source is among the writes the
//! locked-user state lets through; every other write is still refused.

use std::sync::Arc;

use super::*;
use crate::write_param::ParamValue;

const PIN: &str = "~fix016~locksrc~";

fn demo() -> Arc<DspiCore> {
    let core = DspiCore::new();
    core.open_demo().unwrap();
    core
}

fn code_of(e: &DspiError) -> &str {
    match e {
        DspiError::InstallerLock { code, .. } => code,
        _ => "",
    }
}

#[test]
fn fix016_locksrc_a_locked_user_changes_the_source_and_nothing_else() {
    let core = demo();
    core.set_input_source(0).unwrap();
    core.installer_pin_set(PIN.into()).unwrap();
    core.installer_lock_cover(vec!["front".into()]).unwrap();
    assert!(core.installer_lock().unwrap().locked);
    // The source passes, by its setter and by its registry path
    core.set_input_source(1).unwrap();
    assert_eq!(core.get_state().input_source, 1);
    core.write_param("in.source".into(), vec![], ParamValue::Choice(0))
        .unwrap();
    assert_eq!(core.get_state().input_source, 0);
    assert!(LOCKED_USER_WRITES.contains(&"in.source"));
    // A gain is still refused, with the sentence that names the source
    let before = core.get_state().output_gain_db;
    let e = core.set_output_gain(1, 1.5).unwrap_err();
    assert_eq!(code_of(&e), "locked", "{e:?}");
    assert!(
        e.to_string()
            .contains("only volume, mute, source and presets can be changed"),
        "{e}"
    );
    assert_eq!(core.get_state().output_gain_db, before);
}
