//! The `uconsole web` pinning proxy (WP SECWEB; PLAN-SEC 4.6, way 2; OD-SECl (2)).
//!
//! A page of the web console served by `uconsole web` (`http://localhost:11916`, a
//! secure context for WebUSB) cannot pin a hub's key, and an `http` page cannot reach
//! another hub at all (the hub's `Origin` rule). So it opens
//! `ws://localhost:11916/hub/<target>/dspi/v1`, and this process relays that
//! WebSocket, message for message, to the remote hub over TLS checked against the key
//! it pinned there: the browser gets real pinning and no certificate click-through.
//!
//! - **Where.** Only on a hub bound to loopback with its web pages on (`uconsole web`,
//!   which is `uconsole hub --local`): [`enabled`]. Every request must come from a
//!   loopback peer, name a loopback `Host` on the hub's port, and an upgrade must carry
//!   the page's own loopback `Origin` (the server's proxy block). This process holds
//!   every hub's pin, so it is never reachable from a LAN address.
//! - **`<target>`**: a hub id this proxy has pinned (its last address is dialled), or a
//!   `host:port` address, percent-encoded as a path segment.
//! - **The pin rule**, the core's (section 4.3) applied to what the proxy can see:
//!   TLS is tried first. A hub id pinned here is reached over TLS only, with the key
//!   checked before the page's first byte goes out: another key is
//!   [`Refusal::HubKeyChanged`] unless the hello carries a valid `key_next` signed by
//!   the pinned key (a rotation), which re-pins. A TLS hub never pinned is pinned on
//!   first use. A hub without TLS (an upstream hub) is relayed unencrypted, and
//!   [`ProxyStatus::encrypted`] says so (the page's red bar), unless its id or its
//!   address is pinned here: that is a downgrade, [`Refusal::TlsRefused`].
//! - **Before any secret.** The proxy dials (a known key is checked in the TLS handshake)
//!   before it answers the page's upgrade; then it passes on the page's hello (the
//!   first message a client sends, with nothing secret in it), judges the hub's, and
//!   only then relays. On a refusal the page's socket is closed with 4000 and the code
//!   as the reason, before its token (sent after the hub's hello) could go out. The page reads why from
//!   `GET /hub/<target>/dspi/v1/security` ([`ProxyStatus`]: public facts only, the
//!   fingerprint and whether the link is encrypted, never a token).
//! - **Forgetting** a pin (the hub was replaced, a key change the user knows about):
//!   `POST /hub/<target>/dspi/v1/forget`, from the page's own Origin only.
//!
//! **On disk**: `<store>/proxy-pins.json`, 0600 in a 0700 folder: hub id, last address,
//! pin (base64url SPKI hash). Public facts, but the list of hubs is the user's.

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use dspi_link::LinkError;
use dspi_link::credential::{pin_from_b64url, pin_to_b64url};
use dspi_link::messages::KeyNext;
use dspi_link::socket::{
    TcpConnector, TlsTrust, WsConnector, WsMessage, WsReceiver, WsRecv, WsSender,
};
use dspi_link::tls::{self as link_tls, PinMode, SpkiHash};
use serde::{Deserialize, Serialize};

/// The proxy's routes start here (beside the upstream hub page's `/hub/` assets, which
/// never end in `/dspi/v1`).
pub const PROXY_PREFIX: &str = "/hub/";
/// The pins' file, in the hub store's folder.
pub const PINS_FILE: &str = "proxy-pins.json";
/// Longest `<target>`, encoded.
const MAX_TARGET: usize = 300;
/// Most hubs pinned (the file and the status table).
const MAX_HUBS: usize = 256;
const MAX_PINS_FILE: u64 = 256 * 1024;
/// The dial: name, TCP, TLS and the WebSocket upgrade.
const DIAL_TIMEOUT: Duration = Duration::from_secs(8);
const HANDSHAKE_WITHIN: Duration = Duration::from_secs(5);
/// The hub's hello, after the upgrade.
const HELLO_WITHIN: Duration = Duration::from_secs(5);
/// The hub's `/dspi/v1/info`, read for its TLS mode.
const INFO_WITHIN: Duration = Duration::from_secs(3);
const MAX_INFO: usize = 64 * 1024;
const SUBPROTOCOL: &str = "dspi-link-1";

