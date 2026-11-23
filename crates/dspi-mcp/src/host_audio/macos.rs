//! macOS: Core Audio's devices (`AudioObjectGetPropertyData` on the system
//! object), the default output, each output's channels (its output stream
//! configuration), rate, whether it runs, its volume and whether that volume
//! is the device's own control (settable: the USB volume the DSPi's user
//! volume follows). From macOS 14.2 the process objects say which app plays
//! to which device; before that only "in use". A USB device's UID holds its
//! serial (`AppleUSBAudioEngine:<vendor>:<product>:<serial>:…`).
//! Compile-checked in CI against an Apple target; the pure parts are tested
//! on every OS.

use std::ffi::{c_char, c_void};

use super::{HostAudio, HostAudioSource, HostOs, HostOutput, HostStream};

type ObjectId = u32;
type OsStatus = i32;

#[repr(C)]
struct Address {
    selector: u32,
    scope: u32,
    element: u32,
}

#[repr(C)]
struct AudioBuffer {
    channels: u32,
    data_size: u32,
    data: *mut c_void,
}

#[repr(C)]
struct AudioBufferList {
    count: u32,
    buffers: [AudioBuffer; 1],
}

#[link(name = "CoreAudio", kind = "framework")]
unsafe extern "C" {
    fn AudioObjectHasProperty(id: ObjectId, address: *const Address) -> u8;
    fn AudioObjectIsPropertySettable(
        id: ObjectId,
        address: *const Address,
        settable: *mut u8,
    ) -> OsStatus;
    fn AudioObjectGetPropertyDataSize(
        id: ObjectId,
        address: *const Address,
        qualifier_size: u32,
        qualifier: *const c_void,
        size: *mut u32,
    ) -> OsStatus;
    fn AudioObjectGetPropertyData(
        id: ObjectId,
        address: *const Address,
        qualifier_size: u32,
        qualifier: *const c_void,
        size: *mut u32,
        data: *mut c_void,
    ) -> OsStatus;
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFStringGetCString(s: *const c_void, buf: *mut c_char, size: isize, encoding: u32) -> u8;
    fn CFRelease(p: *const c_void);
}

const fn fourcc(s: &[u8; 4]) -> u32 {
    u32::from_be_bytes(*s)
}

const SYSTEM_OBJECT: ObjectId = 1;
const GLOBAL: u32 = fourcc(b"glob");
const OUTPUT: u32 = fourcc(b"outp");
const MAIN: u32 = 0;
const DEVICES: u32 = fourcc(b"dev#");
const DEFAULT_OUTPUT: u32 = fourcc(b"dOut");
const NAME: u32 = fourcc(b"lnam");
const UID: u32 = fourcc(b"uid ");
const STREAM_CONFIGURATION: u32 = fourcc(b"slay");
const NOMINAL_RATE: u32 = fourcc(b"nsrt");
const RUNNING_SOMEWHERE: u32 = fourcc(b"gone");
const VOLUME_SCALAR: u32 = fourcc(b"volm");
const MUTE: u32 = fourcc(b"mute");
// macOS 14.2: the process objects
const PROCESSES: u32 = fourcc(b"prs#");
const PROCESS_BUNDLE_ID: u32 = fourcc(b"pbid");
const PROCESS_DEVICES: u32 = fourcc(b"pdv#");
const PROCESS_RUNNING_OUTPUT: u32 = fourcc(b"piro");
const UTF8: u32 = 0x0800_0100;
const SYSTEM: &str = "Core Audio";

/// The macOS reader.
#[derive(Debug, Clone, Copy, Default)]
pub struct CoreAudio;

fn address(selector: u32, scope: u32, element: u32) -> Address {
    Address {
        selector,
        scope,
        element,
    }
}

fn has(id: ObjectId, a: &Address) -> bool {
    // SAFETY: a valid address struct; Core Audio only reads it.
    unsafe { AudioObjectHasProperty(id, a) != 0 }
}

