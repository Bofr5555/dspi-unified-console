//! Best-effort reading of bulk images in other wire formats, for display.
//!
//! This build reads and writes wire format V32 ([`WIRE_FORMAT_VERSION`]).
//! Since the V16 break the firmware has changed the bulk layout only in these
//! ways (firmware `bulk_params.h` version log, checked against the struct of
//! every shipped release):
//!
//! - sections appended at the end: V17 ADAT output, V23 psybass, V25 upmixer,
//!   V29 subharmonic synth (16 bytes; V30 grew it, then still the last
//!   section, to 36), V31 tube, V32 limiter;
//! - reserved bytes claimed in place, size unchanged: V19 loudness mask, V20
//!   crossfeed pair mask, V21 I2S clock mode / S/PDIF 2-3 / clock-pin mode,
//!   V22 Linkwitz Transform Q, V24 ADAT input, V26 upmixer presence;
//! - two in-place growths that MOVE later bytes: V18 grew the leveller from
//!   16 to 20 bytes (every later section moves by 4), and V28 grew the input
//!   config's S/PDIF pin array from 2 to 3 (the fields after it move by 1
//!   inside the section, whose size is unchanged).
//!
//! [`normalize`] maps a V16..V31 image onto the V32 layout. Fields the firmware
//! did not have yet stay zero, which is also the firmware's own "absent"
//! encoding for the claimed bytes. For a newer image it keeps the V32 prefix,
//! after a plausibility check.
//!
//! The result is for **display only**. A caller must never send it back as a
//! bulk write: an older firmware refuses it, and a newer one could accept a
//! V32 image and zero the sections appended after V32. [`Normalized::read`]
//! says which case applies.
//!
//! Ported from the Android console's patch 0003 (`wip/wire-compat`); design in
//! `dspi-mobile/docs/WIRE-FORMAT-COMPAT.md`.

use crate::generated::{BULK_SIZE, SECTIONS, wire::WIRE_FORMAT_VERSION};
use crate::wire::{BulkPacket, WireError};

/// The oldest wire format this module can map (the V16 layout break).
pub const MIN_WIRE_FORMAT_VERSION: u8 = 16;

const V32: u8 = WIRE_FORMAT_VERSION as u8;
const HEADER_LEN: usize = 16;
const MAX_CHANNELS: usize = 17;
const MAX_INPUTS: usize = 8;
const MAX_OUTPUTS: usize = 9;
const CHANNEL_NAME_LEN: usize = 32;
const LIMITER_OUTPUT_LEN: usize = 12;

/// How an image was read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub enum WireRead {
    /// This build's own wire format: readable and writable.
    Exact,
    /// An older image (V16..V31) mapped onto the V32 layout, display only.
    Older,
    /// The V32 prefix of a newer image (appended sections ignored), display only.
    Newer,
}

impl WireRead {
    /// The numeric code the C ABI and the bindings carry (`WIRE_READ_*` of
    /// the old desktop core): 1 exact, 2 older, 3 newer; 0 is "nothing read".
    pub fn code(self) -> u8 {
        match self {
            WireRead::Exact => 1,
            WireRead::Older => 2,
            WireRead::Newer => 3,
        }
    }
}