/// Whether a hub serves the proxy: bound to loopback, with its web pages.
pub fn enabled(bind: std::net::IpAddr, web: bool) -> bool {
    web && bind.is_loopback()
}

/// What a proxy path asks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Route<'a> {
    /// `/hub/<target>/dspi/v1`: the relayed WebSocket.
    Link(&'a str),
    /// `/hub/<target>/dspi/v1/security`: [`ProxyStatus`] as JSON.
    Security(&'a str),
    /// `/hub/<target>/dspi/v1/forget` (POST): drop the target's pin.
    Forget(&'a str),
}

/// The proxy route `path` names (no query), or `None`: not the proxy's.
pub fn route(path: &str) -> Option<Route<'_>> {
    let rest = path.strip_prefix(PROXY_PREFIX)?;
    let (target, tail) = rest.split_once('/')?;
    if target.is_empty() || target.len() > MAX_TARGET {
        return None;
    }
    match tail {
        "dspi/v1" => Some(Route::Link(target)),
        "dspi/v1/security" => Some(Route::Security(target)),
        "dspi/v1/forget" => Some(Route::Forget(target)),
        _ => None,
    }
}

/// A `<target>` percent-decoded, or `None` when it is not a hub id or an address
/// (only letters, digits and `.-_:[]` survive).
pub fn decode_target(raw: &str) -> Option<String> {
    let bytes = raw.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0usize;
    while let Some(&b) = bytes.get(i) {
        if b == b'%' {
            let hex = raw.get(i.checked_add(1)?..i.checked_add(3)?)?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i = i.checked_add(3)?;
        } else {
            out.push(b);
            i = i.checked_add(1)?;
        }
    }
    let text = String::from_utf8(out).ok()?;
    let plain = !text.is_empty()
        && text.bytes().all(|b| {
            b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_' | b':' | b'[' | b']')
        });
    plain.then_some(text)
}

/// Why the proxy did not connect a page to its hub. [`Refusal::code`] is the core's
/// error code for the same case where there is one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// Not a pinned hub id, nor an address.
    UnknownTarget,
    /// The hub presented another key than the one pinned, with no valid rotation.
    HubKeyChanged,
    /// A pinned hub (or one at a pinned address) did not do TLS: never plaintext.
    TlsRefused,
    /// Another hub answers at a pinned address.
    WrongHub,
    /// Not reached (name, TCP, the upgrade).
    Unreachable(String),
    /// No proper hello.
    Protocol,
}

impl Refusal {
    /// A stable code for the page.
    pub fn code(&self) -> &'static str {
        match self {
            Refusal::UnknownTarget => "unknown_target",
            Refusal::HubKeyChanged => "hub_key_changed",
            Refusal::TlsRefused => "tls_refused",
            Refusal::WrongHub => "wrong_hub",
            Refusal::Unreachable(_) => "io",
            Refusal::Protocol => "protocol",
        }
    }

    /// Words for the page and the 502 body. No secret is ever in them.
    pub fn message(&self) -> String {
        match self {
            Refusal::UnknownTarget => "Not a hub this console knows, nor an address.".into(),
            Refusal::HubKeyChanged => "This hub presented a different key than the one this console trusts. \
                 Someone may be posing as it. If you replaced or reset the hub, remove it and add it again."
                .into(),
            // -- WP PINTLS -- removing the hub is the only way back to plaintext; say so
            Refusal::TlsRefused => "This hub did not accept an encrypted connection, although it did before. \
                 The console does not fall back to an unencrypted one. Removing the hub from this console \
                 is the only way to connect to it without encryption."
                .into(),
            Refusal::WrongHub => "Another hub answers at this address than the one this console trusts there. \
                 If the hub moved or was replaced, remove it and add it again."
                .into(),
            Refusal::Unreachable(why) => format!("The hub was not reached: {why}"),
            Refusal::Protocol => "The hub did not answer as a DSPi Link hub.".into(),
        }
    }
}

/// What the page is told about its link through the proxy
/// (`GET /hub/<target>/dspi/v1/security`). Public facts only.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProxyStatus {
    /// The hub's id from its hello ("" before one).
    pub hub_id: String,
    /// The relay to the hub runs over TLS with a pinned key.
    pub encrypted: bool,
    /// The pinned key's fingerprint (`SHA256:K7Q2-M9XD-4TNB-0ZRA`), "" without TLS.
    pub hub_key: String,
    /// This connection pinned the key (first use, or a verified rotation).
    pub hub_key_new: bool,
    /// Why the last attempt was refused ([`Refusal::code`]), "" when it connected.
    pub refused: String,
    /// [`Refusal::message`], "" when it connected.
    pub message: String,
    /// The hub's `/dspi/v1/info` said it serves TLS (`None`: not read).
    pub tls: Option<bool>,
    /// ... and that its upstream compatibility mode is on.
    pub upstream_compat: Option<bool>,
}

