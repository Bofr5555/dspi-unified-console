//! WP F1: the bindings generate from the built library, for Kotlin and Swift,
//! and their surface is the one `dspi-core` declares. Runs the workspace's own
//! `uniffi-bindgen` on `libdspi_ffi.so`, so it needs the `cli` feature (the
//! gates test with `--all-features`) and a Linux host.
#![cfg(all(feature = "cli", target_os = "linux"))]

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

/// The public `DspiCore` methods a binding cannot carry: the argument is a Rust
/// trait object. Everything else is exported.
const NOT_EXPORTED: &[&str] = &[
    // -- WP MCP2a -- Rust-only: the MCP server asks who opened the undo group
    // (actors exist only in the process; no binding has them).
    "undo_group_owner",
    // -- WP FEAT2 -- Rust-only, for the same server
    "user_editing",
    // -- WP INTI -- Rust-only, for the same server (review RF4: a scene's preset loads)
    "alink_system_scene_preset_loads",
    // -- WP FEATUI -- an agent's presence on the hub: the desktop (C) sends it, the web
    // (wasm) shows it; Android and Swift run no agents and show no strip yet
    "hub_presence_set",
    "hub_presence_list",
    // -- WP ATDEMO -- The demo hub's simulated microphone for a measurement across
    // units: the web and Qt only (Android has no system measurement screen yet,
    // PLAN-AT Later); through the wasm and C bindings.
    "measure_system_sim_set",
    "measure_system_sim_info",
    "open_transport",
    // -- WP HB0 -- Rust-only: takes a trait returning `Box<dyn Transport>`.
    "set_device_broker",
    // -- WP ANDFU -- (ROMODE) the C ABI and the web binding only, not UniFFI.
    "client_notice_key",
    // -- WP CG2 --
    // Behind feature `mdns`, which dspi-ffi leaves off: Android and iOS browse with
    // their system APIs. core_methods() reads the source, so it sees them anyway.
    "link_browse_start",
    "link_browse_start_with",
    "link_browse_stop",
    "link_browse_snapshot",
    "link_browse_resolve",
    "link_browse_reresolve",
    "link_browse_verify",
    // -- WP QD6 --
    // The Qt smoke driver's read-only fixtures, through the C ABI. Android has its own
    // read-only fixtures (bridge ReadOnlyTest), and its write guard counts refusals.
    "open_demo_wire",
    "read_only_refusals",
    // -- WP PGQT -- the Qt smoke driver's fake generations, through the C ABI
    "open_demo_generation",
    "demo_generation_violations",
    // -- WP CG23 -- The C drain's too-small buffer; Rust-only (doc(hidden)).
    "notify_tap_note_dropped",
    // -- WP CG32 -- The web's runtime without threads (WB2, WB3): not exported
    // over UniFFI (rta.rs behind cfg(wasm32)), which core_methods() reads anyway.
    "rta_tick",
    "push_notification",
    "notify_retry_due_in",
    // -- WP WB4b -- The web's caller-driven Link (link.rs): takes a trait object
    // (WsConnector) or is driven by the web binding's timer; not exported.
    "link_connect_over",
    "link_service",
    "link_tick",
    // -- WP SW3 (fixup wb8) -- The web's System view: the hub's `system.*` on the
    // session's own connection (dspi-wasm `hubSystem`, `hubRole`); the other clients
    // use the typed `alink_system_*` calls.
    "link_hub_request",
    "link_hub_role",
    // -- WP WB8 -- The web demo's clip (dspi-wasm `demo {action: "clip"}`), for tests.
    "demo_clip",
    // -- WP P0M -- Rust-only: the history's own records (for the C ABI's and
    // wasm's JSON lines; UniFFI has `event_log_read`) and the hub's sink, a trait
    // object.
    "event_log_page",
    "set_event_sink",
    // -- WP P0E -- Rust-only: the bundle as bytes, for the web, which has no file
    // system; UniFFI has `diagnostics_save`.
    "diagnostics_zip",
    // -- WP FWX -- The clip events' hand-off to WP P0M's clip hold and history
    // (ext_diag.rs), Rust-only until a client needs it.
    "take_clip_mask",
    // -- WP P1TZF -- `write_param_confirmed` is exported now (review p1tz F3).
    // TMCORE's image read for the Terminal's own `Session` over the core
    // (raw.rs, doc(hidden)); Rust-only. Missing here since TMCORE: this test
    // runs only with --all-features, which no step ran on dspi-ffi.
    "read_image_bytes",
    // -- WP P3CC -- the web's image upload as base64 text (dspi-wasm
    // `hubFirmwarePut`); UniFFI has `hub_firmware_put` with the bytes.
    "hub_firmware_put_base64",
    // -- review PG-C -- (WP PGCORE) Rust-only, doc(hidden): the Terminal's
    // transport over the core reads the connection's generation and discovery;
    // the bindings have `device_protocol`.
    "connection_protocol_generation",
    "connection_paged_discovery",
    // -- review PG-C -- (WP PGFWUP) Rust-only: `device_protocol` carries the
    // board to the bindings. -- WP PGFWX -- `firmware_target` is exported now.
    "board",
    // -- WP ATQT -- Qt's System editor: the S/PDIF rows and edits, through the C
    // ABI's dspi_alink_system_view and _save; the UniFFI clients have no S/PDIF
    // editor yet (their records would change, Android's positional callers too).
    "alink_system_spdif",
    "alink_system_save_spdif",
];

