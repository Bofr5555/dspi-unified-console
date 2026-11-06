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

#[cfg(test)]
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
