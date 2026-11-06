//! WP TM20: an empty quoted token, an escaped quote, and every line still
//! parsing as it did.

use dspi_cmd::{Command, Context, format, parse, quote, tokenize};
use dspi_proto::value::Value;
use proptest::prelude::*;

fn set_of(line: &str) -> Command {
    let toks = tokenize(line);
    let refs: Vec<&str> = toks.iter().map(String::as_str).collect();
    parse(&refs, &Context::default()).unwrap_or_else(|e| panic!("`{line}`: {e}"))
}

fn name_of(line: &str) -> String {
    match set_of(line) {
        Command::Set {
            path: "dev.name",
            value: Value::Text(t),
            ..
        } => t,
        other => panic!("`{line}`: {other:?}"),
    }
}

/// The tokenizer as it was before TM20.
fn old_tokenize(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    for c in line.chars() {
        match c {
            '"' => quoted = !quoted,
            c if c.is_whitespace() && !quoted => {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
            }
            c => cur.push(c),
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

#[test]
fn wp_tm20_an_empty_quoted_token_is_kept() {
    assert_eq!(tokenize(r#"dev.name """#), ["dev.name", ""]);
    assert_eq!(tokenize(r#""" a "" b"#), ["", "a", "", "b"]);
    assert_eq!(name_of(r#"dev.name """#), "");
}

#[test]
fn wp_tm20_an_unfinished_quote_does_not_clear() {
    assert_eq!(tokenize(r#"dev.name ""#), ["dev.name"]);
    assert!(parse(&["dev.name"], &Context::default()).is_err());
    // Text after it is still taken, as it always was.
    assert_eq!(tokenize(r#"dev.name "ab c"#), ["dev.name", "ab c"]);
}

#[test]
fn wp_tm20_quotes_inside_a_word_make_no_extra_token() {
    assert_eq!(tokenize(r#"a""b"#), ["ab"]);
    assert_eq!(tokenize(r#"a"" b"#), ["a", "b"]);
    assert_eq!(tokenize(r#"""a"#), ["a"]);
}

#[test]
fn wp_tm20_an_escaped_quote_is_part_of_the_name() {
    assert_eq!(name_of(r#"dev.name "Say \"hi\"""#), r#"Say "hi""#);
    assert_eq!(name_of(r#"dev.name 12\"Sub"#), r#"12"Sub"#);
    // A whole name in literal quotes keeps them.
    assert_eq!(name_of(r#"dev.name \"x\""#), r#""x""#);
}

#[test]
fn wp_tm20_backslashes_only_count_before_a_quote() {
    assert_eq!(
        tokenize(r"preset load C:\dir\f"),
        ["preset", "load", r"C:\dir\f"]
    );
    assert_eq!(tokenize(r"a\\b"), [r"a\\b"]);
    assert_eq!(tokenize(r#""C:\\" x"#), [r"C:\", "x"]);
    assert_eq!(tokenize(r#""a\\\"b""#), [r#"a\"b"#]);
}

#[test]
fn wp_tm20_the_echo_of_a_name_parses_back_to_it() {
    let ctx = Context::default();
    for (name, line) in [
        ("", r#"dev.name """#),
        ("Plain", "dev.name Plain"),
        ("Two words", r#"dev.name "Two words""#),
        (r#"12" Sub"#, r#"dev.name "12\" Sub""#),
        (r"back\", r"dev.name back\"),
        (r"back\ space\", r#"dev.name "back\ space\\""#),
    ] {
        let cmd = Command::Set {
            path: "dev.name",
            indices: vec![],
            value: Value::Text(name.into()),
        };
        assert_eq!(format(&cmd, &ctx), line);
        assert_eq!(name_of(line), name);
    }
}

/// Lines weighted towards what the tokenizer treats specially.
fn line() -> impl Strategy<Value = String> {
    prop_oneof![
        any::<String>(),
        proptest::collection::vec(
            prop_oneof![
                Just("\""),
                Just("\\"),
                Just(" "),
                Just("\t"),
                Just("="),
                Just("dev.name"),
                Just("get"),
                Just("eq"),
                Just("mix"),
                Just("usb.1"),
                Just("1"),
                Just("é"),
            ],
            0..12
        )
        .prop_map(|parts| parts.concat()),
    ]
}

proptest! {
    /// Nothing typed panics, on the way in or on the way back out as an echo.
    #[test]
    fn proptest_wp_tm20_no_input_panics(line in line()) {
        let toks = tokenize(&line);
        let refs: Vec<&str> = toks.iter().map(String::as_str).collect();
        let ctx = Context::default();
        if let Ok(cmd) = parse(&refs, &ctx) {
            let _ = format(&cmd, &ctx);
        }
    }

    /// Any text at all survives being quoted.
    #[test]
    fn proptest_wp_tm20_quote_round_trips(text in line()) {
        let toks = tokenize(&format!("dev.name {}", quote(&text)));
        prop_assert_eq!(toks, vec!["dev.name".to_string(), text]);
    }

    /// Without `\"`, the only difference from the old tokenizer is the empty
    /// tokens it used to drop.
    #[test]
    fn proptest_wp_tm20_other_lines_parse_as_before(line in line()) {
        if line.contains("\\\"") {
            return Ok(());
        }
        let mut new = tokenize(&line);
        new.retain(|t| !t.is_empty());
        prop_assert_eq!(new, old_tokenize(&line));
    }
}
