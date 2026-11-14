//! The mode manifest (WP P1LK, PRO1 U7, `docs/UI-MODES.md` section 6): which
//! screens and operations each mode shows, in which order, under which
//! titles, and which mode a class of client opens in. A client renders the
//! list it is given; it does not decide it.
//!
//! The data is `assets/modes/modes.json`, read into the typed
//! [`ModeManifest`] that every binding returns ([`mode_manifest`]; the C ABI
//! and the web binding as JSON of the same shape). The rules around it are
//! the core's:
//!
//! - **No PIN, no prompt** (UI-MODES section 1): with no installer PIN set,
//!   every mode is reachable and [`ModeChoice::needs_pin`] is never true.
//! - **The locked-user state** (`crate::limits` lock) shows `locked_user` and
//!   no switch.
//! - **A mode the hub pins** ([`DspiCore::mode_set_pinned`], P1HL's
//!   `mode.pinned`) overrides the user's choice and the default; the lock
//!   still comes first, since the core refuses the writes anyway.
//! - Otherwise the user's choice, else the platform class's default.
//!
//! A mode never changes the device and never grants a permission: what may
//! be written stays with the limits, the lock and the hub's roles.

use std::sync::LazyLock;

use serde::{Deserialize, Serialize};

use crate::{DspiCore, DspiError, Result};

const MODES_JSON: &str = include_str!("../../../assets/modes/modes.json");

/// The locked-user state's id where the manifest lists it with the modes.
pub const LOCKED_USER: &str = "locked_user";

/// A mode: Simple, Advanced or Installer.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default, deny_unknown_fields)]
pub struct ModeEntry {
    /// `simple`, `advanced` or `installer`.
    pub id: String,
    /// The mode's name in the UI.
    pub label: String,
    /// The platform classes that open in this mode by default.
    pub default_on: Vec<String>,
    /// Behind the installer's PIN when one is set (and only then).
    pub needs_pin_when_set: bool,
}

/// What the locked-user state shows and allows.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default, deny_unknown_fields)]
pub struct ModesLockedUser {
    /// The state's name in the UI (a badge).
    pub label: String,
    /// Its screens, in order.
    pub screens: Vec<String>,
    /// What its Settings shows (`about`, `installer_note`).
    pub settings: Vec<String>,
    /// The writes it allows, by registry path (the core enforces them).
    pub writes: Vec<String>,
}

/// A screen and the modes that show it.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default, deny_unknown_fields)]
pub struct ModeScreen {
    /// Stable id (`front`, `system`, ...; a lock's cover names these).
    pub id: String,
    /// The title in the UI.
    pub title: String,
    /// The modes (and `locked_user`) that show it.
    pub modes: Vec<String>,
    /// The modes it is the first screen of.
    pub first_in: Vec<String>,
    /// `pro` for a Pro-layer screen a client shows once it has built it
    /// (else the next screen of the list stands in); "" otherwise.
    pub layer: String,
    /// The platform classes it is for; empty: all.
    pub platforms: Vec<String>,
}

/// An operation and the modes that offer it.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default, deny_unknown_fields)]
pub struct ModeOperation {
    /// Stable id (`firmware`, `factory_reset`, ...).
    pub id: String,
    /// The title in the UI.
    pub title: String,
    /// The modes that offer it.
    pub modes: Vec<String>,
    /// The platform classes it is for; empty: all.
    pub platforms: Vec<String>,
    /// Behind its usual confirmation.
    pub confirm: bool,
}

/// The mode manifest (`assets/modes/modes.json`).
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default, deny_unknown_fields)]
pub struct ModeManifest {
    /// The format, 1.
    pub version: u32,
    /// The platform classes: `phone`, `desktop`, `web`.
    pub platforms: Vec<String>,
    /// The modes, in the order a switch lists them.
    pub modes: Vec<ModeEntry>,
    /// The locked-user state.
    pub locked_user: ModesLockedUser,
    /// The screens, in order.
    pub screens: Vec<ModeScreen>,
    /// The operations that depend on the mode.
    pub operations: Vec<ModeOperation>,
}

/// The asset, read once. It is valid (the tests read it); were it not, the
/// manifest is empty and every client falls back to its own screens.
static MANIFEST: LazyLock<ModeManifest> =
    LazyLock::new(|| serde_json::from_str(MODES_JSON).unwrap_or_default());

/// The mode manifest: modes, the locked-user state, screens and operations,
/// with their titles and defaults. No I/O.
#[cfg_attr(feature = "uniffi", uniffi::export)]
pub fn mode_manifest() -> ModeManifest {
    MANIFEST.clone()
}

/// The screen ids of the manifest (what a lock may cover).
pub(crate) fn screen_ids() -> Vec<&'static str> {
    MANIFEST.screens.iter().map(|s| s.id.as_str()).collect()
}

/// The mode a client shows, and why ([`DspiCore::mode_resolve`]).
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModeChoice {
    /// The mode to show: `simple`, `advanced`, `installer` or `locked_user`.
    pub mode: String,
    /// Why: `locked` (the locked-user state), `pinned` (the hub's),
    /// `chosen` (the user's) or `default` (the platform class's).
    pub reason: String,
    /// The user may switch modes (no lock, no pinned mode).
    pub switchable: bool,
    /// The mode asked for (chosen or pinned) is the Installer, an installer
    /// PIN is set and the console is not unlocked: ask for the PIN
    /// (`installer_unlock`); `mode` is the default meanwhile. Never with no
    /// PIN set.
    pub needs_pin: bool,
    /// The modes the switch offers with no PIN prompt.
    pub open_modes: Vec<String>,
    /// The screens of `mode` for this platform class, in order.
    pub screens: Vec<String>,
    /// The first of them.
    pub first_screen: String,
    /// The operations `mode` offers on this platform class.
    pub operations: Vec<String>,
}

