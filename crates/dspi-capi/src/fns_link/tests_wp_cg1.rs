//! WP CG1 tests: the Link functions on the fake hub, `dspi_link_close` from a
//! listener thread, failure values on null and short buffers, the header, and
//! the choice of MIDI port (opening a real port needs a device).

#[cfg(feature = "link")]
mod link {
    use std::ffi::{CStr, CString, c_char, c_void};
    use std::path::PathBuf;
    use std::process::Command;
    use std::ptr::{null, null_mut};
    use std::sync::Mutex;
    use std::thread::{self, ThreadId};
    use std::time::{Duration, Instant};

    use dspi_link::fakehub::{Auth, FakeHub, HubConfig};
    use proptest::prelude::*;
    use serde_json::Value;

    use crate::FfiCore;
    use crate::fns_device::{dspi_core_free, dspi_core_new};
    use crate::fns_link::link::*;

    const WAIT: Duration = Duration::from_secs(8);
    const HUB_ID: &str = "6f1a2b3c-4d5e-4f60-8a9b-0c1d2e3f4a5b";
    const PIN: &str = "123456";
    const NULL: *mut FfiCore = null_mut();

    struct Handle(*mut FfiCore);

    impl Handle {
        fn new() -> Self {
            let h = dspi_core_new();
            assert!(!h.is_null());
            Self(h)
        }
    }

    impl Drop for Handle {
        fn drop(&mut self) {
            dspi_core_free(self.0);
        }
    }

    fn c(s: &str) -> CString {
        CString::new(s).unwrap()
    }

    /// Call a function that takes a buffer, with room for any reply.
    fn json(f: impl FnOnce(*mut c_char, u32) -> u32) -> Value {
        let mut buf = vec![0u8; 64 * 1024];
        let needed = f(buf.as_mut_ptr().cast(), buf.len() as u32);
        assert!(
            needed > 0 && needed as usize <= buf.len(),
            "needed {needed}"
        );
        serde_json::from_slice(&buf[..needed as usize - 1]).unwrap()
    }

    fn connect_with(h: &Handle, url: &str, token: &str, expected: &str) -> Value {
        let (url, name, ver) = (c(url), c("t"), c("1"));
        let (token, expected) = (c(token), c(expected));
        json(|b, l| {
            dspi_link_connect(
                h.0,
                url.as_ptr(),
                name.as_ptr(),
                ver.as_ptr(),
                token.as_ptr(),
                expected.as_ptr(),
                b,
                l,
            )
        })
    }

    fn connect(h: &Handle, hub: &FakeHub) -> Value {
        connect_with(h, &hub.url(), "", "")
    }

    fn status(h: *mut FfiCore) -> Value {
        json(|b, l| dspi_link_status(h, b, l))
    }

    fn wait_until(what: &str, mut f: impl FnMut() -> bool) {
        let start = Instant::now();
        while !f() {
            assert!(start.elapsed() < WAIT, "timed out waiting for {what}");
            thread::sleep(Duration::from_millis(5));
        }
    }

    fn wait_phase(h: *mut FfiCore, phase: &str) {
        wait_until(phase, || status(h)["phase"] == phase);
    }

    fn bind(h: &Handle) {
        let devices = json(|b, l| dspi_link_devices(h.0, b, l));
        let d = &devices["value"][0];
        let serial = c(d["serial"].as_str().unwrap());
        let handle = d["handle"].as_u64().unwrap() as u32;
        let r = json(|b, l| dspi_link_select_device(h.0, handle, serial.as_ptr(), b, l));
        assert_eq!(r, serde_json::json!({ "ok": true }));
        wait_phase(h.0, "bound");
    }

    fn pin_hub() -> FakeHub {
        FakeHub::start(HubConfig {
            auth: Auth::Fixed(PIN.into()),
            ..HubConfig::bridge()
        })
        .unwrap()
    }

    fn open_hub() -> FakeHub {
        FakeHub::start(HubConfig::default()).unwrap()
    }

    fn pair(h: *mut FfiCore, pin: &str) -> Value {
        let pin = c(pin);
        json(|b, l| dspi_link_pair(h, pin.as_ptr(), b, l))
    }

    // ── The functions on the fake hub ─────────────────────────────────

