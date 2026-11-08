//! Finding hubs on the local network: DNS-SD for `_dspi._tcp` (spec 3.1).
//!
//! Owned by WP CG2 (docs/core-merge/PLAN-CLIENTS.md). Reference: macOS
//! `Link/HubBrowser.swift` and `LinkDiscovery.swift`, Android `NsdHubBrowser.kt`
//! and `HubTxt`.
//!
//! Layers:
//! - [`parse_service`] and [`HubAdvert::txt`]: the TXT record, both ways. No
//!   network. A TXT record is whatever anyone on the LAN sends, so everything
//!   is bounded and control characters never get through.
//! - [`BrowseSource`]: where services come from. The real one is [`MdnsSource`]
//!   (feature `mdns`); the fake one is [`FakeAdvertiser`] (feature `fake-hub`),
//!   so nothing above this line needs a network.
//! - [`HubBrowser`]: start, stop, the results listener, and resolving a hub id
//!   to its current address. A hub that took a new DHCP lease announces its
//!   `hid` from a new address; [`HubBrowser::resolve`] gives that address to
//!   whoever is reconnecting to it.
//! - [`MdnsAdvertiser`] (feature `mdns`): the responder half, for the hub.
//!
//! The mobile libraries leave feature `mdns` off: Android browses with
//! `NsdManager` and iOS with `NWBrowser`, which own the permissions and the
//! entitlement those systems ask for.
//!
//! # Threads
//!
//! One thread owns a browser's work: it opens the source, applies its events
//! and calls the listener. Opening never happens on the caller's thread, and a
//! source that fails is opened again with backoff, so a failure is a state
//! ([`BrowseSnapshot::error`]) rather than an end. [`HubBrowser::stop`] waits
//! for that thread, except when called from the listener, which it runs on:
//! after `stop` returns, no listener call is in progress or still to come.

#![warn(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::arithmetic_side_effects)]

use std::collections::BTreeMap;
use std::net::IpAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use crate::clock::Instant;

use crate::{address, sanitize_hub_text, DEFAULT_PATH};

/// The DNS-SD service type and domain of a hub (spec 3.1).
pub const SERVICE_TYPE: &str = "_dspi._tcp.local.";
/// Longest `d` list the spec allows in a TXT record: past this, it is left out.
const TXT_LIMIT: usize = 400;

/// Bounds on what a TXT record can make us keep. Its packet is at most a few
/// kilobytes, but nothing here relies on that.
const MAX_NAME_BYTES: usize = 255;
const MAX_HUB_ID_BYTES: usize = 128;
const MAX_PATH_BYTES: usize = 128;
const MAX_SERIAL_BYTES: usize = 64;
const MAX_SERIALS: usize = 64;
const MAX_ADDRESSES: usize = 16;
/// Hubs kept at once; a LAN holds a handful, and anyone can announce more.
const MAX_HUBS: usize = 256;

/// A hub found on the network. The JSON form (camelCase) is what the C ABI hands out.
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowsedHub {
    /// The `hid`: lowercase, stable for the life of the hub's installation.
    pub id: String,
    /// The instance name, safe to show.
    pub name: String,
    /// `console`, `bridge` or `unknown`.
    pub kind: String,
    /// `none` or `pin`. Anything else counts as `pin`.
    pub auth: String,
    /// Devices it shares.
    pub device_count: u32,
    /// Serials of those devices, when the hub lists them.
    pub serials: Vec<String>,
    /// The address to dial: the first of `addresses`.
    pub host: String,
    /// TCP port.
    pub port: u16,
    /// WebSocket path.
    pub path: String,
    /// `ws://host:port/path`, for `link_connect`.
    pub url: String,
    /// Every usable address, best first: IPv4, then routable IPv6, then IPv4
    /// link-local. IPv6 link-local is left out: it needs a zone, which the
    /// WebSocket URL cannot carry (as on Android).
    pub addresses: Vec<String>,
}

/// One service instance as a [`BrowseSource`] resolved it, not yet read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceRecord {
    /// Full instance name, `Studio Mac._dspi._tcp.local.`, as the source spells it.
    pub instance: String,
    /// The service's port.
    pub port: u16,
    /// Its addresses, in any order.
    pub addresses: Vec<IpAddr>,
    /// TXT keys and values, in record order.
    pub txt: Vec<(String, String)>,
}

/// Why a service is not listed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Rejected {
    /// `v` is absent or below 1.
    #[error("no usable protocol version")]
    Version,
    /// `hid` is absent, empty, too long or not printable.
    #[error("no usable hub id")]
    HubId,
    /// `tls=1`: no hub offers TLS and this client does not speak it.
    #[error("TLS is not supported")]
    Tls,
    /// Port 0.
    #[error("no port")]
    Port,
    /// Nothing in the address list can be dialled.
    #[error("no usable address")]
    Address,
}

