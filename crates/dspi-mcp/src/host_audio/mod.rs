//! What the computer running the app does with its audio (WP FEAT3,
//! FSTORY gaps 1(c) and 9): is each open DSPi an output device of the OS, is
//! it the default output, its format, what plays to it, and the OS volume.
//!
//! Read only. Nothing here changes the default output, moves a stream or
//! sets a volume: the agent tells the user how to (`how_to_set_default`).
//!
//! The reading is behind [`HostAudioSource`] so the crate stays sans-IO: the
//! embedder passes one in [`crate::ServerConfig::host_audio`], the tests a
//! fixture. [`system`] gives this computer's own (feature `host-audio`):
//! PipeWire's `pw-dump` with ALSA's `/proc/asound` behind it on Linux, the
//! MMDevice and session APIs on Windows, Core Audio on macOS. A read that
//! cannot be made is a [`HostAudio`] whose `unknown` says why, never an error.

pub mod alsa;
pub mod pipewire;
// -- WP FBUS -- the function test's simulated host audio bus
#[cfg(feature = "sim-bus")]
pub mod sim_bus;

#[cfg(all(feature = "host-audio", target_os = "windows"))]
mod windows;

// -- macOS. `dspi_check_macos` compiles it for an Apple target the build
// container has (iOS), which has no Core Audio HAL to link: a check only.
#[cfg(all(feature = "host-audio", any(target_os = "macos", dspi_check_macos)))]
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
mod macos;

#[cfg(feature = "host-audio")]
use std::sync::Arc;

/// Which OS the reading comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostOs {
    /// Linux.
    Linux,
    /// Windows.
    Windows,
    /// macOS.
    MacOs,
    /// Anything else (no reading).
    Other,
}

impl HostOs {
    /// The word the tool gives.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Linux => "linux",
            Self::Windows => "windows",
            Self::MacOs => "macos",
            Self::Other => "other",
        }
    }

    /// The OS this build runs on.
    pub fn this() -> Self {
        if cfg!(target_os = "linux") {
            Self::Linux
        } else if cfg!(target_os = "windows") {
            Self::Windows
        } else if cfg!(target_os = "macos") {
            Self::MacOs
        } else {
            Self::Other
        }
    }
}

/// One app's stream playing to an output.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct HostStream {
    /// The app, as the OS names it (untrusted).
    pub app: String,
    /// What it plays (a media title, untrusted; "" if the OS gives none).
    pub title: String,
    /// Its channel count.
    pub channels: Option<u32>,
    /// Its own volume in percent (the OS's slider scale).
    pub volume_pct: Option<f32>,
    /// Muted.
    pub muted: Option<bool>,
    /// Playing now (else paused, corked or idle).
    pub playing: Option<bool>,
}

/// One playback device of the OS.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct HostOutput {
    /// The OS's id for it (a sink name, an endpoint id, a device UID).
    pub id: String,
    /// The name the OS shows (untrusted).
    pub name: String,
    /// Text that holds its USB serial, if the OS gives one ("" else).
    pub serial: String,
    /// The OS's default output.
    pub is_default: Option<bool>,
    /// Channels of the format in use (or the configured one).
    pub channels: Option<u32>,
    /// Sample rate in Hz.
    pub rate_hz: Option<u32>,
    /// Something plays to it now.
    pub running: Option<bool>,
    /// The OS's volume for it, in percent of its slider.
    pub volume_pct: Option<f32>,
    /// Muted at the OS.
    pub muted: Option<bool>,
    /// The OS volume is the device's own USB volume control (the DSPi's
    /// user volume follows it); `false`: the OS scales the samples itself.
    pub hardware_volume: Option<bool>,
    /// What plays to it, where the OS says.
    pub streams: Vec<HostStream>,
}

/// One reading of the computer's audio.
#[derive(Debug, Clone, PartialEq)]
pub struct HostAudio {
    /// The OS.
    pub os: HostOs,
    /// What answered ("PipeWire 1.2.7", "ALSA", "Windows audio", "Core Audio").
    pub audio_system: String,
    /// Why nothing could be read; `None` when the outputs below are the OS's.
    pub unknown: Option<String>,
    /// The playback devices.
    pub outputs: Vec<HostOutput>,
    /// The OS says which app plays to which output.
    pub streams_known: bool,
    /// Why not, when it does not.
    pub streams_note: Option<String>,
}

