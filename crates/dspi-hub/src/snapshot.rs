//! The per-device snapshot cache (`device.snapshot`, spec 7.5). Owned by WP HB3.

use dspi_link::messages::Snapshot;

/// One device's cached bulk image and status block.
#[derive(Debug, Default)]
pub struct SnapshotCache {}

impl SnapshotCache {
    /// A device packet went past: patch the image or invalidate it. Runs on the
    /// device's reader, so it must not block; a re-read goes through the worker's
    /// queue as [`crate::session::INTERNAL_SESSION_ID`]. Nothing to do until
    /// WP HB3 keeps an image.
    pub fn note_packet(&self, packet: &[u8]) {
        let _ = packet;
    }

    /// Store a freshly read image.
    pub fn set_bulk(&self, image: Vec<u8>) {
        let _ = image;
        todo!("WP HB3")
    }

    /// What `device.snapshot` answers for this handle.
    pub fn get(&self, handle: u8) -> Option<Snapshot> {
        let _ = handle;
        todo!("WP HB3")
    }
}
