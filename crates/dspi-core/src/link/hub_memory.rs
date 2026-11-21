//! -- WP LKFAST -- What the core remembers about DSPi Link hubs between
//! sessions, in one small file in the folder the client names
//! ([`crate::DspiCore::link_set_dir`]): nothing is kept without one (the web).
//!
//! - **A hub met without TLS** (owner decision 2026-11-21, option C): an
//!   unpinned hub that did not advertise TLS and refused it is dialled in
//!   plaintext straight away next time, with no TLS try first. TLS is tried
//!   again on every [`RECHECK_EVERY`]th connect, and after
//!   [`crate::DspiCore::link_recheck_tls`] (Pair Again) or
//!   [`crate::DspiCore::link_forget_hub`]. A pinned hub, or one pinned before,
//!   never reads this: it is TLS only.
//! - -- WP LKFAST3 -- (review LKREV 1) **A hub that had TLS** stays TLS only
//!   ([`Store::tls_seen`], the PLAN-SEC ratchet): once a session to it was
//!   encrypted, its advert said `tls=1`, or it refused plaintext
//!   (`tls_required`), it is never dialled in plaintext again, until Pair Again
//!   or Forget clear it. One downgraded session cannot keep it in plaintext.
//! - **The last hub** (option B): the hub whose device was bound last, so a
//!   client can dial it again at start ([`crate::DspiCore::link_start_hub`]), unless
//!   the user left it ([`crate::DspiCore::link_forget_last_hub`]) or forgot it.
//!
//! The file is written whole on every change (a few hundred bytes) and read
//! through one cache per file for the whole process (-- WP LKFAST3 --, review
//! LKREV 3 and 7): every core in the process shares it, each read-modify-write
//! is atomic against the others, and a read only checks the file's stamp. A
//! failed read is an empty memory; a failed write is logged and changes
//! nothing else.

// The web keeps no files: the store is there, never named, so it remembers nothing
#![cfg_attr(all(target_arch = "wasm32", target_os = "unknown"), allow(dead_code))]

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use super::guard;
use crate::LinkLastHub;

/// TLS is tried again on every this many connects to a hub met without it.
pub(crate) const RECHECK_EVERY: u32 = 10;

/// The file's name in the client's folder.
const FILE: &str = "dspi-link-hubs.json";

/// The folder named by the client; `None`: nothing is remembered.
#[derive(Default)]
pub(crate) struct Store {
    dir: Mutex<Option<PathBuf>>,
}

#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
struct Memory {
    /// Hub id (lowercase) -> connects since TLS was last tried and refused.
    #[serde(default)]
    plaintext: BTreeMap<String, u32>,
    /// -- WP LKFAST3 -- Hubs that had TLS: TLS only until Pair Again or Forget.
    #[serde(default)]
    tls_seen: BTreeSet<String>,
    #[serde(default)]
    last: Option<Last>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Last {
    hub_id: String,
    name: String,
    kind: String,
    auth: String,
    url: String,
    tls_advertised: bool,
    /// The user left this hub (Disconnect, another device): no dial at start.
    left: bool,
}

/// What a file held when last read or written, and its stamp then.
struct Cached {
    stamp: Option<(std::time::SystemTime, u64)>,
    memory: Memory,
}

/// -- WP LKFAST3 -- One cache per file for the process; its lock is the one
/// read-modify-write at a time for every core naming that folder.
static FILES: Mutex<BTreeMap<PathBuf, Cached>> = Mutex::new(BTreeMap::new());

#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
fn stamp(p: &std::path::Path) -> Option<(std::time::SystemTime, u64)> {
    let m = std::fs::metadata(p).ok()?;
    Some((m.modified().ok()?, m.len()))
}

/// The file's memory, from the cache while its stamp is unchanged.
#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
fn load(files: &mut BTreeMap<PathBuf, Cached>, p: &std::path::Path) -> Memory {
    let now = stamp(p);
    if let Some(c) = files.get(p).filter(|c| c.stamp.is_some() && c.stamp == now) {
        return c.memory.clone();
    }
    let memory: Memory = std::fs::read(p)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default();
    files.insert(
        p.to_owned(),
        Cached {
            stamp: now,
            memory: memory.clone(),
        },
    );
    memory
}

impl Store {
    pub(crate) fn set_dir(&self, dir: Option<PathBuf>) {
        *guard(&self.dir) = dir;
    }

    fn path(&self) -> Option<PathBuf> {
        guard(&self.dir).as_ref().map(|d| d.join(FILE))
    }

    fn read(&self) -> Memory {
        #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
        if let Some(p) = self.path() {
            return load(&mut guard(&FILES), &p);
        }
        Memory::default()
    }

    /// Change the memory and write it back, if a folder is named.
    fn update(&self, f: impl FnOnce(&mut Memory)) {
        #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
        {
            let Some(p) = self.path() else { return };
            let mut files = guard(&FILES);
            let mut m = load(&mut files, &p);
            let before = m.clone();
            f(&mut m);
            if m == before {
                return;
            }
            let write = || -> std::io::Result<()> {
                if let Some(d) = p.parent() {
                    std::fs::create_dir_all(d)?;
                }
                let tmp = p.with_extension("json.tmp");
                std::fs::write(&tmp, serde_json::to_vec(&m).unwrap_or_default())?;
                std::fs::rename(&tmp, &p)
            };
            match write() {
                Ok(()) => {
                    files.insert(
                        p.clone(),
                        Cached {
                            stamp: stamp(&p),
                            memory: m,
                        },
                    );
                }
                Err(e) => log::warn!("dspi-link: the hub memory could not be written: {e}"),
            }
        }
        #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
        let _ = f;
    }

