//! Link alarms (WP SW2, SYS2 7, PRO1 M3's shape): raised from what the hub
//! reads (`system.status`) and from the units' 0xE1 events as they arrive,
//! acknowledged by a person, cleared when the condition goes. Pure state; the
//! caller says what it saw and when.
//!
//! | Code | Raised | Cleared |
//! |---|---|---|
//! | `link_lost` | a unit not locked (searching, holdover, off), or an 0xE1 with a loss reason; its count is the unit's losses since it was raised, and a new loss raises it again (acknowledged or not) | locked again |
//! | `unit_offline` | a unit of the model not attached | attached |
//! | `link_errors` | [`ERROR_STEP`] or more CRC-16 errors (`crc16_errors`) within any [`ERROR_WINDOW_S`] of the hub's time; the growth between two reads is spread evenly over the time between them, so the rate, not the pollers, decides (a loss is `link_lost`'s) | [`ERROR_WINDOW_S`] with no new error |
//! | `reanchor` | an 0xE1 `REANCHOR` (a short silence) | acknowledged |
//! | `system_muted` | the sender's system mute set (by `system.mute`, a scene or another host), seen at a read or a write | cleared |
//! | `delay_below_min` | `EXT_LINK_PHASE` flag `D_BELOW_MIN` | the flag gone |

use std::collections::BTreeMap;

use serde::Serialize;

use super::align::Observed;

/// CRC-16 errors within [`ERROR_WINDOW_S`] that raise `link_errors`.
pub const ERROR_STEP: u32 = 16;
// -- fixup fs1 (F5) --
/// The window of `link_errors`, seconds of the hub's time; also how long
/// without a new error clears it.
pub const ERROR_WINDOW_S: u64 = 60;
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
    /// The loss counter at the last read, by subject.
    losses: BTreeMap<String, u32>,
    // -- WP FIXS1 --
    /// The losses before a subject's `link_lost` was raised, so its count
    /// follows the unit's loss counter however often it is seen. `None`:
    /// raised by an 0xE1, the next read seeds it (fixup fs1 F1).
    loss_base: BTreeMap<String, Option<u32>>,
    // -- fixup fs1 (F5) --
    /// CRC-16 errors over the hub's time, by subject.
    crc: BTreeMap<String, CrcRate>,
}

// -- fixup fs1 (F5) --
/// One unit's CRC-16 errors over time: what each read added, spread evenly
/// over the time since the read before.
#[derive(Debug, Default)]
struct CrcRate {
    /// The counter and the hub's time at the last read.
    last: Option<(u32, u64)>,
    /// `(from, to, errors)`: errors that arrived in `(from, to]` (at `to`
    /// when the two are equal). Only what a later window can still reach.
    spans: Vec<(u64, u64, u32)>,
    /// The time of the newest error.
    last_error: Option<u64>,
}

impl CrcRate {
    /// The errors in the window `(start, start + ERROR_WINDOW_S]`.
    fn in_window(&self, start: u64) -> f64 {
        let end = start.saturating_add(ERROR_WINDOW_S);
        self.spans
            .iter()
            .map(|&(from, to, n)| {
                if to <= from {
                    return if start < to && to <= end {
                        f64::from(n)
                    } else {
                        0.0
                    };
                }
                let overlap = end.min(to).saturating_sub(start.max(from));
                f64::from(n) * overlap as f64 / to.saturating_sub(from) as f64
            })
            .sum()
    }

    /// A read of the counter at `now`: the most errors in any window that
    /// ends between the last read and this one.
    fn read(&mut self, count: u32, now: u64) -> f64 {
        let Some((c0, t0)) = self.last.replace((count, now)) else {
            return 0.0;
        };
        // A reset counter (a reboot, a stats reset): a new base, no errors.
        let added = if count >= c0 {
            count.saturating_sub(c0)
        } else {
            0
        };
        let from = t0.min(now);
        if added > 0 {
            self.spans.push((from, now, added));
            self.last_error = Some(now);
        }
        // The windows not yet looked at: those ending after the last read
        // (or at it, for errors that came in the same second), started at
        // each place the error rate changes.
        let lo = from.saturating_sub(ERROR_WINDOW_S);
        let hi = now.saturating_sub(ERROR_WINDOW_S);
        let mut starts = Vec::new();
        if hi > lo || added > 0 {
            starts.push(hi);
        }
        for &(a, b, _) in &self.spans {
            for t in [
                a,
                b,
                a.saturating_sub(ERROR_WINDOW_S),
                b.saturating_sub(ERROR_WINDOW_S),
            ] {
                if lo < t && t <= hi {
                    starts.push(t);
                }
            }
        }
        let peak = starts
            .into_iter()
            .map(|t| self.in_window(t))
            .fold(0.0, f64::max);
        // What a later window (ending after now) can still reach.
        self.spans.retain(|&(_, to, _)| to > hi);
        peak
    }

