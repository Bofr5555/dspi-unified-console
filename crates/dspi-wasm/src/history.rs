//! `undo`, `redo`, `canUndo`, `canRedo` and `history` (WP P0W): the core's
//! change history (`DspiCore::undo` and the rest), as JSON.
//!
//! `undo` and `redo` answer `{entry, skipped}`: the change reversed or
//! applied again (`null` when nothing moved) and, for undo, the changes
//! stepped over that cannot be undone. `history` (`{n}`) answers the newest
//! `n` changes, newest first, each `{path, indices, before, after, source,
//! atMs, undoable}` with values as `write` takes them. `poll`'s `flags`
//! carry `NOTIFY_HISTORY` (`1 << 12`) when the history moved.

use dspi_core::DspiCore;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::calls::{CallError, tidy};

/// `history`'s parameters.
#[derive(Debug, Deserialize)]
pub struct HistoryParams {
    pub n: u32,
}

/// `undo`.
pub fn undo(core: &DspiCore) -> Result<Value, CallError> {
    Ok(tidy(&core.undo()?))
}

/// `redo`.
pub fn redo(core: &DspiCore) -> Result<Value, CallError> {
    Ok(tidy(&core.redo()?))
}

/// `canUndo` and `canRedo` at once: `{canUndo, canRedo}`.
pub fn can(core: &DspiCore) -> Value {
    json!({ "canUndo": core.can_undo(), "canRedo": core.can_redo() })
}

/// `history`.
pub fn history(core: &DspiCore, p: &HistoryParams) -> Value {
    tidy(&core.history(p.n))
}
