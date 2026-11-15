//! WP SECPIPE: the TLS pipe and the pin verifier, over loopback sockets.
//!
//! Certificates are made here, by a small DER writer and ring: rcgen is the
//! hub's (SECKEY), and a test needs dates no generator makes on request.
#![allow(clippy::arithmetic_side_effects)]

use super::*;

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use ring::rand::SystemRandom;
use ring::signature::{EcdsaKeyPair, KeyPair, ECDSA_P256_SHA256_ASN1_SIGNING};
use rustls::pki_types::{PrivateKeyDer, PrivatePkcs8KeyDer};
use socket2::SockRef;
use tungstenite::protocol::Role;
use tungstenite::{Message, WebSocket};

// ---------------------------------------------------------------------------
// Keys and certificates

struct TestKey {
    pkcs8: Vec<u8>,
    pair: EcdsaKeyPair,
    spki: Vec<u8>,
}

impl TestKey {
    fn new() -> TestKey {
        let rng = SystemRandom::new();
        let pkcs8 = EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, &rng).unwrap().as_ref().to_vec();
        let pair = EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, &pkcs8, &rng).unwrap();
        let mut spki = P256_SPKI_PREFIX.to_vec();
        spki.extend_from_slice(pair.public_key().as_ref());
        TestKey { pkcs8, pair, spki }
    }

    fn pin(&self) -> SpkiHash {
        SpkiHash::of_spki(&self.spki)
    }

    fn sign(&self, msg: &[u8]) -> Vec<u8> {
        self.pair.sign(&SystemRandom::new(), msg).unwrap().as_ref().to_vec()
    }

    fn der(&self) -> PrivateKeyDer<'static> {
        PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(self.pkcs8.clone()))
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

/// A self-signed X.509 v3 certificate for `subject_key`, signed by `signer`,
/// valid between two UTCTimes (`YYMMDDhhmmssZ`).
fn cert_signed(subject_key: &TestKey, signer: &TestKey, not_before: &str, not_after: &str) -> Vec<u8> {
    let ecdsa_sha256 = seq(&[&tlv(0x06, &[0x2a, 0x86, 0x48, 0xce, 0x3d, 0x04, 0x03, 0x02])]);
    let cn = seq(&[&tlv(0x06, &[0x55, 0x04, 0x03]), &tlv(0x0c, b"DSPi test hub")]);
    let name = seq(&[&tlv(0x31, &cn)]);
    let validity = seq(&[&tlv(0x17, not_before.as_bytes()), &tlv(0x17, not_after.as_bytes())]);
    let san = seq(&[&tlv(0x06, &[0x55, 0x1d, 0x11]), &tlv(0x04, &seq(&[&tlv(0x82, b"hub.local")]))]);
    let tbs = seq(&[
        &tlv(0xa0, &tlv(0x02, &[2])),
        &tlv(0x02, &[0x01]),
        &ecdsa_sha256,
        &name,
        &validity,
        &name,
        &subject_key.spki,
        &tlv(0xa3, &seq(&[&san])),
    ]);
    let mut bits = vec![0u8];
    bits.extend_from_slice(&signer.sign(&tbs));
    seq(&[&tbs, &ecdsa_sha256, &tlv(0x03, &bits)])
}

fn cert(key: &TestKey, not_before: &str, not_after: &str) -> Vec<u8> {
    cert_signed(key, key, not_before, not_after)
}

const NOW_FROM: &str = "250101000000Z";
const NOW_TO: &str = "491231235959Z";

fn config_for(key: &TestKey, cert_der: Vec<u8>) -> Arc<ServerConfig> {
    server_config(vec![CertificateDer::from(cert_der)], key.der()).unwrap()
}

/// A server that presents `presented`'s certificate but signs with `signer`:
/// it does not hold the key it shows.
fn impostor_config(presented: &TestKey, signer: &TestKey) -> Arc<ServerConfig> {
    let chain = vec![CertificateDer::from(cert(presented, NOW_FROM, NOW_TO))];
    let signing = rustls::crypto::ring::sign::any_ecdsa_type(&signer.der()).unwrap();
    build_server_config(provider(), Arc::new(OneKey(Arc::new(CertifiedKey::new(chain, signing))))).unwrap()
}