    /// No new error for a whole window.
    fn quiet(&self, now: u64) -> bool {
        self.last_error
            .is_none_or(|t| now.saturating_sub(t) >= ERROR_WINDOW_S)
    }
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
        let before = match o.loss_count {
            Some(l) => self.losses.insert(subject.to_owned(), l),
            None => self.losses.get(subject).copied(),
        };
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
                // Raised by a read: the losses since the read before. Raised
                // by an 0xE1 (fixup fs1 F1): that event is the one loss, the
                // read before may predate an earlier loss already cleared.
                let one_before = losses.saturating_sub(1);
                let base = self
                    .loss_base
                    .entry(subject.to_owned())
                    .or_insert_with(|| Some(before.map_or(one_before, |l0| l0.min(one_before))))
                    .get_or_insert(one_before);
                let count = losses.saturating_sub(*base).max(1);
                if let Some(a) = self
                    .active
                    .get_mut(&("link_lost".to_owned(), subject.to_owned()))
                    && a.count != count
                {
                    // -- fixup fs1 (F4) -- a new loss is raised again,
                    // acknowledged or not, and said.
                    let rose = count > a.count;
                    a.count = count;
                    if rose {
                        a.state = AlarmState::Raised;
                        out.push(a.clone());
                    }
                }
            }
        }
        // -- fixup fs1 (F5) -- a rate over the hub's time, with hysteresis.
        if let Some(errors) = o.crc16_errors {
            let rate = self.crc.entry(subject.to_owned()).or_default();
            let peak = rate.read(errors, now);
            let quiet = rate.quiet(now);
            if peak + 1e-9 >= f64::from(ERROR_STEP) {
                let n = peak.round();
                self.set(
                    true,
                    "link_errors",
                    subject,
                    format!("{subject}: {n} CRC-16 errors on the audio link within a minute"),
                    now,
                    &mut out,
                );
            } else if quiet {
                out.extend(self.clear("link_errors", subject, now));
            }
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
            let raised = self.raise_counted(
                "link_lost",
                subject,
                format!(
                    "{subject}: the audio link was lost ({})",
                    reason_words(reason)
                ),
                now,
                false,
            );
            if raised.is_some() {
                // -- fixup fs1 (F1) -- the next read seeds its base.
                self.loss_base.insert(subject.to_owned(), None);
            }
            out.extend(raised);
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
            "A: 26 CRC-16 errors on the audio link within a minute"
        );
        // Still rising: the same alarm, not counted as another.
        assert!(
            b.observe("A", Some(&locked(110 + 3 * ERROR_STEP, 3)), 5)
                .is_empty()
        );
        assert_eq!(b.list()[0].count, 1);
        // A reset counter is no growth; quiet for a minute clears it.
        assert!(b.observe("A", Some(&locked(0, 0)), 6).is_empty());
        assert!(b.observe("A", Some(&locked(0, 0)), 64).is_empty());
        let c = b.observe("A", Some(&locked(0, 0)), 65);
        assert_eq!((c.len(), c[0].state), (1, AlarmState::Cleared));
    }
}

// -- fixup fs1 --
#[cfg(test)]
#[allow(clippy::arithmetic_side_effects, clippy::indexing_slicing)]
mod tests_fixup_fs1 {
    use super::tests_wp_sw2::locked;
    use super::*;

    fn on(b: &AlarmBook, code: &str) -> Vec<Alarm> {
        b.list()
            .into_iter()
            .filter(|a| a.code == code && a.state != AlarmState::Cleared)
            .collect()
    }

    fn held(errors: u32, losses: u32) -> Observed {
        let mut o = locked(errors, losses);
        o.state = Some("holdover".into());
        o
    }

    /// Review fs1's probe (i), kept: two losses with a relock between them,
    /// both between two status reads, seen only through 0xE1: the second
    /// alarm counts 1, not 2.
    #[test]
    fn probe_fs1_event_raised_alarm_after_a_quick_relock_counts_only_its_own_loss() {
        let mut b = AlarmBook::default();
        assert!(b.observe("B", Some(&locked(0, 0)), 1).is_empty());
        assert_eq!(b.event("B", "holdover", "los", true, 2).len(), 1);
        assert_eq!(b.event("B", "locked", "locked", false, 3).len(), 1);
        assert_eq!(b.event("B", "holdover", "los", true, 4).len(), 1);
        let mut hold = locked(0, 2);
        hold.state = Some("holdover".into());
        assert!(b.observe("B", Some(&hold), 5).is_empty());
        let a = on(&b, "link_lost");
        assert_eq!(a.len(), 1);
        assert_eq!(
            a[0].count, 1,
            "the second alarm saw one loss, got {}",
            a[0].count
        );
        // A loss after that one still counts.
        let c = b.observe("B", Some(&held(0, 3)), 6);
        assert_eq!((c.len(), c[0].count), (1, 2));
    }

