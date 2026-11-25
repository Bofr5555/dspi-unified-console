//! WP WB6j: the web's analyser, signal generator, Stats for nerbs and
//! Interrupt Monitor calls, over the core's own (the ones the Qt app's
//! `RtaController`, `SiggenController`, `StatsController` and
//! `MonitorController` call through the C ABI).
//!
//! Analyser (the frames themselves are `rta`, the request `rtaStart`):
//! - `analyserView`: the caps with the band centres, the picture and who
//!   drives the device's one analyser (`rta_get_ownership`: the last choice
//!   made wins, WP CG5 / ANLAST), in one answer.
//! - `rtaTakeControl`: stop following another console (`rta_take_control`).
//!
//! Signal generator (every write through the core's `siggen_*`, so its
//! read-only, lock and history rules apply; the generator never starts but
//! by `siggenControl` START or `siggenIdentify`):
//! - `siggenCaps`: the device's caps with the catalogue (names, the blurb, a
//!   parameter's label and unit) and each type's parameters as the device
//!   gives them (the table's where the device's descriptor is missing or
//!   unreadable), the multitone's count held to what the device plays.
//! - `siggenConfig`, `siggenStatus`: the staged config and what it does now.
//! - `siggenSet {config}`: stage a config; `siggenControl {action}`:
//!   `start`, `stop` (fade) or `stopNow`.
//! - `siggenIdentify {output}`: the Mac's Identify (`Commands.swift`'s
//!   `identifyOutput`): channel-ID blips on that output alone, held to the
//!   device's valid channel mask, -12 dBFS, two passes, 120 ms per blip; the
//!   generator goes idle by itself.
//!
//! Stats for nerbs:
//! - `statsRead`: the slow poll: the counters, the output ring health, the
//!   S/PDIF receiver (on firmware with input sources), LG Sound Sync, the
//!   ADAT output, the I2S slave input, the clock diagnostics and the USB
//!   errors; a section the device does not answer is null.
//! - `statsTick`: the fast poll (`fetch_stats_tick`): buffer fills, and the
//!   live outputs' fills with their bands on the ring's own scale.
//! - `statsResetWatermarks`: `reset_buffer_stats`.
//!
//! Interrupt Monitor: `monitorRead {afterId}`, `monitorClear`.

use dspi_core::consts::{
    SIGGEN_CTL_START, SIGGEN_CTL_STOP, SIGGEN_CTL_STOP_NOW, STAT_CLOCK_HZ, STAT_CORE_MV,
    STAT_PDM_DMA_OVERRUNS, STAT_PDM_DMA_UNDERRUNS, STAT_PDM_RING_OVERRUNS, STAT_PDM_RING_UNDERRUNS,
    STAT_SAMPLE_RATE, STAT_SPDIF_OVERRUNS, STAT_SPDIF_UNDERRUNS, STAT_STARVATION_FIRST,
    STAT_STARVATION_TOTAL, STAT_TEMPERATURE, STAT_USB_RING_OVERRUNS,
};
use dspi_core::{DspiCore, DspiError, SiggenCaps, SiggenConfig};
use serde::Deserialize;
use serde_json::{Map, Value, json};

use crate::calls::{CallError, tidy};

type CallResult = Result<Value, CallError>;

