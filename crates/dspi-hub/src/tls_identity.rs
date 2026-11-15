//! The hub's TLS identity (WP SECKEY, PLAN-SEC 4.2): one P-256 key per hub, a
//! self-signed certificate for it, the SPKI pin clients keep, and key rotation.
//!
//! **On disk**, in `<data>/tls/` (the hub store's folder, then `tls`):
//!
//! | File | What | Mode |
//! |---|---|---|
//! | `key.pem` | the private key, PKCS#8 | 0600 |
//! | `cert.pem` | the certificate, for `curl --cacert` and a browser import too | 0600 |
//! | `identity.json` | what the certificate was issued for, and a rotation's announcement | 0600 |
//!
//! The folder is 0700, and tightened to it when found wider. On Windows none of this
//! gets an explicit ACL: the files inherit the profile's under `%LOCALAPPDATA%`, as
//! `hub-store.json` does (accepted in PLAN-SEC 4.2).
//!
//! **The pin is SHA-256 of the SubjectPublicKeyInfo** (RFC 7469, curl
//! `--pinnedpubkey`), so the certificate can be issued again with no re-pairing: it is,
//! keeping the key, when the names it carries change or it is within 30 days of its end.
//! A DHCP address change never does: only addresses configured as static are in it
//! ([`TlsNames`]), since a browser's click-through is per host *and* certificate.
//!
//! **Dates.** 825 days. Clients never check them (the pin is what counts), but browsers
//! do. A hub whose wall clock is not plausible (a Pi with no battery clock boots in 1970)
//! issues from [`CLOCK_FLOOR`] instead, and never re-issues for expiry until the clock is
//! plausible.
//!
//! **Rotation** ([`TlsIdentity::rotate`], `uconsole hub tls rotate`): a new key, a new
//! certificate, and for 30 days an announcement ([`KeyNext`], `hello.hub.key_next`)
//! signed by the old key over `"dspi-key-rotate\0" || hid || new_pin`. Only the old key's
//! *public* half is kept for that window: the signature is made once, at the rotation,
//! and no protocol step needs the old private key again, so it is deleted at once.
//! `forget_old` (a compromise) makes no signature, so every client re-pairs.
//!
//! **A corrupt or missing key** follows the store's rule: the file is set aside as
//! `key.pem.bad-<time>`, a new key is made, a loud log line says so, and the returned
//! [`TlsIdentityEvent::KeyReplaced`] is for the hub to pass on, so clients show "this
//! hub's key changed". A missing certificate or `identity.json` is only issued again.
//!
//! **Secrets.** The key's bytes are in memory only inside [`TlsIdentity`], wiped on drop,
//! reachable only inside this crate ([`TlsIdentity::private_key_der`], for the server's
//! rustls config), and never in a `Debug` string, a log line, an event or an error.
//! A diagnostics bundle reads named files only and never this folder's key.
//!
//! The SPKI hash, its display forms and the rotation message are SECPIPE's
//! (`dspi_link::tls`; -- WP SECSRV --): one hash for one key on both ends.

