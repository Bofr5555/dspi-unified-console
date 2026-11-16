//! A bounded WAV reader and writer (WP P5SIM): captures the user exports
//! (PLAN-P5 S12), replay golden files and the web e2e's fake microphone.
//!
//! Reads RIFF/WAVE with PCM 16-bit or IEEE float 32-bit samples (plain or
//! `WAVE_FORMAT_EXTENSIBLE`), any channel count, and returns one channel as
//! `f32`. Writes mono PCM 16-bit or float 32-bit. A file is at most
//! [`MAX_WAV_BYTES`]; anything else (another sample format, a truncated chunk,
//! a non-finite float) is a typed [`MeasureError::Parse`], never a panic. The
//! input is untrusted: every length is checked against the bytes present
//! before anything is allocated.

use crate::types::MeasureError;

/// The largest WAV file read or written, bytes (64 MiB).
pub const MAX_WAV_BYTES: usize = 64 * 1024 * 1024;
/// The most channels a file may declare.
pub const MAX_WAV_CHANNELS: u16 = 64;

/// A WAV file's sample format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WavFormat {
    /// Signed 16-bit integer PCM.
    Pcm16,
    /// IEEE 754 32-bit float.
    Float32,
}

impl WavFormat {
    fn bytes(self) -> usize {
        match self {
            WavFormat::Pcm16 => 2,
            WavFormat::Float32 => 4,
        }
    }
}

/// What a WAV file holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WavInfo {
    /// Sample rate, Hz.
    pub rate_hz: u32,
    /// Channels (1 to [`MAX_WAV_CHANNELS`]).
    pub channels: u16,
    /// Sample format.
    pub format: WavFormat,
    /// Frames (samples per channel).
    pub frames: usize,
}

/// One channel of a WAV file.
#[derive(Debug, Clone, PartialEq)]
pub struct Wav {
    /// The file's description.
    pub info: WavInfo,
    /// The picked channel's samples, full scale ±1 (PCM 16 as `x / 32768`).
    pub samples: Vec<f32>,
}

const FORMAT_PCM: u16 = 1;
const FORMAT_FLOAT: u16 = 3;
const FORMAT_EXTENSIBLE: u16 = 0xFFFE;

fn parse_err(what: impl Into<String>) -> MeasureError {
    MeasureError::Parse(format!("WAV: {}", what.into()))
}

fn u16_at(b: &[u8], at: usize) -> Option<u16> {
    let s = b.get(at..at.checked_add(2)?)?;
    Some(u16::from_le_bytes([*s.first()?, *s.get(1)?]))
}

fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    let s = b.get(at..at.checked_add(4)?)?;
    let mut a = [0u8; 4];
    a.copy_from_slice(s);
    Some(u32::from_le_bytes(a))
}

/// The format chunk's fields and where the sample data lies.
struct Layout {
    info: WavInfo,
    data_at: usize,
}

