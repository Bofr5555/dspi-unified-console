//! `capi-consts`: writes `crates/dspi-capi/src/types/consts.rs`, the constants
//! of the C ABI (`#define`s in `dspi_core.h`).
//!
//! Every value is read from the crate that owns it: the firmware-generated
//! tables and typed items of `dspi-proto`, `dspi-core::consts`, and
//! `dspi-session`. The few names the ABI keeps although no crate exports them
//! are in [`not_in_the_protocol`], each with its reason.
//!
//! The output holds literals, not paths: cbindgen copies a literal into the
//! header, and the Android and Apple constant generators
//! (`apps/android/tools/gen_kotlin_contract.py`, `apps/apple/Tools/
//! gen_swift_constants.py`) read the file line by line. Their rules shape the
//! format: one `pub const NAME: TYPE = LITERAL;` per line, a literal is a
//! decimal or hex integer, `1 << n`, or a float, and a `// ── key` line
//! starts the group the Kotlin and Swift files sort the constants under.

use std::fmt::Write as _;
use std::path::Path;
use std::process::ExitCode;

use dspi_core::consts;
use dspi_proto::FilterType;
use dspi_proto::generated as gen_;
use dspi_proto::wire_compat::feat;

/// Where the generated file goes, from the workspace root.
const OUT: &str = "crates/dspi-capi/src/types/consts.rs";

/// The command that writes it, as the file's first line names it.
const COMMAND: &str = "cargo run -p xtask -- capi-consts";

/// Firmware opcodes the C ABI does not define: the ones newer than its
/// function set, reached through dspi-core's typed calls instead.
const OPCODES_LEFT_OUT: &[&str] = &[
    "REQ_EXTENSION",
    "REQ_GET_CLOCK_DIAG",
    "REQ_GET_DEVICE_NAME",
    "REQ_SET_DEVICE_NAME",
    "REQ_GET_INPUT_SOURCE_NAME",
    "REQ_SET_INPUT_SOURCE_NAME",
    "REQ_OUTPUT_LATENCY",
];

/// A constant's value.
#[derive(Clone, Copy)]
enum Value {
    Int(i128),
    Float(f64),
}

/// How the literal is spelled.
#[derive(Clone, Copy, PartialEq)]
enum Style {
    Dec,
    Hex,
    /// `1 << n`, for single-bit flags.
    Bit,
}

/// Anything a constant's source can be: the integer widths and the floats.
trait IntoValue {
    fn value(self) -> Value;
}

macro_rules! int_value {
    ($($t:ty),*) => {$(
        impl IntoValue for $t {
            fn value(self) -> Value {
                Value::Int(i128::try_from(self).unwrap_or_else(|_| panic!("{self} does not fit i128")))
            }
        }
    )*};
}
int_value!(u8, u16, u32, u64, usize, i64);

impl IntoValue for f32 {
    fn value(self) -> Value {
        Value::Float(f64::from(self))
    }
}

impl IntoValue for f64 {
    fn value(self) -> Value {
        Value::Float(self)
    }
}

struct Const {
    name: String,
    ty: &'static str,
    value: Value,
    style: Style,
    doc: String,
}

struct Group {
    key: &'static str,
    title: &'static str,
    items: Vec<Const>,
}

impl Group {
    fn new(key: &'static str, title: &'static str) -> Self {
        Self {
            key,
            title,
            items: Vec::new(),
        }
    }

    fn add(&mut self, name: &str, ty: &'static str, style: Style, v: impl IntoValue, doc: String) {
        self.items.push(Const {
            name: name.to_string(),
            ty,
            value: v.value(),
            style,
            doc,
        });
    }
}

/// `take!(group, ty, Style, some::module => [A, B])`: constants that keep
/// their source's name.
macro_rules! take {
    ($g:expr, $ty:literal, $style:ident, $($m:ident)::+ => [$($n:ident),+ $(,)?]) => {
        // The module goes along as one bracketed token tree, so that it can
        // sit inside the repetition over the names.
        take!(@each $g, $ty, $style, [$($m)::+], $($n),+)
    };
    (@each $g:expr, $ty:literal, $style:ident, $m:tt, $($n:ident),+) => {
        $( take!(@one $g, $ty, $style, $m, $n); )+
    };
    (@one $g:expr, $ty:literal, $style:ident, [$($m:ident)::+], $n:ident) => {
        $g.add(
            stringify!($n),
            $ty,
            Style::$style,
            $($m)::+::$n,
            source_doc(concat!(stringify!($($m)::+), "::", stringify!($n))),
        )
    };
}

