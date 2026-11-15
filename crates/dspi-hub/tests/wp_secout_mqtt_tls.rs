//! WP SECOUT: the hub's MQTT client over TLS (PLAN-SEC 4.8), against a fake broker wrapped
//! in TLS on the loopback, with the TM12 test certificates (`crates/dspi-tui/testdata/tm12/`:
//! a CA, and a leaf for `DNS:localhost` it signed, also used here as a client certificate).
//!
//! CONNECT over TLS with a CA, a pin, and a client certificate (inline and from 0600 files);
//! a wrong CA and a wrong pin refused before a byte of CONNECT; without TLS the password goes
//! to a loopback broker only, unless `allow_plaintext_password`; a `mqtt.json` written before
//! TLS migrated to the flag with a notice, once; the password and the client key in no
//! `Debug`, event, status or log line.
#![allow(
    clippy::arithmetic_side_effects,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing
)]

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

use data_encoding::BASE64;
use dspi_hub::mqtt::client::{MQTT_MIGRATED_NOTICE, MqttTls, MqttTlsMode};
use dspi_hub::mqtt::codec::{self, Packet};
use dspi_hub::mqtt::{MqttClient, MqttConfig, MqttEvent};
use rustls::client::danger::HandshakeSignatureValid;
use rustls::crypto::WebPkiSupportedAlgorithms;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer, UnixTime};
use rustls::server::danger::{ClientCertVerified, ClientCertVerifier};
use rustls::{
    DigitallySignedStruct, DistinguishedName, ServerConfig, ServerConnection, SignatureScheme,
    StreamOwned,
};
use serde_json::json;

const CA: &[u8] = include_bytes!("../../dspi-tui/testdata/tm12/ca.der");
const LEAF: &[u8] = include_bytes!("../../dspi-tui/testdata/tm12/leaf.der");
const LEAF_KEY: &[u8] = include_bytes!("../../dspi-tui/testdata/tm12/leaf-key.der");
const LEAF_PIN: &str = "sha256//QIoTMalqjAmU7cqIfPIw0djIOMn8E2uRqx/dfg0/EiE=";
const CA_PIN: &str = "sha256//TvRMfCrv5h+dn2AsWNZpHWAJfVdXDQVjdGLYOIsvft8=";
const PASSWORD: &str = "wp-secout-hunter2";
const WAIT: Duration = Duration::from_secs(20);

// ── Every log line this process writes, to look for secrets in ──────────────

struct Capture;

static LINES: Mutex<Vec<String>> = Mutex::new(Vec::new());

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

fn capture_logs() {
    static ONCE: OnceLock<()> = OnceLock::new();
    ONCE.get_or_init(|| {
        let _ = log::set_logger(&Capture);
        log::set_max_level(log::LevelFilter::Trace);
    });
}

