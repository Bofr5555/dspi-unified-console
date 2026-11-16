//! TLS 1.3 for DSPi Link (WP SECPIPE; PLAN-SEC sections 4.1 to 4.3).
//!
//! What is here, for the hub's accept path (SECSRV) and the core's connect rule
//! (SECCORE) to use:
//! - [`server_config`] and [`connect`]'s client config: TLS 1.3 only, rustls with
//!   the ring provider, ALPN `http/1.1`, no session tickets, no resumption, no
//!   0-RTT, no client certificates.
//! - [`PinVerifier`]: the client's certificate check. It verifies the handshake
//!   signature with the presented key and compares the key's SPKI hash with the
//!   pin, and nothing else: no chain, no names, **no dates** (a hub with no clock
//!   boots in 1970, and a pin needs none). Three [`PinMode`]s.
//! - [`verify_rotation`]: the old key's signature over a new key, for
//!   `hello.hub.key_next` (section 4.2).
//! - [`TlsPipe`]: one TLS connection shared by a reading and a writing half, as
//!   [`crate::socket::Pipe`] shares one `TcpStream`, with the same semantics
//!   (handshake deadline, buffered writes put on the wire whole on `flush`, the
//!   broken flag).
//! - [`sniff_tls`]: does a connection start with a TLS handshake record.
//! - [`SpkiHash`]: the pin, with its display forms.
//!
//! **Locking.** A `rustls` connection cannot be cloned, so both halves share it
//! behind one lock, and **no socket I/O ever happens while that lock is held**
//! (review S2). A reader blocks on the socket outside it and takes it only to
//! feed rustls the bytes and take the plaintext out. A writer takes it only to
//! encrypt onto a shared queue, then writes the queue to the socket under a
//! second lock, the write lock, alone. Otherwise a writer blocked on a full
//! socket would hold the connection, the reader could not decrypt, and two peers
//! that both write past their socket buffers would deadlock. Ciphertext must
//! reach the wire in the order rustls made it, so it goes through the one queue,
//! filled under the connection lock and emptied only under the write lock.
//! Records the reader makes (a KeyUpdate answer, an alert) are sent by the
//! reader only when no writer has bytes waiting: a reader never blocks behind
//! a writer's data, and a waiting writer sends them with its own.
//!
//! The socket's write timeout (set by the caller, 10 s on both ends today) is the
//! write-stall deadline: a write that cannot finish marks the pipe broken.

#![warn(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::arithmetic_side_effects)]

use std::fmt;
use std::io::{self, Read, Write};
use std::net::{Shutdown, SocketAddr, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, TryLockError};
use std::time::Duration;

use data_encoding::{BASE64, BASE64URL_NOPAD, HEXLOWER, HEXLOWER_PERMISSIVE};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::client::Resumption;
use rustls::crypto::{CryptoProvider, WebPkiSupportedAlgorithms};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, ServerName, UnixTime};
use rustls::server::{ClientHello, NoServerSessionStorage, ResolvesServerCert};
use rustls::sign::CertifiedKey;
use rustls::{
    CertificateError, ClientConfig, ClientConnection, Connection, DigitallySignedStruct, ServerConfig,
    ServerConnection, SignatureScheme,
};

use crate::clock::Instant;

/// The first byte of a TLS handshake record (a ClientHello starts with it).
pub const TLS_HANDSHAKE_RECORD: u8 = 0x16;
/// The one application protocol on the Link port.
pub const ALPN_HTTP11: &[u8] = b"http/1.1";
/// What the old key signs before the hub id and the new key's hash (section 4.2).
pub const ROTATION_CONTEXT: &[u8] = b"dspi-key-rotate\0";

/// Ciphertext read from the socket at a time: one full TLS record and its header.
const READ_CHUNK: usize = 16 * 1024 + 512;
/// Ciphertext rustls has not taken yet. It only grows while rustls is being fed,
/// so this is a guard against a bug, not a limit a peer can reach.
const MAX_PENDING_IN: usize = 4 * READ_CHUNK;

// ---------------------------------------------------------------------------
// The pin

/// SHA-256 of a key's DER SubjectPublicKeyInfo, outer SEQUENCE included: the pin
/// of RFC 7469 and `curl --pinnedpubkey`. The certificate can be re-issued with
/// the same key and the pin stays. Not a secret.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct SpkiHash(pub [u8; 32]);

