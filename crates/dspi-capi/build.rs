//! Writes the C header `include/dspi_core.h` with cbindgen (WP Q1), as the
//! old desktop core's build script did.

use std::env;
use std::path::PathBuf;

fn main() {
    // A build script reports failure by panicking; these are the only ways
    // it can fail, and each message says what went wrong.
    #![allow(clippy::expect_used)]

    let crate_dir = env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR is set by cargo");
    let crate_dir = PathBuf::from(crate_dir);
    let output_dir = crate_dir.join("include");
    std::fs::create_dir_all(&output_dir).expect("failed to create include/");

    println!("cargo:rerun-if-changed=cbindgen.toml");
    println!("cargo:rerun-if-changed=src");

    // cbindgen runs `cargo metadata`, which would apply the target's rustc
    // flags to the host: cross builds such as cargo-xwin's (MSVC from Linux)
    // pass a linker flavour the host rejects. The header does not depend on
    // them.
    for (key, _) in env::vars() {
        if key == "RUSTFLAGS"
            || key == "CARGO_ENCODED_RUSTFLAGS"
            || (key.starts_with("CARGO_TARGET_") && key.ends_with("_RUSTFLAGS"))
        {
            // SAFETY: the build script is single-threaded; nothing reads the
            // environment concurrently.
            unsafe { env::remove_var(key) };
        }
    }

    let config = cbindgen::Config::from_file(crate_dir.join("cbindgen.toml"))
        .expect("cbindgen.toml is valid");
    cbindgen::Builder::new()
        .with_crate(&crate_dir)
        .with_config(config)
        .generate()
        .expect("cbindgen failed")
        .write_to_file(output_dir.join("dspi_core.h"));
}
