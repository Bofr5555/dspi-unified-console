//! Spectrum analyser.
//!
//! Owned by WP C9 (docs/core-merge/PLAN.md).
#![allow(unused_variables)]

use std::sync::Arc;

use crate::callbacks::RtaListener;
#[allow(unused_imports)]
use crate::records::*;
#[allow(unused_imports)]
use crate::{DspiCore, Result};

impl DspiCore {
    /// Register (Some) or clear (None) the analyser wake-up listener. Once it
    /// returns after None, no call is in progress (on another thread).
    /// Replacing (Some -> Some) does not wait, and the old listener may then
    /// be released on the RTA thread when its running call ends.
    pub fn rta_set_listener(&self, listener: Option<Arc<dyn RtaListener>>) {
        todo!("WP C9")
    }

    /// Probe and start polling (compatible firmware only); stops on disconnect.
    pub fn rta_start(&self) {
        todo!("WP C9")
    }

    /// What to analyse; mask 0 releases the analyser.
    pub fn rta_set_request(&self, request: RtaRequest) {
        todo!("WP C9")
    }

    pub fn rta_get_caps(&self) -> RtaCapsInfo {
        todo!("WP C9")
    }

    pub fn rta_get_snapshot(&self) -> RtaSnapshot {
        todo!("WP C9")
    }
}

/// The analyser's state and its worker thread (WP C9).
#[derive(Default)]
pub(crate) struct RtaState {}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests {
    #[test]
    #[ignore = "WP C9"]
    fn wp_c9_is_implemented() {
        todo!("WP C9")
    }
}
