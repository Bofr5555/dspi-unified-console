//! Linux: PipeWire's object dump (`pw-dump`), read as JSON.
//!
//! [`parse`] is pure: the sinks (`Audio/Sink` nodes) with their device's
//! serial, the default sink from the `default` metadata, each sink's format
//! and volume from its `Format` and `Props` params, and the app streams
//! (`Stream/Output/Audio` nodes) on the sink their link goes to. [`PwDump`]
//! runs `pw-dump` with a time limit and falls back to ALSA
//! ([`super::alsa`]) when there is no PipeWire.

use std::collections::BTreeMap;

use serde_json::Value;

use super::{HostAudio, HostOs, HostOutput, HostStream, cubic_pct};

const NODE: &str = "PipeWire:Interface:Node";
const DEVICE: &str = "PipeWire:Interface:Device";
const LINK: &str = "PipeWire:Interface:Link";
const METADATA: &str = "PipeWire:Interface:Metadata";
const CORE: &str = "PipeWire:Interface:Core";

fn props(o: &Value) -> Option<&serde_json::Map<String, Value>> {
    o.pointer("/info/props").and_then(Value::as_object)
}

fn prop_str<'a>(o: &'a Value, key: &str) -> &'a str {
    props(o)
        .and_then(|p| p.get(key))
        .and_then(Value::as_str)
        .unwrap_or("")
}

/// A number prop: PipeWire writes some as numbers, some as text.
fn prop_u32(o: &Value, key: &str) -> Option<u32> {
    let v = props(o)?.get(key)?;
    v.as_u64()
        .and_then(|n| u32::try_from(n).ok())
        .or_else(|| v.as_str().and_then(|s| s.trim().parse().ok()))
}

fn id(o: &Value) -> Option<u64> {
    o.get("id").and_then(Value::as_u64)
}

/// The first object of param `name` (`Format`, `Props`) that has `key`.
fn param<'a>(o: &'a Value, name: &str, key: &str) -> Option<&'a Value> {
    o.pointer(&format!("/info/params/{name}"))
        .and_then(Value::as_array)?
        .iter()
        .find_map(|p| p.get(key))
}

fn param_u32(o: &Value, name: &str, key: &str) -> Option<u32> {
    param(o, name, key)
        .and_then(Value::as_u64)
        .and_then(|n| u32::try_from(n).ok())
}

/// Per-channel volumes, linear.
fn volumes(o: &Value, key: &str) -> Option<Vec<f32>> {
    #[allow(clippy::cast_possible_truncation)]
    let v: Vec<f32> = param(o, "Props", key)?
        .as_array()?
        .iter()
        .filter_map(Value::as_f64)
        .map(|x| x as f32)
        .collect();
    (!v.is_empty()).then_some(v)
}

fn mean(v: &[f32]) -> f32 {
    #[allow(clippy::cast_precision_loss)]
    let n = v.len().max(1) as f32;
    v.iter().sum::<f32>() / n
}

/// Whether a sink is an ALSA card's (where a USB DSPi's sink always is),
/// and whether it was told to use a software volume only.
fn alsa_of(node: &Value, device: Option<&&Value>) -> (bool, bool) {
    let is_alsa = |o: &Value| {
        prop_str(o, "device.api") == "alsa"
            || props(o).is_some_and(|p| p.keys().any(|k| k.starts_with("api.alsa.")))
    };
    let alsa = is_alsa(node) || device.is_some_and(|d| is_alsa(d));
    let soft_only = props(node)
        .and_then(|p| p.get("api.alsa.soft-mixer"))
        .is_some_and(|v| v.as_bool() == Some(true) || v.as_str() == Some("true"));
    (alsa, soft_only)
}

/// The OS volume as percent, muted, and whether it is the device's own
/// control. Only an ALSA card has one: there PipeWire moves the hardware
/// mixer and leaves the soft volumes at 1, or, with no mixer (or told to use
/// none), scales the samples itself and the soft volumes follow the channel
/// volumes. All at 1 tells neither. Any other sink (a null sink, Bluetooth,
/// a virtual one) has no USB volume to drive.
fn volume_of(o: &Value, alsa: (bool, bool)) -> (Option<f32>, Option<bool>, Option<bool>) {
    let muted = param(o, "Props", "mute").and_then(Value::as_bool);
    let Some(ch) = volumes(o, "channelVolumes") else {
        return (None, muted, None);
    };
    let near = |a: f32, b: f32| (a - b).abs() < 1e-4;
    let all_one = |v: &[f32]| v.iter().all(|&x| near(x, 1.0));
    let hardware = match alsa {
        (false, _) | (true, true) => Some(false),
        (true, false) if all_one(&ch) => None,
        (true, false) => volumes(o, "softVolumes").map(|soft| {
            let same = soft.len() == ch.len() && soft.iter().zip(&ch).all(|(a, b)| near(*a, *b));
            !same
        }),
    };
    (cubic_pct(mean(&ch)), muted, hardware)
}