fn layout(bytes: &[u8]) -> Result<Layout, MeasureError> {
    if bytes.len() > MAX_WAV_BYTES {
        return Err(parse_err(format!(
            "{} bytes (at most {MAX_WAV_BYTES})",
            bytes.len()
        )));
    }
    if bytes.get(0..4) != Some(b"RIFF".as_slice()) || bytes.get(8..12) != Some(b"WAVE".as_slice()) {
        return Err(parse_err("not a RIFF/WAVE file"));
    }
    let mut at = 12usize;
    let mut fmt: Option<(u16, u16, u32, u16)> = None;
    loop {
        let Some(id) = bytes.get(at..at.saturating_add(4)) else {
            return Err(parse_err("no data chunk"));
        };
        let size = u32_at(bytes, at.saturating_add(4))
            .ok_or_else(|| parse_err("truncated chunk header"))? as usize;
        let body = at.saturating_add(8);
        let end = body
            .checked_add(size)
            .filter(|e| *e <= bytes.len())
            .ok_or_else(|| parse_err("a chunk runs past the end of the file"))?;
        if id == b"fmt " {
            if size < 16 {
                return Err(parse_err("short format chunk"));
            }
            let get16 = |o: usize| u16_at(bytes, body.saturating_add(o));
            let (Some(tag), Some(ch), Some(rate), Some(bits)) = (
                get16(0),
                get16(2),
                u32_at(bytes, body.saturating_add(4)),
                get16(14),
            ) else {
                return Err(parse_err("short format chunk"));
            };
            let tag = if tag == FORMAT_EXTENSIBLE {
                // cbSize 22, then valid bits, channel mask, the sub-format
                // GUID whose first two bytes are the format tag.
                if size < 40 {
                    return Err(parse_err("short extensible format chunk"));
                }
                get16(24).ok_or_else(|| parse_err("short extensible format chunk"))?
            } else {
                tag
            };
            fmt = Some((tag, ch, rate, bits));
        } else if id == b"data" {
            let (tag, ch, rate, bits) = fmt.ok_or_else(|| parse_err("data before format"))?;
            let format = match (tag, bits) {
                (FORMAT_PCM, 16) => WavFormat::Pcm16,
                (FORMAT_FLOAT, 32) => WavFormat::Float32,
                _ => {
                    return Err(parse_err(format!(
                        "format {tag} with {bits} bits (PCM 16 or float 32 only)"
                    )));
                }
            };
            if ch == 0 || ch > MAX_WAV_CHANNELS {
                return Err(parse_err(format!("{ch} channels")));
            }
            if !(1..=768_000).contains(&rate) {
                return Err(parse_err(format!("rate {rate} Hz")));
            }
            let frame = format.bytes().saturating_mul(usize::from(ch));
            let frames = size.checked_div(frame).unwrap_or(0);
            return Ok(Layout {
                info: WavInfo {
                    rate_hz: rate,
                    channels: ch,
                    format,
                    frames,
                },
                data_at: body,
            });
        }
        // Chunks are padded to an even size.
        at = end.saturating_add(size & 1);
    }
}

/// A WAV file's description, without decoding its samples.
///
/// Errors: [`MeasureError::Parse`] as [`read_wav`].
pub fn wav_info(bytes: &[u8]) -> Result<WavInfo, MeasureError> {
    layout(bytes).map(|l| l.info)
}

/// Read channel `channel` (0 first; 0 for a mono file) of a WAV file.
///
/// Errors: [`MeasureError::Parse`] for a file past [`MAX_WAV_BYTES`], not
/// RIFF/WAVE, without a format or data chunk, a chunk running past the end, a
/// format other than PCM 16-bit or float 32-bit, no channels or more than
/// [`MAX_WAV_CHANNELS`], a rate of zero, or a non-finite float sample;
/// [`MeasureError::Invalid`] for a channel the file does not have.
pub fn read_wav(bytes: &[u8], channel: u16) -> Result<Wav, MeasureError> {
    let l = layout(bytes)?;
    let info = l.info;
    if channel >= info.channels {
        return Err(MeasureError::Invalid(format!(
            "channel {channel} of a {}-channel file",
            info.channels
        )));
    }
    let width = info.format.bytes();
    let frame = width.saturating_mul(usize::from(info.channels));
    let first = l
        .data_at
        .saturating_add(width.saturating_mul(usize::from(channel)));
    let data = bytes.get(first..).unwrap_or(&[]);
    let mut samples = Vec::with_capacity(info.frames);
    for chunk in data.chunks(frame).take(info.frames) {
        let v = match info.format {
            WavFormat::Pcm16 => {
                let (Some(a), Some(b)) = (chunk.first(), chunk.get(1)) else {
                    break;
                };
                f32::from(i16::from_le_bytes([*a, *b])) / 32768.0
            }
            WavFormat::Float32 => {
                let Some(s) = chunk.get(0..4) else {
                    break;
                };
                let mut a = [0u8; 4];
                a.copy_from_slice(s);
                let v = f32::from_le_bytes(a);
                if !v.is_finite() {
                    return Err(parse_err("a non-finite sample"));
                }
                v
            }
        };
        samples.push(v);
    }
    Ok(Wav {
        info: WavInfo {
            frames: samples.len(),
            ..info
        },
        samples,
    })
}