fn for_platform(platforms: &[String], platform: &str) -> bool {
    platforms.is_empty() || platforms.iter().any(|p| p == platform)
}

fn invalid(msg: String) -> DspiError {
    DspiError::InvalidArgument { msg }
}

/// The choice for `platform` given what is asked, the lock and the pin, from
/// manifest `m` (pure; [`DspiCore::mode_resolve`] supplies the state).
pub(crate) fn resolve(
    m: &ModeManifest,
    platform: &str,
    chosen: &str,
    locked: bool,
    installer_open: bool,
    pinned: Option<&str>,
) -> Result<ModeChoice> {
    if !m.platforms.iter().any(|p| p == platform) {
        return Err(invalid(format!(
            "\"{platform}\" is not a platform class of the mode manifest"
        )));
    }
    let known = |id: &str| m.modes.iter().any(|e| e.id == id);
    if !chosen.is_empty() && !known(chosen) {
        return Err(invalid(format!(
            "\"{chosen}\" is not a mode of the mode manifest"
        )));
    }
    let default = m
        .modes
        .iter()
        .find(|e| e.default_on.iter().any(|p| p == platform))
        .or_else(|| m.modes.first())
        .map(|e| e.id.clone())
        .unwrap_or_default();
    let reachable =
        |id: &str| installer_open || !m.modes.iter().any(|e| e.id == id && e.needs_pin_when_set);
    let open_modes = m
        .modes
        .iter()
        .filter(|e| reachable(&e.id))
        .map(|e| e.id.clone())
        .collect();
    let (mode, reason, switchable, needs_pin) = match pinned.filter(|p| !p.is_empty()) {
        _ if locked => (LOCKED_USER.to_owned(), "locked", false, false),
        Some(p) if p == LOCKED_USER => (p.to_owned(), "pinned", false, false),
        Some(p) if reachable(p) => (p.to_owned(), "pinned", false, false),
        Some(_) => (default, "pinned", false, true),
        None if chosen.is_empty() => (default, "default", true, false),
        None if reachable(chosen) => (chosen.to_owned(), "chosen", true, false),
        None => (default, "default", true, true),
    };
    let screens: Vec<String> = m
        .screens
        .iter()
        .filter(|s| s.modes.contains(&mode) && for_platform(&s.platforms, platform))
        .map(|s| s.id.clone())
        .collect();
    let first_screen = m
        .screens
        .iter()
        .filter(|s| screens.contains(&s.id))
        .find(|s| s.first_in.contains(&mode))
        .map(|s| s.id.clone())
        .or_else(|| screens.first().cloned())
        .unwrap_or_default();
    let operations = m
        .operations
        .iter()
        .filter(|o| o.modes.contains(&mode) && for_platform(&o.platforms, platform))
        .map(|o| o.id.clone())
        .collect();
    Ok(ModeChoice {
        mode,
        reason: reason.to_owned(),
        switchable,
        needs_pin,
        open_modes,
        screens,
        first_screen,
        operations,
    })
}

#[cfg_attr(feature = "uniffi", uniffi::export)]
impl DspiCore {
    /// The mode to show on a client of `platform` (`phone`, `desktop` or
    /// `web`) when the user has chosen `chosen` (a mode id, or "" for none):
    /// the locked-user state first, then a mode the hub pins, then the
    /// user's choice, then the platform's default. With no PIN set nothing
    /// asks for one. Never writes; no I/O beyond the PIN's files, read once.
    ///
    /// Errors: [`DspiError::InvalidArgument`] for a platform class or a mode
    /// the manifest does not have.
    pub fn mode_resolve(&self, platform: String, chosen: String) -> Result<ModeChoice> {
        let (locked, open, pinned) = {
            let mut inner = self.lock();
            let state = inner.lock_state();
            let pinned = inner
                .limits
                .lock
                .pinned
                .as_ref()
                .filter(|(serial, _)| *serial == state.serial)
                .map(|(_, mode)| mode.clone());
            (state.locked, state.installer_open, pinned)
        };
        resolve(
            &MANIFEST,
            &platform,
            &chosen,
            locked,
            open,
            pinned.as_deref(),
        )
    }

    /// Pin the open device's mode, as the hub says for this console's role
    /// (P1HL's `mode.pinned`): a mode id or `locked_user`; "" clears it. It
    /// overrides the user's choice until cleared or another device opens.
    ///
    /// Errors: [`DspiError::Disconnected`] with no device;
    /// [`DspiError::InvalidArgument`] for an id the manifest does not have.
    pub fn mode_set_pinned(&self, mode: String) -> Result<()> {
        if !mode.is_empty() && mode != LOCKED_USER && !MANIFEST.modes.iter().any(|e| e.id == mode) {
            return Err(invalid(format!(
                "\"{mode}\" is not a mode of the mode manifest"
            )));
        }
        let mut inner = self.lock();
        let serial = inner.conn()?.serial.clone();
        inner.limits.lock.pinned = (!mode.is_empty()).then_some((serial, mode));
        Ok(())
    }

    /// The mode the hub pins for the open device ("" for none).
    pub fn mode_pinned(&self) -> String {
        let inner = self.lock();
        let serial = inner.conn.as_ref().map(|c| c.serial.as_str());
        inner
            .limits
            .lock
            .pinned
            .as_ref()
            .filter(|(s, _)| Some(s.as_str()) == serial)
            .map(|(_, m)| m.clone())
            .unwrap_or_default()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests_wp_p1lk;