/// A failure to browse or advertise.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DiscoveryError {
    /// The system or the mDNS daemon refused.
    #[error("could not use the local network: {0}")]
    Network(String),
    /// A thread could not be started.
    #[error("could not start a thread: {0}")]
    Thread(String),
    /// The advertisement cannot be expressed in DNS-SD.
    #[error("invalid advertisement: {0}")]
    Advert(String),
}

/// First match of a TXT key. Keys ignore case, and the first of a repeated key
/// wins (RFC 6763 section 6.4).
fn txt_value<'a>(txt: &'a [(String, String)], key: &str) -> Option<&'a str> {
    txt.iter().find(|(k, _)| k.eq_ignore_ascii_case(key)).map(|(_, v)| v.trim())
}

/// At most `max` bytes of `s`, cut at a character boundary.
fn bounded(s: &str, max: usize) -> String {
    let cut = (0..=max.min(s.len())).rev().find(|&i| s.is_char_boundary(i)).unwrap_or(0);
    s.get(..cut).unwrap_or("").to_owned()
}

/// The user-visible name inside a full instance name: without the service type,
/// and with DNS-SD's `\.`, `\\` and `\DDD` undone (Android's `unescapeName`).
fn instance_label(instance: &str) -> String {
    let lower = instance.to_ascii_lowercase();
    let label = lower
        .strip_suffix(SERVICE_TYPE)
        .and_then(|l| instance.get(..l.len()))
        .map_or(instance, |l| l.strip_suffix('.').unwrap_or(l));
    let bytes = label.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut rest = bytes;
    while let Some((&b, tail)) = rest.split_first() {
        if b != b'\\' {
            out.push(b);
            rest = tail;
            continue;
        }
        match tail {
            [a, b, c, after @ ..] if a.is_ascii_digit() && b.is_ascii_digit() && c.is_ascii_digit() => {
                let code = [a, b, c]
                    .iter()
                    .fold(0u32, |n, d| n.saturating_mul(10).saturating_add(u32::from(d.saturating_sub(b'0'))));
                out.push(u8::try_from(code).unwrap_or(b'?'));
                rest = after;
            }
            [c, after @ ..] => {
                out.push(*c);
                rest = after;
            }
            [] => {
                out.push(b);
                rest = tail;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Sort key of an address: lower is better. `None` cannot be dialled.
fn address_rank(ip: &IpAddr) -> Option<u8> {
    match ip {
        IpAddr::V4(a) if a.is_unspecified() || a.is_multicast() || a.is_broadcast() => None,
        IpAddr::V4(a) if a.is_link_local() => Some(2),
        IpAddr::V4(_) => Some(0),
        IpAddr::V6(a) if a.is_unspecified() || a.is_multicast() => None,
        // fe80::/10: useless without a zone.
        IpAddr::V6(a) if a.segments().first().is_some_and(|s| s & 0xffc0 == 0xfe80) => None,
        IpAddr::V6(_) => Some(1),
    }
}

/// A `path` that is safe inside `ws://host:port/...`.
fn usable_path(path: &str) -> bool {
    path.starts_with('/')
        && path.len() <= MAX_PATH_BYTES
        && path.bytes().all(|b| b.is_ascii_alphanumeric() || b"/-._~".contains(&b))
}

/// Read one service. The rules are Android's `HubTxt.parse` and the spec's table:
/// required keys, unknown keys ignored, and a `kind`, `auth`, `n` or `path` that
/// makes no sense falls back rather than losing the hub. `auth` falls back to
/// `pin`, the mode that asks.
pub fn parse_service(record: &ServiceRecord) -> Result<BrowsedHub, Rejected> {
    let txt = &record.txt;
    if !txt_value(txt, "v").and_then(|v| v.parse::<u32>().ok()).is_some_and(|v| v >= 1) {
        return Err(Rejected::Version);
    }
    let id = txt_value(txt, "hid").unwrap_or("").to_lowercase();
    if id.is_empty() || id.len() > MAX_HUB_ID_BYTES || !id.bytes().all(|b| b.is_ascii_graphic()) {
        return Err(Rejected::HubId);
    }
    // Only as the spec writes it, as on both reference clients: `tls=0` is plaintext.
    if txt_value(txt, "tls") == Some("1") {
        return Err(Rejected::Tls);
    }
    if record.port == 0 {
        return Err(Rejected::Port);
    }
    let mut ranked: Vec<(u8, IpAddr)> =
        record.addresses.iter().filter_map(|ip| address_rank(ip).map(|rank| (rank, *ip))).collect();
    ranked.sort_unstable();
    ranked.dedup();
    ranked.truncate(MAX_ADDRESSES);
    let addresses: Vec<String> = ranked.iter().map(|(_, ip)| ip.to_string()).collect();
    let host = addresses.first().cloned().ok_or(Rejected::Address)?;

    let serials: Vec<String> = txt_value(txt, "d")
        .unwrap_or("")
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty() && s.len() <= MAX_SERIAL_BYTES)
        .map(sanitize_hub_text)
        .take(MAX_SERIALS)
        .collect();
    let kind = txt_value(txt, "kind").filter(|k| matches!(*k, "console" | "bridge")).unwrap_or("unknown");
    let auth = if txt_value(txt, "auth") == Some("none") { "none" } else { "pin" };
    let device_count = txt_value(txt, "n")
        .and_then(|n| n.parse().ok())
        .unwrap_or_else(|| u32::try_from(serials.len()).unwrap_or(u32::MAX));
    let path = txt_value(txt, "path").filter(|p| usable_path(p)).unwrap_or(DEFAULT_PATH).to_owned();
    let label = bounded(&sanitize_hub_text(&instance_label(&record.instance)), MAX_NAME_BYTES);
    let name = if label.trim().is_empty() { id.clone() } else { label };
    let url = address::ws_url(&host, record.port, Some(&path));
    Ok(BrowsedHub {
        id,
        name,
        kind: kind.to_owned(),
        auth: auth.to_owned(),
        device_count,
        serials,
        host,
        port: record.port,
        path,
        url,
        addresses,
    })
}

/// What a hub says about itself on the network (spec 3.1).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct HubAdvert {
    /// The instance name: the hub's user-visible name.
    pub name: String,
    /// The WebSocket and HTTP port.
    pub port: u16,
    /// `hid`, a lowercase UUID.
    pub hub_id: String,
    /// `console` or `bridge`.
    pub kind: String,
    /// `none` or `pin`.
    pub auth: String,
    /// Devices shared now.
    pub device_count: u32,
    /// Their serials; left out of the record if it would pass 400 bytes.
    pub serials: Vec<String>,
    /// `GET /` serves a browser UI.
    pub web: bool,
    /// The WebSocket path when it is not the default.
    pub path: Option<String>,
}

impl HubAdvert {
    /// The TXT keys and values, in the order macOS writes them. `d` is dropped
    /// past the spec's limit, by macOS's sum: each entry's length byte, key, `=`
    /// and value.
    pub fn txt(&self) -> Vec<(String, String)> {
        let mut pairs: Vec<(String, String)> = vec![
            ("v".into(), "1".into()),
            ("hid".into(), self.hub_id.clone()),
            ("kind".into(), self.kind.clone()),
            ("auth".into(), self.auth.clone()),
            ("n".into(), self.device_count.to_string()),
        ];
        if self.web {
            pairs.push(("web".into(), "1".into()));
        }
        if let Some(path) = &self.path {
            pairs.push(("path".into(), path.clone()));
        }
        let joined = self.serials.join(",");
        if !joined.is_empty() {
            let size = |k: &str, v: &str| [1, k.len(), 1, v.len()].iter().sum::<usize>();
            let others: usize = pairs.iter().map(|(k, v)| size(k, v)).sum();
            if others.saturating_add(size("d", &joined)) < TXT_LIMIT {
                pairs.push(("d".into(), joined));
            }
        }
        pairs
    }
}

// -- WP HB5 --
/// The `.local` host name a hub with this id advertises, without the trailing dot,
/// for its `Origin` check. `None` when the id has nothing to build a name from. Derived
/// from the hub id since std cannot read the machine's name; it only has to be unique.
pub fn mdns_host_name(hub_id: &str) -> Option<String> {
    let id: String = hub_id.chars().filter(char::is_ascii_alphanumeric).take(12).collect();
    (!id.is_empty()).then(|| format!("dspi-{id}.local"))
}

/// Where a responder speaks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AdvertiseScope {
    /// Every interface: what a hub on the network does.
    #[default]
    AllInterfaces,
    /// The IPv4 loopback interface and nothing else: tests, and a hub that listens only
    /// there, whose address would mean nothing to anyone else.
    LoopbackOnly,
}

/// The names a responder holds right now. Another machine holding a name makes the
/// responder take a new one (RFC 6762 section 9), so these can differ from what was asked.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AdvertisedNames {
    /// The instance name, without the service type: `Studio (2)`.
    pub instance: String,
    /// The `.local` host name, without the trailing dot: `dspi-0123456789ab-2.local`.
    pub host: String,
}

