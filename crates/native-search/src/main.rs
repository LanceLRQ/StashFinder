#![windows_subsystem = "windows"]

mod drag;
mod search;
mod window;

use anyhow::{Result, bail, ensure};
use stashfinder_windows::{ipc, portable};

fn main() {
    if let Err(error) = run() {
        portable::show_error(&format!("{error:#}"));
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let pipe = match args.as_slice() {
        [] => ipc::DEFAULT_PIPE.to_owned(),
        [flag, name] if flag == "--pipe" && name.starts_with(r"\\.\pipe\StashFinder.") => {
            name.clone()
        }
        _ => bail!(r"用法：stashfinder-native-search.exe [--pipe \\.\pipe\StashFinder.名称]"),
    };
    ensure!(!portable::is_elevated(), "请用普通用户权限启动搜索窗口");
    window::run(pipe)
}
