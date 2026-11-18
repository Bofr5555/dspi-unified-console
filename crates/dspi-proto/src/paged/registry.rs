//! The paged command registry, generated from upstream's
//! `Documentation/protocol/dspi_commands.json` (vendored in
//! `firmware/protocol/`, `PROVENANCE.toml` `[protocol]`) by `build.rs`.
//!
//! WP PGPROTO. Nothing here is transcribed: ids, directions, address kinds, GET
//! lengths, SET bounds, flags, build conditions and page revisions are the
//! registry's own.
#![warn(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

use super::{CmdId, PagedDir};

/// What `wValue` means for a command (upstream `Documentation/commands.md` 4.5;
/// the numbering is `SYS.COMMAND_INFO` byte 6, upstream
/// `scripts/gen_vendor_protocol.py` `ADDR_KINDS`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub enum AddrKind {
    None,
    Channel,
    Input,
    Output,
    OutputAll,
    Slot,
    PinOutput,
    Preset,
    ChBand,
    InOut,
    Source,
    SpdifRx,
    I2sPair,
    Role,
    CsSlot,
    CsSub,
    CsGroup,
    CsMacro,
    CsMacroStep,
    CsPage,
    CsNoun,
    Page,
    PageCmd,
    Offset,
    Index,
    CsSlotType,
}

impl AddrKind {
    /// Every kind, in wire order: `ALL[code]` is the kind with that code.
    pub const ALL: [AddrKind; 26] = [
        AddrKind::None,
        AddrKind::Channel,
        AddrKind::Input,
        AddrKind::Output,
        AddrKind::OutputAll,
        AddrKind::Slot,
        AddrKind::PinOutput,
        AddrKind::Preset,
        AddrKind::ChBand,
        AddrKind::InOut,
        AddrKind::Source,
        AddrKind::SpdifRx,
        AddrKind::I2sPair,
        AddrKind::Role,
        AddrKind::CsSlot,
        AddrKind::CsSub,
        AddrKind::CsGroup,
        AddrKind::CsMacro,
        AddrKind::CsMacroStep,
        AddrKind::CsPage,
        AddrKind::CsNoun,
        AddrKind::Page,
        AddrKind::PageCmd,
        AddrKind::Offset,
        AddrKind::Index,
        AddrKind::CsSlotType,
    ];

    /// The kind's number in `SYS.COMMAND_INFO` byte 6.
    pub fn code(self) -> u8 {
        // ALL has 26 entries, so the position always fits a u8.
        Self::ALL
            .iter()
            .position(|k| *k == self)
            .map_or(0, |p| p as u8)
    }

    /// The kind with this wire number, `None` for a number this build does not know.
    pub fn from_code(code: u8) -> Option<Self> {
        Self::ALL.get(usize::from(code)).copied()
    }
}

/// A GET's reply length.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub enum GetLen {
    /// The command has no GET.
    None,
    /// Exactly this many bytes (migration guide 7.1).
    Fixed(u16),
    /// Varies: the reply's own length tells (migration guide 7.1).
    Var,
}

/// Command flags, with `SYS.COMMAND_INFO`'s bit numbering (migration guide 4.7).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct Flags(pub u16);

impl Flags {
    /// Refused on UART and I2C in both directions.
    pub const USB_ONLY: u16 = 0x0001;
    /// The SET is refused on UART and I2C.
    pub const SET_USB_ONLY: u16 = 0x0002;
    /// Uses the bulk buffer.
    pub const BULK: u16 = 0x0004;
    /// The main loop finishes the work after the reply.
    pub const DEFERRED: u16 = 0x0008;
    /// Writes flash, so a busy window follows.
    pub const FLASH: u16 = 0x0010;
    /// Can interrupt audio.
    pub const RESTART: u16 = 0x0020;
    /// The GET refreshes the analyser's idle timer.
    pub const KEEPALIVE: u16 = 0x0040;

    /// Whether every bit of `bits` is set.
    pub const fn has(self, bits: u16) -> bool {
        self.0 & bits == bits
    }
}

/// One page of the registry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct PageInfo {
    pub page: u8,
    pub mnemonic: &'static str,
    /// The revision this build was written for; `SYS.PAGE_LIST` reports the device's.
    pub revision: u8,
    pub doc: &'static str,
}

/// One command of the registry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct Command {
    pub id: CmdId,
    /// The page's mnemonic, e.g. `"ROUTE"`.
    pub page: &'static str,
    pub name: &'static str,
    pub get: bool,
    pub set: bool,
    pub addr: AddrKind,
    pub get_len: GetLen,
    /// Smallest SET payload. 0 and 0 for a zero-length SET and for a bulk command.
    pub set_min: u8,
    /// Largest SET payload.
    pub set_max: u8,
    pub flags: Flags,
    /// The firmware build condition (`PICO_RP2350`, `DSPI_LOOP_PROF`, ...), `None` when always built.
    pub build: Option<&'static str>,
    /// The handler's argument token (data for the fakes; not needed on the wire).
    pub arg: &'static str,
}

impl Command {
    /// Whether the command has a handler for `dir`.
    pub const fn has(&self, dir: PagedDir) -> bool {
        match dir {
            PagedDir::Get => self.get,
            PagedDir::Set => self.set,
        }
    }

    /// `PAGE.NAME`, as the registry and the guide spell it.
    pub fn full_name(&self) -> String {
        format!("{}.{}", self.page, self.name)
    }

    /// Whether a SET payload of `len` bytes is one the vendored revision accepts.
    /// A bulk command's own bounds are not the payload's: any length passes.
    pub fn set_len_ok(&self, len: usize) -> bool {
        self.flags.has(Flags::BULK)
            || (usize::from(self.set_min)..=usize::from(self.set_max)).contains(&len)
    }
}

include!(concat!(env!("OUT_DIR"), "/paged_registry.rs"));

/// The registry row of `id`, `None` for an id the vendored registry does not have.
pub fn command(id: CmdId) -> Option<&'static Command> {
    COMMANDS
        .binary_search_by_key(&id, |c| c.id)
        .ok()
        .and_then(|i| COMMANDS.get(i))
}

/// A command by `PAGE.NAME`.
pub fn by_name(full: &str) -> Option<&'static Command> {
    let (page, name) = full.split_once('.')?;
    COMMANDS.iter().find(|c| c.page == page && c.name == name)
}

/// The vendored page `page`.
pub fn page(page: u8) -> Option<&'static PageInfo> {
    PAGES.iter().find(|p| p.page == page)
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_pgproto {
    use super::*;

    #[test]
    fn wp_pgproto_commands_are_sorted_and_unique() {
        assert!(COMMANDS.windows(2).all(|w| w[0].id < w[1].id));
        assert!(PAGES.windows(2).all(|w| w[0].page < w[1].page));
        for c in &COMMANDS {
            assert_eq!(command(c.id), Some(c));
            assert_eq!(by_name(&c.full_name()), Some(c));
            assert!(c.id.cmd() >= 0x10, "{}", c.full_name());
            assert_eq!(page(c.id.page()).map(|p| p.mnemonic), Some(c.page));
        }
    }

    #[test]
    fn wp_pgproto_addr_kind_codes_round_trip() {
        for (i, k) in AddrKind::ALL.iter().enumerate() {
            assert_eq!(usize::from(k.code()), i);
            assert_eq!(AddrKind::from_code(k.code()), Some(*k));
        }
        assert_eq!(AddrKind::from_code(26), None);
        // commands.md 4.5: PRESET is 7, INDEX 24.
        assert_eq!(AddrKind::Preset.code(), 7);
        assert_eq!(AddrKind::Index.code(), 24);
    }
}
