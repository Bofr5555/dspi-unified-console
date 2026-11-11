//! -- WP DISC2 -- Windows' own DNS-SD behind [`BrowseSource`], and the hub's responder:
//! dnsapi.dll's `DnsServiceBrowse`, `DnsServiceResolve`, `DnsServiceRegister` and
//! `DnsServiceDeRegister` (Windows 10 1809+). The DNS Client service does the multicast
//! under its own firewall rule, so this process opens no socket for it, and Windows asks
//! nothing while a client only browses (WP DISC1 measured it).
//!
//! # Native lifetimes
//!
//! dnsapi calls back on threads of its own, with the context pointer it was given, and
//! may read the request, its strings and the context until its last callback. Each
//! context is an [`Arc`]; a share goes to dnsapi when a call is accepted ([`give`]) and
//! comes back in the callback that ends that call ([`take`]):
//! - a browse ends with a callback of status `ERROR_CANCELLED`, after
//!   `DnsServiceBrowseCancel`;
//! - a resolve ends with its one callback: an answer, or `ERROR_CANCELLED`;
//! - a registration has a callback for the registration and one for the
//!   deregistration (or the cancel).
//!
//! A call that never calls back leaves its context allocated, a few hundred bytes, as a
//! use-after-free would be worse. [`native_contexts`] counts the shares dnsapi holds, so a
//! test can see that nothing stays behind.
//!
//! # Addresses
//!
//! A resolve gives one address pair, for one interface Windows picks, and for seconds at
//! a time that can be an IPv6 link-local address alone, which a URL cannot carry. Then
//! the SRV target's host name is looked up (getaddrinfo, which asks over mDNS for a
//! `.local` name), much as upstream's app and the Mac dial a hub by its name; the core
//! still gets addresses to rank.
//!
//! # Updates and conflicts
//!
//! There is no call to change a registration's TXT, so an update is a deregistration and
//! a new registration: browsers see a goodbye and the hub again ~0.75 s later, which the
//! browsers' removal grace hides ([`super::REMOVAL_GRACE`]). A registration that fails,
//! or that Windows does not answer, is tried again with backoff.

#![allow(unsafe_code)]

use std::collections::HashMap;
use std::ffi::c_void;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, ToSocketAddrs};
use std::ptr;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use windows_sys::Win32::Foundation::{DNS_REQUEST_PENDING, HANDLE};
use windows_sys::Win32::NetworkManagement::Dns::{
    DnsFree, DnsFreeRecordList, DnsServiceBrowse, DnsServiceBrowseCancel, DnsServiceConstructInstance,
    DnsServiceDeRegister, DnsServiceFreeInstance, DnsServiceRegister, DnsServiceRegisterCancel, DnsServiceResolve,
    DnsServiceResolveCancel, DNS_QUERY_REQUEST_VERSION1, DNS_RECORDW, DNS_SERVICE_BROWSE_REQUEST,
    DNS_SERVICE_BROWSE_REQUEST_0, DNS_SERVICE_CANCEL, DNS_SERVICE_INSTANCE, DNS_SERVICE_REGISTER_REQUEST,
    DNS_SERVICE_RESOLVE_REQUEST, DNS_TYPE_PTR,
};
use windows_sys::Win32::NetworkManagement::IpHelper::{
    CancelMibChangeNotify2, NotifyIpInterfaceChange, MIB_IPINTERFACE_ROW, MIB_NOTIFICATION_TYPE,
};
use windows_sys::Win32::Networking::WinSock::AF_UNSPEC;
use windows_sys::Win32::System::SystemInformation::{ComputerNameDnsHostname, GetComputerNameExW};

use super::{
    address_rank, instance_label, AdvertiseScope, AdvertisedNames, BrowseSession, BrowseSource, DiscoveryError,
    HubAdvert, ServiceRecord, SourceEvent, SourceSink,
};
use crate::clock::Instant;

/// The service type as Windows wants it: no trailing dot.
const QUERY: &str = "_dspi._tcp.local";
/// How long one resolve may take. Most answer within ~1 s (Avahi waits that long); one
/// that does not now and then never does (measured: a lost answer is not asked again),
/// and is cancelled and tried again.
const RESOLVE_TIMEOUT: Duration = Duration::from_secs(2);
/// Resolves of one announcement that gave nothing to dial.
const RESOLVE_TRIES: u32 = 4;
/// The wait before resolving again a hub that gave only its host name: past mdns-sd's
/// one-second limit on sending its A records again.
const RESOLVE_AGAIN: Duration = Duration::from_millis(1100);
/// The least time between two resolves of one hub. Avahi and Windows put the PTR record
/// in every answer, and each PTR heard asks for a resolve, so without this a hub would
/// be resolved about once a second for as long as it is browsed. One heard meanwhile is
/// resolved when the time is up, so a TXT change shows within this (upstream drops it).
const RERESOLVE_GAP: Duration = Duration::from_secs(5);
/// How long a registration may wait for Windows' answer before it is tried again.
const REGISTER_TIMEOUT: Duration = Duration::from_secs(10);
/// The wait between the goodbye of an update and its new registration. Windows sets the
/// cache-flush bit on the shared PTR record when it announces the new one (RFC 6762
/// section 10.2 says it must not), and mdns-sd then drops the instance some 0.25 s after
/// showing its new data, until Windows next sends the PTR (measured; WP DISC2). Once the
/// goodbye's one second has passed, a browser sees an ordinary goodbye and a new hub,
/// which its removal grace covers.
const REREGISTER_GAP: Duration = Duration::from_millis(1200);
/// How long the network must stay unchanged before registering again after a change.
const NETWORK_SETTLE: Duration = Duration::from_secs(3);
/// How often to check that Windows still holds the registration, and how soon after it
/// answers the first time (a clash on this machine shows only then).
const SELF_CHECK: Duration = Duration::from_secs(30);
const FIRST_CHECK: Duration = Duration::from_secs(2);
/// The most a hub gives way to others on this machine, as the hub's own limit.
const MAX_LOCAL_SUFFIX: u32 = 98;
/// How long a withdrawal waits for Windows to acknowledge it.
const DEREGISTER_WAIT: Duration = Duration::from_secs(2);
/// How long a closing browse waits for its last callback.
const CANCEL_WAIT: Duration = Duration::from_secs(2);
/// Waits before registering again after a failure: doubles to the last.
const RETRY_FIRST: Duration = Duration::from_secs(1);
const RETRY_MAX: Duration = Duration::from_secs(60);
/// Addresses kept per hub.
const MAX_KEPT: usize = 8;
const ERROR_CANCELLED: u32 = 1223;
const PENDING: u32 = DNS_REQUEST_PENDING.unsigned_abs();