impl HostAudio {
    /// A reading that could not be made, and why (the server's own words).
    pub fn unknown(os: HostOs, audio_system: &str, why: &str) -> Self {
        Self {
            os,
            audio_system: audio_system.to_owned(),
            unknown: Some(why.to_owned()),
            outputs: Vec::new(),
            streams_known: false,
            streams_note: None,
        }
    }
}

/// Reads the computer's audio. `read` may block for a moment (a process or
/// an OS call); the server calls it on the transport's thread, not the UI's.
pub trait HostAudioSource: Send + Sync {
    /// One reading. Never fails: a reading it cannot make says why.
    fn read(&self) -> HostAudio;
}

/// A fixed reading (tests, and a build that reads nothing).
#[derive(Debug, Clone)]
pub struct FixedHostAudio(pub HostAudio);

impl HostAudioSource for FixedHostAudio {
    fn read(&self) -> HostAudio {
        self.0.clone()
    }
}

/// This computer's reader.
#[cfg(feature = "host-audio")]
pub fn system() -> Arc<dyn HostAudioSource> {
    // -- WP FBUS -- the function test's bus, while one is loaded
    #[cfg(feature = "sim-bus")]
    if sim_bus::loaded() {
        return Arc::new(sim_bus::SimBus { real: real() });
    }
    real()
}

/// This computer's own reader.
#[cfg(feature = "host-audio")]
fn real() -> Arc<dyn HostAudioSource> {
    #[cfg(target_os = "linux")]
    {
        Arc::new(pipewire::PwDump::default())
    }
    #[cfg(target_os = "windows")]
    {
        Arc::new(windows::Wasapi)
    }
    #[cfg(target_os = "macos")]
    {
        Arc::new(macos::CoreAudio)
    }
    #[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
    {
        Arc::new(FixedHostAudio(HostAudio::unknown(
            HostOs::Other,
            "",
            "this app does not read the audio settings on this system",
        )))
    }
}

/// Which output of a reading is the DSPi with `serial`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Found {
    /// The output whose USB serial is the unit's.
    Serial(usize),
    /// The only output named DSPi that gives no serial, with one DSPi open
    /// on USB (the Mac's rule, as `HostAudioFormat` has it).
    Name(usize),
    /// No output is it.
    None,
    /// Several outputs could be it.
    Ambiguous,
}

/// The shortest serial matched inside a longer serial text (udev's
/// `Vendor_Product_Serial`): a short one could match by chance.
const SERIAL_MIN: usize = 6;

/// Find the DSPi with USB serial `serial` among `outputs`. `usb_open` is how
/// many DSPis are open on USB here: the name rule holds only for one.
pub fn find(outputs: &[HostOutput], serial: &str, usb_open: usize) -> Found {
    let want = serial.trim().to_ascii_lowercase();
    if want.len() >= SERIAL_MIN {
        let hits: Vec<usize> = outputs
            .iter()
            .enumerate()
            .filter(|(_, o)| o.serial.to_ascii_lowercase().contains(&want))
            .map(|(i, _)| i)
            .collect();
        match hits.as_slice() {
            [one] => return Found::Serial(*one),
            [] => {}
            _ => return Found::Ambiguous,
        }
    }
    let named: Vec<usize> = outputs
        .iter()
        .enumerate()
        .filter(|(_, o)| o.serial.is_empty() && o.name.to_ascii_lowercase().contains("dspi"))
        .map(|(i, _)| i)
        .collect();
    match (named.as_slice(), usb_open) {
        ([one], 1) => Found::Name(*one),
        ([], _) => Found::None,
        _ => Found::Ambiguous,
    }
}

/// PipeWire's and PulseAudio's percent: the cube root of the linear gain.
pub fn cubic_pct(linear: f32) -> Option<f32> {
    if !linear.is_finite() || linear < 0.0 {
        return None;
    }
    Some((linear.cbrt() * 1000.0).round() / 10.0)
}