/// A hub the proxy has pinned.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct PinRecord {
    hid: String,
    /// `host:port`, as last dialled.
    address: String,
    /// The SPKI hash, base64url.
    pin: String,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct PinFile {
    #[serde(default)]
    hubs: Vec<PinRecord>,
}

/// A hub dialled (TLS judged by its pin where one is kept), waiting for the page's
/// hello: [`Proxy::judge`] sends it and judges the hub's.
pub struct Dialled {
    /// To the hub.
    pub sender: Box<dyn WsSender>,
    /// From the hub.
    pub receiver: Box<dyn WsReceiver>,
    target: String,
    address: String,
    /// The pinned record the target or the address named.
    by_hid: Option<PinRecord>,
    at_address: Option<PinRecord>,
    /// The key the hub presented over TLS; `None`: no TLS.
    presented: Option<SpkiHash>,
}

/// A hub judged by its hello: relay from here, the hello first.
pub struct Judged {
    /// To the hub.
    pub sender: Box<dyn WsSender>,
    /// From the hub, past its hello.
    pub receiver: Box<dyn WsReceiver>,
    /// The hub's hello, for the page.
    pub hello: String,
}

impl std::fmt::Debug for Dialled {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Dialled").finish_non_exhaustive()
    }
}

/// The proxy's state: the pins and each target's last status. Locks are leaves.
pub struct Proxy {
    path: PathBuf,
    pins: Mutex<Vec<PinRecord>>,
    status: Mutex<BTreeMap<String, ProxyStatus>>,
    connector: TcpConnector,
}

impl std::fmt::Debug for Proxy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Proxy")
            .field("path", &self.path)
            .finish_non_exhaustive()
    }
}

/// `now + d`, saturating.
fn after(d: Duration) -> Instant {
    let now = Instant::now();
    now.checked_add(d).unwrap_or(now)
}

fn guard<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

fn same_pin(a: &str, b: &SpkiHash) -> bool {
    pin_from_b64url(a).is_some_and(|p| SpkiHash(p).ct_eq(b))
}

impl Proxy {
    /// The proxy of a hub whose store is `store_dir`. A missing or unreadable pins file
    /// is an empty one (a damaged file is set aside, as the store does with its own).
    pub fn new(store_dir: PathBuf) -> Proxy {
        let path = store_dir.join(PINS_FILE);
        let pins = match crate::tls_identity::read_limited(&path, MAX_PINS_FILE) {
            Ok(bytes) => match serde_json::from_slice::<PinFile>(&bytes) {
                Ok(f) => f.hubs.into_iter().take(MAX_HUBS).collect(),
                Err(_) => {
                    let aside = path.with_extension(format!(
                        "json.bad-{}",
                        crate::store::unix_seconds(std::time::SystemTime::now())
                    ));
                    log::warn!(
                        "proxy: {} is damaged; set aside, no hub is pinned",
                        path.display()
                    );
                    let _ = std::fs::rename(&path, aside);
                    Vec::new()
                }
            },
            Err(_) => Vec::new(),
        };
        Proxy {
            path,
            pins: Mutex::new(pins),
            status: Mutex::new(BTreeMap::new()),
            connector: TcpConnector {
                connect_timeout: DIAL_TIMEOUT,
            },
        }
    }

    /// The last status for `target` (decoded), if it was ever dialled.
    pub fn status(&self, target: &str) -> Option<ProxyStatus> {
        guard(&self.status).get(target).cloned()
    }

    /// Forget the pin of `target` (a hub id, or every hub last at that address).
    /// Whether one was forgotten.
    pub fn forget(&self, target: &str) -> bool {
        let mut pins = guard(&self.pins);
        let before = pins.len();
        pins.retain(|r| {
            !r.hid.eq_ignore_ascii_case(target) && !r.address.eq_ignore_ascii_case(target)
        });
        let gone = pins.len() != before;
        if gone {
            self.save(&pins);
        }
        guard(&self.status).remove(target);
        gone
    }