/// `crates/dspi-core/src/link.rs`, exported by WP C15.
const LINK_METHODS: &[&str] = &[
    "link_connect",
    "link_pair",
    "link_devices",
    "link_status",
    "link_select_device",
    "link_close",
    "set_link_listener",
];

fn library() -> PathBuf {
    // Cargo builds the library for an integration test in the test's own
    // directory, target/<profile>/deps.
    let exe = std::env::current_exe().unwrap();
    let lib = exe.parent().unwrap().join("libdspi_ffi.so");
    assert!(lib.exists(), "{} was not built", lib.display());
    lib
}

/// Generate `language`'s bindings into a directory of their own under the
/// target directory, never a tmpfs.
fn generate(language: &str) -> PathBuf {
    let lib = library();
    // Tests run in parallel, and each must keep the directory it generated.
    static RUN: AtomicUsize = AtomicUsize::new(0);
    let run = RUN.fetch_add(1, Ordering::Relaxed);
    let out = lib
        .parent()
        .unwrap()
        .join("wp-f1-bindings")
        .join(format!("{language}-{run}"));
    let _ = std::fs::remove_dir_all(&out);
    std::fs::create_dir_all(&out).unwrap();
    // One run per namespace, each with its config by path.
    let crates = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    // -- WP SW-M3 -- and the hub's namespace in a build with `hub`
    let mut namespaces = vec![("dspi_core", "dspi-core"), ("dspi_ffi", "dspi-ffi")];
    if cfg!(feature = "hub") {
        namespaces.push(("dspi_hub", "dspi-hub"));
    }
    for (name, dir) in namespaces {
        let run = Command::new(env!("CARGO_BIN_EXE_uniffi-bindgen"))
            .current_dir(env!("CARGO_MANIFEST_DIR"))
            .args(["generate", "--library"])
            .arg(&lib)
            .args(["--crate", name, "--config"])
            .arg(crates.join(dir).join("uniffi.toml"))
            .args(["--language", language, "--out-dir"])
            .arg(&out)
            .output()
            .unwrap();
        assert!(
            run.status.success(),
            "uniffi-bindgen {language} {name}: {}",
            String::from_utf8_lossy(&run.stderr)
        );
    }
    out
}

fn find(dir: &Path, name: &str) -> PathBuf {
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                stack.push(p);
            } else if p.file_name().is_some_and(|n| n == name) {
                return p;
            }
        }
    }
    panic!("{name} not generated in {}", dir.display());
}

fn camel(snake: &str) -> String {
    let mut parts = snake.split('_');
    let mut out = parts.next().unwrap().to_owned();
    for p in parts {
        let mut c = p.chars();
        if let Some(first) = c.next() {
            out.extend(first.to_uppercase());
            out.push_str(c.as_str());
        }
    }
    out
}

