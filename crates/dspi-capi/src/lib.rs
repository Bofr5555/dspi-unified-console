//! The C ABI the Qt desktop app links: the retired desktop core's functions
//! (header `dspi_core.h`, committed copy in `ci/generated/dspi_core.h`), same
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
//! `fns_misc.rs` (WP Q2b) the functions, `fns_browse.rs` (WP CG2) finding
//! hubs, and `fns_hub.rs` (WP HB6) being one; `build.rs` writes
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
// -- WP CG2 --
mod fns_browse;
pub mod types;
// -- WP CG1 --
#[cfg(any(feature = "link", feature = "midir"))]
mod fns_link;
// -- WP HB6 --
#[cfg(feature = "hub")]
mod fns_hub;

/// Run `f`, turning a panic into `fail`. Every exported function goes
/// through this.
#[allow(dead_code)] // used by every export once WPs Q2a/Q2b land
pub(crate) fn guard<T>(fail: T, f: impl FnOnce() -> T) -> T {
    catch_unwind(AssertUnwindSafe(f)).unwrap_or_else(|_| {
        log::error!("dspi-capi: a call panicked; returning its failure value");
        fail
    })
}

// -- WP HB6 --
/// The logger's filter for this library: `RUST_LOG`, except that `tungstenite` stops at
/// debug. Its trace level prints every WebSocket message whole, and those carry PINs and
/// tokens, so `RUST_LOG=trace` must not put them on stderr.
pub(crate) fn logger_builder(mut builder: env_logger::Builder) -> env_logger::Builder {
    builder.filter_module("tungstenite", log::LevelFilter::Debug);
    builder
}