/// What a [`BrowseSource`] reports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceEvent {
    /// A service was resolved, for the first time or again with new data.
    Resolved(ServiceRecord),
    /// This full instance name is gone.
    Removed(String),
    /// The source stopped working. The browser closes it and opens it again.
    Failed(String),
}

enum Event {
    /// From the source opened as this generation.
    Source(u64, SourceEvent),
    /// Ask the network whether this hub is still there.
    Verify(String),
    Stop,
}

/// Where a source delivers events; events from a source already closed are dropped.
#[derive(Clone)]
pub struct SourceSink {
    tx: Sender<Event>,
    generation: u64,
}

impl SourceSink {
    /// Deliver `event`. False once the browser has gone: time to stop.
    pub fn send(&self, event: SourceEvent) -> bool {
        self.tx.send(Event::Source(self.generation, event)).is_ok()
    }
}

/// Where services come from.
pub trait BrowseSource: Send + Sync {
    /// Start looking and deliver events to `sink` until the session is dropped.
    /// Blocks no longer than setting up sockets takes; it runs on the browser's thread.
    fn open(&self, sink: SourceSink) -> Result<Box<dyn BrowseSession>, DiscoveryError>;
}

/// A source that is looking. Dropping it stops it.
pub trait BrowseSession: Send {
    /// Check with the network that this full instance name still answers (RFC
    /// 6762 section 10.4), and report it [`SourceEvent::Removed`] if not. A hub
    /// that lost power sends no goodbye, and stays listed until its record's TTL
    /// runs out, 75 minutes.
    fn verify(&self, instance: &str);
}