/// The default sink's `node.name` from the `default` metadata.
fn default_sink(objects: &[Value]) -> Option<String> {
    let meta = objects.iter().find(|o| {
        // pw-dump puts a metadata object's props at the top, not in info
        let name = o
            .pointer("/props/metadata.name")
            .and_then(Value::as_str)
            .unwrap_or_else(|| prop_str(o, "metadata.name"));
        o.get("type").and_then(Value::as_str) == Some(METADATA) && name == "default"
    })?;
    let entries = meta.get("metadata")?.as_array()?;
    ["default.audio.sink", "default.configured.audio.sink"]
        .iter()
        .find_map(|key| {
            let e = entries
                .iter()
                .find(|e| e.get("key").and_then(Value::as_str) == Some(*key))?;
            let v = e.get("value")?;
            // A JSON object, or (older dumps) its text
            match v {
                Value::Object(_) => v.get("name")?.as_str().map(str::to_owned),
                Value::String(s) => serde_json::from_str::<Value>(s)
                    .ok()?
                    .get("name")?
                    .as_str()
                    .map(str::to_owned),
                _ => None,
            }
        })
}

/// Read a `pw-dump` document. `Err` with the reason when it is not one.
pub fn parse(dump: &str) -> Result<HostAudio, String> {
    let doc: Value =
        serde_json::from_str(dump).map_err(|_| "PipeWire's dump could not be read".to_owned())?;
    let objects = doc
        .as_array()
        .ok_or_else(|| "PipeWire's dump is not a list of objects".to_owned())?;
    let of = |t: &'static str| {
        objects
            .iter()
            .filter(move |o| o.get("type").and_then(Value::as_str) == Some(t))
    };
    let version = of(CORE)
        .find_map(|c| c.pointer("/info/version").and_then(Value::as_str))
        .map_or_else(|| "PipeWire".to_owned(), |v| format!("PipeWire {v}"));
    let devices: BTreeMap<u64, &Value> = of(DEVICE).filter_map(|d| Some((id(d)?, d))).collect();
    let default = default_sink(objects);

    let mut outputs = Vec::new();
    let mut sink_ids: Vec<u64> = Vec::new();
    for node in of(NODE).filter(|n| prop_str(n, "media.class") == "Audio/Sink") {
        let Some(node_id) = id(node) else { continue };
        let device = prop_u32(node, "device.id").and_then(|d| devices.get(&u64::from(d)));
        let serial = device
            .map(|d| prop_str(d, "device.serial"))
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| prop_str(node, "device.serial"));
        let name_text = [
            prop_str(node, "node.description"),
            prop_str(node, "node.nick"),
        ]
        .into_iter()
        .find(|s| !s.is_empty())
        .unwrap_or_else(|| prop_str(node, "node.name"));
        let node_name = prop_str(node, "node.name");
        let state = node.pointer("/info/state").and_then(Value::as_str);
        let (volume_pct, muted, hardware_volume) = volume_of(node, alsa_of(node, device));
        outputs.push(HostOutput {
            id: node_name.to_owned(),
            name: name_text.to_owned(),
            serial: serial.to_owned(),
            is_default: Some(default.as_deref() == Some(node_name)),
            channels: param_u32(node, "Format", "channels")
                .or_else(|| prop_u32(node, "audio.channels")),
            rate_hz: param_u32(node, "Format", "rate").or_else(|| prop_u32(node, "audio.rate")),
            running: state.map(|s| s == "running"),
            volume_pct,
            muted,
            hardware_volume,
            streams: Vec::new(),
        });
        sink_ids.push(node_id);
    }

    // Each stream's sink: the node its link goes to
    let mut target: BTreeMap<u64, u64> = BTreeMap::new();
    for link in of(LINK) {
        let out = link.pointer("/info/output-node-id").and_then(Value::as_u64);
        let inp = link.pointer("/info/input-node-id").and_then(Value::as_u64);
        if let (Some(o), Some(i)) = (out, inp) {
            target.entry(o).or_insert(i);
        }
    }
    for node in of(NODE).filter(|n| prop_str(n, "media.class") == "Stream/Output/Audio") {
        let Some(sink) = id(node).and_then(|n| target.get(&n)) else {
            continue;
        };
        let Some(slot) = sink_ids
            .iter()
            .position(|s| s == sink)
            .and_then(|i| outputs.get_mut(i))
        else {
            continue;
        };
        let app = [
            prop_str(node, "application.name"),
            prop_str(node, "application.process.binary"),
            prop_str(node, "node.name"),
        ]
        .into_iter()
        .find(|s| !s.is_empty())
        .unwrap_or("");
        let (volume_pct, muted, _) = volume_of(node, (false, false));
        let state = node.pointer("/info/state").and_then(Value::as_str);
        slot.streams.push(HostStream {
            app: app.to_owned(),
            title: prop_str(node, "media.name").to_owned(),
            channels: param_u32(node, "Format", "channels")
                .or_else(|| prop_u32(node, "audio.channels")),
            volume_pct,
            muted,
            playing: state.map(|s| s == "running"),
        });
    }
    Ok(HostAudio {
        sim: false, // -- WP ODEC --
        os: HostOs::Linux,
        audio_system: version,
        unknown: None,
        outputs,
        streams_known: true,
        streams_note: None,
    })
}