use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use data_encoding::{BASE64, HEXLOWER};
use rcgen::{
    CertificateParams, DistinguishedName, DnType, ExtendedKeyUsagePurpose, IsCa, KeyPair,
    KeyUsagePurpose, PKCS_ECDSA_P256_SHA256, PublicKeyData, SanType, SerialNumber, SigningKey,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

use crate::error::HubError;
use crate::store::{random_bytes, unix_seconds};

/// The folder under the hub's data folder.
pub const TLS_DIR: &str = "tls";
const KEY_FILE: &str = "key.pem";
const CERT_FILE: &str = "cert.pem";
const META_FILE: &str = "identity.json";
/// A key or certificate PEM is a few hundred bytes. A larger file is not ours.
const MAX_PEM: u64 = 16 << 10;
const MAX_META: u64 = 64 << 10;
const DAY: u64 = 86_400;
/// How long a certificate is valid (PLAN-SEC 4.2).
pub const VALIDITY_DAYS: u64 = 825;
/// Re-issued this close to its end.
pub const RENEW_DAYS: u64 = 30;
/// How long a rotation is announced.
pub const ROTATION_DAYS: u64 = 30;
/// 2025-01-01T00:00:00Z. A wall clock before it is not the real time.
pub const CLOCK_FLOOR: u64 = 1_735_689_600;
/// What a rotation signature is over, before `hid` and the new pin.
pub const ROTATE_CONTEXT: &[u8] = dspi_link::tls::ROTATION_CONTEXT;
const META_VERSION: u32 = 1;

// -- WP SECSRV -- SECKEY's copies of SECPIPE's helpers replaced by them (PLAN-SEC M2)

/// A hub key's pin: SHA-256 of its SubjectPublicKeyInfo ([`SpkiHash`], dspi-link's,
/// the one the clients pin). Public, so `Debug` shows it.
pub use dspi_link::tls::SpkiHash;

fn sha256(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

/// The names a certificate is issued for, besides `localhost` and the hub's stable
/// `<hid>.dspi.local`. Changing these issues it again; nothing else about the network does.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TlsNames {
    mdns_host: Option<String>,
    static_ips: Vec<IpAddr>,
}

impl TlsNames {
    /// `mdns_host`: the responder's host name now (`dspi-0123456789ab.local`, or `-2`
    /// after a conflict); one that is not a DNS name is left out, with a log line.
    /// `static_ips`: only addresses the machine is *configured* with, never what DHCP
    /// handed out, or every lease change would mean a new browser warning.
    pub fn new(mdns_host: Option<&str>, static_ips: &[IpAddr]) -> Self {
        let mdns_host = mdns_host.and_then(|h| {
            let h = h.trim().trim_end_matches('.').to_ascii_lowercase();
            if h.is_empty() {
                None
            } else if is_dns_name(&h) {
                Some(h)
            } else {
                log::warn!("tls: the host name {h:?} is not a DNS name; it is left out");
                None
            }
        });
        let mut static_ips = static_ips.to_vec();
        static_ips.sort();
        static_ips.dedup();
        Self {
            mdns_host,
            static_ips,
        }
    }

    /// The SANs, in a fixed order, as stored.
    fn sans(&self, hid: &str) -> Vec<String> {
        let mut names = vec!["localhost".to_owned()];
        let stable = format!("{}.dspi.local", hid.to_ascii_lowercase());
        if is_dns_name(&stable) {
            names.push(stable);
        }
        if let Some(h) = &self.mdns_host {
            names.push(h.clone());
        }
        names.sort();
        names.dedup();
        names.extend(self.static_ips.iter().map(IpAddr::to_string));
        names
    }
}

/// LDH labels of 1..=63 bytes, at most 253 in all (RFC 1123).
fn is_dns_name(name: &str) -> bool {
    name.len() <= 253
        && name.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        })
}

/// A rotation's announcement, `hello.hub.key_next` (PLAN-SEC 4.2). All public.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyNext {
    /// The pin of the key the hub now serves.
    pub spki: SpkiHash,
    /// The old key's SubjectPublicKeyInfo, DER: a client checks its SHA-256 against the
    /// pin it holds, then the signature with it.
    pub prev_spki_der: Vec<u8>,
    /// The old key's ECDSA P-256 SHA-256 signature, ASN.1 DER (as in TLS), over
    /// [`rotation_message`]`(hid, spki)`.
    pub sig: Vec<u8>,
    /// Announced until then, Unix seconds.
    pub until: u64,
}

/// `"dspi-key-rotate\0" || hid || new_pin`: what a rotation signature is over
/// (-- WP SECSRV -- dspi-link's, which the clients verify with).
pub use dspi_link::tls::rotation_message;

/// Why a certificate was issued.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IssueReason {
    /// The first one, with a new key.
    New,
    /// The names it carries changed (the mDNS name, a static address).
    NamesChanged,
    /// Within [`RENEW_DAYS`] of its end.
    NearExpiry,
    /// The old one, or what it was issued for, could not be read.
    Unreadable,
    /// A new key from [`TlsIdentity::rotate`].
    Rotated,
}

