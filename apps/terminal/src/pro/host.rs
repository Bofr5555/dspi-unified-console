//! The interface's [`ProHost`] (WP UB11, UXP B11): the Snapshots panel and
//! Settings › Speakers on the console core the interface opened. The core
//! is looked up on every call (`core_route::interface_core`), so a device
//! switch or a reconnect is followed. A recall over DSPi Link has the
//! shell's way back (`pro::guard_choice`); A/B and the speaker layout are
//! the core's own calls, with its own refusals.
#![warn(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

use std::sync::Arc;

use dspi_core::{
    AbCompare, AbSide, DspiCore, GuardPhase, GuardedApply, LayoutState, MeasureRole, RoleSource,
    SnapshotInfo, SpeakerLayout, SpeakerLayoutOutput,
};
use dspi_tui::host::{
    AbView, GuardView, ProHost, Recalled, Role, SnapshotRow, SpeakerRow, Speakers,
};

use crate::core_route;

/// The words with no device open on a core.
const NO_CORE: &str = "no device is open";

/// The interface's core calls.
pub(crate) struct CoreHost {
    /// A core of its own (the tests'); `None`: the interface's, as it is now.
    pinned: Option<(Arc<DspiCore>, bool)>,
}

impl CoreHost {
    /// On the core the interface has open, whichever it is at each call.
    pub(crate) fn interface() -> Self {
        Self { pinned: None }
    }

    fn core(&self) -> Result<(Arc<DspiCore>, bool), String> {
        self.pinned
            .clone()
            .or_else(core_route::interface_core)
            .ok_or_else(|| NO_CORE.to_owned())
    }
}

fn row(s: SnapshotInfo) -> SnapshotRow {
    SnapshotRow {
        id: s.id,
        name: s.name,
        note: s.note,
        when: super::snapshots::when(s.saved_ms),
    }
}

fn ab_view(c: AbCompare) -> AbView {
    AbView {
        active: c.active,
        on_b: c.side == Some(AbSide::B),
        a_name: c.a_name,
        b_name: c.b_name,
        changes: c
            .recall
            .map(|r| r.changes.into_iter().chain(r.notes).collect())
            .unwrap_or_default(),
    }
}

fn guard_view(g: GuardedApply) -> GuardView {
    GuardView {
        waiting: g.phase == GuardPhase::Waiting,
        remaining_ms: g.remaining_ms,
        label: g.label,
        message: g.message,
    }
}

fn role(r: MeasureRole) -> Role {
    match r {
        MeasureRole::Full => Role::Full,
        MeasureRole::Sub => Role::Sub,
        MeasureRole::Low => Role::Low,
        MeasureRole::Mid => Role::Mid,
        MeasureRole::High => Role::High,
    }
}

fn measure_role(r: Role) -> MeasureRole {
    match r {
        Role::Full => MeasureRole::Full,
        Role::Sub => MeasureRole::Sub,
        Role::Low => MeasureRole::Low,
        Role::Mid => MeasureRole::Mid,
        Role::High => MeasureRole::High,
    }
}

/// Whose word a row's role is, when it is not the person's.
fn source_words(s: RoleSource) -> &'static str {
    match s {
        RoleSource::User => "",
        RoleSource::Detected => "detected",
        RoleSource::Suggested => "suggested",
        RoleSource::Stated => "set by AI from your words",
    }
}

/// The layout as the page shows it; `stored`: the person's (or a detection's
/// or an agent's) kept layout, else the suggestion.
fn speakers_view(l: SpeakerLayout, stored: bool) -> Speakers {
    let when = || super::snapshots::when(l.updated_ms);
    let stamp = if !stored {
        "Suggested from the filters and the routing: check which output drives what, then \
         confirm."
            .to_owned()
    } else {
        match l.state() {
            LayoutState::Confirmed => format!("Confirmed {}.", when()),
            LayoutState::AutoDetected => format!("Detected, not confirmed, {}.", when()),
            LayoutState::Stated => format!("Set by AI from your words, not checked, {}.", when()),
            LayoutState::Unconfirmed => format!("Kept, not confirmed, {}.", when()),
        }
    };
    Speakers {
        rows: l
            .outputs
            .iter()
            .map(|o| SpeakerRow {
                output: o.output,
                role: role(o.role),
                speaker: o.speaker.clone(),
                group: o.group,
                source: source_words(o.source).to_owned(),
            })
            .collect(),
        confirmed: stored && l.confirmed_by_user,
        stamp,
        notes: l.notes,
    }
}

