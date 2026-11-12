//! WP P0U: the registry's wording, pinned. `docs/GLOSSARY.md` is the source of
//! truth for terms and units; every client takes its labels from these rows, so a
//! changed label or unit is a change in every client and must be made on purpose:
//! update the table below and the glossary together.

use dspi_proto::registry::{Kind, ParamDesc, REGISTRY};
use dspi_proto::value::Unit;

/// Every row's path, label and unit (the `Unit` name for a number, else the kind),
/// in registry order.
const PINNED: &[(&str, &str, &str)] = &[
    ("vol.user", "Volume", "Db"),
    ("vol.mute", "Mute", "bool"),
    ("vol.master", "Master volume", "Db"),
    ("vol.master.mode", "Master volume storage", "choice"),
    ("vol.master.save", "Save master volume", "trigger"),
    ("pre", "Input trim", "Db"),
    ("pre.all", "Input trim, all", "Db"),
    ("bypass", "EQ bypass", "bool"),
    ("eq.type", "Filter type", "choice"),
    ("eq.freq", "Frequency", "Hz"),
    ("eq.q", "Q", "Q"),
    ("eq.gain", "Gain", "Db"),
    ("eq.fp", "Target frequency", "Hz"),
    ("eq.qp", "Target Q", "Q"),
    ("eq.bypass", "Band bypass", "bool"),
    ("ch.delay", "Delay", "Ms"),
    ("ch.name", "Channel name", "text"),
    ("mix", "Crosspoint", "packet"),
    ("out.enable", "Output enabled", "bool"),
    ("out.gain", "Output gain", "Db"),
    ("out.mute", "Output mute", "bool"),
    ("out.delay", "Output delay", "Ms"),
    ("limit.on", "Output Limiter", "bool"),
    ("limit.threshold", "Threshold", "Dbfs"),
    ("limit.release", "Release", "Ms"),
    ("limit.link", "Link group", "None"),
    ("limit.meter", "Limiter gain reduction", "status"),
    ("limit.status", "Limiter status", "status"),
    ("out.pin", "Output GPIO", "Gpio"),
    ("out.type", "Output bus", "choice"),
    ("loud.on", "Loudness", "bool"),
    ("loud.ref", "Reference level", "Spl"),
    ("loud.intensity", "Intensity", "Percent"),
    ("loud.mask", "Loudness outputs", "mask"),
    ("lev.on", "Leveller", "bool"),
    ("lev.amount", "Amount", "Percent"),
    ("lev.speed", "Speed", "choice"),
    ("lev.maxgain", "Maximum boost", "Db"),
    ("lev.lookahead", "Lookahead", "bool"),
    ("lev.gate", "Silence gate", "Dbfs"),
    ("lev.masks", "Leveller channels", "mask"),
    ("cf.on", "Crossfeed", "bool"),
    ("cf.preset", "Voicing", "choice"),
    ("cf.freq", "Cutoff frequency", "Hz"),
    ("cf.feed", "Feed level", "Db"),
    ("cf.itd", "Interaural delay", "bool"),
    ("cf.outputs", "Crossfeed outputs", "mask"),
    ("bass.on", "Psychoacoustic Bass", "bool"),
    ("bass.cutoff", "Speaker limit", "Hz"),
    ("bass.harmonics", "Harmonics", "Db"),
    ("bass.drive", "Drive", "Db"),
    ("bass.character", "Character", "Percent"),
    ("bass.original", "Original bass", "Db"),
    ("bass.mask", "Bass outputs", "mask"),
    ("up.on", "Upmixer", "bool"),
    ("up.center_mode", "Centre mode", "choice"),
    ("up.surround_mode", "Surround mode", "choice"),
    ("up.strength", "Strength", "Percent"),
    ("up.width", "Centre width", "Percent"),
    ("up.threshold", "Correlation threshold", "Percent"),
    ("up.attack", "Attack", "Ms"),
    ("up.release", "Release", "Ms"),
    ("up.det_hpf", "Detector high-pass", "Hz"),
    ("up.sur_delay", "Surround delay", "Ms"),
    ("up.sur_hpf", "Surround high-pass", "Hz"),
    ("up.sur_lpf", "Surround low-pass", "Hz"),
    ("up.decorr", "Decorrelation", "Percent"),
    ("up.presence", "Centre presence", "Db"),
    ("up.config", "Upmixer config", "packet"),
    ("up.status", "Upmixer status", "status"),
    ("sub.on", "Subharmonic Synthesizer", "bool"),
    ("sub.low", "24 - 36 Hz", "Db"),
    ("sub.high", "36 - 56 Hz", "Db"),
    ("sub.top", "56 - 80 Hz", "Db"),
    ("sub.boost", "70 Hz bell", "Db"),
    ("sub.mask", "Subharmonic outputs", "mask"),
    ("sub.select", "Selectivity", "choice"),
    ("sub.depth", "Depth", "Percent"),
    ("sub.hold", "Hold", "Ms"),
    ("sub.ceiling", "Sub ceiling", "Dbfs"),
    ("sub.link", "Link output pairs", "bool"),
    ("sub.solo", "Solo", "bool"),
    ("sub.headroom", "Headroom cost", "status"),
    ("sub.meter", "Sub meters", "status"),
    ("tube.on", "Tube Modeller", "bool"),
    ("tube.mask", "Tube outputs", "mask"),
    ("tube.type", "Tube", "None"),
    ("tube.drive", "Drive", "Db"),
    ("tube.bias", "Bias", "Percent"),
    ("tube.asym", "Asymmetry", "Db"),
    ("tube.hardness", "Knee hardness", "Percent"),
    ("tube.sag", "Sag", "Percent"),
    ("tube.rectifier", "Rectifier", "choice"),
    ("tube.xfmr", "Output stage", "bool"),
    ("tube.damping", "Damping factor", "None"),
    ("tube.resonance", "Speaker resonance", "Hz"),
    ("tube.mix", "Mix", "Percent"),
    ("tube.trim", "Output trim", "Db"),
    ("in.source", "Input source", "choice"),
    ("in.name", "Source name", "text"),
    ("in.rate", "Sample rate", "Hz"),
    ("in.spdif.pin", "S/PDIF input GPIO", "Gpio"),
    ("in.spdif.enable", "Extra S/PDIF inputs", "bool"),
    ("in.spdif.status", "S/PDIF lock", "status"),
    ("in.spdif.channel", "S/PDIF channel status", "status"),
    ("in.i2s.pin", "I2S input GPIO", "Gpio"),
    ("in.i2s.channels", "I2S input channels", "None"),
    ("in.i2s.clock", "I2S clock", "choice"),
    ("in.i2s.slave.status", "I2S slave status", "status"),
    ("in.adat.enable", "ADAT input", "bool"),
    ("in.adat.pin", "ADAT input GPIO", "Gpio"),
    ("in.adat.clock", "ADAT clock", "choice"),
    ("in.adat.status", "ADAT input status", "status"),
    ("in.lg", "LG Sound Sync", "bool"),
    ("in.lg.status", "LG Sound Sync status", "status"),
    ("i2s.bck", "I2S bit clock GPIO", "Gpio"),
    ("i2s.bck.slave", "I2S slave bit clock GPIO", "Gpio"),
    ("i2s.clockpins", "I2S clock pins", "choice"),
    ("i2s.mck", "Master clock", "bool"),
    ("i2s.mck.pin", "Master clock GPIO", "Gpio"),
    ("i2s.mck.mult", "Master clock rate", "choice"),
    ("adat.enable", "ADAT output", "bool"),
    ("adat.pin", "ADAT output GPIO", "Gpio"),
    ("adat.status", "ADAT output status", "status"),
    ("dev.dacmute", "DAC hardware mute", "packet"),
    ("dev.dacmute.test", "Test DAC mute", "trigger"),
    ("dev.uart", "UART control", "packet"),
    ("dev.i2c", "I2C control", "packet"),
    ("dev.ctrl.status", "Control interface status", "status"),
    ("dev.reset", "Factory reset", "trigger"),
    ("dev.bootloader", "Firmware update mode", "trigger"),
    ("dev.save", "Save settings (legacy)", "trigger"),
    ("dev.save.io", "Save I/O config", "trigger"),
    ("dev.serial", "Serial number", "status"),
    ("dev.name", "Device name", "text"),
    ("out.latency", "Output latency", "choice"),
    ("dev.platform", "Platform", "status"),
    ("dev.build", "Build", "status"),
    ("preset.save", "Save preset", "trigger"),
    ("preset.load", "Load preset", "trigger"),
    ("preset.delete", "Delete preset", "trigger"),
    ("preset.name", "Preset name", "text"),
    ("preset.startup", "Startup preset", "packet"),
    ("preset.dir", "Preset directory", "status"),
    ("preset.iomode", "I/O storage", "choice"),
    ("cs.binding", "Control binding", "packet"),
    ("cs.name", "Binding name", "text"),
    ("cs.ir", "IR command", "packet"),
    ("cs.ir.learn", "Learn a remote button", "trigger"),
    ("cs.save", "Save control surfaces", "trigger"),
    ("cs.revert", "Discard changes", "trigger"),
    ("cs.caps", "Control surface capabilities", "status"),
    ("cs.status", "Control surface status", "status"),
    ("cs.aux", "Auxiliary output", "bool"),
    ("cs.aux.level", "Auxiliary output level", "Percent"),
    ("cs.aux.all", "Auxiliary outputs", "status"),
    ("cs.group", "Channel group", "packet"),
    ("cs.macro", "Macro", "packet"),
    ("cs.macro.step", "Macro step", "packet"),
    ("cs.macro.fire", "Run a macro", "trigger"),
    ("cs.ext.status", "Group and macro status", "status"),
    ("cs.display", "Display settings", "packet"),
    ("cs.display.page", "Display page", "packet"),
    ("cs.display.status", "Display status", "status"),
    ("sig.config", "Signal generator", "packet"),
    ("sig.control", "Start or stop", "choice"),
    ("sig.caps", "Signal generator capabilities", "status"),
    ("rta.config", "Analyser configuration", "packet"),
    ("rta.caps", "Analyser capabilities", "status"),
    ("rta.bands", "Analyser bands", "status"),
    ("rta.bands.all", "Analyser bands, every channel", "status"),
    ("rta.bins", "Analyser bins", "status"),
    ("rta.status", "Analyser status", "status"),
    ("rta.control", "Analyser run state", "choice"),
    ("meters", "Meters", "status"),
    ("meters.clear", "Clear clip indicators", "trigger"),
    ("diag.buffers", "Buffer statistics", "status"),
    ("diag.buffers.reset", "Reset watermarks", "trigger"),
    ("diag.usb", "USB error counters", "status"),
    ("diag.usb.reset", "Reset USB counters", "trigger"),
    ("diag.core1", "Second core", "status"),
    ("diag.core1.conflict", "Core conflict check", "status"),
    ("diag.clock", "Clock diagnostics", "status"),
    ("ext.list", "Extensions", "status"),
    ("bulk", "All settings", "packet"),
    ("bulk.chunk", "All settings, chunked", "packet"),
];

