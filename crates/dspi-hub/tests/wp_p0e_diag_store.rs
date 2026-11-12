//! WP P0E: a diagnostics bundle carries a real hub store without its secrets.
//! The store is made the way a hub makes it: paired through a PIN, saved by
//! the hub's own code. Then the core's bundle is searched, byte for byte, for
//! each token and each token's SHA-256. (The hub never stores a PIN, and six
//! digits could turn up in any timestamp by chance, so the PIN is not searched.)

use std::net::{IpAddr, Ipv4Addr};
use std::path::PathBuf;
use std::time::Instant;

use data_encoding::HEXLOWER;
use dspi_core::{DIAG_REMOVED, DiagOptions, DspiCore};
use dspi_hub::auth::AuthStore;
use dspi_hub::store::HubStore;
use dspi_hub::{AccessMode, Role};
use sha2::{Digest, Sha256};

const PEER: IpAddr = IpAddr::V4(Ipv4Addr::LOCALHOST);

/// A fresh directory. `ci/run.sh` points `TMPDIR` at the home volume, not tmpfs.
fn dir(name: &str) -> PathBuf {
    let d = std::env::temp_dir()
        .join("wp-p0e-hub")
        .join(format!("{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    d
}

/// Every stored entry of a stored-only zip, by name.
fn entries(zip: &[u8]) -> Vec<(String, Vec<u8>)> {
    let u16_at = |at: usize| u16::from_le_bytes([zip[at], zip[at + 1]]) as usize;
    let u32_at =
        |at: usize| u32::from_le_bytes([zip[at], zip[at + 1], zip[at + 2], zip[at + 3]]) as usize;
    let end = zip.len() - 22;
    let mut at = u32_at(end + 16);
    let mut out = Vec::new();
    for _ in 0..u16_at(end + 10) {
        let size = u32_at(at + 20);
        let name_len = u16_at(at + 28);
        let local = u32_at(at + 42);
        let name = String::from_utf8(zip[at + 46..at + 46 + name_len].to_vec()).unwrap();
        let data = local + 30 + u16_at(local + 26) + u16_at(local + 28);
        out.push((name, zip[data..data + size].to_vec()));
        at += 46 + name_len;
    }
    out
}

#[test]
fn wp_p0e_the_bundle_keeps_no_token_or_hash_of_a_real_store() {
    let d = dir("store");
    let auth = AuthStore::new(HubStore::load(&d).unwrap(), AccessMode::Pin).unwrap();
    auth.set_name("Bench hub");
    let mut secrets = Vec::new();
    for (name, role) in [("Phone", Role::Admin), ("Laptop", Role::Viewer)] {
        let window = auth.begin_pairing(Instant::now()).unwrap();
        let token = auth
            .pair(&window.pin, name, Some(role), PEER, Instant::now())
            .unwrap()
            .token
            .unwrap();
        secrets.push(HEXLOWER.encode(&Sha256::digest(token.as_bytes())));
        secrets.push(token);
    }
    // The hub wrote the hashes; the test is pointless if it did not.
    let on_disk = std::fs::read_to_string(d.join("hub-store.json")).unwrap();
    assert!(on_disk.contains(&secrets[0]), "the store holds no hash");

    let core = DspiCore::new();
    let zip = core
        .diagnostics_zip(DiagOptions {
            app: "wp_p0e".into(),
            hub_store_dir: d.to_string_lossy().into_owned(),
            files: Vec::new(),
        })
        .unwrap();
    let files = entries(&zip);
    for s in &secrets {
        // The archive is stored, not compressed: a secret anywhere in it is
        // in these bytes as it is.
        assert!(
            !zip.windows(s.len()).any(|w| w == s.as_bytes()),
            "a secret is in the bundle"
        );
    }
    let (_, store) = files
        .iter()
        .find(|(n, _)| n == "hub/hub-store.json")
        .expect("the store is in the bundle");
    let store: serde_json::Value = serde_json::from_slice(store).unwrap();
    assert_eq!(store["name"], "Bench hub");
    assert_eq!(store["access"], "pin");
    let clients = store["clients"].as_array().unwrap();
    assert_eq!(clients.len(), 2);
    assert_eq!(clients[0]["name"], "Phone");
    assert_eq!(clients[1]["role"], "viewer");
    for c in clients {
        assert_eq!(c["token_sha256"], DIAG_REMOVED);
    }
}
