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

// -- WP P2RS --
/// What a paired client is (OD-P2q): a console that paired with a PIN, or an API key
/// an Admin minted. One table, one cap ([`crate::auth::MAX_PAIRED_CLIENTS`]), one
/// revoke for both.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ClientKind {
    /// Paired with a PIN. Every client stored before API keys existed is one.
    #[default]
    Console,
    /// Minted by an Admin (`auth.mint`), without a PIN.
    Api,
}

impl ClientKind {
    /// Its word in `auth.list` and the store.
    pub fn wire_name(self) -> &'static str {
        match self {
            Self::Console => "console",
            Self::Api => "api",
        }
    }

    fn parse(word: Option<&str>) -> Option<Self> {
        match word {
            None | Some("console") => Some(Self::Console),
            Some("api") => Some(Self::Api),
            Some(_) => None,
        }
    }
}

/// One paired client, as stored: the hash, never the token.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StoredClient {
    pub(crate) cid: u32,
    pub(crate) name: String,
    pub(crate) role: Role,
    pub(crate) created: u64,
    pub(crate) last_seen: Option<u64>,
    pub(crate) token_hash: TokenHash,
    // -- WP P2RS --
    pub(crate) kind: ClientKind,
    /// May be used over plaintext once the Link port speaks TLS (OD-SECk, WP SECSRV).
    /// Stored and listed; no effect yet.
    pub(crate) plaintext_ok: bool,
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
    // -- WP P2RS -- absent for a console, so an older hub reads the file unchanged
    #[serde(default, skip_serializing_if = "Option::is_none")]
    kind: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    plaintext_ok: bool,
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
    // -- WP SW2 -- kept as JSON, so a model this hub cannot read never costs
    // the clients and names; checked when it is used.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    system: Option<serde_json::Value>,
    // -- WP AUSB2 -- the USB-direct start offsets the hub last wrote, by unit id.
    // -- fixup ausb2 (F3) -- kept as JSON: an entry that is not a whole number is
    // dropped alone ([`usb_offsets`]), never the store.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    system_usb_offsets: Option<serde_json::Value>,
    // -- WP P1HL -- limits profiles and installer PIN hashes by serial, and the
    // modes pinned per role. Kept as JSON, read leniently ([`limits_of`]): one bad
    // entry is dropped alone, never the store. `device_pins` holds only salted,
    // slow hashes, and a diagnostics bundle removes it (its key says `pin`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    device_limits: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    device_pins: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    role_modes: Option<serde_json::Value>,
    // -- WP P1DKFIX -- F6: wrong PINs and their window, by serial
    #[serde(default, skip_serializing_if = "Option::is_none")]
    device_pin_tries: Option<serde_json::Value>,
    // -- WP HUBADMIN -- the recovery codes' salted, slow hashes, by serial (the
    // key says `pin`: a diagnostics bundle removes it)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    device_pin_recovery: Option<serde_json::Value>,
}

// -- WP P1HL --
/// The entries of a JSON map that read as `T`; anything else is dropped, with a
/// warning naming `what`.
fn entries_of<T: serde::de::DeserializeOwned>(
    v: Option<&serde_json::Value>,
    what: &str,
) -> BTreeMap<String, T> {
    let Some(v) = v else {
        return BTreeMap::new();
    };
    let Some(m) = v.as_object() else {
        log::warn!("hub store: {what} is not a map; ignored");
        return BTreeMap::new();
    };
    let kept: BTreeMap<String, T> = m
        .iter()
        .filter_map(|(k, v)| Some((k.clone(), serde_json::from_value(v.clone()).ok()?)))
        .collect();
    if kept.len() != m.len() {
        log::warn!(
            "hub store: {} {what} entries unreadable; dropped",
            m.len().saturating_sub(kept.len())
        );
    }
    kept
}