// ---------------------------------------------------------------------------
// Sockets

fn deadline(ms: u64) -> Instant {
    Instant::now() + Duration::from_millis(ms)
}

fn tcp_pair() -> (TcpStream, TcpStream) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
    let (server, _) = listener.accept().unwrap();
    for s in [&client, &server] {
        s.set_write_timeout(Some(Duration::from_secs(10))).unwrap();
        s.set_nodelay(true).unwrap();
    }
    (client, server)
}

/// Small socket buffers, so a megabyte cannot hide in the kernel.
fn small_buffers(s: &TcpStream) {
    let s = SockRef::from(s);
    s.set_send_buffer_size(32 * 1024).unwrap();
    s.set_recv_buffer_size(32 * 1024).unwrap();
}

type ClientResult = Result<(TlsPipe, PinOutcome), TlsError>;

fn handshake_with(config: Arc<ServerConfig>, mode: PinMode) -> (ClientResult, Result<TlsPipe, TlsError>) {
    handshake_on(tcp_pair(), config, mode)
}

fn handshake_on(
    (client, server): (TcpStream, TcpStream),
    config: Arc<ServerConfig>,
    mode: PinMode,
) -> (ClientResult, Result<TlsPipe, TlsError>) {
    let accepting = thread::spawn(move || accept(server, config, deadline(5000)));
    let connected = connect(client, "hub.local", mode, deadline(5000));
    (connected, accepting.join().unwrap())
}

fn pair(key: &TestKey) -> (TlsPipe, TlsPipe) {
    let (c, s) = handshake_with(config_for(key, cert(key, NOW_FROM, NOW_TO)), PinMode::Pinned(key.pin()));
    let (mut c, outcome) = c.unwrap();
    let mut s = s.unwrap();
    assert_eq!(outcome, PinOutcome::Matched(key.pin()));
    c.set_deadline(None);
    s.set_deadline(None);
    (c, s)
}

/// Runs `f` on a thread and fails the test if it is still running after `limit`.
fn within<T: Send + 'static>(limit: Duration, f: impl FnOnce() -> T + Send + 'static) -> T {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let _ = tx.send(f());
    });
    rx.recv_timeout(limit).expect("hung")
}

fn read_exact(p: &mut TlsPipe, n: usize) -> Vec<u8> {
    let mut buf = vec![0u8; n];
    p.read_exact(&mut buf).unwrap();
    buf
}

// ---------------------------------------------------------------------------
// Pins and signatures

#[test]
fn wp_secpipe_pinned_handshake_succeeds_over_tls13_with_alpn() {
    let key = TestKey::new();
    let (mut c, mut s) = pair(&key);
    c.write_all(b"ping").unwrap();
    assert_eq!(read_exact(&mut s, 4), b"ping");
    s.write_all(b"pong").unwrap();
    assert_eq!(read_exact(&mut c, 4), b"pong");
    assert_eq!(c.alpn_protocol().as_deref(), Some(ALPN_HTTP11));
    let g = lock(&c.shared.inner);
    assert_eq!(g.conn.protocol_version(), Some(rustls::ProtocolVersion::TLSv1_3));
}

#[test]
fn wp_secpipe_a_wrong_key_fails_before_any_application_byte() {
    let paired = TestKey::new();
    let other = TestKey::new();
    let (client, server) = tcp_pair();
    let config = config_for(&other, cert(&other, NOW_FROM, NOW_TO));
    // The server sends as soon as it can, and reports whether it got anything back
    let serving = thread::spawn(move || match accept(server, config, deadline(5000)) {
        Ok(mut p) => {
            let _ = p.write_all(b"hello from the impostor");
            let mut buf = [0u8; 64];
            p.read(&mut buf).map_err(|e| e.to_string())
        }
        Err(e) => Err(e.to_string()),
    });
    let r = connect(client, "hub.local", PinMode::Pinned(paired.pin()), deadline(5000));
    assert_eq!(r.err(), Some(TlsError::KeyMismatch));
    // No pipe exists on the client to read from, and the server received nothing
    assert!(!matches!(serving.join().unwrap(), Ok(n) if n > 0));
}

