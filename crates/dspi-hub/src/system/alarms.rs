//! Link alarms (WP SW2, SYS2 7, PRO1 M3's shape): raised from what the hub
//! reads (`system.status`) and from the units' 0xE1 events as they arrive,
//! acknowledged by a person, cleared when the condition goes. Pure state; the
//! caller says what it saw and when.
//!
//! | Code | Raised | Cleared |
//! |---|---|---|
//! | `link_lost` | a unit not locked (searching, holdover, off), or an 0xE1 with a loss reason; its count is the unit's losses since it was raised | locked again |
//! | `unit_offline` | a unit of the model not attached | attached |
//! | `link_errors` | a unit's CRC-16 errors (`crc16_errors`) grew by [`ERROR_STEP`] or more since the last read (a loss is `link_lost`'s) | a read with less growth |
//! | `reanchor` | an 0xE1 `REANCHOR` (a short silence) | acknowledged |
//! | `system_muted` | the sender's system mute set (by `system.mute`, a scene or another host), seen at a read or a write | cleared |
//! | `delay_below_min` | `EXT_LINK_PHASE` flag `D_BELOW_MIN` | the flag gone |

use std::collections::BTreeMap;

use serde::Serialize;

use super::align::Observed;

/// Bank errors between two reads that raise `link_errors`.
pub const ERROR_STEP: u32 = 16;
/// Cleared alarms kept for the record.
const KEEP_CLEARED: usize = 64;

/// An alarm's life.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AlarmState {
    /// On, nobody has seen it.
    Raised,
    /// On, someone has.
    Acknowledged,
    /// Over.
    Cleared,
}

/// One alarm.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Alarm {
    /// Unique for the hub's run.
    pub id: u64,
    /// See the module's table.
    pub code: String,
    /// The element (a unit's id).
    pub subject: String,
    /// `warning` or `critical`.
    pub severity: String,
    /// For people.
    pub message: String,
    /// Its state.
    pub state: AlarmState,
    /// Unix seconds.
    pub raised_at: u64,
    /// Unix seconds, once cleared.
    pub cleared_at: Option<u64>,
    /// How often it was raised again while on.
    pub count: u32,
}

/// What changed, for the `system.alarm` event.
pub type Changes = Vec<Alarm>;

/// The alarms of one hub run.
#[derive(Debug, Default)]
pub struct AlarmBook {
    next_id: u64,
    /// On, by (code, subject).
    active: BTreeMap<(String, String), Alarm>,
    cleared: Vec<Alarm>,
    /// The counters at the last read, by subject: CRC-16 errors, losses.
    counters: BTreeMap<String, (u32, u32)>,
    // -- WP FIXS1 --
    /// The losses before a subject's `link_lost` was raised, so its count
    /// follows the unit's loss counter however often it is seen.
    loss_base: BTreeMap<String, u32>,
}

fn severity(code: &str) -> &'static str {
    match code {
        "link_lost" | "unit_offline" | "delay_below_min" => "critical",
        _ => "warning",
    }
}

impl AlarmBook {
    /// Raise (or count again) an alarm. Returns it if it is new.
    pub fn raise(&mut self, code: &str, subject: &str, message: String, now: u64) -> Option<Alarm> {
        self.raise_counted(code, subject, message, now, true)
    }

    // -- WP FIXS1 --
    /// Raise an alarm; one already on counts again only if `again` (a new
    /// occurrence, not the same condition seen once more).
    fn raise_counted(
        &mut self,
        code: &str,
        subject: &str,
        message: String,
        now: u64,
        again: bool,
    ) -> Option<Alarm> {
        let key = (code.to_owned(), subject.to_owned());
        if let Some(a) = self.active.get_mut(&key) {
            if again {
                a.count = a.count.saturating_add(1);
            }
            a.message = message;
            return None;
        }
        self.next_id = self.next_id.saturating_add(1);
        let a = Alarm {
            id: self.next_id,
            code: code.into(),
            subject: subject.into(),
            severity: severity(code).into(),
            message,
            state: AlarmState::Raised,
            raised_at: now,
            cleared_at: None,
            count: 1,
        };
        self.active.insert(key, a.clone());
        Some(a)
    }

