//! The `real_*` tests: nothing happens unless `DSPI_REAL_HUB` is set.

use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

use dspi_link::socket::TcpConnector;
use dspi_link::{address, AuthState};

use crate::scenarios as sc;
use crate::target::{forget_pairing, run, Bench, Forgotten, Gate, Skip, Target, CLIENT_NAME, TEMP_CLIENT_NAME};

pub struct Config {
    pub host: String,
    pub port: u16,
    pub token_file: Option<PathBuf>,
    pub pin: Option<String>,
    pub allow_pair: bool,
    pub allow_lockout: bool,
    pub revoke: bool,
    pub no_board: bool,
    pub operator: bool,
    pub report: Option<PathBuf>,
}

impl Config {
    /// `Ok(None)`: no hub set, so nothing runs. Only exactly `1` enables a
    /// switch, so `=0` or a stray value never opens a gate.
    pub fn parse(get: impl Fn(&str) -> Option<String>) -> Result<Option<Config>, String> {
        let var = |name: &str| get(name).filter(|v| !v.trim().is_empty());
        let on = |name: &str| get(name).as_deref() == Some("1");
        let Some(hub) = var("DSPI_REAL_HUB") else { return Ok(None) };
        let (host, port) =
            address::parse_address(&hub).ok_or_else(|| format!("DSPI_REAL_HUB: not an address: {hub}"))?;
        Ok(Some(Config {
            host,
            port,
            token_file: var("DSPI_REAL_HUB_TOKEN_FILE").map(PathBuf::from),
            pin: var("DSPI_REAL_HUB_PIN"),
            allow_pair: on("DSPI_REAL_HUB_ALLOW_PAIR"),
            allow_lockout: on("DSPI_REAL_HUB_ALLOW_LOCKOUT"),
            revoke: on("DSPI_REAL_HUB_REVOKE"),
            no_board: on("DSPI_REAL_HUB_NO_BOARD"),
            operator: on("DSPI_REAL_HUB_OPERATOR"),
            report: var("DSPI_REAL_HUB_REPORT").map(PathBuf::from),
        }))
    }

    fn token(&self) -> Option<String> {
        let text = fs::read_to_string(self.token_file.as_ref()?).ok()?;
        Some(text.trim().to_owned()).filter(|t| !t.is_empty())
    }

    pub fn target(self) -> Target {
        let token = self.token();
        Target {
            url: address::ws_url(&self.host, self.port, None),
            host: self.host,
            port: self.port,
            scale: 1,
            slack: Duration::from_secs(3),
            token,
            pin: self.pin,
            allow_pair: self.allow_pair,
            allow_lockout: self.allow_lockout,
            no_board: self.no_board,
            operator: self.operator,
            report: self.report,
            bench: Box::new(RealBench),
        }
    }
}

/// A person at the bench, told what to do on stderr.
struct RealBench;

impl Bench for RealBench {
    fn board(&self, present: bool) {
        let what = if present { "POWER THE BOARD BACK ON" } else { "CUT THE BOARD'S POWER" };
        eprintln!("\nreal_hub: >>> {what} now (2 minutes) <<<\n");
    }
}

/// One real-hub test at a time, also after one has panicked.
static ONE_AT_A_TIME: Mutex<()> = Mutex::new(());

fn hub(test: &str) -> Option<(MutexGuard<'static, ()>, Config)> {
    let config = Config::parse(|name| std::env::var(name).ok()).unwrap_or_else(|e| panic!("{e}"));
    let Some(config) = config else {
        eprintln!("SKIP {test}: DSPI_REAL_HUB is not set");
        return None;
    };
    Some((ONE_AT_A_TIME.lock().unwrap_or_else(|p| p.into_inner()), config))
}

macro_rules! real_test {
    ($name:ident, $run:expr) => {
        #[test]
        fn $name() {
            let Some((_one, config)) = hub(stringify!($name)) else { return };
            let scenario: fn(&Target) -> crate::target::Outcome = $run;
            if let Err(Skip(why)) = scenario(&config.target()) {
                eprintln!("SKIP {}: {why}", stringify!($name));
            }
        }
    };
}

