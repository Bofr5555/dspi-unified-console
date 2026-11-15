//! WP SECKEY: the hub's TLS identity (PLAN-SEC 4.2).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, Once};
use std::time::{Duration, UNIX_EPOCH};

use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer, ServerName};
use rustls::{ClientConfig, ClientConnection, RootCertStore, ServerConfig, ServerConnection};

use super::*;
// -- WP SECSRV -- were the module's own copies
use data_encoding::BASE64URL_NOPAD;
const CROCKFORD: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";
use crate::store::tests_wp_hb1::scratch;

const HID: &str = "0f8e1c2a-3b4d-4e5f-8a9b-0c1d2e3f4a5b";
const HOST: &str = "dspi-0123456789ab.local";

fn at(secs: u64) -> SystemTime {
    UNIX_EPOCH + Duration::from_secs(secs)
}

fn now() -> u64 {
    unix_seconds(SystemTime::now())
}

fn names() -> TlsNames {
    TlsNames::new(Some(HOST), &[])
}

fn load(dir: &Path, names: &TlsNames, t: u64) -> (TlsIdentity, Vec<TlsIdentityEvent>) {
    TlsIdentity::load(dir, HID, names, at(t)).unwrap()
}

#[cfg(unix)]
fn mode(path: &Path) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    fs::metadata(path).unwrap().permissions().mode() & 0o777
}

/// The SPKI a TLS client sees in the certificate, as SECPIPE's verifier reads it.
fn cert_spki(cert_der: &[u8]) -> Vec<u8> {
    let der = CertificateDer::from(cert_der.to_vec());
    let parsed = rustls::server::ParsedCertificate::try_from(&der).unwrap();
    parsed.subject_public_key_info().as_ref().to_vec()
}

/// A TLS 1.3 handshake in memory: the client trusts only this certificate, through
/// webpki (dates, SAN and signature all checked), then one record each way. The
/// certificate the client was shown.
fn handshake(id: &TlsIdentity, server_name: &str) -> Result<Vec<u8>, String> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let cert = CertificateDer::from(id.certificate_der().to_vec());
    let key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(id.private_key_der().to_vec()));
    let server = ServerConfig::builder_with_provider(provider.clone())
        .with_protocol_versions(&[&rustls::version::TLS13])
        .map_err(|e| e.to_string())?
        .with_no_client_auth()
        .with_single_cert(vec![cert.clone()], key)
        .map_err(|e| format!("server config: {e}"))?;
    let mut roots = RootCertStore::empty();
    roots.add(cert).map_err(|e| format!("root: {e}"))?;
    let client = ClientConfig::builder_with_provider(provider)
        .with_protocol_versions(&[&rustls::version::TLS13])
        .map_err(|e| e.to_string())?
        .with_root_certificates(roots)
        .with_no_client_auth();
    let name = ServerName::try_from(server_name.to_owned()).map_err(|e| e.to_string())?;
    let mut c = ClientConnection::new(Arc::new(client), name).map_err(|e| e.to_string())?;
    let mut s = ServerConnection::new(Arc::new(server)).map_err(|e| e.to_string())?;
    c.writer().write_all(b"ping").unwrap();
    let mut got = Vec::new();
    for _ in 0..20 {
        let mut buf = Vec::new();
        while c.wants_write() {
            c.write_tls(&mut buf).unwrap();
        }
        s.read_tls(&mut buf.as_slice()).unwrap();
        s.process_new_packets()
            .map_err(|e| format!("server: {e}"))?;
        let mut buf = Vec::new();
        while s.wants_write() {
            s.write_tls(&mut buf).unwrap();
        }
        c.read_tls(&mut buf.as_slice()).unwrap();
        c.process_new_packets()
            .map_err(|e| format!("client: {e}"))?;
        let mut chunk = [0u8; 16];
        if let Ok(n) = s.reader().read(&mut chunk) {
            got.extend_from_slice(&chunk[..n]);
        }
        if !c.is_handshaking() && !s.is_handshaking() && got == b"ping" {
            let peer = c.peer_certificates().ok_or("no peer certificate")?;
            return Ok(peer[0].as_ref().to_vec());
        }
    }
    Err("no handshake".into())
}