/// A WAVEFORMATEX's channels and rate: `wFormatTag` (2 bytes), `nChannels`
/// (2), `nSamplesPerSec` (4), little-endian. `None` for a short blob.
pub fn wave_format(blob: &[u8]) -> (Option<u32>, Option<u32>) {
    let channels = blob
        .get(2..4)
        .and_then(|b| <[u8; 2]>::try_from(b).ok())
        .map(|b| u32::from(u16::from_le_bytes(b)));
    let rate = blob
        .get(4..8)
        .and_then(|b| <[u8; 4]>::try_from(b).ok())
        .map(u32::from_le_bytes);
    (channels.filter(|&c| c > 0), rate.filter(|&r| r > 0))
}

/// The USB serial in a chain of Windows device instance ids: the last part
/// of the first `USB\VID_…` id, unless it holds `&` (Windows made that one
/// up: the device gave no serial). As the Qt app's `usbSerialFromInstanceIds`.
pub fn usb_serial_from_instance_ids(chain: &[String]) -> String {
    for id in chain {
        let upper = id.to_ascii_uppercase();
        if !upper.starts_with("USB\\VID_") {
            continue;
        }
        let last = id.rsplit('\\').next().unwrap_or("");
        if !last.is_empty() && !last.contains('&') {
            return last.to_owned();
        }
    }
    String::new()
}

#[cfg(test)]
#[allow(clippy::indexing_slicing, clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn wp_feat3_wave_format_and_windows_serial_chain() {
        // WAVE_FORMAT_EXTENSIBLE, 8 channels, 48 kHz
        let blob = [0xFE, 0xFF, 8, 0, 0x80, 0xBB, 0, 0, 0, 0];
        assert_eq!(wave_format(&blob), (Some(8), Some(48000)));
        assert_eq!(wave_format(&blob[..4]), (Some(8), None));
        assert_eq!(wave_format(&[1, 0]), (None, None));
        let chain = |ids: &[&str]| ids.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>();
        assert_eq!(
            usb_serial_from_instance_ids(&chain(&[
                "SWD\\MMDEVAPI\\{0.0.0.00000000}.{abc}",
                "USB\\VID_2E8A&PID_FEAA&MI_00\\7&1A2B3C&0&0000",
                "USB\\VID_2E8A&PID_FEAA\\E66118604B3B5F2A",
            ])),
            "E66118604B3B5F2A"
        );
        assert_eq!(
            usb_serial_from_instance_ids(&chain(&["USB\\VID_1234&PID_0001\\5&2C3D&0&2"])),
            ""
        );
        assert_eq!(usb_serial_from_instance_ids(&[]), "");
    }

    fn out(name: &str, serial: &str) -> HostOutput {
        HostOutput {
            name: name.into(),
            serial: serial.into(),
            ..HostOutput::default()
        }
    }

    #[test]
    fn wp_feat3_find_by_serial_inside_udev_text_then_by_the_only_dspi_name() {
        let outs = [
            out("Laptop speakers", ""),
            out("DSPi Fronts", "WeebLabs_DSPi_E66118604B3B5F2A"),
            out("DSPi K340", "WeebLabs_DSPi_E6611860AAAA0001"),
        ];
        assert_eq!(find(&outs, "e66118604b3b5f2a", 2), Found::Serial(1));
        assert_eq!(find(&outs, "E6611860AAAA0001", 2), Found::Serial(2));
        // A short serial never matches by containment.
        assert_eq!(find(&outs, "E66", 2), Found::None);
        // Name rule: only a DSPi-named output without a serial, one unit open.
        let named = [out("Speakers", ""), out("DSPi Audio", "")];
        assert_eq!(find(&named, "ABCDEF123456", 1), Found::Name(1));
        assert_eq!(find(&named, "ABCDEF123456", 2), Found::Ambiguous);
        let two = [out("DSPi Audio", ""), out("DSPi Audio 2", "")];
        assert_eq!(find(&two, "", 1), Found::Ambiguous);
        assert_eq!(find(&[out("Speakers", "")], "ABCDEF123456", 1), Found::None);
    }

    #[test]
    fn wp_feat3_cubic_percent_is_pipewires() {
        assert_eq!(cubic_pct(1.0), Some(100.0));
        assert_eq!(cubic_pct(0.125), Some(50.0));
        assert_eq!(cubic_pct(0.0), Some(0.0));
        assert_eq!(cubic_pct(f32::NAN), None);
        assert_eq!(cubic_pct(-1.0), None);
    }
}
