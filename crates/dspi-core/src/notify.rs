//! Notifications.
//!
//! Owned by WP C7 (docs/core-merge/PLAN.md).
#![allow(unused_variables)]

use std::sync::Arc;

use crate::callbacks::NotifyListener;
#[allow(unused_imports)]
use crate::records::*;
#[allow(unused_imports)]
use crate::{DspiCore, Result};

impl DspiCore {
    /// Register (Some) or clear (None) the wake-up listener of the notify thread.
    /// Clearing waits for a call in progress on another thread; replacing
    /// (Some -> Some) does not wait, and the old listener may then be
    /// released on the notify thread when its running call ends.
    pub fn set_notify_listener(&self, listener: Option<Arc<dyn NotifyListener>>) {
        todo!("WP C7")
    }

    /// Apply queued notifications to the state; returns what changed.
    pub fn process_notifications(&self) -> NotifyResult {
        todo!("WP C7")
    }
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests {
    #[test]
    #[ignore = "WP C7"]
    fn wp_c7_is_implemented() {
        todo!("WP C7")
    }
}
