//! `dspi autoeq rebuild`, `update`, `import` and `reset` (WP TM12).
//!
//! The same operations as the interface's Update Database menu, for a script or
//! a machine with no terminal to draw on. None of them needs a device.

use std::path::Path;
use std::time::Duration;

use dspi_session::autoeq::{self, AutoEqError, Fetch, RebuildSummary};
use dspi_tui::autoeq_fetch::HttpFetch;
use dspi_tui::screens::autoeq::rebuild_report;

use crate::exit;

/// How often a running rebuild is looked at.
const POLL: Duration = Duration::from_millis(100);

/// Run one of these subcommands. `None`: it is someone else's.
pub fn run(args: &[&str], flags: &[&str]) -> Option<u8> {
    let db = autoeq::user_db_path();
    Some(match args.first().copied()? {
        "rebuild" => rebuild(Box::new(HttpFetch::new()), &db, flags.contains(&"--yes")),
        "update" => match args.get(1) {
            Some(url) => update(&HttpFetch::new(), url, &db),
            None => {
                eprintln!("dspi: autoeq update needs the address of a database");
                exit::USAGE
            }
        },
        "import" => match args.get(1) {
            Some(file) => import(Path::new(file), &db),
            None => {
                eprintln!("dspi: autoeq import needs a file");
                exit::USAGE
            }
        },
        "reset" => reset(),
        _ => return None,
    })
}

/// No network until someone has read what it does and said yes.
fn rebuild(fetch: Box<dyn Fetch>, db: &Path, yes: bool) -> u8 {
    if !yes {
        eprintln!("{}", autoeq::rebuild_warning_for(db));
        eprintln!("\nRun it again with --yes to go ahead.");
        return exit::USAGE;
    }
    let mut handle = autoeq::rebuild_to(fetch, db.to_path_buf());
    let mut shown = String::new();
    let done = loop {
        let progress = handle.poll();
        if progress.status != shown {
            shown.clone_from(&progress.status);
            if progress.done.is_none() {
                eprintln!("{shown}");
            }
        }
        if let Some(done) = progress.done.clone() {
            break done;
        }
        std::thread::sleep(POLL);
    };
    match done {
        Ok(summary) => finished(&summary, db),
        Err(e) => failed(&e),
    }
}

/// A rebuild with parts missing did write a database, and still exits non-zero:
/// a script should not take it for a whole one.
fn finished(summary: &RebuildSummary, db: &Path) -> u8 {
    println!("Database rebuilt: {}", db.display());
    for line in rebuild_report(summary) {
        println!("{line}");
    }
    if summary.failed_profiles > 0 || summary.failed_listings > 0 {
        exit::TRANSPORT
    } else {
        exit::OK
    }
}

fn update(fetch: &dyn Fetch, url: &str, db: &Path) -> u8 {
    match autoeq::update_from_url(fetch, url, db) {
        Ok(new) => {
            println!("Database updated. Entries: {}", new.entries.len());
            exit::OK
        }
        Err(e) => failed(&e),
    }
}

fn import(file: &Path, db: &Path) -> u8 {
    let data = match std::fs::read(file) {
        Ok(data) => data,
        Err(e) => {
            eprintln!("dspi: could not read {}: {e}", file.display());
            return exit::USAGE;
        }
    };
    match autoeq::import_bytes(&data, db) {
        Ok(new) => {
            println!("Database imported. Entries: {}", new.entries.len());
            exit::OK
        }
        Err(e) => failed(&e),
    }
}

fn reset() -> u8 {
    match autoeq::reset_to_builtin() {
        Ok(n) => {
            println!("Reset to built-in database. Entries: {n}");
            exit::OK
        }
        Err(e) => failed(&e),
    }
}

/// Could not get it, as against could not use it: scripts branch on this.
fn failed(e: &AutoEqError) -> u8 {
    eprintln!("dspi: {e}");
    match e {
        AutoEqError::Fetch(_)
        | AutoEqError::NoProfiles { .. }
        | AutoEqError::TooManyFailures { .. } => exit::TRANSPORT,
        _ => exit::USAGE,
    }
}

#[cfg(test)]
mod tests_wp_tm12 {
    use super::*;
    use dspi_session::autoeq::{FetchRequest, FetchResponse};
    use std::path::PathBuf;

    const PROFILE: &str = "Preamp: -6.0 dB\nFilter 1: ON PK Fc 100 Hz Gain 3.0 dB Q 1.0\n";