/// Crockford's base32: no I, L, O or U, so it reads aloud and copies by hand.
const CROCKFORD: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";

impl SpkiHash {
    /// The hash of a DER SubjectPublicKeyInfo (outer SEQUENCE included).
    pub fn of_spki(spki_der: &[u8]) -> SpkiHash {
        let digest = ring::digest::digest(&ring::digest::SHA256, spki_der);
        let mut out = [0u8; 32];
        // A SHA-256 digest is 32 bytes, always.
        out.copy_from_slice(digest.as_ref());
        SpkiHash(out)
    }

    /// The hash of the key in a DER X.509 certificate. Parses the certificate
    /// only: no signature, name or date is checked.
    pub fn of_cert(cert_der: &[u8]) -> Result<SpkiHash, TlsError> {
        let der = CertificateDer::from(cert_der);
        let cert = webpki::EndEntityCert::try_from(&der).map_err(|_| TlsError::BadCertificate)?;
        Ok(SpkiHash::of_spki(cert.subject_public_key_info().as_ref()))
    }

    /// For people: `SHA256:` and the first 80 bits in Crockford base32, four
    /// groups of four (`SHA256:K7Q2-M9XD-4TNB-0ZRA`).
    pub fn display(&self) -> String {
        let mut chars = String::with_capacity(16);
        for five in self.0.chunks(5).take(2) {
            let bits = five.iter().fold(0u64, |acc, b| acc.wrapping_shl(8) | u64::from(*b));
            for i in (0..8u32).rev() {
                let index = bits.checked_shr(i.wrapping_mul(5)).unwrap_or(0) & 31;
                chars.push(CROCKFORD.get(index as usize).map_or('?', |c| char::from(*c)));
            }
        }
        let mut out = String::from("SHA256:");
        for (i, c) in chars.chars().enumerate() {
            if i > 0 && i.is_multiple_of(4) {
                out.push('-');
            }
            out.push(c);
        }
        out
    }

    /// `sha256//<base64>`, for `curl --pinnedpubkey`.
    pub fn curl_pin(&self) -> String {
        format!("sha256//{}", BASE64.encode(&self.0))
    }

    /// The full hash, lowercase hex.
    pub fn to_hex(&self) -> String {
        HEXLOWER.encode(&self.0)
    }

    /// From [`Self::to_hex`] (either case).
    pub fn from_hex(text: &str) -> Option<SpkiHash> {
        Self::from_bytes(&HEXLOWER_PERMISSIVE.decode(text.trim().as_bytes()).ok()?)
    }

    /// base64url without padding (43 characters), for QR codes and credentials.
    pub fn to_b64url(&self) -> String {
        BASE64URL_NOPAD.encode(&self.0)
    }

    /// From [`Self::to_b64url`].
    pub fn from_b64url(text: &str) -> Option<SpkiHash> {
        Self::from_bytes(&BASE64URL_NOPAD.decode(text.trim().as_bytes()).ok()?)
    }

    fn from_bytes(bytes: &[u8]) -> Option<SpkiHash> {
        <[u8; 32]>::try_from(bytes).ok().map(SpkiHash)
    }

    /// Equal, in time independent of where they differ.
    pub fn ct_eq(&self, other: &SpkiHash) -> bool {
        self.0.iter().zip(other.0.iter()).fold(0u8, |acc, (a, b)| acc | (a ^ b)) == 0
    }
}

impl fmt::Debug for SpkiHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "SpkiHash({})", self.display())
    }
}

impl fmt::Display for SpkiHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.display())
    }
}

// ---------------------------------------------------------------------------
// Errors

/// Why a TLS connection could not be set up. Never carries key material.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TlsError {
    /// [`PinMode::Pinned`]: the hub presented another key. Nothing was sent.
    #[error("this hub's key is not the one it was paired with")]
    KeyMismatch,
    /// The peer closed or reset the connection during the handshake: no TLS here
    /// (an `off` hub), or something in between cut it.
    #[error("the TLS handshake was refused")]
    Refused,
    /// The handshake failed: a bad signature (the peer does not hold the key it
    /// presented), an alert, or bytes that are not TLS.
    #[error("TLS handshake failed: {0}")]
    Handshake(String),
    /// The handshake did not finish by its deadline.
    #[error("the TLS handshake did not finish in time")]
    Timeout,
    #[error("network error: {0}")]
    Io(String),
    /// A certificate that cannot be parsed.
    #[error("the certificate cannot be read")]
    BadCertificate,
    /// The key and certificate given to [`server_config`] are unusable or do not
    /// match.
    #[error("TLS configuration: {0}")]
    Config(String),
    /// Not a DNS name or an IP address.
    #[error("not a host name TLS can use: {0}")]
    ServerName(String),
}

