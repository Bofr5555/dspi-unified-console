//! The per-device snapshot cache (`device.snapshot`, spec 7.5). Owned by WP HB3.
//!
//! One bulk image per device, read through the worker, in the device's order. It is
//! kept fresh by patching from `PARAM_CHANGED`, whose `(offset, size)` address the same
//! struct (notification spec 3), and dropped and re-read on `BULK_INVALIDATED` or a
//! `seq` gap, which means a change was missed.
//!
//! **Cold until asked.** Nothing is read until the first `device.snapshot`, so a hub
//! nobody asks costs the device no bulk read, the app's in-process hub included. From
//! then on the image stays fresh, and the next client's first screen is one round trip.
//!
//! **A change during a read** is not lost. A `PARAM_CHANGED` that lands while the read
//! runs may or may not be in the image, so it is noted and patched on top, in order,
//! which ends at the last value notified. An invalidation during a read discards that
//! image, as another read is on its way.
//!
//! **No stream, no trust.** A device with no notification stream cannot say it changed,
//! so each snapshot re-reads, as the bridge does with `notify_enable` off.
//!
//! `status_b64` is left out, which the client takes as absent: it would go stale between
//! re-reads, and the Swift hub never fills it either. A client's meters are a poll.

use std::sync::{Condvar, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use dspi_link::messages::Snapshot;
use dspi_proto::generated::notify as nfy;
use dspi_proto::wire::WireHeader;

const EVT_PARAM_CHANGED: u8 = nfy::NOTIFY_EVT_PARAM_CHANGED as u8;
const EVT_BULK_INVALIDATED: u8 = nfy::NOTIFY_EVT_BULK_INVALIDATED as u8;
/// Bytes before a `PARAM_CHANGED`'s value: version, event, flags, seq, offset, size,
/// source, 3 reserved.
const PARAM_HEADER: usize = 12;
/// Changes noted during one read. A sweep coalesces in the firmware (one per parameter),
/// so past this it is cheaper to read again.
const MAX_NOTED: usize = 256;

/// Why `device.snapshot` has no answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum SnapshotRefused {
    /// No such device online (`no_device`).
    #[error("no such device")]
    NoDevice,
    /// The image could not be read in time (`busy`). The client reads the device itself.
    #[error("the device's settings could not be read just now")]
    Busy,
}

impl SnapshotRefused {
    /// The `code` of the `err` reply (spec 7.1).
    pub fn code(&self) -> &'static str {
        match self {
            SnapshotRefused::NoDevice => "no_device",
            SnapshotRefused::Busy => "busy",
        }
    }
}

#[derive(Debug, Default)]
struct State {
    image: Option<Vec<u8>>,
    updated_at: Option<Instant>,
    /// Someone has asked, so the image is kept.
    wanted: bool,
    /// A read has been asked for and has not ended.
    reading: bool,
    /// Bumped by each invalidation; an image started before one is not good.
    invalidations: u64,
    /// `invalidations` when the current read started.
    read_started_at: u64,
    /// `(offset, value)` of changes since the current read started.
    noted: Vec<(usize, Vec<u8>)>,
    last_seq: Option<u8>,
    /// The device has no notification stream.
    untrusted: bool,
    closed: bool,
}

impl State {
    /// A change was missed or cannot be applied. True: a read is needed.
    fn invalidate(&mut self) -> bool {
        self.image = None;
        self.noted.clear();
        self.invalidations = self.invalidations.wrapping_add(1);
        self.reading = true;
        true
    }
}

/// `offset` and the value of a `PARAM_CHANGED`, if it can be applied to an image of
/// `len` bytes. The header is shape, not data: a notification never changes it.
fn param_change(packet: &[u8]) -> Option<(usize, &[u8])> {
    let offset = usize::from(u16::from_le_bytes([*packet.get(4)?, *packet.get(5)?]));
    let size = usize::from(u16::from_le_bytes([*packet.get(6)?, *packet.get(7)?]));
    let value = packet.get(PARAM_HEADER..PARAM_HEADER.checked_add(size)?)?;
    (size > 0 && offset >= WireHeader::LEN).then_some((offset, value))
}

fn patch(image: &mut [u8], offset: usize, value: &[u8]) -> bool {
    let target = offset
        .checked_add(value.len())
        .and_then(|end| image.get_mut(offset..end));
    match target {
        Some(target) => {
            target.copy_from_slice(value);
            true
        }
        None => false,
    }
}

/// One device's cached bulk image.
#[derive(Debug, Default)]
pub struct SnapshotCache {
    state: Mutex<State>,
    changed: Condvar,
}

