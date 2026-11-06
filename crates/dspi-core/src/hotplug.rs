//! Hotplug: arrival and departure events, and reconnecting the selected
//! device when it comes back.
//!
//! Owned by WP C1b (docs/core-merge/PLAN.md). Listeners are called with the
//! `Inner` lock released (collect the events under the lock, drop the guard,
//! then call), and must only schedule work (`callbacks.rs`).
//!
//! # One poll
//!
//! 1. A connection whose notification reader saw the end of the stream
//!    (`Notifications::is_disconnected`, any transport) is closed. An
//!    intentional close never sets that flag (`device::shut` stops the
//!    reader first), so only a lost device is closed here.
//! 2. The bus is enumerated without opening anything (`scan_quiet`; the
//!    picker's [`DspiCore::scan_devices`] test-opens every unit and is too
//!    heavy for a 500 ms poll). The serials are diffed against the previous
//!    poll's, kept in [`HotplugState`] rather than `Inner::devices`: the
//!    picker's scan filters that list to the openable units, so diffing
//!    against it would announce an inaccessible unit again after every
//!    picker scan.
//! 3. The open USB device is closed when its serial departs.
//! 4. The selected serial (`Inner::selected`, USB only) is reopened through
//!    [`DspiCore::select_device`] when it arrives and nothing is open. A
//!    failed reopen (the OS may not have applied permissions yet) is retried
//!    on the next polls, [`RECONNECT_ATTEMPTS`] tries in all. A device that
//!    is merely present is never reopened: after an explicit
//!    [`DspiCore::disconnect`] it stays closed until it is replugged.
//! 5. With the lock released: the closed connection is shut, the reopen
//!    runs, then the listener hears the departures, then the arrivals.
//!
//! A connection lost by its reader is always reported as departed, whatever
//! its transport: MIDI has no enumeration diff to fall back on, and closing
//! it here means `NOTIFY_DISCONNECTED` is never reported for it
//! (`callbacks.rs`, [`DeviceEvents`]). A lost USB connection whose serial is
//! still listed (a replug faster than one poll, or the OS has not noticed
//! yet) is also reported as arrived again, which reopens it.
//!
//! The poll enumerates USB only. It never lists MIDI ports (midir's
//! `list_ports` must not be polled faster than about once a second, and a
//! MIDI connection is not reopened by serial); a lost MIDI or Link
//! connection is still closed by step 1 and reported as departed.

use std::sync::Arc;

use crate::callbacks::DeviceEvents;
use crate::records::DeviceInfo;
use crate::rta::RtaWorker;
use crate::{Connection, DspiCore, Inner, Result, device, state};

/// How many times polls try to reopen the selected device after it arrived
/// (the first try included), one try per poll. Ten polls at the usual 500 ms
/// are 5 s, long enough for a first-time WinUSB bind.
pub(crate) const RECONNECT_ATTEMPTS: u8 = 10;

/// The hotplug poll's state, behind the `Inner` lock.
#[derive(Debug, Default)]
pub(crate) struct HotplugState {
    /// Serials the last poll saw, sorted; `None` before the first poll,
    /// which then diffs against the picker's last scan (`Inner::devices`).
    last: Option<Vec<String>>,
    /// The selected serial whose reopen has not succeeded yet, and the tries
    /// left.
    retry: Option<(String, u8)>,
}

/// A connection taken out of `Inner`, to be closed with the lock released.
struct Closing {
    conn: Connection,
    worker: Option<RtaWorker>,
}

impl Closing {
    /// Take the open connection out of `inner` and reset the mirror, as
    /// `DspiCore::close_connection` does. Call [`Closing::finish`] after the
    /// guard is dropped.
    fn take(inner: &mut Inner) -> Option<Self> {
        let conn = inner.conn.take()?;
        inner.state = state::default_state();
        let worker = crate::rta::take_worker(inner);
        Some(Self { conn, worker })
    }

    /// Stop the RTA worker and the reader, then drop the transport. Call
    /// with the lock released (PLAN §1.2).
    fn finish(self) {
        if let Some(worker) = self.worker {
            worker.stop();
        }
        device::shut(self.conn);
    }
}