/// The calls of this module, `None` for another method.
pub fn dispatch(core: &DspiCore, method: &str, p: Value) -> Option<CallResult> {
    Some(match method {
        "analyserView" => analyser_view(core),
        "rtaTakeControl" => {
            core.rta_take_control();
            Ok(Value::Null)
        }
        "siggenCaps" => core.siggen_fetch_caps().map(|c| caps(&c)).map_err(Into::into),
        "siggenConfig" => core.siggen_get_config().map(|c| config(&c)).map_err(Into::into),
        "siggenStatus" => core.siggen_get_status().map(|s| status(&s)).map_err(Into::into),
        "siggenSet" => params::<SetParams>(p).and_then(|s| {
            core.siggen_set_config(s.config.into_core())?;
            Ok(Value::Null)
        }),
        "siggenControl" => params::<ControlParams>(p).and_then(|c| {
            let action = match c.action.as_str() {
                "start" => SIGGEN_CTL_START,
                "stop" => SIGGEN_CTL_STOP,
                "stopNow" => SIGGEN_CTL_STOP_NOW,
                other => {
                    return Err(CallError::new(
                        "invalid",
                        format!("unknown signal generator action {other:?}"),
                    ));
                }
            };
            core.siggen_control(action)?;
            Ok(Value::Null)
        }),
        "siggenIdentify" => params::<IdentifyParams>(p).and_then(|i| identify(core, i.output)),
        "statsRead" => stats_read(core),
        "statsTick" => core
            .fetch_stats_tick()
            .map(|t| tidy(&t))
            .map_err(Into::into),
        "statsResetWatermarks" => core
            .reset_buffer_stats()
            .map(|()| Value::Null)
            .map_err(Into::into),
        "monitorRead" => params::<MonitorParams>(p).map(|m| {
            let b = core.monitor_read(m.after_id);
            json!({
                "lastId": b.last_id,
                "entries": b.entries.iter().map(|e| json!({ "id": e.id, "timeMs": e.time_ms, "text": e.text })).collect::<Vec<_>>(),
            })
        }),
        "monitorClear" => {
            core.monitor_clear();
            Ok(Value::Null)
        }
        _ => return None,
    })
}

fn params<T: for<'de> Deserialize<'de>>(v: Value) -> Result<T, CallError> {
    // No parameters at all reads as an empty object, so `{afterId}` may be left out.
    let v = if v.is_null() { json!({}) } else { v };
    serde_json::from_value(v).map_err(|e| CallError::new("invalid", format!("bad parameters: {e}")))
}

// ------------------------------------------------------------------ analyser

fn analyser_view(core: &DspiCore) -> CallResult {
    if !core.is_connected() {
        return Err(DspiError::Disconnected.into());
    }
    let c = core.rta_get_caps();
    let s = core.rta_get_snapshot();
    let o = core.rta_get_ownership();
    let bands = usize::from(c.max_bands);
    let count = usize::from(s.count);
    Ok(json!({
        "caps": {
            "supported": c.supported,
            "inputChannels": c.input_channels,
            "outputChannels": c.output_channels,
            "orderMin": c.order_min,
            "orderMax": c.order_max,
            "orderDefault": c.order_default,
            "bassBands": c.bass_bands,
            "maxBands": c.max_bands,
            "dynamicRangeDb": c.dynamic_range_db,
            "centres": c.centres.iter().take(bands).collect::<Vec<_>>(),
            // -- WP WEB6 -- the bass bands' range, for the Spectrum page's status line
            "bassDynamicRangeDb": c.bass_dynamic_range_db,
        },
        "snapshot": {
            "version": s.version,
            "tap": s.tap,
            "count": s.count,
            "frames": s.frames.iter().take(count).map(|f| json!({
                "channel": f.channel,
                "nBands": f.n_bands,
                "silent": f.silent,
                "avgDb": tidy(&f.avg_db),
                "peakDb": tidy(&f.peak_db),
            })).collect::<Vec<_>>(),
            "populated": s.populated.to_string(),
            "running": s.running,
            "liveMask": s.live_mask,
            "framesPerS": s.frames_per_s,
            "sampleRate": s.sample_rate,
            "fftOrder": s.fft_order,
            "firstBand": s.first_band,
            "configRejected": s.config_rejected,
            // -- WP WEB6 -- the telemetry Qt's status line shows
            "liveCount": s.live_count,
            "refreshIntervalS": tidy(&s.refresh_interval_s),
            "lastFrameUs": s.last_frame_us,
            "busyUsPerS": s.busy_us_per_s,
            "bassBusyUsPerS": s.bass_busy_us_per_s,
        },
        "ownership": {
            "following": o.following,
            "tap": o.tap,
            "mask": o.mask,
            "fftOrder": o.fft_order,
        },
    }))
}

