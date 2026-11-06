//! Names and diagnostics the old desktop core lacked: input-source names
//! and the device name (0x54..0x57), output latency (0x03) and clock
//! diagnostics (0x82). The Terminal's registry already speaks them
//! (`input.name`, `device.name`, ...).
//!
//! Owned by WP C8 (docs/core-merge/PLAN.md).
#![allow(unused_variables)]

use crate::records::*;
use crate::{DspiCore, Result};

impl DspiCore {
    /// The user's name for input source `source` (`REQ_GET_INPUT_SOURCE_NAME`);
    /// "" when unnamed. `Unsupported` on firmware without names.
    pub fn get_input_source_name(&self, source: u8) -> Result<String> {
        todo!("WP C8")
    }

    /// Name input source `source`; "" clears it. At most 32 bytes of UTF-8.
    pub fn set_input_source_name(&self, source: u8, name: String) -> Result<()> {
        todo!("WP C8")
    }

    /// The device's own name (`REQ_GET_DEVICE_NAME`), "" when unnamed.
    pub fn get_device_name(&self) -> Result<String> {
        todo!("WP C8")
    }

    /// Rename the device; "" clears it. At most 32 bytes of UTF-8.
    pub fn set_device_name(&self, name: String) -> Result<()> {
        todo!("WP C8")
    }

    /// `REQ_OUTPUT_LATENCY` status.
    pub fn fetch_output_latency(&self) -> Result<OutputLatency> {
        todo!("WP C8")
    }

    /// `REQ_GET_CLOCK_DIAG`; `available` is false on firmware built without it.
    pub fn fetch_clock_diag(&self) -> Result<ClockDiag> {
        todo!("WP C8")
    }
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests {
    #[test]
    #[ignore = "WP C8"]
    fn names_round_trip_through_the_demo_device() {
        todo!("WP C8")
    }
}
