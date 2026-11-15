//! The control-surface editing rules, for clients outside Rust (WP CG20).
//!
//! [`dspi_session::cs_rows`] says what a page shows for a record and what an edit
//! makes of it; this module gives that the device it needs and speaks JSON, as
//! `cs.rs` does. It reads only what the core already holds: no call here sends
//! anything to the device, so a client may ask as often as it redraws.
//!
//! A draft is the same JSON as in [`DspiCore::cs_snapshot`] and
//! [`DspiCore::cs_apply`], and it is a trust boundary: numbers are range-checked
//! into their wire types, and a bad one refuses the call. The kinds are `binding`,
//! `ir`, `step`, `page`, `display` and `group`.

use serde_json::{Value, json};

use dspi_proto::wire::BulkPacket;
use dspi_session::DeviceState;
use dspi_session::cs_model::{CsData, StaticContext, make_binding, slot_owner};
use dspi_session::cs_rows::{self, Draft};
use dspi_session::pins::{PinClaim, PinRole, claims_from_state};
use dspi_session::surfaces::{CsDisplayCfg, CsDisplayPage, explain_status, status as cs_status};

use crate::cs::{
    binding_from_json, binding_json, display_cfg_from_json, failure, group_from_json, invalid,
    ir_from_json, ir_json, page_from_json, step_from_json, step_json,
};
use crate::{DspiCore, DspiError, Inner, Result};

/// What the rules read about the open device, by value so the lock need not be
/// held while they run.
struct Device {
    state: DeviceState,
    data: CsData,
    claims: Vec<PinClaim>,
    control_i2c_sda: Option<u8>,
    preset_names: Vec<String>,
}

impl Device {
    fn of(inner: &mut Inner) -> Result<Self> {
        let conn = inner.conn.as_mut().ok_or(DspiError::Disconnected)?;
        let serial = conn.serial.clone();
        let caps = conn.session()?.capabilities().clone();
        let data = inner
            .cs
            .data_for(&serial)
            .cloned()
            .ok_or_else(|| invalid("control surfaces have not been read from this device"))?;
        let s = &inner.state;
        let bulk = BulkPacket::decode(s.bulk_raw.clone()).map_err(|e| invalid(e.to_string()))?;
        let state = DeviceState::new(caps, bulk);

        // What the bulk image proves, then what holds a pin while it is live,
        // in the firmware's order: the first claim on a pin is the one named.
        let mut claims = claims_from_state(&state);
        let mut claim = |gpio: u8, owner: String| {
            if !claims.iter().any(|c: &PinClaim| c.gpio == gpio) {
                claims.push(PinClaim {
                    gpio,
                    role: PinRole::Control,
                    owner,
                });
            }
        };
        if s.ctrl_status.uart_live {
            claim(s.uart.tx_pin, "UART Control".into());
            claim(s.uart.rx_pin, "UART Control".into());
        }
        if s.ctrl_status.i2c_live {
            claim(s.i2c.sda_pin, "I2C Control".into());
            claim(s.i2c.scl_pin, "I2C Control".into());
        }
        for (slot, b) in data.bindings.iter().enumerate() {
            let live = u8::try_from(slot).is_ok_and(|n| data.status.is_slot_active(n));
            if live && !b.is_empty() {
                for gpio in b.pins() {
                    claim(gpio, slot_owner(slot));
                }
            }
        }
        Ok(Self {
            control_i2c_sda: s.ctrl_status.i2c_live.then_some(s.i2c.sda_pin),
            preset_names: s.preset_names.clone(),
            state,
            data,
            claims,
        })
    }

    fn context(&self) -> StaticContext<'_> {
        StaticContext {
            state: &self.state,
            claims: &self.claims,
            control_i2c_sda: self.control_i2c_sda,
            preset_names: &self.preset_names,
        }
    }
}