/// A fixed-size property (`T` plain data).
fn get<T: Copy + Default>(id: ObjectId, a: &Address) -> Option<T> {
    if !has(id, a) {
        return None;
    }
    let mut value = T::default();
    let mut size = u32::try_from(std::mem::size_of::<T>()).ok()?;
    // SAFETY: `size` is `value`'s size; Core Audio writes at most that.
    let status = unsafe {
        AudioObjectGetPropertyData(
            id,
            a,
            0,
            std::ptr::null(),
            &mut size,
            (&raw mut value).cast(),
        )
    };
    (status == 0).then_some(value)
}

/// A variable-size property, as 8-byte-aligned words.
fn get_bytes(id: ObjectId, a: &Address) -> Option<Vec<u64>> {
    if !has(id, a) {
        return None;
    }
    let mut size = 0u32;
    // SAFETY: asks the size only.
    if unsafe { AudioObjectGetPropertyDataSize(id, a, 0, std::ptr::null(), &mut size) } != 0 {
        return None;
    }
    let words = usize::try_from(size).ok()?.div_ceil(8);
    let mut buf = vec![0u64; words.max(1)];
    // SAFETY: the buffer holds at least `size` bytes.
    let status = unsafe {
        AudioObjectGetPropertyData(
            id,
            a,
            0,
            std::ptr::null(),
            &mut size,
            buf.as_mut_ptr().cast(),
        )
    };
    (status == 0).then_some(buf)
}

fn ids(id: ObjectId, a: &Address) -> Vec<ObjectId> {
    get_bytes(id, a)
        .map(|words| {
            words
                .iter()
                .flat_map(|w| {
                    let b = w.to_ne_bytes();
                    [
                        u32::from_ne_bytes([b[0], b[1], b[2], b[3]]),
                        u32::from_ne_bytes([b[4], b[5], b[6], b[7]]),
                    ]
                })
                .filter(|&i| i != 0)
                .collect()
        })
        .unwrap_or_default()
}

fn string(id: ObjectId, a: &Address) -> String {
    let Some(cf) = get::<usize>(id, a).filter(|&p| p != 0) else {
        return String::new();
    };
    let cf = cf as *const c_void;
    let mut buf = [0 as c_char; 512];
    // SAFETY: a CFString the property gave us (we own it, released below);
    // the buffer's size is passed.
    let ok = unsafe { CFStringGetCString(cf, buf.as_mut_ptr(), 512, UTF8) != 0 };
    // SAFETY: as above.
    unsafe { CFRelease(cf) };
    if !ok {
        return String::new();
    }
    // SAFETY: CFStringGetCString wrote a NUL-terminated string.
    unsafe { std::ffi::CStr::from_ptr(buf.as_ptr()) }
        .to_string_lossy()
        .into_owned()
}

/// The output channels: the sum over the output stream configuration.
fn output_channels(id: ObjectId) -> Option<u32> {
    let buf = get_bytes(id, &address(STREAM_CONFIGURATION, OUTPUT, MAIN))?;
    let list = buf.as_ptr().cast::<AudioBufferList>();
    // SAFETY: Core Audio wrote an AudioBufferList of `count` buffers into
    // `buf`, which is 8-byte aligned; we read no further than its length.
    let n = unsafe { (*list).count };
    let room = buf
        .len()
        .saturating_mul(8)
        .saturating_sub(8)
        .checked_div(std::mem::size_of::<AudioBuffer>())
        .unwrap_or(0);
    let n = usize::try_from(n).ok()?.min(room);
    // SAFETY: as above, `n` within the buffer.
    let total: u32 = unsafe {
        let first = (&raw const (*list).buffers).cast::<AudioBuffer>();
        (0..n).map(|i| (*first.add(i)).channels).sum()
    };
    Some(total)
}