/// `renamed!(group, ty, Style, ABI_NAME = some::path::SOURCE_NAME)`: the ABI
/// kept an older or shorter name for a protocol constant.
macro_rules! renamed {
    ($g:expr, $ty:literal, $style:ident, $($abi:ident = $($p:ident)::+),+ $(,)?) => {
        $(
            $g.add(
                stringify!($abi),
                $ty,
                Style::$style,
                $($p)::+,
                source_doc(stringify!($($p)::+)),
            );
        )+
    };
}

/// The doc line naming where a value came from, with the import aliases
/// spelled out.
fn source_doc(path: &str) -> String {
    let path: String = path.chars().filter(|c| !c.is_whitespace()).collect();
    let path = if let Some(rest) = path.strip_prefix("gen_::") {
        format!("dspi_proto::generated::{rest}")
    } else if let Some(rest) = path.strip_prefix("consts::") {
        format!("dspi_core::consts::{rest}")
    } else if let Some(rest) = path.strip_prefix("feat::") {
        format!("dspi_proto::wire_compat::feat::{rest}")
    } else {
        path
    };
    format!("`{path}`.")
}

/// A `#define` of a vendored firmware header that dspi-proto's scanner does
/// not lift (its prefix is not one it collects), read the same way: a plain
/// integer or float literal, optionally parenthesised, with a `u` or `f`
/// suffix.
fn firmware_define(root: &Path, header: &str, name: &str) -> Value {
    let path = root.join("crates/dspi-proto/firmware").join(header);
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
    for line in text.lines() {
        let Some(rest) = line.trim_start().strip_prefix("#define") else {
            continue;
        };
        let mut words = rest.split_whitespace();
        if words.next() != Some(name) {
            continue;
        }
        let raw = rest.split("//").next().unwrap_or("");
        let raw = raw.trim().trim_start_matches(name).trim();
        let raw = raw.trim_start_matches('(').trim_end_matches(')');
        if let Some(hex) = raw.strip_prefix("0x").or_else(|| raw.strip_prefix("0X")) {
            if let Ok(v) = i128::from_str_radix(hex.trim_end_matches(['u', 'U']), 16) {
                return Value::Int(v);
            }
        } else if raw.contains('.') {
            if let Ok(v) = raw.trim_end_matches(['f', 'F']).parse::<f64>() {
                return Value::Float(v);
            }
        } else if let Ok(v) = raw.trim_end_matches(['u', 'U']).parse::<i128>() {
            return Value::Int(v);
        }
        panic!("{header}: cannot read the value of {name}: {line}");
    }
    panic!("{header} no longer defines {name}");
}

/// The firmware's filter type name of each type dspi-proto knows. An
/// exhaustive match, so a new variant cannot be left out.
fn filter_name(t: FilterType) -> Option<&'static str> {
    Some(match t {
        FilterType::Flat => "FILTER_FLAT",
        FilterType::Peaking => "FILTER_PEAKING",
        FilterType::LowShelf => "FILTER_LOWSHELF",
        FilterType::HighShelf => "FILTER_HIGHSHELF",
        FilterType::LowPass => "FILTER_LOWPASS",
        FilterType::HighPass => "FILTER_HIGHPASS",
        FilterType::Notch => "FILTER_NOTCH",
        FilterType::AllPass => "FILTER_ALLPASS",
        FilterType::AllPass1 => "FILTER_ALLPASS1",
        FilterType::LowShelf1 => "FILTER_LOWSHELF1",
        FilterType::HighShelf1 => "FILTER_HIGHSHELF1",
        FilterType::LinkwitzTransform => "FILTER_LINKWITZ_TRANSFORM",
        FilterType::LowPass1 => "FILTER_LOWPASS1",
        FilterType::HighPass1 => "FILTER_HIGHPASS1",
        // Crossover types are a family plus an order; the ABI names only
        // their range.
        FilterType::Unknown(_) => return None,
    })
}

