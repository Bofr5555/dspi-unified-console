//! -- review seca -- SECKEY's identity drives SECPIPE's TLS end to end, the rotation
//! signature SECKEY makes is the one SECPIPE verifies, and the two SPKI
//! helpers (SECKEY's pub(crate) copy, dspi_link::tls::SpkiHash) agree.

use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::thread;
use std::time::{Duration, SystemTime};

use data_encoding::BASE64;
use dspi_hub::tls_identity::{TlsIdentity, TlsNames, rotation_message as hub_message};
use dspi_link::clock::Instant;
use dspi_link::tls::{self, PinMode, PinOutcome, RotationError, SpkiHash, verify_rotation};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};

const HID: &str = "0f8e2b5c-1d3a-4c6e-9b7f-2a4d6c8e0b1f";

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::current_exe()
        .unwrap()
        .parent()
        .unwrap()
        .join(format!("review-seca-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn pem_block(text: &str, label: &str) -> Vec<u8> {
    let begin = format!("-----BEGIN {label}-----");
    let end = format!("-----END {label}-----");
    let (_, rest) = text.split_once(&begin).unwrap();
    let (body, _) = rest.split_once(&end).unwrap();
    let b64: String = body.chars().filter(|c| !c.is_ascii_whitespace()).collect();
    BASE64.decode(b64.as_bytes()).unwrap()
}

fn names() -> TlsNames {
    TlsNames::new(Some("dspi-0123456789ab.local"), &[])
}

fn tcp_pair() -> (TcpStream, TcpStream) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
    let (server, _) = listener.accept().unwrap();
    for s in [&client, &server] {
        s.set_write_timeout(Some(Duration::from_secs(10))).unwrap();
    }
    (client, server)
}

fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(5)
}

/// SECPIPE's server config from what SECKEY wrote to disk.
fn server_config(id: &TlsIdentity, data: &std::path::Path) -> std::sync::Arc<rustls::ServerConfig> {
    let key_pem = fs::read_to_string(data.join("tls").join("key.pem")).unwrap();
    let key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(pem_block(&key_pem, "PRIVATE KEY")));
    tls::server_config(
        vec![CertificateDer::from(id.certificate_der().to_vec())],
        key,
    )
    .unwrap()
}

fn handshake(
    config: std::sync::Arc<rustls::ServerConfig>,
    mode: PinMode,
) -> (
    Result<(tls::TlsPipe, PinOutcome), tls::TlsError>,
    Result<tls::TlsPipe, tls::TlsError>,
) {
    let (client, server) = tcp_pair();
    let accepting = thread::spawn(move || tls::accept(server, config, deadline()));
    let connected = tls::connect(client, "localhost", mode, deadline());
    (connected, accepting.join().unwrap())
}

#[test]
fn review_seca_the_two_spki_helpers_agree_on_one_key() {
    let data = scratch("helpers");
    let (id, _) = TlsIdentity::load(&data, HID, &names(), SystemTime::now()).unwrap();
    let link = SpkiHash::of_spki(id.spki_der());
    let hub = id.pin();
    assert_eq!(link.0, hub.0, "the hash");
    assert_eq!(link.display(), hub.display(), "the display form");
    assert_eq!(link.to_hex(), hub.hex());
    assert_eq!(link.curl_pin(), hub.curl());
    assert_eq!(link.to_b64url(), hub.base64url());
    assert_eq!(
        SpkiHash::of_cert(id.certificate_der()).unwrap(),
        link,
        "the certificate carries that key"
    );
    assert_eq!(
        hub_message(HID, &hub),
        tls::rotation_message(HID, &link),
        "the rotation message"
    );
}