/// Where a browse stands, for a UI to show.
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowseSnapshot {
    /// The source is open.
    pub browsing: bool,
    /// Why not, in words. Clears by itself when browsing works again.
    pub error: Option<String>,
    /// The hubs, sorted by name.
    pub hubs: Vec<BrowsedHub>,
}

/// Hears about changes to a browse.
pub trait BrowseListener: Send + Sync {
    /// The snapshot changed. Runs on the browser's thread, so it schedules work
    /// and returns. It may call [`HubBrowser::stop`].
    fn on_hubs_changed(&self, snapshot: BrowseSnapshot);
}

/// A browser's settings.
#[derive(Clone)]
pub struct BrowseOptions {
    /// This app's own hub id. Its own advertisement comes back along with
    /// everyone else's, and is not another hub. Empty = none.
    pub own_hub_id: String,
    /// First wait before opening a failed source again; doubles.
    pub retry_first: Duration,
    /// The longest such wait.
    pub retry_max: Duration,
}

impl Default for BrowseOptions {
    /// As macOS's first wait, and its 30 s ceiling.
    fn default() -> Self {
        Self { own_hub_id: String::new(), retry_first: Duration::from_secs(1), retry_max: Duration::from_secs(30) }
    }
}

/// The services a browse knows of, by full instance name.
struct HubList {
    own_hub_id: String,
    hubs: BTreeMap<String, BrowsedHub>,
}

impl HubList {
    fn apply(&mut self, event: SourceEvent) {
        match event {
            SourceEvent::Failed(_) => {}
            SourceEvent::Removed(instance) => {
                self.hubs.remove(&instance);
            }
            SourceEvent::Resolved(record) => match parse_service(&record) {
                Err(why) => {
                    // A service that stops making sense stops being listed.
                    self.hubs.remove(&record.instance);
                    log::debug!("hub {} not listed: {why}", sanitize_hub_text(&record.instance));
                }
                Ok(hub) if hub.id == self.own_hub_id => {
                    self.hubs.remove(&record.instance);
                }
                Ok(hub) => {
                    // A renamed hub announces a new instance while the old one is
                    // still cached. One hub, one entry: the latest news.
                    self.hubs.retain(|instance, h| h.id != hub.id || *instance == record.instance);
                    if self.hubs.len() < MAX_HUBS || self.hubs.contains_key(&record.instance) {
                        self.hubs.insert(record.instance, hub);
                    } else {
                        log::warn!("more than {MAX_HUBS} hubs on the network: ignoring the rest");
                    }
                }
            },
        }
    }

    fn sorted(&self) -> Vec<BrowsedHub> {
        let mut hubs: Vec<BrowsedHub> = self.hubs.values().cloned().collect();
        hubs.sort_by_cached_key(|h| (h.name.to_lowercase(), h.id.clone()));
        hubs
    }

    fn instance_of(&self, hub_id: &str) -> Option<&str> {
        self.hubs.iter().find(|(_, h)| h.id == hub_id).map(|(instance, _)| instance.as_str())
    }
}

struct Shared {
    stopped: AtomicBool,
    snapshot: Mutex<BrowseSnapshot>,
    listener: Option<Arc<dyn BrowseListener>>,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, BrowseSnapshot> {
        self.snapshot.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Store `next`, and call the listener with no lock held if it differs.
    fn publish(&self, next: BrowseSnapshot) {
        {
            let mut now = self.lock();
            // Under the lock, so a stop's cleared snapshot stays cleared.
            if self.stopped.load(Ordering::SeqCst) || *now == next {
                return;
            }
            now.clone_from(&next);
        }
        if let Some(listener) = &self.listener {
            listener.on_hubs_changed(next);
        }
    }
}

/// A running browse. Dropping it stops it.
pub struct HubBrowser {
    shared: Arc<Shared>,
    tx: Sender<Event>,
    thread: Option<JoinHandle<()>>,
}

impl HubBrowser {
    /// Start browsing `source` on a thread of its own. Errors only when the
    /// thread cannot start: a source that fails is reported in
    /// [`BrowseSnapshot::error`], and tried again.
    pub fn start(
        source: Arc<dyn BrowseSource>,
        listener: Option<Arc<dyn BrowseListener>>,
        options: BrowseOptions,
    ) -> Result<Self, DiscoveryError> {
        let shared = Arc::new(Shared { stopped: AtomicBool::new(false), snapshot: Mutex::default(), listener });
        let (tx, rx) = mpsc::channel();
        let worker = Worker {
            source,
            shared: Arc::clone(&shared),
            tx: tx.clone(),
            list: HubList { own_hub_id: options.own_hub_id.to_lowercase(), hubs: BTreeMap::new() },
            session: None,
            generation: 0,
            error: None,
            retry_first: options.retry_first,
            retry_max: options.retry_max,
            retry_wait: options.retry_first,
            next_open: Some(Instant::now()),
        };
        let thread = thread::Builder::new()
            .name("dspi-link-browse".into())
            .spawn(move || worker.run(&rx))
            .map_err(|e| DiscoveryError::Thread(e.to_string()))?;
        Ok(Self { shared, tx, thread: Some(thread) })
    }

