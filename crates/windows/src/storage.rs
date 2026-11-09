use anyhow::{Context, Result, ensure};
use std::{
    fs::File,
    os::windows::fs::MetadataExt,
    path::{Path, PathBuf},
};
use windows::{
    Win32::{
        Foundation::{HLOCAL, LocalFree},
        Security::{
            ACCESS_ALLOWED_ACE, ACL,
            Authorization::{ConvertSidToStringSidW, GetNamedSecurityInfoW, SE_FILE_OBJECT},
            DACL_SECURITY_INFORMATION, GetAce, GetSecurityDescriptorControl,
            OWNER_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR, PSID,
        },
    },
    core::{HSTRING, PWSTR},
};

pub fn program_data() -> Result<PathBuf> {
    use windows::Win32::{
        System::Com::CoTaskMemFree,
        UI::Shell::{FOLDERID_ProgramData, KF_FLAG_DEFAULT, SHGetKnownFolderPath},
    };
    // SAFETY: The known folder API returns an allocated, null-terminated path.
    unsafe {
        let path = SHGetKnownFolderPath(&FOLDERID_ProgramData, KF_FLAG_DEFAULT, None)?;
        let result = path.to_string().map(PathBuf::from);
        CoTaskMemFree(Some(path.0.cast()));
        Ok(result?)
    }
}

pub fn lock_backend(root: &Path) -> Result<File> {
    let lock = File::options()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(root.join("backend.lock"))?;
    lock.try_lock().context("此索引目录已有后台在运行")?;
    Ok(lock)
}

pub fn use_admin_file_owner() -> Result<()> {
    use windows::Win32::{
        Foundation::{CloseHandle, HANDLE},
        Security::{
            CreateWellKnownSid, SetTokenInformation, TOKEN_ADJUST_DEFAULT, TOKEN_OWNER, TokenOwner,
            WinBuiltinAdministratorsSid,
        },
        System::Threading::{GetCurrentProcess, OpenProcessToken},
    };
    // SAFETY: The SID storage outlives SetTokenInformation, which copies the value.
    unsafe {
        let mut sid = [0u32; 17];
        let mut length = size_of_val(&sid) as u32;
        let owner = TOKEN_OWNER {
            Owner: PSID(sid.as_mut_ptr().cast()),
        };
        CreateWellKnownSid(
            WinBuiltinAdministratorsSid,
            None,
            Some(owner.Owner),
            &mut length,
        )?;
        let mut token = HANDLE::default();
        OpenProcessToken(GetCurrentProcess(), TOKEN_ADJUST_DEFAULT, &mut token)?;
        let result = SetTokenInformation(
            token,
            TokenOwner,
            (&owner as *const TOKEN_OWNER).cast(),
            size_of::<TOKEN_OWNER>() as u32,
        );
        CloseHandle(token)?;
        result.context("无法设置索引文件的默认所有者")
    }
}

// Installation provisions a private, direct child of ProgramData. The GUI has no
// database access: all reads and writes go through the authenticated local pipe.
pub fn validate_service_root(root: &Path) -> Result<()> {
    let base = std::fs::canonicalize(program_data()?)?;
    ensure!(root.is_absolute(), "服务数据目录必须为绝对路径");
    ensure!(
        std::fs::canonicalize(root.parent().context("数据目录无父目录")?)? == base,
        "服务数据目录必须直接位于 ProgramData 下"
    );
    ensure!(root.is_dir(), "服务数据目录必须预先创建");
    let mut pending = vec![root.to_path_buf()];
    while let Some(path) = pending.pop() {
        let metadata = std::fs::symlink_metadata(&path)?;
        ensure!(
            metadata.file_attributes() & 0x400 == 0,
            "服务数据目录不能包含重解析点"
        );
        // SAFETY: Windows supplies a self-contained descriptor. All ACE/SID pointers
        // are used only while it is alive; both allocated buffers are freed below.
        unsafe {
            let mut descriptor = PSECURITY_DESCRIPTOR::default();
            let mut owner = PSID::default();
            let mut acl: *mut ACL = std::ptr::null_mut();
            GetNamedSecurityInfoW(
                &HSTRING::from(path.as_os_str()),
                SE_FILE_OBJECT,
                OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
                Some(&mut owner),
                None,
                Some(&mut acl),
                None,
                &mut descriptor,
            )
            .ok()?;
            let result = (|| -> Result<()> {
                let sid_text = |sid: PSID| -> Result<String> {
                    let mut text = PWSTR::null();
                    ConvertSidToStringSidW(sid, &mut text)?;
                    let result = text.to_string();
                    LocalFree(Some(HLOCAL(text.0.cast())));
                    Ok(result?)
                };
                ensure!(
                    matches!(sid_text(owner)?.as_str(), "S-1-5-18" | "S-1-5-32-544"),
                    "服务数据所有者必须为 SYSTEM 或 Administrators"
                );
                ensure!(
                    !acl.is_null() && (*acl).AceCount == 2,
                    "服务数据 ACL 必须仅授予 SYSTEM 和 Administrators"
                );
                let mut trustees = Vec::new();
                for index in 0..u32::from((*acl).AceCount) {
                    let mut pointer = std::ptr::null_mut();
                    GetAce(acl, index, &mut pointer)?;
                    let ace = &*pointer.cast::<ACCESS_ALLOWED_ACE>();
                    ensure!(
                        ace.Header.AceType == 0
                            && ace.Header.AceSize as usize
                                >= std::mem::size_of::<ACCESS_ALLOWED_ACE>(),
                        "服务数据 ACL 类型无效"
                    );
                    ensure!(
                        ace.Mask == 0x1f01ff && ace.Header.AceFlags & 8 == 0,
                        "服务数据 ACL 权限无效"
                    );
                    if metadata.is_dir() {
                        ensure!(ace.Header.AceFlags & 3 == 3, "目录 ACL 必须向子项继承");
                    }
                    let sid = PSID(std::ptr::addr_of!(ace.SidStart).cast_mut().cast());
                    trustees.push(sid_text(sid)?);
                }
                trustees.sort();
                ensure!(
                    trustees == ["S-1-5-18", "S-1-5-32-544"],
                    "服务数据 ACL 受托人无效"
                );
                if path == root {
                    let (mut control, mut revision) = (0, 0);
                    GetSecurityDescriptorControl(descriptor, &mut control, &mut revision)?;
                    ensure!(control & 0x1000 != 0, "服务数据根目录必须禁用继承");
                }
                Ok(())
            })();
            LocalFree(Some(HLOCAL(descriptor.0)));
            result.with_context(|| format!("数据目录权限检查失败：{}", path.display()))?;
        }
        if metadata.is_dir() {
            for child in std::fs::read_dir(path)? {
                pending.push(child?.path());
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn installed_backends_cannot_write_the_same_catalog_concurrently() {
        let directory = tempfile::tempdir().unwrap();
        let first = super::lock_backend(directory.path()).unwrap();
        assert!(super::lock_backend(directory.path()).is_err());
        drop(first);
        assert!(super::lock_backend(directory.path()).is_ok());
    }
}
