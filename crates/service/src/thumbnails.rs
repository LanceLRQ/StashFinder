use anyhow::{Context, Result, ensure};
use stashfinder_core::{
    Response,
    assets::Snapshot,
    file_operations::{ThumbnailPage, ThumbnailScope},
};
use stashfinder_store::file_operations::ThumbnailPlan;
use std::{
    path::PathBuf,
    sync::{
        Arc, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
};

pub(crate) struct ThumbnailJob {
    partition_id: String,
    filesystem_id: String,
    plan: Arc<OnceLock<Result<ThumbnailPlan, String>>>,
    cancelled: Arc<AtomicBool>,
}

impl ThumbnailJob {
    pub(crate) fn start(path: PathBuf, scope: ThumbnailScope) -> Result<Self> {
        let job = Self {
            partition_id: scope.partition_id.clone(),
            filesystem_id: scope.filesystem_id.clone(),
            plan: Arc::default(),
            cancelled: Arc::default(),
        };
        let (plan, cancel) = (job.plan.clone(), job.cancelled.clone());
        std::thread::Builder::new()
            .name("thumbnail-count".into())
            .spawn(move || {
                let result = ThumbnailPlan::build(&path, &scope, &cancel);
                let _ = plan.set(result.map_err(|e| format!("{e:#}")));
            })?;
        Ok(job)
    }

    pub(crate) fn page(&self, offset: Option<usize>, snapshot: &Snapshot) -> Result<Response> {
        let Some(plan) = self.plan.get() else {
            return Ok(Response::ThumbnailCounting);
        };
        let plan = plan.as_ref().map_err(|e| anyhow::anyhow!("{e}"))?;
        let Some(offset) = offset else {
            return Ok(Response::ThumbnailCount(plan.total()));
        };
        let asset = snapshot
            .assets
            .iter()
            .find(|a| {
                a.registration.partition_id == self.partition_id
                    && a.registration.filesystem_id == self.filesystem_id
            })
            .context("分区登记已变化，请重新开始")?;
        ensure!(
            asset.registration.state == "ready",
            "分区登记已变化，请重新开始"
        );
        if asset.connection != "online" || asset.error.is_some() || snapshot.error.is_some() {
            return Ok(Response::AnalysisOffline);
        }
        let batch = plan.page(offset)?;
        Ok(Response::ThumbnailPage(Box::new(ThumbnailPage {
            analyse_indices: batch.analyse_indices,
            files: batch.files,
            unavailable: batch.unavailable,
            mounts: asset.mounts.clone(),
            volume: asset.registration.observation.candidate_id.clone(),
            identity: asset.registration.observation.identity.clone(),
            partition_name: asset.registration.name.clone(),
        })))
    }
}

impl Drop for ThumbnailJob {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }
}