    /// Where the browse stands now. No blocking.
    pub fn snapshot(&self) -> BrowseSnapshot {
        self.shared.lock().clone()
    }

    /// The hub with this `hid` (any case) as last heard. This is what a
    /// reconnect wants: after a new DHCP lease, the same id at a new address.
    pub fn resolve(&self, hub_id: &str) -> Option<BrowsedHub> {
        let id = hub_id.to_lowercase();
        self.shared.lock().hubs.iter().find(|h| h.id == id).cloned()
    }

    /// Ask the network whether this hub is still there; if not, it leaves the
    /// list within seconds. For when dialling it fails. Returns at once.
    pub fn verify(&self, hub_id: &str) {
        let _ = self.tx.send(Event::Verify(hub_id.to_lowercase()));
    }

    /// Stop and forget the hubs. Waits for the thread, so no listener call is in
    /// progress or to come afterwards, unless called from the listener itself:
    /// then that call simply finishes. Safe to call twice.
    pub fn stop(&mut self) {
        {
            let mut now = self.shared.lock();
            self.shared.stopped.store(true, Ordering::SeqCst);
            *now = BrowseSnapshot::default();
        }
        let _ = self.tx.send(Event::Stop);
        if let Some(thread) = self.thread.take() {
            if thread.thread().id() != thread::current().id() {
                let _ = thread.join();
            }
        }
    }
}

impl Drop for HubBrowser {
    fn drop(&mut self) {
        self.stop();
    }
}

/// The browser's thread.
struct Worker {
    source: Arc<dyn BrowseSource>,
    shared: Arc<Shared>,
    tx: Sender<Event>,
    list: HubList,
    session: Option<Box<dyn BrowseSession>>,
    /// Of the source last opened. Events of an earlier one are stale.
    generation: u64,
    error: Option<String>,
    retry_first: Duration,
    retry_max: Duration,
    retry_wait: Duration,
    /// When to open the source; `None` while it is open.
    next_open: Option<Instant>,
}

impl Worker {
    fn run(mut self, rx: &Receiver<Event>) {
        loop {
            self.open_if_due();
            let event = match self.next_open {
                None => rx.recv().map_err(|_| RecvTimeoutError::Disconnected),
                Some(at) => rx.recv_timeout(at.saturating_duration_since(Instant::now())),
            };
            match event {
                Ok(Event::Stop) | Err(RecvTimeoutError::Disconnected) => return,
                Err(RecvTimeoutError::Timeout) => {}
                Ok(Event::Verify(hub_id)) => {
                    if let (Some(session), Some(instance)) = (&self.session, self.list.instance_of(&hub_id)) {
                        session.verify(instance);
                    }
                }
                Ok(Event::Source(generation, event)) if generation == self.generation => {
                    self.on_source_event(event);
                }
                Ok(Event::Source(..)) => {}
            }
        }
    }

    fn open_if_due(&mut self) {
        if !self.next_open.is_some_and(|at| at <= Instant::now()) {
            return;
        }
        self.generation = self.generation.wrapping_add(1);
        let sink = SourceSink { tx: self.tx.clone(), generation: self.generation };
        match self.source.open(sink) {
            Ok(session) => {
                self.session = Some(session);
                self.next_open = None;
                self.retry_wait = self.retry_first;
                self.error = None;
            }
            Err(e) => {
                self.schedule_reopen();
                self.error = Some(e.to_string());
            }
        }
        self.publish();
    }

    fn on_source_event(&mut self, event: SourceEvent) {
        if let SourceEvent::Failed(why) = &event {
            // Drop what it reported too: the new one reports anew, and an entry
            // it does not report again would otherwise never leave.
            self.session = None;
            self.list.hubs.clear();
            self.error = Some(why.clone());
            self.schedule_reopen();
        }
        self.list.apply(event);
        self.publish();
    }

    fn schedule_reopen(&mut self) {
        self.next_open = Some(Instant::now().checked_add(self.retry_wait).unwrap_or_else(Instant::now));
        self.retry_wait = self.retry_wait.saturating_mul(2).min(self.retry_max);
    }