// --------------------------------------------------------- signal generator

/// A parameter's meaning (`SIGGEN_SEM_*`, the firmware's codes).
const SEM_UNUSED: u8 = 0;
const SEM_HZ: u8 = 1;
const SEM_MS: u8 = 2;
const SEM_CYCLES: u8 = 3;
const SEM_COUNT: u8 = 4;
const SEM_RATIO: u8 = 5;
const SEM_PATTERN: u8 = 6;

const CONTINUOUS: u8 = 0;
const SWEEP: u8 = 1;
const PATTERN: u8 = 2;

/// The channel-ID type (Identify) and the multitone, by id.
const CHANNEL_ID: u8 = 14;
const MULTITONE: u8 = 12;

/// One parameter of the catalogue: label, meaning, the firmware's range and default.
type Row = (&'static str, u8, f32, f32, f32);
const NONE: Row = ("", SEM_UNUSED, 0.0, 0.0, 0.0);

/// The Mac's catalogue (`TestSignalsView`), as the Qt app keeps it: tile label,
/// name, blurb, timing model and the parameters with the firmware's ranges.
const TYPES: [(&str, &str, &str, u8, [Row; 4]); 15] = [
    (
        "Sine",
        "Sine",
        "Pure tone, THD approx -139 dB",
        CONTINUOUS,
        [
            ("Frequency", SEM_HZ, 1.0, 30000.0, 1000.0),
            NONE,
            NONE,
            NONE,
        ],
    ),
    (
        "Square",
        "Square wave",
        "Band-limited (polyBLEP) square",
        CONTINUOUS,
        [("Frequency", SEM_HZ, 1.0, 30000.0, 100.0), NONE, NONE, NONE],
    ),
    (
        "White",
        "White noise",
        "Uniform white noise",
        CONTINUOUS,
        [NONE, NONE, NONE, NONE],
    ),
    (
        "Pink",
        "Pink noise",
        "-3 dB/oct, level-safe normalized",
        CONTINUOUS,
        [NONE, NONE, NONE, NONE],
    ),
    (
        "Log Swp",
        "Log sweep",
        "Exponential sweep for room measurement",
        SWEEP,
        [
            ("Start", SEM_HZ, 1.0, 30000.0, 20.0),
            ("End", SEM_HZ, 1.0, 30000.0, 20000.0),
            NONE,
            NONE,
        ],
    ),
    (
        "Lin Swp",
        "Linear sweep",
        "Linear frequency sweep",
        SWEEP,
        [
            ("Start", SEM_HZ, 1.0, 30000.0, 20.0),
            ("End", SEM_HZ, 1.0, 30000.0, 20000.0),
            NONE,
            NONE,
        ],
    ),
    (
        "Step Swp",
        "Stepped sweep",
        "Discrete tones stepping up the band",
        SWEEP,
        [
            ("Start", SEM_HZ, 1.0, 30000.0, 20.0),
            ("End", SEM_HZ, 1.0, 30000.0, 20000.0),
            ("Steps/octave", SEM_COUNT, 1.0, 24.0, 3.0),
            ("Dwell", SEM_MS, 20.0, 10000.0, 250.0),
        ],
    ),
    (
        "Impulse",
        "Impulse",
        "Single-sample unit impulses",
        PATTERN,
        [("Period", SEM_MS, 10.0, 60000.0, 500.0), NONE, NONE, NONE],
    ),
    (
        "Clicks",
        "Alternating clicks",
        "Clicks with alternating polarity",
        PATTERN,
        [("Period", SEM_MS, 10.0, 60000.0, 500.0), NONE, NONE, NONE],
    ),
    (
        "Polarity",
        "Polarity pulse",
        "Positive half-sine lobe per period",
        PATTERN,
        [
            ("Pulse width", SEM_MS, 1.0, 100.0, 5.0),
            ("Period", SEM_MS, 10.0, 60000.0, 500.0),
            NONE,
            NONE,
        ],
    ),
    (
        "Burst",
        "Tone burst",
        "Sine bursts with raised-cosine edges",
        PATTERN,
        [
            ("Frequency", SEM_HZ, 1.0, 30000.0, 1000.0),
            ("On cycles", SEM_CYCLES, 1.0, 1000.0, 8.0),
            ("Off cycles", SEM_CYCLES, 0.0, 1000.0, 8.0),
            ("Edge cycles", SEM_CYCLES, 0.0, 100.0, 2.0),
        ],
    ),
    (
        "2-Tone",
        "Tone pair",
        "IMD test pair (SMPTE / CCIF)",
        CONTINUOUS,
        [
            ("Tone 1", SEM_HZ, 1.0, 30000.0, 60.0),
            ("Tone 2", SEM_HZ, 1.0, 30000.0, 7000.0),
            ("Ratio A1/A2", SEM_RATIO, 0.1, 10.0, 4.0),
            NONE,
        ],
    ),
    (
        "Multi",
        "Multitone",
        "Log-spaced tones, Schroeder phases",
        CONTINUOUS,
        [
            ("Tones", SEM_COUNT, 2.0, 16.0, 10.0),
            ("Low", SEM_HZ, 1.0, 30000.0, 20.0),
            ("High", SEM_HZ, 1.0, 30000.0, 20000.0),
            NONE,
        ],
    ),
    (
        "ISP",
        "ISP test",
        "Inter-sample-peak over patterns",
        CONTINUOUS,
        [("Pattern", SEM_PATTERN, 0.0, 1.0, 0.0), NONE, NONE, NONE],
    ),
    (
        "Chan ID",
        "Channel ID",
        "Counted pentatonic blips per channel",
        PATTERN,
        [
            ("Blip length", SEM_MS, 30.0, 1000.0, 120.0),
            NONE,
            NONE,
            NONE,
        ],
    ),
];

fn unit(semantic: u8) -> &'static str {
    match semantic {
        SEM_HZ => "Hz",
        SEM_MS => "ms",
        SEM_CYCLES => "cycles",
        _ => "",
    }
}