#[test]
fn wp_secpipe_a_server_without_the_private_key_fails_in_every_mode() {
    let shown = TestKey::new();
    let held = TestKey::new();
    for mode in
        [PinMode::Pinned(shown.pin()), PinMode::FirstUse, PinMode::Rotating(shown.pin()), PinMode::Rotating(held.pin())]
    {
        let (c, s) = handshake_with(impostor_config(&shown, &held), mode);
        match c {
            Err(TlsError::Handshake(_)) => {}
            other => panic!("{mode:?}: {:?}", other.map(|(_, o)| o)),
        }
        assert!(s.is_err(), "{mode:?}");
    }
}

#[test]
fn wp_secpipe_certificate_dates_are_never_checked() {
    let key = TestKey::new();
    for (from, to) in [
        ("000101000000Z", "010101000000Z"), // expired in 2001
        ("490101000000Z", "491231235959Z"), // not valid before 2049
    ] {
        for mode in [PinMode::Pinned(key.pin()), PinMode::FirstUse, PinMode::Rotating(key.pin())] {
            let (c, s) = handshake_with(config_for(&key, cert(&key, from, to)), mode);
            let (mut c, outcome) = c.unwrap_or_else(|e| panic!("{from}..{to} {mode:?}: {e}"));
            assert_eq!(outcome.presented(), key.pin());
            let mut s = s.unwrap();
            c.write_all(b"x").unwrap();
            assert_eq!(read_exact(&mut s, 1), b"x");
        }
    }
}

#[test]
fn wp_secpipe_first_use_reports_the_key() {
    let key = TestKey::new();
    let (c, _s) = handshake_with(config_for(&key, cert(&key, NOW_FROM, NOW_TO)), PinMode::FirstUse);
    assert_eq!(c.unwrap().1, PinOutcome::FirstUse(key.pin()));
}

#[test]
fn wp_secpipe_rotating_reports_the_new_key_and_the_rotation_signature_decides() {
    let old = TestKey::new();
    let new = TestKey::new();
    let stranger = TestKey::new();
    let hid = "0f8e2b5c-1d3a-4c6e-9b7f-2a4d6c8e0b1f";

    // The same key: nothing to do
    let (c, _s) = handshake_with(config_for(&old, cert(&old, NOW_FROM, NOW_TO)), PinMode::Rotating(old.pin()));
    assert_eq!(c.unwrap().1, PinOutcome::Matched(old.pin()));

    // A new key: the handshake completes and reports it
    let (c, s) = handshake_with(config_for(&new, cert(&new, NOW_FROM, NOW_TO)), PinMode::Rotating(old.pin()));
    let (mut c, outcome) = c.unwrap();
    let mut s = s.unwrap();
    assert_eq!(outcome, PinOutcome::Rotated { pinned: old.pin(), presented: new.pin() });
    let presented = outcome.presented();

    // key_next arrives inside TLS (here: raw bytes), before any token goes out
    let sig = old.sign(&rotation_message(hid, &new.pin()));
    s.write_all(&sig).unwrap();
    let got = read_exact(&mut c, sig.len());
    assert_eq!(verify_rotation(&old.pin(), &old.spki, hid, &presented, &got), Ok(()));

    // Forged: random bytes, another key's signature, another hub, another new key
    assert_eq!(
        verify_rotation(&old.pin(), &old.spki, hid, &presented, &[0x30, 0x06, 0x02, 0x01, 0x01, 0x02, 0x01, 0x01]),
        Err(RotationError::BadSignature)
    );
    let by_stranger = stranger.sign(&rotation_message(hid, &new.pin()));
    assert_eq!(verify_rotation(&old.pin(), &old.spki, hid, &presented, &by_stranger), Err(RotationError::BadSignature));
    assert_eq!(
        verify_rotation(&old.pin(), &stranger.spki, hid, &presented, &by_stranger),
        Err(RotationError::NotPinnedKey),
        "the stranger's own key is not the pin"
    );
    assert_eq!(
        verify_rotation(&old.pin(), &old.spki, "another-hub", &presented, &sig),
        Err(RotationError::BadSignature)
    );
    assert_eq!(verify_rotation(&old.pin(), &old.spki, hid, &stranger.pin(), &sig), Err(RotationError::BadSignature));
    assert_eq!(verify_rotation(&old.pin(), &[], hid, &presented, &sig), Err(RotationError::NotPinnedKey));
    let not_p256 = b"not a key".to_vec();
    assert_eq!(
        verify_rotation(&SpkiHash::of_spki(&not_p256), &not_p256, hid, &presented, &sig),
        Err(RotationError::UnsupportedKey)
    );
}

