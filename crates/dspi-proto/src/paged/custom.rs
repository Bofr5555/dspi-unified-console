//! The 14 `custom` rows of `legacy_map.json`: requests with no one-to-one rule,
//! each written by hand from the migration guide's recipe (sections 6.3, 6.5 and
//! 6.7). **Owned by WP PGCUST**; until it lands every custom row is refused
//! locally as [`PlanError::Unsupported`] and has no reverse view.
//!
//! Stub written by WP PGPROTO so the planner can delegate here.

use super::legacy::{Custom, LegacyOp};
use super::plan::{LegacyBody, Plan, PlanError};
use super::reverse::LegacyEquivalent;
use super::{CmdId, PageSet, PagedDir};

/// Plan a custom row (WP PGCUST). A multi-step recipe returns
/// [`Plan::machine`]; a recipe that sends nothing returns [`Plan::local`].
pub(super) fn plan(
    op: &'static LegacyOp,
    _row: &'static Custom,
    _value: u16,
    _body: LegacyBody<'_>,
    _pages: Option<&PageSet>,
) -> Result<Plan, PlanError> {
    Err(PlanError::Unsupported { opcode: op.code })
}

/// The 1.1.6 request a paged request is the custom translation of, if any (WP PGCUST).
pub(super) fn legacy_equivalent(
    _id: CmdId,
    _dir: PagedDir,
    _value: u16,
    _payload: &[u8],
) -> Option<LegacyEquivalent> {
    None
}
