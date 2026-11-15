//! The hub's outbound TLS connector (WP SECOUT, PLAN-SEC section 4.8): one rustls client for
//! webhooks ([`super::http`]) and MQTT ([`crate::mqtt::client`]).
//!
//! - TLS 1.2 and 1.3 (receivers and brokers in the wild still speak 1.2), the ring provider,
//!   SNI from the host name; an IP literal (IPv6 without brackets) is an IP `ServerName`, so
//!   no SNI goes out and the certificate must name the address.
//! - Three trusts per target ([`Trust`], OD-SECf): the `webpki-roots` CAs (the default), a
//!   pasted `ca_pem` (a CA or a self-signed certificate), or `pin_sha256`, the SHA-256 of the
//!   server's SubjectPublicKeyInfo (RFC 7469, curl `--pinnedpubkey`). Host names and dates are
//!   checked by the first two; a pin replaces both (the handshake signature is still
//!   verified, so the server must hold the pinned key).
//! - An optional client certificate (MQTT), from PEM through `rustls-pki-types`.
//! - The handshake is bounded by the caller's deadline, and nothing is written to the
//!   connection before it has completed and the server is trusted.
//!
//! [`plaintext_allowed`] is OD-SECe's rule for `http://`: private, loopback or link-local
//! destinations only, checked on the resolved address of every send.

use std::fmt;
use std::io::{self, Read, Write};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, Shutdown, TcpStream};
use std::sync::Arc;
use std::time::{Duration, Instant};

use data_encoding::{BASE64, HEXLOWER_PERMISSIVE};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::client::{ClientConfig, ClientConnection, WebPkiServerVerifier};
use rustls::crypto::{CryptoProvider, WebPkiSupportedAlgorithms};
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, ServerName, UnixTime};
use rustls::server::ParsedCertificate;
use rustls::{
    CertificateError, DigitallySignedStruct, RootCertStore, SignatureScheme, StreamOwned,
};
use sha2::{Digest, Sha256};

/// A pasted PEM (a CA list, a client certificate chain or key), in bytes, at most.
pub const MAX_PEM_BYTES: usize = 32 * 1024;
/// Certificates in one `ca_pem` or client chain, at most.
pub const MAX_PEM_CERTS: usize = 16;
/// The prefix of a pin's canonical form, as curl writes it.
pub const PIN_PREFIX: &str = "sha256//";

/// Why a TLS connection was not made. Never holds a key.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TlsError {
    /// The settings cannot make a client (a PEM that does not parse, a key that does not
    /// fit the certificate).
    #[error("TLS settings: {0}")]
    Config(String),
    /// The host is not a name or an address TLS can check.
    #[error("{0:?} is not a host name TLS can verify")]
    Name(String),
    /// The server's key is not the pinned one.
    #[error("the server's key does not match the pinned key (pin_sha256)")]
    PinMismatch,
    /// The server's certificate is not trusted, or not for this host.
    #[error("the server's certificate was refused: {0}")]
    Certificate(String),
    /// The handshake failed otherwise (an alert, no common version or cipher suite).
    #[error("TLS handshake failed: {0}")]
    Handshake(String),
    /// Out of time during the handshake.
    #[error("TLS handshake timed out")]
    Timeout,
    /// The connection failed during the handshake.
    #[error("connection failed during the TLS handshake: {0}")]
    Io(String),
}

// ── Pins and certificates ───────────────────────────────────────────────────

/// A strict DER element at the start of `input`: its tag, its contents and what follows.
/// Definite, minimal lengths only (as webpki reads them, so both see the same fields),
/// at most four length bytes.
fn der_element(input: &[u8]) -> Option<(u8, &[u8], &[u8])> {
    let (&tag, rest) = input.split_first()?;
    // Low-tag-number form only; X.509 never uses the high form.
    if tag & 0x1f == 0x1f {
        return None;
    }
    let (&first, rest) = rest.split_first()?;
    let (len, rest) = if first < 0x80 {
        (usize::from(first), rest)
    } else {
        let n = usize::from(first & 0x7f);
        if n == 0 || n > 4 {
            return None;
        }
        let bytes = rest.get(..n)?;
        if bytes.first() == Some(&0) {
            return None;
        }
        let len = bytes.iter().try_fold(0usize, |acc, b| {
            acc.checked_mul(256)?.checked_add(usize::from(*b))
        })?;
        if len < 0x80 {
            return None;
        }
        (len, rest.get(n..)?)
    };
    let body = rest.get(..len)?;
    Some((tag, body, rest.get(len..)?))
}