#[test]
fn wp_secpipe_the_rotation_message_layout() {
    let k = SpkiHash([7u8; 32]);
    let m = rotation_message("hub", &k);
    assert_eq!(&m[..16], b"dspi-key-rotate\0");
    assert_eq!(&m[16..19], b"hub");
    assert_eq!(&m[19..], &[7u8; 32]);
}

// ---------------------------------------------------------------------------
// The SPKI hash

#[test]
fn wp_secpipe_spki_hash_of_a_certificate_is_the_hash_of_its_whole_spki() {
    let key = TestKey::new();
    let c = cert(&key, NOW_FROM, NOW_TO);
    assert_eq!(SpkiHash::of_cert(&c).unwrap(), key.pin());
    let digest = ring::digest::digest(&ring::digest::SHA256, &key.spki);
    assert_eq!(&key.pin().0[..], digest.as_ref(), "SHA-256 over the DER SPKI, SEQUENCE header included");
    assert_eq!(SpkiHash::of_cert(b"\x30\x03\x02\x01\x01"), Err(TlsError::BadCertificate));
}

#[test]
fn wp_secpipe_spki_hash_display_forms() {
    let mut bytes = [0u8; 32];
    for (i, b) in bytes.iter_mut().enumerate() {
        *b = i as u8;
    }
    let k = SpkiHash(bytes);
    // 00 01 02 03 04 | 05 06 07 08 09 in Crockford base32, 5 bits at a time
    assert_eq!(k.display(), "SHA256:000G-40R4-0M30-E209");
    assert_eq!(SpkiHash([0xff; 32]).display(), "SHA256:ZZZZ-ZZZZ-ZZZZ-ZZZZ");
    assert_eq!(k.curl_pin(), "sha256//AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8=");
    assert_eq!(SpkiHash::from_hex(&k.to_hex()), Some(k));
    assert_eq!(SpkiHash::from_hex(&k.to_hex().to_uppercase()), Some(k));
    assert_eq!(k.to_b64url().len(), 43);
    assert_eq!(SpkiHash::from_b64url(&k.to_b64url()), Some(k));
    assert_eq!(SpkiHash::from_b64url("AAEC"), None, "too short");
    assert_eq!(SpkiHash::from_hex("zz"), None);
    assert!(k.ct_eq(&k) && !k.ct_eq(&SpkiHash([0; 32])));
    assert_eq!(format!("{k:?}"), "SpkiHash(SHA256:000G-40R4-0M30-E209)");
}

// ---------------------------------------------------------------------------
// The pipe

