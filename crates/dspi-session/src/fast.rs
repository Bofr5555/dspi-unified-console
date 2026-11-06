//! The fast write path, for GUIs and bindings.
//!
//! [`Session::write`] is the CLI's path: it reads the value before and after,
//! confirms the change and journals it for undo, which costs three round trips
//! per change. A GUI writes at slider rate and keeps its own mirror, so it
//! needs the opposite: encode through the registry, send, done. The mirror is
//! then corrected by the device's own `PARAM_CHANGED` echo.
//!
//! Owned by WP S1 (docs/core-merge/PLAN.md).
#![warn(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

use dspi_proto::value::Value;

use crate::{Session, WriteError};

impl Session {
    /// Send one parameter change without reading anything back.
    ///
    /// Validates the value and the indices exactly as [`Session::write`] does
    /// (the same errors), encodes `wValue` and the payload through the
    /// registry, and sends it. Write-as-read opcodes return their status byte
    /// through [`Session::last_write_status`]. No undo journal entry, no
    /// Core 1 interlock prompt (the caller asks first), no confirm read.
    pub fn send(&mut self, path: &str, indices: &[u8], value: Value) -> Result<(), WriteError> {
        let _ = (path, indices, value);
        todo!("WP S1: fast write path")
    }
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests {
    #[test]
    #[ignore = "WP S1"]
    fn a_fast_write_sends_one_transfer_and_reads_nothing() {
        todo!("WP S1")
    }
}
