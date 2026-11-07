//! What a hub keeps on disk: hub id, name, token hashes, device names. Owned by
//! WP HB1. A plain file, saved atomically. Not a keychain, because the hub holds
//! only hashes (spec 6.2).

use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use data_encoding::HEXLOWER;
use serde::{Deserialize, Serialize};

use crate::error::HubError;
use crate::session::Role;

const FILE_NAME: &str = "hub-store.json";
const TEMP_NAME: &str = "hub-store.json.tmp";
/// Far above what the caps below can produce. A larger file is not ours.
const MAX_FILE: u64 = 1 << 20;
/// Longest device name kept, in characters (a board's own is 31).
const MAX_DEVICE_NAME: usize = 64;
/// Device names kept. Only a board that has been attached gets one.
const MAX_DEVICE_NAMES: usize = 256;
/// A SHA-256.
pub(crate) type TokenHash = [u8; 32];

/// One paired client, as stored: the hash, never the token.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StoredClient {
    pub(crate) cid: u32,
    pub(crate) name: String,
    pub(crate) role: Role,
    pub(crate) created: u64,
    pub(crate) last_seen: Option<u64>,
    pub(crate) token_hash: TokenHash,
}

#[derive(Debug, Serialize, Deserialize)]
struct ClientRecord {
    cid: u32,
    name: String,
    role: String,
    created: u64,
    #[serde(default)]
    last_seen: Option<u64>,
    token_sha256: String,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct FileData {
    hub_id: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    access: Option<String>,
    #[serde(default)]
    next_cid: u32,
    #[serde(default)]
    clients: Vec<ClientRecord>,
    #[serde(default)]
    device_names: BTreeMap<String, String>,
}

/// The persistent state, as loaded.
#[derive(Debug)]
pub struct HubStore {
    dir: PathBuf,
    hub_id: String,
    name: Option<String>,
    access: Option<String>,
    next_cid: u32,
    clients: Vec<StoredClient>,
    device_names: BTreeMap<String, String>,
}

fn io_error(what: &str, path: &Path, e: &io::Error) -> HubError {
    HubError::Io(format!("{what} {}: {e}", path.display()))
}

/// Random bytes from the OS, or an error: never a weaker source.
pub(crate) fn random_bytes<const N: usize>() -> Result<[u8; N], HubError> {
    let mut bytes = [0u8; N];
    getrandom::fill(&mut bytes)
        .map_err(|e| HubError::Io(format!("the system has no random numbers: {e}")))?;
    Ok(bytes)
}

/// A lowercase UUID, version 4 (`hid`, spec 7.2).
fn new_hub_id() -> Result<String, HubError> {
    let mut b: [u8; 16] = random_bytes()?;
    b[6] = (b[6] & 0x0F) | 0x40;
    b[8] = (b[8] & 0x3F) | 0x80;
    let hex = HEXLOWER.encode(&b);
    let part = |r: std::ops::Range<usize>| hex.get(r).unwrap_or_default();
    Ok(format!(
        "{}-{}-{}-{}-{}",
        part(0..8),
        part(8..12),
        part(12..16),
        part(16..20),
        part(20..32)
    ))
}

pub(crate) fn role_name(role: Role) -> &'static str {
    match role {
        Role::Viewer => "viewer",
        Role::Control => "control",
        Role::Admin => "admin",
    }
}

pub(crate) fn parse_role(text: &str) -> Option<Role> {
    match text {
        "viewer" => Some(Role::Viewer),
        "control" => Some(Role::Control),
        "admin" => Some(Role::Admin),
        _ => None,
    }
}

/// Seconds since the epoch; 0 for a clock set before it.
pub(crate) fn unix_seconds(t: SystemTime) -> u64 {
    t.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

pub(crate) fn system_time(secs: u64) -> SystemTime {
    UNIX_EPOCH
        .checked_add(Duration::from_secs(secs))
        .unwrap_or(UNIX_EPOCH)
}

fn read_limited(path: &Path) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    File::open(path)?
        .take(MAX_FILE.saturating_add(1))
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_FILE {
        return Err(io::Error::other("too large"));
    }
    Ok(bytes)
}