/// The DER of a certificate's SubjectPublicKeyInfo, whole (tag and length included).
/// `None` for anything that is not a strict X.509 v3 structure up to that field.
pub fn spki_der(cert: &[u8]) -> Option<&[u8]> {
    const SEQUENCE: u8 = 0x30;
    let (tag, certificate, after) = der_element(cert)?;
    if tag != SEQUENCE || !after.is_empty() {
        return None;
    }
    let (tag, mut tbs, _) = der_element(certificate)?;
    if tag != SEQUENCE {
        return None;
    }
    // [0] EXPLICIT version, optional.
    if tbs.first() == Some(&0xa0) {
        tbs = der_element(tbs)?.2;
    }
    // serialNumber INTEGER, signature, issuer, validity, subject.
    for want in [0x02, SEQUENCE, SEQUENCE, SEQUENCE, SEQUENCE] {
        let (tag, _, rest) = der_element(tbs)?;
        if tag != want {
            return None;
        }
        tbs = rest;
    }
    let (tag, _, rest) = der_element(tbs)?;
    if tag != SEQUENCE {
        return None;
    }
    let used = tbs.len().checked_sub(rest.len())?;
    tbs.get(..used)
}

/// SHA-256 of a certificate's SubjectPublicKeyInfo: what `pin_sha256` names.
pub fn spki_sha256(cert: &[u8]) -> Option<[u8; 32]> {
    spki_der(cert).map(|spki| Sha256::digest(spki).into())
}

/// A pin in its canonical form, `sha256//<base64>`, as curl's `--pinnedpubkey` takes it.
pub fn pin_text(hash: &[u8; 32]) -> String {
    format!("{PIN_PREFIX}{}", BASE64.encode(hash))
}

/// Read a pin: `sha256//<base64>`, the bare base64 of the 32 bytes, or 64 hex characters
/// (colons between bytes allowed).
pub fn parse_pin(text: &str) -> Result<[u8; 32], TlsError> {
    let bad =
        || TlsError::Config("pin_sha256 is not sha256//<base64>, base64 or hex of 32 bytes".into());
    let text = text.trim();
    let b64 = text.strip_prefix(PIN_PREFIX).unwrap_or(text);
    if let Ok(bytes) = BASE64.decode(b64.as_bytes())
        && let Ok(pin) = <[u8; 32]>::try_from(bytes.as_slice())
    {
        return Ok(pin);
    }
    if text.starts_with(PIN_PREFIX) {
        return Err(bad());
    }
    let hex: String = text.chars().filter(|c| *c != ':').collect();
    if hex.len() != 64 {
        return Err(bad());
    }
    let bytes = HEXLOWER_PERMISSIVE
        .decode(hex.as_bytes())
        .map_err(|_| bad())?;
    <[u8; 32]>::try_from(bytes.as_slice()).map_err(|_| bad())
}

/// The certificates of a PEM text, at most [`MAX_PEM_CERTS`], at least one.
pub fn parse_certs(what: &str, pem: &str) -> Result<Vec<CertificateDer<'static>>, TlsError> {
    if pem.len() > MAX_PEM_BYTES {
        return Err(TlsError::Config(format!(
            "{what} is longer than {MAX_PEM_BYTES} bytes"
        )));
    }
    let mut certs = Vec::new();
    for cert in CertificateDer::pem_slice_iter(pem.as_bytes()) {
        let cert =
            cert.map_err(|_| TlsError::Config(format!("{what} is not a PEM certificate")))?;
        if certs.len() >= MAX_PEM_CERTS {
            return Err(TlsError::Config(format!(
                "{what} has more than {MAX_PEM_CERTS} certificates"
            )));
        }
        if spki_der(cert.as_ref()).is_none() {
            return Err(TlsError::Config(format!(
                "{what} holds a malformed certificate"
            )));
        }
        certs.push(cert);
    }
    if certs.is_empty() {
        return Err(TlsError::Config(format!(
            "{what} holds no -----BEGIN CERTIFICATE----- block"
        )));
    }
    Ok(certs)
}