    #[test]
    fn wp_cg1_connect_select_status_close() {
        let hub = open_hub();
        let h = Handle::new();
        let hello = connect(&h, &hub);
        assert_eq!(hello["ok"], true, "{hello}");
        assert_eq!(hello["hubName"], "Fake Hub");
        assert_eq!(hello["hubId"], HUB_ID);
        assert_eq!(hello["needsPairing"], false);
        assert!(hello["caps"].as_array().unwrap().len() > 1);
        wait_phase(h.0, "choose_device");

        let devices = json(|b, l| dspi_link_devices(h.0, b, l));
        assert_eq!(devices["ok"], true);
        assert_eq!(devices["value"][0]["serial"], "0123456789ABCDEF");
        assert_eq!(devices["value"][0]["state"], "online");

        bind(&h);
        let st = status(h.0);
        assert_eq!(st["connected"], true);
        assert_eq!(st["ready"], true);
        assert_eq!(st["closeCode"], -1);
        assert!(st["session"].as_u64().unwrap() > 0);
        assert!(st["denied"].is_array());
        assert!(crate::fns_device::dspi_is_connected(h.0));

        dspi_link_close(h.0);
        assert_eq!(status(h.0)["phase"], "idle");
        assert!(!crate::fns_device::dspi_is_connected(h.0));
        wait_until("the hub to see the close", || hub.sessions().is_empty());
    }

    #[test]
    fn wp_cg1_another_hub_gets_wrong_hub_and_no_token() {
        let hub = open_hub();
        let h = Handle::new();
        let other = "aaaaaaaa-4d5e-4f60-8a9b-0c1d2e3f4a5b";
        let r = connect_with(&h, &hub.url(), "secret", other);
        assert_eq!(r["ok"], false);
        assert_eq!(r["code"], "wrong_hub", "{r}");
        assert!(!r["error"].as_str().unwrap().is_empty());
        assert!(hub.tokens_seen().is_empty());

        // The right id: the same call goes through.
        let r = connect_with(&h, &hub.url(), "secret", HUB_ID);
        assert_eq!(r["ok"], true, "{r}");
    }

    #[test]
    fn wp_cg1_pairing_codes_token_and_refused_token() {
        let hub = pin_hub();
        let h = Handle::new();
        let hello = connect_with(&h, &hub.url(), "stale", "");
        assert_eq!(hello["needsPairing"], true, "{hello}");
        assert_eq!(hello["tokenRefused"], true);
        assert_eq!(hello["pinSource"], "fixed");
        wait_phase(h.0, "needs_pairing");
        assert_eq!(status(h.0)["pinSource"], "fixed");

        let wrong = pair(h.0, "000000");
        assert_eq!(wrong["code"], "wrong_pin", "{wrong}");
        // Written for the user: no "DSPi Link:" prefix.
        assert!(!wrong["error"].as_str().unwrap().starts_with("DSPi Link"));

        let good = pair(h.0, PIN);
        assert_eq!(good["ok"], true, "{good}");
        assert!(!good["token"].as_str().unwrap().is_empty());
        wait_phase(h.0, "choose_device");
    }

    #[test]
    fn wp_cg1_too_many_wrong_pins_are_rate_limited() {
        let hub = FakeHub::start(HubConfig {
            auth: Auth::Fixed(PIN.into()),
            rate_limit_failures: 2,
            ..HubConfig::bridge()
        })
        .unwrap();
        let h = Handle::new();
        connect(&h, &hub);
        let codes: Vec<Value> = (0..3)
            .map(|_| pair(h.0, "000000")["code"].clone())
            .collect();
        assert_eq!(codes[0], "wrong_pin");
        assert_eq!(codes[2], "rate_limited", "{codes:?}");
    }

    #[test]
    fn wp_cg1_a_lost_session_reports_and_retries() {
        let hub = open_hub();
        let h = Handle::new();
        connect(&h, &hub);
        bind(&h);
        hub.drop_connections();
        wait_phase(h.0, "reconnecting");
        assert!(status(h.0)["reconnectInMs"].is_u64());
        wait_until("the session to come back", || {
            dspi_link_retry_now(h.0);
            status(h.0)["phase"] == "bound"
        });
    }

    #[test]
    fn wp_cg1_set_url_takes_a_valid_address_only() {
        let hub = open_hub();
        let h = Handle::new();
        connect(&h, &hub);
        let url = c(&hub.url());
        assert!(dspi_link_set_url(h.0, url.as_ptr()));
        assert!(!dspi_link_set_url(h.0, null()));
        assert!(!dspi_link_set_url(NULL, url.as_ptr()));
        let bad = [0xFFu8, 0];
        assert!(!dspi_link_set_url(h.0, bad.as_ptr().cast()));
    }

