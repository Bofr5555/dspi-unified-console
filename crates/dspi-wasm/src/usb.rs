//! WebUSB (WP WB4a): a DSPi on USB through the browser, Chromium only.
//!
//! The page chooses the device with the browser's own picker
//! (`navigator.usb.requestDevice`, a user gesture on the page) and hands its
//! serial to the session's Worker. The Worker finds it again in
//! `navigator.usb.getDevices()` (permission is the origin's, so a Worker sees
//! what the page was granted), opens it, claims vendor interface 2 and only
//! then calls `open` with `{"kind": "usb", "serial", "device", "vendorId"}`:
//! `device` is the Worker's number for that `USBDevice`
//! (`apps/web/src/core/usb.ts`). Each session is one device; a page holds as
//! many as it has Workers.
//!
//! **Control transfers** are the three `dspi` imports below, Promises under
//! JSPI: `dspi_usb_in` and `dspi_usb_out` are `WebAssembly.Suspending`, so the
//! core's blocking [`Transport`] calls suspend the module while the browser
//! does the transfer. They never call back into the module: what they do is
//! the transfer and nothing else, so nothing re-enters the core from inside a
//! call (not even from `rta_tick`, whose turns are transfers too).
//!
//! **Notifications** are pushed in by the caller (`usbPush`): the Worker
//! reads the notification endpoint itself and hands the packets in, which
//! the core's caller-driven reader takes (WB2). The transport therefore has
//! no [`dspi_transport::NotificationSource`]: a connection has one or the
//! other, never both, or two sequence streams would mix (c6a W2-2).
//!
//! **Unplug**: the browser's `disconnect` event, or a transfer that finds the
//! device gone, makes the Worker call `usbGone`. That ends the connection
//! (every later transfer is `Disconnected` without reaching the browser) and
//! answers the core's own words for it, which the page shows.

use std::cell::RefCell;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use dspi_core::{DspiCore, DspiError};
use dspi_transport::{DeviceDescriptor, Result, Transport, TransportCaps, TransportError};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::calls::CallError;

/// The legacy vendor ID older firmware enumerated with (`dspi-core`'s
/// `LEGACY_USB_VID`, not public there). The page's filters list it too.
pub const LEGACY_USB_VID: u16 = 0x2E8A;

/// The notification endpoint's number as WebUSB names it (no direction bit):
/// `VENDOR_EP_IN` is `0x83`.
pub const NOTIFY_ENDPOINT: u8 = (dspi_proto::generated::usb::VENDOR_EP_IN as u8) & 0x0F;

/// One notification packet's size, `VENDOR_EP_SIZE`.
pub const NOTIFY_PACKET: u16 = dspi_proto::generated::usb::VENDOR_EP_SIZE;

/// The largest control transfer: WinUSB's 4 KB, which Chromium on Windows
/// goes through; the core's chunked bulk opcodes fit it everywhere.
const MAX_TRANSFER: usize = 4096;

/// A host that carries the control transfers: the browser through the
/// imports ([`JsHost`]), or a test's double.
pub trait UsbHost: Send {
    /// IN transfer of up to `len` bytes; the reply as the device sent it.
    fn control_in(&mut self, opcode: u8, value: u16, len: u16) -> Result<Vec<u8>>;
    /// OUT transfer carrying `data`.
    fn control_out(&mut self, opcode: u8, value: u16, data: &[u8]) -> Result<()>;
}

/// A DSPi on WebUSB, as a [`Transport`] for the core.
pub struct WebUsbTransport {
    host: Box<dyn UsbHost>,
    gone: Arc<AtomicBool>,
    descriptor: DeviceDescriptor,
}

impl WebUsbTransport {
    /// Over `host`, named `serial`; `gone` ends it from outside (`usbGone`).
    pub fn new(host: Box<dyn UsbHost>, serial: String, gone: Arc<AtomicBool>) -> Self {
        Self {
            host,
            gone,
            descriptor: DeviceDescriptor {
                serial,
                bus_id: "webusb".into(),
                address: 0,
            },
        }
    }

    fn alive(&self) -> Result<()> {
        if self.gone.load(Ordering::Relaxed) {
            Err(TransportError::Disconnected)
        } else {
            Ok(())
        }
    }