/// The private key of a PEM text (PKCS#8, SEC1 or PKCS#1). The error never quotes it.
pub fn parse_key(what: &str, pem: &str) -> Result<PrivateKeyDer<'static>, TlsError> {
    if pem.len() > MAX_PEM_BYTES {
        return Err(TlsError::Config(format!(
            "{what} is longer than {MAX_PEM_BYTES} bytes"
        )));
    }
    PrivateKeyDer::from_pem_slice(pem.as_bytes())
        .map_err(|_| TlsError::Config(format!("{what} is not a PEM private key")))
}

fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

// ── Trust ───────────────────────────────────────────────────────────────────

/// What a connection trusts (OD-SECf).
#[derive(Clone, PartialEq, Eq)]
pub enum Trust {
    /// The `webpki-roots` CAs, host name and dates checked.
    Roots,
    /// These CA (or self-signed) certificates instead, host name and dates checked.
    Ca(Vec<CertificateDer<'static>>),
    /// The server's SubjectPublicKeyInfo must hash to this; no name or date check.
    Pin([u8; 32]),
}

impl fmt::Debug for Trust {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Trust::Roots => f.write_str("Roots"),
            Trust::Ca(certs) => write!(f, "Ca({} certificate(s))", certs.len()),
            Trust::Pin(pin) => write!(f, "Pin({})", pin_text(pin)),
        }
    }
}

impl Trust {
    /// From a target's optional `ca_pem` and `pin_sha256`; both at once is refused.
    pub fn from_settings(ca_pem: Option<&str>, pin: Option<&str>) -> Result<Trust, TlsError> {
        match (ca_pem, pin) {
            (Some(_), Some(_)) => Err(TlsError::Config(
                "ca_pem and pin_sha256 are each a trust of their own: give one".into(),
            )),
            (Some(pem), None) => parse_certs("ca_pem", pem).map(Trust::Ca),
            (None, Some(pin)) => parse_pin(pin).map(Trust::Pin),
            (None, None) => Ok(Trust::Roots),
        }
    }

    /// For people and `outbound.get`: `webpki`, `ca` or `pin`.
    pub fn name(&self) -> &'static str {
        match self {
            Trust::Roots => "webpki",
            Trust::Ca(_) => "ca",
            Trust::Pin(_) => "pin",
        }
    }
}

/// A client certificate chain and its key. Its `Debug` never shows the key.
pub struct ClientIdentity {
    chain: Vec<CertificateDer<'static>>,
    key: PrivateKeyDer<'static>,
}

impl ClientIdentity {
    /// From PEM texts; the key's errors never quote it.
    pub fn from_pem(cert_pem: &str, key_pem: &str) -> Result<ClientIdentity, TlsError> {
        Ok(ClientIdentity {
            chain: parse_certs("client_cert_pem", cert_pem)?,
            key: parse_key("client_key_pem", key_pem)?,
        })
    }
}

impl Clone for ClientIdentity {
    fn clone(&self) -> Self {
        ClientIdentity {
            chain: self.chain.clone(),
            key: self.key.clone_key(),
        }
    }
}

impl fmt::Debug for ClientIdentity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ClientIdentity")
            .field("certificates", &self.chain.len())
            .field("key", &"set")
            .finish()
    }
}

fn provider() -> Arc<CryptoProvider> {
    Arc::new(rustls::crypto::ring::default_provider())
}

