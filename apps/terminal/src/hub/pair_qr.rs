//! -- WP SECPAIR -- The pairing link as a QR code in Unicode blocks (PLAN-SEC 4.3), for
//! `uconsole hub`'s pairing line: a QR scanner reads `dspi://pair?h=&a=&k=&p=`, so the hub's
//! key reaches it out of band with the PIN, and pairing by it cannot be relayed.
//!
//! Two modules per character cell (`▀`, `▄`, `█`, space), dark on an explicit light
//! background (ANSI), so it stands out on a dark terminal too, with the standard's
//! quiet zone of [`QUIET`] modules. Only for a terminal (-- WP SECFIX --: the link itself too; `--json`
//! carries the link, and a log gets neither).

use qrcodegen::{QrCode, QrCodeEcc};

/// Light modules around the code: the quiet zone the standard asks for (-- WP SECFIX --).
const QUIET: i32 = 4;
/// Black on bright white, then back to the terminal's colours.
const START: &str = "\x1b[30;107m";
const END: &str = "\x1b[0m";

/// The lines to print, each wrapped in [`START`] and [`END`]; `None` if `text` is too
/// long for a QR code (it never is for a pairing link).
pub(super) fn lines(text: &str) -> Option<Vec<String>> {
    let qr = QrCode::encode_text(text, QrCodeEcc::Medium).ok()?;
    let size = qr.size();
    let dark =
        |x: i32, y: i32| (0..size).contains(&x) && (0..size).contains(&y) && qr.get_module(x, y);
    let (from, to) = (-QUIET, size.saturating_add(QUIET));
    let mut out = Vec::new();
    let mut y = from;
    while y < to {
        let mut line = String::from(START);
        for x in from..to {
            line.push(match (dark(x, y), dark(x, y.saturating_add(1))) {
                (true, true) => '█',
                (true, false) => '▀',
                (false, true) => '▄',
                (false, false) => ' ',
            });
        }
        line.push_str(END);
        out.push(line);
        y = y.saturating_add(2);
    }
    Some(out)
}

#[cfg(test)]
mod tests_wp_secpair {
    #![allow(
        clippy::unwrap_used,
        clippy::indexing_slicing,
        clippy::arithmetic_side_effects
    )]
    use super::*;

    /// The modules back out of the blocks: what is printed is the code.
    #[test]
    fn wp_secpair_the_blocks_are_the_code() {
        let link = "dspi://pair?h=0b3e7c1e-1111-4222-8333-444455556666&a=dspi-abc.local:11915\
                    &k=q83vEjRWeJCrze8SNFZ4kKvN7xI0VniQq83vEjRWeJA&p=482913";
        let qr = QrCode::encode_text(link, QrCodeEcc::Medium).unwrap();
        let printed = lines(link).unwrap();
        let size = qr.size();
        let side = (size + 2 * QUIET) as usize;
        assert_eq!(printed.len(), side.div_ceil(2));
        for (row, line) in printed.iter().enumerate() {
            let body = line.strip_prefix(START).unwrap().strip_suffix(END).unwrap();
            let cells: Vec<char> = body.chars().collect();
            assert_eq!(cells.len(), side);
            for (col, c) in cells.iter().enumerate() {
                let (x, y) = (col as i32 - QUIET, 2 * row as i32 - QUIET);
                let module = |x: i32, y: i32| {
                    (0..size).contains(&x) && (0..size).contains(&y) && qr.get_module(x, y)
                };
                let (top, bottom) = match c {
                    '█' => (true, true),
                    '▀' => (true, false),
                    '▄' => (false, true),
                    ' ' => (false, false),
                    other => panic!("{other:?}"),
                };
                assert_eq!((top, bottom), (module(x, y), module(x, y + 1)), "({x},{y})");
            }
        }
        // The quiet zone is light.
        assert!(printed[0].contains(&" ".repeat(side)));
        // It fits an 80-column terminal.
        assert!(side <= 80, "{side}");
    }

    #[test]
    fn wp_secpair_a_text_too_long_is_none() {
        assert!(lines(&"x".repeat(8000)).is_none());
    }
}
