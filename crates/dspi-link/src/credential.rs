//! -- WP SECCORE -- What a platform stores per hub, and the `dspi://pair` link
//! (docs/core-merge/PLAN-SEC.md section 4.3).
//!
//! **The credential** is an opaque string that every platform's existing token
//! store keeps unchanged (Keychain, Keystore, Credential Manager, Secret Service,
//! 0600 files, `localStorage`):
//! - `v1`: the hub's bare token, as before TLS. No pin.
//! - `v2.<token>.<pin>.<flags>[.<next>]`: the token (base64url of its bytes, so a
//!   token may hold any character; empty for a pin with no token yet, a pairing
//!   link's), the SHA-256 of the hub key's SubjectPublicKeyInfo (base64url, 32
//!   bytes), flags (`t`: TLS required, always set for a pin), and the key the
//!   pinned one announced as its successor, signature verified, if any.
//! - -- WP PINTLS -- `v2.<token>.<old pin or empty>.s`: a hub that **was pinned**
//!   and whose pairing was dropped (Pair Again, a refused token; owner decision
//!   2026-11-17). No key is enforced, but the hub is reached over TLS only: a
//!   plaintext connect or pairing is refused (`tls_required_pinned`) before any
//!   PIN or token goes out, and a TLS pairing pins the new key. Only removing
//!   the record (Forget hub) allows plaintext again. See [`unpaired_record`].
//!
//! A string that starts with `v2.` but does not parse carries no token and no key
//! (pair again; -- WP SECBFIXA --): it is never sent as a token, and it still marks
//! the hub TLS-only (-- WP PINTLS --), as the superseded record does. Anything else that is
//! not a `v2` is read as a `v1` token.
//!
//! **The pairing link** `dspi://pair?h=<hid>&a=<host:port>&k=<pin>&p=<pin code>`
//! (a QR code from the hub's display) is a hint, never a bypass: its `k` becomes
//! the pin the TLS key must match, and the pairing screen still shows the hub and
//! its fingerprint and needs a tap (section 4.3).

use std::fmt;

use data_encoding::BASE64URL_NOPAD;

/// SHA-256 of a hub key's DER SubjectPublicKeyInfo (the pin, RFC 7469).
pub type Pin = [u8; 32];

/// The `v2` prefix.
const V2: &str = "v2";
/// Flag: the hub is only ever reached over TLS.
const FLAG_TLS: char = 't';
/// -- WP PINTLS -- Flag: the pin is superseded (the hub was pinned, its pairing
/// dropped): TLS only, any key at the next pairing.
const FLAG_SUPERSEDED: char = 's';
/// Longest credential or pairing link read; anything longer is not one.
const MAX_TEXT: usize = 4096;

/// A hub's stored credential: its token and the key it is pinned to.
#[derive(Clone, PartialEq, Eq, Default)]
pub struct Credential {
    /// The hub's token; empty for none.
    pub token: String,
    /// The key this hub must present. `None`: a `v1` credential (no TLS seen yet).
    pub pin: Option<Pin>,
    /// The key [`Self::pin`]'s key announced as its successor (`hello.hub.key_next`
    /// on an earlier connection, its signature verified): accepted directly.
    pub next: Option<Pin>,
    // -- WP PINTLS --
    /// The hub was pinned before and its pairing was dropped (Pair Again):
    /// TLS only, with no key enforced until the next pairing pins one. Only
    /// meaningful without [`Self::pin`].
    pub pinned_before: bool,
    /// The superseded key, for display only (never checked).
    pub previous: Option<Pin>,
}

/// By hand: the token never reaches a log.
impl fmt::Debug for Credential {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Credential")
            .field("token", &if self.token.is_empty() { "" } else { "<redacted>" })
            .field("pin", &self.pin.map(|p| pin_display(&p)))
            .field("next", &self.next.map(|p| pin_display(&p)))
            .field("pinned_before", &self.pinned_before)
            .finish()
    }
}