    #[test]
    fn wp_cg1_free_functions() {
        let hub = open_hub();
        let host = c(&hub.addr().ip().to_string());
        let info = json(|b, l| dspi_link_fetch_info(host.as_ptr(), hub.addr().port(), b, l));
        assert_eq!(info["ok"], true, "{info}");
        assert_eq!(info["hubId"], HUB_ID);
        assert_eq!(info["hubKind"], "console");
        assert_eq!(info["wsPath"], "/dspi/v1");
        assert_eq!(info["serials"][0], "0123456789ABCDEF");

        let text = c("[fe80::1]:8765");
        let a = json(|b, l| dspi_link_parse_address(text.as_ptr(), b, l));
        assert_eq!(a["host"], "fe80::1", "{a}");
        assert_eq!(a["port"], 8765);
        let text = c("not an address at all");
        let a = json(|b, l| dspi_link_parse_address(text.as_ptr(), b, l));
        assert_eq!(a["ok"], false);
        assert_eq!(a["code"], "invalid");

        let mut buf = [0u8; 128];
        let host = c("hub.local");
        for path in [null(), c"".as_ptr()] {
            let n = dspi_link_ws_url(host.as_ptr(), 8765, path, buf.as_mut_ptr().cast(), 128);
            assert_eq!(&buf[..n as usize - 1], b"ws://hub.local:8765/dspi/v1");
        }
        let n = dspi_link_ws_url(
            host.as_ptr(),
            80,
            c"/x".as_ptr(),
            buf.as_mut_ptr().cast(),
            128,
        );
        let got = std::str::from_utf8(&buf[..n as usize - 1]).unwrap();
        assert!(
            got.starts_with("ws://hub.local") && got.ends_with("/x"),
            "{got}"
        );
    }

    // ── The listener ──────────────────────────────────────────────────

    #[derive(Default)]
    struct Seen(Mutex<Vec<(u8, i32, String, String)>>);

    impl Seen {
        fn of(&self, event: u8) -> Vec<(u8, i32, String, String)> {
            let all = self.0.lock().unwrap();
            all.iter().filter(|e| e.0 == event).cloned().collect()
        }

        fn count(&self) -> usize {
            self.0.lock().unwrap().len()
        }
    }

    fn string(p: *const c_char) -> String {
        assert!(!p.is_null());
        // SAFETY: the callback's contract: NUL-terminated during the call.
        unsafe { CStr::from_ptr(p) }.to_str().unwrap().to_owned()
    }

    extern "C" fn record(
        event: u8,
        code: i32,
        text: *const c_char,
        token: *const c_char,
        user_data: *mut c_void,
    ) {
        // SAFETY: the tests pass a `Seen` that outlives the registration.
        let seen = unsafe { &*user_data.cast::<Seen>() };
        seen.0
            .lock()
            .unwrap()
            .push((event, code, string(text), string(token)));
    }

    fn user<T>(t: &T) -> *mut c_void {
        (t as *const T).cast_mut().cast()
    }

    #[test]
    fn wp_cg1_listener_events_reach_the_callback() {
        let hub = pin_hub();
        let h = Handle::new();
        let seen = Seen::default();
        assert!(dspi_link_set_listener(h.0, Some(record), user(&seen)));

        let hello = connect(&h, &hub);
        let good = pair(h.0, PIN);
        wait_until("the token event", || {
            !seen.of(LINK_EVENT_TOKEN_ISSUED).is_empty()
        });
        let t = &seen.of(LINK_EVENT_TOKEN_ISSUED)[0];
        assert_eq!(t.1, -1);
        assert_eq!(t.2, hello["hubId"].as_str().unwrap());
        assert_eq!(t.3, good["token"].as_str().unwrap());

        wait_until("a status event", || {
            !seen.of(LINK_EVENT_STATUS_CHANGED).is_empty()
        });
        let s = &seen.of(LINK_EVENT_STATUS_CHANGED)[0];
        assert_eq!((s.1, s.2.as_str(), s.3.as_str()), (-1, "", ""));

        // 4000: revoked. The end of the session, with the hub's code.
        hub.close_all(4000);
        wait_until("the closed event", || {
            !seen.of(LINK_EVENT_CLOSED).is_empty()
        });
        assert_eq!(seen.of(LINK_EVENT_CLOSED)[0].1, 4000);

        assert!(dspi_link_set_listener(h.0, None, null_mut()));
    }