impl TlsError {
    fn from_rustls(e: &rustls::Error) -> TlsError {
        match e {
            rustls::Error::InvalidCertificate(CertificateError::ApplicationVerificationFailure) => {
                TlsError::KeyMismatch
            }
            other => TlsError::Handshake(other.to_string()),
        }
    }

    fn from_io(e: &io::Error) -> TlsError {
        match e.kind() {
            io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut => TlsError::Timeout,
            io::ErrorKind::ConnectionReset
            | io::ErrorKind::ConnectionAborted
            | io::ErrorKind::BrokenPipe
            | io::ErrorKind::UnexpectedEof => TlsError::Refused,
            _ => TlsError::Io(e.to_string()),
        }
    }
}

// ---------------------------------------------------------------------------
// Configs and the verifier

fn provider() -> Arc<CryptoProvider> {
    Arc::new(rustls::crypto::ring::default_provider())
}

fn config_err(e: rustls::Error) -> TlsError {
    TlsError::Config(e.to_string())
}

/// The hub's TLS config: TLS 1.3 only, ALPN `http/1.1`, no tickets, no session
/// cache, no early data, no client certificates. `key` is the PKCS#8 (or SEC1)
/// key of the leaf certificate, which must match it.
pub fn server_config(
    cert_chain: Vec<CertificateDer<'static>>,
    key: PrivateKeyDer<'static>,
) -> Result<Arc<ServerConfig>, TlsError> {
    let provider = provider();
    let certified = CertifiedKey::from_der(cert_chain, key, &provider).map_err(config_err)?;
    build_server_config(provider, Arc::new(OneKey(Arc::new(certified))))
}

fn build_server_config(
    provider: Arc<CryptoProvider>,
    resolver: Arc<dyn ResolvesServerCert>,
) -> Result<Arc<ServerConfig>, TlsError> {
    let mut config = ServerConfig::builder_with_provider(provider)
        .with_protocol_versions(&[&rustls::version::TLS13])
        .map_err(config_err)?
        .with_no_client_auth()
        .with_cert_resolver(resolver);
    config.alpn_protocols = vec![ALPN_HTTP11.to_vec()];
    config.session_storage = Arc::new(NoServerSessionStorage {});
    config.send_tls13_tickets = 0;
    config.max_early_data_size = 0;
    config.send_half_rtt_data = false;
    Ok(Arc::new(config))
}

/// The client config for one connection: TLS 1.3 only, ALPN `http/1.1`, no
/// resumption, no early data, and [`PinVerifier`] in `mode` as the only check.
pub fn client_config(mode: PinMode) -> Result<Arc<ClientConfig>, TlsError> {
    let provider = provider();
    let verifier = Arc::new(PinVerifier::new(mode, &provider));
    let mut config = ClientConfig::builder_with_provider(provider)
        .with_protocol_versions(&[&rustls::version::TLS13])
        .map_err(config_err)?
        .dangerous()
        .with_custom_certificate_verifier(verifier)
        .with_no_client_auth();
    config.alpn_protocols = vec![ALPN_HTTP11.to_vec()];
    config.resumption = Resumption::disabled();
    config.enable_early_data = false;
    Ok(Arc::new(config))
}

/// Serves one key for every name.
struct OneKey(Arc<CertifiedKey>);

/// By hand: says nothing about the key.
impl fmt::Debug for OneKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("OneKey")
    }
}

impl ResolvesServerCert for OneKey {
    fn resolve(&self, _hello: ClientHello<'_>) -> Option<Arc<CertifiedKey>> {
        Some(Arc::clone(&self.0))
    }
}

/// What the client trusts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PinMode {
    /// The key must be this one; anything else fails the handshake with
    /// [`TlsError::KeyMismatch`] before any application byte.
    Pinned(SpkiHash),
    /// Any key (first contact or a v1 record): [`PinOutcome::FirstUse`] reports it.
    FirstUse,
    /// The pinned key, or any other one reported as [`PinOutcome::Rotated`]: the
    /// caller withholds the token and checks `hello.hub.key_next` with
    /// [`verify_rotation`] (section 4.2, review M1).
    Rotating(SpkiHash),
}

