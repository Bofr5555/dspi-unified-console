//! WP PGFAKE2 (PLAN-PG section 4, "custom rows (both)"): the fake's own answers
//! to the paged commands that the 14 `custom` rows of `legacy_map.json` reach.
//!
//! These rows have no one-to-one rule, so a registry-driven inverse (PGFAKE1's
//! paged front) cannot answer the commands they use from a 1.1.6 opcode. This
//! module answers each of them natively from the fake's state, written from the
//! migration guide (upstream `release/v1.1.7` `103ba37`,
//! `Documentation/Features/vendor_protocol_migration_guide.md`) and not from
//! `dspi_proto::paged::custom` (PGCUST), so the planner and the fake cannot share
//! a bug (PLAN-PG section 4, "Independence of the oracles"):
//!
//! | Row | Paged commands answered here | Guide |
//! |---|---|---|
//! | `GET_STATUS` 0x50 | `MEAS.LEVELS`, `SYS.SYSTEM_STATUS`, `SYS.PDM_STATS`, `SYS.SPDIF_TX_STATS`, `SYS.USB_AUDIO_STATUS`, `SYS.PIPELINE_STATS`, `INPUT.STATUS`, `DIAG.LOOPBACK_STATS` | 6.3 |
//! | `SET_EQ_PARAM`/`GET_EQ_PARAM` 0x42/0x43 | `FILTER.BAND` GET and SET | 6.5 |
//! | `SET_MATRIX_ROUTE`/`GET_MATRIX_ROUTE` 0x70/0x71 | `ROUTE.MATRIX_ROUTE` GET and SET | 6.5 |
//! | `CLEAR_CLIPS` 0x83 | `MEAS.CLIP_FLAGS` GET, `MEAS.CLIP_CLEAR` SET | 6.5 |
//! | `GET_INPUT_RATE` 0xEE | `SYS.SYSTEM_STATUS`, `INPUT.I2S_RX_RATE` GET | 6.5 |
//! | `PRESET_GET_STARTUP` 0x97 | `STORE.STARTUP`, `STORE.DIRECTORY` GET | 6.5 |
//! | `GET_CS_DISPLAY_CFG` 0x28 | `CS.DISPLAY_CAPS`, `CS.DISPLAY_CONFIG` GET | 6.5 |
//! | `SET_ALL_PARAMS` 0xA1 | `STORE.ALL_PARAMS` SET (exactly 6136 bytes) | 6.5, 9.2 |
//! | `SAVE_PARAMS` 0x51 (retired) | `STORE.PRESET_ACTIVE` GET, `STORE.PRESET_SAVE` SET | 6.7, `legacy_map.json` |
//! | `SET_PREAMP` 0x44 (retired) | `SYS.PLATFORM` GET, `ROUTE.PREAMP` SET | 6.7 |
//! | `GET_USB_ERROR_STATS`/`RESET_USB_ERROR_STATS` 0xB2/0xB3 (retired) | none: nothing reaches the wire | 6.7 |
//!
//! A refusal is `Err(code)`, a `VC_ERR_*` result code (guide 5.3), which the
//! paged front turns into a STALL with `SYS.LAST_RESULT` set. The 1.1.7 rules
//! the commands share are applied here too, so the module stands alone: a GET
//! with `wLength` 0 or a SET outside the registry's bounds is `VC_ERR_LENGTH`, the
//! wrong direction `VC_ERR_DIRECTION`, a non-zero `wValue` on a command that
//! addresses nothing `VC_ERR_ADDRESS`, a non-finite float `VC_ERR_VALUE`. Not
//! modelled: the one pending band write (`VC_ERR_BUSY`, guide 6.5 rule 2; a band
//! write applies at once here), and live counters (they read as the fake's
//! `GET_STATUS` does: quiet and plausible).
//!
//! For the PGFAKE1 merge: the paged front asks [`State::custom_paged_get`] and
//! [`State::custom_paged_set`] for every id in [`CUSTOM_IDS`]; `None` means "not
//! a custom-row command, use the registry inverse". Some of these commands are
//! also the target of a `direct` or `act_*` row (`STORE.PRESET_ACTIVE`,
//! `STORE.PRESET_SAVE`, `STORE.DIRECTORY`, `SYS.PLATFORM`, `ROUTE.PREAMP`); the
//! answers here equal the same-meaning legacy opcode's, so either order works.
#![warn(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

use dspi_proto::generated::{self, notify as nfy, opcodes as op, status as st};
use dspi_proto::paged::legacy::{
    VC_ERR_ADDRESS, VC_ERR_DIRECTION, VC_ERR_LENGTH, VC_ERR_UNKNOWN, VC_ERR_VALUE,
};
use dspi_proto::paged::registry::{self, AddrKind};
use dspi_proto::paged::{CmdId, PagedDir, cmds};

use super::{B_BYPASS, B_QP, BAND_LEN, C_GAIN, CROSSPOINT_LEN, F32, State, field_of};

/// A `VC_ERR_*` result code (migration guide 5.3).
pub(super) type Vc = u8;

