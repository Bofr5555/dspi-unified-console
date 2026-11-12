//! The fast write path, for GUIs and bindings.
//!
//! [`Session::write`] is the CLI's path: it reads the value before and after,
//! confirms the change and journals it for undo, which costs three round trips
//! per change. A GUI writes at slider rate and keeps its own mirror, so it
//! needs the opposite: encode through the registry, send, done. The mirror is
//! then corrected by the device's own `PARAM_CHANGED` echo.
//!
//! Owned by WP S1 (docs/core-merge/PLAN.md).
#![warn(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

use dspi_proto::generated::opcodes as op;
use dspi_proto::registry::{self, ParamDesc, WValue, by_path};
use dspi_proto::value::{EqParamPacket, Unit, Value, ValueError};
use dspi_proto::{Dir, FilterType, Platform};

use crate::{Session, WriteError};

impl Session {
    /// Send one parameter change without reading anything back.
    ///
    /// Validates the value and the indices exactly as [`Session::write`] does
    /// (the same errors), encodes `wValue` and the payload through the
    /// registry, and sends it. Write-as-read opcodes return their status byte
    /// through [`Session::last_write_status`]. No undo journal entry, no
    /// Core 1 interlock prompt (the caller asks first), no confirm read.
    ///
    /// Cost on the wire:
    /// - a scalar: one transfer (an OUT, or an IN for a write-as-read);
    /// - an EQ field (`eq.type`, `eq.freq`, `eq.q`, `eq.gain`, `eq.fp`, `eq.qp`): one band read
    ///   ([`Session::read_band`], five IN transfers, six for a Linkwitz
    ///   Transform) and one band write, because the firmware takes a band only
    ///   whole. A caller that holds the band already should send the packet
    ///   itself (`REQ_SET_EQ_PARAM`) or use [`Session::write_band`].
    ///
    /// With [`Session::dry_run`] set, nothing reaches the wire; validation
    /// still runs.
    ///
    /// # Errors
    ///
    /// The same as [`Session::write`] for the same input, except
    /// [`WriteError::Core1Conflict`], which this path never returns:
    /// - [`WriteError::UnknownParam`], [`WriteError::ReadOnly`];
    /// - [`WriteError::Unavailable`] (platform or feature missing);
    /// - [`WriteError::WrongArity`], [`WriteError::BadTarget`];
    /// - [`WriteError::Value`] (out of range or wrong type),
    ///   [`WriteError::PlatformRange`];
    /// - [`WriteError::Transport`] when a transfer fails, including a reply
    ///   shorter than asked for.
    pub fn send(&mut self, path: &str, indices: &[u8], value: Value) -> Result<(), WriteError> {
        let d = by_path(path).ok_or_else(|| WriteError::UnknownParam(path.into()))?;
        let set = d
            .set
            .ok_or_else(|| WriteError::ReadOnly { path: path.into() })?;

        // Belongs to this write alone, as in `write`: a stale code from the
        // previous one would be read as this one's refusal.
        self.last_status = None;

        self.check_available(d)?;
        self.check_indices(d, indices)?;
        self.check_platform_range(d, &value)?;
        let value = d.kind.validate(&value)?;

        if self.dry_run {
            return Ok(());
        }

        if let WValue::EqScalar(_) = d.wvalue {
            // `check_indices` has established the arity (2) of a `ChannelBand`
            // target; this stays total anyway.
            let (Some(&channel), Some(&band)) = (indices.first(), indices.get(1)) else {
                return Err(WriteError::WrongArity {
                    path: d.path.into(),
                    want: 2,
                    got: indices.len(),
                });
            };
            let mut packet = self.read_band(channel, band)?;
            apply_eq_field(&mut packet, d, &value)?;
            self.transport_mut()
                .control_out(op::REQ_SET_EQ_PARAM, 0, &packet.encode())?;
            return Ok(());
        }

        let wvalue = self.build_wvalue(d, indices, &value)?;
        match d.dir {
            Dir::Out => {
                let payload = d.repr().encode(&value)?;
                self.transport_mut().control_out(set, wvalue, &payload)?;
            }
            // Mutating commands on the IN path: the parameters ride in wValue
            // and the answer is a status byte (see `Session::write`).
            Dir::WriteAsRead | Dir::In => {
                let reply = self.transport_mut().control_in(set, wvalue, 1)?;
                self.last_status = reply.first().copied();
            }
        }
        Ok(())
    }

