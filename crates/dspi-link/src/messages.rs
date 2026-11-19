//! JSON control-plane messages (DSPi Link spec section 7).
//!
//! Envelope `{t, id}`; replies `ok`/`err` echo `id`; events have no `id`.
//! Unknown fields are ignored and unknown event types are dropped (spec 10).
//! Port of the client half of DSPi-Console `release/v1.1.6`
//! `Link/LinkMessages.swift`.

use std::fmt;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// The protocol major of DSPi Link (spec 10).
pub const PROTO_MAJOR: u32 = 1;
// -- PG-C merge -- doc reworded (PGHDEV's hub says LINK_13_MINOR); the value stays 2
/// The DSPi Link 1.2 minor: the last one before 1.3 (paged devices), and what
/// a peer that says no `proto.minor` counts as. Neither side of ours sends it
/// any more (PLAN-PG R4): our client sends [`CLIENT_PROTO_MINOR`], our hub
/// [`LINK_13_MINOR`] (`dspi-hub` `server.rs` `HUB_PROTO_MINOR`, WP PGHDEV,
/// together with `protocol`, `ext`, `legacy_devices` and `denied_ids`).
pub const PROTO_MINOR: u32 = 2;

// -- WP PGLINKC --
/// The minor this **client** sends in its `hello`: it speaks DSPi Link 1.3
/// (paged devices, `denied_ids`, the firmware result codes) and 1.2 (spec 13.1;
/// 1.1 hubs are compatible). PLAN-PG section 1.4, R4.
pub const CLIENT_PROTO_MINOR: u32 = 3;

// -- WP PGLINKC --
/// The first hub minor that is DSPi Link 1.3: policy by command id, `protocol`
/// in the device object, nothing forwarded to a legacy device (spec 13.1).
pub const LINK_13_MINOR: u32 = 3;

// -- WP PGLINKC --
/// Our hub's capability: it lists and forwards to legacy (1.1.6-generation)
/// devices, which an upstream 1.3 hub does not (PLAN-PG OD-PGg).
pub const CAP_LEGACY_DEVICES: &str = "legacy_devices";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Proto {
    pub major: u32,
    pub minor: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClientInfo {
    pub name: String,
    pub app: String,
    pub version: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct HubInfo {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub version: Option<String>,
    // -- WP SECCORE --
    /// `hello.hub.key_next` (PLAN-SEC 4.2): the hub has moved to a new key.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key_next: Option<KeyNext>,
}

// -- WP SECCORE --
/// `hello.hub.key_next`: the pin of the key the hub now serves, signed by the
/// old one. All base64url without padding; all public.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct KeyNext {
    /// SHA-256 of the new key's SubjectPublicKeyInfo (32 bytes).
    #[serde(default)]
    pub spki: String,
    /// The old key's DER SubjectPublicKeyInfo: its hash must be the client's pin.
    #[serde(default)]
    pub prev_spki: String,
    /// The old key's ECDSA P-256 SHA-256 signature (ASN.1 DER) over
    /// `"dspi-key-rotate\0" || hid || new pin`.
    #[serde(default)]
    pub sig: String,
}

/// `hello.limits` (spec 7.2). Missing values fall back to the spec minimums.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Limits {
    #[serde(default = "Limits::default_max_frame")]
    pub max_frame: u32,
    #[serde(default = "Limits::default_max_payload")]
    pub max_payload: u32,
    #[serde(default = "Limits::default_max_inflight")]
    pub max_inflight: u32,
    #[serde(default)]
    pub poll_max_hz: u32,
    #[serde(default)]
    pub poll_budget_bps: u32,
}

impl Limits {
    fn default_max_frame() -> u32 {
        16384
    }
    fn default_max_payload() -> u32 {
        8192
    }
    fn default_max_inflight() -> u32 {
        1
    }
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_frame: Self::default_max_frame(),
            max_payload: Self::default_max_payload(),
            max_inflight: Self::default_max_inflight(),
            poll_max_hz: 0,
            poll_budget_bps: 0,
        }
    }
}

/// Where a hub's PIN comes from (spec 6.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PinSource {
    /// Shown on the hub for a pairing window; pairs one client.
    Window,
    /// The hub's own long-lived PIN.
    Fixed,
}

impl PinSource {
    /// The spec's word for it.
    pub fn as_str(self) -> &'static str {
        match self {
            PinSource::Window => "window",
            PinSource::Fixed => "fixed",
        }
    }
}

