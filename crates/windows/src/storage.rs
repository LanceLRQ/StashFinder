use anyhow::{Context, Result, ensure};
use std::{os::windows::fs::MetadataExt, path::Path};
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

// Installation provisions a private, direct child of ProgramData. The GUI has no
// database access: all reads and writes go through the authenticated local pipe.
pub fn validate_service_root(root: &Path) -> Result<()> {
    let base = std::env::var_os("ProgramData").context("ProgramData unavailable")?;
    let base = std::fs::canonicalize(base)?;
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
