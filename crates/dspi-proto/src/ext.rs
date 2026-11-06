//! The extension channel's codecs: `REQ_EXTENSION` (0x00) with the
//! sub-command in `wValue` (Bofr5555/DSPi `extension_channel_spec.md`).
//!
//! Pure: what the replies mean and how requests are encoded. The I/O lives in
//! `dspi_session::ext`. Multi-byte values are little-endian.

use crate::generated;

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
    pub const SIZE: usize = 4;

    /// The outputs of pair `p`, as `(left, right)`.
    pub fn pair(&self, p: usize) -> (u8, u8) {
        match self.0[p] {
            0 => (2 * p as u8, 2 * p as u8 + 1),
            b => (b & 0x0F, b >> 4),
        }
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
        for p in 0..4 {
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
