use super::*;
use stashfinder_core::{
    PROTOCOL_VERSION, Page, Reply, Request,
    assets::{Identity, Layout, Observation, Registration},
    read_frame, write_frame,
};
use stashfinder_windows::ipc::{create_pipe, current_user_sid};

fn candidate() -> Candidate {
    Candidate {
        observation: Observation {
            candidate_id: "volume-path".into(),
            disk_number: Some(4),
            model: None,
            disk_bytes: None,
            partition_bytes: None,
            volume_label: Some(" 资料 ".into()),
            mounts: vec![r"R:\".into()],
            identity: Identity {
                layout: Layout::Gpt {
                    disk: "disk".into(),
                    partition: "part".into(),
                },
                filesystem: Some("NTFS".into()),
                serial: None,
            },
            capacity: None,
            error: None,
        },
        weak: false,
        blocked: None,
        registered: false,
    }
}

fn ready_asset(c: &Candidate) -> Asset {
    Asset {
        registration: Registration {
            disk_id: "disk".into(),
            partition_id: "new-part".into(),
            filesystem_id: "fs".into(),
            name: "资料".into(),
            revision: 1,
            state: "ready".into(),
            observation: c.observation.clone(),
            last_seen: None,
        },
        connection: "online".into(),
        mounts: c.observation.mounts.clone(),
        capacity_error: None,
        error: None,
    }
}

#[test]
fn reselecting_checks_identity_even_if_the_mount_and_candidate_id_are_reused() {
    let selected = candidate();
    let mut current = selected.clone();
    assert!(same_candidate(&selected, &current));
    current.observation.identity.layout = Layout::Unknown;
    assert!(!same_candidate(&selected, &current));
    current = selected.clone();
    current.registered = true;
    assert!(!same_candidate(&selected, &current));
    current.registered = false;
    current.blocked = Some("身份冲突".into());
    assert!(!same_candidate(&selected, &current));
}

#[test]
fn initial_index_uses_the_existing_journal_or_scan_only_for_an_eligible_partition() {
    let mut asset = ready_asset(&candidate());
    let mut status = IndexStatus {
        phase: "not_indexed".into(),
        ..Default::default()
    };
    assert!(matches!(
        start_mode(&asset, &status),
        Some(IndexMode::Journal)
    ));
    status.journal_missing = true;
    assert!(matches!(
        start_mode(&asset, &status),
        Some(IndexMode::ScanOnly)
    ));
    status.journal_missing = false;
    asset.registration.observation.identity.filesystem = Some("exFAT".into());
    assert!(matches!(
        start_mode(&asset, &status),
        Some(IndexMode::ScanOnly)
    ));
    for phase in ["queued", "scanning", "snapshot", "ready"] {
        status.phase = phase.into();
        assert!(start_mode(&asset, &status).is_none());
    }
    status.phase = "failed".into();
    asset.connection = "offline".into();
    assert!(start_mode(&asset, &status).is_none());
}

fn backend(
    name: &str,
    steps: u64,
    reply: impl Fn(u64, Action) -> Response + Send + 'static,
) -> (String, tokio::task::JoinHandle<()>) {
    let pipe = format!(
        r"\\.\pipe\StashFinder.register-{name}-{}",
        std::process::id()
    );
    let owner = current_user_sid().unwrap();
    let mut server = create_pipe(&pipe, &owner, true).unwrap();
    let endpoint = pipe.clone();
    let task = tokio::spawn(async move {
        for step in 0..steps {
            server.connect().await.unwrap();
            let mut client = server;
            server = create_pipe(&endpoint, &owner, false).unwrap();
            let wire: Request =
                serde_json::from_slice(&read_frame(&mut client).await.unwrap()).unwrap();
            let reply = Reply {
                version: PROTOCOL_VERSION,
                id: wire.id,
                body: reply(step, wire.action),
            };
            write_frame(&mut client, &serde_json::to_vec(&reply).unwrap())
                .await
                .unwrap();
        }
    });
    (pipe, task)
}

fn candidate_page(c: &Candidate, version: u64) -> Response {
    Response::Page(Box::new(Page {
        version,
        busy: false,
        error: None,
        asset: None,
        candidate: Some(c.clone()),
        next: None,
    }))
}

#[tokio::test]
async fn restarting_the_backend_cannot_apply_an_old_selection_to_a_reused_snapshot_version() {
    let chosen = candidate();
    let mut replacement = chosen.clone();
    replacement.observation.identity.filesystem = Some("exFAT".into());
    let (pipe, task) = backend("restart", 1, move |_, action| {
        if matches!(action, Action::Page { .. }) {
            candidate_page(&replacement, 7)
        } else {
            Response::Error("登记请求指向了已替换的分区".into())
        }
    });
    let error = register(&pipe, 7, &chosen, false, true)
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("设备身份或登记状态已变化"), "{error}");
    task.await.unwrap();
}

