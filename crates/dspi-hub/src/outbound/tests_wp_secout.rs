//! WP SECOUT: the outbound TLS connector, HTTPS webhooks, the plaintext rule (OD-SECe), the
//! trusts (OD-SECf), the migration of targets written before TLS, and the review notes on
//! P2WH (a kind change, the name lookup inside the deadline, a corrected URL).
//!
//! TLS receivers run in this test on the loopback with the TM12 test certificates
//! (`crates/dspi-tui/testdata/tm12/`: a CA, and a leaf for `DNS:localhost` it signed).
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

use super::*;
use crate::clock::TestClock;
use std::io::{Read, Write};
use std::net::{IpAddr, TcpListener};
use std::sync::atomic::AtomicU32;
use std::sync::mpsc;

use data_encoding::BASE64;
use proptest::prelude::*;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use rustls::{ServerConfig, ServerConnection, StreamOwned};

const CA: &[u8] = include_bytes!("../../../dspi-tui/testdata/tm12/ca.der");
const LEAF: &[u8] = include_bytes!("../../../dspi-tui/testdata/tm12/leaf.der");
const LEAF_KEY: &[u8] = include_bytes!("../../../dspi-tui/testdata/tm12/leaf-key.der");
// -- WP P2CFIXB --
/// `openssl req -x509` (OpenSSL 3) for DNS:localhost: self-signed and marked CA:TRUE.
const SELF_CA_TRUE: &[u8] = include_bytes!("../../testdata/p2cfixb/self-ca-true.der");
const SELF_CA_TRUE_KEY: &[u8] = include_bytes!("../../testdata/p2cfixb/self-ca-true-key.der");
/// `openssl x509 -pubkey | openssl pkey -pubin -outform der | openssl dgst -sha256 -binary
/// | base64`, of each.
const LEAF_PIN: &str = "sha256//QIoTMalqjAmU7cqIfPIw0djIOMn8E2uRqx/dfg0/EiE=";
const CA_PIN: &str = "sha256//TvRMfCrv5h+dn2AsWNZpHWAJfVdXDQVjdGLYOIsvft8=";

fn pem(label: &str, der: &[u8]) -> String {
    let b64 = BASE64.encode(der);
    let mut out = format!("-----BEGIN {label}-----\n");
    for line in b64.as_bytes().chunks(64) {
        out.push_str(std::str::from_utf8(line).unwrap());
        out.push('\n');
    }
    out.push_str(&format!("-----END {label}-----\n"));
    out
}

fn scratch(what: &str) -> PathBuf {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    // `ci/run.sh` points TMPDIR at ~/.cache/dspi-build/work, not tmpfs.
    let dir = std::env::temp_dir().join("wp-secout-unit").join(format!(
        "{what}-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::SeqCst)
    ));
    let _ = fs::remove_dir_all(&dir);
    dir
}

// ── The connector's pieces ──────────────────────────────────────────────────

#[test]
fn wp_secout_spki_pin_matches_openssl() {
    assert_eq!(tls::pin_text(&tls::spki_sha256(LEAF).unwrap()), LEAF_PIN);
    assert_eq!(tls::pin_text(&tls::spki_sha256(CA).unwrap()), CA_PIN);
    // Not a certificate, a certificate with bytes after it, a truncated one.
    assert!(tls::spki_der(b"").is_none());
    assert!(tls::spki_der(LEAF_KEY).is_none());
    assert!(tls::spki_der(&[LEAF, b"\0"].concat()).is_none());
    assert!(tls::spki_der(&LEAF[..LEAF.len() - 1]).is_none());
    // A non-minimal length (0x82 0x00 0x03 for 3) is not DER.
    assert!(tls::spki_der(&[0x30, 0x82, 0x00, 0x03, 0x02, 0x01, 0x00]).is_none());
    // Indefinite length.
    assert!(tls::spki_der(&[0x30, 0x80, 0x00, 0x00]).is_none());
}

proptest! {
    #[test]
    fn proptest_wp_secout_spki_never_panics(bytes in proptest::collection::vec(any::<u8>(), 0..600)) {
        let _ = tls::spki_der(&bytes);
    }

    #[test]
    fn proptest_wp_secout_spki_of_a_mutated_certificate_never_panics(at in 0..LEAF.len(), b in any::<u8>()) {
        let mut cert = LEAF.to_vec();
        cert[at] = b;
        let _ = tls::spki_der(&cert);
    }
}