fn unit_or_kind(d: &ParamDesc) -> String {
    match d.kind {
        Kind::Float { unit, .. } | Kind::Int { unit, .. } => format!("{unit:?}"),
        Kind::Bool => "bool".into(),
        Kind::Trigger => "trigger".into(),
        Kind::Choice(_) => "choice".into(),
        Kind::Mask => "mask".into(),
        Kind::Text { .. } => "text".into(),
        Kind::Packet => "packet".into(),
        Kind::Status => "status".into(),
    }
}

fn words(text: &str) -> Vec<String> {
    text.to_lowercase()
        .split(|c: char| !c.is_ascii_alphanumeric() && c != '\'')
        .map(|w| w.trim_end_matches("'s").to_string())
        .filter(|w| !w.is_empty())
        .collect()
}

fn glossary() -> String {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../docs/GLOSSARY.md");
    std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{path}: {e}"))
}

#[test]
fn wp_p0u_labels_and_units_are_pinned() {
    let now: Vec<(String, String, String)> = REGISTRY
        .iter()
        .map(|d| (d.path.into(), d.label.into(), unit_or_kind(d)))
        .collect();
    let pinned: Vec<(String, String, String)> = PINNED
        .iter()
        .map(|(p, l, u)| ((*p).into(), (*l).into(), (*u).into()))
        .collect();
    for (a, b) in now.iter().zip(&pinned) {
        assert_eq!(a, b, "a label or unit changed: update docs/GLOSSARY.md too");
    }
    assert_eq!(now.len(), pinned.len(), "a row was added or removed");
}

