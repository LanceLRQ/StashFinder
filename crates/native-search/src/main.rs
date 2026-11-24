#![windows_subsystem = "windows"]

mod actions;
mod search;
mod window;

use anyhow::{Result, bail, ensure};
use stashfinder_windows::{ipc, portable, scm, storage};

fn main() {
    if let Err(error) = run() {
        portable::show_error(&format!("{error:#}"));
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let installed = storage::is_installed_program(&std::env::current_exe()?)?;
    let pipe = search_pipe(&args, installed)?;
    ensure!(!portable::is_elevated(), "请用普通用户权限启动搜索窗口");
    window::run(pipe)
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
