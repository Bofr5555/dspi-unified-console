//! The extension channel's codecs: `REQ_EXTENSION` (0x00) with the
//! sub-command in `wValue` (Bofr5555/DSPi `extension_channel_spec.md`).
//!
//! Pure: what the replies mean and how requests are encoded. The I/O lives in
//! `dspi_session::ext`. Multi-byte values are little-endian.

use crate::generated;

// -- WP FWX -- identify, reset info, the clip event
mod diag;
pub use diag::*;

/// A user-supplied value that does not parse (a pair map, a voltage).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct BadArgument(pub String);

/// Room for 32 sub-command ids (extension_channel_spec.md 2.1).
pub const LIST_READ_LEN: usize = 64;

/// What a sub-command is called, for listings.
pub fn name(id: u16) -> &'static str {
    match id {
        generated::ext::EXT_LIST => "list",
        generated::ext::EXT_CROSSFEED_PAIR_MAP => "crossfeed pair map",
        generated::ext::EXT_CORE_VREG => "core voltage",
        // -- WP FWX --
        generated::ext::EXT_NOTIFY_FEATURES => "notify features",
        generated::ext::EXT_IDENTIFY => "identify",
        generated::ext::EXT_RESET_INFO => "reset info",
        0x8000..=0xFFFF => "experimental",
        _ => "unknown",
    }
}

/// `EXT_LIST`'s reply: uint16 LE ids, ascending, 0 excluded. An odd trailing
/// byte (a truncated reply) is dropped.
pub fn decode_list(b: &[u8]) -> Vec<u16> {
    b.as_chunks::<2>()
        .0
        .iter()
        .map(|c| u16::from_le_bytes(*c))
        .collect()
}

// ------------------------------------------------------------ pair map

/// `EXT_CROSSFEED_PAIR_MAP`: which two outputs each crossfeed pair processes.
///
/// Byte `p` is pair `p`. `0x00` is the default pairing (`2p`, `2p+1`); any
/// other value is `(right << 4) | left` (crossfeed_map.h).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PairMap(pub [u8; 4]);

impl PairMap {
    /// The wire size in bytes, one per pair.
    pub const SIZE: usize = 4;
    /// How many pairs the firmware has: valid indices for [`PairMap::pair`]
    /// are `0..PAIRS`.
    pub const PAIRS: usize = Self::SIZE;
    /// One DAC per ear (the K340), `crossfeed_map.h`'s example: pair 0 is the
    /// woofers (left out 2, right out 0), pair 1 the tweeters (left 3, right 1).
    /// Valid on two pairs and on four.
    pub const K340: PairMap = PairMap([0x02, 0x13, 0, 0]);
    /// The output pair mask that goes with [`PairMap::K340`]: both of its
    /// pairs. The factory mask covers pair 0 only.
    pub const K340_MASK: u8 = 0x03;

    /// The outputs of pair `p`, as `(left, right)`.
    ///
    /// A pair past the map (`p >= 4`; the firmware has four) has no byte, so
    /// it reads as the default pairing `(2p, 2p+1)`, saturated at 255. It
    /// never panics, whatever `p` a caller or a device supplies. Callers
    /// iterating over pairs bound their own index by [`PairMap::PAIRS`]; the
    /// value past it is a fallback, not a pair the device has.
    pub fn pair(&self, p: usize) -> (u8, u8) {
        match self.0.get(p) {
            Some(&b) if b != 0 => (b & 0x0F, b >> 4),
            _ => {
                let left = u8::try_from(p.saturating_mul(2)).unwrap_or(u8::MAX);
                (left, left.saturating_add(1))
            }
        }
    }