/// A level against full scale is dBFS; a gain, boost, cut or trim is dB.
#[test]
fn wp_p0u_full_scale_levels_are_dbfs() {
    for path in ["limit.threshold", "lev.gate", "sub.ceiling"] {
        let d = dspi_proto::registry::by_path(path).unwrap();
        assert_eq!(d.kind.unit(), Unit::Dbfs, "{path}");
    }
    for d in REGISTRY {
        if let Kind::Float {
            unit: Unit::Dbfs,
            max,
            ..
        } = d.kind
        {
            assert!(max <= 0.0, "{}: a dBFS level is at most 0", d.path);
        }
        if d.kind.unit() == Unit::Db {
            assert!(
                !d.plain.contains("dBFS"),
                "{}: says dBFS, unit is dB",
                d.path
            );
        }
    }
    assert_eq!(Unit::Dbfs.suffix(), " dBFS");
}

/// Q is "Q": never "Width" or "Bandwidth" (the upmixer's centre width is a
/// different thing, a percentage).
#[test]
fn wp_p0u_q_is_called_q() {
    for d in REGISTRY {
        if d.kind.unit() == Unit::Q {
            assert!(d.label.ends_with('Q'), "{}: {}", d.path, d.label);
        }
        let w = words(d.label);
        assert!(!w.iter().any(|w| w == "bandwidth"), "{}", d.path);
        if w.iter().any(|w| w == "width") {
            assert_eq!(d.kind.unit(), Unit::Percent, "{}: width is not Q", d.path);
        }
    }
}