    #[test]
    fn wp_cg1_device_changes_reach_the_callback() {
        let hub = open_hub();
        let h = Handle::new();
        let seen = Seen::default();
        connect(&h, &hub);
        assert!(dspi_link_set_listener(h.0, Some(record), user(&seen)));
        hub.set_board_present(false);
        wait_until("the devices event", || {
            !seen.of(LINK_EVENT_DEVICES_CHANGED).is_empty()
        });
        assert!(dspi_link_set_listener(h.0, None, null_mut()));
    }

    extern "C" fn set_inside(
        _: u8,
        _: i32,
        _: *const c_char,
        _: *const c_char,
        user_data: *mut c_void,
    ) {
        // SAFETY: the test passes a `Reentry` that outlives the registration.
        let r = unsafe { &*user_data.cast::<Reentry>() };
        let core = r.core as *mut FfiCore;
        r.answers
            .lock()
            .unwrap()
            .push(dspi_link_set_listener(core, None, null_mut()));
    }

    struct Reentry {
        core: usize,
        answers: Mutex<Vec<bool>>,
    }

    #[test]
    fn wp_cg1_setting_the_listener_from_its_callback_is_refused_not_a_deadlock() {
        let hub = open_hub();
        let h = Handle::new();
        let r = Reentry {
            core: h.0 as usize,
            answers: Mutex::new(Vec::new()),
        };
        assert!(dspi_link_set_listener(h.0, Some(set_inside), user(&r)));
        connect(&h, &hub);
        wait_until("a callback", || !r.answers.lock().unwrap().is_empty());
        assert!(!r.answers.lock().unwrap()[0]);
        // The flag ends with the callback: another thread, and this one, can.
        assert!(dspi_link_set_listener(h.0, None, null_mut()));
    }

    #[test]
    fn wp_cg1_core_free_clears_the_listener_and_ends_the_session() {
        let hub = open_hub();
        let h = dspi_core_new();
        let seen = Seen::default();
        assert!(dspi_link_set_listener(h, Some(record), user(&seen)));
        let (url, n, v, e) = (c(&hub.url()), c("t"), c("1"), c(""));
        let r = json(|b, l| {
            dspi_link_connect(
                h,
                url.as_ptr(),
                n.as_ptr(),
                v.as_ptr(),
                e.as_ptr(),
                e.as_ptr(),
                b,
                l,
            )
        });
        assert_eq!(r["ok"], true);
        dspi_core_free(h);
        let events = seen.count();
        wait_until("the hub to see the close", || hub.sessions().is_empty());
        thread::sleep(Duration::from_millis(100));
        assert_eq!(seen.count(), events, "a callback after dspi_core_free");
    }

    // ── dspi_link_close from a listener thread ────────────────────────

    /// What a callback did when it closed the session.
    struct Closed {
        thread: ThreadId,
        took: Duration,
        phase_after: Value,
    }

    struct Closer {
        core: usize,
        event: u8,
        /// Only in this phase.
        phase: Option<&'static str>,
        done: Mutex<Option<Closed>>,
    }

    extern "C" fn close_inside(
        event: u8,
        _: i32,
        _: *const c_char,
        _: *const c_char,
        user_data: *mut c_void,
    ) {
        // SAFETY: the test passes a `Closer` that outlives the registration.
        let c = unsafe { &*user_data.cast::<Closer>() };
        let core = c.core as *mut FfiCore;
        if event != c.event || c.done.lock().unwrap().is_some() {
            return;
        }
        if let Some(phase) = c.phase
            && status(core)["phase"] != phase
        {
            return;
        }
        let start = Instant::now();
        dspi_link_close(core);
        let took = start.elapsed();
        *c.done.lock().unwrap() = Some(Closed {
            thread: thread::current().id(),
            took,
            phase_after: status(core)["phase"].clone(),
        });
    }

    fn closer(h: &Handle, event: u8, phase: Option<&'static str>) -> Closer {
        Closer {
            core: h.0 as usize,
            event,
            phase,
            done: Mutex::new(None),
        }
    }

    /// The close ran on another thread, quickly, and left the session closed.
    fn assert_closed(c: &Closer, h: &Handle, hub: &FakeHub) {
        wait_until("the callback to close", || c.done.lock().unwrap().is_some());
        {
            let done = c.done.lock().unwrap();
            let done = done.as_ref().unwrap();
            assert_ne!(done.thread, thread::current().id());
            assert!(done.took < Duration::from_secs(1), "{:?}", done.took);
            assert_eq!(done.phase_after, "idle");
        }
        wait_until("the hub to see the close", || hub.sessions().is_empty());
        assert_eq!(status(h.0)["phase"], "idle");
        assert!(!crate::fns_device::dspi_is_connected(h.0));
        assert!(dspi_link_set_listener(h.0, None, null_mut()));
    }

