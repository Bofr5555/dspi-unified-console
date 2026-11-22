//! The installation model (WP SW2). -- WP INST0 -- The types and the
//! validator live in [`dspi_session::system_model`], shared with the clients;
//! they are re-exported here so every `system::model::` path stays. What is
//! the hub's policy alone stays here: the direct command a scene op stands for.

pub use dspi_session::system_model::*;

use dspi_proto::ext::alink::ApplyOp;

// -- fixup sw2x (roles) --
/// The direct command a scene op stands for, as `(bRequest, dir)`: a caller may
/// recall the op only if the hub's policy lets its role send that command.
/// Mute and unmute are `SET_OUTPUT_MUTE`, a preset load `PRESET_LOAD`, the
/// master volume and output delay their setters; a click and a sync pulse
/// have no direct command but the alink's extension write (admin).
pub fn op_authority(op: ApplyOp) -> (u8, dspi_link::Dir) {
    use dspi_link::Dir;
    use dspi_proto::generated::opcodes as opc;
    match op {
        ApplyOp::Mute | ApplyOp::Unmute => (opc::REQ_SET_OUTPUT_MUTE, Dir::Set),
        ApplyOp::PresetLoad => (opc::REQ_PRESET_LOAD, Dir::Get),
        ApplyOp::MasterVolume => (opc::REQ_SET_MASTER_VOLUME, Dir::Set),
        ApplyOp::OutDelay => (opc::REQ_SET_OUTPUT_DELAY, Dir::Set),
        _ => (opc::REQ_EXTENSION, Dir::Set),
    }
}

/// The direct command the system mute stands for: muting every unit's outputs,
/// `SET_OUTPUT_MUTE`.
pub fn system_mute_authority() -> (u8, dspi_link::Dir) {
    op_authority(ApplyOp::Mute)
}

#[cfg(test)]
mod tests_wp_sw2x {
    use super::*;
    use crate::policy::Policy;
    use crate::session::Role;

    /// The coordinator's role rule, on the hub's own policy table: the ops a
    /// console's control role sends directly are control; a click and a sync
    /// pulse have only the alink's extension write, which is admin.
    #[test]
    fn wp_sw2x_ops_are_judged_as_their_direct_commands() {
        let p = Policy::embedded();
        let may = |role, op| {
            let (c, d) = op_authority(op);
            p.allows(role, c, d)
        };
        for op in [
            ApplyOp::Mute,
            ApplyOp::Unmute,
            ApplyOp::PresetLoad,
            ApplyOp::MasterVolume,
            ApplyOp::OutDelay,
        ] {
            assert!(may(Role::Control, op), "{op:?}");
            assert!(!may(Role::Viewer, op), "{op:?}");
        }
        for op in [ApplyOp::Click, ApplyOp::SyncPulse] {
            assert!(!may(Role::Control, op) && may(Role::Admin, op), "{op:?}");
        }
        let (c, d) = system_mute_authority();
        assert!(p.allows(Role::Control, c, d) && !p.allows(Role::Viewer, c, d));
    }
}
