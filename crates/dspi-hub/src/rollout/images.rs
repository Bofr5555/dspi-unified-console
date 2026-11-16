//! The rollout's image store (WP P3RO, `hub-api.md` section 21.2): at most
//! [`MAX_IMAGES`] firmware images of at most [`MAX_IMAGE_BYTES`] each in
//! `<data>/firmware/`, with an index (`index.json`) of what each one is.
//!
//! An image is checked whole before it is kept: `uf2_inspect` must find it valid and for
//! a chip the hub knows, and a file presenting itself as a bundled image with other bytes
//! (`verify_bundled`'s mismatch) is refused. `verified` is the manifest's say (SECFWV):
//! the name and the bytes of a release's own image. The index is read from disk for every
//! operation, so an image put in from the hub's machine (`uconsole hub firmware add`,
//! P3TM, through [`ImageStore::add`]) is seen by a running hub; when an image is used its
//! bytes are read again and must still hash to the index's SHA-256.
//!
//! Each file is written atomically and is 0600 on Unix. An index that does not read whole
//! is kept aside as `index.json.bad-<time>` and rebuilt from the images in the folder. No
//! image byte is ever logged, and `Debug` of an image shows its facts only.

use std::fs::{self, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, PoisonError};

use dspi_core::{
    BundledFirmware, FirmwareGuardBoard, FirmwareGuardBoardSource, firmware_guard_check,
    uf2_inspect, verify_bundled,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::inventory::iso8601;

/// The folder under the hub's data folder.
pub const DIR_NAME: &str = "firmware";
/// The index's file in it.
pub const INDEX_FILE: &str = "index.json";
/// Images kept at most.
pub const MAX_IMAGES: usize = 4;
/// The largest image, bytes (8 MiB).
pub const MAX_IMAGE_BYTES: usize = 8 * 1024 * 1024;
/// The longest image name.
pub const MAX_NAME_CHARS: usize = 64;
/// The index is never longer.
const MAX_INDEX_BYTES: u64 = 64 * 1024;
/// `added_by` is cut to this.
const MAX_BY_CHARS: usize = 64;

/// Why the store refused: a code of `hub-api.md` 1.6 and a line for people.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageError {
    /// The `code`.
    pub code: &'static str,
    /// The `msg`.
    pub msg: String,
}

impl ImageError {
    fn new(code: &'static str, msg: impl Into<String>) -> Self {
        Self {
            code,
            msg: msg.into(),
        }
    }

    fn io(e: &io::Error) -> Self {
        log::error!("hub rollout: the image store: {e}");
        Self::new("internal", "the image store could not be written")
    }
}

/// One stored image, as `fw.image.list` answers it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImageInfo {
    /// As uploaded.
    pub name: String,
    /// Bytes.
    pub size: u64,
    /// Of the whole file, lowercase hex.
    pub sha256: String,
    /// The DSPi version `uf2_inspect` finds in it ("" when none).
    pub firmware_version: String,
    /// `RP2040` or `RP2350`.
    pub platform: String,
    /// The core-voltage stamp the firmware guard reads, mV; `None` when unstamped.
    pub stamp_mv: Option<u16>,
    /// The release's manifest lists this name with these bytes.
    pub verified: bool,
    /// When, ISO-8601.
    pub added_at: String,
    /// Who: the client's name, or `Hub console`.
    pub added_by: String,
}

impl ImageInfo {
    /// `0` RP2040, `1` RP2350 (`Uf2Info::platform`).
    pub fn platform_id(&self) -> i8 {
        match self.platform.as_str() {
            "RP2040" => 0,
            "RP2350" => 1,
            _ => -1,
        }
    }

    /// The object of `fw.image.list` and `fw.image.put`.
    pub fn to_json(&self) -> Value {
        serde_json::to_value(self).unwrap_or_else(|_| json!({"name": self.name}))
    }
}

