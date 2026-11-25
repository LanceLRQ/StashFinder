mod actions;
mod browse;
mod registration;
mod search;
mod window;

use anyhow::{Result, bail, ensure};
use stashfinder_windows::{ipc, portable, scm, session::ManagedSession, storage};
use std::path::Path;

#[derive(Clone)]
enum Endpoint {
    Direct(String),
    WindowsService,
}

impl Endpoint {
    fn pipe(&self) -> &str {
        match self {
            Self::Direct(name) => name,
            Self::WindowsService => scm::INSTALLED_PIPE,
        }
    }

    async fn ready(&self) -> Result<()> {
        if matches!(self, Self::WindowsService) {
            scm::installed_status().await?;
        }
        Ok(())
    }
}

pub fn run(program: &Path, args: &[String]) -> Result<()> {
    ensure!(!portable::is_elevated(), "请用普通用户权限启动搜索窗口");
    let (browse, args) = match args {
        [entry, rest @ ..] if matches!(entry.as_str(), "--browse" | "--quick-search") => {
            (entry == "--browse", rest)
        }
        _ => (false, args),
    };
    let layout = portable::Layout::discover(program)?;
    let installed = layout.is_none() && storage::is_installed_program(program)?;
    if layout.is_some() || (installed && !scm::registered()?) {
        ensure!(args.is_empty(), "快速搜索只连接本实例的索引后台");
        let Some(_session) = ManagedSession::start(program, layout.as_ref(), || {
            portable::confirm_index_backend(portable::ClientKind::QuickSearch)
        })?
        else {
            return Ok(());
        };
        let pipe = layout.map_or_else(|| scm::INSTALLED_PIPE.to_owned(), |l| l.pipe);
        return window::run(Endpoint::Direct(pipe), browse);
    }
    let pipe = search_pipe(args, installed)?;
    window::run(
        if pipe == scm::INSTALLED_PIPE {
            Endpoint::WindowsService
        } else {
            Endpoint::Direct(pipe)
        },
        browse,
    )
}

fn search_pipe(args: &[String], installed: bool) -> Result<String> {
    if installed {
        ensure!(
            args.is_empty(),
            "安装版快速搜索只连接已注册的 StashFinder Windows 服务"
        );
        return Ok(scm::INSTALLED_PIPE.to_owned());
    }
    Ok(match args {
        [] => ipc::DEFAULT_PIPE.to_owned(),
        [flag, name] if flag == "--pipe" && name.starts_with(r"\\.\pipe\StashFinder.") => {
            name.clone()
        }
        _ => bail!(r"用法：stashfinder-native-search.exe [--pipe \\.\pipe\StashFinder.名称]"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn installed_entry_uses_only_the_registered_service() {
        let custom = ["--pipe".into(), r"\\.\pipe\StashFinder.test".into()];
        assert_eq!(search_pipe(&[], true).unwrap(), scm::INSTALLED_PIPE);
        assert!(search_pipe(&custom, true).is_err());
        assert_eq!(search_pipe(&[], false).unwrap(), ipc::DEFAULT_PIPE);
        assert_eq!(search_pipe(&custom, false).unwrap(), custom[1]);
    }
}