/// `siggenCaps`: the device's caps with the catalogue; see the module.
fn caps(c: &SiggenCaps) -> Value {
    let types: Vec<Value> = (0u8..)
        .zip(TYPES.iter())
        .map(|(id, (tile, name, blurb, timing, rows))| {
            let dev = c.supported.then(|| {
                c.types
                    .iter()
                    .take(usize::from(c.type_count))
                    .find(|t| t.id == id)
            });
            let dev = dev.flatten();
            let timing = dev.map_or(*timing, |t| if t.timing <= PATTERN { t.timing } else { *timing });
            let params: Vec<Value> = rows
                .iter()
                .enumerate()
                .map(|(i, &(label, sem, min, max, def))| {
                    let mut p = (sem, min, max, def);
                    if let Some(d) = dev.and_then(|t| t.params.get(i))
                        && d.semantic <= SEM_PATTERN
                        && d.min.is_finite()
                        && d.max.is_finite()
                        && d.def.is_finite()
                    {
                        p = (d.semantic, d.min, d.max, d.def);
                    }
                    if id == MULTITONE && i == 0 && c.multitone_max > 0 {
                        p.2 = p.2.min(f32::from(c.multitone_max));
                    }
                    json!({ "label": label, "semantic": p.0, "unit": unit(p.0), "min": p.1, "max": p.2, "def": p.3 })
                })
                .collect();
            // On a device that lists its types, one it does not list is not offered.
            let offered = !c.supported || dev.is_some();
            json!({ "id": id, "tile": tile, "name": name, "blurb": blurb, "timing": timing, "params": params, "offered": offered })
        })
        .collect();
    json!({
        "supported": c.supported,
        "outputChannels": c.output_channels,
        "multitoneMax": c.multitone_max,
        "validMask": c.valid_mask,
        "types": types,
    })
}

/// A config as the page sends it and reads it.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ConfigIn {
    signal_type: u8,
    channel_mask: u16,
    #[serde(default)]
    invert_mask: u16,
    #[serde(default)]
    flags: u8,
    level_db: f32,
    #[serde(default)]
    duration_ms: u32,
    #[serde(default)]
    repeat: u16,
    #[serde(default)]
    gap_ms: u16,
    p: Vec<f32>,
}