impl Credential {
    /// A credential string (`v1` or `v2`), trimmed. Never fails. -- WP SECBFIXA --
    /// A string that starts with `v2.` but is damaged carries **no token and no
    /// key** (pair again): it is never sent as a bare token, and it stays
    /// [`Self::pinned_before`] (-- WP PINTLS --, review F1), so a pinned record
    /// never degrades to one that could fall back to plaintext. Anything else
    /// that is not a `v2` is a `v1` token.
    pub fn parse(text: &str) -> Credential {
        let text = text.trim();
        if let Some(c) = Self::parse_v2(text) {
            return c;
        }
        if text.starts_with("v2.") {
            log::warn!("dspi-link: a stored v2 credential is damaged; pairing again is needed");
            // -- WP PINTLS -- (review F1) a `v2` record was pinned once: no token,
            // no key, but still TLS only, so a damaged record cannot bring a hub
            // back to plaintext (only Forget hub does)
            return Credential { pinned_before: true, ..Credential::default() };
        }
        Credential { token: text.to_owned(), ..Credential::default() }
    }

    // -- WP PINTLS --
    /// The hub may only be reached over TLS: it is pinned, or was pinned before
    /// (owner decision 2026-11-17). A plaintext connect is then refused.
    pub fn tls_only(&self) -> bool {
        self.pin.is_some() || self.pinned_before
    }

    fn parse_v2(text: &str) -> Option<Credential> {
        if text.len() > MAX_TEXT {
            return None;
        }
        let mut parts = text.split('.');
        if parts.next()? != V2 {
            return None;
        }
        let token = BASE64URL_NOPAD.decode(parts.next()?.as_bytes()).ok()?;
        let token = String::from_utf8(token).ok()?;
        let pin_text = parts.next()?;
        let flags = parts.next()?;
        if !flags.chars().all(|c| c.is_ascii_lowercase()) {
            return None;
        }
        // -- WP PINTLS -- a superseded pin: TLS only, no key enforced, no successor
        if flags.contains(FLAG_SUPERSEDED) {
            let previous = if pin_text.is_empty() { None } else { Some(pin_from_b64url(pin_text)?) };
            // (review F2) such a record never carries a token: one would go out
            // to whatever key answers, unchecked
            if parts.next().is_some() || !token.is_empty() {
                return None;
            }
            return Some(Credential { token, pin: None, next: None, pinned_before: true, previous });
        }
        let pin = pin_from_b64url(pin_text)?;
        let next = match parts.next() {
            Some(n) => Some(pin_from_b64url(n)?),
            None => None,
        };
        if parts.next().is_some() {
            return None;
        }
        Some(Credential { token, pin: Some(pin), next, ..Credential::default() })
    }

    /// The string to store: the bare token without a pin (`v1`), else `v2`
    /// (-- WP PINTLS -- also for a hub [`Self::pinned_before`]).
    pub fn encode(&self) -> String {
        let Some(pin) = &self.pin else {
            if self.pinned_before {
                // No token (review F2): none is ever kept without the key it belongs to
                return format!(
                    "{V2}..{}.{FLAG_SUPERSEDED}",
                    self.previous.map(|p| BASE64URL_NOPAD.encode(&p)).unwrap_or_default()
                );
            }
            return self.token.clone();
        };
        let mut out = format!(
            "{V2}.{}.{}.{FLAG_TLS}",
            BASE64URL_NOPAD.encode(self.token.as_bytes()),
            BASE64URL_NOPAD.encode(pin)
        );
        if let Some(next) = self.next.filter(|n| n != pin) {
            out.push('.');
            out.push_str(&BASE64URL_NOPAD.encode(&next));
        }
        out
    }
}

// -- WP PINTLS --
/// What to keep in place of the stored credential `text` when its pairing is
/// dropped (Pair Again after `hub_key_changed` or a TLS failure, a refused or
/// revoked token): for a hub that was ever pinned (any `v2`, a damaged one
/// included) a token-less record that still requires TLS (`v2..<old pin>.s`),
/// so a later plaintext connect or pairing is refused before any PIN goes out
/// (owner decision 2026-11-17). "" (nothing to keep) for a `v1` token or none:
/// such a hub never had TLS. Forget hub (removing the record) is the only way
/// back to plaintext.
pub fn unpaired_record(text: &str) -> String {
    let text = text.trim();
    let c = Credential::parse(text);
    if !c.tls_only() {
        return String::new();
    }
    Credential { pinned_before: true, previous: c.pin.or(c.previous), ..Credential::default() }.encode()
}

