//! The bundled-firmware manifest: is this the image the app ships, unchanged?
//! (WP SECFWV, docs/core-merge/PLAN-SEC.md section 4.10, OD-SECn.)
//!
//! `firmware/manifest.json` lists the SHA-256, size, chip and version of every
//! `.uf2` the apps bundle. `tools/firmware_manifest.py` writes it from the
//! digests the Android supply-chain test pins, which were checked against the
//! digests GitHub publishes for the release; it is compiled in here.
//!
//! # What it is, and what it is not
//!
//! **A consistency check, not tamper protection.** The manifest ships inside
//! the same bundle as the images (here: inside the same library), so whoever
//! can swap a `.uf2` in an installed app can swap the library too. What it
//! catches is a build that packed the wrong file under a bundled name, and a
//! copy damaged on the way. The real controls are the platforms' code
//! signing and the CI digest test (`ci/check.sh --step firmware-digests`),
//! which holds the Android, Qt and Apple bundles to the pinned digests. If
//! firmware downloads are ever added, the manifest must be signed with the
//! release key and verified here first; until then nothing about it is secret
//! and the comparison need not be constant-time.
//!
//! # The two questions
//!
//! - [`verify_bundled`]: the client says which bundled file it means, by
//!   name. The answer is [`BundledFirmware::Verified`],
//!   [`BundledFirmware::Mismatch`] (a bundled name whose bytes differ) or
//!   [`BundledFirmware::NotBundled`] (any other name: a file the user picked,
//!   for the "not a verified DSPi Console image" label).
//! - Before every flash, the guard ([`crate::firmware_guard_check`], so also
//!   [`crate::DspiCore::flash_confirmed`]) asks without a name, since the
//!   flash path carries none: an image that presents itself as a bundled one
//!   (the same chip, the same DSPi version string and the same size) but
//!   whose hash differs is refused. Anything else is not judged here, and a
//!   user's own file goes on to the existing confirmations.
//!
//! The version string includes the build's commit (`1.1.6-1-g9ffce34`), so
//! only a user's own build of that very commit, at the very same size, could be
//! taken for a damaged bundled copy and refused; the message then says to use
//! the bundled copy, which is that same release.

use std::sync::OnceLock;

use sha2::{Digest, Sha256};

use crate::FirmwareGuardVerdict;
use crate::records::Uf2Info;

/// The manifest, as `tools/firmware_manifest.py` writes it.
const MANIFEST_JSON: &str = include_str!("../../../firmware/manifest.json");

// -- WP SECFIX --
/// The notice row of the label every client shows for a firmware file that is
/// not [`BundledFirmware::Verified`] (a file the user picked): its title is
/// the label, "Not a verified DSPi Console image"; the reason and action say
/// what that means. Information, not a refusal.
pub const NOTICE_NOT_VERIFIED: &str = "firmware.not_verified";

/// [`verify_bundled`]'s answer: is this the firmware image the app bundles?
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "camelCase")]
pub enum BundledFirmware {
    /// A bundled name, and the bytes are the ones the manifest lists.
    Verified,
    /// Not a bundled name: label it "not a verified DSPi Console image" and
    /// keep the usual confirmations. Not a refusal.
    #[default]
    NotBundled,
    /// A bundled name (or, before a flash, a bundled image's identity) with
    /// other bytes: a mixed-up or damaged copy. Flashing it is refused.
    Mismatch,
}

#[derive(serde::Deserialize)]
struct Manifest {
    images: Vec<Entry>,
}

