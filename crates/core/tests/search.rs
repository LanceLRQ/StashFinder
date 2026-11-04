use stashfinder_core::search::Query;

#[test]
fn path_words_are_unordered_literal_and_terms() {
    for (text, name, path, expected) in [
        ("aaa", "bbb.exe", r"aaa\bbb.exe", true),
        ("aaa bbb", "aaacccbbb.exe", "aaacccbbb.exe", true),
        ("aaa bbb", "bbbcccaaa.exe", "bbbcccaaa.exe", true),
        ("aaa bbb", "bbb.exe", r"aaa\bbb.exe", true),
        ("\"aaa bbb\"", "aaacccbbb.exe", "aaacccbbb.exe", false),
        ("\"aaa bbb\"", "aaa bbb.exe", "aaa bbb.exe", true),
        ("项目/AAA bbb", "BBB.exe", r"项目\aaa\BBB.exe", true),
        ("aaa absent", "aaa.txt", "aaa.txt", false),
        ("", "anything", "anything", true),
        ("  \t", "anything", "anything", true),
        (".txt", "资料.TXT", "资料.TXT", true),
    ] {
        assert_eq!(
            Query::parse(text, false).unwrap().matches(name, path, &[]),
            expected,
            "{text}"
        );
    }
    assert!(
        !Query::parse("aaa", true)
            .unwrap()
            .matches("bbb.exe", r"aaa\bbb.exe", &[])
    );
}

#[test]
fn explicit_drive_terms_use_only_current_mounts() {
    let query = Query::parse(r"C:\aaa bbb", false).unwrap();
    assert!(query.matches("bbb.exe", r"aaa\bbb.exe", &[r"C:\".into()]));
    assert!(!query.matches("bbb.exe", r"aaa\bbb.exe", &[]));
    assert!(!query.matches("bbb.exe", r"aaa\bbb.exe", &[r"D:\".into()]));
    assert!(query.matches("bbb.exe", r"aaa\bbb.exe", &[r"D:\".into(), r"c:\".into()]));
    assert!(
        !Query::parse(r"C:\aaa", true)
            .unwrap()
            .matches("aaa", "aaa", &[r"C:\".into()])
    );
}

#[test]
fn malformed_or_excessive_queries_are_not_empty_success() {
    assert!(Query::parse("\"unfinished", false).is_err());
    assert!(Query::parse("a\0b", false).is_err());
    assert!(Query::parse(&"a".repeat(8192), false).is_err());
    assert!(Query::parse(&vec!["x"; 100].join(" "), false).is_err());
    assert!(Query::parse("\"\"", false).is_err());
}