/// Install the logger: stderr, level from `RUST_LOG`, errors by default. A second core or
/// hub, or a host's own logger, keeps the logger there is.
pub(crate) fn init_logger() {
    let env = env_logger::Env::default();
    let _ = logger_builder(env_logger::Builder::from_env(env)).try_init();
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
    guard(fail, || {
        // SAFETY: a non-null handle comes from `dspi_core_new` and stays valid
        // until `dspi_core_free`; that is the C caller's contract.
        let h = unsafe { &*p };
        // -- WP ROMODEF -- the core of this call, for its failure's notice key
        let _in_call = fns_device::InCall::enter(&h.core);
        f(h)
    })
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

    /// Every function of the committed header (`ci/generated/dspi_core.h`,
    /// the ABI the Qt app is built against) is defined by an export here and
    /// is in the header the build writes. Signatures: `wp_q2a_*`, `wp_q2b_*`.
    #[test]
    fn wp_q3_every_old_function_is_exported() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let read = |p: &str| std::fs::read_to_string(dir.join(p)).unwrap();
        let old = called_names(&read("../../ci/generated/dspi_core.h"));
        assert!(
            old.len() > 140,
            "the parser lost the committed header: {}",
            old.len()
        );

        let source = [
            "src/fns_device.rs",
            "src/fns_misc.rs",
            "src/fns_misc/autoeq.rs",
            "src/fns_misc/link_text.rs",
            // -- WP P0M --
            "src/fns_misc/events.rs",
            // -- WP P0E --
            "src/fns_misc/diag.rs",
            // -- WP FWX --
            "src/fns_misc/ext_diag.rs",
            // -- WP SW1 --
            "src/fns_misc/alink.rs",
            // -- WP AUSB1 --
            "src/fns_misc/alink_usb.rs",
            // -- WP P1NT --
            "src/fns_misc/notice_text.rs",
            // -- WP P1TK --
            "src/fns_misc/confirm.rs",
            // -- WP P1LM --
            "src/fns_misc/limits.rs",
            // -- WP P1LK --
            "src/fns_misc/modes.rs",
            // -- WP P1OF --
            "src/fns_misc/offline.rs",
            // -- WP P1SA --
            "src/fns_misc/guarded.rs",
            // -- WP HUBADMIN --
            "src/fns_misc/hub_admin.rs",
            // -- WP P2CC1 --
            "src/fns_misc/hub_server.rs",
            // -- WP P2CC2 --
            "src/fns_misc/hub_server_admin.rs",
            // -- WP P2CC3 --
            "src/fns_misc/hub_server_manage.rs",
            // -- WP P3CC --
            "src/fns_misc/hub_server_fleet.rs",
            // -- WP ROMODE --
            "src/fns_misc/read_only.rs",
            // -- WP CG-SWS-a --
            "src/fns_misc/sws_a.rs",
            // -- WP CG-SWS-b --
            "src/fns_misc/pins.rs",
            // -- WP CLIP1 --
            "src/fns_misc/clipboard.rs",
            // -- WP P5DOCK --
            "src/fns_misc/measure_doc.rs",
            // -- WP P5SES --
            "src/fns_misc/measure.rs",
            // -- WP P5LIVE --
            "src/fns_misc/measure_live.rs",
            // -- WP P5HUB --
            "src/fns_misc/hub_server_measure.rs",
            // -- WP P5DEMO --
            "src/fns_misc/measure_sim.rs",
            // -- WP P5TUNE --
            "src/fns_misc/measure_tune.rs",
            // -- WP PGCORE --
            "src/fns_misc/protocol.rs",
            // -- WP P5CURV --
            "src/fns_misc/measure_curv.rs",
            "src/fns_browse.rs",
            "src/fns_link.rs",
            "src/fns_hub.rs",
        ]
        .map(read)
        .concat();
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

// -- WP CLEAN1 fixup cl1 --
#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_clean1 {
    use std::path::Path;

    const ANCHOR: &str = "tests/data/abi-anchor.txt";

    /// The header's declarations, without comments, whitespace collapsed, one per
    /// line: what an ABI change changes and a doc comment does not.
    fn declarations(header: &str) -> Vec<String> {
        let mut text = String::new();
        let mut rest = header;
        while let Some(start) = rest.find("/*") {
            text.push_str(&rest[..start]);
            rest = rest[start..].split_once("*/").map_or("", |p| p.1);
        }
        text.push_str(rest);
        text.lines()
            .map(|l| l.split("//").next().unwrap_or(""))
            .map(|l| l.split_whitespace().collect::<Vec<_>>().join(" "))
            .filter(|l| !l.is_empty())
            .collect()
    }

    fn generated() -> Vec<String> {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"));
        declarations(&std::fs::read_to_string(dir.join("include/dspi_core.h")).unwrap())
    }

    fn anchored() -> Vec<String> {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"));
        std::fs::read_to_string(dir.join(ANCHOR))
            .unwrap()
            .lines()
            .filter(|l| !l.starts_with("# "))
            .map(str::to_owned)
            .collect()
    }

    /// The C ABI the build generates is the committed anchor, declaration for
    /// declaration: a changed type, field, order, value or symbol fails here.
    #[test]
    fn wp_clean1_the_abi_is_the_anchor() {
        let (now, then) = (generated(), anchored());
        let first = now.iter().zip(&then).position(|(a, b)| a != b);
        assert!(
            now == then,
            "the C ABI changed (header {} lines, anchor {}; first difference at {:?}: {:?} vs {:?}). \
             If intended, regenerate {ANCHOR} (see its header) in the same commit.",
            now.len(),
            then.len(),
            first,
            first.map(|i| &now[i]),
            first.map(|i| &then[i]),
        );
    }

    /// Rewrites the anchor from the current header; run by hand for an intended change.
    #[test]
    #[ignore = "rewrites the anchor; run by hand for an intended ABI change"]
    fn wp_clean1_write_the_abi_anchor() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"));
        let path = dir.join(ANCHOR);
        let old = std::fs::read_to_string(&path).unwrap();
        let head: String = old
            .lines()
            .take_while(|l| l.starts_with("# "))
            .map(|l| format!("{l}\n"))
            .collect();
        std::fs::write(&path, head + &generated().join("\n") + "\n").unwrap();
    }
}
