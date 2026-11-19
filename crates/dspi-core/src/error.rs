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
    /// `timeout`, `closed`, `token_without_hub_id` (-- WP LINKFIX --), or another
    /// spec 7.1 error code. `msg` can be shown.
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
    // -- WP P1TK --
    /// A dangerous call (`crate::confirm`) was not let through: tokens are
    /// required and none was given, or the one given is not good for it.
    /// Nothing was sent. `code`: `missing`, `unknown`, `used`, `wrong_op`,
    /// `expired` or `connection_changed` (`crate::confirm::refusal`).
    #[error("{msg}")]
    ConfirmationRequired {
        /// Why, as above.
        code: String,
        /// Which operation and why, for a log or a detail line.
        msg: String,
    },
    // -- WP P1LM --
    /// The write would cross a limit of the device's limits profile
    /// (`crate::limits`): nothing was sent. `code`: `out_gain`, `eq_boost`,
    /// `highpass`, `delay`, `limiter` or `master_volume`.
    #[error("{msg}")]
    LimitExceeded {
        /// Which limit, as above.
        code: String,
        /// What and why, for a log or a detail line.
        msg: String,
    },
    // -- WP P1LK --
    /// The installer's lock on this console (`crate::limits`, S4): nothing was
    /// sent. `code`: `locked` (the locked-user state allows only volume, mute
    /// and preset), `pin_required` (an installer action needs the unlock),
    /// `wrong_pin`, `wait` (too many wrong PINs: the refusal window is open;
    /// `installer_lock_state().retry_after_ms` says how long), `no_pin`, or
    /// `hub` (WP MERGEP1D: the hub this console reaches the device through
    /// holds the lock and refused the write).
    #[error("{msg}")]
    InstallerLock {
        /// Why, as above.
        code: String,
        /// What and why, for a log or a detail line.
        msg: String,
    },
    // -- WP P1DKFIX --
    /// DSPi Link: the hub refused a whole-state write (an import, a snapshot
    /// recall, an A/B toggle, a guarded apply) from this console's role: the
    /// device has a limits profile or an installer lock on the hub, and only
    /// an Admin may write its whole state. Nothing was sent.
    #[error("{msg}")]
    ImageRefused {
        /// The hub's words, for a log or a detail line.
        msg: String,
    },
    // -- WP P1DKFIX --
    /// DSPi Link: the hub could not read the device's state to judge this
    /// write against its limits profile, so it refused it. Nothing was sent;
    /// trying again may pass.
    #[error("{msg}")]
    LimitsUnknown {
        /// The hub's words, for a log or a detail line.
        msg: String,
    },
}

