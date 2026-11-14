//! Confirmation tokens for dangerous operations (WP P1TK, PRO1 W9).
//!
//! A client that has asked the user to confirm a dangerous operation (a flash
//! save, a delete, a factory reset, the bootloader, a configuration import, a
//! stored core voltage, a firmware install) mints a token for that one
//! operation with [`DspiCore::confirm_token`] and passes it to the gated call
//! (`save_preset_confirmed`, `factory_reset_confirmed`, ...). A token is
//!
//! - for one [`ConfirmOp`]: one for another operation is refused;
//! - single use: the gated call spends it when it accepts it, whatever the
//!   call's outcome after that, so a retry needs a new one;
//! - valid for [`CONFIRM_TOKEN_TTL_MS`] (10 s, as Android's contract);
//! - valid on the connection it was minted on: a disconnect, a reconnect or
//!   another device ends it. One minted with no device open is valid only
//!   while no device has been opened since (a firmware install, with the
//!   board in its bootloader).
//!
//! With [`DspiCore::require_confirmation`] off (the default until every client
//! has adopted tokens) a gated call without a token runs as before; a token
//! that is given is always checked, so a client can adopt tokens before it
//! turns the requirement on. With it on, a gated call without a good token is
//! refused with [`DspiError::ConfirmationRequired`] before anything is sent.
//! The plain calls (`save_preset`, `write_param("preset.save")`, ...) are the
//! gated calls with no token.
//!
//! **A token is a discipline, not a security boundary.** The caller that
//! mints it also uses it; over DSPi Link each client's core mints its own,
//! unseen by the hub, whose roles stay the boundary. Snapshot recall, the A/B
//! toggle and a guarded apply are deliberately not gated (A/B must be instant;
//! a guarded apply has its own way back). The core-voltage word `OVERVOLT`
//! (CG14) stays as it is, beside the token.
#![warn(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

use std::time::Duration;

use dspi_transport::clock::{self, Instant};

use crate::write_param::ParamValue;
use crate::{DspiCore, DspiError, Inner, Result};

/// How long a minted token stays valid, in milliseconds.
pub const CONFIRM_TOKEN_TTL_MS: u32 = 10_000;

const TTL: Duration = Duration::from_millis(CONFIRM_TOKEN_TTL_MS as u64);

/// How long a token is remembered after it was minted, so a late use is told
/// "expired" or "used" rather than "unknown".
const REMEMBER: Duration = Duration::from_secs(60);

/// At most this many tokens are remembered; the oldest go first.
const REMEMBER_MAX: usize = 64;

/// An operation that takes a confirmation token.
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ConfirmOp {
    /// Save the live state to a preset slot ([`DspiCore::save_preset_confirmed`]).
    SavePreset,
    /// Copy a preset ([`DspiCore::copy_preset_confirmed`]).
    CopyPreset,
    /// Delete a preset ([`DspiCore::delete_preset_confirmed`]).
    DeletePreset,
    /// Save the live state to the active slot ([`DspiCore::save_params_confirmed`]).
    SaveParams,
    /// Factory reset ([`DspiCore::factory_reset_confirmed`]).
    FactoryReset,
    /// Save the output configuration ([`DspiCore::save_output_config_confirmed`]).
    SaveOutputConfig,
    /// Restart into the bootloader ([`DspiCore::enter_bootloader_confirmed`]).
    EnterBootloader,
    /// Import a configuration document ([`DspiCore::config_import_confirmed`]).
    ConfigImport,
    /// Store a core voltage ([`DspiCore::core_vreg_cal_commit_confirmed`],
    /// [`DspiCore::core_vreg_manual_set_confirmed`]).
    VregSave,
    /// Write a firmware image to the bootloader drive
    /// ([`DspiCore::flash_confirmed`]).
    FirmwareFlash,
}

impl ConfirmOp {
    /// Every operation, in declaration order.
    pub const ALL: [ConfirmOp; 10] = [
        ConfirmOp::SavePreset,
        ConfirmOp::CopyPreset,
        ConfirmOp::DeletePreset,
        ConfirmOp::SaveParams,
        ConfirmOp::FactoryReset,
        ConfirmOp::SaveOutputConfig,
        ConfirmOp::EnterBootloader,
        ConfirmOp::ConfigImport,
        ConfirmOp::VregSave,
        ConfirmOp::FirmwareFlash,
    ];

    /// The operation's stable key, as the C ABI and the web binding name it:
    /// `save_preset`, `factory_reset`, ...
    pub fn key(self) -> &'static str {
        match self {
            ConfirmOp::SavePreset => "save_preset",
            ConfirmOp::CopyPreset => "copy_preset",
            ConfirmOp::DeletePreset => "delete_preset",
            ConfirmOp::SaveParams => "save_params",
            ConfirmOp::FactoryReset => "factory_reset",
            ConfirmOp::SaveOutputConfig => "save_output_config",
            ConfirmOp::EnterBootloader => "enter_bootloader",
            ConfirmOp::ConfigImport => "config_import",
            ConfirmOp::VregSave => "vreg_save",
            ConfirmOp::FirmwareFlash => "firmware_flash",
        }
    }

    /// The operation with this key.
    pub fn from_key(key: &str) -> Option<ConfirmOp> {
        ConfirmOp::ALL.into_iter().find(|op| op.key() == key)
    }
}

