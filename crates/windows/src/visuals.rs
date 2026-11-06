use anyhow::{Context, Result, ensure};
use std::{mem::size_of, path::Path};
use windows::{
    Win32::{
        Foundation::{SIZE, STG_E_FILENOTFOUND},
        Graphics::Gdi::*,
        Storage::FileSystem::{FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_NORMAL},
        System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance},
        UI::{
            Shell::*,
            WindowsAndMessaging::{DI_NORMAL, DestroyIcon, DrawIconEx, HICON},
        },
    },
    core::HSTRING,
};

struct Bitmap(HBITMAP);
impl Drop for Bitmap {
    fn drop(&mut self) {
        // SAFETY: This wrapper owns the GDI bitmap and never wraps a shared-cache bitmap.
        unsafe {
            let _ = DeleteObject(HGDIOBJ(self.0.0));
        }
    }
}
struct Dc(HDC);
impl Drop for Dc {
    fn drop(&mut self) {
        // SAFETY: This wrapper owns a memory DC created by CreateCompatibleDC.
        unsafe {
            let _ = DeleteDC(self.0);
        }
    }
}
struct Icon(HICON);
impl Drop for Icon {
    fn drop(&mut self) {
        // SAFETY: SHGFI_ICON returns an owned icon handle.
        unsafe {
            let _ = DestroyIcon(self.0);
        }
    }
}
struct Selection {
    dc: HDC,
    old: HGDIOBJ,
}
impl Drop for Selection {
    fn drop(&mut self) {
        // SAFETY: The previous object and DC outlive this selection guard.
        unsafe {
            SelectObject(self.dc, self.old);
        }
    }
}

fn encode(width: u32, height: u32, pixels: &[u8]) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    let mut encoder = png::Encoder::new(&mut bytes, width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header()?;
    writer.write_image_data(pixels)?;
    writer.finish()?;
    Ok(bytes)
}

pub fn type_icon(name: &str, folder: bool, size: u32) -> Result<Vec<u8>> {
    ensure!((1..=256).contains(&size), "图标尺寸无效");
    let _apartment = crate::shell::Apartment::new()?;
    let extension = Path::new(name)
        .extension()
        .unwrap_or_default()
        .to_string_lossy();
    let virtual_name = if folder {
        "folder".into()
    } else if extension.is_empty() {
        "file".into()
    } else {
        format!("file.{extension}")
    };
    ensure!(
        virtual_name.len() <= 1024 && !virtual_name.contains('\0'),
        "扩展名无效"
    );
    let mut info = SHFILEINFOW::default();
    // SAFETY: The name is synthetic; USEFILEATTRIBUTES prevents source-volume access. Buffers are owned.
    unsafe {
        ensure!(
            SHGetFileInfoW(
                &HSTRING::from(virtual_name),
                if folder {
                    FILE_ATTRIBUTE_DIRECTORY
                } else {
                    FILE_ATTRIBUTE_NORMAL
                },
                Some(&mut info),
                size_of::<SHFILEINFOW>() as u32,
                SHGFI_ICON
                    | SHGFI_USEFILEATTRIBUTES
                    | if size <= 16 {
                        SHGFI_SMALLICON
                    } else {
                        SHGFI_LARGEICON
                    }
            ) != 0,
            "Windows 未返回格式图标"
        );
        let icon = Icon(info.hIcon);
        let dc = Dc(CreateCompatibleDC(None));
        ensure!(!dc.0.is_invalid(), "无法创建设备上下文");
        let bitmap_info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: size as i32,
                biHeight: -(size as i32),
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut pointer = std::ptr::null_mut();
        let bitmap = Bitmap(CreateDIBSection(
            Some(dc.0),
            &bitmap_info,
            DIB_RGB_COLORS,
            &mut pointer,
            None,
            0,
        )?);
        let old = SelectObject(dc.0, HGDIOBJ(bitmap.0.0));
        ensure!(!old.is_invalid(), "无法选择图标位图");
        let _selected = Selection { dc: dc.0, old };
        let length = (size * size * 4) as usize;
        std::ptr::write_bytes(pointer.cast::<u8>(), 0, length);
        DrawIconEx(
            dc.0,
            0,
            0,
            icon.0,
            size as i32,
            size as i32,
            0,
            None,
            DI_NORMAL,
        )?;
        let _ = GdiFlush();
        let black = std::slice::from_raw_parts(pointer.cast::<u8>(), length).to_vec();
        for pixel in
            std::slice::from_raw_parts_mut(pointer.cast::<u8>(), length).chunks_exact_mut(4)
        {
            pixel.copy_from_slice(&[255, 255, 255, 0]);
        }
        DrawIconEx(
            dc.0,
            0,
            0,
            icon.0,
            size as i32,
            size as i32,
            0,
            None,
            DI_NORMAL,
        )?;
        let _ = GdiFlush();
        let pixels = std::slice::from_raw_parts(pointer.cast::<u8>(), length);
        let mut rgba = Vec::with_capacity(length);
        for (dark, light) in black.chunks_exact(4).zip(pixels.chunks_exact(4)) {
            let alpha = 255
                - (0..3)
                    .map(|i| light[i].saturating_sub(dark[i]))
                    .max()
                    .unwrap();
            for i in [2, 1, 0] {
                rgba.push(if alpha == 0 {
                    0
                } else {
                    (u32::from(dark[i]) * 255 / u32::from(alpha)).min(255) as u8
                });
            }
            rgba.push(alpha);
        }
        encode(size, size, &rgba)
    }
}

