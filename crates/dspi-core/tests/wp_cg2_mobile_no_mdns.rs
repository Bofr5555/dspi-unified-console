//! WP CG2: the mobile libraries do not link the mDNS stack. `cargo tree` answers
//! for every target, feature selection as `dspi-ffi` declares it. The same check
//! for the desktop feature proves the search finds it when it is there.

use std::path::Path;
use std::process::Command;

fn tree(args: &[&str]) -> String {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let out = Command::new(cargo)
        .current_dir(root)
        .args([
            "tree",
            "--locked",
            "--offline",
            "--target",
            "all",
            "--prefix",
            "none",
        ])
        .args(args)
        .output()
        .expect("cargo runs");
    assert!(
        out.status.success(),
        "cargo tree: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn has_mdns(tree: &str) -> bool {
    tree.lines().any(|l| l.starts_with("mdns-sd "))
}

#[test]
fn wp_cg2_the_mobile_library_does_not_link_mdns() {
    let mobile = tree(&["-p", "dspi-ffi"]);
    assert!(
        mobile.contains("dspi-link "),
        "the search is looking at the right tree"
    );
    assert!(!has_mdns(&mobile), "dspi-ffi pulls in mdns-sd");
}

#[test]
fn wp_cg2_a_link_build_without_the_feature_does_not_link_mdns() {
    assert!(!has_mdns(&tree(&["-p", "dspi-core", "--features", "link"])));
    assert!(!has_mdns(&tree(&["-p", "dspi-capi"])));
}

#[test]
fn wp_cg2_the_desktop_feature_does_link_mdns() {
    assert!(has_mdns(&tree(&["-p", "dspi-core", "--features", "mdns"])));
    assert!(has_mdns(&tree(&["-p", "dspi-capi", "--features", "mdns"])));
}