/// What [`ImageStore::add`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Added {
    /// The image as stored.
    pub image: ImageInfo,
    /// False when the store already held this name with these bytes.
    pub created: bool,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct Index {
    #[serde(default)]
    v: u8,
    #[serde(default)]
    images: Vec<ImageInfo>,
}

/// Whether `name` may name an image: 1 to 64 characters of `A-Z a-z 0-9 . - _ +`, ending
/// in `.uf2`, not starting with a dot (so never `index.json`, never a path).
pub fn valid_name(name: &str) -> bool {
    let len = name.chars().count();
    (5..=MAX_NAME_CHARS).contains(&len)
        && name.ends_with(".uf2")
        && !name.starts_with('.')
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | '+'))
}

/// SHA-256, lowercase hex.
pub fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    // Every section under it leaves the files whole before it could panic.
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Write `bytes` to `path` atomically (a temporary file, then a rename), 0600 on Unix.
pub(crate) fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    let mut o = OpenOptions::new();
    o.write(true).create(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut o, 0o600);
    {
        let mut f = o.open(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
    }
    fs::rename(&tmp, path).inspect_err(|_| {
        let _ = fs::remove_file(&tmp);
    })
}

/// At most `max` bytes of `path`; `Ok(None)` when it is longer.
pub(crate) fn read_limited(path: &Path, max: u64) -> io::Result<Option<Vec<u8>>> {
    let mut out = Vec::new();
    fs::File::open(path)?
        .take(max.saturating_add(1))
        .read_to_end(&mut out)?;
    Ok((u64::try_from(out.len()).unwrap_or(u64::MAX) <= max).then_some(out))
}

/// Keep `path` aside as `<path>.bad-<now_s>`.
pub(crate) fn set_aside(path: &Path, now_s: u64) {
    let mut aside = path.as_os_str().to_owned();
    aside.push(format!(".bad-{now_s}"));
    if let Err(e) = fs::rename(path, PathBuf::from(&aside)) {
        log::error!("hub rollout: cannot keep {} aside: {e}", path.display());
    }
}

/// The facts of `bytes` as an image named `name`, or why it is no image.
pub fn inspect(
    name: &str,
    bytes: &[u8],
    added_at: String,
    added_by: &str,
) -> Result<ImageInfo, ImageError> {
    if !valid_name(name) {
        return Err(ImageError::new(
            "bad_request",
            "an image's name is 1 to 64 characters of A-Z a-z 0-9 . - _ + ending in .uf2",
        ));
    }
    if bytes.len() > MAX_IMAGE_BYTES {
        return Err(ImageError::new(
            "bad_request",
            format!("an image is at most {MAX_IMAGE_BYTES} bytes"),
        ));
    }
    let info = uf2_inspect(bytes.to_vec());
    if !info.valid {
        return Err(ImageError::new(
            "bad_request",
            format!("This is not a firmware file: {}", info.problem),
        ));
    }
    let platform = match info.platform {
        0 => "RP2040",
        1 => "RP2350",
        _ => {
            return Err(ImageError::new(
                "bad_request",
                "the image is not for a chip this hub flashes (RP2040 or RP2350)",
            ));
        }
    };
    let verified = match verify_bundled(bytes.to_vec(), name.to_owned()) {
        BundledFirmware::Verified => true,
        BundledFirmware::NotBundled => false,
        BundledFirmware::Mismatch => {
            return Err(ImageError::new(
                "bad_request",
                format!(
                    "{name} is the name of a bundled image, but these are not its bytes: the \
                     file may be damaged"
                ),
            ));
        }
    };
    // The guard reads the stamp; 1150 on a stock board never asks for a stamp's sake.
    let verdict = firmware_guard_check(
        bytes.to_vec(),
        FirmwareGuardBoard {
            required_mv: 1150,
            source: FirmwareGuardBoardSource::ExtCoreVreg,
        },
        String::new(),
    );
    Ok(ImageInfo {
        name: name.to_owned(),
        size: u64::try_from(bytes.len()).unwrap_or(u64::MAX),
        sha256: sha256_hex(bytes),
        firmware_version: info.firmware_version,
        platform: platform.to_owned(),
        stamp_mv: (verdict.image_mv != 0).then_some(verdict.image_mv),
        verified,
        added_at,
        added_by: added_by.trim().chars().take(MAX_BY_CHARS).collect(),
    })
}

