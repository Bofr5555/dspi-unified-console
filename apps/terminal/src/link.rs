//! `dspi link scan`, and asking a person which hub `--hub` means.
//!
//! Owned by WP TM9. Looking at the network itself is in
//! `dspi_tui::shell::connect`, which the reconnect uses too.

use std::sync::Arc;
use std::time::Duration;

use dspi_link::discovery::{BrowseSource, BrowsedHub};
use dspi_tui::shell::connect::{self, Prompt, printable};

use crate::exit;

/// The longest `--timeout`: past this, something else is wrong.
const MAX_TIMEOUT_SECS: f64 = 600.0;

/// The person at this terminal, if there is one.
pub struct Person;

impl Prompt for Person {
    fn pin(&self, hub: &str, pin_source: &str) -> Option<String> {
        crate::prompt_pin(hub, pin_source)
    }

    fn pick(&self, choices: &[String]) -> Option<usize> {
        use std::io::IsTerminal;
        if !std::io::stdin().is_terminal() {
            return None;
        }
        eprintln!("Several hubs found:");
        for (i, name) in choices.iter().enumerate() {
            eprintln!("  {}. {name}", i.saturating_add(1));
        }
        eprint!("Which one? ");
        parse_choice(&crate::read_stdin_line()?, choices.len())
    }
}

/// A person's answer: a number from 1 to `count`.
fn parse_choice(line: &str, count: usize) -> Option<usize> {
    let number: usize = line.trim().parse().ok()?;
    let index = number.checked_sub(1)?;
    (index < count).then_some(index)
}

/// `dspi link ...`; `flags` starts at the word `link`.
pub fn run(flags: &[&str], json: bool) -> u8 {
    let window = match parse_args(flags.get(1..).unwrap_or_default()) {
        Ok(window) => window,
        Err(e) => {
            eprintln!("dspi: {e}");
            return exit::USAGE;
        }
    };
    match connect::browse_source() {
        Ok(source) => scan(source, window, json),
        Err(e) => crate::fail_open(e),
    }
}

/// The arguments of `link`: how long `scan` should listen.
fn parse_args(args: &[&str]) -> Result<Duration, String> {
    let mut scan = false;
    let mut window = connect::SCAN_WINDOW;
    let mut it = args.iter().copied();
    while let Some(arg) = it.next() {
        match arg {
            "scan" if !scan => scan = true,
            "--timeout" => {
                window = it
                    .next()
                    .and_then(|s| s.parse::<f64>().ok())
                    .filter(|s| *s > 0.0 && *s <= MAX_TIMEOUT_SECS)
                    .map(Duration::from_secs_f64)
                    .ok_or("--timeout needs a number of seconds, at most 600")?;
            }
            // The flags every command may carry.
            flag if flag.starts_with("--") => {}
            other => return Err(format!("`link {other}`: the command is `link scan`")),
        }
    }
    if scan {
        Ok(window)
    } else {
        Err("link what? the command is `link scan`".into())
    }
}

/// As `dspi list` does: nothing there is exit code 2, and `[]` for a script.
fn scan(source: Arc<dyn BrowseSource>, window: Duration, json: bool) -> u8 {
    let hubs = match connect::scan_hubs(source, window, |_| false) {
        Ok(hubs) => hubs,
        Err(e) => return crate::fail_open(e),
    };
    if json {
        match serde_json::to_string_pretty(&hubs) {
            Ok(text) => println!("{text}"),
            Err(e) => {
                eprintln!("dspi: {e}");
                return exit::TRANSPORT;
            }
        }
    } else if hubs.is_empty() {
        eprintln!("No DSPi Link hub found.");
    } else {
        print!("{}", table(&hubs));
    }
    if hubs.is_empty() {
        exit::NO_DEVICE
    } else {
        exit::OK
    }
}

