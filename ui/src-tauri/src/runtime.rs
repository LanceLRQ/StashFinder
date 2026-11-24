use anyhow::{Context, Result, ensure};
pub use stashfinder_windows::portable::show_error;
use stashfinder_windows::{
    ipc,
    portable::{
        ClientKind, Layout, Process, confirm_index_backend, is_elevated, session::PortableSession,
    },
};
use std::{
    fs::File,
    sync::{
        OnceLock,
        atomic::{AtomicBool, Ordering},
    },
};
use tauri::{
    Manager,
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    utils::config::{AppDirectoriesOverride, AppDirectoryOverrides, Config},
};

static PORTABLE: OnceLock<Option<Layout>> = OnceLock::new();
static INSTALLED: OnceLock<bool> = OnceLock::new();
static WINDOWS_SERVICE: OnceLock<bool> = OnceLock::new();
static BACKEND_DEFERRED: AtomicBool = AtomicBool::new(false);
pub fn pipe() -> &'static str {
    PORTABLE
        .get()
        .and_then(Option::as_ref)
        .map(|layout| layout.pipe.as_str())
        .unwrap_or_else(|| {
            if installed() {
                stashfinder_windows::scm::INSTALLED_PIPE
            } else {
                ipc::DEFAULT_PIPE
            }
        })
}
fn installed() -> bool {
    INSTALLED.get().copied().unwrap_or(false)
}
fn windows_service() -> bool {
    WINDOWS_SERVICE.get().copied().unwrap_or(false)
}

fn ensure_backend_requested() -> Result<()> {
    ensure!(
        !BACKEND_DEFERRED.load(Ordering::Relaxed),
        "本次未启动索引后台。退出并重新打开 StashFinder 后可选择启动。"
    );
    Ok(())
}

pub async fn status() -> Result<stashfinder_core::Status> {
    ensure_backend_requested()?;
    if windows_service() {
        stashfinder_windows::scm::installed_status()
            .await
            .map_err(connection_error)
    } else {
        ipc::status(pipe()).await.map_err(connection_error)
    }
}

pub async fn snapshot() -> Result<stashfinder_core::assets::Snapshot> {
    ensure_backend_requested()?;
    if windows_service() {
        status().await?;
    }
    ipc::snapshot(pipe()).await.map_err(connection_error)
}

pub async fn request(action: stashfinder_core::Action) -> Result<stashfinder_core::Response> {
    ensure_backend_requested()?;
    if windows_service() {
        status().await?;
    }
    ipc::request(pipe(), action).await.map_err(connection_error)
}

fn connection_error(error: anyhow::Error) -> anyhow::Error {
    if error
        .downcast_ref::<tokio::time::error::Elapsed>()
        .is_none()
    {
        return error;
    }
    if windows_service() {
        error.context("Windows 后台服务已报告运行，但索引连接尚未就绪，请在设置中重试")
    } else if installed() {
        error.context("桌面管理的索引后台未响应，请退出并重新打开 StashFinder")
    } else if active() {
        error.context("便携后台未响应，请退出并重新启动 StashFinder")
    } else {
        error.context("未连接开发后台。请在仓库根目录运行 pnpm dev，或打开已安装版或完整便携包")
    }
}
pub fn active() -> bool {
    PORTABLE.get().is_some_and(Option::is_some)
}
pub fn configure(config: &mut Config, layout: Option<&Layout>) {
    if let Some(layout) = layout {
        // WebView2 uses Tauri's app-local default. WindowConfig::data_directory
        // resolves relative paths against OS local data, bypassing this override.
        config.app.app_directories_override =
            Some(AppDirectoriesOverride::Directories(AppDirectoryOverrides {
                config: Some(layout.config.clone()),
                data: Some(layout.local.clone()),
                local_data: Some(layout.local.clone()),
                cache: Some(layout.local.join("cache")),
                log: Some(layout.local.join("logs")),
            }));
    }
}

pub struct Session {
    _backend: Option<Backend>,
    _portable: Option<PortableSession>,
    _lock: Option<File>,
}
impl Session {
    fn defer(lock: Option<File>) -> Self {
        BACKEND_DEFERRED.store(true, Ordering::Relaxed);
        Self {
            _backend: None,
            _portable: None,
            _lock: lock,
        }
    }
}