/// How long `pw-dump` may take.
#[cfg(all(feature = "host-audio", target_os = "linux"))]
const PW_DUMP_LIMIT: std::time::Duration = std::time::Duration::from_secs(3);

/// The Linux reader: `pw-dump`, else ALSA.
#[cfg(all(feature = "host-audio", target_os = "linux"))]
#[derive(Debug, Clone)]
pub struct PwDump {
    /// The program ("pw-dump").
    pub program: String,
    /// Where ALSA's `/proc` and `/sys` are ("/": the real ones).
    pub root: std::path::PathBuf,
}

#[cfg(all(feature = "host-audio", target_os = "linux"))]
impl Default for PwDump {
    fn default() -> Self {
        Self {
            program: "pw-dump".into(),
            root: "/".into(),
        }
    }
}

#[cfg(all(feature = "host-audio", target_os = "linux"))]
impl PwDump {
    /// `pw-dump`'s output, or why there is none (`not_found`: no such
    /// program).
    fn run(&self) -> Result<String, (bool, String)> {
        use std::io::Read;
        use std::process::{Command, Stdio};
        let mut child = Command::new(&self.program)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| {
                let missing = e.kind() == std::io::ErrorKind::NotFound;
                let why = if missing {
                    "PipeWire's pw-dump is not installed or not reachable (a sandboxed \
                     app may not see it)"
                } else {
                    "pw-dump could not be started"
                };
                (missing, why.to_owned())
            })?;
        let Some(mut stdout) = child.stdout.take() else {
            let _ = child.kill();
            let _ = child.wait();
            return Err((false, "pw-dump gave no output".into()));
        };
        // Read on a thread so a big dump cannot fill the pipe while we wait
        let reader = std::thread::spawn(move || {
            let mut text = String::new();
            stdout.read_to_string(&mut text).map(|_| text)
        });
        let start = std::time::Instant::now();
        let status = loop {
            match child.try_wait() {
                Ok(Some(s)) => break Some(s),
                Ok(None) if start.elapsed() < PW_DUMP_LIMIT => {
                    std::thread::sleep(std::time::Duration::from_millis(20));
                }
                _ => {
                    let _ = child.kill();
                    let _ = child.wait();
                    break None;
                }
            }
        };
        let text = reader.join().ok().and_then(Result::ok);
        match (status, text) {
            (None, _) => Err((false, "PipeWire did not answer in time".into())),
            (Some(s), _) if !s.success() => Err((
                false,
                "PipeWire is not running for this user, or refused the app".into(),
            )),
            (Some(_), Some(t)) => Ok(t),
            (Some(_), None) => Err((false, "pw-dump's output could not be read".into())),
        }
    }
}

#[cfg(all(feature = "host-audio", target_os = "linux"))]
impl super::HostAudioSource for PwDump {
    fn read(&self) -> HostAudio {
        let why = match self.run() {
            Ok(text) => match parse(&text) {
                Ok(audio) => return audio,
                Err(why) => why,
            },
            Err((_, why)) => why,
        };
        super::alsa::read(&self.root, &why)
    }
}

#[cfg(test)]
#[allow(clippy::indexing_slicing, clippy::unwrap_used)]
mod tests {
    use super::*;

    const DUMP: &str = include_str!("../../tests/fixtures/host_audio/pw-dump.json");

