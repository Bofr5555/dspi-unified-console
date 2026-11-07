//! WP HB7: conformance. Three Link clients against this hub, on real sockets:
//!
//! | Client | Where | Runs |
//! |---|---|---|
//! | ours, `dspi-link` | this binary | `scenario` (the shared list), `faults` (L1's), `fixtures` |
//! | upstream's TypeScript `@dspi/link` 1.2.0 | Node 24 in a container | `ts/conformance.ts`, by `run-clients.sh ts` |
//! | the Swift console's `LinkClient` | the macOS VM | `swift/main.swift`, by `run-clients.sh swift` |
//!
//! The other two talk to `serve::conformance_serve_for_other_clients` (ignored here) and
//! ask the driver for what only the hub's side can do: a new pairing window, a change
//! made at the device, a revoke, a stop. `README.md` has the scenario list all three
//! run, how to run them, and what each last did; the differences from the Swift hub are
//! in `docs/core-merge/HUB.md` section 8.
//!
//! Loopback only, as the other hub tests; the limits are shortened, nothing else.
#![allow(
    clippy::arithmetic_side_effects,
    clippy::unwrap_used,
    clippy::expect_used
)]

mod support;

mod faults;
mod fixtures;
mod scenario;
mod serve;
