//! PAKE pairing, `auth.pair2` (WP SECPAKE; PLAN-SEC 4.3 and OD-SECc (3)).
//!
//! A typed PIN sent inside TLS is safe from a listener, not from a machine in the
//! middle: a client pairing for the first time has no pin yet, so it trusts the
//! first key it meets, and a proxy that terminates TLS on both legs reads the PIN
//! and relays it. `auth.pair2` sends no PIN. Both ends run **SPAKE2**
//! (RustCrypto's `spake2`, the Ed25519 group) with the PIN as the password, and
//! confirm the key they agreed on with MACs **bound to the TLS connection**:
//! - the password is SHA-256 of the PIN ([`pin_secret`]), which is what the hub
//!   keeps of a window's PIN; SPAKE2 does not need more entropy than the PIN has;
//! - the identities are the client's name as sent (A) and the hub's `hid` (B);
//! - the confirmation keys come from HKDF-SHA256 over the SPAKE2 key, salted
//!   with the TLS exporter ([`EXPORTER_LABEL`], 32 bytes, RFC 8446 7.5), and
//!   bound to the transcript (hid, name, requested role, both SPAKE2 messages);
//! - **the client confirms first** and the hub answers with its own MAC and the
//!   token only after the client's has verified. A wrong confirmation is a wrong
//!   PIN to the hub's ledgers, so five of them end the window as before.
//!
//! What a machine in the middle gets: a relay of both SPAKE2 messages agrees a
//! key with neither end that it knows, and the MACs it relays were made for the
//! other leg's exporter, so the hub refuses the client's at the first guess. If
//! it plays the hub to the client instead, its one SPAKE2 message commits it to
//! one PIN guess: the client's confirmation lets it test that one guess and no
//! other, offline or not (`tests_wp_secpake.rs` tries).
//!
//! Wire (base64url, no padding; full text in `docs/integrator/hub-api.md`):
//! - `{"t":"auth.pair2","id":1,"step":1,"name":..,"role":..,"pake":<33 bytes>}`
//!   is answered `{"t":"ok","id":1,"pake":<33 bytes>}`;
//! - `{"t":"auth.pair2","id":2,"step":2,"confirm":<32 bytes>}` is answered with
//!   `auth.pair`'s `ok` (`session`, `role`, `token`, `hub_key`, `policy`) plus
//!   `"confirm":<32 bytes>`, the hub's MAC, which the client checks before it
//!   keeps the token.
//!
//! Over TLS only: there is no exporter in plaintext. Native clients use it
//! through the core; browsers cannot reach the exporter and keep `auth.pair`.
//! Nothing here logs, and no `Debug` shows key material.

#![warn(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::arithmetic_side_effects)]

use std::convert::Infallible;
use std::fmt;

use data_encoding::BASE64URL_NOPAD;
use ring::rand::{SecureRandom, SystemRandom};
use ring::{digest, hkdf, hmac};
use spake2::rand_core::{TryCryptoRng, TryRng};
use spake2::{Ed25519Group, Identity, Password, Spake2};

use crate::tls::TlsPipe;

/// The TLS exporter label the pairing is bound to (PLAN-SEC SECPAKE).
pub const EXPORTER_LABEL: &[u8] = b"EXPORTER-dspi-pair";
/// Bytes taken from the exporter.
pub const BINDING_LEN: usize = 32;
/// A SPAKE2 message: a side byte and a compressed Edwards point.
pub const MESSAGE_LEN: usize = 33;
/// A confirmation MAC (HMAC-SHA256).
pub const CONFIRM_LEN: usize = 32;
/// The hub's cap: the message `auth.pair2` is listed under in `hello.caps`.
pub const CAP: &str = "pair2";
/// What every transcript starts with: a new version is a new label.
const TRANSCRIPT_CONTEXT: &[u8] = b"dspi-pair2 v1";
/// HKDF `info` of the client's and the hub's confirmation keys.
const CLIENT_CONFIRM: &[u8] = b"dspi-pair2 confirm client";
const HUB_CONFIRM: &[u8] = b"dspi-pair2 confirm hub";
/// Longest base64url text a field may have before it is decoded.
const MAX_FIELD_TEXT: usize = 64;