fn assert_no_secret_logged() {
    let key_b64 = BASE64.encode(LEAF_KEY);
    for line in LINES.lock().unwrap().iter() {
        assert!(!line.contains(PASSWORD), "a password in a log line: {line}");
        assert!(
            !line.contains(&key_b64[8..40]),
            "a key in a log line: {line}"
        );
    }
}

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
    let dir = std::env::temp_dir().join("wp-secout").join(format!(
        "{what}-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::SeqCst)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

// ── A fake broker in TLS ────────────────────────────────────────────────────

/// Any client certificate, its signature checked: the test looks at which one came.
#[derive(Debug)]
struct AnyClient(WebPkiSupportedAlgorithms);

impl ClientCertVerifier for AnyClient {
    fn root_hint_subjects(&self) -> &[DistinguishedName] {
        &[]
    }
    fn verify_client_cert(
        &self,
        _: &CertificateDer<'_>,
        _: &[CertificateDer<'_>],
        _: UnixTime,
    ) -> Result<ClientCertVerified, rustls::Error> {
        Ok(ClientCertVerified::assertion())
    }
    fn verify_tls12_signature(
        &self,
        m: &[u8],
        c: &CertificateDer<'_>,
        d: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(m, c, d, &self.0)
    }
    fn verify_tls13_signature(
        &self,
        m: &[u8],
        c: &CertificateDer<'_>,
        d: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(m, c, d, &self.0)
    }
    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.0.supported_schemes()
    }
}

/// What the broker saw of one connection.
#[derive(Debug, Clone)]
enum Seen {
    /// The handshake failed; application bytes that had come (none, ever).
    Refused(usize),
    /// CONNECT, and the client certificate that came with it.
    Connect {
        username: Option<String>,
        password: Option<Vec<u8>>,
        client_cert: Option<Vec<u8>>,
    },
}

#[derive(Clone)]
struct TlsBroker {
    addr: SocketAddr,
    seen: Arc<Mutex<Vec<Seen>>>,
}

impl TlsBroker {
    fn start(want_client_cert: bool) -> TlsBroker {
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let builder = ServerConfig::builder_with_provider(Arc::clone(&provider))
            .with_safe_default_protocol_versions()
            .unwrap();
        let builder = if want_client_cert {
            builder.with_client_cert_verifier(Arc::new(AnyClient(
                provider.signature_verification_algorithms,
            )))
        } else {
            builder.with_no_client_auth()
        };
        let config = Arc::new(
            builder
                .with_single_cert(
                    vec![CertificateDer::from(LEAF.to_vec())],
                    PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(LEAF_KEY.to_vec())),
                )
                .unwrap(),
        );
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let broker = TlsBroker {
            addr: listener.local_addr().unwrap(),
            seen: Arc::new(Mutex::new(Vec::new())),
        };
        let seen = Arc::clone(&broker.seen);
        thread::spawn(move || {
            for s in listener.incoming() {
                let Ok(mut s) = s else { return };
                let config = Arc::clone(&config);
                let seen = Arc::clone(&seen);
                thread::spawn(move || {
                    s.set_read_timeout(Some(WAIT)).unwrap();
                    let mut conn = ServerConnection::new(config).unwrap();
                    while conn.is_handshaking() {
                        if conn.complete_io(&mut s).is_err() {
                            let mut plain = Vec::new();
                            let _ = conn.reader().read_to_end(&mut plain);
                            seen.lock().unwrap().push(Seen::Refused(plain.len()));
                            return;
                        }
                    }
                    let client_cert = conn
                        .peer_certificates()
                        .and_then(|c| c.first())
                        .map(|c| c.as_ref().to_vec());
                    let mut tls = StreamOwned::new(conn, s);
                    let mut buf = Vec::new();
                    let mut chunk = [0u8; 4096];
                    loop {
                        while let Ok(Some((p, used))) = codec::decode(&buf, 1 << 20) {
                            buf.drain(..used);
                            match p {
                                Packet::Connect(c) => {
                                    seen.lock().unwrap().push(Seen::Connect {
                                        username: c.username,
                                        password: c.password,
                                        client_cert: client_cert.clone(),
                                    });
                                    let ack = codec::encode(&Packet::Connack {
                                        session_present: false,
                                        return_code: 0,
                                    })
                                    .unwrap();
                                    let _ = tls.write_all(&ack);
                                    let _ = tls.flush();
                                }
                                Packet::Pingreq => {
                                    let _ =
                                        tls.write_all(&codec::encode(&Packet::Pingresp).unwrap());
                                    let _ = tls.flush();
                                }
                                Packet::Disconnect => return,
                                _ => {}
                            }
                        }
                        match tls.read(&mut chunk) {
                            Ok(0) | Err(_) => return,
                            Ok(n) => buf.extend_from_slice(&chunk[..n]),
                        }
                    }
                });
            }
        });
        broker
    }

    fn seen(&self) -> Vec<Seen> {
        self.seen.lock().unwrap().clone()
    }
}

fn wait_for(what: &str, mut f: impl FnMut() -> bool) {
    let until = Instant::now() + WAIT;
    while !f() {
        assert!(Instant::now() < until, "timed out waiting for {what}");
        thread::sleep(Duration::from_millis(10));
    }
}

type Events = Arc<Mutex<Vec<MqttEvent>>>;

fn start(c: MqttConfig) -> (MqttClient, Events) {
    capture_logs();
    let events: Events = Arc::new(Mutex::new(Vec::new()));
    let e = Arc::clone(&events);
    let client = MqttClient::start(c, move |ev| e.lock().unwrap().push(ev)).unwrap();
    (client, events)
}

fn config(host: &str, port: u16, tls: MqttTls) -> MqttConfig {
    let mut c = MqttConfig::new(host, port, "hub-secout");
    c.username = Some("hub".into());
    c.password = Some(PASSWORD.into());
    c.backoff_min = Duration::from_millis(50);
    c.backoff_max = Duration::from_millis(200);
    c.tls = tls;
    c
}

fn on(f: impl FnOnce(&mut MqttTls)) -> MqttTls {
    let mut t = MqttTls {
        mode: MqttTlsMode::On,
        ..MqttTls::default()
    };
    f(&mut t);
    t
}

fn connected(events: &Events) -> bool {
    events
        .lock()
        .unwrap()
        .iter()
        .any(|e| matches!(e, MqttEvent::Connected { .. }))
}

fn reasons(events: &Events) -> Vec<String> {
    events
        .lock()
        .unwrap()
        .iter()
        .filter_map(|e| match e {
            MqttEvent::Disconnected { reason } => Some(reason.clone()),
            _ => None,
        })
        .collect()
}

fn assert_connect_with_password(seen: &[Seen], want_cert: Option<&[u8]>) {
    let Some(Seen::Connect {
        username,
        password,
        client_cert,
    }) = seen.first()
    else {
        panic!("{seen:?}")
    };
    assert_eq!(username.as_deref(), Some("hub"));
    assert_eq!(password.as_deref(), Some(PASSWORD.as_bytes()));
    assert_eq!(client_cert.as_deref(), want_cert);
}

#[test]
fn wp_secout_mqtt_connect_over_tls_with_a_ca() {
    let broker = TlsBroker::start(false);
    let tls = on(|t| t.ca_pem = Some(pem("CERTIFICATE", CA)));
    assert_eq!(tls.default_port(), 8883);
    let (mut client, events) = start(config("localhost", broker.addr.port(), tls));
    wait_for("CONNACK over TLS", || connected(&events));
    let status = client.status();
    assert!(status.connected && status.encrypted, "{status:?}");
    assert_connect_with_password(&broker.seen(), None);
    client.stop();
    assert!(!client.status().encrypted);
    assert_no_secret_logged();
}

#[test]
fn wp_secout_mqtt_connect_over_tls_with_a_pin() {
    let broker = TlsBroker::start(false);
    // The address is not in the certificate: the pin replaces the name check.
    let tls = on(|t| t.pin_sha256 = Some(LEAF_PIN.into()));
    let (mut client, events) = start(config("127.0.0.1", broker.addr.port(), tls));
    wait_for("CONNACK over TLS", || connected(&events));
    assert!(client.status().encrypted);
    assert_connect_with_password(&broker.seen(), None);
    client.stop();
}

#[test]
fn wp_secout_mqtt_connect_with_a_client_certificate_inline_and_from_files() {
    let broker = TlsBroker::start(true);
    let tls = on(|t| {
        t.pin_sha256 = Some(LEAF_PIN.into());
        t.client_cert_pem = Some(pem("CERTIFICATE", LEAF));
        t.client_key_pem = Some(pem("PRIVATE KEY", LEAF_KEY));
    });
    let shown = format!("{:?}", config("127.0.0.1", 1, tls.clone()));
    assert!(!shown.contains(PASSWORD), "{shown}");
    assert!(!shown.contains(&BASE64.encode(LEAF_KEY)[8..40]), "{shown}");
    let view = tls.view_json().to_string();
    assert!(!view.contains(&BASE64.encode(LEAF_KEY)[8..40]), "{view}");
    assert_eq!(tls.view_json()["client_key_pem"], "set");
    let (mut client, events) = start(config("127.0.0.1", broker.addr.port(), tls));
    wait_for("CONNACK with a client certificate", || connected(&events));
    assert_connect_with_password(&broker.seen(), Some(LEAF));
    client.stop();

    // From PEM files; the key's must be 0600.
    let dir = scratch("client-files");
    let (cert_file, key_file) = (dir.join("client.pem"), dir.join("client.key"));
    std::fs::write(&cert_file, pem("CERTIFICATE", LEAF)).unwrap();
    std::fs::write(&key_file, pem("PRIVATE KEY", LEAF_KEY)).unwrap();
    let files = on(|t| {
        t.pin_sha256 = Some(LEAF_PIN.into());
        t.client_cert_file = Some(cert_file.clone());
        t.client_key_file = Some(key_file.clone());
    });
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&key_file, std::fs::Permissions::from_mode(0o644)).unwrap();
        let refused = MqttClient::start(
            config("127.0.0.1", broker.addr.port(), files.clone()),
            |_| {},
        );
        let err = refused.err().unwrap().to_string();
        assert!(err.contains("0600"), "{err}");
        std::fs::set_permissions(&key_file, std::fs::Permissions::from_mode(0o600)).unwrap();
    }
    let (mut client, events) = start(config("127.0.0.1", broker.addr.port(), files));
    wait_for("CONNACK with a client certificate from files", || {
        connected(&events)
    });
    assert_eq!(broker.seen().len(), 2);
    let Seen::Connect { client_cert, .. } = &broker.seen()[1] else {
        panic!()
    };
    assert_eq!(client_cert.as_deref(), Some(LEAF));
    client.stop();
    assert_no_secret_logged();
}

