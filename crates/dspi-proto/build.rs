//! Generates Rust constants from the vendored DSPi firmware headers.
//!
//! The headers in `firmware/` are the source of truth for the protocol; nothing
//! in this crate transcribes an opcode, a range, or a struct size by hand. A
//! firmware bump is therefore a file copy plus a build, and the coverage test in
//! `tests/coverage.rs` reports exactly what changed.
//!
//! This is a purpose-built scanner, not a C parser. It handles the subset of C
//! the headers actually use for constants, and it is deliberately loud about
//! anything it cannot interpret rather than guessing.
//!
//! Platform-conditional constants (`NUM_CHANNELS` and friends, which differ
//! between RP2040 and RP2350 via `#if PICO_RP2350`) are **deliberately not
//! emitted**. Device topology is discovered at connect, never compiled in; see
//! REDESIGN_SPEC.md 3.2 mechanism 1. The scanner detects conflicting
//! redefinitions and skips them, so a new platform-conditional constant cannot
//! accidentally become a hardcoded assumption.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::Path;

const HEADERS: &[&str] = &[
    "config.h",
    "bulk_params.h",
    "control_surfaces.h",
    "psybass.h",
    "upmix.h",
    "siggen.h",
    "subharm.h",
    "tube.h",
    "limiter.h",
    "rta.h",
    "rta_fft.h",
    "rta_bass.h",
    "notify.h",
];

/// Prefixes we lift into generated `u16` constants, with the module they land in.
const DEFINE_GROUPS: &[(&str, &str)] = &[
    ("REQ_", "opcodes"),
    ("VENDOR_EP_", "usb"),
    ("WIRE_", "wire"),
    ("CS_", "cs"),
    ("PIN_CONFIG_", "status"),
    ("PRESET_", "status"),
    ("FLASH_", "status"),
    ("SIGGEN_", "siggen"),
    ("UPMIX_", "upmix"),
    ("FILTER_", "filter"),
    ("XOVER_", "filter"),
    ("MAX_XOVER_", "filter"),
    ("SUBHARM_", "subharm"),
    ("TUBE_", "tube"),
    ("LIMITER_", "limiter"),
    ("RTA_", "rta"),
    ("NOTIFY_", "notify"),
    ("PARAM_SRC_", "notify"),
    // The version this header set describes (config.h:659-667). Used only to
    // name the firmware a refused device should be updated to.
    ("FW_VERSION_", "firmware"),
    // Extension channel sub-command ids, selected by wValue under
    // REQ_EXTENSION (0x00). Not opcodes: they never reach ALL_OPCODES.
    ("EXT_", "ext"),
];

/// Enumerators lifted alongside the defines. Some headers carry protocol ids
/// as `enum` members rather than `#define`s (the tube and limiter parameter
/// indices, the notification source tags, the control-surface component types
/// and nouns), so the scanner reads those enums too, but only for these
/// prefixes: an enumerator anywhere else is internal and stays internal.
const ENUM_PREFIXES: &[&str] = &[
    "TUBE_PARAM_",
    "TUBE_NUM_PARAMS",
    "LIMITER_PARAM_",
    "LIMITER_NUM_PARAMS",
    "PARAM_SRC_",
    "CS_TYPE_",
    "CS_NOUN_",
];

/// Float ranges and defaults, lifted as `f32`.
const FLOAT_PREFIXES: &[&str] = &[
    "PSYBASS_",
    "UPMIX_",
    "LOUDNESS_",
    "LEVELLER_",
    "SUBHARM_",
    "TUBE_",
    "LIMITER_",
];