    #[test]
    fn wp_feat3_pw_dump_fixture_gives_sinks_default_format_volume_and_streams() {
        let a = parse(DUMP).unwrap();
        assert_eq!(a.os, HostOs::Linux);
        assert_eq!(a.audio_system, "PipeWire 1.2.7");
        assert!(a.unknown.is_none());
        assert!(a.streams_known);
        let names: Vec<&str> = a.outputs.iter().map(|o| o.name.as_str()).collect();
        assert_eq!(names, ["Laptop speakers", "DSPi Desk", "DSPi K340"]);
        let laptop = &a.outputs[0];
        assert_eq!(laptop.is_default, Some(true));
        assert_eq!(laptop.hardware_volume, Some(false), "soft volume");
        assert_eq!(laptop.streams.len(), 1);
        assert_eq!(laptop.streams[0].app, "Game");
        assert_eq!(laptop.streams[0].volume_pct, Some(150.0));
        assert_eq!(laptop.streams[0].playing, Some(true));
        let desk = &a.outputs[1];
        assert_eq!(desk.is_default, Some(false));
        assert!(desk.serial.contains("E66118604B3B5F2A"));
        assert_eq!(desk.channels, Some(8));
        assert_eq!(desk.rate_hz, Some(48000));
        assert_eq!(desk.running, Some(true));
        assert_eq!(desk.volume_pct, Some(50.0));
        assert_eq!(desk.muted, Some(false));
        assert_eq!(desk.hardware_volume, Some(true), "the USB volume control");
        assert_eq!(desk.streams.len(), 1);
        assert_eq!(desk.streams[0].app, "Discord");
        assert_eq!(desk.streams[0].title, "Voice chat");
        assert_eq!(desk.streams[0].channels, Some(2));
        let k340 = &a.outputs[2];
        // Idle: the configured format, no stream; all at 1 tells nothing.
        assert_eq!(k340.running, Some(false));
        assert_eq!(k340.channels, Some(4));
        assert_eq!(k340.rate_hz, Some(96000));
        assert_eq!(k340.hardware_volume, None);
        assert!(k340.streams.is_empty());
    }

