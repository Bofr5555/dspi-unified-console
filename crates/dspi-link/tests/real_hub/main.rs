//! WP XC3: the scripted parts of LINK-HUB-FINDINGS.md section 5, against a real
//! hub when asked to and always against the fake hub, with the same assertions.
//!
//! `fake_*` tests need the `fake-hub` feature and never read the environment.
//! `real_*` tests do nothing unless `DSPI_REAL_HUB` is set.
//!
//! | Variable | Meaning |
//! |---|---|
//! | `DSPI_REAL_HUB` | `host` or `host:port`. Unset: every `real_*` test skips. |
//! | `DSPI_REAL_HUB_TOKEN_FILE` | The harness's token, on a hub that wants one. |
//! | `DSPI_REAL_HUB_PIN` | The hub's PIN, for the pairing tests. Never logged. |
//! | `DSPI_REAL_HUB_ALLOW_PAIR=1` | May add client records (table of 16 on a bridge; a full table evicts someone). |
//! | `DSPI_REAL_HUB_ALLOW_LOCKOUT=1` | May run what shuts others out: wrong PINs, all slots full. |
//! | `DSPI_REAL_HUB_REVOKE=1` | `real_teardown_revoke` may revoke the harness's own records. |
//! | `DSPI_REAL_HUB_NO_BOARD=1` | The operator says no board is attached or its UART is off. |
//! | `DSPI_REAL_HUB_OPERATOR=1` | Someone is there to cut board power when prompted (`--nocapture`). |
//! | `DSPI_REAL_HUB_REPORT` | Append the JSON-lines measurements to this file. |
//!
//! Read-only by construction ([`guard`]): the harness sends only GETs that every
//! client sends at attach, takes no lock, subscribes to nothing, and has no way
//! to send audio. `self_checks` holds it to that, and each scenario compares the
//! board's settings image before and after. Pairing asks for the `viewer` role.
//!
//! Timing: every duration is written in real-hub seconds. The fake runs them ten
//! times faster, so the same bounds run in seconds rather than minutes.
//!
//! Real-hub tests take one lock, so they run one at a time whatever
//! `--test-threads` says.
//!
//! Run through `ci/run.sh`, with the variables inside the quoted command, and the
//! hub by IP address (no mDNS in the container). The filter is `real::`, with the
//! token file under `/work` so it survives between runs:
//!
//! 1. `real::real_setup_pair` (`ALLOW_PAIR`, `PIN`, `TOKEN_FILE`) once, on a PIN hub;
//! 2. `real::` for the safe set, then again with `ALLOW_LOCKOUT` and `ALLOW_PAIR`;
//! 3. `real::real_teardown_revoke` (`REVOKE`, `TOKEN_FILE`).
//!
//! Each pairing scenario spends one PIN on a window hub: open a window per test.
//! A lockout scenario shuts out every client behind the container's address (for
//! a rootless container, the host's) for about a minute.
//!
//! Not scripted: a physical Wi-Fi pull and the phone's UI (item 3), web UI tabs
//! (4), a DHCP change (5), a preset save (6), and Doze (9).

#![allow(dead_code)] // The fake's helpers are unused without the `fake-hub` feature

mod guard;
mod proxy;
mod real;
mod scenarios;
mod self_checks;
mod target;

#[cfg(feature = "fake-hub")]
mod fake;