fn main() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("firmware");
    for h in HEADERS {
        println!("cargo:rerun-if-changed=firmware/{h}");
    }
    println!("cargo:rerun-if-changed=firmware/PROVENANCE.toml");
    println!("cargo:rerun-if-changed=build.rs");

    let mut ints: BTreeMap<String, (i64, String, String)> = BTreeMap::new();
    let mut floats: BTreeMap<String, (f64, String)> = BTreeMap::new();
    let mut conflicts: Vec<String> = Vec::new();

    for header in HEADERS {
        let path = dir.join(header);
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("cannot read vendored header {}: {e}", path.display()));

        for (name, v) in scan_enums(&text) {
            match ints.get(&name) {
                Some((prev, _, _)) if *prev != v => conflicts.push(name.clone()),
                Some(_) => {}
                None => {
                    let group = group_for(&name).unwrap_or_else(|| {
                        panic!("enum prefix of {name} has no DEFINE_GROUPS entry")
                    });
                    ints.insert(name, (v, group.to_string(), header.to_string()));
                }
            }
        }

        for raw in text.lines() {
            let Some((name, value)) = parse_define(raw) else {
                continue;
            };

            if let Some(group) = group_for(&name)
                && let Some(v) = parse_int(value)
            {
                match ints.get(&name) {
                    // Platform-conditional redefinition: refuse to emit.
                    Some((prev, _, _)) if *prev != v => {
                        conflicts.push(name.clone());
                    }
                    Some(_) => {}
                    None => {
                        ints.insert(name, (v, group.to_string(), header.to_string()));
                    }
                }
                continue;
            }

            if FLOAT_PREFIXES.iter().any(|p| name.starts_with(p))
                && let Some(v) = parse_float(value)
            {
                floats.entry(name).or_insert((v, header.to_string()));
            }
        }
    }

    // A conflicting definition is platform-dependent by construction. Drop it so
    // it can never be mistaken for a fixed fact, and say so in the build log.
    for name in &conflicts {
        ints.remove(name);
        println!("cargo:warning=platform-conditional, not emitted: {name}");
    }

    let mut out = String::new();
    out.push_str(
        "// @generated by build.rs from the vendored firmware headers. Do not edit.\n\
         // Source of truth: crates/dspi-proto/firmware/ (see PROVENANCE.toml).\n\n",
    );

    emit_provenance(&dir, &mut out);

    // Group the integer constants into modules.
    let mut by_group: BTreeMap<&str, Vec<(&String, &i64, &String)>> = BTreeMap::new();
    for (name, (v, group, header)) in &ints {
        by_group
            .entry(group.as_str())
            .or_default()
            .push((name, v, header));
    }

    for (group, items) in &by_group {
        let _ = writeln!(out, "pub mod {group} {{");
        for (name, v, _) in items {
            let ty = if **v >= 0 && **v <= u8::MAX as i64 && name.starts_with("REQ_") {
                "u8"
            } else if **v >= 0 && **v <= u16::MAX as i64 {
                "u16"
            } else {
                "i32"
            };
            let _ = writeln!(out, "    pub const {name}: {ty} = {v};");
        }
        let _ = writeln!(out, "}}\n");
    }

    if !floats.is_empty() {
        out.push_str("pub mod ranges {\n");
        for (name, (v, _)) in &floats {
            let _ = writeln!(out, "    pub const {name}: f32 = {v:?};");
        }
        out.push_str("}\n\n");
    }

    emit_opcode_table(&ints, &mut out);
    emit_wire_layout(&ints, &mut out);

    let dest = Path::new(&std::env::var("OUT_DIR").unwrap()).join("generated.rs");
    std::fs::write(&dest, out).expect("write generated.rs");

    // -- WP PGPROTO --
    paged::generate(&dir);
}

/// `#define NAME VALUE  // comment` -> `(NAME, VALUE)`.
fn parse_define(line: &str) -> Option<(String, &str)> {
    let rest = line.trim().strip_prefix("#define")?;
    let rest = rest.trim_start();
    let split = rest.find(|c: char| c.is_whitespace())?;
    let (name, value) = rest.split_at(split);

    // Function-like macros are not constants.
    if name.contains('(') {
        return None;
    }

    let value = value.split("//").next().unwrap_or("");
    let value = value.split("/*").next().unwrap_or("").trim();
    if value.is_empty() {
        return None;
    }
    Some((name.to_string(), value))
}

/// The members of every `enum { ... }` block whose names match
/// [`ENUM_PREFIXES`], with their values.
///
/// Handles exactly what C gives an enumerator: an explicit integer literal, or
/// nothing, meaning one more than the member before. An initializer this
/// scanner cannot read as a literal breaks the chain, so the members after it
/// are skipped until the next explicit literal rather than guessed at.
fn scan_enums(text: &str) -> Vec<(String, i64)> {
    let mut out = Vec::new();
    let mut in_enum = false;
    let mut next: Option<i64> = None;

    for raw in text.lines() {
        let line = raw.split("//").next().unwrap_or("");
        let line = line.split("/*").next().unwrap_or("").trim();
        if !in_enum {
            let head = line.strip_prefix("typedef ").unwrap_or(line);
            if (head.starts_with("enum ") || head.starts_with("enum{")) && line.contains('{') {
                in_enum = true;
                next = Some(0);
            }
            continue;
        }

        let (body, closes) = match line.find('}') {
            Some(i) => (&line[..i], true),
            None => (line, false),
        };
        for member in body.split(',') {
            let member = member.trim();
            if member.is_empty() {
                continue;
            }
            let (name, value) = match member.split_once('=') {
                Some((n, v)) => (n.trim(), parse_int(v.trim())),
                None => (member, next),
            };
            next = value.map(|v| v + 1);
            if let Some(v) = value
                && ENUM_PREFIXES.iter().any(|p| name.starts_with(p))
            {
                out.push((name.to_string(), v));
            }
        }
        if closes {
            in_enum = false;
        }
    }
    out
}

fn group_for(name: &str) -> Option<&'static str> {
    DEFINE_GROUPS
        .iter()
        .find(|(prefix, _)| name.starts_with(prefix))
        .map(|(_, group)| *group)
}

/// Plain integer literals only. Anything parenthesised or computed is an
/// expression we refuse to evaluate, because a wrong guess would be silent.
fn parse_int(v: &str) -> Option<i64> {
    let v = v.trim().trim_end_matches('u').trim_end_matches('U');
    if let Some(hex) = v.strip_prefix("0x").or_else(|| v.strip_prefix("0X")) {
        return i64::from_str_radix(hex, 16).ok();
    }
    v.parse::<i64>().ok()
}