/// Why a gated call refused: [`DspiError::ConfirmationRequired`]'s `code`.
pub mod refusal {
    /// No token, and tokens are required.
    pub const MISSING: &str = "missing";
    /// Not a token this core minted (or one forgotten long ago).
    pub const UNKNOWN: &str = "unknown";
    /// Already spent.
    pub const USED: &str = "used";
    /// Minted for another operation.
    pub const WRONG_OP: &str = "wrong_op";
    /// Older than its 10 s.
    pub const EXPIRED: &str = "expired";
    /// The connection changed since it was minted.
    pub const CONNECTION_CHANGED: &str = "connection_changed";
}

/// Where the core stood when a token was minted, or is used.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Place {
    /// The open connection's generation.
    generation: Option<u64>,
    /// With no connection: the generation of the last connection this core
    /// opened (0: none yet), so a device opened and closed again meanwhile in
    /// this core is noticed. Its own core's, never the process-wide counter:
    /// another core in the process connecting does not end the token
    /// (-- WP P1BFIX -- F5).
    last: u64,
}

impl Place {
    fn of(inner: &Inner) -> Self {
        let generation = inner.conn.as_ref().map(|c| c.generation);
        Self {
            generation,
            last: match generation {
                Some(_) => 0,
                None => inner.confirm.last_generation,
            },
        }
    }
}

struct Minted {
    token: String,
    op: ConfirmOp,
    at: Instant,
    place: Place,
    spent: bool,
}

/// The confirmation state of one core: the requirement and the tokens minted.
#[derive(Default)]
pub(crate) struct Confirm {
    required: bool,
    minted: Vec<Minted>,
    count: u64,
    /// The generation of the last connection this core opened (-- WP P1BFIX --).
    last_generation: u64,
}

/// What a gate let through: the connection the call must still be on.
#[must_use]
pub(crate) struct Ticket(Option<Place>);

impl Ticket {
    /// [`DspiError::Disconnected`] if the connection the token was accepted on
    /// has gone. A call let through without a token is never stopped here.
    pub(crate) fn still_on(&self, inner: &Inner) -> Result<()> {
        match self.0 {
            Some(p) if p.generation.is_some() && Place::of(inner).generation != p.generation => {
                Err(DspiError::Disconnected)
            }
            _ => Ok(()),
        }
    }
}

pub(crate) fn refused(op: ConfirmOp, code: &str) -> DspiError {
    let why = match code {
        refusal::MISSING => "no confirmation token was given",
        refusal::USED => "its confirmation token was already used",
        refusal::WRONG_OP => "its confirmation token is for another operation",
        refusal::EXPIRED => "its confirmation token ran out",
        refusal::CONNECTION_CHANGED => "the connection changed since it was confirmed",
        _ => "its confirmation token is not one this console gave",
    };
    DspiError::ConfirmationRequired {
        code: code.to_owned(),
        msg: format!("{} was not done: {why}.", op.key()),
    }
}

/// A 64-bit mix (splitmix64's finaliser): tokens that do not count up.
fn mix(mut z: u64) -> u64 {
    z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

impl Confirm {
    /// This core opened the connection of `generation` (-- WP P1BFIX -- F5).
    pub(crate) fn connected(&mut self, generation: u64) {
        self.last_generation = generation;
    }

    fn forget_old(&mut self, now: Instant) {
        self.minted.retain(|m| {
            now.checked_duration_since(m.at)
                .is_none_or(|age| age <= REMEMBER)
        });
        let excess = self
            .minted
            .len()
            .saturating_sub(REMEMBER_MAX.saturating_sub(1));
        self.minted.drain(..excess);
    }

    /// A new token for `op` at `place`.
    pub(crate) fn mint(&mut self, op: ConfirmOp, now: Instant, place: Place) -> String {
        self.forget_old(now);
        self.count = self.count.wrapping_add(1);
        let wall = clock::system_time()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos() as u64);
        let token = format!(
            "{}-{:016x}{:08x}",
            op.key(),
            mix(wall ^ self.count.rotate_left(40)),
            self.count & 0xFFFF_FFFF
        );
        self.minted.push(Minted {
            token: token.clone(),
            op,
            at: now,
            place,
            spent: false,
        });
        token
    }

    /// Let `op` through with `token`, spending it, or say why not.
    pub(crate) fn accept(
        &mut self,
        op: ConfirmOp,
        token: Option<&str>,
        now: Instant,
        place: Place,
    ) -> Result<Ticket> {
        let Some(token) = token else {
            return if self.required {
                Err(refused(op, refusal::MISSING))
            } else {
                Ok(Ticket(None))
            };
        };
        let Some(m) = self.minted.iter_mut().find(|m| m.token == token) else {
            return Err(refused(op, refusal::UNKNOWN));
        };
        let code = if m.spent {
            refusal::USED
        } else if m.op != op {
            refusal::WRONG_OP
        } else if now
            .checked_duration_since(m.at)
            .is_some_and(|age| age > TTL)
        {
            refusal::EXPIRED
        } else if m.place != place {
            refusal::CONNECTION_CHANGED
        } else {
            m.spent = true;
            return Ok(Ticket(Some(place)));
        };
        Err(refused(op, code))
    }

    /// Age every token by `by`, for tests of the expiry.
    #[cfg(test)]
    pub(crate) fn age_for_test(&mut self, by: Duration) {
        for m in &mut self.minted {
            if let Some(at) = m.at.checked_sub(by) {
                m.at = at;
            }
        }
    }
}

