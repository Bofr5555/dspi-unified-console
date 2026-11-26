//! What the interface asks of the program around it (WP UB11, UXP B11): the
//! console core's snapshot store, its A/B compare, its guarded recall and the
//! speaker layout. dspi-tui does not link the core; the Terminal does, and
//! hands the interface a [`ProHost`] (`Live::set_host`). Without one (the
//! fixture, `uconsole screenshot`), the Snapshots panel and Settings ›
//! Speakers say where these live instead.
//!
//! Every call is the core's own, with its own refusals (a read-only client,
//! the installer's lock, a guard that waits): the error is its sentence.

/// One snapshot in the store, as the panel lists it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SnapshotRow {
    /// The store's id, for the other calls.
    pub id: String,
    pub name: String,
    pub note: String,
    /// When it was taken, in the person's words ("26 Nov 21:23").
    pub when: String,
}

/// The A/B compare as the panel shows it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AbView {
    /// A compare is running.
    pub active: bool,
    /// The device is on B (else on A); after a keep, the side kept.
    pub on_b: bool,
    pub a_name: String,
    pub b_name: String,
    /// What the last switch changed, in the unsaved-changes list's words.
    pub changes: Vec<String>,
}

/// A recall with a way back (over DSPi Link), as the panel shows it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct GuardView {
    /// Waiting for Keep or Revert.
    pub waiting: bool,
    /// What is left of the countdown, ms.
    pub remaining_ms: u32,
    /// What was applied (the snapshot's name).
    pub label: String,
    /// How it ended, in one sentence; "" while waiting.
    pub message: String,
}

/// What a recall did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Recalled {
    /// Recalled: what changed, and what was not written and why.
    Done {
        changes: Vec<String>,
        notes: Vec<String>,
    },
    /// Applied with a way back: Keep or Revert before the countdown ends.
    Guarded(GuardView),
}

/// An output's role in the speaker layout (the measurement's roles).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Role {
    #[default]
    Full,
    Sub,
    Low,
    Mid,
    High,
}

impl Role {
    pub const ALL: [Role; 5] = [Role::Full, Role::Sub, Role::Low, Role::Mid, Role::High];

    /// The words the Qt and web Speakers pages use.
    pub fn words(self) -> &'static str {
        match self {
            Role::Full => "Full range",
            Role::Sub => "Subwoofer",
            Role::Low => "Low (woofer)",
            Role::Mid => "Mid",
            Role::High => "High (tweeter)",
        }
    }
}

/// One output of the speaker layout.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SpeakerRow {
    /// The output, 0-based.
    pub output: u8,
    pub role: Role,
    /// The speaker it belongs to ("L", "R", "Sub" or the person's word).
    pub speaker: String,
    /// Its alignment group: the outputs of one speaker share one.
    pub group: u8,
    /// Whose word the role is, when it is not the person's: "detected",
    /// "suggested", "set by AI from your words"; "" for the person's own.
    pub source: String,
}

/// The device's speaker layout, as Settings › Speakers shows it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Speakers {
    /// The outputs in the layout, in output order.
    pub rows: Vec<SpeakerRow>,
    /// The person confirmed it.
    pub confirmed: bool,
    /// One sentence on where it stands ("Confirmed 26 Nov 21:23.",
    /// "Suggested from the filters and the routing: …").
    pub stamp: String,
    /// The proposal's remarks (outputs left out and why).
    pub notes: Vec<String>,
}

/// The core's PRO calls the interface makes. Each answers the core's own
/// sentence on a refusal.
pub trait ProHost {
    /// The open device's snapshots, newest first.
    fn snapshots(&self) -> Result<Vec<SnapshotRow>, String>;
    /// Capture the device as it is now under `name`.
    fn snapshot_save(&self, name: &str) -> Result<SnapshotRow, String>;
    fn snapshot_delete(&self, id: &str) -> Result<(), String>;
    /// Recall `id`: at once over USB, with a way back over DSPi Link.
    fn snapshot_recall(&self, id: &str) -> Result<Recalled, String>;
    /// The recall's way back as it stands now (polled while it waits).
    fn guard(&self) -> GuardView;
    fn guard_keep(&self) -> Result<GuardView, String>;
    fn guard_revert(&self) -> Result<GuardView, String>;
    /// The compare as it stands.
    fn ab(&self) -> AbView;
    /// Compare the device now (A, captured) with snapshot `b_id` (B).
    fn ab_start(&self, b_id: &str) -> Result<AbView, String>;
    /// Switch to the other side.
    fn ab_toggle(&self) -> Result<AbView, String>;
    /// End the compare on B (`true`) or A.
    fn ab_keep(&self, b: bool) -> Result<AbView, String>;
    /// The layout as the person set it, else the suggestion.
    fn speakers(&self) -> Result<Speakers, String>;
    /// What the filters and the routing suggest (never kept by itself).
    fn speakers_suggest(&self) -> Result<Speakers, String>;
    /// Keep `rows` as the person's confirmed layout.
    fn speakers_confirm(&self, rows: &[SpeakerRow]) -> Result<Speakers, String>;
}

/// The host the screens share: set once a device is open on a core.
pub type HostCell = std::rc::Rc<std::cell::RefCell<Option<std::rc::Rc<dyn ProHost>>>>;

/// A countdown in the banner's words: "0:28".
pub fn countdown(ms: u32) -> String {
    let s = ms.div_ceil(1000);
    format!("{}:{:02}", s / 60, s % 60)
}

/// The snapshot `wanted` names, as `uconsole snapshot` reads it: its id, its
/// number in the list (from 1), or its name when one snapshot has it.
pub fn find_snapshot(rows: &[SnapshotRow], wanted: &str) -> Result<SnapshotRow, String> {
    if let Some(r) = rows.iter().find(|r| r.id == wanted) {
        return Ok(r.clone());
    }
    if let Some(r) = wanted
        .parse::<usize>()
        .ok()
        .and_then(|n| n.checked_sub(1))
        .and_then(|i| rows.get(i))
    {
        return Ok(r.clone());
    }
    let named: Vec<&SnapshotRow> = rows.iter().filter(|r| r.name == wanted).collect();
    match named.as_slice() {
        [one] => Ok((*one).clone()),
        [] => Err(format!(
            "No snapshot called “{wanted}” of this DSPi (N lists them)"
        )),
        many => Err(format!(
            "{} snapshots are called “{wanted}”: choose one in the Snapshots panel (N)",
            many.len()
        )),
    }
}
