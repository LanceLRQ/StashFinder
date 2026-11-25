use super::*;
use crate::visuals::{
    self, Image, Kind, Paths,
    batch::{self, BatchAction, BatchOptions, BatchState},
};
use anyhow::Context;
use base64::{Engine, engine::general_purpose::STANDARD};
use std::{
    collections::{HashMap, HashSet},
    io::Cursor,
    sync::Arc,
};
use windows::Win32::{
    Graphics::Gdi::{
        BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateDIBSection, DIB_RGB_COLORS, DeleteObject,
        GetSysColor,
    },
    System::SystemServices::{SS_BITMAP, SS_CENTERIMAGE},
};

pub(super) const PICTURE_STYLE: u32 = SS_BITMAP.0 | SS_CENTERIMAGE.0;
pub(super) const REBUILD_THUMBNAIL: usize = 315;
type Key = (TabId, u64, usize);

#[derive(Default)]
pub(super) struct Images {
    list: Cell<HIMAGELIST>,
    size: Cell<i32>,
    generation: Cell<u64>,
    rows: RefCell<HashMap<Key, i32>>,
    pending: RefCell<HashSet<Key>>,
    bitmap: Cell<HBITMAP>,
    preview_key: Cell<Option<Key>>,
    preview_request: RefCell<Option<String>>,
    sequence: Cell<u64>,
    batch_line: RefCell<String>,
}
impl Drop for Images {
    fn drop(&mut self) {
        let list = self.list.get();
        let bitmap = self.bitmap.get();
        // SAFETY: The owner window has been destroyed, and these handles were allocated by Images.
        unsafe {
            if !list.is_invalid() {
                let _ = ImageList_Destroy(Some(list));
            }
            if !bitmap.is_invalid() {
                let _ = DeleteObject(bitmap.into());
            }
        }
    }
}