/// The key a successful handshake met.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PinOutcome {
    /// The pinned key ([`PinMode::Pinned`], or [`PinMode::Rotating`] with no change).
    Matched(SpkiHash),
    /// [`PinMode::FirstUse`]: this key, trusted on first use if the caller decides so.
    FirstUse(SpkiHash),
    /// [`PinMode::Rotating`]: another key than the pinned one. Not trusted yet: the
    /// token must not be sent until [`verify_rotation`] accepts it.
    Rotated { pinned: SpkiHash, presented: SpkiHash },
}

impl PinOutcome {
    /// The key the hub presented.
    pub fn presented(&self) -> SpkiHash {
        match *self {
            PinOutcome::Matched(k) | PinOutcome::FirstUse(k) => k,
            PinOutcome::Rotated { presented, .. } => presented,
        }
    }
}

impl PinMode {
    fn outcome(self, presented: SpkiHash) -> Result<PinOutcome, TlsError> {
        match self {
            PinMode::Pinned(pin) if pin.ct_eq(&presented) => Ok(PinOutcome::Matched(pin)),
            PinMode::Pinned(_) => Err(TlsError::KeyMismatch),
            PinMode::FirstUse => Ok(PinOutcome::FirstUse(presented)),
            PinMode::Rotating(pin) if pin.ct_eq(&presented) => Ok(PinOutcome::Matched(pin)),
            PinMode::Rotating(pinned) => Ok(PinOutcome::Rotated { pinned, presented }),
        }
    }
}

/// The client's certificate check: the SPKI pin and the handshake signature,
/// nothing else. notBefore and notAfter are never looked at, nor the chain or
/// the names: the pin is the identity.
#[derive(Debug)]
pub struct PinVerifier {
    mode: PinMode,
    algorithms: WebPkiSupportedAlgorithms,
}

impl PinVerifier {
    pub fn new(mode: PinMode, provider: &CryptoProvider) -> PinVerifier {
        PinVerifier { mode, algorithms: provider.signature_verification_algorithms }
    }
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
        let presented = SpkiHash::of_cert(end_entity)
            .map_err(|_| rustls::Error::InvalidCertificate(CertificateError::BadEncoding))?;
        match self.mode.outcome(presented) {
            Ok(_) => Ok(ServerCertVerified::assertion()),
            // The one place this error is made: TlsError::from_rustls reads it as KeyMismatch.
            Err(_) => Err(rustls::Error::InvalidCertificate(CertificateError::ApplicationVerificationFailure)),
        }
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        // The configs allow TLS 1.3 only.
        Err(rustls::Error::General("TLS 1.2 is not used on DSPi Link".into()))
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

// ---------------------------------------------------------------------------
// Key rotation (section 4.2)

/// Why [`verify_rotation`] refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum RotationError {
    /// The old key sent with `key_next` is not the pinned one.
    #[error("the announced old key is not the pinned key")]
    NotPinnedKey,
    /// The old key is not an ECDSA P-256 key.
    #[error("the old key is not a P-256 key")]
    UnsupportedKey,
    /// The signature does not verify.
    #[error("the key rotation signature does not verify")]
    BadSignature,
}

/// The DER SubjectPublicKeyInfo header of every uncompressed P-256 key:
/// SEQUENCE { SEQUENCE { id-ecPublicKey, prime256v1 }, BIT STRING (0 unused) }.
const P256_SPKI_PREFIX: [u8; 26] = [
    0x30, 0x59, 0x30, 0x13, 0x06, 0x07, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x02, 0x01, 0x06, 0x08, 0x2a, 0x86, 0x48, 0xce,
    0x3d, 0x03, 0x01, 0x07, 0x03, 0x42, 0x00,
];

/// The uncompressed point of a P-256 SubjectPublicKeyInfo.
fn p256_point(spki_der: &[u8]) -> Option<&[u8]> {
    let point = spki_der.strip_prefix(&P256_SPKI_PREFIX[..])?;
    (point.len() == 65 && point.first() == Some(&0x04)).then_some(point)
}

