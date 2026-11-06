//! The UniFFI bindgen CLI at the library's exact version:
//! `cargo run -p dspi-ffi --features cli --bin uniffi-bindgen -- generate
//! --library <libdspi_ffi.so> --language kotlin --out-dir <dir>`.

fn main() {
    uniffi::uniffi_bindgen_main()
}
