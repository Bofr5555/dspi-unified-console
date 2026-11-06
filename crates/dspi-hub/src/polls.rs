//! Poll subscriptions (spec 7.6, 8.4). Owned by WP HB3.
//!
//! The scheduler is pure and lives with the device. The device worker alone runs
//! the reads ([`Polls::next_due`], [`Polls::take_due`], [`Polls::finished`]), so a
//! poll takes its turn with the commands and no second thread touches a transport.

use std::time::Instant;

use dspi_link::messages::{PollGrant, PollSpec};

use crate::session::{Role, SessionId};

/// One read for the worker to run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PollJob {
    /// The subscriber.
    pub session: SessionId,
    /// Its slot.
    pub slot: u8,
    /// The GET's `bRequest`.
    pub req: u8,
    /// The GET's `wValue`.
    pub val: u16,
    /// The GET's `wLength`.
    pub len: u16,
}

/// One device's subscriptions.
#[derive(Debug, Default)]
pub struct Polls {}

impl Polls {
    /// When the worker should next wake for a poll; `None` if nothing is subscribed.
    pub fn next_due(&self) -> Option<Instant> {
        None
    }

    /// The reads due now, oldest due first.
    pub fn take_due(&self, now: Instant) -> Vec<PollJob> {
        let _ = now;
        Vec::new()
    }

    /// A read finished. `reply` is `None` if it failed (spec 8.4: nothing is sent;
    /// three failures in a row give `poll.error`).
    pub fn finished(&self, job: &PollJob, reply: Option<Vec<u8>>) {
        let _ = (job, reply);
        todo!("WP HB3")
    }

    /// `poll.subscribe`: what each slot was granted.
    pub fn subscribe(&self, session: SessionId, role: Role, specs: &[PollSpec]) -> Vec<PollGrant> {
        let _ = (session, role, specs);
        todo!("WP HB3")
    }

    /// `poll.unsubscribe`.
    pub fn unsubscribe(&self, session: SessionId, slots: &[u8]) {
        let _ = (session, slots);
        todo!("WP HB3")
    }

    /// A session ended. Nothing is subscribed until WP HB3.
    pub fn session_closed(&self, session: SessionId) {
        let _ = session;
    }

    /// Whether a session other than `excluding` polls one of these opcodes, for the
    /// analyser's STOP arbitration (spec 7.15).
    pub fn others_poll(&self, opcodes: &[u8], excluding: SessionId) -> bool {
        let _ = (opcodes, excluding);
        todo!("WP HB3")
    }
}
