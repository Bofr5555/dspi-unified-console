//! The wasm gate's scenario module (WP WB2): `dspi-core` built for
//! `wasm32-unknown-unknown` and run by `run.mjs` under Node with JSPI, the way
//! a page's Worker would run it (docs/core-merge/WEB-STUDY.md section 2).
//!
//! Exports take a session handle, so one module holds several device sessions
//! at once, each its own [`DspiCore`]: the web hub's many installations, and no
//! single-device state here. Every call is made by `run.mjs` one at a time, each
//! through `WebAssembly.promising`, as the binding's serial queue will (WB3).
//!
//! Results: 0 or a non-negative value for success, -1 for an error, whose text
//! [`last_error_ptr`] and [`last_error_len`] give.
#![cfg(all(target_arch = "wasm32", target_os = "unknown"))]
#![allow(unsafe_code)]

use std::cell::RefCell;
use std::sync::Arc;
use std::time::Duration;

use dspi_core::{DspiCore, RtaRequest};
use dspi_session::autoeq;
use dspi_transport::{DeviceDescriptor, FakeDevice, NotificationSource, Transport, TransportCaps};

/// One device session: the core, and for [`session_open_demo_endpoint`] the
/// demo's notification endpoint, which the page reads itself.
struct Session {
    core: Arc<DspiCore>,
    endpoint: Option<Box<dyn NotificationSource>>,
}

thread_local! {
    static REBUILD: RefCell<Option<autoeq::RebuildHandle>> = const { RefCell::new(None) };
    static SESSIONS: RefCell<Vec<Option<Session>>> = const { RefCell::new(Vec::new()) };
    static LAST_ERROR: RefCell<String> = const { RefCell::new(String::new()) };
}

fn session(h: u32) -> Option<Arc<DspiCore>> {
    SESSIONS.with_borrow(|s| s.get(h as usize)?.as_ref().map(|s| Arc::clone(&s.core)))
}

/// The demo device with its notification endpoint taken away, as a WebUSB
/// transport whose page runs the endpoint loop: the core gets the packets
/// only through `push_notification`.
struct EndpointElsewhere(FakeDevice);

impl Transport for EndpointElsewhere {
    fn control_in(&mut self, opcode: u8, value: u16, len: u16) -> dspi_transport::Result<Vec<u8>> {
        self.0.control_in(opcode, value, len)
    }
    fn control_in_upto(
        &mut self,
        opcode: u8,
        value: u16,
        max_len: u16,
    ) -> dspi_transport::Result<Vec<u8>> {
        self.0.control_in_upto(opcode, value, max_len)
    }
    fn control_out(&mut self, opcode: u8, value: u16, data: &[u8]) -> dspi_transport::Result<()> {
        self.0.control_out(opcode, value, data)
    }
    fn descriptor(&self) -> &DeviceDescriptor {
        self.0.descriptor()
    }
    fn max_transfer(&self) -> usize {
        self.0.max_transfer()
    }
    fn notifications(&self) -> Option<Box<dyn NotificationSource>> {
        None
    }
    fn caps(&self) -> TransportCaps {
        self.0.caps()
    }
}

fn fail(msg: impl std::fmt::Display) -> i32 {
    LAST_ERROR.with_borrow_mut(|e| *e = msg.to_string());
    -1
}

/// Run `f` on session `h`; -1 for an unknown handle or an error.
fn with(h: u32, f: impl FnOnce(&DspiCore) -> Result<i32, String>) -> i32 {
    let Some(core) = session(h) else {
        return fail(format!("no session {h}"));
    };
    f(&core).unwrap_or_else(fail)
}

#[unsafe(no_mangle)]
pub extern "C" fn last_error_ptr() -> *const u8 {
    LAST_ERROR.with_borrow(|e| e.as_ptr())
}

#[unsafe(no_mangle)]
pub extern "C" fn last_error_len() -> usize {
    LAST_ERROR.with_borrow(|e| e.len())
}

/// `len` bytes for the page to write into; free with [`buf_free`].
#[unsafe(no_mangle)]
pub extern "C" fn buf_alloc(len: usize) -> *mut u8 {
    let mut v = vec![0u8; len];
    let p = v.as_mut_ptr();
    std::mem::forget(v);
    p
}

/// # Safety
/// `ptr` and `len` from one [`buf_alloc`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn buf_free(ptr: *mut u8, len: usize) {
    // SAFETY: as the caller promises, a vector of exactly this length.
    drop(unsafe { Vec::from_raw_parts(ptr, len, len) });
}

/// A new session with no device; its handle.
#[unsafe(no_mangle)]
pub extern "C" fn session_new() -> u32 {
    SESSIONS.with_borrow_mut(|s| {
        s.push(Some(Session {
            core: DspiCore::new(),
            endpoint: None,
        }));
        u32::try_from(s.len() - 1).unwrap_or(u32::MAX)
    })
}