#[test]
fn wp_secout_pins_in_every_form_and_none_else() {
    let want = tls::spki_sha256(LEAF).unwrap();
    let hex = HEXLOWER.encode(&want);
    let colons: Vec<String> = want.iter().map(|b| format!("{b:02X}")).collect();
    for text in [
        LEAF_PIN.to_owned(),
        LEAF_PIN.trim_start_matches("sha256//").to_owned(),
        hex,
        colons.join(":"),
        format!("  {LEAF_PIN}\n"),
    ] {
        assert_eq!(tls::parse_pin(&text), Ok(want), "{text}");
    }
    let zeros = "0".repeat(63);
    for bad in [
        "",
        "sha256//",
        "sha256//abc",
        "00",
        zeros.as_str(),
        "sha256//00ff",
    ] {
        assert!(tls::parse_pin(bad).is_err(), "{bad}");
    }
}

#[test]
fn wp_secout_pem_rules() {
    let ca = pem("CERTIFICATE", CA);
    assert_eq!(tls::parse_certs("ca_pem", &ca).unwrap().len(), 1);
    let two = format!("{ca}{}", pem("CERTIFICATE", LEAF));
    assert_eq!(tls::parse_certs("ca_pem", &two).unwrap().len(), 2);
    for bad in [
        String::new(),
        "not a pem".to_owned(),
        pem("PRIVATE KEY", LEAF_KEY),
        pem("CERTIFICATE", b"garbage"),
        ca.repeat(17),
        format!("{ca}{}", " ".repeat(tls::MAX_PEM_BYTES)),
    ] {
        assert!(tls::parse_certs("ca_pem", &bad).is_err(), "{bad:.40}");
    }
    let key_pem = pem("PRIVATE KEY", LEAF_KEY);
    assert!(tls::parse_key("client_key_pem", &key_pem).is_ok());
    let err = tls::parse_key(
        "client_key_pem",
        "-----BEGIN PRIVATE KEY-----\nsecret!!\n-----END PRIVATE KEY-----\n",
    )
    .unwrap_err()
    .to_string();
    assert!(!err.contains("secret!!"), "{err}");
    // A key that decodes but is not one: refused when the client is made, unquoted.
    let junk = tls::ClientIdentity::from_pem(
        &pem("CERTIFICATE", LEAF),
        "-----BEGIN PRIVATE KEY-----\nc2VjcmV0LWtleS1ieXRlcw==\n-----END PRIVATE KEY-----\n",
    )
    .unwrap();
    let err = tls::client_config(&tls::Trust::Roots, Some(&junk), &[])
        .unwrap_err()
        .to_string();
    assert!(
        !err.contains("c2VjcmV0") && !err.contains("secret-key"),
        "{err}"
    );
    // The identity's Debug never holds the key.
    let id = tls::ClientIdentity::from_pem(&pem("CERTIFICATE", LEAF), &key_pem).unwrap();
    let shown = format!("{id:?}");
    let b64 = BASE64.encode(LEAF_KEY);
    assert!(!shown.contains(&b64[10..30]), "{shown}");
    assert!(shown.contains("\"set\""), "{shown}");
    // Both trusts at once is refused.
    assert!(tls::Trust::from_settings(Some(&ca), Some(LEAF_PIN)).is_err());
}

#[test]
fn wp_secout_where_plaintext_may_go() {
    let ip = |s: &str| s.parse::<IpAddr>().unwrap();
    for ok in [
        "127.0.0.1",
        "127.8.9.1",
        "10.1.2.3",
        "172.16.0.1",
        "172.31.255.255",
        "192.168.1.5",
        "169.254.7.7",
        "::1",
        "fe80::1",
        "febf::1",
        "fd12:3456::1",
        "fc00::1",
        "::ffff:192.168.1.5",
        "::ffff:127.0.0.1",
    ] {
        assert!(tls::plaintext_allowed(ip(ok)), "{ok}");
    }
    for no in [
        "8.8.8.8",
        "1.1.1.1",
        "172.32.0.1",
        "100.64.0.1",
        "192.0.2.1",
        "0.0.0.0",
        "255.255.255.255",
        "224.0.0.1",
        "2001:db8::1",
        "2606:4700::1111",
        "::",
        "ff02::1",
        "fec0::1",
        "::ffff:8.8.8.8",
    ] {
        assert!(!tls::plaintext_allowed(ip(no)), "{no}");
    }
    assert!(tls::is_loopback(ip("127.0.0.1")) && tls::is_loopback(ip("::1")));
    assert!(tls::is_loopback(ip("::ffff:127.0.0.1")));
    assert!(!tls::is_loopback(ip("192.168.1.5")) && !tls::is_loopback(ip("fe80::1")));
}