    fn publish(&self) {
        self.shared.publish(BrowseSnapshot {
            browsing: self.session.is_some(),
            error: self.error.clone(),
            hubs: self.list.sorted(),
        });
    }
}

#[cfg(feature = "mdns")]
pub use self::mdns::{MdnsAdvertiser, MdnsSource};

#[cfg(feature = "mdns")]
mod mdns {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    use std::thread;
    use std::time::Duration;

    use mdns_sd::{DaemonEvent, IfKind, RRType, Receiver, ResolvedService, ServiceDaemon, ServiceEvent, ServiceInfo};

    use super::{
        instance_label, mdns_host_name, AdvertiseScope, AdvertisedNames, BrowseSession, BrowseSource, DiscoveryError,
        HubAdvert, ServiceRecord, SourceEvent, SourceSink, SERVICE_TYPE,
    };

    /// How long [`BrowseSession::verify`] gives a hub to answer.
    const VERIFY_TIMEOUT: Duration = Duration::from_secs(3);
    /// How often the forwarding thread looks whether its session has gone.
    const FORWARD_TICK: Duration = Duration::from_millis(500);
    /// How long a closing advertiser waits for its goodbye to go out.
    const GOODBYE_WAIT: Duration = Duration::from_secs(1);

    fn network(e: mdns_sd::Error) -> DiscoveryError {
        DiscoveryError::Network(e.to_string())
    }

    fn record(service: &ResolvedService) -> ServiceRecord {
        ServiceRecord {
            instance: service.get_fullname().to_owned(),
            port: service.get_port(),
            addresses: service.get_addresses().iter().map(mdns_sd::ScopedIp::to_ip_addr).collect(),
            txt: service
                .get_properties()
                .iter()
                .map(|p| (p.key().to_owned(), String::from_utf8_lossy(p.val().unwrap_or_default()).into_owned()))
                .collect(),
        }
    }

    /// The events a browse cares about; the rest are the daemon's bookkeeping.
    pub(super) fn map_event(event: ServiceEvent) -> Option<SourceEvent> {
        match event {
            ServiceEvent::ServiceResolved(service) => Some(SourceEvent::Resolved(record(&service))),
            ServiceEvent::ServiceRemoved(_, instance) => Some(SourceEvent::Removed(instance)),
            _ => None,
        }
    }

    /// Browses with the `mdns-sd` daemon: a pure-Rust responder and resolver on
    /// its own thread, on every interface.
    #[derive(Debug, Default, Clone, Copy)]
    pub struct MdnsSource;

    struct MdnsSession {
        daemon: ServiceDaemon,
        closed: Arc<AtomicBool>,
    }

    impl BrowseSource for MdnsSource {
        fn open(&self, sink: SourceSink) -> Result<Box<dyn BrowseSession>, DiscoveryError> {
            let daemon = ServiceDaemon::new().map_err(network)?;
            let events = daemon.browse(SERVICE_TYPE).map_err(network)?;
            let closed = Arc::new(AtomicBool::new(false));
            let session = MdnsSession { daemon, closed: Arc::clone(&closed) };
            thread::Builder::new()
                .name("dspi-link-mdns".into())
                .spawn(move || loop {
                    match events.recv_timeout(FORWARD_TICK) {
                        Ok(event) => {
                            if let Some(event) = map_event(event) {
                                if !sink.send(event) {
                                    return;
                                }
                            }
                        }
                        Err(mdns_sd::RecvTimeoutError::Timeout) => {}
                        Err(mdns_sd::RecvTimeoutError::Disconnected) => {
                            sink.send(SourceEvent::Failed("the mDNS daemon stopped".to_owned()));
                            return;
                        }
                    }
                    if closed.load(Ordering::SeqCst) {
                        return;
                    }
                })
                .map_err(|e| DiscoveryError::Thread(e.to_string()))?;
            Ok(Box::new(session))
        }
    }

    impl BrowseSession for MdnsSession {
        fn verify(&self, instance: &str) {
            if let Err(e) = self.daemon.verify(instance.to_owned(), VERIFY_TIMEOUT) {
                log::debug!("mDNS verify: {e}");
            }
        }
    }

    impl Drop for MdnsSession {
        fn drop(&mut self) {
            self.closed.store(true, Ordering::SeqCst);
            let _ = self.daemon.stop_browse(SERVICE_TYPE);
            let _ = self.daemon.shutdown();
        }
    }

    /// The service to register for `advert`. The host name is derived from the
    /// hub id, since std cannot read the machine's; it only has to be unique.
    pub(super) fn service_info(advert: &HubAdvert) -> Result<ServiceInfo, DiscoveryError> {
        service_info_rev(advert, None)
    }

    // -- WP HB5 --
    /// The TXT key that makes each announcement a record of its own.
    pub(super) const REVISION_KEY: &str = "rev";