/// What happened to the identity: for the hub's log and its clients. No key material.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TlsIdentityEvent {
    /// A first key, on a hub that had none.
    Created {
        /// The new pin.
        pin: SpkiHash,
    },
    /// The key could not be read and a new one was made: clients will see "key changed".
    KeyReplaced {
        /// The new pin.
        pin: SpkiHash,
        /// Where the unreadable file went, if it was there.
        set_aside: Option<PathBuf>,
    },
    /// The certificate was issued again for the same key: no client notices.
    CertificateIssued {
        /// The pin, unchanged.
        pin: SpkiHash,
        /// Why.
        reason: IssueReason,
    },
    /// [`TlsIdentity::rotate`] made a new key.
    Rotated {
        /// The new pin.
        pin: SpkiHash,
        /// The old one.
        previous: SpkiHash,
        /// Whether the old key signed the new one (not with `forget_old`).
        signed: bool,
    },
}

/// The key's PKCS#8 bytes: wiped on drop, never shown.
struct SecretDer(Zeroizing<Vec<u8>>);

impl fmt::Debug for SecretDer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("[private key]")
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct RotationMeta {
    at: u64,
    /// base64 DER.
    prev_spki_der: String,
    /// base64 DER.
    sig: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct Meta {
    v: u32,
    /// Hex: the key the certificate is for.
    pin: String,
    /// Hex: SHA-256 of the certificate's DER, so a stray `cert.pem` is not taken for it.
    cert_sha256: String,
    names: Vec<String>,
    not_before: u64,
    not_after: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    rotation: Option<RotationMeta>,
}

/// The hub's key, its certificate and what is announced about them.
pub struct TlsIdentity {
    dir: PathBuf,
    hid: String,
    key: SecretDer,
    pin: SpkiHash,
    spki_der: Vec<u8>,
    cert_der: Vec<u8>,
    cert_pem: String,
    meta: Meta,
}

impl fmt::Debug for TlsIdentity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TlsIdentity")
            .field("dir", &self.dir)
            .field("pin", &self.pin)
            .field("names", &self.meta.names)
            .field("not_after", &self.meta.not_after)
            .field("rotating", &self.meta.rotation.is_some())
            .field("key", &self.key)
            .finish()
    }
}

fn io_error(what: &str, path: &Path, e: &io::Error) -> HubError {
    HubError::Io(format!("{what} {}: {e}", path.display()))
}

fn tls_error(what: &str, e: &rcgen::Error) -> HubError {
    HubError::Io(format!("tls identity: {what}: {e}"))
}

/// Create `dir` and its missing parents as 0700 (Unix; elsewhere the platform's
/// default, see the module docs). An existing folder is left as it is.
pub(crate) fn create_private_dir_all(dir: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(dir)
    }
    #[cfg(not(unix))]
    {
        fs::create_dir_all(dir)
    }
}

/// `tls/` is ours: 0700 whatever it was.
fn ensure_tls_dir(dir: &Path) -> Result<(), HubError> {
    create_private_dir_all(dir).map_err(|e| io_error("cannot create", dir, &e))?;
    let meta = fs::symlink_metadata(dir).map_err(|e| io_error("cannot read", dir, &e))?;
    if !meta.is_dir() {
        return Err(HubError::Io(format!(
            "{} is not a folder; the hub's key is not kept there",
            dir.display()
        )));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if meta.permissions().mode() & 0o077 != 0 {
            log::warn!("tls: {} was open to others; made 0700", dir.display());
            fs::set_permissions(dir, fs::Permissions::from_mode(0o700))
                .map_err(|e| io_error("cannot restrict", dir, &e))?;
        }
    }
    Ok(())
}

/// A key file someone widened goes back to 0600.
fn tighten_file(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(meta) = fs::metadata(path)
            && meta.permissions().mode() & 0o077 != 0
        {
            log::warn!("tls: {} was readable by others; made 0600", path.display());
            let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o600));
        }
    }
    #[cfg(not(unix))]
    let _ = path;
}

fn read_limited(path: &Path, max: u64) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    File::open(path)?
        .take(max.saturating_add(1))
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > max {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "too large"));
    }
    Ok(bytes)
}

#[cfg(unix)]
fn private_file(path: &Path) -> io::Result<File> {
    use std::os::unix::fs::OpenOptionsExt;
    OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)
}

#[cfg(not(unix))]
fn private_file(path: &Path) -> io::Result<File> {
    OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(path)
}

