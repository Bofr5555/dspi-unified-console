//! WP P2MQ: the hub's MQTT setting in the diagnostics bundle, without the password.

use super::*;
use crate::records::DiagOptions;

/// A fresh directory. `ci/run.sh` points `TMPDIR` at the home volume, not tmpfs.
fn dir(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir()
        .join("wp-p2mq-core")
        .join(format!("{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn bundle(d: &std::path::Path) -> (Vec<String>, String) {
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
    // Entries are stored, not compressed: the text is in the bytes.
    let raw = String::from_utf8_lossy(&std::fs::read(&out).unwrap()).into_owned();
    (names, raw)
}

#[test]
fn wp_p2mq_the_bundle_carries_mqtt_json_without_the_password() {
    let d = dir("mqtt");
    // As the hub writes it, with a `tls` object as SECOUT may add one.
    std::fs::write(
        d.join("mqtt.json"),
        r#"{"v":1,"enabled":true,"broker":"broker.lan","port":1883,"client_id":null,
            "username":"hub-user","password":"mq-sekrit-91","base_topic":"venue/a",
            "ha":{"enabled":true,"key":4},
            "tls":{"mode":"on","client_key_pem":"-----BEGIN KEY----- tls-sekrit-42"}}"#,
    )
    .unwrap();
    let (names, raw) = bundle(&d);
    assert!(names.iter().any(|n| n == "hub/mqtt.json"), "{names:?}");
    assert!(raw.contains("broker.lan"));
    assert!(raw.contains("venue/a"));
    assert!(
        !raw.contains("mq-sekrit-91"),
        "the password is in the bundle"
    );
    assert!(!raw.contains("tls-sekrit-42"), "a TLS key is in the bundle");
    assert!(raw.contains(r#""password": "set""#) || raw.contains(r#""password":"set""#));
    assert!(raw.contains(r#""tls": "set""#) || raw.contains(r#""tls":"set""#));
}

#[test]
fn wp_p2mq_no_mqtt_json_is_no_entry_and_a_bad_one_is_left_out() {
    let d = dir("no-mqtt");
    let (names, _) = bundle(&d);
    assert!(!names.iter().any(|n| n == "hub/mqtt.json"));

    std::fs::write(d.join("mqtt.json"), r#"{"password":"mq-sekrit-92" "#).unwrap();
    let (names, raw) = bundle(&d);
    assert!(!names.iter().any(|n| n == "hub/mqtt.json"));
    assert!(!raw.contains("mq-sekrit-92"));
    assert!(raw.contains("mqtt.json is not a JSON object and was left out"));

    // A password of null reads `not set`.
    std::fs::write(d.join("mqtt.json"), r#"{"broker":"b","password":null}"#).unwrap();
    let (_, raw) = bundle(&d);
    assert!(raw.contains("not set"));
}