/// Verify a rotation the way a client does: the old key from the announcement, its
/// hash against the pin held, then the signature. ring itself, not this module.
fn client_accepts(held: &SpkiHash, hid: &str, presented: &SpkiHash, next: &KeyNext) -> bool {
    if SpkiHash::of_spki(&next.prev_spki_der) != *held || next.spki != *presented {
        return false;
    }
    // A P-256 SPKI is a fixed 26-byte prefix and the 65-byte uncompressed point.
    let der = &next.prev_spki_der;
    if der.len() != 91 {
        return false;
    }
    let key = ring::signature::UnparsedPublicKey::new(
        &ring::signature::ECDSA_P256_SHA256_ASN1,
        &der[26..],
    );
    key.verify(&rotation_message(hid, presented), &next.sig)
        .is_ok()
}

#[test]
fn wp_seckey_the_first_start_makes_a_private_key_and_a_certificate() {
    let data = scratch("seckey-first");
    let (id, events) = load(&data, &names(), now());
    assert_eq!(events, vec![TlsIdentityEvent::Created { pin: id.pin() }]);
    let tls = data.join(TLS_DIR);
    assert_eq!(id.dir(), tls);
    #[cfg(unix)]
    {
        assert_eq!(mode(&tls), 0o700);
        for f in [KEY_FILE, CERT_FILE, META_FILE] {
            assert_eq!(mode(&tls.join(f)), 0o600, "{f}");
        }
        // And the data folder the hub made on the way.
        assert_eq!(mode(&data), 0o700);
    }
    let key = fs::read_to_string(tls.join(KEY_FILE)).unwrap();
    assert!(key.starts_with("-----BEGIN PRIVATE KEY-----"), "{key:.30}");
    assert_eq!(
        fs::read_to_string(tls.join(CERT_FILE)).unwrap(),
        id.certificate_pem()
    );
    // The pin is SHA-256 of the SPKI the certificate carries.
    assert_eq!(cert_spki(id.certificate_der()), id.spki_der());
    assert_eq!(id.pin(), SpkiHash::of_spki(id.spki_der()));
    assert_eq!(
        id.pin().to_hex(),
        HEXLOWER.encode(&Sha256::digest(id.spki_der()))
    );
    // Display forms.
    let shown = id.fingerprint();
    assert_eq!(shown.len(), "SHA256:".len() + 19, "{shown}");
    assert!(shown.starts_with("SHA256:"));
    let groups: Vec<&str> = shown["SHA256:".len()..].split('-').collect();
    assert_eq!(groups.len(), 4);
    assert!(
        groups
            .iter()
            .all(|g| g.len() == 4 && g.bytes().all(|b| CROCKFORD.contains(&b)))
    );
    assert!(id.pin().curl_pin().starts_with("sha256//"));
    assert_eq!(
        BASE64.decode(&id.pin().curl_pin().as_bytes()[8..]).unwrap(),
        id.pin().0
    );
    assert_eq!(
        BASE64URL_NOPAD
            .decode(id.pin().to_b64url().as_bytes())
            .unwrap(),
        id.pin().0
    );
    // SANs: localhost, the stable name, the mDNS name; no address.
    let stable = format!("{HID}.dspi.local");
    assert_eq!(id.names(), [stable.as_str(), HOST, "localhost"]);
    assert_eq!(id.not_after() - (now() - 3600), VALIDITY_DAYS * DAY);
}

#[test]
fn wp_seckey_the_display_form_is_80_bits_of_crockford_base32() {
    let mut pin = [0u8; 32];
    assert_eq!(SpkiHash(pin).display(), "SHA256:0000-0000-0000-0000");
    pin[..10].copy_from_slice(&[0xFF; 10]);
    pin[10] = 0xAB; // past 80 bits: not shown
    assert_eq!(SpkiHash(pin).display(), "SHA256:ZZZZ-ZZZZ-ZZZZ-ZZZZ");
    pin = [0; 32];
    pin[0] = 0b0000_0100; // the second 5-bit group: 100 then byte 1's top two, 00
    assert_eq!(SpkiHash(pin).display(), "SHA256:0G00-0000-0000-0000");
    pin[0] = 0b0000_1000; // the first group: 00001
    assert_eq!(SpkiHash(pin).display(), "SHA256:1000-0000-0000-0000");
}

