//! WP P0E: the diagnostics bundle, against the demo device. The zip is read
//! back with a reader written here from the format, apart from the writer.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

use dspi_transport::FakeDevice;

use super::zip::crc32;
use super::*;
use crate::records::{DiagFile, DiagOptions};

fn demo() -> Arc<DspiCore> {
    let core = DspiCore::new();
    core.attach_probed(Box::new(FakeDevice::demo()), "P0E-1".into(), 0)
        .unwrap();
    core.process_notifications();
    core
}

/// A fresh directory. `ci/run.sh` points `TMPDIR` at the home volume, not tmpfs.
fn dir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join("wp-p0e-core").join(format!(
        "{name}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn u16_at(b: &[u8], at: usize) -> usize {
    u16::from_le_bytes([b[at], b[at + 1]]) as usize
}

fn u32_at(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

/// Every entry by name, through the central directory, each checked against
/// its local header and its CRC.
fn unzip(zip: &[u8]) -> BTreeMap<String, Vec<u8>> {
    let end = zip.len() - 22;
    assert_eq!(u32_at(zip, end), 0x0605_4b50, "no end record");
    let count = u16_at(zip, end + 10);
    let mut at = u32_at(zip, end + 16) as usize;
    let mut out = BTreeMap::new();
    for _ in 0..count {
        assert_eq!(u32_at(zip, at), 0x0201_4b50);
        assert_eq!(u16_at(zip, at + 10), 0, "stored");
        let crc = u32_at(zip, at + 16);
        let size = u32_at(zip, at + 20) as usize;
        let name_len = u16_at(zip, at + 28);
        let local = u32_at(zip, at + 42) as usize;
        let name = String::from_utf8(zip[at + 46..at + 46 + name_len].to_vec()).unwrap();
        assert_eq!(u32_at(zip, local), 0x0403_4b50);
        assert_eq!(u32_at(zip, local + 14), crc);
        let data_at = local + 30 + u16_at(zip, local + 26) + u16_at(zip, local + 28);
        let data = zip[data_at..data_at + size].to_vec();
        assert_eq!(crc32(&data), crc, "{name}");
        out.insert(name, data);
        at += 46 + name_len;
    }
    out
}

fn text(files: &BTreeMap<String, Vec<u8>>, name: &str) -> String {
    String::from_utf8(
        files
            .get(name)
            .unwrap_or_else(|| panic!("no {name}"))
            .clone(),
    )
    .unwrap()
}

#[test]
fn wp_p0e_crc32_matches_the_check_value() {
    assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    assert_eq!(crc32(b""), 0);
}

#[test]
fn wp_p0e_utc_calendar() {
    assert_eq!(utc(0), (1970, 1, 1, 0, 0, 0));
    // 2026-11-12 13:45:07 UTC
    assert_eq!(utc(1_794_491_107_000), (2026, 11, 12, 13, 45, 7));
    assert_eq!(utc(951_782_400_000), (2000, 2, 29, 0, 0, 0));
    assert_eq!(utc(-1000), (1969, 12, 31, 23, 59, 59));
    // Nothing panics at the ends.
    let _ = utc(i64::MIN);
    let _ = utc(i64::MAX);
}

#[test]
fn wp_p0e_a_connected_bundle_has_every_section() {
    let core = demo();
    let options = DiagOptions {
        app: "test client 1.0".into(),
        hub_store_dir: String::new(),
        files: vec![DiagFile {
            name: "doctor.txt".into(),
            text: "ok    USB subsystem reachable\n".into(),
        }],
    };
    let files = unzip(&core.diagnostics_zip(options).unwrap());
    for name in [
        "README.txt",
        "versions.json",
        "device/dump.json",
        "device/stats.json",
        "device/interrupts.txt",
        "events/P0E-1.jsonl",
        "system/os.json",
        "system/usb.json",
        "client/doctor.txt",
        "notes.txt",
    ] {
        assert!(
            files.contains_key(name),
            "{name} missing: {:?}",
            files.keys()
        );
    }
    let versions: Value = serde_json::from_str(&text(&files, "versions.json")).unwrap();
    assert_eq!(versions["client"], "test client 1.0");
    assert_eq!(versions["core"], env!("CARGO_PKG_VERSION"));
    assert!(versions["device"]["firmware"].is_string());

    let dump: Value = serde_json::from_str(&text(&files, "device/dump.json")).unwrap();
    assert_eq!(dump["serial"], "P0E-1");
    assert_eq!(dump["readOnly"], false);
    assert!(dump["capabilities"]["features"].is_array(), "{dump}");
    assert!(dump["state"]["numChannels"].as_u64().unwrap() > 0);

    let stats: Value = serde_json::from_str(&text(&files, "device/stats.json")).unwrap();
    assert!(stats["counters"]["sample_rate"].is_u64(), "{stats}");
    assert!(stats["buffers"].is_object());

    assert!(text(&files, "events/P0E-1.jsonl").contains("\"kind\":\"connected\""));
    assert!(text(&files, "client/doctor.txt").contains("USB subsystem"));
    let readme = text(&files, "README.txt");
    assert!(readme.contains("Nothing in this file was sent anywhere"));
    assert!(readme.contains("device/dump.json"), "the list of files");
    assert!(text(&files, "notes.txt").contains("hub: no store named"));
}

#[test]
fn wp_p0e_the_interrupt_log_holds_the_monitor_lines() {
    use dspi_session::notify::{Event, Notification};
    let core = demo();
    for (seq, slot) in [(1, 0), (2, 3)] {
        core.lock().monitor.record(&Notification {
            seq,
            event: Event::PresetLoaded { slot },
            lost: false,
            origin: None,
            received: dspi_transport::clock::Instant::now(),
        });
    }
    let lines = core.monitor_read(0).entries;
    assert!(lines.len() >= 2);
    let files = unzip(&core.diagnostics_zip(DiagOptions::default()).unwrap());
    let log = text(&files, "device/interrupts.txt");
    for l in &lines {
        assert!(log.contains(&l.text), "{log}");
    }
}

#[test]
fn wp_p0e_without_a_device_the_bundle_says_so_and_is_still_written() {
    let core = DspiCore::new();
    let files = unzip(&core.diagnostics_zip(DiagOptions::default()).unwrap());
    assert!(!files.contains_key("device/dump.json"));
    assert!(files.contains_key("system/os.json"));
    let notes = text(&files, "notes.txt");
    assert!(notes.contains("device: none open"), "{notes}");
    assert!(text(&files, "README.txt").contains("No device was open."));
    let versions: Value = serde_json::from_str(&text(&files, "versions.json")).unwrap();
    assert!(versions["device"].is_null());
}

#[test]
fn wp_p0e_the_hub_store_loses_its_tokens_and_secrets() {
    // -- WP FLAKE4 -- a PIN no bundle can hold by chance ("123456" could, in a
    // stamp or a zip header); '~' is in no number, hex or base64.
    const PIN: &str = "~flake4~123456~";
    let d = dir("hub");
    let hash = "9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08";
    std::fs::write(
        d.join("hub-store.json"),
        json!({
            "hub_id": "0b9a3c2e-5d1f-4c11-9e0a-1f2e3d4c5b6a",
            "name": "Studio",
            "access": "pin",
            "next_cid": 3,
            "clients": [
                { "cid": 1, "name": "Phone", "role": "admin", "created": 1,
                  "token_sha256": hash },
                { "cid": 2, "name": "Laptop", "role": "viewer", "created": 2,
                  "last_seen": 5, "token_sha256": "aa".repeat(32) }
            ],
            "device_names": { "SER1": "Kitchen" },
            "future": { "apiKey": "k-123", "nested": [{ "Client-Secret": "s-456" }] },
            "pin": PIN
        })
        .to_string(),
    )
    .unwrap();
    let core = DspiCore::new();
    let options = DiagOptions {
        hub_store_dir: d.to_string_lossy().into_owned(),
        ..Default::default()
    };
    let zip = core.diagnostics_zip(options).unwrap();
    let raw = String::from_utf8_lossy(&zip);
    for secret in [hash, &"aa".repeat(32), "k-123", "s-456", PIN] {
        assert!(!raw.contains(secret), "{secret} is in the bundle");
    }
    let files = unzip(&zip);
    let store: Value = serde_json::from_str(&text(&files, "hub/hub-store.json")).unwrap();
    assert_eq!(store["clients"][0]["token_sha256"], REMOVED);
    assert_eq!(store["clients"][1]["name"], "Laptop");
    assert_eq!(store["clients"][1]["role"], "viewer");
    assert_eq!(store["access"], "pin", "the access mode is not a secret");
    assert_eq!(store["name"], "Studio");
    assert_eq!(store["device_names"]["SER1"], "Kitchen");
    assert_eq!(store["future"]["apiKey"], REMOVED);
    assert_eq!(store["future"]["nested"][0]["Client-Secret"], REMOVED);
    assert_eq!(store["pin"], REMOVED);
}

#[test]
fn wp_p0e_a_hub_store_that_is_not_json_is_left_out() {
    let d = dir("hub-bad");
    std::fs::write(d.join("hub-store.json"), "token=abcdef0123456789 {").unwrap();
    let core = DspiCore::new();
    let zip = core
        .diagnostics_zip(DiagOptions {
            hub_store_dir: d.to_string_lossy().into_owned(),
            ..Default::default()
        })
        .unwrap();
    assert!(!String::from_utf8_lossy(&zip).contains("abcdef0123456789"));
    let files = unzip(&zip);
    assert!(!files.contains_key("hub/hub-store.json"));
    assert!(text(&files, "notes.txt").contains("not JSON and was left out"));
}

#[test]
fn wp_p0e_save_writes_one_zip_and_names_its_files() {
    let d = dir("save");
    let path = d.join("bundle.zip");
    let core = demo();
    let names = core
        .diagnostics_save(path.to_string_lossy().into_owned(), DiagOptions::default())
        .unwrap();
    let files = unzip(&std::fs::read(&path).unwrap());
    let mut sorted = names.clone();
    sorted.sort();
    assert_eq!(sorted, files.keys().cloned().collect::<Vec<_>>());
    assert_eq!(names.first().map(String::as_str), Some("README.txt"));
    assert!(!d.join("bundle.zip.part").exists());
    // A folder that does not exist is an I/O error, not a panic.
    let err = core
        .diagnostics_save(
            d.join("missing/x.zip").to_string_lossy().into_owned(),
            DiagOptions::default(),
        )
        .unwrap_err();
    assert!(matches!(err, DspiError::Io { .. }), "{err:?}");
}

#[test]
fn wp_p0e_client_file_names_cannot_leave_their_folder() {
    let core = DspiCore::new();
    let files = unzip(
        &core
            .diagnostics_zip(DiagOptions {
                files: vec![
                    DiagFile {
                        name: "../../etc/passwd".into(),
                        text: "a".into(),
                    },
                    DiagFile {
                        name: "../../etc/passwd".into(),
                        text: "b".into(),
                    },
                    DiagFile {
                        name: String::new(),
                        text: "c".into(),
                    },
                ],
                ..Default::default()
            })
            .unwrap(),
    );
    let client: Vec<_> = files.keys().filter(|k| k.starts_with("client/")).collect();
    assert_eq!(
        client,
        [
            "client/_.._etc_passwd",
            "client/_.._etc_passwd-2",
            "client/unnamed"
        ]
    );
}

#[test]
fn wp_p0e_the_file_name_is_dated() {
    let name = DspiCore::new().diagnostics_file_name();
    assert!(
        name.starts_with("dspi-diagnostics-") && name.ends_with("Z.zip"),
        "{name}"
    );
    assert_eq!(name.len(), "dspi-diagnostics-20261112-134507Z.zip".len());
}

#[test]
fn wp_p0e_secret_keys() {
    for k in [
        "token",
        "token_sha256",
        "PIN",
        "apiKey",
        "password",
        "Client-Secret",
        "auth",
    ] {
        assert!(is_secret_key(k), "{k}");
    }
    for k in [
        "access",
        "name",
        "cid",
        "role",
        "hub_id",
        "device_names",
        "created",
    ] {
        assert!(!is_secret_key(k), "{k}");
    }
}

/// Review p0e N4: the home folder reads `~` and the user name `<user>` in
/// paths, and nothing else is touched.
#[test]
fn wp_p0e_paths_lose_the_user() {
    let s = Scrub::new(Some("/home/bob/".into()), Some("bob".into()));
    assert_eq!(
        s.apply("hub: no store at /home/bob/.local/share/uconsole/hub/hub-store.json"),
        "hub: no store at ~/.local/share/uconsole/hub/hub-store.json"
    );
    assert_eq!(
        s.apply("/run/media/bob/RPI-RP2"),
        "/run/media/<user>/RPI-RP2"
    );
    assert_eq!(s.apply("--events /home/bob: none"), "--events ~: none");
    for kept in [
        "/home/bobby/x",
        "serial bob123",
        "bob",
        "/media/bobcat/RP2350",
    ] {
        assert_eq!(s.apply(kept), kept);
    }
    let w = Scrub::new(Some(r"C:\Users\Bob".into()), Some("Bob".into()));
    assert_eq!(w.apply(r"C:\Users\Bob\AppData\x; E:\"), r"~\AppData\x; E:\");
    assert_eq!(w.apply(r"D:\Shares\Bob\y"), r"D:\Shares\<user>\y");
    // A home of the root is no home to hide.
    for root in ["/", r"C:\"] {
        assert_eq!(
            Scrub::new(Some(root.into()), None).apply("/etc/x"),
            "/etc/x"
        );
    }
}

#[test]
fn wp_p0e_notes_and_readme_say_so() {
    let Some(home) = std::env::var("HOME").ok().filter(|h| h.len() > 2) else {
        return;
    };
    let core = DspiCore::new();
    let files = unzip(
        &core
            .diagnostics_zip(DiagOptions {
                hub_store_dir: format!("{home}/wp-p0e-no-store"),
                ..Default::default()
            })
            .unwrap(),
    );
    let notes = text(&files, "notes.txt");
    assert!(
        notes.contains("hub: no store at ~/wp-p0e-no-store/hub-store.json"),
        "{notes}"
    );
    assert!(!notes.contains(&home), "{notes}");
    let readme = text(&files, "README.txt");
    assert!(
        readme.contains("\"~\"") && readme.contains("\"<user>\""),
        "{readme}"
    );
}