#[derive(serde::Deserialize)]
struct Entry {
    name: String,
    platform: String,
    version: String,
    size: u64,
    sha256: String,
    // -- WP PGFWUP -- (PLAN-PG section 1.8) optional: the bundles are upstream's
    // releases, and `tools/firmware_manifest.py` writes neither field yet
    /// `upstream` (the default) or `bofr`.
    #[serde(default)]
    lineage: Option<String>,
    /// `legacy` or `paged`; by default from the version (1.1.7 and later are
    /// paged).
    #[serde(default)]
    generation: Option<String>,
    /// The PAGEGUARD stamp: the `bofr-main` commit whose guard the image
    /// contains (`df53ede` or a later one), as hex. Required of a legacy bofr
    /// image (R6d).
    #[serde(default)]
    page_guard: Option<String>,
    /// Filled from the fields above when the manifest is read: the lineage and
    /// generation, or why the entry is refused (never offered, never flashed).
    #[serde(skip, default = "unjudged")]
    judged: Judged,
}

// -- WP PGFWUP --
/// Where a firmware comes from (PLAN-PG section 1.8): upstream Weeb Labs, or
/// this fork's `bofr-main` (extensions, alink, the 1.20 V core setting, our
/// preset directory).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum FirmwareLineage {
    /// Not known: every doubt resolves to "asks".
    #[default]
    Unknown,
    /// Weeb Labs' firmware (stock).
    Upstream,
    /// This fork's `bofr-main`.
    Bofr,
}

/// Which protocol generation a firmware speaks (PLAN-PG section 1.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum FirmwareGeneration {
    /// Not known.
    #[default]
    Unknown,
    /// 1.1.6 and older: page 0 only.
    Legacy,
    /// 1.1.7 and later: paged.
    Paged,
}

impl FirmwareGeneration {
    /// The generation a DSPi version string names: 1.1.7 and later (betas
    /// included) are paged, anything older legacy, an unreadable one unknown.
    pub fn of_version(version: &str) -> Self {
        match crate::hardware::parse_version(version) {
            Some((major, minor, patch, _)) if (major, minor, patch) >= (1, 1, 7) => Self::Paged,
            Some(_) => Self::Legacy,
            None => Self::Unknown,
        }
    }
}

/// One image the manifest offers ([`bundled_images`]).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BundledImage {
    /// The bundled file's name.
    pub name: String,
    /// `RP2040` or `RP2350`.
    pub platform: String,
    /// The DSPi version string inside the image.
    pub version: String,
    /// Whose firmware it is.
    pub lineage: FirmwareLineage,
    /// Which protocol generation it speaks.
    pub generation: FirmwareGeneration,
    /// The PAGEGUARD stamp of a legacy bofr image, else `None`.
    pub page_guard: Option<String>,
}

/// An entry's lineage and generation, or why it is refused.
type Judged = std::result::Result<(FirmwareLineage, FirmwareGeneration), String>;

fn unjudged() -> Judged {
    Err("not judged".into())
}

/// The rule an entry meets to be offered: a known lineage and generation, and
/// on a legacy bofr image the PAGEGUARD stamp (a host-supplied MIDI pipe can
/// outlive a re-flash, and a stale paged wrapper would then send paged requests
/// to a downgraded board that only its guard can refuse, R6d).
fn judge_entry(e: &Entry) -> Judged {
    let lineage = match e.lineage.as_deref() {
        None | Some("upstream") => FirmwareLineage::Upstream,
        Some("bofr") => FirmwareLineage::Bofr,
        Some(other) => return Err(format!("{}: unknown lineage {other:?}", e.name)),
    };
    let generation = match e.generation.as_deref() {
        None => FirmwareGeneration::of_version(&e.version),
        Some("legacy") => FirmwareGeneration::Legacy,
        Some("paged") => FirmwareGeneration::Paged,
        Some(other) => return Err(format!("{}: unknown generation {other:?}", e.name)),
    };
    if generation == FirmwareGeneration::Unknown {
        return Err(format!(
            "{}: the version {:?} names no generation",
            e.name, e.version
        ));
    }
    let stamped = e
        .page_guard
        .as_deref()
        .is_some_and(|c| (7..=40).contains(&c.len()) && c.bytes().all(|b| b.is_ascii_hexdigit()));
    if lineage == FirmwareLineage::Bofr && generation == FirmwareGeneration::Legacy && !stamped {
        return Err(format!(
            "{} is a 1.1.6-generation bofr image without the PAGEGUARD stamp: it is never \
             offered (PLAN-PG R6d)",
            e.name
        ));
    }
    Ok((lineage, generation))
}

