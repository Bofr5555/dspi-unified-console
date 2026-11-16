//! -- WP SECCORE -- A TLS identity for the fake hub (PLAN-SEC 4.2): a P-256
//! key, a self-signed certificate, and the rotation announcement. The
//! certificate is written by a small DER writer, as SECPIPE's tests do: rcgen
//! is the real hub's (SECKEY).
#![allow(clippy::arithmetic_side_effects, clippy::indexing_slicing)]

use std::fmt;
use std::sync::Arc;

use data_encoding::BASE64URL_NOPAD;
use ring::rand::SystemRandom;
use ring::signature::{EcdsaKeyPair, KeyPair, ECDSA_P256_SHA256_ASN1_SIGNING};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use rustls::ServerConfig;
use serde_json::{json, Value};

use crate::credential::Pin;
use crate::tls::{rotation_message, server_config, SpkiHash};

/// SEQUENCE { SEQUENCE { id-ecPublicKey, prime256v1 }, BIT STRING (0 unused) }.
const P256_SPKI_PREFIX: [u8; 26] = [
    0x30, 0x59, 0x30, 0x13, 0x06, 0x07, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x02, 0x01, 0x06, 0x08, 0x2a, 0x86, 0x48, 0xce,
    0x3d, 0x03, 0x01, 0x07, 0x03, 0x42, 0x00,
];

/// A hub key, its certificate, and the server config that presents it.
#[derive(Clone)]
pub struct FakeTlsKey {
    pkcs8: Vec<u8>,
    pair: Arc<EcdsaKeyPair>,
    spki: Vec<u8>,
    cert: Vec<u8>,
}

/// By hand: no key material.
impl fmt::Debug for FakeTlsKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "FakeTlsKey({})", SpkiHash(self.pin()).display())
    }
}

impl FakeTlsKey {
    /// A fresh key with a certificate valid 2025 to 2049.
    pub fn generate() -> FakeTlsKey {
        let rng = SystemRandom::new();
        let pkcs8 = EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, &rng)
            .map(|d| d.as_ref().to_vec())
            .unwrap_or_default();
        let pair = EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, &pkcs8, &rng)
            .unwrap_or_else(|e| panic!("a key ring just made: {e}"));
        let mut spki = P256_SPKI_PREFIX.to_vec();
        spki.extend_from_slice(pair.public_key().as_ref());
        let mut key = FakeTlsKey { pkcs8, pair: Arc::new(pair), spki, cert: Vec::new() };
        key.cert = key.self_signed();
        key
    }

    /// The pin: SHA-256 of the SubjectPublicKeyInfo.
    pub fn pin(&self) -> Pin {
        SpkiHash::of_spki(&self.spki).0
    }

    /// The pin as base64url, as `ok.hub_key`, a `k=` and a credential carry it.
    pub fn pin_b64url(&self) -> String {
        BASE64URL_NOPAD.encode(&self.pin())
    }

    /// The DER SubjectPublicKeyInfo.
    pub fn spki_der(&self) -> &[u8] {
        &self.spki
    }

    /// ECDSA P-256 SHA-256, ASN.1 DER.
    pub fn sign(&self, msg: &[u8]) -> Vec<u8> {
        self.pair.sign(&SystemRandom::new(), msg).map(|s| s.as_ref().to_vec()).unwrap_or_default()
    }

    /// The server config presenting this key.
    pub fn server_config(&self) -> Arc<ServerConfig> {
        let key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(self.pkcs8.clone()));
        server_config(vec![CertificateDer::from(self.cert.clone())], key)
            .unwrap_or_else(|e| panic!("the fake hub's TLS config: {e}"))
    }

    /// `hello.hub.key_next` for a move of hub `hid` from this key to `new`, signed
    /// by this key: what a real hub announces after `tls rotate`.
    pub fn key_next_to(&self, new: &FakeTlsKey, hid: &str) -> Value {
        let new_pin = SpkiHash(new.pin());
        json!({
            "spki": new.pin_b64url(),
            "prev_spki": BASE64URL_NOPAD.encode(&self.spki),
            "sig": BASE64URL_NOPAD.encode(&self.sign(&rotation_message(hid, &new_pin))),
        })
    }

    fn self_signed(&self) -> Vec<u8> {
        let ecdsa_sha256 = seq(&[&tlv(0x06, &[0x2a, 0x86, 0x48, 0xce, 0x3d, 0x04, 0x03, 0x02])]);
        let cn = seq(&[&tlv(0x06, &[0x55, 0x04, 0x03]), &tlv(0x0c, b"DSPi fake hub")]);
        let name = seq(&[&tlv(0x31, &cn)]);
        let validity = seq(&[&tlv(0x17, b"250101000000Z"), &tlv(0x17, b"491231235959Z")]);
        let san = seq(&[&tlv(0x06, &[0x55, 0x1d, 0x11]), &tlv(0x04, &seq(&[&tlv(0x82, b"localhost")]))]);
        let tbs = seq(&[
            &tlv(0xa0, &tlv(0x02, &[2])),
            &tlv(0x02, &[0x01]),
            &ecdsa_sha256,
            &name,
            &validity,
            &name,
            &self.spki,
            &tlv(0xa3, &seq(&[&san])),
        ]);
        let mut bits = vec![0u8];
        bits.extend_from_slice(&self.sign(&tbs));
        seq(&[&tbs, &ecdsa_sha256, &tlv(0x03, &bits)])
    }
}

fn tlv(tag: u8, body: &[u8]) -> Vec<u8> {
    let mut out = vec![tag];
    match body.len() {
        n if n < 0x80 => out.push(n as u8),
        n if n < 0x100 => out.extend_from_slice(&[0x81, n as u8]),
        n => out.extend_from_slice(&[0x82, (n >> 8) as u8, n as u8]),
    }
    out.extend_from_slice(body);
    out
}

fn seq(parts: &[&[u8]]) -> Vec<u8> {
    tlv(0x30, &parts.concat())
}
