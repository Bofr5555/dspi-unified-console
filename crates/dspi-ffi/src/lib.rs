//! `libdspi_ffi`: the DSPi core for Kotlin (Android) and Swift (iOS), through
//! UniFFI proc-macros.
//!
//! The interface lives next to the code it describes: `dspi-core`'s `uniffi`
//! feature derives the records and the error and exports [`dspi_core::DspiCore`],
//! the listeners and the free functions, in the UniFFI namespace `dspi_core`.
//! This crate is the library that carries them, and adds [`init_logging`] in
//! its own namespace `dspi_ffi`, so the core does not depend on a logging
//! backend. Each namespace generates its own file and needs its own package
//! (see the `uniffi.toml` files):
//!
//! ```text
//! cargo run -p dspi-ffi --features cli --bin uniffi-bindgen -- \
//!     generate --library libdspi_ffi.so --crate dspi_core \
//!     --config crates/dspi-core/uniffi.toml --language kotlin --out-dir out
//! ```
//!
//! and again with `--crate dspi_ffi --config crates/dspi-ffi/uniffi.toml`. Name
//! the config until WP Q3 deletes the old core, whose library is also called
//! `dspi_core`; afterwards `--library` alone finds both.
//!
//! # Calling it from an app
//!
//! - **Off the UI thread.** A call that touches the device blocks for its
//!   transfers. `poll_hotplug` is the one that looks harmless: a reconnect runs
//!   inside it, so a poll after a replug blocks for the whole connect sequence.
//! - **Listeners run on core threads** (`dspi_core::callbacks`): they schedule
//!   work and return. Android's `poll_hotplug` only closes a connection whose
//!   stream ended, since USB broadcasts replace the bus scan there.
//! - **Panics** in the core come out as UniFFI's internal error, not an abort.
//!   Foreign callbacks that throw are contained.
//! - **Link** bindings are on: this library always builds `dspi-core/link`.

#![warn(missing_docs)]

uniffi::setup_scaffolding!();

// The linker keeps the core's scaffolding symbols only when something in this
// library names them.
dspi_core::uniffi_reexport_scaffolding!();

/// Send the core's `log` output to logcat on Android and to stderr elsewhere,
/// at level Info. Call it once at startup; further calls change nothing.
#[uniffi::export]
pub fn init_logging() {
    #[cfg(target_os = "android")]
    android_logger::init_once(
        android_logger::Config::default()
            .with_max_level(log::LevelFilter::Info)
            .with_tag("dspi-core"),
    );
    #[cfg(not(target_os = "android"))]
    {
        static STDERR: StderrLog = StderrLog;
        // An error means a logger is already set, by an earlier call or by
        // the embedder: keep it.
        if log::set_logger(&STDERR).is_ok() {
            log::set_max_level(log::LevelFilter::Info);
        }
    }
}

/// `log` lines on stderr, the one sink a desktop test host or an iOS console
/// has without another dependency.
#[cfg(not(target_os = "android"))]
struct StderrLog;

#[cfg(not(target_os = "android"))]
impl log::Log for StderrLog {
    fn enabled(&self, metadata: &log::Metadata<'_>) -> bool {
        metadata.level() <= log::Level::Info
    }

    fn log(&self, record: &log::Record<'_>) {
        if self.enabled(record.metadata()) {
            eprintln!(
                "[{}] {}: {}",
                record.level(),
                record.target(),
                record.args()
            );
        }
    }

    fn flush(&self) {}
}

#[cfg(test)]
mod tests_wp_f1 {
    use super::*;

    #[test]
    fn wp_f1_init_logging_can_be_called_twice() {
        init_logging();
        init_logging();
        log::info!("wp_f1: stderr logger installed");
        assert_eq!(log::max_level(), log::LevelFilter::Info);
    }
}
