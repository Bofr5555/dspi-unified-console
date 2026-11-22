//! WP MD0: the set of cores through C.

#![allow(clippy::arithmetic_side_effects)]

use std::ffi::{CStr, CString};

use super::*;
use crate::fns_device::{dspi_get_state, dspi_is_connected, dspi_set_master_volume};

fn reply(f: impl FnOnce(*mut c_char, u32) -> u32) -> Value {
    let mut buf = vec![0u8; 65536];
    let needed = f(buf.as_mut_ptr().cast(), buf.len() as u32);
    assert!(needed as usize <= buf.len(), "{needed}");
    let end = buf.iter().position(|b| *b == 0).unwrap();
    serde_json::from_slice(&buf[..end]).unwrap()
}

fn open(set: *mut FfiCoreSet, target: &str) -> Value {
    let t = CString::new(target).unwrap();
    reply(|b, l| dspi_core_set_open(set, t.as_ptr(), b, l))
}

fn core_of(set: *mut FfiCoreSet, id: &Value) -> *mut FfiCore {
    let id = CString::new(id.as_str().unwrap()).unwrap();
    dspi_core_set_core(set, id.as_ptr())
}

fn close(set: *mut FfiCoreSet, id: &str) -> Value {
    let id = CString::new(id).unwrap();
    reply(|b, l| dspi_core_set_close(set, id.as_ptr(), b, l))
}

extern "C" fn heard(event: *const c_char, user: *mut c_void) {
    // SAFETY: `user` is the test's `Mutex<Vec<Value>>`, alive for the test.
    let seen = unsafe { &*user.cast::<Mutex<Vec<Value>>>() };
    // SAFETY: the set passes a NUL-terminated string that lives for the call.
    let text = unsafe { CStr::from_ptr(event) }.to_str().unwrap();
    seen.lock()
        .unwrap()
        .push(serde_json::from_str(text).unwrap());
}

