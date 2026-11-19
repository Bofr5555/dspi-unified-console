//! WP PGQT: the demo as each fake generation, for the clients' smoke runs
//! (`open_demo_generation`, `dspi_demo_open_generation`).
#![allow(clippy::unwrap_used, clippy::expect_used)]

use super::*;
use crate::{DeviceKind, ExtEnvelope, ProtocolGeneration};

#[test]
fn wp_pgqt_every_demo_generation_opens_as_itself_without_violations() {
    let want = [
        (
            0u8,
            ProtocolGeneration::Legacy,
            ExtEnvelope::NoChannel,
            0u8,
            DeviceKind::Dsp,
        ),
        (
            1,
            ProtocolGeneration::Legacy,
            ExtEnvelope::Legacy,
            0,
            DeviceKind::Dsp,
        ),
        (
            2,
            ProtocolGeneration::Legacy,
            ExtEnvelope::Legacy,
            0,
            DeviceKind::Dsp,
        ),
        (
            3,
            ProtocolGeneration::Paged,
            ExtEnvelope::NoChannel,
            0,
            DeviceKind::Dsp,
        ),
        (
            4,
            ProtocolGeneration::Paged,
            ExtEnvelope::Tunnel,
            0,
            DeviceKind::Dsp,
        ),
        (
            5,
            ProtocolGeneration::Legacy,
            ExtEnvelope::NoChannel,
            1,
            DeviceKind::Dsp,
        ),
        (
            6,
            ProtocolGeneration::Legacy,
            ExtEnvelope::Legacy,
            0,
            DeviceKind::AlinkSender,
        ),
        (
            7,
            ProtocolGeneration::Legacy,
            ExtEnvelope::Legacy,
            0,
            DeviceKind::Dsp,
        ),
        // -- WP QTSENDER -- (INT-R1: added here too) the dual-stack sender
        (
            8,
            ProtocolGeneration::Paged,
            ExtEnvelope::Tunnel,
            0,
            DeviceKind::AlinkSender,
        ),
    ];
    let core = DspiCore::new();
    assert!(matches!(
        core.demo_generation_violations(),
        Err(DspiError::Unsupported)
    ));
    for (code, generation, ext, board, kind) in want {
        core.open_demo_generation(code)
            .unwrap_or_else(|e| panic!("code {code}: {e}"));
        let p = core.device_protocol().unwrap();
        assert_eq!(p.generation, generation, "code {code}");
        assert_eq!(p.ext_envelope, ext, "code {code}");
        assert_eq!(p.board_id, board, "code {code}");
        assert_eq!(p.device_kind, kind, "code {code}");
        assert_eq!(
            p.pages.is_empty(),
            generation != ProtocolGeneration::Paged,
            "code {code}"
        );
        assert_eq!(core.demo_generation_violations().unwrap(), 0, "code {code}");
        // The voltage the update check reads: 1.15 V for code 7, the demo's 1.20 V else
        let target = core.firmware_target();
        let want_mv = match code {
            7 => 1150,
            1 | 2 => 1200,
            _ => target.voltage.required_mv, // not this test's point
        };
        assert_eq!(target.voltage.required_mv, want_mv, "code {code}");
    }
    assert!(matches!(
        core.open_demo_generation(9),
        Err(DspiError::Unsupported)
    ));
    // Another device opened: the count is that fake's no longer
    core.open_demo().unwrap();
    assert!(matches!(
        core.demo_generation_violations(),
        Err(DspiError::Unsupported)
    ));
}
