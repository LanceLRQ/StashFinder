use anyhow::{Context, Result, bail, ensure};
use stashfinder_core::{
    Action, Response, SNAPSHOT_EXPIRED,
    assets::{Asset, Candidate, Observation, Snapshot},
    index::{IndexMode, IndexStatus},
};
use stashfinder_windows::ipc;
use std::time::Duration;

pub(super) fn volume_name(volume: &Observation) -> &str {
    volume
        .volume_label
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .or_else(|| {
            volume
                .mounts
                .first()
                .map(|s| s.trim_end_matches(['\\', '/']))
                .filter(|s| !s.is_empty())
        })
        .unwrap_or("未命名分区")
}

pub(super) fn selectable(candidate: &Candidate) -> bool {
    !candidate.registered && candidate.blocked.is_none() && candidate.observation.eligible()
}

fn same_candidate(chosen: &Candidate, current: &Candidate) -> bool {
    selectable(current)
        && chosen.weak == current.weak
        && chosen.observation.candidate_id == current.observation.candidate_id
        && chosen.observation.identity == current.observation.identity
}

pub(super) async fn idle_snapshot(pipe: &str) -> Result<Snapshot> {
    tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            let snapshot = ipc::snapshot(pipe).await?;
            if !snapshot.busy {
                return Ok(snapshot);
            }
            tokio::time::sleep(Duration::from_millis(150)).await;
        }
    })
    .await
    .context("后台仍在处理分区，请刷新查看结果")?
}

pub(super) async fn accepted(pipe: &str, action: Action) -> Result<()> {
    ensure!(
        matches!(ipc::request(pipe, action).await?, Response::Accepted),
        "后台操作响应无效"
    );
    Ok(())
}

pub(super) async fn register(
    pipe: &str,
    version: u64,
    chosen: &Candidate,
    allow_weak: bool,
    auto_index: bool,
) -> Result<String> {
    ensure!(selectable(chosen), "该分区无法登记，请刷新后重新选择");
    ensure!(!chosen.weak || allow_weak, "身份信息不足，需要单独确认");
    let volume = &chosen.observation;
    let submit = async || {
        // Snapshot versions restart with the backend. Check the selected identity before writing.
        let current = idle_snapshot(pipe).await?;
        ensure!(
            !chosen.weak || current.version == version,
            "{SNAPSHOT_EXPIRED}"
        );
        ensure!(
            current.candidates.iter().any(|c| same_candidate(chosen, c)),
            "设备身份或登记状态已变化，请刷新后重新选择"
        );
        accepted(
            pipe,
            Action::Register {
                version: current.version,
                candidate_id: volume.candidate_id.clone(),
                name: volume_name(volume).chars().take(128).collect(),
                allow_weak: chosen.weak && allow_weak,
                auto_index: Some(auto_index),
            },
        )
        .await
    };
    if let Err(error) = submit().await {
        if error.to_string() != SNAPSHOT_EXPIRED {
            return Err(error);
        }
        ensure!(
            !chosen.weak,
            "设备快照已更新，身份信息不足，请刷新后重新选择"
        );
        submit().await?;
    }
    let snapshot = idle_snapshot(pipe).await?;
    if let Some(error) = snapshot.error {
        bail!(error);
    }
    snapshot
        .assets
        .into_iter()
        .map(|a| a.registration)
        .find(|r| {
            r.state == "ready"
                && r.observation.candidate_id == volume.candidate_id
                && r.observation.identity == volume.identity
        })
        .map(|r| r.partition_id)
        .context("尚未确认登记结果，请刷新已登记分区")
}

pub(super) fn start_mode(asset: &Asset, status: &IndexStatus) -> Option<IndexMode> {
    if asset.registration.state != "ready"
        || asset.connection != "online"
        || asset.error.is_some()
        || status.task_id.is_some()
        || !(status.journal_missing
            || matches!(
                status.phase.as_str(),
                "not_indexed" | "failed" | "needs_rebuild" | "needs_validation"
            ))
    {
        return None;
    }
    match asset
        .registration
        .observation
        .identity
        .filesystem
        .as_deref()?
    {
        "NTFS" if !status.journal_missing => Some(IndexMode::Journal),
        "NTFS" | "FAT32" | "exFAT" => Some(IndexMode::ScanOnly),
        _ => None,
    }
}

pub(super) fn index_label(status: &IndexStatus) -> String {
    if status.journal_missing {
        return "缺少 USN 日志，可使用仅扫描索引".into();
    }
    let phase = match status.phase.as_str() {
        "not_indexed" => "尚未索引",
        "loading" => "正在恢复索引状态",
        "queued" => "等待索引",
        "building" => "准备建立索引",
        "enumerating" => "正在读取 NTFS 记录",
        "scanning" => "正在扫描文件",
        "replaying" => "正在补齐文件变更",
        "publishing" => "正在发布索引",
        "syncing" => "正在同步文件变更",
        "ready" => "索引已建立",
        "snapshot" => "扫描索引已建立",
        "waiting" => "等待分区接入或恢复",
        "failed" => "索引未完成",
        "needs_validation" => "索引需要校验",
        "needs_rebuild" => "索引需要重建",
        other => other,
    };
    if let Some(error) = &status.error {
        format!("{phase} · {error}")
    } else if matches!(
        status.phase.as_str(),
        "ready" | "snapshot" | "scanning" | "enumerating"
    ) {
        format!("{phase} · {} 项", status.records)
    } else {
        phase.into()
    }
}

#[cfg(test)]
mod tests;