impl ConfigIn {
    fn into_core(self) -> SiggenConfig {
        SiggenConfig {
            signal_type: self.signal_type,
            channel_mask: self.channel_mask,
            invert_mask: self.invert_mask,
            flags: self.flags,
            level_db: self.level_db,
            duration_ms: self.duration_ms,
            repeat: self.repeat,
            gap_ms: self.gap_ms,
            p: self.p,
        }
    }
}

#[derive(Deserialize)]
struct SetParams {
    config: ConfigIn,
}

#[derive(Deserialize)]
struct ControlParams {
    action: String,
}

#[derive(Deserialize)]
struct IdentifyParams {
    output: u8,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct MonitorParams {
    #[serde(default)]
    after_id: u64,
}

fn config(c: &SiggenConfig) -> Value {
    json!({
        "signalType": c.signal_type,
        "channelMask": c.channel_mask,
        "invertMask": c.invert_mask,
        "flags": c.flags,
        "levelDb": tidy(&c.level_db),
        "durationMs": c.duration_ms,
        "repeat": c.repeat,
        "gapMs": c.gap_ms,
        "p": tidy(&c.p),
    })
}

fn status(s: &dspi_core::SiggenStatus) -> Value {
    json!({
        "state": s.state,
        "signalType": s.signal_type,
        "activeChannel": s.active_channel,
        "elapsedMs": s.elapsed_ms,
        "cyclesDone": s.cycles_done,
        "stopReason": s.stop_reason,
        "currentFreq": tidy(&s.current_freq),
    })
}

/// Identify's level, passes and blip length (the Mac's `identifyOutput`).
const IDENTIFY_DB: f32 = -12.0;
const IDENTIFY_PASSES: u16 = 2;
const IDENTIFY_BLIP_MS: f32 = 120.0;

/// The config Identify plays on `output`, or `None` when the device's valid
/// channel mask leaves that output out (or it is past the 16 the mask holds).
fn identify_config(valid_mask: u16, output: u8) -> Option<SiggenConfig> {
    let bit = 1u16.checked_shl(u32::from(output))?;
    let mask = bit & if valid_mask == 0 { bit } else { valid_mask };
    (mask != 0).then(|| SiggenConfig {
        signal_type: CHANNEL_ID,
        channel_mask: mask,
        level_db: IDENTIFY_DB,
        repeat: IDENTIFY_PASSES,
        p: vec![IDENTIFY_BLIP_MS, 0.0, 0.0, 0.0],
        ..SiggenConfig::default()
    })
}

fn identify(core: &DspiCore, output: u8) -> CallResult {
    let c = core.siggen_fetch_caps()?;
    if !c.supported {
        return Err(DspiError::Unsupported.into());
    }
    let cfg = identify_config(c.valid_mask, output).ok_or_else(|| {
        CallError::new(
            "invalid",
            format!("output {} cannot play a test signal", u16::from(output) + 1),
        )
    })?;
    core.siggen_set_config(cfg)?;
    core.siggen_control(SIGGEN_CTL_START)?;
    Ok(Value::Null)
}

// ------------------------------------------------------------- statistics

/// The counters Stats for nerbs shows, by the names the page reads.
const COUNTERS: [(&str, u16); 12] = [
    ("pdmRingOverruns", STAT_PDM_RING_OVERRUNS),
    ("pdmRingUnderruns", STAT_PDM_RING_UNDERRUNS),
    ("pdmDmaOverruns", STAT_PDM_DMA_OVERRUNS),
    ("pdmDmaUnderruns", STAT_PDM_DMA_UNDERRUNS),
    ("spdifOverruns", STAT_SPDIF_OVERRUNS),
    ("spdifUnderruns", STAT_SPDIF_UNDERRUNS),
    ("usbRingOverruns", STAT_USB_RING_OVERRUNS),
    ("clockHz", STAT_CLOCK_HZ),
    ("coreMv", STAT_CORE_MV),
    ("sampleRate", STAT_SAMPLE_RATE),
    ("temperature", STAT_TEMPERATURE),
    ("starvationTotal", STAT_STARVATION_TOTAL),
];

/// A section's read: the value, null for a device that does not answer it;
/// only a lost device fails the whole read.
fn section<T>(r: dspi_core::Result<T>, f: impl FnOnce(T) -> Value) -> Result<Value, CallError> {
    match r {
        Ok(v) => Ok(f(v)),
        Err(DspiError::Disconnected) => Err(DspiError::Disconnected.into()),
        Err(_) => Ok(Value::Null),
    }
}

fn stats_read(core: &DspiCore) -> CallResult {
    if !core.is_connected() {
        return Err(DspiError::Disconnected.into());
    }
    let mut counters = Map::new();
    for (name, which) in COUNTERS {
        counters.insert(name.into(), section(core.fetch_stat(which), |v| json!(v))?);
    }
    let starving = counters
        .get("starvationTotal")
        .and_then(Value::as_u64)
        .unwrap_or(0)
        > 0;
    let mut starvation = Vec::new();
    if starving {
        for i in 0..4 {
            starvation.push(section(
                core.fetch_stat(STAT_STARVATION_FIRST.saturating_add(i)),
                |v| json!(v),
            )?);
        }
    }
    // The ring health: null when the read failed (keep the last), `{present: false}` on firmware without it.
    let ring = section(core.fetch_ring_health(), |h| match h {
        Some(h) => {
            let mut v = tidy(&h);
            if let Value::Object(o) = &mut v {
                o.insert("present".into(), Value::Bool(true));
            }
            v
        }
        None => json!({ "present": false }),
    })?;
    let spdif_rx = if core.feature_present("input_source") {
        let rx = section(core.fetch_spdif_rx_status(), |r| {
            json!({
                "state": r.state,
                "inputSource": r.input_source,
                "lockCount": r.lock_count,
                "lossCount": r.loss_count,
                "sampleRate": r.sample_rate,
                "parityErrors": r.parity_errors,
                "fifoFillPct": r.fifo_fill_pct,
                "libState": r.lib_state,
                "stableCallbacks": r.stable_callbacks,
                "lostCallbacks": r.lost_callbacks,
            })
        })?;
        if let Value::Object(mut o) = rx {
            let source = o.get("inputSource").and_then(Value::as_u64).unwrap_or(0);
            let locked = o.get("state").and_then(Value::as_u64) == Some(2);
            // The active receiver's pin: S/PDIF 2..4 are sources 4..6, the rest receiver 0.
            let index = u8::try_from(source.saturating_sub(3))
                .ok()
                .filter(|i| (1..=3).contains(i) && source >= 4)
                .unwrap_or(0);
            let pin = match core.fetch_spdif_rx_pin(index) {
                Ok(p) if p != 0 => Some(p),
                Err(DspiError::Disconnected) => return Err(DspiError::Disconnected.into()),
                _ => core.fetch_spdif_rx_pin(0).ok(),
            };
            o.insert("pin".into(), json!(pin));
            let cs = if locked {
                section(core.fetch_spdif_rx_channel_status(), |b| json!(b))?
            } else {
                Value::Null
            };
            o.insert("channelStatus".into(), cs);
            Value::Object(o)
        } else {
            rx
        }
    } else {
        Value::Null
    };
    let lg = section(
        core.fetch_lg_status(),
        |l| json!({ "enabled": l.enabled, "present": l.present, "volume": l.volume, "muted": l.muted }),
    )?;
    let adat = section(
        core.fetch_adat_out_status(),
        |a| json!({ "enabled": a.enabled, "active": a.active, "pin": a.pin, "rateOk": a.rate_ok, "resyncCount": a.resync_count, "slipCount": a.slip_count }),
    )?;
    let i2s = section(
        core.fetch_input_lock(0),
        |l| json!({ "state": l.state, "clockMode": l.clock_mode, "detectedRate": l.detected_rate, "measuredHz": l.measured_hz, "lockCount": l.lock_count, "lossCount": l.loss_count }),
    )?;
    let clock = section(core.fetch_clock_diag(), |d| {
        if d.available {
            tidy(&d.fields)
        } else {
            Value::Null
        }
    })?;
    let usb = section(core.fetch_usb_error_stats(), |u| tidy(&u))?;
    Ok(json!({
        "counters": counters,
        "starvation": starvation,
        "ring": ring,
        "spdifRx": spdif_rx,
        "lg": lg,
        "adat": adat,
        "i2s": i2s,
        "clockDiag": clock,
        "usbErrors": usb,
    }))
}

#[cfg(test)]
mod tests_wp_wb6j {
    use super::identify_config;
    use crate::calls::{call, session_new};
    use crate::guard;
    use serde_json::{Value, json};

