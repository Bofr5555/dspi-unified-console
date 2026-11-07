//! WP TM3: firmware images and bootloader drives for tests, shared by the unit
//! tests in `src/flash/` and the binary tests in `wp_tm3.rs`.

use std::path::Path;

const FLAG_FAMILY: u32 = 0x2000;
const XIP: u32 = 0x1000_0000;
const BI_START: u32 = 0x7188_EBF2;
const BI_END: u32 = 0xE71A_A390;
const BI_ID_AND_STRING: u16 = 6;
const TAG_RP: u16 = 0x5052;
const ID_NAME: u32 = 0x0203_1C86;
const ID_VERSION: u32 = 0x11A9_BC3A;
/// `BINARY_INFO_MAKE_TAG('D', 'S')`, and `dspi_core::STAMP_ID`.
const TAG_DS: u16 = 0x5344;
const ID_STAMP: u32 = 0x0B7A_4C15;

pub const RP2350: u32 = 0xE48B_FF59;
pub const RP2040: u32 = 0xE48B_FF56;

fn put(f: &mut [u8], at: usize, v: u32) {
    f[at..at + 4].copy_from_slice(&v.to_le_bytes());
}

/// A DSPi image for `family`: its version, and its core-voltage stamp if any.
pub fn image(family: u32, version: &str, stamp: Option<&str>) -> Vec<u8> {
    let mut entries = vec![(TAG_RP, ID_NAME, "DSPi"), (TAG_RP, ID_VERSION, version)];
    if let Some(mv) = stamp {
        entries.push((TAG_DS, ID_STAMP, mv));
    }
    let mut f = vec![0u8; 0x1000];
    let a = |off: usize| XIP + off as u32;
    put(&mut f, 0x100, BI_START);
    put(&mut f, 0x104, a(0x200));
    put(&mut f, 0x108, a(0x200 + 4 * entries.len()));
    put(&mut f, 0x10C, a(0x180));
    put(&mut f, 0x110, BI_END);
    for (i, (tag, id, text)) in entries.iter().enumerate() {
        let at = 0x400 + 16 * i;
        let s = 0x800 + 64 * i;
        put(&mut f, 0x200 + 4 * i, a(at));
        f[at..at + 2].copy_from_slice(&BI_ID_AND_STRING.to_le_bytes());
        f[at + 2..at + 4].copy_from_slice(&tag.to_le_bytes());
        put(&mut f, at + 4, *id);
        put(&mut f, at + 8, a(s));
        f[s..s + text.len()].copy_from_slice(text.as_bytes());
    }
    let pages: Vec<&[u8]> = f.chunks(256).collect();
    let mut out = Vec::new();
    for (i, page) in pages.iter().enumerate() {
        let mut b = vec![0u8; 512];
        put(&mut b, 0, 0x0A32_4655);
        put(&mut b, 4, 0x9E5D_5157);
        put(&mut b, 8, FLAG_FAMILY);
        put(&mut b, 12, XIP + 256 * i as u32);
        put(&mut b, 16, 256);
        put(&mut b, 20, i as u32);
        put(&mut b, 24, pages.len() as u32);
        put(&mut b, 28, family);
        b[32..32 + page.len()].copy_from_slice(page);
        put(&mut b, 508, 0x0AB1_6F30);
        out.extend(b);
    }
    out
}

/// What an RP2350's boot ROM shows at the root of its drive.
pub const RP2350_INFO: &str = "UF2 Bootloader v1.0\nModel: Raspberry Pi RP2350\nBoard-ID: RP2350\n";
pub const RP2040_INFO: &str = "UF2 Bootloader v3.0\nModel: Raspberry Pi RP2\nBoard-ID: RPI-RP2\n";

/// The boot ROM's drive arrives.
pub fn mount(drive: &Path, info: &str) {
    std::fs::create_dir_all(drive).unwrap();
    std::fs::write(drive.join("INFO_UF2.TXT"), info).unwrap();
}
