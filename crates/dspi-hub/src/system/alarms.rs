//! Link alarms (WP SW2, SYS2 7, PRO1 M3's shape): raised from what the hub
//! reads (`system.status`) and from the units' 0xE1 events as they arrive,
//! acknowledged by a person, cleared when the condition goes. Pure state; the
//! caller says what it saw and when.
//!
//! | Code | Raised | Cleared |
//! |---|---|---|
//! | `link_lost` | a unit not locked (searching, holdover, off), or an 0xE1 with a loss reason | locked again |
//! | `unit_offline` | a unit of the model not attached | attached |
//! | `link_errors` | a unit's bank errors (`crc16_errors`) grew by [`ERROR_STEP`] or more since the last read, or its `loss_count` grew | a read with no growth |
//! | `reanchor` | an 0xE1 `REANCHOR` (a short silence) | acknowledged |
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
    /// The counters at the last read, by subject: bank errors, losses.
    counters: BTreeMap<String, (u32, u32)>,
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
        let key = (code.to_owned(), subject.to_owned());
        if let Some(a) = self.active.get_mut(&key) {
            a.count = a.count.saturating_add(1);
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
        let change = if on {
            self.raise(code, subject, msg, now)
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
        if let Some(state) = &o.state {
            let locked = state == "locked";
            self.set(
                !locked,
                "link_lost",
                subject,
                format!("{subject}: the audio link is {state}"),
                now,
                &mut out,
            );
        }
        if let (Some(errors), Some(losses)) = (o.crc16_errors, o.loss_count) {
            let grew = match self.counters.insert(subject.to_owned(), (errors, losses)) {
                // A reset counter (lower now) is no growth.
                Some((e0, l0)) => errors.saturating_sub(e0) >= ERROR_STEP || losses > l0,
                None => false,
            };
            self.set(
                grew,
                "link_errors",
                subject,
                format!("{subject}: bank errors {errors}, losses {losses}"),
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
            out.extend(self.raise(
                "link_lost",
                subject,
                format!("{subject}: the audio link was lost ({reason})"),
                now,
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

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects, clippy::indexing_slicing)]
mod tests_wp_sw2 {
    use super::*;

    fn locked(errors: u32, losses: u32) -> Observed {
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
        assert_eq!(codes, ["link_lost", "link_errors"]);
        assert_eq!(c[0].severity, "critical");
        // Again: counted, not new.
        assert!(
            b.observe("A", Some(&lost), 3)
                .iter()
                .all(|a| a.code != "link_lost")
        );
        assert_eq!(b.active(), 1, "no growth since: link_errors cleared");
        let c = b.observe("A", Some(&locked(0, 1)), 4);
        assert_eq!(c[0].state, AlarmState::Cleared);
        assert_eq!(b.active(), 0);
        assert_eq!(b.list().len(), 2);
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
