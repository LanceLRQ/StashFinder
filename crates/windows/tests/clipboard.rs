use std::{path::PathBuf, process::Command};
use windows::{
    Win32::{
        System::{
            Com::{
                CLSCTX_INPROC_SERVER, CoCreateInstance, DVASPECT_CONTENT, FORMATETC, TYMED_HGLOBAL,
            },
            Ole::{OleGetClipboard, OleInitialize, OleUninitialize, ReleaseStgMedium},
        },
        UI::Shell::{
            DragQueryFileW, FOF_NOCONFIRMATION, FOF_NOERRORUI, FOF_SILENT, FileOperation, HDROP,
            IFileOperation, IShellItem, SHCreateItemFromParsingName,
        },
    },
    core::HSTRING,
};

#[test]
#[ignore = "Uses the desktop clipboard; run explicitly without concurrent clipboard use"]
fn clipboard_copy_across_processes() {
    if let Some(root) = std::env::var_os("STASHFINDER_CLIPBOARD_TEST_ROOT") {
        let root = PathBuf::from(root);
        // SAFETY: The test owns this STA and all COM objects. Only the generated fixture
        // is accepted, and the destination is a new temporary directory owned by the parent.
        unsafe {
            OleInitialize(None).unwrap();
            {
                let data = OleGetClipboard().unwrap();
                let format = FORMATETC {
                    cfFormat: 15,
                    dwAspect: DVASPECT_CONTENT.0,
                    lindex: -1,
                    tymed: TYMED_HGLOBAL.0 as u32,
                    ..Default::default()
                };
                let mut medium = data.GetData(&format).unwrap();
                let drop = HDROP(medium.u.hGlobal.0);
                assert_eq!(DragQueryFileW(drop, u32::MAX, None), 1);
                let length = DragQueryFileW(drop, 0, None);
                let mut text = vec![0u16; length as usize + 1];
                DragQueryFileW(drop, 0, Some(&mut text));
                let source = PathBuf::from(String::from_utf16(&text[..length as usize]).unwrap());
                ReleaseStgMedium(&mut medium);
                assert_eq!(source, root.join("source.txt"));
                assert_eq!(
                    std::fs::read_to_string(&source).unwrap(),
                    "cross-process sample"
                );
                let destination: IShellItem = SHCreateItemFromParsingName(
                    &HSTRING::from(root.join("target").as_os_str()),
                    None,
                )
                .unwrap();
                let operation: IFileOperation =
                    CoCreateInstance(&FileOperation, None, CLSCTX_INPROC_SERVER).unwrap();
                operation
                    .SetOperationFlags(FOF_NOCONFIRMATION | FOF_NOERRORUI | FOF_SILENT)
                    .unwrap();
                operation.CopyItems(&data, &destination).unwrap();
                operation.PerformOperations().unwrap();
                assert!(!operation.GetAnyOperationsAborted().unwrap().as_bool());
            }
            OleUninitialize();
        }
        assert_eq!(
            std::fs::read_to_string(root.join("target/source.txt")).unwrap(),
            "cross-process sample"
        );
        return;
    }
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source.txt");
    std::fs::write(&source, "cross-process sample").unwrap();
    std::fs::create_dir(root.path().join("target")).unwrap();
    stashfinder_windows::shell::copy_samples(std::slice::from_ref(&source)).unwrap();
    let output = Command::new(std::env::current_exe().unwrap())
        .args([
            "--ignored",
            "--exact",
            "clipboard_copy_across_processes",
            "--nocapture",
        ])
        .env("STASHFINDER_CLIPBOARD_TEST_ROOT", root.path())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        std::fs::read_to_string(source).unwrap(),
        "cross-process sample"
    );
}
