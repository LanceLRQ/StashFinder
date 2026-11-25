use crate::visual_cache::{Cache, Key, Progress, Stats};

mod analysis;
pub mod batch;
pub mod ffmpeg;
const THUMBNAIL_SIZE: u32 = 512;
use anyhow::{Result, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};
use stashfinder_core::{
    Action, Response,
    file_operations::{EntryRef, PreparedTarget, ThumbnailTypes, VisualRecord},
};
use stashfinder_windows::file_operations::verify;
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    },
};
use tauri::{AppHandle, Emitter, Manager, State};

#[derive(Clone)]
pub struct Paths {
    root: PathBuf,
    config: PathBuf,
}
impl Paths {
    pub fn new(config: &std::path::Path, local: &std::path::Path) -> Self {
        Self {
            root: local.join("visual-cache"),
            config: config.join("visual-cache.json"),
        }
    }
    fn from_app(app: &AppHandle) -> Result<Self> {
        Ok(Self {
            root: app.path().app_local_data_dir()?.join("visual-cache"),
            config: app.path().app_config_dir()?.join("visual-cache.json"),
        })
    }
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Icon,
    Thumbnail,
}
#[derive(Serialize)]
pub struct Image {
    pub data_url: Option<String>,
    pub source: &'static str,
    pub historical: bool,
    pub warning: Option<String>,
}
impl Image {
    fn png(bytes: &[u8], source: &'static str, historical: bool, warning: Option<String>) -> Self {
        Self {
            data_url: Some(format!("data:image/png;base64,{}", STANDARD.encode(bytes))),
            source,
            historical,
            warning,
        }
    }
    fn unavailable(source: &'static str, historical: bool) -> Self {
        Self {
            data_url: None,
            source,
            historical,
            warning: None,
        }
    }
}
trait Provider: Send + Sync {
    fn type_icon(&self, name: &str, folder: bool, size: u32) -> Result<Vec<u8>>;
    fn file_icon(&self, target: &PreparedTarget, size: u32) -> Result<Vec<u8>>;
    fn thumbnail(
        &self,
        target: &PreparedTarget,
        size: u32,
        cached_only: bool,
    ) -> Result<Option<Vec<u8>>>;
    fn rebuild_thumbnail(&self, target: &PreparedTarget, size: u32) -> Result<Option<Vec<u8>>>;
}
struct WindowsProvider;
impl Provider for WindowsProvider {
    fn type_icon(&self, name: &str, folder: bool, size: u32) -> Result<Vec<u8>> {
        stashfinder_windows::visuals::type_icon(name, folder, size)
    }
    fn file_icon(&self, target: &PreparedTarget, size: u32) -> Result<Vec<u8>> {
        let path = verify(target, false)?;
        let image = stashfinder_windows::visuals::file_icon(&path, size)?;
        verify(target, false)?;
        Ok(image)
    }
    fn rebuild_thumbnail(&self, target: &PreparedTarget, size: u32) -> Result<Option<Vec<u8>>> {
        let path = verify(target, false)?;
        let image = stashfinder_windows::visuals::rebuild_thumbnail(&path, size)?;
        verify(target, false)?;
        Ok(image)
    }
    fn thumbnail(
        &self,
        target: &PreparedTarget,
        size: u32,
        cached_only: bool,
    ) -> Result<Option<Vec<u8>>> {
        let path = verify(target, false)?;
        let image = stashfinder_windows::visuals::thumbnail(&path, size, cached_only)?;
        verify(target, false)?;
        Ok(image)
    }
}
fn file_key(record: &VisualRecord, kind: Kind, size: u32) -> Result<Key> {
    let file = &record.target.file;
    let mut fields = serde_json::json!({
        "version": 1, "kind": kind, "size": size,
        "partition": file.reference.partition_id, "filesystem": file.reference.filesystem_id,
        "entry": file.reference.entry, "object": file.object, "identity": file.identity,
        "path": file.relative, "folder": file.folder, "bytes": file.size, "created": file.created, "modified": file.modified,
    });
    if let Some(version) = file.content_version {
        fields["content_version"] = version.into();
    }
    Ok(Key::new(&serde_json::to_vec(&fields)?))
}
struct Shared {
    batch: batch::Batch,
    cache: Mutex<Option<Cache>>,
    provider: Arc<dyn Provider>,
    epoch: AtomicU64,
    sequence: AtomicU64,
    slots: Arc<tokio::sync::Semaphore>,
    requests: Mutex<HashMap<String, Arc<AtomicBool>>>,
    waiting: AtomicUsize,
    active: AtomicUsize,
}
impl Shared {
    fn new(provider: Arc<dyn Provider>) -> Self {
        Self {
            batch: batch::Batch::default(),
            cache: Mutex::new(None),
            provider,
            epoch: AtomicU64::new(0),
            sequence: AtomicU64::new(0),
            slots: Arc::new(tokio::sync::Semaphore::new(2)),
            requests: Mutex::new(HashMap::new()),
            waiting: AtomicUsize::new(0),
            active: AtomicUsize::new(0),
        }
    }
    fn cached<T>(
        &self,
        paths: &Paths,
        operation: impl FnOnce(&mut Cache) -> Result<T>,
    ) -> Result<T> {
        let mut cache = self.cache.lock().unwrap_or_else(|e| e.into_inner());
        if cache.is_none() {
            *cache = Some(Cache::open(paths.root.clone(), paths.config.clone())?);
        }
        operation(cache.as_mut().unwrap())
    }
    fn store(
        &self,
        paths: &Paths,
        key: Key,
        bytes: &[u8],
        epoch: u64,
        cancel: &AtomicBool,
    ) -> Result<bool> {
        self.cached(paths, |cache| {
            if self.epoch.load(Ordering::Acquire) != epoch || cancel.load(Ordering::Acquire) {
                return Ok(false);
            }
            cache.put(key, bytes)
        })
    }
    fn load_type(&self, name: &str, folder: bool, size: u32, cancel: &AtomicBool) -> Result<Image> {
        ensure!(!cancel.load(Ordering::Acquire), "图片请求已取消");
        let bytes = self.provider.type_icon(name, folder, size)?;
        ensure!(!cancel.load(Ordering::Acquire), "图片请求已取消");
        Ok(Image::png(&bytes, "system_icon", false, None))
    }
    fn load_file(
        &self,
        paths: &Paths,
        record: &VisualRecord,
        kind: Kind,
        size: u32,
        cancel: &AtomicBool,
        notify: &dyn Fn(&str),
    ) -> Result<Image> {
        ensure!(
            (1..=1024).contains(&size) && (!matches!(kind, Kind::Icon) || size <= 256),
            "图片尺寸无效"
        );
        ensure!(!cancel.load(Ordering::Acquire), "图片请求已取消");
        if matches!(kind, Kind::Icon) {
            if !record.online {
                return Ok(Image::unavailable("offline", true));
            }
            let bytes = self.provider.file_icon(&record.target, size)?;
            ensure!(!cancel.load(Ordering::Acquire), "图片请求已取消");
            return Ok(Image::png(&bytes, "system_icon", false, None));
        }
        let epoch = self.epoch.load(Ordering::Acquire);
        let canonical_size = THUMBNAIL_SIZE;
        let key = file_key(record, kind, canonical_size)?;
        notify("local_cache");
        if let Some(bytes) = self.cached(paths, |cache| cache.get(key))? {
            return Ok(Image::png(&bytes, "local_cache", !record.online, None));
        }
        if !record.online {
            if size != canonical_size
                && let Some(bytes) =
                    self.cached(paths, |cache| cache.get(file_key(record, kind, size)?))?
            {
                return Ok(Image::png(&bytes, "local_cache", true, None));
            }
            return Ok(Image::unavailable("offline", true));
        }
        ensure!(!cancel.load(Ordering::Acquire), "图片请求已取消");
        notify("system_cache");
        let (bytes, source) = if record.target.file.content_version.is_some() {
            notify("system_generate");
            (
                self.provider
                    .rebuild_thumbnail(&record.target, canonical_size)?,
                "system_generated",
            )
        } else {
            match self
                .provider
                .thumbnail(&record.target, canonical_size, true)?
            {
                Some(bytes) => (Some(bytes), "system_cache"),
                None => {
                    ensure!(!cancel.load(Ordering::Acquire), "图片请求已取消");
                    notify("system_generate");
                    (
                        self.provider
                            .thumbnail(&record.target, canonical_size, false)?,
                        "system_generated",
                    )
                }
            }
        };
        ensure!(!cancel.load(Ordering::Acquire), "图片请求已取消");
        let Some(bytes) = bytes else {
            return Ok(Image::unavailable("unsupported", false));
        };
        let warning = self
            .store(paths, key, &bytes, epoch, cancel)
            .err()
            .map(|e| format!("{e:#}"));
        Ok(Image::png(&bytes, source, false, warning))
    }
    fn clear(&self, paths: &Paths, progress: impl FnMut(Progress)) -> Result<()> {
        self.batch.cancel();
        self.epoch.fetch_add(1, Ordering::AcqRel);
        for cancel in self
            .requests
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .values()
        {
            cancel.store(true, Ordering::Release);
        }
        self.cached(paths, |cache| cache.clear(progress))
    }
    fn status(&self, paths: &Paths) -> Result<CacheStatus> {
        let (stats, thumbnail_types) = self.cached(paths, |cache| {
            Ok((cache.stats(), cache.thumbnail_types().clone()))
        })?;
        Ok(CacheStatus {
            stats,
            thumbnail_types,
            waiting: self.waiting.load(Ordering::Relaxed),
            active: self.active.load(Ordering::Relaxed),
        })
    }
}
pub struct Visuals(Arc<Shared>);
impl Default for Visuals {
    fn default() -> Self {
        Self(Arc::new(Shared::new(Arc::new(WindowsProvider))))
    }
}
#[derive(Serialize)]
pub struct CacheStatus {
    #[serde(flatten)]
    pub stats: Stats,
    pub thumbnail_types: ThumbnailTypes,
    pub waiting: usize,
    pub active: usize,
}
type Notify = Arc<dyn Fn(&str) + Send + Sync>;
struct Work {
    shared: Arc<Shared>,
    id: String,
    cancel: Arc<AtomicBool>,
    active: bool,
}
impl Work {
    fn new(shared: Arc<Shared>, id: String) -> Result<Self> {
        ensure!(!id.is_empty() && id.len() <= 128, "图片请求标识无效");
        let cancel = Arc::new(AtomicBool::new(false));
        {
            let mut requests = shared.requests.lock().unwrap_or_else(|e| e.into_inner());
            ensure!(
                requests.len() < 128 && !requests.contains_key(&id),
                "图片请求繁忙，请稍后重试"
            );
            requests.insert(id.clone(), cancel.clone());
        }
        shared.waiting.fetch_add(1, Ordering::Relaxed);
        Ok(Self {
            shared,
            id,
            cancel,
            active: false,
        })
    }
    async fn enter(&mut self, notify: &Notify) -> Result<tokio::sync::OwnedSemaphorePermit> {
        notify("waiting");
        let permit = self.shared.slots.clone().acquire_owned().await?;
        self.shared.waiting.fetch_sub(1, Ordering::Relaxed);
        self.shared.active.fetch_add(1, Ordering::Relaxed);
        self.active = true;
        ensure!(!self.cancel.load(Ordering::Acquire), "图片请求已取消");
        Ok(permit)
    }
}
impl Drop for Work {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Release);
        if self.active {
            self.shared.active.fetch_sub(1, Ordering::Relaxed);
        } else {
            self.shared.waiting.fetch_sub(1, Ordering::Relaxed);
        }
        self.shared
            .requests
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&self.id);
    }
}
async fn background<T: Send + 'static>(
    operation: impl FnOnce() -> Result<T> + Send + 'static,
) -> Result<T, String> {
    tokio::task::spawn_blocking(operation)
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| format!("{e:#}"))
}
#[tauri::command]
pub async fn visual_icon(
    state: State<'_, Visuals>,
    name: String,
    folder: bool,
    size: u32,
) -> Result<Image, String> {
    type_icon(&state, name, folder, size).await
}
pub async fn type_icon(
    state: &Visuals,
    name: String,
    folder: bool,
    size: u32,
) -> Result<Image, String> {
    let shared = state.0.clone();
    let id = format!("icon-{}", shared.sequence.fetch_add(1, Ordering::Relaxed));
    let mut work = Work::new(shared.clone(), id).map_err(|e| e.to_string())?;
    let notify: Notify = Arc::new(|_| {});
    let _permit = work.enter(&notify).await.map_err(|e| e.to_string())?;
    let cancel = work.cancel.clone();
    background(move || shared.load_type(&name, folder, size, &cancel)).await
}
#[tauri::command]
pub async fn visual_preview(
    app: AppHandle,
    state: State<'_, Visuals>,
    request_id: String,
    reference: EntryRef,
    kind: Kind,
    size: u32,
) -> Result<Image, String> {
    let paths = Paths::from_app(&app).map_err(|e| e.to_string())?;
    let event_id = request_id.clone();
    let notify: Notify = Arc::new(move |phase| {
        let _ = app.emit(
            "visual-progress",
            serde_json::json!({ "request_id": event_id, "phase": phase }),
        );
    });
    preview(&state, paths, request_id, reference, kind, size, notify).await
}

