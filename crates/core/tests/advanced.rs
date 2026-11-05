use stashfinder_core::{index::SearchRow, search::*};
fn row() -> SearchRow {
    SearchRow {
        entry: vec![2],
        parent: vec![1],
        object: vec![2],
        name: "BBB 电影.mkv".into(),
        path: "aaa\\BBB 电影.mkv".into(),
        kind: "file".into(),
        size: Some("0".into()),
        created: None,
        modified: None,
        attributes: 0,
    }
}
fn matches(text: &str) -> bool {
    Query::compile(text, false, &QueryOptions::default())
        .unwrap()
        .matches_row(&row(), &[])
        .unwrap()
}
#[test]
fn boolean_precedence_quotes_paths_and_wildcards() {
    for (q, want) in [
        ("aaa bbb", true),
        ("aaa | absent bbb", true),
        ("aaa absent | bbb", true),
        ("<aaa absent> | missing", false),
        ("aaa !absent", true),
        ("!<aaa|bbb>", false),
        ("\"BBB 电影\"", true),
        ("*.mkv", true),
        ("*.txt", false),
        ("path:aaa", true),
        ("parent:aaa", true),
        ("ancestor:aa", false),
        ("file: ext:txt;mkv", true),
        ("folder:", false),
        ("size:0", true),
        ("size:>0", false),
        ("size:0..1KiB", true),
        ("dm:today", false),
    ] {
        assert_eq!(matches(q), want, "{q}");
    }
    let mut r = row();
    r.path = "aaa\\nested\\BBB 电影.mkv".into();
    assert!(
        Query::compile("ancestor:aaa", false, &QueryOptions::default())
            .unwrap()
            .matches_row(&r, &[])
            .unwrap()
    );
    assert!(
        !Query::compile("parent:aaa", false, &QueryOptions::default())
            .unwrap()
            .matches_row(&r, &[])
            .unwrap()
    );
}
#[test]
fn graphical_filters_match_modes_and_missing_values() {
    let mut o = QueryOptions {
        case_sensitive: true,
        ..Default::default()
    };
    assert!(
        !Query::compile("bbb", false, &o)
            .unwrap()
            .matches_row(&row(), &[])
            .unwrap()
    );
    o.case_sensitive = false;
    o.mode = MatchMode::WholeName;
    assert!(
        Query::compile("\"BBB 电影.mkv\"", false, &o)
            .unwrap()
            .matches_row(&row(), &[])
            .unwrap()
    );
    assert!(
        !Query::compile("\"BBB 电影\"", false, &o)
            .unwrap()
            .matches_row(&row(), &[])
            .unwrap()
    );
    assert!(
        !Query::compile("BBB", false, &o)
            .unwrap()
            .matches_row(&row(), &[])
            .unwrap()
    );
    assert!(Query::compile("BBB 电影.mkv", false, &o).is_ok());
    o.mode = MatchMode::Contains;
    o.extensions = vec![".MKV".into(), "txt".into()];
    o.size = Range {
        min: Some("0".into()),
        max: Some("0".into()),
    };
    let q = Query::compile("", false, &o).unwrap();
    assert!(q.matches_row(&row(), &[]).unwrap());
    let mut r = row();
    r.size = None;
    assert!(!q.matches_row(&r, &[]).unwrap());
    assert!(
        Query::compile(
            "",
            false,
            &QueryOptions {
                extensions: vec!["*.txt".into()],
                ..Default::default()
            }
        )
        .is_err()
    );
}
#[test]
fn pcre2_unicode_lookahead_backreferences_and_errors() {
    assert!(matches(r#"regex:"(?=BBB).*电影""#));
    let o = QueryOptions {
        mode: MatchMode::Regex,
        ..Default::default()
    };
    let q = Query::compile(r"(?<word>B)\k<word>B", true, &o).unwrap();
    assert!(q.matches_row(&row(), &[]).unwrap());
    assert!(Query::compile("(", true, &o).is_err());
    let q = Query::compile("(*NO_START_OPT)(a+)+$", true, &o).unwrap();
    let mut r = row();
    r.name = format!("{}!", "a".repeat(1000));
    assert!(q.matches_row(&r, &[]).is_err());
}
#[test]
fn duplicate_keys_and_invalid_expressions_are_explicit() {
    assert!(
        Query::compile(
            &format!("{}bbb", "!".repeat(50)),
            false,
            &Default::default()
        )
        .is_err()
    );
    assert_eq!(
        Query::compile("ext:mkv dupe:name;size;dm", false, &Default::default())
            .unwrap()
            .duplicate(),
        &[
            DuplicateField::Name,
            DuplicateField::Size,
            DuplicateField::Modified
        ]
    );
    assert_eq!(
        Query::compile("dupesize:", false, &Default::default())
            .unwrap()
            .duplicate(),
        &[DuplicateField::Size]
    );
    for q in [
        "a |",
        "<a",
        "a>",
        "!",
        "unknown:x",
        "size:bad",
        "dm:2026-99-99",
        "dupe:bad",
        "<dupe:>",
        "!dupe:",
    ] {
        assert!(
            Query::compile(q, false, &Default::default()).is_err(),
            "{q}"
        );
    }
}