/// Every paged command a `custom` row of `legacy_map.json` uses, with the
/// direction it uses it in (the rows' `uses` lists, deduplicated). A test holds
/// it equal to the generated table.
pub(super) const CUSTOM_IDS: &[(CmdId, PagedDir)] = &[
    (cmds::CS_DISPLAY_CAPS, PagedDir::Get),
    (cmds::CS_DISPLAY_CONFIG, PagedDir::Get),
    (cmds::FILTER_BAND, PagedDir::Get),
    (cmds::FILTER_BAND, PagedDir::Set),
    (cmds::SYS_PLATFORM, PagedDir::Get),
    (cmds::ROUTE_PREAMP, PagedDir::Set),
    (cmds::MEAS_LEVELS, PagedDir::Get),
    (cmds::SYS_SYSTEM_STATUS, PagedDir::Get),
    (cmds::SYS_PDM_STATS, PagedDir::Get),
    (cmds::SYS_SPDIF_TX_STATS, PagedDir::Get),
    (cmds::SYS_USB_AUDIO_STATUS, PagedDir::Get),
    (cmds::SYS_PIPELINE_STATS, PagedDir::Get),
    (cmds::INPUT_STATUS, PagedDir::Get),
    (cmds::DIAG_LOOPBACK_STATS, PagedDir::Get),
    (cmds::STORE_PRESET_ACTIVE, PagedDir::Get),
    (cmds::STORE_PRESET_SAVE, PagedDir::Set),
    (cmds::ROUTE_MATRIX_ROUTE, PagedDir::Set),
    (cmds::ROUTE_MATRIX_ROUTE, PagedDir::Get),
    (cmds::MEAS_CLIP_FLAGS, PagedDir::Get),
    (cmds::MEAS_CLIP_CLEAR, PagedDir::Set),
    (cmds::STORE_STARTUP, PagedDir::Get),
    (cmds::STORE_DIRECTORY, PagedDir::Get),
    (cmds::STORE_ALL_PARAMS, PagedDir::Set),
    (cmds::INPUT_I2S_RX_RATE, PagedDir::Get),
];

/// Whether `id` in `dir` is answered here.
pub(super) fn is_custom(id: CmdId, dir: PagedDir) -> bool {
    CUSTOM_IDS.contains(&(id, dir))
}

/// The little-endian bytes of a list of `u32` fields.
fn words(v: &[u32]) -> Vec<u8> {
    v.iter().flat_map(|w| w.to_le_bytes()).collect()
}

/// A finite `f32` at `b[at..at + 4]`.
fn finite_at(b: &[u8], at: usize) -> bool {
    b.get(at..at.saturating_add(4))
        .and_then(|s| <[u8; 4]>::try_from(s).ok())
        .is_some_and(|w| f32::from_le_bytes(w).is_finite())
}

impl State {
    /// The 1.1.7 rules every request here shares: the direction, the GET's
    /// `wLength`, the SET's length bounds, and `wValue` 0 on a command that
    /// addresses nothing.
    fn custom_common(id: CmdId, dir: PagedDir, value: u16, len: usize) -> Result<(), Vc> {
        let Some(c) = registry::command(id) else {
            return Err(VC_ERR_UNKNOWN);
        };
        if !c.has(dir) {
            return Err(VC_ERR_DIRECTION);
        }
        match dir {
            PagedDir::Get if len == 0 => return Err(VC_ERR_LENGTH),
            PagedDir::Set if !c.set_len_ok(len) => return Err(VC_ERR_LENGTH),
            _ => {}
        }
        if c.addr == AddrKind::None && value != 0 {
            return Err(VC_ERR_ADDRESS);
        }
        Ok(())
    }

    /// A `GET_STATUS` selector's `u32`, as the fake answers it on page 0.
    fn sel(&self, selector: u16) -> u32 {
        let b = self.get_status(selector);
        b.get(..4)
            .and_then(|w| <[u8; 4]>::try_from(w).ok())
            .map_or(0, u32::from_le_bytes)
    }

    /// Byte `2N + k` of the meter block (`MEAS.LEVELS`), N = `NUM_CHANNELS`.
    fn meter_tail(&self, k: usize, len: usize) -> Vec<u8> {
        let at = usize::from(self.num_channels())
            .saturating_mul(2)
            .saturating_add(k);
        self.status
            .get(at..at.saturating_add(len))
            .map_or_else(|| vec![0; len], <[u8]>::to_vec)
    }

    /// The fake's legacy reply to `opcode`, for a split or combined row.
    fn legacy_reply(&mut self, opcode: u8, value: u16, want: usize) -> Option<Vec<u8>> {
        self.get(opcode, value, want).ok()
    }

    /// A paged GET of a command a custom row uses: `None` for any other command;
    /// `Some(Err(code))` refuses it. The reply is whole: the caller truncates to
    /// `wLength`.
    pub(super) fn custom_paged_get(
        &mut self,
        id: CmdId,
        value: u16,
        want: usize,
    ) -> Option<Result<Vec<u8>, Vc>> {
        if !is_custom(id, PagedDir::Get) {
            return None;
        }
        if let Err(code) = Self::custom_common(id, PagedDir::Get, value, want) {
            return Some(Err(code));
        }
        Some(self.custom_get(id, value))
    }