pub async fn preview(
    state: &Visuals,
    paths: Paths,
    request_id: String,
    reference: EntryRef,
    kind: Kind,
    size: u32,
    notify: Notify,
) -> Result<Image, String> {
    let shared = state.0.clone();
    let mut work = Work::new(shared.clone(), request_id).map_err(|e| e.to_string())?;
    let _permit = work.enter(&notify).await.map_err(|e| e.to_string())?;
    notify("record");
    let response = crate::runtime::request(Action::FileInspect(reference))
        .await
        .map_err(|e| format!("{e:#}"))?;
    let Response::VisualRecord(record) = response else {
        return Err("图片条目响应无效".into());
    };
    let cancel = work.cancel.clone();
    background(move || shared.load_file(&paths, &record, kind, size, &cancel, &*notify)).await
}
#[tauri::command]
pub fn visual_cancel(state: State<'_, Visuals>, request_id: String) {
    cancel(&state, &request_id)
}
pub fn cancel(state: &Visuals, request_id: &str) {
    if let Some(cancel) = state
        .0
        .requests
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(request_id)
    {
        cancel.store(true, Ordering::Release);
    }
}
#[tauri::command]
pub async fn visual_cache_status(
    app: AppHandle,
    state: State<'_, Visuals>,
) -> Result<CacheStatus, String> {
    let paths = Paths::from_app(&app).map_err(|e| e.to_string())?;
    cache_status(&state, paths).await
}
pub async fn cache_status(state: &Visuals, paths: Paths) -> Result<CacheStatus, String> {
    let shared = state.0.clone();
    background(move || shared.status(&paths)).await
}
#[tauri::command]
pub async fn visual_cache_limit(
    app: AppHandle,
    state: State<'_, Visuals>,
    limit_bytes: Option<u64>,
    request_id: String,
) -> Result<CacheStatus, String> {
    let paths = Paths::from_app(&app).map_err(|e| e.to_string())?;
    let progress = maintenance_progress(app, request_id);
    cache_limit(&state, paths, limit_bytes, progress).await
}
pub async fn cache_limit(
    state: &Visuals,
    paths: Paths,
    limit_bytes: Option<u64>,
    progress: impl FnMut(Progress) + Send + 'static,
) -> Result<CacheStatus, String> {
    let shared = state.0.clone();
    background(move || {
        shared.cached(&paths, |cache| cache.set_limit(limit_bytes, progress))?;
        shared.status(&paths)
    })
    .await
}