    /// A host's answer, noting a departed device so later calls stop at once.
    fn seen<T>(&self, r: Result<T>) -> Result<T> {
        if matches!(r, Err(TransportError::Disconnected)) {
            self.gone.store(true, Ordering::Relaxed);
        }
        r
    }
}

impl Transport for WebUsbTransport {
    fn control_in(&mut self, opcode: u8, value: u16, len: u16) -> Result<Vec<u8>> {
        self.alive()?;
        let r = self.host.control_in(opcode, value, len);
        let data = self.seen(r)?;
        // As the native transport: a short reply answered another question.
        if data.len() < usize::from(len) {
            return Err(TransportError::ShortRead {
                opcode,
                wanted: usize::from(len),
                got: data.len(),
            });
        }
        Ok(data)
    }

    fn control_in_upto(&mut self, opcode: u8, value: u16, max_len: u16) -> Result<Vec<u8>> {
        self.alive()?;
        let r = self.host.control_in(opcode, value, max_len);
        self.seen(r)
    }

    fn control_out(&mut self, opcode: u8, value: u16, data: &[u8]) -> Result<()> {
        self.alive()?;
        let r = self.host.control_out(opcode, value, data);
        self.seen(r)
    }

    fn descriptor(&self) -> &DeviceDescriptor {
        &self.descriptor
    }

    fn max_transfer(&self) -> usize {
        MAX_TRANSFER
    }

    // `notifications` stays `None`: the caller pushes them (module docs).

    fn caps(&self) -> TransportCaps {
        TransportCaps::USB
    }
}

/// The imports' error codes (`apps/web/src/core/usb.ts`, `UsbResult`).
pub mod code {
    /// The device is gone (unplugged, closed).
    pub const GONE: i32 = -1;
    /// The device stalled the request.
    pub const STALL: i32 = -2;
    /// No answer within the transfer's time (1 s).
    pub const TIMEOUT: i32 = -3;
    /// Anything else; `dspi_usb_error` has the browser's words.
    pub const OTHER: i32 = -4;
}

/// An import's error code as the transport's error.
pub fn error_of(rc: i32, opcode: u8, message: impl FnOnce() -> String) -> TransportError {
    match rc {
        code::GONE => TransportError::Disconnected,
        code::STALL => TransportError::Stalled { opcode },
        code::TIMEOUT => TransportError::TimedOut { opcode, retries: 0 },
        _ => TransportError::Usb(message()),
    }
}

/// The browser's WebUSB, through the Worker's imports (module `dspi`).
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
#[allow(unsafe_code)]
mod js {
    use super::*;

    #[link(wasm_import_module = "dspi")]
    unsafe extern "C" {
        /// `controlTransferIn` of up to `len` bytes into `out`: the length
        /// read, or a [`code`]. Suspends.
        fn dspi_usb_in(device: u32, opcode: u32, value: u32, len: u32, out: *mut u8) -> i32;
        /// `controlTransferOut` of `len` bytes at `data`: 0, or a [`code`].
        /// Suspends.
        fn dspi_usb_out(device: u32, opcode: u32, value: u32, data: *const u8, len: u32) -> i32;
        /// The last failure's words for `device`, up to `cap` bytes into
        /// `out`; their length. Does not suspend.
        fn dspi_usb_error(device: u32, out: *mut u8, cap: u32) -> u32;
    }

    /// The Worker's `USBDevice` number `device`.
    pub struct JsHost {
        pub device: u32,
    }

    impl JsHost {
        fn message(&self) -> String {
            let mut buf = vec![0u8; 256];
            let cap = u32::try_from(buf.len()).unwrap_or(0);
            // SAFETY: `buf` is valid for `cap` bytes; the import writes at most that.
            let n = unsafe { dspi_usb_error(self.device, buf.as_mut_ptr(), cap) };
            buf.truncate(usize::try_from(n).unwrap_or(0).min(256));
            String::from_utf8_lossy(&buf).into_owned()
        }
    }