impl ProHost for CoreHost {
    fn snapshots(&self) -> Result<Vec<SnapshotRow>, String> {
        let (core, _) = self.core()?;
        Ok(core
            .snapshot_list(String::new())
            .into_iter()
            .map(row)
            .collect())
    }

    fn snapshot_save(&self, name: &str) -> Result<SnapshotRow, String> {
        let (core, _) = self.core()?;
        core.snapshot_capture(name.to_owned(), String::new())
            .map(row)
            .map_err(|e| e.to_string())
    }

    fn snapshot_delete(&self, id: &str) -> Result<(), String> {
        let (core, _) = self.core()?;
        core.snapshot_delete(id.to_owned())
            .map_err(|e| e.to_string())
    }

    fn snapshot_recall(&self, id: &str) -> Result<Recalled, String> {
        let (core, dlink) = self.core()?;
        // the shell's rule: a way back over DSPi Link, none locally
        match super::guard_choice(None, false, dlink) {
            Ok(Some(secs)) => core
                .apply_guarded_snapshot(id.to_owned(), secs)
                .map(|g| Recalled::Guarded(guard_view(g)))
                .map_err(|e| e.to_string()),
            _ => core
                .snapshot_recall(id.to_owned())
                .map(|r| Recalled::Done {
                    changes: r.changes,
                    notes: r.notes,
                })
                .map_err(|e| e.to_string()),
        }
    }

    fn guard(&self) -> GuardView {
        self.core().map_or_else(
            |_| GuardView::default(),
            |(c, _)| guard_view(c.apply_poll()),
        )
    }

    fn guard_keep(&self) -> Result<GuardView, String> {
        let (core, _) = self.core()?;
        core.apply_confirm()
            .map(guard_view)
            .map_err(|e| e.to_string())
    }

    fn guard_revert(&self) -> Result<GuardView, String> {
        let (core, _) = self.core()?;
        core.apply_revert()
            .map(guard_view)
            .map_err(|e| e.to_string())
    }

    fn ab(&self) -> AbView {
        self.core()
            .map_or_else(|_| AbView::default(), |(c, _)| ab_view(c.ab_state()))
    }

    fn ab_start(&self, b_id: &str) -> Result<AbView, String> {
        let (core, _) = self.core()?;
        core.ab_start(String::new(), b_id.to_owned())
            .map(ab_view)
            .map_err(|e| e.to_string())
    }

    fn ab_toggle(&self) -> Result<AbView, String> {
        let (core, _) = self.core()?;
        core.ab_toggle().map(ab_view).map_err(|e| e.to_string())
    }

    fn ab_keep(&self, b: bool) -> Result<AbView, String> {
        let (core, _) = self.core()?;
        core.ab_keep(if b { AbSide::B } else { AbSide::A })
            .map(ab_view)
            .map_err(|e| e.to_string())
    }

    fn speakers(&self) -> Result<Speakers, String> {
        let (core, _) = self.core()?;
        match core.speaker_layout().map_err(|e| e.to_string())? {
            Some(l) => Ok(speakers_view(l, true)),
            None => self.speakers_suggest(),
        }
    }

    fn speakers_suggest(&self) -> Result<Speakers, String> {
        let (core, _) = self.core()?;
        core.speaker_layout_propose()
            .map(|l| speakers_view(l, false))
            .map_err(|e| e.to_string())
    }