    /// The pinned hubs' ids and fingerprints (for tests and diagnostics; public facts).
    pub fn pinned(&self) -> Vec<(String, String)> {
        guard(&self.pins)
            .iter()
            .filter_map(|r| Some((r.hid.clone(), SpkiHash(pin_from_b64url(&r.pin)?).display())))
            .collect()
    }

    fn save(&self, pins: &[PinRecord]) {
        let file = PinFile {
            hubs: pins.to_vec(),
        };
        let Ok(bytes) = serde_json::to_vec_pretty(&file) else {
            return;
        };
        let Some(dir) = self.path.parent() else {
            return;
        };
        if let Err(e) = crate::tls_identity::create_private_dir_all(dir) {
            log::warn!("proxy: cannot create {}: {e}", dir.display());
            return;
        }
        if let Err(e) = crate::tls_identity::write_private(dir, PINS_FILE, &bytes) {
            log::warn!("proxy: the pins were not saved: {e}");
        }
    }

    fn record(&self, target: &str, status: ProxyStatus) {
        let mut all = guard(&self.status);
        if all.len() >= MAX_HUBS && !all.contains_key(target) {
            all.clear();
        }
        all.insert(target.to_owned(), status);
    }

    fn refused(&self, target: &str, r: Refusal) -> Refusal {
        self.record(
            target,
            ProxyStatus {
                refused: r.code().into(),
                message: r.message(),
                ..ProxyStatus::default()
            },
        );
        r
    }

    /// Dial `target` (decoded): TLS first, checked against the pin kept for the target's
    /// hub id or address; never plaintext for one of those (module docs). A refusal is
    /// also kept as the target's [`ProxyStatus`].
    pub fn dial(&self, target: &str) -> Result<Dialled, Refusal> {
        self.dial_inner(target).map_err(|r| self.refused(target, r))
    }

    fn dial_inner(&self, target: &str) -> Result<Dialled, Refusal> {
        // A pinned hub id, else an address.
        let by_hid = guard(&self.pins)
            .iter()
            .find(|r| r.hid.eq_ignore_ascii_case(target))
            .cloned();
        let address = match &by_hid {
            Some(r) => r.address.clone(),
            None => {
                let (host, port) =
                    dspi_link::address::parse_address(target).ok_or(Refusal::UnknownTarget)?;
                if host.contains(':') && !host.starts_with('[') {
                    format!("[{host}]:{port}")
                } else {
                    format!("{host}:{port}")
                }
            }
        };
        let at_address = guard(&self.pins)
            .iter()
            .find(|r| r.address.eq_ignore_ascii_case(&address))
            .cloned();
        let known = by_hid.as_ref().or(at_address.as_ref());
        let url = format!("ws://{address}/dspi/v1");
        let trust = known
            .and_then(|r| pin_from_b64url(&r.pin))
            .map_or(TlsTrust::FirstUse, TlsTrust::Pinned);
        let ((sender, receiver), presented) =
            match self
                .connector
                .connect_tls(&url, SUBPROTOCOL, trust, Some(HANDSHAKE_WITHIN))
            {
                Ok((pair, presented)) => (pair, Some(SpkiHash(presented))),
                Err(LinkError::TlsRefused { .. }) if known.is_some() => {
                    return Err(Refusal::TlsRefused);
                }
                Err(LinkError::TlsRefused { .. }) => (
                    self.connector
                        .connect(&url, SUBPROTOCOL)
                        .map_err(|e| Refusal::Unreachable(e.to_string()))?,
                    None,
                ),
                Err(e) => return Err(Refusal::Unreachable(e.to_string())),
            };
        Ok(Dialled {
            sender,
            receiver,
            target: target.to_owned(),
            address,
            by_hid,
            at_address,
            presented,
        })
    }

    /// Pass the page's hello (`client_hello`, the first thing a client sends: no secret
    /// in it) to the dialled hub and judge the hub's: its id, and its key against the
    /// pin (a rotation needs a valid `key_next`). Pins a new key (first use, rotation).
    /// The outcome is kept as the target's [`ProxyStatus`].
    pub fn judge(&self, d: Dialled, client_hello: String) -> Result<Judged, Refusal> {
        let target = d.target.clone();
        match self.judge_inner(d, client_hello) {
            Ok((j, status)) => {
                self.record(&target, status);
                Ok(j)
            }
            Err(r) => Err(self.refused(&target, r)),
        }
    }