#[tauri::command]
pub async fn visual_thumbnail_types(
    app: AppHandle,
    state: State<'_, Visuals>,
    types: ThumbnailTypes,
) -> Result<CacheStatus, String> {
    let paths = Paths::from_app(&app).map_err(|e| e.to_string())?;
    thumbnail_types(&state, paths, types).await
}
pub async fn thumbnail_types(
    state: &Visuals,
    paths: Paths,
    types: ThumbnailTypes,
) -> Result<CacheStatus, String> {
    let shared = state.0.clone();
    background(move || {
        shared.cached(&paths, |cache| cache.set_thumbnail_types(types))?;
        shared.status(&paths)
    })
    .await
}
#[tauri::command]
pub async fn visual_cache_clear(
    app: AppHandle,
    state: State<'_, Visuals>,
    request_id: String,
) -> Result<CacheStatus, String> {
    let paths = Paths::from_app(&app).map_err(|e| e.to_string())?;
    let progress = maintenance_progress(app, request_id);
    cache_clear(&state, paths, progress).await
}
pub async fn cache_clear(
    state: &Visuals,
    paths: Paths,
    progress: impl FnMut(Progress) + Send + 'static,
) -> Result<CacheStatus, String> {
    let shared = state.0.clone();
    background(move || {
        shared.clear(&paths, progress)?;
        shared.status(&paths)
    })
    .await
}