/// The manifest `text`, each entry judged. One that does not parse gives
/// none.
fn parse(text: &str) -> Vec<Entry> {
    match serde_json::from_str::<Manifest>(text) {
        Ok(m) => m
            .images
            .into_iter()
            .map(|mut e| {
                e.sha256.make_ascii_lowercase();
                e.judged = judge_entry(&e);
                if let Err(why) = &e.judged {
                    log::error!("firmware manifest: {why}");
                }
                e
            })
            .collect(),
        Err(e) => {
            log::error!("firmware manifest: unreadable ({e}); no image counts as bundled");
            Vec::new()
        }
    }
}

/// The compiled-in manifest. A manifest that does not parse (a test fails
/// first) gives none, so every image is `NotBundled`: the existing checks
/// still run, and nothing is refused on the manifest's account.
fn manifest() -> &'static [Entry] {
    static M: OnceLock<Vec<Entry>> = OnceLock::new();
    M.get_or_init(|| parse(MANIFEST_JSON))
}

// -- WP PGFWUP --
/// The images the manifest offers, with their lineage and generation. An entry
/// the manifest refuses (a legacy bofr image without the PAGEGUARD stamp, an
/// unknown lineage or generation) is not among them.
pub fn bundled_images() -> Vec<BundledImage> {
    offered(manifest())
}

fn offered(all: &[Entry]) -> Vec<BundledImage> {
    all.iter()
        .filter_map(|e| {
            let (lineage, generation) = e.judged.clone().ok()?;
            Some(BundledImage {
                name: e.name.clone(),
                platform: e.platform.clone(),
                version: e.version.clone(),
                lineage,
                generation,
                page_guard: e.page_guard.clone().filter(|_| {
                    lineage == FirmwareLineage::Bofr && generation == FirmwareGeneration::Legacy
                }),
            })
        })
        .collect()
}

/// The lineage and generation of `image` when it is a bundled image the
/// manifest offers (by its hash), else `None`.
pub(crate) fn bundled_lineage(image: &[u8]) -> Option<(FirmwareLineage, FirmwareGeneration)> {
    let hash = sha256_hex(image);
    manifest()
        .iter()
        .find(|e| e.sha256 == hash)
        .and_then(|e| e.judged.clone().ok())
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest: [u8; 32] = Sha256::digest(bytes).into();
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

/// The last path component, so `Firmware/DSPi-RP2350-v1.1.6.uf2` and a
/// Windows path name the same file.
fn base_name(name: &str) -> &str {
    name.rsplit(['/', '\\']).next().unwrap_or(name)
}

/// Whether `image` is the bundled file `name` (a file name; a leading path
/// is ignored), byte for byte. Pass the bundled name for a bundled image, and
/// the picked file's name, or "", for one the user chose. A consistency check
/// against build mix-ups and corruption, not tamper protection: the manifest
/// ships in the same bundle as the images. Pure; hashes the image once.
#[cfg_attr(feature = "uniffi", uniffi::export)]
pub fn verify_bundled(image: Vec<u8>, name: String) -> BundledFirmware {
    verify_in(manifest(), &image, &name)
}

fn verify_in(all: &[Entry], image: &[u8], name: &str) -> BundledFirmware {
    let name = base_name(name);
    match all.iter().find(|e| e.name == name) {
        None => BundledFirmware::NotBundled,
        // -- WP PGFWUP -- an entry the manifest refuses is never flashed
        Some(e) if e.judged.is_err() => BundledFirmware::Mismatch,
        Some(e) if e.sha256 == sha256_hex(image) => BundledFirmware::Verified,
        Some(_) => BundledFirmware::Mismatch,
    }
}

fn platform_name(platform: i8) -> &'static str {
    match platform {
        0 => "RP2040",
        1 => "RP2350",
        _ => "",
    }
}