#[cfg(unix)]
#[test]
fn wp_seckey_a_widened_folder_and_key_are_made_private_again() {
    use std::os::unix::fs::PermissionsExt;
    let data = scratch("seckey-wide");
    load(&data, &names(), now());
    let tls = data.join(TLS_DIR);
    fs::set_permissions(&tls, fs::Permissions::from_mode(0o755)).unwrap();
    fs::set_permissions(tls.join(KEY_FILE), fs::Permissions::from_mode(0o644)).unwrap();
    let (_, events) = load(&data, &names(), now());
    assert!(events.is_empty(), "{events:?}");
    assert_eq!(mode(&tls), 0o700);
    assert_eq!(mode(&tls.join(KEY_FILE)), 0o600);
}

#[test]
fn wp_seckey_rustls_accepts_the_certificate_for_every_name_it_carries() {
    let data = scratch("seckey-rustls");
    let ip: IpAddr = "192.168.1.50".parse().unwrap();
    let ip6: IpAddr = "fd00::50".parse().unwrap();
    let (id, _) = load(&data, &TlsNames::new(Some(HOST), &[ip, ip6]), now());
    let stable = format!("{HID}.dspi.local");
    for name in [
        "localhost",
        HOST,
        stable.as_str(),
        "192.168.1.50",
        "fd00::50",
    ] {
        let seen = handshake(&id, name).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(seen, id.certificate_der());
        // What a pinning client hashes is the pin.
        assert_eq!(SpkiHash::of_spki(&cert_spki(&seen)), id.pin());
    }
    // A name it does not carry: webpki refuses, so the SANs are what it says.
    let e = handshake(&id, "other.local").unwrap_err();
    assert!(e.contains("client"), "{e}");
    let e = handshake(&id, "192.168.1.51").unwrap_err();
    assert!(e.contains("client"), "{e}");
}

#[test]
fn wp_seckey_a_restart_keeps_the_key_and_the_certificate() {
    let data = scratch("seckey-restart");
    let (a, _) = load(&data, &names(), now());
    let (b, events) = load(&data, &names(), now() + 3 * DAY);
    assert!(events.is_empty(), "{events:?}");
    assert_eq!(a.pin(), b.pin());
    assert_eq!(a.certificate_der(), b.certificate_der());
    assert_eq!(a.private_key_der(), b.private_key_der());
    handshake(&b, HOST).unwrap();
}

/// S7: an address change is not a reason. Only configured static addresses are an
/// input at all, and their order does not matter.
#[test]
fn wp_seckey_an_ip_address_change_does_not_reissue() {
    let data = scratch("seckey-ip");
    let s1: IpAddr = "10.0.0.2".parse().unwrap();
    let s2: IpAddr = "10.0.0.3".parse().unwrap();
    let (mut id, _) = load(&data, &TlsNames::new(Some(HOST), &[s1, s2]), now());
    let cert = id.certificate_der().to_vec();
    // The DHCP lease moved (the hub's interfaces now have other addresses): the names
    // the hub computes are the same, so nothing is issued, now or after a restart.
    let same = TlsNames::new(Some("DSPI-0123456789AB.local."), &[s2, s1, s2]);
    assert_eq!(id.refresh(&same, at(now() + DAY)).unwrap(), None);
    assert_eq!(id.certificate_der(), cert);
    let (id, events) = load(&data, &same, now() + 2 * DAY);
    assert!(events.is_empty(), "{events:?}");
    assert_eq!(id.certificate_der(), cert);
}

