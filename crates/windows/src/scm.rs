use anyhow::{Context, Result, bail, ensure};
use std::time::{Duration, Instant};
use windows_service::{
    Error,
    service::{ServiceAccess, ServiceState, ServiceStatus},
    service_manager::{ServiceManager, ServiceManagerAccess},
};

pub const INSTALLED_SERVICE: &str = "StashFinder";
pub const INSTALLED_PIPE: &str = r"\\.\pipe\StashFinder.Installed";

pub fn registered() -> Result<bool> {
    let manager = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT)
        .context("无法访问 Windows 服务管理器")?;
    match manager.open_service(INSTALLED_SERVICE, ServiceAccess::QUERY_STATUS) {
        Ok(_) => Ok(true),
        Err(Error::Winapi(error)) if error.raw_os_error() == Some(1060) => Ok(false),
        result => result
            .context("无法查询 StashFinder 后台服务")
            .map(|_| true),
    }
}

pub fn ensure_running() -> Result<u32> {
    let manager = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT)
        .context("无法访问 Windows 服务管理器")?;
    let service = match manager.open_service(INSTALLED_SERVICE, ServiceAccess::QUERY_STATUS) {
        Err(Error::Winapi(error)) if error.raw_os_error() == Some(1060) => {
            bail!("未安装 StashFinder 后台服务，请运行 StashFinder 安装程序")
        }
        result => result.context("无法查询 StashFinder 后台服务")?,
    };
    wait_ready(
        || {
            service
                .query_status()
                .context("无法查询 StashFinder 后台服务")
        },
        || {
            let starter = manager
                .open_service(INSTALLED_SERVICE, ServiceAccess::START)
                .context("没有启动 StashFinder 后台服务的权限，请检查安装账户或重新运行安装程序")?;
            match starter.start::<&str>(&[]) {
                Err(Error::Winapi(error)) if error.raw_os_error() == Some(1056) => Ok(()),
                result => result.context("无法启动 StashFinder 后台服务"),
            }
        },
        Duration::from_secs(20),
    )
}

fn wait_ready(
    mut query: impl FnMut() -> Result<ServiceStatus>,
    start: impl FnOnce() -> Result<()>,
    timeout: Duration,
) -> Result<u32> {
    let deadline = Instant::now() + timeout;
    let mut start = Some(start);
    loop {
        let status = query()?;
        match status.current_state {
            ServiceState::Running => return status.process_id.context("Windows 未报告后台进程 ID"),
            ServiceState::Stopped => {
                if let Some(start) = start.take() {
                    start()?;
                } else {
                    bail!("StashFinder 后台服务启动后停止（{:?}）", status.exit_code);
                }
            }
            ServiceState::StartPending => {
                start = None;
            }
            ServiceState::StopPending => (),
            _ => bail!("StashFinder 后台服务状态异常（{:?}）", status.current_state),
        }
        ensure!(
            Instant::now() < deadline,
            "等待 StashFinder 后台服务进入运行状态超时"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use windows_service::service::{ServiceControlAccept, ServiceExitCode, ServiceType};

    fn state(current_state: ServiceState, process_id: Option<u32>) -> ServiceStatus {
        ServiceStatus {
            service_type: ServiceType::OWN_PROCESS,
            current_state,
            controls_accepted: ServiceControlAccept::empty(),
            exit_code: ServiceExitCode::Win32(0),
            checkpoint: 0,
            wait_hint: Duration::ZERO,
            process_id,
        }
    }

    #[test]
    fn starts_stopped_service_once_and_waits_for_running_process() {
        let states = [
            state(ServiceState::Stopped, None),
            state(ServiceState::StartPending, None),
            state(ServiceState::Running, Some(42)),
        ];
        let mut states = states.into_iter();
        let calls = Cell::new(0);
        let pid = wait_ready(
            || Ok(states.next().unwrap()),
            || {
                calls.set(calls.get() + 1);
                Ok(())
            },
            Duration::from_secs(2),
        )
        .unwrap();
        assert_eq!(pid, 42);
        assert_eq!(calls.get(), 1);
    }

    #[test]
    fn running_service_is_reused_and_failed_start_is_not_repeated() {
        assert_eq!(
            wait_ready(
                || Ok(state(ServiceState::Running, Some(7))),
                || panic!("already running"),
                Duration::ZERO
            )
            .unwrap(),
            7
        );
        let calls = Cell::new(0);
        let error = wait_ready(
            || Ok(state(ServiceState::Stopped, None)),
            || {
                calls.set(calls.get() + 1);
                Ok(())
            },
            Duration::from_secs(2),
        )
        .unwrap_err();
        assert!(error.to_string().contains("停止"));
        assert_eq!(calls.get(), 1);
    }
}