    #[test]
    fn wp_cg1_link_close_works_from_a_listener_thread_while_reconnecting() {
        let hub = open_hub();
        let h = Handle::new();
        connect(&h, &hub);
        bind(&h);
        let c = closer(&h, LINK_EVENT_STATUS_CHANGED, Some("reconnecting"));
        assert!(dspi_link_set_listener(h.0, Some(close_inside), user(&c)));
        hub.drop_connections();
        assert_closed(&c, &h, &hub);
        // Past the first backoff: it stays closed.
        thread::sleep(Duration::from_millis(1300));
        assert!(hub.sessions().is_empty(), "a retry after the close");
        assert_eq!(status(h.0)["phase"], "idle");
    }

    #[test]
    fn wp_cg1_link_close_works_from_the_device_list_event() {
        let hub = open_hub();
        let h = Handle::new();
        connect(&h, &hub);
        bind(&h);
        let c = closer(&h, LINK_EVENT_DEVICES_CHANGED, None);
        assert!(dspi_link_set_listener(h.0, Some(close_inside), user(&c)));
        hub.set_board_present(false);
        assert_closed(&c, &h, &hub);
    }

    #[test]
    fn wp_cg1_link_close_works_from_the_token_event_during_a_pairing() {
        let hub = pin_hub();
        let h = Handle::new();
        connect(&h, &hub);
        let c = closer(&h, LINK_EVENT_TOKEN_ISSUED, None);
        assert!(dspi_link_set_listener(h.0, Some(close_inside), user(&c)));
        // Answered whichever way the race goes; the point is that it returns.
        let answer = pair(h.0, PIN);
        assert!(answer["ok"].is_boolean());
        assert_closed(&c, &h, &hub);
    }

    #[test]
    fn wp_cg1_link_close_with_no_session_is_a_no_op() {
        let h = Handle::new();
        dspi_link_close(h.0);
        dspi_link_close(h.0);
        assert_eq!(status(h.0)["phase"], "idle");
    }

    // ── Failure values ────────────────────────────────────────────────

    #[test]
    fn wp_cg1_a_null_handle_gives_the_failure_value() {
        let s = c("x");
        let mut b = [7u8; 8192];
        let (t, n) = (b.as_mut_ptr().cast::<c_char>(), 8192);
        let p = s.as_ptr();
        assert_eq!(dspi_link_connect(NULL, p, p, p, p, p, t, n), 0);
        assert_eq!(dspi_link_pair(NULL, p, t, n), 0);
        assert_eq!(dspi_link_devices(NULL, t, n), 0);
        assert_eq!(dspi_link_status(NULL, t, n), 0);
        assert_eq!(dspi_link_select_device(NULL, 0, p, t, n), 0);
        assert!(!dspi_link_set_url(NULL, p));
        assert!(!dspi_link_set_listener(NULL, None, null_mut()));
        dspi_link_close(NULL);
        dspi_link_retry_now(NULL);
        assert!(b.iter().all(|x| *x == 7));
    }

    #[test]
    fn wp_cg1_a_null_or_invalid_string_is_refused_before_any_dial() {
        let hub = open_hub();
        let h = Handle::new();
        let url = c(&hub.url());
        let e = c("");
        let bad = [0xC3u8, 0x28, 0];
        let bad = bad.as_ptr().cast::<c_char>();
        for arg in 0..5 {
            for wrong in [null(), bad] {
                let mut a = [url.as_ptr(), e.as_ptr(), e.as_ptr(), e.as_ptr(), e.as_ptr()];
                a[arg] = wrong;
                let r = json(|b, l| dspi_link_connect(h.0, a[0], a[1], a[2], a[3], a[4], b, l));
                assert_eq!(r["ok"], false, "argument {arg}");
                assert_eq!(r["code"], "invalid", "argument {arg}");
            }
        }
        assert!(hub.sessions().is_empty());
        assert_eq!(
            json(|b, l| dspi_link_pair(h.0, null(), b, l))["code"],
            "invalid"
        );
        assert_eq!(
            json(|b, l| dspi_link_pair(h.0, bad, b, l))["code"],
            "invalid"
        );
        for wrong in [null(), bad] {
            let r = json(|b, l| dspi_link_select_device(h.0, 0, wrong, b, l));
            assert_eq!(r["code"], "invalid");
            assert_eq!(
                json(|b, l| dspi_link_parse_address(wrong, b, l))["code"],
                "invalid"
            );
            assert_eq!(
                json(|b, l| dspi_link_fetch_info(wrong, 80, b, l))["code"],
                "invalid"
            );
            let mut buf = [0u8; 64];
            let out = buf.as_mut_ptr().cast();
            assert_eq!(dspi_link_ws_url(wrong, 80, null(), out, 64), 0);
        }
        let mut buf = [0u8; 64];
        let host = c("h");
        let out = buf.as_mut_ptr().cast();
        assert_eq!(dspi_link_ws_url(host.as_ptr(), 80, bad, out, 64), 0);
    }