/// What the old key signs: `"dspi-key-rotate\0" || hid || new_spki`, the hub id's
/// UTF-8 bytes and the new key's 32-byte [`SpkiHash`]. The hash comes last at a
/// fixed length, so the split is unambiguous.
pub fn rotation_message(hid: &str, new_spki: &SpkiHash) -> Vec<u8> {
    let mut msg = Vec::with_capacity(ROTATION_CONTEXT.len().saturating_add(hid.len()).saturating_add(32));
    msg.extend_from_slice(ROTATION_CONTEXT);
    msg.extend_from_slice(hid.as_bytes());
    msg.extend_from_slice(&new_spki.0);
    msg
}

/// Whether the pinned key signed the move of hub `hid` to `new_spki`. A client
/// keeps only the pin, a hash, so the hub sends the old key's DER
/// SubjectPublicKeyInfo with the signature (`old_spki_der`); it must hash to
/// `pinned`. `sig` is ECDSA P-256 with SHA-256, ASN.1 DER, as TLS uses.
pub fn verify_rotation(
    pinned: &SpkiHash,
    old_spki_der: &[u8],
    hid: &str,
    new_spki: &SpkiHash,
    sig: &[u8],
) -> Result<(), RotationError> {
    if !SpkiHash::of_spki(old_spki_der).ct_eq(pinned) {
        return Err(RotationError::NotPinnedKey);
    }
    let point = p256_point(old_spki_der).ok_or(RotationError::UnsupportedKey)?;
    ring::signature::UnparsedPublicKey::new(&ring::signature::ECDSA_P256_SHA256_ASN1, point)
        .verify(&rotation_message(hid, new_spki), sig)
        .map_err(|_| RotationError::BadSignature)
}

// ---------------------------------------------------------------------------
// The first byte

/// Whether the connection's first byte starts a TLS handshake record, without
/// consuming it. Waits for a byte as long as the socket's read timeout allows
/// (an error then, `WouldBlock` or `TimedOut`); a peer that closes before
/// sending anything is `UnexpectedEof`.
pub fn sniff_tls(tcp: &TcpStream) -> io::Result<bool> {
    let mut first = [0u8; 1];
    match tcp.peek(&mut first)? {
        0 => Err(io::ErrorKind::UnexpectedEof.into()),
        _ => Ok(first[0] == TLS_HANDSHAKE_RECORD),
    }
}

// ---------------------------------------------------------------------------
// The pipe

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

/// The connection and what is queued around it. Behind [`Shared::inner`]; never
/// held across socket I/O.
struct Inner {
    conn: Connection,
    /// Ciphertext read from the socket that rustls has not taken yet.
    pending_in: Vec<u8>,
    /// Ciphertext rustls made, in order, for the holder of the write lock to send.
    outq: Vec<u8>,
    /// Writers whose bytes are on `outq` and who will send them (with whatever
    /// follows). A reader leaves the queue to them.
    writers: usize,
    /// The first TLS error: the connection is dead after it.
    failed: Option<rustls::Error>,
}

impl Inner {
    /// Feeds pending ciphertext to rustls while it takes it.
    fn pump(&mut self) -> Result<(), rustls::Error> {
        if let Some(e) = &self.failed {
            return Err(e.clone());
        }
        while !self.pending_in.is_empty() && self.conn.wants_read() {
            let mut rd: &[u8] = &self.pending_in;
            let taken = match self.conn.read_tls(&mut rd) {
                Ok(0) | Err(_) => break,
                Ok(n) => n.min(self.pending_in.len()),
            };
            self.pending_in.drain(..taken);
            if let Err(e) = self.conn.process_new_packets() {
                // rustls has queued the alert; collect() sends it
                self.failed = Some(e.clone());
                return Err(e);
            }
        }
        Ok(())
    }

    /// Moves what rustls has to send onto `outq`.
    fn collect(&mut self) {
        while self.conn.wants_write() {
            match self.conn.write_tls(&mut self.outq) {
                Ok(0) | Err(_) => break,
                Ok(_) => {}
            }
        }
    }
}

struct Shared {
    inner: Mutex<Inner>,
    /// Held while writing to the socket, and only then. Never taken with `inner`
    /// held for longer than a queue swap.
    wlock: Mutex<()>,
    /// A socket write failed part way: a record may be cut in two on the wire, so
    /// the stream can never be trusted again. As [`crate::socket::Pipe`]'s.
    broken: AtomicBool,
}