/// Write `samples` as a mono WAV file. PCM 16 clamps to ±1 and rounds
/// `x * 32767`; float 32 is exact.
///
/// Errors: [`MeasureError::Invalid`] for a rate outside 1 Hz to 768 kHz, a
/// non-finite sample, or a file that would pass [`MAX_WAV_BYTES`].
pub fn write_wav(
    samples: &[f32],
    rate_hz: u32,
    format: WavFormat,
) -> Result<Vec<u8>, MeasureError> {
    if !(1..=768_000).contains(&rate_hz) {
        return Err(MeasureError::Invalid(format!("rate {rate_hz} Hz")));
    }
    if !samples.iter().all(|v| v.is_finite()) {
        return Err(MeasureError::Invalid("a non-finite sample".into()));
    }
    let width = format.bytes();
    let data = samples
        .len()
        .checked_mul(width)
        .filter(|d| d.saturating_add(44) <= MAX_WAV_BYTES)
        .ok_or_else(|| MeasureError::Invalid("too long for a WAV file".into()))?;
    // Bounded by MAX_WAV_BYTES above, so these fit in u32.
    let data32 = data as u32;
    let (tag, bits) = match format {
        WavFormat::Pcm16 => (FORMAT_PCM, 16u16),
        WavFormat::Float32 => (FORMAT_FLOAT, 32u16),
    };
    let block = width as u16;
    let mut out = Vec::with_capacity(data.saturating_add(44));
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&data32.saturating_add(36).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&tag.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&rate_hz.to_le_bytes());
    out.extend_from_slice(&rate_hz.saturating_mul(u32::from(block)).to_le_bytes());
    out.extend_from_slice(&block.to_le_bytes());
    out.extend_from_slice(&bits.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data32.to_le_bytes());
    for &v in samples {
        match format {
            WavFormat::Pcm16 => {
                let s = (f64::from(v).clamp(-1.0, 1.0) * 32767.0).round() as i16;
                out.extend_from_slice(&s.to_le_bytes());
            }
            WavFormat::Float32 => out.extend_from_slice(&v.to_le_bytes()),
        }
    }
    Ok(out)
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_p5sim_wav {
    use super::*;
    use proptest::prelude::*;

    fn ramp(n: usize) -> Vec<f32> {
        (0..n).map(|i| ((i as f32) * 0.37).sin() * 0.9).collect()
    }

    /// A stereo file built by hand: left = a, right = b.
    fn stereo(a: &[f32], b: &[f32], format: WavFormat, extensible: bool) -> Vec<u8> {
        let width = format.bytes();
        let tag = match format {
            WavFormat::Pcm16 => FORMAT_PCM,
            WavFormat::Float32 => FORMAT_FLOAT,
        };
        let mut fmt = Vec::new();
        fmt.extend_from_slice(&(if extensible { FORMAT_EXTENSIBLE } else { tag }).to_le_bytes());
        fmt.extend_from_slice(&2u16.to_le_bytes());
        fmt.extend_from_slice(&48_000u32.to_le_bytes());
        fmt.extend_from_slice(&(48_000u32 * 2 * width as u32).to_le_bytes());
        fmt.extend_from_slice(&(2 * width as u16).to_le_bytes());
        fmt.extend_from_slice(&(8 * width as u16).to_le_bytes());
        if extensible {
            fmt.extend_from_slice(&22u16.to_le_bytes());
            fmt.extend_from_slice(&(8 * width as u16).to_le_bytes());
            fmt.extend_from_slice(&3u32.to_le_bytes());
            fmt.extend_from_slice(&tag.to_le_bytes());
            fmt.extend_from_slice(&[0, 0, 0, 0, 0x10, 0, 0x80, 0, 0, 0xAA, 0, 0x38, 0x9B, 0x71]);
        }
        let mut data = Vec::new();
        for (x, y) in a.iter().zip(b) {
            for v in [x, y] {
                match format {
                    WavFormat::Pcm16 => data.extend_from_slice(
                        &((f64::from(*v) * 32767.0).round() as i16).to_le_bytes(),
                    ),
                    WavFormat::Float32 => data.extend_from_slice(&v.to_le_bytes()),
                }
            }
        }
        let mut out = Vec::new();
        out.extend_from_slice(b"RIFF");
        let riff = 4 + 8 + fmt.len() + 8 + 6 + 8 + data.len();
        out.extend_from_slice(&(riff as u32).to_le_bytes());
        out.extend_from_slice(b"WAVE");
        out.extend_from_slice(b"fmt ");
        out.extend_from_slice(&(fmt.len() as u32).to_le_bytes());
        out.extend_from_slice(&fmt);
        // An odd-sized unknown chunk with its pad byte, which must be skipped.
        out.extend_from_slice(b"LIST");
        out.extend_from_slice(&5u32.to_le_bytes());
        out.extend_from_slice(&[1, 2, 3, 4, 5, 0]);
        out.extend_from_slice(b"data");
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out.extend_from_slice(&data);
        out
    }

    #[test]
    fn wp_p5sim_wav_float32_round_trips_exactly() {
        let s = ramp(1000);
        let bytes = write_wav(&s, 48_000, WavFormat::Float32).unwrap();
        let w = read_wav(&bytes, 0).unwrap();
        assert_eq!(w.info.rate_hz, 48_000);
        assert_eq!(w.info.channels, 1);
        assert_eq!(w.info.format, WavFormat::Float32);
        assert_eq!(w.info.frames, 1000);
        assert_eq!(w.samples, s);
        assert_eq!(wav_info(&bytes).unwrap(), w.info);
    }

    #[test]
    fn wp_p5sim_wav_pcm16_round_trips_within_one_step() {
        let mut s = ramp(1000);
        s.push(1.0);
        s.push(-1.0);
        s.push(2.0); // clamps
        let bytes = write_wav(&s, 44_100, WavFormat::Pcm16).unwrap();
        assert_eq!(bytes.len(), 44 + 2 * s.len());
        let w = read_wav(&bytes, 0).unwrap();
        assert_eq!(w.info.format, WavFormat::Pcm16);
        assert_eq!(w.info.rate_hz, 44_100);
        for (a, b) in s.iter().zip(&w.samples) {
            assert!((a.clamp(-1.0, 1.0) - b).abs() <= 1.5 / 32768.0, "{a} {b}");
        }
    }

    #[test]
    fn wp_p5sim_wav_picks_a_channel_of_a_stereo_file() {
        let a = ramp(300);
        let b: Vec<f32> = a.iter().map(|v| -v * 0.5).collect();
        for format in [WavFormat::Pcm16, WavFormat::Float32] {
            for ext in [false, true] {
                let bytes = stereo(&a, &b, format, ext);
                let l = read_wav(&bytes, 0).unwrap();
                let r = read_wav(&bytes, 1).unwrap();
                assert_eq!(l.info.channels, 2);
                assert_eq!(l.samples.len(), 300);
                let tol = if format == WavFormat::Pcm16 {
                    1.5 / 32768.0
                } else {
                    0.0
                };
                for i in 0..300 {
                    assert!((l.samples[i] - a[i]).abs() <= tol);
                    assert!((r.samples[i] - b[i]).abs() <= tol);
                }
                assert!(matches!(read_wav(&bytes, 2), Err(MeasureError::Invalid(_))));
            }
        }
    }

    #[test]
    fn wp_p5sim_wav_refuses_what_it_cannot_read() {
        let good = write_wav(&ramp(10), 48_000, WavFormat::Pcm16).unwrap();
        let parse = |b: &[u8]| matches!(read_wav(b, 0), Err(MeasureError::Parse(_)));
        assert!(parse(b""));
        assert!(parse(b"RIFF"));
        assert!(parse(b"not a wav file at all, just text"));
        // Truncated data chunk.
        assert!(parse(&good[..good.len() - 3]));
        // 24-bit PCM.
        let mut b24 = good.clone();
        b24[34] = 24;
        assert!(parse(&b24));
        // 8-bit and a-law are refused too.
        let mut alaw = good.clone();
        alaw[20] = 6;
        assert!(parse(&alaw));
        // Zero channels, zero rate.
        let mut c0 = good.clone();
        c0[22] = 0;
        assert!(parse(&c0));
        let mut r0 = good.clone();
        r0[24..28].copy_from_slice(&0u32.to_le_bytes());
        assert!(parse(&r0));
        // A data size far past the file: no allocation, a typed error.
        let mut huge = good.clone();
        huge[40..44].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(parse(&huge));
        // Data before format.
        let mut swapped = b"RIFF\0\0\0\0WAVEdata\x02\0\0\0\0\0".to_vec();
        swapped.extend_from_slice(&good[12..36]);
        assert!(parse(&swapped));
        // A NaN float sample.
        let nan = write_wav(&[0.5, 0.25], 48_000, WavFormat::Float32).unwrap();
        let mut nan = nan;
        nan[44..48].copy_from_slice(&f32::NAN.to_le_bytes());
        assert!(parse(&nan));
        // Past 64 MiB.
        let big = vec![0u8; MAX_WAV_BYTES + 1];
        assert!(parse(&big));
    }

    #[test]
    fn wp_p5sim_wav_writer_refuses_bad_input() {
        assert!(write_wav(&[f32::NAN], 48_000, WavFormat::Pcm16).is_err());
        assert!(write_wav(&[f32::INFINITY], 48_000, WavFormat::Float32).is_err());
        assert!(write_wav(&[0.0], 0, WavFormat::Pcm16).is_err());
        let too_long = vec![0f32; MAX_WAV_BYTES / 4];
        assert!(write_wav(&too_long, 48_000, WavFormat::Float32).is_err());
        assert!(write_wav(&[], 48_000, WavFormat::Pcm16).is_ok());
    }

    proptest! {
        #[test]
        fn proptest_wp_p5sim_wav_reader_never_panics_on_bytes(bytes in proptest::collection::vec(any::<u8>(), 0..512), ch in 0u16..4) {
            if let Ok(w) = read_wav(&bytes, ch) {
                prop_assert!(w.samples.iter().all(|v| v.is_finite()));
                prop_assert_eq!(w.samples.len(), w.info.frames);
            }
            let _ = wav_info(&bytes);
        }

        #[test]
        fn proptest_wp_p5sim_wav_reader_never_panics_on_mutated_files(
            float in any::<bool>(),
            n in 0usize..64,
            muts in proptest::collection::vec((0usize..400, any::<u8>()), 1..8),
            cut in 0usize..400,
        ) {
            let fmt = if float { WavFormat::Float32 } else { WavFormat::Pcm16 };
            let mut b = write_wav(&ramp(n), 48_000, fmt).unwrap();
            for (at, v) in muts {
                if let Some(x) = b.get_mut(at) { *x = v; }
            }
            b.truncate(cut.max(1).min(b.len()));
            if let Ok(w) = read_wav(&b, 0) {
                prop_assert!(w.samples.iter().all(|v| v.is_finite() && v.abs() <= f32::MAX));
            }
        }

        #[test]
        fn proptest_wp_p5sim_wav_round_trips(samples in proptest::collection::vec(-1.0f32..=1.0, 0..300), rate in 1u32..=768_000) {
            let b = write_wav(&samples, rate, WavFormat::Float32).unwrap();
            let w = read_wav(&b, 0).unwrap();
            prop_assert_eq!(w.samples, samples.clone());
            let b = write_wav(&samples, rate, WavFormat::Pcm16).unwrap();
            let w = read_wav(&b, 0).unwrap();
            prop_assert!(w.samples.iter().zip(&samples).all(|(a, b)| (a - b).abs() <= 1.5 / 32768.0));
        }

        #[test]
        fn proptest_wp_p5sim_wav_writer_never_panics(samples in proptest::collection::vec(proptest::num::f32::ANY, 0..64), rate in any::<u32>()) {
            for fmt in [WavFormat::Pcm16, WavFormat::Float32] {
                if let Ok(b) = write_wav(&samples, rate, fmt) {
                    prop_assert!(read_wav(&b, 0).is_ok());
                }
            }
        }
    }
}