#[test]
fn wp_secpipe_ten_thousand_websocket_frames_each_way_concurrently() {
    let key = TestKey::new();
    let (c, s) = pair(&key);
    let run = |reader: TlsPipe, writer: TlsPipe, role: Role, seed: u32| {
        let (mut reader, mut writer) = (reader, writer);
        reader.set_buffered(true);
        writer.set_buffered(true);
        reader.set_read_timeout(Some(Duration::from_millis(200))).unwrap();
        let mut rx = WebSocket::from_raw_socket(reader, role, None);
        let mut tx = WebSocket::from_raw_socket(writer, role, None);
        let sending = thread::spawn(move || {
            for i in 0..10_000u32 {
                let len = (i as usize * 37) % 3000 + if i.is_multiple_of(500) { 40_000 } else { 0 };
                let body: Vec<u8> = (0..len).map(|j| (j as u32 ^ i ^ seed) as u8).collect();
                tx.send(Message::binary(body)).unwrap();
            }
            tx
        });
        let receiving = thread::spawn(move || {
            let peer = seed ^ 1;
            let mut i = 0u32;
            while i < 10_000 {
                match rx.read() {
                    Ok(Message::Binary(b)) => {
                        let len = (i as usize * 37) % 3000 + if i.is_multiple_of(500) { 40_000 } else { 0 };
                        assert_eq!(b.len(), len, "frame {i}");
                        assert!(b.iter().enumerate().all(|(j, x)| *x == (j as u32 ^ i ^ peer) as u8), "frame {i}");
                        i += 1;
                    }
                    Ok(other) => panic!("{other:?}"),
                    Err(tungstenite::Error::Io(e))
                        if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {}
                    Err(e) => panic!("frame {i}: {e}"),
                }
            }
            rx
        });
        (sending, receiving)
    };
    let (c2, s2) = (c.try_clone().unwrap(), s.try_clone().unwrap());
    within(Duration::from_secs(120), move || {
        let (cs, cr) = run(c, c2, Role::Client, 0);
        let (ss, sr) = run(s, s2, Role::Server, 1);
        let kept = (cs.join().unwrap(), cr.join().unwrap(), ss.join().unwrap(), sr.join().unwrap());
        drop(kept);
    });
}

#[test]
fn wp_secpipe_both_halves_writing_a_mebibyte_without_reading_stall_and_nothing_hangs() {
    let key = TestKey::new();
    let (client, server) = tcp_pair();
    for s in [&client, &server] {
        small_buffers(s);
    }
    let (c, s) = handshake_on((client, server), config_for(&key, cert(&key, NOW_FROM, NOW_TO)), PinMode::FirstUse);
    let (c, s) = (c.unwrap().0, s.unwrap());
    for p in [&c, &s] {
        p.tcp().set_write_timeout(Some(Duration::from_millis(300))).unwrap();
    }
    let started = Instant::now();
    let results = within(Duration::from_secs(30), move || {
        let write = |mut p: TlsPipe| {
            thread::spawn(move || {
                p.set_buffered(true);
                p.write_all(&vec![0x5a; 1 << 20]).unwrap();
                let r = p.flush().map_err(|e| e.kind());
                (r, p.is_broken())
            })
        };
        let (a, b) = (write(c), write(s));
        (a.join().unwrap(), b.join().unwrap())
    });
    assert!(results.0 .0.is_err() && results.0 .1, "{results:?}");
    assert!(results.1 .0.is_err() && results.1 .1, "{results:?}");
    assert!(started.elapsed() < Duration::from_secs(10));
}

#[test]
fn wp_secpipe_both_halves_writing_while_their_readers_read_do_not_deadlock() {
    // Review S2: a writer blocked on a full socket must not keep its reader from
    // decrypting, or two peers writing past their buffers wait for each other.
    let key = TestKey::new();
    let (client, server) = tcp_pair();
    for s in [&client, &server] {
        small_buffers(s);
    }
    let (c, s) = handshake_on((client, server), config_for(&key, cert(&key, NOW_FROM, NOW_TO)), PinMode::FirstUse);
    let (c, s) = (c.unwrap().0, s.unwrap());
    const SIZE: usize = 8 << 20;
    within(Duration::from_secs(60), move || {
        let side = |p: TlsPipe, byte: u8, expect: u8| {
            let mut reader = p.try_clone().unwrap();
            let mut writer = p;
            let w = thread::spawn(move || {
                writer.set_buffered(true);
                for _ in 0..(SIZE / (64 << 10)) {
                    writer.write_all(&vec![byte; 64 << 10]).unwrap();
                    writer.flush().unwrap();
                }
                writer
            });
            let r = thread::spawn(move || {
                let mut got = 0usize;
                let mut buf = vec![0u8; 20_000];
                while got < SIZE {
                    let n = reader.read(&mut buf).unwrap();
                    assert!(n > 0);
                    assert!(buf[..n].iter().all(|b| *b == expect));
                    got += n;
                }
                reader
            });
            (w, r)
        };
        let (cw, cr) = side(c, 1, 2);
        let (sw, sr) = side(s, 2, 1);
        let kept = (cw.join().unwrap(), cr.join().unwrap(), sw.join().unwrap(), sr.join().unwrap());
        drop(kept);
    });
}