#[test]
fn wp_secout_mqtt_a_wrong_ca_or_pin_is_refused_and_no_connect_is_sent() {
    for (host, tls, says) in [
        // The test CA is not among the public ones.
        ("localhost", on(|_| {}), "trusted CA"),
        // The leaf is not the issuer of itself.
        (
            "localhost",
            on(|t| t.ca_pem = Some(pem("CERTIFICATE", LEAF))),
            "refused",
        ),
        // The right CA, the wrong name.
        (
            "127.0.0.1",
            on(|t| t.ca_pem = Some(pem("CERTIFICATE", CA))),
            "not for 127.0.0.1",
        ),
        // Another key's pin.
        (
            "127.0.0.1",
            on(|t| t.pin_sha256 = Some(CA_PIN.into())),
            "pinned key",
        ),
    ] {
        let broker = TlsBroker::start(false);
        let (mut client, events) = start(config(host, broker.addr.port(), tls));
        wait_for("a refused handshake", || !reasons(&events).is_empty());
        client.stop();
        let why = reasons(&events).remove(0);
        assert!(why.contains(says), "{host} {says}: {why}");
        assert!(!why.contains(PASSWORD));
        assert!(!connected(&events));
        let seen = broker.seen();
        assert!(!seen.is_empty(), "{seen:?}");
        for s in &seen {
            assert!(matches!(s, Seen::Refused(0)), "no CONNECT bytes: {seen:?}");
        }
        let status = client.status();
        assert!(!status.connected && !status.encrypted);
        assert!(!status.last_error.unwrap_or_default().contains(PASSWORD));
    }
    assert_no_secret_logged();
}