/// The output volume (main element, else channel 1) and whether the
/// device lets it be set: the device's own control.
fn volume(id: ObjectId) -> (Option<f32>, Option<bool>) {
    for element in [MAIN, 1] {
        let a = address(VOLUME_SCALAR, OUTPUT, element);
        if let Some(v) = get::<f32>(id, &a) {
            let mut settable = 0u8;
            // SAFETY: a valid address; writes one byte.
            let ok = unsafe { AudioObjectIsPropertySettable(id, &a, &mut settable) } == 0;
            return (
                Some((v * 1000.0).round() / 10.0),
                ok.then_some(settable != 0),
            );
        }
    }
    // No OS volume of its own: the OS cannot drive the device's volume
    (None, Some(false))
}

/// Which apps play to which device (macOS 14.2 and later).
fn processes() -> Option<Vec<(String, bool, Vec<ObjectId>)>> {
    let list = address(PROCESSES, GLOBAL, MAIN);
    if !has(SYSTEM_OBJECT, &list) {
        return None;
    }
    let out = ids(SYSTEM_OBJECT, &list)
        .into_iter()
        .map(|p| {
            let app = string(p, &address(PROCESS_BUNDLE_ID, GLOBAL, MAIN));
            let running = get::<u32>(p, &address(PROCESS_RUNNING_OUTPUT, GLOBAL, MAIN))
                .is_some_and(|r| r != 0);
            let devices = ids(p, &address(PROCESS_DEVICES, OUTPUT, MAIN));
            (app, running, devices)
        })
        .collect();
    Some(out)
}

impl HostAudioSource for CoreAudio {
    fn read(&self) -> HostAudio {
        let all = ids(SYSTEM_OBJECT, &address(DEVICES, GLOBAL, MAIN));
        if all.is_empty() {
            return HostAudio::unknown(
                HostOs::MacOs,
                SYSTEM,
                "Core Audio listed no audio devices to this app",
            );
        }
        let default = get::<ObjectId>(SYSTEM_OBJECT, &address(DEFAULT_OUTPUT, GLOBAL, MAIN));
        let apps = processes();
        let mut outputs = Vec::new();
        for id in all {
            let Some(channels) = output_channels(id).filter(|&c| c > 0) else {
                continue; // an input-only device
            };
            let (volume_pct, hardware_volume) = volume(id);
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let rate_hz = get::<f64>(id, &address(NOMINAL_RATE, GLOBAL, MAIN))
                .filter(|r| r.is_finite() && *r > 0.0)
                .map(|r| r.round() as u32);
            let streams = apps
                .as_ref()
                .map(|list| {
                    list.iter()
                        .filter(|(_, _, devices)| devices.contains(&id))
                        .map(|(app, running, _)| HostStream {
                            app: app.clone(),
                            playing: Some(*running),
                            ..HostStream::default()
                        })
                        .collect()
                })
                .unwrap_or_default();
            let uid = string(id, &address(UID, GLOBAL, MAIN));
            outputs.push(HostOutput {
                name: string(id, &address(NAME, GLOBAL, MAIN)),
                serial: uid.clone(),
                id: uid,
                is_default: Some(default == Some(id)),
                channels: Some(channels),
                rate_hz,
                running: get::<u32>(id, &address(RUNNING_SOMEWHERE, GLOBAL, MAIN)).map(|r| r != 0),
                volume_pct,
                muted: get::<u32>(id, &address(MUTE, OUTPUT, MAIN)).map(|m| m != 0),
                hardware_volume,
                streams,
            });
        }
        let known = apps.is_some();
        HostAudio {
            os: HostOs::MacOs,
            audio_system: SYSTEM.into(),
            unknown: None,
            outputs,
            streams_known: known,
            streams_note: (!known).then(|| {
                "macOS before 14.2 does not say which app plays to which output; \
                 `running` says whether something does"
                    .to_owned()
            }),
        }
    }
}
