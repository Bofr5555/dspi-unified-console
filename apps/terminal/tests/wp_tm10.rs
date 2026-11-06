//! WP TM10: `dspi export`, `import` and `cs`, as a user runs them, on the demo
//! device. Files go in Cargo's target-side scratch directory, never on tmpfs.

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};

use serde_json::Value;

const MAC: &str = include_str!("../../../tests/fixtures/preset/mac-swift-v32.dspipreset");
const QT: &str = include_str!("../../../tests/fixtures/preset/qt-core-v32.dspipreset");

fn scratch(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("wp_tm10");
    std::fs::create_dir_all(&dir).unwrap();
    dir.join(name)
}

fn dspi(args: &[&str], stdin: Option<&str>) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_dspi"))
        .arg("--demo")
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut pipe = child.stdin.take().unwrap();
    if let Some(text) = stdin {
        pipe.write_all(text.as_bytes()).unwrap();
    }
    drop(pipe);
    child.wait_with_output().unwrap()
}

fn out(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}

fn ok(o: Output) -> Output {
    assert_eq!(
        o.status.code(),
        Some(0),
        "{}{}",
        out(&o),
        String::from_utf8_lossy(&o.stderr)
    );
    o
}

fn json(o: &Output) -> Value {
    serde_json::from_str(&out(o)).unwrap_or_else(|e| panic!("{e}: {}", out(o)))
}

#[test]
fn wp_tm10_a_json_name_gets_the_whole_device_document() {
    let path = scratch("whole.json");
    let p = path.to_str().unwrap();
    let said = out(&ok(dspi(&["export", p], None)));
    assert!(said.contains("17 channels"), "{said}");
    let doc: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(doc["schemaVersion"], 1);
    assert_eq!(doc["meta"]["name"], "whole");
    assert_eq!(doc["channels"].as_array().unwrap().len(), 17);
    // Back in, from a name that does not say what it is.
    let said = out(&ok(dspi(&["import", p], None)));
    assert!(said.contains("Applied 17 channels"), "{said}");
}

#[test]
fn wp_tm10_export_to_standard_output() {
    // Filter text by default, with nothing else on the stream.
    let o = ok(dspi(&["export", "-"], None));
    assert!(out(&o).starts_with("# DSPi Console Filter Settings"));
    assert!(!out(&o).contains("Wrote"));
    // The whole-device document with --json.
    let o = ok(dspi(&["export", "-", "--json"], None));
    assert_eq!(json(&o)["schemaVersion"], 1);
    // Filters as JSON on request.
    let o = ok(dspi(&["export", "-", "--format", "filters-json"], None));
    let doc = json(&o);
    assert_eq!(doc["kind"], "dspi-filters");
    assert_eq!(doc["channels"][8]["crossover"].as_array().unwrap().len(), 4);
    assert!(doc["channels"][0]["preampDb"].is_number());
}

#[test]
fn wp_tm10_export_to_a_file_reports_as_json() {
    let path = scratch("report.dspipreset");
    let o = ok(dspi(&["export", path.to_str().unwrap(), "--json"], None));
    let r = json(&o);
    assert_eq!(r["ok"], true);
    assert_eq!(r["format"], "preset");
    assert_eq!(r["channels"], 17);
}

#[test]
fn wp_tm10_import_from_standard_input_goes_by_the_content() {
    // Both shared documents, and a report a script can read.
    for doc in [MAC, QT] {
        let o = ok(dspi(&["import", "-", "--dry-run", "--json"], Some(doc)));
        let r = json(&o);
        assert_eq!(r["format"], "preset");
        assert_eq!(r["dryRun"], true);
        assert_eq!(r["channels"], 17);
        assert_eq!(r["missing"].as_array().unwrap().len(), 0);
    }
    // Filter text.
    let text = out(&ok(dspi(&["export", "-"], None)));
    let o = ok(dspi(&["import", "-", "--json"], Some(&text)));
    assert_eq!(json(&o)["format"], "filters");
    assert_eq!(json(&o)["preamps"], 8);
    assert_eq!(json(&o)["crossoverBands"], 36);
    // Filter JSON.
    let doc = out(&ok(dspi(&["export", "-", "--format=filters-json"], None)));
    let o = ok(dspi(&["import", "-", "--json"], Some(&doc)));
    assert_eq!(json(&o)["format"], "filters");
    assert_eq!(json(&o)["crossoverBands"], 36);
}