/// Why a pairing step failed. None carries key material.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum PakeError {
    /// A SPAKE2 message or a MAC that is not one (length, encoding, point).
    #[error("malformed pairing message")]
    Malformed,
    /// The other end's confirmation does not verify: a wrong PIN, or a machine in
    /// the middle.
    #[error("the pairing confirmation does not match")]
    Mismatch,
    /// The system has no random numbers, or a derivation failed.
    #[error("no pairing key could be made")]
    Internal,
}

/// The TLS exporter of one connection ([`EXPORTER_LABEL`]): what both ends'
/// confirmations are bound to. Secret to the connection; never shown.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Binding([u8; BINDING_LEN]);

impl fmt::Debug for Binding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Binding(<redacted>)")
    }
}

impl Binding {
    /// The binding of `pipe`'s connection; `None` before its handshake finished.
    pub fn of(pipe: &TlsPipe) -> Option<Binding> {
        let mut out = [0u8; BINDING_LEN];
        pipe.export_keying_material(EXPORTER_LABEL, &mut out)?;
        Some(Binding(out))
    }

    /// From bytes an exporter gave (a socket's [`crate::socket::WsSender::tls_binding`],
    /// tests).
    pub fn from_bytes(bytes: [u8; BINDING_LEN]) -> Binding {
        Binding(bytes)
    }

    /// The bytes, for a socket to hold until a pairing needs them.
    pub fn into_bytes(self) -> [u8; BINDING_LEN] {
        self.0
    }
}

/// SHA-256 of the PIN: the SPAKE2 password, and what the hub keeps of a window's
/// PIN, so the hub never needs the PIN itself.
pub fn pin_secret(pin: &str) -> [u8; 32] {
    let d = digest::digest(&digest::SHA256, pin.as_bytes());
    let mut out = [0u8; 32];
    out.copy_from_slice(d.as_ref());
    out
}

/// base64url text of a message or a MAC.
pub fn encode(bytes: &[u8]) -> String {
    BASE64URL_NOPAD.encode(bytes)
}

/// A field of `len` bytes from base64url text (padding tolerated), bounded
/// before it is decoded.
pub fn decode(text: &str, len: usize) -> Result<Vec<u8>, PakeError> {
    let text = text.trim_end_matches('=');
    if text.len() > MAX_FIELD_TEXT {
        return Err(PakeError::Malformed);
    }
    let bytes = BASE64URL_NOPAD.decode(text.as_bytes()).map_err(|_| PakeError::Malformed)?;
    if bytes.len() != len {
        return Err(PakeError::Malformed);
    }
    Ok(bytes)
}

/// What both ends bind: one SHA-256 over length-prefixed parts, so no two
/// different transcripts read the same.
fn transcript(hid: &str, name: &str, role: &str, msg_a: &[u8], msg_b: &[u8]) -> [u8; 32] {
    let mut ctx = digest::Context::new(&digest::SHA256);
    for part in [TRANSCRIPT_CONTEXT, hid.as_bytes(), name.as_bytes(), role.as_bytes(), msg_a, msg_b] {
        let len = u64::try_from(part.len()).unwrap_or(u64::MAX);
        ctx.update(&len.to_be_bytes());
        ctx.update(part);
    }
    let mut out = [0u8; 32];
    out.copy_from_slice(ctx.finish().as_ref());
    out
}

/// The client's and the hub's confirmation keys, from the SPAKE2 key `k`.
fn confirm_keys(k: &[u8], binding: &Binding, transcript: &[u8; 32]) -> Result<(hmac::Key, hmac::Key), PakeError> {
    let prk = hkdf::Salt::new(hkdf::HKDF_SHA256, &binding.0).extract(k);
    let key = |label: &[u8]| -> Result<hmac::Key, PakeError> {
        let info = [label, transcript.as_slice()];
        let okm = prk.expand(&info, hmac::HMAC_SHA256).map_err(|_| PakeError::Internal)?;
        Ok(hmac::Key::from(okm))
    };
    Ok((key(CLIENT_CONFIRM)?, key(HUB_CONFIRM)?))
}