#[test]
fn wp_seckey_a_reissue_keeps_the_spki() {
    let data = scratch("seckey-reissue");
    let t = now();
    let (mut id, _) = load(&data, &names(), t);
    let pin = id.pin();
    let first = id.certificate_der().to_vec();

    // The responder took another host name after a conflict.
    let renamed = TlsNames::new(Some("dspi-0123456789ab-2.local"), &[]);
    let event = id.refresh(&renamed, at(t + 60)).unwrap();
    assert_eq!(
        event,
        Some(TlsIdentityEvent::CertificateIssued {
            pin,
            reason: IssueReason::NamesChanged
        })
    );
    assert_ne!(id.certificate_der(), first);
    assert_eq!(id.pin(), pin);
    assert_eq!(SpkiHash::of_spki(&cert_spki(id.certificate_der())), pin);
    handshake(&id, "dspi-0123456789ab-2.local").unwrap();
    assert!(handshake(&id, HOST).is_err());

    // A configured static address is a name too.
    let ip: IpAddr = "10.1.1.1".parse().unwrap();
    let with_ip = TlsNames::new(Some("dspi-0123456789ab-2.local"), &[ip]);
    let event = id.refresh(&with_ip, at(t + 120)).unwrap();
    assert!(matches!(
        event,
        Some(TlsIdentityEvent::CertificateIssued {
            reason: IssueReason::NamesChanged,
            ..
        })
    ));

    // Near the end: issued again, same key. Not 31 days before, yes 29.
    let end = id.not_after();
    assert_eq!(id.refresh(&with_ip, at(end - 31 * DAY)).unwrap(), None);
    let before = id.certificate_der().to_vec();
    let event = id.refresh(&with_ip, at(end - 29 * DAY)).unwrap();
    assert_eq!(
        event,
        Some(TlsIdentityEvent::CertificateIssued {
            pin,
            reason: IssueReason::NearExpiry
        })
    );
    assert_ne!(id.certificate_der(), before);
    assert_eq!(SpkiHash::of_spki(&cert_spki(id.certificate_der())), pin);
    assert!(id.not_after() > end);

    // And at the next start the same key and the new certificate are found.
    let (again, events) = load(&data, &with_ip, end - 29 * DAY + 1);
    assert!(events.is_empty(), "{events:?}");
    assert_eq!(again.certificate_der(), id.certificate_der());
    assert_eq!(again.pin(), pin);
}

/// A Pi with no battery clock: issued from the floor, never renewed on that clock.
#[test]
fn wp_seckey_a_clock_in_1970_issues_from_the_floor_and_does_not_churn() {
    let data = scratch("seckey-1970");
    let (mut id, _) = load(&data, &names(), 5);
    assert_eq!(id.not_after(), CLOCK_FLOOR + VALIDITY_DAYS * DAY);
    let cert = id.certificate_der().to_vec();
    assert_eq!(id.refresh(&names(), at(600)).unwrap(), None);
    let (id2, events) = load(&data, &names(), 900);
    assert!(events.is_empty());
    assert_eq!(id2.certificate_der(), cert);
    // NTP set the clock: still inside its 825 days, so no new certificate either.
    assert_eq!(
        id.refresh(&names(), at(CLOCK_FLOOR + 400 * DAY)).unwrap(),
        None
    );
}