    fn custom_get(&mut self, id: CmdId, value: u16) -> Result<Vec<u8>, Vc> {
        match id {
            // Guide 6.5 `GET_CS_DISPLAY_CFG`: the old 16 bytes were the 4 caps
            // bytes, then the 12-byte configuration. A build without control
            // surfaces has neither command.
            cmds::CS_DISPLAY_CAPS | cmds::CS_DISPLAY_CONFIG => {
                let old = self
                    .legacy_reply(op::REQ_GET_CS_DISPLAY_CFG, 0, 16)
                    .ok_or(VC_ERR_UNKNOWN)?;
                let part = if id == cmds::CS_DISPLAY_CAPS {
                    old.get(..4)
                } else {
                    old.get(4..16)
                };
                part.map(<[u8]>::to_vec).ok_or(VC_ERR_UNKNOWN)
            }
            // Guide 6.5: `wValue = (channel << 8) | band`, one 16-byte layout:
            // type, bypass (0 or 1), Qp (0 for a crossover band), freq, Q, gain.
            cmds::FILTER_BAND => {
                let [band, ch] = value.to_le_bytes();
                let (off, peq) = self.band_at(ch, band).ok_or(VC_ERR_ADDRESS)?;
                let mut rec = self.bytes(off, BAND_LEN).ok_or(VC_ERR_ADDRESS)?;
                if let Some(b) = rec.get_mut(B_BYPASS.1) {
                    *b = u8::from(*b == 1);
                }
                if !peq && let Some(qp) = rec.get_mut(B_QP.1..B_QP.1.saturating_add(B_QP.2)) {
                    qp.fill(0);
                }
                Ok(rec)
            }
            // Guide 6.5: `wValue = (input << 8) | output`; the GET no longer
            // echoes the address: enabled, phase, reserved u16, gain.
            cmds::ROUTE_MATRIX_ROUTE => {
                let [output, input] = value.to_le_bytes();
                let off = self.route_at(input, output).ok_or(VC_ERR_ADDRESS)?;
                let rec = self.bytes(off, CROSSPOINT_LEN).ok_or(VC_ERR_ADDRESS)?;
                let (en, ph) = (
                    rec.first().copied().unwrap_or(0),
                    rec.get(1).copied().unwrap_or(0),
                );
                let mut r = vec![u8::from(en != 0), u8::from(ph != 0), 0, 0];
                r.extend_from_slice(field_of(&rec, C_GAIN).ok_or(VC_ERR_ADDRESS)?);
                Ok(r)
            }
            // Guide 6.3: byte for byte the old selector-9 packet.
            cmds::MEAS_LEVELS => Ok(self.status.clone()),
            // Guide 6.5 `CLEAR_CLIPS`: the latched flags, `2N + 2` of
            // `MEAS.LEVELS`; reading does not clear.
            cmds::MEAS_CLIP_FLAGS => Ok(self.meter_tail(2, 4)),
            // Guide 6.3, `SYS.SYSTEM_STATUS` (0x0140, 16 bytes).
            cmds::SYS_SYSTEM_STATUS => {
                let mut r = words(&[self.sel(13), self.sel(15)]);
                r.extend_from_slice(&u16::try_from(self.sel(14)).unwrap_or(u16::MAX).to_le_bytes());
                // Selector 16 is the temperature sign-extended to 32 bits.
                let [t0, t1, ..] = self.sel(16).to_le_bytes();
                r.extend([t0, t1]);
                r.extend(self.meter_tail(0, 2));
                r.extend([0, 0]);
                Ok(r)
            }
            // Guide 6.3, `SYS.USB_AUDIO_STATUS` (0x0141, 16 bytes).
            cmds::SYS_USB_AUDIO_STATUS => {
                let mut r = words(&[self.sel(10), self.sel(22), self.sel(30)]);
                let [alt, ..] = self.sel(11).to_le_bytes();
                let [mounted, ..] = self.sel(12).to_le_bytes();
                r.extend([alt, mounted]);
                r.extend([0, 0]);
                Ok(r)
            }
            // Guide 6.3, `SYS.PIPELINE_STATS` (0x0143, 24 bytes).
            cmds::SYS_PIPELINE_STATS => Ok(words(&[
                self.sel(26),
                self.sel(27),
                self.sel(28),
                self.sel(29),
                self.sel(31),
                0,
            ])),
            // Guide 6.3, `SYS.PDM_STATS` (0x0144): four u32 counters.
            cmds::SYS_PDM_STATS => Ok(words(&[self.sel(3), self.sel(4), self.sel(5), self.sel(6)])),
            // Guide 6.3, `SYS.SPDIF_TX_STATS` (0x0145): six u32 counters.
            cmds::SYS_SPDIF_TX_STATS => Ok(words(&[
                self.sel(8),
                self.sel(17),
                self.sel(18),
                self.sel(19),
                self.sel(20),
                self.sel(21),
            ])),
            // Guide 6.3, `INPUT.STATUS` (0x0311, 12 bytes).
            cmds::INPUT_STATUS => {
                let source = self
                    .legacy_reply(op::REQ_GET_INPUT_SOURCE, 0, 1)
                    .and_then(|b| b.first().copied())
                    .unwrap_or(0);
                let [count, ..] = self.sel(23).to_le_bytes();
                let mut r = vec![source, count, 0, 0];
                r.extend(words(&[self.sel(32), self.sel(33)]));
                Ok(r)
            }
            // Guide 6.3: `DSPI_LOOPBACK` builds only; a release build (this
            // fake) answers `VC_ERR_UNKNOWN`, which the row reads as 0.
            cmds::DIAG_LOOPBACK_STATS => Err(VC_ERR_UNKNOWN),
            // `SAVE_PARAMS`' first step, as `PRESET_GET_ACTIVE`.
            cmds::STORE_PRESET_ACTIVE => Ok(vec![self.presets.active]),
            // Guide 6.5 `PRESET_GET_STARTUP`: `{mode, default slot}`.
            cmds::STORE_STARTUP => Ok(vec![self.presets.startup_mode, self.presets.default_slot]),
            // Seven bytes, unchanged; the last active slot is byte 4.
            cmds::STORE_DIRECTORY => Ok(self.preset_dir()),
            // Guide 6.5 `GET_INPUT_RATE`: the selected I2S rate, the old reply's
            // bytes 4 to 7.
            cmds::INPUT_I2S_RX_RATE => self
                .legacy_reply(op::REQ_GET_INPUT_RATE, 0, 8)
                .and_then(|b| b.get(4..8).map(<[u8]>::to_vec))
                .ok_or(VC_ERR_UNKNOWN),
            // Guide 4.6: the 1.1.6 layout, 7 bytes.
            cmds::SYS_PLATFORM => {
                let mut p = self.platform.clone();
                p.resize(7, 0);
                Ok(p)
            }
            _ => Err(VC_ERR_UNKNOWN),
        }
    }

    /// A paged SET of a command a custom row uses: `None` for any other command;
    /// `Some(Err(code))` refuses it and changes nothing.
    pub(super) fn custom_paged_set(
        &mut self,
        id: CmdId,
        value: u16,
        data: &[u8],
    ) -> Option<Result<(), Vc>> {
        if !is_custom(id, PagedDir::Set) {
            return None;
        }
        if let Err(code) = Self::custom_common(id, PagedDir::Set, value, data.len()) {
            return Some(Err(code));
        }
        Some(self.custom_set(id, value, data))
    }

