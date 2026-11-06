//! Named values for the raw codes the records carry (`COMPAT_*`,
//! `NOTIFY_*` flags of `process_notifications`, ...), where the firmware
//! headers (`dspi_proto::generated`) do not already name them.
//!
//! The old desktop core's names are kept, so the C header and the Kotlin
//! `DspiConstants` keep matching. Shared file: add constants, never change
//! one (reviewed per wave).

/// `DspState.compat`: not judged yet.
pub const COMPAT_UNKNOWN: u8 = 0;
/// Wire format and firmware match this core.
pub const COMPAT_OK: u8 = 1;
/// Firmware older than this core's wire format (shown read-only).
pub const COMPAT_FIRMWARE_TOO_OLD: u8 = 2;
/// Firmware newer than this core's wire format (shown read-only).
pub const COMPAT_FIRMWARE_TOO_NEW: u8 = 3;

// Work-package sections: append only inside your own section.
// -- WP C0 --
// -- WP C1 --
// -- WP C1b --
// -- WP C2 --
// -- WP C3 --
// -- WP C4 --
// -- WP C5 --
// -- WP C6 --
// -- WP C7 --
// -- WP C8 --
// -- WP C9 --
// -- WP C10 --
// -- WP C11 --
// -- WP C12 --
// -- WP C13 --
// -- WP C14 --
// -- WP C15 --
// -- WP F1 --