#[test]
fn wp_secout_server_names_sni_from_the_host_ip_for_literals() {
    use rustls::pki_types::ServerName;
    assert!(
        matches!(tls::server_name("Hooks.Example.com").unwrap(), ServerName::DnsName(d) if d.as_ref() == "hooks.example.com")
    );
    assert!(matches!(
        tls::server_name("10.0.0.2").unwrap(),
        ServerName::IpAddress(_)
    ));
    assert!(matches!(
        tls::server_name("fd00::5").unwrap(),
        ServerName::IpAddress(_)
    ));
    assert!(matches!(
        tls::server_name("[fd00::5]").unwrap(),
        ServerName::IpAddress(_)
    ));
    assert!(tls::server_name("bad_name!").is_err());
}

// ── A TLS receiver on the loopback ──────────────────────────────────────────

/// What the receiver saw of one connection.
#[derive(Debug)]
enum Got {
    /// The handshake failed: why, and how many application bytes had come.
    Refused(String, usize),
    /// A whole request.
    Request(Vec<u8>),
}

fn server_config(tls12_only: bool) -> Arc<ServerConfig> {
    server_config_with(LEAF, LEAF_KEY, tls12_only)
}

fn server_config_with(cert: &[u8], key: &[u8], tls12_only: bool) -> Arc<ServerConfig> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let versions: &[&rustls::SupportedProtocolVersion] = if tls12_only {
        &[&rustls::version::TLS12]
    } else {
        &[&rustls::version::TLS13, &rustls::version::TLS12]
    };
    Arc::new(
        ServerConfig::builder_with_provider(provider)
            .with_protocol_versions(versions)
            .unwrap()
            .with_no_client_auth()
            .with_single_cert(
                vec![CertificateDer::from(cert.to_vec())],
                PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key.to_vec())),
            )
            .unwrap(),
    )
}

/// A TLS receiver answering each request with `reply`.
fn tls_receiver(reply: &'static [u8], tls12_only: bool) -> (u16, mpsc::Receiver<Got>) {
    tls_receiver_with(server_config(tls12_only), reply)
}

fn tls_receiver_with(
    config: Arc<ServerConfig>,
    reply: &'static [u8],
) -> (u16, mpsc::Receiver<Got>) {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        for s in l.incoming() {
            let Ok(mut s) = s else { return };
            let tx = tx.clone();
            let config = Arc::clone(&config);
            std::thread::spawn(move || {
                s.set_read_timeout(Some(Duration::from_secs(20))).unwrap();
                let mut conn = ServerConnection::new(config).unwrap();
                while conn.is_handshaking() {
                    if let Err(e) = conn.complete_io(&mut s) {
                        let mut plain = Vec::new();
                        let _ = conn.reader().read_to_end(&mut plain);
                        let _ = tx.send(Got::Refused(e.to_string(), plain.len()));
                        return;
                    }
                }
                let mut tls = StreamOwned::new(conn, s);
                let mut req = Vec::new();
                let mut b = [0u8; 4096];
                loop {
                    let n = tls.read(&mut b).unwrap_or(0);
                    if n == 0 {
                        break;
                    }
                    req.extend_from_slice(&b[..n]);
                    let text = String::from_utf8_lossy(&req).to_string();
                    if let Some(end) = text.find("\r\n\r\n") {
                        let len: usize = text
                            .lines()
                            .find_map(|l| l.strip_prefix("Content-Length: "))
                            .map_or(0, |v| v.trim().parse().unwrap());
                        if req.len() >= end + 4 + len {
                            break;
                        }
                    }
                }
                let _ = tls.write_all(reply);
                let _ = tls.flush();
                tls.conn.send_close_notify();
                let _ = tls.flush();
                let _ = tx.send(Got::Request(req));
            });
        }
    });
    (port, rx)
}

fn send(url: &str, trust: &tls::Trust) -> Result<u16, PostError> {
    let u = http::parse_url(url).unwrap();
    let config = tls::client_config(trust, None, &[http::ALPN]).unwrap();
    let p = Post {
        url: &u,
        tls: Some(&config),
        content_type: "application/json",
        headers: &[],
        body: b"{\"a\":1}",
    };
    http::post(&p, &HttpLimits::default())
}

