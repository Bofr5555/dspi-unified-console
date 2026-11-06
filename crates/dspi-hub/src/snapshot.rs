//! The per-device snapshot cache (`device.snapshot`, spec 7.5). Owned by WP HB3.

use std::sync::{Mutex, PoisonError};

use dspi_link::messages::Snapshot;

/// One device's cached bulk image and status block.
#[derive(Debug, Default)]
pub struct SnapshotCache {
    image: Mutex<Option<Vec<u8>>>,
}

impl SnapshotCache {
    /// A device packet went past: patch the image or invalidate it. Runs on the
    /// device's reader, so it must not block. Returns whether the image needs a
    /// re-read; the reader then queues one, coalesced to one pending per device, so a
    /// notification flood cannot grow the worker's queue. Nothing to do until WP HB3
    /// keeps an image.
    pub fn note_packet(&self, packet: &[u8]) -> bool {
        let _ = packet;
        false
    }

    /// Store a freshly read image. Called on the device's worker.
    pub fn set_bulk(&self, image: Vec<u8>) {
        *self.image.lock().unwrap_or_else(PoisonError::into_inner) = Some(image);
    }

    /// The stored image, if there is one.
    pub fn bulk(&self) -> Option<Vec<u8>> {
        self.image
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// What `device.snapshot` answers for this handle.
    pub fn get(&self, handle: u8) -> Option<Snapshot> {
        let _ = handle;
        todo!("WP HB3")
    }
}