/// The ABI's name of a bulk-image section (`dspi_proto::generated::SECTIONS`).
fn section_name(section: &str) -> String {
    let short = match section {
        "i2s_config" => "i2s",
        "crossovers" => "crossover",
        "adat_config" => "adat",
        other => other,
    };
    format!("OFF_{}", short.to_uppercase())
}

/// Names the ABI keeps that no crate exports.
fn not_in_the_protocol(
    opcodes: &mut Vec<Const>,
    usb: &mut Group,
    wire: &mut Group,
    files: &mut Group,
) {
    // Older firmware's per-channel gain and mute. Current firmware reuses
    // 0x54..0x57 for the input-source and device name requests (config.h), so
    // these symbols are kept for the ABI only: sending them to current firmware
    // reads or sets a name. Nothing calls them.
    let channel_ops: [(&str, u8); 4] = [
        ("REQ_SET_CHANNEL_GAIN", 0x54),
        ("REQ_GET_CHANNEL_GAIN", 0x55),
        ("REQ_SET_CHANNEL_MUTE", 0x56),
        ("REQ_GET_CHANNEL_MUTE", 0x57),
    ];
    for (name, v) in channel_ops {
        opcodes.push(Const {
            name: name.to_string(),
            ty: "u8",
            value: v.value(),
            style: Style::Hex,
            doc: "Older firmware's opcode, kept for the ABI; current firmware uses this value \
                  for a name request."
                .to_string(),
        });
    }
    usb.add(
        "LEGACY_VENDOR_ID",
        "u16",
        Style::Hex,
        0x2E8Au16,
        "Raspberry Pi's vendor id, which early DSPi firmware enumerated with.".to_string(),
    );
    usb.add(
        "WINDEX_GLOBAL",
        "u16",
        Style::Dec,
        0u16,
        "`wIndex` of the old ABI's global requests.".to_string(),
    );
    usb.add(
        "WINDEX_OUTPUT",
        "u16",
        Style::Dec,
        2u16,
        "`wIndex` of the old ABI's per-output requests.".to_string(),
    );
    wire.add(
        "BULK_CHUNK_SIZE",
        "usize",
        Style::Dec,
        2048usize,
        "Bytes per chunked bulk-image transfer (`REQ_*_ALL_PARAMS_CHUNK`).".to_string(),
    );
    files.add(
        "SCHEMA_VERSION",
        "i64",
        Style::Dec,
        1i64,
        "`schemaVersion` of the configuration documents dspi-core writes.".to_string(),
    );
}