    /// Dial hub `hub_id` in plaintext without a TLS try: it was met without TLS
    /// fewer than [`RECHECK_EVERY`] connects ago and never had TLS. The caller
    /// asks only for an unpinned hub that does not advertise TLS.
    pub(crate) fn skip_tls(&self, hub_id: &str) -> bool {
        if hub_id.is_empty() {
            return false;
        }
        let id = hub_id.to_lowercase();
        let m = self.read();
        let n = m.plaintext.get(&id).copied().unwrap_or(0);
        !m.tls_seen.contains(&id) && (1..RECHECK_EVERY).contains(&n)
    }

    /// `hub_id` was met without TLS and not with it since.
    pub(crate) fn met_without_tls(&self, hub_id: &str) -> bool {
        !hub_id.is_empty() && self.read().plaintext.contains_key(&hub_id.to_lowercase())
    }

    // -- WP LKFAST3 --
    /// `hub_id` had TLS ([`Self::saw_tls`]): it is reached over TLS only.
    pub(crate) fn tls_seen(&self, hub_id: &str) -> bool {
        !hub_id.is_empty() && self.read().tls_seen.contains(&hub_id.to_lowercase())
    }

    /// `hub_id` had TLS: an encrypted session, an advert of `tls=1`, or a
    /// refusal of plaintext. TLS only from now on, until Pair Again or Forget.
    pub(crate) fn saw_tls(&self, hub_id: &str) {
        if hub_id.is_empty() {
            return;
        }
        let id = hub_id.to_lowercase();
        self.update(|m| {
            m.plaintext.remove(&id);
            m.tls_seen.insert(id);
        });
    }

    /// A session to `hub_id` came up in plaintext; `tried` says whether TLS was
    /// tried (and refused) first, which starts the count again.
    pub(crate) fn met_plaintext(&self, hub_id: &str, tried: bool) {
        if hub_id.is_empty() {
            return;
        }
        let id = hub_id.to_lowercase();
        self.update(|m| {
            if m.tls_seen.contains(&id) {
                return; // -- WP LKFAST3 -- TLS only: no note (not reached: such a dial needs TLS)
            }
            let n = m.plaintext.entry(id).or_insert(0);
            *n = if tried { 1 } else { n.saturating_add(1) };
        });
    }

    /// TLS is to be tried at `hub_id`'s next connect, as at first contact: the
    /// user paired again (the note and the had-TLS mark go).
    pub(crate) fn recheck(&self, hub_id: &str) {
        let id = hub_id.to_lowercase();
        self.update(|m| {
            m.plaintext.remove(&id);
            m.tls_seen.remove(&id);
        });
    }

    // -- WP LKFAST3 -- (review LKREV 2)
    /// The user forgot `hub_id`: everything about it goes, the start's dial too.
    pub(crate) fn forget(&self, hub_id: &str) {
        let id = hub_id.to_lowercase();
        self.update(|m| {
            m.plaintext.remove(&id);
            m.tls_seen.remove(&id);
            if m.last.as_ref().is_some_and(|l| l.hub_id == id) {
                m.last = None;
            }
        });
    }

    /// The hub a device was just bound on, to dial again at start.
    pub(crate) fn bound(&self, hub: &LinkLastHub, url: &str) {
        let last = Last {
            hub_id: hub.hub_id.to_lowercase(),
            name: hub.name.clone(),
            kind: hub.kind.clone(),
            auth: hub.auth.clone(),
            url: url.to_owned(),
            tls_advertised: hub.tls_advertised,
            left: false,
        };
        self.update(|m| m.last = Some(last));
    }

    /// The user left the last hub: no dial at start until another bind.
    pub(crate) fn left(&self) {
        self.update(|m| {
            if let Some(l) = m.last.as_mut() {
                l.left = true;
            }
        });
    }

    /// The last hub, unless the user left it.
    pub(crate) fn last(&self) -> Option<LinkLastHub> {
        let m = self.read();
        let seen = m
            .last
            .as_ref()
            .is_some_and(|l| m.tls_seen.contains(&l.hub_id));
        let l = m.last.filter(|l| !l.left)?;
        let (host, port, path) = split_url(&l.url)?;
        Some(LinkLastHub {
            hub_id: l.hub_id,
            name: l.name,
            kind: l.kind,
            auth: l.auth,
            host,
            port,
            path,
            // -- WP LKFAST3 -- a hub that had TLS is dialled as one that says so
            tls_advertised: l.tls_advertised || seen,
        })
    }
}

/// `ws://host:port/path` -> (host, port, path); `None` for anything else.
fn split_url(url: &str) -> Option<(String, u16, String)> {
    let rest = url
        .strip_prefix("ws://")
        .or_else(|| url.strip_prefix("wss://"))?;
    let (authority, path) = rest
        .find('/')
        .map_or((rest, ""), |i| (&rest[..i], &rest[i..]));
    let host = dspi_link::address::url_host(url)?.to_owned();
    let port = authority
        .rsplit_once(':')
        .filter(|(h, _)| !h.ends_with(':'))?
        .1
        .parse()
        .ok()?;
    Some((host, port, path.to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wp_lkfast_split_url_reads_what_link_ws_url_writes() {
        assert_eq!(
            split_url("ws://192.168.1.20:11915/dspi/v1"),
            Some(("192.168.1.20".into(), 11915, "/dspi/v1".into()))
        );
        assert_eq!(
            split_url("ws://[fe80::1]:8080/x"),
            Some(("fe80::1".into(), 8080, "/x".into()))
        );
        assert_eq!(split_url("http://a:1/"), None);
        assert_eq!(split_url("ws://a/"), None, "a port is always written");
    }
}