/// The guard's question, without a name: `Mismatch` for an image that
/// presents itself as a bundled one (chip, version string and size) with
/// other bytes, `Verified` for bundled bytes, else `NotBundled`.
fn judge<'a>(
    all: &'a [Entry],
    image: &[u8],
    info: &Uf2Info,
) -> (BundledFirmware, Option<&'a Entry>) {
    let hash = sha256_hex(image);
    if let Some(e) = all.iter().find(|e| e.sha256 == hash) {
        // -- WP PGFWUP -- the bytes of an entry the manifest refuses
        if e.judged.is_err() {
            return (BundledFirmware::Mismatch, Some(e));
        }
        return (BundledFirmware::Verified, None);
    }
    let size = u64::try_from(image.len()).unwrap_or(u64::MAX);
    let chip = platform_name(info.platform);
    match all.iter().find(|e| {
        !chip.is_empty()
            && e.platform == chip
            && e.version == info.firmware_version
            && e.size == size
    }) {
        Some(e) => (BundledFirmware::Mismatch, Some(e)),
        None => (BundledFirmware::NotBundled, None),
    }
}

/// For [`crate::firmware_guard_check`], on a valid image: the refusal when it
/// is a damaged or mixed-up copy of a bundled image, else `None`.
pub(crate) fn refusal(
    image: &[u8],
    info: &Uf2Info,
    required_mv: u16,
) -> Option<FirmwareGuardVerdict> {
    refusal_in(manifest(), image, info, required_mv)
}

fn refusal_in(
    all: &[Entry],
    image: &[u8],
    info: &Uf2Info,
    required_mv: u16,
) -> Option<FirmwareGuardVerdict> {
    match judge(all, image, info) {
        // -- WP PGFWUP --
        (BundledFirmware::Mismatch, Some(e)) if e.judged.is_err() => Some(FirmwareGuardVerdict {
            message: format!(
                "{} is not offered: {}",
                e.name,
                e.judged.as_ref().err().map_or("", String::as_str)
            ),
            required_mv,
            ..FirmwareGuardVerdict::default()
        }),
        (BundledFirmware::Mismatch, Some(e)) => Some(FirmwareGuardVerdict {
            message: format!(
                "This file looks like the bundled {} but is not the same file: it may be damaged \
                 or mixed up. Flash the copy that comes with the app instead.",
                e.name
            ),
            required_mv,
            ..FirmwareGuardVerdict::default()
        }),
        _ => None,
    }
}

#[cfg(test)]
mod tests_wp_secfwv {
    #![allow(clippy::arithmetic_side_effects)]

    use super::*;
    use crate::records::BootloaderDrive;
    use crate::uf2_inspect;
    use crate::{
        DspiError, FirmwareGuardBoard, FirmwareGuardBoardSource, FirmwareGuardDecision,
        firmware_guard_check, flash_to_drive_guarded,
    };
    use std::path::{Path, PathBuf};

    fn judge_for_test(image: &[u8]) -> BundledFirmware {
        judge(manifest(), image, &uf2_inspect(image.to_vec())).0
    }

    /// Every app's bundle directory, from the repository root.
    const BUNDLES: [&str; 3] = [
        "apps/android/feature-firmware/src/main/assets/firmware",
        "apps/desktop/qt/firmware",
        "apps/apple/DSPi Console/Firmware",
    ];