struct Backend {
    marker: Option<tempfile::NamedTempFile>,
    process: Process,
}
impl Drop for Backend {
    fn drop(&mut self) {
        self.marker.take();
        if !matches!(self.process.wait(15_000), Ok(true)) {
            show_error("索引后台尚未退出，请稍候再重新打开 StashFinder。");
        }
    }
}
pub fn start(config: &mut Config) -> Result<Option<Session>> {
    let executable = std::env::current_exe()?;
    let layout = Layout::discover(&executable)?;
    let installed = layout.is_none() && executable.with_file_name("installed.marker").is_file();
    ensure!(INSTALLED.set(installed).is_ok(), "运行模式已初始化");
    let windows_service = installed && stashfinder_windows::scm::registered()?;
    ensure!(
        WINDOWS_SERVICE.set(windows_service).is_ok(),
        "运行模式已初始化"
    );
    configure(config, layout.as_ref());
    ensure!(PORTABLE.set(layout.clone()).is_ok(), "运行模式已初始化");
    if layout.is_none() && (!installed || windows_service) {
        return Ok(None);
    }
    ensure!(
        !is_elevated(),
        "请以普通权限双击 StashFinder。程序会单独为索引后台申请管理员权限。"
    );
    if let Some(layout) = &layout {
        let lock = layout.lock()?;
        return Ok(Some(
            match PortableSession::start(layout, || confirm_index_backend(ClientKind::Desktop))? {
                Some(session) => Session {
                    _backend: None,
                    _portable: Some(session),
                    _lock: Some(lock),
                },
                None => Session::defer(Some(lock)),
            },
        ));
    }
    let backend = executable.with_file_name("stashfinder-service.exe");
    ensure!(
        backend.is_file(),
        "缺少 stashfinder-service.exe，请重新安装"
    );
    if !confirm_index_backend(ClientKind::Desktop)? {
        return Ok(Some(Session::defer(None)));
    }

    let marker = tempfile::Builder::new()
        .prefix("stashfinder-session-")
        .suffix(".guard")
        .tempfile()?;
    let args = [
        "--desktop-session".into(),
        std::process::id().to_string(),
        marker.path().to_string_lossy().into_owned(),
    ];
    let Some(process) = Process::launch_elevated(&backend, &args)? else {
        return Ok(Some(Session::defer(None)));
    };
    let backend = Backend {
        marker: Some(marker),
        process,
    };
    let runtime = tokio::runtime::Runtime::new()?;
    backend
        .process
        .wait_ready(&runtime, pipe(), "desktop_session")?;
    Ok(Some(Session {
        _backend: Some(backend),
        _portable: None,
        _lock: None,
    }))
}

pub fn setup(app: &tauri::AppHandle) -> Result<()> {
    if windows_service() {
        tauri::async_runtime::spawn(async {
            if let Err(error) = status().await {
                eprintln!("installed backend startup: {error:#}");
            }
        });
    }
    if !active() {
        return Ok(());
    }
    let show = MenuItem::with_id(app, "portable-show", "显示 StashFinder", true, None::<&str>)?;
    let exit = MenuItem::with_id(app, "portable-exit", "退出", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show, &exit])?;
    TrayIconBuilder::with_id("portable")
        .icon(app.default_window_icon().context("应用图标缺失")?.clone())
        .tooltip("StashFinder 便携版")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| {
            if event.id.as_ref() == "portable-exit" {
                app_exit(app.clone());
            } else if let Some(window) = app.get_webview_window("main") {
                let _ = window.show();
                let _ = window.unminimize();
                let _ = window.set_focus();
            }
        })
        .on_tray_icon_event(|tray, event| {
            if matches!(
                event,
                TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                }
            ) && let Some(window) = tray.app_handle().get_webview_window("main")
            {
                let _ = window.show();
                let _ = window.unminimize();
                let _ = window.set_focus();
            }
        })
        .build(app)?;
    Ok(())
}

#[tauri::command]
pub fn app_exit(app: tauri::AppHandle) {
    if app.state::<crate::search_export::SearchExport>().cancel() {
        return;
    }
    if crate::file_operations::prevent_close(&app.state::<crate::file_operations::Operations>()) {
        if let Some(window) = app.get_webview_window("main") {
            let _ = window.show();
            let _ = window.unminimize();
            let _ = window.set_focus();
        }
    } else {
        app.exit(0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deferring_the_backend_preserves_the_portable_lock_and_explains_its_absence() {
        let root = tempfile::tempdir().unwrap();
        let executable = root.path().join("StashFinder.exe");
        std::fs::write(&executable, b"test").unwrap();
        std::fs::write(root.path().join("portable.json"), br#"{"version":1}"#).unwrap();
        let layout = Layout::discover(&executable).unwrap().unwrap();

        let session = Session::defer(Some(layout.lock().unwrap()));
        assert!(layout.lock().is_err());
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let status_error = runtime.block_on(status()).unwrap_err();
        let snapshot_error = runtime.block_on(snapshot()).unwrap_err();
        let request_error = runtime
            .block_on(request(stashfinder_core::Action::Refresh))
            .unwrap_err();
        assert!(status_error.to_string().contains("本次未启动索引后台"));
        assert_eq!(snapshot_error.to_string(), status_error.to_string());
        assert_eq!(request_error.to_string(), status_error.to_string());

        drop(session);
        assert!(layout.lock().is_ok());
    }

    #[test]
    fn portable_overrides_every_tauri_directory_and_webview_storage() {
        let root = tempfile::tempdir().unwrap();
        let executable = root.path().join("StashFinder.exe");
        std::fs::write(&executable, b"test").unwrap();
        std::fs::write(root.path().join("portable.json"), br#"{"version":1}"#).unwrap();
        let layout = stashfinder_windows::portable::Layout::discover(&executable)
            .unwrap()
            .unwrap();
        let mut config = tauri::utils::config::Config::default();
        config.app.windows.push(Default::default());
        configure(&mut config, Some(&layout));
        let value = serde_json::to_value(&config).unwrap();
        let overrides = &value["app"]["appDirectoriesOverride"];
        for key in ["config", "data", "localData", "cache", "log"] {
            let path = std::path::Path::new(overrides[key].as_str().unwrap());
            assert!(
                path.starts_with(&layout.root),
                "{key} escaped the portable directory"
            );
        }
        assert!(config.app.windows[0].data_directory.is_none());
        assert_eq!(
            overrides["localData"].as_str(),
            Some(layout.local.to_string_lossy().as_ref())
        );
    }
    #[test]
    fn normal_distribution_keeps_tauri_directory_defaults() {
        let mut config = tauri::utils::config::Config::default();
        let before = serde_json::to_value(&config).unwrap();
        configure(&mut config, None);
        assert_eq!(serde_json::to_value(&config).unwrap(), before);
    }
}