    /// The platform limits [`Session::write`] checks beyond the registry's
    /// range. Shared by `send` and `write`; run before the registry's own
    /// check, so a delay out of range names the line's range, not the
    /// schema's 85 ms.
    pub(crate) fn check_platform_range(
        &self,
        d: &ParamDesc,
        value: &Value,
    ) -> Result<(), WriteError> {
        // -- WP DL3 -- NaN and infinity pass every range test (`f < min` is
        // false for NaN), so they are refused here, for every parameter,
        // before any range is looked at.
        if let Value::Float(f) = value {
            check_finite(*f)?;
        }
        // A value of the wrong kind is the registry's to refuse.
        if !matches!(value, Value::Float(_) | Value::Int(_)) {
            return Ok(());
        }
        // -- WP CG37 -- the delay line holds 21 ms on an RP2040 and 42 ms on
        // an RP2350 (`Platform::max_delay_ms`, the core's source too).
        if matches!(d.path, "ch.delay" | "out.delay")
            && let Some(v) = value.as_f32()
        {
            let max = self.capabilities().platform.max_delay_ms();
            if v > max {
                return Err(WriteError::PlatformRange {
                    path: d.path.into(),
                    max: format!("{max} ms"),
                });
            }
            if v < 0.0 {
                return Err(WriteError::Value(ValueError::OutOfRange {
                    value: f64::from(v),
                    min: 0.0,
                    max: f64::from(max),
                    unit: Unit::Ms,
                }));
            }
        }
        // The registry carries the RP2350's +24 dB; an RP2040 applies its
        // preamp as a Q28 multiplier, which overflows above about +18 dB.
        if matches!(d.path, "pre" | "pre.all")
            && self.capabilities().platform == Platform::Rp2040
            && value.as_f32().is_some_and(|v| v > 18.0)
        {
            return Err(WriteError::PlatformRange {
                path: d.path.into(),
                max: "+18 dB".into(),
            });
        }
        Ok(())
    }

    // -- WP P0U fixup --
    /// The range a number written to `d` must fall in on this device: the
    /// registry's, narrowed by the platform's limits as
    /// [`check_platform_range`](Self::check_platform_range) narrows it. `None`
    /// for a parameter that is not a number.
    pub fn write_range(&self, d: &ParamDesc) -> Option<(f64, f64)> {
        let (min, max) = match d.kind {
            registry::Kind::Float { min, max, .. } => (f64::from(min), f64::from(max)),
            registry::Kind::Int { min, max, .. } => (min as f64, max as f64),
            _ => return None,
        };
        let platform = self.capabilities().platform;
        let max = match d.path {
            "ch.delay" | "out.delay" => max.min(f64::from(platform.max_delay_ms())),
            "pre" | "pre.all" if platform == Platform::Rp2040 => max.min(18.0),
            _ => max,
        };
        Some((min, max))
    }
}

// -- WP DL3 --
/// Refuse NaN and infinity, which no parameter holds and every range test
/// lets through.
pub(crate) fn check_finite(f: f32) -> Result<(), WriteError> {
    if f.is_finite() {
        return Ok(());
    }
    Err(WriteError::Value(ValueError::WrongType {
        expected: "a finite number",
        got: if f.is_nan() {
            "NaN"
        } else if f > 0.0 {
            "infinity"
        } else {
            "-infinity"
        },
    }))
}

/// The Linkwitz Transform's target Q when the firmware stores 0 (`decode_qp`).
pub(crate) const DEFAULT_QP: f32 = 0.707;

