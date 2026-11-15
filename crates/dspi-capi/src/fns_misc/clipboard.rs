//! C functions: the channel clipboard (WP CLIP1, PARITY M45), over
//! `DspiCore::copy_channel` and `DspiCore::paste_channel`.
//!
//! A copy answers the clip as its JSON document (`dspi_core::
//! channel_clip_to_json`: `{kind: "dspi.channelClip", version, ...}`), which
//! the client keeps as it likes (its clipboard, a file) and hands back to
//! `dspi_paste_channel`. A document of another version is refused.

use std::ffi::c_char;

use dspi_core::{ChannelPasteOptions, DspiError, channel_clip_from_json, channel_clip_to_json};

use super::{arg, document_reply, done_reply, text};
use crate::FfiCore;

// -- WP CLIP1 --
/// Copy wire channel `ch` (inputs, then outputs) from the mirror, nothing
/// sent: the clip document, or a JSON failure (`disconnected`, `invalid` for
/// a channel out of range). An input gives its PEQ bank; an output its PEQ
/// and crossover banks, gain, delay and mute.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_copy_channel(core: *mut FfiCore, ch: u8, buf: *mut c_char, len: u32) -> u32 {
    text(core, buf, len, |c| {
        document_reply(c.copy_channel(ch).and_then(channel_clip_to_json))
    })
}

// -- WP CLIP1 --
/// Paste the clip document `clip` (from `dspi_copy_channel`) onto wire
/// channel `ch`, by the Console's rules: the PEQ bank (onto `mirror` too,
/// the linked partner's wire channel, when 0..255; a negative `mirror` is
/// none), and from an output to an output the crossover bank, gain, delay
/// (clamped to this device's line) and mute. One undo step. JSON `{ok}` or
/// a failure; a refusal (`invalid` for another clip version or a bad
/// channel or value, `read_only`, `installer_lock`, `limit_exceeded`, ...)
/// writes nothing.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_paste_channel(
    core: *mut FfiCore,
    clip: *const c_char,
    ch: u8,
    mirror: i32,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    text(core, buf, len, |c| {
        done_reply((|| {
            let clip = channel_clip_from_json(arg(clip, "clip")?.to_owned())?;
            let mirror = match mirror {
                m if m < 0 => None,
                m => Some(u8::try_from(m).map_err(|_| DspiError::InvalidArgument {
                    msg: format!("linked partner {m} (0..255, or negative for none)"),
                })?),
            };
            c.paste_channel(clip, ch, ChannelPasteOptions { mirror })
        })())
    })
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_clip1 {
    use std::ffi::{CString, c_char};
    use std::ptr;

    use serde_json::Value;

    use super::super::{dspi_open_demo, dspi_undo};
    use super::*;
    use crate::fns_device::{dspi_core_free, dspi_core_new};
    use crate::with_core;

    /// One call's reply, into a buffer large enough for any of these.
    fn reply_text(f: impl Fn(*mut c_char, u32) -> u32) -> String {
        let mut buf = vec![0u8; 64 * 1024];
        let n = f(buf.as_mut_ptr().cast(), buf.len() as u32) as usize;
        assert!(n > 0 && n <= buf.len());
        buf.truncate(n);
        assert_eq!(buf.pop(), Some(0));
        String::from_utf8(buf).unwrap()
    }

    fn json(s: &str) -> Value {
        serde_json::from_str(s).unwrap()
    }

    #[test]
    fn wp_clip1_through_c_a_copy_pastes_and_one_undo_takes_it_back() {
        assert_eq!(dspi_copy_channel(ptr::null_mut(), 0, ptr::null_mut(), 0), 0);
        let h = dspi_core_new();
        let r = json(&reply_text(|b, l| dspi_copy_channel(h, 0, b, l)));
        assert_eq!(r["code"], "disconnected", "{r}");
        assert!(dspi_open_demo(h));
        let ni = with_core(h, 0, |x| x.core.get_state().num_input_channels);
        let (src, dst) = (ni, ni + 1);
        with_core(h, (), |x| {
            x.core.set_output_gain(0, -7.0).unwrap();
            x.core.set_output_mute(0, true).unwrap();
        });
        let before = with_core(h, None, |x| Some(x.core.get_state())).unwrap();

        let doc = reply_text(|b, l| dspi_copy_channel(h, src, b, l));
        let v = json(&doc);
        assert_eq!(v["kind"], "dspi.channelClip", "{doc}");
        assert_eq!(v["version"], 1);
        let clip = CString::new(doc.clone()).unwrap();
        let r = json(&reply_text(|b, l| {
            dspi_paste_channel(h, clip.as_ptr(), dst, -1, b, l)
        }));
        assert_eq!(r["ok"], true, "{r}");
        let s = with_core(h, None, |x| Some(x.core.get_state())).unwrap();
        assert_eq!((s.output_gain_db[1], s.output_muted[1]), (-7.0, true));

        let r = json(&reply_text(|b, l| dspi_undo(h, b, l)));
        assert_eq!(r["ok"], true, "{r}");
        let s = with_core(h, None, |x| Some(x.core.get_state())).unwrap();
        assert_eq!(s.output_gain_db, before.output_gain_db);
        assert_eq!(s.output_muted, before.output_muted);

        // Another version, a bad partner, a null clip: refused.
        let mut old = json(&doc);
        old["version"] = 2.into();
        let old = CString::new(old.to_string()).unwrap();
        for (what, p, m) in [
            ("version", old.as_ptr(), -1),
            ("partner", clip.as_ptr(), 300),
            ("null", ptr::null(), -1),
        ] {
            let r = json(&reply_text(|b, l| dspi_paste_channel(h, p, dst, m, b, l)));
            assert_eq!(r["code"], "invalid", "{what}: {r}");
        }
        dspi_core_free(h);
    }
}
