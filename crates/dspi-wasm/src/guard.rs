//! One call at a time, in the module itself (WEB-STUDY section 2, "JSPI's two
//! sharp edges").
//!
//! Under JSPI a call into the core can suspend (a transfer, a sleep) and the
//! page's event loop runs meanwhile. Anything that enters the module then runs
//! on the same linear-memory stack and finds the core's locks held: a read
//! traps, and two suspended calls that finish out of order overwrite each
//! other's stack with no error. The Worker's queue keeps calls apart; this
//! guard is the module's own line, so a call that slips past the queue (a
//! listener, a timer, a second caller) is refused before it touches anything.
//!
//! The state is module-wide on purpose: the stack it protects is the module's,
//! shared by every session the module holds. It is not device state.
//!
//! A panic ends the module: on `wasm32-unknown-unknown` a panic aborts (a
//! `RuntimeError: unreachable` in the page), so nothing after it unwinds,
//! locks stay held and the stack pointer is lost. The panic hook marks the
//! module dead first, with the message, and every later call is refused as
//! dead. Natively the panic unwinds and [`crate::calls`] catches it; the
//! module is marked dead the same way, so both targets behave alike.

use std::cell::{Cell, RefCell};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Idle,
    Busy,
    Dead,
}

thread_local! {
    static STATE: Cell<State> = const { Cell::new(State::Idle) };
    static PANIC: RefCell<String> = const { RefCell::new(String::new()) };
}

/// Why a call was not run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// Another call is running (suspended under JSPI): this one did not run.
    Busy,
    /// A call panicked earlier; the module must be dropped.
    Dead,
}

/// Held while a call runs; dropping it lets the next call in.
#[must_use]
pub struct Entered(());

impl Drop for Entered {
    fn drop(&mut self) {
        STATE.with(|s| {
            if s.get() == State::Busy {
                s.set(State::Idle);
            }
        });
    }
}

/// Enter the module, or say why not. Touches nothing else, so it is safe to
/// run while another call is suspended.
pub fn enter() -> Result<Entered, Refusal> {
    STATE.with(|s| match s.get() {
        State::Idle => {
            s.set(State::Busy);
            Ok(Entered(()))
        }
        State::Busy => Err(Refusal::Busy),
        State::Dead => Err(Refusal::Dead),
    })
}

/// Mark the module dead with the panic's message.
pub fn mark_dead(message: &str) {
    STATE.with(|s| s.set(State::Dead));
    // `try_borrow_mut`: a panic while the message is being read must not
    // panic again inside the hook.
    PANIC.with(|p| {
        if let Ok(mut p) = p.try_borrow_mut() {
            message.clone_into(&mut p);
        }
    });
}

/// Whether a panic has ended the module.
pub fn is_dead() -> bool {
    STATE.with(|s| s.get() == State::Dead)
}

/// The panic's message, empty while the module is alive.
pub fn panic_message() -> String {
    PANIC.with(|p| p.try_borrow().map(|p| p.clone()).unwrap_or_default())
}

/// Where the panic's message is, for the page to read after a trap: the
/// pointer and length of the stored bytes, valid until the next panic.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub fn panic_message_raw() -> (*const u8, usize) {
    PANIC.with(|p| {
        p.try_borrow()
            .map(|p| (p.as_ptr(), p.len()))
            .unwrap_or((std::ptr::null(), 0))
    })
}

/// Install the hook that marks the module dead before the abort. Once per
/// module; later calls do nothing.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub fn install_panic_hook() {
    use std::sync::Once;
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        std::panic::set_hook(Box::new(|info| {
            let what = info
                .payload()
                .downcast_ref::<&str>()
                .map(|s| (*s).to_owned())
                .or_else(|| info.payload().downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "a panic".to_owned());
            let at = info
                .location()
                .map(|l| format!(" at {}:{}", l.file(), l.line()))
                .unwrap_or_default();
            mark_dead(&format!("{what}{at}"));
        }));
    });
}

/// Back to a live, idle module: tests only (each test thread has its own).
#[cfg(test)]
pub fn reset_for_test() {
    STATE.with(|s| s.set(State::Idle));
    PANIC.with(|p| p.borrow_mut().clear());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wb3_a_second_entry_is_refused_until_the_first_leaves() {
        reset_for_test();
        let first = enter().unwrap();
        assert_eq!(enter().err(), Some(Refusal::Busy));
        drop(first);
        let again = enter().unwrap();
        drop(again);
    }

    #[test]
    fn wb3_a_dead_module_refuses_every_call_and_keeps_the_message() {
        reset_for_test();
        let held = enter().unwrap();
        mark_dead("boom");
        drop(held);
        assert_eq!(enter().err(), Some(Refusal::Dead));
        assert!(is_dead());
        assert_eq!(panic_message(), "boom");
    }
}