/// The hub's `hello` (spec 7.2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct HelloHub {
    #[serde(default)]
    pub proto: Proto,
    #[serde(default)]
    pub hub: HubInfo,
    /// `none` or `pin`.
    #[serde(default)]
    pub auth: String,
    /// `window` or `fixed`; absent (1.1 hubs) means `window` (spec 7.2).
    #[serde(default)]
    pub pin_source: Option<String>,
    #[serde(default)]
    pub caps: Vec<String>,
    #[serde(default)]
    pub limits: Limits,
}

impl HelloHub {
    pub fn has_cap(&self, cap: &str) -> bool {
        self.caps.iter().any(|c| c == cap)
    }

    pub fn pin_required(&self) -> bool {
        self.auth == "pin"
    }

    /// `window` unless the hub says `fixed` (spec 7.2).
    pub fn pin_source(&self) -> &str {
        self.pin_source_kind().as_str()
    }

    // -- WP PGLINKC --
    /// A DSPi Link 1.3 hub (minor 3 or later): policy by command id, `protocol`
    /// in the device object. A 1.2 or older hub (`false`) classes by `bRequest`
    /// and may forward or drop a page, so nothing paged goes through it
    /// (PLAN-PG section 1.1 step 6, OD-PGe).
    pub fn is_link_13(&self) -> bool {
        self.proto.minor >= LINK_13_MINOR
    }

    // -- WP PGLINKC --
    /// The hub forwards to legacy devices (our hub, cap `legacy_devices`).
    pub fn serves_legacy_devices(&self) -> bool {
        self.has_cap(CAP_LEGACY_DEVICES)
    }

    /// [`Self::pin_source`], typed.
    pub fn pin_source_kind(&self) -> PinSource {
        match self.pin_source.as_deref() {
            Some("fixed") => PinSource::Fixed,
            _ => PinSource::Window,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Policy {
    /// `[bRequest, dir]` pairs this session may not send (dir 0 = SET, 1 = GET).
    #[serde(default)]
    pub denied: Vec<[u16; 2]>,
    // -- WP PGLINKC --
    /// `[command id, dir]` pairs (spec 1.3 section 7.3), for paged devices. A
    /// 1.3 upstream hub sends `denied` empty and fills only this; our hub fills
    /// both (PLAN-PG section 1.5). Absent on 1.2 hubs.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub denied_ids: Vec<[u32; 2]>,
}

/// The `ok` to `auth.token` / `auth.pair` (spec 7.3).
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct AuthOk {
    #[serde(default)]
    pub session: u32,
    #[serde(default)]
    pub role: String,
    #[serde(default)]
    pub token: Option<String>,
    #[serde(default)]
    pub policy: Option<Policy>,
    // -- WP SECCORE --
    /// The hub's key pin (base64url), which the client checks against the TLS key
    /// (PLAN-SEC 4.3). Absent from hubs without TLS.
    #[serde(default)]
    pub hub_key: Option<String>,
}

/// By hand, so a token never reaches a log.
impl fmt::Debug for AuthOk {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AuthOk")
            .field("session", &self.session)
            .field("role", &self.role)
            .field("token", &self.token.as_ref().map(|_| "<redacted>"))
            .field("policy", &self.policy)
            .finish()
    }
}

/// One entry of `device.list` / `device.added` / `device.changed` (spec 7.4).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct DeviceInfo {
    pub handle: u32,
    #[serde(default)]
    pub serial: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub platform: Option<u32>,
    #[serde(default)]
    pub fw: Option<String>,
    /// Absent = unknown, not 0 (spec 7.4).
    #[serde(default)]
    pub fw_beta: Option<u32>,
    #[serde(default)]
    pub outputs: Option<u32>,
    #[serde(default)]
    pub inputs: Option<u32>,
    #[serde(default)]
    pub wire_version: Option<u32>,
    /// `online`, `offline`, `updating`.
    #[serde(default)]
    pub state: String,
    /// `usb` or `uart`.
    #[serde(default)]
    pub link: String,
    #[serde(default)]
    pub locked_by: Option<u32>,
    // -- WP PGLINKC --
    /// The device's vendor protocol version (spec 1.3 section 7.4, from
    /// `SYS.PROTOCOL_INFO` bytes 4 and 5): present for a paged (1.1.7) device,
    /// absent for a legacy one and on every 1.2 hub.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub protocol: Option<Proto>,
    // -- WP PGLINKC --
    /// Our hub only (additive): whether the device has our extension channel.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ext: Option<bool>,
    // -- PG-C merge --
    /// Our hub only (additive): the board id (PLAN-PG section 1.9: 1 the
    /// DSPi-Zero) when it is not 0, from the platform's byte 7 or, for a paged
    /// Zero, the USB descriptor the hub sees and its clients do not. Absent
    /// for a Pico and on every other hub. A `u32` as the other numbers here:
    /// a value past a byte is no board this client knows.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub board: Option<u32>,
}

// -- WP PGLINKC --
impl DeviceInfo {
    /// The device speaks the paged protocol whose major this client knows
    /// (`protocol.major` 1): only then may paged commands and polls go to it
    /// (spec 1.3 section 7.4).
    pub fn is_paged(&self) -> bool {
        self.protocol.is_some_and(|p| p.major == PROTO_MAJOR)
    }
}

/// The `ok` to `device.snapshot` (spec 7.5).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Snapshot {
    #[serde(default)]
    pub handle: u32,
    #[serde(default)]
    pub wire_version: Option<u32>,
    #[serde(default)]
    pub age_ms: Option<u64>,
    #[serde(default)]
    pub bulk_b64: Option<String>,
    #[serde(default)]
    pub status_b64: Option<String>,
}

