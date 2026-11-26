//! The filter types' names, one table for every client (WP CG34).
//!
//! The names are the Console's band-list names (`FilterType.name`,
//! DSPMath.swift at release/v1.1.6): "Off", "Bell", "High Cut (12dB)" for
//! a second-order low pass, "LR4 Low pass" for a crossover (-- INT016 -- (A10
//! one filter vocabulary): Bell for the Console's Peaking, the direction in
//! sentence case). The table itself
//! is `dspi_proto::FilterType::display_name`, which the terminal UI reads
//! directly; this module hands it to the bindings (UniFFI here, the C ABI
//! and the web binding on top).

use dspi_proto::FilterType;

/// One filter type and the name clients show for it.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct FilterTypeName {
    /// The firmware's type code (`FILTER_*`): 0..13 PEQ, 32..63 crossover.
    pub code: u8,
    /// The name, e.g. "High Cut (12dB)".
    pub name: String,
}

/// Every filter type the firmware defines with its name, in the Console's
/// order: the PEQ types 0..13, then the crossovers 32..63.
#[cfg_attr(feature = "uniffi", uniffi::export)]
pub fn filter_type_names() -> Vec<FilterTypeName> {
    FilterType::all_defined()
        .map(|t| FilterTypeName {
            code: t.to_raw(),
            name: t.display_name(),
        })
        .collect()
}

/// The name of filter type `code`; "Type 14" for a code the firmware does
/// not define (yet).
#[cfg_attr(feature = "uniffi", uniffi::export)]
pub fn filter_type_name(code: u8) -> String {
    FilterType::from_raw(code).display_name()
}

#[cfg(test)]
mod tests_wp_cg34 {
    use super::*;

    #[test]
    fn wp_cg34_the_table_has_every_defined_type_once_with_the_consoles_names() {
        let t = filter_type_names();
        assert_eq!(t.len(), 14 + 32);
        let codes: Vec<u8> = t.iter().map(|n| n.code).collect();
        let want: Vec<u8> = (0..=13).chain(32..=63).collect();
        assert_eq!(codes, want);
        let name = |c: u8| t.iter().find(|n| n.code == c).map(|n| n.name.as_str());
        assert_eq!(name(0), Some("Off"));
        assert_eq!(name(4), Some("High Cut (12dB)"));
        assert_eq!(name(13), Some("Low Cut (6dB)"));
        assert_eq!(name(11), Some("Linkwitz Transform"));
        assert_eq!(name(1), Some("Bell")); // -- INT016 -- (A10 one filter vocabulary)
        assert_eq!(name(35), Some("LR4 High pass"));
        assert_eq!(name(63), Some("BES8 High pass"));
        for n in &t {
            assert_eq!(filter_type_name(n.code), n.name);
        }
        assert_eq!(filter_type_name(14), "Type 14");
        assert_eq!(filter_type_name(255), "Type 255");
    }
}
