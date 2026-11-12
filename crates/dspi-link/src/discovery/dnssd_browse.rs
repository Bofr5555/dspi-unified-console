//! -- WP DISC2 -- The part of the Windows DNS-SD source that is not dnsapi: what a browse
//! knows of each instance, one resolving thread per announced hub, and the order in which
//! goodbyes and records reach the browser. Behind [`Resolver`], so it builds and is tested
//! on every platform; only `windows.rs` uses it in a product build.

// Off Windows only the tests below use it.
#![cfg_attr(not(windows), allow(dead_code))]

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::Duration;

use super::{address_rank, ServiceRecord, SourceEvent, SourceSink};
use crate::clock::Instant;

/// Addresses kept per hub.
const MAX_KEPT: usize = 8;

/// What a resolving thread hears on its channel.
pub(super) enum Answer {
    /// The callback of resolve number `.0`: the record and the SRV target's host name.
    Done(u64, Option<(ServiceRecord, String)>),
    /// The browse closed.
    Abort,
}

/// One resolve against the system's DNS-SD service.
pub(super) trait Resolver: Send + Sync + 'static {
    /// Resolve `instance` and wait for the answer: the record and the SRV target's host
    /// name. An [`Answer::Abort`] on `rx` cuts it short. A resolve that gives up may leave
    /// its own late [`Answer::Done`] on `rx` (dnsapi's cancelled callback); whoever reads
    /// `rx` next ignores it.
    fn resolve(
        &self,
        instance: &str,
        tx: &Sender<Answer>,
        rx: &Receiver<Answer>,
    ) -> Result<(ServiceRecord, String), String>;

    /// `host`'s addresses as the system resolver has them now.
    fn look_up(&self, host: &str, port: u16) -> Vec<IpAddr>;
}

/// The waits of a browse; tests shorten them.
#[derive(Debug, Clone, Copy)]
pub(super) struct Timing {
    /// Resolves of one announcement that gave nothing to dial.
    pub(super) resolve_tries: u32,
    /// The wait before resolving again a hub that gave only its host name: past
    /// mdns-sd's one-second limit on sending its A records again.
    pub(super) resolve_again: Duration,
    /// The least time between two resolves of one hub. Avahi and Windows put the PTR
    /// record in every answer, and each PTR heard asks for a resolve, so without this a
    /// hub would be resolved about once a second for as long as it is browsed. One heard
    /// meanwhile is resolved when the time is up, so a TXT change shows within this
    /// (upstream drops it).
    pub(super) reresolve_gap: Duration,
}

impl Default for Timing {
    fn default() -> Self {
        Self { resolve_tries: 4, resolve_again: Duration::from_millis(1100), reresolve_gap: Duration::from_secs(5) }
    }
}