/// What one poll decided under the lock.
#[derive(Debug, Default, PartialEq)]
struct Plan {
    arrived: Vec<String>,
    departed: Vec<String>,
    /// The selected serial to reopen.
    reopen: Option<String>,
}

impl DspiCore {
    /// Register (`Some`) or clear (`None`) the arrival/departure listener.
    /// Clearing waits for a call in progress on another thread. The
    /// listener is called on the thread that calls
    /// [`DspiCore::poll_hotplug`], without the core's lock held; it may call
    /// back into the core (`scan_devices`, `is_connected`), but must not set
    /// listeners from inside the callback.
    pub fn set_hotplug_callback(&self, listener: Option<Arc<dyn DeviceEvents>>) {
        self.listeners.device.set(listener);
    }

    /// Compare the USB bus with the last poll and report arrivals and
    /// departures (by serial) to the hotplug listener; reconnect the
    /// selected device when it reappears, and close the open device when it
    /// departs or its notification stream ends. Desktop apps call it about
    /// every 500 ms; it enumerates without opening anything.
    ///
    /// A reconnect runs the connect sequence on the polling thread: an
    /// embedder polling from its UI thread sees a stall of a few hundred
    /// milliseconds on a replug.
    ///
    /// Call [`DspiCore::scan_devices`] before the first poll: the first poll
    /// diffs against that scan, so without one every unit present is
    /// announced as arrived.
    ///
    /// Never fails: a failed reconnect is logged and retried on the next
    /// polls (a few times). On Android there is nothing to enumerate (the
    /// app gets USB broadcasts instead); there it only closes a connection
    /// whose stream ended, and apps need not call it.
    pub fn poll_hotplug(&self) {
        self.poll_with(
            || self.scan_quiet(),
            |serial| self.select_device(serial.to_owned()),
        );
    }

    /// [`DspiCore::poll_hotplug`] with the enumeration and the reopen
    /// supplied (tests use a fake bus).
    pub(crate) fn poll_with(
        &self,
        scan: impl FnOnce() -> Vec<DeviceInfo>,
        reopen: impl FnOnce(&str) -> Result<()>,
    ) {
        // 1. A connection whose reader saw the stream end is gone.
        let lost = {
            let mut inner = self.lock();
            let ended = inner
                .conn
                .as_ref()
                .and_then(|c| c.notifications.as_ref())
                .is_some_and(|n| n.is_disconnected());
            if ended {
                Closing::take(&mut inner)
            } else {
                None
            }
        };
        let lost_id = lost
            .as_ref()
            .map(|c| (c.conn.serial.clone(), c.conn.vid != 0));
        if let Some(lost) = lost {
            log::info!(
                "device {} lost: its notification stream ended",
                lost.conn.serial
            );
            lost.finish();
        }

        // 2. Enumerate with the lock released.
        let mut now: Vec<String> = scan().into_iter().map(|d| d.serial).collect();
        now.sort();
        now.dedup();

        // 3.-4. Diff, take out a departed device, pick a reopen.
        let (plan, departed) = {
            let mut inner = self.lock();
            let plan = plan(&mut inner, now, lost_id);
            // Still listed means step 1's event, not this device leaving: a
            // connection another thread opened meanwhile stays open.
            let listed = inner.hotplug.last.as_deref().unwrap_or_default();
            let gone = inner.conn.as_ref().is_some_and(|c| {
                c.vid != 0 && plan.departed.contains(&c.serial) && !listed.contains(&c.serial)
            });
            let departed = if gone {
                Closing::take(&mut inner)
            } else {
                None
            };
            (plan, departed)
        };

        // 5. Lock released: close, reopen, then tell the app.
        if let Some(closing) = departed {
            log::info!("device {} departed", closing.conn.serial);
            closing.finish();
        }
        if let Some(serial) = &plan.reopen {
            match reopen(serial) {
                Ok(()) => {
                    log::info!("device {serial} reconnected");
                    let moved = {
                        let mut inner = self.lock();
                        if inner
                            .hotplug
                            .retry
                            .as_ref()
                            .is_some_and(|(s, _)| s == serial)
                        {
                            inner.hotplug.retry = None;
                        }
                        // The app chose something else while the reopen ran.
                        // Only our own reopen is closed: another thread's
                        // newer open stays.
                        let ours = inner.conn.as_ref().is_some_and(|c| c.serial == *serial);
                        if ours && inner.selected.as_ref() != Some(serial) {
                            Closing::take(&mut inner)
                        } else {
                            None
                        }
                    };
                    if let Some(closing) = moved {
                        log::info!("selection moved during the reconnect of {serial}");
                        closing.finish();
                    }
                }
                Err(e) => log::warn!("reconnecting {serial} failed: {e}"),
            }
        }
        for serial in plan.departed {
            self.listeners.device.call(|l| l.on_departed(serial));
        }
        for serial in plan.arrived {
            self.listeners.device.call(|l| l.on_arrived(serial));
        }
    }
}