    /// Clear an alarm if on. Returns it if it was.
    pub fn clear(&mut self, code: &str, subject: &str, now: u64) -> Option<Alarm> {
        if code == "link_lost" {
            self.loss_base.remove(subject);
        }
        let mut a = self.active.remove(&(code.to_owned(), subject.to_owned()))?;
        a.state = AlarmState::Cleared;
        a.cleared_at = Some(now);
        self.cleared.push(a.clone());
        if self.cleared.len() > KEEP_CLEARED {
            self.cleared.remove(0);
        }
        Some(a)
    }

    /// Acknowledge by id. A `reanchor` acknowledged is over.
    pub fn acknowledge(&mut self, id: u64, now: u64) -> Option<Alarm> {
        let key = self
            .active
            .iter()
            .find(|(_, a)| a.id == id)
            .map(|(k, _)| k.clone())?;
        if key.0 == "reanchor" {
            return self.clear(&key.0, &key.1, now);
        }
        let a = self.active.get_mut(&key)?;
        a.state = AlarmState::Acknowledged;
        Some(a.clone())
    }

    /// The alarms on, then the cleared ones kept, newest first.
    pub fn list(&self) -> Vec<Alarm> {
        let mut on: Vec<Alarm> = self.active.values().cloned().collect();
        on.sort_by_key(|a| std::cmp::Reverse(a.id));
        on.extend(self.cleared.iter().rev().cloned());
        on
    }

    /// The count of alarms on.
    pub fn active(&self) -> usize {
        self.active.len()
    }

    fn set(
        &mut self,
        on: bool,
        code: &str,
        subject: &str,
        msg: String,
        now: u64,
        out: &mut Changes,
    ) {
        // A condition still there is the same alarm, not another (WP FIXS1).
        let change = if on {
            self.raise_counted(code, subject, msg, now, false)
        } else {
            self.clear(code, subject, now)
        };
        out.extend(change);
    }

    /// What a read of unit `subject` showed (`None`: not attached). Returns
    /// what changed.
    pub fn observe(&mut self, subject: &str, seen: Option<&Observed>, now: u64) -> Changes {
        let mut out = Vec::new();
        let online = seen.is_some_and(|o| o.online);
        self.set(
            !online,
            "unit_offline",
            subject,
            format!("{subject}: not attached to the hub"),
            now,
            &mut out,
        );
        let Some(o) = seen.filter(|_| online) else {
            return out;
        };
        let before = self.counters.get(subject).copied();
        if let Some(state) = &o.state {
            let locked = state == "locked";
            self.set(
                !locked,
                "link_lost",
                subject,
                format!("{subject}: the audio link {}", state_words(state)),
                now,
                &mut out,
            );
            // -- WP FIXS1 -- one loss is one, however often it is seen (a
            // read and the 0xE1 alike): the count is the losses since.
            if let (false, Some(losses)) = (locked, o.loss_count) {
                let base = *self.loss_base.entry(subject.to_owned()).or_insert_with(|| {
                    before.map_or(losses.saturating_sub(1), |(_, l0)| {
                        l0.min(losses.saturating_sub(1))
                    })
                });
                if let Some(a) = self
                    .active
                    .get_mut(&("link_lost".to_owned(), subject.to_owned()))
                {
                    a.count = losses.saturating_sub(base).max(1);
                }
            }
        }
        if let (Some(errors), Some(losses)) = (o.crc16_errors, o.loss_count) {
            let grown = match self.counters.insert(subject.to_owned(), (errors, losses)) {
                // A reset counter (lower now) is no growth.
                Some((e0, _)) => errors.saturating_sub(e0),
                None => 0,
            };
            self.set(
                grown >= ERROR_STEP,
                "link_errors",
                subject,
                format!("{subject}: {grown} CRC-16 errors on the audio link since the last read"),
                now,
                &mut out,
            );
        }
        if let Some((_, _, _, flags)) = o.phase {
            self.set(
                flags & 1 != 0,
                "delay_below_min",
                subject,
                format!("{subject}: its delay is below its minimum; it runs without phase lock"),
                now,
                &mut out,
            );
        }
        out
    }

