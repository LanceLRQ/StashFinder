use stashfinder_store::{
    initial::InitialPathsIncomplete,
    volume::{Entry, Object, Volume},
};

fn node(id: u8, parent: Option<u8>, directory: bool) -> (Object, Entry) {
    (
        Object {
            id: vec![id],
            identity: None,
            source: "ntfs64".into(),
            kind: if directory { "folder" } else { "file" }.into(),
            size: (!directory).then_some(0),
            created: Some(1u64.to_le_bytes()),
            modified: Some(2u64.to_le_bytes()),
            attributes: if directory { 16 } else { 32 },
        },
        Entry {
            id: vec![id],
            object: vec![id],
            parent: parent.map(|value| vec![value]),
            name: vec![u16::from(id) + 65],
        },
    )
}

#[test]
fn children_before_parents_remain_hidden_until_relationship_import() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("volume.sqlite");
    let mut db = Volume::create(&file, "{}").unwrap();
    let generation = db.begin_generation().unwrap();
    db.begin_initial_paths(generation).unwrap();
    let (child, link) = node(3, Some(2), false);
    let alias = Entry {
        id: vec![4],
        object: vec![3],
        parent: Some(vec![1]),
        name: vec![0x4e2d],
    };
    db.write_initial_batch(generation, &[child], &[link, alias])
        .unwrap();
    assert!(!db.has_path(generation, &[3]).unwrap());
    assert!(db.complete(generation).is_err());
    let (parent, parent_link) = node(2, Some(1), true);
    let (root, root_link) = node(1, None, true);
    db.write_initial_batch(generation, &[parent, root], &[parent_link, root_link])
        .unwrap();
    db.finish_initial_paths(generation, &[1]).unwrap();
    db.complete(generation).unwrap();
    db.publish(generation).unwrap();
    assert!(db.has_path(generation, &[3]).unwrap());
    assert!(db.has_path(generation, &[4]).unwrap());
    assert_eq!(db.object_count(generation).unwrap(), 3);
    let sql = rusqlite::Connection::open(&file).unwrap();
    assert_eq!(
        sql.query_row("SELECT count(*) FROM pragma_foreign_key_check", [], |r| r
            .get::<_, i64>(
            0
        ))
        .unwrap(),
        0
    );
    assert_eq!(
        sql.query_row("SELECT count(*) FROM pending_metadata", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        sql.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        5
    );
}

#[test]
fn invalid_candidate_is_discarded_without_touching_the_active_generation() {
    let dir = tempfile::tempdir().unwrap();
    let mut db = Volume::create(&dir.path().join("volume.sqlite"), "{}").unwrap();
    let old = db.begin_generation().unwrap();
    let (root, link) = node(1, None, true);
    db.write_batch(old, std::slice::from_ref(&root), &[link], &[], None)
        .unwrap();
    db.complete(old).unwrap();
    db.publish(old).unwrap();
    let generation = db.begin_generation().unwrap();
    db.begin_initial_paths(generation).unwrap();
    let (_, link) = node(1, None, true);
    let (child, child_link) = node(2, Some(99), false);
    db.write_initial_batch(generation, &[root, child], &[link, child_link])
        .unwrap();
    assert!(
        db.finish_initial_paths(generation, &[1])
            .unwrap_err()
            .is::<InitialPathsIncomplete>()
    );
    assert!(db.discard_initial(old, "must keep active").is_err());
    db.discard_initial(generation, "missing parent").unwrap();
    assert_eq!(db.active_generation().unwrap(), Some(old));
    assert!(db.has_path(old, &[1]).unwrap());
    assert_eq!(db.object_count(generation).unwrap(), 0);
}

#[test]
fn reparse_directory_is_visible_but_its_descendants_are_not() {
    let dir = tempfile::tempdir().unwrap();
    let mut db = Volume::create(&dir.path().join("volume.sqlite"), "{}").unwrap();
    let generation = db.begin_generation().unwrap();
    db.begin_initial_paths(generation).unwrap();
    let (root, root_link) = node(1, None, true);
    let (mut reparse, reparse_link) = node(2, Some(1), true);
    reparse.attributes |= 0x400;
    let (child, child_link) = node(3, Some(2), false);
    db.write_initial_batch(
        generation,
        &[root, reparse, child],
        &[root_link, reparse_link, child_link],
    )
    .unwrap();
    db.finish_initial_paths(generation, &[1]).unwrap();
    assert!(db.has_path(generation, &[2]).unwrap());
    assert!(!db.has_path(generation, &[3]).unwrap());
    assert_eq!(db.object_count(generation).unwrap(), 2);
}

#[test]
fn duplicate_object_rolls_back_batch_and_reopening_cannot_publish_unfinished_paths() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("volume.sqlite");
    let mut db = Volume::create(&file, "{}").unwrap();
    let generation = db.begin_generation().unwrap();
    db.begin_initial_paths(generation).unwrap();
    let (root, root_link) = node(1, None, true);
    db.write_initial_batch(generation, std::slice::from_ref(&root), &[root_link])
        .unwrap();
    let (child, link) = node(2, Some(1), false);
    assert!(
        db.write_initial_batch(generation, &[child, root], &[link])
            .is_err()
    );
    assert_eq!(db.object_count(generation).unwrap(), 1);
    drop(db);
    let mut reopened = Volume::open(&file).unwrap();
    assert!(reopened.complete(generation).is_err());
    assert!(reopened.publish(generation).is_err());
    assert!(reopened.active_generation().unwrap().is_none());
}

#[test]
fn a_disconnected_directory_cycle_is_rejected_as_invalid_structure() {
    let dir = tempfile::tempdir().unwrap();
    let mut db = Volume::create(&dir.path().join("volume.sqlite"), "{}").unwrap();
    let generation = db.begin_generation().unwrap();
    db.begin_initial_paths(generation).unwrap();
    let (root, root_link) = node(1, None, true);
    let (first, first_link) = node(2, Some(3), true);
    let (second, second_link) = node(3, Some(2), true);
    db.write_initial_batch(
        generation,
        &[root, first, second],
        &[root_link, first_link, second_link],
    )
    .unwrap();
    let error = db.finish_initial_paths(generation, &[1]).unwrap_err();
    assert!(!error.is::<InitialPathsIncomplete>());
    assert!(!db.has_path(generation, &[1]).unwrap());
}