    fn custom_set(&mut self, id: CmdId, value: u16, data: &[u8]) -> Result<(), Vc> {
        let host = nfy::PARAM_SRC_HOST_SET as u8;
        match id {
            // Guide 6.5: 16 bytes, stored as sent (the Qp included: 1.1.7 never
            // keeps the stored one); exactly 1 bypasses; a crossover band's Qp is
            // ignored. A band in the gap 10-19, or a crossover band on an input,
            // is `VC_ERR_ADDRESS`; a non-finite float `VC_ERR_VALUE`.
            cmds::FILTER_BAND => {
                let [band, ch] = value.to_le_bytes();
                let (off, peq) = self.band_at(ch, band).ok_or(VC_ERR_ADDRESS)?;
                if ![4, 8, 12].iter().all(|at| finite_at(data, *at)) {
                    return Err(VC_ERR_VALUE);
                }
                let mut rec = data.get(..BAND_LEN).ok_or(VC_ERR_LENGTH)?.to_vec();
                if let Some(b) = rec.get_mut(B_BYPASS.1) {
                    *b = u8::from(*b == 1);
                }
                if !peq && let Some(qp) = rec.get_mut(B_QP.1..B_QP.1.saturating_add(B_QP.2)) {
                    qp.fill(0);
                }
                self.write(off, &rec, host);
                Ok(())
            }
            // Guide 6.5: `wValue = (input << 8) | output`, payload enabled,
            // phase, reserved u16, gain.
            cmds::ROUTE_MATRIX_ROUTE => {
                let [output, input] = value.to_le_bytes();
                let off = self.route_at(input, output).ok_or(VC_ERR_ADDRESS)?;
                if !finite_at(data, 4) {
                    return Err(VC_ERR_VALUE);
                }
                let [en, ph, _, _, g0, g1, g2, g3] =
                    *data.get(..CROSSPOINT_LEN).ok_or(VC_ERR_LENGTH)?
                else {
                    return Err(VC_ERR_LENGTH);
                };
                let rec = [u8::from(en != 0), u8::from(ph != 0), 0, 0, g0, g1, g2, g3];
                self.write(off, &rec, host);
                Ok(())
            }
            // Guide 6.5 `CLEAR_CLIPS`, step 2.
            cmds::MEAS_CLIP_CLEAR => {
                self.clear_clips();
                Ok(())
            }
            // Guide 6.5 and 9.2: exactly 6136 bytes (the length rule is the
            // registry's); a header the apply refuses is `VC_ERR_VALUE`.
            cmds::STORE_ALL_PARAMS => {
                if data.len() != generated::BULK_SIZE {
                    return Err(VC_ERR_LENGTH);
                }
                if self.apply_bulk(data.to_vec()) {
                    Ok(())
                } else {
                    Err(VC_ERR_VALUE)
                }
            }
            // `SAVE_PARAMS`' second step: save the live state to the slot.
            cmds::STORE_PRESET_SAVE => {
                if self.preset_save(value) == st::PRESET_OK as u8 {
                    Ok(())
                } else {
                    Err(VC_ERR_ADDRESS)
                }
            }
            // `SET_PREAMP`'s per-input write (`wValue` = input), as
            // `SET_PREAMP_CH`; an input past the end is `VC_ERR_ADDRESS`.
            cmds::ROUTE_PREAMP => {
                if value >= u16::from(self.num_inputs()) {
                    return Err(VC_ERR_ADDRESS);
                }
                if !finite_at(data, 0) {
                    return Err(VC_ERR_VALUE);
                }
                let off = self
                    .locate("preamp", 4, usize::from(value), F32)
                    .ok_or(VC_ERR_ADDRESS)?;
                self.write(off, data.get(..F32.2).ok_or(VC_ERR_LENGTH)?, host);
                self.sync_legacy_preamp();
                Ok(())
            }
            _ => Err(VC_ERR_UNKNOWN),
        }
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]
mod tests_wp_pgfake2 {
    use super::*;
    use crate::Transport;
    use crate::fake::FakeDevice;
    use dspi_proto::paged::legacy::{Adapter, LEGACY_OPS};
    use std::collections::BTreeSet;

    fn get(d: &FakeDevice, id: CmdId, value: u16, want: usize) -> Result<Vec<u8>, Vc> {
        let mut s = d.shared.lock();
        let mut r = s.custom_paged_get(id, value, want).expect("a custom-row command")?;
        r.truncate(want);
        Ok(r)
    }

    fn set(d: &FakeDevice, id: CmdId, value: u16, data: &[u8]) -> Result<(), Vc> {
        d.shared
            .lock()
            .custom_paged_set(id, value, data)
            .expect("a custom-row command")
    }

    fn u32_at(b: &[u8], at: usize) -> u32 {
        u32::from_le_bytes(b[at..at + 4].try_into().unwrap())
    }

    /// `CUSTOM_IDS` is exactly the union of the generated custom rows' `uses`,
    /// and every entry is a registry command with that direction.
    #[test]
    fn wp_pgfake2_custom_ids_are_the_custom_rows_uses() {
        let mut from_map = BTreeSet::new();
        let mut rows = 0;
        for o in LEGACY_OPS.iter() {
            for a in [o.r#in, o.out].into_iter().flatten() {
                if let Adapter::Custom(c) = a {
                    rows += 1;
                    for (id, dir) in c.uses {
                        from_map.insert((id.0, *dir == PagedDir::Set));
                    }
                }
            }
        }
        assert_eq!(rows, 14, "the 14 custom rows of legacy_map.json");
        let ours: BTreeSet<_> = CUSTOM_IDS
            .iter()
            .map(|(id, dir)| (id.0, *dir == PagedDir::Set))
            .collect();
        assert_eq!(ours.len(), CUSTOM_IDS.len(), "no duplicates");
        assert_eq!(ours, from_map);
        for (id, dir) in CUSTOM_IDS {
            assert!(registry::command(*id).unwrap().has(*dir), "{:04X}", id.0);
        }
    }