fn got(rx: &mpsc::Receiver<Got>) -> Got {
    rx.recv_timeout(Duration::from_secs(20)).unwrap()
}

fn ca_trust() -> tls::Trust {
    tls::Trust::from_settings(Some(&pem("CERTIFICATE", CA)), None).unwrap()
}

fn pin_trust(pin: &str) -> tls::Trust {
    tls::Trust::from_settings(None, Some(pin)).unwrap()
}

#[test]
fn wp_secout_https_delivered_with_a_pin_and_with_a_ca() {
    let (port, rx) = tls_receiver(b"HTTP/1.1 204 No Content\r\n\r\n", false);
    // A pin: the address is not in the certificate, and that is fine.
    assert_eq!(
        send(
            &format!("https://127.0.0.1:{port}/hook"),
            &pin_trust(LEAF_PIN)
        ),
        Ok(204)
    );
    let Got::Request(req) = got(&rx) else {
        panic!("refused")
    };
    let req = String::from_utf8(req).unwrap();
    assert!(req.starts_with("POST /hook HTTP/1.1\r\n"), "{req}");
    assert!(
        req.contains(&format!("\r\nHost: 127.0.0.1:{port}\r\n")),
        "{req}"
    );
    assert!(req.ends_with("\r\n\r\n{\"a\":1}"), "{req}");
    // A CA and the host name in the certificate.
    assert_eq!(
        send(&format!("https://localhost:{port}/x"), &ca_trust()),
        Ok(204)
    );
    assert!(matches!(got(&rx), Got::Request(_)));
    // WP P2CFIXB: the receiver's exact certificate as the trust is taken (its name checked),
    // whoever issued it.
    let own = tls::Trust::from_settings(Some(&pem("CERTIFICATE", LEAF)), None).unwrap();
    assert_eq!(send(&format!("https://localhost:{port}/x"), &own), Ok(204));
    assert!(matches!(got(&rx), Got::Request(_)));
    let r = send(&format!("https://127.0.0.1:{port}/x"), &own);
    assert!(
        matches!(&r, Err(PostError::Tls(TlsError::Certificate(why))) if why.contains("not for 127.0.0.1")),
        "{r:?}"
    );
    assert!(matches!(got(&rx), Got::Refused(why, 0) if !why.is_empty()));
}

// -- WP P2CFIXB --
#[test]
fn wp_p2cfixb_a_ca_true_self_signed_receiver_given_as_ca_pem_is_trusted() {
    let (port, rx) = tls_receiver_with(
        server_config_with(SELF_CA_TRUE, SELF_CA_TRUE_KEY, false),
        b"HTTP/1.1 204 No Content\r\n\r\n",
    );
    let exact = tls::Trust::from_settings(Some(&pem("CERTIFICATE", SELF_CA_TRUE)), None).unwrap();
    assert_eq!(
        send(&format!("https://localhost:{port}/x"), &exact),
        Ok(204)
    );
    assert!(matches!(got(&rx), Got::Request(_)));
    // The name is still checked.
    let r = send(&format!("https://127.0.0.1:{port}/x"), &exact);
    assert!(
        matches!(&r, Err(PostError::Tls(TlsError::Certificate(why))) if why.contains("not for 127.0.0.1")),
        "{r:?}"
    );
    assert!(matches!(got(&rx), Got::Refused(_, 0)));
    // Given something else (another CA, or the public roots): refused, saying what to do.
    for other in [ca_trust(), tls::Trust::Roots] {
        let r = send(&format!("https://localhost:{port}/x"), &other);
        assert_eq!(
            r,
            Err(PostError::Tls(TlsError::Certificate(
                tls::CA_AS_END_ENTITY.into()
            ))),
            "{other:?}"
        );
        assert!(matches!(got(&rx), Got::Refused(_, 0)));
    }
    // Its pin works too.
    let pin = tls::pin_text(&tls::spki_sha256(SELF_CA_TRUE).unwrap());
    assert_eq!(
        send(&format!("https://127.0.0.1:{port}/x"), &pin_trust(&pin)),
        Ok(204)
    );
}

#[test]
fn wp_secout_tls12_receivers_still_work() {
    let (port, rx) = tls_receiver(b"HTTP/1.1 200 OK\r\n\r\n", true);
    assert_eq!(
        send(&format!("https://localhost:{port}/"), &ca_trust()),
        Ok(200)
    );
    assert!(matches!(got(&rx), Got::Request(_)));
    assert_eq!(
        send(&format!("https://127.0.0.1:{port}/"), &pin_trust(LEAF_PIN)),
        Ok(200)
    );
}

