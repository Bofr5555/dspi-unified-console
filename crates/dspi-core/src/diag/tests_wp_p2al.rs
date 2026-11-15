//! WP P2AL: the hub's alarm rules and notification targets in the diagnostics bundle,
//! with no secret: a webhook secret, a CA, a pin and a URL's path shown as `set`, `not
//! set` or cut.

use super::*;
use crate::records::DiagOptions;

/// A fresh directory. `ci/run.sh` points `TMPDIR` at the home volume, not tmpfs.
fn dir(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir()
        .join("wp-p2al-core")
        .join(format!("{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

const SECRET: &str = "5ec7e75ec7e75ec7e75ec7e75ec7e75ec7e75ec7e75ec7e75ec7e75ec7e75ec7";

#[test]
fn wp_p2al_the_bundle_carries_alarms_and_outbound_without_secrets() {
    let d = dir("both");
    // As dspi-hub writes them (`device_alarms.rs`, `outbound/mod.rs`), with PLAN-SEC's
    // TLS fields a later build adds.
    std::fs::write(
        d.join("alarms.json"),
        r#"{"v":1,"rules":{"temperature":{"on":true,"above_c":65.0},
            "clip":{"on":true,"per_minute":10,"mute_on_storm":true}}}"#,
    )
    .unwrap();
    std::fs::write(
        d.join("outbound.json"),
        format!(
            r#"{{"v":1,"targets":[
              {{"id":"ha","name":"Home Assistant","kind":"webhook",
                "url":"http://user:pw-in-url@10.0.0.2:8123/api/webhook/hook-token-991?x=1",
                "enabled":true,"kinds":["alarm.raised"],"secret":"{SECRET}",
                "tls":{{"mode":"on","ca_pem":"-----BEGIN CERTIFICATE-----MIIca-body","pin_sha256":"pin-value-443"}}}},
              {{"id":"phone","name":"Phone","kind":"push","url":"http://ntfy.local",
                "enabled":true,"kinds":[],"ca_pem":null}}]}}"#
        ),
    )
    .unwrap();
    let out = d.join("bundle.zip");
    let names = DspiCore::new()
        .diagnostics_save(
            out.to_string_lossy().into_owned(),
            DiagOptions {
                hub_store_dir: d.to_string_lossy().into_owned(),
                ..Default::default()
            },
        )
        .unwrap();
    assert!(names.iter().any(|n| n == "hub/alarms.json"), "{names:?}");
    assert!(names.iter().any(|n| n == "hub/outbound.json"), "{names:?}");
    // Entries are stored, not compressed: the text is in the bytes.
    let zip = std::fs::read(&out).unwrap();
    let raw = String::from_utf8_lossy(&zip);
    assert!(raw.contains("mute_on_storm"));
    assert!(raw.contains("Home Assistant"));
    assert!(
        raw.contains("http://10.0.0.2:8123/(removed)"),
        "the host stays"
    );
    assert!(raw.contains("http://ntfy.local"));
    for secret in [
        SECRET,
        "pw-in-url",
        "hook-token-991",
        "MIIca-body",
        "pin-value-443",
    ] {
        assert!(!raw.contains(secret), "{secret} is in the bundle");
    }
    // Set or not set, so a supporter can tell.
    let mut v: Value =
        serde_json::from_str(&std::fs::read_to_string(d.join("outbound.json")).unwrap()).unwrap();
    redact_outbound(&mut v);
    assert_eq!(v["targets"][0]["secret"], "set");
    assert_eq!(v["targets"][0]["tls"]["ca_pem"], "set");
    assert_eq!(v["targets"][0]["tls"]["pin_sha256"], "set");
    assert_eq!(v["targets"][0]["tls"]["mode"], "on");
    assert_eq!(v["targets"][1]["ca_pem"], "not set");
    assert_eq!(v["targets"][1]["kinds"], json!([]));
}

#[test]
fn wp_p2al_no_files_no_entries_and_a_bad_outbound_is_left_out() {
    let d = dir("none");
    let options = || DiagOptions {
        hub_store_dir: d.to_string_lossy().into_owned(),
        ..Default::default()
    };
    let out = d.join("bundle.zip");
    let names = DspiCore::new()
        .diagnostics_save(out.to_string_lossy().into_owned(), options())
        .unwrap();
    assert!(
        !names
            .iter()
            .any(|n| n == "hub/alarms.json" || n == "hub/outbound.json")
    );
    std::fs::write(
        d.join("outbound.json"),
        format!("{{\"targets\": [\"{SECRET}\""),
    )
    .unwrap();
    let zip = DspiCore::new().diagnostics_zip(options()).unwrap();
    let raw = String::from_utf8_lossy(&zip);
    assert!(!raw.contains(SECRET));
    assert!(raw.contains("outbound.json is not JSON and was left out"));
}

#[test]
fn wp_p2al_a_url_keeps_its_scheme_and_host_only() {
    assert_eq!(url_host("http://10.0.0.2:8080"), "http://10.0.0.2:8080");
    assert_eq!(url_host("http://h/p?q"), "http://h/(removed)");
    assert_eq!(url_host("http://a:b@h:1/"), "http://h:1/(removed)");
    assert_eq!(url_host("http://h?token=x"), "http://h/(removed)");
    assert_eq!(url_host("no scheme"), REMOVED);
    assert_eq!(url_host(""), REMOVED);
}