/// Every `pub fn name(&self` of an `impl DspiCore` in the core's sources.
fn core_methods() -> BTreeSet<String> {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("../dspi-core/src");
    let mut names = BTreeSet::new();
    // -- fixup wb8 -- the module folders too (`alink/system.rs`, LKUI1's exported
    // `alink_system_*`), not only the top level; test modules are left out
    let mut files = Vec::new();
    let mut dirs = vec![src];
    while let Some(dir) = dirs.pop() {
        for e in std::fs::read_dir(dir).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                dirs.push(p);
            } else if p.extension().is_some_and(|x| x == "rs")
                && !p
                    .file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with("tests"))
            {
                files.push(p);
            }
        }
    }
    for p in files {
        let text = std::fs::read_to_string(&p).unwrap();
        let mut in_core = false;
        for line in text.lines() {
            if line.starts_with("impl ") {
                in_core = line == "impl DspiCore {";
            }
            if in_core && let Some(rest) = line.strip_prefix("    pub fn ") {
                let name = rest.split(['(', '<']).next().unwrap();
                if rest.contains("&self") || !rest.contains(')') {
                    names.insert(name.to_owned());
                }
            }
        }
    }
    names.remove("new");
    names
}

/// The method names of the generated `interface DspiCoreInterface`.
fn kotlin_methods(kotlin: &str) -> BTreeSet<String> {
    let start = kotlin.find("interface DspiCoreInterface").unwrap();
    let body = &kotlin[start..];
    let end = body.find("\n}").unwrap();
    body[..end]
        .lines()
        .filter_map(|l| {
            l.trim()
                .strip_prefix("fun `")
                .or(l.trim().strip_prefix("fun "))
        })
        .map(|l| l.split(['`', '(']).next().unwrap().to_owned())
        .collect()
}

/// The generated files with this extension, relative to `dir`.
fn generated_files(dir: &Path, extension: &str) -> BTreeSet<PathBuf> {
    let mut found = BTreeSet::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                stack.push(p);
            } else if p.extension().is_some_and(|x| x == extension) {
                found.insert(p.strip_prefix(dir).unwrap().to_path_buf());
            }
        }
    }
    found
}

/// `generate` names the namespaces by hand; a run without `--crate` finds them
/// all. `ci/check.sh --step kotlin` compiles them together, so a package two
/// namespaces share fails there; this says why, without a compiler.
#[test]
fn wp_cg9_every_namespace_is_generated_with_its_own_package() {
    let out = generate("kotlin");
    let lib = library();
    let all = lib.parent().unwrap().join("wp-cg9-bindings");
    let _ = std::fs::remove_dir_all(&all);
    std::fs::create_dir_all(&all).unwrap();
    let run = Command::new(env!("CARGO_BIN_EXE_uniffi-bindgen"))
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .args(["generate", "--library"])
        .arg(&lib)
        .args(["--language", "kotlin", "--no-format", "--out-dir"])
        .arg(&all)
        .output()
        .unwrap();
    assert!(
        run.status.success(),
        "uniffi-bindgen: {}",
        String::from_utf8_lossy(&run.stderr)
    );
    let files = generated_files(&out, "kt");
    assert_eq!(files, generated_files(&all, "kt"), "a namespace is missing");

    let dirs: BTreeSet<_> = files.iter().filter_map(|f| f.parent()).collect();
    assert_eq!(dirs.len(), files.len(), "two namespaces share a package");
}