#[test]
fn wp_secout_wrong_host_wrong_pin_and_unknown_ca_are_refused_before_a_byte() {
    let (port, rx) = tls_receiver(b"HTTP/1.1 200 OK\r\n\r\n", false);
    // The certificate is for localhost, not for the address.
    let r = send(&format!("https://127.0.0.1:{port}/"), &ca_trust());
    match &r {
        Err(PostError::Tls(TlsError::Certificate(why))) => {
            assert!(why.contains("not for 127.0.0.1"), "{why}");
        }
        other => panic!("{other:?}"),
    }
    assert!(matches!(got(&rx), Got::Refused(why, 0) if !why.is_empty()));
    // Another key's pin.
    let r = send(&format!("https://127.0.0.1:{port}/"), &pin_trust(CA_PIN));
    assert_eq!(r, Err(PostError::Tls(TlsError::PinMismatch)));
    assert!(matches!(got(&rx), Got::Refused(why, 0) if !why.is_empty()));
    // The public CAs do not know the test CA.
    let r = send(&format!("https://localhost:{port}/"), &tls::Trust::Roots);
    match &r {
        Err(PostError::Tls(TlsError::Certificate(why))) => {
            assert!(why.contains("trusted CA"), "{why}");
        }
        other => panic!("{other:?}"),
    }
    assert!(matches!(got(&rx), Got::Refused(why, 0) if !why.is_empty()));
}

#[test]
fn wp_secout_a_silent_tls_receiver_is_cut_at_the_total_limit() {
    // Takes the connection (the backlog does) and never answers the ClientHello.
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let u = http::parse_url(&format!(
        "https://127.0.0.1:{}/",
        l.local_addr().unwrap().port()
    ))
    .unwrap();
    let config = tls::client_config(&pin_trust(LEAF_PIN), None, &[http::ALPN]).unwrap();
    let p = Post {
        url: &u,
        tls: Some(&config),
        content_type: "text/plain",
        headers: &[],
        body: b"x",
    };
    let limits = HttpLimits {
        total: Duration::from_millis(300),
        ..HttpLimits::default()
    };
    let started = Instant::now();
    assert_eq!(
        http::post(&p, &limits),
        Err(PostError::Tls(TlsError::Timeout))
    );
    assert!(started.elapsed() < Duration::from_secs(3));
    drop(l);
}

#[test]
fn wp_secout_plaintext_goes_to_private_addresses_only_checked_per_send() {
    // A public address: refused before any connection is tried.
    let u = http::parse_url("http://192.0.2.1:9/hook").unwrap();
    let p = Post {
        url: &u,
        tls: None,
        content_type: "text/plain",
        headers: &[],
        body: b"x",
    };
    let started = Instant::now();
    let r = http::post(&p, &HttpLimits::default());
    assert_eq!(r, Err(PostError::PublicPlaintext("192.0.2.1".into())));
    assert!(r.unwrap_err().to_string().contains("unencrypted"));
    assert!(started.elapsed() < Duration::from_secs(1));
    // A private one (the loopback) is delivered.
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port();
    std::thread::spawn(move || {
        let (mut s, _) = l.accept().unwrap();
        let mut b = [0u8; 1024];
        let _ = s.read(&mut b);
        let _ = s.write_all(b"HTTP/1.1 202 Accepted\r\n\r\n");
    });
    let u = http::parse_url(&format!("http://127.0.0.1:{port}/")).unwrap();
    assert_eq!(
        http::post(&Post { url: &u, ..p }, &HttpLimits::default()),
        Ok(202)
    );
}

#[test]
fn wp_secout_the_name_lookup_is_inside_the_deadline() {
    // Whatever the resolver does with this name (an answer, a refusal, or a hang until its
    // own timeout), the lookup gives up at the deadline.
    let started = Instant::now();
    let deadline = started + Duration::from_millis(50);
    let r = http::resolve("wp-secout-deadline.invalid", 443, deadline);
    assert!(r.is_err(), "{r:?}");
    assert!(
        started.elapsed() < Duration::from_millis(1500),
        "{:?}",
        started.elapsed()
    );
    // An address needs no lookup at all.
    let past = Instant::now();
    assert_eq!(
        http::resolve("10.0.0.1", 80, past).unwrap(),
        vec!["10.0.0.1:80".parse().unwrap()]
    );
}

// ── Targets ─────────────────────────────────────────────────────────────────