fn mac(key: &hmac::Key, transcript: &[u8; 32]) -> [u8; CONFIRM_LEN] {
    let tag = hmac::sign(key, transcript);
    let mut out = [0u8; CONFIRM_LEN];
    out.copy_from_slice(tag.as_ref());
    out
}

/// In constant time (ring's HMAC verification).
fn verify(key: &hmac::Key, transcript: &[u8; 32], presented: &[u8]) -> Result<(), PakeError> {
    hmac::verify(key, transcript, presented).map_err(|_| PakeError::Mismatch)
}

// ---------------------------------------------------------------------------
// Randomness

/// The scalar's randomness for `spake2`, whose RNG interface cannot fail: 32
/// bytes from the OS (which can, and is asked first), stretched with SHA-256 in
/// counter mode should more be asked for than one block.
struct Drbg {
    seed: [u8; 32],
    counter: u64,
}

impl Drbg {
    fn from_os() -> Result<Drbg, PakeError> {
        let mut seed = [0u8; 32];
        SystemRandom::new().fill(&mut seed).map_err(|_| PakeError::Internal)?;
        Ok(Drbg { seed, counter: 0 })
    }

    #[cfg(test)]
    fn from_seed(seed: [u8; 32]) -> Drbg {
        Drbg { seed, counter: 0 }
    }

    fn block(&mut self) -> [u8; 32] {
        let mut ctx = digest::Context::new(&digest::SHA256);
        ctx.update(&self.seed);
        ctx.update(&self.counter.to_be_bytes());
        self.counter = self.counter.wrapping_add(1);
        let mut out = [0u8; 32];
        out.copy_from_slice(ctx.finish().as_ref());
        out
    }
}

impl TryRng for Drbg {
    type Error = Infallible;

    fn try_next_u32(&mut self) -> Result<u32, Infallible> {
        let mut b = [0u8; 4];
        self.try_fill_bytes(&mut b)?;
        Ok(u32::from_le_bytes(b))
    }

    fn try_next_u64(&mut self) -> Result<u64, Infallible> {
        let mut b = [0u8; 8];
        self.try_fill_bytes(&mut b)?;
        Ok(u64::from_le_bytes(b))
    }

    fn try_fill_bytes(&mut self, dst: &mut [u8]) -> Result<(), Infallible> {
        for chunk in dst.chunks_mut(32) {
            let block = self.block();
            for (d, s) in chunk.iter_mut().zip(block) {
                *d = s;
            }
        }
        Ok(())
    }
}

impl TryCryptoRng for Drbg {}

// ---------------------------------------------------------------------------
// The client

/// The client's side between its message and the hub's.
pub struct ClientPake {
    spake: Spake2<Ed25519Group>,
    msg_a: Vec<u8>,
    hid: String,
    name: String,
    role: String,
    binding: Binding,
}

impl fmt::Debug for ClientPake {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ClientPake").field("hid", &self.hid).finish_non_exhaustive()
    }
}

impl ClientPake {
    /// Step 1: the message to send as `pake`, for `pin` typed by the user, to hub
    /// `hid`, as `name` asking `role` (`""`: none), on the connection `binding`.
    pub fn start(
        pin: &str,
        hid: &str,
        name: &str,
        role: &str,
        binding: Binding,
    ) -> Result<(ClientPake, Vec<u8>), PakeError> {
        let mut rng = Drbg::from_os()?;
        Ok(Self::start_with(pin, hid, name, role, binding, &mut rng))
    }

    fn start_with(
        pin: &str,
        hid: &str,
        name: &str,
        role: &str,
        binding: Binding,
        rng: &mut Drbg,
    ) -> (ClientPake, Vec<u8>) {
        let password = Password::new(pin_secret(pin));
        let (spake, msg_a) = Spake2::<Ed25519Group>::start_a_with_rng(
            &password,
            &Identity::new(name.as_bytes()),
            &Identity::new(hid.as_bytes()),
            rng,
        );
        let client = ClientPake {
            spake,
            msg_a: msg_a.clone(),
            hid: hid.to_owned(),
            name: name.to_owned(),
            role: role.to_owned(),
            binding,
        };
        (client, msg_a)
    }