/// The diff and the reopen decision for one poll, under the lock: `now` is
/// the sorted, deduplicated scan, `lost` the serial of a connection step 1
/// closed and whether it was USB. Updates the baseline and the retry.
fn plan(inner: &mut Inner, now: Vec<String>, lost: Option<(String, bool)>) -> Plan {
    let before: Vec<String> = match inner.hotplug.last.take() {
        Some(last) => last,
        None => inner.devices.iter().map(|d| d.serial.clone()).collect(),
    };
    let mut arrived: Vec<String> = now
        .iter()
        .filter(|s| !before.contains(s))
        .cloned()
        .collect();
    let mut departed: Vec<String> = before
        .iter()
        .filter(|s| !now.contains(s))
        .cloned()
        .collect();
    if let Some((serial, usb)) = lost {
        if usb && now.contains(&serial) && !arrived.contains(&serial) {
            arrived.push(serial.clone());
        }
        if !departed.contains(&serial) {
            departed.push(serial);
        }
    }

    // An arrival of the selected serial (re)arms the retry; anything that
    // makes a reopen pointless (something is open, the device is absent,
    // the selection changed) disarms it.
    let wanted = match &inner.selected {
        Some(sel) if inner.conn.is_none() && now.contains(sel) => Some(sel.clone()),
        _ => None,
    };
    let reopen = wanted.and_then(|sel| {
        let left = if arrived.contains(&sel) {
            RECONNECT_ATTEMPTS
        } else {
            match &inner.hotplug.retry {
                Some((s, n)) if *s == sel => *n,
                _ => 0,
            }
        };
        let rest = left.checked_sub(1)?;
        inner.hotplug.retry = Some((sel.clone(), rest));
        Some(sel)
    });
    if reopen.is_none() {
        inner.hotplug.retry = None;
    }

    inner.hotplug.last = Some(now);
    Plan {
        arrived,
        departed,
        reopen,
    }
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_c1b {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Mutex, Weak, mpsc};
    use std::time::{Duration, Instant};

    use dspi_transport::{FakeDevice, FakeRemote};

    use super::*;
    use crate::DspiError;

    const VID: u16 = dspi_proto::USB_VID;

    /// A fake bus listing `serials`.
    fn bus(serials: &[&str]) -> impl FnOnce() -> Vec<DeviceInfo> {
        let list: Vec<DeviceInfo> = serials
            .iter()
            .map(|s| DeviceInfo {
                serial: (*s).to_owned(),
                location_id: 0,
            })
            .collect();
        move || list
    }

    /// Opens a fresh demo fake as USB device `serial`, counting the opens and
    /// keeping its remote.
    struct Opener {
        remote: Mutex<Option<FakeRemote>>,
        opens: AtomicUsize,
    }

    impl Opener {
        fn new() -> Self {
            Self {
                remote: Mutex::new(None),
                opens: AtomicUsize::new(0),
            }
        }

        fn on<'a>(&'a self, core: &'a DspiCore) -> impl FnOnce(&str) -> Result<()> + 'a {
            move |serial| {
                self.opens.fetch_add(1, Ordering::SeqCst);
                let dev = FakeDevice::demo();
                *self.remote.lock().unwrap() = Some(dev.remote());
                core.attach_probed(Box::new(dev), serial.to_owned(), VID)
            }
        }

        fn opens(&self) -> usize {
            self.opens.load(Ordering::SeqCst)
        }

        fn unplug(&self) {
            self.remote.lock().unwrap().as_ref().unwrap().disconnect();
        }
    }

    fn never(_: &str) -> Result<()> {
        panic!("no reopen expected")
    }

    fn failing(tries: &AtomicUsize) -> impl FnOnce(&str) -> Result<()> + '_ {
        move |_| {
            tries.fetch_add(1, Ordering::SeqCst);
            Err(DspiError::Disconnected)
        }
    }

    #[derive(Default)]
    struct Events(Mutex<Vec<String>>);

    impl DeviceEvents for Events {
        fn on_arrived(&self, serial: String) {
            self.0.lock().unwrap().push(format!("+{serial}"));
        }
        fn on_departed(&self, serial: String) {
            self.0.lock().unwrap().push(format!("-{serial}"));
        }
    }

    impl Events {
        fn take(&self) -> Vec<String> {
            std::mem::take(&mut *self.0.lock().unwrap())
        }
    }

    fn core_with_events() -> (Arc<DspiCore>, Arc<Events>) {
        let core = DspiCore::new();
        let events = Arc::new(Events::default());
        core.set_hotplug_callback(Some(events.clone()));
        (core, events)
    }

    fn connected_serial(core: &DspiCore) -> Option<String> {
        core.lock().conn.as_ref().map(|c| c.serial.clone())
    }

    fn wait_for_reader_end(core: &DspiCore) {
        let start = Instant::now();
        loop {
            let ended = core
                .lock()
                .conn
                .as_ref()
                .and_then(|c| c.notifications.as_ref())
                .is_some_and(|n| n.is_disconnected());
            if ended {
                return;
            }
            assert!(
                start.elapsed() < Duration::from_secs(5),
                "the reader never saw the unplug"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn wp_c1b_a_replugged_device_reconnects() {
        let (core, events) = core_with_events();
        let open = Opener::new();
        open.on(&core)("A").unwrap();
        assert_eq!(core.lock().selected.as_deref(), Some("A"));

        core.poll_with(bus(&["A"]), never);
        assert_eq!(events.take(), ["+A"], "the first poll announces the bus");
        core.poll_with(bus(&["A"]), never);
        assert!(events.take().is_empty());

        // Unplugged: the departure closes it, the selection stays.
        core.poll_with(bus(&[]), never);
        assert_eq!(events.take(), ["-A"]);
        assert!(!core.is_connected());
        assert!(!core.lock().state.bulk_valid, "the mirror is reset");
        assert_eq!(core.lock().selected.as_deref(), Some("A"));

        // Plugged back in: reopened before the app hears of it.
        core.poll_with(bus(&["A"]), open.on(&core));
        assert_eq!(events.take(), ["+A"]);
        assert_eq!(connected_serial(&core).as_deref(), Some("A"));
        assert!(core.lock().state.bulk_valid);
        assert_eq!(open.opens(), 2);
        assert!(core.lock().hotplug.retry.is_none());
    }

    #[test]
    fn wp_c1b_a_lost_stream_closes_and_reopens_a_device_still_listed() {
        let (core, events) = core_with_events();
        let open = Opener::new();
        open.on(&core)("A").unwrap();
        core.poll_with(bus(&["A"]), never);
        events.take();

        // Replugged faster than one poll: only the reader saw it.
        open.unplug();
        wait_for_reader_end(&core);
        core.poll_with(bus(&["A"]), open.on(&core));
        assert_eq!(events.take(), ["-A", "+A"]);
        assert_eq!(connected_serial(&core).as_deref(), Some("A"));
        assert_eq!(open.opens(), 2);
    }

    #[test]
    fn wp_c1b_a_lost_stream_closes_a_device_that_is_gone() {
        let (core, events) = core_with_events();
        let open = Opener::new();
        open.on(&core)("A").unwrap();
        core.poll_with(bus(&["A"]), never);
        events.take();

        open.unplug();
        wait_for_reader_end(&core);
        core.poll_with(bus(&[]), never);
        assert_eq!(events.take(), ["-A"]);
        assert!(!core.is_connected());
    }

    #[test]
    fn wp_c1b_a_lost_non_usb_connection_is_closed_and_reported_as_departed() {
        let (core, events) = core_with_events();
        let dev = FakeDevice::demo();
        let remote = dev.remote();
        core.open_transport(Box::new(dev), "MIDI".into(), 0)
            .unwrap();
        core.poll_with(bus(&[]), never);
        assert!(core.is_connected());
        remote.disconnect();
        wait_for_reader_end(&core);
        core.poll_with(bus(&[]), never);
        assert!(!core.is_connected());
        assert_eq!(events.take(), ["-MIDI"], "no arrival: not on the USB bus");
        core.poll_with(bus(&[]), never);
        assert!(events.take().is_empty(), "reported once");
    }

    #[test]
    fn wp_c1b_an_explicit_disconnect_is_not_undone() {
        let (core, events) = core_with_events();
        let open = Opener::new();
        open.on(&core)("A").unwrap();
        core.poll_with(bus(&["A"]), never);
        core.disconnect();
        // The device is still there and stays selected, but nothing
        // reopens it until it is replugged.
        core.poll_with(bus(&["A"]), never);
        core.poll_with(bus(&["A"]), never);
        assert!(!core.is_connected());
        assert_eq!(events.take(), ["+A"]);
    }

    #[test]
    fn wp_c1b_only_the_selected_device_is_reopened_or_closed() {
        let (core, events) = core_with_events();
        core.lock().selected = Some("A".into());
        core.poll_with(bus(&[]), never);
        core.poll_with(bus(&["B"]), never);
        assert_eq!(events.take(), ["+B"]);
        assert!(!core.is_connected());

        // Another unit coming and going leaves the open device alone.
        let open = Opener::new();
        open.on(&core)("A").unwrap();
        core.poll_with(bus(&["A"]), never);
        events.take();
        core.poll_with(bus(&["A", "C"]), never);
        core.poll_with(bus(&["A"]), never);
        assert_eq!(events.take(), ["+C", "-C"]);
        assert_eq!(connected_serial(&core).as_deref(), Some("A"));
    }

    #[test]
    fn wp_c1b_a_non_usb_device_is_neither_closed_nor_replaced() {
        let (core, _events) = core_with_events();
        core.lock().selected = Some("A".into());
        core.open_demo().unwrap();
        assert!(core.lock().selected.is_none(), "C1 clears it for non-USB");
        core.poll_with(bus(&[]), never);
        core.poll_with(bus(&["A"]), never);
        core.poll_with(bus(&[]), never);
        assert!(core.is_connected());
    }

    #[test]
    fn wp_c1b_a_failed_reopen_is_retried_a_few_times() {
        let (core, _events) = core_with_events();
        core.lock().selected = Some("A".into());
        core.poll_with(bus(&[]), never);
        let tries = AtomicUsize::new(0);
        for _ in 0..RECONNECT_ATTEMPTS + 2 {
            core.poll_with(bus(&["A"]), failing(&tries));
        }
        assert_eq!(
            tries.load(Ordering::SeqCst),
            usize::from(RECONNECT_ATTEMPTS)
        );
        assert!(core.lock().hotplug.retry.is_none());

        // A retry that succeeds ends the retries.
        core.poll_with(bus(&[]), never);
        core.poll_with(bus(&["A"]), failing(&tries));
        let open = Opener::new();
        core.poll_with(bus(&["A"]), open.on(&core));
        assert_eq!(connected_serial(&core).as_deref(), Some("A"));
        assert!(core.lock().hotplug.retry.is_none());
        core.poll_with(bus(&["A"]), never);
    }

    #[test]
    fn wp_c1b_a_lost_midi_serial_listed_on_usb_does_not_reopen() {
        let (core, events) = core_with_events();
        core.poll_with(bus(&["A"]), never);
        events.take();
        let dev = FakeDevice::demo();
        let remote = dev.remote();
        core.open_transport(Box::new(dev), "A".into(), 0).unwrap();
        remote.disconnect();
        wait_for_reader_end(&core);
        core.poll_with(bus(&["A"]), never);
        assert_eq!(events.take(), ["-A"]);
        assert!(!core.is_connected());
    }

    #[test]
    fn wp_c1b_a_selection_moved_during_the_reopen_closes_it() {
        let (core, _events) = core_with_events();
        core.lock().selected = Some("A".into());
        core.poll_with(bus(&[]), never);
        let open = Opener::new();
        core.poll_with(bus(&["A"]), |serial| {
            open.on(&core)(serial)?;
            core.lock().selected = Some("B".into());
            Ok(())
        });
        assert!(!core.is_connected());
        assert_eq!(core.lock().selected.as_deref(), Some("B"));
    }

    #[test]
    fn wp_c1b_a_newer_open_during_the_reopen_stays() {
        let (core, _events) = core_with_events();
        core.lock().selected = Some("A".into());
        core.poll_with(bus(&[]), never);
        // Another thread opens the demo while the reopen runs: the selection
        // is cleared, but what is open is not the reopened device.
        core.poll_with(bus(&["A"]), |_| core.open_demo());
        assert!(core.is_connected());
    }

    #[test]
    fn wp_c1b_a_departure_disarms_the_retry() {
        let (core, _events) = core_with_events();
        core.lock().selected = Some("A".into());
        core.poll_with(bus(&[]), never);
        let tries = AtomicUsize::new(0);
        core.poll_with(bus(&["A"]), failing(&tries));
        assert!(core.lock().hotplug.retry.is_some());
        core.poll_with(bus(&[]), never);
        assert!(core.lock().hotplug.retry.is_none());
        core.poll_with(bus(&[]), never);
    }

    #[test]
    fn wp_c1b_the_first_poll_diffs_against_the_picker_scan() {
        let (core, events) = core_with_events();
        core.lock().devices = vec![DeviceInfo {
            serial: "A".into(),
            location_id: 0,
        }];
        core.poll_with(bus(&["A", "B"]), never);
        assert_eq!(events.take(), ["+B"]);
        // A later picker scan that leaves out an inaccessible unit does not
        // make it arrive again.
        core.lock().devices.clear();
        core.poll_with(bus(&["A", "B"]), never);
        assert!(events.take().is_empty());
    }

    #[test]
    fn wp_c1b_a_cleared_listener_hears_nothing() {
        let (core, events) = core_with_events();
        core.set_hotplug_callback(None);
        core.poll_with(bus(&["A"]), never);
        assert!(events.take().is_empty());
    }

    /// Calls back into the core from both callbacks.
    struct Reentrant {
        core: Weak<DspiCore>,
        calls: AtomicUsize,
    }

    impl Reentrant {
        fn call_back(&self) {
            if let Some(core) = self.core.upgrade() {
                let _ = core.scan_devices();
                let _ = core.is_connected();
                let _ = core.preamp_max_db();
            }
            self.calls.fetch_add(1, Ordering::SeqCst);
        }
    }

    impl DeviceEvents for Reentrant {
        fn on_arrived(&self, _: String) {
            self.call_back();
        }
        fn on_departed(&self, _: String) {
            self.call_back();
        }
    }

    #[test]
    fn wp_c1b_a_listener_calling_scan_devices_does_not_deadlock() {
        let core = DspiCore::new();
        let listener = Arc::new(Reentrant {
            core: Arc::downgrade(&core),
            calls: AtomicUsize::new(0),
        });
        core.set_hotplug_callback(Some(listener.clone()));
        let (tx, rx) = mpsc::channel();
        let c = Arc::clone(&core);
        std::thread::spawn(move || {
            let open = Opener::new();
            open.on(&c)("A").unwrap();
            c.poll_with(bus(&["A"]), never);
            c.poll_with(bus(&[]), never);
            c.poll_with(bus(&["A"]), open.on(&c));
            tx.send(()).unwrap();
        });
        rx.recv_timeout(Duration::from_secs(10))
            .expect("a re-entrant hotplug listener deadlocked the poll");
        assert_eq!(listener.calls.load(Ordering::SeqCst), 3);
        assert!(core.is_connected());
    }

    #[test]
    fn wp_c1b_poll_hotplug_without_usb_is_quiet() {
        let (core, events) = core_with_events();
        core.poll_hotplug();
        core.poll_hotplug();
        assert!(events.take().is_empty());
        assert!(!core.is_connected());
    }
}