impl Snapshot {
    pub fn bulk(&self) -> Option<Vec<u8>> {
        self.bulk_b64.as_deref().and_then(|s| data_encoding::BASE64.decode(s.as_bytes()).ok())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PollSpec {
    pub slot: u8,
    pub req: u8,
    pub val: u16,
    pub idx: u16,
    pub len: u16,
    pub hz: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PollGrant {
    pub slot: u8,
    pub hz: u32,
}

/// An `err` reply (spec 7.1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ErrReply {
    #[serde(default)]
    pub id: Option<u64>,
    #[serde(default)]
    pub code: String,
    #[serde(default)]
    pub msg: Option<String>,
    #[serde(default)]
    pub field: Option<String>,
}

/// `GET /dspi/v1/info` (spec 3.2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct InfoDoc {
    #[serde(default)]
    pub proto: Proto,
    #[serde(default)]
    pub hub: HubInfo,
    #[serde(default)]
    pub auth: String,
    #[serde(default)]
    pub tls: bool,
    #[serde(default)]
    pub ws: Option<String>,
    #[serde(default)]
    pub devices: Vec<InfoDevice>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct InfoDevice {
    #[serde(default)]
    pub serial: String,
    #[serde(default)]
    pub name: String,
}

/// A message from the hub, by `t`.
#[derive(Clone, PartialEq)]
pub enum Inbound {
    Hello(HelloHub),
    Ok {
        id: u64,
        body: Value,
    },
    Err(ErrReply),
    DeviceAdded(DeviceInfo),
    DeviceChanged(DeviceInfo),
    DeviceRemoved {
        handle: u32,
        serial: String,
    },
    PollError {
        handle: u32,
        slot: Option<u8>,
        code: Option<String>,
    },
    /// Any other event (fw.progress, hub.config.changed, ...): ignored by this client,
    /// but handed whole to the session's listener ([`crate::client::LinkEvents::hub_event`]).
    Other {
        t: String,
        // -- WP HUBADMIN -- the message as it came, for the listener
        body: Value,
    },
}

/// By hand: the body of an `ok` may carry a token, so it never reaches a log.
impl fmt::Debug for Inbound {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Inbound::Hello(h) => f.debug_tuple("Hello").field(h).finish(),
            Inbound::Ok { id, .. } => f.debug_struct("Ok").field("id", id).finish_non_exhaustive(),
            Inbound::Err(e) => f.debug_tuple("Err").field(e).finish(),
            Inbound::DeviceAdded(d) => f.debug_tuple("DeviceAdded").field(d).finish(),
            Inbound::DeviceChanged(d) => f.debug_tuple("DeviceChanged").field(d).finish(),
            Inbound::DeviceRemoved { handle, serial } => {
                f.debug_struct("DeviceRemoved").field("handle", handle).field("serial", serial).finish()
            }
            Inbound::PollError { handle, slot, code } => {
                f.debug_struct("PollError").field("handle", handle).field("slot", slot).field("code", code).finish()
            }
            Inbound::Other { t, .. } => f.debug_struct("Other").field("t", t).finish_non_exhaustive(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum MessageError {
    #[error("not JSON: {0}")]
    NotJson(String),
    #[error("no string `t`")]
    NoType,
    #[error("malformed `{t}`: {msg}")]
    Malformed { t: String, msg: String },
}

fn field<T: serde::de::DeserializeOwned>(t: &str, v: Value) -> Result<T, MessageError> {
    serde_json::from_value(v).map_err(|e| MessageError::Malformed { t: t.to_owned(), msg: e.to_string() })
}

impl Inbound {
    pub fn parse(text: &str) -> Result<Inbound, MessageError> {
        let v: Value = serde_json::from_str(text).map_err(|e| MessageError::NotJson(e.to_string()))?;
        let t = v.get("t").and_then(Value::as_str).ok_or(MessageError::NoType)?.to_owned();
        Ok(match t.as_str() {
            "hello" => Inbound::Hello(field(&t, v)?),
            "ok" => {
                let id = v
                    .get("id")
                    .and_then(Value::as_u64)
                    .ok_or(MessageError::Malformed { t: t.clone(), msg: "ok without id".into() })?;
                Inbound::Ok { id, body: v }
            }
            "err" => Inbound::Err(field(&t, v)?),
            "device.added" | "device.changed" => {
                let dev = v
                    .get("device")
                    .cloned()
                    .ok_or(MessageError::Malformed { t: t.clone(), msg: "no device".into() })?;
                let dev: DeviceInfo = field(&t, dev)?;
                if t == "device.added" {
                    Inbound::DeviceAdded(dev)
                } else {
                    Inbound::DeviceChanged(dev)
                }
            }
            "device.removed" => Inbound::DeviceRemoved {
                handle: v.get("handle").and_then(Value::as_u64).unwrap_or(u64::MAX) as u32,
                serial: v.get("serial").and_then(Value::as_str).unwrap_or_default().to_owned(),
            },
            "poll.error" => Inbound::PollError {
                handle: v.get("handle").and_then(Value::as_u64).unwrap_or(0) as u32,
                slot: v.get("slot").and_then(Value::as_u64).map(|s| s as u8),
                code: v.get("code").and_then(Value::as_str).map(str::to_owned),
            },
            _ => Inbound::Other { t, body: v },
        })
    }
}

/// Requests and the client hello, as JSON text.
pub mod out {
    use super::*;

    pub fn hello(client: &ClientInfo) -> String {
        // -- WP PGLINKC -- the client's own minor (R4), not the hub's
        json!({"t": "hello", "proto": {"major": PROTO_MAJOR, "minor": CLIENT_PROTO_MINOR}, "client": client})
            .to_string()
    }

    pub fn auth_token(id: u64, token: &str) -> String {
        json!({"t": "auth.token", "id": id, "token": token}).to_string()
    }

    /// `role` None leaves it to the hub (the `none`-mode courtesy probe).
    pub fn auth_pair(id: u64, pin: &str, name: &str, role: Option<&str>) -> String {
        let mut v = json!({"t": "auth.pair", "id": id, "pin": pin, "name": name});
        if let Some(r) = role {
            v["role"] = json!(r);
        }
        v.to_string()
    }

    // -- WP SECPAKE --
    /// `auth.pair2` step 1: our SPAKE2 message (base64url).
    pub fn auth_pair2_start(id: u64, name: &str, role: &str, pake: &str) -> String {
        json!({"t": "auth.pair2", "id": id, "step": 1, "name": name, "role": role, "pake": pake}).to_string()
    }

    // -- WP SECPAKE --
    /// `auth.pair2` step 2: our confirmation MAC (base64url).
    pub fn auth_pair2_confirm(id: u64, confirm: &str) -> String {
        json!({"t": "auth.pair2", "id": id, "step": 2, "confirm": confirm}).to_string()
    }

    // -- WP SECPAKE --
    /// `auth.rotate` (OD-SECg): a fresh token for this session's client.
    pub fn auth_rotate(id: u64) -> String {
        json!({"t": "auth.rotate", "id": id}).to_string()
    }

    pub fn device_list(id: u64) -> String {
        json!({"t": "device.list", "id": id}).to_string()
    }

    pub fn device_snapshot(id: u64, handle: u8) -> String {
        json!({"t": "device.snapshot", "id": id, "handle": handle}).to_string()
    }

    pub fn poll_subscribe(id: u64, handle: u8, polls: &[PollSpec]) -> String {
        json!({"t": "poll.subscribe", "id": id, "handle": handle, "polls": polls}).to_string()
    }

    pub fn poll_unsubscribe(id: u64, handle: u8, slots: &[u8]) -> String {
        json!({"t": "poll.unsubscribe", "id": id, "handle": handle, "slots": slots}).to_string()
    }

    pub fn lock_acquire(id: u64, handle: u8, reason: &str, timeout_ms: u32) -> String {
        json!({"t": "lock.acquire", "id": id, "handle": handle, "reason": reason, "timeout_ms": timeout_ms}).to_string()
    }

    pub fn lock_release(id: u64, handle: u8) -> String {
        json!({"t": "lock.release", "id": id, "handle": handle}).to_string()
    }
}

// -- WP HUBADMIN --
#[cfg(test)]
mod tests_wp_hubadmin {
    use super::*;

    /// An event this client does not handle keeps its body, for the session's
    /// listener (`LinkEvents::hub_event`), and the body never reaches a log.
    #[test]
    fn wp_hubadmin_other_events_keep_their_body() {
        let text = r#"{"t":"limits.held","v":1,"serial":"S1","handle":2,"rows":["a"]}"#;
        let Ok(Inbound::Other { t, body }) = Inbound::parse(text) else {
            panic!("not Other");
        };
        assert_eq!(t, "limits.held");
        assert_eq!(body["rows"][0], "a");
        assert_eq!(body["serial"], "S1");
        let shown = format!("{:?}", Inbound::parse(text).unwrap());
        assert!(!shown.contains("rows"), "{shown}");
    }
}

// -- WP PGLINKC --
#[cfg(test)]
mod tests_wp_pglinkc {
    use super::*;

    /// Spec 1.3 section 7.4: `protocol` for a paged device, absent for a legacy
    /// one; our additive `ext`. A 1.2 object serialises as before.
    #[test]
    fn wp_pglinkc_the_device_object_carries_protocol_and_ext() {
        let text =
            r#"{"t":"device.added","device":{"handle":0,"serial":"S","protocol":{"major":1,"minor":0},"ext":true}}"#;
        let Ok(Inbound::DeviceAdded(d)) = Inbound::parse(text) else { panic!("not DeviceAdded") };
        assert_eq!(d.protocol, Some(Proto { major: 1, minor: 0 }));
        assert_eq!(d.ext, Some(true));
        assert!(d.is_paged());
        let newer = DeviceInfo { protocol: Some(Proto { major: 2, minor: 0 }), ..d.clone() };
        assert!(!newer.is_paged(), "a later major may change what a command means");
        let legacy = DeviceInfo { protocol: None, ext: None, ..d };
        assert!(!legacy.is_paged());
        let v = serde_json::to_value(&legacy).unwrap();
        assert!(v.get("protocol").is_none() && v.get("ext").is_none(), "{v}");
    }

    /// Spec 1.3 section 7.3: `denied_ids` next to `denied`, both optional.
    #[test]
    fn wp_pglinkc_policy_parses_denied_ids() {
        let p: Policy = serde_json::from_str(r#"{"denied": [], "denied_ids": [[304,0],[560,0]]}"#).unwrap();
        assert!(p.denied.is_empty());
        assert_eq!(p.denied_ids, vec![[304, 0], [560, 0]]);
        let old: Policy = serde_json::from_str(r#"{"denied": [[83,0]]}"#).unwrap();
        assert!(old.denied_ids.is_empty());
    }

    /// R4: the client says 1.3; `PROTO_MINOR` is the 1.2 minor (our hub says
    /// `LINK_13_MINOR` since PGHDEV).
    #[test]
    fn wp_pglinkc_hello_minor_and_hub_kinds() {
        let v: Value =
            serde_json::from_str(&out::hello(&ClientInfo { name: "n".into(), app: "a".into(), version: "v".into() }))
                .unwrap();
        assert_eq!(v["proto"]["minor"], CLIENT_PROTO_MINOR);
        assert_eq!(PROTO_MINOR, 2);
        let hub = |minor, caps: &[&str]| HelloHub {
            proto: Proto { major: 1, minor },
            caps: caps.iter().map(|c| c.to_string()).collect(),
            ..HelloHub::default()
        };
        assert!(!hub(2, &[]).is_link_13());
        assert!(hub(3, &[]).is_link_13());
        assert!(hub(3, &[CAP_LEGACY_DEVICES]).serves_legacy_devices());
        assert!(!hub(3, &[]).serves_legacy_devices());
    }
}