    #[test]
    fn wp_cg1_no_session_replies_disconnected() {
        let h = Handle::new();
        let serial = c("x");
        let r = json(|b, l| dspi_link_select_device(h.0, 0, serial.as_ptr(), b, l));
        assert_eq!(r["code"], "disconnected");
        assert_eq!(pair(h.0, PIN)["code"], "disconnected");
        assert_eq!(
            json(|b, l| dspi_link_devices(h.0, b, l))["value"],
            serde_json::json!([])
        );
        let st = status(h.0);
        assert_eq!(st["phase"], "idle");
        assert_eq!(st["closeCode"], -1);
    }

    #[test]
    fn wp_cg1_a_handle_over_255_is_invalid() {
        let hub = open_hub();
        let h = Handle::new();
        connect(&h, &hub);
        let serial = c("x");
        let r = json(|b, l| dspi_link_select_device(h.0, 256, serial.as_ptr(), b, l));
        assert_eq!(r["code"], "invalid");
    }

    #[test]
    fn wp_cg1_a_short_buffer_acts_on_nothing() {
        let hub = pin_hub();
        let h = Handle::new();
        let (url, n, v, e) = (c(&hub.url()), c("t"), c("1"), c(""));
        let mut buf = vec![0x55u8; LINK_REPLY_MIN as usize];
        let out = buf.as_mut_ptr().cast::<c_char>();
        let dial = |b, l| {
            dspi_link_connect(
                h.0,
                url.as_ptr(),
                n.as_ptr(),
                v.as_ptr(),
                e.as_ptr(),
                e.as_ptr(),
                b,
                l,
            )
        };
        assert_eq!(dial(null_mut(), 0), LINK_REPLY_MIN);
        assert_eq!(dial(null_mut(), 100_000), LINK_REPLY_MIN);
        assert_eq!(dial(out, LINK_REPLY_MIN - 1), LINK_REPLY_MIN);
        assert!(hub.sessions().is_empty(), "dialled on a short buffer");
        assert_eq!(status(h.0)["phase"], "idle");

        // A second go with the same PIN would be refused, so the first short
        // one must not have spent it.
        assert_eq!(json(dial)["needsPairing"], true);
        let pin = c(PIN);
        assert_eq!(
            dspi_link_pair(h.0, pin.as_ptr(), null_mut(), 0),
            LINK_REPLY_MIN
        );
        assert_eq!(dspi_link_pair(h.0, pin.as_ptr(), out, 16), LINK_REPLY_MIN);
        assert_eq!(status(h.0)["phase"], "needs_pairing");

        let serial = c("x");
        let got = dspi_link_select_device(h.0, 0, serial.as_ptr(), out, 16);
        assert_eq!(got, LINK_REPLY_MIN);
        assert!(buf.iter().all(|x| *x == 0x55), "wrote to a short buffer");

        assert_eq!(pair(h.0, PIN)["ok"], true);
    }

    #[test]
    fn wp_cg1_a_reply_longer_than_the_buffer_writes_nothing_and_has_happened() {
        let hub = FakeHub::start(HubConfig {
            name: "n".repeat(2 * LINK_REPLY_MIN as usize),
            ..HubConfig::default()
        })
        .unwrap();
        let h = Handle::new();
        let (url, n, v, e) = (c(&hub.url()), c("t"), c("1"), c(""));
        let len = LINK_REPLY_MIN;
        let mut buf = vec![0x55u8; len as usize];
        let needed = dspi_link_connect(
            h.0,
            url.as_ptr(),
            n.as_ptr(),
            v.as_ptr(),
            e.as_ptr(),
            e.as_ptr(),
            buf.as_mut_ptr().cast(),
            len,
        );
        assert!(needed > len, "{needed}");
        assert!(buf.iter().all(|x| *x == 0x55));
        assert_eq!(hub.sessions().len(), 1, "the session is up");
    }