/// Feature bits: which sections a firmware's wire format carries.
pub mod feat {
    /// ADAT output section (V17).
    pub const ADAT_OUT: u32 = 1 << 0;
    /// Leveller detector/apply masks (V18).
    pub const LEVELLER_MASKS: u32 = 1 << 1;
    /// Per-output loudness mask (V19).
    pub const LOUDNESS_MASK: u32 = 1 << 2;
    /// Crossfeed output-pair mask (V20).
    pub const CROSSFEED_MASK: u32 = 1 << 3;
    /// I2S clock master/slave, S/PDIF inputs 2-3, I2S clock-pin mode (V21).
    pub const I2S_CLOCK_MODE: u32 = 1 << 4;
    /// Linkwitz Transform target Q in the band bytes (V22).
    pub const LINKWITZ: u32 = 1 << 5;
    /// Psychoacoustic bass (V23).
    pub const PSYBASS: u32 = 1 << 6;
    /// ADAT input (V24).
    pub const ADAT_IN: u32 = 1 << 7;
    /// Stereo upmixer (V25; the official clients need V26).
    pub const UPMIX: u32 = 1 << 8;
    /// Upmixer presence bell (V26).
    pub const UPMIX_PRESENCE: u32 = 1 << 9;
    /// Fourth S/PDIF input; first-order low/high pass filter types (V28).
    pub const SPDIF_IN4: u32 = 1 << 10;
    /// Subharmonic synthesizer (V29).
    pub const SUBHARM: u32 = 1 << 11;
    /// Subharmonic third band, selectivity, ceiling, pair link (V30).
    pub const SUBHARM_EXT: u32 = 1 << 12;
    /// Tube modeller (V31).
    pub const TUBE: u32 = 1 << 13;
    /// Output limiter (V32).
    pub const LIMITER: u32 = 1 << 14;
    /// Every feature the V32 layout carries.
    pub const ALL: u32 = (1 << 15) - 1;

    /// (feature, first wire format that carries it)
    pub(super) const SINCE: [(u32, u8); 15] = [
        (ADAT_OUT, 17),
        (LEVELLER_MASKS, 18),
        (LOUDNESS_MASK, 19),
        (CROSSFEED_MASK, 20),
        (I2S_CLOCK_MODE, 21),
        (LINKWITZ, 22),
        (PSYBASS, 23),
        (ADAT_IN, 24),
        (UPMIX, 25),
        (UPMIX_PRESENCE, 26),
        (SPDIF_IN4, 28),
        (SUBHARM, 29),
        (SUBHARM_EXT, 30),
        (TUBE, 31),
        (LIMITER, 32),
    ];
}

/// The `feat::*` bits a firmware of wire format `version` carries in its
/// bulk image (0 below V16; every known bit for V32 and newer).
pub fn features(version: u8) -> u32 {
    if version < MIN_WIRE_FORMAT_VERSION {
        return 0;
    }
    feat::SINCE
        .iter()
        .filter(|(_, v)| version >= *v)
        .fold(0, |m, (f, _)| m | f)
}

/// `sizeof(WireBulkParams)` of wire format `version` (V16..V32).
pub fn bulk_size(version: u8) -> Option<usize> {
    Some(match version {
        16 => 5864,
        17 => 5872,
        18..=22 => 5876,
        23 | 24 => 5900,
        25..=28 => 5944,
        29 => 5960,
        30 => 5980,
        31 => 6028,
        32 => BULK_SIZE,
        _ => return None,
    })
}

fn off(name: &str) -> usize {
    SECTIONS
        .iter()
        .find(|(n, _, _)| *n == name)
        .map(|(_, o, _)| *o)
        .unwrap_or_else(|| panic!("no bulk section {name}"))
}

fn u16_at(b: &[u8], i: usize) -> u16 {
    u16::from_le_bytes([b[i], b[i + 1]])
}

/// How many bytes to read for the image whose first bytes are `head`.
///
/// The header's `payload_length`: older firmware stalls a chunk read past its
/// end, and newer firmware appends, so stopping early would leave the device's
/// snapshot session open. A V32 image, a short head or an implausible length
/// give the V32 size.
pub fn image_length(head: &[u8]) -> usize {
    if head.len() < 8 || head[0] == V32 {
        return BULK_SIZE;
    }
    let len = u16_at(head, 6) as usize;
    if len < off("global") { BULK_SIZE } else { len }
}

