//! WP P2SW tests: the Mac hub's start options (the optional services, TLS and the
//! upstream compatibility mode) over real sockets on loopback, on the demo device. No
//! USB.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use super::*;

/// A scratch directory: `temp_dir` is under the build cache in the build container.
fn scratch(what: &str) -> String {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let dir = std::env::temp_dir().join("wp-p2sw-unit").join(format!(
        "{what}-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::SeqCst)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    dir.to_string_lossy().into_owned()
}

/// The defaults, on loopback and any free port (the only two changes a test makes).
fn defaults(dir: &str) -> HubStartOptions {
    let mut options = hub_default_start_options(dir.to_owned());
    options.loopback = true;
    options.port = 0;
    options
}

/// The status line of a plaintext `GET path` on loopback.
fn plaintext_get(port: u16, path: &str) -> String {
    let mut tcp = TcpStream::connect(("127.0.0.1", port)).unwrap();
    tcp.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    write!(
        tcp,
        "GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"
    )
    .unwrap();
    let mut answer = Vec::new();
    let _ = tcp.read_to_end(&mut answer);
    let text = String::from_utf8_lossy(&answer).into_owned();
    text.lines().next().unwrap_or_default().to_owned()
}

/// Every cap an optional service adds to `hello` (the switches of `service.rs`).
/// `param_api` is not a switch: it comes with feature `api`, which the Mac build does
/// not carry.
const OPTIONAL_CAPS: &[&str] = &[
    crate::health::CAP,
    crate::inventory::CAP,
    crate::identify::CAP,
    crate::history::CAP,
    crate::device_alarms::CAP,
    crate::groups::CAP,
    crate::schedule::CAP,
];

#[test]
fn wp_p2sw_default_options() {
    let o = hub_default_start_options("/somewhere".to_owned());
    assert_eq!(o.store_dir, "/somewhere");
    assert_eq!(o.name, "");
    assert_eq!(o.port, crate::DEFAULT_PORT);
    assert_eq!(o.access, HubAccess::Pin);
    assert_eq!(
        o.tls,
        HubTls::Require,
        "TLS is required by default (OD-SECb, SECDEF)"
    );
    assert!(
        !o.upstream_compat,
        "the compatibility mode is off by default"
    );
    assert_eq!(o.services, HubServices::default());
    let s = o.services;
    assert!(
        !(s.health || s.inventory || s.identify || s.history || s.alarms || s.groups),
        "every optional service is off by default (review S8)"
    );
    assert!(!o.usb && !o.app && !o.loopback && !o.web);
    assert_eq!(o.mdns_name, None);
}

#[test]
fn wp_p2sw_a_default_hub_runs_no_optional_service_and_requires_tls() {
    let dir = scratch("default");
    let host = HubHost::new();
    let started = host.start(defaults(&dir)).unwrap();
    host.attach_demo().unwrap();

    let security = started.security.clone();
    assert_eq!(security.tls, HubTls::Require);
    assert!(!security.upstream_compat);
    assert!(security.advertises_tls, "tls=1 while TLS is served");
    let fingerprint = security.fingerprint.clone().unwrap();
    assert!(fingerprint.starts_with("SHA256:"), "{fingerprint}");
    for cap in OPTIONAL_CAPS {
        assert!(
            !security.services.iter().any(|c| c == cap),
            "a default hub runs no optional service, yet `{cap}` answers: {:?}",
            security.services
        );
    }
    // The status says the same while it runs.
    assert_eq!(host.status().security, security);

    // TLS required: a plaintext request is told so (426), the web pages included.
    let line = plaintext_get(started.port, "/dspi/v1/info");
    assert!(line.contains(" 426"), "plaintext answered: {line}");
    let line = plaintext_get(started.port, "/");
    assert!(line.contains(" 426"), "plaintext page answered: {line}");
    // The key is the hub's own, in its store.
    assert!(Path::new(&dir).join(crate::tls_identity::TLS_DIR).is_dir());

    // The same key on the next start: the fingerprint clients pinned holds.
    assert!(host.stop());
    assert_eq!(host.status().security, HubSecurity::none());
    let again = host.start(defaults(&dir)).unwrap();
    assert_eq!(again.security.fingerprint, Some(fingerprint));
    assert!(host.stop());
}

#[test]
fn wp_p2sw_services_run_when_asked() {
    let dir = scratch("services");
    let host = HubHost::new();
    let mut options = defaults(&dir);
    options.services = HubServices {
        health: true,
        inventory: true,
        identify: true,
        history: true,
        alarms: true,
        groups: true,
    };
    let started = host.start(options).unwrap();
    let caps = started.security.services;
    for cap in [
        crate::health::CAP,
        crate::inventory::CAP,
        crate::identify::CAP,
        crate::history::CAP,
        crate::device_alarms::CAP,
        crate::groups::CAP,
    ] {
        assert!(caps.iter().any(|c| c == cap), "{cap} missing from {caps:?}");
    }
    // Only what was asked: one on, the others off.
    assert!(host.stop());
    let mut options = defaults(&dir);
    options.services.history = true;
    let caps = host.start(options).unwrap().security.services;
    assert!(caps.iter().any(|c| c == crate::history::CAP));
    assert!(!caps.iter().any(|c| c == crate::health::CAP));
    assert!(!caps.iter().any(|c| c == crate::device_alarms::CAP));
    assert!(host.stop());
}

#[test]
fn wp_p2sw_upstream_compat_serves_both_and_is_checked() {
    let dir = scratch("compat");
    let host = HubHost::new();
    for tls in [HubTls::Off, HubTls::Require] {
        let mut bad = defaults(&dir);
        bad.tls = tls;
        bad.upstream_compat = true;
        assert!(
            matches!(host.start(bad), Err(HubHostError::Invalid { .. })),
            "compat with {tls:?}"
        );
    }
    assert!(
        !host.status().running,
        "a refused start leaves nothing running"
    );

    let mut options = defaults(&dir);
    options.tls = HubTls::On;
    options.upstream_compat = true;
    let started = host.start(options).unwrap();
    let security = started.security;
    assert_eq!(security.tls, HubTls::On);
    assert!(security.upstream_compat);
    assert!(
        !security.advertises_tls,
        "no tls=1 in the compatibility mode (an upstream client would try wss://)"
    );
    assert!(security.fingerprint.is_some(), "TLS is served too");
    let line = plaintext_get(started.port, "/dspi/v1/info");
    assert!(line.contains(" 200"), "plaintext answered: {line}");
    assert!(host.stop());

    // The effects the warning lists are the hub's own.
    let effects = hub_security_effects();
    assert_eq!(
        effects.upstream_compat.len(),
        crate::server::UPSTREAM_COMPAT_EFFECTS.len()
    );
    assert!(!effects.upstream_compat.is_empty() && !effects.tls_off.is_empty());
    assert_eq!(effects.tls_off[0], crate::server::TLS_OFF_EFFECTS[0]);
}

#[test]
fn wp_p2sw_tls_off_on_loopback_stays_plaintext() {
    // HB7's conformance client (the Swift LinkClient) speaks plaintext: `off` on loopback.
    let dir = scratch("off");
    let host = HubHost::new();
    let mut options = defaults(&dir);
    options.tls = HubTls::Off;
    let started = host.start(options).unwrap();
    assert_eq!(started.security.tls, HubTls::Off);
    assert!(!started.security.advertises_tls);
    assert_eq!(started.security.fingerprint, None);
    let line = plaintext_get(started.port, "/dspi/v1/info");
    assert!(line.contains(" 200"), "plaintext answered: {line}");
    assert_eq!(
        host.pairing_link("127.0.0.1".to_owned(), String::new()),
        None,
        "no key, nothing to pin"
    );
    assert!(host.stop());
}

#[test]
fn wp_p2sw_pairing_link() {
    let dir = scratch("link");
    let host = HubHost::new();
    assert_eq!(
        host.pairing_link("127.0.0.1".to_owned(), String::new()),
        None
    );
    let started = host.start(defaults(&dir)).unwrap();
    let pairing = host.begin_pairing().unwrap();

    let link = host
        .pairing_link("192.168.1.20".to_owned(), pairing.pin.clone())
        .unwrap();
    let parsed = dspi_link::credential::parse_pair_link(&link).unwrap();
    assert_eq!(parsed.hub_id, started.hub_id);
    assert_eq!(parsed.host, "192.168.1.20");
    assert_eq!(parsed.port, started.port);
    assert_eq!(parsed.pin_code, pairing.pin);

    // Without the PIN: no `p=`.
    let bare = host
        .pairing_link("studio.local".to_owned(), String::new())
        .unwrap();
    assert!(!bare.contains("&p="), "{bare}");
    assert!(dspi_link::credential::parse_pair_link(&bare).is_ok());

    // What cannot go in a link is refused.
    for (h, p) in [("", ""), ("a b", ""), ("x&y=1", ""), ("host", "12&3")] {
        assert_eq!(
            host.pairing_link(h.to_owned(), p.to_owned()),
            None,
            "{h:?} {p:?}"
        );
    }
    assert!(host.stop());
}
