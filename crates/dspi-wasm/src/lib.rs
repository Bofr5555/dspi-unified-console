//! The web binding (WP WB3): `dspi-core` in wasm, behind the web console's
//! Worker (`apps/web/src/core/wasm.ts`), under JavaScript Promise Integration
//! (docs/core-merge/WEB-STUDY.md section 2).
//!
//! **Calls** are JSON ([`calls`]): one export, [`dspi_call`], takes a session
//! handle and `{"method", "params"}` and answers `{"ok", "value"}` or
//! `{"ok": false, "code", "error"}`. The shapes are the core's records, as
//! the C ABI's JSON carries them; `apps/web/src/core/protocol.ts` is the same
//! contract for the page.
//!
//! **Sessions**: any number per module, each its own `DspiCore`. The page
//! runs one per Worker; nothing here assumes one device.
//!
//! **One call at a time** ([`guard`]): a call that enters while another is
//! suspended is refused (`busy`), not run. That covers whatever might enter:
//! a second caller, a timer, a listener called back from inside `rta_tick`.
//!
//! **Panics** end the module ([`guard`]): the module is marked dead, the page
//! sees a trap, reads [`dspi_panic_message`], and the session is gone. Every
//! later call is refused (`dead`).
//!
//! **Imports** are the four the core's seams use, all from module `dspi`:
//! `dspi_now_ms() -> f64`, `dspi_unix_ms() -> f64`, `dspi_sleep_ms(f64)` (a
//! `WebAssembly.Suspending` timer) from `dspi_transport::clock`, and
//! `dspi_random_fill(ptr, len) -> i32` from `dspi-link`'s getrandom backend.
//! This crate adds WebUSB's three ([`usb`], WP WB4a): `dspi_usb_in` and
//! `dspi_usb_out` (Suspending control transfers) and `dspi_usb_error`.
//!
//! **The core is caller-driven** on the web (WB2): the Worker calls `poll`
//! for notifications and `rta_tick` for the analyser on its own timers,
//! through the same queue as the page's calls.
//!
//! **Link** (WP WB4b, [`link`]): `dspi-link`'s client and the core's session
//! over the browser's WebSocket, through the imports `dspi_ws_*`; the Worker
//! turns the session with `linkTick` (no threads here either).

// The core-merge quality bar (docs/core-merge/PLAN.md §1.7, §1.8): no
// panicking shortcuts on the page's data. clippy.toml exempts test code.
#![warn(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

// -- WP SW1 --
pub mod alink;
// -- WP WB8 --
pub mod band;
pub mod calls;
// -- WP WB6b --
pub mod curves;
// -- WP WB8 --
pub mod diag;
// -- WP WB6H -- the effect screens' feature gates and choice words
pub mod effects;
pub mod guard;
// -- WP P1SA --
pub mod guarded;
// -- WP P0W --
pub mod history;
// -- WP WB4b --
pub mod link;
// -- WP WB6G -- the input pages' source names
pub mod inputs;
// -- WP P1OF --
pub mod offline;
// -- WP WB6c --
pub mod outputs;
// -- WP WB6a --
pub mod presets;
// -- WP P1SN --
pub mod snapshots;
// -- WP P1AB --
pub mod ab;
// -- WP P1LM --
pub mod limits;
// -- WP P1LK --
pub mod modes;
// -- WP HUBADMIN -- the hub's admin API
pub mod hub_admin;
// -- WP P2CC1 -- the hub's own families (inventory, identify, health, history, snapshots)
pub mod hub_server;
// -- WP P2CC2 -- the hub's admin families (alarms, schedules, outbound, MQTT, API keys)
pub mod hub_server_admin;
// -- WP P2CC3 -- the hub's device and name writes (inventory set/forget, rename, identify by serial)
pub mod hub_server_manage;
// -- WP P3CC -- the hub's fleet families (groups, group writes, bulk apply, rollout)
pub mod hub_server_fleet;
// -- WP P1AQ --
pub mod autoeq;
// -- WP WB6K -- the AutoEQ database the page keeps, and filter files
pub mod autoeq_db;
pub mod filters;
// -- WP P1WB2 -- the DAC mute for the power-on page
pub mod power_on;
// -- WP SW3 --
pub mod system;
// -- WP ROMODE -- the client read-only mode
pub mod read_only;
// -- WP P5DOCK -- the measurement documents as text
pub mod measure_docs;
// -- WP CG-SWS-b -- the pin map
pub mod pins;
// -- WP WB6L -- the control-surface editor and the quick strip
pub mod surfaces;
// -- WP CLIP1 -- the channel clipboard
pub mod clipboard;
// -- WP P5SES -- the measurement session
pub mod measure;
// -- WP WB6I -- the Settings page: names, latency, the core-voltage flow
pub mod settings;
// -- WP WB4a --
pub mod usb;
// -- WP WB6J -- the analyser view, the signal generator, Stats for nerbs, the Interrupt Monitor
pub mod tools;
pub mod write;
// -- WP CG-SWS-a -- small stateless facts from the core
pub mod sws_a;

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
#[allow(unsafe_code)]
mod abi {
    //! The exports. Pointers are offsets in the module's memory; the page
    //! writes a call into a buffer from [`dspi_alloc`] and reads the reply at
    //! [`dspi_reply_ptr`] for the length [`dspi_call`] returned.