/// Shares dnsapi holds now.
static NATIVE: AtomicUsize = AtomicUsize::new(0);

/// How many contexts dnsapi still holds: zero once every browse, resolve and
/// advertiser has ended. For tests and the probe.
pub fn native_contexts() -> usize {
    NATIVE.load(Ordering::SeqCst)
}

/// Hand a share of `arc` to dnsapi.
fn give<T>(arc: &Arc<T>) -> *const T {
    let n = NATIVE.fetch_add(1, Ordering::SeqCst);
    log::trace!("Windows DNS-SD: give {} -> {}", std::any::type_name::<T>(), n.wrapping_add(1));
    Arc::into_raw(Arc::clone(arc))
}

/// Take back a share [`give`] handed over.
///
/// # Safety
/// `raw` came from [`give`], and each share is taken back once.
unsafe fn take<T>(raw: *const T) {
    let n = NATIVE.fetch_sub(1, Ordering::SeqCst);
    log::trace!("Windows DNS-SD: take {} -> {}", std::any::type_name::<T>(), n.wrapping_sub(1));
    // SAFETY: the caller's contract.
    drop(unsafe { Arc::from_raw(raw) });
}

/// An owned [`Arc`] from a share dnsapi still holds, which stays with dnsapi.
///
/// # Safety
/// `raw` came from [`give`] and has not been taken back.
unsafe fn borrow<T>(raw: *const T) -> Arc<T> {
    // SAFETY: the share keeps the count above zero while this adds one of ours.
    unsafe {
        Arc::increment_strong_count(raw);
        Arc::from_raw(raw)
    }
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

/// # Safety
/// `p` is null or a NUL-terminated UTF-16 string.
unsafe fn from_wide(p: *const u16) -> String {
    if p.is_null() {
        return String::new();
    }
    let mut len = 0usize;
    // SAFETY: NUL-terminated, so every unit up to the NUL may be read.
    while unsafe { *p.add(len) } != 0 {
        len = len.saturating_add(1);
    }
    // SAFETY: the `len` units just read.
    String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(p, len) })
}

/// Windows spells names without the root's dot; discovery.rs with it.
fn rooted(name: String) -> String {
    if name.ends_with('.') {
        name
    } else {
        name + "."
    }
}

fn dialable(addresses: &[IpAddr]) -> Vec<IpAddr> {
    addresses.iter().filter(|ip| address_rank(ip).is_some()).copied().collect()
}

// ---- browsing ----

/// Browses through the DNS Client service.
#[derive(Debug, Default, Clone, Copy)]
pub struct DnsSdSource;

/// What a browse knows of one instance.
#[derive(Default)]
struct Instance {
    /// Goodbyes heard: a resolve started before one must not bring the hub back.
    epoch: u64,
    announced: bool,
    /// A resolve of this epoch is running.
    resolving: bool,
    /// Announced again while it ran: the data may have changed since it asked.
    again: bool,
    /// The last addresses to dial, for a later resolve that gives none.
    kept: Vec<IpAddr>,
}

struct BrowseCtx {
    sink: SourceSink,
    closed: AtomicBool,
    /// Windows may read it until the browse ends.
    query: Vec<u16>,
    /// By lower-case instance name.
    instances: Mutex<HashMap<String, Instance>>,
    /// The resolves running, to stop when the browse closes.
    aborts: Mutex<HashMap<u64, Sender<Answer>>>,
    next_resolve: AtomicU64,
    /// The last callback has come.
    ended: (Mutex<bool>, Condvar),
}

impl BrowseCtx {
    /// A channel for one resolving thread, which the browse can stop.
    fn resolver(&self) -> Option<(u64, Sender<Answer>, Receiver<Answer>)> {
        let id = self.next_resolve.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = mpsc::channel();
        let mut aborts = lock(&self.aborts);
        // Checked under the lock the closing browse drains, so none is missed.
        if self.closed.load(Ordering::SeqCst) {
            return None;
        }
        aborts.insert(id, tx.clone());
        Some((id, tx, rx))
    }

    fn resolved(&self, id: u64) {
        lock(&self.aborts).remove(&id);
    }
}

struct WinSession {
    ctx: Arc<BrowseCtx>,
    cancel: DNS_SERVICE_CANCEL,
}

// SAFETY: the cancel handle is an opaque token dnsapi accepts from any thread; the rest
// is an Arc of Send + Sync parts.
unsafe impl Send for WinSession {}

impl BrowseSource for DnsSdSource {
    fn open(&self, sink: SourceSink) -> Result<Box<dyn BrowseSession>, DiscoveryError> {
        let ctx = Arc::new(BrowseCtx {
            sink,
            closed: AtomicBool::new(false),
            query: wide(QUERY),
            instances: Mutex::default(),
            aborts: Mutex::default(),
            next_resolve: AtomicU64::new(0),
            ended: (Mutex::new(false), Condvar::new()),
        });
        let raw = give(&ctx);
        let request = DNS_SERVICE_BROWSE_REQUEST {
            Version: DNS_QUERY_REQUEST_VERSION1,
            InterfaceIndex: 0,
            QueryName: ctx.query.as_ptr(),
            Anonymous: DNS_SERVICE_BROWSE_REQUEST_0 { pBrowseCallback: Some(on_browse) },
            pQueryContext: raw.cast_mut().cast(),
        };
        let mut cancel = DNS_SERVICE_CANCEL { reserved: ptr::null_mut() };
        // SAFETY: request and cancel are valid for the call; the query name lives in the
        // context, whose share dnsapi holds until the last callback.
        let status = unsafe { DnsServiceBrowse(&request, &mut cancel) };
        if status != DNS_REQUEST_PENDING {
            // SAFETY: not accepted, so no callback takes it.
            unsafe { take(raw) };
            return Err(DiscoveryError::Network(format!("Windows DNS-SD browse refused: {status}")));
        }
        Ok(Box::new(WinSession { ctx, cancel }))
    }
}