    proptest! {
        /// A read with any buffer length stays inside it, writes all of the
        /// reply or nothing, and says how much it needs.
        #[test]
        fn proptest_wp_cg1_replies_stay_in_the_buffer(len in 0u32..512, text in "[^\\x00]{0,48}") {
            let h = Handle::new();
            let text = c(&text);
            let calls: [&dyn Fn(*mut c_char, u32) -> u32; 4] = [
                &|b, l| dspi_link_status(h.0, b, l),
                &|b, l| dspi_link_devices(h.0, b, l),
                &|b, l| dspi_link_parse_address(text.as_ptr(), b, l),
                &|b, l| dspi_link_ws_url(text.as_ptr(), 80, text.as_ptr(), b, l),
            ];
            for call in calls {
                let mut buf = vec![0x55u8; len as usize + 1];
                let needed = call(buf.as_mut_ptr().cast(), len);
                prop_assert!(needed > 0);
                prop_assert_eq!(buf[len as usize], 0x55, "the guard byte");
                if needed <= len {
                    prop_assert_eq!(buf[needed as usize - 1], 0);
                    prop_assert!(
                        std::str::from_utf8(&buf[..needed as usize - 1]).is_ok()
                    );
                } else {
                    prop_assert!(buf.iter().all(|x| *x == 0x55));
                }
                prop_assert_eq!(call(null_mut(), len), needed);
                prop_assert_eq!(call(null_mut(), u32::MAX), needed);
            }
        }

        /// Whatever is typed as an address: an answer in the reply's form.
        #[test]
        fn proptest_wp_cg1_any_address_text_gets_a_reply(text in "\\PC{0,64}") {
            let text = c(&text.replace('\0', ""));
            let r = json(|b, l| dspi_link_parse_address(text.as_ptr(), b, l));
            prop_assert!(r["ok"].is_boolean());
        }
    }

    // ── The header ────────────────────────────────────────────────────

    fn header() -> String {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        std::fs::read_to_string(dir.join("include/dspi_core.h")).unwrap()
    }

    #[test]
    fn wp_cg1_the_functions_are_in_the_header() {
        let h = header();
        for proto in [
            "uint32_t dspi_link_connect(struct FfiCore *core,",
            "const char *expected_hub_id,",
            "uint32_t dspi_link_pair(struct FfiCore *core,",
            "uint32_t dspi_link_devices(struct FfiCore *core, char *buf, uint32_t len)",
            "uint32_t dspi_link_status(struct FfiCore *core, char *buf, uint32_t len)",
            "uint32_t dspi_link_select_device(struct FfiCore *core,",
            "void dspi_link_close(struct FfiCore *core)",
            "void dspi_link_retry_now(struct FfiCore *core)",
            "bool dspi_link_set_url(struct FfiCore *core, const char *url)",
            "bool dspi_link_set_listener(struct FfiCore *core,",
            "uint32_t dspi_link_parse_address(const char *text, char *buf, uint32_t len)",
            "uint32_t dspi_link_ws_url(const char *host,",
            "uint32_t dspi_link_fetch_info(const char *host,",
            "uint32_t dspi_midi_ports(char *buf, uint32_t len)",
            "bool dspi_open_midi(struct FfiCore *core, const char *name)",
            "#define LINK_REPLY_MIN 4096",
            "#define LINK_EVENT_TOKEN_ISSUED 3",
            "LinkEventCallback",
        ] {
            assert!(h.contains(proto), "{proto} is not in the header");
        }
    }

    /// A C compiler takes the header, and calling the functions type-checks.
    #[test]
    fn wp_cg1_the_header_compiles_as_c() {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("include");
        let out = PathBuf::from(env!("OUT_DIR")).join("wp_cg1_header.c");
        std::fs::write(
            &out,
            "#include <stdbool.h>\n#include <stdint.h>\n#include \"dspi_core.h\"\n\
             static void cb(uint8_t e, int32_t c, const char *t, const char *k, void *u)\n\
             { (void)e; (void)c; (void)t; (void)k; (void)u; }\n\
             bool f(struct FfiCore *h, char *b)\n{\n\
             dspi_link_close(h);\n\
             dspi_link_connect(h, \"u\", \"n\", \"v\", \"\", \"\", b, LINK_REPLY_MIN);\n\
             LinkEventCallback typed = cb;\n\
             dspi_link_set_listener(h, typed, 0);\n\
             return dspi_link_set_listener(h, cb, 0) && dspi_open_midi(h, 0);\n}\n",
        )
        .unwrap();
        let got = Command::new("cc")
            .args(["-std=c11", "-Wall", "-Werror", "-fsyntax-only", "-I"])
            .arg(&dir)
            .arg(&out)
            .output()
            .unwrap();
        assert!(
            got.status.success(),
            "{}",
            String::from_utf8_lossy(&got.stderr)
        );
        std::fs::remove_file(&out).unwrap();
    }
}