/// `pin_sha256`: the end-entity's SPKI must match; the handshake signature is checked with
/// the provider's algorithms, so the server must hold the pinned key. Names, dates and
/// intermediates are not looked at (the pin replaces them).
#[derive(Debug)]
struct PinVerifier {
    pin: [u8; 32],
    algorithms: WebPkiSupportedAlgorithms,
}

impl ServerCertVerifier for PinVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        match spki_sha256(end_entity.as_ref()) {
            Some(got) if ct_eq(&got, &self.pin) => Ok(ServerCertVerified::assertion()),
            Some(_) => Err(rustls::Error::InvalidCertificate(
                CertificateError::ApplicationVerificationFailure,
            )),
            None => Err(rustls::Error::InvalidCertificate(
                CertificateError::BadEncoding,
            )),
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(message, cert, dss, &self.algorithms)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(message, cert, dss, &self.algorithms)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.algorithms.supported_schemes()
    }
}

/// `ca_pem` (WP P2CFIXB, review p2c F2): a presented end-entity certificate that is
/// byte-identical to one of the pasted certificates is accepted with its name checked and
/// the handshake signature verified (dates are not looked at: the exact certificate was
/// given, as with a pin). This is what a self-signed receiver needs, since OpenSSL 3's
/// `req -x509` marks such a certificate CA:TRUE and webpki refuses a CA used as an end
/// entity. Anything else goes to webpki with the pasted certificates as roots: real chains,
/// dates and names checked as usual.
#[derive(Debug)]
struct CaPemVerifier {
    exact: Vec<CertificateDer<'static>>,
    webpki: Arc<WebPkiServerVerifier>,
}

impl ServerCertVerifier for CaPemVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        server_name: &ServerName<'_>,
        ocsp_response: &[u8],
        now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        if self
            .exact
            .iter()
            .any(|c| ct_eq(c.as_ref(), end_entity.as_ref()))
        {
            let parsed = ParsedCertificate::try_from(end_entity)?;
            rustls::client::verify_server_name(&parsed, server_name)?;
            return Ok(ServerCertVerified::assertion());
        }
        self.webpki
            .verify_server_cert(end_entity, intermediates, server_name, ocsp_response, now)
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.webpki.verify_tls12_signature(message, cert, dss)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.webpki.verify_tls13_signature(message, cert, dss)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.webpki.supported_verify_schemes()
    }
}

/// A client configuration for `trust`, with a client certificate if given, and these ALPN
/// protocols (`http/1.1` for webhooks, none for MQTT).
pub fn client_config(
    trust: &Trust,
    identity: Option<&ClientIdentity>,
    alpn: &[&[u8]],
) -> Result<Arc<ClientConfig>, TlsError> {
    let provider = provider();
    let builder = ClientConfig::builder_with_provider(Arc::clone(&provider))
        .with_protocol_versions(&[&rustls::version::TLS13, &rustls::version::TLS12])
        .map_err(|e| TlsError::Config(e.to_string()))?;
    let roots = |certs: &[CertificateDer<'static>]| -> Result<RootCertStore, TlsError> {
        let mut store = RootCertStore::empty();
        for cert in certs {
            store
                .add(cert.clone())
                .map_err(|e| TlsError::Config(format!("ca_pem: {e}")))?;
        }
        Ok(store)
    };
    let store = match trust {
        Trust::Roots => Some(RootCertStore {
            roots: webpki_roots::TLS_SERVER_ROOTS.to_vec(),
        }),
        Trust::Ca(certs) => Some(roots(certs)?),
        Trust::Pin(_) => None,
    };
    let builder = match (store, trust) {
        (Some(store), _) => {
            let verifier =
                WebPkiServerVerifier::builder_with_provider(Arc::new(store), Arc::clone(&provider))
                    .build()
                    .map_err(|e| TlsError::Config(e.to_string()))?;
            match trust {
                // -- WP P2CFIXB --
                Trust::Ca(certs) => builder
                    .dangerous()
                    .with_custom_certificate_verifier(Arc::new(CaPemVerifier {
                        exact: certs.clone(),
                        webpki: verifier,
                    })),
                _ => builder.with_webpki_verifier(verifier),
            }
        }
        (None, Trust::Pin(pin)) => builder
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(PinVerifier {
                pin: *pin,
                algorithms: provider.signature_verification_algorithms,
            })),
        (None, _) => return Err(TlsError::Config("no trust".into())),
    };
    let mut config = match identity {
        None => builder.with_no_client_auth(),
        Some(id) => builder
            .with_client_auth_cert(id.chain.clone(), id.key.clone_key())
            .map_err(|_| TlsError::Config("client_key_pem does not fit client_cert_pem".into()))?,
    };
    config.alpn_protocols = alpn.iter().map(|p| p.to_vec()).collect();
    Ok(Arc::new(config))
}