    /// Guide 6.3: every `GET_STATUS` selector rebuilt from the structures, as
    /// the table's last column says, equals the fake's page-0 answer.
    #[test]
    fn wp_pgfake2_get_status_selectors_from_the_structures() {
        let mut d = FakeDevice::demo();
        // A meter block with every byte distinct: peaks, loads, flags, count.
        let n = d.shared.lock().status_len();
        let meters: Vec<u8> = (1..=n as u8).collect();
        d.remote().set_status(meters.clone());
        let levels = get(&d, cmds::MEAS_LEVELS, 0, 256).unwrap();
        assert_eq!(levels, meters, "MEAS.LEVELS is the selector-9 packet");
        let sys = get(&d, cmds::SYS_SYSTEM_STATUS, 0, 16).unwrap();
        let usb = get(&d, cmds::SYS_USB_AUDIO_STATUS, 0, 16).unwrap();
        let pipe = get(&d, cmds::SYS_PIPELINE_STATS, 0, 24).unwrap();
        let pdm = get(&d, cmds::SYS_PDM_STATS, 0, 16).unwrap();
        let spdif = get(&d, cmds::SYS_SPDIF_TX_STATS, 0, 24).unwrap();
        let input = get(&d, cmds::INPUT_STATUS, 0, 12).unwrap();
        assert_eq!(
            (sys.len(), usb.len(), pipe.len(), pdm.len(), spdif.len(), input.len()),
            (16, 16, 24, 16, 24, 12)
        );
        // Selector 24/25: a release build refuses the DIAG read; the row reads 0.
        assert_eq!(get(&d, cmds::DIAG_LOOPBACK_STATS, 0, 8), Err(VC_ERR_UNKNOWN));
        let u16_at = |b: &[u8], at: usize| u32::from(u16::from_le_bytes([b[at], b[at + 1]]));
        // The guide's table, transcribed: (selector, rebuilt value).
        let rebuilt = |sel: u16| -> u32 {
            match sel {
                0 => u16_at(&levels, 0) | (u16_at(&levels, 2) << 16),
                1 => u16_at(&levels, 4) | (u16_at(&levels, 6) << 16),
                2 => u16_at(&levels, 8) | (u32::from(sys[12]) << 16) | (u32::from(sys[13]) << 24),
                3..=6 => u32_at(&pdm, usize::from(sel - 3) * 4),
                7 => 0,
                8 => u32_at(&spdif, 0),
                10 => u32_at(&usb, 0),
                11 => u32::from(usb[12]),
                12 => u32::from(usb[13]),
                13 => u32_at(&sys, 0),
                14 => u16_at(&sys, 8),
                15 => u32_at(&sys, 4),
                16 => i16::from_le_bytes([sys[10], sys[11]]) as i32 as u32,
                17..=21 => u32_at(&spdif, 4 + usize::from(sel - 17) * 4),
                22 => u32_at(&usb, 4),
                23 => u32::from(input[1]),
                24 | 25 => 0,
                26..=29 => u32_at(&pipe, usize::from(sel - 26) * 4),
                30 => u32_at(&usb, 8),
                31 => u32_at(&pipe, 16),
                32 => u32_at(&input, 4),
                33 => u32_at(&input, 8),
                _ => 0,
            }
        };
        for sel in (0..=40u16).filter(|s| *s != 9) {
            let old = d.control_in(op::REQ_GET_STATUS, sel, 4).unwrap();
            let want = match sel {
                // The fake answers 0..2 with its quiet counters (0), the
                // firmware with peaks; compare the guide's mapping to the meters.
                0..=2 => continue,
                _ => u32_at(&old, 0),
            };
            assert_eq!(rebuilt(sel), want, "selector {sel}");
        }
        // Selectors 0 to 2 from the meter block itself (guide 6.3 rows 0-2).
        assert_eq!(rebuilt(0), u32_at(&meters, 0));
        assert_eq!(rebuilt(1), u32_at(&meters, 4));
        let nn = n - 7;
        let two = u32::from(u16::from_le_bytes([meters[8], meters[9]]))
            | (u32::from(meters[nn]) << 16)
            | (u32::from(meters[nn + 1]) << 24);
        assert_eq!(rebuilt(2), two);
        // A few known values, so the comparison is not all zeros.
        assert_eq!(u32_at(&sys, 4), 48_000, "sample rate at offset 4, not 0");
        assert_eq!(u32_at(&sys, 0), 307_200_000);
        assert_eq!(u32_at(&pipe, 8), u32::MAX, "lowest fill before the first write");
        assert_eq!(input[1], meters[n - 1], "active input count");
        // Guide 6.3: "a 4-byte read of SYS.SYSTEM_STATUS returns clk_sys".
        assert_eq!(get(&d, cmds::SYS_SYSTEM_STATUS, 0, 4).unwrap(), sys[..4]);
    }

