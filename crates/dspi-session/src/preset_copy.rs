//! -- WP FIN015 -- The rule a preset copy keeps (REVE B1, REVF S1).
//!
//! The device has no copy: a host saves the live settings to the destination
//! and then to the source, so the source's stored contents are lost unless
//! the source is the active slot (whose stored contents the live settings
//! are). The core's `copy_preset_with` refuses through this rule, and so does
//! the Terminal, which writes `preset.save` through this crate without the
//! core: one rule and one set of words for both.

use dspi_proto::generated::status::PRESET_SLOTS;

/// The active slot when it is one the device has (0 to 9); `None` for an
/// unknown one (a read that failed, or a reply of 10 or more).
pub fn known_slot(active: Option<u8>) -> Option<u8> {
    active.filter(|&s| s < PRESET_SLOTS as u8)
}

/// Why a copy from `source` must not go, when `active` is the slot the device
/// names active (`None`: unknown): `None` when it may.
pub fn copy_refusal(source: u8, active: Option<u8>) -> Option<String> {
    if known_slot(active) == Some(source) {
        return None;
    }
    Some(format!(
        "preset {source} is not the active preset: a copy saves the current settings, \
         so it can only copy from the active preset"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_copy_goes_only_from_the_known_active_slot() {
        assert_eq!(copy_refusal(2, Some(2)), None);
        assert!(copy_refusal(2, Some(5)).is_some());
        assert!(copy_refusal(0, None).is_some(), "unknown is not slot 0");
        assert!(copy_refusal(255, Some(255)).is_some(), "past the last slot");
        assert_eq!(known_slot(Some(9)), Some(9));
        assert_eq!(known_slot(Some(10)), None);
        assert_eq!(known_slot(Some(255)), None);
    }
}