    // -- fixup sw2x (F1) --
    /// The system mute of sender `subject` as last read or written.
    pub fn system_mute(&mut self, subject: &str, muted: bool, now: u64) -> Changes {
        let mut out = Vec::new();
        self.set(
            muted,
            "system_muted",
            subject,
            format!("{subject}: the system is muted; every unit on its link plays silence"),
            now,
            &mut out,
        );
        out
    }

    /// An 0xE1 event from unit `subject`: `state` and `reason` by name.
    pub fn event(
        &mut self,
        subject: &str,
        state: &str,
        reason: &str,
        is_loss: bool,
        now: u64,
    ) -> Changes {
        let mut out = Vec::new();
        if is_loss || matches!(state, "searching" | "holdover") {
            // The read that follows counts the loss (WP FIXS1).
            out.extend(self.raise_counted(
                "link_lost",
                subject,
                format!(
                    "{subject}: the audio link was lost ({})",
                    reason_words(reason)
                ),
                now,
                false,
            ));
        } else if state == "locked" {
            out.extend(self.clear("link_lost", subject, now));
        }
        if reason == "reanchor" {
            out.extend(self.raise(
                "reanchor",
                subject,
                format!("{subject}: re-anchored (a few ms of silence)"),
                now,
            ));
        }
        out
    }
}

// -- WP FIXS1 --
/// "the audio link …" for a state's name, in the glossary's words.
fn state_words(state: &str) -> String {
    match state {
        "holdover" => "is in holdover".into(),
        "searching" => "is searching".into(),
        "off" => "is off".into(),
        "unknown" => "is in an unknown state".into(),
        s => format!("is {s}"),
    }
}

/// Why a link was lost, for people (an 0xE1 reason's name).
fn reason_words(reason: &str) -> &str {
    match reason {
        "los" => "no signal at the input",
        "header_errors" => "bad frame heads",
        "comma_lost" => "framing lost",
        "vcxo_limit" => "clock beyond its range",
        "disabled" => "turned off",
        r => r,
    }
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects, clippy::indexing_slicing)]
mod tests_wp_sw2 {
    use super::*;

    pub(super) fn locked(errors: u32, losses: u32) -> Observed {
        Observed {
            online: true,
            state: Some("locked".into()),
            crc16_errors: Some(errors),
            loss_count: Some(losses),
            phase: Some((1, 128, 0, 0)),
            ..Observed::default()
        }
    }

    #[test]
    fn wp_sw2_loss_raises_and_lock_clears() {
        let mut b = AlarmBook::default();
        assert!(b.observe("A", Some(&locked(0, 0)), 1).is_empty());
        let mut lost = locked(0, 1);
        lost.state = Some("holdover".into());
        let c = b.observe("A", Some(&lost), 2);
        let codes: Vec<&str> = c.iter().map(|a| a.code.as_str()).collect();
        // -- WP FIXS1 -- a loss is link_lost's alone, not a counters alarm.
        assert_eq!(codes, ["link_lost"]);
        assert_eq!(c[0].severity, "critical");
        // Again: not new.
        assert!(
            b.observe("A", Some(&lost), 3)
                .iter()
                .all(|a| a.code != "link_lost")
        );
        assert_eq!(b.active(), 1);
        let c = b.observe("A", Some(&locked(0, 1)), 4);
        assert_eq!(c[0].state, AlarmState::Cleared);
        assert_eq!(b.active(), 0);
        assert_eq!(b.list().len(), 1);
    }

