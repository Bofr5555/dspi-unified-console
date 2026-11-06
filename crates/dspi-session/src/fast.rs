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
use dspi_proto::registry::{ParamDesc, WValue, by_path};
use dspi_proto::value::{EqParamPacket, Value};
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
    /// - an EQ field (`eq.type`, `eq.freq`, `eq.q`, `eq.gain`): one band read
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
        let value = d.kind.validate(&value)?;
        self.check_platform_range(d, &value)?;

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

    /// The platform limit [`Session::write`] checks beyond the registry's
    /// range: the delay buffers hold 42 ms on an RP2040 against 85 ms on an
    /// RP2350 (config.h:94-99). Shared by `send` and `write`.
    pub(crate) fn check_platform_range(
        &self,
        d: &ParamDesc,
        value: &Value,
    ) -> Result<(), WriteError> {
        if matches!(d.path, "ch.delay" | "out.delay")
            && self.capabilities().platform == Platform::Rp2040
            && value.as_f32().is_some_and(|v| v > 42.0)
        {
            return Err(WriteError::PlatformRange {
                path: d.path.into(),
                max: "42 ms".into(),
            });
        }
        Ok(())
    }
}

/// Put one validated EQ field into a band read from the device. Shared by
/// `send` and the EQ path of `Session::write`.
pub(crate) fn apply_eq_field(
    packet: &mut EqParamPacket,
    d: &ParamDesc,
    value: &Value,
) -> Result<(), WriteError> {
    match d.path {
        "eq.type" => {
            packet.filter_type = FilterType::from_raw(value.as_u8().unwrap_or(0));
            // A Linkwitz Transform needs a Qp to send; the firmware reads 0 as
            // "use the 0.707 default".
            if packet.filter_type.is_linkwitz() && packet.qp.is_none() {
                packet.qp = Some(0.707);
            }
        }
        "eq.freq" => packet.freq = value.as_f32().unwrap_or(packet.freq),
        "eq.q" => packet.q = value.as_f32().unwrap_or(packet.q),
        "eq.gain" => packet.gain_db = value.as_f32().unwrap_or(packet.gain_db),
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

    fn rig_on(t: MockTransport, platform: Platform) -> (Session, LogHandle) {
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
}