struct Raster {
    width: usize,
    height: usize,
    channels: usize,
    data: Vec<u8>,
}
fn raster(image: &Image) -> Result<Option<Raster>> {
    let Some(data) = &image.data_url else {
        return Ok(None);
    };
    let bytes = STANDARD.decode(
        data.strip_prefix("data:image/png;base64,")
            .context("图片格式无效")?,
    )?;
    let mut decoder = png::Decoder::new(Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = decoder.read_info()?;
    let (width, height) = reader.info().size();
    ensure!(
        (1..=1024).contains(&width) && (1..=1024).contains(&height),
        "图片尺寸无效"
    );
    let mut buffer = vec![0; reader.output_buffer_size().context("图片过大")?];
    let info = reader.next_frame(&mut buffer)?;
    let channels = info.color_type.samples();
    buffer.truncate(info.buffer_size());
    Ok(Some(Raster {
        width: width as usize,
        height: height as usize,
        channels,
        data: buffer,
    }))
}

fn bitmap(image: &Raster, size: i32) -> Result<HBITMAP> {
    let header = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: size,
            biHeight: -size,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut address = std::ptr::null_mut();
    let size = size as usize;
    let (background, surface) =
        // SAFETY: GDI allocates a writable DIB section for exactly the specified 32-bit rows.
        unsafe { (GetSysColor(COLOR_WINDOW), CreateDIBSection(None, &header, DIB_RGB_COLORS, &mut address, None, 0)?) };
    let rgb = [
        background as u8,
        (background >> 8) as u8,
        (background >> 16) as u8,
    ];
    // SAFETY: The allocation above is retained through the entire write.
    let pixels = unsafe { std::slice::from_raw_parts_mut(address.cast::<[u8; 4]>(), size * size) };
    pixels.fill([rgb[2], rgb[1], rgb[0], 255]);
    let factor = image.width.max(image.height);
    let width = (image.width * size / factor).max(1);
    let height = (image.height * size / factor).max(1);
    let left = (size - width) / 2;
    let top = (size - height) / 2;
    for y in 0..height {
        for x in 0..width {
            let position = (y * image.height / height * image.width + x * image.width / width)
                * image.channels;
            let pixel = &image.data[position..position + image.channels];
            let (color, alpha) = match *pixel {
                [v] => ([v, v, v], 255),
                [v, a] => ([v, v, v], a),
                [r, g, b] => ([r, g, b], 255),
                [r, g, b, a] => ([r, g, b], a),
                _ => unreachable!("PNG color channels"),
            };
            let mixed = |i: usize| {
                ((u16::from(color[i]) * u16::from(alpha)
                    + u16::from(rgb[i]) * u16::from(255 - alpha))
                    / 255) as u8
            };
            pixels[(top + y) * size + left + x] = [mixed(2), mixed(1), mixed(0), 255];
        }
    }
    Ok(surface)
}

fn batch_status(job: &batch::BatchStatus) -> String {
    let state = match job.state {
        BatchState::Running => "运行中",
        BatchState::Paused => "已暂停",
        BatchState::Offline => "等待分区重连",
        BatchState::WaitingForIndex => "等待索引释放磁盘",
        BatchState::Quota => "达到缓存上限",
        BatchState::Full => "磁盘空间不足",
        BatchState::Cancelling => "正在取消",
        BatchState::Cancelled => "已取消",
        BatchState::Completed => "已完成",
        BatchState::Failed => "失败",
    };
    format!(
        "{} · {state} · {} / {}",
        job.label,
        job.saved + job.skipped + job.unsupported + job.failed,
        job.total
            .map(|n| n.to_string())
            .unwrap_or_else(|| "正在计数".into())
    )
}

impl Desktop {
    fn paths(&self) -> Paths {
        Paths::new(
            self.config_path
                .parent()
                .expect("configured preferences folder"),
            &self.local_path,
        )
    }
    pub(super) fn visible_range(&self) -> (usize, usize) {
        if self.images.size.get() > 18
            && let Some((start, end)) = self.view_hint.get()
        {
            return (start, end.max(start + 1).min(start + 192));
        }
        let top = send(self.list, LVM_GETTOPINDEX, 0, 0).max(0) as usize;
        (
            top,
            top + (send(self.list, LVM_GETCOUNTPERPAGE, 0, 0).max(1) as usize).min(192),
        )
    }
    fn image_id(&self, prefix: &str) -> String {
        let next = self.images.sequence.get().wrapping_add(1);
        self.images.sequence.set(next);
        format!("native-{prefix}-{next}")
    }
    pub(super) fn set_images(&self, size: i32) {
        let reset = self.images.size.get() != size || self.images.rows.borrow().len() >= 384;
        if !reset {
            return;
        }
        // SAFETY: The UI owns this resource, and releases the previous one after detaching it.
        let next = unsafe {
            if size == 0 {
                HIMAGELIST::default()
            } else {
                ImageList_Create(size, size, ILC_COLOR32, 16, 16)
            }
        };
        send(
            self.list,
            LVM_SETIMAGELIST,
            LVSIL_SMALL as usize,
            if size == 18 { next.0 } else { 0 },
        );
        send(
            self.list,
            LVM_SETIMAGELIST,
            LVSIL_NORMAL as usize,
            if size > 18 { next.0 } else { 0 },
        );
        let previous = self.images.list.replace(next);
        if !previous.is_invalid() {
            // SAFETY: No control retains the previous image list.
            let _ = unsafe { ImageList_Destroy(Some(previous)) };
        }
        self.images.size.set(size);
        self.images
            .generation
            .set(self.images.generation.get().wrapping_add(1));
        self.images.rows.borrow_mut().clear();
        self.images.pending.borrow_mut().clear();
        if !next.is_invalid() {
            // SAFETY: The standard icon is shared and copied into the owned list.
            unsafe {
                if let Ok(icon) = LoadIconW(None, IDI_APPLICATION) {
                    ImageList_ReplaceIcon(next, -1, icon);
                }
            }
        }
    }
    pub(super) fn row_image(&self, row: usize) -> i32 {
        if self.images.size.get() == 0 {
            return -1;
        }
        let tabs = self.tabs.borrow();
        let tab = tabs.active();
        self.images
            .rows
            .borrow()
            .get(&(tab.id, tab.epoch, row))
            .copied()
            .unwrap_or(0)
    }
    pub(super) fn images_tick(&self) {
        let status = batch::status(&self.visuals);
        let next = status
            .as_ref()
            .filter(|job| !job.state.finished())
            .map(batch_status)
            .unwrap_or_default();
        if *self.images.batch_line.borrow() != next {
            let was_active = !self.images.batch_line.borrow().is_empty();
            *self.images.batch_line.borrow_mut() = next.clone();
            if next.is_empty()
                && was_active
                && let Some(job) = &status
            {
                self.set_images(0);
                self.images.preview_key.set(None);
                self.present(true);
                self.status(&batch_status(job));
            } else if !next.is_empty() && matches!(self.tabs.borrow().active().pane, Page::Tasks) {
                set_text(self.status, &next);
            }
        }
        let size = self.images.size.get();
        if size == 0 || self.images.list.get().is_invalid() {
            return;
        }
        if self.images.rows.borrow().len() >= 384 {
            self.set_images(size);
        }
        let (tab, epoch) = {
            let tabs = self.tabs.borrow();
            (tabs.active().id, tabs.active().epoch)
        };
        let (start, end) = self.visible_range();
        let total = send(self.list, LVM_GETITEMCOUNT, 0, 0).max(0) as usize;
        let available = 6_usize.saturating_sub(self.images.pending.borrow().len());
        let mut requests = vec![];
        for pos in start..end.min(total) {
            if requests.len() == available {
                break;
            }
            let key = (tab, epoch, pos);
            if self.images.rows.borrow().contains_key(&key)
                || self.images.pending.borrow().contains(&key)
            {
                continue;
            }
            if let Some(r) = self.record(pos) {
                self.images.pending.borrow_mut().insert(key);
                requests.push((key, r));
            }
        }
        let generation = self.images.generation.get();
        for (key, record) in requests {
            let request = self.image_id("row");
            let visuals = self.visuals.clone();
            let paths = self.paths();
            self.spawn(
                async move {
                    let folder = record.row.kind == "folder";
                    let kind = if size > 18 && !folder {
                        Kind::Thumbnail
                    } else {
                        Kind::Icon
                    };
                    let mut image = if record.online || matches!(kind, Kind::Thumbnail) {
                        visuals::preview(
                            &visuals,
                            paths,
                            request,
                            record.reference,
                            kind,
                            size as u32,
                            Arc::new(|_| {}),
                        )
                        .await
                        .ok()
                    } else {
                        None
                    };
                    if image.as_ref().is_none_or(|i| i.data_url.is_none()) {
                        image = visuals::type_icon(&visuals, record.row.name, folder, size as u32)
                            .await
                            .ok();
                    }
                    image.and_then(|image| raster(&image).ok().flatten())
                },
                move |app, raster| {
                    if generation != app.images.generation.get() {
                        return;
                    }
                    app.images.pending.borrow_mut().remove(&key);
                    let index = raster.and_then(|p| bitmap(&p, size).ok()).map_or(0, |bm| {
                        // SAFETY: The image list copies the bitmap before the owned bitmap is deleted.
                        unsafe {
                            let index = ImageList_Add(app.images.list.get(), bm, None);
                            let _ = DeleteObject(bm.into());
                            index.max(0)
                        }
                    });
                    app.images.rows.borrow_mut().insert(key, index);
                    let active = {
                        let tabs = app.tabs.borrow();
                        (tabs.active().id, tabs.active().epoch) == (key.0, key.1)
                    };
                    if active {
                        send(app.list, LVM_REDRAWITEMS, key.2, key.2 as isize);
                    }
                },
            );
        }
    }
    pub(super) fn has_picture(&self) -> bool {
        !self.images.bitmap.get().is_invalid()
    }
    fn display_picture(&self, picture: HBITMAP) {
        let previous = self.images.bitmap.replace(picture);
        let returned = HBITMAP(send(
            self.picture,
            STM_SETIMAGE,
            IMAGE_BITMAP.0 as usize,
            picture.0 as isize,
        ) as *mut _);
        // SAFETY: The detached original and any separate copy created by the static control are owned here.
        unsafe {
            if !returned.is_invalid() && returned != previous {
                let _ = DeleteObject(returned.into());
            }
            if !previous.is_invalid() {
                let _ = DeleteObject(previous.into());
            }
        }
    }
    pub(super) fn update_preview(&self) {
        let (home, enabled, tab, epoch) = {
            let tabs = self.tabs.borrow();
            let tab = tabs.active();
            (
                matches!(tab.pane, Page::Overview),
                tab.preview && matches!(tab.pane, Page::Browse(_) | Page::Search(_)),
                tab.id,
                tab.epoch,
            )
        };
        if home {
            set_text(self.preview, &self.asset_detail());
        }
        let selected = enabled.then(|| self.single()).flatten();
        let key = selected.map(|i| (tab, epoch, i));
        if key.is_some() && key == self.images.preview_key.get() {
            return;
        }
        if let Some(request) = self.images.preview_request.take() {
            visuals::cancel(&self.visuals, &request);
        }
        self.images.preview_key.set(None);
        let had_picture = self.has_picture();
        if had_picture {
            self.display_picture(HBITMAP::default());
        }
        if !enabled {
            return;
        }
        let record = selected.and_then(|i| self.record(i));
        let Some(record) = record else {
            let count = send(self.list, LVM_GETSELECTEDCOUNT, 0, 0);
            let desc = if count > 1 {
                format!("已选择 {count} 项")
            } else if count == 1 {
                "正在读取文件详情…".into()
            } else {
                "选择文件可预览".into()
            };
            set_text(self.preview, &desc);
            if had_picture {
                self.layout();
            }
            return;
        };
        let desc = format!(
            "{}\r\n\r\n{}\r\n分区：{} · {}\r\n大小：{}\r\n创建：{}\r\n修改：{}{}",
            record.row.name,
            record.row.path,
            record.source.name,
            if record.history {
                "历史记录"
            } else if record.online {
                "在线"
            } else {
                "离线"
            },
            record.row.size.as_deref().unwrap_or("未知"),
            files::file_date(record.row.created.as_deref()),
            files::file_date(record.row.modified.as_deref()),
            if record.partial {
                "\r\n部分索引"
            } else {
                ""
            }
        );
        set_text(self.preview, &desc);
        self.images.preview_key.set(key);
        if had_picture {
            self.layout();
        }
        if record.row.kind == "folder" {
            return;
        }
        set_text(self.preview, &format!("{desc}\r\n\r\n正在读取缩略图…"));
        let request = self.image_id("preview");
        self.images.preview_request.replace(Some(request.clone()));
        let paths = self.paths();
        let visuals = self.visuals.clone();
        self.spawn(
            async move {
                let image = visuals::preview(
                    &visuals,
                    paths,
                    request,
                    record.reference,
                    Kind::Thumbnail,
                    256,
                    Arc::new(|_| {}),
                )
                .await
                .map_err(anyhow::Error::msg)?;
                let raster = raster(&image)?;
                Ok::<_, anyhow::Error>((image, raster))
            },
            move |app, result| {
                if key != app.images.preview_key.get() {
                    return;
                }
                app.images.preview_request.take();
                match result {
                    Ok((image, Some(raster))) => {
                        match bitmap(&raster, 232) {
                            Ok(bm) => app.display_picture(bm),
                            Err(e) => {
                                set_text(app.preview, &format!("{desc}\r\n缩略图无法显示：{e:#}"));
                                return;
                            }
                        }
                        let state = if image.historical {
                            "缓存缩略图（历史）"
                        } else if image.source == "local_cache" {
                            "缓存缩略图"
                        } else {
                            "缩略图"
                        };
                        set_text(
                            app.preview,
                            &format!(
                                "{desc}\r\n\r\n{state}{}",
                                image.warning.map_or(String::new(), |w| format!(" · {w}"))
                            ),
                        );
                    }
                    Ok((image, None)) => set_text(
                        app.preview,
                        &format!(
                            "{desc}\r\n\r\n{}",
                            if image.historical {
                                "离线，暂无缓存缩略图"
                            } else {
                                "此格式暂无缩略图"
                            }
                        ),
                    ),
                    Err(e) => {
                        set_text(app.preview, &format!("{desc}\r\n\r\n缩略图读取失败：{e:#}"))
                    }
                }
                app.layout();
            },
        );
    }

    pub(super) fn rebuild_thumbnail(&self) {
        let Some(record) = self
            .single()
            .and_then(|i| self.record(i))
            .filter(|r| r.online && r.row.kind != "folder")
        else {
            return;
        };
        let visuals = self.visuals.clone();
        let paths = self.paths();
        let request = self.image_id("ffmpeg");
        self.status("正在用 FFmpeg 补建缩略图…");
        self.spawn(async move { visuals::ffmpeg::rebuild(&visuals, paths, request, record.reference).await }, |app, result| {
            match result {
                Ok(()) => {
                    app.images.preview_key.set(None);
                    app.set_images(0);
                    app.present(true);
                    app.status("已补建缩略图");
                }
                Err(e) => info(app.hwnd, "缩略图补建失败", &e),
            }
        });
    }
    pub(super) fn cache_settings(&self) {
        let visuals = self.visuals.clone();
        let paths = self.paths();
        self.spawn(
            async move { visuals::cache_status(&visuals, paths).await },
            |app, result| {
                let data = match result {
                    Ok(data) => data,
                    Err(e) => {
                        info(app.hwnd, "图片缓存读取失败", &e);
                        return;
                    }
                };
                let stats = data.stats;
                let draft = data.thumbnail_types;
                let title = format!(
                    "图片缓存 · {} 项 · {:.2} GB",
                    stats.files,
                    stats.used_bytes as f64 / 1_000_000_000.0
                );
                let choices = vec!["保存设置".into(), "清空缓存".into(), "查看剩余空间".into()];
                let Some(mut fields) = app.ask(
                    &title,
                    &[
                        Field::Choice("操作", choices, 0),
                        Field::Edit(
                            "容量上限（GB）",
                            stats.limit_bytes.map_or(String::new(), |n| {
                                (n as f64 / 1_000_000_000.0).to_string()
                            }),
                        ),
                        Field::Check("不限容量", stats.limit_bytes.is_none()),
                        Field::Check("图片", draft.images),
                        Field::Check("视频", draft.videos),
                        Field::Check("文档", draft.documents),
                        Field::Edit("自定义扩展名", draft.extensions.join(",")),
                    ],
                ) else {
                    return;
                };
                let action = fields.choice();
                if action == 2 {
                    info(
                        app.hwnd,
                        "图片缓存占用",
                        &format!(
                            "已用 {:.2} GB\n剩余空间：{}\n等待 {} · 处理中 {}",
                            stats.used_bytes as f64 / 1_000_000_000.0,
                            stats.free_bytes.map_or("未知".into(), |bytes| format!(
                                "{:.2} GB",
                                bytes as f64 / 1_000_000_000.0
                            )),
                            data.waiting,
                            data.active
                        ),
                    );
                    return;
                }
                if action == 1
                    && !confirm(
                        app.hwnd,
                        "清空图片缓存",
                        "删除图片缓存中的缩略图？离线图片需要重新生成后才能预览。",
                    )
                {
                    return;
                }
                let capacity = fields.text();
                let unlimited = fields.check();
                let (images, videos, documents) = (fields.check(), fields.check(), fields.check());
                let extension_text = fields.text();
                let progress = app.sender.clone();
                let mut last = std::time::Instant::now();
                let notify = move |work: crate::visual_cache::Progress| {
                    if work.processed == work.total || last.elapsed().as_millis() >= 100 {
                        last = std::time::Instant::now();
                        let _ = progress.send(Box::new(move |app| {
                            app.status(&format!(
                                "正在整理图片缓存 {} / {}",
                                work.processed, work.total
                            ))
                        }));
                    }
                };
                let paths = app.paths();
                let visuals = app.visuals.clone();
                app.status("正在更新图片缓存…");
                app.spawn(
                    async move {
                        if action == 1 {
                            return visuals::cache_clear(&visuals, paths, notify)
                                .await
                                .map_err(anyhow::Error::msg);
                        }
                        let mut types = stashfinder_core::file_operations::ThumbnailTypes {
                            images,
                            videos,
                            documents,
                            extensions: dialogs::extensions(&extension_text)?,
                        };
                        types.normalize()?;
                        let limit = if unlimited {
                            None
                        } else {
                            let bytes =
                                capacity.trim().parse::<f64>().context("容量需要填写数字")?
                                    * 1_000_000_000.0;
                            ensure!(
                                bytes.is_finite() && (1.0..=u64::MAX as f64).contains(&bytes),
                                "容量上限超出范围"
                            );
                            Some(bytes.round() as u64)
                        };
                        if stats.limit_bytes != limit {
                            visuals::cache_limit(&visuals, paths.clone(), limit, notify)
                                .await
                                .map_err(anyhow::Error::msg)?;
                        }
                        visuals::thumbnail_types(&visuals, paths, types)
                            .await
                            .map_err(anyhow::Error::msg)
                    },
                    |app, result| match result {
                        Ok(_) => {
                            app.images.preview_key.set(None);
                            app.set_images(0);
                            app.present(true);
                            app.status("图片缓存已更新");
                        }
                        Err(e) => info(app.hwnd, "图片缓存未能完成更新", &format!("{e:#}")),
                    },
                );
            },
        );
    }
    pub(super) fn analyze_files(&self, selected: Option<stashfinder_core::assets::Asset>) {
        let mounted = self
            .snapshot
            .borrow()
            .assets
            .iter()
            .filter(|a| a.registration.state == "ready")
            .cloned()
            .collect::<Vec<_>>();
        if mounted.is_empty() {
            info(self.hwnd, "分析文件", "尚无已登记分区。");
            return;
        }
        let selected_index = selected
            .as_ref()
            .and_then(|s| {
                mounted
                    .iter()
                    .position(|a| a.registration.partition_id == s.registration.partition_id)
            })
            .unwrap_or(0);
        let folder = {
            let tabs = self.tabs.borrow();
            if let Page::Browse(b) = &tabs.active().pane {
                b.location().zip(b.directory.as_ref()).and_then(|(p, d)| {
                    p.parent
                        .as_ref()
                        .map(|parent| (p.volume.id.clone(), parent.clone(), d.generation.clone()))
                })
            } else {
                None
            }
        };
        let mut fields = vec![
            Field::Choice(
                "分区",
                mounted
                    .iter()
                    .map(|r| r.registration.name.clone())
                    .collect(),
                selected_index,
            ),
            Field::Check("预生成缩略图", true),
            Field::Check("分析视频容器（ffprobe）", false),
            Field::Choice("已有结果", vec!["跳过已有".into(), "全部重建".into()], 0),
        ];
        if folder.is_some() {
            fields.push(Field::Check("仅当前目录", true));
        }
        let Some(mut form) = self.ask("分析文件", &fields) else {
            return;
        };
        let r = &mounted[form.choice()].registration;
        let thumbnails = form.check();
        let analyse = form.check();
        let rebuild = form.choice() == 1;
        let use_folder = folder.is_some() && form.check();
        let scoped = folder.filter(|(id, _, _)| use_folder && id == &r.partition_id);
        if use_folder && scoped.is_none() {
            info(self.hwnd, "分析文件", "当前目录属于另一个分区。");
            return;
        }
        if !analyse && !thumbnails {
            info(self.hwnd, "分析文件", "请选择一项操作。");
            return;
        }
        let options = BatchOptions {
            partition_id: r.partition_id.clone(),
            filesystem_id: r.filesystem_id.clone(),
            label: r.name.clone(),
            folder: scoped.as_ref().map(|(_, folder, _)| folder.clone()),
            generation: scoped.map(|(_, _, generation)| generation),
            rebuild,
            analyse,
            thumbnails,
        };
        let paths = self.paths();
        let visuals = self.visuals.clone();
        self.spawn(
            async move { batch::start(&visuals, paths, options, || {}).await },
            |app, result| match result {
                Ok(()) => {
                    app.open_tab(Page::Tasks);
                    app.status("已启动文件分析任务");
                }
                Err(e) => info(app.hwnd, "任务启动失败", &e),
            },
        );
    }
    pub(super) fn batch_progress(&self) {
        loop {
            let Some(job) = batch::status(&self.visuals) else {
                info(self.hwnd, "缩略图与文件分析", "暂无文件分析或缩略图任务。");
                return;
            };
            let subtitle = format!(
                "已保存 {} · 跳过 {} · 不支持 {} · 失败 {}{}",
                job.saved,
                job.skipped,
                job.unsupported,
                job.failed,
                if job.analysing {
                    format!(
                        " · 视频 {} · 其他 {} · 未识别 {}",
                        job.videos, job.other, job.unknown
                    )
                } else {
                    String::new()
                }
            );
            let actions = vec![
                "刷新".into(),
                "暂停".into(),
                "继续".into(),
                "取消任务".into(),
                "查看日志".into(),
            ];
            let Some(mut form) = self.ask(
                &batch_status(&job),
                &[
                    Field::Choice("任务操作", actions, 0),
                    Field::Choice("处理统计", vec![subtitle], 0),
                    Field::Choice(
                        "分区 / 错误",
                        vec![job.error.clone().unwrap_or(job.partition_id)],
                        0,
                    ),
                ],
            ) else {
                return;
            };
            match form.choice() {
                1 => batch::control(&self.visuals, BatchAction::Pause),
                2 => batch::control(&self.visuals, BatchAction::Resume),
                3 => batch::control(&self.visuals, BatchAction::Cancel),
                4 => {
                    let path = self.local_path.join("thumbnail-jobs.log");
                    match std::fs::read_to_string(path) {
                        Ok(log) => {
                            let tail = log
                                .lines()
                                .rev()
                                .take(18)
                                .collect::<Vec<_>>()
                                .into_iter()
                                .rev()
                                .collect::<Vec<_>>()
                                .join("\n");
                            info(self.hwnd, "文件分析日志", &tail);
                        }
                        Err(_) => info(self.hwnd, "文件分析日志", "暂无错误日志。"),
                    }
                }
                _ => {}
            }
        }
    }
}