    #[test]
    fn wp_sw2_events_offline_ack_and_below_min() {
        let mut b = AlarmBook::default();
        let c = b.event("B", "holdover", "los", true, 1);
        assert_eq!(c.len(), 1);
        let id = c[0].id;
        assert_eq!(
            b.acknowledge(id, 2).unwrap().state,
            AlarmState::Acknowledged
        );
        let c = b.event("B", "locked", "reanchor", false, 3);
        assert_eq!(c.len(), 2, "{c:?}");
        let re = c.iter().find(|a| a.code == "reanchor").unwrap().id;
        assert_eq!(b.acknowledge(re, 4).unwrap().state, AlarmState::Cleared);
        assert_eq!(b.observe("C", None, 5)[0].code, "unit_offline");
        let mut low = locked(0, 0);
        low.phase = Some((1, 50, 0, 1));
        let c = b.observe("C", Some(&low), 6);
        assert!(c.iter().any(|a| a.code == "delay_below_min"));
        assert!(
            c.iter()
                .any(|a| a.code == "unit_offline" && a.state == AlarmState::Cleared)
        );
        assert!(b.acknowledge(9999, 7).is_none());
    }
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects, clippy::indexing_slicing)]
mod tests_wp_fixs1 {
    use super::tests_wp_sw2::locked;
    use super::*;

    fn lost_one(b: &AlarmBook) -> Alarm {
        let on: Vec<Alarm> = b
            .list()
            .into_iter()
            .filter(|a| a.code == "link_lost" && a.state != AlarmState::Cleared)
            .collect();
        assert_eq!(on.len(), 1, "{on:?}");
        on[0].clone()
    }

    #[test]
    fn wp_fixs1_one_loss_is_one_alarm_counted_once() {
        // SHOTS1 finding 2: an 0xE1 and the status reads during one loss.
        let mut b = AlarmBook::default();
        assert!(b.observe("B", Some(&locked(0, 0)), 1).is_empty());
        let c = b.event("B", "holdover", "los", true, 2);
        assert_eq!(c.len(), 1);
        assert_eq!(
            c[0].message,
            "B: the audio link was lost (no signal at the input)"
        );
        let mut hold = locked(0, 1);
        hold.state = Some("holdover".into());
        for t in 3..8 {
            let c = b.observe("B", Some(&hold), t);
            assert!(c.is_empty(), "no new alarm at a read: {c:?}");
        }
        let a = lost_one(&b);
        assert_eq!(a.count, 1, "Losses is 1");
        assert_eq!(a.message, "B: the audio link is in holdover");
        assert_eq!(b.active(), 1, "no counters alarm: CRC-16 errors are 0");
        // Locked again; a second loss is a new alarm, counted once too.
        let c = b.observe("B", Some(&locked(0, 1)), 8);
        assert_eq!((c.len(), c[0].state), (1, AlarmState::Cleared));
        let mut again = locked(0, 2);
        again.state = Some("searching".into());
        b.observe("B", Some(&again), 9);
        b.event("B", "searching", "comma_lost", true, 9);
        b.observe("B", Some(&again), 10);
        let a = lost_one(&b);
        assert_eq!(
            (a.count, a.message.as_str()),
            (1, "B: the audio link is searching")
        );
        // Two more losses before it locks: the count follows Losses since.
        let mut more = locked(0, 4);
        more.state = Some("holdover".into());
        b.observe("B", Some(&more), 11);
        assert_eq!(lost_one(&b).count, 3);
    }

    #[test]
    fn wp_fixs1_errors_alarm_only_when_crc16_errors_rise() {
        let mut b = AlarmBook::default();
        b.observe("A", Some(&locked(100, 0)), 1);
        // Losses alone, or a few errors: nothing.
        assert!(
            b.observe("A", Some(&locked(100, 3)), 2)
                .iter()
                .all(|a| a.code != "link_errors")
        );
        assert!(b.observe("A", Some(&locked(110, 3)), 3).is_empty());
        let c = b.observe("A", Some(&locked(110 + ERROR_STEP, 3)), 4);
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].code, "link_errors");
        assert_eq!(
            c[0].message,
            "A: 16 CRC-16 errors on the audio link since the last read"
        );
        // Still rising: the same alarm, not counted as another.
        assert!(
            b.observe("A", Some(&locked(110 + 3 * ERROR_STEP, 3)), 5)
                .is_empty()
        );
        assert_eq!(b.list()[0].count, 1);
        // A reset counter is no growth.
        let c = b.observe("A", Some(&locked(0, 0)), 6);
        assert_eq!((c.len(), c[0].state), (1, AlarmState::Cleared));
    }
}