    /// Whether the firmware accepts this map on a platform with `num_pairs`
    /// stereo pairs (4 on RP2350, 2 on RP2040): `crossfeed_map_valid` in
    /// crossfeed_map.h. Each of the first `num_pairs` pairs takes two different
    /// outputs below `2 * num_pairs`, which leaves out the PDM sub; no output is
    /// in two pairs; bytes past `num_pairs` are 0. A `num_pairs` of 0 or above
    /// [`PairMap::PAIRS`] is invalid.
    ///
    /// This is the only copy of the rule; every client asks here or through
    /// `dspi_core::crossfeed_pair_map_valid`.
    pub fn is_valid(&self, num_pairs: usize) -> bool {
        if num_pairs == 0 || num_pairs > Self::PAIRS {
            return false;
        }
        // At most four pairs, so at most eight outputs.
        let outputs = num_pairs.saturating_mul(2);
        let mut used = 0u16;
        for (p, b) in self.0.iter().enumerate() {
            if p >= num_pairs {
                if *b != 0 {
                    return false;
                }
                continue;
            }
            let (l, r) = self.pair(p);
            if usize::from(l) >= outputs || usize::from(r) >= outputs || l == r {
                return false;
            }
            // Both are below eight, from `outputs`.
            let bits = (1u16 << l) | (1u16 << r);
            if used & bits != 0 {
                return false;
            }
            used |= bits;
        }
        true
    }

    /// The firmware's normal form: an explicit default byte reads back as 0.
    pub fn normalized(&self) -> PairMap {
        let mut out = self.0;
        for (p, b) in out.iter_mut().enumerate() {
            if *b == ((2 * p as u8 + 1) << 4) | (2 * p as u8) {
                *b = 0;
            }
        }
        PairMap(out)
    }

    /// Parse up to four pairs: `default` (or `-`), `L:R` with output numbers,
    /// or a raw byte `0xNN`. Missing pairs are the default.
    pub fn parse(args: &[&str]) -> std::result::Result<PairMap, BadArgument> {
        if args.len() > 4 {
            return Err(BadArgument(format!(
                "at most four pairs, got {}",
                args.len()
            )));
        }
        let mut map = [0u8; 4];
        for (p, a) in args.iter().enumerate() {
            map[p] = if *a == "default" || *a == "-" {
                0
            } else if let Some(hex) = a.strip_prefix("0x").or_else(|| a.strip_prefix("0X")) {
                u8::from_str_radix(hex, 16)
                    .map_err(|_| BadArgument(format!("pair {p}: bad byte {a}")))?
            } else if let Some((l, r)) = a.split_once(':') {
                let out = |s: &str| {
                    s.parse::<u8>()
                        .ok()
                        .filter(|v| *v < 16)
                        .ok_or_else(|| BadArgument(format!("pair {p}: bad output {s}")))
                };
                let (l, r) = (out(l)?, out(r)?);
                (r << 4) | l
            } else {
                return Err(BadArgument(format!(
                    "pair {p}: expected L:R, default or 0xNN, got {a}"
                )));
            };
        }
        Ok(PairMap(map))
    }
}

impl std::fmt::Display for PairMap {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for p in 0..Self::PAIRS {
            let (l, r) = self.pair(p);
            let tag = if self.0[p] == 0 { " (default)" } else { "" };
            if p > 0 {
                writeln!(f)?;
            }
            write!(f, "pair {p}: L=out {l}  R=out {r}{tag}")?;
        }
        Ok(())
    }
}

/// `EXT_CORE_VREG`'s 16-byte status (core_vreg.h).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreVregStatus {
    pub live_vreg: u8,
    /// 0 = the build default.
    pub stored_sel: u8,
    pub default_vreg: u8,
    pub flags: u8,
    pub hang_count: u8,
    pub trial_active: bool,
    pub trial_vreg: u8,
    /// 0 = none.
    pub failed_trial_vreg: u8,
    pub failed_trial_uptime_s: u32,
    pub uptime_s: u32,
}

impl CoreVregStatus {
    pub const SIZE: usize = 16;
    pub const FLAG_AUTO_RAISED: u8 = 0x01;
    pub const FLAG_AUTO_DISABLED: u8 = 0x02;