    use std::cell::RefCell;

    use crate::calls;
    use crate::guard::{self, Refusal};

    /// [`dspi_call`]'s answer when another call is running: nothing ran.
    pub const CALL_BUSY: u32 = u32::MAX;
    /// [`dspi_call`]'s answer when a panic has ended the module.
    pub const CALL_DEAD: u32 = u32::MAX - 1;

    thread_local! {
        static REPLY: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
    }

    /// `len` bytes for the page to write into; free with [`dspi_free`].
    #[unsafe(no_mangle)]
    pub extern "C" fn dspi_alloc(len: usize) -> *mut u8 {
        let mut v = vec![0u8; len];
        let p = v.as_mut_ptr();
        std::mem::forget(v);
        p
    }

    /// # Safety
    /// `ptr` and `len` from one [`dspi_alloc`].
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn dspi_free(ptr: *mut u8, len: usize) {
        if ptr.is_null() {
            return;
        }
        // SAFETY: as the caller promises, a vector of exactly this length.
        drop(unsafe { Vec::from_raw_parts(ptr, len, len) });
    }

    /// A new session; its handle, or `u32::MAX` while a call runs or after
    /// a panic.
    #[unsafe(no_mangle)]
    pub extern "C" fn dspi_session_new() -> u32 {
        guard::install_panic_hook();
        calls::session_new().unwrap_or(u32::MAX)
    }

    /// Close the session and forget it: 1 done, 0 unknown handle, -1 busy,
    /// -2 dead.
    #[unsafe(no_mangle)]
    pub extern "C" fn dspi_session_free(h: u32) -> i32 {
        match calls::session_free(h) {
            Ok(true) => 1,
            Ok(false) => 0,
            Err(Refusal::Busy) => -1,
            Err(Refusal::Dead) => -2,
        }
    }

    /// Run the JSON call at `ptr`, `len` on session `h`. The reply's length
    /// (read it at [`dspi_reply_ptr`]), or [`CALL_BUSY`] / [`CALL_DEAD`]
    /// when the call did not run; the reply buffer is then untouched.
    ///
    /// # Safety
    /// `ptr` is valid for `len` bytes.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn dspi_call(h: u32, ptr: *const u8, len: usize) -> u32 {
        // The refusal first, before anything else is touched: this may run
        // while another call is suspended.
        match guard::enter() {
            Err(Refusal::Busy) => return CALL_BUSY,
            Err(Refusal::Dead) => return CALL_DEAD,
            Ok(probe) => drop(probe),
        }
        let request = if ptr.is_null() || len == 0 {
            &[][..]
        } else {
            // SAFETY: as the caller promises.
            unsafe { std::slice::from_raw_parts(ptr, len) }
        };
        let out = calls::call(h, request);
        let n = u32::try_from(out.len()).unwrap_or(0);
        REPLY.with_borrow_mut(|r| *r = out);
        n
    }

    /// Where the last reply is; valid until the next [`dspi_call`].
    #[unsafe(no_mangle)]
    pub extern "C" fn dspi_reply_ptr() -> *const u8 {
        REPLY.with_borrow(|r| r.as_ptr())
    }

    /// The panic's message after a trap: its pointer; [`dspi_panic_len`]
    /// bytes. Reads one stored string and nothing else.
    #[unsafe(no_mangle)]
    pub extern "C" fn dspi_panic_ptr() -> *const u8 {
        guard::panic_message_raw().0
    }

    /// The panic message's length; 0 while the module is alive.
    #[unsafe(no_mangle)]
    pub extern "C" fn dspi_panic_len() -> usize {
        guard::panic_message_raw().1
    }
}
