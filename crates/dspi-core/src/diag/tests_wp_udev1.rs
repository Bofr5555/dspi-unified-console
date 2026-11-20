//! WP UDEV1: the bundle's udev facts name the one rule of every component
//! (xtask/70-dspi.rules) and say whether the installed copy is this release's.

use super::*;

/// The repository's rule file, standing in for an installed one.
const SOURCE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../xtask/70-dspi.rules");

#[test]
fn wp_udev1_the_built_in_rule_is_the_source_file() {
    assert_eq!(
        std::fs::read_to_string(SOURCE).ok().as_deref(),
        Some(UDEV_RULE)
    );
}

#[test]
fn wp_udev1_the_first_existing_copy_is_reported_and_judged() {
    let missing = "/nonexistent/udev1/70-dspi.rules";
    let v = udev_rule_facts(&[missing, SOURCE]);
    assert_eq!(v["path"], json!(SOURCE));
    assert_eq!(v["present"], json!(true));
    assert_eq!(v["current"], json!(true));
    let v = udev_rule_facts(&[missing]);
    assert_eq!(v["path"], json!(missing));
    assert_eq!(v["present"], json!(false));
    assert_eq!(v["current"], Value::Null);
}

#[test]
fn wp_udev1_an_older_rule_is_not_current() {
    // Cargo.toml exists and is not the rule: what an older or edited rule looks like
    let other = concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml");
    assert_eq!(udev_rule_facts(&[other])["current"], json!(false));
}