#[tokio::test]
async fn registration_confirms_the_published_partition_after_acceptance_and_busy_snapshot() {
    let chosen = candidate();
    let asset = ready_asset(&chosen);
    let current = chosen.clone();
    let (pipe, backend) = backend("finish", 4, move |step, action| {
        if step == 0 {
            assert!(matches!(action, Action::Page { offset: 0, .. }));
            return candidate_page(&current, 7);
        }
        if step == 1 {
            let Action::Register {
                version,
                candidate_id,
                name,
                allow_weak,
                auto_index,
            } = action
            else {
                panic!()
            };
            assert_eq!(
                (version, candidate_id.as_str(), name.as_str()),
                (7, "volume-path", "资料")
            );
            assert!(!allow_weak);
            assert_eq!(auto_index, Some(false));
            Response::Accepted
        } else {
            assert!(matches!(action, Action::Page { offset: 0, .. }));
            Response::Page(Box::new(Page {
                version: 6 + step,
                busy: step == 2,
                error: None,
                asset: (step == 3).then(|| asset.clone()),
                candidate: None,
                next: None,
            }))
        }
    });
    let id = register(&pipe, 7, &chosen, false, false).await.unwrap();
    assert_eq!(id, "new-part");
    backend.await.unwrap();
}

#[tokio::test]
async fn expired_weak_confirmation_requires_reselection_and_is_never_replayed() {
    let mut weak = candidate();
    weak.weak = true;
    let current = weak.clone();
    let (pipe, task) = backend("weak", 2, move |step, action| {
        if step == 0 {
            assert!(matches!(action, Action::Page { offset: 0, .. }));
            return candidate_page(&current, 7);
        }
        assert!(matches!(
            action,
            Action::Register {
                allow_weak: true,
                auto_index: Some(true),
                ..
            }
        ));
        Response::Error(SNAPSHOT_EXPIRED.into())
    });
    assert!(
        register(&pipe, 7, &weak, false, true)
            .await
            .unwrap_err()
            .to_string()
            .contains("单独确认")
    );
    assert!(
        register(&pipe, 7, &weak, true, true)
            .await
            .unwrap_err()
            .to_string()
            .contains("重新选择")
    );
    task.await.unwrap();
}

#[tokio::test]
async fn expired_snapshot_retries_only_the_same_volume_and_reports_the_service_write_error() {
    let chosen = candidate();
    let current = chosen.clone();
    let (pipe, task) = backend("expired", 5, move |step, action| match step {
        0 | 2 => {
            assert!(matches!(action, Action::Page { offset: 0, .. }));
            candidate_page(&current, 7 + step)
        }
        1 | 3 => {
            let Action::Register {
                version,
                candidate_id,
                allow_weak: false,
                auto_index: Some(true),
                ..
            } = action
            else {
                panic!()
            };
            assert_eq!(version, if step == 1 { 7 } else { 9 });
            assert_eq!(candidate_id, current.observation.candidate_id);
            if step == 1 {
                Response::Error(SNAPSHOT_EXPIRED.into())
            } else {
                Response::Accepted
            }
        }
        _ => {
            assert!(matches!(action, Action::Page { offset: 0, .. }));
            Response::Page(Box::new(Page {
                version: 9 + step / 2,
                busy: false,
                asset: None,
                next: None,
                candidate: Some(current.clone()),
                error: Some("分区已断开".into()),
            }))
        }
    });
    assert_eq!(
        register(&pipe, 7, &chosen, false, true)
            .await
            .unwrap_err()
            .to_string(),
        "分区已断开"
    );
    task.await.unwrap();
}
