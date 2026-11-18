//! Decoders for the discovery replies and the per-connection page set (WP PGPROTO).
//!
//! Every decoder takes device bytes and never panics: a short reply is `None`,
//! extra bytes are ignored (a later page revision may grow a reply at the end,
//! migration guide 4.7).
#![warn(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

use std::collections::BTreeMap;

use super::registry::{AddrKind, Command, Flags};
use super::{CmdId, PagedDir};

/// One `{page, revision}` pair of `SYS.PAGE_LIST`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct PageRev {
    pub page: u8,
    pub revision: u8,
}

/// Decode `SYS.PAGE_LIST` (guide 4.7): two bytes per implemented page. A trailing
/// odd byte is ignored.
pub fn decode_page_list(b: &[u8]) -> Vec<PageRev> {
    b.as_chunks::<2>()
        .0
        .iter()
        .map(|[page, revision]| PageRev {
            page: *page,
            revision: *revision,
        })
        .collect()
}

/// `SYS.PAGE_COMMANDS` (guide 4.7): the GET and SET bitmaps of one page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PageCommands {
    pub get: [u8; 32],
    pub set: [u8; 32],
}

/// `SYS.PAGE_COMMANDS`' reply length.
pub const PAGE_COMMANDS_LEN: usize = 64;

impl PageCommands {
    /// Nothing implemented.
    pub const EMPTY: Self = Self {
        get: [0; 32],
        set: [0; 32],
    };

    /// Decode the 64-byte reply: bytes 0-31 the GET bitmap, 32-63 the SET bitmap.
    pub fn decode(b: &[u8]) -> Option<Self> {
        let get = b.get(..32)?.try_into().ok()?;
        let set = b.get(32..64)?.try_into().ok()?;
        Some(Self { get, set })
    }

    /// The 64-byte reply.
    pub fn encode(&self) -> [u8; 64] {
        let mut out = [0u8; 64];
        for (o, i) in out.iter_mut().zip(self.get.iter().chain(self.set.iter())) {
            *o = *i;
        }
        out
    }

    /// Whether command `cmd` has a handler for `dir`: bit `cmd & 7` of byte `cmd >> 3`.
    pub fn has(&self, cmd: u8, dir: PagedDir) -> bool {
        let map = match dir {
            PagedDir::Get => &self.get,
            PagedDir::Set => &self.set,
        };
        map.get(usize::from(cmd >> 3))
            .is_some_and(|byte| byte & (1u8 << (cmd & 7)) != 0)
    }

    /// Mark `cmd` as implemented for `dir`.
    pub fn insert(&mut self, cmd: u8, dir: PagedDir) {
        let map = match dir {
            PagedDir::Get => &mut self.get,
            PagedDir::Set => &mut self.set,
        };
        if let Some(byte) = map.get_mut(usize::from(cmd >> 3)) {
            *byte |= 1u8 << (cmd & 7);
        }
    }
}

/// `SYS.COMMAND_INFO` (guide 4.7), 8 bytes about one command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CommandInfo {
    pub flags: Flags,
    /// The GET reply length: 0 when there is no GET, `0xFFFF` when it varies.
    pub get_len: u16,
    pub set_min: u8,
    pub set_max: u8,
    /// The address kind; `Err(code)` for a number this build does not know.
    pub addr: Result<AddrKind, u8>,
    pub get: bool,
    pub set: bool,
}

impl CommandInfo {
    /// Decode the 8-byte reply.
    pub fn decode(b: &[u8]) -> Option<Self> {
        let [f0, f1, g0, g1, set_min, set_max, addr, dirs] = *b.get(..8)? else {
            return None;
        };
        Some(Self {
            flags: Flags(u16::from_le_bytes([f0, f1])),
            get_len: u16::from_le_bytes([g0, g1]),
            set_min,
            set_max,
            addr: AddrKind::from_code(addr).ok_or(addr),
            get: dirs & 1 != 0,
            set: dirs & 2 != 0,
        })
    }