    #[test]
    fn fixup_fs1_a_new_loss_re_raises_an_acknowledged_alarm() {
        let mut b = AlarmBook::default();
        b.observe("B", Some(&locked(0, 0)), 1);
        let c = b.observe("B", Some(&held(0, 1)), 2);
        let id = c[0].id;
        b.acknowledge(id, 3).unwrap();
        // The same loss seen again: still acknowledged, nothing said.
        assert!(b.observe("B", Some(&held(0, 1)), 4).is_empty());
        assert_eq!(on(&b, "link_lost")[0].state, AlarmState::Acknowledged);
        // Relock missed by the reads, a second loss: raised again, said, counted.
        let c = b.observe("B", Some(&held(0, 2)), 5);
        assert_eq!(c.len(), 1, "{c:?}");
        assert_eq!(
            (c[0].id, c[0].count, c[0].state),
            (id, 2, AlarmState::Raised)
        );
        assert_eq!(on(&b, "link_lost")[0].state, AlarmState::Raised);
    }

    /// The review's flapping case: 20 errors on every other read, read each
    /// second. One alarm, on while they come; cleared a minute after the last.
    #[test]
    fn fixup_fs1_crc16_bursts_raise_once_and_clear_after_a_quiet_minute() {
        let mut b = AlarmBook::default();
        b.observe("A", Some(&locked(0, 0)), 1);
        let mut log = Vec::new();
        for (t, e) in [(2, 20), (3, 20), (4, 40), (5, 40), (6, 60), (7, 60)] {
            for a in b.observe("A", Some(&locked(e, 0)), t) {
                log.push((t, a.state));
            }
        }
        assert_eq!(log, [(2, AlarmState::Raised)]);
        assert!(b.observe("A", Some(&locked(60, 0)), 65).is_empty());
        let c = b.observe("A", Some(&locked(60, 0)), 66);
        assert_eq!((c.len(), c[0].state), (1, AlarmState::Cleared));
    }

    /// The same error stream, polled every second, every 10 s or by nobody
    /// for 59 s, raises the same alarm: the rate decides, not the pollers.
    #[test]
    fn fixup_fs1_crc16_alarm_does_not_depend_on_the_poll_rate() {
        // 15 errors a second for 100 s.
        for step in [1u64, 10, 59] {
            let mut b = AlarmBook::default();
            b.observe("A", Some(&locked(0, 0)), 1000);
            let mut raised = None;
            let mut t = 0;
            while t < 100 {
                t += step;
                let c = b.observe("A", Some(&locked(15 * t as u32, 0)), 1000 + t);
                if c.iter().any(|a| a.state == AlarmState::Raised) {
                    raised.get_or_insert(t);
                }
            }
            assert!(raised.is_some(), "polled every {step} s: no alarm");
            assert_eq!(on(&b, "link_errors").len(), 1);
        }
        // Below the rate: 15 errors spread over each minute never alarm.
        for step in [1u64, 30, 60] {
            let mut b = AlarmBook::default();
            b.observe("A", Some(&locked(0, 0)), 0);
            let mut t = 0;
            while t < 600 {
                t += step;
                let c = b.observe("A", Some(&locked((t / 4) as u32, 0)), t);
                assert!(c.is_empty(), "15/min polled every {step} s: {c:?}");
            }
        }
    }

    #[test]
    fn fixup_fs1_crc16_errors_over_a_long_gap_are_spread_over_it() {
        let mut b = AlarmBook::default();
        b.observe("A", Some(&locked(1500, 0)), 200);
        // 16 errors over four hours: no alarm.
        assert!(
            b.observe("A", Some(&locked(1516, 0)), 4 * 3600 + 200)
                .is_empty()
        );
        // 16 errors over 30 s: an alarm.
        let c = b.observe("A", Some(&locked(1532, 0)), 4 * 3600 + 230);
        assert_eq!((c.len(), c[0].code.as_str()), (1, "link_errors"));
        // 32 errors over 2 minutes: 16 in the minute, a window between the
        // reads: also an alarm (another unit).
        b.observe("C", Some(&locked(0, 0)), 0);
        let c = b.observe("C", Some(&locked(32, 0)), 120);
        assert_eq!(c.len(), 1);
        assert_eq!(
            c[0].message,
            "C: 16 CRC-16 errors on the audio link within a minute"
        );
    }
}