fn entry(v: Value) -> Vec<Value> {
    vec![v]
}

#[test]
fn wp_secout_set_rules_for_https_http_and_the_trusts() {
    let ca = pem("CERTIFICATE", CA);
    let ok = |v: Value| plan(&entry(v), &[]).unwrap().0.remove(0).0;
    let refused = |v: Value| plan(&entry(v), &[]).err().unwrap().1;

    let t = ok(json!({"kind": "webhook", "url": "https://hooks.example/x"}));
    assert!(!t.plaintext && t.ca_pem.is_none() && t.pin_sha256.is_none());
    let t = ok(json!({"kind": "webhook", "url": "https://10.0.0.2/x", "ca_pem": ca}));
    assert_eq!(t.ca_pem.as_deref(), Some(ca.as_str()));
    // A pin in any form is kept in curl's.
    let hex = HEXLOWER.encode(&tls::spki_sha256(LEAF).unwrap());
    let t = ok(json!({"kind": "webhook", "url": "https://10.0.0.2/x", "pin_sha256": hex}));
    assert_eq!(t.pin_sha256.as_deref(), Some(LEAF_PIN));
    let t = ok(json!({"kind": "push", "url": "http://192.168.1.9/x", "plaintext": true}));
    assert!(t.plaintext);
    // A name on http:// is taken; where it points is checked on every send.
    ok(json!({"kind": "push", "url": "http://ntfy.lan/x", "plaintext": true}));

    for (bad, says) in [
        (
            json!({"kind": "webhook", "url": "http://10.0.0.2/x"}),
            "plaintext",
        ),
        (
            json!({"kind": "webhook", "url": "http://8.8.8.8/x", "plaintext": true}),
            "unencrypted",
        ),
        (
            json!({"kind": "webhook", "url": "http://[2001:db8::1]/x", "plaintext": true}),
            "unencrypted",
        ),
        (
            json!({"kind": "webhook", "url": "https://h/x", "plaintext": true}),
            "http:// URLs only",
        ),
        (
            json!({"kind": "webhook", "url": "http://10.0.0.2/x", "plaintext": true, "ca_pem": ca}),
            "https:// URLs only",
        ),
        (
            json!({"kind": "webhook", "url": "https://h/x", "ca_pem": ca, "pin_sha256": LEAF_PIN}),
            "give one",
        ),
        (
            json!({"kind": "webhook", "url": "https://h/x", "ca_pem": "nope"}),
            "ca_pem",
        ),
        (
            json!({"kind": "webhook", "url": "https://h/x", "pin_sha256": "nope"}),
            "pin_sha256",
        ),
        (
            json!({"kind": "webhook", "url": "https://h/x", "plaintext": "yes"}),
            "not a bool",
        ),
        (json!({"kind": "webhook", "url": "ftp://h/x"}), "https://"),
    ] {
        let why = refused(bad.clone());
        assert!(why.contains(says), "{bad}: {why}");
    }
}

fn target(id: &str, url: &str) -> (Target, HttpUrl) {
    let t = Target {
        id: id.into(),
        name: id.into(),
        kind: TargetKind::Webhook,
        url: url.into(),
        enabled: true,
        kinds: KINDS.iter().map(|k| (*k).to_owned()).collect(),
        secret: None,
        plaintext: url.starts_with("http://"),
        ca_pem: None,
        pin_sha256: None,
    };
    let u = http::parse_url(url).unwrap();
    (t, u)
}

/// A notice every target here wants.
fn notice(n: u64) -> Notice {
    let mut notice = Notice::test(1_800_000_000 + n);
    notice.kinds = vec!["alarm.raised"];
    notice
}

fn take_ready(o: &mut Outbox, epoch: u64, now: Instant) -> Job {
    match o.take(epoch, now) {
        Take::Ready(job) => job,
        other => panic!("not ready: {other:?}"),
    }
}

fn epoch_of(o: &Outbox, id: &str) -> u64 {
    o.slots.iter().find(|s| s.target.id == id).unwrap().epoch
}