fn parse_float(v: &str) -> Option<f64> {
    let v = v.trim().trim_end_matches('f').trim_end_matches('F');
    if !v.contains('.') && !v.contains('e') && !v.contains('E') {
        return None;
    }
    v.parse::<f64>().ok()
}

fn emit_provenance(dir: &Path, out: &mut String) {
    let toml = std::fs::read_to_string(dir.join("PROVENANCE.toml")).unwrap_or_default();
    let field = |key: &str| -> String {
        toml.lines()
            .find(|l| l.trim_start().starts_with(key))
            .and_then(|l| l.split('=').nth(1))
            .map(|v| v.trim().trim_matches('"').to_string())
            .unwrap_or_else(|| "unknown".into())
    };
    let _ = writeln!(out, "/// The firmware revision these constants came from.");
    let _ = writeln!(out, "pub mod provenance {{");
    let _ = writeln!(out, "    pub const BRANCH: &str = {:?};", field("branch"));
    let _ = writeln!(out, "    pub const COMMIT: &str = {:?};", field("commit"));
    let _ = writeln!(out, "    pub const SHORT: &str = {:?};", field("short"));
    let _ = writeln!(
        out,
        "    pub const COMMITTED: &str = {:?};",
        field("committed")
    );
    let _ = writeln!(out, "}}\n");
}

/// Every `REQ_*` opcode as a `(name, value)` table. The coverage test walks this
/// and fails the build if an opcode is neither registered nor explicitly
/// excluded, which is what makes "controls every setting" checkable.
fn emit_opcode_table(ints: &BTreeMap<String, (i64, String, String)>, out: &mut String) {
    let reqs: Vec<_> = ints
        .iter()
        .filter(|(n, _)| n.starts_with("REQ_"))
        .map(|(n, (v, _, _))| (n.clone(), *v as u8))
        .collect();

    let _ = writeln!(
        out,
        "/// Every vendor opcode defined by the firmware, sorted by name.\n\
         pub const ALL_OPCODES: [(&str, u8); {}] = [",
        reqs.len()
    );
    for (name, v) in &reqs {
        let _ = writeln!(out, "    ({name:?}, 0x{v:02X}),");
    }
    out.push_str("];\n\n");
}

/// Section offsets for `WireBulkParams`, computed from the sizing constants
/// rather than transcribed, then checked against the firmware's own total.
fn emit_wire_layout(ints: &BTreeMap<String, (i64, String, String)>, out: &mut String) {
    let g = |k: &str| -> i64 {
        ints.get(k)
            .map(|(v, _, _)| *v)
            .unwrap_or_else(|| panic!("vendored headers no longer define {k}; update build.rs"))
    };

    let n_ch = g("WIRE_MAX_CHANNELS");
    let n_in = g("WIRE_MAX_INPUT_CHANNELS");
    let n_out = g("WIRE_MAX_OUTPUT_CHANNELS");
    let n_bands = g("WIRE_MAX_BANDS");
    let n_xover = g("WIRE_MAX_XOVER_BANDS");
    let n_pins = g("WIRE_MAX_PIN_OUTPUTS");
    let name_len = g("WIRE_NAME_LEN");

    // (name, size) in declaration order; sizes derive from the constants above.
    let sections: Vec<(&str, i64)> = vec![
        ("header", 16),
        ("global", 16),
        ("crossfeed", 16),
        ("legacy", 16),
        ("delays", 4 * n_ch),
        ("crosspoints", 8 * n_in * n_out),
        ("outputs", 12 * n_out),
        ("pins", 1 + n_pins + 2),
        ("eq", 16 * n_ch * n_bands),
        ("channel_names", n_ch * name_len),
        ("i2s_config", 16),
        ("leveller", 20),
        ("preamp", 4 * n_in),
        ("master_volume", 16),
        ("input_config", 16),
        ("lg_sound_sync", 16),
        ("user_volume", 16),
        ("dac_hw_mute", 16),
        ("crossovers", 16 * n_ch * n_xover),
        ("adat_config", 8),
        ("psybass", 24),
        ("upmix", 44),
        // V29, grown at V30: WireSubharmParams (bulk_params.h:378-400).
        ("subharm", 36),
        // V31: WireTubeParams (bulk_params.h:403-427).
        ("tube", 48),
        // V32: WireLimiterParams, one 12-byte WireLimiterOutput per slot of
        // the output wire array (bulk_params.h:430-446).
        ("limiter", 12 * n_out),
    ];

    let mut offset = 0i64;
    let mut rows = String::new();
    for (name, size) in &sections {
        let upper = name.to_uppercase();
        let _ = writeln!(out, "pub const OFF_{upper}: usize = {offset};");
        let _ = writeln!(out, "pub const LEN_{upper}: usize = {size};");
        let _ = writeln!(rows, "    (\"{name}\", {offset}, {size}),");
        offset += size;
    }

    let _ = writeln!(
        out,
        "\n/// Total `WireBulkParams` size, derived from the header constants."
    );
    let _ = writeln!(out, "pub const BULK_SIZE: usize = {offset};");
    let _ = writeln!(
        out,
        "\n/// `(section, offset, len)` in declaration order.\n\
         pub const SECTIONS: [(&str, usize, usize); {}] = [\n{rows}];\n",
        sections.len()
    );
}