/// Times a user sets are milliseconds. Samples appear only where the firmware
/// fixes a latency in samples, and then with the milliseconds beside them.
#[test]
fn wp_p0u_times_are_milliseconds() {
    for path in [
        "ch.delay",
        "out.delay",
        "up.sur_delay",
        "up.attack",
        "up.release",
        "limit.release",
        "sub.hold",
    ] {
        let d = dspi_proto::registry::by_path(path).unwrap();
        assert_eq!(d.kind.unit(), Unit::Ms, "{path}");
    }
    for d in REGISTRY {
        if words(d.plain).iter().any(|w| w == "samples") {
            assert!(d.plain.contains(" ms"), "{}: samples without ms", d.path);
        }
    }
}

/// Owner rule: user-visible text never names the host device (the word list is
/// Android's `NoHostNounsAn33Test`, plus "host"). "Device" means the DSPi.
#[test]
fn wp_p0u_no_host_device_nouns() {
    const HOST: &[&str] = &[
        "phone",
        "phones",
        "tablet",
        "tablets",
        "computer",
        "computers",
        "mac",
        "macs",
        "pc",
        "pcs",
        "iphone",
        "ipad",
        "laptop",
        "laptops",
        "smartphone",
        "handset",
        "host",
        "hosts",
    ];
    for d in REGISTRY {
        for text in [d.label, d.plain] {
            let hit = words(text).into_iter().find(|w| HOST.contains(&w.as_str()));
            assert!(hit.is_none(), "{}: {text:?} names the host", d.path);
        }
    }
}

/// One word per concept, in the glossary's spelling.
#[test]
fn wp_p0u_one_word_per_concept() {
    // Synonyms the glossary replaces, and American spellings it does not use.
    // "Synthesizer" is the firmware's name for the feature and stays.
    const NOT: &[&str] = &[
        "preamp",
        "pre-gain",
        "bandwidth",
        "look ahead",
        "high pass",
        "low pass",
        "center",
        "leveler",
        "modeler",
        "equalizer",
        "analyzer",
        "color",
        "nerbs",
    ];
    for d in REGISTRY {
        for text in [d.label, d.plain] {
            let lower = text.to_lowercase();
            for n in NOT {
                assert!(!lower.contains(n), "{}: {text:?} says {n:?}", d.path);
            }
        }
    }
    // "Input" alone is a channel; where audio comes from is the input source.
    let source = dspi_proto::registry::by_path("in.source").unwrap();
    assert_eq!(source.label, "Input source");
}

/// Labels are sentence case without a full stop; plain lines are one line without
/// one. Feature names and acronyms keep their capitals.
#[test]
fn wp_p0u_wording_style() {
    for d in REGISTRY {
        for text in [d.label, d.plain] {
            assert!(!text.is_empty(), "{}", d.path);
            assert!(
                !text.ends_with('.'),
                "{}: {text:?} ends with a full stop",
                d.path
            );
            assert!(!text.contains("  ") && !text.contains('\n'), "{}", d.path);
            assert_eq!(text.trim(), text, "{}", d.path);
        }
        let first = d.label.chars().next().unwrap();
        assert!(first.is_uppercase() || first.is_ascii_digit(), "{}", d.path);
    }
}

/// The glossary names every feature and every unit the registry uses.
#[test]
fn wp_p0u_glossary_names_every_feature_and_unit() {
    let g = glossary();
    for feature in [
        "Output Limiter",
        "Loudness",
        "Leveller",
        "Crossfeed",
        "Psychoacoustic Bass",
        "Upmixer",
        "Subharmonic Synthesizer",
        "Tube Modeller",
    ] {
        assert!(REGISTRY.iter().any(|d| d.label == feature), "{feature}");
        assert!(
            g.contains(&format!("**{feature}**")),
            "glossary lacks {feature}"
        );
    }
    for unit in [
        Unit::Db,
        Unit::Dbfs,
        Unit::Hz,
        Unit::Percent,
        Unit::Ms,
        Unit::Spl,
    ] {
        let s = unit.suffix().trim();
        assert!(g.contains(&format!("`{s}`")), "glossary lacks the unit {s}");
    }
    for d in REGISTRY {
        if d.kind.unit() == Unit::Q {
            assert!(g.contains("**Q**"), "glossary lacks Q");
        }
    }
}