#[test]
fn wp_seckey_the_rotation_signature_verifies_and_a_forged_one_does_not() {
    let data = scratch("seckey-rotate");
    let t = now();
    let (mut id, _) = load(&data, &names(), t);
    let old = id.pin();
    assert_eq!(id.key_next(at(t)), None);
    let event = id.rotate(&names(), at(t + 10), false).unwrap();
    let new = id.pin();
    assert_ne!(new, old);
    assert_eq!(
        event,
        TlsIdentityEvent::Rotated {
            pin: new,
            previous: old,
            signed: true
        }
    );
    assert_eq!(SpkiHash::of_spki(&cert_spki(id.certificate_der())), new);
    handshake(&id, HOST).unwrap();

    let next = id.key_next(at(t + 20)).unwrap();
    assert_eq!(next.spki, new);
    assert_eq!(next.until, t + 10 + ROTATION_DAYS * DAY);
    assert!(client_accepts(&old, HID, &new, &next));

    // Forgeries.
    let mut bad = next.clone();
    let last = bad.sig.len() - 1;
    bad.sig[last] ^= 1;
    assert!(
        !client_accepts(&old, HID, &new, &bad),
        "a changed signature"
    );
    assert!(
        !client_accepts(&old, "another-hub", &new, &next),
        "another hub's id"
    );
    let other = SpkiHash([9; 32]);
    assert!(!client_accepts(&old, HID, &other, &next), "another new key");
    let attacker = KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256).unwrap();
    let forged = KeyNext {
        sig: attacker.sign(&rotation_message(HID, &new)).unwrap(),
        ..next.clone()
    };
    assert!(
        !client_accepts(&old, HID, &new, &forged),
        "signed by another key"
    );
    let swapped = KeyNext {
        prev_spki_der: attacker.subject_public_key_info(),
        ..forged
    };
    assert!(
        !client_accepts(&old, HID, &new, &swapped),
        "another key passed off as the old"
    );

    // Kept across a restart; only the public half of the old key is kept.
    let (mut id, events) = load(&data, &names(), t + DAY);
    assert!(events.is_empty(), "{events:?}");
    assert_eq!(id.key_next(at(t + DAY)), Some(next.clone()));
    let mut files: Vec<String> = fs::read_dir(data.join(TLS_DIR))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    files.sort();
    assert_eq!(files, [CERT_FILE, META_FILE, KEY_FILE]);

    // Over after 30 days.
    let after = t + 10 + ROTATION_DAYS * DAY;
    assert_eq!(id.key_next(at(after - 1)), Some(next));
    assert_eq!(id.key_next(at(after)), None);
    assert_eq!(id.refresh(&names(), at(after)).unwrap(), None);
    let (id, _) = load(&data, &names(), t + DAY);
    assert_eq!(id.key_next(at(t + DAY)), None, "the end was saved");
}

#[test]
fn wp_seckey_rotate_forget_old_signs_nothing() {
    let data = scratch("seckey-forget");
    let t = now();
    let (mut id, _) = load(&data, &names(), t);
    let old = id.pin();
    let event = id.rotate(&names(), at(t), true).unwrap();
    assert_eq!(
        event,
        TlsIdentityEvent::Rotated {
            pin: id.pin(),
            previous: old,
            signed: false
        }
    );
    assert_eq!(id.key_next(at(t)), None);
    let (id, _) = load(&data, &names(), t);
    assert_eq!(id.key_next(at(t)), None);
    assert_ne!(id.pin(), old);
}

#[test]
fn wp_seckey_a_corrupt_key_is_set_aside_and_replaced() {
    let data = scratch("seckey-corrupt");
    let t = now();
    let (first, _) = load(&data, &names(), t);
    let tls = data.join(TLS_DIR);
    let ed25519 = KeyPair::generate_for(&rcgen::PKCS_ED25519)
        .unwrap()
        .serialize_pem();
    let big = "A".repeat(usize::try_from(MAX_PEM).unwrap() + 1);
    let cases: [(&str, &[u8]); 4] = [
        ("garbage", b"not a key at all \xff\x00"),
        ("not P-256", ed25519.as_bytes()),
        (
            "broken base64",
            b"-----BEGIN PRIVATE KEY-----\n!!!!\n-----END PRIVATE KEY-----\n",
        ),
        ("oversized", big.as_bytes()),
    ];
    let mut pins = vec![first.pin()];
    for (i, (what, bytes)) in cases.iter().enumerate() {
        fs::write(tls.join(KEY_FILE), bytes).unwrap();
        let when = t + 10 * (i as u64 + 1);
        let (id, events) = load(&data, &names(), when);
        let aside = tls.join(format!("{KEY_FILE}.bad-{when}"));
        assert_eq!(
            events,
            vec![TlsIdentityEvent::KeyReplaced {
                pin: id.pin(),
                set_aside: Some(aside.clone())
            }],
            "{what}"
        );
        assert_eq!(fs::read(&aside).unwrap(), *bytes, "{what}");
        assert!(!pins.contains(&id.pin()), "{what}: a new key");
        pins.push(id.pin());
        assert_eq!(
            SpkiHash::of_spki(&cert_spki(id.certificate_der())),
            id.pin()
        );
        handshake(&id, HOST).unwrap();
        #[cfg(unix)]
        assert_eq!(mode(&tls.join(KEY_FILE)), 0o600);
    }
    // Missing, where a hub had one: replaced, and said so.
    fs::remove_file(tls.join(KEY_FILE)).unwrap();
    let (id, events) = load(&data, &names(), t + 100);
    assert_eq!(
        events,
        vec![TlsIdentityEvent::KeyReplaced {
            pin: id.pin(),
            set_aside: None
        }]
    );
    // A lost certificate or identity.json: issued again, same key, no "key changed".
    let pin = id.pin();
    fs::remove_file(tls.join(CERT_FILE)).unwrap();
    let (id, events) = load(&data, &names(), t + 200);
    assert_eq!(
        events,
        vec![TlsIdentityEvent::CertificateIssued {
            pin,
            reason: IssueReason::Unreadable
        }]
    );
    fs::write(tls.join(META_FILE), b"{").unwrap();
    let (id2, events) = load(&data, &names(), t + 300);
    assert_eq!(
        events,
        vec![TlsIdentityEvent::CertificateIssued {
            pin,
            reason: IssueReason::Unreadable
        }]
    );
    assert_eq!(id2.pin(), id.pin());
    // Another certificate dropped in place of ours is not taken for it.
    fs::write(tls.join(CERT_FILE), first.certificate_pem()).unwrap();
    let (id3, events) = load(&data, &names(), t + 400);
    assert_eq!(events.len(), 1, "{events:?}");
    assert_eq!(SpkiHash::of_spki(&cert_spki(id3.certificate_der())), pin);
}

