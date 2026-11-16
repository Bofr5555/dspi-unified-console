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
//!
//! A string that starts with `v2.` but does not parse is no credential at all (pair
//! again; -- WP SECBFIXA --): it is never sent as a token. Anything else that is
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
}

/// By hand: the token never reaches a log.
impl fmt::Debug for Credential {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Credential")
            .field("token", &if self.token.is_empty() { "" } else { "<redacted>" })
            .field("pin", &self.pin.map(|p| pin_display(&p)))
            .field("next", &self.next.map(|p| pin_display(&p)))
            .finish()
    }
}

impl Credential {
    /// A credential string (`v1` or `v2`), trimmed. Never fails. -- WP SECBFIXA --
    /// A string that starts with `v2.` but is damaged is **no credential** (pair
    /// again): it is never sent as a bare token, and a pinned record never
    /// degrades to an unpinned one that could fall back to plaintext. Anything
    /// else that is not a `v2` is a `v1` token.
    pub fn parse(text: &str) -> Credential {
        let text = text.trim();
        if let Some(c) = Self::parse_v2(text) {
            return c;
        }
        if text.starts_with("v2.") {
            log::warn!("dspi-link: a stored v2 credential is damaged; pairing again is needed");
            return Credential::default();
        }
        Credential { token: text.to_owned(), pin: None, next: None }
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
        let pin = pin_from_b64url(parts.next()?)?;
        let flags = parts.next()?;
        if !flags.chars().all(|c| c.is_ascii_lowercase()) {
            return None;
        }
        let next = match parts.next() {
            Some(n) => Some(pin_from_b64url(n)?),
            None => None,
        };
        if parts.next().is_some() {
            return None;
        }
        Some(Credential { token, pin: Some(pin), next })
    }

    /// The string to store: the bare token without a pin (`v1`), else `v2`.
    pub fn encode(&self) -> String {
        let Some(pin) = &self.pin else {
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
        Credential { token: String::new(), pin: Some(self.key), next: None }
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

    #[test]
    fn wp_seccore_v1_is_the_bare_token() {
        let c = Credential::parse(" tok-12-0 ");
        assert_eq!(c, Credential { token: "tok-12-0".into(), pin: None, next: None });
        assert_eq!(c.encode(), "tok-12-0");
    }

    #[test]
    fn wp_seccore_v2_round_trips() {
        for c in [
            Credential { token: "abc.def+/=".into(), pin: Some(PIN), next: None },
            Credential { token: String::new(), pin: Some(PIN), next: None },
            Credential { token: "t".into(), pin: Some(PIN), next: Some(NEXT) },
        ] {
            let s = c.encode();
            assert!(s.starts_with("v2."), "{s}");
            assert_eq!(Credential::parse(&s), c);
        }
        // A successor equal to the pin is not written
        let same = Credential { token: "t".into(), pin: Some(PIN), next: Some(PIN) };
        assert_eq!(Credential::parse(&same.encode()).next, None);
    }

    // -- WP SECBFIXA -- (review secb F1) a damaged v2 is no credential, never a token
    #[test]
    fn wp_seccore_malformed_v2_is_no_credential() {
        let good = Credential { token: "t".into(), pin: Some(PIN), next: None }.encode();
        let pin = pin_to_b64url(&PIN);
        for bad in [
            "v2.dA.short.t".to_owned(),
            "v2.dA".to_owned(),
            format!("{good}.x.y"),
            good.replace(".t", ".T"),
            "v2.!!.AAAA.t".to_owned(),
            good.replace(&pin, &pin[..pin.len() - 1]),
        ] {
            assert_eq!(Credential::parse(&bad), Credential::default(), "{bad}");
        }
        // Not v2-shaped at all: still a v1 token
        assert_eq!(Credential::parse("v2token").token, "v2token");
        assert_eq!(Credential::parse("v3.x").token, "v3.x");
    }

    #[test]
    fn wp_seccore_debug_never_shows_the_token() {
        let c = Credential { token: "secret-token".into(), pin: Some(PIN), next: None };
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
}