#[test]
fn wp_md0_demos_through_c_have_their_own_cores() {
    let set = dspi_core_set_new();
    let seen = Box::new(Mutex::new(Vec::<Value>::new()));
    dspi_core_set_set_listener(set, Some(heard), ptr::from_ref(&*seen).cast_mut().cast());

    let a = open(set, r#"{"kind":"demo"}"#);
    let b = open(set, r#"{"kind":"demo"}"#);
    assert_eq!(a, json!({"ok": true, "id": "demo:1"}));
    assert_eq!(b["id"], "demo:2");
    let (ca, cb) = (core_of(set, &a["id"]), core_of(set, &b["id"]));
    assert!(!ca.is_null() && !cb.is_null() && ca != cb);
    assert_eq!(core_of(set, &a["id"]), ca, "the same handle each time");
    assert!(dspi_set_master_volume(ca, -31.0));
    // SAFETY: the handles are live; the pointers stay valid until the next dspi_get_state.
    let (va, vb) = unsafe {
        (
            (*dspi_get_state(ca)).master_volume_db,
            (*dspi_get_state(cb)).master_volume_db,
        )
    };
    assert_eq!(va, -31.0);
    assert_ne!(vb, -31.0);

    let list = reply(|b, l| dspi_core_set_list(set, b, l));
    assert_eq!(list["ok"], true);
    assert_eq!(
        list["value"][1],
        json!({"id": "demo:2", "label": "Demo 2", "transport": "demo", "serial": list["value"][1]["serial"], "state": "live"})
    );

    assert_eq!(close(set, "demo:1"), json!({"ok": true}));
    assert!(core_of(set, &a["id"]).is_null());
    assert!(dspi_is_connected(cb), "the other stays open");
    assert_eq!(close(set, "demo:1")["code"], "unknown");
    assert_eq!(close(set, "nonsense")["code"], "unknown");

    let kinds: Vec<String> = seen
        .lock()
        .unwrap()
        .iter()
        .map(|e| format!("{} {}", e["kind"], e["id"]))
        .collect();
    assert_eq!(
        kinds,
        [
            r#""opened" "demo:1""#,
            r#""opened" "demo:2""#,
            r#""closed" "demo:1""#
        ]
    );
    dspi_core_set_set_listener(set, None, ptr::null_mut());
    dspi_core_set_free(set);
}

#[test]
fn wp_md0_bad_targets_and_a_full_set_are_refused_through_c() {
    let set = dspi_core_set_new();
    for t in ["", "{", r#"{"kind":"tape"}"#, r#"{"kind":"usb"}"#, r#"{}"#] {
        assert_eq!(open(set, t)["code"], "invalid", "{t}");
    }
    let mut doc = open(set, r#"{"kind":"document","text":"no","fileName":"x"}"#);
    assert_eq!(doc["code"], "invalid");
    assert!(
        doc.get("noticeKey").is_some(),
        "a device failure is the device's reply"
    );
    for _ in 0..dspi_core::CORE_SET_CAP {
        assert_eq!(open(set, r#"{"kind":"demo"}"#)["ok"], true);
    }
    doc = open(set, r#"{"kind":"demo"}"#);
    assert_eq!(doc["code"], "full");
    assert_eq!(dspi_core_set_scan_devices(set, ptr::null_mut(), 0), 0);
    dspi_core_set_poll_hotplug(set);
    let none = CString::new("X").unwrap();
    assert_eq!(
        reply(|b, l| dspi_core_set_find_serial(set, none.as_ptr(), b, l)),
        json!({"ok": true, "value": null})
    );
    let cfg = CString::new(r#"{"snapshotDir":""}"#).unwrap();
    assert_eq!(
        reply(|b, l| dspi_core_set_config(set, cfg.as_ptr(), b, l)),
        json!({"ok": true})
    );
    let bad = CString::new("[").unwrap();
    assert_eq!(
        reply(|b, l| dspi_core_set_config(set, bad.as_ptr(), b, l))["code"],
        "invalid"
    );
    dspi_core_set_free(set);
}

#[test]
fn wp_md0_null_sets_fail_quietly() {
    let n = ptr::null_mut();
    assert_eq!(dspi_core_set_open(n, ptr::null(), ptr::null_mut(), 0), 0);
    assert_eq!(dspi_core_set_list(n, ptr::null_mut(), 0), 0);
    assert!(dspi_core_set_core(n, ptr::null()).is_null());
    dspi_core_set_poll_hotplug(n);
    dspi_core_set_set_listener(n, None, ptr::null_mut());
    dspi_core_set_free(n);
    let set = dspi_core_set_new();
    assert!(dspi_core_set_core(set, ptr::null()).is_null());
    assert_eq!(open(set, r#"{"kind":"demo"}"#)["ok"], true);
    dspi_core_set_free(set);
}

#[cfg(feature = "link")]
#[test]
fn wp_md0_a_hub_unit_through_c_holds_its_serial() {
    use dspi_link::fakehub::{FakeHub, HubConfig};
    let cfg = HubConfig::default();
    let hub = FakeHub::start(cfg.clone()).unwrap();
    let set = dspi_core_set_new();
    let target = json!({
        "kind": "hub", "url": hub.url(), "hubId": cfg.hub_id, "serial": cfg.serial,
        "clientName": "wp-md0", "appVersion": "test",
    })
    .to_string();
    let unit = open(set, &target);
    let id = format!("hub:{}/{}", cfg.hub_id, cfg.serial);
    assert_eq!(unit, json!({"ok": true, "id": id}));
    assert!(dspi_is_connected(core_of(set, &unit["id"])));
    let usb = open(
        set,
        &json!({"kind": "usb", "serial": cfg.serial}).to_string(),
    );
    assert_eq!(usb["code"], "already_open");
    assert_eq!(usb["id"], id.as_str());
    let serial = CString::new(cfg.serial.clone()).unwrap();
    assert_eq!(
        reply(|b, l| dspi_core_set_find_serial(set, serial.as_ptr(), b, l))["value"],
        id.as_str()
    );
    assert_eq!(hub.sessions().len(), 1);
    dspi_core_set_free(set);
}

/// Review RA F2: a closed device's handle is retired, not freed, so a call
/// another thread makes on it late answers "no device" until the set is freed.
#[test]
fn wp_md0_a_closed_devices_handle_stays_valid_until_the_set_is_freed() {
    let set = dspi_core_set_new();
    let a = open(set, r#"{"kind":"demo"}"#);
    let ca = core_of(set, &a["id"]);
    assert!(dspi_is_connected(ca));
    let worker = ca as usize;
    let late = std::thread::spawn(move || {
        let ca = worker as *mut FfiCore;
        // Racing the close: before it the call works, after it it is
        // refused; neither touches freed memory.
        for _ in 0..200 {
            let _ = dspi_set_master_volume(ca, -20.0);
            // SAFETY: the handle stays valid until dspi_core_set_free.
            let _ = unsafe { (*dspi_get_state(ca)).master_volume_db };
        }
    });
    assert_eq!(close(set, "demo:1"), json!({"ok": true}));
    late.join().unwrap();
    assert!(!dspi_is_connected(ca), "a retired handle has no device");
    assert!(!dspi_set_master_volume(ca, -10.0));
    assert!(!dspi_get_state(ca).is_null(), "still a live handle");
    let b = open(set, r#"{"kind":"demo"}"#);
    let cb = core_of(set, &b["id"]);
    assert!(!cb.is_null() && cb != ca, "a new open gets a new handle");
    assert!(dspi_is_connected(cb));
    dspi_core_set_free(set);
}