fn collect(root: &Path) -> Vec<Group> {
    let mut usb = Group::new("usb", "USB identity and control transfers");
    renamed!(
        usb,
        "u16",
        Hex,
        VENDOR_ID = dspi_proto::USB_VID,
        PRODUCT_ID = dspi_proto::USB_PID,
    );
    take!(usb, "u8", Dec, dspi_proto => [VENDOR_INTERFACE]);
    renamed!(usb, "u8", Hex, NOTIFY_ENDPOINT = gen_::usb::VENDOR_EP_IN);
    take!(usb, "u8", Hex, dspi_proto => [REQ_TYPE_OUT, REQ_TYPE_IN]);

    let mut opcodes: Vec<Const> = gen_::ALL_OPCODES
        .iter()
        .filter(|(name, _)| !OPCODES_LEFT_OUT.contains(name))
        .map(|&(name, v)| Const {
            name: name.to_string(),
            ty: "u8",
            value: v.value(),
            style: Style::Hex,
            doc: source_doc(&format!("gen_::opcodes::{name}")),
        })
        .collect();

    let mut status = Group::new("status", "Status bytes and codes the calls return");
    take!(status, "u8", Dec, gen_::status => [
        PIN_CONFIG_SUCCESS,
        PIN_CONFIG_INVALID_PIN,
        PIN_CONFIG_PIN_IN_USE,
        PIN_CONFIG_INVALID_OUTPUT,
        PIN_CONFIG_OUTPUT_ACTIVE,
        PIN_CONFIG_INVALID_PARAM,
    ]);
    if let Value::Int(v) = firmware_define(root, "config.h", "PIN_RESET_TO_DEFAULT") {
        let v = u8::try_from(v).unwrap_or_else(|_| panic!("PIN_RESET_TO_DEFAULT {v}"));
        status.add(
            "PIN_RESET_TO_DEFAULT",
            "u8",
            Style::Hex,
            v,
            "`PIN_RESET_TO_DEFAULT` in the firmware's config.h.".to_string(),
        );
    }
    take!(status, "u8", Dec, gen_::status => [
        PRESET_OK,
        PRESET_ERR_INVALID_SLOT,
        PRESET_ERR_SLOT_EMPTY,
        PRESET_ERR_CRC,
        PRESET_ERR_FLASH_WRITE,
    ]);
    take!(status, "u8", Dec, consts => [
        FLASH_OK,
        FLASH_ERR_WRITE,
        FLASH_ERR_NO_DATA,
        FLASH_ERR_CRC,
        COMPAT_UNKNOWN,
        COMPAT_OK,
        COMPAT_FIRMWARE_TOO_OLD,
        COMPAT_FIRMWARE_TOO_NEW,
    ]);
    take!(status, "u8", Hex, consts => [FW_BETA_EARLY]);

    let mut wire = Group::new("wire", "The bulk parameter image (WireBulkParams)");
    renamed!(
        wire,
        "u8",
        Dec,
        WIRE_FORMAT_VERSION = gen_::wire::WIRE_FORMAT_VERSION
    );
    take!(wire, "u8", Dec, dspi_proto::wire_compat => [MIN_WIRE_FORMAT_VERSION]);
    renamed!(
        wire,
        "u32",
        Bit,
        // -- WP QPI2 -- the three the desktop's effects windows test
        WIRE_FEAT_LEVELLER_MASKS = feat::LEVELLER_MASKS,
        WIRE_FEAT_LOUDNESS_MASK = feat::LOUDNESS_MASK,
        WIRE_FEAT_CROSSFEED_MASK = feat::CROSSFEED_MASK,
        WIRE_FEAT_PSYBASS = feat::PSYBASS,
        WIRE_FEAT_UPMIX = feat::UPMIX,
        WIRE_FEAT_SUBHARM = feat::SUBHARM,
        WIRE_FEAT_TUBE = feat::TUBE,
        WIRE_FEAT_LIMITER = feat::LIMITER,
    );
    renamed!(wire, "usize", Dec, BULK_PARAMS_SIZE = gen_::BULK_SIZE);
    let mut files = Group::new("files", "Configuration and filter files");
    not_in_the_protocol(&mut opcodes, &mut usb, &mut wire, &mut files);
    for &(section, offset, _) in gen_::SECTIONS.iter() {
        wire.add(
            &section_name(section),
            "usize",
            Style::Dec,
            offset,
            format!("Offset of the `{section}` section (`dspi_proto::generated::SECTIONS`)."),
        );
    }

    let mut dims = Group::new("dims", "Array sizes of the records");
    take!(dims, "usize", Dec, consts => [
        MAX_CHANNELS,
        MAX_INPUTS,
        MAX_OUTPUTS,
        BANDS_PER_CHANNEL,
        FIRMWARE_BANDS_PER_CHANNEL,
        MAX_XOVER_BANDS,
    ]);
    take!(dims, "u8", Dec, gen_::filter => [XOVER_BAND_BASE]);
    take!(dims, "usize", Dec, consts => [MAX_PRESETS, CHANNEL_NAME_LEN, MAX_PHYSICAL_OUTPUTS]);

    let mut eq = Group::new("eq", "Filter types and response curves");
    for t in FilterType::all_defined() {
        if let Some(name) = filter_name(t) {
            eq.add(
                name,
                "u8",
                Style::Dec,
                t.to_raw(),
                format!("`dspi_proto::FilterType::{t:?}`."),
            );
        }
    }
    renamed!(
        eq,
        "u8",
        Dec,
        FILTER_XOVER_FIRST = FilterType::XOVER_FIRST,
        FILTER_XOVER_LAST = FilterType::XOVER_LAST,
    );
    take!(eq, "f32", Dec, consts => [DEFAULT_Q]);
    take!(eq, "f64", Dec, dspi_proto::dsp => [SAMPLE_RATE]);
    renamed!(eq, "usize", Dec, MAGNITUDE_POINTS = dspi_core::CURVE_POINTS);

    let mut volume = Group::new("volume", "Master volume");
    if let Value::Float(v) = firmware_define(root, "config.h", "MASTER_VOL_MUTE_DB") {
        volume.add(
            "MASTER_VOL_MUTE_DB",
            "f32",
            Style::Dec,
            v,
            "`MASTER_VOL_MUTE_DB` in the firmware's config.h.".to_string(),
        );
    }

    let mut effects = Group::new("effects", "Parameter ids of the effect blocks");
    take!(effects, "u8", Dec, consts => [
        PSYBASS_PARAM_CUTOFF,
        PSYBASS_PARAM_HARMONICS,
        PSYBASS_PARAM_DRIVE,
        PSYBASS_PARAM_CHARACTER,
        PSYBASS_PARAM_ORIGINAL,
        UPMIX_PARAM_ENABLED,
        UPMIX_PARAM_CENTER_MODE,
        UPMIX_PARAM_SURROUND_MODE,
        UPMIX_PARAM_STRENGTH,
        UPMIX_PARAM_WIDTH,
        UPMIX_PARAM_THRESHOLD,
        UPMIX_PARAM_ATTACK,
        UPMIX_PARAM_RELEASE,
        UPMIX_PARAM_DETECTOR_HPF,
        UPMIX_PARAM_SURROUND_DELAY,
        UPMIX_PARAM_SURROUND_HPF,
        UPMIX_PARAM_SURROUND_LPF,
        UPMIX_PARAM_DECORRELATION,
        UPMIX_PARAM_PRESENCE,
    ]);
    take!(effects, "usize", Dec, consts => [UPMIX_PARAM_COUNT]);
    take!(effects, "u8", Dec, consts => [
        SUBHARM_PARAM_ENABLED,
        SUBHARM_PARAM_MASK,
        SUBHARM_PARAM_LOW,
        SUBHARM_PARAM_HIGH,
        SUBHARM_PARAM_TOP,
        SUBHARM_PARAM_BOOST,
        SUBHARM_PARAM_SELECT,
        SUBHARM_PARAM_DEPTH,
        SUBHARM_PARAM_HOLD,
        SUBHARM_PARAM_CEILING,
        SUBHARM_PARAM_LINK,
    ]);
    take!(effects, "usize", Dec, consts => [SUBHARM_PARAM_COUNT]);
    renamed!(
        effects,
        "u8",
        Dec,
        TUBE_PARAM_ENABLED = gen_::tube::TUBE_PARAM_ENABLED,
        TUBE_PARAM_MASK = gen_::tube::TUBE_PARAM_OUTPUT_MASK,
        TUBE_PARAM_TYPE = gen_::tube::TUBE_PARAM_TUBE_TYPE,
        TUBE_PARAM_DRIVE = gen_::tube::TUBE_PARAM_DRIVE_DB,
        TUBE_PARAM_BIAS = gen_::tube::TUBE_PARAM_BIAS_PCT,
        TUBE_PARAM_ASYM = gen_::tube::TUBE_PARAM_ASYM_DB,
        TUBE_PARAM_HARDNESS = gen_::tube::TUBE_PARAM_HARDNESS_PCT,
        TUBE_PARAM_SAG = gen_::tube::TUBE_PARAM_SAG_PCT,
        TUBE_PARAM_RECTIFIER = gen_::tube::TUBE_PARAM_RECTIFIER,
        TUBE_PARAM_XFMR = gen_::tube::TUBE_PARAM_XFMR_ENABLED,
        TUBE_PARAM_DAMPING = gen_::tube::TUBE_PARAM_XFMR_DAMPING,
        TUBE_PARAM_RESONANCE = gen_::tube::TUBE_PARAM_XFMR_RES_HZ,
        TUBE_PARAM_MIX = gen_::tube::TUBE_PARAM_MIX_PCT,
        TUBE_PARAM_TRIM = gen_::tube::TUBE_PARAM_TRIM_DB,
    );
    take!(effects, "usize", Dec, consts => [TUBE_PARAM_COUNT]);
    take!(effects, "u8", Dec, gen_::limiter => [
        LIMITER_PARAM_ENABLED,
        LIMITER_PARAM_THRESHOLD_DB,
        LIMITER_PARAM_RELEASE_MS,
        LIMITER_PARAM_LINK_GROUP,
    ]);
    take!(effects, "u8", Hex, gen_::limiter => [
        LIMITER_GET_METER,
        LIMITER_GET_STATUS,
        LIMITER_ALL_OUTPUTS,
    ]);

    let mut rta = Group::new("rta", "The analyser");
    take!(rta, "u8", Dec, dspi_proto::packets => [RTA_VERSION]);
    take!(rta, "usize", Dec, consts => [RTA_MAX_BANDS, RTA_MAX_CHANNELS, RTA_MAX_BINS]);
    take!(rta, "u8", Dec, consts => [RTA_TAP_INPUT, RTA_TAP_OUTPUT]);
    take!(rta, "f32", Dec, consts => [RTA_FLOOR_DB]);

    let mut siggen = Group::new("siggen", "The signal generator");
    take!(siggen, "u8", Dec, consts => [SIGGEN_VERSION]);
    take!(siggen, "usize", Dec, consts => [SIGGEN_TYPE_COUNT]);
    take!(siggen, "u8", Hex, consts => [SIGGEN_FLAG_RAW, SIGGEN_FLAG_DECORR, SIGGEN_FLAG_WALK]);
    take!(siggen, "u16", Dec, consts => [SIGGEN_CTL_STOP, SIGGEN_CTL_START, SIGGEN_CTL_STOP_NOW]);

    let mut stats = Group::new("stats", "Statistics ids (`dspi_fetch_stat`)");
    take!(stats, "u16", Dec, consts => [
        STAT_PDM_RING_OVERRUNS,
        STAT_PDM_RING_UNDERRUNS,
        STAT_PDM_DMA_OVERRUNS,
        STAT_PDM_DMA_UNDERRUNS,
        STAT_SPDIF_OVERRUNS,
        STAT_SPDIF_UNDERRUNS,
        STAT_CLOCK_HZ,
        STAT_CORE_MV,
        STAT_SAMPLE_RATE,
        STAT_TEMPERATURE,
        STAT_STARVATION_TOTAL,
        STAT_STARVATION_FIRST,
        STAT_USB_RING_OVERRUNS,
    ]);

    let mut notify = Group::new("notify", "Bits of `NotifyResult::flags`");
    take!(notify, "u32", Bit, consts => [
        NOTIFY_STATE,
        NOTIFY_CURVES,
        NOTIFY_REFRESHED,
        NOTIFY_INPUT_FORMAT,
        NOTIFY_PRESET,
        NOTIFY_SIGGEN,
        NOTIFY_CS_AUX,
        NOTIFY_IR_LEARN,
    ]);

    take!(files, "u32", Dec, dspi_session::filterfile => [FORMAT_VERSION]);
    take!(files, "u32", Dec, consts => [IMPORT_VOLUMES, IMPORT_HARDWARE]);

    let mut cs = Group::new("cs", "Control surface table sizes");
    renamed!(
        cs,
        "usize",
        Dec,
        CS_MAX_BINDINGS = gen_::cs::CS_MAX_BINDINGS,
        CS_MAX_IR = gen_::cs::CS_MAX_IR_COMMANDS,
        CS_MAX_GROUPS = gen_::cs::CS_MAX_GROUPS,
        CS_MAX_MACROS = gen_::cs::CS_MAX_MACROS,
        CS_MAX_STEPS = gen_::cs::CS_MAX_MACRO_STEPS,
        CS_MAX_PAGES = gen_::cs::CS_MAX_DISPLAY_PAGES,
    );

    // Opcodes in wire order; equal values (none today) by name.
    opcodes.sort_by(|a, b| int_of(a).cmp(&int_of(b)).then_with(|| a.name.cmp(&b.name)));
    let mut ops = Group::new("opcodes", "Vendor request opcodes (bRequest)");
    ops.items = opcodes;

    vec![
        usb, ops, status, wire, dims, eq, volume, effects, rta, siggen, stats, notify, files, cs,
    ]
}