/// An image mapped onto the V32 layout.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Normalized {
    /// The V32-layout packet. Its header says V32 and 6136 bytes so that it
    /// decodes; the device's own version is [`Normalized::device_version`].
    pub packet: BulkPacket,
    pub read: WireRead,
    /// The wire format the device reported.
    pub device_version: u8,
    /// `feat::*` bits the device's format carries.
    pub features: u32,
}

impl Normalized {
    /// Whether this image may be written back to the device.
    pub fn is_writable(&self) -> bool {
        self.read == WireRead::Exact
    }
}

fn unsupported(v: u8) -> WireError {
    WireError::UnsupportedVersion {
        got: v,
        expected: V32,
    }
}

fn too_short(got: usize, expected: usize) -> WireError {
    WireError::WrongSize { got, expected }
}

/// Map `data` (a bulk image of any supported wire format) onto the V32 layout.
pub fn normalize(data: &[u8]) -> Result<Normalized, WireError> {
    let (img, read) = normalize_bytes(data)?;
    let device_version = data[0];
    let mut raw = img;
    // The header now describes the V32 layout it was mapped onto.
    raw[0] = V32;
    raw[6..8].copy_from_slice(&(BULK_SIZE as u16).to_le_bytes());
    let packet = BulkPacket::decode(raw)?;
    Ok(Normalized {
        packet,
        read,
        device_version,
        features: if read == WireRead::Older {
            features(device_version)
        } else {
            feat::ALL
        },
    })
}

/// The byte-level mapping. Returns the V32-sized image (its header still
/// carries the device's version and length) and how it was read.
pub fn normalize_bytes(data: &[u8]) -> Result<(Vec<u8>, WireRead), WireError> {
    let global = off("global");
    if data.len() < global {
        return Err(too_short(data.len(), BULK_SIZE));
    }
    let v = data[0];
    if v < MIN_WIRE_FORMAT_VERSION {
        return Err(unsupported(v));
    }
    if v >= V32 {
        if data.len() < BULK_SIZE {
            return Err(if v == V32 {
                too_short(data.len(), BULK_SIZE)
            } else {
                // A newer image shorter than V32 broke the append-only rule.
                unsupported(v)
            });
        }
        let img = data[..BULK_SIZE].to_vec();
        if v == V32 {
            return Ok((img, WireRead::Exact));
        }
        if !plausible_prefix(&img) {
            return Err(unsupported(v));
        }
        return Ok((img, WireRead::Newer));
    }

    let size = bulk_size(v).ok_or_else(|| unsupported(v))?;
    if data.len() < size {
        return Err(too_short(data.len(), size));
    }
    let (leveller, preamp, input_config, adat) = (
        off("leveller"),
        off("preamp"),
        off("input_config"),
        off("adat_config"),
    );
    let mut out = vec![0u8; BULK_SIZE];
    // Header through the first 16 leveller bytes: same offsets in every version.
    out[..leveller + 16].copy_from_slice(&data[..leveller + 16]);
    // V18 grew the leveller by 4 (detector/apply masks); older images have
    // every later section 4 bytes earlier.
    let shift = if v < 18 { 4 } else { 0 };
    if v >= 18 {
        out[leveller + 16..preamp].copy_from_slice(&data[leveller + 16..preamp]);
    }
    // Preamp .. crossovers, plus the ADAT output section from V17.
    let end = if v >= 17 { adat + 8 } else { adat };
    out[preamp..end].copy_from_slice(&data[preamp - shift..end - shift]);
    // V28 grew spdif_rx_pin_ext from 2 to 3 entries: before it, the enable
    // mask, I2S clock mode and the three ADAT input bytes sat one byte earlier.
    if v < 28 {
        let ic = input_config;
        // Cannot fail: a 5-byte range converts to [u8; 5]. `out` is
        // BULK_SIZE long and the input-config section ends 16 bytes after
        // `ic`, well inside it; `data` was checked against bulk_size(v) above,
        // and every slice of it here ends before that size.
        let tail: [u8; 5] = out[ic + 10..ic + 15].try_into().unwrap();
        out[ic + 10] = 0; // S/PDIF 4 pin: absent
        out[ic + 11..ic + 16].copy_from_slice(&tail);
    }
    // Appended sections (all after V18, so unshifted).
    let (psybass, upmix, subharm, tube, limiter) = (
        off("psybass"),
        off("upmix"),
        off("subharm"),
        off("tube"),
        off("limiter"),
    );
    let mut tail = |since: u8, at: usize, len: usize| {
        if v >= since {
            out[at..at + len].copy_from_slice(&data[at..at + len]);
        }
    };
    tail(23, psybass, upmix - psybass);
    tail(25, upmix, subharm - upmix);
    tail(29, subharm, if v == 29 { 16 } else { tube - subharm });
    tail(31, tube, limiter - tube);
    Ok((out, WireRead::Older))
}