impl SnapshotCache {
    fn lock(&self) -> MutexGuard<'_, State> {
        // Each update is complete before anything that could panic.
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// A device packet went past: patch the image or invalidate it. Runs on the
    /// device's reader, so it must not block. Returns whether the image needs a
    /// re-read; the reader then queues one, coalesced to one pending per device, so a
    /// notification flood cannot grow the worker's queue.
    pub fn note_packet(&self, packet: &[u8]) -> bool {
        self.note_packet_at(packet, Instant::now())
    }

    fn note_packet_at(&self, packet: &[u8], now: Instant) -> bool {
        let mut s = self.lock();
        // Idle packets never get here (`relay::is_relayable`), and carry no seq.
        let seq = packet.get(3).copied();
        let lost =
            matches!((s.last_seq, seq), (Some(last), Some(seq)) if last.wrapping_add(1) != seq);
        s.last_seq = seq.or(s.last_seq);
        if !s.wanted {
            return false;
        }
        if lost {
            return s.invalidate();
        }
        match packet.get(1).copied() {
            Some(EVT_BULK_INVALIDATED) => s.invalidate(),
            Some(EVT_PARAM_CHANGED) => {
                let Some((offset, value)) = param_change(packet) else {
                    return s.invalidate();
                };
                if let Some(image) = s.image.as_mut() {
                    if !patch(image, offset, value) {
                        return s.invalidate();
                    }
                    s.updated_at = Some(now);
                    false
                } else if s.reading {
                    if s.noted.len() >= MAX_NOTED {
                        return s.invalidate();
                    }
                    s.noted.push((offset, value.to_vec()));
                    false
                } else {
                    // The last read failed: this is the moment to try again.
                    s.reading = true;
                    true
                }
            }
            _ => false,
        }
    }

    /// The worker is about to read the image. What was noted before now is in it.
    pub(crate) fn read_started(&self) {
        let mut s = self.lock();
        s.reading = true;
        s.read_started_at = s.invalidations;
        s.noted.clear();
    }

    /// Store a freshly read image. Called on the device's worker.
    pub fn set_bulk(&self, image: Vec<u8>) {
        self.set_bulk_at(image, Instant::now());
    }

    fn set_bulk_at(&self, mut image: Vec<u8>, now: Instant) {
        let mut s = self.lock();
        if s.read_started_at == s.invalidations && !image.is_empty() {
            let noted = std::mem::take(&mut s.noted);
            let fits = noted
                .iter()
                .all(|(offset, value)| patch(&mut image, *offset, value));
            if fits {
                s.image = Some(image);
                s.updated_at = Some(now);
            }
            s.reading = false;
        } else if image.is_empty() {
            s.reading = false;
        }
        // Otherwise it was invalidated during the read: another is queued, and `reading` holds.
        drop(s);
        self.changed.notify_all();
    }

    /// The read failed, or will not run. The next packet or snapshot asks again.
    pub(crate) fn read_failed(&self) {
        self.lock().reading = false;
        self.changed.notify_all();
    }

    /// The device has no notification stream to keep an image fresh with.
    pub(crate) fn distrust(&self) {
        self.lock().untrusted = true;
    }

    /// The device is going: nobody will read for a waiter.
    pub(crate) fn close(&self) {
        self.lock().closed = true;
        self.changed.notify_all();
    }

    /// The stored image, if there is one.
    pub fn bulk(&self) -> Option<Vec<u8>> {
        self.lock().image.clone()
    }

    /// What `device.snapshot` answers for this handle now, without waiting.
    pub fn get(&self, handle: u8) -> Option<Snapshot> {
        let s = self.lock();
        snapshot_of(&s, handle, Instant::now())
    }

    /// A client wants the image. From now on it is kept. True: a read is needed.
    pub(crate) fn ask(&self) -> bool {
        let mut s = self.lock();
        s.wanted = true;
        if s.closed {
            return false;
        }
        if s.untrusted {
            return s.invalidate();
        }
        if s.image.is_some() || s.reading {
            return false;
        }
        s.reading = true;
        true
    }

    /// Wait for what [`SnapshotCache::ask`] asked for, at most `timeout`.
    pub(crate) fn wait(&self, handle: u8, timeout: Duration) -> Option<Snapshot> {
        let start = Instant::now();
        let mut s = self.lock();
        loop {
            if s.image.is_some() {
                return snapshot_of(&s, handle, Instant::now());
            }
            let left = timeout.saturating_sub(start.elapsed());
            if !s.reading || s.closed || left.is_zero() {
                return None;
            }
            s = self
                .changed
                .wait_timeout(s, left)
                .map(|(g, _)| g)
                .unwrap_or_else(|e| e.into_inner().0);
        }
    }
}

