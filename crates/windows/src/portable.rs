use anyhow::{Context, Result, ensure};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    fs::File,
    os::windows::ffi::OsStrExt,
    path::{Path, PathBuf},
    time::Duration,
};
use windows::{
    Win32::{
        Foundation::{CloseHandle, ERROR_CANCELLED, HANDLE, WAIT_OBJECT_0, WAIT_TIMEOUT},
        System::Threading::{
            GetProcessId, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE,
            WaitForSingleObject,
        },
        UI::{
            Shell::{
                IsUserAnAdmin, SEE_MASK_NOASYNC, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW,
                ShellExecuteExW,
            },
            WindowsAndMessaging::SW_HIDE,
        },
    },
    core::HSTRING,
};

#[derive(Clone, Debug)]
pub struct Layout {
    pub root: PathBuf,
    pub index: PathBuf,
    pub config: PathBuf,
    pub local: PathBuf,
    pub runtime: PathBuf,
    pub pipe: String,
}
impl Layout {
    pub fn discover(executable: &Path) -> Result<Option<Self>> {
        let root = executable.parent().context("程序路径无父目录")?;
        if crate::storage::is_installed_program(executable)? {
            return Ok(None);
        }
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Marker {
            version: u32,
        }
        match std::fs::read(root.join("portable.json")) {
            Ok(bytes) => {
                let marker: Marker = serde_json::from_slice(&bytes).context("便携配置损坏")?;
                ensure!(marker.version == 1, "便携配置版本不受支持");
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                if !executable
                    .file_name()
                    .is_some_and(|n| n.eq_ignore_ascii_case("StashFinder.exe"))
                {
                    return Ok(None);
                }
            }
            Err(error) => return Err(error).context("无法读取便携配置"),
        }
        let root = std::fs::canonicalize(root)?;
        let key: Vec<u8> = root
            .as_os_str()
            .encode_wide()
            .flat_map(u16::to_le_bytes)
            .collect();
        let hash = Sha256::digest(&key);
        let suffix: String = hash.iter().map(|v| format!("{v:02x}")).collect();
        let data = root.join("data");
        Ok(Some(Self {
            root,
            index: data.join("index"),
            config: data.join("config"),
            local: data.join("local"),
            runtime: data.join("runtime"),
            pipe: format!(r"\\.\pipe\StashFinder.Portable.{suffix}"),
        }))
    }
    pub fn lock(&self) -> Result<File> {
        std::fs::create_dir_all(&self.runtime).context("便携目录不可写")?;
        let lock = File::options()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(self.runtime.join("instance.lock"))
            .context("无法打开便携会话锁")?;
        lock.try_lock().context("此目录的 StashFinder 已在运行")?;
        Ok(lock)
    }
}

pub fn quote_argument(value: &str) -> String {
    let mut quoted = String::from("\"");
    let mut slashes = 0;
    for character in value.chars() {
        if character == '\\' {
            slashes += 1;
            continue;
        }
        quoted.extend(std::iter::repeat_n(
            '\\',
            if character == '"' {
                slashes * 2 + 1
            } else {
                slashes
            },
        ));
        slashes = 0;
        quoted.push(character);
    }
    quoted.extend(std::iter::repeat_n('\\', slashes * 2));
    quoted.push('"');
    quoted
}

pub struct Backend {
    pub layout: Layout,
    pub owner: String,
    pub parent: u32,
    pub session: PathBuf,
}
impl Backend {
    pub fn parse(executable: &Path, args: &[String]) -> Result<Self> {
        ensure!(args.len() == 5, "便携后台参数不完整");
        let layout = Layout::discover(executable)?.context("缺少便携配置")?;
        ensure!(
            args[1] == layout.pipe && Path::new(&args[2]) == layout.index,
            "便携后台与程序目录不匹配"
        );
        let parent: u32 = args[3].parse().context("便携父进程无效")?;
        ensure!(parent > 0, "便携父进程无效");
        let session = std::fs::canonicalize(&args[4]).context("便携会话不存在")?;
        ensure!(
            session.is_file()
                && session.parent() == Some(std::fs::canonicalize(&layout.runtime)?.as_path()),
            "便携会话不属于此目录"
        );
        Ok(Self {
            layout,
            owner: args[0].clone(),
            parent,
            session,
        })
    }
}