    impl UsbHost for JsHost {
        fn control_in(&mut self, opcode: u8, value: u16, len: u16) -> Result<Vec<u8>> {
            let mut buf = vec![0u8; usize::from(len)];
            // SAFETY: `buf` is valid for `len` bytes and outlives the call; the
            // import writes at most `len`.
            let rc = unsafe {
                dspi_usb_in(
                    self.device,
                    u32::from(opcode),
                    u32::from(value),
                    u32::from(len),
                    buf.as_mut_ptr(),
                )
            };
            if rc < 0 {
                return Err(error_of(rc, opcode, || self.message()));
            }
            buf.truncate(usize::try_from(rc).unwrap_or(0).min(usize::from(len)));
            Ok(buf)
        }

        fn control_out(&mut self, opcode: u8, value: u16, data: &[u8]) -> Result<()> {
            let len =
                u32::try_from(data.len()).map_err(|_| TransportError::Usb("too long".into()))?;
            // SAFETY: `data` is valid for `len` bytes for the whole call; the
            // import copies it before it suspends.
            let rc = unsafe {
                dspi_usb_out(
                    self.device,
                    u32::from(opcode),
                    u32::from(value),
                    data.as_ptr(),
                    len,
                )
            };
            if rc < 0 {
                return Err(error_of(rc, opcode, || self.message()));
            }
            Ok(())
        }
    }
}

thread_local! {
    /// The open USB connections, per session handle: the end switch and the
    /// serial WebUSB reported.
    static OPEN: RefCell<Vec<Open>> = const { RefCell::new(Vec::new()) };
}

struct Open {
    session: u32,
    gone: Arc<AtomicBool>,
    serial: String,
}

fn remember(h: u32, gone: Arc<AtomicBool>, serial: String) {
    OPEN.with_borrow_mut(|o| {
        o.retain(|x| x.session != h);
        o.push(Open {
            session: h,
            gone,
            serial,
        });
    });
}

fn forget(h: u32) -> Option<Arc<AtomicBool>> {
    OPEN.with_borrow_mut(|o| {
        let at = o.iter().position(|x| x.session == h)?;
        Some(o.swap_remove(at).gone)
    })
}

/// A snapshot (`calls::snapshot`) with the USB device's serial, which the
/// core's mirror does not carry, when session `h` is on USB.
pub fn with_serial(h: u32, mut snapshot: Value) -> Value {
    let serial = OPEN.with_borrow(|o| o.iter().find(|x| x.session == h).map(|x| x.serial.clone()));
    if let (Some(serial), Some(d)) = (serial, snapshot.get_mut("device")) {
        d["serial"] = json!(serial);
    }
    snapshot
}

/// `open`'s parameters for USB: protocol.ts `DeviceTarget` as the Worker
/// completes it.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsbTarget {
    #[serde(default)]
    pub serial: String,
    /// The Worker's number for the claimed `USBDevice`; absent when the
    /// Worker has none (no WebUSB, or a caller past the Worker).
    #[serde(default)]
    pub device: Option<u32>,
    #[serde(default)]
    pub vendor_id: u16,
}

/// `open` over USB on session `h`: the connect sequence over the claimed
/// device.
pub fn open(h: u32, core: &DspiCore, t: UsbTarget) -> std::result::Result<(), CallError> {
    if t.serial.is_empty() {
        return Err(CallError::new("invalid", "No DSPi was chosen"));
    }
    let Some(device) = t.device else {
        return Err(CallError::new(
            "unsupported",
            "USB needs a Chromium-based browser, such as Chrome or Edge",
        ));
    };
    let host = new_host(device)?;
    open_with(h, core, host, t.serial, t.vendor_id)
}

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
fn new_host(device: u32) -> std::result::Result<Box<dyn UsbHost>, CallError> {
    Ok(Box::new(js::JsHost { device }))
}

/// Natively there is no browser: only tests open USB, through [`open_with`].
#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
fn new_host(_device: u32) -> std::result::Result<Box<dyn UsbHost>, CallError> {
    Err(CallError::new(
        "unsupported",
        "USB in this binding is the browser's (WebUSB)",
    ))
}