// -- WP PGPROTO --
/// The paged protocol of firmware 1.1.7 (PLAN-PG section 1.2): tables generated
/// from upstream's `dspi_commands.json` (the registry) and `legacy_map.json` (the
/// old-to-new map), vendored in `firmware/protocol/` (`PROVENANCE.toml`,
/// `[protocol]`). Nothing here is transcribed: every id, length, bound, flag,
/// revision and adapter row comes from the two files.
///
/// A row this generator cannot read (an unknown address kind, flag, adapter kind,
/// status family or result name, a target that is not in the registry, a
/// direction the target does not have, a malformed field) is **not** guessed at:
/// it is left out and listed in `UNREAD`, which `tests/paged_coverage.rs` fails on,
/// together with the counts recorded in `PROVENANCE.toml`.
mod paged {
    use serde_json::{Map, Value};
    use std::collections::BTreeMap;
    use std::fmt::Write as _;
    use std::path::Path;

    /// Address kinds in upstream's numbering (`scripts/gen_vendor_protocol.py`
    /// `ADDR_KINDS`, `vendor/vc_core.h` `VC_A_*`; `SYS.COMMAND_INFO` byte 6).
    const ADDR_KINDS: &[&str] = &[
        "NONE",
        "CHANNEL",
        "INPUT",
        "OUTPUT",
        "OUTPUT_ALL",
        "SLOT",
        "PIN_OUTPUT",
        "PRESET",
        "CH_BAND",
        "IN_OUT",
        "SOURCE",
        "SPDIF_RX",
        "I2S_PAIR",
        "ROLE",
        "CS_SLOT",
        "CS_SUB",
        "CS_GROUP",
        "CS_MACRO",
        "CS_MACRO_STEP",
        "CS_PAGE",
        "CS_NOUN",
        "PAGE",
        "PAGE_CMD",
        "OFFSET",
        "INDEX",
        "CS_SLOT_TYPE",
    ];

    /// Command flags, bit n = entry n (migration guide 4.7, `SYS.COMMAND_INFO` bytes 0-1).
    const FLAGS: &[&str] = &[
        "USB_ONLY",
        "SET_USB_ONLY",
        "BULK",
        "DEFERRED",
        "FLASH",
        "RESTART",
        "KEEPALIVE",
    ];

    struct Cmd {
        id: u16,
        get: bool,
        set: bool,
        set_min: u64,
        set_max: u64,
    }

    struct Gen {
        unread: Vec<(String, String)>,
        cmds: BTreeMap<String, Cmd>,
        results: BTreeMap<String, u64>,
        families: Map<String, Value>,
        page_nums: BTreeMap<String, u64>,
    }