    /// What a device built from the vendored registry answers for `c`.
    pub fn of(c: &Command) -> Self {
        use super::registry::GetLen;
        Self {
            flags: c.flags,
            get_len: match c.get_len {
                GetLen::None => 0,
                GetLen::Fixed(n) => n,
                GetLen::Var => 0xFFFF,
            },
            set_min: c.set_min,
            set_max: c.set_max,
            addr: Ok(c.addr),
            get: c.get,
            set: c.set,
        }
    }
}

/// `SYS.LAST_RESULT` (guide 5.2): the result of the last request on this transport.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct LastResult {
    pub id: CmdId,
    /// `None` for a direction byte other than 1 (GET) or 0 (SET).
    pub dir: Option<PagedDir>,
    pub result: u8,
    pub value: u16,
}

impl LastResult {
    /// Decode the 8-byte reply.
    pub fn decode(b: &[u8]) -> Option<Self> {
        let [i0, i1, dir, result, v0, v1] = *b.get(..6)? else {
            return None;
        };
        if b.len() < 8 {
            return None;
        }
        Some(Self {
            id: CmdId(u16::from_le_bytes([i0, i1])),
            dir: match dir {
                1 => Some(PagedDir::Get),
                0 => Some(PagedDir::Set),
                _ => None,
            },
            result,
            value: u16::from_le_bytes([v0, v1]),
        })
    }

    /// Whether this record is about the request `(id, dir, value)` (guide 5.2:
    /// the record is per transport, not per host, so check before trusting it).
    pub fn matches(&self, id: CmdId, dir: PagedDir, value: u16) -> bool {
        self.id == id && self.dir == Some(dir) && self.value == value
    }
}

/// `SYS.PLATFORM` (guide 4.6), the layout of the 1.1.6 `GET_PLATFORM` reply.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct SysPlatform {
    /// 0 RP2040, 1 RP2350.
    pub platform: u8,
    pub major: u8,
    /// `(minor << 4) | patch`, the legacy packed form.
    pub packed: u8,
    pub outputs: u8,
    /// Bytes 4-6, absent from older firmware.
    pub minor: Option<u8>,
    pub patch: Option<u8>,
    pub beta: Option<u8>,
}

impl SysPlatform {
    /// Decode a reply of at least 4 bytes.
    pub fn decode(b: &[u8]) -> Option<Self> {
        let [platform, major, packed, outputs] = *b.get(..4)? else {
            return None;
        };
        Some(Self {
            platform,
            major,
            packed,
            outputs,
            minor: b.get(4).copied(),
            patch: b.get(5).copied(),
            beta: b.get(6).copied(),
        })
    }
}

/// What one connected paged device implements: its pages with their revisions
/// and each page's command bitmaps, read once at connect (`SYS.PAGE_LIST`, then
/// `SYS.PAGE_COMMANDS` per page) and kept for the connection (PLAN-PG section 1.1
/// step 2). The planner refuses locally what is not in it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PageSet {
    pages: BTreeMap<u8, (u8, PageCommands)>,
}

impl PageSet {
    /// Nothing implemented.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add (or replace) a page with its revision and bitmaps.
    pub fn insert(&mut self, page: u8, revision: u8, commands: PageCommands) {
        self.pages.insert(page, (revision, commands));
    }

