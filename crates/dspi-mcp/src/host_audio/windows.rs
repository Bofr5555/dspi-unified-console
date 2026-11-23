//! Windows: the render endpoints (MMDevice), the default one, each one's
//! volume (`IAudioEndpointVolume`, with whether it is the device's own
//! control) and its audio sessions (`IAudioSessionManager2`: one per app).
//! The USB serial comes from the device tree above the endpoint, as the Qt
//! app's `HostAudioFormatWindows.cpp` finds it. Compile-checked in CI
//! (cargo xwin); the pure parts are tested on every OS.

use windows::Win32::Devices::DeviceAndDriverInstallation::{
    CM_Get_Device_IDW, CM_Get_Parent, CM_LOCATE_DEVNODE_NORMAL, CM_Locate_DevNodeW, CR_SUCCESS,
    MAX_DEVICE_ID_LEN,
};
use windows::Win32::Foundation::{CloseHandle, S_OK};
use windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
use windows::Win32::Media::Audio::{
    AudioSessionStateActive, AudioSessionStateExpired, DEVICE_STATE_ACTIVE, IAudioSessionControl2,
    IAudioSessionManager2, IMMDevice, IMMDeviceEnumerator, ISimpleAudioVolume, MMDeviceEnumerator,
    eConsole, eRender,
};
use windows::Win32::System::Com::{
    CLSCTX_ALL, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoTaskMemFree,
    CoUninitialize, STGM_READ,
};
use windows::Win32::System::Threading::{
    OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
};
use windows::Win32::UI::Shell::PropertiesSystem::{IPropertyStore, PROPERTYKEY};
use windows::core::{GUID, Interface, PCWSTR, PWSTR};

use super::{HostAudio, HostAudioSource, HostOs, HostOutput, HostStream};

const SYSTEM: &str = "Windows audio";

// Spelled out as the Qt app has them (FunctionDiscoveryKeys)
const FRIENDLY_NAME: PROPERTYKEY = PROPERTYKEY {
    fmtid: GUID::from_u128(0xa45c254e_df1c_4efd_8020_67d146a850e0),
    pid: 14,
};
const INTERFACE_NAME: PROPERTYKEY = PROPERTYKEY {
    fmtid: GUID::from_u128(0x026e516e_b814_414b_83cd_856d6fef4822),
    pid: 2,
};
const DEVICE_FORMAT: PROPERTYKEY = PROPERTYKEY {
    fmtid: GUID::from_u128(0xf19f064d_082c_4e27_bc73_6882a1bb8e4c),
    pid: 0,
};
/// `ENDPOINT_HARDWARE_SUPPORT_VOLUME`.
const HARDWARE_VOLUME: u32 = 0x1;
/// `VT_BLOB`.
const VT_BLOB: u16 = 65;

/// The Windows reader.
#[derive(Debug, Clone, Copy, Default)]
pub struct Wasapi;

impl HostAudioSource for Wasapi {
    fn read(&self) -> HostAudio {
        // SAFETY: COM on this thread for this read; balanced below.
        let init = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
        let out =
            read_endpoints().unwrap_or_else(|why| HostAudio::unknown(HostOs::Windows, SYSTEM, why));
        if init.is_ok() {
            // SAFETY: matches the successful CoInitializeEx above.
            unsafe { CoUninitialize() };
        }
        out
    }
}

fn take_wide(p: PWSTR) -> String {
    if p.is_null() {
        return String::new();
    }
    // SAFETY: a COM-allocated, NUL-terminated string; freed once here.
    let s = unsafe { p.to_string() }.unwrap_or_default();
    // SAFETY: as above.
    unsafe { CoTaskMemFree(Some(p.0.cast_const().cast())) };
    s
}

fn string_prop(store: &IPropertyStore, key: &PROPERTYKEY) -> String {
    // SAFETY: a valid key; the PROPVARIANT owns and frees its value.
    unsafe { store.GetValue(key) }
        .map(|v| v.to_string())
        .unwrap_or_default()
}

/// The endpoint format's channels and rate (a WAVEFORMATEX blob).
fn format(store: &IPropertyStore) -> (Option<u32>, Option<u32>) {
    // SAFETY: as string_prop; the blob is read only while `v` lives.
    let Ok(v) = (unsafe { store.GetValue(&DEVICE_FORMAT) }) else {
        return (None, None);
    };
    let raw = v.as_raw();
    // SAFETY: the union's variant is the one `vt` names.
    let (vt, blob) = unsafe {
        (
            raw.Anonymous.Anonymous.vt,
            raw.Anonymous.Anonymous.Anonymous.blob,
        )
    };
    if vt != VT_BLOB || blob.pBlobData.is_null() {
        return (None, None);
    }
    let len = usize::try_from(blob.cbSize).unwrap_or(0);
    // SAFETY: cbSize bytes at pBlobData, owned by `v`.
    let bytes = unsafe { std::slice::from_raw_parts(blob.pBlobData, len) };
    super::wave_format(bytes)
}

