//! WP TM12: the Update Database menu, driven by its keys, on a scripted fetch.
//!
//! This is its own test binary with one test because the menu reads and writes
//! the real config path, and setting `XDG_CONFIG_HOME` is only sound while no
//! other thread is about. The path is in the target directory, never a tmpfs.
#![cfg(target_os = "linux")]

use std::path::PathBuf;
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use dspi_session::Session;
use dspi_session::autoeq::{Fetch, FetchRequest, FetchResponse};
use dspi_transport::MockTransport;
use dspi_tui::live::{ConsoleScreens, Live};
use dspi_tui::perf::Performance;
use dspi_tui::shell::fixture;
use dspi_tui::theme::{ColorDepth, Glyphs};

const PROFILE: &str = "Preamp: -6.0 dB\nFilter 1: ON PK Fc 100 Hz Gain 3.0 dB Q 1.0\n";

/// A scripted GitHub. No process, no socket.
#[derive(Clone, Copy, PartialEq)]
enum GitHub {
    Whole,
    PartsMissing,
    RateLimited,
}

fn answer(status: u16, body: &str) -> FetchResponse {
    FetchResponse {
        status,
        body: body.as_bytes().to_vec(),
        ..Default::default()
    }
}

impl Fetch for GitHub {
    fn get(&self, r: &FetchRequest) -> Result<FetchResponse, String> {
        let dirs = |names: &[&str]| {
            let items: Vec<_> = names
                .iter()
                .map(|n| format!(r#"{{"name":"{n}","type":"dir"}}"#))
                .collect();
            format!("[{}]", items.join(","))
        };
        let url = r.url.as_str();
        Ok(if *self == GitHub::RateLimited {
            FetchResponse {
                retry_after_s: Some(120),
                ..answer(403, "")
            }
        } else if url.ends_with("contents/results/oratory1990") {
            answer(200, &dirs(&["t_over-ear"]))
        } else if url.ends_with("contents/results/oratory1990/t_over-ear") {
            answer(200, &dirs(&["Sony A", "Sony B"]))
        } else if *self == GitHub::PartsMissing && url.ends_with("contents/results/crinacle") {
            answer(502, "Bad Gateway")
        } else if url.contains("Sony%20A") {
            answer(200, PROFILE)
        } else if url.contains("Sony%20B") {
            if *self == GitHub::PartsMissing {
                return Err("timed out".to_string());
            }
            answer(200, PROFILE)
        } else {
            answer(404, "Not Found")
        })
    }
}

struct Ui {
    live: Live,
    session: Session,
}

impl Ui {
    fn frame(&mut self) -> String {
        let live = &mut self.live;
        dspi_tui::render_frame(120, 40, |area, buf| live.draw(area, buf))
    }

    fn key(&mut self, code: KeyCode) {
        self.live
            .handle_key(&mut self.session, KeyEvent::new(code, KeyModifiers::NONE));
    }

    /// Open the menu and start a rebuild on this script, then wait for it.
    fn rebuild(&mut self, script: GitHub) -> String {
        self.live.set_autoeq_fetch(move || Box::new(script));
        self.live.run_command(&mut self.session, "autoeq update");
        assert!(self.frame().contains("Rebuild from GitHub"));
        self.key(KeyCode::Enter);
        let confirm = self.frame();
        assert!(confirm.contains("Rebuild AutoEQ Database"), "{confirm}");
        // The confirm's default is Cancel.
        self.key(KeyCode::Left);
        self.key(KeyCode::Enter);
        let started = Instant::now();
        loop {
            self.live.tick(&mut self.session, None);
            let frame = self.frame();
            if !frame.contains("Updating Database") {
                return frame;
            }
            assert!(started.elapsed() < Duration::from_secs(20), "{frame}");
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

#[test]
fn tm12_update_rebuild_and_reset_on_a_scripted_fetch() {
    let config = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("tm12-config");
    let _ = std::fs::remove_dir_all(&config);
    std::fs::create_dir_all(&config).unwrap();
    // SAFETY: the only test in this binary, before it starts any thread.
    unsafe { std::env::set_var("XDG_CONFIG_HOME", &config) };
    let db = dspi_session::autoeq::user_db_path();
    assert!(db.starts_with(&config), "{}", db.display());

    let state = fixture::state();
    let mock = MockTransport::new().answering_everything(vec![0; 64]);
    let session = Session::new(Box::new(mock), state.caps.clone()).unwrap();
    let theme = dspi_tui::Theme::console(ColorDepth::TrueColor, Glyphs::Braille);
    let live = Live::new(
        state,
        theme,
        Performance::lite(),
        Box::new(ConsoleScreens::new()),
    );
    let mut ui = Ui { live, session };

    // Nothing to throw away yet, so Reset is not offered.
    ui.live.run_command(&mut ui.session, "autoeq update");
    let menu = ui.frame();
    assert!(menu.contains("Update AutoEQ Database"), "{menu}");
    assert!(!menu.contains("Reset to Built-in"), "{menu}");
    ui.key(KeyCode::Esc);

    // A rate limit changes nothing, and says why.
    let frame = ui.rebuild(GitHub::RateLimited);
    assert!(frame.contains("Rebuild failed"), "{frame}");
    assert!(frame.contains("rate limit"), "{frame}");
    assert!(!db.exists());

    // A clean rebuild.
    let frame = ui.rebuild(GitHub::Whole);
    assert!(frame.contains("Database rebuilt successfully"), "{frame}");
    let written = dspi_session::autoeq::load_user_database(&db).unwrap();
    assert_eq!(written.entries.len(), 2);

    // Parts missing: both counts stay on screen until dismissed.
    let frame = ui.rebuild(GitHub::PartsMissing);
    assert!(frame.contains("rebuilt with parts missing"), "{frame}");
    assert!(frame.contains("Entries: 1"), "{frame}");
    assert!(
        frame.contains("1 profile could not be downloaded"),
        "{frame}"
    );
    assert!(frame.contains("1 folder could not be listed"), "{frame}");
    ui.key(KeyCode::Enter);
    assert!(!ui.frame().contains("parts missing"));
    assert_eq!(
        dspi_session::autoeq::load_user_database(&db)
            .unwrap()
            .entries
            .len(),
        1
    );

    // Now there is a copy to throw away: the third button.
    ui.live.run_command(&mut ui.session, "autoeq update");
    let menu = ui.frame();
    assert!(menu.contains("Reset to Built-in"), "{menu}");
    ui.key(KeyCode::Right);
    ui.key(KeyCode::Right);
    ui.key(KeyCode::Enter);
    let frame = ui.frame();
    assert!(frame.contains("Reset to built-in database"), "{frame}");
    assert!(!db.exists());
}