/// Write through a 0600 temp file and rename, as the store does.
fn write_private(dir: &Path, name: &str, bytes: &[u8]) -> Result<(), HubError> {
    let path = dir.join(name);
    let temp = dir.join(format!("{name}.tmp"));
    let written = (|| {
        let mut file = private_file(&temp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        fs::rename(&temp, &path)
    })();
    if written.is_err() {
        let _ = fs::remove_file(&temp);
    }
    written.map_err(|e| io_error("cannot write", &path, &e))?;
    #[cfg(unix)]
    if let Ok(d) = File::open(dir) {
        let _ = d.sync_all();
    }
    Ok(())
}

/// The one `-----BEGIN <label>-----` block of `text`, decoded. `None` for anything else.
fn pem_block(text: &str, label: &str) -> Option<Vec<u8>> {
    let begin = format!("-----BEGIN {label}-----");
    let end = format!("-----END {label}-----");
    let (_, rest) = text.split_once(&begin)?;
    let (body, _) = rest.split_once(&end)?;
    let b64: String = body.chars().filter(|c| !c.is_ascii_whitespace()).collect();
    BASE64.decode(b64.as_bytes()).ok()
}

fn to_pem(label: &str, der: &[u8]) -> String {
    let b64 = BASE64.encode(der);
    let mut out = format!("-----BEGIN {label}-----\n");
    let mut rest = b64.as_str();
    while !rest.is_empty() {
        let (line, tail) = rest.split_at(rest.len().min(64));
        out.push_str(line);
        out.push('\n');
        rest = tail;
    }
    out.push_str(&format!("-----END {label}-----\n"));
    out
}

/// A P-256 key from `key.pem`'s text, or why not (never the text itself).
fn parse_key(bytes: &[u8]) -> Result<KeyPair, String> {
    let text = std::str::from_utf8(bytes).map_err(|_| "not text".to_owned())?;
    let der = Zeroizing::new(pem_block(text, "PRIVATE KEY").ok_or("no PKCS#8 PEM block")?);
    let key =
        KeyPair::from_pkcs8_der_and_sign_algo(&der.as_slice().into(), &PKCS_ECDSA_P256_SHA256)
            .map_err(|_| "not a P-256 key".to_owned())?;
    Ok(key)
}

fn new_key() -> Result<KeyPair, HubError> {
    KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256).map_err(|e| tls_error("cannot make a key", &e))
}

fn offset_time(secs: u64) -> Result<time::OffsetDateTime, HubError> {
    i64::try_from(secs)
        .ok()
        .and_then(|s| time::OffsetDateTime::from_unix_timestamp(s).ok())
        .ok_or_else(|| HubError::Io(format!("tls identity: no date for {secs}")))
}

/// Whether `now` can be the real time.
fn plausible(now: u64) -> bool {
    now >= CLOCK_FLOOR
}

struct Issued {
    der: Vec<u8>,
    pem: String,
    not_before: u64,
    not_after: u64,
}

fn issue(key: &KeyPair, hid: &str, names: &[String], now: u64) -> Result<Issued, HubError> {
    let mut params = CertificateParams::default();
    for name in names {
        let san = match name.parse::<IpAddr>() {
            Ok(ip) => SanType::IpAddress(ip),
            Err(_) => SanType::DnsName(
                name.as_str()
                    .try_into()
                    .map_err(|e| tls_error("a name the certificate cannot carry", &e))?,
            ),
        };
        params.subject_alt_names.push(san);
    }
    let mut dn = DistinguishedName::new();
    dn.push(DnType::CommonName, format!("DSPi hub {hid}"));
    params.distinguished_name = dn;
    params.is_ca = IsCa::ExplicitNoCa;
    params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
    params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
    // A serial of its own each time: Firefox refuses two certificates with one issuer
    // and serial, and the issuer stays the same across a re-issue.
    let mut serial: [u8; 16] = random_bytes()?;
    if let Some(first) = serial.first_mut() {
        *first = (*first & 0x7F) | 0x40;
    }
    params.serial_number = Some(SerialNumber::from_slice(&serial));
    // An hour back, for a browser whose clock is a little behind.
    let start = if plausible(now) {
        now.saturating_sub(3600)
    } else {
        CLOCK_FLOOR
    };
    let end = start.saturating_add(VALIDITY_DAYS.saturating_mul(DAY));
    params.not_before = offset_time(start)?;
    params.not_after = offset_time(end)?;
    let cert = params
        .self_signed(key)
        .map_err(|e| tls_error("cannot sign the certificate", &e))?;
    let der = cert.der().to_vec();
    Ok(Issued {
        pem: to_pem("CERTIFICATE", &der),
        der,
        not_before: start,
        not_after: end,
    })
}