/// The device instance ids from the endpoint up the tree.
fn instance_chain(id: &str) -> Vec<String> {
    let mut chain = Vec::new();
    let node: Vec<u16> = format!("SWD\\MMDEVAPI\\{id}")
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    let mut inst = 0u32;
    // SAFETY: a NUL-terminated id; `inst` is written on success.
    if unsafe { CM_Locate_DevNodeW(&mut inst, PCWSTR(node.as_ptr()), CM_LOCATE_DEVNODE_NORMAL) }
        != CR_SUCCESS
    {
        return chain;
    }
    // A few levels are all there is; the bound keeps a broken tree short
    for _ in 0..8 {
        let mut text = [0u16; MAX_DEVICE_ID_LEN as usize + 1];
        // SAFETY: the buffer is MAX_DEVICE_ID_LEN + 1 wide.
        if unsafe { CM_Get_Device_IDW(inst, &mut text, 0) } != CR_SUCCESS {
            break;
        }
        let end = text.iter().position(|&c| c == 0).unwrap_or(text.len());
        chain.push(String::from_utf16_lossy(text.get(..end).unwrap_or(&[])));
        let mut parent = 0u32;
        // SAFETY: `parent` is written on success.
        if unsafe { CM_Get_Parent(&mut parent, inst, 0) } != CR_SUCCESS {
            break;
        }
        inst = parent;
    }
    chain
}

fn process_name(pid: u32) -> String {
    if pid == 0 {
        return String::new();
    }
    // SAFETY: a query-only handle, closed below.
    let Ok(h) = (unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }) else {
        return String::new();
    };
    let mut buf = [0u16; 512];
    let mut len = buf.len() as u32;
    // SAFETY: `len` is the buffer's size in characters.
    let ok = unsafe {
        QueryFullProcessImageNameW(h, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut len)
    }
    .is_ok();
    // SAFETY: opened above.
    let _ = unsafe { CloseHandle(h) };
    if !ok {
        return String::new();
    }
    let path = String::from_utf16_lossy(buf.get(..len as usize).unwrap_or(&[]));
    path.rsplit('\\')
        .next()
        .unwrap_or("")
        .trim_end_matches(".exe")
        .to_owned()
}

fn sessions(device: &IMMDevice) -> Vec<HostStream> {
    let mut out = Vec::new();
    // SAFETY: COM calls on live interfaces; errors skip the session.
    unsafe {
        let Ok(manager) = device.Activate::<IAudioSessionManager2>(CLSCTX_ALL, None) else {
            return out;
        };
        let Ok(list) = manager.GetSessionEnumerator() else {
            return out;
        };
        let n = list.GetCount().unwrap_or(0);
        for i in 0..n {
            let Ok(control) = list.GetSession(i) else {
                continue;
            };
            let Ok(state) = control.GetState() else {
                continue;
            };
            if state == AudioSessionStateExpired {
                continue;
            }
            let Ok(c2) = control.cast::<IAudioSessionControl2>() else {
                continue;
            };
            // S_OK: the system sounds; S_FALSE: an app's
            let system = c2.IsSystemSoundsSession() == S_OK;
            let display = c2.GetDisplayName().map(take_wide).unwrap_or_default();
            let app = if system {
                "System sounds".to_owned()
            } else {
                process_name(c2.GetProcessId().unwrap_or(0))
            };
            let volume = control.cast::<ISimpleAudioVolume>().ok();
            out.push(HostStream {
                app,
                title: display,
                channels: None,
                volume_pct: volume
                    .as_ref()
                    .and_then(|v| v.GetMasterVolume().ok())
                    .map(|x| (x * 1000.0).round() / 10.0),
                muted: volume
                    .as_ref()
                    .and_then(|v| v.GetMute().ok())
                    .map(|b| b.as_bool()),
                playing: Some(state == AudioSessionStateActive),
            });
        }
    }
    out
}

fn read_endpoints() -> Result<HostAudio, &'static str> {
    // SAFETY: COM calls on live interfaces; each failure is a reason.
    unsafe {
        let enumerator: IMMDeviceEnumerator =
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)
                .map_err(|_| "the Windows audio service could not be reached")?;
        let default = enumerator
            .GetDefaultAudioEndpoint(eRender, eConsole)
            .ok()
            .and_then(|d| d.GetId().ok())
            .map(take_wide);
        let list = enumerator
            .EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE)
            .map_err(|_| "Windows did not list its playback devices")?;
        let n = list.GetCount().unwrap_or(0);
        let mut outputs = Vec::new();
        for i in 0..n {
            let Ok(device) = list.Item(i) else { continue };
            let id = device.GetId().map(take_wide).unwrap_or_default();
            let Ok(store) = device.OpenPropertyStore(STGM_READ) else {
                continue;
            };
            let friendly = string_prop(&store, &FRIENDLY_NAME);
            let interface = string_prop(&store, &INTERFACE_NAME);
            let (channels, rate_hz) = format(&store);
            let volume = device
                .Activate::<IAudioEndpointVolume>(CLSCTX_ALL, None)
                .ok();
            let streams = sessions(&device);
            outputs.push(HostOutput {
                serial: super::usb_serial_from_instance_ids(&instance_chain(&id)),
                is_default: Some(default.as_deref() == Some(id.as_str())),
                name: if interface.is_empty() || friendly.contains(&interface) {
                    friendly
                } else {
                    format!("{friendly} ({interface})")
                },
                id,
                channels,
                rate_hz,
                running: Some(streams.iter().any(|s| s.playing == Some(true))),
                volume_pct: volume
                    .as_ref()
                    .and_then(|v| v.GetMasterVolumeLevelScalar().ok())
                    .map(|x| (x * 1000.0).round() / 10.0),
                muted: volume
                    .as_ref()
                    .and_then(|v| v.GetMute().ok())
                    .map(|b| b.as_bool()),
                hardware_volume: volume
                    .as_ref()
                    .and_then(|v| v.QueryHardwareSupport().ok())
                    .map(|f| f & HARDWARE_VOLUME != 0),
                streams,
            });
        }
        Ok(HostAudio {
            os: HostOs::Windows,
            audio_system: SYSTEM.into(),
            unknown: None,
            outputs,
            streams_known: true,
            streams_note: None,
        })
    }
}
