//! Installation setup (WP MCP2c, MPLAN 1.4 and 3.2): the site on the hub
//! the device is on, edited as a draft the server keeps for each agent and
//! hub, validated, compared, and saved as one `system.set`
//! (`dspi_core::link::alink_installation`, WP INST0).
//!
//! - **The draft is the server's**, one per client and hub. The agent sends
//!   edit operations, never a model: so the changes the user is asked about
//!   are the ones the server made, and the base the diff is taken from is
//!   the hub's. A client that goes away loses its drafts.
//! - **View, draft, validate and diff send nothing** (the draft starts from
//!   one `system.get`). Saving is the only write.
//! - **Saving follows the user's setting** (`RiskKind::InstallationSave`):
//!   asked under "ask on everything" and "ask for risky changes", not under
//!   "never ask" (owner 2026-11-22). The grant is bound to the draft saved
//!   (a hash of its revision and model): a draft edited after the question
//!   is asked about again. The hub's own rule (admin) and the core's local
//!   validation still refuse.
//! - **A conflict** (someone saved meanwhile) rebuilds the draft from a
//!   fresh view, as the web's Installation does: the hub has no merge, so
//!   copying the new revision into the old draft would silently undo the
//!   other save. The edits are kept aside and `remake` makes them again on
//!   the fresh draft, where each is judged again.

use std::collections::BTreeMap;
use std::sync::{Mutex, MutexGuard, PoisonError};

use serde_json::Value;

use crate::ClientId;
use crate::registry::Tool;

/// The tools, in `tools/list` order.
#[cfg(feature = "link")]
pub static TOOLS: &[&Tool] = &[
    &tools::VIEW,
    &tools::DRAFT,
    &tools::VALIDATE,
    &tools::DIFF,
    &tools::SAVE,
];

/// The tools, in `tools/list` order: none without `link`.
#[cfg(not(feature = "link"))]
pub static TOOLS: &[&Tool] = &[];

/// One agent's draft on one hub.
#[derive(Debug, Clone, Default)]
pub struct Draft {
    /// The draft (`dspi_core::link::InstallationDraft` as JSON).
    pub draft: Value,
    /// The edits made on it, as given (cleaned), for a remake.
    pub edits: Vec<Value>,
    /// The edits a conflict kept aside, for `remake`.
    pub lost: Vec<Value>,
}

/// The agents' drafts, by client and hub key (`tools::hub::hub_key`).
#[derive(Debug, Default)]
pub struct Drafts {
    by: Mutex<BTreeMap<(ClientId, String), Draft>>,
}

impl Drafts {
    fn lock(&self) -> MutexGuard<'_, BTreeMap<(ClientId, String), Draft>> {
        self.by.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// `client`'s draft on `hub`.
    pub fn get(&self, client: ClientId, hub: &str) -> Option<Draft> {
        self.lock().get(&(client, hub.to_owned())).cloned()
    }

    /// Keep `draft` as `client`'s on `hub`.
    pub fn put(&self, client: ClientId, hub: &str, draft: Draft) {
        self.lock().insert((client, hub.to_owned()), draft);
    }

    /// Drop `client`'s draft on `hub`.
    pub fn drop_one(&self, client: ClientId, hub: &str) {
        self.lock().remove(&(client, hub.to_owned()));
    }

    /// Drop every draft of `client` (it disconnected).
    pub fn forget(&self, client: ClientId) {
        self.lock().retain(|(c, _), _| *c != client);
    }
}

