//! Notification monitor.
//!
//! Owned by WP C7 (docs/core-merge/PLAN.md).
#![allow(unused_variables)]

#[allow(unused_imports)]
use crate::records::*;
#[allow(unused_imports)]
use crate::{DspiCore, Result};

impl DspiCore {
    /// Log entries after `after_id` (typed replacement of the TSV buffer).
    pub fn monitor_read(&self, after_id: u64) -> MonitorBatch {
        todo!("WP C7")
    }

    pub fn monitor_clear(&self) {
        todo!("WP C7")
    }
}

/// The notification monitor's log: what arrived, decoded, with ids for
/// `monitor_read(after_id)`. Bounded.
#[derive(Default)]
pub(crate) struct MonitorLog {}

#[cfg(test)]
mod tests {
    #[test]
    #[ignore = "WP C7"]
    fn wp_c7_is_implemented() {
        todo!("WP C7")
    }
}
