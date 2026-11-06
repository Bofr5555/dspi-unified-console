//! Control surfaces (JSON for the editor).
//!
//! Owned by WP C14 (docs/core-merge/PLAN.md).
#![allow(unused_variables)]

#[allow(unused_imports)]
use crate::records::*;
use crate::{DspiCore, Result};

impl DspiCore {
    /// Read the whole CS configuration from the device.
    pub fn cs_fetch(&self) -> Result<()> {
        todo!("WP C14")
    }

    /// The CS configuration as JSON (`cs::cs_snapshot`).
    pub fn cs_snapshot(&self) -> String {
        todo!("WP C14")
    }

    /// One CS operation `{op, index, ...}`; JSON {ok, status, error, ...}.
    /// Some ops write flash (save) — the bridge gates them (DangerousOps).
    pub fn cs_apply(&self, op: String) -> String {
        todo!("WP C14")
    }

    /// GPIOs held by live CS bindings.
    pub fn cs_pin_uses(&self) -> Vec<PinUse> {
        todo!("WP C14")
    }
}

/// The control-surface editor's state (WP C14), on the session's CS
/// model (WP S4).
#[derive(Default)]
pub(crate) struct CsState {}

#[cfg(test)]
mod tests {
    #[test]
    #[ignore = "WP C14"]
    fn wp_c14_is_implemented() {
        todo!("WP C14")
    }
}
