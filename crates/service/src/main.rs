use anyhow::{Context, Result, bail};
use stashfinder_windows::ipc::{DEFAULT_PIPE, create_pipe, current_user_sid, status};
use stashfinder_windows::scm::{INSTALLED_PIPE, INSTALLED_SERVICE};
use stashfinder_windows::{
    portable::{Process, enable_backup_read, is_elevated},
    storage,
};
use std::{ffi::OsString, path::PathBuf, time::Duration};
use tokio::sync::watch;
use windows_service::{
    define_windows_service,
    service::{
        ServiceControl, ServiceControlAccept, ServiceExitCode, ServiceState, ServiceStatus,
        ServiceType,
    },
    service_control_handler::{self, ServiceControlHandlerResult},
    service_dispatcher,
};

const SERVICE_NAME: &str = "StashFinderM0";
define_windows_service!(ffi_service_main, service_main);

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("--portable") => run_portable(&args[1..])?,
        Some("--desktop-session") if args.len() == 3 => {
            anyhow::ensure!(is_elevated(), "索引后台需要管理员权限");
            let parent = Process::open(args[1].parse().context("桌面进程 ID 无效")?)?;
            let root = storage::program_data()?.join("StashFinder");
            storage::validate_service_root(&root)?;
            let owner = std::fs::read_to_string(root.join("owner.sid"))?;
            let owner = owner.trim();
            anyhow::ensure!(parent.user_sid()? == owner, "桌面账户与安装账户不匹配");
            storage::use_admin_file_owner()?;
            enable_backup_read()?;
            let _lock = storage::lock_backend(&root)?;
            run_session(
                root,
                INSTALLED_PIPE,
                owner,
                parent,
                PathBuf::from(&args[2]),
                "desktop_session",
            )?;
        }
        Some("--service") if args.len() == 2 || args.len() == 4 => {
            service_dispatcher::start(SERVICE_NAME, ffi_service_main)?;
        }
        Some("--installed-service") if args.len() == 2 => {
            service_dispatcher::start(INSTALLED_SERVICE, ffi_service_main)?;
        }
        Some("--foreground") => {
            let root = data_root(&args, false)?;
            let name = args
                .get(1)
                .filter(|s| !s.starts_with("--"))
                .map(String::as_str)
                .unwrap_or(DEFAULT_PIPE);
            let sid = current_user_sid()?;
            tokio::runtime::Runtime::new()?.block_on(async {
                let pipe = create_pipe(name, &sid, true)?;
                stashfinder_store::catalog::Catalog::open(&root)?;
                let indexes = stashfinder_service::jobs::Indexes::start(root.clone());
                let assets = stashfinder_service::assets::Assets::start(
                    root,
                    stashfinder_windows::volumes::discover,
                );
                let (stop, receiver) = watch::channel(false);
                if args.iter().any(|arg| arg == "--watch-stdin") {
                    std::thread::spawn(move || {
                        // PowerShell may write an encoding preamble before closing the pipe.
                        let _ = std::io::copy(&mut std::io::stdin(), &mut std::io::sink());
                        let _ = stop.send(true);
                    });
                } else {
                    tokio::spawn(async move {
                        let _ = tokio::signal::ctrl_c().await;
                        let _ = stop.send(true);
                    });
                }
                println!("listening: {name}");
                stashfinder_service::serve_runtime(
                    pipe,
                    name,
                    &sid,
                    "foreground",
                    receiver,
                    stashfinder_service::Services {
                        assets: Some(&assets),
                        indexes: Some(&indexes),
                    },
                )
                .await
            })?;
        }
        Some("--probe") if args.len() <= 2 => {
            let name = args.get(1).map(String::as_str).unwrap_or(DEFAULT_PIPE);
            let response = tokio::runtime::Runtime::new()?.block_on(status(name))?;
            println!("{}", serde_json::to_string(&response)?);
        }
        Some("--sid") if args.len() == 1 => println!("{}", current_user_sid()?),
        Some("--devices") if args.len() == 1 => println!(
            "{}",
            serde_json::to_string_pretty(&stashfinder_windows::volumes::discover()?)?
        ),
        Some("--snapshot") if args.len() <= 2 => {
            let name = args.get(1).map(String::as_str).unwrap_or(DEFAULT_PIPE);
            let snapshot = tokio::runtime::Runtime::new()?
                .block_on(stashfinder_windows::ipc::snapshot(name))?;
            println!("{}", serde_json::to_string_pretty(&snapshot)?);
        }
        Some("--request") if (2..=3).contains(&args.len()) => {
            let action: stashfinder_core::Action = serde_json::from_str(&args[1])?;
            let name = args.get(2).map(String::as_str).unwrap_or(DEFAULT_PIPE);
            let response = tokio::runtime::Runtime::new()?
                .block_on(stashfinder_windows::ipc::request(name, action))?;
            println!("{}", serde_json::to_string_pretty(&response)?);
        }
        _ => bail!(
            "usage: stashfinder-service --foreground [pipe] --data-dir <absolute directory> | --probe [pipe] | --snapshot [pipe] | --request <action-json> [pipe] | --devices | --sid | --service <owner SID> [--data-dir <directory>]"
        ),
    }
    Ok(())
}