    fn speakers_confirm(&self, rows: &[SpeakerRow]) -> Result<Speakers, String> {
        let (core, _) = self.core()?;
        let layout = SpeakerLayout {
            outputs: rows
                .iter()
                .map(|r| SpeakerLayoutOutput {
                    output: r.output,
                    role: measure_role(r.role),
                    speaker: r.speaker.clone(),
                    group: r.group,
                    source: RoleSource::User,
                })
                .collect(),
            confirmed_by_user: true,
            ..SpeakerLayout::default()
        };
        core.speaker_layout_set(layout)
            .map(|l| speakers_view(l, true))
            .map_err(|e| e.to_string())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests_wp_ub11 {
    use super::*;
    use crate::core_route::{OpenedCore, open_core};
    use crate::pro::Stores;
    use crate::pro::tests::scratch;
    use dspi_tui::shell::connect::{self, Target};

    /// The demo on a core with its stores in `dir`, and a host on it.
    fn opened(dir: &std::path::Path) -> (OpenedCore, CoreHost) {
        let o = open_core(&Target::Demo, &Stores::under(dir), &connect::no_prompt)
            .expect("the demo opens on a core");
        let host = CoreHost {
            pinned: Some((Arc::clone(&o.handle.core), false)),
        };
        (o, host)
    }

    #[test]
    fn ub11_the_host_saves_lists_recalls_and_compares_on_the_interfaces_core() {
        let dir = scratch("ub11_host");
        let (o, host) = opened(&dir);
        let saved = host.snapshot_save("Before").expect("saved");
        assert_eq!(saved.name, "Before");
        assert!(saved.when.ends_with("UTC"), "{}", saved.when);
        let rows = host.snapshots().expect("listed");
        assert!(rows.iter().any(|r| r.id == saved.id), "{rows:?}");
        // a change, then the recall puts it back (USB/demo: no way back)
        o.handle
            .core
            .set_loudness(true)
            .expect("a change to recall over");
        match host.snapshot_recall(&saved.id).expect("recalled") {
            Recalled::Done { changes, .. } => assert!(!changes.is_empty(), "{changes:?}"),
            Recalled::Guarded(g) => panic!("no way back on the demo: {g:?}"),
        }
        assert!(!host.guard().waiting);
        // A/B: the device now (A) against the snapshot (B)
        let v = host.ab_start(&saved.id).expect("started");
        assert!(v.active && !v.on_b, "{v:?}");
        assert_eq!(v.b_name, "Before");
        let v = host.ab_toggle().expect("switched");
        assert!(v.on_b, "{v:?}");
        assert!(host.ab().active);
        let v = host.ab_keep(true).expect("kept");
        assert!(!v.active && v.on_b, "{v:?}");
        host.snapshot_delete(&saved.id).expect("deleted");
        assert!(
            host.snapshots()
                .expect("listed")
                .iter()
                .all(|r| r.id != saved.id)
        );
    }

    #[test]
    fn ub11_the_host_suggests_then_keeps_the_persons_confirmed_layout() {
        let dir = scratch("ub11_speakers");
        let (o, host) = opened(&dir);
        // -- INT016 -- (WP UB12) a new board: the demo's example keeps a confirmed layout
        o.handle.core.open_demo_blank().expect("the blank demo");
        let s = host.speakers().expect("a layout");
        assert!(!s.confirmed, "nothing kept yet: the suggestion");
        assert!(s.stamp.starts_with("Suggested"), "{}", s.stamp);
        let mut rows = s.rows.clone();
        if rows.is_empty() {
            rows.push(SpeakerRow {
                output: 0,
                ..SpeakerRow::default()
            });
        }
        rows[0].role = Role::High;
        rows[0].speaker = "L".into();
        let kept = host.speakers_confirm(&rows).expect("confirmed");
        assert!(kept.confirmed, "{kept:?}");
        assert!(kept.stamp.starts_with("Confirmed "), "{}", kept.stamp);
        assert_eq!(kept.rows[0].role, Role::High);
        assert_eq!(kept.rows[0].source, "", "the person's own word");
        // read back from the store
        let again = host.speakers().expect("read back");
        assert!(again.confirmed);
        assert_eq!(again.rows[0].role, Role::High);
        assert!(
            dir.join("speaker-layouts").exists(),
            "kept in the console's folder"
        );
    }
}