#[test]
fn wp_secout_a_kind_change_drops_bodies_made_for_the_other_kind() {
    let mut o = Outbox::default();
    o.set_targets(vec![target("a", "https://h/a")]);
    let e = epoch_of(&o, "a");
    for n in 0..3 {
        o.enqueue(&notice(n), "h");
    }
    let now = Instant::now();
    let in_flight = take_ready(&mut o, e, now);
    assert!(in_flight.delivery.body.starts_with('{'), "a webhook's JSON");
    // Now a push target: no JSON body made for the webhook goes to it as text.
    let (mut push, u) = target("a", "https://h/a");
    push.kind = TargetKind::Push;
    o.set_targets(vec![(push, u)]);
    assert!(o.queued("a").is_empty());
    assert_eq!(
        o.counts("a"),
        Some((1, 2)),
        "the in-flight one, and two dropped"
    );
    // The one in flight fails: it is not queued again as the new kind.
    o.finish(in_flight, &Outcome::Failed("x".into()), now, 1);
    assert_eq!(o.counts("a"), Some((0, 3)));
    // New ones are made for the new kind.
    o.enqueue(&notice(9), "h");
    assert_eq!(&*o.queued("a")[0].body, "A test from the DSPi hub");
}

#[test]
fn wp_secout_a_corrected_url_is_tried_at_once() {
    let mut o = Outbox::default();
    o.set_targets(vec![target("a", "https://wrong.example/a")]);
    let e = epoch_of(&o, "a");
    o.enqueue(&notice(0), "h");
    let now = Instant::now();
    for _ in 0..4 {
        let job = take_ready(&mut o, e, now + Duration::from_secs(3600));
        o.finish(job, &Outcome::Failed("x".into()), now, 1);
    }
    assert!(matches!(o.take(e, now), Take::Wait(Some(_))), "backed off");
    // The same URL again (a rename): still backed off.
    let (mut same, u) = target("a", "https://wrong.example/a");
    same.name = "renamed".into();
    o.set_targets(vec![(same, u)]);
    assert!(matches!(o.take(e, now), Take::Wait(Some(_))));
    // A corrected URL: due now, and the next failure waits the first backoff again.
    o.set_targets(vec![target("a", "https://right.example/a")]);
    let job = take_ready(&mut o, e, now);
    assert_eq!(job.url.host, "right.example");
    o.finish(job, &Outcome::Failed("x".into()), now, 1);
    match o.take(e, now) {
        Take::Wait(Some(due)) => assert_eq!(due, now + BACKOFF_FIRST),
        other => panic!("{other:?}"),
    }
    // A changed trust counts as a corrected destination too.
    let (mut pinned, u) = target("a", "https://right.example/a");
    pinned.pin_sha256 = Some(LEAF_PIN.into());
    o.set_targets(vec![(pinned, u)]);
    assert!(matches!(o.take(e, now), Take::Ready(_)));
}

#[test]
fn wp_secout_targets_written_before_tls_are_kept_as_plaintext_with_a_notice() {
    let dir = scratch("migrate");
    fs::create_dir_all(&dir).unwrap();
    // As P2WH wrote it: http:// with no plaintext field, one on the LAN, one public.
    let old = json!({"v": 1, "targets": [
        {"id": "lan", "name": "HA", "kind": "webhook", "url": "http://192.168.1.20:8123/hook",
         "enabled": true, "kinds": ["alarm.raised"]},
        {"id": "cloud", "name": "ntfy", "kind": "push", "url": "http://203.0.113.9/topic",
         "enabled": true, "kinds": ["alarm.raised"]},
        {"id": "tls", "name": "ok", "kind": "push", "url": "https://ntfy.sh/topic",
         "enabled": true, "kinds": ["alarm.raised"]}]});
    fs::write(dir.join(FILE_NAME), old.to_string()).unwrap();
    let (loaded, migrated) = load_migrating(&dir, 1);
    assert_eq!(loaded.len(), 3, "nothing set aside");
    assert_eq!(migrated, vec!["lan".to_owned(), "cloud".to_owned()]);
    assert!(loaded[0].0.plaintext && loaded[1].0.plaintext && !loaded[2].0.plaintext);
    assert!(!dir.join(format!("{FILE_NAME}.bad-1")).exists());
    // The file now carries the flag: the next start migrates nothing.
    let text: Value = serde_json::from_slice(&fs::read(dir.join(FILE_NAME)).unwrap()).unwrap();
    assert_eq!(text["targets"][0]["plaintext"], true);
    let (again, migrated) = load_migrating(&dir, 2);
    assert_eq!(again.len(), 3);
    assert!(migrated.is_empty(), "once");

    // Through the service: the notice is shown until the next outbound.set.
    fs::write(dir.join(FILE_NAME), old.to_string()).unwrap();
    let clock: Arc<dyn HubClock> = Arc::new(TestClock::new());
    let hub = crate::Hub::start(crate::HubConfig {
        data_dir: Some(dir.clone()),
        services: ServiceConfig {
            clock,
            ..ServiceConfig::default()
        },
        ..crate::HubConfig::in_process("SECOUT migrate")
    })
    .unwrap();
    let h = hub.handle();
    let admin = h.open_session(Role::Admin, None).unwrap();
    let ask = |msg: Value| -> Value {
        serde_json::from_str(&h.service_message(&admin, &msg).unwrap()).unwrap()
    };
    let get = ask(json!({"t": "outbound.get", "id": 1}));
    let targets = get["targets"].as_array().unwrap().clone();
    assert_eq!(targets.len(), 3, "{get}");
    assert_eq!(targets[0]["notice"], MIGRATED_NOTICE);
    assert_eq!(targets[0]["encrypted"], false);
    assert_eq!(targets[0]["plaintext"], true);
    assert!(targets[2].get("notice").is_none());
    assert_eq!(targets[2]["encrypted"], true);
    assert_eq!(targets[2]["trust"], "webpki");
    // The public one is kept, and refused on its send: nothing crosses the internet in clear.
    let test = ask(json!({"t": "outbound.test", "id": 2, "target": "cloud"}));
    assert_eq!(test["delivered"], false, "{test}");
    assert!(
        test["msg"].as_str().unwrap().contains("unencrypted"),
        "{test}"
    );
    // outbound.get's list sent back as it is: kept, notice gone.
    let set = ask(
        json!({"t": "outbound.set", "id": 3, "targets": [targets[0].clone(), targets[2].clone()]}),
    );
    assert_eq!(set["t"], "ok", "{set}");
    assert!(set["targets"][0].get("notice").is_none(), "{set}");
    assert_eq!(set["targets"][0]["plaintext"], true);
    hub.stop();
}