    /// Guide 6.5 `SET_EQ_PARAM`/`GET_EQ_PARAM`: the conversion of an old packet
    /// and the old 4-byte reads, both ways, against the fake's page-0 answers.
    #[test]
    fn wp_pgfake2_filter_band_is_the_reshaped_eq_param() {
        let mut d = FakeDevice::demo();
        let (ch, band) = (2u8, 3u8);
        // Old 18-byte packet: ch, band, type 3, bypass 1, 1000 Hz, Q 0.7, -3 dB, Qp 0x0200.
        let mut p = vec![ch, band, 3, 1];
        p.extend(1000f32.to_le_bytes());
        p.extend(0.7f32.to_le_bytes());
        p.extend((-3f32).to_le_bytes());
        p.extend(0x0200u16.to_le_bytes());
        d.control_out(op::REQ_SET_EQ_PARAM, 0, &p).unwrap();
        // "send wValue = (p[0] << 8) | p[1] and p[2], p[3], qp lo, qp hi, p[4..15]".
        let value = (u16::from(p[0]) << 8) | u16::from(p[1]);
        let mut converted = vec![p[2], p[3], p[16], p[17]];
        converted.extend_from_slice(&p[4..16]);
        assert_eq!(get(&d, cmds::FILTER_BAND, value, 16).unwrap(), converted);

        // A paged write reads back through the old 4-byte reads (guide's table).
        let mut new = vec![5, 0, 0x34, 0x12];
        new.extend(250f32.to_le_bytes());
        new.extend(1.5f32.to_le_bytes());
        new.extend(6f32.to_le_bytes());
        set(&d, cmds::FILTER_BAND, value, &new).unwrap();
        let old = |d: &mut FakeDevice, param: u16| {
            let v = (u16::from(ch) << 8) | (u16::from(band) << 3) | param;
            d.control_in(op::REQ_GET_EQ_PARAM, v, 4).unwrap()
        };
        assert_eq!(old(&mut d, 0), [5, 0, 0, 0], "type, zero-extended");
        assert_eq!(old(&mut d, 1), new[4..8], "freq");
        assert_eq!(old(&mut d, 2), new[8..12], "Q");
        assert_eq!(old(&mut d, 3), new[12..16], "gain");
        assert_eq!(old(&mut d, 4), [0, 0, 0, 0], "bypass");
        assert_eq!(old(&mut d, 5), [0x34, 0x12, 0, 0], "Qp, zero-extended");
        // Bypass: "exactly 1 bypasses. Reads back as 0 or 1".
        new[1] = 2;
        set(&d, cmds::FILTER_BAND, value, &new).unwrap();
        assert_eq!(get(&d, cmds::FILTER_BAND, value, 16).unwrap()[1], 0);
        new[1] = 1;
        set(&d, cmds::FILTER_BAND, value, &new).unwrap();
        assert_eq!(get(&d, cmds::FILTER_BAND, value, 16).unwrap()[1], 1);

        // The gap 10-19 and a crossover band on an input: VC_ERR_ADDRESS both ways.
        assert_eq!(get(&d, cmds::FILTER_BAND, 0x020C, 16), Err(VC_ERR_ADDRESS));
        assert_eq!(set(&d, cmds::FILTER_BAND, 0x020C, &new), Err(VC_ERR_ADDRESS));
        assert_eq!(get(&d, cmds::FILTER_BAND, 0x0014, 16), Err(VC_ERR_ADDRESS));
        // A crossover band on an output: its Qp is ignored and reads 0.
        let out = u16::from(d.shared.lock().num_inputs()) << 8 | 20;
        set(&d, cmds::FILTER_BAND, out, &new).unwrap();
        assert_eq!(get(&d, cmds::FILTER_BAND, out, 16).unwrap()[2..4], [0, 0]);
        // A non-finite float is VC_ERR_VALUE and changes nothing.
        let before = d.remote().image();
        let mut nan = new.clone();
        nan[8..12].copy_from_slice(&f32::NAN.to_le_bytes());
        assert_eq!(set(&d, cmds::FILTER_BAND, value, &nan), Err(VC_ERR_VALUE));
        nan[8..12].copy_from_slice(&f32::INFINITY.to_le_bytes());
        assert_eq!(set(&d, cmds::FILTER_BAND, value, &nan), Err(VC_ERR_VALUE));
        assert_eq!(d.remote().image(), before);
        // The 1.1.6 lengths are refused: one layout, 16 bytes.
        assert_eq!(set(&d, cmds::FILTER_BAND, value, &p), Err(VC_ERR_LENGTH));
        assert_eq!(set(&d, cmds::FILTER_BAND, value, &p[..15]), Err(VC_ERR_LENGTH));
    }

    /// Guide 6.5 `SET_MATRIX_ROUTE`/`GET_MATRIX_ROUTE`: the address moved into
    /// `wValue`, the GET no longer echoes it.
    #[test]
    fn wp_pgfake2_matrix_route_conversions() {
        let mut d = FakeDevice::demo();
        // Old SET packet: input 1, output 4, enabled, phase, -2.5 dB.
        let mut p = vec![1u8, 4, 1, 1];
        p.extend((-2.5f32).to_le_bytes());
        d.control_out(op::REQ_SET_MATRIX_ROUTE, 0, &p).unwrap();
        let value = (u16::from(p[0]) << 8) | u16::from(p[1]);
        let r = get(&d, cmds::ROUTE_MATRIX_ROUTE, value, 8).unwrap();
        // "send p[2], p[3], 00, 00, p[4..7]"
        let mut converted = vec![p[2], p[3], 0, 0];
        converted.extend_from_slice(&p[4..8]);
        assert_eq!(r, converted);
        // A paged write; "rebuild the old GET reply ... input, output, r[0], r[1], r[4..7]".
        let mut new = vec![1u8, 0, 0, 0];
        new.extend(3f32.to_le_bytes());
        set(&d, cmds::ROUTE_MATRIX_ROUTE, value, &new).unwrap();
        let r = get(&d, cmds::ROUTE_MATRIX_ROUTE, value, 8).unwrap();
        let mut rebuilt = vec![1u8, 4, r[0], r[1]];
        rebuilt.extend_from_slice(&r[4..8]);
        assert_eq!(d.control_in(op::REQ_GET_MATRIX_ROUTE, value, 8).unwrap(), rebuilt);
        // Out of range, NaN, the old length.
        assert_eq!(get(&d, cmds::ROUTE_MATRIX_ROUTE, 0x0900, 8), Err(VC_ERR_ADDRESS));
        assert_eq!(get(&d, cmds::ROUTE_MATRIX_ROUTE, 0x0009, 8), Err(VC_ERR_ADDRESS));
        let mut nan = new.clone();
        nan[4..8].copy_from_slice(&f32::NAN.to_le_bytes());
        assert_eq!(set(&d, cmds::ROUTE_MATRIX_ROUTE, value, &nan), Err(VC_ERR_VALUE));
        assert_eq!(set(&d, cmds::ROUTE_MATRIX_ROUTE, value, &new[..7]), Err(VC_ERR_LENGTH));
    }

