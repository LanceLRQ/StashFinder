use super::{Layout, Process};
use crate::ipc;
use anyhow::{Context, Result, ensure};
use std::{
    fs::{File, TryLockError},
    io::{Read, Seek, Write},
    path::Path,
    time::{Duration, Instant},
};
use windows::{
    Win32::{
        Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SEM_TIMEOUT},
        System::Pipes::{NMPWAIT_NOWAIT, WaitNamedPipeW},
    },
    core::HSTRING,
};

fn pipe_present(name: &str) -> Result<bool> {
    let name = HSTRING::from(name);
    // SAFETY: The pipe name stays alive for this nonblocking availability check.
    if unsafe { WaitNamedPipeW(&name, NMPWAIT_NOWAIT) }.as_bool() {
        return Ok(true);
    }
    let error = std::io::Error::last_os_error();
    match error.raw_os_error().map(|n| n as u32) {
        Some(n) if n == ERROR_FILE_NOT_FOUND.0 => Ok(false),
        Some(n) if n == ERROR_SEM_TIMEOUT.0 => Ok(true),
        _ => Err(error).context("无法检查便携后台连接"),
    }
}

fn try_exclusive(file: &File) -> Result<bool> {
    match file.try_lock() {
        Ok(()) => Ok(true),
        Err(TryLockError::WouldBlock) => Ok(false),
        Err(TryLockError::Error(e)) => Err(e.into()),
    }
}

pub struct SessionFiles {
    gate: File,
    clients: File,
}

impl SessionFiles {
    pub fn open(runtime: &Path) -> Result<Self> {
        std::fs::create_dir_all(runtime).context("便携目录不可写")?;
        let open = |name| {
            File::options()
                .read(true)
                .write(true)
                .create(true)
                .truncate(false)
                .open(runtime.join(name))
                .context("无法打开便携会话锁")
        };
        Ok(Self {
            gate: open("startup.lock")?,
            clients: open("clients.lock")?,
        })
    }

    fn unused(&self) -> Result<bool> {
        if !try_exclusive(&self.clients)? {
            return Ok(false);
        }
        self.clients.unlock()?;
        Ok(true)
    }

    /// Keep the returned gate until the pipe and index have both been released.
    pub async fn until_unused(self) -> Result<File> {
        loop {
            if try_exclusive(&self.gate)? {
                if self.unused()? {
                    return Ok(self.gate);
                }
                self.gate.unlock()?;
            }
            tokio::time::sleep(Duration::from_millis(150)).await;
        }
    }
}

pub struct PortableSession {
    files: SessionFiles,
    pipe: String,
}

impl PortableSession {
    pub fn start(layout: &Layout, confirm: impl FnOnce() -> Result<bool>) -> Result<Option<Self>> {
        let mut files = SessionFiles::open(&layout.runtime)?;
        files.gate.lock()?;
        files.clients.lock_shared()?;
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        if pipe_present(&layout.pipe)? {
            let status = runtime.block_on(ipc::status(&layout.pipe))?;
            let mut recorded_pid = [0; 4];
            ensure!(
                status.mode == "portable"
                    && files.gate.read_exact(&mut recorded_pid).is_ok()
                    && status.process_id == u32::from_le_bytes(recorded_pid),
                "此目录的后台不属于当前便携会话，请退出后台并使用同一便携包重新启动"
            );
        } else {
            let executable = layout.root.join("stashfinder-service.exe");
            ensure!(
                executable.is_file(),
                "缺少 stashfinder-service.exe，请完整解压便携包"
            );
            if !confirm()? {
                return Ok(None);
            }
            let args = ["--portable-shared".into(), std::process::id().to_string()];
            let Some(process) = Process::launch_elevated(&executable, &args)? else {
                return Ok(None);
            };
            files.gate.rewind()?;
            files.gate.write_all(&process.id().to_le_bytes())?;
            process.wait_ready(&runtime, &layout.pipe, "portable")?;
        }
        files.gate.unlock()?;
        Ok(Some(Self {
            files,
            pipe: layout.pipe.clone(),
        }))
    }

    fn leave(&self) -> Result<()> {
        self.files.clients.unlock()?;
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            if try_exclusive(&self.files.gate)? {
                // The backend owns the gate throughout shutdown. A new client may
                // legitimately join while this process is waiting for the last exit.
                let done = !self.files.unused()? || !pipe_present(&self.pipe)?;
                self.files.gate.unlock()?;
                if done {
                    return Ok(());
                }
            }
            ensure!(
                Instant::now() < deadline,
                "便携后台尚未退出，请稍候再移动目录"
            );
            std::thread::sleep(Duration::from_millis(100));
        }
    }
}

impl Drop for PortableSession {
    fn drop(&mut self) {
        if let Err(error) = self.leave() {
            super::show_error(&format!("{error:#}"));
        }
    }
}