#[test]
fn wp_tm10_import_says_the_console_things_in_text() {
    let o = ok(dspi(&["import", "-", "--dry-run"], Some(MAC)));
    let said = out(&o);
    assert!(said.contains("Would apply 17 channels"), "{said}");
    assert!(said.contains("Carries: "), "{said}");
    assert!(said.contains("crossfeed pairs"), "{said}");
    assert!(said.contains("input pair links"), "{said}");
}

#[test]
fn wp_tm10_bad_input_is_a_usage_error() {
    for (args, stdin) in [
        (vec!["export"], None),
        (vec!["import"], None),
        (vec!["export", "-", "--format", "yaml"], None),
        (vec!["import", "-"], Some("")),
        (vec!["import", "-"], Some("{not json")),
        (vec!["import", "-", "--format", "preset"], Some("[USB 1]\n")),
        (
            vec!["import", "-"],
            Some(r#"{"schemaVersion": 99, "channels": [{}]}"#),
        ),
        (vec!["import", "/nonexistent/x.dspipreset"], None),
        (vec!["cs"], None),
        (vec!["cs", "import", "-"], Some(MAC)),
        // A control-surface document does not slip in through a plain import.
        (
            vec!["import", "-"],
            Some(r#"{"kind": "dspi-control-surfaces", "schemaVersion": 1}"#),
        ),
    ] {
        let o = dspi(&args, stdin);
        assert_eq!(o.status.code(), Some(1), "{args:?}: {}", out(&o));
        assert!(!o.stderr.is_empty(), "{args:?}");
    }
    let huge = " ".repeat((4 << 20) + 2);
    let o = dspi(&["import", "-"], Some(&huge));
    assert_eq!(o.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&o.stderr).contains("MB"));
}

#[test]
fn wp_tm10_cs_export_and_import_work_on_the_demo_device() {
    let path = scratch("surfaces.json");
    let p = path.to_str().unwrap();
    let said = out(&ok(dspi(&["cs", "export", p], None)));
    assert!(said.starts_with("Wrote"), "{said}");
    let text = std::fs::read_to_string(&path).unwrap();
    let doc: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(doc["kind"], "dspi-control-surfaces");
    assert_eq!(doc["bindings"].as_array().unwrap().len(), 16);

    // The same on standard output, apart from the time.
    let piped = json(&ok(dspi(&["cs", "export", "-"], None)));
    assert_eq!(piped["bindings"], doc["bindings"]);
    assert_eq!(piped["macros"], doc["macros"]);

    // Its own document changes nothing.
    let r = json(&ok(dspi(&["cs", "import", p, "--json"], None)));
    assert_eq!(r["ok"], true);
    assert_eq!(r["bindings"], 0);
    assert_eq!(r["macros"], 0);

    // An edited one changes exactly that, and can be stored.
    let mut edited = doc.clone();
    edited["names"][0] = "Renamed".into();
    edited["groups"][0]["name"] = "Renamed".into();
    let text = edited.to_string();
    let r = json(&ok(dspi(
        &["cs", "import", "-", "--json", "--save"],
        Some(&text),
    )));
    assert_eq!((&r["names"], &r["groups"]), (&1.into(), &1.into()));
    assert_eq!(r["saved"], true);

    // A dry run says so and stores nothing.
    let said = out(&ok(dspi(
        &["cs", "import", "-", "--dry-run", "--save"],
        Some(&text),
    )));
    assert!(
        said.starts_with("Would change 0 bindings, 1 names"),
        "{said}"
    );
    assert!(!said.contains("Saved"), "{said}");
}

#[test]
fn wp_tm10_a_refused_control_surface_is_exit_3() {
    let mut doc = json(&ok(dspi(&["cs", "export", "-"], None)));
    let bindings = doc["bindings"].as_array().unwrap().clone();
    let holder = bindings.iter().find(|b| b["type"] == 1).unwrap().clone();
    let free = bindings.iter().position(|b| b["type"] == 0).unwrap();
    doc["bindings"][free] = holder;
    let text = doc.to_string();
    let o = dspi(&["cs", "import", "-", "--save"], Some(&text));
    assert_eq!(o.status.code(), Some(3), "{}", out(&o));
    assert!(out(&o).contains("Refused: binding"), "{}", out(&o));
    assert!(!out(&o).contains("Saved"), "{}", out(&o));
}

#[test]
fn wp_tm10_a_closed_pipe_ends_an_export_quietly() {
    for args in [
        &["export", "-"][..],
        &["export", "-", "--json"][..],
        &["cs", "export", "-"][..],
    ] {
        let mut child = Command::new(env!("CARGO_BIN_EXE_dspi"))
            .arg("--demo")
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        drop(child.stdout.take());
        let o = child.wait_with_output().unwrap();
        assert_eq!(o.status.code(), Some(0), "{args:?}");
        assert!(o.stderr.is_empty(), "{args:?}");
    }
}

/// Runs with standard output going to `stdout`.
fn with_stdout(args: &[&str], stdout: Stdio) -> Output {
    Command::new(env!("CARGO_BIN_EXE_dspi"))
        .arg("--demo")
        .args(args)
        .stdin(Stdio::null())
        .stdout(stdout)
        .stderr(Stdio::piped())
        .spawn()
        .unwrap()
        .wait_with_output()
        .unwrap()
}

/// The commands that report on standard output, with their documents on file.
fn reporting_commands(tag: &str) -> Vec<Vec<String>> {
    let preset = scratch(&format!("{tag}-in.dspipreset"));
    std::fs::write(&preset, MAC).unwrap();
    let filters = scratch(&format!("{tag}-in.txt"));
    std::fs::write(&filters, out(&ok(dspi(&["export", "-"], None)))).unwrap();
    let cs = scratch(&format!("{tag}-cs.json"));
    let cs_doc = out(&ok(dspi(&["cs", "export", "-"], None)));
    std::fs::write(&cs, cs_doc).unwrap();
    let (preset, filters, cs) = (
        preset.to_str().unwrap().to_string(),
        filters.to_str().unwrap().to_string(),
        cs.to_str().unwrap().to_string(),
    );
    let s = |v: &[&str]| v.iter().map(|a| a.to_string()).collect::<Vec<_>>();
    vec![
        s(&["import", &preset]),
        s(&["import", &preset, "--json"]),
        s(&["import", &filters]),
        s(&["import", &filters, "--json"]),
        s(&["cs", "import", &cs]),
        s(&["cs", "import", &cs, "--json"]),
    ]
}

#[cfg(target_os = "linux")]
#[test]
fn wp_tm10_a_full_disk_is_a_failure_with_a_reason() {
    let full = || {
        let f = std::fs::File::options()
            .write(true)
            .open("/dev/full")
            .unwrap();
        Stdio::from(f)
    };
    let mut commands = reporting_commands("full");
    // A document on standard output, and a report about one written to a file.
    let path = scratch("full-out.dspipreset");
    let path = path.to_str().unwrap();
    for args in [
        &["export", "-"][..],
        &["export", "-", "--json"],
        &["cs", "export", "-"],
        &["export", path],
        &["cs", "export", path],
    ] {
        commands.push(args.iter().map(|a| a.to_string()).collect());
    }
    for args in &commands {
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        let o = with_stdout(&args, full());
        let said = String::from_utf8_lossy(&o.stderr);
        assert_eq!(o.status.code(), Some(4), "{args:?}: {said}");
        assert!(said.contains("could not write"), "{args:?}: {said}");
    }
}

#[test]
fn wp_tm10_a_closed_reader_does_not_fail_an_import() {
    for args in reporting_commands("closed") {
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        let (reader, writer) = std::io::pipe().unwrap();
        drop(reader);
        let o = with_stdout(&args, Stdio::from(writer));
        assert_eq!(o.status.code(), Some(0), "{args:?}");
        assert!(o.stderr.is_empty(), "{args:?}");
    }
}