    fn judge_inner(
        &self,
        d: Dialled,
        client_hello: String,
    ) -> Result<(Judged, ProxyStatus), Refusal> {
        let Dialled {
            sender,
            mut receiver,
            address,
            by_hid,
            at_address,
            presented,
            ..
        } = d;
        let refuse = |sender: &dyn WsSender, r: Refusal| {
            sender.close(1000);
            Err(r)
        };
        if sender.send(WsMessage::Text(client_hello)).is_err() {
            return refuse(
                sender.as_ref(),
                Refusal::Unreachable("the hub went away".into()),
            );
        }
        let Some(hello) = read_hello(receiver.as_mut()) else {
            return refuse(sender.as_ref(), Refusal::Protocol);
        };
        let Some((hid, key_next)) = hello_facts(&hello) else {
            return refuse(sender.as_ref(), Refusal::Protocol);
        };
        // The hub the target named, or that the address was pinned for, must be this one.
        if let Some(r) = by_hid.as_ref().or(at_address.as_ref())
            && !r.hid.eq_ignore_ascii_case(&hid)
        {
            return refuse(sender.as_ref(), Refusal::WrongHub);
        }
        let pinned = guard(&self.pins)
            .iter()
            .find(|r| r.hid.eq_ignore_ascii_case(&hid))
            .cloned();
        let Some(presented) = presented else {
            // Unencrypted: only a hub never pinned here (an upstream hub).
            if pinned.is_some() {
                return refuse(sender.as_ref(), Refusal::TlsRefused);
            }
            let info = read_info(&address, None);
            let status = ProxyStatus {
                hub_id: hid,
                encrypted: false,
                tls: info.map(|i| i.0),
                upstream_compat: info.map(|i| i.1),
                ..ProxyStatus::default()
            };
            return Ok((
                Judged {
                    sender,
                    receiver,
                    hello,
                },
                status,
            ));
        };
        let key_new = match &pinned {
            Some(r) if same_pin(&r.pin, &presented) => false,
            Some(r) => {
                let valid = pin_from_b64url(&r.pin)
                    .map(SpkiHash)
                    .is_some_and(|old| rotation_valid(&old, &hid, &presented, key_next.as_ref()));
                if !valid {
                    log::warn!("proxy: hub {hid} presented another key; refused");
                    return refuse(sender.as_ref(), Refusal::HubKeyChanged);
                }
                true
            }
            None => true,
        };
        // Pin (first use, a rotation) and keep the address it was reached at.
        {
            let mut pins = guard(&self.pins);
            pins.retain(|r| !r.hid.eq_ignore_ascii_case(&hid));
            pins.push(PinRecord {
                hid: hid.clone(),
                address: address.clone(),
                pin: pin_to_b64url(&presented.0),
            });
            while pins.len() > MAX_HUBS {
                pins.remove(0);
            }
            self.save(&pins);
        }
        let info = read_info(&address, Some(presented));
        let status = ProxyStatus {
            hub_id: hid,
            encrypted: true,
            hub_key: presented.display(),
            hub_key_new: key_new,
            tls: info.map(|i| i.0),
            upstream_compat: info.map(|i| i.1),
            ..ProxyStatus::default()
        };
        Ok((
            Judged {
                sender,
                receiver,
                hello,
            },
            status,
        ))
    }
}

/// The hub's first message, its hello (spec 7.1), within [`HELLO_WITHIN`].
fn read_hello(receiver: &mut dyn WsReceiver) -> Option<String> {
    let by = after(HELLO_WITHIN);
    while Instant::now() < by {
        match receiver.recv_within(Duration::from_millis(250)) {
            WsRecv::Message(WsMessage::Text(t)) => return Some(t),
            WsRecv::Message(WsMessage::Binary(_)) | WsRecv::Closed { .. } => return None,
            WsRecv::Alive | WsRecv::Idle => {}
        }
    }
    None
}

/// The hello's hub id and its `key_next`.
fn hello_facts(hello: &str) -> Option<(String, Option<KeyNext>)> {
    let v: serde_json::Value = serde_json::from_str(hello).ok()?;
    let hub = v.get("hub")?;
    let hid = hub.get("id")?.as_str()?.trim().to_owned();
    if hid.is_empty() || hid.len() > 128 {
        return None;
    }
    let key_next = hub
        .get("key_next")
        .and_then(|k| serde_json::from_value::<KeyNext>(k.clone()).ok());
    Some((hid, key_next))
}