/// The store in `<data>/firmware/`.
#[derive(Debug)]
pub struct ImageStore {
    dir: PathBuf,
    /// Held across each read-modify-write of the index, in this process.
    gate: Mutex<()>,
}

impl ImageStore {
    /// The store of the hub whose data folder is `data_dir`. Reads the index once, keeps
    /// aside a torn one and drops entries whose file is gone or changed.
    pub fn open(data_dir: &Path, now_s: u64) -> Self {
        let store = Self {
            dir: data_dir.join(DIR_NAME),
            gate: Mutex::new(()),
        };
        {
            let _g = lock(&store.gate);
            let (index, whole) = store.read_index(now_s);
            let kept: Vec<ImageInfo> = index
                .images
                .iter()
                .filter(|i| store.intact(i))
                .cloned()
                .collect();
            if !whole || kept.len() != index.images.len() {
                let _ = store.write_index(&kept);
            }
        }
        store
    }

    /// The folder.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn index_path(&self) -> PathBuf {
        self.dir.join(INDEX_FILE)
    }

    fn file(&self, name: &str) -> PathBuf {
        self.dir.join(name)
    }

    /// The image's file is there and still hashes to its index entry.
    fn intact(&self, i: &ImageInfo) -> bool {
        if !valid_name(&i.name) {
            return false;
        }
        match read_limited(&self.file(&i.name), MAX_IMAGE_BYTES as u64) {
            Ok(Some(bytes)) => sha256_hex(&bytes) == i.sha256,
            _ => false,
        }
    }

    /// The index, and whether it read whole. A missing one is empty and whole; a torn
    /// one is kept aside and rebuilt from the folder's images.
    fn read_index(&self, now_s: u64) -> (Index, bool) {
        let path = self.index_path();
        let bytes = match read_limited(&path, MAX_INDEX_BYTES) {
            Ok(Some(b)) => b,
            Ok(None) => {
                log::error!("hub rollout: {} is too long", path.display());
                set_aside(&path, now_s);
                return (self.rebuild(now_s), false);
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => return (Index::default(), true),
            Err(e) => {
                log::error!("hub rollout: cannot read {}: {e}", path.display());
                return (Index::default(), false);
            }
        };
        match serde_json::from_slice::<Index>(&bytes) {
            Ok(mut index) => {
                let before = index.images.len();
                index.images.truncate(MAX_IMAGES);
                let whole = before == index.images.len();
                (index, whole)
            }
            Err(e) => {
                log::error!(
                    "hub rollout: {} does not read ({e}); kept aside",
                    path.display()
                );
                set_aside(&path, now_s);
                (self.rebuild(now_s), false)
            }
        }
    }

    /// An index of the folder's images (at most [`MAX_IMAGES`], by name).
    fn rebuild(&self, now_s: u64) -> Index {
        let mut names: Vec<String> = fs::read_dir(&self.dir)
            .map(|rd| {
                rd.flatten()
                    .filter_map(|e| e.file_name().to_str().map(str::to_owned))
                    .filter(|n| valid_name(n))
                    .collect()
            })
            .unwrap_or_default();
        names.sort();
        let mut images = Vec::new();
        for name in names {
            if images.len() >= MAX_IMAGES {
                break;
            }
            if let Ok(Some(bytes)) = read_limited(&self.file(&name), MAX_IMAGE_BYTES as u64)
                && let Ok(info) = inspect(&name, &bytes, iso8601(now_s), "unknown")
            {
                images.push(info);
            }
        }
        Index { v: 1, images }
    }

    fn write_index(&self, images: &[ImageInfo]) -> Result<(), ImageError> {
        let text = serde_json::to_string_pretty(&json!({"v": 1, "images": images}))
            .map_err(|e| ImageError::new("internal", e.to_string()))?;
        write_atomic(&self.index_path(), text.as_bytes()).map_err(|e| ImageError::io(&e))
    }

    /// Every image, oldest first.
    pub fn list(&self, now_s: u64) -> Vec<ImageInfo> {
        let _g = lock(&self.gate);
        self.read_index(now_s).0.images
    }

    /// One image's facts and bytes, read again from disk: `not_found` for an unknown
    /// name, `image_refused` (reason `image_invalid`) for a file that changed.
    pub fn load(&self, name: &str, now_s: u64) -> Result<(ImageInfo, Vec<u8>), ImageError> {
        let _g = lock(&self.gate);
        let info = self
            .read_index(now_s)
            .0
            .images
            .into_iter()
            .find(|i| i.name == name)
            .ok_or_else(|| ImageError::new("not_found", format!("no image {name} is stored")))?;
        let changed = || {
            ImageError::new(
                "image_refused",
                format!("the stored image {name} changed on disk; delete it and add it again"),
            )
        };
        let bytes = read_limited(&self.file(name), MAX_IMAGE_BYTES as u64)
            .map_err(|_| changed())?
            .ok_or_else(changed)?;
        if sha256_hex(&bytes) != info.sha256 {
            return Err(changed());
        }
        Ok((info, bytes))
    }

    /// Keep `bytes` as image `name` (module docs). The same name with the same bytes is
    /// `created: false`; with other bytes `conflict`; a fifth image `conflict`.
    pub fn add(&self, name: &str, bytes: &[u8], by: &str, now_s: u64) -> Result<Added, ImageError> {
        let image = inspect(name, bytes, iso8601(now_s), by)?;
        let _g = lock(&self.gate);
        let mut images = self.read_index(now_s).0.images;
        if let Some(old) = images.iter().find(|i| i.name == name) {
            if old.sha256 == image.sha256 {
                return Ok(Added {
                    image: old.clone(),
                    created: false,
                });
            }
            return Err(ImageError::new(
                "conflict",
                format!("an image {name} with other bytes is stored; delete it first"),
            ));
        }
        if images.len() >= MAX_IMAGES {
            return Err(ImageError::new(
                "conflict",
                format!("the hub keeps {MAX_IMAGES} images; delete one first"),
            ));
        }
        write_atomic(&self.file(name), bytes).map_err(|e| ImageError::io(&e))?;
        images.push(image.clone());
        if let Err(e) = self.write_index(&images) {
            let _ = fs::remove_file(self.file(name));
            return Err(e);
        }
        log::info!(
            "hub rollout: image {name} added ({} bytes, sha256 {}, {})",
            image.size,
            image.sha256,
            if image.verified {
                "verified"
            } else {
                "not verified"
            }
        );
        Ok(Added {
            image,
            created: true,
        })
    }

    /// Remove image `name`: `not_found` when there is none.
    pub fn delete(&self, name: &str, now_s: u64) -> Result<(), ImageError> {
        let _g = lock(&self.gate);
        let mut images = self.read_index(now_s).0.images;
        let before = images.len();
        images.retain(|i| i.name != name);
        if images.len() == before {
            return Err(ImageError::new(
                "not_found",
                format!("no image {name} is stored"),
            ));
        }
        self.write_index(&images)?;
        if let Err(e) = fs::remove_file(self.file(name))
            && e.kind() != io::ErrorKind::NotFound
        {
            log::warn!("hub rollout: image {name}: its file stays: {e}");
        }
        Ok(())
    }
}

/// Put `bytes` into the image store of the hub whose data folder is `data_dir`, from the
/// hub's machine (`uconsole hub firmware add FILE`, P3TM): the same checks as an upload,
/// `by` [`crate::service::LOCAL_LABEL`].
pub fn add_file(
    data_dir: &Path,
    name: &str,
    bytes: &[u8],
    now_s: u64,
) -> Result<Added, ImageError> {
    ImageStore::open(data_dir, now_s).add(name, bytes, crate::service::LOCAL_LABEL, now_s)
}