    /// [`service_info`], with `rev=` when given. mdns-sd's browser takes data it has
    /// cached before for no change, though it may since have shown other data: so a
    /// count that goes 1, 2, 1 stays at 2 for anyone browsing with it. A key that differs
    /// every time keeps each announcement new. Spec 3.1: unknown keys are ignored.
    pub(super) fn service_info_rev(advert: &HubAdvert, revision: Option<u32>) -> Result<ServiceInfo, DiscoveryError> {
        let Some(host) = mdns_host_name(&advert.hub_id) else {
            return Err(DiscoveryError::Advert("no hub id".to_owned()));
        };
        let host = format!("{host}.");
        let mut txt = advert.txt();
        // -- WP HB5 --
        if let Some(revision) = revision {
            txt.push((REVISION_KEY.to_owned(), revision.to_string()));
        }
        let props: Vec<(&str, &str)> = txt.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        ServiceInfo::new(SERVICE_TYPE, &advert.name, &host, "", advert.port, props.as_slice())
            .map(ServiceInfo::enable_addr_auto)
            .map_err(|e| DiscoveryError::Advert(e.to_string()))
    }

    /// Advertises a hub until stopped or dropped. Dropping it sends a goodbye,
    /// which can take up to a second: stop it off the UI thread.
    pub struct MdnsAdvertiser {
        daemon: ServiceDaemon,
        fullname: String,
        // -- WP HB5 --
        /// What the daemon says about name conflicts, read by [`Self::names`].
        monitor: Receiver<DaemonEvent>,
        names: AdvertisedNames,
        /// `rev=` of the next registration.
        revision: u32,
    }

    fn names_of(info: &ServiceInfo) -> AdvertisedNames {
        AdvertisedNames {
            instance: instance_label(info.get_fullname()),
            host: info.get_hostname().trim_end_matches('.').to_owned(),
        }
    }

    impl MdnsAdvertiser {
        /// Register `advert` on every interface.
        pub fn start(advert: &HubAdvert) -> Result<Self, DiscoveryError> {
            Self::start_on(advert, AdvertiseScope::AllInterfaces)
        }

        // -- WP HB5 --
        /// Register `advert` in `scope`. The monitor and the interface selection are in
        /// the daemon's queue ahead of the registration, so no conflict goes unheard and
        /// a loopback responder announces nothing anywhere else.
        pub fn start_on(advert: &HubAdvert, scope: AdvertiseScope) -> Result<Self, DiscoveryError> {
            let info = service_info_rev(advert, Some(1))?;
            let daemon = ServiceDaemon::new().map_err(network)?;
            let monitor = daemon.monitor().map_err(network)?;
            if scope == AdvertiseScope::LoopbackOnly {
                daemon.disable_interface(IfKind::All).map_err(network)?;
                daemon.enable_interface(IfKind::LoopbackV4).map_err(network)?;
            }
            let fullname = info.get_fullname().to_owned();
            let names = names_of(&info);
            daemon.register(info).map_err(network)?;
            Ok(Self { daemon, fullname, monitor, names, revision: 1 })
        }

        /// The names held now, after what the daemon has said so far. Read this often
        /// enough to keep up: its queue is bounded, and it also carries every multicast
        /// answer. On a machine with several interfaces a conflict is per interface, and
        /// this has the latest. Never blocks.
        pub fn names(&mut self) -> AdvertisedNames {
            while let Ok(event) = self.monitor.try_recv() {
                let DaemonEvent::NameChange(change) = event else {
                    continue;
                };
                match change.rr_type {
                    RRType::A | RRType::AAAA => {
                        log::info!("mDNS: host name taken, now {}", change.new_name);
                        change.new_name.trim_end_matches('.').clone_into(&mut self.names.host);
                    }
                    // What is left over from a name this has since given up says nothing.
                    _ if change.original.eq_ignore_ascii_case(&self.fullname) => {
                        log::info!("mDNS: instance name taken, now {}", change.new_name);
                        self.names.instance = instance_label(&change.new_name);
                    }
                    _ => {}
                }
            }
            self.names.clone()
        }

        /// Announce changed data (devices came or went), keeping the registration.
        /// A new name replaces the old one, with a goodbye for that.
        pub fn update(&mut self, advert: &HubAdvert) -> Result<(), DiscoveryError> {
            // -- WP HB5 -- A new name is probed, and the responder must not hear its own
            // records under the old one, sent as it renamed itself, as another's: so the
            // revision moves only with new data under the same name.
            let same_name = service_info(advert)?.get_fullname() == self.fullname;
            if same_name {
                self.revision = self.revision.wrapping_add(1);
            }
            let mut info = service_info_rev(advert, Some(self.revision))?;
            if info.get_fullname() != self.fullname {
                self.goodbye();
                info.get_fullname().clone_into(&mut self.fullname);
                // -- WP HB5 --
                self.names.instance = names_of(&info).instance;
            } else {
                // -- WP HB5 -- New data under a name already held is announced, not
                // probed (RFC 6762 section 8.4). A probe would also hear this responder's
                // own late answer with the old data, take it for a conflict and rename.
                info.set_requires_probe(false);
            }
            self.daemon.register(info).map_err(network)
        }

        fn goodbye(&self) {
            if let Ok(done) = self.daemon.unregister(&self.fullname) {
                let _ = done.recv_timeout(GOODBYE_WAIT);
            }
        }
    }

