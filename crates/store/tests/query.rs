use stashfinder_core::search::*;
use stashfinder_store::{
    search::{SearchIndex, query::*},
    volume::*,
};
use std::{
    collections::HashSet,
    sync::{Arc, atomic::AtomicBool},
};
fn source(name: &str, files: &[(&str, Option<u64>, u8)]) -> Source {
    let d = tempfile::tempdir().unwrap();
    let mut v = Volume::create(&d.path().join("v.sqlite"), "{}").unwrap();
    let g = v.begin_generation().unwrap();
    let mut objects = vec![Object {
        identity: None,
        id: vec![0],
        source: "test".into(),
        kind: "folder".into(),
        size: None,
        created: None,
        modified: None,
        attributes: 0,
    }];
    let mut entries = vec![Entry {
        id: vec![0],
        object: vec![0],
        parent: None,
        name: vec![92],
    }];
    let mut seen = HashSet::new();
    for (i, (name, size, object)) in files.iter().enumerate() {
        if seen.insert(object) {
            objects.push(Object {
                identity: None,
                id: vec![*object],
                source: "test".into(),
                kind: "file".into(),
                size: size.map(|s| s as i64),
                created: None,
                modified: None,
                attributes: 0,
            });
        }
        entries.push(Entry {
            id: ((i + 1) as u32).to_le_bytes().to_vec(),
            object: vec![*object],
            parent: Some(vec![0]),
            name: name.encode_utf16().collect(),
        });
    }
    v.write_batch(g, &objects, &entries, &[], None).unwrap();
    v.complete(g).unwrap();
    v.publish(g).unwrap();
    Source {
        id: name.into(),
        name: name.into(),
        filesystem: name.into(),
        connection: "offline".into(),
        mounts: vec![],
        index: Arc::new(SearchIndex::load(&v).unwrap().unwrap()),
        needs_validation: false,
    }
}
#[test]
fn duplicates_are_filtered_globally_before_pagination_and_sorting() {
    let sources = vec![
        source(
            "a",
            &[
                ("same.txt", Some(0), 1),
                ("same.txt", Some(10), 2),
                ("unknown.txt", None, 3),
            ],
        ),
        source(
            "b",
            &[
                ("same.txt", Some(0), 1),
                ("other.txt", Some(10), 2),
                ("unknown.txt", None, 3),
            ],
        ),
    ];
    let options = QueryOptions {
        sort: SortField::Partition,
        ..Default::default()
    };
    let q = Query::compile("dupe:name;size", false, &options).unwrap();
    let result = ResultSet::run(sources, &q, &options, &AtomicBool::new(false)).unwrap();
    assert_eq!(result.len(), 2);
    assert_eq!(result.missing, 2);
    assert_eq!(result.groups, 1);
    let a = result.page(0, 1).unwrap();
    let b = result.page(1, 1).unwrap();
    assert_eq!(a[0].partition_id, "a");
    assert_eq!(b[0].partition_id, "b");
    assert_eq!(a[0].duplicate_group, b[0].duplicate_group);
    assert_eq!(a[0].file.size.as_deref(), Some("0"));
}
#[test]
fn hard_links_do_not_make_a_duplicate_object_but_remain_visible_in_real_groups() {
    let options = QueryOptions::default();
    let q = Query::compile("dupesize:", false, &options).unwrap();
    let links = || source("a", &[("one.txt", Some(9), 1), ("link.txt", Some(9), 1)]);
    assert_eq!(
        ResultSet::run(vec![links()], &q, &options, &AtomicBool::new(false))
            .unwrap()
            .len(),
        0
    );
    let r = ResultSet::run(
        vec![links(), source("b", &[("second.txt", Some(9), 1)])],
        &q,
        &options,
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(r.len(), 3);
    assert!(
        r.page(0, 100)
            .unwrap()
            .iter()
            .filter(|r| r.hard_link)
            .count()
            == 2
    );
}
#[test]
fn all_results_are_sorted_not_just_first_200_and_cancelled_work_is_rejected() {
    let names = (0..240)
        .rev()
        .map(|i| format!("{i:03}.txt"))
        .collect::<Vec<_>>();
    let records = names
        .iter()
        .enumerate()
        .map(|(i, n)| (n.as_str(), Some(i as u64), (i + 1) as u8))
        .collect::<Vec<_>>();
    let o = QueryOptions::default();
    let q = Query::compile("", false, &o).unwrap();
    let s = source("a", &records);
    assert!(ResultSet::run(vec![s.clone()], &q, &o, &AtomicBool::new(true)).is_err());
    let r = ResultSet::run(vec![s], &q, &o, &AtomicBool::new(false)).unwrap();
    assert_eq!(r.len(), 240);
    assert_eq!(r.page(0, 100).unwrap()[0].file.name, "000.txt");
    assert_eq!(
        r.page(220, 100).unwrap().last().unwrap().file.name,
        "239.txt"
    );
}

#[test]
fn combined_duplicate_result_matches_independent_reference_groups() {
    use std::collections::{BTreeSet, HashMap};
    let names = (0..200)
        .map(|i| {
            format!(
                "FILE-{:02}.{}",
                i % 20,
                if i % 2 == 0 { "txt" } else { "mkv" }
            )
        })
        .collect::<Vec<_>>();
    let records = names
        .iter()
        .enumerate()
        .map(|(i, n)| {
            (
                n.as_str(),
                if i % 7 == 0 {
                    None
                } else {
                    Some((i % 10) as u64)
                },
                (i + 1) as u8,
            )
        })
        .collect::<Vec<_>>();
    let mut groups = HashMap::<(String, u64), Vec<usize>>::new();
    for (i, (name, size, _)) in records.iter().enumerate() {
        if let Some(size) = size {
            groups
                .entry((name.to_lowercase(), *size))
                .or_default()
                .push(i);
        }
    }
    let expected = groups
        .values()
        .filter(|g| g.len() > 1)
        .flatten()
        .map(|i| records[*i].2)
        .collect::<BTreeSet<_>>();
    let o = QueryOptions::default();
    let q = Query::compile("dupe:name;size", false, &o).unwrap();
    let result =
        ResultSet::run(vec![source("p", &records)], &q, &o, &AtomicBool::new(false)).unwrap();
    let actual = (0..result.len())
        .step_by(100)
        .flat_map(|offset| result.page(offset, 100).unwrap())
        .map(|r| r.file.object[0])
        .collect::<BTreeSet<_>>();
    assert_eq!(actual, expected);
}
