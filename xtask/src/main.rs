//! Developer tasks for the workspace.
//!
//!   cargo run -p xtask -- capi-consts           write crates/dspi-capi/src/types/consts.rs
//!   cargo run -p xtask -- capi-consts --check   exit 1 if the committed file is stale
//!
//! `ci/drift.sh` runs the second form (and the first under `write`).

mod capi_consts;

use std::path::PathBuf;
use std::process::ExitCode;

fn workspace_root() -> PathBuf {
    // xtask/ sits directly under the workspace root.
    let here = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    here.parent().map(PathBuf::from).unwrap_or(here)
}

fn usage() -> ExitCode {
    eprintln!("usage: cargo run -p xtask -- capi-consts [--check]");
    ExitCode::from(2)
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    match args.as_slice() {
        ["capi-consts"] => capi_consts::run(&workspace_root(), false),
        ["capi-consts", "--check"] => capi_consts::run(&workspace_root(), true),
        _ => usage(),
    }
}