/// A draft from JSON, and its index checked against what the device has. A macro
/// step and the display settings have no index of their own.
fn parse_draft(d: &CsData, kind: &str, slot: u32, text: &str) -> Result<(usize, Draft)> {
    let v: Value =
        serde_json::from_str(text).map_err(|e| invalid(format!("malformed draft: {e}")))?;
    if !v.is_object() {
        return Err(invalid("a draft is a JSON object"));
    }
    let limit = match kind {
        "binding" => d.slot_count(),
        "ir" => d.ir.len(),
        "page" => usize::from(d.max_pages),
        "group" => d.groups.len(),
        "step" | "display" => usize::MAX,
        other => return Err(invalid(format!("unknown record kind {other}"))),
    };
    let index = usize::try_from(slot).unwrap_or(usize::MAX);
    let index = if limit == usize::MAX { 0 } else { index };
    if index >= limit {
        return Err(invalid(format!(
            "{kind} {slot} does not exist on this device"
        )));
    }
    let draft = match kind {
        "binding" => Draft::Binding(binding_from_json(&v)?),
        "ir" => Draft::Ir(ir_from_json(&v)?),
        "step" => Draft::Step(step_from_json(&v)?),
        "page" => Draft::Page(page_from_json(&v)?),
        "display" => Draft::Display(display_cfg_from_json(&v)?),
        _ => Draft::Group(group_from_json(&v)?),
    };
    Ok((index, draft))
}

fn page_json(p: &CsDisplayPage) -> Value {
    json!({ "noun": p.noun, "target": p.target, "index": p.index, "flags": p.flags })
}

fn display_json(c: &CsDisplayCfg) -> Value {
    json!({ "mode": c.mode, "homePage": c.home_page, "dwell": c.dwell,
            "overlayHold": c.overlay_hold, "brightness": c.brightness,
            "flags": c.flags, "editTimeout": c.edit_timeout })
}

fn draft_json(d: &Draft) -> Value {
    match d {
        Draft::Binding(b) => binding_json(b),
        Draft::Ir(c) => ir_json(c),
        Draft::Step(s) => step_json(s),
        Draft::Page(p) => page_json(p),
        Draft::Display(c) => display_json(c),
        Draft::Group(g) => json!({ "targetKind": g.target_kind, "memberMask": g.member_mask,
                                   "name": g.name }),
    }
}

fn reply(r: Result<Value>) -> String {
    match r {
        Ok(v) => v,
        Err(e) => failure(&e),
    }
    .to_string()
}

fn drafted(d: &Draft) -> Value {
    json!({ "ok": true, "draft": draft_json(d) })
}

impl DspiCore {
    /// The rules' view of the open device. The lock is released on return.
    fn rules_device(&self) -> Result<Device> {
        self.ensure_probed(); // -- WP LK4 --
        Device::of(&mut self.lock())
    }
}

#[cfg_attr(feature = "uniffi", uniffi::export)]
impl DspiCore {
    /// What a page shows for one record, from the last [`DspiCore::cs_fetch`]:
    /// `{ok, title, summary, rows}`. `kind` is `binding`, `ir`, `step`, `page`,
    /// `display` or `group`; `slot` is the record's index; `draft` is the record as
    /// in [`DspiCore::cs_snapshot`].
    ///
    /// Each row has `id`, `title`, `detail` and `kind`. A `pick` has `value`, `text`
    /// and `options` of `{value, text, section?, owner?, disabled?}`; a `number`
    /// has `value`, `min`, `max`, `step`, `unit` and `decimals`, in natural units; a
    /// `toggle` has `on` and `enabled`; a `note` has `warning`, and its words in
    /// `detail`. `dspi_session::cs_rows` lists the ids.
    ///
    /// No device I/O. `{ok: false, error}` on a bad draft, an index the device does
    /// not have, no device, or before the first fetch. Never fails otherwise.
    pub fn cs_describe(&self, kind: String, slot: u32, draft: String) -> String {
        reply((|| {
            let dev = self.rules_device()?;
            let (index, d) = parse_draft(&dev.data, &kind, slot, &draft)?;
            let described = cs_rows::describe(&dev.context(), &dev.data, index, &d);
            let mut v = serde_json::to_value(&described).map_err(|e| invalid(e.to_string()))?;
            if let Some(o) = v.as_object_mut() {
                o.insert("ok".into(), json!(true));
            }
            Ok(v)
        })())
    }