impl Shared {
    /// Writes `outq` to the socket. `writer`: the caller counted itself into
    /// `writers` when it queued its bytes, and waits for the write lock; the count
    /// is released in the same critical section that finds the queue empty, so no
    /// byte a reader leaves behind is ever stranded. Otherwise (a reader) only
    /// tries the lock, and sends only when no writer has bytes queued.
    fn drain(&self, tcp: &TcpStream, writer: bool) -> io::Result<()> {
        let mut counted = writer;
        let result = 'drain: loop {
            let guard = if counted {
                lock(&self.wlock)
            } else {
                match self.wlock.try_lock() {
                    Ok(g) => g,
                    Err(TryLockError::Poisoned(p)) => p.into_inner(),
                    Err(TryLockError::WouldBlock) => break Ok(()),
                }
            };
            loop {
                let chunk = {
                    let mut g = lock(&self.inner);
                    if !counted && g.writers > 0 {
                        Vec::new() // theirs to send, with ours behind it
                    } else {
                        if g.outq.is_empty() && counted {
                            g.writers = g.writers.saturating_sub(1);
                            counted = false;
                        }
                        std::mem::take(&mut g.outq)
                    }
                };
                if chunk.is_empty() {
                    break;
                }
                if self.broken.load(Ordering::SeqCst) {
                    break 'drain Err(io::ErrorKind::BrokenPipe.into());
                }
                let mut socket: &TcpStream = tcp;
                if let Err(e) = socket.write_all(&chunk) {
                    self.broken.store(true, Ordering::SeqCst);
                    break 'drain Err(e);
                }
            }
            drop(guard);
            // Queued after our last look, by someone who left it to us
            let g = lock(&self.inner);
            if g.writers > 0 || g.outq.is_empty() {
                break Ok(());
            }
        };
        if counted {
            let mut g = lock(&self.inner);
            g.writers = g.writers.saturating_sub(1);
        }
        if writer && result.is_ok() && self.broken.load(Ordering::SeqCst) {
            // Another holder failed to send what we queued
            return Err(io::ErrorKind::BrokenPipe.into());
        }
        result
    }
}

/// One half of a TLS connection: [`Read`] and [`Write`] over the shared
/// `rustls` connection, with [`crate::socket::Pipe`]'s semantics. Make the
/// second half with [`TlsPipe::try_clone`].
pub struct TlsPipe {
    tcp: TcpStream,
    shared: Arc<Shared>,
    buffered: bool,
    /// Plaintext collected by `write` until `flush`, when `buffered`.
    out: Vec<u8>,
    /// Reads end here however slowly bytes trickle in (the handshake, and
    /// whatever the caller sets it for).
    deadline: Option<Instant>,
    rbuf: Vec<u8>,
}

/// By hand: the buffers may hold an `auth.token`, and never reach a log.
impl fmt::Debug for TlsPipe {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TlsPipe")
            .field("peer", &self.tcp.peer_addr().ok())
            .field("buffered", &self.buffered)
            .field("pending_out", &self.out.len())
            .field("broken", &self.is_broken())
            .finish()
    }
}

/// A TLS client handshake on `tcp` to `host` (a DNS name or an IP address,
/// brackets allowed). Fails by `deadline`, and with [`TlsError::KeyMismatch`]
/// for a pinned record and another key, before any application byte. The
/// returned pipe is unbuffered and keeps `deadline` for its reads (the WebSocket
/// upgrade usually shares it): clear it with [`TlsPipe::set_deadline`]. The
/// socket's write timeout is the caller's to set first.
pub fn connect(
    tcp: TcpStream,
    host: &str,
    mode: PinMode,
    deadline: Instant,
) -> Result<(TlsPipe, PinOutcome), TlsError> {
    let bare = host.strip_prefix('[').and_then(|h| h.strip_suffix(']')).unwrap_or(host);
    let name = ServerName::try_from(bare.to_owned()).map_err(|_| TlsError::ServerName(bare.to_owned()))?;
    let conn = ClientConnection::new(client_config(mode)?, name).map_err(config_err)?;
    let mut pipe = TlsPipe::new(tcp, Connection::Client(conn), deadline);
    pipe.handshake()?;
    let presented = {
        let g = lock(&pipe.shared.inner);
        let leaf = g.conn.peer_certificates().and_then(|c| c.first()).ok_or(TlsError::BadCertificate)?;
        SpkiHash::of_cert(leaf)?
    };
    let outcome = mode.outcome(presented)?;
    Ok((pipe, outcome))
}