    fn run(h: u32, method: &str, params: Value) -> Value {
        let out = call(
            h,
            json!({ "method": method, "params": params })
                .to_string()
                .as_bytes(),
        );
        serde_json::from_slice(&out).unwrap()
    }

    fn ok(h: u32, method: &str, params: Value) -> Value {
        let r = run(h, method, params);
        assert_eq!(r["ok"], true, "{method}: {r}");
        r["value"].clone()
    }

    fn demo() -> u32 {
        guard::reset_for_test();
        let h = session_new().unwrap();
        ok(h, "open", json!({ "kind": "demo" }));
        h
    }

    #[test]
    fn wb6j_identify_is_held_to_the_valid_mask() {
        let c = identify_config(0b1111, 2).unwrap();
        assert_eq!(c.channel_mask, 0b100);
        assert_eq!(c.signal_type, 14);
        assert!((c.level_db - -12.0).abs() < f32::EPSILON);
        assert_eq!(c.repeat, 2);
        assert_eq!(c.p.len(), 4);
        assert!(
            identify_config(0b0011, 2).is_none(),
            "outside the valid mask"
        );
        assert!(identify_config(0, 16).is_none(), "past the mask's 16 bits");
        assert_eq!(identify_config(0, 3).unwrap().channel_mask, 0b1000);
    }

