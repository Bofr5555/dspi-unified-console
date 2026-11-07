//! [`DspiError`]: one typed error for the whole embedder API.
//!
//! It replaces the old C ABI's `bool` / `0xFF` / `-1` / status-byte
//! conventions; the C ABI maps it back onto them (`dspi-capi`).

use dspi_session::WriteError;
use dspi_transport::TransportError;

/// Why a [`crate::DspiCore`] call failed.
#[cfg_attr(feature = "uniffi", derive(uniffi::Error))]
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum DspiError {
    /// No device is open, or it went away.
    #[error("device not connected")]
    Disconnected,
    /// A transfer failed (timeout, stall, short read, flash blackout).
    #[error("transfer failed: {msg}")]
    Transfer {
        /// What failed, for a log or an error line.
        msg: String,
    },
    /// The device answered but refused: the raw status byte (`PRESET_*`,
    /// `PIN_CONFIG_*`, `FLASH_*`, or a command-specific code).
    #[error("device refused the request (status {status})")]
    Refused {
        /// The status byte, never 0.
        status: u8,
    },
    /// Bad argument (index out of range, malformed JSON op, ...).
    #[error("invalid argument: {msg}")]
    InvalidArgument {
        /// Which argument, and why.
        msg: String,
    },
    /// The firmware or transport does not have this feature.
    #[error("not supported by the connected firmware")]
    Unsupported,
    /// The device's wire format is not this build's: its state is shown
    /// read-only and nothing may be written.
    #[error("the connected firmware is read-only in this version")]
    ReadOnly,
    /// DSPi Link: another console holds the device lock.
    #[error("another console is applying changes")]
    Locked,
    /// DSPi Link: this session's role may not send that command.
    #[error("this console's role on the hub does not allow that")]
    Denied,
    /// DSPi Link session failure. `code`: `wrong_pin`, `rate_limited`,
    /// `hub_busy`, `handshake_refused`, `io`, `handshake`, `protocol`,
    /// `timeout`, `closed`, or another spec 7.1 error code. `msg` can be shown.
    #[error("DSPi Link: {msg}")]
    Link {
        /// The error code, as above.
        code: String,
        /// The hub's or the client's words, for the user.
        msg: String,
    },
    /// A file could not be read or written (presets, firmware images).
    #[error("{msg}")]
    Io {
        /// What failed, with the operating system's reason.
        msg: String,
    },
    // Work-package sections: add variants only inside your own section.
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
    // -- WP CG6 --
    /// An AutoEQ database update failed (`crate::autoeq`). `code`:
    /// `rate_limited`, `http_status`, `network`, `bad_content`, `no_profiles`,
    /// `too_many_failures`, `cancelled`, `stopped` or `missing`. `msg` can be
    /// shown.
    #[error("{msg}")]
    AutoEq {
        /// The failure, as above.
        code: String,
        /// The words for the user.
        msg: String,
        /// With `rate_limited`: seconds until it is worth trying again, when
        /// the server said.
        retry_after_s: Option<u32>,
    },
    // -- WP CG8 --
    /// The firmware guard stopped an image that can leave the board at too low
    /// a core voltage (`firmware_guard_check`); nothing was written.
    #[error("{msg}")]
    FirmwareGuardBlocked {
        /// The guard's words, for the user.
        msg: String,
    },
    // -- WP CG14 --
    /// This raises the core voltage above 1.20 V from what the unit reports
    /// now, and the word `OVERVOLT` was not given. Nothing was written.
    #[error("{msg}")]
    OvervoltWordRequired {
        /// Which voltage, and what to type.
        msg: String,
    },
}

/// The result of a [`crate::DspiCore`] call.
pub type Result<T> = std::result::Result<T, DspiError>;

// A foreign callback (`MidiSender`) that throws something other than a
// `DspiError`, or whose call could not complete.
#[cfg(feature = "uniffi")]
impl From<uniffi::UnexpectedUniFFICallbackError> for DspiError {
    fn from(e: uniffi::UnexpectedUniFFICallbackError) -> Self {
        DspiError::Transfer { msg: e.reason }
    }
}

impl From<TransportError> for DspiError {
    fn from(e: TransportError) -> Self {
        match e {
            TransportError::Disconnected
            | TransportError::NotFound
            | TransportError::SerialNotFound(_) => DspiError::Disconnected,
            TransportError::Locked => DspiError::Locked,
            TransportError::Denied(_) => DspiError::Denied,
            other => DspiError::Transfer {
                msg: other.to_string(),
            },
        }
    }
}

impl From<WriteError> for DspiError {
    fn from(e: WriteError) -> Self {
        match e {
            WriteError::Transport(t) => t.into(),
            WriteError::Unavailable { .. } => DspiError::Unsupported,
            other => DspiError::InvalidArgument {
                msg: other.to_string(),
            },
        }
    }
}

/// Map a device status byte (0 = success) to `Result<()>`.
pub(crate) fn status(code: u8) -> Result<()> {
    if code == 0 {
        Ok(())
    } else {
        Err(DspiError::Refused { status: code })
    }
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests {
    use super::*;

    #[test]
    fn transport_errors_map_to_typed_errors() {
        assert_eq!(
            DspiError::from(TransportError::Disconnected),
            DspiError::Disconnected
        );
        assert_eq!(DspiError::from(TransportError::Locked), DspiError::Locked);
        assert_eq!(
            DspiError::from(TransportError::Denied("x".into())),
            DspiError::Denied
        );
        assert!(matches!(
            DspiError::from(TransportError::Stalled { opcode: 1 }),
            DspiError::Transfer { .. }
        ));
    }

    #[test]
    fn status_bytes_map_to_refused() {
        assert_eq!(status(0), Ok(()));
        assert_eq!(status(2), Err(DspiError::Refused { status: 2 }));
    }
}
