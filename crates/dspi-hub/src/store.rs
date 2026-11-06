//! What a hub keeps on disk: hub id, name, token hashes, device names. Owned by
//! WP HB1. A plain file, saved atomically. Not a keychain, because the hub holds
//! only hashes (spec 6.2).

use std::path::Path;

use crate::error::HubError;

/// The persistent state, as loaded.
#[derive(Debug, Default)]
pub struct HubStore {}

impl HubStore {
    /// Load from `dir`, creating it and a new hub id on first use. An unreadable
    /// file is set aside, never a panic: the hub then has no paired clients.
    pub fn load(dir: &Path) -> Result<Self, HubError> {
        let _ = dir;
        todo!("WP HB1")
    }

    /// Save, replacing the old file atomically.
    pub fn save(&self) -> Result<(), HubError> {
        todo!("WP HB1")
    }
}