    /// Guide 6.5 `CLEAR_CLIPS`: a read that does not clear, then the action; the
    /// flags are also `MEAS.LEVELS` bytes `2N + 2` to `2N + 5`.
    #[test]
    fn wp_pgfake2_clear_clips_is_a_read_and_an_action() {
        let d = FakeDevice::demo();
        let n = d.shared.lock().status_len();
        let mut meters = vec![0u8; n];
        let at = n - 5;
        meters[at..at + 4].copy_from_slice(&0x0001_0005u32.to_le_bytes());
        d.remote().set_status(meters.clone());
        let flags = get(&d, cmds::MEAS_CLIP_FLAGS, 0, 4).unwrap();
        assert_eq!(flags, 0x0001_0005u32.to_le_bytes());
        assert_eq!(get(&d, cmds::MEAS_CLIP_FLAGS, 0, 4).unwrap(), flags, "reading does not clear");
        assert_eq!(get(&d, cmds::MEAS_LEVELS, 0, 64).unwrap()[at..at + 4], flags);
        // The old write-as-read answered the same flags and cleared them.
        let mut twin = FakeDevice::demo();
        twin.remote().set_status(meters);
        assert_eq!(twin.control_in(op::REQ_CLEAR_CLIPS, 0, 4).unwrap(), flags);
        set(&d, cmds::MEAS_CLIP_CLEAR, 0, &[]).unwrap();
        assert_eq!(get(&d, cmds::MEAS_CLIP_FLAGS, 0, 4).unwrap(), [0; 4]);
        // An action takes no payload and addresses nothing.
        assert_eq!(set(&d, cmds::MEAS_CLIP_CLEAR, 0, &[0]), Err(VC_ERR_LENGTH));
        assert_eq!(set(&d, cmds::MEAS_CLIP_CLEAR, 1, &[]), Err(VC_ERR_ADDRESS));
    }

    /// Guide 6.5 `GET_INPUT_RATE`, `PRESET_GET_STARTUP`, `GET_CS_DISPLAY_CFG`:
    /// the old combined replies are the new parts, joined.
    #[test]
    fn wp_pgfake2_combined_replies_split() {
        let mut d = FakeDevice::demo();
        // GET_INPUT_RATE: SYS.SYSTEM_STATUS bytes 4-7, then INPUT.I2S_RX_RATE.
        d.control_out(op::REQ_SET_INPUT_RATE, 0, &96_000u32.to_le_bytes()).unwrap();
        let old = d.control_in(op::REQ_GET_INPUT_RATE, 0, 8).unwrap();
        let mut joined = get(&d, cmds::SYS_SYSTEM_STATUS, 0, 16).unwrap()[4..8].to_vec();
        joined.extend(get(&d, cmds::INPUT_I2S_RX_RATE, 0, 4).unwrap());
        assert_eq!(old, joined);
        assert_eq!(u32_at(&joined, 4), 96_000);
        // PRESET_GET_STARTUP: STORE.STARTUP {mode, default}, then STORE.DIRECTORY byte 4.
        d.control_out(op::REQ_PRESET_SET_STARTUP, 0, &[1, 3]).unwrap();
        d.control_in(op::REQ_PRESET_SAVE, 2, 1).unwrap();
        let old = d.control_in(op::REQ_PRESET_GET_STARTUP, 0, 3).unwrap();
        let mut joined = get(&d, cmds::STORE_STARTUP, 0, 2).unwrap();
        assert_eq!(joined, [1, 3], "exactly what its SET takes");
        joined.push(get(&d, cmds::STORE_DIRECTORY, 0, 7).unwrap()[4]);
        assert_eq!(old, joined);
        assert_eq!(joined[2], 2, "the last active slot");
        // GET_CS_DISPLAY_CFG: CS.DISPLAY_CAPS (16 pages, 9 models, 2 reserved), CS.DISPLAY_CONFIG.
        let old = d.control_in(op::REQ_GET_CS_DISPLAY_CFG, 0, 16).unwrap();
        let caps = get(&d, cmds::CS_DISPLAY_CAPS, 0, 4).unwrap();
        assert_eq!(caps[..2], [16, 9]);
        let mut joined = caps;
        joined.extend(get(&d, cmds::CS_DISPLAY_CONFIG, 0, 12).unwrap());
        assert_eq!(old, joined);
        // Without control surfaces neither command exists.
        let bare = FakeDevice::demo_without_surfaces();
        assert_eq!(get(&bare, cmds::CS_DISPLAY_CAPS, 0, 4), Err(VC_ERR_UNKNOWN));
        assert_eq!(get(&bare, cmds::CS_DISPLAY_CONFIG, 0, 12), Err(VC_ERR_UNKNOWN));
        // GET rules: wLength 0, a stray wValue, the wrong direction.
        assert_eq!(get(&d, cmds::STORE_STARTUP, 0, 0), Err(VC_ERR_LENGTH));
        assert_eq!(get(&d, cmds::STORE_STARTUP, 1, 2), Err(VC_ERR_ADDRESS));
        assert_eq!(d.shared.lock().custom_paged_get(cmds::MEAS_CLIP_CLEAR, 0, 4), None);
        assert_eq!(
            State::custom_common(cmds::MEAS_CLIP_CLEAR, PagedDir::Get, 0, 4),
            Err(VC_ERR_DIRECTION)
        );
    }

