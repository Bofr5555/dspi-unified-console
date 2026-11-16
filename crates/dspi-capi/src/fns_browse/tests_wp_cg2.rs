//! WP CG2: the C functions: failure values on null and short buffers, the header,
//! and a browse from a fake advertiser (installed under the C functions, since a C
//! caller cannot pass one).

use std::ffi::CString;
use std::ptr::{null, null_mut};
use std::sync::Mutex;

use dspi_core::DspiCore;

use super::*;
use crate::types::DspState;

fn handle() -> *mut FfiCore {
    let core = DspiCore::new();
    let state = DspState::from(&core.get_state());
    Box::into_raw(Box::new(FfiCore {
        core,
        state: Mutex::new(Box::new(state)),
    }))
}

fn free(h: *mut FfiCore) {
    drop(unsafe { Box::from_raw(h) });
}

/// One call into one generous buffer: a browse may change between two calls, so
/// asking for the length first and then reading would race the fake advertiser.
fn read(f: impl Fn(*mut c_char, u32) -> u32) -> serde_json::Value {
    let mut buf = vec![0u8; 256 * 1024];
    let needed = f(buf.as_mut_ptr().cast(), buf.len() as u32) as usize;
    assert!(needed > 0 && needed <= buf.len(), "needed {needed}");
    buf.truncate(needed);
    assert_eq!(buf.pop(), Some(0), "NUL-terminated");
    serde_json::from_slice(&buf).unwrap()
}

#[test]
fn wp_cg2_every_function_survives_a_null_handle() {
    assert!(!dspi_link_browse_start(
        null_mut(),
        None,
        null_mut(),
        null()
    ));
    dspi_link_browse_stop(null_mut());
    assert_eq!(dspi_link_browse_hubs(null_mut(), null_mut(), 0), 0);
    assert_eq!(
        dspi_link_browse_resolve(null_mut(), null(), null_mut(), 0),
        0
    );
    assert!(!dspi_link_browse_reresolve(null_mut(), null()));
    dspi_link_browse_verify(null_mut(), null());
}

#[test]
fn wp_cg2_the_functions_are_in_the_header() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let header = std::fs::read_to_string(dir.join("include/dspi_core.h")).unwrap();
    for f in [
        "bool dspi_link_browse_start(struct FfiCore *core,\n",
        "void dspi_link_browse_stop(struct FfiCore *core);",
        "uint32_t dspi_link_browse_hubs(struct FfiCore *core, char *buf, uint32_t len);",
        "uint32_t dspi_link_browse_resolve(struct FfiCore *core,",
        "bool dspi_link_browse_reresolve(struct FfiCore *core, const char *hub_id);",
        "void dspi_link_browse_verify(struct FfiCore *core, const char *hub_id);",
    ] {
        assert!(header.contains(f), "{f} not in the header");
    }
}

#[test]
fn wp_cg2_a_short_buffer_writes_nothing_and_says_how_much() {
    let h = handle();
    let needed = dspi_link_browse_hubs(h, null_mut(), 0);
    assert!(needed > 1);
    let mut buf = vec![0x55u8; needed as usize - 1];
    let short = buf.len() as u32;
    assert_eq!(
        dspi_link_browse_hubs(h, buf.as_mut_ptr().cast(), short),
        needed
    );
    assert!(buf.iter().all(|b| *b == 0x55));
    assert_eq!(
        dspi_link_browse_hubs(h, null_mut(), 4096),
        needed,
        "a null buffer stays unwritten"
    );
    free(h);
}

#[cfg(not(feature = "mdns"))]
#[test]
fn wp_cg2_without_the_feature_the_functions_refuse_and_say_so() {
    let h = handle();
    assert!(!dspi_link_browse_start(h, None, null_mut(), null()));
    let id = CString::new("x").unwrap();
    assert_eq!(
        read(|b, l| dspi_link_browse_hubs(h, b, l))["code"],
        "unsupported"
    );
    assert_eq!(
        read(|b, l| dspi_link_browse_resolve(h, id.as_ptr(), b, l))["code"],
        "unsupported"
    );
    assert!(!dspi_link_browse_reresolve(h, id.as_ptr()));
    dspi_link_browse_verify(h, id.as_ptr());
    dspi_link_browse_stop(h);
    free(h);
}