#[test]
fn review_seca_seckey_identity_serves_secpipe_tls_to_a_pinned_client() {
    let data = scratch("serve");
    let (id, _) = TlsIdentity::load(&data, HID, &names(), SystemTime::now()).unwrap();
    let pin = SpkiHash(id.pin().0);
    let config = server_config(&id, &data);

    let (c, s) = handshake(config.clone(), PinMode::Pinned(pin));
    let (mut c, outcome) = c.unwrap();
    let mut s = s.unwrap();
    assert_eq!(outcome, PinOutcome::Matched(pin));
    c.write_all(b"{\"t\":\"hello\"}").unwrap();
    let mut buf = [0u8; 64];
    let n = s.read(&mut buf).unwrap();
    assert_eq!(&buf[..n], b"{\"t\":\"hello\"}");

    // Another pin: refused before any byte
    let (c, _s) = handshake(config, PinMode::Pinned(SpkiHash([7; 32])));
    assert_eq!(c.err(), Some(tls::TlsError::KeyMismatch));
}

#[test]
fn review_seca_seckeys_rotation_signature_is_what_secpipe_verifies() {
    let data = scratch("rotate");
    let now = SystemTime::now();
    let (mut id, _) = TlsIdentity::load(&data, HID, &names(), now).unwrap();
    let old = SpkiHash(id.pin().0);
    id.rotate(&names(), now, false).unwrap();
    let new = SpkiHash(id.pin().0);
    assert_ne!(old, new);
    let next = id.key_next(now).expect("announced");
    assert_eq!(
        SpkiHash(next.spki.0),
        new,
        "key_next names the key now served"
    );

    // The client's check, as SECCORE will run it from hello.hub.key_next
    assert_eq!(
        verify_rotation(&old, &next.prev_spki_der, HID, &new, &next.sig),
        Ok(())
    );
    assert_eq!(
        verify_rotation(&old, &next.prev_spki_der, "another-hub", &new, &next.sig),
        Err(RotationError::BadSignature)
    );
    assert_eq!(
        verify_rotation(
            &old,
            &next.prev_spki_der,
            HID,
            &SpkiHash([1; 32]),
            &next.sig
        ),
        Err(RotationError::BadSignature)
    );
    let mut forged = next.sig.clone();
    forged[10] ^= 0x40;
    assert_eq!(
        verify_rotation(&old, &next.prev_spki_der, HID, &new, &forged),
        Err(RotationError::BadSignature)
    );
    assert_eq!(
        verify_rotation(&new, &next.prev_spki_der, HID, &new, &next.sig),
        Err(RotationError::NotPinnedKey),
        "a client pinned to the new key is not in rotation"
    );

    // Rotating mode against the rotated hub: the handshake completes and reports both keys
    let config = server_config(&id, &data);
    let (c, s) = handshake(config, PinMode::Rotating(old));
    let (_c, outcome) = c.unwrap();
    s.unwrap();
    assert_eq!(
        outcome,
        PinOutcome::Rotated {
            pinned: old,
            presented: new
        }
    );

    // forget_old: nothing to verify
    let (mut id2, _) = TlsIdentity::load(&scratch("rotate-forget"), HID, &names(), now).unwrap();
    id2.rotate(&names(), now, true).unwrap();
    assert!(id2.key_next(now).is_none());
}

/// SECPIPE probe: tiny reads and a burst of many small records never stall
/// or trip the pending-input guard.
#[test]
fn review_seca_probe_tiny_reads_and_small_record_bursts() {
    let data = scratch("burst");
    let (id, _) = TlsIdentity::load(&data, HID, &names(), SystemTime::now()).unwrap();
    let pin = SpkiHash(id.pin().0);
    let (c, s) = handshake(server_config(&id, &data), PinMode::Pinned(pin));
    let (mut c, _) = c.unwrap();
    let mut s = s.unwrap();
    c.set_deadline(None);
    s.set_deadline(None);
    // 3000 one-byte records in one go (each its own TLS record, ~23 bytes on the wire)
    let writer = thread::spawn(move || {
        for i in 0..3000u32 {
            c.write_all(&[(i % 251) as u8]).unwrap();
        }
        c
    });
    let mut got = Vec::new();
    let mut one = [0u8; 1];
    while got.len() < 3000 {
        let n = s.read(&mut one).unwrap();
        assert_eq!(n, 1);
        got.push(one[0]);
    }
    assert!(
        got.iter()
            .enumerate()
            .all(|(i, b)| *b == (i as u32 % 251) as u8)
    );
    drop(writer.join().unwrap());
}