#[test]
fn wp_secout_mqtt_without_tls_the_password_goes_to_loopback_only() {
    // A broker that is not on loopback, no TLS, no flag: nothing is sent, not even a SYN.
    let started = Instant::now();
    let (mut client, events) = start(config("192.0.2.1", 1883, MqttTls::default()));
    wait_for("the refusal", || !reasons(&events).is_empty());
    client.stop();
    let why = reasons(&events).remove(0);
    assert!(
        why.contains("password unencrypted") && why.contains("192.0.2.1"),
        "{why}"
    );
    assert!(!why.contains(PASSWORD));
    assert!(
        started.elapsed() < Duration::from_secs(3),
        "no connect tried"
    );

    // A loopback broker gets it in clear, with no flag needed.
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port();
    let got = Arc::new(Mutex::new(None));
    let g = Arc::clone(&got);
    thread::spawn(move || {
        let (mut s, _) = l.accept().unwrap();
        let mut buf = Vec::new();
        let mut chunk = [0u8; 1024];
        loop {
            if let Ok(Some((Packet::Connect(c), _))) = codec::decode(&buf, 1 << 16) {
                *g.lock().unwrap() = c.password;
                let _ = s.write_all(
                    &codec::encode(&Packet::Connack {
                        session_present: false,
                        return_code: 0,
                    })
                    .unwrap(),
                );
                thread::sleep(Duration::from_millis(500));
                return;
            }
            match s.read(&mut chunk) {
                Ok(0) | Err(_) => return,
                Ok(n) => buf.extend_from_slice(&chunk[..n]),
            }
        }
    });
    let (mut client, events) = start(config("127.0.0.1", port, MqttTls::default()));
    wait_for("CONNACK in clear on loopback", || connected(&events));
    assert!(!client.status().encrypted);
    assert_eq!(got.lock().unwrap().as_deref(), Some(PASSWORD.as_bytes()));
    client.stop();
    assert_no_secret_logged();
}