    /// A scripted GitHub: one source, two headphones, and whatever the test
    /// breaks. Everything else is a 404, as it would be there.
    #[derive(Clone, Default)]
    struct GitHub {
        /// A second folder, which cannot be listed.
        broken_listing: bool,
        /// One of the two headphones, which cannot be downloaded.
        broken_profile: bool,
    }

    fn answer(status: u16, body: &str) -> Result<FetchResponse, String> {
        Ok(FetchResponse {
            status,
            body: body.as_bytes().to_vec(),
            ..Default::default()
        })
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
            if url.ends_with("contents/results/oratory1990") {
                answer(200, &dirs(&["t_over-ear"]))
            } else if url.ends_with("contents/results/oratory1990/t_over-ear") {
                answer(200, &dirs(&["Sony A", "Sony B"]))
            } else if self.broken_listing && url.ends_with("contents/results/crinacle") {
                answer(502, "Bad Gateway")
            } else if url.contains("Sony%20A") {
                answer(200, PROFILE)
            } else if url.contains("Sony%20B") {
                if self.broken_profile {
                    Err("timed out".to_string())
                } else {
                    answer(200, PROFILE)
                }
            } else {
                answer(404, "Not Found")
            }
        }
    }

    /// A scratch directory in the target directory: never a tmpfs.
    fn dir(name: &str) -> PathBuf {
        let dir = std::env::current_exe()
            .unwrap()
            .with_file_name("test-scratch")
            .join(format!("tm12-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn tm12_rebuild_says_no_until_told_yes() {
        struct Never;
        impl Fetch for Never {
            fn get(&self, _: &FetchRequest) -> Result<FetchResponse, String> {
                panic!("nothing should be fetched");
            }
        }
        let db = dir("no").join("db.json");
        assert_eq!(rebuild(Box::new(Never), &db, false), exit::USAGE);
        assert!(!db.exists());
    }

    #[test]
    fn tm12_rebuild_writes_the_database() {
        let db = dir("whole").join("db.json");
        assert_eq!(rebuild(Box::new(GitHub::default()), &db, true), exit::OK);
        assert_eq!(autoeq::load_user_database(&db).unwrap().entries.len(), 2);
    }

    #[test]
    fn tm12_a_rebuild_with_parts_missing_writes_and_does_not_exit_ok() {
        let db = dir("parts").join("db.json");
        let fetch = GitHub {
            broken_listing: true,
            broken_profile: true,
        };
        assert_eq!(rebuild(Box::new(fetch), &db, true), exit::TRANSPORT);
        assert_eq!(autoeq::load_user_database(&db).unwrap().entries.len(), 1);
    }

    #[test]
    fn tm12_a_failed_rebuild_keeps_the_database() {
        struct Offline;
        impl Fetch for Offline {
            fn get(&self, _: &FetchRequest) -> Result<FetchResponse, String> {
                Err("could not reach the server".to_string())
            }
        }
        let db = dir("offline").join("db.json");
        std::fs::write(&db, "the old database").unwrap();
        assert_eq!(rebuild(Box::new(Offline), &db, true), exit::TRANSPORT);
        assert_eq!(std::fs::read_to_string(&db).unwrap(), "the old database");
    }

    #[test]
    fn tm12_update_replaces_the_database_only_with_a_database() {
        struct Serves(&'static str);
        impl Fetch for Serves {
            fn get(&self, _: &FetchRequest) -> Result<FetchResponse, String> {
                answer(200, self.0)
            }
        }
        let db = dir("update").join("db.json");
        let good = r#"{"version":1,"entries":[{"id":"a"}]}"#;
        assert_eq!(update(&Serves(good), "https://x/db", &db), exit::OK);
        assert_eq!(std::fs::read_to_string(&db).unwrap(), good);
        assert_eq!(
            update(&Serves("not json"), "https://x/db", &db),
            exit::USAGE
        );
        assert_eq!(std::fs::read_to_string(&db).unwrap(), good);
    }

    #[test]
    fn tm12_a_failed_update_is_a_transport_failure() {
        struct Gone;
        impl Fetch for Gone {
            fn get(&self, _: &FetchRequest) -> Result<FetchResponse, String> {
                answer(500, "oops")
            }
        }
        let db = dir("gone").join("db.json");
        assert_eq!(update(&Gone, "https://x/db", &db), exit::TRANSPORT);
        assert!(!db.exists());
    }

    #[test]
    fn tm12_other_subcommands_are_left_alone() {
        assert_eq!(run(&[], &[]), None);
        assert_eq!(run(&["search", "hd"], &[]), None);
    }
}