#[cfg_attr(feature = "uniffi", uniffi::export)]
impl DspiCore {
    /// A single-use token for `op`, valid for 10 s on the current connection:
    /// mint it when the user has confirmed `op` (the confirmation dialog's OK),
    /// and pass it to the gated call. No I/O.
    pub fn confirm_token(&self, op: ConfirmOp) -> String {
        let mut inner = self.lock();
        let place = Place::of(&inner);
        inner.confirm.mint(op, clock::Instant::now(), place)
    }

    /// Whether a gated call without a token is refused from now on
    /// ([`DspiError::ConfirmationRequired`]). Off by default. Kept across
    /// connections; minted tokens stay as they are.
    pub fn require_confirmation(&self, on: bool) {
        self.lock().confirm.required = on;
    }

    /// Whether gated calls need a token ([`DspiCore::require_confirmation`]).
    pub fn confirmation_required(&self) -> bool {
        self.lock().confirm.required
    }
}

impl DspiCore {
    /// The gate of a dangerous call: `token` accepted (and spent) for `op` on
    /// the current connection, or no token while none is required. Takes the
    /// core lock for the check only; the call re-checks the connection with
    /// [`Ticket::still_on`] where it writes.
    pub(crate) fn confirm_gate(&self, op: ConfirmOp, token: Option<&str>) -> Result<Ticket> {
        let mut inner = self.lock();
        let place = Place::of(&inner);
        inner
            .confirm
            .accept(op, token, clock::Instant::now(), place)
    }

    /// [`DspiCore::write_param`] with a confirmation token: a path whose typed
    /// call is gated (`preset.save`, `preset.delete`, `dev.save`,
    /// `dev.save.io`) goes through that call with `token`, and answers
    /// `Trigger` as `write_param` does for an action; any other path is
    /// `write_param`'s, and a token given with it is refused. For the web
    /// binding's `write`.
    ///
    /// Errors: those of the gated call or of `write_param`;
    /// [`DspiError::InvalidArgument`] for a gated path with the wrong number
    /// of indices or a value that is not `Trigger`.
    pub fn write_param_confirmed(
        &self,
        path: String,
        indices: Vec<u8>,
        value: ParamValue,
        token: Option<String>,
    ) -> Result<ParamValue> {
        let bad = |what: &str| DspiError::InvalidArgument {
            msg: format!("{path} takes {what}"),
        };
        let Some(op) = gated_path(&path) else {
            // A token with a path that is not gated is a client's mistake.
            if token.is_some() {
                return Err(bad("no confirmation token"));
            }
            return self.write_param(path, indices, value);
        };
        if value != ParamValue::Trigger {
            return Err(bad("no value (\"trigger\")"));
        }
        let token = token.as_deref();
        match (op, indices.as_slice()) {
            (ConfirmOp::SavePreset, [slot]) => self.save_preset_with(*slot, token),
            (ConfirmOp::DeletePreset, [slot]) => self.delete_preset_with(*slot, token),
            (ConfirmOp::SaveParams, []) => self.save_params_with(token),
            (ConfirmOp::SaveOutputConfig, []) => self.save_output_config_with(token),
            (ConfirmOp::SavePreset | ConfirmOp::DeletePreset, _) => Err(bad("one index")),
            _ => Err(bad("no index")),
        }
        .map(|()| ParamValue::Trigger)
    }
}

/// The operation a registry path's write is, when that is a gated one.
pub fn gated_path(path: &str) -> Option<ConfirmOp> {
    match path {
        "preset.save" => Some(ConfirmOp::SavePreset),
        "preset.delete" => Some(ConfirmOp::DeletePreset),
        "dev.save" => Some(ConfirmOp::SaveParams),
        "dev.save.io" => Some(ConfirmOp::SaveOutputConfig),
        _ => None,
    }
}

#[cfg(test)]
#[path = "confirm/tests_wp_p1tk.rs"]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_p1tk;