/// A KeyUpdate asked for by one side, answered by the other's reader.
fn key_update(p: &TlsPipe) {
    let mut g = lock(&p.shared.inner);
    g.conn.refresh_traffic_keys().unwrap();
    g.collect();
    // sent with the next write
}

#[test]
fn wp_secpipe_a_key_update_in_the_middle_of_a_stream() {
    let key = TestKey::new();
    let (c, s) = pair(&key);
    within(Duration::from_secs(30), move || {
        let (mut c, mut s) = (c, s);
        let mut s_reader = s.try_clone().unwrap();
        let echo = thread::spawn(move || {
            // Echoes everything back on the writing half
            let mut buf = vec![0u8; 4096];
            let mut total = 0usize;
            while total < 300 * 1000 {
                let n = s_reader.read(&mut buf).unwrap();
                s.write_all(&buf[..n]).unwrap();
                total += n;
                if total > 150 * 1000 && total - n <= 150 * 1000 {
                    key_update(&s); // the server asks too, half way
                }
            }
            (s, s_reader)
        });
        let mut c_reader = c.try_clone().unwrap();
        let reading = thread::spawn(move || {
            let mut got = Vec::new();
            let mut buf = vec![0u8; 4096];
            while got.len() < 300 * 1000 {
                let n = c_reader.read(&mut buf).unwrap();
                got.extend_from_slice(&buf[..n]);
            }
            got
        });
        let mut sent = Vec::new();
        for i in 0..300u32 {
            if i == 100 || i == 200 {
                key_update(&c);
            }
            let chunk: Vec<u8> = (0..1000u32).map(|j| (i.wrapping_mul(31) ^ j) as u8).collect();
            c.write_all(&chunk).unwrap();
            sent.extend_from_slice(&chunk);
        }
        assert_eq!(reading.join().unwrap(), sent);
        drop(echo.join().unwrap());
    });
}

#[test]
fn wp_secpipe_close_notify_in_both_directions() {
    let key = TestKey::new();
    let (mut c, mut s) = pair(&key);
    c.write_all(b"last").unwrap();
    c.send_close_notify().unwrap();
    assert_eq!(read_exact(&mut s, 4), b"last");
    let mut buf = [0u8; 8];
    assert_eq!(s.read(&mut buf).unwrap(), 0, "a clean end");
    s.write_all(b"bye").unwrap();
    s.send_close_notify().unwrap();
    assert_eq!(read_exact(&mut c, 3), b"bye");
    assert_eq!(c.read(&mut buf).unwrap(), 0);
    // A socket that ends with no close_notify is not a clean end
    let (mut c, s) = pair(&key);
    s.shutdown().unwrap();
    assert_eq!(c.read(&mut buf).unwrap_err().kind(), std::io::ErrorKind::UnexpectedEof);
}

