//! The reverse view: a paged request as the 1.1.6 request with the same effect
//! (PLAN-PG section 1.6, WP PGPROTO), for the hub's judges (policy class, lock
//! scope, the core-voltage rule, limits). `None` when there is none, as for the
//! commands 1.1.7 added (`SYS.PIN_STATE` and the like) and the discovery reads.
//!
//! Written with the planner: every candidate row is planned forward again and
//! kept only when it reproduces the request exactly (id, direction, `wValue`,
//! payload), so the two can never disagree.
#![warn(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

use super::legacy::{Act, Adapter, AddrRule, Encode, Field, LEGACY_OPS, LegacyOp};
use super::plan::{LegacyBody, PagedBody, UsbDir, plan_in};
use super::registry::{self, AddrKind};
use super::{CmdId, EXT_TUNNEL, PagedDir};
use crate::generated::opcodes::REQ_EXTENSION;

/// A 1.1.6 request equivalent to a paged one.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct LegacyEquivalent {
    pub opcode: u8,
    pub dir: UsbDir,
    /// A former write-as-read (an IN request that changes state).
    pub action: bool,
    pub value: u16,
    /// The OUT payload; empty for an IN request.
    pub data: Vec<u8>,
}

/// The 1.1.6 request with the same effect as the paged request `(id, dir, value,
/// payload)` (`payload` is ignored for a GET). The first matching row wins,
/// rows not marked retired before retired ones, then by opcode.
pub fn legacy_equivalent(
    id: CmdId,
    dir: PagedDir,
    value: u16,
    payload: &[u8],
) -> Option<LegacyEquivalent> {
    if id == EXT_TUNNEL {
        return Some(LegacyEquivalent {
            opcode: REQ_EXTENSION,
            dir: match dir {
                PagedDir::Get => UsbDir::In,
                PagedDir::Set => UsbDir::Out,
            },
            action: false,
            value,
            data: match dir {
                PagedDir::Get => Vec::new(),
                PagedDir::Set => payload.to_vec(),
            },
        });
    }
    if id.is_legacy() {
        return None;
    }
    let payload = match dir {
        PagedDir::Get => &[][..],
        PagedDir::Set => payload,
    };
    let mut retired = None;
    for op in &LEGACY_OPS {
        for (usb, side) in [(UsbDir::Out, op.out), (UsbDir::In, op.r#in)] {
            let Some(a) = side else { continue };
            for leaf in leaves(a) {
                let Some(cand) = candidate(op, usb, &leaf, id, dir, value, payload) else {
                    continue;
                };
                if !verify(&cand, id, dir, value, payload) {
                    continue;
                }
                if leaf.retired {
                    retired.get_or_insert(cand);
                } else {
                    return Some(cand);
                }
            }
        }
    }
    retired.or_else(|| super::custom::legacy_equivalent(id, dir, value, payload))
}

/// One non-split adapter reached through a split's case (or directly).
struct Leaf {
    adapter: &'static Adapter,
    /// The selector field and the case key that leads here (`None` key: the default).
    selector: Option<(Field, Option<u16>)>,
    retired: bool,
}

fn leaves(a: &'static Adapter) -> Vec<Leaf> {
    match a {
        Adapter::Split(s) => {
            let mut out: Vec<Leaf> = s
                .cases
                .iter()
                .map(|(k, c)| Leaf {
                    adapter: c,
                    selector: Some((s.selector, Some(*k))),
                    retired: is_retired(c),
                })
                .collect();
            if let Some(d) = s.default {
                out.push(Leaf {
                    adapter: d,
                    selector: Some((s.selector, None)),
                    retired: is_retired(d),
                });
            }
            out
        }
        other => vec![Leaf {
            adapter: other,
            selector: None,
            retired: is_retired(other),
        }],
    }
}

fn is_retired(a: &Adapter) -> bool {
    matches!(a, Adapter::Direct(d) if d.retired)
}

/// Put `bits` into `old` at `field` (`(x - add)` must fit the mask).
fn place(old: u16, field: Field, x: u16) -> Option<u16> {
    let raw = i64::from(x).checked_sub(i64::from(field.add))?;
    let raw = u16::try_from(raw).ok()?;
    if raw & !field.mask != 0 {
        return None;
    }
    let shifted = u32::from(raw).checked_shl(u32::from(field.shift))?;
    let shifted = u16::try_from(shifted).ok()?;
    Some(old | shifted)
}

/// The old `wValue` bits an address rule needs for paged `value`.
fn old_for_addr(rule: AddrRule, kind: AddrKind, value: u16, old: u16) -> Option<u16> {
    if kind == AddrKind::None {
        // Forward planning sends 0 whatever the old wValue was.
        return (value == 0).then_some(old);
    }
    match rule {
        AddrRule::Old => Some(old | value),
        AddrRule::Const(k) => (k == value).then_some(old),
        AddrRule::Field(f) => place(old, f, value),
    }
}

fn candidate(
    op: &'static LegacyOp,
    usb: UsbDir,
    leaf: &Leaf,
    id: CmdId,
    dir: PagedDir,
    value: u16,
    payload: &[u8],
) -> Option<LegacyEquivalent> {
    let mut old = 0u16;
    if let Some((sel, Some(key))) = leaf.selector {
        old = place(old, sel, key)?;
    }
    let action = match leaf.adapter {
        Adapter::Direct(d) => {
            let want = match usb {
                UsbDir::In => PagedDir::Get,
                UsbDir::Out => PagedDir::Set,
            };
            if d.target != id || want != dir {
                return None;
            }
            let kind = registry::command(id)?.addr;
            old = old_for_addr(d.addr, kind, value, old)?;
            false
        }
        Adapter::ActValue(x) | Adapter::ActAddr(x) | Adapter::ActPacked(x) => {
            if x.target != id || dir != PagedDir::Set {
                return None;
            }
            let kind = registry::command(id)?.addr;
            old = old_for_addr(x.addr, kind, value, old)?;
            old = old_for_value(x, payload, old)?;
            true
        }
        Adapter::Split(_) | Adapter::Custom(_) => return None,
    };
    Some(LegacyEquivalent {
        opcode: op.code,
        dir: usb,
        action,
        value: old,
        data: match usb {
            UsbDir::Out => payload.to_vec(),
            UsbDir::In => Vec::new(),
        },
    })
}

/// The old `wValue` bits a write-as-read's payload byte came from.
fn old_for_value(x: &Act, payload: &[u8], old: u16) -> Option<u16> {
    match (x.value, payload) {
        (None, []) => Some(old),
        (Some((field, encode)), [b]) => {
            let v = u16::from(*b);
            match encode {
                Encode::U8 | Encode::U8Sat => place(old, field, v),
                Encode::Bool if *b <= 1 => place(old, field, v),
                Encode::Bool => None,
            }
        }
        _ => None,
    }
}

/// Plan the candidate forward (without device checks) and compare.
fn verify(cand: &LegacyEquivalent, id: CmdId, dir: PagedDir, value: u16, payload: &[u8]) -> bool {
    let body = match cand.dir {
        UsbDir::In => LegacyBody::In { len: 1 },
        UsbDir::Out => LegacyBody::Out(&cand.data),
    };
    let Ok(plan) = plan_in(cand.opcode, cand.value, body, None) else {
        return false;
    };
    let Some(req) = plan.single() else {
        return false;
    };
    req.id == id
        && req.value == value
        && match (&req.body, dir) {
            (PagedBody::Get { .. }, PagedDir::Get) => true,
            (PagedBody::Set(d), PagedDir::Set) => d.as_slice() == payload,
            _ => false,
        }
}