/// [`open`] over any host (the tests' double natively).
pub fn open_with(
    h: u32,
    core: &DspiCore,
    host: Box<dyn UsbHost>,
    serial: String,
    vid: u16,
) -> std::result::Result<(), CallError> {
    let gone = Arc::new(AtomicBool::new(false));
    let transport = WebUsbTransport::new(host, serial.clone(), Arc::clone(&gone));
    // Replaces any open device, as `open_transport` does; the old switch goes.
    forget(h);
    core.open_transport(Box::new(transport), serial.clone(), vid)?;
    remember(h, gone, serial);
    Ok(())
}

/// `usbPush`'s parameters: packets as the endpoint delivered them, oldest
/// first.
#[derive(Debug, Deserialize)]
pub struct PushParams {
    pub packets: Vec<Vec<u8>>,
}

/// `usbPush`: hand the packets to the connection's reader. The Worker calls
/// `poll` after it, as after a wake.
pub fn push(core: &DspiCore, p: PushParams) -> std::result::Result<Value, CallError> {
    let mut taken = 0u32;
    for packet in p.packets {
        core.push_notification(packet, None)?;
        taken = taken.saturating_add(1);
    }
    Ok(json!(taken))
}

/// `usbGone`: the device went away. Ends the connection and answers the
/// core's words for it, `{"reason"}`.
pub fn gone(h: u32, core: &DspiCore) -> Value {
    if let Some(g) = forget(h) {
        g.store(true, Ordering::Relaxed);
    }
    core.disconnect();
    json!({ "reason": DspiError::Disconnected.to_string() })
}

/// Forget session `h`'s switch (`close`).
pub fn closed(h: u32) {
    forget(h);
}

/// `usbIds`: what the page's filters and the Worker's claim use, from the
/// protocol crate, so the page's copies are tested against it.
pub fn ids() -> Value {
    json!({
        "vendorId": dspi_proto::USB_VID,
        "productId": dspi_proto::USB_PID,
        "legacyVendorId": LEGACY_USB_VID,
        "interface": dspi_proto::VENDOR_INTERFACE,
        "notifyEndpoint": NOTIFY_ENDPOINT,
        "notifyPacket": NOTIFY_PACKET,
    })
}

/// The binding's own WebUSB mock device for its tests (`test-hooks` only):
/// the core's simulated DSPi (`dspi_transport::FakeDevice::demo`) behind
/// plain exports, so a WebUSB mock in JavaScript (`apps/web/test/pages/
/// webusb-mock.ts`) answers control transfers and the notification endpoint
/// as a real device would. It runs in its own instance of the test module,
/// never in the instance that holds the sessions, and never touches the
/// guard. Never in the page's module.
#[cfg(all(feature = "test-hooks", target_arch = "wasm32", target_os = "unknown"))]
#[allow(unsafe_code)]
mod mock {
    use std::cell::RefCell;

    use dspi_transport::{FakeDevice, FakeRemote, NotificationSource, Transport, TransportError};

    use super::code;

    struct Mock {
        device: FakeDevice,
        remote: FakeRemote,
        notes: Option<Box<dyn NotificationSource>>,
    }

    thread_local! {
        static MOCKS: RefCell<Vec<Mock>> = const { RefCell::new(Vec::new()) };
        static BUF: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
    }

    fn rc_of(e: &TransportError) -> i32 {
        match e {
            TransportError::Disconnected => code::GONE,
            TransportError::Stalled { .. } => code::STALL,
            TransportError::TimedOut { .. } => code::TIMEOUT,
            _ => code::OTHER,
        }
    }

    fn with<R>(id: u32, f: impl FnOnce(&mut Mock) -> R) -> Option<R> {
        MOCKS.with_borrow_mut(|m| m.get_mut(id as usize).map(f))
    }

    fn answer(r: Result<Vec<u8>, TransportError>) -> i32 {
        match r {
            Ok(v) => {
                let n = i32::try_from(v.len()).unwrap_or(0);
                BUF.with_borrow_mut(|b| *b = v);
                n
            }
            Err(e) => rc_of(&e),
        }
    }

    /// A new simulated DSPi; its number.
    #[unsafe(no_mangle)]
    pub extern "C" fn dspi_mock_new() -> u32 {
        let device = FakeDevice::demo();
        let remote = device.remote();
        let notes = device.notifications();
        MOCKS.with_borrow_mut(|m| {
            m.push(Mock {
                device,
                remote,
                notes,
            });
            u32::try_from(m.len().saturating_sub(1)).unwrap_or(u32::MAX)
        })
    }