fn snapshot_of(s: &State, handle: u8, now: Instant) -> Option<Snapshot> {
    let image = s.image.as_deref()?;
    Some(Snapshot {
        handle: u32::from(handle),
        // The first byte of the image's header.
        wire_version: image.first().map(|v| u32::from(*v)),
        age_ms: s.updated_at.map(|at| {
            u64::try_from(now.saturating_duration_since(at).as_millis()).unwrap_or(u64::MAX)
        }),
        bulk_b64: Some(data_encoding::BASE64.encode(image)),
        status_b64: None,
    })
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_hb3 {
    use std::sync::Arc;

    use super::*;

    const LEN: usize = 64;

    fn param(seq: u8, offset: u16, value: &[u8]) -> Vec<u8> {
        let mut p = vec![2, EVT_PARAM_CHANGED, 0, seq];
        p.extend_from_slice(&offset.to_le_bytes());
        p.extend_from_slice(&(value.len() as u16).to_le_bytes());
        p.extend_from_slice(&[nfy::PARAM_SRC_GPIO as u8, 0, 0, 0]);
        p.extend_from_slice(value);
        p
    }

    fn invalidated(seq: u8) -> Vec<u8> {
        vec![2, EVT_BULK_INVALIDATED, 0, seq, 0, 0, 0, 0]
    }

    fn warm() -> SnapshotCache {
        let c = SnapshotCache::default();
        assert!(c.ask());
        c.read_started();
        c.set_bulk(vec![32; LEN]);
        c
    }

    #[test]
    fn wp_hb3_cold_until_asked() {
        let c = SnapshotCache::default();
        assert!(!c.note_packet(&invalidated(0)));
        assert!(!c.note_packet(&param(1, 20, &[1])));
        assert_eq!(c.get(0), None);
        assert!(c.ask(), "the first ask reads");
        assert!(!c.ask(), "one is on its way");
    }

    #[test]
    fn wp_hb3_a_param_change_patches_the_image() {
        let c = warm();
        assert!(!c.note_packet(&param(0, 20, &[1, 2, 3, 4])));
        let bulk = c.bulk().unwrap();
        assert_eq!(bulk[20..24], [1, 2, 3, 4]);
        assert_eq!(bulk[19], 32);
        assert_eq!(bulk[24], 32);
        assert!(!c.ask(), "served from the cache");
    }

    #[test]
    fn wp_hb3_a_snapshot_reads_back_as_the_client_does() {
        let t = Instant::now();
        let c = SnapshotCache::default();
        c.ask();
        c.read_started();
        c.set_bulk_at(vec![32; LEN], t);
        let snap = {
            let s = c.lock();
            snapshot_of(&s, 3, t + Duration::from_millis(120)).unwrap()
        };
        assert_eq!(snap.handle, 3);
        assert_eq!(snap.wire_version, Some(32));
        assert_eq!(snap.age_ms, Some(120));
        assert_eq!(snap.bulk(), Some(vec![32; LEN]));
        assert_eq!(snap.status_b64, None);
        c.note_packet_at(&param(0, 20, &[1]), t + Duration::from_millis(100));
        let s = c.lock();
        assert_eq!(
            snapshot_of(&s, 3, t + Duration::from_millis(120))
                .unwrap()
                .age_ms,
            Some(20),
            "a patch freshens it"
        );
    }

    #[test]
    fn wp_hb3_what_cannot_be_patched_is_read_again() {
        for packet in [
            invalidated(0),
            param(0, LEN as u16, &[1]),
            param(0, LEN as u16 - 1, &[1, 2]),
            param(0, 3, &[1]),
            param(0, 20, &[]),
            param(0, 20, &[1, 2])[..13].to_vec(),
            vec![2, EVT_PARAM_CHANGED, 0, 0, 1],
        ] {
            let c = warm();
            assert!(c.note_packet(&packet), "{packet:?}");
            assert_eq!(c.get(0), None, "{packet:?}");
            assert!(!c.ask(), "already asked");
        }
    }

    #[test]
    fn wp_hb3_the_header_is_never_patched() {
        let c = warm();
        c.note_packet(&param(0, 0, &[99]));
        c.read_started();
        c.set_bulk(vec![32; LEN]);
        assert_eq!(c.get(0).unwrap().wire_version, Some(32));
    }

    #[test]
    fn wp_hb3_a_sequence_gap_means_a_missed_change() {
        let c = warm();
        assert!(!c.note_packet(&param(255, 20, &[1])));
        assert!(!c.note_packet(&param(0, 20, &[2])), "a wrap is no gap");
        assert!(c.note_packet(&param(2, 20, &[3])), "seq 1 never came");
        assert_eq!(c.get(0), None);
    }

    #[test]
    fn wp_hb3_other_events_change_nothing() {
        let c = warm();
        let name = [2, nfy::NOTIFY_EVT_DEVICE_NAME as u8, 0, 0, 0, 0, 0, 0];
        assert!(!c.note_packet(&name));
        assert!(c.get(0).is_some());
    }

    /// The read may have run before the change reached the image: the patch goes on
    /// top. Changes from before the read began are in the image, and later ones win.
    #[test]
    fn wp_hb3_a_change_during_the_read_is_not_lost() {
        let c = SnapshotCache::default();
        c.ask();
        assert!(
            !c.note_packet(&param(0, 20, &[1])),
            "before the read: in it"
        );
        c.read_started();
        assert!(!c.note_packet(&param(1, 30, &[7])));
        assert!(!c.note_packet(&param(2, 40, &[8])));
        assert!(!c.note_packet(&param(3, 40, &[9])));
        assert_eq!(c.get(0), None);
        c.set_bulk(vec![32; LEN]);
        let bulk = c.bulk().unwrap();
        assert_eq!(bulk[20], 32);
        assert_eq!(bulk[30], 7);
        assert_eq!(bulk[40], 9);
    }

    #[test]
    fn wp_hb3_too_many_changes_during_a_read_read_again() {
        let c = SnapshotCache::default();
        c.ask();
        c.read_started();
        let asked = (0..=MAX_NOTED)
            .filter(|n| c.note_packet(&param(*n as u8, 20, &[1])))
            .count();
        assert_eq!(asked, 1);
    }

    #[test]
    fn wp_hb3_an_invalidation_during_a_read_discards_that_image() {
        let c = SnapshotCache::default();
        c.ask();
        c.read_started();
        assert!(c.note_packet(&invalidated(0)), "another read");
        c.set_bulk(vec![1; LEN]);
        assert_eq!(c.get(0), None, "that image predates the change");
        c.read_started();
        c.set_bulk(vec![2; LEN]);
        assert_eq!(c.bulk(), Some(vec![2; LEN]));
    }

    #[test]
    fn wp_hb3_a_failed_read_is_tried_again_when_someone_cares() {
        let c = warm();
        assert!(c.note_packet(&invalidated(0)));
        c.read_started();
        c.read_failed();
        assert!(c.note_packet(&param(1, 20, &[1])), "the next change");
        c.read_started();
        c.read_failed();
        assert!(c.ask(), "the next client");
    }

    #[test]
    fn wp_hb3_an_empty_read_is_no_image() {
        let c = SnapshotCache::default();
        c.ask();
        c.read_started();
        c.set_bulk(Vec::new());
        assert_eq!(c.get(0), None);
        assert!(c.ask());
    }

    #[test]
    fn wp_hb3_without_a_stream_each_snapshot_reads_again() {
        let c = warm();
        c.distrust();
        assert!(c.ask());
        assert_eq!(c.get(0), None);
    }

    #[test]
    fn wp_hb3_a_waiter_gets_the_image_or_gives_up() {
        let c = Arc::new(SnapshotCache::default());
        assert_eq!(
            c.wait(0, Duration::from_secs(5)),
            None,
            "nothing on its way"
        );
        assert!(c.ask());
        assert_eq!(c.wait(0, Duration::from_millis(1)), None, "timed out");

        let worker = Arc::clone(&c);
        let t = std::thread::spawn(move || {
            worker.read_started();
            worker.set_bulk(vec![32; LEN]);
        });
        assert!(c.wait(0, Duration::from_secs(10)).is_some());
        t.join().unwrap();

        // A failed read and a device going both end the wait at once.
        let c = Arc::new(SnapshotCache::default());
        c.ask();
        let worker = Arc::clone(&c);
        let t = std::thread::spawn(move || worker.read_failed());
        assert_eq!(c.wait(0, Duration::from_secs(10)), None);
        t.join().unwrap();
        c.ask();
        c.close();
        assert_eq!(c.wait(0, Duration::from_secs(10)), None);
        assert!(!c.ask(), "nobody to read");
    }
}