/// Whether `key_next` moves `old` to `presented` for `hid`, signed by `old`.
fn rotation_valid(
    old: &SpkiHash,
    hid: &str,
    presented: &SpkiHash,
    key_next: Option<&KeyNext>,
) -> bool {
    let Some(k) = key_next else {
        return false;
    };
    let (Some(new), Ok(prev), Ok(sig)) = (
        SpkiHash::from_b64url(&k.spki),
        data_encoding::BASE64URL_NOPAD.decode(k.prev_spki.trim_end_matches('=').as_bytes()),
        data_encoding::BASE64URL_NOPAD.decode(k.sig.trim_end_matches('=').as_bytes()),
    ) else {
        return false;
    };
    new.ct_eq(presented) && link_tls::verify_rotation(old, &prev, hid, &new, &sig).is_ok()
}

/// The hub's `/dspi/v1/info`: `(tls, upstream_compat)`, over TLS pinned to `pin` when
/// given. `None` when it cannot be read; bounded in bytes and time.
fn read_info(address: &str, pin: Option<SpkiHash>) -> Option<(bool, bool)> {
    let (host, port) = dspi_link::address::parse_address(address)?;
    let addr = std::net::ToSocketAddrs::to_socket_addrs(&(
        host.trim_matches(|c| c == '[' || c == ']'),
        port,
    ))
    .ok()?
    .next()?;
    let tcp = std::net::TcpStream::connect_timeout(&addr, INFO_WITHIN).ok()?;
    let deadline = after(INFO_WITHIN);
    tcp.set_read_timeout(Some(INFO_WITHIN)).ok()?;
    tcp.set_write_timeout(Some(INFO_WITHIN)).ok()?;
    let request =
        format!("GET /dspi/v1/info HTTP/1.1\r\nHost: {address}\r\nConnection: close\r\n\r\n");
    let mut body = Vec::new();
    match pin {
        Some(pin) => {
            let (mut tls, _) =
                link_tls::connect(tcp, &host, PinMode::Pinned(pin), deadline).ok()?;
            tls.write_all(request.as_bytes()).ok()?;
            tls.flush().ok()?;
            read_bounded(&mut tls, &mut body, deadline);
        }
        None => {
            let mut tcp = tcp;
            tcp.write_all(request.as_bytes()).ok()?;
            read_bounded(&mut tcp, &mut body, deadline);
        }
    }
    let text = String::from_utf8_lossy(&body);
    let (_, json) = text.split_once("\r\n\r\n")?;
    let v: serde_json::Value = serde_json::from_str(json.trim()).ok()?;
    let tls = v
        .get("tls")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    let compat = v
        .get("upstream_compat")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    Some((tls, compat))
}