    /// An IN transfer: its length (the bytes at `dspi_mock_buf_ptr`), or a
    /// [`code`].
    #[unsafe(no_mangle)]
    pub extern "C" fn dspi_mock_in(id: u32, opcode: u32, value: u32, len: u32) -> i32 {
        let (Ok(op), Ok(v), Ok(l)) = (
            u8::try_from(opcode),
            u16::try_from(value),
            u16::try_from(len),
        ) else {
            return code::OTHER;
        };
        with(id, |m| answer(m.device.control_in_upto(op, v, l))).unwrap_or(code::GONE)
    }

    /// An OUT transfer of `len` bytes at `ptr`: 0, or a [`code`].
    ///
    /// # Safety
    /// `ptr` is valid for `len` bytes.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn dspi_mock_out(
        id: u32,
        opcode: u32,
        value: u32,
        ptr: *const u8,
        len: usize,
    ) -> i32 {
        let (Ok(op), Ok(v)) = (u8::try_from(opcode), u16::try_from(value)) else {
            return code::OTHER;
        };
        let data = if ptr.is_null() || len == 0 {
            &[][..]
        } else {
            // SAFETY: as the caller promises.
            unsafe { std::slice::from_raw_parts(ptr, len) }
        };
        with(id, |m| match m.device.control_out(op, v, data) {
            Ok(()) => 0,
            Err(e) => rc_of(&e),
        })
        .unwrap_or(code::GONE)
    }

    /// The next notification packet, without waiting: its length, 0 when
    /// there is none, or [`code::GONE`] after an unplug.
    #[unsafe(no_mangle)]
    pub extern "C" fn dspi_mock_notify(id: u32) -> i32 {
        with(id, |m| match m.notes.as_mut() {
            Some(n) => answer(n.read(std::time::Duration::ZERO)),
            None => code::GONE,
        })
        .unwrap_or(code::GONE)
    }

    /// A change made at the device: the device's name, as another host
    /// renaming it (`len` bytes of UTF-8 at `ptr`). 0, or a [`code`].
    ///
    /// # Safety
    /// `ptr` is valid for `len` bytes.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn dspi_mock_rename(id: u32, ptr: *const u8, len: usize) -> i32 {
        let name = if ptr.is_null() || len == 0 {
            &[][..]
        } else {
            // SAFETY: as the caller promises.
            unsafe { std::slice::from_raw_parts(ptr, len) }
        };
        // The demo's own "renamed elsewhere" request (dspi-core's
        // `demo_rename_elsewhere`): target 0xFF is the device name.
        let mut payload = vec![0xFF];
        payload.extend_from_slice(name);
        payload.push(0);
        with(id, |m| {
            match m.device.control_out(
                dspi_proto::generated::opcodes::REQ_EXTENSION,
                dspi_transport::fake::EXT_FAKE_RENAME_ELSEWHERE,
                &payload,
            ) {
                Ok(()) => 0,
                Err(e) => rc_of(&e),
            }
        })
        .unwrap_or(code::GONE)
    }

    /// Unplug the simulated device: its transfers and its endpoint end.
    #[unsafe(no_mangle)]
    pub extern "C" fn dspi_mock_unplug(id: u32) {
        with(id, |m| m.remote.disconnect());
    }

    /// Where the last reply is; valid until the next mock call.
    #[unsafe(no_mangle)]
    pub extern "C" fn dspi_mock_buf_ptr() -> *const u8 {
        BUF.with_borrow(|b| b.as_ptr())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::calls;
    use crate::guard;
    use dspi_transport::{FakeDevice, FakeRemote};
    use std::sync::Mutex;

    /// The tests' WebUSB: the core's simulated DSPi, counting transfers.
    struct FakeHost {
        dev: FakeDevice,
        calls: Arc<Mutex<u32>>,
    }

    impl UsbHost for FakeHost {
        fn control_in(&mut self, opcode: u8, value: u16, len: u16) -> Result<Vec<u8>> {
            let mut n = self.calls.lock().unwrap();
            *n = n.saturating_add(1);
            drop(n);
            self.dev.control_in_upto(opcode, value, len)
        }
        fn control_out(&mut self, opcode: u8, value: u16, data: &[u8]) -> Result<()> {
            let mut n = self.calls.lock().unwrap();
            *n = n.saturating_add(1);
            drop(n);
            self.dev.control_out(opcode, value, data)
        }
    }

    fn fake() -> (Box<FakeHost>, FakeRemote, Arc<Mutex<u32>>) {
        let dev = FakeDevice::demo();
        let remote = dev.remote();
        let calls = Arc::new(Mutex::new(0));
        (
            Box::new(FakeHost {
                dev,
                calls: Arc::clone(&calls),
            }),
            remote,
            calls,
        )
    }

    fn run(h: u32, method: &str, params: Value) -> Value {
        let out = calls::call(
            h,
            json!({ "method": method, "params": params })
                .to_string()
                .as_bytes(),
        );
        serde_json::from_slice(&out).unwrap()
    }

    fn ok(h: u32, method: &str, params: Value) -> Value {
        let r = run(h, method, params);
        assert_eq!(r["ok"], true, "{method}: {r}");
        r["value"].clone()
    }

    /// Session `h`'s core, as `calls` holds it.
    fn open_fake(h: u32, serial: &str) -> (FakeRemote, Arc<Mutex<u32>>) {
        let (host, remote, calls) = fake();
        let core = calls::core_of(h).unwrap();
        open_with(h, &core, host, serial.into(), dspi_proto::USB_VID).unwrap();
        (remote, calls)
    }

    #[test]
    fn wb4a_two_usb_sessions_connect_write_with_read_back_and_one_unplugs() {
        guard::reset_for_test();
        let a = calls::session_new().unwrap();
        let b = calls::session_new().unwrap();
        let (_ra, calls_a) = open_fake(a, "USBA0001");
        let (_rb, _) = open_fake(b, "USBB0002");

        let snap = ok(a, "state", Value::Null);
        assert_eq!(snap["device"]["demo"], false, "a USB session, not the demo");
        assert_eq!(snap["device"]["serial"], "USBA0001");
        assert_eq!(snap["device"]["readOnly"], false);
        assert_eq!(ok(b, "state", Value::Null)["device"]["serial"], "USBB0002");
        assert!(
            *calls_a.lock().unwrap() > 0,
            "the connect went through the host"
        );

        let v = ok(
            a,
            "write",
            json!({ "path": "vol.user", "value": { "float": -14.5 } }),
        );
        assert_eq!(v, json!({ "float": -14.5 }));
        assert_eq!(ok(a, "state", Value::Null)["state"]["userVolumeDb"], -14.5);
        assert_ne!(ok(b, "state", Value::Null)["state"]["userVolumeDb"], -14.5);
        // Not the demo: its simulations are refused.
        let r = run(
            a,
            "demo",
            json!({ "action": "renameElsewhere", "name": "X" }),
        );
        assert_eq!(r["code"], "refused");

        // Unplug a: ended in the core's words; b goes on.
        let before = *calls_a.lock().unwrap();
        let g = ok(a, "usbGone", Value::Null);
        assert_eq!(g["reason"], DspiError::Disconnected.to_string());
        assert_eq!(run(a, "state", Value::Null)["code"], "not_open");
        assert_eq!(ok(a, "poll", Value::Null)["connected"], false);
        assert_eq!(
            *calls_a.lock().unwrap(),
            before,
            "nothing reached the host after"
        );
        let v = ok(
            b,
            "write",
            json!({ "path": "vol.user", "value": { "float": -9.0 } }),
        );
        assert_eq!(v, json!({ "float": -9.0 }));

        ok(b, "close", Value::Null);
        for h in [a, b] {
            calls::session_free(h).unwrap();
        }
    }

    #[test]
    fn wb4a_a_device_that_leaves_mid_call_ends_the_transport() {
        let (host, remote, calls) = fake();
        let gone = Arc::new(AtomicBool::new(false));
        let mut t = WebUsbTransport::new(host, "S".into(), Arc::clone(&gone));
        assert!(
            t.control_in_upto(dspi_proto::generated::opcodes::REQ_GET_PLATFORM, 0, 7)
                .is_ok()
        );
        remote.disconnect();
        assert!(matches!(
            t.control_in(dspi_proto::generated::opcodes::REQ_GET_PLATFORM, 0, 7),
            Err(TransportError::Disconnected)
        ));
        assert!(gone.load(Ordering::Relaxed));
        let n = *calls.lock().unwrap();
        assert!(matches!(
            t.control_out(0x42, 0, &[1]),
            Err(TransportError::Disconnected)
        ));
        assert_eq!(
            *calls.lock().unwrap(),
            n,
            "a gone device is not asked again"
        );
        assert!(t.notifications().is_none(), "the caller pushes (c6a W2-2)");
        assert_eq!(t.max_transfer(), 4096);
    }

    #[test]
    fn wb4a_a_short_reply_is_refused_unless_the_opcode_allows_it() {
        struct Short;
        impl UsbHost for Short {
            fn control_in(&mut self, _: u8, _: u16, _: u16) -> Result<Vec<u8>> {
                Ok(vec![1, 2])
            }
            fn control_out(&mut self, _: u8, _: u16, _: &[u8]) -> Result<()> {
                Ok(())
            }
        }
        let mut t = WebUsbTransport::new(Box::new(Short), "S".into(), Arc::default());
        assert!(matches!(
            t.control_in(0x10, 0, 4),
            Err(TransportError::ShortRead {
                wanted: 4,
                got: 2,
                ..
            })
        ));
        assert_eq!(t.control_in_upto(0x10, 0, 4).unwrap(), vec![1, 2]);
    }

    #[test]
    fn wb4a_import_codes_map_to_the_transports_errors() {
        assert!(matches!(
            error_of(code::GONE, 1, String::new),
            TransportError::Disconnected
        ));
        assert!(matches!(
            error_of(code::STALL, 7, String::new),
            TransportError::Stalled { opcode: 7 }
        ));
        assert!(matches!(
            error_of(code::TIMEOUT, 7, String::new),
            TransportError::TimedOut { retries: 0, .. }
        ));
        assert_eq!(
            error_of(code::OTHER, 7, || "babble".into()).to_string(),
            "usb error: babble"
        );
    }

    #[test]
    fn wb4a_usb_open_without_a_browser_device_says_why() {
        guard::reset_for_test();
        let h = calls::session_new().unwrap();
        let r = run(h, "open", json!({ "kind": "usb" }));
        assert_eq!(r["code"], "invalid", "no serial: {r}");
        let r = run(h, "open", json!({ "kind": "usb", "serial": "X" }));
        assert_eq!(r["code"], "unsupported", "{r}");
        let r = run(
            h,
            "open",
            json!({ "kind": "usb", "serial": "X", "device": 0, "vendorId": 0x2E8B }),
        );
        assert_eq!(r["code"], "unsupported", "natively no browser: {r}");
        let ids = ok(h, "usbIds", Value::Null);
        assert_eq!(ids["vendorId"], 0x2E8B);
        assert_eq!(ids["productId"], 0xFEAA);
        assert_eq!(ids["interface"], 2);
        assert_eq!(ids["notifyEndpoint"], 3);
        assert_eq!(ids["notifyPacket"], 64);
        calls::session_free(h).unwrap();
    }

    // -- WP WB6b -- (c6p WB6a-2)
    #[test]
    fn wb6b_a_failed_read_after_a_usb_open_leaves_no_claim() {
        guard::reset_for_test();
        let h = calls::session_new().unwrap();
        let core = calls::core_of(h).unwrap();
        // The device goes between the open and the reads after it (an
        // unplug): the reads (fetch_all) fail.
        let (host, _remote, _) = fake();
        open_with(h, &core, host, "USBF0001".into(), dspi_proto::USB_VID).unwrap();
        assert_eq!(
            with_serial(h, json!({ "device": {} }))["device"]["serial"],
            "USBF0001"
        );
        core.disconnect();
        assert!(calls::usb_opened(h, &core).is_err());
        assert!(!core.is_connected());
        // The session keeps no serial and no switch of the device.
        let snap = with_serial(h, json!({ "device": { "serial": "" } }));
        assert_eq!(snap["device"]["serial"], "");
        assert!(forget(h).is_none());
        calls::session_free(h).unwrap();
    }
}