/// # Safety
/// Called by dnsapi with the context [`DnsSdSource::open`] gave it and a record list it
/// hands over.
unsafe extern "system" fn on_browse(status: u32, context: *const c_void, records: *const DNS_RECORDW) {
    let raw = context.cast::<BrowseCtx>();
    let mut events = Vec::new();
    let mut rec = records;
    while !rec.is_null() {
        // SAFETY: a list dnsapi built, valid until freed below.
        let r = unsafe { &*rec };
        if r.wType == DNS_TYPE_PTR {
            // SAFETY: a PTR record's data is its PTR member, a string dnsapi owns.
            let instance = rooted(unsafe { from_wide(r.Data.PTR.pNameHost) });
            events.push((instance, r.dwTtl == 0));
        }
        rec = r.pNext;
    }
    if !records.is_null() {
        // SAFETY: handed to us, freed once.
        unsafe { DnsFree(records.cast(), DnsFreeRecordList) };
    }
    if status == ERROR_CANCELLED {
        log::debug!("Windows DNS-SD: browse ended");
        {
            // SAFETY: the share is still held.
            let ctx = unsafe { &*raw };
            let (ended, cv) = &ctx.ended;
            *lock(ended) = true;
            cv.notify_all();
        }
        // SAFETY: the browse's last callback.
        unsafe { take(raw) };
        return;
    }
    // SAFETY: the share is held until the cancelled callback above.
    let ctx = unsafe { borrow(raw) };
    if ctx.closed.load(Ordering::SeqCst) {
        return;
    }
    if status != 0 {
        ctx.sink.send(SourceEvent::Failed(format!("Windows DNS-SD browse failed: {status}")));
        return;
    }
    for (instance, goodbye) in events {
        heard(&ctx, instance, goodbye);
    }
}

/// One PTR record: a goodbye, or a hub to resolve.
fn heard(ctx: &Arc<BrowseCtx>, instance: String, goodbye: bool) {
    let key = instance.to_ascii_lowercase();
    let epoch = {
        let mut instances = lock(&ctx.instances);
        let entry = instances.entry(key.clone()).or_default();
        if goodbye {
            entry.epoch = entry.epoch.wrapping_add(1);
            entry.announced = false;
            entry.resolving = false;
            entry.again = false;
            entry.kept.clear();
            None
        } else if entry.resolving {
            entry.again = true;
            None
        } else {
            entry.announced = true;
            entry.resolving = true;
            Some(entry.epoch)
        }
    };
    if goodbye {
        ctx.sink.send(SourceEvent::Removed(instance));
        return;
    }
    let Some(epoch) = epoch else {
        return;
    };
    let Some((id, tx, rx)) = ctx.resolver() else {
        return;
    };
    let owned = Arc::clone(ctx);
    let thread_key = key.clone();
    let spawned = thread::Builder::new().name("dspi-dnssd-resolve".into()).spawn(move || {
        // Stopped early by the browse closing.
        while resolve_hub(&owned, &thread_key, epoch, &instance, &tx, &rx)
            && matches!(rx.recv_timeout(RERESOLVE_GAP), Err(RecvTimeoutError::Timeout))
        {}
        owned.resolved(id);
    });
    if let Err(e) = spawned {
        log::warn!("Windows DNS-SD: no thread to resolve with: {e}");
        ctx.resolved(id);
        if let Some(entry) = lock(&ctx.instances).get_mut(&key) {
            entry.resolving = false;
        }
    }
}