#[test]
fn wp_f1_kotlin_surface_is_the_cores_public_api() {
    let out = generate("kotlin");
    let core = std::fs::read_to_string(find(&out, "dspi_core.kt")).unwrap();

    let want: BTreeSet<String> = core_methods()
        .into_iter()
        .filter(|m| !NOT_EXPORTED.contains(&m.as_str()))
        .map(|m| camel(&m))
        .collect();
    let got: BTreeSet<String> = kotlin_methods(&core).into_iter().collect();
    assert_eq!(
        want.difference(&got).collect::<Vec<_>>(),
        Vec::<&String>::new(),
        "public but not exported"
    );
    assert_eq!(
        got.difference(&want).collect::<Vec<_>>(),
        Vec::<&String>::new(),
        "exported but not public: a private helper leaked into an exported impl"
    );

    assert!(core.contains("package se.bofr.dspi.ffi.uniffi"));
    for item in [
        "fun `firmwareVersionMatches`",
        "fun `computeMagnitudeCurve`",
        "fun `uf2Inspect`",
        "fun `flashConfirmed`", // -- WP P1TZF -- the one drive write, with a token
        "fun `configInspect`",
        "class DspiException",
        "data class DspState",
        "interface NotifyListener",
        "interface MidiSender",
        "interface MidiInputHandleInterface",
        "fun `closePort`",
        "fun `withSender`",
    ] {
        assert!(core.contains(item), "{item} is not in the Kotlin binding");
    }
    assert!(
        core.contains("**Poll off the UI thread.**"),
        "poll_hotplug's warning reaches the binding's docs"
    );

    let ffi = std::fs::read_to_string(find(&out, "dspi_ffi.kt")).unwrap();
    assert!(ffi.contains("package se.bofr.dspi.ffi.uniffi.logging"));
    assert!(ffi.contains("fun `initLogging`"));
}

#[test]
fn wp_f1_swift_binding_generates() {
    let out = generate("swift");
    let core = std::fs::read_to_string(find(&out, "dspi_core.swift")).unwrap();
    for item in [
        "open func pollHotplug()",
        "firmwareVersionMatches(version: String)",
        "public struct DspState",
        "public enum DspiError",
        "public protocol NotifyListener",
        "public protocol MidiSender",
    ] {
        assert!(core.contains(item), "{item} is not in the Swift binding");
    }
    find(&out, "dspi_coreFFI.h");
    let ffi = std::fs::read_to_string(find(&out, "dspi_ffi.swift")).unwrap();
    assert!(ffi.contains("public func initLogging()"));
}

#[test]
fn wp_f1_link_api_is_exported() {
    let out = generate("kotlin");
    let core = std::fs::read_to_string(find(&out, "dspi_core.kt")).unwrap();
    let got = kotlin_methods(&core);
    for m in LINK_METHODS {
        assert!(got.contains(&camel(m)), "{m} is not exported");
    }
    for f in [
        "fun `linkParseAddress`",
        "fun `linkWsUrl`",
        "fun `linkFetchInfo`",
    ] {
        assert!(core.contains(f), "{f} is not exported");
    }
}

// -- WP CG6 --
#[test]
fn wp_cg6_autoeq_upkeep_is_exported() {
    let out = generate("kotlin");
    let core = std::fs::read_to_string(find(&out, "dspi_core.kt")).unwrap();
    for f in [
        "fun `autoeqDatabaseInfo`",
        "fun `autoeqImport`",
        "fun `autoeqUpdate`",
        "fun `autoeqResetToBuiltin`",
        "fun `autoeqRebuildWarning`",
        "fun `autoeqRebuildStart`",
        "fun `poll`",
        "fun `cancel`",
        "interface AutoEqFetch",
        "class AutoEqRebuildProgress",
    ] {
        assert!(core.contains(f), "{f} is not exported");
    }
    let swift = generate("swift");
    let core = std::fs::read_to_string(find(&swift, "dspi_core.swift")).unwrap();
    for f in ["func autoeqRebuildStart(", "protocol AutoEqFetch"] {
        assert!(core.contains(f), "{f} is not exported");
    }
}

// -- WP CG8 --
#[test]
fn wp_cg8_firmware_guard_is_exported() {
    let kotlin = std::fs::read_to_string(find(&generate("kotlin"), "dspi_core.kt")).unwrap();
    for item in [
        "fun `firmwareGuardCheck`",
        "fun `firmwareGuardBoard`",
        "fun `firmwareGuardBoardInBootloader`",
        "fun `flashConfirmed`", // -- WP P1TZ -- the guarded write, with a token
        "data class FirmwareGuardBoard",
        "data class FirmwareGuardVerdict",
        "enum class FirmwareGuardDecision",
        "enum class FirmwareGuardReason",
        "enum class FirmwareGuardBoardSource",
        "class FirmwareGuardBlocked",
    ] {
        assert!(kotlin.contains(item), "{item} is not in the Kotlin binding");
    }
    let swift = std::fs::read_to_string(find(&generate("swift"), "dspi_core.swift")).unwrap();
    for item in [
        "func firmwareGuardCheck(",
        "func firmwareGuardBoard(",
        "func firmwareGuardBoardInBootloader(",
        "func flashConfirmed(", // -- WP P1TZ --
        "struct FirmwareGuardVerdict",
    ] {
        assert!(swift.contains(item), "{item} is not in the Swift binding");
    }
}