    /// The record after one row's value changed: `{ok, draft}`. `change` is
    /// `{id, value}`: the row's id, and an option's value, a number in natural units,
    /// or a boolean. The edit resets what the change leaves meaningless, so the
    /// record stays one the firmware takes.
    ///
    /// No device I/O: nothing changes on the device until the record goes to
    /// [`DspiCore::cs_apply`]. `{ok: false, error}` when the row or the value is not
    /// one [`DspiCore::cs_describe`] offers for this record, or as `cs_describe`.
    pub fn cs_edit(&self, kind: String, slot: u32, draft: String, change: String) -> String {
        reply((|| {
            let dev = self.rules_device()?;
            let (index, d) = parse_draft(&dev.data, &kind, slot, &draft)?;
            let c: Value = serde_json::from_str(&change)
                .map_err(|e| invalid(format!("malformed change: {e}")))?;
            let id = c
                .get("id")
                .and_then(Value::as_str)
                .ok_or_else(|| invalid("a change names its row in id"))?;
            let value = match c.get("value") {
                Some(Value::Bool(b)) => f64::from(u8::from(*b)),
                Some(Value::Number(n)) => n.as_f64().unwrap_or(f64::NAN),
                _ => return Err(invalid("a change has a number or a boolean in value")),
            };
            let next = cs_rows::edit(&dev.context(), &dev.data, index, &d, id, value)
                .map_err(|e| invalid(e.to_string()))?;
            Ok(drafted(&next))
        })())
    }

    /// A fresh binding of type `component` in `slot`, on free pins: `{ok, draft}`.
    /// No device I/O. `{ok: false, error}` as [`DspiCore::cs_describe`].
    pub fn cs_new_binding(&self, slot: u32, component: u8) -> String {
        reply((|| {
            let dev = self.rules_device()?;
            let index = usize::try_from(slot).unwrap_or(usize::MAX);
            if index >= dev.data.slot_count() {
                return Err(invalid(format!(
                    "binding {slot} does not exist on this device"
                )));
            }
            let b = make_binding(
                &dev.context(),
                &dev.data,
                &dev.data.bindings,
                index,
                component,
            );
            Ok(drafted(&Draft::Binding(b)))
        })())
    }

    /// A fresh record of `kind`, `ir`, `step` or `page`: `{ok, draft}`. A step and a
    /// page are ready to apply; a remote button needs its code from a learn first.
    /// No device I/O. `{ok: false, error}` as [`DspiCore::cs_describe`].
    pub fn cs_new_record(&self, kind: String) -> String {
        reply((|| {
            let dev = self.rules_device()?;
            let d = match kind.as_str() {
                "ir" => Draft::Ir(cs_rows::new_ir(&dev.data)),
                "step" => Draft::Step(cs_rows::new_step(&dev.data)),
                "page" => Draft::Page(cs_rows::new_page(&dev.data)),
                other => return Err(invalid(format!("no new record of kind {other}"))),
            };
            Ok(drafted(&d))
        })())
    }

    /// A control-surface or pin-configuration status code as a sentence; empty for
    /// success. Needs no device.
    pub fn cs_status_text(&self, code: u8) -> String {
        if code == cs_status::SUCCESS {
            return String::new();
        }
        dspi_session::cs_model::capitalise(&explain_status(code))
    }