/// Resolve an announced hub until there is an address to dial, and report it unless a
/// goodbye came meanwhile. True when it was announced again meanwhile, to be resolved
/// once more.
///
/// A resolve that gives only an IPv6 link-local address (measured against mdns-sd on
/// Linux: it answers Windows' follow-up A query only after its one-second rate limit, and
/// Windows does not use the A records it sent with the SRV) is reported at once with the
/// host name to dial, as upstream's app and the Mac dial a hub, and resolved again a
/// second later for an address.
fn resolve_hub(
    ctx: &BrowseCtx,
    key: &str,
    epoch: u64,
    instance: &str,
    tx: &Sender<Answer>,
    rx: &Receiver<Answer>,
) -> bool {
    let mut by_name = false;
    for _ in 0..RESOLVE_TRIES {
        // Stopped early by the browse closing.
        if by_name && !matches!(rx.recv_timeout(RESOLVE_AGAIN), Err(RecvTimeoutError::Timeout)) {
            break;
        }
        if !current(ctx, key, epoch) {
            break;
        }
        // No answer in time: at once again.
        let Ok((mut record, host)) = resolve(instance, tx, rx) else {
            continue;
        };
        let mut addresses = dialable(&record.addresses);
        if addresses.is_empty() && !host.is_empty() {
            addresses = look_up(&host, record.port);
        }
        if addresses.is_empty() {
            addresses = lock(&ctx.instances).get(key).map(|e| e.kept.clone()).unwrap_or_default();
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
        if !report(ctx, key, epoch, instance, record) || dialled_by_address {
            break;
        }
    }
    let mut instances = lock(&ctx.instances);
    let Some(entry) = instances.get_mut(key).filter(|e| e.epoch == epoch) else {
        return false;
    };
    let again = std::mem::take(&mut entry.again) && !ctx.closed.load(Ordering::SeqCst);
    entry.resolving = again;
    again
}

/// Still announced, as of this epoch, and the browse open.
fn current(ctx: &BrowseCtx, key: &str, epoch: u64) -> bool {
    lock(&ctx.instances).get(key).is_some_and(|e| e.epoch == epoch && e.announced) && !ctx.closed.load(Ordering::SeqCst)
}

/// Hand a resolved record to the browser, unless a goodbye came meanwhile. False then.
fn report(ctx: &BrowseCtx, key: &str, epoch: u64, instance: &str, mut record: ServiceRecord) -> bool {
    let mut instances = lock(&ctx.instances);
    let Some(entry) = instances.get_mut(key).filter(|e| e.epoch == epoch && e.announced) else {
        return false;
    };
    if ctx.closed.load(Ordering::SeqCst) {
        return false;
    }
    if !record.addresses.is_empty() {
        record.addresses.clone_into(&mut entry.kept);
    }
    // The name the browse heard, which a goodbye will name too.
    instance.clone_into(&mut record.instance);
    // Under the lock, so a goodbye cannot pass it.
    ctx.sink.send(SourceEvent::Resolved(record));
    true
}

/// The SRV target's addresses as the system resolver has them now, from the DNS
/// Client's cache, which often has what the resolve left out.
fn look_up(host: &str, port: u16) -> Vec<IpAddr> {
    let host = host.trim_end_matches('.');
    let t0 = Instant::now();
    match (host, port).to_socket_addrs() {
        Ok(found) => {
            let ips: Vec<IpAddr> = found.map(|a| a.ip()).collect();
            log::debug!("Windows DNS-SD: {host} is {ips:?} ({:?})", t0.elapsed());
            dialable(&ips)
        }
        Err(e) => {
            log::debug!("Windows DNS-SD: looking up {host}: {e}");
            Vec::new()
        }
    }
}

impl BrowseSession for WinSession {
    /// Resolve it again; no answer is a removal. Windows may still answer from its cache
    /// for a while, so a hub that vanished can take until its record's TTL to leave.
    fn verify(&self, instance: &str) {
        let Some((id, tx, rx)) = self.ctx.resolver() else {
            return;
        };
        let ctx = Arc::clone(&self.ctx);
        let instance = instance.to_owned();
        let spawned = thread::Builder::new().name("dspi-dnssd-verify".into()).spawn(move || {
            let gone = resolve(&instance, &tx, &rx).is_err();
            if gone && !ctx.closed.load(Ordering::SeqCst) {
                ctx.sink.send(SourceEvent::Removed(instance));
            }
            ctx.resolved(id);
        });
        if let Err(e) = spawned {
            log::debug!("Windows DNS-SD verify: {e}");
            self.ctx.resolved(id);
        }
    }
}

impl Drop for WinSession {
    fn drop(&mut self) {
        {
            let mut aborts = lock(&self.ctx.aborts);
            self.ctx.closed.store(true, Ordering::SeqCst);
            for (_, abort) in aborts.drain() {
                let _ = abort.send(Answer::Abort);
            }
        }
        // SAFETY: the handle DnsServiceBrowse filled in, for a browse that only this
        // cancel ends.
        let status = unsafe { DnsServiceBrowseCancel(&self.cancel) };
        if status != 0 {
            log::debug!("DnsServiceBrowseCancel: {status}");
        }
        let (ended, cv) = &self.ctx.ended;
        let (guard, _) =
            cv.wait_timeout_while(lock(ended), CANCEL_WAIT, |e| !*e).unwrap_or_else(PoisonError::into_inner);
        if !*guard {
            log::warn!("Windows DNS-SD: a browse did not end; its context stays allocated");
        }
    }
}

// ---- resolving ----

enum Answer {
    /// The callback of resolve number `.0`.
    Done(u64, Option<(ServiceRecord, String)>),
    /// The browse closed.
    Abort,
}

struct ResolveCtx {
    attempt: u64,
    tx: Mutex<Option<Sender<Answer>>>,
    /// Between the callback and a cancel.
    completion: Completion,
    /// Windows may read it until the callback.
    name: Vec<u16>,
}

static ATTEMPTS: AtomicU64 = AtomicU64::new(0);

/// One resolve, waited for: the record and the SRV target's host name. An `Abort` on
/// `rx` cuts it short.
fn resolve(instance: &str, tx: &Sender<Answer>, rx: &Receiver<Answer>) -> Result<(ServiceRecord, String), String> {
    let attempt = ATTEMPTS.fetch_add(1, Ordering::SeqCst);
    let ctx = Arc::new(ResolveCtx {
        attempt,
        tx: Mutex::new(Some(tx.clone())),
        completion: Completion::default(),
        name: wide(instance.trim_end_matches('.')),
    });
    let raw = give(&ctx);
    log::debug!("Windows DNS-SD: resolve {attempt} {instance}");
    let request = DNS_SERVICE_RESOLVE_REQUEST {
        Version: DNS_QUERY_REQUEST_VERSION1,
        InterfaceIndex: 0,
        // dnsapi does not write through it.
        QueryName: ctx.name.as_ptr().cast_mut(),
        pResolveCompletionCallback: Some(on_resolve),
        pQueryContext: raw.cast_mut().cast(),
    };
    let mut cancel = DNS_SERVICE_CANCEL { reserved: ptr::null_mut() };
    // SAFETY: request and cancel are valid for the call; the name lives in the context,
    // whose share dnsapi holds until the callback.
    let status = unsafe { DnsServiceResolve(&request, &mut cancel) };
    if status != DNS_REQUEST_PENDING {
        // SAFETY: not accepted, so no callback takes it.
        unsafe { take(raw) };
        return Err(format!("DnsServiceResolve refused: {status}"));
    }
    let deadline = Instant::now().checked_add(RESOLVE_TIMEOUT);
    loop {
        let left = deadline.map_or(Duration::ZERO, |d| d.saturating_duration_since(Instant::now()));
        match rx.recv_timeout(left) {
            Ok(Answer::Done(n, answer)) if n == attempt => return answer.ok_or_else(|| "no answer".to_owned()),
            // A late answer to an earlier resolve on this channel.
            Ok(Answer::Done(..)) => {}
            Ok(Answer::Abort) | Err(_) => break,
        }
    }
    if ctx.completion.begin_cancel() {
        // SAFETY: the handle DnsServiceResolve filled in, for a resolve whose callback had
        // not started (`begin_cancel`). The callback (cancelled) takes the share back.
        let status = unsafe { DnsServiceResolveCancel(&cancel) };
        ctx.completion.end_cancel();
        if status != 0 {
            log::debug!("DnsServiceResolveCancel: {status}");
        }
    }
    Err("no answer in time".to_owned())
}

thread_local! {
    /// This thread is inside a dnsapi cancel.
    static CANCELLING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Where a call stands between dnsapi's callback and a cancel of it. Measured on
/// Windows 11 (WP DISC2): a cancel calls the cancelled callback before it returns,
/// sometimes on the cancelling thread and sometimes on another one that it waits for;
/// and a call that has called back must not be cancelled, as its handle may be gone.
/// So no lock may be held across a cancel, and a callback that finds a cancel under way
/// on another thread gives it a moment to reach dnsapi before returning.
#[derive(Default)]
struct Completion(std::sync::atomic::AtomicU8);

impl Completion {
    const PENDING: u8 = 0;
    const CANCELLING: u8 = 1;
    const DONE: u8 = 2;
    /// How long a callback waits for a cancel under way.
    const CANCEL_GRACE: Duration = Duration::from_millis(50);

    /// Before a cancel: false when the callback has started, and it is too late.
    fn begin_cancel(&self) -> bool {
        let ok = self.0.compare_exchange(Self::PENDING, Self::CANCELLING, Ordering::SeqCst, Ordering::SeqCst).is_ok();
        if ok {
            CANCELLING.with(|c| c.set(true));
        }
        ok
    }

    /// After the cancel returned.
    fn end_cancel(&self) {
        CANCELLING.with(|c| c.set(false));
        self.0.store(Self::DONE, Ordering::SeqCst);
    }

    /// First thing in a callback.
    fn called_back(&self) {
        if self.0.compare_exchange(Self::PENDING, Self::DONE, Ordering::SeqCst, Ordering::SeqCst).is_ok()
            || CANCELLING.with(std::cell::Cell::get)
        {
            return;
        }
        let end = Instant::now().checked_add(Self::CANCEL_GRACE);
        while self.0.load(Ordering::SeqCst) == Self::CANCELLING && end.is_some_and(|e| Instant::now() < e) {
            thread::sleep(Duration::from_millis(1));
        }
    }
}

/// # Safety
/// Called by dnsapi with [`resolve`]'s context and an instance it hands over.
unsafe extern "system" fn on_resolve(status: u32, context: *const c_void, instance: *const DNS_SERVICE_INSTANCE) {
    let raw = context.cast::<ResolveCtx>();
    // SAFETY: the share is held until the end of this callback.
    log::debug!("Windows DNS-SD: resolve {} answered {status}", unsafe { (*raw).attempt });
    {
        // SAFETY: the share is held until the end of this callback.
        let ctx = unsafe { &*raw };
        ctx.completion.called_back();
        let answer = if status == 0 && !instance.is_null() {
            // SAFETY: a filled-in instance dnsapi handed over.
            Some(unsafe { read_instance(&*instance) })
        } else {
            None
        };
        if !instance.is_null() {
            // SAFETY: ours to free, once.
            unsafe { DnsServiceFreeInstance(instance) };
        }
        if let Some(tx) = lock(&ctx.tx).take() {
            let _ = tx.send(Answer::Done(ctx.attempt, answer));
        }
    }
    // SAFETY: a resolve's only callback.
    unsafe { take(raw) };
}

/// # Safety
/// `inst` is a filled-in instance from dnsapi.
unsafe fn read_instance(inst: &DNS_SERVICE_INSTANCE) -> (ServiceRecord, String) {
    let mut addresses = Vec::new();
    if !inst.ip4Address.is_null() {
        // SAFETY: non-null, so it points at the address, in network order in memory.
        addresses.push(IpAddr::V4(Ipv4Addr::from(unsafe { *inst.ip4Address }.to_ne_bytes())));
    }
    if !inst.ip6Address.is_null() {
        // SAFETY: non-null, so it points at the address; every member of the union
        // is the same 16 bytes.
        addresses.push(IpAddr::V6(Ipv6Addr::from(unsafe { (*inst.ip6Address).IP6Byte })));
    }
    let count = usize::try_from(inst.dwPropertyCount).unwrap_or(0);
    let txt = if inst.keys.is_null() || inst.values.is_null() {
        Vec::new()
    } else {
        (0..count)
            .map(|i| {
                // SAFETY: both arrays hold dwPropertyCount strings.
                unsafe { (from_wide(*inst.keys.add(i)), from_wide(*inst.values.add(i))) }
            })
            .collect()
    };
    // SAFETY: strings of the instance, or null.
    let (name, host) = unsafe { (from_wide(inst.pszInstanceName), from_wide(inst.pszHostName)) };
    (ServiceRecord { instance: rooted(name), port: inst.wPort, addresses, txt, host_name: String::new() }, host)
}

// ---- the hub role ----

/// This machine's `.local` name, which the DNS Client service answers for.
fn host_name() -> String {
    let mut buf = [0u16; 256];
    let mut len = u32::try_from(buf.len()).unwrap_or(0);
    // SAFETY: `buf` holds `len` units; Windows writes at most that and says how many.
    let ok = unsafe { GetComputerNameExW(ComputerNameDnsHostname, buf.as_mut_ptr(), &mut len) } != 0;
    let name = buf.get(..usize::try_from(len).unwrap_or(0)).map(String::from_utf16_lossy).unwrap_or_default();
    let name = if ok && !name.is_empty() {
        name
    } else {
        std::env::var("COMPUTERNAME").unwrap_or_else(|_| "dspi-hub".to_owned())
    };
    format!("{name}.local")
}

/// The instance label: Windows takes it as the first label of a dotted name, with no
/// escaping, so dots and control characters go, and it is cut to 63 bytes.
fn label(name: &str) -> String {
    let mut out = String::new();
    for c in name.trim().chars().map(|c| if c == '.' || c.is_control() { '-' } else { c }) {
        if out.len().saturating_add(c.len_utf8()) > 63 {
            break;
        }
        out.push(c);
    }
    out
}

enum Cmd {
    Update(HubAdvert),
    /// Windows answered registration `id`.
    Answered {
        id: u64,
        status: u32,
        name: String,
    },
    /// An interface came, went or changed.
    NetworkChanged,
    Stop,
}

/// Interface changes, as [`Cmd::NetworkChanged`]. Windows drops a registration when an
/// adapter goes down and does not bring it back, with no callback to say so (measured,
/// for this and for upstream's app: WP DISC2), so the advertiser registers again after
/// the network settles.
struct NetWatch {
    handle: HANDLE,
    /// The callback's context; freed after the cancel, which waits for callbacks.
    ctx: *mut Mutex<Sender<Cmd>>,
}

impl NetWatch {
    fn start(tx: &Sender<Cmd>) -> Option<Self> {
        let ctx = Box::into_raw(Box::new(Mutex::new(tx.clone())));
        let mut handle: HANDLE = ptr::null_mut();
        // SAFETY: the context stays allocated until CancelMibChangeNotify2 has returned;
        // `handle` receives the registration.
        let status = unsafe { NotifyIpInterfaceChange(AF_UNSPEC, Some(on_net_change), ctx.cast(), false, &mut handle) };
        if status != 0 {
            // SAFETY: not registered, so never called with it.
            drop(unsafe { Box::from_raw(ctx) });
            log::warn!(
                "Windows DNS-SD: no interface notifications ({status}): a lost registration waits for the self-check"
            );
            return None;
        }
        Some(Self { handle, ctx })
    }
}

impl Drop for NetWatch {
    fn drop(&mut self) {
        // SAFETY: the handle NotifyIpInterfaceChange gave; the cancel returns once no
        // callback is running or to come, and it is not called from one.
        unsafe { CancelMibChangeNotify2(self.handle) };
        // SAFETY: allocated in `start`, no longer reachable by a callback.
        drop(unsafe { Box::from_raw(self.ctx) });
    }
}

/// # Safety
/// Called by iphlpapi with [`NetWatch::start`]'s context.
unsafe extern "system" fn on_net_change(
    context: *const c_void,
    _: *const MIB_IPINTERFACE_ROW,
    _: MIB_NOTIFICATION_TYPE,
) {
    // SAFETY: alive until the cancel, which waits for this call.
    let tx = unsafe { &*context.cast::<Mutex<Sender<Cmd>>>() };
    let _ = lock(tx).send(Cmd::NetworkChanged);
}

/// Where a registration's callbacks stand.
#[derive(Default)]
struct RegState {
    /// Calls dnsapi accepted and has not called back.
    pending: u32,
    /// Deregistered or cancelled: the next callbacks are not the registration's answer.
    withdrawing: bool,
    /// The registration's answer has come.
    answered: bool,
    /// ... and it was a success: it stands until deregistered.
    registered: bool,
}

/// One registration and everything dnsapi may read while it stands.
struct Reg {
    id: u64,
    tx: Mutex<Sender<Cmd>>,
    request: DNS_SERVICE_REGISTER_REQUEST,
    /// Ours, built by DnsServiceConstructInstance; freed with this.
    instance: *mut DNS_SERVICE_INSTANCE,
    state: Mutex<RegState>,
    withdrawn: Condvar,
    /// Between the registration's answer and a cancel of it.
    completion: Completion,
    /// What Windows registered when it renamed the instance, to deregister by.
    renamed: Mutex<Option<Renamed>>,
}

/// A registration Windows took under another name ("Studio(1)", on a conflict). It is
/// deregistered by that name: by the name asked for, nothing goes, and Windows answers
/// for the renamed instance after this process has ended (measured; WP DISC2).
struct Renamed {
    /// dnsapi's copy, handed to the callback; ours to free.
    instance: *mut DNS_SERVICE_INSTANCE,
    /// [`Reg::request`] with that instance.
    request: Box<DNS_SERVICE_REGISTER_REQUEST>,
}

// SAFETY: `request` and `instance` are written once, before dnsapi sees them, and only
// dnsapi reads them; the rest is Mutex and Condvar.
unsafe impl Send for Reg {}
// SAFETY: as above.
unsafe impl Sync for Reg {}

impl Drop for Reg {
    fn drop(&mut self) {
        // SAFETY: built by DnsServiceConstructInstance; the last share is gone, so
        // dnsapi is done with it.
        unsafe { DnsServiceFreeInstance(self.instance) };
        if let Some(renamed) = lock(&self.renamed).take() {
            // SAFETY: dnsapi's copy, kept for the deregistration, which is over.
            unsafe { DnsServiceFreeInstance(renamed.instance) };
        }
    }
}

/// A registration dnsapi accepted, with the handle to cancel it.
struct Current {
    reg: Arc<Reg>,
    cancel: Box<DNS_SERVICE_CANCEL>,
    since: Instant,
}

// SAFETY: the cancel handle is an opaque token dnsapi accepts from any thread; the rest
// is Send.
unsafe impl Send for Current {}

static REGISTRATIONS: AtomicU64 = AtomicU64::new(0);

impl Reg {
    /// Ask Windows to register `advert` as `full`; `None`, logged, if it refuses at once.
    fn start(advert: &HubAdvert, full: &str, host: &str, tx: &Sender<Cmd>) -> Option<Current> {
        let txt = advert.txt();
        let keys: Vec<Vec<u16>> = txt.iter().map(|(k, _)| wide(k)).collect();
        let values: Vec<Vec<u16>> = txt.iter().map(|(_, v)| wide(v)).collect();
        let key_ptrs: Vec<*const u16> = keys.iter().map(|k| k.as_ptr()).collect();
        let value_ptrs: Vec<*const u16> = values.iter().map(|v| v.as_ptr()).collect();
        let (wname, whost) = (wide(full), wide(host));
        // SAFETY: every pointer is valid for the call, and the function copies what it
        // keeps into the instance it returns.
        let instance = unsafe {
            DnsServiceConstructInstance(
                wname.as_ptr(),
                whost.as_ptr(),
                ptr::null(),
                ptr::null(),
                advert.port,
                0,
                0,
                u32::try_from(txt.len()).unwrap_or(0),
                key_ptrs.as_ptr(),
                value_ptrs.as_ptr(),
            )
        };
        if instance.is_null() {
            log::warn!("Windows DNS-SD: cannot build a registration for {full}");
            return None;
        }
        let reg = Arc::new_cyclic(|me: &std::sync::Weak<Reg>| Reg {
            id: REGISTRATIONS.fetch_add(1, Ordering::SeqCst),
            tx: Mutex::new(tx.clone()),
            request: DNS_SERVICE_REGISTER_REQUEST {
                Version: DNS_QUERY_REQUEST_VERSION1,
                InterfaceIndex: 0,
                pServiceInstance: instance,
                pRegisterCompletionCallback: Some(on_register),
                pQueryContext: me.as_ptr().cast_mut().cast(),
                hCredentials: ptr::null_mut(),
                unicastEnabled: 0,
            },
            instance,
            state: Mutex::default(),
            withdrawn: Condvar::new(),
            completion: Completion::default(),
            renamed: Mutex::new(None),
        });
        let mut cancel = Box::new(DNS_SERVICE_CANCEL { reserved: ptr::null_mut() });
        lock(&reg.state).pending = 1;
        let raw = give(&reg);
        // SAFETY: the request and its instance live in `reg`, whose share dnsapi holds
        // until the callbacks; `cancel` is boxed and kept with it.
        let status = unsafe { DnsServiceRegister(&reg.request, &mut *cancel) };
        if status != PENDING {
            lock(&reg.state).pending = 0;
            // SAFETY: not accepted, so no callback takes it.
            unsafe { take(raw) };
            log::warn!("Windows DNS-SD: registering {full} refused: {status}");
            return None;
        }
        Some(Current { reg, cancel, since: Instant::now() })
    }
}

impl Current {
    /// Withdraw it, and wait a little for Windows to say it has: a goodbye for a
    /// registration it answered, a cancel for one it did not. True if a goodbye went out.
    fn withdraw(self) -> bool {
        lock(&self.reg.state).withdrawing = true;
        if self.reg.completion.begin_cancel() {
            // SAFETY: the handle DnsServiceRegister filled in, for a registration not yet
            // answered (`begin_cancel`); its callback (cancelled) takes its share back.
            let status = unsafe { DnsServiceRegisterCancel(&*self.cancel) };
            self.reg.completion.end_cancel();
            if status != 0 {
                log::debug!("DnsServiceRegisterCancel: {status}");
            }
        }
        // Its answer, whichever it was.
        self.settle();
        // A success that came as it was cancelled stands, and needs its goodbye too.
        let registered = lock(&self.reg.state).registered;
        if registered {
            self.deregister();
            self.settle();
        }
        registered
    }

    fn deregister(&self) {
        let reg = &self.reg;
        {
            let mut state = lock(&reg.state);
            state.pending = state.pending.saturating_add(1);
        }
        let request: *const DNS_SERVICE_REGISTER_REQUEST =
            lock(&reg.renamed).as_ref().map_or(&raw const reg.request, |r| &raw const *r.request);
        let raw = give(reg);
        // SAFETY: the request of the instance Windows registered, alive in `reg` (its
        // `renamed` is only ever set once, before this) while dnsapi holds a share.
        let status = unsafe { DnsServiceDeRegister(request, ptr::null_mut()) };
        if status != PENDING {
            let mut state = lock(&reg.state);
            state.pending = state.pending.saturating_sub(1);
            drop(state);
            // SAFETY: not accepted, so no callback takes it.
            unsafe { take(raw) };
            log::debug!("DnsServiceDeRegister: {status}");
        }
    }

    /// Wait a little for every call dnsapi accepted to call back.
    fn settle(&self) {
        let state = lock(&self.reg.state);
        let (state, _) = self
            .reg
            .withdrawn
            .wait_timeout_while(state, DEREGISTER_WAIT, |s| s.pending > 0)
            .unwrap_or_else(PoisonError::into_inner);
        if state.pending > 0 {
            log::debug!("Windows DNS-SD: a withdrawal not acknowledged in time");
        }
    }
}

/// # Safety
/// Called by dnsapi with a registration's context.
unsafe extern "system" fn on_register(status: u32, context: *const c_void, instance: *const DNS_SERVICE_INSTANCE) {
    let raw = context.cast::<Reg>();
    {
        // SAFETY: the share is held until the end of this callback.
        let reg = unsafe { &*raw };
        reg.completion.called_back();
        let name = if instance.is_null() {
            String::new()
        } else {
            // SAFETY: an instance dnsapi filled in.
            unsafe { from_wide((*instance).pszInstanceName) }
        };
        let mut state = lock(&reg.state);
        state.pending = state.pending.saturating_sub(1);
        let first = !state.answered;
        // SAFETY: our own instance's name.
        let asked = unsafe { from_wide((*reg.instance).pszInstanceName) };
        let renamed = first && status == 0 && !name.is_empty() && !name.eq_ignore_ascii_case(&asked);
        if renamed && instance != reg.instance.cast_const() {
            let request =
                Box::new(DNS_SERVICE_REGISTER_REQUEST { pServiceInstance: instance.cast_mut(), ..reg.request });
            *lock(&reg.renamed) = Some(Renamed { instance: instance.cast_mut(), request });
        } else if !instance.is_null() && instance != reg.instance.cast_const() {
            // SAFETY: a copy dnsapi handed over, not ours; freed once.
            unsafe { DnsServiceFreeInstance(instance) };
        }
        if first {
            state.answered = true;
            state.registered = status == 0;
        }
        if first && !state.withdrawing {
            let _ = lock(&reg.tx).send(Cmd::Answered { id: reg.id, status, name });
        }
        if state.pending == 0 {
            reg.withdrawn.notify_all();
        }
    }
    // SAFETY: one share per accepted call, one callback each.
    unsafe { take(raw) };
}

/// Advertises a hub through the DNS Client service, from a thread of its own, so an
/// update or a retry never blocks the caller. Dropping it withdraws the registration
/// (a goodbye) and waits up to [`DEREGISTER_WAIT`] for that.
pub struct DnsSdAdvertiser {
    tx: Sender<Cmd>,
    names: Arc<Mutex<AdvertisedNames>>,
    thread: Option<JoinHandle<()>>,
}

impl std::fmt::Debug for DnsSdAdvertiser {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DnsSdAdvertiser").field("names", &*lock(&self.names)).finish_non_exhaustive()
    }
}

impl DnsSdAdvertiser {
    /// Register `advert` on every interface.
    pub fn start(advert: &HubAdvert) -> Result<Self, DiscoveryError> {
        Self::start_on(advert, AdvertiseScope::AllInterfaces)
    }

    /// Register `advert`. Windows registers on every interface or none, so
    /// [`AdvertiseScope::LoopbackOnly`] (tests) is refused. Returns at once: the
    /// registration and any retries run on the advertiser's thread.
    pub fn start_on(advert: &HubAdvert, scope: AdvertiseScope) -> Result<Self, DiscoveryError> {
        if scope == AdvertiseScope::LoopbackOnly {
            return Err(DiscoveryError::Network("Windows DNS-SD cannot advertise on loopback alone".to_owned()));
        }
        let host = host_name();
        let names = Arc::new(Mutex::new(AdvertisedNames { instance: label(&advert.name), host: host.clone() }));
        let (tx, rx) = mpsc::channel();
        let worker = Worker {
            tx: tx.clone(),
            names: Arc::clone(&names),
            host,
            want: advert.clone(),
            current: None,
            retry_at: None,
            retry_wait: RETRY_FIRST,
            settled_at: None,
            check_at: None,
            local_suffix: 0,
            registered_label: String::new(),
        };
        let thread = thread::Builder::new()
            .name("dspi-dnssd-advertise".into())
            .spawn(move || worker.run(&rx))
            .map_err(|e| DiscoveryError::Thread(e.to_string()))?;
        Ok(Self { tx, names, thread: Some(thread) })
    }

    /// The names held now. Never blocks.
    pub fn names(&mut self) -> AdvertisedNames {
        lock(&self.names).clone()
    }

    /// Announce changed data or a new name: a goodbye and a new registration, on the
    /// advertiser's thread. [`Self::names`] has the name asked for at once.
    pub fn update(&mut self, advert: &HubAdvert) -> Result<(), DiscoveryError> {
        // At once, as the embedded responder does: the hub judges a conflict by
        // comparing what it asked for with [`Self::names`], and the thread may be busy
        // with a goodbye for a second or two.
        lock(&self.names).instance = label(&advert.name);
        self.tx
            .send(Cmd::Update(advert.clone()))
            .map_err(|_| DiscoveryError::Network("the Windows DNS-SD advertiser has stopped".to_owned()))
    }
}

impl Drop for DnsSdAdvertiser {
    fn drop(&mut self) {
        let _ = self.tx.send(Cmd::Stop);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// The advertiser's thread.
struct Worker {
    tx: Sender<Cmd>,
    names: Arc<Mutex<AdvertisedNames>>,
    host: String,
    /// What should be registered.
    want: HubAdvert,
    current: Option<Current>,
    /// When to register again after a failure.
    retry_at: Option<Instant>,
    retry_wait: Duration,
    /// When the network will have settled after a change.
    settled_at: Option<Instant>,
    /// When to check that Windows still holds the registration.
    check_at: Option<Instant>,
    /// Clashes on this machine so far (0: none): `Studio (2)` after the first.
    local_suffix: u32,
    /// The label last asked for.
    registered_label: String,
}

impl Worker {
    fn run(mut self, rx: &Receiver<Cmd>) {
        let _watch = NetWatch::start(&self.tx);
        self.register();
        loop {
            let unanswered = self
                .current
                .as_ref()
                .filter(|c| !lock(&c.reg.state).answered)
                .and_then(|c| c.since.checked_add(REGISTER_TIMEOUT));
            let wake = [unanswered, self.retry_at, self.settled_at, self.check_at].into_iter().flatten().min();
            let cmd = match wake {
                Some(at) => rx.recv_timeout(at.saturating_duration_since(Instant::now())),
                None => rx.recv().map_err(|_| RecvTimeoutError::Disconnected),
            };
            match cmd {
                Ok(Cmd::Stop) | Err(RecvTimeoutError::Disconnected) => break,
                Ok(Cmd::Update(advert)) => {
                    if advert != self.want {
                        if advert.name != self.want.name {
                            self.local_suffix = 0;
                        }
                        // The hub taking the name this already registered under (after
                        // a clash on this machine): nothing to send again.
                        let same = self.current.is_some()
                            && self.label_for(&advert) == self.registered_label
                            && advert.port == self.want.port
                            && advert.txt() == self.want.txt();
                        self.want = advert;
                        if !same {
                            self.retry_wait = RETRY_FIRST;
                            self.register();
                        }
                    }
                }
                Ok(Cmd::Answered { id, status, name }) => self.answered(id, status, &name),
                Ok(Cmd::NetworkChanged) => self.settled_at = Instant::now().checked_add(NETWORK_SETTLE),
                Err(RecvTimeoutError::Timeout) => {
                    let now = Instant::now();
                    if unanswered.is_some_and(|at| at <= now) {
                        log::warn!("Windows DNS-SD: no answer to registering {}; trying again", self.want.name);
                        self.failed();
                    } else if self.retry_at.is_some_and(|at| at <= now) {
                        self.register();
                    } else if self.settled_at.is_some_and(|at| at <= now) {
                        self.settled_at = None;
                        log::info!("Windows DNS-SD: the network changed; registering {} again", self.want.name);
                        self.register();
                    } else if self.check_at.is_some_and(|at| at <= now) {
                        self.check_at = None;
                        self.check();
                    }
                }
            }
        }
        if let Some(current) = self.current.take() {
            let _ = current.withdraw();
        }
    }

    /// Withdraw what stands and register what is wanted.
    fn register(&mut self) {
        if self.current.take().is_some_and(Current::withdraw) {
            thread::sleep(REREGISTER_GAP);
        }
        self.retry_at = None;
        let label = self.label_for(&self.want);
        lock(&self.names).instance.clone_from(&label);
        let full = format!("{label}.{QUERY}");
        self.registered_label.clone_from(&label);
        match Reg::start(&self.want, &full, &self.host, &self.tx) {
            Some(current) => self.current = Some(current),
            None => self.schedule_retry(),
        }
    }

    /// Resolve our own instance: Windows answers for its registrations at once, and
    /// for nothing it has dropped. Gone, or someone else's: register again.
    /// The instance label to register `advert` under: its name, or after a clash on
    /// this machine the name with a suffix, as Bonjour does (`Studio (2)`).
    fn label_for(&self, advert: &HubAdvert) -> String {
        let base = label(&advert.name);
        if self.local_suffix == 0 {
            return base;
        }
        let suffix = format!(" ({})", self.local_suffix.saturating_add(1));
        let mut cut = base.len().min(63usize.saturating_sub(suffix.len()));
        while !base.is_char_boundary(cut) {
            cut = cut.saturating_sub(1);
        }
        format!("{}{suffix}", base.get(..cut).unwrap_or_default().trim_end())
    }

    fn check(&mut self) {
        if self.current.is_none() {
            return;
        }
        let full = format!("{}.{QUERY}", lock(&self.names).instance);
        let (tx, rx) = mpsc::channel();
        match resolve(&full, &tx, &rx) {
            Ok((record, _))
                if record.port == self.want.port
                    && super::txt_value(&record.txt, "hid") == Some(self.want.hub_id.as_str()) =>
            {
                self.check_at = Instant::now().checked_add(SELF_CHECK);
            }
            // Windows lets a second registration on this machine take a name another
            // process holds, and answers for the first (measured): take another name.
            Ok(_) if self.local_suffix < MAX_LOCAL_SUFFIX => {
                self.local_suffix = self.local_suffix.saturating_add(1);
                log::info!(
                    "Windows DNS-SD: {} is taken on this machine; now {}",
                    self.want.name,
                    self.label_for(&self.want)
                );
                self.register();
            }
            Ok(_) => self.check_at = Instant::now().checked_add(SELF_CHECK),
            Err(_) => {
                log::warn!("Windows DNS-SD: the registration of {} is gone; registering again", self.want.name);
                self.register();
            }
        }
    }

    fn answered(&mut self, id: u64, status: u32, name: &str) {
        if self.current.as_ref().is_none_or(|c| c.reg.id != id) {
            return;
        }
        if status != 0 {
            log::warn!("Windows DNS-SD: registering {} failed ({status}); trying again", self.want.name);
            self.failed();
            return;
        }
        self.retry_wait = RETRY_FIRST;
        self.check_at = Instant::now().checked_add(FIRST_CHECK);
        if !name.is_empty() {
            let held = instance_label(&rooted(name.to_owned()));
            let mut names = lock(&self.names);
            if held != names.instance {
                log::info!("Windows DNS-SD: registered as {held}");
            }
            names.instance = held;
        }
        log::debug!("Windows DNS-SD: registered {}", self.want.name);
    }

    fn failed(&mut self) {
        if let Some(current) = self.current.take() {
            let _ = current.withdraw();
        }
        self.schedule_retry();
    }

    fn schedule_retry(&mut self) {
        self.retry_at = Instant::now().checked_add(self.retry_wait);
        self.retry_wait = self.retry_wait.saturating_mul(2).min(RETRY_MAX);
    }
}

#[cfg(test)]
mod tests_wp_disc2 {
    use super::*;

    #[test]
    fn wp_disc2_an_instance_label_has_no_dot_and_fits_a_dns_label() {
        assert_eq!(label(" Studio 1.2\n "), "Studio 1-2");
        let long = label(&"å".repeat(40));
        assert!(long.len() <= 63 && long.chars().all(|c| c == 'å'));
    }

    #[test]
    fn wp_disc2_windows_names_get_the_roots_dot() {
        assert_eq!(rooted("a._dspi._tcp.local".into()), "a._dspi._tcp.local.");
        assert_eq!(rooted("a._dspi._tcp.local.".into()), "a._dspi._tcp.local.");
    }
}