/// One line per hub. What it says came off the network, so it is made safe to
/// print. The address is what `--link` takes, and the name what `--hub` takes.
fn table(hubs: &[BrowsedHub]) -> String {
    let row = |h: &BrowsedHub| {
        [
            printable(&h.name),
            connect::hub_address(h),
            printable(&h.kind),
            printable(&h.auth),
            format!("{} device(s)", h.device_count),
            printable(&h.id),
        ]
    };
    let rows: Vec<[String; 6]> = hubs.iter().map(row).collect();
    let width = |column: usize| {
        rows.iter()
            .filter_map(|r| r.get(column))
            .map(|cell| cell.chars().count())
            .max()
            .unwrap_or(0)
    };
    let widths = [width(0), width(1), width(2), width(3), width(4)];
    let mut out = String::new();
    for [name, address, kind, auth, devices, id] in &rows {
        let [w0, w1, w2, w3, w4] = widths;
        out.push_str(&format!(
            "{name:<w0$}  {address:<w1$}  {kind:<w2$}  {auth:<w3$}  {devices:<w4$}  {id}\n"
        ));
    }
    out
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_tm9 {
    use super::*;
    use dspi_link::discovery::{FakeAdvertiser, HubAdvert};
    use std::net::IpAddr;

    const WINDOW: Duration = Duration::from_millis(150);

    fn advert(name: &str, id: &str) -> HubAdvert {
        HubAdvert {
            name: name.into(),
            port: 4848,
            hub_id: id.into(),
            kind: "console".into(),
            auth: "pin".into(),
            device_count: 2,
            serials: vec!["AAA".into(), "BBB".into()],
            web: false,
            path: None,
        }
    }

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    fn net() -> FakeAdvertiser {
        let net = FakeAdvertiser::new();
        net.announce(&advert("Studio", "aaaa"), &[ip("192.168.1.20")]);
        net.announce(&advert("Attic", "bbbb"), &[ip("fd00::2")]);
        net
    }

    fn heard(net: &FakeAdvertiser) -> Vec<BrowsedHub> {
        connect::scan_hubs(Arc::new(net.clone()), WINDOW, |_| false).unwrap()
    }

    #[test]
    fn wp_tm9_scan_json_is_the_hubs_as_the_c_abi_names_them() {
        let hubs = heard(&net());
        let doc: serde_json::Value =
            serde_json::from_str(&serde_json::to_string_pretty(&hubs).unwrap()).unwrap();
        let list = doc.as_array().unwrap();
        assert_eq!(list.len(), 2);
        // By name, and every field a script wants.
        assert_eq!(list[0]["name"], "Attic");
        assert_eq!(list[1]["name"], "Studio");
        assert_eq!(list[1]["id"], "aaaa");
        assert_eq!(list[1]["host"], "192.168.1.20");
        assert_eq!(list[1]["port"], 4848);
        assert_eq!(list[1]["deviceCount"], 2);
        assert_eq!(list[1]["serials"], serde_json::json!(["AAA", "BBB"]));
        assert_eq!(list[1]["url"], "ws://192.168.1.20:4848/dspi/v1");
        assert_eq!(list[1]["auth"], "pin");
        assert_eq!(list[1]["kind"], "console");
    }

    #[test]
    fn wp_tm9_scan_exit_codes_are_those_of_dspi_list() {
        assert_eq!(scan(Arc::new(net()), WINDOW, true), exit::OK);
        assert_eq!(scan(Arc::new(net()), WINDOW, false), exit::OK);
        let empty = Arc::new(FakeAdvertiser::new());
        assert_eq!(scan(empty.clone(), WINDOW, true), exit::NO_DEVICE);
        assert_eq!(scan(empty, WINDOW, false), exit::NO_DEVICE);
    }

    #[test]
    fn wp_tm9_a_network_that_cannot_be_browsed_is_not_an_empty_one() {
        let net = FakeAdvertiser::new();
        net.fail_next_opens(&["no interface", "no interface", "no interface"]);
        assert_eq!(scan(Arc::new(net), WINDOW, true), exit::TRANSPORT);
    }

    #[test]
    fn wp_tm9_the_table_shows_what_link_and_hub_take_and_cannot_hurt_a_terminal() {
        let net = net();
        // Control characters are the discovery layer's job; this holds it to that.
        net.announce(&advert("Ev\u{1b}[2Jil", "cccc"), &[ip("192.168.1.30")]);
        let text = table(&heard(&net));
        assert_eq!(text.lines().count(), 3, "{text}");
        assert!(text.contains("192.168.1.20:4848"), "{text}");
        assert!(text.contains("[fd00::2]:4848"), "{text}");
        assert!(text.contains("2 device(s)"), "{text}");
        assert!(!text.contains('\u{1b}'), "{text}");
    }

    #[test]
    fn wp_tm9_link_takes_scan_and_a_timeout_and_nothing_else() {
        assert_eq!(parse_args(&["scan"]), Ok(connect::SCAN_WINDOW));
        assert_eq!(
            parse_args(&["scan", "--json", "--timeout", "0.5"]),
            Ok(Duration::from_millis(500))
        );
        assert!(parse_args(&[]).is_err());
        assert!(parse_args(&["--json"]).is_err());
        assert!(parse_args(&["connect"]).is_err());
        assert!(parse_args(&["scan", "scan"]).is_err());
        for bad in ["0", "-1", "NaN", "inf", "601", "soon"] {
            assert!(parse_args(&["scan", "--timeout", bad]).is_err(), "{bad}");
        }
        assert!(parse_args(&["scan", "--timeout"]).is_err());
    }

    #[test]
    fn wp_tm9_a_choice_is_a_number_on_the_list() {
        assert_eq!(parse_choice("1\n", 2), Some(0));
        assert_eq!(parse_choice(" 2 ", 2), Some(1));
        for bad in ["0", "3", "", "x", "-1"] {
            assert_eq!(parse_choice(bad, 2), None, "{bad}");
        }
    }
}