/// What `load` found of `key.pem`.
enum KeyRead {
    Found(Box<KeyPair>),
    Missing,
    Bad(String),
}

fn read_key(path: &Path) -> Result<KeyRead, HubError> {
    match read_limited(path, MAX_PEM) {
        Ok(bytes) => {
            let bytes = Zeroizing::new(bytes);
            Ok(match parse_key(&bytes) {
                Ok(key) => KeyRead::Found(Box::new(key)),
                Err(why) => KeyRead::Bad(why),
            })
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(KeyRead::Missing),
        Err(e) if e.kind() == io::ErrorKind::InvalidData => Ok(KeyRead::Bad("too large".into())),
        Err(e) => Err(io_error("cannot read", path, &e)),
    }
}

fn read_meta(path: &Path) -> Option<Meta> {
    let bytes = read_limited(path, MAX_META).ok()?;
    let meta: Meta = serde_json::from_slice(&bytes).ok()?;
    (meta.v == META_VERSION).then_some(meta)
}

fn read_cert(path: &Path, meta: &Meta) -> Option<(Vec<u8>, String)> {
    let bytes = read_limited(path, MAX_PEM).ok()?;
    let text = String::from_utf8(bytes).ok()?;
    let der = pem_block(&text, "CERTIFICATE")?;
    (HEXLOWER.encode(&sha256(&der)) == meta.cert_sha256).then_some((der, text))
}

impl TlsIdentity {
    /// The identity kept under `data_dir` (the hub store's folder), made on first use.
    /// Issues the certificate again when it is missing, unreadable, for other names or
    /// near its end, keeping the key. A key that cannot be read is set aside and
    /// replaced (see the module docs). `now` is the hub clock's wall time.
    ///
    /// Errors: [`HubError::Io`] when the folder or a file cannot be written, or a key
    /// cannot be read for a reason other than its contents (permissions).
    pub fn load(
        data_dir: &Path,
        hid: &str,
        names: &TlsNames,
        now: SystemTime,
    ) -> Result<(Self, Vec<TlsIdentityEvent>), HubError> {
        let dir = data_dir.join(TLS_DIR);
        ensure_tls_dir(&dir)?;
        let now = unix_seconds(now);
        let key_path = dir.join(KEY_FILE);
        let meta_path = dir.join(META_FILE);
        let mut events = Vec::new();

        let (key, new) = match read_key(&key_path)? {
            KeyRead::Found(key) => {
                tighten_file(&key_path);
                (*key, None)
            }
            KeyRead::Missing => {
                let key = new_key()?;
                let had_one = meta_path.exists();
                if had_one {
                    log::error!(
                        "tls: the hub's key {} is gone; a new one is made, and every \
                         client will report that this hub's key changed",
                        key_path.display()
                    );
                }
                (key, Some((had_one, None)))
            }
            KeyRead::Bad(why) => {
                let aside = dir.join(format!("{KEY_FILE}.bad-{now}"));
                fs::rename(&key_path, &aside)
                    .map_err(|e| io_error("cannot move", &key_path, &e))?;
                log::error!(
                    "tls: the hub's key {} could not be read ({why}); kept as {} and a \
                     new key made: every client will report that this hub's key changed",
                    key_path.display(),
                    aside.display()
                );
                (new_key()?, Some((true, Some(aside))))
            }
        };
        if new.is_some() {
            write_private(
                &dir,
                KEY_FILE,
                Zeroizing::new(key.serialize_pem()).as_bytes(),
            )?;
        }
        let spki_der = key.subject_public_key_info();
        let pin = SpkiHash::of_spki(&spki_der);
        if let Some((replaced, set_aside)) = new {
            events.push(if replaced {
                TlsIdentityEvent::KeyReplaced { pin, set_aside }
            } else {
                log::info!("tls: a new hub key, {pin}");
                TlsIdentityEvent::Created { pin }
            });
        }

        // The certificate and what it is for; either unreadable means issuing anew.
        let meta = read_meta(&meta_path).filter(|m| m.pin == pin.to_hex());
        let cert = meta
            .as_ref()
            .and_then(|m| read_cert(&dir.join(CERT_FILE), m));
        let mut identity = Self {
            dir,
            hid: hid.to_owned(),
            key: SecretDer(Zeroizing::new(key.serialized_der().to_vec())),
            pin,
            spki_der,
            cert_der: Vec::new(),
            cert_pem: String::new(),
            meta: Meta::default(),
        };
        match (meta, cert) {
            (Some(meta), Some((der, pem))) => {
                identity.meta = meta;
                identity.cert_der = der;
                identity.cert_pem = pem;
                if let Some(event) = identity.refresh_with(&key, names, now)? {
                    events.push(event);
                }
            }
            _ => {
                let reason = if events.is_empty() {
                    log::warn!("tls: the certificate could not be read; issued again");
                    IssueReason::Unreadable
                } else {
                    IssueReason::New
                };
                identity.reissue(&key, names, now, None)?;
                if reason == IssueReason::Unreadable {
                    events.push(TlsIdentityEvent::CertificateIssued { pin, reason });
                }
            }
        }
        Ok((identity, events))
    }

    /// Issue the certificate again if the names changed or it is near its end, and end
    /// a rotation's announcement after [`ROTATION_DAYS`]. For the hub's tick and an
    /// mDNS name change. `Some` when a new certificate is to be served.
    pub fn refresh(
        &mut self,
        names: &TlsNames,
        now: SystemTime,
    ) -> Result<Option<TlsIdentityEvent>, HubError> {
        let key = self.key_pair()?;
        self.refresh_with(&key, names, unix_seconds(now))
    }

    fn refresh_with(
        &mut self,
        key: &KeyPair,
        names: &TlsNames,
        now: u64,
    ) -> Result<Option<TlsIdentityEvent>, HubError> {
        let mut changed = false;
        if let Some(r) = &self.meta.rotation
            && plausible(now)
            && now >= r.at.saturating_add(ROTATION_DAYS.saturating_mul(DAY))
        {
            log::info!("tls: the rotation to {} is no longer announced", self.pin);
            self.meta.rotation = None;
            changed = true;
        }
        let wanted = names.sans(&self.hid);
        let reason = if wanted != self.meta.names {
            Some(IssueReason::NamesChanged)
        } else if plausible(now)
            && now.saturating_add(RENEW_DAYS.saturating_mul(DAY)) >= self.meta.not_after
        {
            Some(IssueReason::NearExpiry)
        } else {
            None
        };
        if let Some(reason) = reason {
            let rotation = self.meta.rotation.take();
            self.reissue(key, names, now, rotation)?;
            log::info!(
                "tls: certificate issued again ({reason:?}), key {}",
                self.pin
            );
            return Ok(Some(TlsIdentityEvent::CertificateIssued {
                pin: self.pin,
                reason,
            }));
        }
        if changed {
            self.save_meta()?;
        }
        Ok(None)
    }

    /// A new key, `uconsole hub tls rotate` (Admin, on the hub's machine). The old key
    /// signs the new pin, announced by [`Self::key_next`] for [`ROTATION_DAYS`], unless
    /// `forget_old` (a compromised key): then there is no signature and every client
    /// re-pairs. Rotating again within the window signs with the key served now, so a
    /// client still on the first key re-pairs.
    pub fn rotate(
        &mut self,
        names: &TlsNames,
        now: SystemTime,
        forget_old: bool,
    ) -> Result<TlsIdentityEvent, HubError> {
        let now = unix_seconds(now);
        let old = self.key_pair()?;
        let new = new_key()?;
        let new_spki = new.subject_public_key_info();
        let new_pin = SpkiHash::of_spki(&new_spki);
        let rotation = if forget_old {
            None
        } else {
            let sig = old
                .sign(&rotation_message(&self.hid, &new_pin))
                .map_err(|e| tls_error("cannot sign the new key", &e))?;
            Some(RotationMeta {
                at: now,
                prev_spki_der: BASE64.encode(&self.spki_der),
                sig: BASE64.encode(&sig),
            })
        };
        let previous = self.pin;
        // The key first: a stop between this and the metadata leaves a key whose
        // certificate is issued again at the next start, and an announcement lost
        // (clients re-pair), never a certificate for a key that is not there.
        write_private(
            &self.dir,
            KEY_FILE,
            Zeroizing::new(new.serialize_pem()).as_bytes(),
        )?;
        self.key = SecretDer(Zeroizing::new(new.serialized_der().to_vec()));
        self.pin = new_pin;
        self.spki_der = new_spki;
        self.reissue(&new, names, now, rotation)?;
        let signed = !forget_old;
        log::warn!("tls: the hub's key rotated from {previous} to {new_pin} (signed: {signed})");
        Ok(TlsIdentityEvent::Rotated {
            pin: new_pin,
            previous,
            signed,
        })
    }

    fn reissue(
        &mut self,
        key: &KeyPair,
        names: &TlsNames,
        now: u64,
        rotation: Option<RotationMeta>,
    ) -> Result<(), HubError> {
        let sans = names.sans(&self.hid);
        let issued = issue(key, &self.hid, &sans, now)?;
        self.meta = Meta {
            v: META_VERSION,
            pin: self.pin.to_hex(),
            cert_sha256: HEXLOWER.encode(&sha256(&issued.der)),
            names: sans,
            not_before: issued.not_before,
            not_after: issued.not_after,
            rotation,
        };
        write_private(&self.dir, CERT_FILE, issued.pem.as_bytes())?;
        self.cert_der = issued.der;
        self.cert_pem = issued.pem;
        self.save_meta()
    }

    fn save_meta(&self) -> Result<(), HubError> {
        let bytes = serde_json::to_vec_pretty(&self.meta)
            .map_err(|e| HubError::Io(format!("cannot encode {META_FILE}: {e}")))?;
        write_private(&self.dir, META_FILE, &bytes)
    }

    fn key_pair(&self) -> Result<KeyPair, HubError> {
        KeyPair::from_pkcs8_der_and_sign_algo(
            &self.key.0.as_slice().into(),
            &PKCS_ECDSA_P256_SHA256,
        )
        .map_err(|e| tls_error("the key in memory", &e))
    }

    /// The pin clients keep.
    pub fn pin(&self) -> SpkiHash {
        self.pin
    }

    /// `SHA256:K7Q2-M9XD-…`, for the pairing screen and the PIN line.
    pub fn fingerprint(&self) -> String {
        self.pin.display()
    }

    /// The key's SubjectPublicKeyInfo, DER.
    pub fn spki_der(&self) -> &[u8] {
        &self.spki_der
    }

    /// The certificate, DER: what the server presents.
    pub fn certificate_der(&self) -> &[u8] {
        &self.cert_der
    }

    /// The certificate, PEM: what `cert.pem` holds, and what a diagnostics bundle may carry.
    pub fn certificate_pem(&self) -> &str {
        &self.cert_pem
    }

    /// The names the certificate carries.
    pub fn names(&self) -> &[String] {
        &self.meta.names
    }

    /// The certificate's end, Unix seconds.
    pub fn not_after(&self) -> u64 {
        self.meta.not_after
    }

    /// The folder, `<data>/tls`.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// The private key, PKCS#8 DER, for the server's rustls config. Crate-only, and
    /// never to be logged.
    pub(crate) fn private_key_der(&self) -> &[u8] {
        &self.key.0
    }

    /// The rotation's announcement while it lasts: `hello.hub.key_next`. A clock that
    /// went back keeps announcing; one that is not plausible never ends it.
    pub fn key_next(&self, now: SystemTime) -> Option<KeyNext> {
        let r = self.meta.rotation.as_ref()?;
        let until = r.at.saturating_add(ROTATION_DAYS.saturating_mul(DAY));
        let now = unix_seconds(now);
        if plausible(now) && now >= until {
            return None;
        }
        Some(KeyNext {
            spki: self.pin,
            prev_spki_der: BASE64.decode(r.prev_spki_der.as_bytes()).ok()?,
            sig: BASE64.decode(r.sig.as_bytes()).ok()?,
            until,
        })
    }
}

#[cfg(test)]
#[path = "tls_identity/tests_wp_seckey.rs"]
mod tests_wp_seckey;