pub(super) fn dialable(addresses: &[IpAddr]) -> Vec<IpAddr> {
    addresses.iter().filter(|ip| address_rank(ip).is_some()).copied().collect()
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Wait up to `d` on `rx`. True when the browse closed. A late [`Answer::Done`] of an
/// earlier, cancelled resolve is no reason to stop waiting: one used to end the wait
/// and leave the instance marked as resolving for good (review d2 F1).
fn aborted(rx: &Receiver<Answer>, d: Duration) -> bool {
    let deadline = Instant::now().checked_add(d);
    loop {
        let left = deadline.map_or(Duration::ZERO, |at| at.saturating_duration_since(Instant::now()));
        match rx.recv_timeout(left) {
            Ok(Answer::Done(..)) => {}
            Ok(Answer::Abort) | Err(RecvTimeoutError::Disconnected) => return true,
            Err(RecvTimeoutError::Timeout) => return false,
        }
    }
}

/// What a browse knows of one instance.
#[derive(Default)]
struct Instance {
    /// Goodbyes heard: a resolve started before one must not bring the hub back.
    epoch: u64,
    announced: bool,
    /// A resolving thread of this epoch is running.
    resolving: bool,
    /// Announced again while it ran: the data may have changed since it asked.
    again: bool,
    /// The last addresses to dial, for a later resolve that gives none and no host name.
    kept: Vec<IpAddr>,
}

/// One browse's instances and resolving threads.
pub(super) struct Browse<R> {
    sink: SourceSink,
    closed: AtomicBool,
    /// By lower-case instance name.
    instances: Mutex<HashMap<String, Instance>>,
    /// The resolving threads running, to stop when the browse closes.
    aborts: Mutex<HashMap<u64, Sender<Answer>>>,
    next_resolve: AtomicU64,
    resolver: R,
    timing: Timing,
}

impl<R: Resolver> Browse<R> {
    pub(super) fn new(sink: SourceSink, resolver: R, timing: Timing) -> Arc<Self> {
        Arc::new(Self {
            sink,
            closed: AtomicBool::new(false),
            instances: Mutex::default(),
            aborts: Mutex::default(),
            next_resolve: AtomicU64::new(0),
            resolver,
            timing,
        })
    }

    pub(super) fn is_closed(&self) -> bool {
        self.closed.load(Ordering::SeqCst)
    }

    /// The source stopped working.
    pub(super) fn failed(&self, why: String) {
        if !self.is_closed() {
            self.sink.send(SourceEvent::Failed(why));
        }
    }

    /// Stop every resolving thread; nothing is reported afterwards.
    pub(super) fn close(&self) {
        let mut aborts = lock(&self.aborts);
        self.closed.store(true, Ordering::SeqCst);
        for (_, abort) in aborts.drain() {
            let _ = abort.send(Answer::Abort);
        }
    }

    /// A channel for one resolving thread, which [`Self::close`] can stop.
    fn channel(&self) -> Option<(u64, Sender<Answer>, Receiver<Answer>)> {
        let id = self.next_resolve.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = mpsc::channel();
        let mut aborts = lock(&self.aborts);
        // Checked under the lock `close` drains, so none is missed.
        if self.is_closed() {
            return None;
        }
        aborts.insert(id, tx.clone());
        Some((id, tx, rx))
    }

    fn done(&self, id: u64) {
        lock(&self.aborts).remove(&id);
    }

    /// One PTR record: a goodbye, or a hub to resolve.
    pub(super) fn heard(self: &Arc<Self>, instance: String, goodbye: bool) {
        if self.is_closed() {
            return;
        }
        let key = instance.to_ascii_lowercase();
        let epoch = {
            let mut instances = lock(&self.instances);
            let entry = instances.entry(key.clone()).or_default();
            if goodbye {
                entry.epoch = entry.epoch.wrapping_add(1);
                entry.announced = false;
                entry.resolving = false;
                entry.again = false;
                entry.kept.clear();
                // Under the lock `report` sends under, so a record resolved after this
                // goodbye cannot reach the browser before it (review d2 F3).
                self.sink.send(SourceEvent::Removed(instance));
                return;
            }
            if entry.resolving {
                entry.again = true;
                return;
            }
            entry.announced = true;
            entry.resolving = true;
            entry.epoch
        };
        let Some((id, tx, rx)) = self.channel() else {
            return;
        };
        let owned = Arc::clone(self);
        let thread_key = key.clone();
        let spawned = thread::Builder::new().name("dspi-dnssd-resolve".into()).spawn(move || {
            // Only a closing browse ends the wait between two resolves.
            while owned.resolve_hub(&thread_key, epoch, &instance, &tx, &rx)
                && !aborted(&rx, owned.timing.reresolve_gap)
            {}
            owned.done(id);
        });
        if let Err(e) = spawned {
            log::warn!("Windows DNS-SD: no thread to resolve with: {e}");
            self.done(id);
            if let Some(entry) = lock(&self.instances).get_mut(&key) {
                entry.resolving = false;
            }
        }
    }

    /// Resolve it again; no answer is a removal. Windows may still answer from its cache
    /// for a while, so a hub that vanished can take until its record's TTL to leave.
    pub(super) fn verify(self: &Arc<Self>, instance: &str) {
        let Some((id, tx, rx)) = self.channel() else {
            return;
        };
        let owned = Arc::clone(self);
        let instance = instance.to_owned();
        let spawned = thread::Builder::new().name("dspi-dnssd-verify".into()).spawn(move || {
            let gone = owned.resolver.resolve(&instance, &tx, &rx).is_err();
            if gone && !owned.is_closed() {
                owned.sink.send(SourceEvent::Removed(instance));
            }
            owned.done(id);
        });
        if let Err(e) = spawned {
            log::debug!("Windows DNS-SD verify: {e}");
            self.done(id);
        }
    }

    /// Resolve an announced hub until there is an address to dial, and report it unless
    /// a goodbye came meanwhile. True when it was announced again meanwhile, to be
    /// resolved once more.
    ///
    /// A resolve that gives only an IPv6 link-local address (measured against mdns-sd on
    /// Linux: it answers Windows' follow-up A query only after its one-second rate limit,
    /// and Windows does not use the A records it sent with the SRV) is reported at once
    /// with the host name to dial, as upstream's app and the Mac dial a hub, and resolved
    /// again a second later for an address.
    fn resolve_hub(&self, key: &str, epoch: u64, instance: &str, tx: &Sender<Answer>, rx: &Receiver<Answer>) -> bool {
        let mut by_name = false;
        for _ in 0..self.timing.resolve_tries {
            if by_name && aborted(rx, self.timing.resolve_again) {
                break;
            }
            if !self.current(key, epoch) {
                break;
            }
            // No answer in time: at once again.
            let Ok((mut record, host)) = self.resolver.resolve(instance, tx, rx) else {
                continue;
            };
            let mut addresses = dialable(&record.addresses);
            if addresses.is_empty() && !host.is_empty() {
                addresses = self.resolver.look_up(&host, record.port);
            }
            // Addresses an earlier resolve gave serve only a record without a host name:
            // the name is current, and they may be from a lease the hub has given up
            // (review d2 F4).
            if addresses.is_empty() && host.is_empty() {
                addresses = lock(&self.instances).get(key).map(|e| e.kept.clone()).unwrap_or_default();
            }
            let dialled_by_address = !addresses.is_empty();
            if !dialled_by_address {
                log::debug!("Windows DNS-SD: {instance} resolved to {:?} only; listed as {host}", record.addresses);
                if by_name {
                    continue;
                }
                by_name = true;
            }
            addresses.truncate(MAX_KEPT);
            record.addresses = addresses;
            host.trim_end_matches('.').clone_into(&mut record.host_name);
            if !self.report(key, epoch, instance, record) || dialled_by_address {
                break;
            }
        }
        let mut instances = lock(&self.instances);
        let Some(entry) = instances.get_mut(key).filter(|e| e.epoch == epoch) else {
            return false;
        };
        let again = std::mem::take(&mut entry.again) && !self.is_closed();
        entry.resolving = again;
        again
    }

    /// Still announced, as of this epoch, and the browse open.
    fn current(&self, key: &str, epoch: u64) -> bool {
        lock(&self.instances).get(key).is_some_and(|e| e.epoch == epoch && e.announced) && !self.is_closed()
    }

    /// Hand a resolved record to the browser, unless a goodbye came meanwhile. False then.
    fn report(&self, key: &str, epoch: u64, instance: &str, mut record: ServiceRecord) -> bool {
        let mut instances = lock(&self.instances);
        let Some(entry) = instances.get_mut(key).filter(|e| e.epoch == epoch && e.announced) else {
            return false;
        };
        if self.is_closed() {
            return false;
        }
        if !record.addresses.is_empty() {
            record.addresses.clone_into(&mut entry.kept);
        }
        // The name the browse heard, which a goodbye will name too.
        instance.clone_into(&mut record.instance);
        // Under the lock, so a goodbye cannot pass it.
        self.sink.send(SourceEvent::Resolved(record));
        true
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::arithmetic_side_effects)]
mod tests_wp_disc2 {
    use std::sync::atomic::AtomicU32;

    use super::super::{BrowseOptions, BrowseSession, BrowseSource, DiscoveryError, HubAdvert, HubBrowser};
    use super::*;

    const HID: &str = "3f2a7c1e-0000-4000-8000-0000000000f1";
    const INSTANCE: &str = "Hub._dspi._tcp.local.";

    /// A resolver that times out as dnsapi does, its cancelled callback's late `Done`
    /// left on the channel, for the first `timeouts` calls, and answers after that.
    struct Script {
        timeouts: Mutex<u32>,
        calls: AtomicU32,
        attempts: AtomicU64,
        /// What an answer gives: addresses, and the host name.
        answer: Mutex<(Vec<IpAddr>, String)>,
        looked_up: Mutex<Vec<IpAddr>>,
    }

    impl Script {
        fn new(timeouts: u32, addresses: &[&str], host: &str) -> Arc<Self> {
            Arc::new(Self {
                timeouts: Mutex::new(timeouts),
                calls: AtomicU32::new(0),
                attempts: AtomicU64::new(0),
                answer: Mutex::new((addresses.iter().map(|a| a.parse().unwrap()).collect(), host.to_owned())),
                looked_up: Mutex::default(),
            })
        }
    }

    impl Resolver for Arc<Script> {
        fn resolve(
            &self,
            _: &str,
            tx: &Sender<Answer>,
            rx: &Receiver<Answer>,
        ) -> Result<(ServiceRecord, String), String> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let attempt = self.attempts.fetch_add(1, Ordering::SeqCst);
            let timed_out = {
                let mut left = lock(&self.timeouts);
                let some = *left > 0;
                *left = left.saturating_sub(1);
                some
            };
            if !timed_out {
                let (addresses, host) = lock(&self.answer).clone();
                let advert = HubAdvert {
                    name: "Hub".into(),
                    port: 11915,
                    hub_id: HID.into(),
                    kind: "console".into(),
                    auth: "none".into(),
                    ..HubAdvert::default()
                };
                let record = ServiceRecord {
                    instance: INSTANCE.into(),
                    port: 11915,
                    addresses,
                    txt: advert.txt(),
                    ..ServiceRecord::default()
                };
                return Ok((record, host));
            }
            // As `windows::resolve`: wait, skipping late answers of earlier attempts...
            let end = Instant::now() + Duration::from_millis(30);
            while let Ok(Answer::Done(..)) = rx.recv_timeout(end.saturating_duration_since(Instant::now())) {}
            // ... and cancel: the cancelled callback answers before the cancel returns,
            // to a resolve that has stopped reading.
            let _ = tx.send(Answer::Done(attempt, None));
            Err("no answer in time".into())
        }

        fn look_up(&self, _: &str, _: u16) -> Vec<IpAddr> {
            lock(&self.looked_up).clone()
        }
    }

    /// A source over [`Browse`] whose PTR records the test sends.
    struct Source {
        script: Arc<Script>,
        browse: Mutex<Option<Arc<Browse<Arc<Script>>>>>,
    }

    struct Session(Arc<Browse<Arc<Script>>>);

    impl BrowseSession for Session {
        fn verify(&self, instance: &str) {
            self.0.verify(instance);
        }
    }

    impl Drop for Session {
        fn drop(&mut self) {
            self.0.close();
        }
    }

    impl BrowseSource for Source {
        fn open(&self, sink: SourceSink) -> Result<Box<dyn BrowseSession>, DiscoveryError> {
            let timing = Timing {
                resolve_tries: 4,
                resolve_again: Duration::from_millis(50),
                reresolve_gap: Duration::from_millis(100),
            };
            let browse = Browse::new(sink, Arc::clone(&self.script), timing);
            *lock(&self.browse) = Some(Arc::clone(&browse));
            Ok(Box::new(Session(browse)))
        }
    }

    fn start(script: &Arc<Script>) -> (HubBrowser, Arc<Source>) {
        let source = Arc::new(Source { script: Arc::clone(script), browse: Mutex::default() });
        let options = BrowseOptions { removal_grace: Duration::ZERO, ..BrowseOptions::default() };
        let browser = HubBrowser::start(Arc::clone(&source) as Arc<dyn BrowseSource>, None, options).unwrap();
        let end = Instant::now() + Duration::from_secs(5);
        while lock(&source.browse).is_none() {
            assert!(Instant::now() < end, "never opened");
            thread::sleep(Duration::from_millis(2));
        }
        (browser, source)
    }

    fn browse(source: &Source) -> Arc<Browse<Arc<Script>>> {
        Arc::clone(lock(&source.browse).as_ref().unwrap())
    }

    /// Review d2 F1: four timeouts in a row, while the PTR keeps coming. The last
    /// cancelled callback's `Done` used to end the wait before the next resolve, and the
    /// instance stayed "resolving" for good: never listed again.
    #[test]
    fn wp_disc2_four_resolve_timeouts_in_a_row_do_not_lose_the_hub() {
        let script = Script::new(4, &["10.0.0.5"], "hub.local");
        let (browser, source) = start(&script);
        let b = browse(&source);
        let end = Instant::now() + Duration::from_secs(5);
        while browser.resolve(HID).is_none() {
            assert!(Instant::now() < end, "never listed after {} resolves", script.calls.load(Ordering::SeqCst));
            // The PTR still arriving: Avahi and Windows put it in every answer.
            b.heard(INSTANCE.into(), false);
            thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(browser.resolve(HID).unwrap().host, "10.0.0.5");
        assert_eq!(script.calls.load(Ordering::SeqCst), 5, "four timeouts, then the answer");
    }

    /// The same stale answer in the wait before resolving a hub listed by name again.
    #[test]
    fn wp_disc2_a_stale_answer_does_not_cut_the_wait_for_an_address_short() {
        let script = Script::new(1, &["fe80::1"], "hub.local");
        let (browser, source) = start(&script);
        let b = browse(&source);
        b.heard(INSTANCE.into(), false);
        let end = Instant::now() + Duration::from_secs(5);
        while browser.resolve(HID).is_none() {
            assert!(Instant::now() < end, "never listed by name");
            thread::sleep(Duration::from_millis(2));
        }
        assert_eq!(browser.resolve(HID).unwrap().host, "hub.local");
        // Then the hub's A record arrives in the cache: the next resolve finds it.
        lock(&script.looked_up).push("10.0.0.5".parse().unwrap());
        while browser.resolve(HID).unwrap().host != "10.0.0.5" {
            assert!(Instant::now() < end, "the address never replaced the name");
            thread::sleep(Duration::from_millis(2));
        }
    }

    /// Review d2 F4: a fresh host name beats an address kept from an earlier lease.
    #[test]
    fn wp_disc2_a_fresh_name_beats_a_kept_address() {
        let script = Script::new(0, &["10.0.0.5"], "hub.local");
        let (browser, source) = start(&script);
        let b = browse(&source);
        b.heard(INSTANCE.into(), false);
        let end = Instant::now() + Duration::from_secs(5);
        while browser.resolve(HID).is_none() {
            assert!(Instant::now() < end, "never listed");
            thread::sleep(Duration::from_millis(2));
        }
        // The hub moved; this resolve gives only its link-local address.
        *lock(&script.answer) = (vec!["fe80::1".parse().unwrap()], "hub.local".into());
        while browser.resolve(HID).unwrap().host != "hub.local" {
            assert!(Instant::now() < end, "still dialling the old lease");
            b.heard(INSTANCE.into(), false);
            thread::sleep(Duration::from_millis(10));
        }
    }

    /// Review d2 F3: a goodbye and a quick re-announcement arrive in that order.
    #[test]
    fn wp_disc2_a_goodbye_is_ordered_before_the_record_that_follows_it() {
        let (tx, rx) = mpsc::channel();
        let sink = SourceSink { tx, generation: 1 };
        let script = Script::new(0, &["10.0.0.5"], "hub.local");
        let timing = Timing {
            resolve_tries: 4,
            resolve_again: Duration::from_millis(50),
            reresolve_gap: Duration::from_millis(100),
        };
        let b = Browse::new(sink, script, timing);
        for _ in 0..50 {
            b.heard(INSTANCE.into(), false);
            b.heard(INSTANCE.into(), true);
            b.heard(INSTANCE.into(), false);
        }
        thread::sleep(Duration::from_millis(200));
        b.close();
        // After the last goodbye, the last event is a record of the last announcement.
        let events: Vec<SourceEvent> = rx
            .try_iter()
            .filter_map(|e| match e {
                super::super::Event::Source(_, e) => Some(e),
                _ => None,
            })
            .collect();
        assert!(matches!(events.last(), Some(SourceEvent::Resolved(_))), "{:?}", events.last());
    }
}