fn maintenance_progress(app: AppHandle, request_id: String) -> impl FnMut(Progress) {
    let mut last = std::time::Instant::now();
    move |progress| {
        if progress.processed == 0
            || progress.processed == progress.total
            || last.elapsed() >= std::time::Duration::from_millis(100)
        {
            last = std::time::Instant::now();
            let _ = app.emit(
                "visual-cache-progress",
                serde_json::json!({ "request_id": request_id, "progress": progress }),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use stashfinder_core::{
        assets::{Identity, Layout},
        file_operations::*,
    };

    #[derive(Default)]
    struct Fake {
        calls: Mutex<Vec<bool>>,
        on_extract: Mutex<Option<Box<dyn Fn() + Send + Sync>>>,
        fail: AtomicBool,
    }
    impl Provider for Fake {
        fn rebuild_thumbnail(
            &self,
            _target: &PreparedTarget,
            _size: u32,
        ) -> Result<Option<Vec<u8>>> {
            ensure!(
                !self.fail.load(Ordering::Acquire),
                "native extraction failed"
            );
            Ok(Some(b"rebuilt".to_vec()))
        }
        fn type_icon(&self, _name: &str, _folder: bool, _size: u32) -> Result<Vec<u8>> {
            Ok(b"type".to_vec())
        }
        fn file_icon(&self, _target: &PreparedTarget, _size: u32) -> Result<Vec<u8>> {
            Ok(b"file".to_vec())
        }
        fn thumbnail(
            &self,
            _target: &PreparedTarget,
            _size: u32,
            cached_only: bool,
        ) -> Result<Option<Vec<u8>>> {
            self.calls.lock().unwrap().push(cached_only);
            ensure!(
                !self.fail.load(Ordering::Acquire),
                "native extraction failed"
            );
            if cached_only {
                return Ok(None);
            }
            if let Some(callback) = self.on_extract.lock().unwrap().as_ref() {
                callback();
            }
            Ok(Some(b"generated".to_vec()))
        }
    }
    pub(super) fn record() -> VisualRecord {
        VisualRecord {
            online: true,
            target: PreparedTarget {
                file: ResolvedEntry {
                    content_version: None,
                    reference: EntryRef {
                        partition_id: "p".into(),
                        filesystem_id: "fs".into(),
                        generation: "1".into(),
                        revision: "1".into(),
                        entry: vec![2],
                    },
                    relative: "image.png".encode_utf16().collect(),
                    object: vec![2],
                    parent: Some(vec![1]),
                    folder: false,
                    size: Some(10),
                    created: Some(100),
                    modified: Some(200),
                    identity: Some(ExpectedIdentity::Native(NativeIdentity {
                        volume: 1,
                        file: 2,
                    })),
                },
                mounts: vec!["E:\\".into()],
                volume: "volume".into(),
                partition_name: "Photos".into(),
                identity: Identity {
                    layout: Layout::Gpt {
                        disk: "d".into(),
                        partition: "p".into(),
                    },
                    filesystem: Some("NTFS".into()),
                    serial: None,
                },
            },
        }
    }
    pub(super) fn paths(temp: &tempfile::TempDir) -> Paths {
        Paths {
            root: temp.path().join("cache"),
            config: temp.path().join("config.json"),
        }
    }

    #[test]
    fn system_icons_do_not_create_a_disk_cache() {
        let temp = tempfile::tempdir().unwrap();
        let paths = paths(&temp);
        let shared = Shared::new(Arc::new(Fake::default()));
        let cancel = AtomicBool::new(false);
        let type_icon = shared.load_type("movie.mp4", false, 128, &cancel).unwrap();
        assert_eq!(type_icon.source, "system_icon");
        let icon = shared
            .load_file(&paths, &record(), Kind::Icon, 128, &cancel, &|_| {})
            .unwrap();
        assert_eq!(icon.source, "system_icon");
        assert!(!paths.root.exists());
    }

    #[test]
    fn offline_images_use_only_local_cache_and_keys_ignore_mount_and_index_versions() {
        let temp = tempfile::tempdir().unwrap();
        let paths = paths(&temp);
        let provider = Arc::new(Fake::default());
        let shared = Shared::new(provider.clone());
        let cancel = AtomicBool::new(false);
        let mut row = record();
        let image = shared
            .load_file(&paths, &row, Kind::Thumbnail, 256, &cancel, &|_| {})
            .unwrap();
        assert_eq!(image.source, "system_generated");
        assert_eq!(*provider.calls.lock().unwrap(), vec![true, false]);
        let key = file_key(&row, Kind::Thumbnail, 256).unwrap();
        row.target.mounts = vec!["Z:\\".into()];
        row.target.file.reference.generation = "2".into();
        row.target.file.reference.revision = "30".into();
        assert_eq!(file_key(&row, Kind::Thumbnail, 256).unwrap(), key);
        row.online = false;
        let image = shared
            .load_file(&paths, &row, Kind::Thumbnail, 256, &cancel, &|_| {})
            .unwrap();
        assert_eq!(image.source, "local_cache");
        assert!(image.historical);
        assert_eq!(provider.calls.lock().unwrap().len(), 2);
        row.target.file.modified = Some(201);
        assert_ne!(file_key(&row, Kind::Thumbnail, 256).unwrap(), key);
        assert!(
            shared
                .load_file(&paths, &row, Kind::Thumbnail, 256, &cancel, &|_| {})
                .unwrap()
                .data_url
                .is_none()
        );
        assert_eq!(provider.calls.lock().unwrap().len(), 2);
    }

    #[test]
    fn restarted_cache_serves_the_same_thumbnail_to_grid_and_preview_offline() {
        let temp = tempfile::tempdir().unwrap();
        let paths = paths(&temp);
        let provider = Arc::new(Fake::default());
        let cancel = AtomicBool::new(false);
        let mut row = record();
        let image = Shared::new(provider.clone())
            .load_file(&paths, &row, Kind::Thumbnail, 32, &cancel, &|_| {})
            .unwrap();
        assert_eq!(image.source, "system_generated");
        row.online = false;
        row.target.mounts.clear();
        let restarted = Shared::new(provider.clone());
        for size in [32, 64, 128, 512] {
            let offline = restarted
                .load_file(&paths, &row, Kind::Thumbnail, size, &cancel, &|_| {})
                .unwrap();
            assert_eq!(offline.data_url, image.data_url);
            assert!(offline.historical);
        }
        assert_eq!(restarted.status(&paths).unwrap().stats.files, 1);
        assert_eq!(provider.calls.lock().unwrap().len(), 2);
    }

    #[test]
    fn clearing_during_extraction_prevents_the_old_request_from_refilling_cache() {
        let temp = tempfile::tempdir().unwrap();
        let paths = paths(&temp);
        let provider = Arc::new(Fake::default());
        let shared = Arc::new(Shared::new(provider.clone()));
        let weak = Arc::downgrade(&shared);
        let clear_paths = paths.clone();
        *provider.on_extract.lock().unwrap() = Some(Box::new(move || {
            weak.upgrade().unwrap().clear(&clear_paths, |_| {}).unwrap()
        }));
        shared
            .load_file(
                &paths,
                &record(),
                Kind::Thumbnail,
                256,
                &AtomicBool::new(false),
                &|_| {},
            )
            .unwrap();
        assert_eq!(
            shared
                .cached(&paths, |cache| Ok(cache.stats().files))
                .unwrap(),
            0
        );
    }

    #[test]
    fn failed_and_cancelled_requests_do_not_leave_cached_images() {
        let temp = tempfile::tempdir().unwrap();
        let paths = paths(&temp);
        let provider = Arc::new(Fake::default());
        provider.fail.store(true, Ordering::Release);
        let shared = Shared::new(provider.clone());
        assert!(
            shared
                .load_file(
                    &paths,
                    &record(),
                    Kind::Thumbnail,
                    256,
                    &AtomicBool::new(false),
                    &|_| {}
                )
                .is_err()
        );
        assert_eq!(shared.cached(&paths, |c| Ok(c.stats().files)).unwrap(), 0);
        let calls = provider.calls.lock().unwrap().len();
        assert!(
            shared
                .load_file(
                    &paths,
                    &record(),
                    Kind::Thumbnail,
                    256,
                    &AtomicBool::new(true),
                    &|_| {}
                )
                .is_err()
        );
        assert_eq!(provider.calls.lock().unwrap().len(), calls);
    }

    #[tokio::test]
    async fn cancelled_waiting_requests_release_their_queue_slot() {
        let shared = Arc::new(Shared::new(Arc::new(Fake::default())));
        let permits = shared.slots.clone().acquire_many_owned(2).await.unwrap();
        let worker = shared.clone();
        let pending = tokio::spawn(async move {
            let mut work = Work::new(worker, "waiting".into()).unwrap();
            let notify: Notify = Arc::new(|_| {});
            work.enter(&notify).await.map(|_| ())
        });
        tokio::task::yield_now().await;
        assert_eq!(shared.waiting.load(Ordering::Relaxed), 1);
        shared
            .requests
            .lock()
            .unwrap()
            .get("waiting")
            .unwrap()
            .store(true, Ordering::Release);
        drop(permits);
        assert!(pending.await.unwrap().is_err());
        assert_eq!(shared.waiting.load(Ordering::Relaxed), 0);
        assert_eq!(shared.active.load(Ordering::Relaxed), 0);
        assert!(shared.requests.lock().unwrap().is_empty());
    }
}