    /// Step 2: from the hub's `pake`, the confirmation to send, and what the hub's
    /// must be.
    pub fn finish(self, msg_b: &[u8]) -> Result<ClientConfirm, PakeError> {
        if msg_b.len() != MESSAGE_LEN {
            return Err(PakeError::Malformed);
        }
        let k = self.spake.finish(msg_b).map_err(|_| PakeError::Malformed)?;
        let t = transcript(&self.hid, &self.name, &self.role, &self.msg_a, msg_b);
        let (client_key, hub_key) = confirm_keys(&k, &self.binding, &t)?;
        Ok(ClientConfirm { confirm: mac(&client_key, &t), hub_key, transcript: t })
    }
}

/// The client's confirmation, and the check of the hub's.
pub struct ClientConfirm {
    confirm: [u8; CONFIRM_LEN],
    hub_key: hmac::Key,
    transcript: [u8; 32],
}

impl fmt::Debug for ClientConfirm {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ClientConfirm(<redacted>)")
    }
}

impl ClientConfirm {
    /// What to send as `confirm`.
    pub fn confirm(&self) -> &[u8] {
        &self.confirm
    }

    /// The hub's `confirm`: it agreed the same key on the same connection.
    pub fn verify_hub(&self, presented: &[u8]) -> Result<(), PakeError> {
        verify(&self.hub_key, &self.transcript, presented)
    }
}

// ---------------------------------------------------------------------------
// The hub

/// The hub's side between its answer and the client's confirmation.
pub struct HubPake {
    client_key: hmac::Key,
    hub_key: hmac::Key,
    transcript: [u8; 32],
}

impl fmt::Debug for HubPake {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("HubPake(<redacted>)")
    }
}

impl HubPake {
    /// Step 1 on the hub: the answer's `pake` to the client's `msg_a`, with
    /// `secret` the window's [`pin_secret`] (or anything, with no window: the
    /// confirmation then fails as a wrong PIN does).
    pub fn respond(
        secret: &[u8; 32],
        hid: &str,
        name: &str,
        role: &str,
        msg_a: &[u8],
        binding: Binding,
    ) -> Result<(HubPake, Vec<u8>), PakeError> {
        let mut rng = Drbg::from_os()?;
        Self::respond_with(secret, hid, name, role, msg_a, binding, &mut rng)
    }

    fn respond_with(
        secret: &[u8; 32],
        hid: &str,
        name: &str,
        role: &str,
        msg_a: &[u8],
        binding: Binding,
        rng: &mut Drbg,
    ) -> Result<(HubPake, Vec<u8>), PakeError> {
        if msg_a.len() != MESSAGE_LEN {
            return Err(PakeError::Malformed);
        }
        let password = Password::new(secret);
        let (spake, msg_b) = Spake2::<Ed25519Group>::start_b_with_rng(
            &password,
            &Identity::new(name.as_bytes()),
            &Identity::new(hid.as_bytes()),
            rng,
        );
        let k = spake.finish(msg_a).map_err(|_| PakeError::Malformed)?;
        let t = transcript(hid, name, role, msg_a, &msg_b);
        let (client_key, hub_key) = confirm_keys(&k, &binding, &t)?;
        Ok((HubPake { client_key, hub_key, transcript: t }, msg_b))
    }

    /// Step 2 on the hub: the client's `confirm` checked (in constant time), and
    /// the hub's own to send with the token. [`PakeError::Mismatch`] is a wrong PIN.
    pub fn confirm(&self, presented: &[u8]) -> Result<[u8; CONFIRM_LEN], PakeError> {
        verify(&self.client_key, &self.transcript, presented)?;
        Ok(mac(&self.hub_key, &self.transcript))
    }
}

#[cfg(test)]
#[path = "pake/tests_wp_secpake.rs"]
mod tests_wp_secpake;
