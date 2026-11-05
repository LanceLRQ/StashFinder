use super::{Edit, execute, verify};
use anyhow::{Context, Result, ensure};
use stashfinder_core::file_operations::*;
use std::{
    os::windows::ffi::OsStringExt,
    path::PathBuf,
    sync::{Arc, atomic::AtomicBool},
};
use windows::{
    Win32::{
        System::{
            Com::{DVASPECT_CONTENT, FORMATETC, IDataObject, STGMEDIUM, TYMED_HGLOBAL},
            DataExchange::{GetClipboardSequenceNumber, RegisterClipboardFormatW},
            Memory::{GlobalLock, GlobalSize, GlobalUnlock},
            Ole::{
                CF_HDROP, DROPEFFECT_COPY, DROPEFFECT_MOVE, OleGetClipboard, OleSetClipboard,
                ReleaseStgMedium,
            },
        },
        UI::Shell::{DragQueryFileW, HDROP},
    },
    core::w,
};

struct Medium(STGMEDIUM);
impl Drop for Medium {
    fn drop(&mut self) {
        // SAFETY: Each successful GetData medium is released exactly once on its owning STA.
        unsafe {
            ReleaseStgMedium(&mut self.0);
        }
    }
}
fn format(value: u16) -> FORMATETC {
    FORMATETC {
        cfFormat: value,
        ptd: std::ptr::null_mut(),
        dwAspect: DVASPECT_CONTENT.0,
        lindex: -1,
        tymed: TYMED_HGLOBAL.0 as u32,
    }
}
fn read_object(data: &IDataObject) -> Result<(Vec<PathBuf>, bool)> {
    // SAFETY: All formats use HGLOBAL media; owned media outlive every buffer read.
    unsafe {
        let files = Medium(
            data.GetData(&format(CF_HDROP.0))
                .context("剪贴板没有可粘贴的真实文件")?,
        );
        ensure!(
            files.0.tymed == TYMED_HGLOBAL.0 as u32,
            "不支持的文件剪贴板格式"
        );
        let drop = HDROP(files.0.u.hGlobal.0);
        let count = DragQueryFileW(drop, u32::MAX, None) as usize;
        ensure!(
            count > 0 && count <= MAX_SELECTED,
            "单次最多粘贴 10,000 个文件或目录"
        );
        let mut paths = Vec::with_capacity(count);
        for index in 0..count {
            let length = DragQueryFileW(drop, index as u32, None);
            ensure!(length > 0, "剪贴板文件路径为空");
            let mut name = vec![0u16; length as usize + 1];
            ensure!(
                DragQueryFileW(drop, index as u32, Some(&mut name)) == length,
                "剪贴板文件路径读取不完整"
            );
            let path = PathBuf::from(std::ffi::OsString::from_wide(&name[..length as usize]));
            ensure!(path.is_absolute(), "剪贴板中包含非绝对路径");
            paths.push(path);
        }
        let preferred = RegisterClipboardFormatW(w!("Preferred DropEffect"));
        ensure!(preferred != 0, "无法读取剪贴板复制语义");
        let mut moving = false;
        if data.QueryGetData(&format(preferred as u16)).is_ok() {
            let effect = Medium(data.GetData(&format(preferred as u16))?);
            ensure!(
                effect.0.tymed == TYMED_HGLOBAL.0 as u32 && GlobalSize(effect.0.u.hGlobal) >= 4,
                "剪贴板复制语义损坏"
            );
            let pointer = GlobalLock(effect.0.u.hGlobal).cast::<u32>();
            ensure!(!pointer.is_null(), "无法读取剪贴板复制语义");
            let value = pointer.read_unaligned();
            let _ = GlobalUnlock(effect.0.u.hGlobal);
            moving = value == DROPEFFECT_MOVE.0;
            ensure!(
                moving || value == 0 || value == DROPEFFECT_COPY.0,
                "仅支持复制或移动文件"
            );
        }
        Ok((paths, moving))
    }
}

pub fn paste(
    target: PreparedTarget,
    owner: usize,
    cancel: Arc<AtomicBool>,
) -> Result<OperationReport> {
    let _apartment = crate::shell::Apartment::new()?;
    let folder = verify(&target, true)?;
    // SAFETY: The clipboard interface remains in this worker STA for the entire operation.
    let object = unsafe { OleGetClipboard() }?;
    let (paths, moving) = read_object(&object)?;
    let folders = paths
        .iter()
        .filter_map(|p| {
            super::inspect(p)
                .ok()
                .filter(|i| i.attributes & 16 != 0)
                .map(|_| p.clone())
        })
        .collect::<std::collections::HashSet<_>>();
    let mut unique = std::collections::HashSet::new();
    let paths = paths
        .into_iter()
        .filter(|p| {
            unique.insert(p.clone())
                && !p.ancestors().skip(1).any(|parent| folders.contains(parent))
        })
        .collect::<Vec<_>>();
    // SAFETY: Read-only sequence number, captured after delayed clipboard formats have rendered.
    let sequence = unsafe { GetClipboardSequenceNumber() };
    let mut report = execute(
        &paths,
        &vec![None; paths.len()],
        Some(target),
        &Edit::Paste {
            folder,
            move_files: moving,
        },
        owner,
        cancel,
    )?;
    // Never replace a clipboard that another application changed while copying.
    // SAFETY: This read-only Win32 call takes no pointers and has no ownership requirements.
    if moving && sequence != 0 && unsafe { GetClipboardSequenceNumber() } == sequence {
        let remaining = paths
            .into_iter()
            .zip(&report.items)
            .filter_map(|(p, r)| (r.state != OutcomeState::Completed).then_some(p))
            .collect::<Vec<_>>();
        let updated = if remaining.is_empty() {
            // SAFETY: Clearing only the unchanged source clipboard after all selected moves completed.
            unsafe { OleSetClipboard(None::<&IDataObject>) }.map_err(anyhow::Error::from)
        } else {
            crate::shell::cut_samples(&remaining)
        };
        if let Err(error) = updated {
            report.error = Some(format!("文件处理已结束，但剪贴板更新失败：{error:#}"));
        }
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reads_file_drop_paths_and_move_effect_without_touching_global_clipboard() {
        let _apartment = crate::shell::Apartment::new().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("中文.txt");
        std::fs::write(&path, b"keep").unwrap();
        let data = crate::shell::data_object(
            std::slice::from_ref(&path),
            windows::Win32::System::Ole::DROPEFFECT_MOVE,
        )
        .unwrap();
        let (paths, moving) = read_object(&data).unwrap();
        assert_eq!(paths, vec![path]);
        assert!(moving);
    }
    #[test]
    fn ambiguous_drop_effect_is_not_assumed_to_be_a_move() {
        let _apartment = crate::shell::Apartment::new().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("source");
        std::fs::write(&path, b"keep").unwrap();
        let data = crate::shell::data_object(&[path], DROPEFFECT_COPY | DROPEFFECT_MOVE).unwrap();
        assert!(read_object(&data).is_err());
    }
}
