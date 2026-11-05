use stashfinder_core::index::IndexMode;
use stashfinder_store::tasks::Tasks;

#[test]
fn queued_tasks_and_policy_survive_restart_and_cancellation_targets_one_id() {
    let dir = tempfile::tempdir().unwrap();
    let mut tasks = Tasks::open(dir.path()).unwrap();
    assert!(tasks.enabled().unwrap());
    let first = tasks
        .enqueue("one", "build", IndexMode::Journal, false)
        .unwrap();
    let second = tasks
        .enqueue("two", "build", IndexMode::Journal, false)
        .unwrap();
    assert_eq!(
        tasks
            .enqueue("one", "build", IndexMode::Journal, false)
            .unwrap()
            .id,
        first.id
    );
    tasks.cancel(&first.id).unwrap();
    assert_eq!(tasks.get(&first.id).unwrap().state, "cancelled");
    assert_eq!(tasks.get(&second.id).unwrap().state, "queued");
    assert!(tasks.paused("one").unwrap());
    tasks.set_enabled(false).unwrap();
    drop(tasks);
    let tasks = Tasks::open(dir.path()).unwrap();
    assert!(!tasks.enabled().unwrap());
    assert!(tasks.paused("one").unwrap());
    assert_eq!(tasks.get(&second.id).unwrap().state, "queued");
    assert!(tasks.get("unknown").is_err());
}

#[test]
fn interrupted_tasks_are_explicit_and_history_is_bounded() {
    let dir = tempfile::tempdir().unwrap();
    let mut tasks = Tasks::open(dir.path()).unwrap();
    let first = tasks
        .enqueue("one", "sync", IndexMode::Journal, true)
        .unwrap();
    tasks.set_state(&first.id, "running", None).unwrap();
    drop(tasks);
    let mut tasks = Tasks::open(dir.path()).unwrap();
    tasks.recover().unwrap();
    assert_eq!(tasks.get(&first.id).unwrap().state, "interrupted");
    for _ in 0..220 {
        let row = tasks
            .enqueue("two", "sync", IndexMode::Journal, true)
            .unwrap();
        tasks.set_state(&row.id, "completed", None).unwrap();
    }
    let rows = tasks.list().unwrap();
    assert_eq!(rows.len(), 201);
    assert!(rows.iter().any(|r| r.id == first.id));
}

#[test]
fn journal_creation_consent_is_consumed_when_the_task_starts() {
    let dir = tempfile::tempdir().unwrap();
    let mut tasks = Tasks::open(dir.path()).unwrap();
    let task = tasks
        .enqueue("one", "build", IndexMode::CreateJournal, false)
        .unwrap();
    tasks.set_state(&task.id, "running", None).unwrap();
    tasks.set_state(&task.id, "waiting", None).unwrap();
    assert!(matches!(
        tasks.get(&task.id).unwrap().mode,
        IndexMode::Journal
    ));
}

#[test]
fn failed_policy_write_preserves_old_setting_and_invalidates_stale_pages_only_on_commit() {
    let dir = tempfile::tempdir().unwrap();
    let tasks = Tasks::open(dir.path()).unwrap();
    let revision = tasks.revision().unwrap();
    let other = rusqlite::Connection::open(dir.path().join("tasks.sqlite")).unwrap();
    other.execute_batch("BEGIN IMMEDIATE").unwrap();
    assert!(tasks.set_enabled(false).is_err());
    other.execute_batch("ROLLBACK").unwrap();
    assert!(tasks.enabled().unwrap());
    assert_eq!(tasks.revision().unwrap(), revision);
    tasks.set_enabled(false).unwrap();
    assert!(!tasks.enabled().unwrap());
    assert!(tasks.revision().unwrap() > revision);
}

#[test]
fn scan_mode_survives_task_history_pruning_and_restart() {
    let dir = tempfile::tempdir().unwrap();
    let mut tasks = Tasks::open(dir.path()).unwrap();
    tasks.set_scan_mode("one", true).unwrap();
    for _ in 0..201 {
        let task = tasks
            .enqueue("one", "scan", IndexMode::ScanOnly, true)
            .unwrap();
        tasks.set_state(&task.id, "completed", None).unwrap();
    }
    drop(tasks);
    let tasks = Tasks::open(dir.path()).unwrap();
    assert!(tasks.scan_mode("one").unwrap());
    tasks.set_scan_mode("one", false).unwrap();
    assert!(!tasks.scan_mode("one").unwrap());
}