#[test]
fn wp_seckey_a_tls_path_that_is_not_a_folder_is_an_error_not_a_panic() {
    let data = scratch("seckey-notdir");
    fs::create_dir_all(&data).unwrap();
    fs::write(data.join(TLS_DIR), b"x").unwrap();
    let e = TlsIdentity::load(&data, HID, &names(), at(now())).unwrap_err();
    assert!(matches!(e, HubError::Io(_)), "{e:?}");
}

#[test]
fn wp_seckey_bad_host_names_are_left_out() {
    let n = TlsNames::new(Some("Living room.local"), &[]);
    assert_eq!(
        n.sans(HID),
        [
            "0f8e1c2a-3b4d-4e5f-8a9b-0c1d2e3f4a5b.dspi.local",
            "localhost"
        ]
    );
    let n = TlsNames::new(Some(&format!("{}.local", "a".repeat(64))), &[]);
    assert_eq!(n.sans(HID).len(), 2);
    let n = TlsNames::new(Some("  "), &[]);
    assert_eq!(n.sans(HID).len(), 2);
    assert!(is_dns_name("dspi-0123456789ab-2.local"));
    assert!(!is_dns_name("-x.local"));
    assert!(!is_dns_name("a..local"));
}

// -- no key material anywhere (S9) --

struct Capture;
static LINES: Mutex<Vec<String>> = Mutex::new(Vec::new());
static CAPTURE: Capture = Capture;

impl log::Log for Capture {
    fn enabled(&self, _: &log::Metadata<'_>) -> bool {
        true
    }
    fn log(&self, record: &log::Record<'_>) {
        LINES
            .lock()
            .unwrap()
            .push(format!("{} {}", record.target(), record.args()));
    }
    fn flush(&self) {}
}

fn capture_logs() -> bool {
    static ONCE: Once = Once::new();
    static OURS: AtomicBool = AtomicBool::new(false);
    ONCE.call_once(|| {
        OURS.store(log::set_logger(&CAPTURE).is_ok(), Ordering::SeqCst);
        log::set_max_level(log::LevelFilter::Trace);
    });
    OURS.load(Ordering::SeqCst)
}