    pub fn generate(dir: &Path) {
        let proto = dir.join("protocol");
        for f in ["dspi_commands.json", "legacy_map.json"] {
            println!("cargo:rerun-if-changed=firmware/protocol/{f}");
        }
        let read = |name: &str| -> Value {
            let path = proto.join(name);
            let text = std::fs::read_to_string(&path)
                .unwrap_or_else(|e| panic!("cannot read vendored {}: {e}", path.display()));
            serde_json::from_str(&text)
                .unwrap_or_else(|e| panic!("vendored {} is not JSON: {e}", path.display()))
        };
        let registry = read("dspi_commands.json");
        let map = read("legacy_map.json");

        let mut g = Gen {
            unread: Vec::new(),
            cmds: BTreeMap::new(),
            results: BTreeMap::new(),
            families: map["status_families"]
                .as_object()
                .cloned()
                .unwrap_or_default(),
            page_nums: BTreeMap::new(),
        };
        let head = "// @generated by build.rs (WP PGPROTO) from firmware/protocol/*.json. Do not edit.\n\n";
        let mut reg = String::from(head);
        emit_registry(&mut g, &registry, &mut reg);
        let mut out = String::from(head);
        emit_legacy(&mut g, &map, &mut out);

        let _ = writeln!(
            out,
            "/// Rows the generator could not read, `(row, reason)`. Must be empty.\n\
             pub static UNREAD: [(&str, &str); {}] = [",
            g.unread.len()
        );
        for (row, why) in &g.unread {
            println!("cargo:warning=paged: not generated: {row}: {why}");
            let _ = writeln!(out, "    ({row:?}, {why:?}),");
        }
        out.push_str("];\n");

        let dest = std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap());
        std::fs::write(dest.join("paged_registry.rs"), reg).expect("write paged_registry.rs");
        std::fs::write(dest.join("paged_legacy.rs"), out).expect("write paged_legacy.rs");
    }

    fn camel(s: &str) -> String {
        s.split('_')
            .map(|w| {
                let mut c = w.chars();
                match c.next() {
                    Some(f) => f.to_ascii_uppercase().to_string() + &c.as_str().to_lowercase(),
                    None => String::new(),
                }
            })
            .collect()
    }

    fn emit_registry(g: &mut Gen, reg: &Value, out: &mut String) {
        let major = reg["protocol"]["major"].as_u64();
        let minor = reg["protocol"]["minor"].as_u64();
        match (major, minor) {
            (Some(a), Some(b)) if a <= 255 && b <= 255 => {
                let _ = writeln!(
                    out,
                    "/// `protocol.major` of the vendored registry.\npub const VENDORED_MAJOR: u8 = {a};\n\
                     /// `protocol.minor` of the vendored registry.\npub const VENDORED_MINOR: u8 = {b};\n"
                );
            }
            _ => panic!("dspi_commands.json has no protocol major/minor"),
        }

        let mut pages = String::new();
        let mut rows = String::new();
        let mut consts = String::new();
        let (mut n_pages, mut n_rows) = (0usize, 0usize);
        let empty = Vec::new();
        for p in reg["pages"].as_array().unwrap_or(&empty) {
            let (Some(page), Some(mn), Some(rev)) = (
                p["page"].as_u64(),
                p["mnemonic"].as_str(),
                p["revision"].as_u64(),
            ) else {
                g.unread.push((
                    format!("page {p}"),
                    "page, mnemonic or revision missing".into(),
                ));
                continue;
            };
            if page == 0 || page > 255 || rev > 255 {
                g.unread
                    .push((mn.to_string(), "page or revision out of range".into()));
                continue;
            }
            let doc = p["doc"].as_str().unwrap_or("");
            let _ = writeln!(
                pages,
                "    PageInfo {{ page: 0x{page:02X}, mnemonic: {mn:?}, revision: {rev}, doc: {doc:?} }},"
            );
            n_pages += 1;
            g.page_nums.insert(mn.to_string(), page);
            for c in p["commands"].as_array().unwrap_or(&empty) {
                let name = c["name"].as_str().unwrap_or("?");
                let row = format!("{mn}.{name}");
                match command_row(c, page, mn) {
                    Ok((line, cmd)) => {
                        let _ = writeln!(rows, "    {line},");
                        let _ = writeln!(
                            consts,
                            "    pub const {mn}_{name}: CmdId = CmdId(0x{:04X});",
                            cmd.id
                        );
                        g.cmds.insert(row, cmd);
                        n_rows += 1;
                    }
                    Err(why) => g.unread.push((row, why)),
                }
            }
        }
        let _ = writeln!(
            out,
            "/// Every page of the vendored registry, ascending.\n\
             pub static PAGES: [PageInfo; {n_pages}] = [\n{pages}];\n"
        );
        let _ = writeln!(
            out,
            "/// Every command of the vendored registry, by page then command number.\n\
             pub static COMMANDS: [Command; {n_rows}] = [\n{rows}];\n"
        );
        let _ = writeln!(
            out,
            "/// Every command id, named `PAGE_NAME` (`ROUTE.MASTER_VOLUME` is `ROUTE_MASTER_VOLUME`).\n\
             pub mod cmds {{\n    use super::CmdId;\n{consts}}}\n"
        );
    }

    fn command_row(c: &Value, page: u64, mn: &str) -> Result<(String, Cmd), String> {
        let num = c["cmd"].as_u64().ok_or("no cmd number")?;
        if !(0x10..=0xFF).contains(&num) {
            return Err(format!("cmd 0x{num:X} outside 0x10..=0xFF"));
        }
        let id = (page << 8) | num;
        if c["id"].as_u64() != Some(id) {
            return Err(format!("id {} is not (page << 8) | cmd", c["id"]));
        }
        let name = c["name"].as_str().ok_or("no name")?;
        if !name
            .chars()
            .all(|ch| ch.is_ascii_uppercase() || ch.is_ascii_digit() || ch == '_')
        {
            return Err(format!("name {name:?} is not an upper-case identifier"));
        }
        let get = c["get"].as_bool().ok_or("get is not a bool")?;
        let set = c["set"].as_bool().ok_or("set is not a bool")?;
        let addr = c["addr"].as_str().ok_or("no addr")?;
        if !ADDR_KINDS.contains(&addr) {
            return Err(format!("unknown address kind {addr}"));
        }
        let get_len = match &c["get_len"] {
            Value::Null if !get => "GetLen::None".to_string(),
            Value::String(s) if s == "var" && get => "GetLen::Var".to_string(),
            Value::Number(n) if get && n.as_u64().is_some_and(|v| (1..=0xFFFE).contains(&v)) => {
                format!("GetLen::Fixed({n})")
            }
            other => return Err(format!("get_len {other} does not fit get = {get}")),
        };
        let set_min = c["set_min"].as_u64().ok_or("no set_min")?;
        let set_max = c["set_max"].as_u64().ok_or("no set_max")?;
        if set_min > set_max || set_max > 255 {
            return Err(format!("SET bounds {set_min}..={set_max}"));
        }
        let mut flags = 0u16;
        for f in c["flags"].as_array().ok_or("flags is not a list")? {
            let f = f.as_str().ok_or("a flag is not a string")?;
            let bit = FLAGS
                .iter()
                .position(|k| *k == f)
                .ok_or(format!("unknown flag {f}"))?;
            flags |= 1 << bit;
        }
        let build = match &c["build"] {
            Value::Null => "None".to_string(),
            Value::String(s) => format!("Some({s:?})"),
            other => return Err(format!("build {other}")),
        };
        let arg = c["arg"].as_str().ok_or("arg is not a string")?;
        let line = format!(
            "Command {{ id: CmdId(0x{id:04X}), page: {mn:?}, name: {name:?}, get: {get}, set: {set}, \
             addr: AddrKind::{}, get_len: {get_len}, set_min: {set_min}, set_max: {set_max}, \
             flags: Flags(0x{flags:04X}), build: {build}, arg: {arg:?} }}",
            camel(addr)
        );
        Ok((
            line,
            Cmd {
                id: id as u16,
                get,
                set,
                set_min,
                set_max,
            },
        ))
    }

    fn emit_legacy(g: &mut Gen, map: &Value, out: &mut String) {
        if map["format"].as_u64() != Some(1) {
            panic!(
                "legacy_map.json format {} is not 1: read its _schema first",
                map["format"]
            );
        }
        let _ = writeln!(
            out,
            "/// `legacy_map.json` `format`.\npub const LEGACY_FORMAT: u8 = 1;\n"
        );

        // Result codes (migration guide 5.3).
        let mut res = String::new();
        let mut n = 0usize;
        if let Some(obj) = map["vc_results"].as_object() {
            let mut sorted: Vec<_> = obj
                .iter()
                .filter_map(|(k, v)| Some((v.as_u64()?, k)))
                .collect();
            sorted.sort();
            for (v, k) in sorted {
                if v > 255 {
                    g.unread.push((k.clone(), "result code above 0xFF".into()));
                    continue;
                }
                g.results.insert(k.clone(), v);
                let _ = writeln!(res, "    ({k:?}, 0x{v:02X}),");
                let _ = writeln!(out, "pub const {k}: u8 = 0x{v:02X};");
                n += 1;
            }
        }
        let _ = writeln!(
            out,
            "\n/// Every result code, `(name, code)`, ascending.\npub static RESULTS: [(&str, u8); {n}] = [\n{res}];\n"
        );

        // The mnemonic table must agree with the registry's.
        let listed: BTreeMap<String, u64> = map["pages"]
            .as_object()
            .map(|o| {
                o.iter()
                    .filter_map(|(k, v)| Some((k.clone(), v.as_u64()?)))
                    .collect()
            })
            .unwrap_or_default();
        if listed != g.page_nums {
            g.unread.push((
                "pages".into(),
                "legacy_map.json pages differ from the registry's".into(),
            ));
        }

        let empty = Map::new();
        let ops = map["opcodes"].as_object().unwrap_or(&empty);
        let mut rows: Vec<(u64, String)> = Vec::new();
        let mut seen = BTreeMap::new();
        for (name, rec) in ops {
            let Some(code) = rec["code"].as_u64().filter(|c| *c <= 255) else {
                g.unread
                    .push((name.clone(), "code missing or above 0xFF".into()));
                continue;
            };
            if let Some(prev) = seen.insert(code, name.clone()) {
                g.unread.push((
                    name.clone(),
                    format!("code 0x{code:02X} also used by {prev}"),
                ));
                continue;
            }
            let old_dir = match rec["old_dir"].as_str() {
                Some("W") => "OldDir::Write",
                Some("R") => "OldDir::Read",
                Some("WR") => "OldDir::WriteAsRead",
                Some("W+R") => "OldDir::WriteAndRead",
                other => {
                    g.unread.push((name.clone(), format!("old_dir {other:?}")));
                    continue;
                }
            };
            let mut sides = Vec::new();
            let mut bad = false;
            for (usb, is_in) in [("out", false), ("in", true)] {
                match rec.get(usb) {
                    None => sides.push("None".to_string()),
                    Some(a) => match adapter(g, a, is_in, false) {
                        Ok(text) => sides.push(format!("Some(&{text})")),
                        Err(why) => {
                            g.unread.push((format!("{name}.{usb}"), why));
                            bad = true;
                        }
                    },
                }
            }
            if bad {
                continue;
            }
            let note = match rec["note"].as_str() {
                Some(s) => format!("Some({s:?})"),
                None => "None".to_string(),
            };
            rows.push((
                code,
                format!(
                    "    LegacyOp {{ name: {name:?}, code: 0x{code:02X}, old_dir: {old_dir}, out: {}, r#in: {}, note: {note} }},",
                    sides[0], sides[1]
                ),
            ));
        }
        rows.sort();
        let _ = writeln!(
            out,
            "/// Every 1.1.6 opcode of `legacy_map.json`, ascending by code.\n\
             pub static LEGACY_OPS: [LegacyOp; {}] = [",
            rows.len()
        );
        for (_, r) in rows {
            let _ = writeln!(out, "{r}");
        }
        out.push_str("];\n\n");
    }

    fn u(v: &Value, key: &str, default: u64) -> Result<u64, String> {
        match v.get(key) {
            None => Ok(default),
            Some(x) => x
                .as_u64()
                .ok_or(format!("{key} {x} is not an unsigned integer")),
        }
    }

    /// `{shift, mask, add}` (shift < 16, mask <= 0xFFFF).
    fn field(v: &Value) -> Result<String, String> {
        let shift = u(v, "shift", 0)?;
        let mask = u(v, "mask", 0xFFFF)?;
        let add = match v.get("add") {
            None => 0,
            Some(x) => x.as_i64().ok_or(format!("add {x}"))?,
        };
        if shift >= 16 || mask > 0xFFFF || !(-0xFFFF..=0xFFFF).contains(&add) {
            return Err(format!("field {v} out of range"));
        }
        Ok(format!(
            "Field {{ shift: {shift}, mask: 0x{mask:04X}, add: {add} }}"
        ))
    }

    /// An address rule; `default` is the kind's default when `addr` is absent.
    fn addr_rule(a: &Value, default: Option<&str>) -> Result<String, String> {
        match a.get("addr") {
            None => default.map(str::to_string).ok_or("addr is required".into()),
            Some(v) if v.get("const").is_some() => {
                let c = v["const"]
                    .as_u64()
                    .filter(|c| *c <= 0xFFFF)
                    .ok_or(format!("const {v}"))?;
                Ok(format!("AddrRule::Const({c})"))
            }
            Some(v) => Ok(format!("AddrRule::Field({})", field(v)?)),
        }
    }

    fn out_stall(a: &Value) -> Result<&'static str, String> {
        match a.get("out_stall") {
            None => Ok("OutStall::Swallow"),
            Some(v) if v == "raise" => Ok("OutStall::Raise"),
            Some(v) => Err(format!("out_stall {v}")),
        }
    }

    fn target<'g>(g: &'g Gen, a: &Value) -> Result<(String, &'g Cmd), String> {
        let t = a["target"].as_str().ok_or("no target")?;
        let c = g
            .cmds
            .get(t)
            .ok_or(format!("target {t} is not in the registry"))?;
        Ok((t.to_string(), c))
    }

    fn adapter(g: &Gen, a: &Value, is_in: bool, nested: bool) -> Result<String, String> {
        let kind = a["kind"].as_str().ok_or("no kind")?;
        match kind {
            "direct" => {
                let (t, c) = target(g, a)?;
                let (id, get, set) = (c.id, c.get, c.set);
                if is_in && !get {
                    return Err(format!("{t} has no GET"));
                }
                if !is_in && !set {
                    return Err(format!("{t} has no SET"));
                }
                let trunc = match a.get("trunc") {
                    None => "None".to_string(),
                    Some(v) => format!(
                        "Some({})",
                        v.as_u64()
                            .filter(|t| *t <= 0xFFFF)
                            .ok_or(format!("trunc {v}"))?
                    ),
                };
                let on_stall = match a.get("on_stall") {
                    None => "None".to_string(),
                    Some(v) => {
                        let fill = u(v, "fill", 0)?;
                        let len = u(v, "len", 0)?;
                        if fill > 255 || len > 0xFFFF {
                            return Err(format!("on_stall {v}"));
                        }
                        let mut codes = Vec::new();
                        for c in v["codes"].as_array().ok_or("on_stall.codes")? {
                            let name = c.as_str().ok_or("on_stall code")?;
                            let code = g
                                .results
                                .get(name)
                                .ok_or(format!("unknown result {name}"))?;
                            codes.push(format!("0x{code:02X}"));
                        }
                        format!(
                            "Some(OnStall {{ fill: {fill}, len: {len}, codes: &[{}] }})",
                            codes.join(", ")
                        )
                    }
                };
                Ok(format!(
                    "Adapter::Direct(Direct {{ target: CmdId(0x{id:04X}), addr: {}, trunc: {trunc}, bulk: {}, out_stall: {}, on_stall: {on_stall}, retired: {} }})",
                    addr_rule(a, Some("AddrRule::Old"))?,
                    a.get("bulk").and_then(Value::as_bool).unwrap_or(false),
                    out_stall(a)?,
                    a.get("retired").and_then(Value::as_bool).unwrap_or(false),
                ))
            }
            "act_value" | "act_addr" | "act_packed" => {
                if !is_in {
                    return Err(format!("{kind} on an OUT request"));
                }
                let (t, c) = target(g, a)?;
                let (id, set, set_min, set_max) = (c.id, c.set, c.set_min, c.set_max);
                if !set {
                    return Err(format!("{t} has no SET"));
                }
                let value = if kind == "act_addr" {
                    if set_min != 0 {
                        return Err(format!("{t} is not a zero-length SET"));
                    }
                    "None".to_string()
                } else {
                    if set_min > 1 || set_max < 1 {
                        return Err(format!("{t} does not take a 1-byte payload"));
                    }
                    let encode = match a["encode"].as_str() {
                        Some("u8") => "Encode::U8",
                        Some("u8_sat") => "Encode::U8Sat",
                        Some("bool") => "Encode::Bool",
                        other => return Err(format!("encode {other:?}")),
                    };
                    format!(
                        "Some(({}, {encode}))",
                        field(a.get("value").ok_or("no value")?)?
                    )
                };
                let addr = match kind {
                    "act_value" => addr_rule(a, Some("AddrRule::Const(0)"))?,
                    "act_addr" => addr_rule(a, Some("AddrRule::Old"))?,
                    _ => addr_rule(a, None)?,
                };
                let fam_name = a["status"].as_str().ok_or("no status family")?;
                let fam = g
                    .families
                    .get(fam_name)
                    .ok_or(format!("unknown status family {fam_name}"))?;
                let success = fam["success"]
                    .as_u64()
                    .filter(|s| *s <= 255)
                    .ok_or("family success")?;
                let mut table: BTreeMap<u64, u64> = BTreeMap::new();
                let overrides = a
                    .get("status_overrides")
                    .and_then(Value::as_object)
                    .cloned()
                    .unwrap_or_default();
                let from = fam["from_vc"].as_object().cloned().unwrap_or_default();
                for (vc, byte) in from.iter().chain(overrides.iter()) {
                    let code = g.results.get(vc).ok_or(format!("unknown result {vc}"))?;
                    let byte = byte
                        .as_u64()
                        .filter(|b| *b <= 255)
                        .ok_or(format!("status byte {byte}"))?;
                    table.insert(*code, byte);
                }
                let map: Vec<String> = table
                    .iter()
                    .map(|(c, b)| format!("(0x{c:02X}, 0x{b:02X})"))
                    .collect();
                let variant = camel(kind);
                Ok(format!(
                    "Adapter::{variant}(Act {{ target: CmdId(0x{id:04X}), addr: {addr}, value: {value}, status: Status {{ family: {fam_name:?}, success: 0x{success:02X}, from_vc: &[{}] }} }})",
                    map.join(", ")
                ))
            }
            "split" => {
                if nested {
                    return Err("a split inside a split".into());
                }
                let sel = a.get("selector").ok_or("no selector")?;
                if sel.get("add").is_some() {
                    return Err("a selector with add".into());
                }
                let mut cases = Vec::new();
                for (k, c) in a["cases"].as_object().ok_or("no cases")? {
                    let key: u64 = k.parse().map_err(|_| format!("case key {k}"))?;
                    if key > 0xFFFF {
                        return Err(format!("case key {k}"));
                    }
                    cases.push((key, adapter(g, c, is_in, true)?));
                }
                cases.sort_by_key(|(k, _)| *k);
                let cases: Vec<String> =
                    cases.iter().map(|(k, a)| format!("({k}, &{a})")).collect();
                let default = match a.get("default") {
                    None | Some(Value::Null) => "None".to_string(),
                    Some(d) => format!("Some(&{})", adapter(g, d, is_in, true)?),
                };
                Ok(format!(
                    "Adapter::Split(Split {{ selector: {}, cases: &[{}], default: {default}, out_stall: {} }})",
                    field(sel)?,
                    cases.join(", "),
                    out_stall(a)?
                ))
            }
            "custom" => {
                let func = a["fn"].as_str().ok_or("no fn")?;
                let mut uses = Vec::new();
                for u in a["uses"].as_array().ok_or("no uses")? {
                    let u = u.as_str().ok_or("use is not a string")?;
                    let (t, d) = u.rsplit_once(':').ok_or(format!("use {u}"))?;
                    let c = g
                        .cmds
                        .get(t)
                        .ok_or(format!("use {t} is not in the registry"))?;
                    let dir = match d {
                        "G" if c.get => "PagedDir::Get",
                        "S" if c.set => "PagedDir::Set",
                        _ => return Err(format!("use {u}: no such direction")),
                    };
                    uses.push(format!("(CmdId(0x{:04X}), {dir})", c.id));
                }
                let len = match a.get("len") {
                    None => "None".to_string(),
                    Some(v) => format!(
                        "Some({})",
                        v.as_u64()
                            .filter(|l| *l <= 0xFFFF)
                            .ok_or(format!("len {v}"))?
                    ),
                };
                // Everything else is the recipe's own data, kept verbatim for PGCUST.
                let mut data = a.as_object().cloned().unwrap_or_default();
                for k in ["kind", "fn", "uses", "retired", "bulk", "len"] {
                    data.remove(k);
                }
                let data = Value::Object(data).to_string();
                Ok(format!(
                    "Adapter::Custom(Custom {{ func: {func:?}, uses: &[{}], retired: {}, bulk: {}, len: {len}, data: {data:?} }})",
                    uses.join(", "),
                    a.get("retired").and_then(Value::as_bool).unwrap_or(false),
                    a.get("bulk").and_then(Value::as_bool).unwrap_or(false),
                ))
            }
            other => Err(format!("unknown adapter kind {other}")),
        }
    }
}