    pub fn decode(b: &[u8]) -> Option<Self> {
        let b = b.get(..Self::SIZE)?;
        let u32_at = |i: usize| u32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]]);
        Some(Self {
            live_vreg: b[0],
            stored_sel: b[1],
            default_vreg: b[2],
            flags: b[3],
            hang_count: b[4],
            trial_active: b[5] != 0,
            trial_vreg: b[6],
            failed_trial_vreg: b[7],
            failed_trial_uptime_s: u32_at(8),
            uptime_s: u32_at(12),
        })
    }

    /// Readable `key value` pairs, in wire order.
    pub fn fields(&self) -> Vec<(&'static str, String)> {
        let flag = |bit: u8| if self.flags & bit != 0 { "yes" } else { "no" };
        let mut v = vec![
            ("live", vreg_label(self.live_vreg)),
            (
                "stored",
                if self.stored_sel == 0 {
                    "build default".into()
                } else {
                    vreg_label(self.stored_sel)
                },
            ),
            ("default", vreg_label(self.default_vreg)),
            ("auto_raised", flag(Self::FLAG_AUTO_RAISED).into()),
            ("auto_disabled", flag(Self::FLAG_AUTO_DISABLED).into()),
            ("hang_count", self.hang_count.to_string()),
            (
                "trial",
                if self.trial_active {
                    vreg_label(self.trial_vreg)
                } else {
                    "none".into()
                },
            ),
        ];
        v.push((
            "failed_trial",
            if self.failed_trial_vreg == 0 {
                "none".into()
            } else {
                format!(
                    "{} after {} s",
                    vreg_label(self.failed_trial_vreg),
                    self.failed_trial_uptime_s
                )
            },
        ));
        v.push(("uptime_s", self.uptime_s.to_string()));
        v
    }
}

/// A vreg code in millivolts. The RP2040 and RP2350 share codes 0x06-0x0F:
/// 0.85 V in 50 mV steps, so 0x0C is 1.15 V and 0x0F is 1.30 V.
pub fn vreg_mv(code: u8) -> Option<u16> {
    (0x06..=0x0F)
        .contains(&code)
        .then(|| 850 + 50 * (code as u16 - 0x06))
}

pub fn vreg_label(code: u8) -> String {
    match vreg_mv(code) {
        Some(mv) => format!("{}.{:02} V (0x{code:02X})", mv / 1000, (mv % 1000) / 10),
        None => format!("0x{code:02X}"),
    }
}

/// A voltage as written: `1.20`, `1200`, `1.2V`, or a code `0x0D`.
pub fn parse_vreg(s: &str) -> std::result::Result<u8, BadArgument> {
    let bad = || BadArgument(format!("not a core voltage: {s}"));
    if let Some(hex) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        return u8::from_str_radix(hex, 16).map_err(|_| bad());
    }
    let t = s.trim_end_matches(['V', 'v']);
    let v: f64 = t.parse().map_err(|_| bad())?;
    let mv = if v < 10.0 { v * 1000.0 } else { v };
    let mv = mv.round() as u16;
    (0x06..=0x0F)
        .find(|c| vreg_mv(*c) == Some(mv))
        .ok_or_else(bad)
}

/// `EXT_CORE_VREG`'s OUT `{op, value}`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VregOp {
    /// Persist a voltage; 0 is the build default.
    Set(u8),
    /// RAM-only trial voltage.
    Trial(u8),
    TrialEnd,
    /// Automatic raising after repeated hangs.
    Auto(bool),
    /// Forget the hang count and the last trial failure.
    Clear,
}

impl VregOp {
    pub fn encode(self) -> [u8; 2] {
        match self {
            VregOp::Set(v) => [0, v],
            VregOp::Trial(v) => [1, v],
            VregOp::TrialEnd => [2, 0],
            VregOp::Auto(on) => [3, on as u8],
            VregOp::Clear => [4, 0],
        }
    }
}