/// The `ServerName` for a host: an IP literal (IPv6 without brackets) as an address, else a
/// DNS name for SNI and the certificate check.
pub fn server_name(host: &str) -> Result<ServerName<'static>, TlsError> {
    let bare = host
        .strip_prefix('[')
        .and_then(|h| h.strip_suffix(']'))
        .unwrap_or(host);
    if let Ok(ip) = bare.parse::<IpAddr>() {
        return Ok(ServerName::IpAddress(ip.into()));
    }
    ServerName::try_from(host.to_ascii_lowercase()).map_err(|_| TlsError::Name(host.to_owned()))
}

// ── The stream ──────────────────────────────────────────────────────────────

/// A connection to a receiver or broker: plain TCP, or TLS over it.
pub enum Stream {
    /// No TLS.
    Plain(TcpStream),
    /// TLS, the handshake done.
    Tls(Box<StreamOwned<ClientConnection, TcpStream>>),
}

impl fmt::Debug for Stream {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Stream::Plain(_) => "Stream::Plain",
            Stream::Tls(_) => "Stream::Tls",
        })
    }
}

impl Stream {
    /// The TCP socket under it, for timeouts.
    pub fn tcp(&self) -> &TcpStream {
        match self {
            Stream::Plain(s) => s,
            Stream::Tls(s) => &s.sock,
        }
    }

    /// Whether it is TLS.
    pub fn encrypted(&self) -> bool {
        matches!(self, Stream::Tls(_))
    }

    /// Close it: `close_notify` first for TLS (best effort), then the socket.
    pub fn close(&mut self) {
        if let Stream::Tls(s) = self {
            s.conn.send_close_notify();
            let _ = s.conn.write_tls(&mut s.sock);
        }
        let _ = self.tcp().shutdown(Shutdown::Both);
    }
}

/// A TLS read that ended without `close_notify` is a closed connection, not an error.
fn read_eof(e: io::Error) -> io::Result<usize> {
    if e.kind() == io::ErrorKind::UnexpectedEof {
        Ok(0)
    } else {
        Err(e)
    }
}

impl Read for Stream {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self {
            Stream::Plain(s) => s.read(buf),
            Stream::Tls(s) => s.read(buf).or_else(read_eof),
        }
    }
}

impl Write for Stream {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match self {
            Stream::Plain(s) => s.write(buf),
            Stream::Tls(s) => s.write(buf),
        }
    }

    /// For TLS this sends what `write` encrypted: call it after every message.
    fn flush(&mut self) -> io::Result<()> {
        match self {
            Stream::Plain(s) => s.flush(),
            Stream::Tls(s) => s.flush(),
        }
    }
}

/// The reason given when a server presents a CA certificate as its own and it is not the
/// exact certificate given in `ca_pem` (WP P2CFIXB).
pub const CA_AS_END_ENTITY: &str = "the server's certificate is a CA certificate used as its own: give that exact certificate as ca_pem, pin it with pin_sha256, or issue a leaf from it";