#[test]
fn wp_secpipe_a_trickled_handshake_is_cut_at_the_deadline() {
    // Bytes keep coming, so no single read times out: a record header that
    // announces a long record, then one byte every 10 ms
    let trickle = |mut s: TcpStream| {
        thread::spawn(move || {
            let _ = s.write_all(&[0x16, 0x03, 0x03, 0x40, 0x00]);
            while s.write_all(&[0x00]).is_ok() {
                thread::sleep(Duration::from_millis(10));
            }
        })
    };
    let (client, server) = tcp_pair();
    let _feeder = trickle(server);
    let started = Instant::now();
    let r = connect(client, "hub.local", PinMode::FirstUse, deadline(200));
    assert_eq!(r.err(), Some(TlsError::Timeout));
    assert!(started.elapsed() < Duration::from_secs(3));

    // and the hub's side, against a client that trickles its ClientHello
    let key = TestKey::new();
    let (client, server) = tcp_pair();
    let _feeder = trickle(client);
    let started = Instant::now();
    let r = accept(server, config_for(&key, cert(&key, NOW_FROM, NOW_TO)), deadline(200));
    assert_eq!(r.err(), Some(TlsError::Timeout));
    assert!(started.elapsed() < Duration::from_secs(3));
}

#[test]
fn wp_secpipe_a_closed_or_plaintext_peer_refuses_the_handshake() {
    let (client, server) = tcp_pair();
    drop(server);
    assert_eq!(connect(client, "hub.local", PinMode::FirstUse, deadline(2000)).err(), Some(TlsError::Refused));
    // An `off` hub answers the ClientHello as HTTP
    let (client, mut server) = tcp_pair();
    let answering = thread::spawn(move || {
        let mut buf = [0u8; 512];
        let _ = server.read(&mut buf);
        let _ = server.write_all(b"HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\n\r\n");
    });
    let r = connect(client, "hub.local", PinMode::FirstUse, deadline(2000));
    assert!(matches!(r.err(), Some(TlsError::Handshake(_) | TlsError::Refused)));
    answering.join().unwrap();
}

#[test]
fn wp_secpipe_sniff_tells_tls_from_plaintext_and_consumes_nothing() {
    let (mut client, server) = tcp_pair();
    client.write_all(b"GET /dspi/v1/info HTTP/1.1\r\n\r\n").unwrap();
    assert!(!sniff_tls(&server).unwrap());
    let mut first = [0u8; 4];
    (&server).read_exact(&mut first).unwrap();
    assert_eq!(&first, b"GET ", "nothing consumed");

    // A ClientHello: sniffed, and the handshake still completes after it
    let key = TestKey::new();
    let (client, server) = tcp_pair();
    let config = config_for(&key, cert(&key, NOW_FROM, NOW_TO));
    let serving = thread::spawn(move || {
        server.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        assert!(sniff_tls(&server).unwrap());
        accept(server, config, deadline(5000)).map(|_| ())
    });
    let (mut c, _) = connect(client, "hub.local", PinMode::Pinned(key.pin()), deadline(5000)).unwrap();
    serving.join().unwrap().unwrap();
    c.write_all(b"x").unwrap();

    // A peer that closes before its first byte
    let (client, server) = tcp_pair();
    drop(client);
    assert_eq!(sniff_tls(&server).unwrap_err().kind(), std::io::ErrorKind::UnexpectedEof);
}

#[test]
fn wp_secpipe_ip_and_bracketed_hosts_are_server_names() {
    let key = TestKey::new();
    for host in ["127.0.0.1", "[::1]", "hub.local"] {
        let (client, server) = tcp_pair();
        let config = config_for(&key, cert(&key, NOW_FROM, NOW_TO));
        let serving = thread::spawn(move || accept(server, config, deadline(5000)).map(|_| ()));
        let r = connect(client, host, PinMode::Pinned(key.pin()), deadline(5000));
        assert!(r.is_ok(), "{host}: {:?}", r.err());
        serving.join().unwrap().unwrap();
    }
    let (client, _server) = tcp_pair();
    assert!(matches!(
        connect(client, "not a name", PinMode::FirstUse, deadline(1000)).err(),
        Some(TlsError::ServerName(_))
    ));
}