/// A pin from base64url (no padding; trailing `=` tolerated).
pub fn pin_from_b64url(text: &str) -> Option<Pin> {
    let bytes = BASE64URL_NOPAD.decode(text.trim().trim_end_matches('=').as_bytes()).ok()?;
    <[u8; 32]>::try_from(bytes.as_slice()).ok()
}

/// A pin as base64url without padding (43 characters).
pub fn pin_to_b64url(pin: &Pin) -> String {
    BASE64URL_NOPAD.encode(pin)
}

/// The pin for people: `SHA256:K7Q2-M9XD-4TNB-0ZRA` (the TLS module's form).
/// Empty where there is no TLS (the web build).
pub fn pin_display(pin: &Pin) -> String {
    #[cfg(all(feature = "tls", not(all(target_arch = "wasm32", target_os = "unknown"))))]
    {
        crate::tls::SpkiHash(*pin).display()
    }
    #[cfg(not(all(feature = "tls", not(all(target_arch = "wasm32", target_os = "unknown")))))]
    {
        let _ = pin;
        String::new()
    }
}

/// Equal, in time independent of where they differ.
pub fn pin_eq(a: &Pin, b: &Pin) -> bool {
    a.iter().zip(b.iter()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

// ---------------------------------------------------------------------------
// The pairing link

/// A parsed `dspi://pair` link.
#[derive(Clone, PartialEq, Eq)]
pub struct PairLink {
    /// `h`: the hub's id, lowercase.
    pub hub_id: String,
    /// `a`: where it answers.
    pub host: String,
    pub port: u16,
    /// `k`: the key it must present.
    pub key: Pin,
    /// `p`: the PIN to pair with; empty when the link carries none.
    pub pin_code: String,
}

/// By hand: the PIN never reaches a log.
impl fmt::Debug for PairLink {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PairLink")
            .field("hub_id", &self.hub_id)
            .field("host", &self.host)
            .field("port", &self.port)
            .field("key", &pin_display(&self.key))
            .field("pin_code", &if self.pin_code.is_empty() { "" } else { "<redacted>" })
            .finish()
    }
}

impl PairLink {
    /// The credential to connect with: pinned to `k`, no token yet.
    pub fn credential(&self) -> Credential {
        Credential { token: String::new(), pin: Some(self.key), ..Credential::default() }
    }
}

/// Why a pairing link was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PairLinkError {
    #[error("not a DSPi pairing link")]
    NotPairLink,
    #[error("the pairing link has no {0}")]
    Missing(&'static str),
    #[error("the pairing link's {0} is not valid")]
    Invalid(&'static str),
}

/// Parse `dspi://pair?h=<hid>&a=<host:port>&k=<pin b64url>&p=<pin code>`.
/// `h`, `a` and `k` are required, `p` optional; other keys are ignored, a
/// repeated key is refused.
pub fn parse_pair_link(text: &str) -> Result<PairLink, PairLinkError> {
    let text = text.trim();
    if text.len() > MAX_TEXT {
        return Err(PairLinkError::NotPairLink);
    }
    let (scheme, rest) = text.split_once("://").ok_or(PairLinkError::NotPairLink)?;
    let (target, query) = rest.split_once('?').unwrap_or((rest, ""));
    if !scheme.eq_ignore_ascii_case("dspi") || !target.trim_end_matches('/').eq_ignore_ascii_case("pair") {
        return Err(PairLinkError::NotPairLink);
    }
    let (mut h, mut a, mut k, mut p) = (None, None, None, None);
    for pair in query.split('&').filter(|s| !s.is_empty()) {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        let value = percent_decode(value).ok_or(PairLinkError::Invalid("encoding"))?;
        let slot = match key {
            "h" => &mut h,
            "a" => &mut a,
            "k" => &mut k,
            "p" => &mut p,
            _ => continue,
        };
        if slot.replace(value).is_some() {
            return Err(PairLinkError::Invalid("repeated field"));
        }
    }
    let hub_id = h.filter(|v| !v.is_empty()).ok_or(PairLinkError::Missing("hub id"))?;
    if hub_id.len() > 128 || !hub_id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
        return Err(PairLinkError::Invalid("hub id"));
    }
    let address = a.filter(|v| !v.is_empty()).ok_or(PairLinkError::Missing("address"))?;
    let (host, port) = crate::address::parse_address(&address).ok_or(PairLinkError::Invalid("address"))?;
    let key = k.filter(|v| !v.is_empty()).ok_or(PairLinkError::Missing("key"))?;
    let key = pin_from_b64url(&key).ok_or(PairLinkError::Invalid("key"))?;
    let pin_code = p.unwrap_or_default();
    if pin_code.len() > 32 || !pin_code.chars().all(|c| c.is_ascii_alphanumeric()) {
        return Err(PairLinkError::Invalid("PIN"));
    }
    Ok(PairLink { hub_id: hub_id.to_ascii_lowercase(), host, port, key, pin_code })
}

/// `%XX` decoding (and `+` as a space), into UTF-8.
fn percent_decode(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0usize;
    while let Some(&b) = bytes.get(i) {
        match b {
            b'%' => {
                let hex = bytes.get(i.checked_add(1)?..i.checked_add(3)?)?;
                let hex = std::str::from_utf8(hex).ok()?;
                out.push(u8::from_str_radix(hex, 16).ok()?);
                i = i.checked_add(3)?;
            }
            b'+' => {
                out.push(b' ');
                i = i.checked_add(1)?;
            }
            _ => {
                out.push(b);
                i = i.checked_add(1)?;
            }
        }
    }
    String::from_utf8(out).ok()
}

#[cfg(test)]
mod tests_wp_seccore {
    use super::*;

    const PIN: Pin = [7u8; 32];
    const NEXT: Pin = [9u8; 32];
    /// -- WP PINTLS -- (review F1) what a damaged `v2` record reads as.
    const DAMAGED: Credential =
        Credential { token: String::new(), pin: None, next: None, pinned_before: true, previous: None };

    #[test]
    fn wp_seccore_v1_is_the_bare_token() {
        let c = Credential::parse(" tok-12-0 ");
        assert_eq!(c, Credential { token: "tok-12-0".into(), ..Credential::default() });
        assert_eq!(c.encode(), "tok-12-0");
    }

    #[test]
    fn wp_seccore_v2_round_trips() {
        for c in [
            Credential { token: "abc.def+/=".into(), pin: Some(PIN), ..Credential::default() },
            Credential { token: String::new(), pin: Some(PIN), ..Credential::default() },
            Credential { token: "t".into(), pin: Some(PIN), next: Some(NEXT), ..Credential::default() },
        ] {
            let s = c.encode();
            assert!(s.starts_with("v2."), "{s}");
            assert_eq!(Credential::parse(&s), c);
        }
        // A successor equal to the pin is not written
        let same = Credential { token: "t".into(), pin: Some(PIN), next: Some(PIN), ..Credential::default() };
        assert_eq!(Credential::parse(&same.encode()).next, None);
    }

    // -- WP SECBFIXA -- (review secb F1) a damaged v2 is no credential, never a token
    #[test]
    fn wp_seccore_malformed_v2_is_no_credential() {
        let good = Credential { token: "t".into(), pin: Some(PIN), ..Credential::default() }.encode();
        let pin = pin_to_b64url(&PIN);
        for bad in [
            "v2.dA.short.t".to_owned(),
            "v2.dA".to_owned(),
            format!("{good}.x.y"),
            good.replace(".t", ".T"),
            "v2.!!.AAAA.t".to_owned(),
            good.replace(&pin, &pin[..pin.len() - 1]),
        ] {
            // -- WP PINTLS -- (review F1) no token, no key, and still TLS only
            assert_eq!(Credential::parse(&bad), DAMAGED, "{bad}");
        }
        // Not v2-shaped at all: still a v1 token
        assert_eq!(Credential::parse("v2token").token, "v2token");
        assert_eq!(Credential::parse("v3.x").token, "v3.x");
    }

    #[test]
    fn wp_seccore_debug_never_shows_the_token() {
        let c = Credential { token: "secret-token".into(), pin: Some(PIN), ..Credential::default() };
        assert!(!format!("{c:?}").contains("secret-token"));
        let l = parse_pair_link(&format!("dspi://pair?h=abc&a=hub.local:11915&k={}&p=482913", pin_to_b64url(&PIN)))
            .unwrap();
        assert!(!format!("{l:?}").contains("482913"));
    }

    #[test]
    fn wp_seccore_pair_link_parses() {
        let k = pin_to_b64url(&PIN);
        let l = parse_pair_link(&format!("DSPI://pair?h=AB-12&a=%5Bfe80%3A%3A1%5D:11915&k={k}&p=482913&x=1")).unwrap();
        assert_eq!(l.hub_id, "ab-12");
        assert_eq!((l.host.as_str(), l.port), ("fe80::1", 11915));
        assert_eq!(l.key, PIN);
        assert_eq!(l.pin_code, "482913");
        assert_eq!(l.credential().pin, Some(PIN));
        assert!(l.credential().token.is_empty());
        // No PIN: the user types it
        let l = parse_pair_link(&format!("dspi://pair?h=a&a=10.0.0.2:11915&k={k}")).unwrap();
        assert_eq!(l.pin_code, "");
    }

    #[test]
    fn wp_seccore_pair_link_refusals() {
        let k = pin_to_b64url(&PIN);
        let cases = [
            ("https://pair?h=a&a=h:1&k=K".to_owned(), PairLinkError::NotPairLink),
            ("dspi://other?h=a".to_owned(), PairLinkError::NotPairLink),
            (format!("dspi://pair?a=h:1&k={k}"), PairLinkError::Missing("hub id")),
            (format!("dspi://pair?h=a&k={k}"), PairLinkError::Missing("address")),
            ("dspi://pair?h=a&a=h:1".to_owned(), PairLinkError::Missing("key")),
            ("dspi://pair?h=a&a=h:1&k=AAAA".to_owned(), PairLinkError::Invalid("key")),
            (format!("dspi://pair?h=a&a=h:1&k={k}&k={k}"), PairLinkError::Invalid("repeated field")),
            (format!("dspi://pair?h=a b&a=h:1&k={k}"), PairLinkError::Invalid("hub id")),
            (format!("dspi://pair?h=a&a=h:1&k={k}&p=12%3B4"), PairLinkError::Invalid("PIN")),
            (format!("dspi://pair?h=a&a=h:1&k={k}&p=%ZZ"), PairLinkError::Invalid("encoding")),
            (format!("dspi://pair?h=a&a=h/x:1&k={k}"), PairLinkError::Invalid("address")),
        ];
        for (text, want) in cases {
            assert_eq!(parse_pair_link(&text), Err(want), "{text}");
        }
    }

    // -- WP PINTLS -- (owner decision 2026-11-17) a hub once pinned stays TLS-only
    #[test]
    fn wp_pintls_unpaired_record_keeps_tls_only() {
        let pinned = Credential { token: "secret".into(), pin: Some(PIN), next: Some(NEXT), ..Credential::default() };
        let kept = unpaired_record(&pinned.encode());
        assert!(kept.starts_with("v2."), "{kept}");
        assert!(!kept.contains(&BASE64URL_NOPAD.encode(b"secret")), "the token is dropped");
        let c = Credential::parse(&kept);
        assert_eq!(c.token, "");
        assert_eq!(c.pin, None, "no key is enforced: the next pairing pins the new one");
        assert_eq!(c.next, None);
        assert!(c.pinned_before && c.tls_only());
        assert_eq!(c.previous, Some(PIN));
        // Pair Again twice: still TLS-only, same record
        assert_eq!(unpaired_record(&kept), kept);
        // A damaged v2 was pinned once too: TLS-only, no previous key
        let damaged = Credential::parse(&unpaired_record("v2.dA.short.t"));
        assert!(damaged.pinned_before && damaged.previous.is_none());
        // A pairing link's pin with no token
        let link_only = Credential { pin: Some(PIN), ..Credential::default() };
        assert!(Credential::parse(&unpaired_record(&link_only.encode())).pinned_before);
        // A v1 token or nothing: never pinned, nothing to keep
        assert_eq!(unpaired_record("tok-12-0"), "");
        assert_eq!(unpaired_record(""), "");
        assert!(!Credential::parse("tok-12-0").tls_only());
    }

    #[test]
    fn wp_pintls_superseded_round_trips_and_refuses_damage() {
        for c in [
            Credential { pinned_before: true, previous: Some(PIN), ..Credential::default() },
            Credential { pinned_before: true, ..Credential::default() },
        ] {
            let s = c.encode();
            assert!(s.ends_with(".s"), "{s}");
            assert_eq!(Credential::parse(&s), c);
        }
        // (review F2) a superseded record never carries a token: not written, and
        // one that has it is damaged (no token, still TLS only)
        let with_token = Credential { token: "t".into(), pinned_before: true, ..Credential::default() };
        assert_eq!(with_token.encode(), "v2...s");
        assert_eq!(Credential::parse(&format!("v2.dA.{}.s", pin_to_b64url(&PIN))), DAMAGED);
        assert_eq!(Credential::parse("v2.dA..s"), DAMAGED);
        // A new pin replaces the superseded state
        let repinned = Credential {
            token: "t".into(),
            pin: Some(NEXT),
            pinned_before: true,
            previous: Some(PIN),
            ..Credential::default()
        };
        let back = Credential::parse(&repinned.encode());
        assert_eq!((back.pin, back.pinned_before, back.previous), (Some(NEXT), false, None));
        // Damaged superseded records are no credential, never a token
        let p = pin_to_b64url(&PIN);
        for bad in [format!("v2..{p}.s.{p}"), "v2..AAAA.s".to_owned(), "v2.!!..s".to_owned()] {
            assert_eq!(Credential::parse(&bad), DAMAGED, "{bad}");
            assert!(Credential::parse(&unpaired_record(&bad)).pinned_before, "{bad}");
        }
        // Debug shows the state, never the token
        let c = Credential { token: "secret-token".into(), pinned_before: true, ..Credential::default() };
        let d = format!("{c:?}");
        assert!(d.contains("pinned_before: true") && !d.contains("secret-token"), "{d}");
    }

    // -- WP PINTLS -- (review) the record format under damage: thousands of
    // mutated and random records. Never a panic; anything that starts with
    // `v2.` stays TLS only whatever is done to it (only removing the record
    // allows plaintext), never yields its own text as a token, and a record
    // without a key never carries a token.
    #[test]
    fn wp_pintls_fuzz_a_v2_record_never_degrades_to_plaintext() {
        let mut state = 0x9E37_79B9_7F4A_7C15_u64;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        let seeds = [
            Credential { token: "tok-12-0".into(), pin: Some(PIN), ..Credential::default() }.encode(),
            Credential { token: "t".into(), pin: Some(PIN), next: Some(NEXT), ..Credential::default() }.encode(),
            Credential { pinned_before: true, previous: Some(PIN), ..Credential::default() }.encode(),
            Credential { pinned_before: true, ..Credential::default() }.encode(),
            "v2.".to_owned(),
        ];
        let alphabet: Vec<char> = "v2.st=-_ AZaz09\n\u{e9}%".chars().collect();
        for round in 0..20_000_usize {
            let mut text: Vec<char> = seeds[round % seeds.len()].chars().collect();
            for _ in 0..=(next() % 4) {
                let at = if text.len() <= 3 { 3 } else { 3 + (next() as usize) % (text.len() - 2) };
                let c = alphabet[(next() as usize) % alphabet.len()];
                match next() % 4 {
                    0 if at < text.len() => text[at] = c,
                    1 => text.insert(at.min(text.len()), c),
                    2 if at < text.len() => drop(text.remove(at)),
                    _ => text.truncate(at.max(3)),
                }
            }
            let text: String = text.into_iter().collect();
            assert!(text.starts_with("v2."), "{text:?}");
            let c = Credential::parse(&text);
            assert!(c.tls_only(), "{text:?} -> {c:?}");
            assert_ne!(c.token, text.trim(), "{text:?}");
            assert!(c.pin.is_some() || c.token.is_empty(), "a token with no key to check: {text:?}");
            assert_eq!(c.pinned_before, c.pin.is_none(), "{text:?}");
            // What is written back reads the same, and dropping the pairing keeps it TLS only
            assert_eq!(Credential::parse(&c.encode()), c, "{text:?}");
            let kept = Credential::parse(&unpaired_record(&text));
            assert!(kept.pinned_before && kept.token.is_empty() && kept.pin.is_none(), "{text:?} -> {kept:?}");
            assert_eq!(kept.previous, c.pin.or(c.previous), "{text:?}");
        }
        // Not a `v2` record at all: a `v1` token, nothing kept
        for v1 in ["tok", "v2", "v2x.a.b.s", "V2.a.b.s", "v3..s"] {
            assert!(!Credential::parse(v1).tls_only(), "{v1}");
            assert_eq!(unpaired_record(v1), "", "{v1}");
        }
    }
}
