#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use anyhow::{Context, Result, ensure};
mod file_operations;
use serde::Serialize;
use stashfinder_windows::shell;
use std::{path::PathBuf, sync::Mutex};
use tauri::{AppHandle, Manager, State};
mod preferences;
mod runtime;
mod visual_cache;
mod visuals;

#[tauri::command]
fn preferences_get(app: AppHandle) -> Result<preferences::Preferences, String> {
    let path = app
        .path()
        .app_config_dir()
        .map_err(|e| e.to_string())?
        .join("preferences.json");
    preferences::read(&path).map_err(|e| format!("{e:#}"))
}

#[tauri::command]
fn preferences_set(
    app: AppHandle,
    preferences: preferences::Preferences,
) -> Result<preferences::Preferences, String> {
    let path = app
        .path()
        .app_config_dir()
        .map_err(|e| e.to_string())?
        .join("preferences.json");
    preferences::write(&path, &preferences).map_err(|e| format!("{e:#}"))?;
    preferences::read(&path).map_err(|e| format!("{e:#}"))
}

#[tauri::command]
fn app_version(app: AppHandle) -> String {
    app.package_info().version.to_string()
}

#[derive(Clone, Serialize)]
struct Sample {
    id: usize,
    label: String,
    path: PathBuf,
    kind: String,
    available: bool,
}
#[derive(Default)]
struct Samples(Mutex<Vec<Sample>>);

#[tauri::command]
async fn service_status() -> Result<stashfinder_core::Status, String> {
    runtime::status()
        .await
        .map_err(|error| format!("{error:#}"))
}

#[tauri::command]
async fn asset_snapshot() -> Result<stashfinder_core::assets::Snapshot, String> {
    runtime::snapshot()
        .await
        .map_err(|error| format!("{error:#}"))
}

#[tauri::command]
async fn asset_command(action: stashfinder_core::Action) -> Result<(), String> {
    match action {
        stashfinder_core::Action::Refresh
        | stashfinder_core::Action::Register { .. }
        | stashfinder_core::Action::Retry { .. }
        | stashfinder_core::Action::Rename { .. }
        | stashfinder_core::Action::Unregister { .. } => runtime::request(action)
            .await
            .map(|_| ())
            .map_err(|error| format!("{error:#}")),
        _ => Err("不是资产操作".into()),
    }
}

#[tauri::command]
async fn index_request(
    action: stashfinder_core::Action,
) -> Result<stashfinder_core::Response, String> {
    match action {
        stashfinder_core::Action::IndexStart { .. }
        | stashfinder_core::Action::TaskCancel { .. }
        | stashfinder_core::Action::TaskRetry { .. }
        | stashfinder_core::Action::TaskPage { .. }
        | stashfinder_core::Action::AutoUpdate { .. }
        | stashfinder_core::Action::PartitionPause { .. }
        | stashfinder_core::Action::IndexState { .. }
        | stashfinder_core::Action::SearchStart(_)
        | stashfinder_core::Action::Directory(_)
        | stashfinder_core::Action::SearchPage { .. }
        | stashfinder_core::Action::SearchBatch { .. }
        | stashfinder_core::Action::SearchCancel { .. } => runtime::request(action)
            .await
            .map_err(|error| format!("{error:#}")),
        _ => Err("不是索引操作".into()),
    }
}

#[tauri::command]
fn create_samples(app: AppHandle, state: State<'_, Samples>) -> Result<Vec<Sample>, String> {
    (|| -> Result<Vec<Sample>> {
        let mut samples = state
            .0
            .lock()
            .map_err(|_| anyhow::anyhow!("sample state unavailable"))?;
        if !samples.is_empty() {
            return Ok(samples.clone());
        }
        if let Some(root) = std::env::var_os("STASHFINDER_M0_SAMPLE_ROOT") {
            *samples = sample_files(&PathBuf::from(root), false)?;
        } else {
            let base = app.path().app_local_data_dir()?.join("m0-samples");
            std::fs::create_dir_all(&base)?;
            let temporary = tempfile::Builder::new().prefix("run-").tempdir_in(&base)?;
            *samples = sample_files(temporary.path(), true)?;
            // Keep generated samples available after the GUI exits.
            let _ = temporary.keep();
        }
        Ok(samples.clone())
    })()
    .map_err(|error| format!("{error:#}"))
}