#[cfg(feature = "midir")]
mod midi {
    use std::ptr::{null, null_mut};

    use dspi_transport::midi::midir_port::PortInfo;

    use crate::fns_link::midi::*;

    fn port(name: &str) -> PortInfo {
        PortInfo {
            name: name.into(),
            input_id: format!("{name}-in"),
            output_id: format!("{name}-out"),
        }
    }

    #[test]
    fn wp_cg1_the_only_port_or_the_named_one() {
        let one = [port("a")];
        let two = [port("a"), port("b")];
        assert_eq!(pick(&one, None), Some(&one[0]));
        assert_eq!(pick(&two, None), None, "no name and a choice");
        assert_eq!(pick(&two, Some("b")), Some(&two[1]));
        assert_eq!(pick(&two, Some("c")), None);
        assert_eq!(pick(&[], None), None);
    }

    #[test]
    fn wp_cg1_open_midi_failure_values() {
        let name = c"no such port";
        assert!(!dspi_open_midi(null_mut(), null()));
        let h = crate::fns_device::dspi_core_new();
        assert!(!dspi_open_midi(h, name.as_ptr()));
        let bad = [0xFFu8, 0];
        assert!(!dspi_open_midi(h, bad.as_ptr().cast()));
        crate::fns_device::dspi_core_free(h);
    }

    #[test]
    fn wp_cg1_midi_ports_reply() {
        let mut buf = vec![0u8; 16 * 1024];
        let n = dspi_midi_ports(buf.as_mut_ptr().cast(), buf.len() as u32);
        let r: serde_json::Value = serde_json::from_slice(&buf[..n as usize - 1]).unwrap();
        assert_eq!(r["ok"], true);
        assert!(r["value"].is_array());
        // A short buffer: nothing written, the length reported.
        let mut short = [0x55u8; 2];
        assert_eq!(dspi_midi_ports(short.as_mut_ptr().cast(), 1), n);
        assert_eq!(short, [0x55, 0x55]);
    }
}

// -- WP PGWIRE (review/pga) --
#[cfg(feature = "midir")]
mod pgwire {
    use dspi_core::DspiCore;
    use dspi_transport::paged::Refusal;
    use dspi_transport::{CmdId, FakeDevice, MockTransport, mock::Reply};

    use crate::fns_link::midi::open_probed;

    /// The migration guide's probe reply (section 4.2).
    const GUIDE_REPLY: [u8; 16] = [
        0x44, 0x53, 0x50, 0x69, 0x01, 0x00, 0x0C, 0x00, 0x40, 0x00, 0x00, 0x01, 0, 0, 0, 0,
    ];

    /// The C ABI's MIDI open probes the generation before the connect: a
    /// paged device is refused with its notice after the probe alone (no
    /// page-0 request reaches it), and a 1.1.6 device is opened after exactly
    /// one probe request.
    #[test]
    fn wp_pgwire_the_c_abi_midi_open_probes_the_generation_first() {
        let paged = MockTransport::new()
            .carrying_pages()
            .paged(CmdId(0x0110), Reply::Data(GUIDE_REPLY.to_vec()))
            .answering_everything(vec![0; 64]);
        let log = paged.log_handle();
        let core = DspiCore::new();
        let e = open_probed(&core, Box::new(paged), "port".into()).unwrap_err();
        assert_eq!(
            e.notice_key(),
            format!("transfer.{}", Refusal::PagedNotYet.code())
        );
        let log = log.lock().unwrap().clone();
        assert_eq!(log.len(), 1, "only the probe: {log:?}");
        assert_eq!(
            log[0].setup(),
            [0xC1, 0x10, 0x00, 0x00, 0x02, 0x01, 0x10, 0x00]
        );
        assert!(!core.is_connected());

        let legacy = FakeDevice::demo();
        let remote = legacy.remote();
        open_probed(&core, Box::new(legacy), "port".into()).unwrap();
        assert!(core.is_connected());
        assert_eq!(remote.paged_requests(), 1, "the probe and no other page");
    }
}