#[cfg(feature = "mdns")]
mod with_feature {
    use std::net::IpAddr;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicPtr, AtomicU32, Ordering};
    use std::thread;
    use std::time::{Duration, Instant};

    use dspi_link::discovery::{FakeAdvertiser, HubAdvert};

    use super::*;

    const HID: &str = "3f2a7c1e-0000-4000-8000-000000000001";

    fn start_fake(
        h: *mut FfiCore,
        net: &FakeAdvertiser,
        callback: Option<extern "C" fn(*mut c_void)>,
        user_data: *mut c_void,
    ) {
        let listener = on::listener(callback, user_data);
        let core = unsafe { &(*h).core };
        core.link_browse_start_with(Arc::new(net.clone()), listener, "own".into())
            .unwrap();
    }

    fn announce(net: &FakeAdvertiser, name: &str, id: &str) {
        let advert = HubAdvert {
            name: name.into(),
            port: 11915,
            hub_id: id.into(),
            kind: "bridge".into(),
            auth: "none".into(),
            device_count: 1,
            serials: vec!["S1".into()],
            web: false,
            path: None,
            tls: false, // -- WP SECDISC --
        };
        let ip: IpAddr = "10.0.0.1".parse().unwrap();
        net.announce(&advert, &[ip]);
    }

    fn until(what: &str, f: impl Fn() -> bool) {
        let end = Instant::now() + Duration::from_secs(5);
        while !f() {
            assert!(Instant::now() < end, "timed out waiting: {what}");
            thread::sleep(Duration::from_millis(2));
        }
    }

    extern "C" fn wake(user_data: *mut c_void) {
        let counter = unsafe { &*user_data.cast::<AtomicU32>() };
        counter.fetch_add(1, Ordering::SeqCst);
    }

    #[test]
    fn wp_cg2_a_browse_through_the_c_functions() {
        let h = handle();
        let quiet = read(|b, l| dspi_link_browse_hubs(h, b, l));
        assert_eq!(quiet["ok"], true);
        assert_eq!(quiet["browsing"], false);
        assert_eq!(quiet["hubs"].as_array().unwrap().len(), 0);

        let net = FakeAdvertiser::new();
        let calls = Box::leak(Box::new(AtomicU32::new(0)));
        let user_data = std::ptr::from_ref(&*calls).cast_mut().cast::<c_void>();
        start_fake(h, &net, Some(wake), user_data);
        announce(&net, "Mine", "own");
        announce(&net, "Hub", HID);
        until("listed", || {
            read(|b, l| dspi_link_browse_hubs(h, b, l))["hubs"]
                .as_array()
                .unwrap()
                .len()
                == 1
        });
        until("woken", || calls.load(Ordering::SeqCst) > 0);

        let all = read(|b, l| dspi_link_browse_hubs(h, b, l));
        assert_eq!(all["browsing"], true);
        assert!(all["error"].is_null());
        let hub = &all["hubs"][0];
        assert_eq!(hub["id"], HID);
        assert_eq!(hub["name"], "Hub");
        assert_eq!(hub["kind"], "bridge");
        assert_eq!(hub["auth"], "none");
        assert_eq!(hub["deviceCount"], 1);
        assert_eq!(hub["serials"][0], "S1");
        assert_eq!(hub["host"], "10.0.0.1");
        assert_eq!(hub["port"], 11915);
        assert_eq!(hub["url"], "ws://10.0.0.1:11915/dspi/v1");

        let id = CString::new(HID.to_uppercase()).unwrap();
        let one = read(|b, l| dspi_link_browse_resolve(h, id.as_ptr(), b, l));
        assert_eq!(one["ok"], true);
        assert_eq!(one["hub"]["url"], "ws://10.0.0.1:11915/dspi/v1");
        let other = CString::new("nobody").unwrap();
        let none = read(|b, l| dspi_link_browse_resolve(h, other.as_ptr(), b, l));
        assert_eq!(none["ok"], true);
        assert!(none["hub"].is_null());
        let null_id = read(|b, l| dspi_link_browse_resolve(h, null(), b, l));
        assert!(null_id["hub"].is_null());

        assert!(
            dspi_link_browse_reresolve(h, id.as_ptr()),
            "no session, but the hub is found"
        );
        assert!(!dspi_link_browse_reresolve(h, other.as_ptr()));
        assert!(!dspi_link_browse_reresolve(h, null()));

        dspi_link_browse_verify(h, id.as_ptr());
        dspi_link_browse_verify(h, null());
        until("asked", || net.verified().len() == 1);

        dspi_link_browse_stop(h);
        let after = calls.load(Ordering::SeqCst);
        announce(&net, "Late", "late");
        thread::sleep(Duration::from_millis(20));
        assert_eq!(
            calls.load(Ordering::SeqCst),
            after,
            "no callback after stop"
        );
        assert_eq!(net.open_now(), 0);
        free(h);
    }

    /// The handle the callback below stops, since a C callback has only `user_data`.
    static STOPPING: AtomicPtr<FfiCore> = AtomicPtr::new(null_mut());
    static STOPPED: AtomicU32 = AtomicU32::new(0);

    extern "C" fn stop_from_callback(_: *mut c_void) {
        let h = STOPPING.load(Ordering::SeqCst);
        if !read(|b, l| dspi_link_browse_hubs(h, b, l))["hubs"]
            .as_array()
            .unwrap()
            .is_empty()
        {
            dspi_link_browse_stop(h);
            STOPPED.fetch_add(1, Ordering::SeqCst);
        }
    }

    #[test]
    fn wp_cg2_stop_works_from_the_callback() {
        let h = handle();
        STOPPING.store(h, Ordering::SeqCst);
        let net = FakeAdvertiser::new();
        start_fake(h, &net, Some(stop_from_callback), null_mut());
        announce(&net, "Hub", HID);
        until("stopped", || STOPPED.load(Ordering::SeqCst) == 1);
        until("closed", || net.open_now() == 0);
        free(h);
    }

    #[test]
    fn wp_cg2_freeing_the_handle_stops_the_browse() {
        let h = handle();
        let net = FakeAdvertiser::new();
        start_fake(h, &net, None, null_mut());
        until("open", || net.open_now() == 1);
        free(h);
        assert_eq!(net.open_now(), 0);
    }
}
