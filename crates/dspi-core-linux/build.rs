use std::env;
use std::path::PathBuf;

fn main() {
    let crate_dir = env::var("CARGO_MANIFEST_DIR").unwrap();
    // The crate's own include/ (crates/dspi-core-linux/include/dspi_core.h)
    let output_dir = PathBuf::from(&crate_dir).join("include");

    std::fs::create_dir_all(&output_dir).expect("failed to create include/");

    // cbindgen runs `cargo metadata`, which would apply the target's rustc
    // flags to the host: cross builds such as cargo-xwin's (MSVC from Linux)
    // pass a linker flavour the host rejects. The header doesn't depend on them.
    for (key, _) in env::vars() {
        if key == "RUSTFLAGS" || key == "CARGO_ENCODED_RUSTFLAGS"
            || (key.starts_with("CARGO_TARGET_") && key.ends_with("_RUSTFLAGS"))
        {
            env::remove_var(key);
        }
    }

    cbindgen::Builder::new()
        .with_crate(&crate_dir)
        .with_config(cbindgen::Config::from_file("cbindgen.toml").unwrap())
        .generate()
        .expect("cbindgen failed")
        .write_to_file(output_dir.join("dspi_core.h"));
}