real_test!(real_smoke, sc::smoke);
real_test!(real_1_idle, sc::idle_session);
real_test!(real_2_wrong_pin_keeps_token, sc::wrong_pin_keeps_token);
real_test!(real_3_dark_5s_polling, |t| sc::dark_network(t, 5.0, true));
real_test!(real_3_dark_20s_polling, |t| sc::dark_network(t, 20.0, true));
real_test!(real_3_dark_32s_polling, |t| sc::dark_network(t, 32.0, true));
real_test!(real_3_dark_60s_polling, |t| sc::dark_network(t, 60.0, true));
real_test!(real_3_dark_60s_idle, |t| sc::dark_network(t, 60.0, false));
real_test!(real_4_slots_full, sc::slots_full);
real_test!(real_6_no_board_pairing, sc::no_board_pairing);
real_test!(real_6_board_flap, sc::board_flap);
real_test!(real_7_attach_gets, sc::attach_gets);
real_test!(real_8_pair_cut_at_ok, sc::pair_cut_at_ok);
real_test!(real_8_pair_late_reply, sc::pair_late_reply);

/// Pair once as `viewer` and keep the token for the run. Does nothing when the
/// token file already holds a token.
#[test]
fn real_setup_pair() {
    let Some((_one, config)) = hub("real_setup_pair") else { return };
    let Some(file) = config.token_file.clone() else {
        eprintln!("SKIP real_setup_pair: needs DSPI_REAL_HUB_TOKEN_FILE");
        return;
    };
    let t = config.target();
    let _ = run(&t, "setup_pair", &[Gate::Pair], |t, _rec| {
        if t.token.is_some() {
            return Err(Skip("the token file already has a token".into()));
        }
        let (c, state) = t.dial(&t.url, None, None).expect("connect");
        if state == AuthState::Ready {
            c.close();
            return Err(Skip("an open hub needs no token".into()));
        }
        let token = c.pair(t.pin.as_deref().expect("Gate::Pair"), "viewer").expect("pairing");
        c.close();
        write_private(&file, &token);
        Ok(())
    });
}

fn write_private(file: &PathBuf, token: &str) {
    let mut open = fs::OpenOptions::new();
    open.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut open, 0o600);
    let mut f = open.open(file).unwrap_or_else(|e| panic!("{}: {e}", file.display()));
    writeln!(f, "{token}").unwrap_or_else(|e| panic!("{}: {e}", file.display()));
}

/// Undo [`real_setup_pair`]: revoke the records named after the harness, and
/// delete the token file. Only with `DSPI_REAL_HUB_REVOKE=1`.
#[test]
fn real_teardown_revoke() {
    let Some((_one, config)) = hub("real_teardown_revoke") else { return };
    if !config.revoke {
        eprintln!("SKIP real_teardown_revoke: needs DSPI_REAL_HUB_REVOKE=1");
        return;
    }
    let Some(file) = config.token_file.clone() else {
        eprintln!("SKIP real_teardown_revoke: needs DSPI_REAL_HUB_TOKEN_FILE");
        return;
    };
    let t = config.target();
    let Some(token) = t.token.clone() else {
        eprintln!("SKIP real_teardown_revoke: no token in the file");
        return;
    };
    match forget_pairing(&TcpConnector::default(), &t.url, &token, &[TEMP_CLIENT_NAME, CLIENT_NAME]) {
        Forgotten::Revoked(n) => {
            eprintln!("real_hub: revoked {n} harness client record(s)");
            fs::remove_file(&file).unwrap_or_else(|e| panic!("{}: {e}", file.display()));
            // Once only: each try with a revoked token counts as a failed attempt
            let (c, state) = t.dial(&t.url, Some(token), None).expect("connect");
            c.close();
            assert!(matches!(state, AuthState::TokenRefused { .. }), "the token still works: {state:?}");
        }
        Forgotten::Refused(why) => panic!("not revoked ({why}): remove it in the hub's own UI, then delete the file"),
    }
}
