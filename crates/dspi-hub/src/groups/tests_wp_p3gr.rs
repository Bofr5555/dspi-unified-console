//! WP P3GR: the group book, its bounds, its file and the expansion, without a hub.
#![allow(clippy::unwrap_used, clippy::indexing_slicing)]

use super::*;

fn edit(id: Option<&str>, name: &str, members: &[&str]) -> GroupEdit {
    GroupEdit {
        id: id.map(str::to_owned),
        name: name.to_owned(),
        note: None,
        members: members.iter().map(|m| (*m).to_owned()).collect(),
    }
}

fn serials(n: usize) -> Vec<Value> {
    (0..n).map(|i| json!(format!("S{i:03}"))).collect()
}

/// A fresh directory. `ci/run.sh` points `TMPDIR` at the home volume, not tmpfs.
fn scratch(name: &str) -> PathBuf {
    let d = std::env::temp_dir()
        .join("wp-p3gr-unit")
        .join(format!("{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&d);
    fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn wp_p3gr_ids_are_the_hubs_and_never_reused() {
    let mut book = GroupBook::new();
    let a = book.set(edit(None, "Hall", &["A"])).unwrap();
    let b = book.set(edit(None, "Bar", &[])).unwrap();
    assert_eq!((a.id.as_str(), b.id.as_str()), ("g1", "g2"));
    book.delete("g2").unwrap();
    let c = book.set(edit(None, "Foyer", &[])).unwrap();
    assert_eq!(c.id, "g3", "an id is never given twice");
    // Replaced whole.
    let a2 = book
        .set(edit(Some("g1"), "Main hall", &["B", "C"]))
        .unwrap();
    assert_eq!(a2.id, "g1");
    assert_eq!(book.get("g1").unwrap().members, ["B", "C"]);
    assert_eq!(book.get("g1").unwrap().note, None);
    assert_eq!(book.groups().len(), 2);
    assert_eq!(
        book.set(edit(Some("g9"), "X", &[])).unwrap_err().code(),
        "not_found"
    );
    assert_eq!(book.delete("g9").unwrap_err().code(), "not_found");
}

#[test]
fn wp_p3gr_the_bounds_are_checked() {
    let mut book = GroupBook::new();
    for i in 0..MAX_GROUPS {
        book.set(edit(None, &format!("G{i}"), &[])).unwrap();
    }
    let full = book.set(edit(None, "One more", &[])).unwrap_err();
    assert_eq!(full.code(), "conflict", "the 65th group");
    // Replacing one of the 64 is fine.
    book.set(edit(Some("g64"), "Last", &[])).unwrap();

    let parse = |v: Value| GroupEdit::parse(&v);
    assert!(parse(json!({"name": "Hall", "members": serials(64)})).is_ok());
    let e = parse(json!({"name": "Hall", "members": serials(65)})).unwrap_err();
    assert_eq!(e.code(), "bad_request", "the 65th member");
    for bad in [
        json!({"members": []}),
        json!({"name": "", "members": []}),
        json!({"name": "   ", "members": []}),
        json!({"name": "x".repeat(49), "members": []}),
        json!({"name": "a\u{7}b", "members": []}),
        json!({"name": "Hall", "note": "n".repeat(257), "members": []}),
        json!({"name": "Hall", "note": 3, "members": []}),
        json!({"name": "Hall"}),
        json!({"name": "Hall", "members": "A"}),
        json!({"name": "Hall", "members": [1]}),
        json!({"name": "Hall", "members": [""]}),
        json!({"name": "Hall", "members": ["A", "A"]}),
        json!({"name": "Hall", "members": ["group:g1"]}),
        json!({"name": "Hall", "members": ["x".repeat(65)]}),
        json!({"id": "g 1", "name": "Hall", "members": []}),
        json!({"id": 7, "name": "Hall", "members": []}),
    ] {
        assert_eq!(
            parse(bad.clone()).unwrap_err().code(),
            "bad_request",
            "{bad}"
        );
    }
    let ok = parse(json!({"name": " Hall ", "note": "line\nnext", "members": ["A"]})).unwrap();
    assert_eq!(ok.name, "Hall");
    assert_eq!(ok.note.as_deref(), Some("line\nnext"));
    assert!(parse(json!({"name": "x".repeat(48), "note": "n".repeat(256), "members": []})).is_ok());
}

#[test]
fn wp_p3gr_a_forgotten_device_leaves_every_group() {
    let mut book = GroupBook::new();
    book.set(edit(None, "Hall", &["A", "B"])).unwrap();
    book.set(edit(None, "Bar", &["C"])).unwrap();
    book.set(edit(None, "All", &["C", "B", "A"])).unwrap();
    assert_eq!(book.forget("B"), ["g1", "g3"]);
    assert_eq!(book.get("g1").unwrap().members, ["A"]);
    assert_eq!(book.get("g3").unwrap().members, ["C", "A"]);
    assert!(book.forget("B").is_empty());
}

#[test]
fn wp_p3gr_the_file_round_trips() {
    let mut book = GroupBook::new();
    book.set(GroupEdit {
        note: Some("left \"stack\"".into()),
        ..edit(None, "Hall", &["A", "B"])
    })
    .unwrap();
    book.set(edit(None, "Bar", &[])).unwrap();
    book.delete("g1").unwrap();
    book.set(edit(None, "Foyer", &["Z"])).unwrap();
    let text = book.to_file();
    assert!(
        text.starts_with("{\"v\":1,\"next_id\":4,\"groups\":[\n"),
        "{text}"
    );
    let (back, whole) = GroupBook::from_file(&text);
    assert!(whole);
    assert_eq!(back, book);
}

#[test]
fn wp_p3gr_a_torn_file_reads_what_it_can() {
    let mut book = GroupBook::new();
    book.set(edit(None, "Hall", &["A"])).unwrap();
    book.set(edit(None, "Bar", &["B"])).unwrap();
    book.set(edit(None, "Foyer", &["C"])).unwrap();
    let text = book.to_file();
    // Torn in the third group's line.
    let cut_at = text.find("Foyer").unwrap();
    let (back, whole) = GroupBook::from_file(&text[..cut_at]);
    assert!(!whole);
    assert_eq!(back.groups().len(), 2);
    assert_eq!(back.get("g2").unwrap().members, ["B"]);
    // The header line still says where ids go on: g3 was given, so never again.
    assert_eq!(back.next_id(), 4);
    // A header lost too: past the highest id kept.
    let body: String = text.lines().skip(1).take(2).collect::<Vec<_>>().join("\n");
    let (back, whole) = GroupBook::from_file(&body);
    assert!(!whole);
    assert_eq!(back.next_id(), 3);
    // A bad entry in a whole file is left out, and the file is not whole.
    let (back, whole) = GroupBook::from_file(
        "{\"v\":1,\"next_id\":9,\"groups\":[{\"id\":\"x1\",\"name\":\"Bad id\"},\
         {\"id\":\"g2\",\"name\":\"\"},{\"id\":\"g3\",\"name\":\"Ok\",\
         \"members\":[\"A\",\"A\",\"\"]},{\"id\":\"g3\",\"name\":\"Twice\"}]}",
    );
    assert!(!whole);
    assert_eq!(back.groups().len(), 1);
    assert_eq!(back.get("g3").unwrap().members, ["A"]);
    assert_eq!(back.next_id(), 9);
    // Garbage is an empty book, never a panic.
    for junk in [
        "",
        "\u{0}\u{1}",
        "[",
        "{\"groups\":7}",
        "{\"v\":1,\"next_id\":\"x\"",
    ] {
        let (back, _) = GroupBook::from_file(junk);
        assert!(back.is_empty(), "{junk:?}");
    }
}

#[test]
fn wp_p3gr_a_corrupt_file_is_kept_aside_and_rewritten() {
    let dir = scratch("corrupt");
    let path = dir.join(FILE_NAME);
    let mut book = GroupBook::new();
    book.set(edit(None, "Hall", &["A"])).unwrap();
    book.set(edit(None, "Bar", &["B"])).unwrap();
    let text = book.to_file();
    fs::write(&path, &text[..text.find("Bar").unwrap()]).unwrap();
    let (back, rewrite) = load(&path, 1_800_000_000);
    assert!(rewrite);
    assert_eq!(back.groups().len(), 1);
    assert!(dir.join("groups.json.bad-1800000000").exists());
    assert!(!path.exists(), "moved aside, to be written again");
    // A whole file reads as it is; none is an empty book.
    fs::write(&path, &text).unwrap();
    assert_eq!(load(&path, 1), (book, false));
    fs::remove_file(&path).unwrap();
    assert_eq!(load(&path, 1), (GroupBook::new(), false));
}

struct Fixed(Vec<(&'static str, Vec<&'static str>)>);

impl GroupResolver for Fixed {
    fn members(&self, id: &str) -> Option<Vec<String>> {
        self.0
            .iter()
            .find(|(g, _)| *g == id)
            .map(|(_, m)| m.iter().map(|s| (*s).to_owned()).collect())
    }
}

#[test]
fn wp_p3gr_expand_resolves_groups_in_order_once_per_device() {
    let groups = Fixed(vec![("g1", vec!["A", "B"]), ("g2", vec!["B", "C"])]);
    let targets: Vec<String> = ["X", "group:g1", "group:g2", "A", "group:g7"]
        .iter()
        .map(|s| (*s).to_owned())
        .collect();
    let s = |v: &str| Expanded::Serial(v.into());
    assert_eq!(
        expand(&targets, Some(&groups)),
        [
            s("X"),
            s("A"),
            s("B"),
            s("C"),
            Expanded::Unknown("group:g7".into())
        ]
    );
    // A hub without groups: each group target is one unknown row.
    assert_eq!(
        expand(&targets, None),
        [
            s("X"),
            Expanded::Unknown("group:g1".into()),
            Expanded::Unknown("group:g2".into()),
            s("A"),
            Expanded::Unknown("group:g7".into())
        ]
    );
    assert!(valid_group_target("group:g1"));
    assert!(valid_group_target("group:hall-2"));
    assert!(!valid_group_target("group:"));
    assert!(!valid_group_target("group:a b"));
    assert!(!valid_group_target("g1"));
    let row = unknown_row("group:g7", Some("0:param.set".into()));
    assert_eq!(
        (row.serial.as_str(), row.ok, row.code.as_deref()),
        ("group:g7", false, Some("not_found"))
    );
    let row = truncated_row(None);
    assert_eq!(row.code.as_deref(), Some("bad_request"));
    assert!(row.msg.unwrap().starts_with("truncated"));
}