pub struct Process(HANDLE);
// SAFETY: This uniquely owned kernel process handle supports waiting from any thread.
// Moving the wrapper transfers ownership; only Drop closes the handle.
unsafe impl Send for Process {}
impl Drop for Process {
    fn drop(&mut self) {
        // SAFETY: This wrapper owns the process handle and closes it once.
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}
impl Process {
    pub fn open(pid: u32) -> Result<Self> {
        // SAFETY: Only synchronization and query rights are requested for this process.
        Ok(Self(unsafe {
            OpenProcess(
                PROCESS_SYNCHRONIZE | PROCESS_QUERY_LIMITED_INFORMATION,
                false,
                pid,
            )?
        }))
    }
    pub fn launch_elevated(executable: &Path, args: &[String]) -> Result<Option<Self>> {
        let _apartment = crate::shell::Apartment::new()?;
        let executable = crate::file_operations::shell_path(executable)?;
        let verb = HSTRING::from("runas");
        let file = HSTRING::from(executable.as_os_str());
        let parameters = HSTRING::from(
            args.iter()
                .map(|arg| quote_argument(arg))
                .collect::<Vec<_>>()
                .join(" "),
        );
        let mut info = SHELLEXECUTEINFOW {
            cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
            fMask: SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NOASYNC,
            lpVerb: windows::core::PCWSTR(verb.as_ptr()),
            lpFile: windows::core::PCWSTR(file.as_ptr()),
            lpParameters: windows::core::PCWSTR(parameters.as_ptr()),
            nShow: SW_HIDE.0,
            ..Default::default()
        };
        // SAFETY: All strings and the writable result structure outlive ShellExecuteExW.
        // NOCLOSEPROCESS transfers the returned handle to this wrapper.
        match unsafe { ShellExecuteExW(&mut info) } {
            Err(error) if error.code() == ERROR_CANCELLED.to_hresult() => return Ok(None),
            result => result.context("无法启动索引后台")?,
        }
        ensure!(!info.hProcess.is_invalid(), "Windows 未返回索引后台进程");
        Ok(Some(Self(info.hProcess)))
    }
    pub fn id(&self) -> u32 {
        // SAFETY: The owned process handle is alive for the duration of this call.
        unsafe { GetProcessId(self.0) }
    }