fn decode(bytes: &[u8]) -> Result<FileData, String> {
    let data: FileData = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
    if data.hub_id.is_empty() {
        return Err("no hub id".into());
    }
    Ok(data)
}

fn stored_client(record: ClientRecord) -> Option<StoredClient> {
    let token_hash = HEXLOWER
        .decode(record.token_sha256.as_bytes())
        .ok()?
        .try_into()
        .ok()?;
    Some(StoredClient {
        cid: record.cid,
        name: record.name,
        role: parse_role(&record.role)?,
        created: record.created,
        last_seen: record.last_seen,
        token_hash,
    })
}

impl HubStore {
    /// Load from `dir`, creating it and a new hub id on first use. An unreadable
    /// file is set aside, never a panic: the hub then has no paired clients.
    pub fn load(dir: &Path) -> Result<Self, HubError> {
        fs::create_dir_all(dir).map_err(|e| io_error("cannot create", dir, &e))?;
        let path = dir.join(FILE_NAME);
        let data = match read_limited(&path) {
            Ok(bytes) => decode(&bytes).map(Some),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.to_string()),
        };
        let data = match data {
            Ok(data) => data,
            Err(why) => {
                let aside = dir.join(format!(
                    "{FILE_NAME}.bad-{}",
                    unix_seconds(SystemTime::now())
                ));
                log::warn!("hub store {} set aside: {why}", path.display());
                fs::rename(&path, &aside).map_err(|e| io_error("cannot move", &path, &e))?;
                None
            }
        };
        let Some(data) = data else {
            let store = Self {
                dir: dir.to_owned(),
                hub_id: new_hub_id()?,
                name: None,
                access: None,
                next_cid: 1,
                clients: Vec::new(),
                device_names: BTreeMap::new(),
            };
            // Now, so the id is the same at the next start even with nothing paired.
            store.save()?;
            return Ok(store);
        };
        let clients: Vec<StoredClient> =
            data.clients.into_iter().filter_map(stored_client).collect();
        // A cid is never reused, even after an edited file.
        let next_cid = clients
            .iter()
            .map(|c| c.cid.saturating_add(1))
            .fold(data.next_cid.max(1), u32::max);
        Ok(Self {
            dir: dir.to_owned(),
            hub_id: data.hub_id,
            name: data.name,
            access: data.access,
            next_cid,
            clients,
            device_names: data.device_names,
        })
    }

    /// Save, replacing the old file atomically.
    pub fn save(&self) -> Result<(), HubError> {
        let data = FileData {
            hub_id: self.hub_id.clone(),
            name: self.name.clone(),
            access: self.access.clone(),
            next_cid: self.next_cid,
            clients: self
                .clients
                .iter()
                .map(|c| ClientRecord {
                    cid: c.cid,
                    name: c.name.clone(),
                    role: role_name(c.role).to_owned(),
                    created: c.created,
                    last_seen: c.last_seen,
                    token_sha256: HEXLOWER.encode(&c.token_hash),
                })
                .collect(),
            device_names: self.device_names.clone(),
        };
        let bytes = serde_json::to_vec_pretty(&data)
            .map_err(|e| HubError::Io(format!("cannot encode the hub store: {e}")))?;
        let temp = self.dir.join(TEMP_NAME);
        let path = self.dir.join(FILE_NAME);
        let written = (|| {
            let mut file = private_file(&temp)?;
            file.write_all(&bytes)?;
            file.sync_all()?;
            fs::rename(&temp, &path)
        })();
        written.map_err(|e| io_error("cannot write", &path, &e))?;
        sync_dir(&self.dir);
        Ok(())
    }

    /// `hid`.
    pub fn hub_id(&self) -> &str {
        &self.hub_id
    }

    /// The name stored for the hub, if one was ever set.
    pub fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }

    /// Set the hub's name, for [`Self::save`].
    pub fn set_name(&mut self, name: &str) {
        self.name = Some(name.to_owned());
    }

    /// The name stored for a device (WP HB3).
    pub fn device_name(&self, serial: &str) -> Option<&str> {
        self.device_names.get(serial).map(String::as_str)
    }

    /// Store a device's name, or forget it with `None`, for [`Self::save`] (WP HB3).
    /// False when the table is full and this is a new device. The name is cut to
    /// a size a UI can show.
    pub fn set_device_name(&mut self, serial: &str, name: Option<&str>) -> bool {
        let Some(name) = name else {
            self.device_names.remove(serial);
            return true;
        };
        if self.device_names.len() >= MAX_DEVICE_NAMES && !self.device_names.contains_key(serial) {
            return false;
        }
        let name = name.chars().take(MAX_DEVICE_NAME).collect();
        self.device_names.insert(serial.to_owned(), name);
        true
    }

    pub(crate) fn access(&self) -> Option<&str> {
        self.access.as_deref()
    }

    pub(crate) fn set_access(&mut self, access: &str) {
        self.access = Some(access.to_owned());
    }

    pub(crate) fn clients(&self) -> &[StoredClient] {
        &self.clients
    }

    pub(crate) fn clients_mut(&mut self) -> &mut Vec<StoredClient> {
        &mut self.clients
    }

    /// The next `cid`, never given out before.
    pub(crate) fn take_cid(&mut self) -> u32 {
        let cid = self.next_cid;
        self.next_cid = cid.saturating_add(1);
        cid
    }
}

