//! C functions: the open device's protocol generation and raw paged requests
//! by command id (WP PGCORE, PLAN-PG sections 1.1, 1.2 and 1.6), over
//! `dspi_core`'s `device_protocol`, `raw_get_id` and `raw_set_id`.
//!
//! Replies as in the parent module's docs: JSON (camelCase), the bytes
//! needed returned, nothing written when they do not fit. The raw calls
//! block for their transfer: call them off the UI thread.

use std::ffi::c_char;

use super::{c_bytes, done_reply, json_reply};
use crate::{FfiCore, fns_device::text};

/// The open device's protocol in JSON: `{ok, generation ("unknown",
/// "legacy" or "paged"), protocolMajor, protocolMinor, pages: [{page,
/// revision}], extEnvelope ("noChannel", "legacy" or "tunnel")}`. No
/// traffic. Returns the bytes needed including the NUL.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_device_protocol(core: *mut FfiCore, buf: *mut c_char, len: u32) -> u32 {
    text(core, buf, len, |c| json_reply(c.device_protocol()))
}

/// One paged GET of command `id` (`page << 8 | command`), `value` as
/// `wValue`, at most `want` bytes, in JSON: `{ok, value: [bytes]}` or the
/// failure reply (`refused` with the firmware's result code as `status` in
/// the error, `unsupported` on a 1.1.6-generation device, `invalid` for a
/// page-0 id or a length of 0).
#[unsafe(no_mangle)]
pub extern "C" fn dspi_raw_get_id(
    core: *mut FfiCore,
    id: u16,
    value: u16,
    want: u16,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    text(core, buf, len, |c| {
        json_reply(c.raw_get_id(id, value, want))
    })
}

/// One paged SET of command `id`, `value` as `wValue`, `data_len` bytes of
/// `data` as the payload (null: none, an action), in JSON: `{ok}` or the
/// failure reply (`invalid` with nothing sent for a request behind a
/// confirmation token, a core-voltage request, a flash or restart command,
/// a page-0 id, or one with no 1.1.6 equivalent that is not a plain value
/// SET).
#[unsafe(no_mangle)]
pub extern "C" fn dspi_raw_set_id(
    core: *mut FfiCore,
    id: u16,
    value: u16,
    data: *const u8,
    data_len: u32,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    let data = c_bytes(data, data_len);
    text(core, buf, len, |c| {
        done_reply(c.raw_set_id(id, value, data))
    })
}

#[cfg(test)]
mod tests_wp_pgcore {
    use std::ptr::null_mut;

    use serde_json::Value;

    use super::*;
    use crate::FfiCore;
    use crate::fns_device::{dspi_core_free, dspi_core_new};

    fn reply(f: impl Fn(*mut c_char, u32) -> u32) -> Value {
        let mut buf = vec![0u8; 4096];
        let n = f(buf.as_mut_ptr().cast(), 4096);
        assert!(n > 0 && n <= 4096);
        let end = buf.iter().position(|b| *b == 0).unwrap();
        serde_json::from_slice(&buf[..end]).unwrap()
    }

    #[test]
    fn wp_pgcore_the_c_calls_answer_without_a_device() {
        let h: *mut FfiCore = dspi_core_new();
        let r = reply(|b, l| dspi_device_protocol(h, b, l));
        assert_eq!(r["ok"], false);
        assert_eq!(r["code"], "disconnected");
        let r = reply(|b, l| dspi_raw_get_id(h, 0x0550, 0, 4, b, l));
        assert_eq!(r["code"], "disconnected");
        // Refused before the device: a page-0 id, the bootloader by its id
        let r = reply(|b, l| dspi_raw_get_id(h, 0x0050, 0, 4, b, l));
        assert_eq!(r["code"], "invalid", "{r}");
        let r = reply(|b, l| dspi_raw_set_id(h, 0x0130, 0, std::ptr::null(), 0, b, l));
        assert_eq!(r["code"], "invalid", "{r}");
        dspi_core_free(h);
        assert_eq!(dspi_device_protocol(null_mut(), null_mut(), 0), 0);
    }
}