    /// Guide 6.5 and 9.2 `SET_ALL_PARAMS`: exactly 6136 bytes, a bad header is
    /// `VC_ERR_VALUE` instead of being dropped after the ACK.
    #[test]
    fn wp_pgfake2_all_params_length_is_strict() {
        let mut d = FakeDevice::demo();
        let img = d.control_in(op::REQ_GET_ALL_PARAMS, 0, generated::BULK_SIZE as u16).unwrap();
        assert_eq!(img.len(), 6136);
        set(&d, cmds::STORE_ALL_PARAMS, 0, &img).unwrap();
        for len in [0usize, 16, 64, 6135] {
            assert_eq!(set(&d, cmds::STORE_ALL_PARAMS, 0, &img[..len]), Err(VC_ERR_LENGTH), "{len}");
        }
        let mut longer = img.clone();
        longer.push(0);
        assert_eq!(set(&d, cmds::STORE_ALL_PARAMS, 0, &longer), Err(VC_ERR_LENGTH));
        let mut bad = img.clone();
        bad[0] ^= 0xFF;
        assert_eq!(set(&d, cmds::STORE_ALL_PARAMS, 0, &bad), Err(VC_ERR_VALUE));
        assert_eq!(set(&d, cmds::STORE_ALL_PARAMS, 1, &img), Err(VC_ERR_ADDRESS));
    }

    /// The retired rows: `SAVE_PARAMS` (the active slot, then a save to it),
    /// `SET_PREAMP` (the platform, then every input), the USB error stats
    /// (nothing reaches the wire).
    #[test]
    fn wp_pgfake2_retired_rows() {
        let mut d = FakeDevice::demo();
        // SAVE_PARAMS.
        d.control_in(op::REQ_PRESET_SAVE, 4, 1).unwrap();
        d.remote().poke(64, &[0x5A], nfy::PARAM_SRC_HOST_SET as u8);
        let slot = get(&d, cmds::STORE_PRESET_ACTIVE, 0, 1).unwrap()[0];
        assert_eq!(slot, 4);
        set(&d, cmds::STORE_PRESET_SAVE, u16::from(slot), &[]).unwrap();
        {
            let s = d.shared.lock();
            assert_eq!(s.presets.slots[4].as_deref(), Some(&s.image[..]));
        }
        assert_eq!(set(&d, cmds::STORE_PRESET_SAVE, 0x00FF, &[]), Err(VC_ERR_ADDRESS));
        // SET_PREAMP: SYS.PLATFORM is the 1.1.6 layout; then ROUTE.PREAMP per input.
        let plat = get(&d, cmds::SYS_PLATFORM, 0, 7).unwrap();
        assert_eq!(plat, d.control_in_upto(op::REQ_GET_PLATFORM, 0, 7).unwrap());
        let inputs = d.shared.lock().num_inputs();
        for i in 0..u16::from(inputs) {
            set(&d, cmds::ROUTE_PREAMP, i, &(-4f32).to_le_bytes()).unwrap();
        }
        for i in 0..u16::from(inputs) {
            assert_eq!(
                d.control_in(op::REQ_GET_PREAMP_CH, i, 4).unwrap(),
                (-4f32).to_le_bytes()
            );
        }
        assert_eq!(d.control_in(op::REQ_GET_PREAMP, 0, 4).unwrap(), (-4f32).to_le_bytes());
        assert_eq!(
            set(&d, cmds::ROUTE_PREAMP, u16::from(inputs), &[0; 4]),
            Err(VC_ERR_ADDRESS)
        );
        assert_eq!(
            set(&d, cmds::ROUTE_PREAMP, 0, &f32::NAN.to_le_bytes()),
            Err(VC_ERR_VALUE)
        );
        // The USB error stats: no paged command at all.
        for code in [op::REQ_GET_USB_ERROR_STATS, op::REQ_RESET_USB_ERROR_STATS] {
            let row = LEGACY_OPS.iter().find(|o| o.code == code).unwrap();
            let Some(Adapter::Custom(c)) = row.r#in else {
                panic!("{} is a custom IN row", row.name)
            };
            assert!(c.uses.is_empty() && c.retired, "{}", row.name);
        }
    }

    /// Anything else is not ours: the registry inverse answers it.
    #[test]
    fn wp_pgfake2_other_commands_are_not_answered_here() {
        let d = FakeDevice::demo();
        let mut s = d.shared.lock();
        assert_eq!(s.custom_paged_get(cmds::ROUTE_OUTPUT_GAIN, 1, 4), None);
        assert_eq!(s.custom_paged_set(cmds::ROUTE_OUTPUT_GAIN, 1, &[0; 4]), None);
        assert_eq!(s.custom_paged_set(cmds::SYS_PLATFORM, 0, &[0; 7]), None);
    }

    proptest::proptest! {
        /// No request, however odd, panics; a refusal changes nothing.
        #[test]
        fn proptest_wp_pgfake2_custom_never_panics(
            i in 0usize..CUSTOM_IDS.len(),
            value in proptest::prelude::any::<u16>(),
            want in 0usize..300,
            data in proptest::collection::vec(proptest::prelude::any::<u8>(), 0..80),
        ) {
            let d = FakeDevice::demo();
            let (id, dir) = CUSTOM_IDS[i];
            let mut s = d.shared.lock();
            match dir {
                PagedDir::Get => {
                    let _ = s.custom_paged_get(id, value, want);
                }
                PagedDir::Set => {
                    let before = s.image.clone();
                    if let Some(Err(_)) = s.custom_paged_set(id, value, &data) {
                        proptest::prop_assert_eq!(&s.image, &before);
                    }
                }
            }
        }
    }
}