#[cfg(unix)]
fn private_file(path: &Path) -> io::Result<File> {
    use std::os::unix::fs::OpenOptionsExt;
    OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)
}

#[cfg(not(unix))]
fn private_file(path: &Path) -> io::Result<File> {
    OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(path)
}

/// So the rename survives a power cut. Windows has no such call, and it is worth no
/// failure: the file itself is whole either way.
fn sync_dir(dir: &Path) {
    #[cfg(unix)]
    if let Ok(d) = File::open(dir) {
        let _ = d.sync_all();
    }
    #[cfg(not(unix))]
    let _ = dir;
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
pub(crate) mod tests_wp_hb1 {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    /// A fresh directory. `ci/run.sh` points `TMPDIR` at the home volume, not tmpfs.
    pub(crate) fn scratch(what: &str) -> PathBuf {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let dir = std::env::temp_dir().join("wp-hb1-unit").join(format!(
            "{what}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::SeqCst)
        ));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    fn client(cid: u32) -> StoredClient {
        StoredClient {
            cid,
            name: "phone".into(),
            role: Role::Control,
            created: 1,
            last_seen: None,
            token_hash: [7; 32],
        }
    }

    #[test]
    fn wp_hb1_the_hub_id_is_a_uuid_and_survives_a_restart() {
        let dir = scratch("id");
        let a = HubStore::load(&dir).unwrap();
        let id = a.hub_id().to_owned();
        assert_eq!(id.len(), 36);
        assert_eq!(id.as_bytes()[14], b'4');
        assert_eq!(id, id.to_lowercase());
        assert_eq!(HubStore::load(&dir).unwrap().hub_id(), id);
    }

    #[test]
    fn wp_hb1_clients_and_names_survive_and_a_cid_is_not_reused() {
        let dir = scratch("round");
        let mut a = HubStore::load(&dir).unwrap();
        let cid = a.take_cid();
        a.clients_mut().push(client(cid));
        a.set_name("Studio");
        assert!(a.set_device_name("E46", Some("Living room")));
        a.save().unwrap();
        let mut b = HubStore::load(&dir).unwrap();
        assert_eq!(b.clients(), &[client(cid)]);
        assert_eq!(b.name(), Some("Studio"));
        assert_eq!(b.device_name("E46"), Some("Living room"));
        b.clients_mut().clear();
        b.save().unwrap();
        assert!(HubStore::load(&dir).unwrap().take_cid() > cid);
    }

    #[test]
    fn wp_hb1_a_corrupt_file_is_set_aside_and_never_a_panic() {
        for junk in [
            &b"{ not json"[..],
            b"",
            b"[]",
            b"{\"hub_id\":\"\"}",
            &[0xFF, 0xFE, 0x00],
        ] {
            let dir = scratch("corrupt");
            fs::create_dir_all(&dir).unwrap();
            fs::write(dir.join(FILE_NAME), junk).unwrap();
            let s = HubStore::load(&dir).unwrap();
            assert!(s.clients().is_empty());
            assert_eq!(s.hub_id().len(), 36);
            let kept = fs::read_dir(&dir)
                .unwrap()
                .filter(|e| {
                    let name = e.as_ref().unwrap().file_name();
                    name.to_string_lossy().contains(".bad-")
                })
                .count();
            assert_eq!(kept, 1, "the bad file is kept to look at");
        }
    }

    #[test]
    fn wp_hb1_an_oversized_file_is_set_aside() {
        let dir = scratch("huge");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join(FILE_NAME), vec![b' '; MAX_FILE as usize + 1]).unwrap();
        assert_eq!(HubStore::load(&dir).unwrap().hub_id().len(), 36);
    }

    #[test]
    fn wp_hb1_a_mangled_client_record_is_dropped_alone() {
        let dir = scratch("record");
        fs::create_dir_all(&dir).unwrap();
        let good = "ab".repeat(32);
        let text = format!(
            r#"{{"hub_id":"x","clients":[
              {{"cid":1,"name":"bad hash","role":"admin","created":0,"token_sha256":"zz"}},
              {{"cid":2,"name":"short","role":"admin","created":0,"token_sha256":"abab"}},
              {{"cid":3,"name":"bad role","role":"root","created":0,"token_sha256":"{good}"}},
              {{"cid":9,"name":"good","role":"viewer","created":0,"token_sha256":"{good}"}}
            ]}}"#
        );
        fs::write(dir.join(FILE_NAME), text).unwrap();
        let mut s = HubStore::load(&dir).unwrap();
        assert_eq!(s.clients().len(), 1);
        assert_eq!(s.clients()[0].cid, 9);
        assert_eq!(s.take_cid(), 10);
    }

    #[test]
    fn wp_hb1_a_save_leaves_no_temp_file() {
        let dir = scratch("temp");
        HubStore::load(&dir).unwrap();
        assert!(!dir.join(TEMP_NAME).exists());
    }

    #[cfg(unix)]
    #[test]
    fn wp_hb1_the_file_is_private_to_its_owner() {
        use std::os::unix::fs::PermissionsExt;
        let dir = scratch("mode");
        HubStore::load(&dir).unwrap();
        let mode = fs::metadata(dir.join(FILE_NAME))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o077, 0, "{mode:o}");
    }

    #[test]
    fn wp_hb1_device_names_are_capped() {
        let dir = scratch("caps");
        let mut s = HubStore::load(&dir).unwrap();
        assert!(s.set_device_name("a", Some(&"x".repeat(1000))));
        assert_eq!(s.device_name("a").unwrap().len(), MAX_DEVICE_NAME);
        for n in 0..MAX_DEVICE_NAMES {
            s.set_device_name(&format!("s{n}"), Some("n"));
        }
        assert!(!s.set_device_name("one more", Some("n")));
        assert!(s.set_device_name("a", Some("still may rename")));
        assert!(s.set_device_name("a", None));
        assert_eq!(s.device_name("a"), None);
    }
}