/// A TLS server handshake on `tcp`, by `deadline`. As [`connect`]: unbuffered,
/// the deadline kept for reads (the request head usually shares it).
pub fn accept(tcp: TcpStream, config: Arc<ServerConfig>, deadline: Instant) -> Result<TlsPipe, TlsError> {
    let conn = ServerConnection::new(config).map_err(config_err)?;
    let mut pipe = TlsPipe::new(tcp, Connection::Server(conn), deadline);
    pipe.handshake()?;
    Ok(pipe)
}

impl TlsPipe {
    fn new(tcp: TcpStream, conn: Connection, deadline: Instant) -> TlsPipe {
        let inner = Inner { conn, pending_in: Vec::new(), outq: Vec::new(), writers: 0, failed: None };
        TlsPipe {
            tcp,
            shared: Arc::new(Shared {
                inner: Mutex::new(inner),
                wlock: Mutex::new(()),
                broken: AtomicBool::new(false),
            }),
            buffered: false,
            out: Vec::new(),
            deadline: Some(deadline),
            rbuf: Vec::new(),
        }
    }

    fn handshake(&mut self) -> Result<(), TlsError> {
        loop {
            let (handshaking, failed, queued) = {
                let mut g = lock(&self.shared.inner);
                let failed = g.pump().err();
                g.collect();
                let queued = !g.outq.is_empty();
                if queued {
                    g.writers = g.writers.saturating_add(1);
                }
                (g.conn.is_handshaking(), failed, queued)
            };
            if let Err(e) = self.shared.drain(&self.tcp, queued) {
                // An alert we could not send matters less than why it was sent
                return Err(failed.map_or_else(|| TlsError::from_io(&e), |f| TlsError::from_rustls(&f)));
            }
            if let Some(e) = failed {
                return Err(TlsError::from_rustls(&e));
            }
            if !handshaking {
                return Ok(());
            }
            match self.fill() {
                Ok(true) => {}
                Ok(false) => return Err(TlsError::Refused),
                Err(e) => return Err(TlsError::from_io(&e)),
            }
        }
    }