/// Put one validated EQ field into a band read from the device. Shared by
/// `send` and the EQ path of `Session::write`.
pub(crate) fn apply_eq_field(
    packet: &mut EqParamPacket,
    d: &ParamDesc,
    value: &Value,
) -> Result<(), WriteError> {
    // -- WP CG34 -- `eq.gain` or `eq.fp` and `eq.qp` by the band's type.
    registry::eq_field_fits(d.path, packet.band, packet.filter_type).map_err(|why| {
        WriteError::Unavailable {
            path: d.path.into(),
            why,
        }
    })?;
    match d.path {
        "eq.type" => {
            let to = FilterType::from_raw(value.as_u8().unwrap_or(0));
            // -- WP CG34 -- the Console's retype: into or out of a Linkwitz
            // Transform, the gain field changes meaning (registry::retyped).
            let qp = packet.qp.unwrap_or(DEFAULT_QP);
            let [freq, q, gain, qp] = registry::retyped(
                packet.filter_type,
                to,
                [packet.freq, packet.q, packet.gain_db, qp],
            );
            packet.filter_type = to;
            packet.freq = freq;
            packet.q = q;
            packet.gain_db = gain;
            // A Linkwitz Transform needs a Qp to send; the firmware reads 0 as
            // "use the 0.707 default".
            if to.is_linkwitz() {
                packet.qp = Some(qp);
            }
        }
        "eq.freq" => packet.freq = value.as_f32().unwrap_or(packet.freq),
        "eq.q" => packet.q = value.as_f32().unwrap_or(packet.q),
        "eq.gain" | "eq.fp" => packet.gain_db = value.as_f32().unwrap_or(packet.gain_db),
        "eq.qp" => packet.qp = value.as_f32().or(packet.qp),
        other => return Err(WriteError::UnknownParam(other.into())),
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_s1 {
    use crate::probe::{Capabilities, ChannelInfo, ControlSurfaceCaps, Feature};
    use crate::{Session, WriteError};
    use dspi_proto::generated::opcodes as op;
    use dspi_proto::value::Value;
    use dspi_proto::{FilterType, Platform};
    use dspi_transport::MockTransport;
    use dspi_transport::mock::{Direction, Exchange, LogHandle, Reply};

    /// The topology `write.rs`'s tests use: 8 in, 9 out, 17 channels.
    fn caps(platform: Platform) -> Capabilities {
        Capabilities {
            serial: "TEST".into(),
            platform,
            firmware: "1.1.5".into(),
            firmware_version: dspi_proto::packets::FirmwareVersion::new(1, 1, 5, 0),
            build_info: None,
            wire_format: 32,
            num_channels: 17,
            num_inputs: 8,
            num_outputs: 9,
            max_bands: 10,
            band_storage: 12,
            channels: (0..17)
                .map(|i| ChannelInfo {
                    index: i,
                    name: format!("Ch {i}"),
                    slug: format!("ch.{i}"),
                    is_output: i >= 8,
                })
                .collect(),
            // The tube preamp and limiter are present, the bass enhancer is not.
            features: [
                ("tube_preamp", true),
                ("output_limiter", true),
                ("psychoacoustic_bass", false),
            ]
            .into_iter()
            .map(|(name, present)| Feature {
                name: name.into(),
                present,
                evidence: "test".into(),
            })
            .collect(),
            cs: Some(ControlSurfaceCaps {
                caps_version: 13,
                max_bindings: 16,
                type_count: 9,
                noun_count: 57,
                max_ir_commands: 16,
                max_groups: 8,
                max_macros: 8,
                max_macro_steps: 8,
                max_pages: 16,
                display_models: 8,
                types: Vec::new(),
            }),
            siggen: None,
            active_preset: Some(0),
        }
    }

    pub(super) fn rig_on(t: MockTransport, platform: Platform) -> (Session, LogHandle) {
        let log = t.log_handle();
        let s = Session::new(Box::new(t), caps(platform)).unwrap();
        (s, log)
    }

    fn rig(t: MockTransport) -> (Session, LogHandle) {
        rig_on(t, Platform::Rp2350)
    }

    fn log_of(log: &LogHandle) -> Vec<Exchange> {
        log.lock().unwrap().clone()
    }

    fn outs(log: &LogHandle) -> Vec<Exchange> {
        log_of(log)
            .into_iter()
            .filter(|e| e.direction == Direction::Out)
            .collect()
    }

    #[test]
    fn wp_s1_a_fast_write_sends_one_transfer_and_reads_nothing() {
        let (mut s, log) = rig(MockTransport::new());

        s.send("vol.user", &[], Value::Float(-18.0)).unwrap();

        let log = log_of(&log);
        assert_eq!(log.len(), 1, "one transfer: {log:?}");
        let w = &log[0];
        assert_eq!(w.direction, Direction::Out);
        assert_eq!(w.opcode, op::REQ_SET_USER_VOLUME);
        assert_eq!(w.payload, (-18.0f32).to_le_bytes().to_vec());
        assert!(s.journal.is_empty(), "the fast path keeps no journal");
        #[cfg(feature = "undo")]
        assert!(!s.can_undo(), "nor an undo step");
    }

    /// The packing rules are `write`'s: an indexed parameter goes out with the
    /// same opcode, wValue and payload on both paths.
    #[test]
    fn wp_s1_encodes_exactly_like_the_confirmed_write() {
        for (path, indices, value) in [
            ("tube.drive", vec![], Value::Float(6.0)),
            ("limit.threshold", vec![2u8], Value::Float(-6.0)),
            ("out.gain", vec![2], Value::Float(-4.5)),
            ("eq.bypass", vec![1, 21], Value::Bool(true)),
            ("out.enable", vec![8], Value::Bool(false)),
        ] {
            let (mut fast, fast_log) = rig(MockTransport::new());
            fast.send(path, &indices, value.clone()).unwrap();
            let (mut slow, slow_log) = rig(MockTransport::new().answering_everything(vec![0; 32]));
            slow.write(path, &indices, value).unwrap();

            let f = outs(&fast_log);
            assert_eq!(log_of(&fast_log).len(), 1, "{path}: one transfer");
            assert_eq!(f, outs(&slow_log), "{path}: same transfer on both paths");
        }
    }

    #[test]
    fn wp_s1_an_eq_field_costs_one_band_read_and_one_write() {
        // Every field reads 1000.0: type and bypass take the low byte (0, a
        // flat active band), freq and Q the float.
        let (mut s, log) =
            rig(MockTransport::new().data(op::REQ_GET_EQ_PARAM, 1000.0f32.to_le_bytes().to_vec()));

        s.send("eq.gain", &[8, 3], Value::Float(-6.0)).unwrap();

        let log = log_of(&log);
        let reads: Vec<_> = log
            .iter()
            .filter(|e| e.direction == Direction::In)
            .collect();
        let writes: Vec<_> = log
            .iter()
            .filter(|e| e.direction == Direction::Out)
            .collect();
        assert!(
            reads.iter().all(|e| e.opcode == op::REQ_GET_EQ_PARAM),
            "only the band read: {reads:?}"
        );
        assert_eq!(reads.len(), 5, "one band read of five fields, no confirm");
        assert_eq!(writes.len(), 1, "one band write");
        let w = writes[0];
        assert_eq!(w.opcode, op::REQ_SET_EQ_PARAM);
        assert_eq!(w.value, 0, "wValue carries nothing for this opcode");
        assert_eq!(w.payload.len(), 16);
        assert_eq!(&w.payload[0..2], &[8, 3], "channel and band in the payload");
        assert_eq!(&w.payload[4..8], &1000.0f32.to_le_bytes(), "freq kept");
        assert_eq!(&w.payload[8..12], &1000.0f32.to_le_bytes(), "Q kept");
        assert_eq!(&w.payload[12..16], &(-6.0f32).to_le_bytes(), "new gain");
        assert_eq!(
            log.last().map(|e| e.direction),
            Some(Direction::Out),
            "nothing is read after the write"
        );
    }

    #[test]
    fn wp_s1_switching_a_band_to_linkwitz_sends_the_default_qp() {
        let lt = FilterType::LinkwitzTransform.to_raw();
        let (mut s, log) =
            rig(MockTransport::new().data(op::REQ_GET_EQ_PARAM, 0u32.to_le_bytes().to_vec()));
        s.send("eq.type", &[0, 1], Value::Choice(lt)).unwrap();

        let w = log_of(&log)
            .into_iter()
            .find(|e| e.opcode == op::REQ_SET_EQ_PARAM)
            .unwrap();
        assert_eq!(w.payload.len(), 18, "a Linkwitz band carries its Qp");
        assert_eq!(w.payload[2], lt);
    }

    #[test]
    fn wp_s1_write_as_read_goes_in_and_stores_the_status() {
        let (mut s, log) = rig(MockTransport::new().data(op::REQ_SET_MCK_ENABLE, vec![3]));

        s.send("i2s.mck", &[], Value::Bool(true)).unwrap();

        let log = log_of(&log);
        assert_eq!(log.len(), 1, "one transfer: {log:?}");
        assert_eq!(log[0].direction, Direction::In);
        assert_eq!(log[0].opcode, op::REQ_SET_MCK_ENABLE);
        assert_eq!(log[0].value, 1, "the value rides in wValue");
        assert_eq!(s.last_write_status(), Some(3));
    }

    #[test]
    fn wp_s1_the_next_write_clears_a_stale_status() {
        let (mut s, _log) = rig(MockTransport::new().data(op::REQ_SET_MCK_ENABLE, vec![2]));
        s.send("i2s.mck", &[], Value::Bool(true)).unwrap();
        assert_eq!(s.last_write_status(), Some(2));

        s.send("vol.user", &[], Value::Float(-3.0)).unwrap();
        assert_eq!(s.last_write_status(), None);

        // A refused write clears it too: the code was not this write's.
        s.send("i2s.mck", &[], Value::Bool(true)).unwrap();
        s.send("vol.user", &[], Value::Float(99.0)).unwrap_err();
        assert_eq!(s.last_write_status(), None);
    }

    #[test]
    fn wp_s1_dry_run_sends_nothing_but_still_validates() {
        let (mut s, log) = rig(MockTransport::new());
        s.dry_run = true;

        s.send("vol.user", &[], Value::Float(-3.0)).unwrap();
        s.send("eq.gain", &[0, 0], Value::Float(1.0)).unwrap();
        assert!(log_of(&log).is_empty());
        assert!(s.send("vol.user", &[], Value::Float(99.0)).is_err());
    }

    #[test]
    fn wp_s1_a_transport_failure_is_a_typed_error() {
        let (mut s, _log) =
            rig(MockTransport::new().reply(op::REQ_SET_USER_VOLUME, Reply::Disconnect));
        let e = s.send("vol.user", &[], Value::Float(-3.0)).unwrap_err();
        assert!(matches!(e, WriteError::Transport(_)), "{e:?}");

        // An empty write-as-read status is a short read, not a panic.
        let (mut s, _log) = rig(MockTransport::new().data(op::REQ_SET_MCK_ENABLE, vec![]));
        let e = s.send("i2s.mck", &[], Value::Bool(true)).unwrap_err();
        assert!(matches!(e, WriteError::Transport(_)), "{e:?}");

        // So is a short band read on the EQ path, and nothing is written.
        let (mut s, log) = rig(MockTransport::new().data(op::REQ_GET_EQ_PARAM, vec![1, 2]));
        let e = s.send("eq.freq", &[0, 0], Value::Float(100.0)).unwrap_err();
        assert!(matches!(e, WriteError::Transport(_)), "{e:?}");
        assert!(outs(&log).is_empty());
    }

    /// Every refusal `write` makes before the wire, `send` makes the same way,
    /// and neither touches the wire for it.
    #[test]
    fn wp_s1_errors_are_the_same_as_write() {
        let cases: &[(Platform, &str, &[u8], Value)] = &[
            (Platform::Rp2350, "no.such.thing", &[], Value::Float(0.0)),
            (Platform::Rp2350, "dev.serial", &[], Value::Float(0.0)),
            (Platform::Rp2350, "vol.user", &[], Value::Float(99.0)),
            (Platform::Rp2350, "vol.user", &[], Value::Text("x".into())),
            (Platform::Rp2350, "vol.user", &[1], Value::Float(-3.0)),
            (Platform::Rp2350, "out.gain", &[], Value::Float(0.0)),
            (Platform::Rp2350, "out.gain", &[9], Value::Float(0.0)),
            (Platform::Rp2350, "eq.freq", &[17, 0], Value::Float(100.0)),
            (Platform::Rp2350, "eq.freq", &[0, 15], Value::Float(100.0)),
            (Platform::Rp2350, "eq.freq", &[0, 0], Value::Float(1.0)),
            (Platform::Rp2350, "bass.drive", &[], Value::Float(1.0)),
            (Platform::Rp2040, "ch.delay", &[0], Value::Float(60.0)),
        ];
        for (platform, path, indices, value) in cases {
            let (mut fast, fast_log) = rig_on(MockTransport::new(), *platform);
            let (mut slow, _) = rig_on(MockTransport::new(), *platform);
            let f = fast.send(path, indices, value.clone());
            let w = slow.write(path, indices, value.clone());
            let (Err(f), Err(w)) = (f, w) else {
                panic!("{path} {indices:?}: both must refuse");
            };
            assert_eq!(format!("{f:?}"), format!("{w:?}"), "{path} {indices:?}");
            assert!(
                log_of(&fast_log).is_empty(),
                "{path}: refused before the wire"
            );
        }
    }

    /// The fast path skips the Core 1 interlock: the caller asks first.
    #[test]
    fn wp_s1_the_core1_interlock_is_the_callers() {
        let (mut s, log) = rig(MockTransport::new().data(op::REQ_GET_CORE1_CONFLICT, vec![1]));
        s.send("out.enable", &[8], Value::Bool(true)).unwrap();
        let log = log_of(&log);
        assert_eq!(log.len(), 1, "no interlock query: {log:?}");
        assert_eq!(log[0].direction, Direction::Out);
    }

    /// `pre`'s range in the registry is the RP2350's; an RP2040's Q28
    /// multiplier stops at +18 dB, a platform fact checked before the wire.
    #[test]
    fn wp_b2_an_rp2040_refuses_a_preamp_above_18_db() {
        let session =
            |platform| Session::new(Box::new(MockTransport::new()), caps(platform)).unwrap();
        let mut s = session(Platform::Rp2040);
        assert!(matches!(
            s.send("pre", &[0], Value::Float(18.5)),
            Err(WriteError::PlatformRange { .. })
        ));
        assert!(matches!(
            s.send("pre.all", &[], Value::Float(24.0)),
            Err(WriteError::PlatformRange { .. })
        ));
        assert!(s.send("pre", &[0], Value::Float(18.0)).is_ok());
        let mut s = session(Platform::Rp2350);
        assert!(s.send("pre", &[0], Value::Float(24.0)).is_ok());
    }

    // -- WP CG34 --
    /// A band that reads as a Linkwitz Transform: every GET_EQ_PARAM
    /// answers 11, so the type is 11 (and the floats tiny).
    fn lt_band() -> MockTransport {
        MockTransport::new().data(op::REQ_GET_EQ_PARAM, 11u32.to_le_bytes().to_vec())
    }

    #[test]
    fn wp_cg34_fp_and_qp_go_out_in_the_whole_band() {
        let (mut s, log) = rig(lt_band());
        s.send("eq.fp", &[8, 1], Value::Float(30.0)).unwrap();
        s.send("eq.qp", &[8, 1], Value::Float(0.5)).unwrap();
        let sets: Vec<Exchange> = outs(&log)
            .into_iter()
            .filter(|e| e.opcode == op::REQ_SET_EQ_PARAM)
            .collect();
        assert_eq!(sets.len(), 2);
        // fp is gain_db (bytes 12..16); Qp the two bytes after, Q * 512.
        assert_eq!(sets[0].payload[2], 11);
        assert_eq!(sets[0].payload[12..16], 30.0f32.to_le_bytes());
        assert_eq!(sets[1].payload.len(), 18);
        assert_eq!(sets[1].payload[16..18], 256u16.to_le_bytes());
    }

    #[test]
    fn wp_cg34_the_band_type_decides_gain_or_fp() {
        let (mut s, _) = rig(lt_band());
        assert!(matches!(
            s.send("eq.gain", &[8, 1], Value::Float(3.0)),
            Err(WriteError::Unavailable { why, .. }) if why.contains("eq.fp")
        ));
        // A flat band (type 0): no fp, no Qp; its gain as before.
        let (mut s, _) = rig(MockTransport::new().data(op::REQ_GET_EQ_PARAM, vec![0; 4]));
        for (path, v) in [("eq.fp", 30.0), ("eq.qp", 0.7)] {
            assert!(matches!(
                s.send(path, &[8, 1], Value::Float(v)),
                Err(WriteError::Unavailable { .. })
            ));
        }
        assert!(s.send("eq.gain", &[8, 1], Value::Float(3.0)).is_ok());
        // Out of the firmware's range, before the wire.
        let (mut s, log) = rig(lt_band());
        assert!(matches!(
            s.send("eq.fp", &[8, 1], Value::Float(5.0)),
            Err(WriteError::Value(_))
        ));
        assert!(matches!(
            s.send("eq.qp", &[8, 1], Value::Float(25.0)),
            Err(WriteError::Value(_))
        ));
        assert!(log_of(&log).is_empty());
    }

    #[test]
    fn wp_cg34_qp_reads_back_from_param_5() {
        let (mut s, log) =
            rig(MockTransport::new().data(op::REQ_GET_EQ_PARAM, 307u32.to_le_bytes().to_vec()));
        assert_eq!(
            s.read("eq.qp", &[8, 1]).unwrap(),
            Value::Float(307.0 / 512.0)
        );
        assert_eq!(log_of(&log)[0].value & 7, 5);
        let (mut s, _) = rig(MockTransport::new().data(op::REQ_GET_EQ_PARAM, vec![0; 4]));
        assert_eq!(s.read("eq.qp", &[8, 1]).unwrap(), Value::Float(0.707));
    }
}

// -- WP CG37 --
#[cfg(test)]
mod tests_wp_cg37 {
    use super::tests_wp_s1::rig_on;
    use crate::WriteError;
    use dspi_proto::Platform;
    use dspi_proto::value::Value;
    use dspi_transport::MockTransport;

    #[test]
    fn wp_cg37_the_line_is_21_ms_on_an_rp2040_and_42_on_an_rp2350() {
        assert_eq!(Platform::Rp2040.max_delay_ms(), 21.0);
        assert_eq!(Platform::Rp2350.max_delay_ms(), 42.0);
        assert_eq!(Platform::Unknown(7).max_delay_ms(), 21.0);
    }

    /// `write`, `send` and `check` refuse a delay past the platform's line
    /// and take one at it, for both delay paths.
    #[test]
    fn wp_cg37_write_send_and_check_hold_the_line() {
        for (platform, max) in [(Platform::Rp2040, 21.0f32), (Platform::Rp2350, 42.0)] {
            for (path, ix) in [("ch.delay", 0u8), ("out.delay", 1)] {
                let (mut s, _) = rig_on(MockTransport::new(), platform);
                assert!(s.check(path, &[ix], &Value::Float(max)).is_ok(), "{path}");
                assert!(s.send(path, &[ix], Value::Float(max)).is_ok(), "{path}");
                for over in [Value::Float(max + 0.5), Value::Int(85)] {
                    let w = s.write(path, &[ix], over.clone()).unwrap_err();
                    let f = s.send(path, &[ix], over.clone()).unwrap_err();
                    assert_eq!(format!("{w:?}"), format!("{f:?}"));
                    assert!(
                        matches!(&w, WriteError::PlatformRange { max: m, .. } if *m == format!("{max} ms")),
                        "{path} {over:?}: {w:?}"
                    );
                    assert_eq!(
                        w.to_string(),
                        format!("`{path}` is limited to {max} ms on this platform")
                    );
                }
            }
        }
    }

    /// Below 0 the message names the line's range, not the schema's 85 ms.
    #[test]
    fn wp_cg37_a_negative_delay_names_the_lines_range() {
        for (platform, max) in [(Platform::Rp2040, 21), (Platform::Rp2350, 42)] {
            let (mut s, log) = rig_on(MockTransport::new(), platform);
            let e = s.write("out.delay", &[0], Value::Float(-1.0)).unwrap_err();
            assert!(matches!(e, WriteError::Value(_)), "{e:?}");
            assert_eq!(
                e.to_string(),
                format!("-1 is outside the allowed range 0 to {max} ms")
            );
            let f = s.send("ch.delay", &[0], Value::Float(-1.0)).unwrap_err();
            assert_eq!(e.to_string(), f.to_string());
            assert!(log.lock().unwrap().is_empty(), "nothing on the wire");
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests_wp_dl3 {
    use super::tests_wp_s1::rig_on;
    use crate::WriteError;
    use dspi_proto::registry::{Kind, REGISTRY};
    use dspi_proto::value::{EqParamPacket, Value};
    use dspi_proto::{FilterType, Platform};
    use dspi_transport::MockTransport;

    const NOT_FINITE: [f32; 3] = [f32::NAN, f32::INFINITY, f32::NEG_INFINITY];

    /// Every float parameter in the registry refuses NaN and infinity in the
    /// check `write`, `send` and `check` share, before its range is looked at.
    #[test]
    fn wp_dl3_every_float_parameter_refuses_a_value_that_is_not_finite() {
        let (s, _) = rig_on(MockTransport::new(), Platform::Rp2350);
        let floats: Vec<_> = REGISTRY
            .iter()
            .filter(|d| matches!(d.kind, Kind::Float { .. }))
            .collect();
        assert!(floats.len() > 10, "the registry has float parameters");
        for d in floats {
            for f in NOT_FINITE {
                let e = s.check_platform_range(d, &Value::Float(f)).unwrap_err();
                assert!(matches!(e, WriteError::Value(_)), "{} {f}: {e:?}", d.path);
            }
        }
    }

    /// `uconsole set out.delay 1 nan`, through each session path: refused in
    /// words, nothing on the wire.
    #[test]
    fn wp_dl3_a_nan_delay_is_refused_before_the_wire() {
        for platform in [Platform::Rp2040, Platform::Rp2350] {
            let (mut s, log) = rig_on(MockTransport::new(), platform);
            for (f, word) in [
                (f32::NAN, "NaN"),
                (f32::INFINITY, "infinity"),
                (f32::NEG_INFINITY, "-infinity"),
            ] {
                let want = format!("expected a finite number, got {word}");
                let w = s.write("out.delay", &[1], Value::Float(f)).unwrap_err();
                assert_eq!(w.to_string(), want);
                let n = s.send("out.delay", &[1], Value::Float(f)).unwrap_err();
                assert_eq!(n.to_string(), want);
                let c = s.check("ch.delay", &[0], &Value::Float(f)).unwrap_err();
                assert_eq!(c.to_string(), want);
                let g = s.send("vol.user", &[], Value::Float(f)).unwrap_err();
                assert_eq!(g.to_string(), want);
            }
            assert!(log.lock().unwrap().is_empty(), "nothing on the wire");
        }
    }

    /// A whole band with a field that is not finite is refused before it is sent.
    #[test]
    fn wp_dl3_a_band_with_a_nan_field_is_not_sent() {
        let (mut s, log) = rig_on(MockTransport::new(), Platform::Rp2350);
        let band = EqParamPacket {
            channel: 0,
            band: 0,
            filter_type: FilterType::Peaking,
            bypass: false,
            freq: 1000.0,
            q: 0.707,
            gain_db: 0.0,
            qp: None,
        };
        for f in NOT_FINITE {
            for p in [
                EqParamPacket { freq: f, ..band },
                EqParamPacket { q: f, ..band },
                EqParamPacket { gain_db: f, ..band },
                EqParamPacket {
                    qp: Some(f),
                    ..band
                },
            ] {
                let e = s.write_band(&p).unwrap_err();
                assert!(matches!(e, WriteError::Value(_)), "{e:?}");
            }
        }
        assert!(log.lock().unwrap().is_empty(), "nothing on the wire");
    }
}