    impl Drop for MdnsAdvertiser {
        fn drop(&mut self) {
            self.goodbye();
            // -- WP HB5 -- Waits for the daemon's thread to end: a hub's stop promises
            // nothing is left running, and the same wait is what the goodbye gets above.
            if let Ok(done) = self.daemon.shutdown() {
                let _ = done.recv_timeout(GOODBYE_WAIT);
            }
        }
    }
}

#[cfg(any(test, feature = "fake-hub"))]
pub use self::fake::FakeAdvertiser;

#[cfg(any(test, feature = "fake-hub"))]
mod fake {
    use std::collections::{BTreeMap, VecDeque};
    use std::net::IpAddr;
    use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

    use super::{
        BrowseSession, BrowseSource, DiscoveryError, HubAdvert, ServiceRecord, SourceEvent, SourceSink, SERVICE_TYPE,
    };

    #[derive(Default)]
    struct State {
        services: BTreeMap<String, ServiceRecord>,
        sink: Option<SourceSink>,
        open_failures: VecDeque<String>,
        opens: usize,
        open_now: usize,
        verified: Vec<String>,
    }

    /// A network of hubs for tests, with no socket: announces and withdraws
    /// services as a real source would report them.
    #[derive(Default, Clone)]
    pub struct FakeAdvertiser {
        state: Arc<Mutex<State>>,
    }

    struct FakeSession {
        state: Arc<Mutex<State>>,
    }

    fn lock(state: &Mutex<State>) -> MutexGuard<'_, State> {
        state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    impl FakeAdvertiser {
        /// A network with nothing on it.
        pub fn new() -> Self {
            Self::default()
        }

        /// The full instance name `advert.name` gets.
        pub fn instance(name: &str) -> String {
            format!("{}.{SERVICE_TYPE}", name.replace('\\', "\\\\").replace('.', "\\."))
        }

        /// A hub announces itself, or announces new data, at these addresses.
        pub fn announce(&self, advert: &HubAdvert, addresses: &[IpAddr]) {
            self.announce_record(ServiceRecord {
                instance: Self::instance(&advert.name),
                port: advert.port,
                addresses: addresses.to_vec(),
                txt: advert.txt(),
            });
        }

        /// Anything at all announces itself.
        pub fn announce_record(&self, record: ServiceRecord) {
            let mut state = lock(&self.state);
            state.services.insert(record.instance.clone(), record.clone());
            if let Some(sink) = &state.sink {
                sink.send(SourceEvent::Resolved(record));
            }
        }

        /// A hub says goodbye.
        pub fn withdraw(&self, name: &str) {
            let instance = Self::instance(name);
            let mut state = lock(&self.state);
            state.services.remove(&instance);
            if let Some(sink) = &state.sink {
                sink.send(SourceEvent::Removed(instance));
            }
        }

        /// The open source stops working.
        pub fn fail(&self, why: &str) {
            if let Some(sink) = lock(&self.state).sink.take() {
                sink.send(SourceEvent::Failed(why.to_owned()));
            }
        }

        /// The next opens fail with these words, one each.
        pub fn fail_next_opens(&self, why: &[&str]) {
            lock(&self.state).open_failures.extend(why.iter().map(|w| (*w).to_owned()));
        }

        /// Sends what a session that was replaced would still send.
        pub fn stale_sink(&self) -> Option<SourceSink> {
            lock(&self.state).sink.clone()
        }

        /// How many times the source was opened, failures included.
        pub fn opens(&self) -> usize {
            lock(&self.state).opens
        }

        /// How many sessions are open.
        pub fn open_now(&self) -> usize {
            lock(&self.state).open_now
        }

        /// The instances [`BrowseSession::verify`] asked about.
        pub fn verified(&self) -> Vec<String> {
            lock(&self.state).verified.clone()
        }
    }

    impl BrowseSource for FakeAdvertiser {
        fn open(&self, sink: SourceSink) -> Result<Box<dyn BrowseSession>, DiscoveryError> {
            let mut state = lock(&self.state);
            state.opens = state.opens.saturating_add(1);
            if let Some(why) = state.open_failures.pop_front() {
                return Err(DiscoveryError::Network(why));
            }
            state.open_now = state.open_now.saturating_add(1);
            for record in state.services.values() {
                sink.send(SourceEvent::Resolved(record.clone()));
            }
            state.sink = Some(sink);
            Ok(Box::new(FakeSession { state: Arc::clone(&self.state) }))
        }
    }

    impl BrowseSession for FakeSession {
        fn verify(&self, instance: &str) {
            lock(&self.state).verified.push(instance.to_owned());
        }
    }

    impl Drop for FakeSession {
        fn drop(&mut self) {
            let mut state = lock(&self.state);
            state.open_now = state.open_now.saturating_sub(1);
            state.sink = None;
        }
    }
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
#[path = "discovery/tests_wp_cg2.rs"]
mod tests_wp_cg2;
