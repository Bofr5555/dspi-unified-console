//! WP P3RO: the history's two P3-B items: a schedule run's rows that name no device
//! (a `group:<id>` row, the `""` of a truncated run) are not filed, and the rollout's
//! end on the bus writes nothing of its own (its `firmware` record comes as an `Event`).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Instant;

use super::*;
use crate::bus::TargetResult;

const S: &str = "P3RO-UNIT";

fn history(name: &str) -> History {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    // `ci/run.sh` points TMPDIR at ~/.cache/dspi-build/work, not tmpfs.
    let dir = std::env::temp_dir().join("wp-P3RO").join(format!(
        "{name}-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::SeqCst)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    History::open(dir, &HistoryConfig::default()).unwrap()
}

fn now() -> Now {
    Now {
        ms: 1_763_200_000_000,
        mono: Instant::now(),
    }
}

#[test]
fn wp_p3ro_schedule_rows_without_a_device_are_not_filed() {
    let mut h = history("rows");
    let row = |serial: &str, ok: bool, code: &str| TargetResult {
        serial: serial.into(),
        action: Some("0:preset.load".into()),
        ok,
        code: (!ok).then(|| code.into()),
        msg: None,
    };
    let w = h.on_message(
        &BusMessage::ScheduleRan {
            rule: "r1".into(),
            name: "Morning".into(),
            at: 0,
            by: None,
            results: vec![
                row(S, true, ""),
                row("group:g9", false, "not_found"),
                row("", false, "bad_request"),
            ],
        },
        now(),
    );
    assert_eq!(w.len(), 1, "{w:?}");
    assert_eq!(w[0].serial, S);
    assert_eq!(w[0].lines, ["0:preset.load: ok"]);
}

#[test]
fn wp_p3ro_a_rollout_end_writes_nothing_itself() {
    let mut h = history("rollout");
    let w = h.on_message(
        &BusMessage::RolloutEnded {
            serial: S.into(),
            ok: false,
            step: "find".into(),
            text: "no board".into(),
            firmware_version: "1.2.0".into(),
        },
        now(),
    );
    assert!(w.is_empty(), "{w:?}");
}