fn int_of(c: &Const) -> i128 {
    match c.value {
        Value::Int(v) => v,
        Value::Float(_) => panic!("{} is not an integer", c.name),
    }
}

/// The literal for `c`, checked against its type.
fn literal(c: &Const) -> String {
    let range = |lo: i128, hi: i128| {
        let v = int_of(c);
        assert!(
            (lo..=hi).contains(&v),
            "{} = {v} does not fit {}",
            c.name,
            c.ty
        );
        v
    };
    let v = match c.ty {
        "u8" => range(0, u8::MAX.into()),
        "u16" => range(0, u16::MAX.into()),
        "u32" => range(0, u32::MAX.into()),
        "usize" => range(0, u32::MAX.into()),
        "i64" => range(i64::MIN.into(), i64::MAX.into()),
        "f32" | "f64" => {
            let Value::Float(f) = c.value else {
                panic!("{} is {} but not a float", c.name, c.ty)
            };
            assert!(f.is_finite(), "{} is not finite", c.name);
            // f32 values print as the shortest f32 spelling, so a value that
            // came through f64 does not grow digits.
            #[allow(clippy::cast_possible_truncation)]
            let text = if c.ty == "f32" {
                format!("{:?}", f as f32)
            } else {
                format!("{f:?}")
            };
            return text;
        }
        other => panic!("{}: no literal rule for type {other}", c.name),
    };
    match c.style {
        Style::Dec => v.to_string(),
        Style::Hex if c.ty == "u16" => format!("0x{v:04X}"),
        Style::Hex => format!("0x{v:02X}"),
        Style::Bit => {
            assert!(
                v > 0 && v.count_ones() == 1,
                "{} = {v} is not a single bit",
                c.name
            );
            format!("1 << {}", v.trailing_zeros())
        }
    }
}