    fn root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
    }

    fn bundled(name: &str) -> Vec<u8> {
        std::fs::read(root().join(BUNDLES[1]).join(name)).unwrap()
    }

    const RP2350: &str = "DSPi-RP2350-v1.1.6.uf2";

    /// A copy with one byte flipped in the middle of a block's payload, so
    /// it is still a valid UF2 with the same version string.
    fn flipped(mut image: Vec<u8>) -> Vec<u8> {
        let block = image.len() / 512 / 2;
        image[block * 512 + 32 + 100] ^= 0x01;
        image
    }

    fn stock_board() -> FirmwareGuardBoard {
        FirmwareGuardBoard {
            required_mv: 1150,
            source: FirmwareGuardBoardSource::StatusBlock,
        }
    }

    fn drive(name: &str) -> (PathBuf, BootloaderDrive) {
        let dir = std::env::current_exe()
            .unwrap()
            .parent()
            .unwrap()
            .join(format!("wp-secfwv-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("INFO_UF2.TXT"),
            "UF2 Bootloader v1.0\nModel: Raspberry Pi RP2350\nBoard-ID: RP2350\n",
        )
        .unwrap();
        let d = BootloaderDrive {
            path: dir.to_string_lossy().into_owned(),
            platform: 1,
            info: String::new(),
        };
        (dir, d)
    }

    #[test]
    fn wp_secfwv_the_manifest_matches_every_bundled_image_in_all_three_apps() {
        let entries = manifest();
        assert!(entries.len() >= 2, "the manifest did not parse");
        for bundle in BUNDLES {
            let dir = root().join(bundle);
            let mut seen = 0;
            for e in std::fs::read_dir(&dir).unwrap().filter_map(|e| e.ok()) {
                let name = e.file_name().to_string_lossy().into_owned();
                if !name.ends_with(".uf2") {
                    continue;
                }
                let image = std::fs::read(e.path()).unwrap();
                assert_eq!(
                    verify_bundled(image.clone(), name.clone()),
                    BundledFirmware::Verified,
                    "{bundle}/{name}"
                );
                // The identity fields are the image's own.
                let entry = entries.iter().find(|x| x.name == name).unwrap();
                let info = uf2_inspect(image.clone());
                assert!(info.valid, "{name}: {}", info.problem);
                assert_eq!(platform_name(info.platform), entry.platform, "{name}");
                assert_eq!(info.firmware_version, entry.version, "{name}");
                assert_eq!(image.len() as u64, entry.size, "{name}");
                assert_eq!(judge_for_test(&image), BundledFirmware::Verified);
                seen += 1;
            }
            assert_eq!(seen, entries.len(), "{bundle}: one file per manifest entry");
        }
    }

    #[test]
    fn wp_secfwv_a_flipped_byte_is_a_mismatch_and_refused() {
        let bad = flipped(bundled(RP2350));
        assert!(uf2_inspect(bad.clone()).valid, "still a valid UF2");
        assert_eq!(
            verify_bundled(bad.clone(), RP2350.into()),
            BundledFirmware::Mismatch
        );
        assert_eq!(
            verify_bundled(bad.clone(), format!("Firmware/{RP2350}")),
            BundledFirmware::Mismatch
        );
        // The wrong chip's image under a bundled name: a mix-up.
        assert_eq!(
            verify_bundled(bundled("DSPi-RP2040-v1.1.6.uf2"), RP2350.into()),
            BundledFirmware::Mismatch
        );

        let v = firmware_guard_check(bad.clone(), stock_board(), String::new());
        assert!(!v.allowed, "{v:?}");
        assert_eq!(v.decision, FirmwareGuardDecision::Refused);
        assert!(v.message.contains(RP2350), "{v:?}");
        // Typing the phrase does not help.
        let v = firmware_guard_check(
            bad.clone(),
            FirmwareGuardBoard::default(),
            "I ACCEPT THE CORE VOLTAGE RISK".into(),
        );
        assert_eq!(v.decision, FirmwareGuardDecision::Refused);

        // The flash path: nothing reaches the drive.
        let (dir, d) = drive("flipped");
        let r = flash_to_drive_guarded(bad, d, stock_board(), String::new());
        assert!(matches!(r, Err(DspiError::InvalidArgument { .. })), "{r:?}");
        assert!(!dir.join("firmware.uf2").exists());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn wp_secfwv_the_bundled_image_still_flashes() {
        let image = bundled(RP2350);
        let v = firmware_guard_check(image.clone(), stock_board(), String::new());
        assert!(v.allowed, "{v:?}");
        let (dir, d) = drive("bundled");
        flash_to_drive_guarded(image.clone(), d, stock_board(), String::new()).unwrap();
        assert_eq!(std::fs::read(dir.join("firmware.uf2")).unwrap(), image);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn wp_secfwv_a_user_file_is_labelled_not_refused() {
        // A user's build: another version string (as this fork's are), so
        // another identity, though the bytes differ from the bundled image.
        let mut image = bundled(RP2350);
        let at = image
            .windows(5)
            .position(|w| w == b"1.1.6")
            .expect("the version string");
        image[at..at + 5].copy_from_slice(b"1.1.7");
        let info = uf2_inspect(image.clone());
        assert!(info.valid, "{}", info.problem);
        assert!(info.firmware_version.starts_with("1.1.7-"), "{info:?}");

        for name in ["my-build.uf2", "", "DSPi-RP2350-v1.1.7.uf2"] {
            assert_eq!(
                verify_bundled(image.clone(), name.into()),
                BundledFirmware::NotBundled,
                "{name:?}"
            );
        }
        assert_eq!(judge_for_test(&image), BundledFirmware::NotBundled);
        let v = firmware_guard_check(image.clone(), stock_board(), String::new());
        assert!(v.allowed, "{v:?}");
        let (dir, d) = drive("user");
        flash_to_drive_guarded(image.clone(), d, stock_board(), String::new()).unwrap();
        assert_eq!(std::fs::read(dir.join("firmware.uf2")).unwrap(), image);
        std::fs::remove_dir_all(dir).unwrap();

        // Not even a DSPi image, under any name: not this check's business.
        assert_eq!(
            verify_bundled(vec![0; 512], "x.uf2".into()),
            BundledFirmware::NotBundled
        );
    }

    #[test]
    fn wp_secfwv_names_are_exact() {
        let image = bundled(RP2350);
        assert_eq!(
            verify_bundled(image.clone(), format!("C:\\Apps\\Firmware\\{RP2350}")),
            BundledFirmware::Verified
        );
        assert_eq!(
            verify_bundled(image, RP2350.to_lowercase()),
            BundledFirmware::NotBundled
        );
    }

    // -- WP SECFIX --
    /// The clients' label for a picked file: one row of the core's notice
    /// table, information (never a refusal), the plan's words.
    #[test]
    fn wp_secfix_the_not_verified_label_is_one_notice_row() {
        let n = crate::notice_for_key(NOTICE_NOT_VERIFIED.into(), String::new());
        assert_eq!(n.key, NOTICE_NOT_VERIFIED);
        assert_eq!(n.kind, crate::NoticeKind::Info);
        assert_eq!(n.title, "Not a verified DSPi Console image");
        assert!(!n.reason.is_empty() && !n.action.is_empty());
    }
}

// -- WP PGFWUP --
#[cfg(test)]
mod tests_wp_pgfwup {
    use super::*;
    use crate::{FirmwareGuardDecision, uf2_inspect};

    /// A manifest of `entries` (JSON objects without the common fields).
    fn manifest_of(entries: &[&str]) -> Vec<Entry> {
        let images: Vec<String> = entries
            .iter()
            .enumerate()
            .map(|(i, extra)| {
                format!(
                    r#"{{"name": "img{i}.uf2", "platform": "RP2350", "version": "{}",
                       "size": 512, "sha256": "{}"{extra}}}"#,
                    if extra.contains("\"v117\"") {
                        "1.1.7-beta1"
                    } else {
                        "1.1.6-3-gabc"
                    },
                    sha256_hex(&bytes(i)),
                )
            })
            .collect();
        parse(&format!(r#"{{"images": [{}]}}"#, images.join(",")))
    }

    fn bytes(i: usize) -> Vec<u8> {
        vec![u8::try_from(i).unwrap_or(0); 512]
    }

    #[test]
    fn wp_pgfwup_the_bundled_images_are_upstream_and_legacy() {
        let all = bundled_images();
        assert_eq!(
            all.len(),
            manifest().len(),
            "every bundled image is offered"
        );
        for i in all {
            assert_eq!(
                (i.lineage, i.generation, i.page_guard.as_deref()),
                (FirmwareLineage::Upstream, FirmwareGeneration::Legacy, None),
                "{}",
                i.name
            );
        }
    }

    #[test]
    fn wp_pgfwup_a_legacy_bofr_image_needs_the_pageguard_stamp() {
        let all = manifest_of(&[
            r#", "lineage": "bofr", "generation": "legacy""#,
            r#", "lineage": "bofr", "generation": "legacy", "page_guard": "df53ede""#,
            r#", "lineage": "bofr", "generation": "legacy", "page_guard": "not-hex!""#,
            r#", "lineage": "bofr", "generation": "legacy", "page_guard": "abc""#,
            r#", "lineage": "bofr", "generation": "paged""#,
            r#", "lineage": "bofr", "v117": 1"#,
            r#", "lineage": "fork""#,
            r#", "generation": "both""#,
            r#""#,
        ]);
        let offered = offered(&all);
        let names: Vec<&str> = offered.iter().map(|i| i.name.as_str()).collect();
        assert_eq!(names, ["img1.uf2", "img4.uf2", "img5.uf2", "img8.uf2"]);
        assert_eq!(offered[0].page_guard.as_deref(), Some("df53ede"));
        assert_eq!(
            (offered[0].lineage, offered[0].generation),
            (FirmwareLineage::Bofr, FirmwareGeneration::Legacy)
        );
        // The generation from the version: 1.1.7 betas are paged.
        assert_eq!(offered[2].generation, FirmwareGeneration::Paged);
        assert_eq!(offered[3].lineage, FirmwareLineage::Upstream);
        assert!(all[0].judged.as_ref().unwrap_err().contains("PAGEGUARD"));

        // Never flashed: by its name, and by its bytes before every flash.
        for i in [0usize, 2, 3, 6, 7] {
            let name = format!("img{i}.uf2");
            assert_eq!(
                verify_in(&all, &bytes(i), &name),
                BundledFirmware::Mismatch,
                "{name}"
            );
            let info = uf2_inspect(bytes(i));
            let v = refusal_in(&all, &bytes(i), &info, 1200).expect("refused");
            assert!(!v.allowed);
            assert_eq!(v.decision, FirmwareGuardDecision::Refused, "{name}");
            assert!(v.message.contains(&name), "{v:?}");
        }
        for i in [1usize, 4, 5, 8] {
            let name = format!("img{i}.uf2");
            assert_eq!(
                verify_in(&all, &bytes(i), &name),
                BundledFirmware::Verified,
                "{name}"
            );
            assert!(refusal_in(&all, &bytes(i), &uf2_inspect(bytes(i)), 1200).is_none());
        }
    }

    #[test]
    fn wp_pgfwup_the_generation_of_a_version() {
        for (v, g) in [
            ("1.1.6", FirmwareGeneration::Legacy),
            ("1.1.6-1-g9ffce34", FirmwareGeneration::Legacy),
            ("1.0.9", FirmwareGeneration::Legacy),
            ("1.1.7-beta1", FirmwareGeneration::Paged),
            ("1.1.7", FirmwareGeneration::Paged),
            ("1.2.0", FirmwareGeneration::Paged),
            ("", FirmwareGeneration::Unknown),
            ("garbage", FirmwareGeneration::Unknown),
        ] {
            assert_eq!(FirmwareGeneration::of_version(v), g, "{v}");
        }
    }
}