/// Close the device and forget the session.
#[unsafe(no_mangle)]
pub extern "C" fn session_free(h: u32) -> i32 {
    let core = SESSIONS.with_borrow_mut(|s| s.get_mut(h as usize).and_then(Option::take));
    match core {
        Some(s) => {
            s.core.disconnect();
            0
        }
        None => fail(format!("no session {h}")),
    }
}

/// Connect to the demo device: the whole connect sequence.
#[unsafe(no_mangle)]
pub extern "C" fn session_open_demo(h: u32) -> i32 {
    with(h, |c| c.open_demo().map(|()| 0).map_err(|e| e.to_string()))
}

/// Connect to the demo device without its notification endpoint, which the
/// page then reads with [`session_endpoint_pump`].
#[unsafe(no_mangle)]
pub extern "C" fn session_open_demo_endpoint(h: u32) -> i32 {
    let demo = FakeDevice::demo();
    let endpoint = demo.notifications();
    let serial = demo.descriptor().serial.clone();
    let opened = with(h, |c| {
        c.open_transport(Box::new(EndpointElsewhere(demo)), serial, 0)
            .map(|()| 0)
            .map_err(|e| e.to_string())
    });
    SESSIONS.with_borrow_mut(|s| {
        if let Some(Some(s)) = s.get_mut(h as usize) {
            s.endpoint = endpoint;
        }
    });
    opened
}

/// The page's endpoint loop, one turn: read what the demo sent and push each
/// packet into the core. The sequence number of the last one, or -2 if none.
#[unsafe(no_mangle)]
pub extern "C" fn session_endpoint_pump(h: u32) -> i32 {
    let Some(core) = session(h) else {
        return fail(format!("no session {h}"));
    };
    let mut packets = Vec::new();
    SESSIONS.with_borrow_mut(|s| {
        if let Some(Some(Session {
            endpoint: Some(e), ..
        })) = s.get_mut(h as usize)
        {
            while let Ok(p) = e.read(Duration::ZERO) {
                if p.is_empty() {
                    break;
                }
                packets.push(p);
            }
        }
    });
    let mut last = -2;
    for p in packets {
        // A v2 packet's sequence number; the v1 volume packet has none.
        if p.len() > 4 && p[0] == 2 {
            last = i32::from(p[3]);
        }
        if let Err(e) = core.push_notification(p, None) {
            return fail(e);
        }
    }
    last
}

/// 1 while a device is open.
#[unsafe(no_mangle)]
pub extern "C" fn session_connected(h: u32) -> i32 {
    with(h, |c| Ok(i32::from(c.is_connected())))
}

/// Set the user volume, then read the device's image again and return what
/// it holds, in hundredths of a dB: the write with read-back.
#[unsafe(no_mangle)]
pub extern "C" fn session_write_read_back(h: u32, db: f32) -> i32 {
    with(h, |c| {
        c.set_user_volume(db).map_err(|e| e.to_string())?;
        c.refresh_params().map_err(|e| e.to_string())?;
        Ok((c.get_state().user_volume_db * 100.0).round() as i32)
    })
}

/// The mirror's master volume, in hundredths of a dB.
#[unsafe(no_mangle)]
pub extern "C" fn session_master_volume(h: u32) -> i32 {
    with(h, |c| {
        Ok((c.get_state().master_volume_db * 100.0).round() as i32)
    })
}

/// Have another host rename the device (the demo's own simulation): it
/// arrives as a notification that is not ours.
#[unsafe(no_mangle)]
pub extern "C" fn session_rename_elsewhere(h: u32) -> i32 {
    with(h, |c| {
        c.demo_rename_elsewhere(0xFF, "Elsewhere".into())
            .map(|()| 0)
            .map_err(|e| e.to_string())
    })
}

/// Hand in a notification packet the page read itself.
///
/// # Safety
/// `ptr` is valid for `len` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn session_push_notification(h: u32, ptr: *const u8, len: usize) -> i32 {
    // SAFETY: as the caller promises.
    let packet = unsafe { std::slice::from_raw_parts(ptr, len) }.to_vec();
    with(h, |c| {
        c.push_notification(packet, None)
            .map(|()| 0)
            .map_err(|e| e.to_string())
    })
}

/// Process what is queued; the `NOTIFY_*` flags.
#[unsafe(no_mangle)]
pub extern "C" fn session_process_notifications(h: u32) -> i32 {
    with(h, |c| {
        Ok(i32::try_from(c.process_notifications().flags).unwrap_or(i32::MAX))
    })
}