#[test]
fn wp_secpipe_no_secret_in_debug_output() {
    let key = TestKey::new();
    let (mut c, _s) = pair(&key);
    c.set_buffered(true);
    c.write_all(b"{\"op\":\"auth.token\",\"token\":\"SECRET-TOKEN-123\"}").unwrap();
    let shown = format!("{c:?}");
    assert!(!shown.contains("SECRET"), "{shown}");
    let config = config_for(&key, cert(&key, NOW_FROM, NOW_TO));
    let shown = format!(
        "{config:?} {:?}",
        OneKey(Arc::new(CertifiedKey::new(vec![], {
            rustls::crypto::ring::sign::any_ecdsa_type(&key.der()).unwrap()
        })))
    );
    // The private key's bytes in any of Debug's forms
    let private = &key.pkcs8[key.pkcs8.len() - 40..];
    assert!(!shown.contains(&HEXLOWER.encode(private)), "{shown}");
    assert!(!shown.contains(format!("{:?}", &private[..8]).trim_matches(['[', ']'])), "{shown}");
    assert!(!format!("{:?}", TlsError::KeyMismatch).contains("SECRET"));
}

#[test]
fn wp_secpipe_server_config_refuses_a_key_that_does_not_match() {
    let a = TestKey::new();
    let b = TestKey::new();
    let r = server_config(vec![CertificateDer::from(cert(&a, NOW_FROM, NOW_TO))], b.der());
    assert!(matches!(r.err(), Some(TlsError::Config(_))));
}

#[test]
fn wp_secpipe_a_failed_write_breaks_the_pipe_for_good() {
    let key = TestKey::new();
    let (client, server) = tcp_pair();
    small_buffers(&client);
    small_buffers(&server);
    let (c, _s) = handshake_on((client, server), config_for(&key, cert(&key, NOW_FROM, NOW_TO)), PinMode::FirstUse);
    let mut c = c.unwrap().0;
    c.tcp().set_write_timeout(Some(Duration::from_millis(50))).unwrap();
    c.set_buffered(true);
    c.write_all(&vec![0u8; 4 << 20]).unwrap();
    assert!(c.flush().is_err());
    assert!(c.is_broken());
    let mut other_half = c.try_clone().unwrap();
    other_half.set_buffered(true);
    other_half.write_all(b"x").unwrap();
    assert_eq!(other_half.flush().unwrap_err().kind(), std::io::ErrorKind::BrokenPipe, "nothing more goes out");
}

/// The same vectors as SECKEY's copy of these helpers (wp/SECKEY
/// `tests_wp_seckey.rs`), so the merge can swap the copy for these unchanged.
#[test]
fn wp_secpipe_helpers_match_seckeys_vectors() {
    let mut pin = [0u8; 32];
    assert_eq!(SpkiHash(pin).display(), "SHA256:0000-0000-0000-0000");
    pin[..10].copy_from_slice(&[0xFF; 10]);
    pin[10] = 0xAB; // past 80 bits: not shown
    assert_eq!(SpkiHash(pin).display(), "SHA256:ZZZZ-ZZZZ-ZZZZ-ZZZZ");
    pin = [0; 32];
    pin[0] = 0b0000_0100;
    assert_eq!(SpkiHash(pin).display(), "SHA256:0G00-0000-0000-0000");
    pin[0] = 0b0000_1000;
    assert_eq!(SpkiHash(pin).display(), "SHA256:1000-0000-0000-0000");
    // The hash itself: SHA-256 (FIPS 180-2 "abc"), as sha2 gives on the hub
    let h = SpkiHash::of_spki(b"abc");
    assert_eq!(h.to_hex(), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    assert_eq!(h.curl_pin(), "sha256//ungWv48Bz+pBQUDeXa4iI7ADYaOWF3qctBD/YfIAFa0=");
    assert_eq!(h.to_b64url(), "ungWv48Bz-pBQUDeXa4iI7ADYaOWF3qctBD_YfIAFa0");
    // The rotation message, byte for byte as SECKEY signs it
    let mut expected = b"dspi-key-rotate\0hid-1".to_vec();
    expected.extend_from_slice(&h.0);
    assert_eq!(rotation_message("hid-1", &h), expected);
}
