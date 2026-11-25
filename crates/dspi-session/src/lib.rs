//! Orchestration: connect, discover what the device can do, read state.
//!
//! No rendering happens here, which is what lets the TUI and the one-shot CLI
//! share every behaviour rather than reimplementing it.

// -- WP CG15 --
pub mod about;
pub mod autoeq;
// -- WP BULKFIX --
pub mod bulk_gain;
pub mod clipboard;
pub mod compat;
pub mod cs_model;
// -- WP CG20 --
pub mod cs_rows;
pub mod ext;
pub mod ext_state;
// -- WP ATPROTO -- the time-base contract's I/O (PLAN-AT 1.3)
pub mod ext_timebase;
// -- WP P0M --
pub mod events;
mod fast;
pub mod filterfile;
// -- WP P0W --
pub mod history;
// -- WP P1LM --
pub mod limits;
// -- WP TM8 --
pub mod link_words;
// -- WP P0M --
pub mod meters;
pub mod notify;
// -- BUG1 --
pub mod output_label;
pub mod pins;
pub mod preset_file;
// -- WP FIN015 --
pub mod preset_copy;
pub mod probe;
pub mod readout;
// -- WP CG12 --
pub mod ring_health;
pub mod rta;
// -- WP TM5 --
pub mod rta_sharing;
pub mod runtime;
// -- WP P1SN --
pub mod snapshot_store;
// -- WP P5DOCK -- the measurement documents' store (PLAN-P5 T11)
pub mod measure_store;
pub mod state;
pub mod surfaces;
// -- WP INST0 -- the hub's installation model and validator, shared with the clients
pub mod system_model;
pub mod tube;
#[cfg(feature = "undo")]
pub mod undo;
// -- WP CG11 --
pub mod vreg_calibrate;
// -- WP CG14 --
pub mod overvolt;
pub mod write;
// -- WP TMCORE --
pub mod route;

// -- WP P0W --
pub use history::{Change, ChangeSource};
pub use notify::{Event, Notification, Notifications, Source};
pub use pins::{PinClaim, PinConstraint, PinMap, PinRole};
pub use probe::{Capabilities, Meters, probe, probe_any, read_bulk, read_meters};
pub use readout::Readout;
pub use state::{Applied, DeviceState, PresetSnapshot};
#[cfg(feature = "undo")]
pub use undo::Undone;
// -- WP TMCORE --
pub use route::{RouteFailure, RoutedStep, WriteRoute};
pub use write::{
    Core1Conflict, Crosspoint, EnableOutcome, JournalEntry, Outcome, OutputStrip, Session,
    WriteError,
};