// -- fixup ausb2 (F3) --
/// The start offsets kept, leniently: entries whose value is not a whole
/// number are left out; anything but an object is none.
fn usb_offsets(v: Option<serde_json::Value>) -> Option<BTreeMap<String, i64>> {
    let serde_json::Value::Object(m) = v? else {
        log::warn!("hub store: system_usb_offsets is not a map; ignored");
        return None;
    };
    let total = m.len();
    let kept: BTreeMap<String, i64> = m
        .into_iter()
        .filter_map(|(k, v)| Some((k, v.as_i64()?)))
        .collect();
    if kept.len() != total {
        log::warn!(
            "hub store: {} system_usb_offsets entries are not whole numbers; dropped",
            total.saturating_sub(kept.len())
        );
    }
    Some(kept)
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
    // -- WP SW2 --
    system: Option<serde_json::Value>,
    // -- WP AUSB2 --
    system_usb_offsets: Option<BTreeMap<String, i64>>,
    // -- WP P1HL --
    limits: StoredLimitsJson,
}

// -- WP P1HL --
/// [`FileData`]'s limits fields, as kept.
#[derive(Debug, Clone, Default)]
struct StoredLimitsJson {
    device_limits: Option<serde_json::Value>,
    device_pins: Option<serde_json::Value>,
    role_modes: Option<serde_json::Value>,
    device_pin_tries: Option<serde_json::Value>, // -- WP P1DKFIX --
    device_pin_recovery: Option<serde_json::Value>, // -- WP HUBADMIN --
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
        // -- WP P2RS -- an unknown kind is dropped alone, as a mangled record is
        kind: ClientKind::parse(record.kind.as_deref())?,
        plaintext_ok: record.plaintext_ok,
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
                // -- WP SW2 --
                system: None,
                system_usb_offsets: None,
                // -- WP P1HL --
                limits: StoredLimitsJson::default(),
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
            // -- WP SW2 --
            system: data.system,
            system_usb_offsets: usb_offsets(data.system_usb_offsets),
            // -- WP P1HL --
            limits: StoredLimitsJson {
                device_limits: data.device_limits,
                device_pins: data.device_pins,
                role_modes: data.role_modes,
                device_pin_tries: data.device_pin_tries, // -- WP P1DKFIX --
                device_pin_recovery: data.device_pin_recovery, // -- WP HUBADMIN --
            },
        })
    }

    // -- WP HB6 --
    /// What [`Self::load`] would find, for a settings page with no hub running. Creates
    /// nothing and moves nothing. `None`: no file, or one `load` would set aside, so
    /// nobody is paired either way. [`HubError::Io`]: the file is there but cannot be read.
    pub fn peek(dir: &Path) -> Result<Option<Self>, HubError> {
        let path = dir.join(FILE_NAME);
        let bytes = match read_limited(&path) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(io_error("cannot read", &path, &e)),
        };
        let Ok(data) = decode(&bytes) else {
            return Ok(None);
        };
        let clients: Vec<StoredClient> =
            data.clients.into_iter().filter_map(stored_client).collect();
        Ok(Some(Self {
            dir: dir.to_owned(),
            hub_id: data.hub_id,
            name: data.name,
            access: data.access,
            next_cid: data.next_cid,
            clients,
            device_names: data.device_names,
            // -- WP SW2 --
            system: data.system,
            system_usb_offsets: usb_offsets(data.system_usb_offsets),
            // -- WP P1HL --
            limits: StoredLimitsJson {
                device_limits: data.device_limits,
                device_pins: data.device_pins,
                role_modes: data.role_modes,
                device_pin_tries: data.device_pin_tries, // -- WP P1DKFIX --
                device_pin_recovery: data.device_pin_recovery, // -- WP HUBADMIN --
            },
        }))
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
                    // -- WP P2RS --
                    kind: (c.kind != ClientKind::Console).then(|| c.kind.wire_name().to_owned()),
                    plaintext_ok: c.plaintext_ok,
                })
                .collect(),
            device_names: self.device_names.clone(),
            // -- WP SW2 --
            system: self.system.clone(),
            system_usb_offsets: self
                .system_usb_offsets
                .as_ref()
                .and_then(|m| serde_json::to_value(m).ok()),
            // -- WP P1HL --
            device_limits: self.limits.device_limits.clone(),
            device_pins: self.limits.device_pins.clone(),
            role_modes: self.limits.role_modes.clone(),
            device_pin_tries: self.limits.device_pin_tries.clone(), // -- WP P1DKFIX --
            device_pin_recovery: self.limits.device_pin_recovery.clone(), // -- WP HUBADMIN --
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

    /// The names stored for devices, by serial.
    pub fn device_names(&self) -> &BTreeMap<String, String> {
        &self.device_names
    }

    /// Replace the device names, for [`Self::save`]. Cut to a size a file and a UI can
    /// hold: the serials come from what has been attached, and the names from clients.
    pub fn set_device_names(&mut self, names: &BTreeMap<String, String>) {
        self.device_names = names
            .iter()
            .take(MAX_DEVICE_NAMES)
            .map(|(serial, name)| {
                let name = name.chars().take(MAX_DEVICE_NAME).collect();
                (serial.clone(), name)
            })
            .collect();
    }

    // -- WP SW2 --
    /// The system model kept, if it is one this hub reads and it is valid. One
    /// that is not stays in the file, untouched, until a new one is set.
    pub fn system(&self) -> Option<crate::system::model::SystemModel> {
        let value = self.system.clone()?;
        match serde_json::from_value::<crate::system::model::SystemModel>(value) {
            Ok(m) if m.validate().is_ok() => Some(m),
            Ok(_) | Err(_) => {
                log::warn!("hub store: the system model is not one this hub reads; ignored");
                None
            }
        }
    }

    // -- WP AUSB2 --
    /// The USB-direct start offsets the hub last wrote, if kept.
    pub fn system_usb_offsets(&self) -> Option<&BTreeMap<String, i64>> {
        self.system_usb_offsets.as_ref()
    }

    // -- WP AUSB2 --
    /// Replace them, for [`Self::save`].
    pub fn set_system_usb_offsets(&mut self, offsets: &BTreeMap<String, i64>) {
        self.system_usb_offsets = Some(offsets.clone());
    }

    /// Replace the system model, for [`Self::save`].
    pub fn set_system(
        &mut self,
        model: &crate::system::model::SystemModel,
    ) -> Result<(), HubError> {
        let value = serde_json::to_value(model)
            .map_err(|e| HubError::Io(format!("cannot encode the system model: {e}")))?;
        self.system = Some(value);
        Ok(())
    }

    // -- WP P1HL --
    /// The limits profiles, PIN hashes and pinned modes kept: each entry that
    /// reads; one that does not is dropped alone, never the store.
    pub fn limits(&self) -> crate::limits::StoredLimits {
        let modes: BTreeMap<String, String> =
            entries_of(self.limits.role_modes.as_ref(), "role_modes");
        crate::limits::StoredLimits {
            profiles: entries_of(self.limits.device_limits.as_ref(), "device_limits"),
            pins: entries_of(self.limits.device_pins.as_ref(), "device_pins"),
            modes: modes
                .into_iter()
                .filter(|(r, m)| {
                    parse_role(r).is_some() && crate::limits::MODES.contains(&m.as_str())
                })
                .collect(),
            // -- WP P1DKFIX --
            tries: entries_of(self.limits.device_pin_tries.as_ref(), "device_pin_tries"),
            // -- WP HUBADMIN --
            recovery: entries_of(
                self.limits.device_pin_recovery.as_ref(),
                "device_pin_recovery",
            ),
        }
    }

    // -- WP P1HL --
    /// Replace them, for [`Self::save`].
    pub fn set_limits(&mut self, data: &crate::limits::StoredLimits) {
        let map = |v: serde_json::Result<serde_json::Value>| {
            v.ok()
                .filter(|v| v.as_object().is_some_and(|m| !m.is_empty()))
        };
        self.limits = StoredLimitsJson {
            device_limits: map(serde_json::to_value(&data.profiles)),
            device_pins: map(serde_json::to_value(&data.pins)),
            role_modes: map(serde_json::to_value(&data.modes)),
            device_pin_tries: map(serde_json::to_value(&data.tries)), // -- WP P1DKFIX --
            device_pin_recovery: map(serde_json::to_value(&data.recovery)), // -- WP HUBADMIN --
        };
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
            kind: ClientKind::Console,
            plaintext_ok: false,
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
        a.set_device_names(&BTreeMap::from([("E46".into(), "Living room".into())]));
        a.save().unwrap();
        let mut b = HubStore::load(&dir).unwrap();
        assert_eq!(b.clients(), &[client(cid)]);
        assert_eq!(b.name(), Some("Studio"));
        assert_eq!(b.device_names()["E46"], "Living room");
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

    // -- fixup ausb2 (F3) -- review ausb2's probe D
    /// An odd `system_usb_offsets` entry is dropped alone: the clients, the hub
    /// id and the system model stay, and nothing is set aside.
    #[test]
    fn wp_ausb2_an_odd_usb_offsets_entry_is_dropped_alone() {
        for (label, offsets, kept) in [
            (
                "string value",
                r#"{"U1": "abc", "U2": 48}"#,
                Some(vec![("U2", 48)]),
            ),
            ("float value", r#"{"U1": 1.5}"#, Some(vec![])),
            ("null value", r#"{"U1": null}"#, Some(vec![])),
            ("list", r#"[1, 2]"#, None),
            ("good", r#"{"U1": 96}"#, Some(vec![("U1", 96)])),
        ] {
            let dir = scratch(&format!("offsets-{}", label.replace(' ', "-")));
            fs::create_dir_all(&dir).unwrap();
            let good = "ab".repeat(32);
            let text = format!(
                r#"{{"hub_id":"11111111-2222-3333-4444-555555555555","clients":[
                  {{"cid":9,"name":"good","role":"viewer","created":0,"token_sha256":"{good}"}}
                ],
                "system": {{"rooms": [{{"id": "main", "name": "Main room"}}]}},
                "system_usb_offsets": {offsets}}}"#
            );
            fs::write(dir.join(FILE_NAME), text).unwrap();
            let s = HubStore::load(&dir).unwrap();
            assert_eq!(
                s.hub_id(),
                "11111111-2222-3333-4444-555555555555",
                "{label}"
            );
            assert!(s.system().is_some(), "{label}");
            let want: Option<BTreeMap<String, i64>> =
                kept.map(|k| k.into_iter().map(|(a, b)| (a.to_owned(), b)).collect());
            assert_eq!(s.system_usb_offsets(), want.as_ref(), "{label}");
            let bad = fs::read_dir(&dir)
                .unwrap()
                .filter(|e| {
                    e.as_ref()
                        .unwrap()
                        .file_name()
                        .to_string_lossy()
                        .contains(".bad-")
                })
                .count();
            assert_eq!(bad, 0, "{label}: nothing set aside");
            s.save().unwrap();
            assert_eq!(
                HubStore::load(&dir).unwrap().system_usb_offsets(),
                want.as_ref()
            );
        }
    }

    // -- WP SW2 --
    /// A system model this hub cannot read costs nothing else, and stays in the
    /// file until a new one is set.
    #[test]
    fn wp_sw2_an_unreadable_system_model_is_ignored_and_kept() {
        let dir = scratch("system");
        fs::create_dir_all(&dir).unwrap();
        let text = r#"{"hub_id":"x","device_names":{"E46":"Den"},"system":{"units":7}}"#;
        fs::write(dir.join(FILE_NAME), text).unwrap();
        let mut s = HubStore::load(&dir).unwrap();
        assert_eq!(s.device_names()["E46"], "Den");
        assert!(s.system().is_none());
        s.save().unwrap();
        let kept = fs::read_to_string(dir.join(FILE_NAME)).unwrap();
        assert!(kept.contains("\"units\": 7"), "{kept}");
        let model = crate::system::model::SystemModel::default();
        s.set_system(&model).unwrap();
        s.save().unwrap();
        assert_eq!(HubStore::load(&dir).unwrap().system(), Some(model));
    }

    #[test]
    fn wp_hb1_device_names_are_capped() {
        let dir = scratch("caps");
        let mut s = HubStore::load(&dir).unwrap();
        let mut names: BTreeMap<String, String> = (0..MAX_DEVICE_NAMES * 2)
            .map(|n| (format!("s{n:05}"), "n".to_owned()))
            .collect();
        names.insert("a".into(), "x".repeat(1000));
        s.set_device_names(&names);
        assert_eq!(s.device_names().len(), MAX_DEVICE_NAMES);
        names.retain(|k, _| k == "a");
        s.set_device_names(&names);
        assert_eq!(s.device_names()["a"].len(), MAX_DEVICE_NAME);
    }
}
