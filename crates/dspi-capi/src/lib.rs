//! The C ABI the Qt desktop app links: the old desktop core's functions
//! (`crates/dspi-core-linux`, header `dspi_core.h`), same names, same
//! signatures, same `#[repr(C)]` records, now over [`dspi_core::DspiCore`].
//!
//! Rules for every `extern "C"` function here:
//! - **No unwinding across the boundary.** The body runs inside [`guard`],
//!   which catches a panic and returns the function's failure value (`false`,
//!   `0xFF`, `-1`, a zeroed record), as the old ABI's conventions say.
//! - **Null and length checks first.** Every pointer argument may be null;
//!   every buffer comes with its length; strings are NUL-terminated UTF-8
//!   (invalid UTF-8 is an invalid argument, not a panic).
//! - **Copy out.** Records are copied into caller-owned memory. The one
//!   exception kept from the old ABI is `dspi_get_state`, which returns a
//!   pointer to a `DspState` owned by the core handle, valid until the next
//!   call on that handle from the same thread (the Qt app's usage).
//! - Errors map onto the old conventions: `bool` false, a `PRESET_*` /
//!   `PIN_CONFIG_*` status byte from `DspiError::Refused`, `0xFF` otherwise.
//!
//! Layout: `types.rs` holds the `#[repr(C)]` records and their conversions
//! from `dspi_core::records` (WP Q1); `fns_device.rs` (WP Q2a) and
//! `fns_misc.rs` (WP Q2b) the functions; `build.rs` writes
//! `include/dspi_core.h` with cbindgen (WP Q1).

// The core-merge quality bar (docs/core-merge/PLAN.md §1.7, §1.8): no
// panicking shortcuts on outside data. clippy.toml exempts test code from the
// first three; test modules allow arithmetic_side_effects themselves.
// missing_docs is turned on by WP F1, which documents every record.
#![warn(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

use std::panic::{AssertUnwindSafe, catch_unwind};

mod fns_device;
mod fns_misc;
pub mod types;

/// Run `f`, turning a panic into `fail`. Every exported function goes
/// through this.
#[allow(dead_code)] // used by every export once WPs Q2a/Q2b land
pub(crate) fn guard<T>(fail: T, f: impl FnOnce() -> T) -> T {
    catch_unwind(AssertUnwindSafe(f)).unwrap_or_else(|_| {
        log::error!("dspi-capi: a call panicked; returning its failure value");
        fail
    })
}

/// The handle behind the C ABI's `struct FfiCore *`: created by
/// `dspi_core_new` and freed by `dspi_core_free`.
pub struct FfiCore {
    pub(crate) core: std::sync::Arc<dspi_core::DspiCore>,
    /// The record `dspi_get_state` points at, refreshed on each call.
    #[allow(dead_code)] // read only by dspi_get_state
    pub(crate) state: std::sync::Mutex<Box<types::DspState>>,
}

/// Run `f` on the handle `p` inside [`guard`]; `fail` for a null handle or a
/// panic.
pub(crate) fn with_core<T>(p: *const FfiCore, fail: T, f: impl FnOnce(&FfiCore) -> T) -> T {
    if p.is_null() {
        return fail;
    }
    // SAFETY: a non-null handle comes from `dspi_core_new` and stays valid
    // until `dspi_core_free`; that is the C caller's contract.
    guard(fail, || f(unsafe { &*p }))
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests {
    use super::*;

    #[test]
    fn a_panic_becomes_the_failure_value() {
        assert!(!guard(false, || panic!("boom")));
        assert_eq!(guard(0xFFu8, || 3), 3);
    }

    #[test]
    #[ignore = "WP Q3"]
    fn wp_q3_every_old_function_is_exported() {
        todo!("WP Q3: compare the exported names with crates/dspi-core-linux's header")
    }
}
