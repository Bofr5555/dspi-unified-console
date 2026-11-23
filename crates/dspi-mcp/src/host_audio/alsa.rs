//! Linux without PipeWire: ALSA's USB sound cards, as the Qt app's
//! `HostAudioFormatLinux.cpp` reads them. Each card's serial is in its USB
//! device (`/sys/class/sound/cardN/device/../serial`), its name in
//! `/proc/asound/cardN/id` and `stream0`, and the format in use in the
//! playback stream's `hw_params` while something has it open. ALSA knows no
//! default output, app streams or volume slider: those stay unknown.

use std::path::Path;

use super::{HostAudio, HostOs, HostOutput};

fn text(path: &Path) -> String {
    std::fs::read_to_string(path)
        .map(|t| t.trim().to_owned())
        .unwrap_or_default()
}

/// `key: value` from `hw_params` ("channels: 8", "rate: 48000 (48000/1)").
fn field(params: &str, key: &str) -> Option<u32> {
    params.lines().find_map(|l| {
        let rest = l.trim().strip_prefix(key)?.strip_prefix(':')?;
        rest.split_whitespace().next()?.parse().ok()
    })
}

/// The USB sound cards under `root` ("/" for this computer). `why` is why
/// PipeWire gave nothing; it goes into the reading's notes.
pub fn read(root: &Path, why: &str) -> HostAudio {
    let proc_dir = root.join("proc/asound");
    let Ok(entries) = std::fs::read_dir(&proc_dir) else {
        return HostAudio::unknown(
            HostOs::Linux,
            "",
            &format!("{why}; and no ALSA sound cards could be read"),
        );
    };
    let mut cards: Vec<(u32, String)> = entries
        .filter_map(Result::ok)
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let n = name.strip_prefix("card")?.parse().ok()?;
            Some((n, name))
        })
        .collect();
    cards.sort();
    let mut outputs = Vec::new();
    for (_, card) in cards {
        let dir = proc_dir.join(&card);
        // Only USB cards have a usbid
        if !dir.join("usbid").exists() {
            continue;
        }
        let stream = text(&dir.join("stream0"));
        if !stream.contains("Playback:") {
            continue;
        }
        let product = stream
            .lines()
            .next()
            .and_then(|l| l.split(" at ").next())
            .unwrap_or("")
            .trim();
        let id = text(&dir.join("id"));
        let params = text(&dir.join("pcm0p/sub0/hw_params"));
        let open = !params.is_empty() && params != "closed";
        let serial = text(
            &root
                .join("sys/class/sound")
                .join(&card)
                .join("device/../serial"),
        );
        outputs.push(HostOutput {
            id: format!("hw:{id}"),
            name: if product.is_empty() {
                id
            } else {
                product.to_owned()
            },
            serial,
            is_default: None,
            channels: if open {
                field(&params, "channels")
            } else {
                None
            },
            rate_hz: if open { field(&params, "rate") } else { None },
            running: Some(open),
            volume_pct: None,
            muted: None,
            hardware_volume: None,
            streams: Vec::new(),
        });
    }
    HostAudio {
        os: HostOs::Linux,
        audio_system: "ALSA".into(),
        unknown: None,
        outputs,
        streams_known: false,
        streams_note: Some(format!(
            "{why}. ALSA alone does not say which output is the default, which app \
             plays where, or the volume; it lists the USB sound cards and the format \
             of one in use"
        )),
    }
}

#[cfg(test)]
#[allow(clippy::indexing_slicing, clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn wp_feat3_alsa_fixture_lists_the_usb_card_with_serial_and_open_format() {
        let root = Path::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/host_audio/alsa"
        ));
        let a = read(root, "no PipeWire");
        assert_eq!(a.audio_system, "ALSA");
        assert!(a.unknown.is_none());
        // card0 is the laptop's own (no usbid), card2 has no playback
        assert_eq!(a.outputs.len(), 1, "{a:?}");
        let o = &a.outputs[0];
        assert_eq!(o.name, "WeebLabs DSPi");
        assert_eq!(o.id, "hw:DSPi");
        assert_eq!(o.serial, "E66118604B3B5F2A");
        assert_eq!(o.channels, Some(8));
        assert_eq!(o.rate_hz, Some(48000));
        assert_eq!(o.running, Some(true));
        assert_eq!(o.is_default, None);
        assert!(a.streams_note.unwrap().starts_with("no PipeWire."));
    }

    #[test]
    fn wp_feat3_alsa_fields() {
        let p = "access: MMAP_INTERLEAVED\nformat: S16_LE\nchannels: 2\nrate: 44100 (44100/1)\n";
        assert_eq!(field(p, "channels"), Some(2));
        assert_eq!(field(p, "rate"), Some(44100));
        assert_eq!(field(p, "period_size"), None);
    }
}