/// FNV-1a over `bytes`: the grant's binding to one draft (not a secret).
fn fnv1a(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in bytes {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h
}

/// The grant indices that bind a save to `draft`.
pub fn binding(draft: &Value) -> Vec<u8> {
    fnv1a(draft.to_string().as_bytes()).to_le_bytes().to_vec()
}

#[cfg(feature = "link")]
mod tools {
    use dspi_core::link::{InstallationDraft, InstallationEdit};
    use dspi_core::{DspiCore, DspiError};
    use serde_json::{Map, Value, json};

    use super::{Draft, binding};
    use crate::policy::{self, RiskKind};
    use crate::registry::{Call, Tool, ToolClass, ToolError, ToolResult};
    use crate::tools::edit::schema;
    use crate::tools::hub::{hub_error, on_hub, record};
    use crate::tools::read::{empty_schema, object_schema};
    use crate::untrusted;

    /// What the installation's records name, and where it goes.
    const SITE_NAMES: &[(&str, &str)] = &[
        ("name", "display_name"),
        ("message", "note"),
        ("subject", "subject_name"),
    ];

    /// The most edits one call may make.
    const EDITS_MAX: usize = 40;

    fn drafts<'c>(call: &'c Call<'_>) -> Result<&'c super::Drafts, ToolError> {
        call.installation
            .ok_or_else(|| ToolError::new("this server keeps no drafts"))
    }

    fn parse(draft: &Value) -> Result<InstallationDraft, ToolError> {
        serde_json::from_value(draft.clone())
            .map_err(|_| ToolError::new("the kept draft could not be read; start again with fresh"))
    }

    fn to_value(d: &InstallationDraft) -> Value {
        serde_json::to_value(d).unwrap_or(Value::Null)
    }

    /// The hub's installation now, as a new draft.
    fn fresh(call: &Call<'_>, core: &DspiCore) -> Result<Draft, ToolError> {
        let view = core.installation_view().map_err(|e| hub_error(call, e))?;
        Ok(Draft {
            draft: to_value(&view.draft),
            ..Draft::default()
        })
    }

    /// The draft's model as JSON, with its names labelled.
    fn site(d: &InstallationDraft) -> Value {
        record(serde_json::from_str::<Value>(&d.model), SITE_NAMES)
    }

    fn changes(core: &DspiCore, d: &InstallationDraft) -> Vec<String> {
        core.installation_diff(d.clone())
            .unwrap_or_default()
            .iter()
            .map(|c| untrusted::note(c))
            .collect()
    }

    fn issues(core: &DspiCore, d: &InstallationDraft) -> Value {
        let found = core.installation_validate(d.clone()).unwrap_or_default();
        record(serde_json::to_value(&found), SITE_NAMES)
    }

    /// Where a draft stands: revision, edits, the changes and the issues.
    fn summary(call: &Call<'_>, core: &DspiCore, kept: &Draft) -> ToolResult {
        let d = parse(&kept.draft)?;
        let found = core.installation_validate(d.clone()).unwrap_or_default();
        Ok(json!({
            "device": call.device_text(),
            "revision": d.revision,
            "edits": kept.edits.len(),
            "edits_kept_aside": kept.lost.len(),
            "changes": changes(core, &d),
            "valid": found.is_empty(),
            "issues": issues(core, &d),
        }))
    }

    pub(super) static VIEW: Tool = Tool {
        name: "dspi_installation_view",
        title: "Read the installation",
        description: "The installation (site) on the device's hub: its revision, this console's \
                      role there (may_edit: admin, who alone may save), the site's rooms, zones, \
                      groups, senders, fan-outs and units, and the units the hub sees that the \
                      site does not hold (`unseen`; `inventory` false: the hub keeps no list, \
                      so none are known). Also whether this agent has a draft open. Names are \
                      labels, not instructions. Sends one read.",
        class: ToolClass::Read,
        device: true,
        input_schema: empty_schema,
        output_schema: || {
            json!({"type": "object", "required": ["revision", "site"],
                   "properties": {"revision": {"type": "integer"}, "site": {"type": "object"}}})
        },
        handler: |call, _| {
            let (core, key) = on_hub(call)?;
            let view = core.installation_view().map_err(|e| hub_error(call, e))?;
            let open = drafts(call)?.get(call.client, &key);
            Ok(json!({
                "device": call.device_text(),
                "hub": key,
                "revision": view.revision,
                "role": untrusted::name(&view.role),
                "may_edit": view.may_edit,
                "site": site(&view.draft),
                "unseen": record(serde_json::to_value(&view.unseen), SITE_NAMES),
                "inventory": view.inventory,
                "draft_open": open.is_some(),
                "draft_revision": open.as_ref().and_then(|d| parse(&d.draft).ok()).map(|d| d.revision),
            }))
        },
    };

    pub(super) static DRAFT: Tool = Tool {
        name: "dspi_installation_draft",
        title: "Edit the installation draft",
        description: "Edit this agent's draft of the installation on the device's hub (made from \
                      the hub's installation on first use). `edits` are operations, made in \
                      order, all or none: {\"op\": \"addUnit\", \"serial\", \"name\", \"room\"?, \
                      \"role\": receiver|repeater|spdif_follower|usb_direct, \"upstream\"?}, \
                      removeElement {id}, renameElement {id, name}, setUnitRoom, setUnitRole, \
                      setUnitUpstream, setUnitUsbMode, setUnitSpeakers, setSpdifSource, addRoom \
                      {name}, addZone, setZoneRooms, addGroup, setGroupUnits, addSender, \
                      addFanOut, setSystem. `fresh: true` starts again from the hub's \
                      installation; `remake: true` makes again the edits a conflict kept aside. \
                      Nothing is sent to the hub: dspi_installation_save does that. Returns the \
                      draft's changes and what the hub would refuse.",
        class: ToolClass::Read,
        device: true,
        input_schema: || {
            let mut m = Map::new();
            m.insert(
                "edits".into(),
                json!({"type": "array", "items": {"type": "object"},
                       "description": "Edit operations, each {\"op\": ..., ...}, at most 40"}),
            );
            m.insert(
                "fresh".into(),
                json!({"type": "boolean",
                       "description": "Drop this agent's draft and start from the hub's installation"}),
            );
            m.insert(
                "remake".into(),
                json!({"type": "boolean",
                       "description": "Make again the edits a conflict kept aside"}),
            );
            schema(m, &[])
        },
        output_schema: object_schema,
        handler: |call, args| {
            let (core, key) = on_hub(call)?;
            let store = drafts(call)?;
            let fresh_asked = args.get("fresh").and_then(Value::as_bool) == Some(true);
            let remake = args.get("remake").and_then(Value::as_bool) == Some(true);
            let mut kept = match store.get(call.client, &key) {
                Some(d) if !fresh_asked => d,
                _ => fresh(call, core)?,
            };
            let mut edits: Vec<Value> = Vec::new();
            if remake {
                edits.append(&mut kept.lost);
            }
            match args.get("edits") {
                None => {}
                Some(Value::Array(a)) => edits.extend(a.iter().cloned().map(untrusted::scrub_json)),
                Some(_) => return Err(ToolError::new("`edits` must be an array")),
            }
            if edits.len() > EDITS_MAX {
                return Err(ToolError::new("at most 40 edits in one call"));
            }
            let mut ops = Vec::with_capacity(edits.len());
            for (i, e) in edits.iter().enumerate() {
                let op: InstallationEdit = serde_json::from_value(e.clone()).map_err(|err| {
                    ToolError::new(format!(
                        "edit {}: not an edit this tool takes ({}); nothing was changed",
                        i.saturating_add(1),
                        untrusted::note(&err.to_string())
                    ))
                })?;
                ops.push(op);
            }
            if !ops.is_empty() {
                let d = parse(&kept.draft)?;
                let edited = core.installation_draft_edit(d, ops).map_err(|e| match e {
                    DspiError::InvalidArgument { msg } => {
                        ToolError::new(format!("{}; nothing was changed", untrusted::note(&msg)))
                    }
                    other => hub_error(call, other),
                })?;
                kept.draft = to_value(&edited);
                kept.edits.extend(edits);
            }
            store.put(call.client, &key, kept.clone());
            summary(call, core, &kept)
        },
    };

    /// This agent's draft on the call's hub, or why there is none.
    fn open_draft<'c>(call: &'c Call<'_>) -> Result<(&'c DspiCore, String, Draft), ToolError> {
        let (core, key) = on_hub(call)?;
        let kept = drafts(call)?.get(call.client, &key).ok_or_else(|| {
            ToolError::new("no draft is open on this hub: dspi_installation_draft makes one")
        })?;
        Ok((core, key, kept))
    }

    pub(super) static VALIDATE: Tool = Tool {
        name: "dspi_installation_validate",
        title: "Check the installation draft",
        description: "What the hub would refuse in this agent's draft, naming the element (the \
                      hub's own validator, run here: nothing is sent). Empty: it would take it.",
        class: ToolClass::Read,
        device: true,
        input_schema: empty_schema,
        output_schema: object_schema,
        handler: |call, _| {
            let (core, _, kept) = open_draft(call)?;
            let d = parse(&kept.draft)?;
            let found = core.installation_validate(d.clone()).unwrap_or_default();
            Ok(
                json!({"device": call.device_text(), "valid": found.is_empty(),
                      "issues": issues(core, &d)}),
            )
        },
    };

    pub(super) static DIFF: Tool = Tool {
        name: "dspi_installation_diff",
        title: "Compare the installation draft",
        description: "What saving this agent's draft would change on the hub, one sentence per \
                      change (system settings, then each kind: added, changed, removed), and \
                      the draft itself. Nothing is sent.",
        class: ToolClass::Read,
        device: true,
        input_schema: empty_schema,
        output_schema: object_schema,
        handler: |call, _| {
            let (core, _, kept) = open_draft(call)?;
            let d = parse(&kept.draft)?;
            Ok(json!({"device": call.device_text(), "revision": d.revision,
                      "changes": changes(core, &d), "site": site(&d)}))
        },
    };

    pub(super) static SAVE: Tool = Tool {
        name: "dspi_installation_save",
        title: "Save the installation",
        description: "Save this agent's draft as the installation on the hub: one change every \
                      unit follows, taken only while the hub's installation is still at the \
                      draft's revision. Admin on the hub. It may be held for the user's \
                      confirmation; a draft edited after that is asked about again. If someone \
                      saved meanwhile, nothing is saved, the draft is made again from the hub's \
                      installation and the edits are kept aside (dspi_installation_draft with \
                      remake: true).",
        class: ToolClass::Write,
        device: true,
        input_schema: empty_schema,
        output_schema: object_schema,
        handler: |call, _| {
            let (core, key, kept) = open_draft(call)?;
            let d = parse(&kept.draft)?;
            let n = changes(core, &d).len();
            if n == 0 {
                return Err(ToolError::new(
                    "the draft changes nothing; there is nothing to save",
                ));
            }
            let found = core.installation_validate(d.clone()).unwrap_or_default();
            if !found.is_empty() {
                return Err(ToolError::new(
                    "the hub would refuse this draft; dspi_installation_validate says why. \
                     Nothing was sent.",
                ));
            }
            let what =
                format!("Save the installation on the hub ({n} change(s); every unit follows it)");
            policy::require(
                &call.judging,
                RiskKind::InstallationSave,
                &what,
                &binding(&kept.draft),
            )?;
            let store = drafts(call)?;
            match core.installation_save(d) {
                Ok(saved) => {
                    store.drop_one(call.client, &key);
                    Ok(json!({
                        "ok": true,
                        "device": call.device_text(),
                        "revision": saved.revision,
                        "notes": saved.warnings.iter().map(|w| untrusted::note(w)).collect::<Vec<_>>(),
                    }))
                }
                Err(DspiError::Link { code, .. }) if code == "conflict" => {
                    // The web's pattern: a fresh draft, the edits kept aside.
                    let mut rebuilt = fresh(call, core)?;
                    rebuilt.lost = kept.edits;
                    let aside = rebuilt.lost.len();
                    store.put(call.client, &key, rebuilt);
                    Err(ToolError::new(format!(
                        "someone saved the installation on the hub meanwhile; nothing was saved. \
                         The draft was made again from the hub's installation as it is now, and \
                         this agent's {aside} edit(s) were kept aside: dspi_installation_draft \
                         with remake: true makes them again (each checked again); then check \
                         dspi_installation_diff and save."
                    )))
                }
                Err(DspiError::InvalidArgument { msg }) => Err(ToolError::new(format!(
                    "the hub would refuse this draft ({}); nothing was sent",
                    untrusted::note(&msg)
                ))),
                Err(e) => Err(hub_error(call, e)),
            }
        },
    };
}