fn sample_files(root: &std::path::Path, generate: bool) -> Result<Vec<Sample>> {
    ensure!(
        root.is_absolute() && root.is_dir(),
        "样本目录不存在或不是绝对路径"
    );
    let folder = root.join("样本目录");
    let mut long_folder = root.join("long-path");
    while long_folder.as_os_str().len() < 270 {
        long_folder = long_folder.join("long-segment-0123456789");
    }
    let paths = [
        root.join("中文样本.txt"),
        root.join("second.txt"),
        folder.clone(),
        long_folder.join("long-sample.txt"),
    ];
    if generate {
        std::fs::create_dir(&folder)?;
        std::fs::write(
            folder.join("inside.txt"),
            b"StashFinder directory sample\r\n",
        )?;
        std::fs::create_dir_all(&long_folder)?;
        for (index, path) in paths.iter().enumerate() {
            if index != 2 {
                std::fs::write(path, format!("StashFinder sample {index}\r\n"))?;
            }
        }
    }
    for (index, path) in paths.iter().enumerate() {
        ensure!(
            if index == 2 {
                path.is_dir()
            } else {
                path.is_file()
            },
            "验证样本缺失或类型不符：{}",
            path.display()
        );
    }
    Ok(paths
        .into_iter()
        .enumerate()
        .map(|(id, path)| Sample {
            id,
            label: if id == 3 {
                "长路径样本 (>260)".to_owned()
            } else {
                path.file_name().unwrap().to_string_lossy().into_owned()
            },
            path,
            kind: if id == 2 { "目录" } else { "文件" }.to_owned(),
            available: true,
        })
        .collect())
}

fn selected_paths(state: &Samples, ids: &[usize]) -> Result<Vec<PathBuf>> {
    ensure!(!ids.is_empty() && ids.len() <= 4, "请选择 1～4 个样本");
    let samples = state
        .0
        .lock()
        .map_err(|_| anyhow::anyhow!("sample state unavailable"))?;
    let mut paths = Vec::new();
    for id in ids {
        let sample = samples.get(*id).context("样本编号无效，请重新选择")?;
        ensure!(!paths.contains(&sample.path), "重复的样本编号");
        paths.push(sample.path.clone());
    }
    Ok(paths)
}

enum Transfer {
    Copy,
    Cut,
    Drag,
}

async fn transfer(app: AppHandle, paths: Vec<PathBuf>, action: Transfer) -> Result<String, String> {
    let (sender, receiver) = tokio::sync::oneshot::channel();
    app.run_on_main_thread(move || {
        let result = match action {
            Transfer::Drag => shell::drag_samples(&paths).map(|copied| {
                if copied {
                    "目标已接受复制，请核对目标文件。"
                } else {
                    "拖出已取消或目标未接受复制。"
                }
                .to_owned()
            }),
            Transfer::Copy => shell::copy_samples(&paths)
                .map(|()| "已复制到剪贴板，请到资源管理器粘贴。".to_owned()),
            Transfer::Cut => shell::cut_samples(&paths).map(|()| {
                "已剪切到剪贴板，源文件尚未移动。请到资源管理器粘贴，再按 F5 核对。".to_owned()
            }),
        };
        let _ = sender.send(result.map_err(|error| format!("{error:#}")));
    })
    .map_err(|error| error.to_string())?;
    receiver.await.map_err(|error| error.to_string())?
}

#[tauri::command]
async fn copy_samples(
    app: AppHandle,
    state: State<'_, Samples>,
    ids: Vec<usize>,
) -> Result<String, String> {
    let paths = selected_paths(&state, &ids).map_err(|error| format!("{error:#}"))?;
    transfer(app, paths, Transfer::Copy).await
}

#[tauri::command]
async fn drag_samples(
    app: AppHandle,
    state: State<'_, Samples>,
    ids: Vec<usize>,
) -> Result<String, String> {
    let paths = selected_paths(&state, &ids).map_err(|error| format!("{error:#}"))?;
    transfer(app, paths, Transfer::Drag).await
}

#[tauri::command]
async fn cut_samples(
    app: AppHandle,
    state: State<'_, Samples>,
    ids: Vec<usize>,
) -> Result<String, String> {
    let paths = selected_paths(&state, &ids).map_err(|error| format!("{error:#}"))?;
    transfer(app, paths, Transfer::Cut).await
}

#[tauri::command]
fn refresh_samples(state: State<'_, Samples>) -> Result<Vec<Sample>, String> {
    let mut samples = state.0.lock().map_err(|_| "sample state unavailable")?;
    for sample in samples.iter_mut() {
        sample.available = if sample.kind == "目录" {
            sample.path.is_dir()
        } else {
            sample.path.is_file()
        };
    }
    Ok(samples.clone())
}

