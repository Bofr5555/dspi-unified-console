//! The old-to-new map: every 1.1.6 opcode and how a 1.1.6 request is reproduced
//! on paged commands, generated from upstream's
//! `Documentation/protocol/legacy_map.json` (vendored in `firmware/protocol/`) by
//! `build.rs`. Its `_schema` block defines every field; the doc comments below
//! quote it in short.
//!
//! WP PGPROTO. Also the result codes (`VC_*`, migration guide 5.3) and the status
//! families of the former write-as-reads (guide 5.4), merged per row with its
//! `status_overrides`.
#![warn(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

use super::{CmdId, PagedDir};

/// How a 1.1.6 host issued an opcode (`old_dir`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub enum OldDir {
    /// `W`: an OUT transfer.
    Write,
    /// `R`: an IN transfer that reads.
    Read,
    /// `WR`: an IN transfer that changed state.
    WriteAsRead,
    /// `W+R`: both an OUT and an IN form.
    WriteAndRead,
}

/// `{shift, mask, add}`: `((wValue >> shift) & mask) + add`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct Field {
    pub shift: u8,
    pub mask: u16,
    pub add: i32,
}

impl Field {
    /// `(value >> shift) & mask`, without `add`.
    pub fn extract(self, value: u16) -> u16 {
        value.checked_shr(u32::from(self.shift)).unwrap_or(0) & self.mask
    }

    /// The field with `add` applied, `None` when that leaves 0..=0xFFFF.
    pub fn apply(self, value: u16) -> Option<u16> {
        let v = i64::from(self.extract(value)).checked_add(i64::from(self.add))?;
        u16::try_from(v).ok()
    }
}

/// Where a paged request's `wValue` comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub enum AddrRule {
    /// The old `wValue`, unchanged.
    Old,
    /// A constant.
    Const(u16),
    /// A field of the old `wValue`.
    Field(Field),
}

/// What the adapter does when 1.1.7 STALLs an OUT write (`out_stall`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub enum OutStall {
    /// The default: 1.1.6 ACKed a rejected OUT write silently.
    Swallow,
    /// `raise`: 1.1.6 STALLed a rejected write too.
    Raise,
}

/// `on_stall`: when the paged GET STALLs with a result in `codes`, answer `len`
/// bytes of `fill`, reproducing a documented 1.1.6 zero-fill.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct OnStall {
    pub fill: u8,
    pub len: u16,
    pub codes: &'static [u8],
}

/// `direct`: same direction and payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct Direct {
    pub target: CmdId,
    pub addr: AddrRule,
    /// Cut an OUT payload to this many bytes.
    pub trunc: Option<u16>,
    /// The OUT payload may be longer than 64 bytes.
    pub bulk: bool,
    pub out_stall: OutStall,
    pub on_stall: Option<OnStall>,
    /// No paged replacement as such; the row emulates the 1.1.6 answer (guide 6.7).
    pub retired: bool,
}

/// How a former write-as-read builds its one-byte payload from the old `wValue`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub enum Encode {
    /// `u8`: the low byte.
    U8,
    /// `u8_sat`: saturated at 255.
    U8Sat,
    /// `bool`: 1 when non-zero.
    Bool,
}

impl Encode {
    /// The payload byte for a field value.
    pub fn byte(self, v: u16) -> u8 {
        match self {
            Encode::U8 => v.to_le_bytes()[0],
            Encode::U8Sat => u8::try_from(v).unwrap_or(u8::MAX),
            Encode::Bool => u8::from(v != 0),
        }
    }
}

/// A former write-as-read's status byte (guide 5.4): `success` on an ACK; on a
/// STALL the result code maps through `from_vc` (`status_overrides` merged in);
/// an unmapped or unknown result raises the STALL.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct Status {
    pub family: &'static str,
    pub success: u8,
    /// `(result code, old status byte)`, ascending by code.
    pub from_vc: &'static [(u8, u8)],
}

impl Status {
    /// The old status byte for a refusal with `result`.
    pub fn byte_for(&self, result: u8) -> Option<u8> {
        self.from_vc
            .iter()
            .find(|(c, _)| *c == result)
            .map(|(_, b)| *b)
    }
}

/// `act_value`, `act_addr`, `act_packed`: a write-as-read that is a SET now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct Act {
    pub target: CmdId,
    pub addr: AddrRule,
    /// The value field and its encoding; `None` for `act_addr` (a zero-length SET).
    pub value: Option<(Field, Encode)>,
    pub status: Status,
}

/// `split`: `wValue` (shifted and masked by `selector`) picks a case.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct Split {
    pub selector: Field,
    /// `(selector value, adapter)`, ascending.
    pub cases: &'static [(u16, &'static Adapter)],
    /// Every other selector; `None` means 1.1.6 refused them (IN STALLed, OUT was swallowed).
    pub default: Option<&'static Adapter>,
    pub out_stall: OutStall,
}