    /// Build from the discovery replies: the `SYS.PAGE_LIST` pairs and a
    /// `SYS.PAGE_COMMANDS` reply per page. A listed page without a decodable
    /// bitmap counts as implementing nothing.
    pub fn from_discovery<'a>(
        list: &[PageRev],
        mut bitmap: impl FnMut(u8) -> Option<&'a [u8]>,
    ) -> Self {
        let mut s = Self::new();
        for p in list {
            let cmds = bitmap(p.page)
                .and_then(PageCommands::decode)
                .unwrap_or(PageCommands::EMPTY);
            s.insert(p.page, p.revision, cmds);
        }
        s
    }

    /// Every vendored command for which `keep` is true, at the vendored revisions:
    /// what a stock build answers (e.g. `|c| c.build.is_none()` for a release build
    /// of an RP2040, `|c| c.build.is_none() || c.build == Some("PICO_RP2350")` for an RP2350).
    pub fn vendored(keep: impl Fn(&Command) -> bool) -> Self {
        let mut s = Self::new();
        for c in super::registry::COMMANDS.iter().filter(|c| keep(c)) {
            let page = c.id.page();
            let revision = super::registry::page(page).map_or(0, |p| p.revision);
            let entry = s
                .pages
                .entry(page)
                .or_insert((revision, PageCommands::EMPTY));
            for dir in [PagedDir::Get, PagedDir::Set] {
                if c.has(dir) {
                    entry.1.insert(c.id.cmd(), dir);
                }
            }
        }
        s
    }

    /// Whether the device implements `id` in direction `dir`.
    pub fn contains(&self, id: CmdId, dir: PagedDir) -> bool {
        self.pages
            .get(&id.page())
            .is_some_and(|(_, cmds)| cmds.has(id.cmd(), dir))
    }

    /// Whether the device lists `page`.
    pub fn has_page(&self, page: u8) -> bool {
        self.pages.contains_key(&page)
    }

    /// The device's revision of `page`.
    pub fn revision(&self, page: u8) -> Option<u8> {
        self.pages.get(&page).map(|(r, _)| *r)
    }

    /// The listed pages, ascending, with their revisions.
    pub fn pages(&self) -> impl Iterator<Item = PageRev> + '_ {
        self.pages.iter().map(|(p, (r, _))| PageRev {
            page: *p,
            revision: *r,
        })
    }

    /// Whether the device's revision of `page` is the one this build was written
    /// for, so SET lengths may be checked locally (PLAN-PG R8b: a higher revision
    /// may grow a payload at the end; then the device judges).
    pub fn revision_is_vendored(&self, page: u8) -> bool {
        match (self.revision(page), super::registry::page(page)) {
            (Some(have), Some(vendored)) => have == vendored.revision,
            _ => false,
        }
    }
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_pgproto {
    use super::*;

    #[test]
    fn wp_pgproto_short_replies_are_none() {
        assert_eq!(PageCommands::decode(&[0; 63]), None);
        assert_eq!(CommandInfo::decode(&[0; 7]), None);
        assert_eq!(LastResult::decode(&[0; 7]), None);
        assert_eq!(SysPlatform::decode(&[0; 3]), None);
        assert_eq!(
            decode_page_list(&[1, 1, 2]),
            vec![PageRev {
                page: 1,
                revision: 1
            }]
        );
    }

    #[test]
    fn wp_pgproto_bitmaps_round_trip() {
        let mut m = PageCommands::EMPTY;
        m.insert(0x10, PagedDir::Get);
        m.insert(0xFF, PagedDir::Set);
        assert!(m.has(0x10, PagedDir::Get));
        assert!(!m.has(0x10, PagedDir::Set));
        assert!(m.has(0xFF, PagedDir::Set));
        assert_eq!(PageCommands::decode(&m.encode()), Some(m));
    }

    #[test]
    fn wp_pgproto_vendored_set_has_every_command() {
        let all = PageSet::vendored(|_| true);
        for c in &super::super::registry::COMMANDS {
            assert_eq!(
                all.contains(c.id, PagedDir::Get),
                c.get,
                "{}",
                c.full_name()
            );
            assert_eq!(
                all.contains(c.id, PagedDir::Set),
                c.set,
                "{}",
                c.full_name()
            );
            assert!(all.revision_is_vendored(c.id.page()));
        }
        let release = PageSet::vendored(|c| c.build.is_none());
        assert!(!release.has_page(0xF0), "DIAG is bench-only");
    }
}