/// Checks that a newer image's V32 prefix still lines up: the header counts,
/// the channel names (NUL-terminated, middle of the image) and the limiter
/// records (its end). A newer firmware that grew a section in place, as V18
/// and V28 did, fails here instead of being shown garbled.
fn plausible_prefix(d: &[u8]) -> bool {
    if d.len() < HEADER_LEN {
        return false;
    }
    if d[2] as usize > MAX_CHANNELS
        || d[3] as usize > MAX_OUTPUTS
        || d[4] as usize > MAX_INPUTS
        || d[5] != 12
    {
        return false;
    }
    let names = off("channel_names");
    let names_ok = (0..MAX_CHANNELS).all(|ch| {
        let at = names + ch * CHANNEL_NAME_LEN;
        d[at..at + CHANNEL_NAME_LEN].contains(&0)
    });
    let limiter = off("limiter");
    let limiter_ok = (0..MAX_OUTPUTS).all(|o| {
        let at = limiter + o * LIMITER_OUTPUT_LEN;
        d[at] <= 1 && d[at + 1] <= 4 && d[at + 2] == 0 && d[at + 3] == 0
    });
    names_ok && limiter_ok
}

#[cfg(test)]
mod tests {
    use super::*;

    fn put_u16(d: &mut [u8], i: usize, v: u16) {
        d[i..i + 2].copy_from_slice(&v.to_le_bytes());
    }

    #[test]
    fn sizes_follow_the_version_log() {
        // Appended sections: ADAT 8, psybass 24, upmix 44, subharm 16 -> 36, tube 48, limiter 108.
        assert_eq!(bulk_size(17).unwrap() - bulk_size(16).unwrap(), 8);
        assert_eq!(bulk_size(18).unwrap() - bulk_size(17).unwrap(), 4);
        assert_eq!(
            bulk_size(23).unwrap() - bulk_size(22).unwrap(),
            off("upmix") - off("psybass")
        );
        assert_eq!(
            bulk_size(25).unwrap() - bulk_size(24).unwrap(),
            off("subharm") - off("upmix")
        );
        assert_eq!(bulk_size(30).unwrap(), off("tube"));
        assert_eq!(bulk_size(31).unwrap(), off("limiter"));
        assert_eq!(bulk_size(32), Some(BULK_SIZE));
        assert_eq!(bulk_size(15), None);
        assert_eq!(bulk_size(33), None);
    }

    #[test]
    fn features_by_version() {
        assert_eq!(features(15), 0);
        assert_eq!(features(16), 0);
        assert_eq!(features(17), feat::ADAT_OUT);
        assert_eq!(features(28) & feat::SPDIF_IN4, feat::SPDIF_IN4);
        assert_eq!(features(28) & feat::SUBHARM, 0);
        assert_eq!(features(32), feat::ALL);
        assert_eq!(features(40), feat::ALL);
    }

