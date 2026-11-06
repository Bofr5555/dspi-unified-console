//! The C ABI the Qt desktop app links: the retired desktop core's functions
//! (header `dspi_core.h`, snapshot in `tests/data/dspi_core_old.h`), same
//! names, same signatures, same `#[repr(C)]` records, now over [`dspi_core::DspiCore`].
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
//!   pointer to a `DspState` owned by the core handle, at a fixed address until
//!   `dspi_core_free`. Each `dspi_get_state` rewrites that record, so readers
//!   serialise with that call (the Qt app: GUI thread only).
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
#![warn(
    missing_docs,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

use std::panic::{AssertUnwindSafe, catch_unwind};

mod fns_device;
mod fns_misc;
pub mod types;
// -- WP CG1 --
#[cfg(any(feature = "link", feature = "midir"))]
mod fns_link;

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

    /// The `dspi_*` names called in `text`, its comments left out.
    fn called_names(text: &str) -> std::collections::BTreeSet<String> {
        let code = text
            .split("/*")
            .enumerate()
            .map(|(i, part)| {
                if i == 0 {
                    part
                } else {
                    part.split_once("*/").map_or("", |p| p.1)
                }
            })
            .collect::<String>();
        code.match_indices("dspi_")
            .filter_map(|(i, _)| {
                let rest = &code[i..];
                let end = rest.find(|c: char| !(c.is_alphanumeric() || c == '_'))?;
                rest[end..].starts_with('(').then(|| rest[..end].to_owned())
            })
            .collect()
    }

    /// Every function of the old core's header (the committed snapshot; the
    /// crate is gone) is defined by an export here and is in the header the
    /// build writes. Signatures: `wp_q2a_*`, `wp_q2b_*`.
    #[test]
    fn wp_q3_every_old_function_is_exported() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let read = |p: &str| std::fs::read_to_string(dir.join(p)).unwrap();
        let old = called_names(&read("tests/data/dspi_core_old.h"));
        assert!(
            old.len() > 140,
            "the parser lost the old header: {}",
            old.len()
        );

        let source = read("src/fns_device.rs") + &read("src/fns_misc.rs");
        let exported: std::collections::BTreeSet<&str> = source
            .split("pub extern \"C\" fn ")
            .skip(1)
            .filter_map(|s| s.split('(').next())
            .collect();
        let header = called_names(&read("include/dspi_core.h"));
        for name in &old {
            assert!(exported.contains(name.as_str()), "{name} is not exported");
            assert!(header.contains(name), "{name} is not in the header");
        }
    }

    #[test]
    fn wp_q3_the_first_failure_logs_and_repeats_are_held_back() {
        let mut t = std::collections::BTreeMap::new();
        let start = std::time::Instant::now();
        assert_eq!(fns_device::warn_now(&mut t, "a", start), Some(0));
        let soon = start + std::time::Duration::from_secs(1);
        assert_eq!(fns_device::warn_now(&mut t, "a", soon), None);
        assert_eq!(fns_device::warn_now(&mut t, "a", soon), None);
        // Another message is its own.
        assert_eq!(fns_device::warn_now(&mut t, "b", soon), Some(0));
        let later = start + std::time::Duration::from_secs(31);
        assert_eq!(fns_device::warn_now(&mut t, "a", later), Some(2));
        assert_eq!(fns_device::warn_now(&mut t, "a", later), None);
    }

    #[test]
    fn wp_q3_the_failure_table_stays_bounded() {
        let mut t = std::collections::BTreeMap::new();
        let now = std::time::Instant::now();
        for i in 0..1000 {
            assert_eq!(fns_device::warn_now(&mut t, &i.to_string(), now), Some(0));
        }
        assert!(t.len() <= 64);
    }
}