    #[test]
    fn wb6j_siggen_caps_carry_the_catalogue_and_the_device_ranges() {
        let h = demo();
        let c = ok(h, "siggenCaps", Value::Null);
        assert_eq!(c["supported"], true, "{c}");
        let types = c["types"].as_array().unwrap();
        assert_eq!(types.len(), 15);
        assert_eq!(types[0]["name"], "Sine");
        assert_eq!(types[0]["params"][0]["label"], "Frequency");
        assert_eq!(types[0]["params"][0]["unit"], "Hz");
        assert_eq!(types[14]["name"], "Channel ID");
        let tones = &types[12]["params"][0];
        assert!(
            tones["max"].as_f64().unwrap() <= c["multitoneMax"].as_f64().unwrap(),
            "{c}"
        );
    }

    #[test]
    fn wb6j_siggen_plays_only_when_started_and_stops() {
        let h = demo();
        // Staging does not play.
        let cfg = json!({ "signalType": 0, "channelMask": 3, "levelDb": -20.0, "p": [1000.0, 0.0, 0.0, 0.0] });
        ok(h, "siggenSet", json!({ "config": cfg }));
        assert_eq!(ok(h, "siggenStatus", Value::Null)["state"], 0);
        let back = ok(h, "siggenConfig", Value::Null);
        assert_eq!(back["channelMask"], 3, "{back}");
        ok(h, "siggenControl", json!({ "action": "start" }));
        assert_ne!(ok(h, "siggenStatus", Value::Null)["state"], 0);
        ok(h, "siggenControl", json!({ "action": "stopNow" }));
        assert_eq!(ok(h, "siggenStatus", Value::Null)["state"], 0);
        // A wrong action and a config without its four parameters are refused, nothing sent.
        assert_eq!(
            run(h, "siggenControl", json!({ "action": "louder" }))["ok"],
            false
        );
        let bad = json!({ "signalType": 0, "channelMask": 3, "levelDb": -20.0, "p": [1000.0] });
        assert_eq!(run(h, "siggenSet", json!({ "config": bad }))["ok"], false);
        // The monitor saw the generator's start and stop notifications.
        // (Natively the notifications come in on the core's reader: wait for them.)
        let until = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let mut m = Value::Null;
        while std::time::Instant::now() < until {
            ok(h, "poll", Value::Null);
            m = ok(h, "monitorRead", json!({ "afterId": 0 }));
            if m["entries"].as_array().is_some_and(|e| e.len() >= 2) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert!(
            m["entries"]
                .as_array()
                .unwrap()
                .iter()
                .any(|e| e["text"].as_str().unwrap_or("").contains("SiggenState")),
            "{m}"
        );
        ok(h, "monitorClear", Value::Null);
        let after = ok(h, "monitorRead", Value::Null);
        assert_eq!(after["entries"].as_array().unwrap().len(), 0, "{after}");
        // Ids carry on: a reader's last id stays valid.
        let next = ok(h, "monitorRead", json!({ "afterId": m["lastId"] }));
        assert_eq!(next["lastId"], m["lastId"], "{next}");
    }

    #[test]
    fn wb6j_identify_plays_channel_id_on_one_output() {
        let h = demo();
        ok(h, "siggenIdentify", json!({ "output": 1 }));
        let c = ok(h, "siggenConfig", Value::Null);
        assert_eq!(c["signalType"], 14, "{c}");
        assert_eq!(c["channelMask"], 2, "{c}");
        assert_eq!(c["levelDb"], -12.0, "{c}");
        assert_eq!(c["repeat"], 2, "{c}");
        assert_ne!(ok(h, "siggenStatus", Value::Null)["state"], 0);
        ok(h, "siggenControl", json!({ "action": "stopNow" }));
    }

    #[test]
    fn wb6j_siggen_is_refused_read_only() {
        let h = demo();
        ok(h, "setClientReadOnly", json!({ "on": true }));
        let r = run(h, "siggenIdentify", json!({ "output": 0 }));
        assert_eq!(r["ok"], false, "{r}");
        assert_eq!(ok(h, "siggenStatus", Value::Null)["state"], 0);
    }

    #[test]
    fn wb6j_stats_read_and_tick_on_the_demo() {
        let h = demo();
        let s = ok(h, "statsRead", Value::Null);
        assert!(
            s["counters"].as_object().unwrap().contains_key("clockHz"),
            "{s}"
        );
        assert!(s["ring"].is_object(), "{s}");
        let t = ok(h, "statsTick", Value::Null);
        assert!(t["buffers"].is_object(), "{t}");
        ok(h, "statsResetWatermarks", Value::Null);
    }

    #[test]
    fn wb6j_analyser_view_has_centres_and_ownership() {
        let h = demo();
        ok(h, "rtaStart", json!({ "tap": 0, "mask": 3 }));
        let v = ok(h, "analyserView", Value::Null);
        assert!(v["ownership"]["following"].is_boolean(), "{v}");
        assert!(v["caps"]["centres"].is_array(), "{v}");
        // -- WP WEB6 -- the telemetry of the Spectrum page's status line
        for k in [
            "liveCount",
            "refreshIntervalS",
            "lastFrameUs",
            "busyUsPerS",
            "bassBusyUsPerS",
        ] {
            assert!(v["snapshot"][k].is_number(), "{k}: {v}");
        }
        assert!(v["caps"]["bassDynamicRangeDb"].is_number(), "{v}");
        ok(h, "rtaTakeControl", Value::Null);
        ok(h, "rtaStop", Value::Null);
    }

    #[test]
    fn wb6j_calls_need_an_open_device() {
        guard::reset_for_test();
        let h = session_new().unwrap();
        for m in ["analyserView", "statsRead", "siggenStatus"] {
            assert_eq!(run(h, m, Value::Null)["ok"], false, "{m}");
        }
    }
}
