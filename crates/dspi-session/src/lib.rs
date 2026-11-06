//! Orchestration: connect, discover what the device can do, read state.
//!
//! No rendering happens here, which is what lets the TUI and the one-shot CLI
//! share every behaviour rather than reimplementing it.

pub mod autoeq;
pub mod clipboard;
pub mod compat;
pub mod cs_model;
pub mod ext;
pub mod ext_state;
mod fast;
pub mod filterfile;
pub mod notify;
pub mod pins;
pub mod preset_file;
pub mod probe;
pub mod readout;
pub mod rta;
pub mod runtime;
pub mod state;
pub mod surfaces;
pub mod tube;
#[cfg(feature = "undo")]
pub mod undo;
// -- WP CG11 --
pub mod vreg_calibrate;
pub mod write;

pub use notify::{Event, Notification, Notifications, Source};
pub use pins::{PinClaim, PinConstraint, PinMap, PinRole};
pub use probe::{Capabilities, Meters, probe, probe_any, read_bulk, read_meters};
pub use readout::Readout;
pub use state::{Applied, DeviceState, PresetSnapshot};
#[cfg(feature = "undo")]
pub use undo::Undone;
pub use write::{
    Core1Conflict, Crosspoint, EnableOutcome, JournalEntry, Outcome, OutputStrip, Session,
    WriteError,
};