/// Whether the mirror holds the name [`session_rename_elsewhere`] gave.
#[unsafe(no_mangle)]
pub extern "C" fn session_renamed(h: u32) -> i32 {
    with(h, |c| {
        let name = c.cached_device_name().map_err(|e| e.to_string())?;
        Ok(i32::from(name == "Elsewhere"))
    })
}

/// Start the analyser and ask for the output tap's first two channels.
#[unsafe(no_mangle)]
pub extern "C" fn session_rta_start(h: u32) -> i32 {
    with(h, |c| {
        c.rta_start();
        c.rta_set_request(RtaRequest {
            tap: 1,
            mask: 0b11,
            ..RtaRequest::default()
        });
        Ok(0)
    })
}

/// One turn of the analyser; the wait before the next, in ms, or -2 when
/// the worker has ended.
#[unsafe(no_mangle)]
pub extern "C" fn session_rta_tick(h: u32) -> i32 {
    with(h, |c| {
        Ok(c.rta_tick()
            .map_or(-2, |d| i32::try_from(d.as_millis()).unwrap_or(i32::MAX)))
    })
}

/// Channels in the latest picture, times 1000, plus the bands of the first.
#[unsafe(no_mangle)]
pub extern "C" fn session_rta_frames(h: u32) -> i32 {
    with(h, |c| {
        let s = c.rta_get_snapshot();
        let bands = s.frames.first().map_or(0, |f| i32::from(f.n_bands));
        Ok(i32::from(s.count) * 1000 + bands)
    })
}

/// Whether the analyser's caps were probed.
#[unsafe(no_mangle)]
pub extern "C" fn session_rta_supported(h: u32) -> i32 {
    with(h, |c| Ok(i32::from(c.rta_get_caps().supported)))
}

/// Where the master volume sits in the bulk image, for a pushed packet.
#[unsafe(no_mangle)]
pub extern "C" fn off_master_volume() -> u32 {
    u32::try_from(dspi_proto::generated::OFF_MASTER_VOLUME).unwrap_or(u32::MAX)
}

/// `dspi_transport::clock::sleep`: under JSPI the call suspends and the
/// page's event loop runs meanwhile.
#[unsafe(no_mangle)]
pub extern "C" fn clock_sleep(ms: u32) {
    dspi_transport::clock::sleep(Duration::from_millis(u64::from(ms)));
}

/// An offline GitHub for the AutoEQ rebuild: one source with one headphone,
/// every answer after a 2 ms suspended wait, as a page's `fetch` would.
struct OfflineGitHub;

impl autoeq::Fetch for OfflineGitHub {
    fn get(&self, request: &autoeq::FetchRequest) -> Result<autoeq::FetchResponse, String> {
        const API: &str = "https://api.github.com/repos/jaakkopasanen/AutoEq/contents/results/";
        dspi_transport::clock::sleep(Duration::from_millis(2));
        let body = match request.url.strip_prefix(API) {
            Some("oratory1990") => Some(r#"[{"name":"t","type":"dir"}]"#.to_string()),
            Some("oratory1990/t") => Some(r#"[{"name":"Demo One","type":"dir"}]"#.to_string()),
            Some(_) => None,
            None if request.url.ends_with("Demo%20One%20ParametricEQ.txt") => {
                Some("Preamp: -6.5 dB\nFilter 1: ON PK Fc 105 Hz Gain 3.0 dB Q 0.70\n".to_string())
            }
            None => None,
        };
        Ok(autoeq::FetchResponse {
            status: if body.is_some() { 200 } else { 404 },
            body: body.unwrap_or_default().into_bytes(),
            ..Default::default()
        })
    }
}

/// Start an AutoEQ rebuild into bytes (no file system), driven by
/// [`rebuild_poll`].
#[unsafe(no_mangle)]
pub extern "C" fn rebuild_start() {
    let handle = autoeq::rebuild_bytes(Box::new(OfflineGitHub));
    REBUILD.with_borrow_mut(|r| *r = Some(handle));
}

/// One poll, which on the web does one step: -3 while it runs, the profile
/// count once it succeeded with the database in hand, -1 for a failure.
#[unsafe(no_mangle)]
pub extern "C" fn rebuild_poll() -> i32 {
    REBUILD.with_borrow_mut(|r| {
        let Some(handle) = r.as_mut() else {
            return fail("no rebuild");
        };
        match handle.poll().done.clone() {
            None => -3,
            Some(Err(e)) => fail(e),
            Some(Ok(summary)) => match handle.take_database() {
                Some(bytes) => match autoeq::parse_user_database(&bytes) {
                    Ok(db) if db.entries.len() == summary.entries => {
                        i32::try_from(summary.entries).unwrap_or(i32::MAX)
                    }
                    Ok(_) => fail("the database does not match the summary"),
                    Err(e) => fail(e),
                },
                None => fail("no database after success"),
            },
        }
    })
}