#[test]
fn wp_secout_mqtt_settings_written_before_tls_migrate_to_the_flag_once() {
    // No `tls` in mqtt.json: a LAN broker keeps its password going, with the notice.
    let (lan, notice) = MqttTls::load(None, "192.168.1.10").unwrap();
    assert_eq!(notice, Some(MQTT_MIGRATED_NOTICE));
    assert!(lan.allow_plaintext_password && lan.mode == MqttTlsMode::Off);
    let (null, notice) = MqttTls::load(Some(&serde_json::Value::Null), "broker.lan").unwrap();
    assert!(null.allow_plaintext_password && notice.is_some());
    // What the caller writes back reads the same, and says nothing again.
    let written = lan.to_json();
    assert_eq!(written["allow_plaintext_password"], true);
    let (again, notice) = MqttTls::load(Some(&written), "192.168.1.10").unwrap();
    assert_eq!(again, lan);
    assert_eq!(notice, None, "once");
    // A loopback broker needs no flag and gets no notice.
    for host in ["localhost", "127.0.0.1", "::1", "[::1]"] {
        let (local, notice) = MqttTls::load(None, host).unwrap();
        assert!(
            !local.allow_plaintext_password && notice.is_none(),
            "{host}"
        );
    }
    // The migrated settings really send the password to that broker in clear.
    let mut c = config("192.0.2.1", 9, lan);
    c.connect_timeout = Duration::from_millis(200);
    let (mut client, events) = start(c);
    wait_for("a connect attempt", || !reasons(&events).is_empty());
    client.stop();
    let why = reasons(&events).remove(0);
    assert!(
        !why.contains("password unencrypted"),
        "the flag is honoured: {why}"
    );
}

#[test]
fn wp_secout_mqtt_tls_settings_parse_strictly_and_keep_the_key_on_update() {
    let ca = pem("CERTIFICATE", CA);
    let key = pem("PRIVATE KEY", LEAF_KEY);
    let full = json!({"mode": "on", "pin_sha256": LEAF_PIN,
        "client_cert_pem": pem("CERTIFICATE", LEAF), "client_key_pem": key});
    let (t, _) = MqttTls::load(Some(&full), "h").unwrap();
    assert_eq!(t.client_key_pem.as_deref(), Some(key.as_str()));
    // mqtt.get's view sent back: the key is kept.
    let kept = MqttTls::update(&t.view_json(), &t).unwrap();
    assert_eq!(kept, t);
    let cleared = MqttTls::update(&json!({"mode": "on", "pin_sha256": LEAF_PIN}), &t).unwrap();
    assert_eq!(cleared.client_key_pem, None);
    for (bad, says) in [
        (json!("on"), "not an object"),
        (json!({"mode": "maybe"}), "off or on"),
        (json!({"mode": "off", "ca_pem": ca}), "need mode on"),
        (
            json!({"mode": "on", "ca_pem": ca, "pin_sha256": LEAF_PIN}),
            "give one",
        ),
        (json!({"mode": "on", "ca_pem": "nope"}), "ca_pem"),
        (json!({"mode": "on", "pin_sha256": "nope"}), "pin_sha256"),
        (
            json!({"mode": "on", "client_cert_pem": pem("CERTIFICATE", LEAF)}),
            "needs its key",
        ),
        (
            json!({"mode": "on", "allow_plaintext_password": true}),
            "mode off only",
        ),
        (
            json!({"mode": "off", "allow_plaintext_password": "yes"}),
            "not a bool",
        ),
    ] {
        let err = MqttTls::load(Some(&bad), "h").unwrap_err().to_string();
        assert!(err.contains(says), "{bad}: {err}");
        assert!(!err.contains(&BASE64.encode(LEAF_KEY)[8..40]));
    }
    // A bad PEM in the config is refused at start, before any thread.
    let mut c = MqttConfig::new("127.0.0.1", 1, "hub");
    c.tls = on(|t| t.ca_pem = Some("nope".into()));
    assert!(MqttClient::start(c, |_| {}).is_err());
}