// -- WP P1NT --
impl DspiError {
    /// The variant's stable key, before any refinement by `code`: the C ABI's
    /// failure kinds, with DSPi Link as `dlink`. Keys are never reused.
    pub(crate) fn kind_key(&self) -> &'static str {
        match self {
            Self::Disconnected => "disconnected",
            Self::Transfer { .. } => "transfer",
            Self::Refused { .. } => "refused",
            Self::InvalidArgument { .. } => "invalid",
            Self::Unsupported => "unsupported",
            Self::ReadOnly => "read_only",
            Self::Locked => "locked",
            Self::Denied => "denied",
            Self::Link { .. } => "dlink",
            Self::Io { .. } => "io",
            Self::AutoEq { .. } => "autoeq",
            Self::FirmwareGuardBlocked { .. } => "firmware_guard",
            Self::OvervoltWordRequired { .. } => "overvolt_word",
            // -- WP P1TK --
            Self::ConfirmationRequired { .. } => "confirmation_required",
            // -- WP P1LM --
            Self::LimitExceeded { .. } => "limit_exceeded",
            // -- WP P1LK --
            Self::InstallerLock { .. } => "installer_lock",
            // -- WP P1DKFIX --
            Self::ImageRefused { .. } => "image_refused",
            Self::LimitsUnknown { .. } => "limits_unknown",
        }
    }

    /// The stable key of this error's notice (`crate::notice_text`): the
    /// variant's key, or `dlink.<code>` / `autoeq.<code>` /
    /// `installer_lock.<code>` where the table has a row for that code.
    pub fn notice_key(&self) -> String {
        let kind = self.kind_key();
        let code = match self {
            Self::Link { code, .. } | Self::AutoEq { code, .. } => Some(code.as_str()),
            Self::InstallerLock { code, .. } => Some(code.as_str()), // -- WP P1LK --
            // -- WP PGWIRE -- `paged::open`'s refusals keep their words
            Self::Transfer { msg } => dspi_transport::paged::Refusal::of_text(msg)
                .map(|r| r.code())
                .or_else(|| link_notice_code(msg)), // -- WP PGCORE --
            _ => None,
        };
        code.map(|c| format!("{kind}.{c}"))
            .filter(|k| crate::notice_text::has_row(k))
            .unwrap_or_else(|| kind.to_owned())
    }

    /// The error's own words, for a notice's detail line: the message a variant
    /// carries, the status byte of a refusal, or nothing.
    pub fn notice_detail(&self) -> String {
        match self {
            Self::Transfer { msg }
            | Self::InvalidArgument { msg }
            | Self::Link { msg, .. }
            | Self::Io { msg }
            | Self::AutoEq { msg, .. }
            | Self::FirmwareGuardBlocked { msg }
            | Self::OvervoltWordRequired { msg }
            | Self::ConfirmationRequired { msg, .. }
            | Self::LimitExceeded { msg, .. }
            | Self::InstallerLock { msg, .. } // -- WP P1LK --
            | Self::ImageRefused { msg } // -- WP P1DKFIX --
            | Self::LimitsUnknown { msg } => msg.clone(),
            Self::Refused { status } => format!("status {status}"),
            Self::Disconnected
            | Self::Unsupported
            | Self::ReadOnly
            | Self::Locked
            | Self::Denied => String::new(),
        }
    }
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

// -- WP P1DKFIX -- F5
/// The limits a hub names in a `limit_exceeded` refusal (P1LM's codes).
const LIMIT_CODES: [&str; 6] = [
    "out_gain",
    "eq_boost",
    "highpass",
    "delay",
    "limiter",
    "master_volume",
];

// -- WP P1DKFIX -- F5
/// A hub's named refusal (`TransportError::hub_refusal`) as the core's own
/// error, so a client shows the same notice over Link as over USB.
fn hub_refusal(e: &TransportError) -> Option<DspiError> {
    let (key, msg) = e.hub_refusal()?;
    let msg = msg.to_owned();
    Some(match key {
        "limit_exceeded" => DspiError::LimitExceeded {
            code: e
                .hub_refusal_limit()
                .filter(|l| LIMIT_CODES.contains(l))
                .unwrap_or("limit")
                .to_owned(),
            msg,
        },
        // -- WP MERGEP1D -- a hub's lock refusal: the hub holds the lock, so
        // the notice is `installer_lock.hub`, not the "on this console" row.
        "installer_lock" => DspiError::InstallerLock {
            code: "hub".into(),
            msg,
        },
        "image_refused" => DspiError::ImageRefused { msg },
        "limits_unknown" => DspiError::LimitsUnknown { msg },
        _ => return None,
    })
}

// -- WP PGCORE --
/// The code of the DSPi Link notice whose words are `msg`
/// ([`dspi_transport::link::LinkNotice`]): its key is `transfer.<code>`.
#[cfg(feature = "link")]
fn link_notice_code(msg: &str) -> Option<&'static str> {
    dspi_transport::link::LinkNotice::of_text(msg).map(|n| n.code())
}

// -- WP PGCORE --
/// A build without DSPi Link has no Link notices.
#[cfg(not(feature = "link"))]
fn link_notice_code(_msg: &str) -> Option<&'static str> {
    None
}