/// Every form a key's secret could leak in: the PKCS#8 DER, the private scalar, and
/// their hex and base64 (whole, and the PEM's lines).
fn secret_forms(pkcs8: &[u8]) -> Vec<Vec<u8>> {
    let mut forms = vec![pkcs8.to_vec()];
    // ECPrivateKey: version 1, then the 32-byte scalar (RFC 5915).
    let at = pkcs8
        .windows(5)
        .position(|w| w == [0x02, 0x01, 0x01, 0x04, 0x20])
        .expect("an ECPrivateKey");
    let scalar = pkcs8[at + 5..at + 37].to_vec();
    forms.push(scalar.clone());
    for bytes in [pkcs8, scalar.as_slice()] {
        forms.push(HEXLOWER.encode(bytes).into_bytes());
        forms.push(data_encoding::HEXUPPER.encode(bytes).into_bytes());
        forms.push(BASE64.encode(bytes).into_bytes());
        forms.push(BASE64URL_NOPAD.encode(bytes).into_bytes());
    }
    // Any 40 characters of the PEM body (a PEM line is 64).
    let b64 = BASE64.encode(pkcs8);
    for start in (0..b64.len().saturating_sub(40)).step_by(20) {
        forms.push(b64.as_bytes()[start..start + 40].to_vec());
    }
    forms
}

fn assert_no_secret(what: &str, haystack: &[u8], forms: &[Vec<u8>]) {
    for f in forms {
        assert!(
            !haystack.windows(f.len()).any(|w| w == f.as_slice()),
            "{what} carries key material ({} bytes)",
            f.len()
        );
    }
}

#[test]
fn wp_seckey_no_key_material_in_debug_logs_events_errors_or_the_bundle() {
    let ours = capture_logs();
    let data = scratch("seckey-secret");
    let t = now();
    // A hub store beside it, as on a real hub, so the bundle reads the folder.
    crate::store::HubStore::load(&data).unwrap();

    let mut secrets = Vec::new();
    let mut texts = Vec::new();
    let (mut id, events) = load(&data, &names(), t);
    secrets.extend(secret_forms(id.private_key_der()));
    texts.push(format!("{id:?} {events:?} {:?} {}", id.pin(), id.pin()));
    // A rotation, and the old key too.
    let event = id.rotate(&names(), at(t + 1), false).unwrap();
    secrets.extend(secret_forms(id.private_key_der()));
    texts.push(format!("{id:?} {event:?} {:?}", id.key_next(at(t + 1))));
    // A re-issue and a replaced key.
    let renamed = TlsNames::new(Some("other.local"), &[]);
    texts.push(format!("{:?}", id.refresh(&renamed, at(t + 2))));
    fs::write(data.join(TLS_DIR).join(KEY_FILE), b"junk").unwrap();
    let (id, events) = load(&data, &names(), t + 3);
    secrets.extend(secret_forms(id.private_key_der()));
    texts.push(format!("{id:?} {events:?} {:?}", names()));
    // An error from this module.
    let not_dir = scratch("seckey-secret-err");
    fs::create_dir_all(&not_dir).unwrap();
    fs::write(not_dir.join(TLS_DIR), b"x").unwrap();
    let e = TlsIdentity::load(&not_dir, HID, &names(), at(t)).unwrap_err();
    texts.push(format!("{e} {e:?}"));

    for (i, text) in texts.iter().enumerate() {
        assert_no_secret(&format!("text {i}"), text.as_bytes(), &secrets);
    }
    assert!(texts[0].contains("[private key]"), "{}", texts[0]);

    // The log, which the hub's diagnostics and journald keep.
    if ours {
        let lines = LINES.lock().unwrap().join("\n");
        assert!(
            lines.contains("could not be read"),
            "the capture works: {lines}"
        );
        assert_no_secret("the log", lines.as_bytes(), &secrets);
    }

    // The diagnostics bundle over the hub's data folder: stored, not compressed, so
    // the bytes are searched as they are.
    let core = dspi_core::DspiCore::new();
    let zip = core
        .diagnostics_zip(dspi_core::DiagOptions {
            app: "wp-seckey".into(),
            hub_store_dir: data.to_string_lossy().into_owned(),
            files: Vec::new(),
        })
        .unwrap();
    assert!(
        zip.windows(b"hub/hub-store.json".len())
            .any(|w| w == b"hub/hub-store.json"),
        "the bundle read the folder"
    );
    assert_no_secret("the diagnostics bundle", &zip, &secrets);
}