// -- WP CG10 --
#[test]
fn wp_cg10_wording_and_onboarding_are_exported() {
    let kotlin = std::fs::read_to_string(find(&generate("kotlin"), "dspi_core.kt")).unwrap();
    for item in [
        "fun `linkPhaseText`",
        "fun `linkPhaseTexts`",
        "fun `onboardingWhatsNewJson`",
        "fun `onboardingGettingStartedJson`",
        "data class LinkPhaseText",
    ] {
        assert!(kotlin.contains(item), "{item} is not in the Kotlin binding");
    }
    let swift = std::fs::read_to_string(find(&generate("swift"), "dspi_core.swift")).unwrap();
    for item in [
        "func linkPhaseText(",
        "func linkPhaseTexts(",
        "func onboardingWhatsNewJson(",
        "func onboardingGettingStartedJson(",
        "struct LinkPhaseText",
    ] {
        assert!(swift.contains(item), "{item} is not in the Swift binding");
    }
}

// -- WP CG15 --
#[test]
fn wp_cg15_about_is_exported() {
    let kotlin = std::fs::read_to_string(find(&generate("kotlin"), "dspi_core.kt")).unwrap();
    assert!(
        kotlin.contains("fun `aboutJson`"),
        "not in the Kotlin binding"
    );
    let swift = std::fs::read_to_string(find(&generate("swift"), "dspi_core.swift")).unwrap();
    assert!(
        swift.contains("func aboutJson("),
        "not in the Swift binding"
    );
}

// -- WP CG34 --
#[test]
fn wp_cg34_filter_type_names_are_exported() {
    let kotlin = std::fs::read_to_string(find(&generate("kotlin"), "dspi_core.kt")).unwrap();
    for item in [
        "fun `filterTypeNames`",
        "fun `filterTypeName`",
        "data class FilterTypeName",
    ] {
        assert!(kotlin.contains(item), "{item} is not in the Kotlin binding");
    }
    let swift = std::fs::read_to_string(find(&generate("swift"), "dspi_core.swift")).unwrap();
    for item in [
        "func filterTypeNames(",
        "func filterTypeName(",
        "struct FilterTypeName",
    ] {
        assert!(swift.contains(item), "{item} is not in the Swift binding");
    }
}

// -- WP CG-SWS-a --
#[test]
fn cgswsa_small_core_exports_are_in_both_bindings() {
    let kotlin = std::fs::read_to_string(find(&generate("kotlin"), "dspi_core.kt")).unwrap();
    let swift = std::fs::read_to_string(find(&generate("swift"), "dspi_core.swift")).unwrap();
    for f in [
        "quantizeMasterVolume",
        "curveSections",
        "vregCodeMv",
        "vregCodeLabel",
        "platformShape",
        "knownFeatureNames",
        "defaultChannelNames",
        "enumValueLabels",
        "bulkImageExtras",
        "linkParseTxt",
        "imageExtras",
    ] {
        assert!(
            kotlin.contains(&format!("fun `{f}`")),
            "{f} is not in the Kotlin binding"
        );
        assert!(
            swift.contains(&format!("func {f}(")),
            "{f} is not in the Swift binding"
        );
    }
    for r in [
        "BiquadSection",
        "PlatformShape",
        "BulkImageExtras",
        "EnumValueLabel",
        "LinkTxtRecord",
        "LinkTxtEntry",
    ] {
        assert!(
            kotlin.contains(&format!("data class {r}")),
            "{r} is not in the Kotlin binding"
        );
        assert!(
            swift.contains(&format!("struct {r}")),
            "{r} is not in the Swift binding"
        );
    }
}