    #[test]
    fn image_length_uses_the_header() {
        let mut h = [0u8; 16];
        h[0] = 28;
        put_u16(&mut h, 6, 5944);
        assert_eq!(image_length(&h), 5944);
        h[0] = 32;
        put_u16(&mut h, 6, 1);
        assert_eq!(image_length(&h), BULK_SIZE, "V32 never trusts the field");
        h[0] = 33;
        put_u16(&mut h, 6, 6200);
        assert_eq!(image_length(&h), 6200);
        put_u16(&mut h, 6, 3);
        assert_eq!(image_length(&h), BULK_SIZE);
        assert_eq!(image_length(&h[..4]), BULK_SIZE);
    }

    #[test]
    fn rejects_what_it_cannot_map() {
        let mut d = vec![0u8; 6200];
        d[0] = 15;
        assert_eq!(normalize_bytes(&d).unwrap_err(), unsupported(15));
        d[0] = 28;
        assert!(matches!(
            normalize_bytes(&d[..5000]),
            Err(WireError::WrongSize { .. })
        ));
        d[0] = 33; // shorter than V32: not append-only any more
        assert_eq!(normalize_bytes(&d[..6000]).unwrap_err(), unsupported(33));
        d[5] = 12;
        d[off("limiter")] = 7; // a limiter "enabled" byte that is not 0/1
        assert_eq!(normalize_bytes(&d).unwrap_err(), unsupported(33));
        d[off("limiter")] = 1;
        assert_eq!(normalize_bytes(&d).unwrap().1, WireRead::Newer);
    }

    #[test]
    fn v17_tail_moves_by_four_and_input_config_by_one() {
        let size = bulk_size(17).unwrap();
        let mut d: Vec<u8> = (0..size).map(|i| (i % 251) as u8 | 1).collect();
        d[0] = 17;
        let (img, read) = normalize_bytes(&d).unwrap();
        assert_eq!(read, WireRead::Older);
        let (lev, pre, ic, adat) = (
            off("leveller"),
            off("preamp"),
            off("input_config"),
            off("adat_config"),
        );
        assert_eq!(&img[lev + 16..pre], &[0, 0, 0, 0], "no masks yet");
        assert_eq!(&img[pre..ic], &d[pre - 4..ic - 4]);
        assert_eq!(&img[ic..ic + 10], &d[ic - 4..ic + 6]);
        assert_eq!(img[ic + 10], 0);
        assert_eq!(&img[ic + 11..ic + 16], &d[ic + 6..ic + 11]);
        assert_eq!(&img[adat..adat + 8], &d[adat - 4..adat + 4]);
        assert!(
            img[adat + 8..].iter().all(|&b| b == 0),
            "nothing appended yet"
        );
    }

    #[test]
    fn an_older_image_decodes_as_a_display_only_packet() {
        let mut d = vec![0u8; bulk_size(28).unwrap()];
        d[0] = 28;
        d[2] = 17;
        d[3] = 9;
        d[4] = 8;
        let mv = off("master_volume");
        d[mv..mv + 4].copy_from_slice(&(-12.0f32).to_le_bytes());
        let n = normalize(&d).unwrap();
        assert_eq!(n.read, WireRead::Older);
        assert!(!n.is_writable());
        assert_eq!(n.device_version, 28);
        assert_eq!(n.features, features(28));
        assert_eq!(n.packet.header().format_version, 32);
        let b = n.packet.as_bytes();
        assert_eq!(f32::from_le_bytes(b[mv..mv + 4].try_into().unwrap()), -12.0);
    }

    #[test]
    fn a_v32_image_is_exact_and_writable() {
        let mut d = vec![0u8; BULK_SIZE];
        d[0] = 32;
        d[2] = 17;
        d[3] = 9;
        d[4] = 8;
        put_u16(&mut d, 6, BULK_SIZE as u16);
        let n = normalize(&d).unwrap();
        assert_eq!(n.read, WireRead::Exact);
        assert!(n.is_writable());
        assert_eq!(n.packet.as_bytes(), &d[..]);
    }
}