#[tauri::command]
async fn edit_samples(
    app: AppHandle,
    state: State<'_, Samples>,
    ids: Vec<usize>,
    action: String,
    name: Option<String>,
) -> Result<String, String> {
    let paths = selected_paths(&state, &ids).map_err(|error| format!("{error:#}"))?;
    let edit = match action.as_str() {
        "rename" => shell::SampleEdit::Rename(name.ok_or("请输入新名称")?),
        "recycle" => shell::SampleEdit::Recycle,
        "delete" => shell::SampleEdit::Delete,
        _ => return Err("未知文件操作".into()),
    };
    let (sender, receiver) = tokio::sync::oneshot::channel();
    let handle = app.clone();
    app.run_on_main_thread(move || {
        let result = (|| -> Result<String> {
            shell::edit_samples(&paths, &edit)?;
            if let shell::SampleEdit::Rename(name) = &edit {
                let state = handle.state::<Samples>();
                let mut samples = state
                    .0
                    .lock()
                    .map_err(|_| anyhow::anyhow!("sample state unavailable"))?;
                let sample = &mut samples[ids[0]];
                sample.path.set_file_name(name);
                sample.label = name.clone();
            }
            Ok(match edit {
                shell::SampleEdit::Rename(_) => "重命名已完成。",
                shell::SampleEdit::Recycle => "删除操作已完成，请在回收站核对并尝试还原。",
                shell::SampleEdit::Delete => "永久删除已完成，不经过回收站。",
            }
            .to_owned())
        })();
        let _ = sender.send(result.map_err(|error| format!("{error:#}")));
    })
    .map_err(|error| error.to_string())?;
    receiver.await.map_err(|error| error.to_string())?
}

fn main() {
    if let Err(error) = run() {
        runtime::show_error(&format!("{error:#}"));
    }
}

fn run() -> Result<(), anyhow::Error> {
    let mut context = tauri::generate_context!();
    let mut session = runtime::start(context.config_mut())?;
    let app = tauri::Builder::default()
        .setup(|app| runtime::setup(app.handle()).map_err(Into::into))
        .manage(Samples::default())
        .manage(file_operations::Operations::default())
        .manage(visuals::Visuals::default())
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                if file_operations::prevent_close(&window.state::<file_operations::Operations>()) {
                    api.prevent_close();
                } else if runtime::active() {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            preferences_get,
            preferences_set,
            app_version,
            runtime::app_exit,
            service_status,
            asset_snapshot,
            asset_command,
            index_request,
            file_operations::file_prepare,
            file_operations::file_run,
            file_operations::file_status,
            file_operations::file_cancel,
            file_operations::file_paste,
            file_operations::file_report,
            file_operations::file_keep_open,
            visuals::visual_icon,
            visuals::visual_preview,
            visuals::visual_cancel,
            visuals::visual_cache_status,
            visuals::visual_cache_limit,
            visuals::visual_cache_clear,
            create_samples,
            copy_samples,
            drag_samples,
            cut_samples,
            refresh_samples,
            edit_samples
        ])
        .build(context)?;
    app.run(move |_, event| {
        if matches!(event, tauri::RunEvent::Exit) {
            session.take();
        }
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn loads_external_samples_without_writing_and_rejects_incomplete_sets() {
        let root = tempfile::tempdir().unwrap();
        let generated = sample_files(root.path(), true).unwrap();
        std::fs::write(&generated[1].path, "existing content").unwrap();
        let loaded = sample_files(root.path(), false).unwrap();
        assert_eq!(loaded.len(), 4);
        assert_eq!(loaded[1].path, generated[1].path);
        assert_eq!(
            std::fs::read_to_string(&loaded[1].path).unwrap(),
            "existing content"
        );
        std::fs::remove_file(&generated[0].path).unwrap();
        assert!(sample_files(root.path(), false).is_err());
        assert!(!generated[0].path.exists());
        assert!(sample_files(std::path::Path::new("relative"), false).is_err());
    }
    #[test]
    fn rejects_missing_empty_and_duplicate_selection() {
        let state = Samples(Mutex::new(vec![Sample {
            id: 0,
            label: "sample".into(),
            path: PathBuf::from(r"C:\generated\sample.txt"),
            kind: "文件".into(),
            available: true,
        }]));
        assert!(selected_paths(&state, &[]).is_err());
        assert!(selected_paths(&state, &[1]).is_err());
        assert!(selected_paths(&state, &[0, 0]).is_err());
        assert_eq!(selected_paths(&state, &[0]).unwrap().len(), 1);
    }
}