fn run_portable(args: &[String]) -> Result<()> {
    use stashfinder_windows::portable::Backend;
    let config = Backend::parse(&std::env::current_exe()?, args)?;
    if is_elevated() {
        enable_backup_read()?;
    }
    let parent = Process::open(config.parent)?;
    run_session(
        config.layout.index,
        &config.layout.pipe,
        &config.owner,
        parent,
        config.session,
        "portable",
    )
}

fn run_session(
    root: PathBuf,
    endpoint: &str,
    owner: &str,
    parent: Process,
    session: PathBuf,
    mode: &str,
) -> Result<()> {
    anyhow::ensure!(!parent.wait(0)? && session.is_file(), "桌面会话已结束");
    tokio::runtime::Runtime::new()?.block_on(async {
        // Own the endpoint before starting any writer for this portable directory.
        let pipe = create_pipe(endpoint, owner, true)?;
        let (stop, receiver) = watch::channel(false);
        std::thread::spawn(move || {
            let _ = parent.wait_session(&session);
            let _ = stop.send(true);
        });
        stashfinder_store::catalog::Catalog::open(&root)?;
        let indexes = stashfinder_service::jobs::Indexes::start(root.clone());
        let assets = stashfinder_service::assets::Assets::start(
            root,
            stashfinder_windows::volumes::discover,
        );
        stashfinder_service::serve_runtime(
            pipe,
            endpoint,
            owner,
            mode,
            receiver,
            stashfinder_service::Services {
                assets: Some(&assets),
                indexes: Some(&indexes),
            },
        )
        .await
    })
}

fn data_root(args: &[String], service: bool) -> Result<PathBuf> {
    let root = if let Some(index) = args.iter().position(|s| s == "--data-dir") {
        anyhow::ensure!(
            index + 2 == args.len(),
            "--data-dir must be the final option"
        );
        PathBuf::from(args.get(index + 1).context("missing data directory")?)
    } else if service {
        storage::program_data()?.join("StashFinder")
    } else {
        bail!("foreground development requires --data-dir <absolute test directory>")
    };
    anyhow::ensure!(root.is_absolute(), "data directory must be absolute");
    Ok(root)
}

fn service_main(_args: Vec<OsString>) {
    if let Err(error) = run_service() {
        eprintln!("service failed: {error:#}");
    }
}

fn run_service() -> Result<()> {
    let sid = std::env::args()
        .nth(2)
        .context("missing configured user SID")?;
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    let (name, endpoint) = if args[0] == "--installed-service" {
        (INSTALLED_SERVICE, INSTALLED_PIPE)
    } else {
        (SERVICE_NAME, DEFAULT_PIPE)
    };
    let (stop, receiver) = watch::channel(false);
    let handle = service_control_handler::register(name, move |control| match control {
        ServiceControl::Stop => {
            let _ = stop.send(true);
            ServiceControlHandlerResult::NoError
        }
        ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,
        _ => ServiceControlHandlerResult::NotImplemented,
    })?;
    let mut state = ServiceStatus {
        service_type: ServiceType::OWN_PROCESS,
        current_state: ServiceState::StartPending,
        controls_accepted: ServiceControlAccept::empty(),
        exit_code: ServiceExitCode::Win32(0),
        checkpoint: 1,
        wait_hint: Duration::from_secs(20),
        process_id: None,
    };
    handle.set_service_status(state.clone())?;
    let result = (|| -> Result<()> {
        let root = data_root(&args, true)?;
        storage::validate_service_root(&root)?;
        let _lock = storage::lock_backend(&root)?;
        tokio::runtime::Runtime::new()?.block_on(async {
            let pipe = create_pipe(endpoint, &sid, true)?;
            stashfinder_store::catalog::Catalog::open(&root)?;
            let indexes = stashfinder_service::jobs::Indexes::start(root.clone());
            let assets = stashfinder_service::assets::Assets::start(
                root,
                stashfinder_windows::volumes::discover,
            );
            state.current_state = ServiceState::Running;
            state.controls_accepted = ServiceControlAccept::STOP;
            state.checkpoint = 0;
            state.wait_hint = Duration::ZERO;
            handle.set_service_status(state.clone())?;
            stashfinder_service::serve_runtime(
                pipe,
                endpoint,
                &sid,
                "windows_service",
                receiver,
                stashfinder_service::Services {
                    assets: Some(&assets),
                    indexes: Some(&indexes),
                },
            )
            .await
        })
    })();
    state.current_state = ServiceState::Stopped;
    state.controls_accepted = ServiceControlAccept::empty();
    state.exit_code = ServiceExitCode::Win32(if result.is_ok() { 0 } else { 1 });
    state.checkpoint = 0;
    state.wait_hint = Duration::ZERO;
    handle.set_service_status(state)?;
    result
}