fn read_bounded<R: Read>(r: &mut R, out: &mut Vec<u8>, deadline: Instant) {
    let mut buf = [0u8; 4096];
    while out.len() < MAX_INFO && Instant::now() < deadline {
        match r.read(&mut buf) {
            Ok(0) | Err(_) => return,
            Ok(n) => out.extend_from_slice(buf.get(..n).unwrap_or_default()),
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use super::*;

    #[test]
    fn wp_secweb_routes_and_targets() {
        assert_eq!(route("/hub/abc/dspi/v1"), Some(Route::Link("abc")));
        assert_eq!(
            route("/hub/abc/dspi/v1/security"),
            Some(Route::Security("abc"))
        );
        assert_eq!(route("/hub/abc/dspi/v1/forget"), Some(Route::Forget("abc")));
        for not in [
            "/hub/",
            "/hub/index.html",
            "/hub/assets/x.js",
            "/hub//dspi/v1",
            "/hub/a/dspi/v2",
            "/dspi/v1",
            "/hub/a/b/dspi/v1",
        ] {
            assert_eq!(route(not), None, "{not}");
        }
        assert_eq!(
            decode_target("10.0.0.2%3A11915").as_deref(),
            Some("10.0.0.2:11915")
        );
        assert_eq!(
            decode_target("%5B%3A%3A1%5D%3A80").as_deref(),
            Some("[::1]:80")
        );
        assert_eq!(
            decode_target("hub.local:11915").as_deref(),
            Some("hub.local:11915")
        );
        for bad in ["a%2Fb", "a%20b", "%", "%4", "%zz", "a@b", "a%00"] {
            assert_eq!(decode_target(bad), None, "{bad}");
        }
    }

    #[test]
    fn wp_secweb_refusals_say_no_secret_and_have_codes() {
        for r in [
            Refusal::UnknownTarget,
            Refusal::HubKeyChanged,
            Refusal::TlsRefused,
            Refusal::WrongHub,
            Refusal::Unreachable("x".into()),
            Refusal::Protocol,
        ] {
            assert!(!r.code().is_empty());
            assert!(!r.message().is_empty());
        }
        assert_eq!(Refusal::HubKeyChanged.code(), "hub_key_changed");
        // -- WP PINTLS -- removing the hub is the one way back to plaintext, and it says so
        assert!(
            Refusal::TlsRefused
                .message()
                .contains("only way to connect to it without encryption")
        );
    }

    #[test]
    fn wp_secweb_hello_facts() {
        let (hid, k) = hello_facts(r#"{"t":"hello","hub":{"id":"ab12","name":"x"}}"#).unwrap();
        assert_eq!(hid, "ab12");
        assert!(k.is_none());
        assert!(hello_facts(r#"{"t":"hello","hub":{"id":""}}"#).is_none());
        assert!(hello_facts("not json").is_none());
        assert!(!rotation_valid(
            &SpkiHash([1; 32]),
            "ab12",
            &SpkiHash([2; 32]),
            None
        ));
        let forged = KeyNext {
            spki: SpkiHash([2; 32]).to_b64url(),
            prev_spki: "AA".into(),
            sig: "AA".into(),
        };
        assert!(!rotation_valid(
            &SpkiHash([1; 32]),
            "ab12",
            &SpkiHash([2; 32]),
            Some(&forged)
        ));
    }

    #[test]
    fn wp_secweb_a_signed_rotation_is_valid_and_a_forged_one_is_not() {
        use crate::tls_identity::{TlsIdentity, TlsNames};
        let dir = crate::store::tests_wp_hb1::scratch("wp-secweb-rotation");
        let names = TlsNames::new(None, &[]);
        let now = std::time::SystemTime::now();
        let (mut id, _) = TlsIdentity::load(&dir, "secwebhub", &names, now).unwrap();
        let old = id.pin();
        id.rotate(&names, now, false).unwrap();
        let k = id.key_next(now).unwrap();
        let wire = KeyNext {
            spki: k.spki.to_b64url(),
            prev_spki: data_encoding::BASE64URL_NOPAD.encode(&k.prev_spki_der),
            sig: data_encoding::BASE64URL_NOPAD.encode(&k.sig),
        };
        assert!(rotation_valid(&old, "secwebhub", &id.pin(), Some(&wire)));
        // Another hub id, another presented key, another pinned key: all refused.
        assert!(!rotation_valid(&old, "otherhub", &id.pin(), Some(&wire)));
        assert!(!rotation_valid(
            &old,
            "secwebhub",
            &SpkiHash([7; 32]),
            Some(&wire)
        ));
        assert!(!rotation_valid(
            &SpkiHash([7; 32]),
            "secwebhub",
            &id.pin(),
            Some(&wire)
        ));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The same vector as the web's `fingerprint` (apps/web/test/unit/secweb.test.ts).
    #[test]
    fn wp_secweb_the_display_form_the_web_computes_too() {
        let mut bytes = [0u8; 32];
        for (i, b) in bytes.iter_mut().enumerate() {
            *b = u8::try_from(i).unwrap();
        }
        let pin = SpkiHash(bytes);
        assert_eq!(
            pin.to_b64url(),
            "AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8"
        );
        assert_eq!(pin.display(), "SHA256:000G-40R4-0M30-E209");
    }

    #[test]
    fn wp_secweb_enabled_on_loopback_with_web_only() {
        use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
        assert!(enabled(IpAddr::V4(Ipv4Addr::LOCALHOST), true));
        assert!(enabled(IpAddr::V6(Ipv6Addr::LOCALHOST), true));
        assert!(!enabled(IpAddr::V4(Ipv4Addr::LOCALHOST), false));
        assert!(!enabled(IpAddr::V6(Ipv6Addr::UNSPECIFIED), true));
        assert!(!enabled(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 2)), true));
    }
}