    /// On the demo device only: the remote button a learn is waiting for was
    /// pressed. The learn ends with this protocol and code and notifies, as on a
    /// real receiver. False when nothing was armed. As
    /// [`cs_demo_busy`](Self::cs_demo_busy), it answers [`DspiError::Unsupported`]
    /// on any other device.
    pub fn cs_demo_learn(&self, protocol: u8, code: u32) -> Result<bool> {
        let mut inner = self.lock();
        let mut w = inner.writable_conn()?;
        if w.conn().caps.kind != dspi_transport::TransportKind::Fake {
            return Err(DspiError::Unsupported);
        }
        let mut data = vec![protocol];
        data.extend_from_slice(&code.to_le_bytes());
        match w.sending().control_out(
            dspi_proto::generated::opcodes::REQ_EXTENSION,
            dspi_transport::fake::EXT_FAKE_CS_LEARN,
            &data,
        ) {
            Ok(()) => Ok(true),
            Err(dspi_transport::TransportError::Stalled { .. }) => Ok(false),
            Err(e) => Err(e.into()),
        }
    }
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_cg20 {
    use std::sync::Arc;

    use proptest::prelude::*;
    use serde_json::{Value, json};

    use super::*;
    use dspi_session::cs_model::{act, flag, ty};
    use dspi_session::surfaces::status as st;

    fn demo() -> Arc<DspiCore> {
        let core = DspiCore::new();
        core.open_demo().unwrap();
        core.cs_fetch().unwrap();
        core
    }

    fn parse(s: String) -> Value {
        serde_json::from_str(&s).unwrap()
    }

    fn describe(core: &DspiCore, kind: &str, slot: u32, draft: &Value) -> Value {
        parse(core.cs_describe(kind.into(), slot, draft.to_string()))
    }

    fn edit(core: &DspiCore, kind: &str, slot: u32, draft: &Value, id: &str, v: f64) -> Value {
        let change = json!({ "id": id, "value": v }).to_string();
        parse(core.cs_edit(kind.into(), slot, draft.to_string(), change))
    }

    fn row<'a>(d: &'a Value, id: &str) -> Option<&'a Value> {
        d["rows"].as_array().unwrap().iter().find(|r| r["id"] == id)
    }

    fn offered(r: &Value) -> Vec<i64> {
        r["options"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|o| o["disabled"] != json!(true))
            .map(|o| o["value"].as_i64().unwrap())
            .collect()
    }

    fn apply(core: &DspiCore, op: Value) -> Value {
        parse(core.cs_apply(op.to_string()))
    }

    fn snapshot(core: &DspiCore) -> Value {
        parse(core.cs_snapshot())
    }

    fn free_slot(core: &DspiCore) -> u32 {
        let s = snapshot(core);
        s["bindings"]
            .as_array()
            .unwrap()
            .iter()
            .position(|b| b["type"] == 0)
            .unwrap() as u32
    }

    /// The named bug: Qt offered Hold on a long press, and the firmware says
    /// `INVALID_EVENT` (control_surfaces.c:1736-1741). The refusal is real, and the
    /// rules never get there.
    #[test]
    fn cg20_hold_on_a_long_press_is_refused_by_the_device_and_never_offered() {
        let core = demo();
        let slot = free_slot(&core);
        let b = parse(core.cs_new_binding(slot, ty::BUTTON));
        let mut draft = b["draft"].clone();
        // A noun that takes Hold.
        let d = describe(&core, "binding", slot, &draft);
        let hold = i64::from(act::MOMENTARY);
        let noun = offered(row(&d, "noun").unwrap())
            .into_iter()
            .find(|n| {
                let e = edit(&core, "binding", slot, &draft, "noun", *n as f64);
                offered(
                    row(&describe(&core, "binding", slot, &e["draft"]), "action")
                        .unwrap_or(&json!({"options":[]})),
                )
                .contains(&hold)
            })
            .expect("the demo has a noun that takes Hold");
        draft = edit(&core, "binding", slot, &draft, "noun", noun as f64)["draft"].clone();
        draft = edit(&core, "binding", slot, &draft, "action", hold as f64)["draft"].clone();

        // Hold chosen: the gesture row offers the short press alone.
        let d = describe(&core, "binding", slot, &draft);
        assert_eq!(offered(row(&d, "event").unwrap()), vec![0]);
        assert!(edit(&core, "binding", slot, &draft, "event", 1.0)["error"].is_string());

        // What Qt could build: refused by the device.
        let mut refused = draft.clone();
        refused["event"] = json!(1);
        let r = apply(
            &core,
            json!({ "op": "binding", "index": slot, "binding": refused }),
        );
        assert_eq!(r["status"], st::INVALID_EVENT, "{r}");

        // A long press: no Hold on offer, and an edit cannot pick it.
        let toggle = i64::from(act::TOGGLE);
        draft = edit(&core, "binding", slot, &draft, "action", toggle as f64)["draft"].clone();
        draft = edit(&core, "binding", slot, &draft, "event", 1.0)["draft"].clone();
        let d = describe(&core, "binding", slot, &draft);
        assert!(!offered(row(&d, "action").unwrap()).contains(&hold));
        assert!(edit(&core, "binding", slot, &draft, "action", hold as f64)["error"].is_string());
        let r = apply(
            &core,
            json!({ "op": "binding", "index": slot, "binding": draft }),
        );
        assert_eq!(r["ok"], true, "{r}");
    }

    /// Describing and editing send nothing, as a client calls them on every redraw:
    /// with the device gone they still answer from what was read.
    #[test]
    fn cg20_the_rules_do_no_device_io() {
        let dev = dspi_transport::FakeDevice::demo();
        let remote = dev.remote();
        let core = DspiCore::new();
        core.attach_probed(Box::new(dev), "DEMO".into(), 0).unwrap();
        core.cs_fetch().unwrap();
        let d = parse(core.cs_new_binding(5, ty::BUTTON))["draft"].clone();
        remote.disconnect();
        assert_eq!(describe(&core, "binding", 5, &d)["ok"], true);
        let n = edit(&core, "binding", 5, &d, "event", 1.0);
        assert_eq!(n["draft"]["event"], 1, "{n}");
        assert_eq!(parse(core.cs_new_record("step".into()))["ok"], true);
        assert_eq!(parse(core.cs_new_binding(5, ty::LED))["ok"], true);
    }

    /// Firmware read as read-only still describes its records, for the lists.
    #[test]
    fn cg20_an_older_wire_format_describes_or_says_why() {
        let mut seen = 0;
        for wire in 17..=31 {
            let core = DspiCore::new();
            if core.open_demo_wire(wire).is_err() || core.cs_fetch().is_err() {
                continue;
            }
            let s = snapshot(&core);
            if s["supported"] != true {
                continue;
            }
            seen += 1;
            let b = s["bindings"][0].clone();
            let d = describe(&core, "binding", 0, &b);
            assert_eq!(d["ok"], true, "wire {wire}: {d}");
        }
        eprintln!("cg20: {seen} older formats with surfaces");
    }

    #[test]
    fn cg20_bad_input_is_a_json_reply() {
        let core = DspiCore::new();
        assert_eq!(describe(&core, "binding", 0, &json!({}))["ok"], false);
        assert_eq!(parse(core.cs_new_record("ir".into()))["ok"], false);

        let core = demo();
        assert_eq!(describe(&core, "nonsense", 0, &json!({}))["ok"], false);
        assert_eq!(describe(&core, "binding", 99, &json!({}))["ok"], false);
        assert_eq!(
            describe(&core, "binding", 0, &json!({ "noun": 999 }))["ok"],
            false
        );
        assert_eq!(describe(&core, "binding", 0, &json!([1]))["ok"], false);
        assert_eq!(parse(core.cs_new_binding(99, 1))["ok"], false);
        assert_eq!(parse(core.cs_new_record("binding".into()))["ok"], false);
        let bad = parse(core.cs_edit("binding".into(), 0, "{}".into(), "{}".into()));
        assert_eq!(bad["ok"], false);
    }

    #[test]
    fn cg20_status_text_is_the_sessions() {
        let core = DspiCore::new();
        assert_eq!(core.cs_status_text(0), "");
        assert_eq!(core.cs_status_text(0x1E), "Set up an IR receiver first");
    }

    /// `cs_demo_learn` finishes a learn through the API alone.
    #[test]
    fn cg20_a_learn_finishes_on_the_demo_device() {
        let core = demo();
        assert!(!core.cs_demo_learn(1, 0x1234).unwrap(), "nothing armed");
        assert_eq!(apply(&core, json!({ "op": "learnArm" }))["ok"], true);
        assert!(core.cs_demo_learn(1, 0x0102_0304).unwrap());
        let r = apply(&core, json!({ "op": "learnRead" }));
        assert_eq!(r["state"], 2, "{r}");
        assert_eq!(r["protocol"], 1);
        assert_eq!(r["code"], 0x0102_0304u32);
        // The notification says the same, as on a real receiver.
        // The reader thread delivers it; give it a moment.
        let mut seen = Value::Null;
        for _ in 0..200 {
            core.process_notifications();
            seen = snapshot(&core)["learn"]["state"].clone();
            if seen == 2 {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert_eq!(seen, 2);

        // A learned, fresh remote button applies.
        let mut cmd = parse(core.cs_new_record("ir".into()))["draft"].clone();
        cmd["protocol"] = r["protocol"].clone();
        cmd["code"] = r["code"].clone();
        let free = snapshot(&core)["ir"]
            .as_array()
            .unwrap()
            .iter()
            .position(|c| c["protocol"] == 0)
            .unwrap();
        let a = apply(&core, json!({ "op": "ir", "index": free, "command": cmd }));
        assert_eq!(a["ok"], true, "{a}");
    }

    #[test]
    fn cg20_demo_learn_needs_the_demo_device() {
        let core = DspiCore::new();
        assert!(core.cs_demo_learn(1, 1).is_err());
    }

    #[test]
    fn cg20_a_pin_clash_cannot_be_picked() {
        let core = demo();
        let s = snapshot(&core);
        // The first live input: its pin is held against an LED.
        let (held, _) = s["bindings"]
            .as_array()
            .unwrap()
            .iter()
            .enumerate()
            .find_map(|(i, b)| {
                (b["type"] == 1 || b["type"] == 4).then_some((b["gpio0"].as_i64().unwrap(), i))
            })
            .expect("the demo has a button or encoder");
        let slot = free_slot(&core);
        let led = parse(core.cs_new_binding(slot, ty::LED))["draft"].clone();
        let d = describe(&core, "binding", slot, &led);
        let pin = row(&d, "gpio0").unwrap();
        let entry = pin["options"]
            .as_array()
            .unwrap()
            .iter()
            .find(|o| o["value"] == held)
            .unwrap();
        assert_eq!(entry["disabled"], true, "{entry}");
        assert!(entry["owner"].is_string(), "{entry}");
        assert!(edit(&core, "binding", slot, &led, "gpio0", held as f64)["error"].is_string());
    }

    /// Whatever is offered applies: the PWM slice rule is the core's too now.
    fn acceptable(r: &Value) -> bool {
        r["ok"] == true
    }

    /// `proptest!` for the walks, with at most `max` cases. Every applied edit
    /// waits out the Console's 20 ms status poll, also on the demo device, so a
    /// case costs 0.2 s to 3 s and the gate's `PROPTEST_CASES=20000` would keep
    /// each walk busy for hours: the macro takes that count as it is.
    /// `PROPTEST_CASES` still sets the count up to `max`, which each walk sizes
    /// to about 45 s in release; a plain `cargo test` runs 12. Failures persist
    /// to `proptest-regressions/cs_rules.txt` as the macro's do.
    fn walk_cases<S: Strategy>(
        name: &'static str,
        max: u32,
        strategy: S,
        test: impl Fn(S::Value) -> std::result::Result<(), TestCaseError>,
    ) {
        let mut config =
            proptest::test_runner::contextualize_config(ProptestConfig::with_cases(12));
        config.cases = config.cases.min(max);
        config.source_file = Some(file!());
        config.test_name = Some(name);
        let mut runner = proptest::test_runner::TestRunner::new(config);
        if let Err(e) = runner.run(&strategy, test) {
            panic!("{e}\n{runner}");
        }
    }

    /// Choose a value for a row from `seed`. None for a row that takes no edit.
    fn pick_value(r: &Value, seed: u64) -> Option<f64> {
        match r["kind"].as_str()? {
            "pick" => {
                let o = offered(r);
                (!o.is_empty()).then(|| o[seed as usize % o.len()] as f64)
            }
            "toggle" => (r["enabled"] == true).then_some(((seed >> 3) & 1) as f64),
            "number" => {
                let (lo, hi) = (r["min"].as_f64()?, r["max"].as_f64()?);
                Some(lo + (hi - lo) * (((seed >> 5) % 101) as f64 / 100.0))
            }
            _ => None,
        }
    }

    /// Walk a record's rows, apply after every edit: whatever is offered, the
    /// firmware's model takes.
    fn walk(
        core: &DspiCore,
        kind: &str,
        slot: u32,
        mut draft: Value,
        seeds: &[u64],
        op: &dyn Fn(&Value) -> Value,
    ) {
        for seed in seeds {
            let d = describe(core, kind, slot, &draft);
            assert_eq!(d["ok"], true, "{d}");
            let rows: Vec<&Value> = d["rows"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|r| r["id"] != "type")
                .collect();
            let r = rows[*seed as usize % rows.len()];
            let Some(v) = pick_value(r, *seed) else {
                continue;
            };
            let next = edit(core, kind, slot, &draft, r["id"].as_str().unwrap(), v);
            assert_eq!(next["ok"], true, "{} {v}: {next}", r["id"]);
            draft = next["draft"].clone();
            // A group without members is a draft the page holds back: `noMembers`.
            if kind == "group" && draft["memberMask"] == 0 {
                continue;
            }
            let a = apply(core, op(&draft));
            assert!(
                acceptable(&a),
                "{kind} {draft} after {} = {v}: {a}",
                r["id"]
            );
        }
    }

    #[test]
    fn proptest_cg20_every_offered_binding_edit_applies() {
        let strategy = (
            prop::sample::select(vec![
                ty::BUTTON,
                ty::SWITCH,
                ty::POT,
                ty::ENCODER,
                ty::LED,
                ty::LED_PWM,
                ty::AUX_OUT,
                ty::AUX_PWM,
            ]),
            proptest::collection::vec(any::<u64>(), 1..40),
        );
        walk_cases(
            "proptest_cg20_every_offered_binding_edit_applies",
            100, // 0.44 s a case
            strategy,
            |(t, seeds)| {
                let core = demo();
                // The demo holds all three ADC pins: free one, so a new pot has a pin.
                let off = apply(
                    &core,
                    json!({ "op": "binding", "index": 2, "binding": { "type": 0 } }),
                );
                prop_assert_eq!(&off["ok"], true);
                let slot = free_slot(&core);
                let live = snapshot(&core);
                // The demo already holds its one receiver and display, so the
                // types drawn leave them out: a reject per draw would pass
                // proptest's 1024-reject limit long before 20000 cases.
                let has = |ty: u8| {
                    live["bindings"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|b| b["type"] == ty)
                };
                prop_assert!(has(ty::IR) && has(ty::DISPLAY));
                let b = parse(core.cs_new_binding(slot, t))["draft"].clone();
                let first = apply(
                    &core,
                    json!({ "op": "binding", "index": slot, "binding": b }),
                );
                prop_assert!(acceptable(&first), "{first}");
                walk(
                    &core,
                    "binding",
                    slot,
                    b,
                    &seeds,
                    &|d: &Value| json!({ "op": "binding", "index": slot, "binding": d }),
                );
                Ok(())
            },
        );
    }

    #[test]
    fn proptest_cg20_every_offered_step_edit_applies() {
        let strategy = proptest::collection::vec(any::<u64>(), 1..30);
        walk_cases(
            "proptest_cg20_every_offered_step_edit_applies",
            15, // 3 s a case: a macro is several deferred writes
            strategy,
            |seeds| {
                let core = demo();
                let st = parse(core.cs_new_record("step".into()))["draft"].clone();
                let group_free = snapshot(&core)["macros"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .position(|m| m["steps"].as_array().unwrap().is_empty())
                    .unwrap();
                walk(
                    &core,
                    "step",
                    0,
                    st,
                    &seeds,
                    &|d: &Value| json!({ "op": "macro", "index": group_free, "macro": { "name": "t", "steps": [d] } }),
                );
                Ok(())
            },
        );
    }

    #[test]
    fn proptest_cg20_every_offered_page_edit_applies() {
        let strategy = proptest::collection::vec(any::<u64>(), 1..30);
        walk_cases(
            "proptest_cg20_every_offered_page_edit_applies",
            220, // 0.2 s a case
            strategy,
            |seeds| {
                let core = demo();
                let p = parse(core.cs_new_record("page".into()))["draft"].clone();
                walk(
                    &core,
                    "page",
                    3,
                    p,
                    &seeds,
                    &|d: &Value| json!({ "op": "displayPage", "index": 3, "page": d }),
                );
                Ok(())
            },
        );
    }

    #[test]
    fn proptest_cg20_every_offered_display_edit_applies() {
        let strategy = proptest::collection::vec(any::<u64>(), 1..30);
        walk_cases(
            "proptest_cg20_every_offered_display_edit_applies",
            140, // 0.32 s a case
            strategy,
            |seeds| {
                let core = demo();
                let c = snapshot(&core)["display"]["config"].clone();
                walk(
                    &core,
                    "display",
                    0,
                    c,
                    &seeds,
                    &|d: &Value| json!({ "op": "displayConfig", "config": d }),
                );
                Ok(())
            },
        );
    }

    #[test]
    fn proptest_cg20_every_offered_remote_button_edit_applies() {
        let strategy = proptest::collection::vec(any::<u64>(), 1..30);
        walk_cases(
            "proptest_cg20_every_offered_remote_button_edit_applies",
            220, // 0.2 s a case
            strategy,
            |seeds| {
                let core = demo();
                let mut c = parse(core.cs_new_record("ir".into()))["draft"].clone();
                c["protocol"] = json!(1);
                c["code"] = json!(0x1234);
                walk(
                    &core,
                    "ir",
                    5,
                    c,
                    &seeds,
                    &|d: &Value| json!({ "op": "ir", "index": 5, "command": d }),
                );
                Ok(())
            },
        );
    }

    #[test]
    fn proptest_cg20_every_offered_group_edit_applies() {
        let strategy = proptest::collection::vec(any::<u64>(), 1..30);
        walk_cases(
            "proptest_cg20_every_offered_group_edit_applies",
            220, // 0.2 s a case
            strategy,
            |seeds| {
                let core = demo();
                let free = snapshot(&core)["groups"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .position(|g| g["memberMask"] == 0)
                    .unwrap();
                let g = json!({ "targetKind": 2, "memberMask": 0, "name": "t" });
                walk(
                    &core,
                    "group",
                    free as u32,
                    g,
                    &seeds,
                    &|d: &Value| json!({ "op": "group", "index": free, "group": d }),
                );
                Ok(())
            },
        );
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(12))]

        /// Any text, through every export: JSON back, never a panic.
        #[test]
        fn proptest_cg20_any_text_is_a_reply(kind in ".{0,12}", draft in ".{0,80}", change in ".{0,80}") {
            let core = demo();
            for s in [
                core.cs_describe(kind.clone(), 0, draft.clone()),
                core.cs_describe("binding".into(), 0, draft.clone()),
                core.cs_edit("binding".into(), 0, "{}".into(), change.clone()),
                core.cs_edit(kind.clone(), 0, draft.clone(), change.clone()),
                core.cs_new_record(kind.clone()),
            ] {
                prop_assert!(serde_json::from_str::<Value>(&s).is_ok());
            }
        }
    }

    /// A remote button never has more flags than the firmware takes.
    #[test]
    fn cg20_a_remote_button_drops_repeat_with_its_action() {
        let core = demo();
        let mut c = parse(core.cs_new_record("ir".into()))["draft"].clone();
        c["protocol"] = json!(1);
        c["code"] = json!(5);
        let d = describe(&core, "ir", 0, &c);
        let inc = offered(row(&d, "noun").unwrap())
            .into_iter()
            .find_map(|n| {
                let e = edit(&core, "ir", 0, &c, "noun", n as f64)["draft"].clone();
                let dd = describe(&core, "ir", 0, &e);
                let a = offered(row(&dd, "action")?);
                (a.contains(&i64::from(act::INC)) && a.contains(&i64::from(act::SET))).then_some(e)
            })
            .expect("a noun that steps and sets");
        let e = edit(&core, "ir", 0, &inc, "action", f64::from(act::INC))["draft"].clone();
        let e = edit(&core, "ir", 0, &e, "repeat", 1.0)["draft"].clone();
        assert_ne!(e["flags"].as_u64().unwrap() as u8 & flag::REPEAT, 0);
        let e = edit(&core, "ir", 0, &e, "action", f64::from(act::SET))["draft"].clone();
        assert_eq!(e["flags"].as_u64().unwrap() as u8 & flag::REPEAT, 0);
        assert_eq!(e["code"], 5, "the learned code stays");
    }
}