/// The whole file.
fn render(groups: &[Group]) -> String {
    let mut seen = std::collections::BTreeSet::new();
    let mut out = String::new();
    let _ = writeln!(
        out,
        "// Generated, do not edit: regenerate with `{COMMAND}`."
    );
    out.push_str(
        "// The values come from dspi-proto (the firmware headers), dspi-core and\n\
         // dspi-session; ci/drift.sh fails when this file is stale.\n\
         \n\
         //! The C ABI's constants: each is a `#define` in `dspi_core.h`.\n\
         \n\
         // `1 << 0` spells a flag like its neighbours.\n\
         #![allow(clippy::identity_op)]\n",
    );
    for g in groups {
        let _ = writeln!(out, "\n// ── {} ── {}", g.key, g.title);
        for c in &g.items {
            assert!(seen.insert(c.name.clone()), "{} is defined twice", c.name);
            let _ = writeln!(out, "\n/// {}", c.doc);
            let _ = writeln!(out, "pub const {}: {} = {};", c.name, c.ty, literal(c));
        }
    }
    out
}

pub fn run(root: &Path, check: bool) -> ExitCode {
    let text = render(&collect(root));
    let path = root.join(OUT);
    let current = std::fs::read_to_string(&path).unwrap_or_default();
    if check {
        if current == text {
            println!("capi-consts: {OUT} is current");
            return ExitCode::SUCCESS;
        }
        eprintln!("capi-consts: {OUT} differs from what the sources generate; run `{COMMAND}`");
        return ExitCode::FAILURE;
    }
    if current != text {
        if let Err(e) = std::fs::write(&path, &text) {
            eprintln!("capi-consts: cannot write {}: {e}", path.display());
            return ExitCode::FAILURE;
        }
        println!("capi-consts: wrote {OUT}");
    } else {
        println!("capi-consts: {OUT} unchanged");
    }
    ExitCode::SUCCESS
}
