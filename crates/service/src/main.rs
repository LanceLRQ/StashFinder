use anyhow::{Context, Result, bail};
use stashfinder_windows::ipc::{DEFAULT_PIPE, create_pipe, current_user_sid, status};
use std::{ffi::OsString, time::Duration};
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
        Some("--service") if args.len() == 2 => {
            service_dispatcher::start(SERVICE_NAME, ffi_service_main)?;
        }
        Some("--foreground") if args.len() <= 2 => {
            let name = args.get(1).map(String::as_str).unwrap_or(DEFAULT_PIPE);
            let sid = current_user_sid()?;
            tokio::runtime::Runtime::new()?.block_on(async {
                let pipe = create_pipe(name, &sid, true)?;
                let (stop, receiver) = watch::channel(false);
                tokio::spawn(async move {
                    let _ = tokio::signal::ctrl_c().await;
                    let _ = stop.send(true);
                });
                println!("listening: {name}");
                stashfinder_service::serve(pipe, name, &sid, "foreground", receiver).await
            })?;
        }
        Some("--probe") if args.len() <= 2 => {
            let name = args.get(1).map(String::as_str).unwrap_or(DEFAULT_PIPE);
            let response = tokio::runtime::Runtime::new()?.block_on(status(name))?;
            println!("{}", serde_json::to_string(&response)?);
        }
        Some("--sid") if args.len() == 1 => println!("{}", current_user_sid()?),
        _ => bail!(
            "usage: stashfinder-service --foreground [pipe] | --probe [pipe] | --sid | --service <owner SID>"
        ),
    }
    Ok(())
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
    let (stop, receiver) = watch::channel(false);
    let handle = service_control_handler::register(SERVICE_NAME, move |control| match control {
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
        wait_hint: Duration::from_secs(10),
        process_id: None,
    };
    handle.set_service_status(state.clone())?;
    let result = (|| -> Result<()> {
        tokio::runtime::Runtime::new()?.block_on(async {
            let pipe = create_pipe(DEFAULT_PIPE, &sid, true)?;
            state.current_state = ServiceState::Running;
            state.controls_accepted = ServiceControlAccept::STOP;
            state.checkpoint = 0;
            state.wait_hint = Duration::ZERO;
            handle.set_service_status(state.clone())?;
            stashfinder_service::serve(pipe, DEFAULT_PIPE, &sid, "windows_service", receiver).await
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
