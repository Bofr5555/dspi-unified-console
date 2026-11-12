//! A zip archive with every entry stored, not compressed (WP P0E).
//!
//! A diagnostics bundle is a few hundred kilobytes of text, so compression
//! buys little and a dependency would buy less: this is the format's plain
//! subset (APPNOTE 6.3.10 sections 4.3 and 4.4: local headers, the central
//! directory, its end record), which every unzip tool reads. Names are UTF-8
//! (general-purpose flag bit 11). No ZIP64: an entry or archive that would
//! need it is refused with [`TooLarge`].

/// An archive past what a zip without ZIP64 can describe (4 GiB, 65 535
/// entries).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TooLarge;

/// Entries in order, with the time they all carry.
pub(crate) struct ZipWriter {
    out: Vec<u8>,
    central: Vec<u8>,
    entries: u16,
    dos_time: u16,
    dos_date: u16,
}

const LOCAL_SIG: u32 = 0x0403_4b50;
const CENTRAL_SIG: u32 = 0x0201_4b50;
const END_SIG: u32 = 0x0605_4b50;
/// Version 2.0: what a stored entry with a directory needs to extract.
const VERSION: u16 = 20;
/// Bit 11: the name is UTF-8.
const FLAG_UTF8: u16 = 1 << 11;

fn u16le(v: &mut Vec<u8>, x: u16) {
    v.extend_from_slice(&x.to_le_bytes());
}

fn u32le(v: &mut Vec<u8>, x: u32) {
    v.extend_from_slice(&x.to_le_bytes());
}

/// CRC-32 (IEEE 802.3, reflected, as zip uses), bit by bit: a bundle is
/// small enough that a table is not worth its 1 KiB.
pub(crate) fn crc32(data: &[u8]) -> u32 {
    let mut crc = !0u32;
    for &b in data {
        crc ^= u32::from(b);
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}

impl ZipWriter {
    /// An empty archive whose entries carry this UTC time (`(y, mo, d, h,
    /// mi, s)`). Before 1980, which DOS dates cannot hold, reads as 1980-01-01.
    pub(crate) fn new(utc: (i64, u32, u32, u32, u32, u32)) -> Self {
        let (y, mo, d, h, mi, s) = utc;
        let (dos_date, dos_time) = match u16::try_from(y.saturating_sub(1980)) {
            Ok(years) if years <= 127 => (
                (years << 9) | (to16(mo) << 5) | to16(d),
                (to16(h) << 11) | (to16(mi) << 5) | (to16(s) / 2),
            ),
            _ => ((1 << 5) | 1, 0),
        };
        Self {
            out: Vec::new(),
            central: Vec::new(),
            entries: 0,
            dos_time,
            dos_date,
        }
    }

    /// Add a file. `name` uses `/` between folders.
    pub(crate) fn add(&mut self, name: &str, data: &[u8]) -> Result<(), TooLarge> {
        let size = u32::try_from(data.len()).map_err(|_| TooLarge)?;
        let name_len = u16::try_from(name.len()).map_err(|_| TooLarge)?;
        let offset = u32::try_from(self.out.len()).map_err(|_| TooLarge)?;
        let entries = self.entries.checked_add(1).ok_or(TooLarge)?;
        let crc = crc32(data);

        let header = |v: &mut Vec<u8>| {
            u16le(v, FLAG_UTF8);
            u16le(v, 0); // stored
            u16le(v, self.dos_time);
            u16le(v, self.dos_date);
            u32le(v, crc);
            u32le(v, size);
            u32le(v, size);
            u16le(v, name_len);
            u16le(v, 0); // no extra field
        };

        let mut local = Vec::with_capacity(30usize.saturating_add(name.len()));
        u32le(&mut local, LOCAL_SIG);
        u16le(&mut local, VERSION);
        header(&mut local);
        local.extend_from_slice(name.as_bytes());

        let mut central = Vec::with_capacity(46usize.saturating_add(name.len()));
        u32le(&mut central, CENTRAL_SIG);
        u16le(&mut central, VERSION); // made by: MS-DOS, 2.0
        u16le(&mut central, VERSION);
        header(&mut central);
        u16le(&mut central, 0); // no comment
        u16le(&mut central, 0); // disk 0
        u16le(&mut central, 0); // internal attributes
        u32le(&mut central, 0); // external attributes
        u32le(&mut central, offset);
        central.extend_from_slice(name.as_bytes());

        // The archive must stay describable: the central directory's offset
        // is a u32 too.
        let end = self
            .out
            .len()
            .checked_add(local.len())
            .and_then(|n| n.checked_add(data.len()))
            .ok_or(TooLarge)?;
        u32::try_from(end).map_err(|_| TooLarge)?;

        self.out.extend_from_slice(&local);
        self.out.extend_from_slice(data);
        self.central.extend_from_slice(&central);
        self.entries = entries;
        Ok(())
    }

    /// The archive's bytes.
    pub(crate) fn finish(mut self) -> Result<Vec<u8>, TooLarge> {
        let dir_offset = u32::try_from(self.out.len()).map_err(|_| TooLarge)?;
        let dir_size = u32::try_from(self.central.len()).map_err(|_| TooLarge)?;
        dir_offset.checked_add(dir_size).ok_or(TooLarge)?;
        let central = std::mem::take(&mut self.central);
        self.out.extend_from_slice(&central);
        let mut end = Vec::with_capacity(22);
        u32le(&mut end, END_SIG);
        u16le(&mut end, 0); // this disk
        u16le(&mut end, 0); // the directory's disk
        u16le(&mut end, self.entries);
        u16le(&mut end, self.entries);
        u32le(&mut end, dir_size);
        u32le(&mut end, dir_offset);
        u16le(&mut end, 0); // no comment
        self.out.extend_from_slice(&end);
        Ok(self.out)
    }
}

/// A calendar field as a DOS bit field; every caller's value is below 60.
fn to16(v: u32) -> u16 {
    u16::try_from(v).unwrap_or(0)
}