// ── Through the service: https with a pin, end to end ───────────────────────

#[test]
fn wp_secout_outbound_test_delivers_over_https_with_a_pin_signed() {
    let (port, rx) = tls_receiver(b"HTTP/1.1 200 OK\r\n\r\n", false);
    let clock: Arc<dyn HubClock> = Arc::new(TestClock::new());
    let hub = crate::Hub::start(crate::HubConfig {
        services: ServiceConfig {
            clock,
            ..ServiceConfig::default()
        },
        ..crate::HubConfig::in_process("SECOUT https")
    })
    .unwrap();
    let h = hub.handle();
    let admin = h.open_session(Role::Admin, None).unwrap();
    let ask = |msg: Value| -> Value {
        serde_json::from_str(&h.service_message(&admin, &msg).unwrap()).unwrap()
    };
    let set = ask(json!({"t": "outbound.set", "id": 1, "targets": [
        {"id": "pinned", "kind": "webhook", "url": format!("https://127.0.0.1:{port}/hook"),
         "pin_sha256": LEAF_PIN, "secret": "new"},
        {"id": "ca", "kind": "push", "url": format!("https://localhost:{port}/topic"),
         "ca_pem": pem("CERTIFICATE", CA)}]}));
    assert_eq!(set["t"], "ok", "{set}");
    assert_eq!(set["targets"][0]["trust"], "pin");
    assert_eq!(set["targets"][0]["pin_sha256"], LEAF_PIN);
    assert_eq!(set["targets"][1]["trust"], "ca");
    let secret = set["secrets"]["pinned"].as_str().unwrap().to_owned();
    let test = ask(json!({"t": "outbound.test", "id": 2, "target": "pinned"}));
    assert_eq!(test["delivered"], true, "{test}");
    let Got::Request(req) = got(&rx) else {
        panic!("refused")
    };
    let req = String::from_utf8(req).unwrap();
    let ts = req
        .lines()
        .find_map(|l| l.strip_prefix("X-DSPi-Timestamp: "))
        .unwrap()
        .parse::<u64>()
        .unwrap();
    let body = req.split("\r\n\r\n").nth(1).unwrap();
    // The HMAC is kept over TLS: it authenticates the hub to the receiver.
    assert!(
        req.contains(&format!(
            "X-DSPi-Signature: {}",
            hmac::signature(&secret, ts, body.as_bytes())
        )),
        "{req}"
    );
    let test = ask(json!({"t": "outbound.test", "id": 3, "target": "ca"}));
    assert_eq!(test["delivered"], true, "{test}");
    assert!(matches!(got(&rx), Got::Request(_)));
    hub.stop();
}