    pub fn wait_ready(
        &self,
        runtime: &tokio::runtime::Runtime,
        pipe: &str,
        mode: &str,
    ) -> Result<()> {
        let deadline = std::time::Instant::now() + Duration::from_secs(20);
        loop {
            ensure!(
                !self.wait(0)?,
                "索引后台启动失败。请确认当前账户有权访问索引，且没有其他索引后台在运行。"
            );
            match runtime.block_on(crate::ipc::status(pipe)) {
                Ok(status) => {
                    ensure!(
                        status.process_id == self.id() && status.mode == mode,
                        "索引后台身份不匹配"
                    );
                    return Ok(());
                }
                Err(error)
                    if error
                        .downcast_ref::<tokio::time::error::Elapsed>()
                        .is_some() => {}
                Err(error) => return Err(error),
            }
            ensure!(std::time::Instant::now() < deadline, "索引后台启动超时");
            std::thread::sleep(Duration::from_millis(100));
        }
    }
    pub fn user_sid(&self) -> Result<String> {
        crate::ipc::process_user_sid(self.0)
    }
    pub fn wait(&self, timeout_ms: u32) -> Result<bool> {
        // SAFETY: The owned handle remains alive while Windows waits on it.
        match unsafe { WaitForSingleObject(self.0, timeout_ms) } {
            WAIT_OBJECT_0 => Ok(true),
            WAIT_TIMEOUT => Ok(false),
            _ => Err(std::io::Error::last_os_error()).context("无法等待便携进程"),
        }
    }
    pub async fn wait_session(self, session: &Path) -> Result<()> {
        while session.is_file() && !self.wait(0)? {
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
        Ok(())
    }
}

pub fn is_elevated() -> bool {
    // SAFETY: This is a read-only query of the calling process token.
    unsafe { IsUserAnAdmin().as_bool() }
}

pub struct DesktopInstance(HANDLE);

impl DesktopInstance {
    pub fn acquire(identifier: &str) -> Result<Option<Self>> {
        use windows::Win32::{
            Foundation::{ERROR_ALREADY_EXISTS, ERROR_SUCCESS, GetLastError, SetLastError},
            System::Threading::{CreateMutexExW, SYNCHRONIZATION_SYNCHRONIZE},
        };
        let sid = crate::ipc::current_user_sid()?;
        let name = HSTRING::from(format!(r"Local\{identifier}.Desktop.{sid}"));
        // SAFETY: The name is NUL-terminated, and the returned handle stays open for the desktop lifetime.
        unsafe {
            SetLastError(ERROR_SUCCESS);
            let instance = Self(
                CreateMutexExW(None, &name, 0, SYNCHRONIZATION_SYNCHRONIZE.0)
                    .context("无法创建桌面会话锁")?,
            );
            if GetLastError() == ERROR_ALREADY_EXISTS {
                return Ok(None);
            }
            Ok(Some(instance))
        }
    }
}

impl Drop for DesktopInstance {
    fn drop(&mut self) {
        // SAFETY: This instance uniquely owns the handle returned by CreateMutexExW.
        let _ = unsafe { CloseHandle(self.0) };
    }
}

pub fn enable_backup_read() -> Result<()> {
    use windows::Win32::{
        Foundation::{ERROR_SUCCESS, GetLastError, LUID, SetLastError},
        Security::{
            AdjustTokenPrivileges, LUID_AND_ATTRIBUTES, LookupPrivilegeValueW, SE_BACKUP_NAME,
            SE_PRIVILEGE_ENABLED, TOKEN_ADJUST_PRIVILEGES, TOKEN_PRIVILEGES, TOKEN_QUERY,
        },
        System::Threading::{GetCurrentProcess, OpenProcessToken},
    };
    // SAFETY: Enable only the existing read/backup privilege on this backend's token.
    // No account rights, source ACLs or restore/write privileges are changed.
    unsafe {
        let mut token = HANDLE::default();
        OpenProcessToken(
            GetCurrentProcess(),
            TOKEN_ADJUST_PRIVILEGES | TOKEN_QUERY,
            &mut token,
        )?;
        let result = (|| -> Result<()> {
            let mut luid = LUID::default();
            LookupPrivilegeValueW(None, SE_BACKUP_NAME, &mut luid)?;
            let privileges = TOKEN_PRIVILEGES {
                PrivilegeCount: 1,
                Privileges: [LUID_AND_ATTRIBUTES {
                    Luid: luid,
                    Attributes: SE_PRIVILEGE_ENABLED,
                }],
            };
            SetLastError(ERROR_SUCCESS);
            AdjustTokenPrivileges(token, false, Some(&privileges), 0, None, None)?;
            ensure!(
                GetLastError() == ERROR_SUCCESS,
                "当前账户未授予备份读取权限"
            );
            Ok(())
        })();
        CloseHandle(token)?;
        result.context("无法启用索引后台的备份读取权限")
    }
}

pub enum ClientKind {
    Desktop,
    QuickSearch,
}

pub fn confirm_index_backend(kind: ClientKind) -> Result<bool> {
    use windows::{
        Win32::UI::Controls::{
            TASKDIALOG_BUTTON, TASKDIALOGCONFIG, TASKDIALOGCONFIG_0, TD_INFORMATION_ICON,
            TDF_ALLOW_DIALOG_CANCELLATION, TaskDialogIndirect,
        },
        core::w,
    };

    const START: i32 = 100;
    let buttons = [
        TASKDIALOG_BUTTON {
            nButtonID: START,
            pszButtonText: w!("启动索引后台"),
        },
        TASKDIALOG_BUTTON {
            nButtonID: 101,
            pszButtonText: w!("稍后启动"),
        },
    ];
    let dialog = TASKDIALOGCONFIG {
        cbSize: std::mem::size_of::<TASKDIALOGCONFIG>() as u32,
        dwFlags: TDF_ALLOW_DIALOG_CANCELLATION,
        pszWindowTitle: w!("StashFinder"),
        pszMainInstruction: w!("磁盘索引需要管理员权限"),
        pszContent: match kind {
            ClientKind::Desktop => {
                w!("启动后台时，Windows 可能会请求授权。选择稍后启动仍可打开设置。")
            }
            ClientKind::QuickSearch => {
                w!("启动后台时，Windows 可能会请求授权。选择稍后启动将关闭此窗口。")
            }
        },
        Anonymous1: TASKDIALOGCONFIG_0 {
            pszMainIcon: TD_INFORMATION_ICON,
        },
        cButtons: buttons.len() as u32,
        pButtons: buttons.as_ptr(),
        nDefaultButton: START,
        ..Default::default()
    };
    let mut selected = 0;
    // SAFETY: The config, button array, static text and result live through the modal call.
    unsafe { TaskDialogIndirect(&dialog, Some(&mut selected), None, None) }
        .context("无法显示索引后台启动提示")?;
    Ok(selected == START)
}

pub fn show_error(message: &str) {
    use windows::Win32::UI::WindowsAndMessaging::{MB_ICONERROR, MB_OK, MessageBoxW};
    // SAFETY: Owned UTF-16 strings live through this synchronous message box call.
    unsafe {
        MessageBoxW(
            None,
            &HSTRING::from(message),
            &HSTRING::from("StashFinder"),
            MB_OK | MB_ICONERROR,
        );
    }
}