    /// Reads ciphertext from the socket (outside the connection lock) onto the
    /// pending queue. `false` at the end of the stream, which rustls is told.
    fn fill(&mut self) -> io::Result<bool> {
        if let Some(deadline) = self.deadline {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Err(io::ErrorKind::TimedOut.into());
            }
            self.tcp.set_read_timeout(Some(left))?;
        }
        if self.rbuf.len() != READ_CHUNK {
            self.rbuf.resize(READ_CHUNK, 0);
        }
        let n = self.tcp.read(&mut self.rbuf)?;
        let mut g = lock(&self.shared.inner);
        if n == 0 {
            let mut end: &[u8] = &[];
            let _ = g.conn.read_tls(&mut end);
            return Ok(false);
        }
        if g.pending_in.len() > MAX_PENDING_IN {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "TLS input is not being consumed"));
        }
        g.pending_in.extend_from_slice(self.rbuf.get(..n).unwrap_or_default());
        Ok(true)
    }

    /// Encrypts `data` onto the queue and sends the queue.
    fn send(&mut self, data: &[u8]) -> io::Result<()> {
        if self.shared.broken.load(Ordering::SeqCst) {
            return Err(io::ErrorKind::BrokenPipe.into());
        }
        {
            let mut g = lock(&self.shared.inner);
            if let Some(e) = &g.failed {
                return Err(io::Error::new(io::ErrorKind::InvalidData, e.to_string()));
            }
            let mut rest = data;
            let mut stuck = false;
            while !rest.is_empty() {
                let n = g.conn.writer().write(rest)?;
                // rustls takes data again once its output has been collected
                g.collect();
                if n == 0 {
                    if stuck {
                        return Err(io::Error::other("TLS connection does not take data"));
                    }
                    stuck = true;
                    continue;
                }
                stuck = false;
                rest = rest.get(n..).unwrap_or_default();
            }
            g.writers = g.writers.saturating_add(1);
        }
        self.shared.drain(&self.tcp, true)
    }

    /// A second half of the same connection, on a clone of the socket: one for
    /// the reader, one for the writer. Starts unbuffered, with no deadline.
    pub fn try_clone(&self) -> io::Result<TlsPipe> {
        Ok(TlsPipe {
            tcp: self.tcp.try_clone()?,
            shared: Arc::clone(&self.shared),
            buffered: false,
            out: Vec::new(),
            deadline: None,
            rbuf: Vec::new(),
        })
    }

    /// Buffered: `write` collects and `flush` puts it on the wire whole.
    pub fn set_buffered(&mut self, buffered: bool) {
        self.buffered = buffered;
    }

    /// Reads end at `deadline` however slowly bytes trickle in; `None`: only the
    /// socket's read timeout applies (which the deadline overwrites while set).
    pub fn set_deadline(&mut self, deadline: Option<Instant>) {
        self.deadline = deadline;
    }

    /// A write failed part way, so nothing more can be sent on this connection.
    pub fn is_broken(&self) -> bool {
        self.shared.broken.load(Ordering::SeqCst)
    }

    /// The socket under it (timeouts, shutdown).
    pub fn tcp(&self) -> &TcpStream {
        &self.tcp
    }

    pub fn peer_addr(&self) -> io::Result<SocketAddr> {
        self.tcp.peer_addr()
    }

    pub fn set_read_timeout(&self, timeout: Option<Duration>) -> io::Result<()> {
        self.tcp.set_read_timeout(timeout)
    }

    /// Sends `close_notify` (after any buffered data): the peer then reads the end
    /// of the stream cleanly. Reading goes on until the peer's own.
    pub fn send_close_notify(&mut self) -> io::Result<()> {
        self.flush()?;
        {
            let mut g = lock(&self.shared.inner);
            g.conn.send_close_notify();
            g.collect();
            g.writers = g.writers.saturating_add(1);
        }
        self.shared.drain(&self.tcp, true)
    }

    /// Shuts the socket down, which also ends a read blocked on another half.
    pub fn shutdown(&self) -> io::Result<()> {
        self.tcp.shutdown(Shutdown::Both)
    }

    /// The negotiated ALPN protocol.
    pub fn alpn_protocol(&self) -> Option<Vec<u8>> {
        lock(&self.shared.inner).conn.alpn_protocol().map(<[u8]>::to_vec)
    }

    // -- WP SECPAKE --
    /// RFC 8446 7.5's exporter of this connection: `out.len()` bytes under
    /// `label`, no context. Both ends of one TLS connection get the same bytes;
    /// two connections (a proxy's two legs) never do. `None` before the
    /// handshake has finished or after it failed.
    pub fn export_keying_material(&self, label: &[u8], out: &mut [u8]) -> Option<()> {
        let g = lock(&self.shared.inner);
        if g.failed.is_some() || g.conn.is_handshaking() {
            return None;
        }
        g.conn.export_keying_material(out, label, None).ok().map(|_| ())
    }
}

impl Read for TlsPipe {
    /// Plaintext; `Ok(0)` after the peer's `close_notify`, and `UnexpectedEof`
    /// when the socket ends without one. A read timeout is returned as the
    /// socket's error, with nothing lost.
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        loop {
            let (got, failed) = {
                let mut g = lock(&self.shared.inner);
                let failed = g.pump().err();
                let got = match g.conn.reader().read(buf) {
                    Ok(n) => Some(Ok(n)),
                    Err(e) if e.kind() == io::ErrorKind::WouldBlock => None,
                    Err(e) => Some(Err(e)),
                };
                g.collect();
                (got, failed)
            };
            // A KeyUpdate answer or an alert, unless a writer will send it
            let _ = self.shared.drain(&self.tcp, false);
            if let Some(r) = got {
                return r;
            }
            if let Some(e) = failed {
                return Err(io::Error::new(io::ErrorKind::InvalidData, e.to_string()));
            }
            self.fill()?;
        }
    }
}

impl Write for TlsPipe {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if self.buffered {
            self.out.extend_from_slice(buf);
        } else {
            self.send(buf)?;
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        if self.shared.broken.load(Ordering::SeqCst) {
            self.out.clear();
            return Err(io::ErrorKind::BrokenPipe.into());
        }
        if !self.out.is_empty() {
            let out = std::mem::take(&mut self.out);
            self.send(&out)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests_wp_secpipe;