impl Split {
    /// The case for an old `wValue`; `None` when it falls to a `null` default.
    pub fn case(&self, value: u16) -> Option<&'static Adapter> {
        let sel = self.selector.extract(value);
        self.cases
            .iter()
            .find(|(k, _)| *k == sel)
            .map(|(_, a)| *a)
            .or(self.default)
    }
}

/// `custom`: no one-to-one rule; written by hand in `paged::custom` (WP PGCUST)
/// from the migration guide's recipe.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct Custom {
    /// The recipe's name (`fn`).
    pub func: &'static str,
    /// Every paged command it touches.
    pub uses: &'static [(CmdId, PagedDir)],
    pub retired: bool,
    pub bulk: bool,
    pub len: Option<u16>,
    /// The row's other fields as compact JSON, verbatim (`GET_STATUS`'s `fields`,
    /// `peaks` and so on), for the recipe.
    pub data: &'static str,
}

/// One adapter of `legacy_map.json`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub enum Adapter {
    Direct(Direct),
    ActValue(Act),
    ActAddr(Act),
    ActPacked(Act),
    Split(Split),
    Custom(Custom),
}

impl Adapter {
    /// The `kind` as `legacy_map.json` spells it.
    pub fn kind(&self) -> &'static str {
        match self {
            Adapter::Direct(_) => "direct",
            Adapter::ActValue(_) => "act_value",
            Adapter::ActAddr(_) => "act_addr",
            Adapter::ActPacked(_) => "act_packed",
            Adapter::Split(_) => "split",
            Adapter::Custom(_) => "custom",
        }
    }
}

/// One 1.1.6 opcode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct LegacyOp {
    /// Its `REQ_` name without the prefix.
    pub name: &'static str,
    pub code: u8,
    pub old_dir: OldDir,
    /// The adapter for the OUT form, `None` when 1.1.6 never handled one.
    pub out: Option<&'static Adapter>,
    /// The adapter for the IN form (reads and write-as-reads).
    pub r#in: Option<&'static Adapter>,
    pub note: Option<&'static str>,
}

include!(concat!(env!("OUT_DIR"), "/paged_legacy.rs"));

/// The row of a 1.1.6 opcode, `None` for one `legacy_map.json` does not have
/// (our `REQ_EXTENSION` 0x00 is the planner's own row).
pub fn op(code: u8) -> Option<&'static LegacyOp> {
    LEGACY_OPS
        .binary_search_by_key(&code, |o| o.code)
        .ok()
        .and_then(|i| LEGACY_OPS.get(i))
}

/// The name of a result code (`VC_OK`, `VC_ERR_*`), `None` for one this build does not know.
pub fn result_name(code: u8) -> Option<&'static str> {
    RESULTS.iter().find(|(_, c)| *c == code).map(|(n, _)| *n)
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_pgproto {
    use super::*;

    #[test]
    fn wp_pgproto_ops_are_sorted_and_found() {
        assert!(LEGACY_OPS.windows(2).all(|w| w[0].code < w[1].code));
        for o in &LEGACY_OPS {
            assert_eq!(op(o.code), Some(o));
            assert!(o.out.is_some() || o.r#in.is_some(), "{}", o.name);
        }
        assert_eq!(op(0x00), None);
    }

    #[test]
    fn wp_pgproto_fields_and_encodings() {
        let f = Field {
            shift: 8,
            mask: 0xFF,
            add: 0,
        };
        assert_eq!(f.extract(0x0503), 5);
        let m1 = Field {
            shift: 0,
            mask: 0xFFFF,
            add: -1,
        };
        assert_eq!(m1.apply(0), None);
        assert_eq!(m1.apply(3), Some(2));
        assert_eq!(Encode::U8.byte(0x1234), 0x34);
        assert_eq!(Encode::U8Sat.byte(0x1234), 0xFF);
        assert_eq!(Encode::U8Sat.byte(7), 7);
        assert_eq!(Encode::Bool.byte(0x100), 1);
        assert_eq!(Encode::Bool.byte(0), 0);
    }

    #[test]
    fn wp_pgproto_result_codes_match_the_guide() {
        // Migration guide 5.3.
        assert_eq!(VC_OK, 0x00);
        assert_eq!(VC_ERR_UNKNOWN, 0x10);
        assert_eq!(VC_ERR_ADDRESS, 0x13);
        assert_eq!(VC_ERR_BUSY, 0x17);
        assert_eq!(VC_ERR_PIN_IN_USE, 0x1A);
        assert_eq!(result_name(0x14), Some("VC_ERR_VALUE"));
        assert_eq!(result_name(0x02), None);
    }
}