/// WP CG17: the one pair-map rule, held to the firmware's.
#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_cg17 {
    use super::*;

    const fn map(a: u8, b: u8, c: u8, d: u8) -> PairMap {
        PairMap([a, b, c, d])
    }

    /// firmware/tests/test_crossfeed_map.c's cases.
    #[test]
    fn wp_cg17_the_firmware_test_vectors() {
        let cases = [
            (map(0, 0, 0, 0), 4, true),
            (map(0, 0, 0, 0), 2, true),
            (PairMap::K340, 4, true),
            (PairMap::K340, 2, true),
            (map(0x11, 0, 0, 0), 4, false),
            (map(0x02, 0, 0, 0), 4, false),
            (map(0x10, 0x20, 0, 0), 4, false),
            (map(0x40, 0, 0, 0), 2, false),
            (map(0x01, 0x42, 0, 0), 2, false),
            (map(0x80, 0, 0, 0), 4, false),
            (map(0xF0, 0, 0, 0), 4, false),
            (map(0x76, 0x54, 0x32, 0x10), 4, true),
            (map(0, 0, 0x54, 0), 2, false),
            (map(0, 0, 0, 0), 0, false),
            (map(0, 0, 0, 0), 5, false),
            (map(0, 0, 0, 0), usize::MAX, false),
        ];
        for (m, pairs, want) in cases {
            assert_eq!(m.is_valid(pairs), want, "{:02x?} on {pairs}", m.0);
        }
    }

    #[test]
    fn wp_cg17_the_k340_map_is_already_normal_and_its_mask_is_its_pairs() {
        assert_eq!(PairMap::K340.normalized(), PairMap::K340);
        assert_eq!(PairMap::K340.0, 0x0000_1302u32.to_le_bytes());
        assert_eq!(PairMap::K340_MASK, 0b11);
    }

    /// `is_valid` and `normalized` against `crossfeed_map_valid` and
    /// `crossfeed_map_normalize`, compiled from DSPi-bofr 04122d3: every first
    /// two bytes, eight values each for the other two, 0 to 5 pairs. The C ran
    /// the same loop, and these are its printed count and FNV-1a hash.
    #[test]
    fn wp_cg17_agrees_with_the_compiled_firmware_rule() {
        const REST: [u8; 8] = [0, 0x10, 0x32, 0x54, 0x76, 0x02, 0x13, 0x45];
        let mut hash = 0xcbf2_9ce4_8422_2325u64;
        let mut valid = 0u64;
        for pairs in 0..=5 {
            for a in 0..=255u8 {
                for b in 0..=255u8 {
                    for c in REST {
                        for d in REST {
                            let m = map(a, b, c, d);
                            let out = if m.is_valid(pairs) {
                                valid += 1;
                                m.normalized().0
                            } else {
                                [0xFF; 4]
                            };
                            for byte in out {
                                hash = (hash ^ u64::from(byte)).wrapping_mul(0x100_0000_01b3);
                            }
                        }
                    }
                }
            }
        }
        assert_eq!(valid, 1347);
        assert_eq!(hash, 0x092b_7d00_4f6f_9884);
    }
}

/// WP P1: device- and caller-sourced indices never panic (PLAN §1.7).
#[cfg(test)]
mod tests_wp_p1 {
    use super::*;

    #[test]
    fn wp_p1_a_pair_past_the_map_reads_as_the_default_pairing() {
        let m = PairMap([0x10, 0, 0x54, 0x76]);
        assert_eq!(m.pair(0), (0, 1));
        assert_eq!(m.pair(1), (2, 3));
        assert_eq!(m.pair(2), (4, 5));
        // Past the map: this used to index out of bounds.
        assert_eq!(m.pair(4), (8, 9));
        assert_eq!(m.pair(127), (254, 255));
        assert_eq!(m.pair(128), (255, 255));
        assert_eq!(m.pair(usize::MAX), (255, 255));
    }
}