impl From<TransportError> for DspiError {
    fn from(e: TransportError) -> Self {
        if let Some(mapped) = hub_refusal(&e) {
            return mapped; // -- WP P1DKFIX -- F5
        }
        // -- WP PGCORE -- a DSPi Link notice keeps its words exactly, so its
        // notice row is found by them (`notice_key`)
        #[cfg(feature = "link")]
        if let Some(n) = dspi_transport::link::LinkNotice::of(&e) {
            return DspiError::Transfer {
                msg: n.text().to_owned(),
            };
        }
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
            // -- WP LK4 --
            WriteError::Unprobed { .. } => DspiError::Link {
                code: "still_reading".into(),
                msg: "Still reading the device; try again in a moment.".into(),
            },
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

    // -- WP P1DKFIX -- F5 (review p1d-core probe, adopted)
    #[test]
    fn wp_p1dkfix_a_hub_refusal_keeps_its_notice_in_the_core() {
        let e = DspiError::from(TransportError::Denied(
            "limit_exceeded/out_gain: output 1 gain 6 dB".into(),
        ));
        assert_eq!(
            e,
            DspiError::LimitExceeded {
                code: "out_gain".into(),
                msg: "output 1 gain 6 dB".into()
            }
        );
        assert_eq!(e.notice_key(), "limit_exceeded");
        // No limit named, or one this core does not know: still the notice.
        for text in ["limit_exceeded: x", "limit_exceeded/rm_rf: x"] {
            let e = DspiError::from(TransportError::Denied(text.into()));
            assert!(
                matches!(&e, DspiError::LimitExceeded { code, .. } if code == "limit"),
                "{e:?}"
            );
        }
        let e = DspiError::from(TransportError::Denied("installer_lock: locked".into()));
        assert_eq!(
            e,
            DspiError::InstallerLock {
                code: "hub".into(),
                msg: "locked".into()
            }
        );
        assert_eq!(e.notice_key(), "installer_lock.hub"); // -- WP MERGEP1D --
        let e = DspiError::from(TransportError::Denied("image_refused: x".into()));
        assert_eq!(e, DspiError::ImageRefused { msg: "x".into() });
        assert_eq!(e.notice_key(), "image_refused");
        let e = DspiError::from(TransportError::Denied("limits_unknown: y".into()));
        assert_eq!(e, DspiError::LimitsUnknown { msg: "y".into() });
        assert_eq!(e.notice_key(), "limits_unknown");
        assert_eq!(e.notice_detail(), "y");
        // A role's plain denial stays one.
        assert_eq!(
            DspiError::from(TransportError::Denied("opcode 0x74 is not allowed".into())),
            DspiError::Denied
        );
    }

    #[test]
    fn status_bytes_map_to_refused() {
        assert_eq!(status(0), Ok(()));
        assert_eq!(status(2), Err(DspiError::Refused { status: 2 }));
    }
}

// -- WP PGWIRE --
#[cfg(test)]
mod tests_wp_pgwire {
    use super::*;
    use dspi_transport::paged::Refusal;

    /// Each refusal of `paged::open` has its own notice row, and its words
    /// are the detail.
    #[test]
    fn wp_pgwire_generation_refusals_have_their_notices() {
        // -- WP PGADAPT -- `PagedNotYet` is gone (a paged device is controlled)
        for r in Refusal::ALL {
            let e = DspiError::from(r.error());
            let key = format!("transfer.{}", r.code());
            assert_eq!(e.notice_key(), key);
            assert!(crate::notice_text::has_row(&key), "{key}");
            assert_eq!(e.notice_detail(), r.text());
            let n = crate::notice_text::notice_for(e);
            assert_eq!(n.key, key);
            assert!(!n.title.is_empty() && !n.action.is_empty());
        }
        // Another incompatibility keeps the plain key.
        let other = DspiError::from(TransportError::Incompatible("wire V12".into()));
        assert_eq!(other.notice_key(), "transfer");
    }
}

// -- WP PGCORE --
#[cfg(all(test, feature = "link"))]
mod tests_wp_pgcore {
    use super::*;
    use dspi_transport::link::LinkNotice;

    /// Each DSPi Link notice of the client (PGLINKC) has its own row, found
    /// by its words through `LinkNotice::of`, and its words are the detail.
    #[test]
    fn wp_pgcore_the_link_notices_have_their_rows() {
        for n in LinkNotice::ALL {
            let e = DspiError::from(n.error());
            let key = format!("transfer.{}", n.code());
            assert_eq!(e.notice_key(), key);
            assert!(crate::notice_text::has_row(&key), "{key}");
            assert_eq!(e.notice_detail(), n.text());
            let notice = crate::notice_text::notice_for(e);
            assert_eq!(notice.key, key);
            assert!(!notice.title.is_empty() && !notice.action.is_empty());
        }
    }
}
