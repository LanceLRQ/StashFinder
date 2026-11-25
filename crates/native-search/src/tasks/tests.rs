use super::*;
use stashfinder_core::{
    PROTOCOL_VERSION, Reply, Request,
    assets::{Asset, Identity, Layout, Observation, Registration},
    index::IndexMode,
    read_frame,
    tasks::TaskOverviewPage,
    write_frame,
};
use stashfinder_windows::ipc::{create_pipe, current_user_sid};

fn task(id: &str, state: &str) -> Task {
    Task {
        id: id.into(),
        partition_id: "part".into(),
        kind: "scan".into(),
        mode: IndexMode::ScanOnly,
        automatic: false,
        state: state.into(),
        created: 1,
        updated: 2,
        error: None,
        phase: None,
    }
}

fn partitions(connection: &str) -> Snapshot {
    let observation = Observation {
        candidate_id: "v".into(),
        disk_number: None,
        model: None,
        disk_bytes: None,
        partition_bytes: None,
        volume_label: None,
        mounts: vec![],
        capacity: None,
        error: None,
        identity: Identity {
            layout: Layout::Unknown,
            filesystem: Some("exFAT".into()),
            serial: None,
        },
    };
    Snapshot {
        assets: vec![Asset {
            connection: connection.into(),
            error: None,
            mounts: vec![],
            capacity_error: None,
            registration: Registration {
                disk_id: "d".into(),
                partition_id: "part".into(),
                filesystem_id: "f".into(),
                name: "测试盘".into(),
                revision: 1,
                state: "ready".into(),
                observation,
                last_seen: None,
            },
        }],
        ..Default::default()
    }
}

#[test]
fn actions_follow_the_selected_task_id_across_reordering_and_states() {
    let mut running = task("current", "running");
    running.phase = Some("scanning".into());
    let cancelled = task("previous", "cancelled");
    let list = TaskList::new(vec![cancelled.clone(), running], partitions("online"));
    assert_eq!(list.cell(0, 0), "测试盘");
    assert_eq!(list.cell(0, 3), "正在扫描文件");
    assert!(
        matches!(list.action(0, Operation::Cancel), Some(Action::TaskCancel { id }) if id == "current")
    );
    assert!(list.action(1, Operation::Retry).is_none());
    let mut list = TaskList::new(vec![cancelled], partitions("online"));
    assert!(
        matches!(list.action(0, Operation::Retry), Some(Action::TaskRetry { id }) if id == "previous")
    );
    assert!(list.action(0, Operation::Cancel).is_none());
    list.stale = true;
    assert!(list.action(0, Operation::Retry).is_none());
}

#[test]
fn offline_retries_and_confirmation_tasks_cannot_be_submitted_as_generic_retries() {
    for state in ["failed", "interrupted", "cancelled"] {
        let list = TaskList::new(vec![task("t", state)], partitions("offline"));
        assert!(list.action(0, Operation::Retry).is_none());
        assert!(list.detail(0).contains("接入并确认分区"));
        let mut unavailable = partitions("online");
        unavailable.error = Some("设备读取失败".into());
        assert!(
            TaskList::new(vec![task("t", state)], unavailable)
                .action(0, Operation::Retry)
                .is_none()
        );
        assert!(
            TaskList::new(vec![task("t", state)], partitions("online"))
                .action(0, Operation::Retry)
                .is_some()
        );
    }
    let mut confirmation = task("t", "needs_confirmation");
    confirmation.automatic = true;
    confirmation.error = Some("等待授权".into());
    let mut normal = task("auto", "completed");
    normal.automatic = true;
    let list = TaskList::new(vec![confirmation, normal], partitions("online"));
    assert_eq!(list.rows.len(), 1);
    assert!(list.action(0, Operation::Retry).is_none());
    assert!(list.action(0, Operation::Cancel).is_none());
    assert!(list.detail(0).contains("等待授权"));
}

#[tokio::test]
async fn changing_paginated_snapshot_discards_all_rows_from_the_old_version() {
    let pipe = format!(r"\\.\pipe\StashFinder.native-tasks-{}", std::process::id());
    let address = pipe.clone();
    let owner = current_user_sid().unwrap();
    let mut server = create_pipe(&pipe, &owner, true).unwrap();
    let backend = tokio::spawn(async move {
        for (step, (version, offset)) in [(None, 0), (Some("old"), 1), (None, 0), (Some("new"), 1)]
            .into_iter()
            .enumerate()
        {
            server.connect().await.unwrap();
            let mut client = server;
            server = create_pipe(&address, &owner, false).unwrap();
            let wire: Request =
                serde_json::from_slice(&read_frame(&mut client).await.unwrap()).unwrap();
            assert!(
                matches!(wire.action, Action::TaskOverview { version: v, offset: n } if v.as_deref() == version && n == offset)
            );
            let body = if step == 1 {
                Response::Error("任务快照已更新，请重试".into())
            } else {
                let id = ["discard", "", "running", "done"][step];
                Response::TaskOverview(Box::new(TaskOverviewPage {
                    version: if step == 0 { "old" } else { "new" }.into(),
                    rows: vec![TaskOverviewRow::Task(task(id, "queued"))],
                    next: (offset == 0).then_some(1),
                    automatic: true,
                }))
            };
            let reply = Reply {
                id: wire.id,
                version: PROTOCOL_VERSION,
                body,
            };
            write_frame(&mut client, &serde_json::to_vec(&reply).unwrap())
                .await
                .unwrap();
        }
    });
    let rows = read_tasks(&pipe).await.unwrap();
    assert_eq!(
        rows.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
        ["running", "done"]
    );
    backend.await.unwrap();
}
