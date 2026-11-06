//! Configuration and filter files.
//!
//! Owned by WP C13 (docs/core-merge/PLAN.md).
#![allow(unused_variables)]

#[allow(unused_imports)]
use crate::records::*;
#[allow(unused_imports)]
use crate::{DspiCore, Result};

impl DspiCore {
    /// The device configuration as a `.dspipreset` document. `links` = the
    /// app's input-pair link mask.
    pub fn config_export(&self, name: String, app_version: String, links: u8) -> String {
        todo!("WP C13")
    }

    /// Apply a document (`options`: 1 = volumes, 2 = hardware I/O). JSON
    /// {ok, error, lines, clean, links (or null), hardwareWritten}.
    pub fn config_import(&self, text: String, options: u32) -> String {
        todo!("WP C13")
    }

    /// Every channel's filters as text (`inputs` = live inputs to write).
    pub fn filters_export(&self, inputs: u8) -> String {
        todo!("WP C13")
    }

    /// What a filter file holds, for the channel picker (JSON).
    pub fn filters_inspect(&self, text: String, inputs: u8) -> String {
        todo!("WP C13")
    }

    /// Apply a filter file to `wires` (wire indices). JSON {ok, error, lines}.
    pub fn filters_import(&self, text: String, wires: Vec<u8>) -> String {
        todo!("WP C13")
    }
}

/// Check a `.dspipreset` document: JSON {ok, error, name, platform,
/// firmware, savedUtc} (`dspi_config_inspect`).
pub fn config_inspect(text: String) -> String {
    todo!("WP C13")
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests {
    #[test]
    #[ignore = "WP C13"]
    fn wp_c13_is_implemented() {
        todo!("WP C13")
    }
}