    #[test]
    fn wp_feat3_pw_dump_default_from_text_value_and_bad_documents() {
        let doc = r#"[
          {"id": 30, "type": "PipeWire:Interface:Node", "info": {"state": "idle",
            "props": {"media.class": "Audio/Sink", "node.name": "a", "node.description": "A"}}},
          {"id": 31, "type": "PipeWire:Interface:Node", "info": {"state": "idle",
            "props": {"media.class": "Audio/Sink", "node.name": "b"}}},
          {"id": 0, "type": "PipeWire:Interface:Metadata", "props": {"metadata.name": "default"},
           "info": {"props": {"metadata.name": "default"}},
           "metadata": [{"subject": 0, "key": "default.configured.audio.sink",
                         "type": "Spa:String:JSON", "value": "{\"name\":\"b\"}"}]}
        ]"#;
        let a = parse(doc).unwrap();
        assert_eq!(a.audio_system, "PipeWire");
        assert_eq!(a.outputs[0].is_default, Some(false));
        assert_eq!(a.outputs[1].is_default, Some(true));
        assert_eq!(a.outputs[1].name, "b", "falls back to node.name");
        assert!(a.outputs[0].channels.is_none());
        assert!(parse("not json").is_err());
        assert!(parse("{}").is_err());
        assert!(parse("[]").unwrap().outputs.is_empty());
    }

    /// A dump a real PipeWire 1.6.11 (Fedora 44) gave (tests/real_pipewire/run.sh,
    /// trimmed to the core, the default metadata, sinks, streams and links;
    /// user and host names replaced).
    #[test]
    fn wp_feat3_a_real_pipewire_dump_parses_as_the_live_one_did() {
        let a = parse(include_str!(
            "../../tests/fixtures/host_audio/pw-dump-real-1.6.11.json"
        ))
        .unwrap();
        assert_eq!(a.audio_system, "PipeWire 1.6.11");
        let desk = a.outputs.iter().find(|o| o.name == "DSPi Desk").unwrap();
        assert_eq!(
            (desk.is_default, desk.channels, desk.rate_hz, desk.running),
            (Some(false), Some(8), Some(48000), Some(true))
        );
        assert_eq!(desk.volume_pct, Some(50.0));
        assert_eq!(desk.hardware_volume, Some(false), "a null sink");
        assert!(desk.serial.contains("E66118604B3B5F2A"));
        let game = desk.streams.iter().find(|s| s.app == "Game").unwrap();
        assert_eq!(
            (game.title.as_str(), game.playing),
            ("Game audio", Some(true))
        );
        let laptop = a
            .outputs
            .iter()
            .find(|o| o.name == "Laptop speakers")
            .unwrap();
        assert_eq!(laptop.is_default, Some(true));
    }

    #[test]
    fn wp_feat3_only_an_alsa_mixer_is_the_devices_own_volume() {
        let sink = |props: &str| {
            format!(
                r#"{{"id": 9, "type": "PipeWire:Interface:Node", "info": {{"state": "idle",
                "props": {{"media.class": "Audio/Sink", "node.name": "n" {props}}},
                "params": {{"Props": [{{"channelVolumes": [0.125, 0.125],
                                       "softVolumes": [1.0, 1.0]}}]}}}}}}"#
            )
        };
        let hw =
            |props: &str| parse(&format!("[{}]", sink(props))).unwrap().outputs[0].hardware_volume;
        // The pattern of a hardware mixer, on an ALSA card: the USB volume
        assert_eq!(hw(r#", "api.alsa.path": "front:1""#), Some(true));
        // The same pattern on a null sink: nothing scales, nothing to drive
        assert_eq!(
            hw(r#", "factory.name": "support.null-audio-sink""#),
            Some(false)
        );
        // An ALSA card told to use a software volume
        assert_eq!(
            hw(r#", "api.alsa.path": "front:1", "api.alsa.soft-mixer": true"#),
            Some(false)
        );
    }

    #[cfg(all(feature = "host-audio", target_os = "linux"))]
    #[test]
    fn wp_feat3_no_pw_dump_falls_back_to_alsa_with_the_reason() {
        use crate::host_audio::HostAudioSource;
        let r = PwDump {
            program: "dspi-feat3-no-such-program".into(),
            root: concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/tests/fixtures/host_audio/alsa"
            )
            .into(),
        }
        .read();
        assert_eq!(r.audio_system, "ALSA");
        assert!(r.unknown.is_none());
        assert!(!r.streams_known);
        assert!(
            r.streams_note
                .as_deref()
                .unwrap()
                .contains("pw-dump is not installed"),
            "{r:?}"
        );
        assert_eq!(r.outputs.len(), 1);
    }

    #[cfg(all(feature = "host-audio", target_os = "linux"))]
    #[test]
    fn wp_feat3_a_failing_pw_dump_and_no_alsa_is_unknown_not_an_error() {
        use crate::host_audio::HostAudioSource;
        let r = PwDump {
            program: "false".into(),
            root: concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/tests/fixtures/host_audio/none"
            )
            .into(),
        }
        .read();
        let why = r.unknown.unwrap();
        assert!(why.contains("PipeWire is not running"), "{why}");
        assert!(why.contains("no ALSA"), "{why}");
    }

    /// Against a live PipeWire: crates/dspi-mcp/tests/real_pipewire/run.sh
    /// starts one with a "DSPi Desk" null sink at 50 %, "Laptop speakers"
    /// as the default and "Game" playing to the DSPi, then runs this.
    #[cfg(all(feature = "host-audio", target_os = "linux"))]
    #[test]
    #[ignore = "needs a running PipeWire: tests/real_pipewire/run.sh"]
    fn wp_feat3_real_pipewire_reads_sinks_default_volume_and_a_stream() {
        use crate::host_audio::{Found, HostAudioSource, find};
        let r = PwDump::default().read();
        assert!(r.unknown.is_none(), "{r:?}");
        assert!(r.audio_system.starts_with("PipeWire 1."), "{r:?}");
        assert!(r.streams_known);
        let desk = r.outputs.iter().find(|o| o.name == "DSPi Desk").unwrap();
        assert_eq!(desk.is_default, Some(false));
        assert_eq!(desk.channels, Some(8), "{desk:?}");
        assert_eq!(desk.rate_hz, Some(48000), "{desk:?}");
        assert_eq!(desk.running, Some(true), "{desk:?}");
        assert_eq!(desk.volume_pct, Some(50.0), "{desk:?}");
        // A null sink has no mixer: PipeWire scales the samples itself
        assert_eq!(desk.hardware_volume, Some(false), "{desk:?}");
        let game = desk.streams.iter().find(|s| s.app == "Game").unwrap();
        assert_eq!(game.title, "Game audio");
        assert_eq!(game.playing, Some(true));
        let laptop = r
            .outputs
            .iter()
            .find(|o| o.name == "Laptop speakers")
            .unwrap();
        assert_eq!(laptop.is_default, Some(true));
        assert!(laptop.streams.is_empty());
        let at = r
            .outputs
            .iter()
            .position(|o| o.name == "DSPi Desk")
            .unwrap();
        assert_eq!(find(&r.outputs, "E66118604B3B5F2A", 1), Found::Serial(at));
        eprintln!("wp_feat3 real PipeWire: {r:#?}");
    }
}
