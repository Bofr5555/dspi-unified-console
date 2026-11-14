//! A session whose writes another layer makes (WP TMCORE).
//!
//! The Terminal's one-shot commands, its scripts and its interface write
//! through [`Session`](crate::Session). When the console core (`dspi-core`)
//! holds the device, those writes must be the core's: its limits profile,
//! its installer lock, its confirmation tokens and its undo history apply
//! to them as to every other client's. A [`WriteRoute`] set on the session
//! with [`Session::set_route`](crate::Session::set_route) takes them:
//!
//! - [`Session::write`](crate::Session::write) checks the path, the indices
//!   and the value as before (the registry's words for a mistake), asks the
//!   Core 1 interlock, keeps `dry_run` for itself, and then hands the write
//!   to the route. A route may decline a path ([`WriteRoute::write`] answers
//!   `None`): the session then sends it itself, over its own transport.
//! - [`Session::write_band`](crate::Session::write_band), undo and redo go to
//!   the route whole.
//! - [`Session::begin_group`](crate::Session::begin_group) and
//!   [`Session::end_group`](crate::Session::end_group) make one undo step of
//!   the writes between them. Without a route they do nothing.
//!
//! A route's refusal is [`WriteError::Routed`](crate::WriteError::Routed),
//! in the route's own words.
//!
//! [`Session::send`](crate::Session::send) (the fast path) is not routed: a
//! routed session's writes must go through `write`.

use dspi_proto::value::{EqParamPacket, Value};

use crate::write::{Outcome, WriteError};

/// What a routed undo or redo did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RoutedStep {
    /// The change reversed or applied again, in words for an echo line;
    /// `None` when nothing moved.
    pub command: Option<String>,
    /// What was stepped over, each with its reason.
    pub skipped: Vec<String>,
}

/// Why a routed write failed, for the exit code a script branches on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RouteFailure {
    /// The request itself is wrong (an argument out of range, a feature the
    /// device lacks).
    Input,
    /// Refused before anything was sent: a limit, a missing confirmation,
    /// the hub's role.
    Refused,
    // -- WP TMCOREF --
    /// Refused before anything was sent by the installer lock (this
    /// console's or the hub's): [`crate::Session::unlock`] with the
    /// installer's PIN opens it.
    Locked,
    /// No device.
    NoDevice,
    /// The link or the device failed.
    Transport,
}

/// The layer a session's writes go through (see the module docs).
pub trait WriteRoute: Send {
    /// Write registry parameter `path`, already checked by the session.
    /// `None`: not this route's to write; the session sends it itself.
    fn write(
        &mut self,
        path: &'static str,
        indices: &[u8],
        value: &Value,
    ) -> Option<Result<Outcome, WriteError>>;

    /// Write a whole EQ band (PEQ band 0..9, crossover band 20..23).
    fn write_band(&mut self, packet: &EqParamPacket) -> Result<Outcome, WriteError>;

    /// Undo (`redo` false) or redo the newest step. `Ok(None)`: nothing to
    /// step through.
    fn step(&mut self, redo: bool) -> Result<Option<RoutedStep>, WriteError>;

    /// Begin an undo group: the writes until the matching
    /// [`WriteRoute::end_group`] are one undo step called `label`.
    fn begin_group(&mut self, label: &str);

    /// End the group [`WriteRoute::begin_group`] began.
    fn end_group(&mut self);

    // -- WP TMCOREF --
    /// Unlock the installer lock with `pin` (or the recovery code). `None`
    /// when this route has no lock to open; otherwise what to tell the user,
    /// or why it stayed locked.
    fn unlock(&mut self, _pin: &str) -> Option<Result<String, String>> {
        None
    }
}
