//! Hotplug: arrival and departure events, and reconnecting the selected
//! device when it comes back.
//!
//! Owned by WP C1b (docs/core-merge/PLAN.md). Listeners are called with the
//! `Inner` lock released (collect the events under the lock, drop the guard,
//! then call), and must only schedule work (`callbacks.rs`).
#![allow(unused_variables)]

use std::sync::Arc;

use crate::DspiCore;
use crate::callbacks::DeviceEvents;

impl DspiCore {
    /// Register (`Some`) or clear (`None`) the arrival/departure listener.
    /// Clearing waits for a call in progress on another thread.
    pub fn set_hotplug_callback(&self, listener: Option<Arc<dyn DeviceEvents>>) {
        todo!("WP C1b")
    }

    /// Compare the bus with the last scan and report arrivals and
    /// departures; reconnect the selected device when it reappears, and
    /// close it when it departs. Desktop apps call it about every 500 ms.
    /// A no-op on Android (the app gets USB broadcasts instead).
    pub fn poll_hotplug(&self) {
        todo!("WP C1b")
    }
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests {
    #[test]
    #[ignore = "WP C1b"]
    fn wp_c1b_a_replugged_device_reconnects() {
        todo!("WP C1b")
    }
}