fn handshake_error(e: &io::Error, host: &str) -> TlsError {
    match e
        .get_ref()
        .and_then(|inner| inner.downcast_ref::<rustls::Error>())
    {
        Some(rustls::Error::InvalidCertificate(why)) => match why {
            CertificateError::ApplicationVerificationFailure => TlsError::PinMismatch,
            CertificateError::NotValidForName | CertificateError::NotValidForNameContext { .. } => {
                TlsError::Certificate(format!("it is not for {host}"))
            }
            CertificateError::UnknownIssuer => {
                TlsError::Certificate("it is not signed by a trusted CA".into())
            }
            CertificateError::Expired | CertificateError::ExpiredContext { .. } => {
                TlsError::Certificate("it has expired".into())
            }
            CertificateError::NotValidYet | CertificateError::NotValidYetContext { .. } => {
                TlsError::Certificate("it is not valid yet".into())
            }
            // -- WP P2CFIXB --
            CertificateError::Other(o) if format!("{o:?}").contains("CaUsedAsEndEntity") => {
                TlsError::Certificate(CA_AS_END_ENTITY.into())
            }
            other => TlsError::Certificate(format!("{other:?}")),
        },
        Some(other) => TlsError::Handshake(other.to_string()),
        None => match e.kind() {
            io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock => TlsError::Timeout,
            io::ErrorKind::UnexpectedEof => {
                TlsError::Handshake("the server closed the connection".into())
            }
            _ => TlsError::Io(e.to_string()),
        },
    }
}

fn left(deadline: Instant) -> Result<Duration, TlsError> {
    let left = deadline.saturating_duration_since(Instant::now());
    if left.is_zero() {
        Err(TlsError::Timeout)
    } else {
        Ok(left)
    }
}

/// TLS over `tcp` to `host`, the handshake complete by `deadline`. No application byte is
/// written before the server is verified. The socket's timeouts are left at the time left;
/// the caller sets its own.
pub fn connect(
    tcp: TcpStream,
    config: Arc<ClientConfig>,
    host: &str,
    deadline: Instant,
) -> Result<Stream, TlsError> {
    let name = server_name(host)?;
    let mut conn =
        ClientConnection::new(config, name).map_err(|e| TlsError::Handshake(e.to_string()))?;
    let mut tcp = tcp;
    while conn.is_handshaking() {
        let wait = left(deadline)?;
        tcp.set_read_timeout(Some(wait))
            .and_then(|()| tcp.set_write_timeout(Some(wait)))
            .map_err(|e| TlsError::Io(e.to_string()))?;
        match conn.complete_io(&mut tcp) {
            Ok(_) => {}
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => {
                // Tell the server why (an alert rustls queued), best effort.
                let _ = conn.write_tls(&mut tcp);
                let _ = tcp.shutdown(Shutdown::Both);
                return Err(handshake_error(&e, host));
            }
        }
    }
    Ok(Stream::Tls(Box::new(StreamOwned::new(conn, tcp))))
}

// ── Where plaintext may go ──────────────────────────────────────────────────

fn private_v4(ip: Ipv4Addr) -> bool {
    ip.is_loopback() || ip.is_private() || ip.is_link_local()
}

fn private_v6(ip: Ipv6Addr) -> bool {
    let first = ip.segments().first().copied().unwrap_or(0);
    ip.is_loopback()
        // fe80::/10, link-local
        || first & 0xffc0 == 0xfe80
        // fc00::/7, unique local
        || first & 0xfe00 == 0xfc00
}

/// OD-SECe: an `http://` body may go to a loopback, private (RFC 1918, IPv6 ULA) or
/// link-local address, nothing else. An IPv4-mapped IPv6 address is judged as its IPv4.
pub fn plaintext_allowed(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => private_v4(v4),
        IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
            Some(v4) => private_v4(v4),
            None => private_v6(v6),
        },
    }
}

/// A loopback address (an IPv4-mapped one too): where an MQTT password may go in clear
/// without `allow_plaintext_password`.
pub fn is_loopback(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => v4.is_loopback(),
        IpAddr::V6(v6) => {
            v6.is_loopback() || v6.to_ipv4_mapped().is_some_and(|v4| v4.is_loopback())
        }
    }
}