fn bitmap_png(bitmap: HBITMAP, alpha_type: WTS_ALPHATYPE) -> Result<Vec<u8>> {
    let mut object = BITMAP::default();
    // SAFETY: The caller keeps its owned/shared bitmap alive; buffers and DC stay local to this call.
    unsafe {
        ensure!(
            GetObjectW(
                HGDIOBJ(bitmap.0),
                size_of::<BITMAP>() as i32,
                Some((&mut object as *mut BITMAP).cast())
            ) != 0,
            "无法读取系统位图"
        );
        ensure!(
            (1..=1024).contains(&object.bmWidth) && (1..=1024).contains(&object.bmHeight),
            "系统位图尺寸无效"
        );
        let (width, height) = (object.bmWidth as u32, object.bmHeight as u32);
        let dc = Dc(CreateCompatibleDC(None));
        ensure!(!dc.0.is_invalid(), "无法创建设备上下文");
        let mut info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: object.bmWidth,
                biHeight: -object.bmHeight,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut pixels = vec![0; (width * height * 4) as usize];
        ensure!(
            GetDIBits(
                dc.0,
                bitmap,
                0,
                height,
                Some(pixels.as_mut_ptr().cast()),
                &mut info,
                DIB_RGB_COLORS
            ) == height as i32,
            "系统位图读取不完整"
        );
        let opaque = alpha_type == WTSAT_RGB
            || alpha_type == WTSAT_UNKNOWN && pixels.chunks_exact(4).all(|p| p[3] == 0);
        for pixel in pixels.chunks_exact_mut(4) {
            pixel.swap(0, 2);
            if opaque {
                pixel[3] = 255;
            } else if pixel[3] > 0 {
                let alpha = u32::from(pixel[3]);
                for value in &mut pixel[..3] {
                    *value = (u32::from(*value) * 255 / alpha).min(255) as u8;
                }
            }
        }
        encode(width, height, &pixels)
    }
}

pub fn file_icon(path: &Path, size: u32) -> Result<Vec<u8>> {
    ensure!((1..=256).contains(&size), "图标尺寸无效");
    let _apartment = crate::shell::Apartment::new()?;
    // SAFETY: COM remains initialized on this thread until all interfaces and the owned bitmap are dropped.
    unsafe {
        let factory: IShellItemImageFactory =
            SHCreateItemFromParsingName(&HSTRING::from(path.as_os_str()), None)?;
        let bitmap = Bitmap(factory.GetImage(
            SIZE {
                cx: size as i32,
                cy: size as i32,
            },
            SIIGBF_ICONONLY,
        )?);
        bitmap_png(bitmap.0, WTSAT_ARGB)
    }
}

pub fn thumbnail(path: &Path, size: u32, cached_only: bool) -> Result<Option<Vec<u8>>> {
    ensure!((1..=1024).contains(&size), "缩略图尺寸无效");
    let _apartment = crate::shell::Apartment::new()?;
    // SAFETY: Interfaces and the borrowed shared bitmap remain alive within this apartment; no shared handle is deleted.
    unsafe {
        let item: IShellItem = SHCreateItemFromParsingName(&HSTRING::from(path.as_os_str()), None)?;
        let cache: IThumbnailCache =
            CoCreateInstance(&LocalThumbnailCache, None, CLSCTX_INPROC_SERVER)?;
        let mut shared = None;
        match cache.GetThumbnail(
            &item,
            size,
            if cached_only {
                WTS_INCACHEONLY
            } else {
                WTS_EXTRACT
            },
            Some(&mut shared),
            None,
            None,
        ) {
            Ok(()) => {
                let shared = shared.context("Windows 未返回缩略图")?;
                Ok(Some(bitmap_png(
                    shared.GetSharedBitmap()?,
                    shared.GetFormat()?,
                )?))
            }
            Err(error) if cached_only && error.code() == STG_E_FILENOTFOUND => Ok(None),
            Err(error)
                if error.code() == WTS_E_FAILEDEXTRACTION
                    || error.code() == WTS_E_NOSTORAGEPROVIDERTHUMBNAILHANDLER =>
            {
                Ok(None)
            }
            Err(error) => Err(error.into()),
        }
    }
}
