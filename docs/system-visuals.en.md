# System icons and thumbnails

[简体中文](system-visuals.md)

File lists display icons associated with file types in Windows. Folders, applications, shortcuts, and similar entries request their individual icons when online, with generic type icons as a fallback. Type icons do not require access to the source file and also work for offline records.

The preview pane and thumbnail list check the local image cache, then the Windows thumbnail cache. On a miss, Windows may generate a thumbnail. Both views share the image requested at 512 pixels. Older small images remain readable at their original requested size when offline. Format support depends on the Windows thumbnail providers installed on the computer.

Offline previews read only the local image cache. Cached previews are labeled as historical; otherwise the pane shows a type icon and the offline state. Metadata still comes from the partition index, so a thumbnail failure does not interrupt browsing or searching. Multiple selections show metadata summaries without generating a batch of previews.

## Pre-generate thumbnails

Choose **Pre-generate thumbnails** in the overview details for an online partition or in a folder's context menu. Folder jobs include subfolders and filter entries from the existing index. By default the job skips existing thumbnails. Choose **Rebuild all** to have Windows extract new images. Each old image is replaced after its new image is saved successfully.

Open **Tools → Thumbnail jobs** to see discovered, processed, saved, skipped, and failed counts, or to pause, resume, and cancel the job. A disconnected partition waits for reconnection. The job keeps its progress when it reaches the cache limit or runs low on disk space. After adjusting the limit or freeing space, choose **Resume** to retry the current file. Exiting ends the job and retains saved images, which can be reused by choosing **Skip existing** next time.

## Cache settings

Open **Settings → Image cache** to view cache usage, free space on the cache drive, and current activity. The default limit is **2 GB (2,000,000,000 bytes)**. Apply **Unlimited** to disable eviction based on the cache limit.

Images, videos, and documents (PDF) are enabled for pre-generation by default. Disable any category or add comma-separated custom extensions such as `jxl, docx`. Changes apply to new jobs. The installed Windows thumbnail providers determine whether an added format can produce a thumbnail.

- Icons and thumbnails are separate PNG files, outside the SQLite indexes.
- Least recently used images are evicted when necessary. Lowering the limit immediately trims the cache. An image larger than the limit can be displayed without being stored.
- Usage counts cached image file bytes, rather than allocated filesystem space. Atomic writes may temporarily require additional space for a temporary file.
- Clearing requires confirmation in the application and removes only application images. The limit, partition registrations, indexes, and source files remain. Images are requested again when needed.
- Settings show actual waiting and active counts and the completed count for maintenance batches. Individual Windows thumbnail calls do not supply a reliable percentage, so the UI reports their stage instead.

On Windows, images reside in `%LOCALAPPDATA%\io.github.lancelrq.stashfinder\visual-cache` and configuration in `%APPDATA%\io.github.lancelrq.stashfinder\visual-cache.json`. Windows may redirect these paths when launched through a packaged application. They are separate from the service's partition indexes. Do not delete the entire application data directory as an image-cache cleanup.

## Consistency and limitations

Image keys include the partition, filesystem instance, entry identity, indexed size and timestamps, and requested size, without the drive letter. A drive-letter change does not require regenerating every image. Updated index metadata produces a new key. Online identity is verified before and after extraction, and stale or canceled requests cannot replace the current preview.

Images are historical snapshots and cannot prove that an offline file still exists. Cache reuse follows indexed metadata versions rather than content hashes. External changes not yet reflected in the index invalidate images only after indexing catches up.

Type associations are queried again on first use after each application start. Thumbnail providers can be slow, return no image, or fail. Changing selection cancels queued work, but a Windows call already in progress may still need to return. The application limits concurrent image work rather than attempting to terminate third-party system providers.

## Developer verification

See the [development guide](development.en.md) for the unified checks. Focused commands:

```powershell
cargo test -p stashfinder-windows --test visuals --locked
cargo test -p stashfinder-desktop --locked
pnpm --dir ui test src/Visuals.test.tsx src/CacheSettings.test.tsx
```

Windows interface tests create temporary samples and verify system type icons, actual thumbnail generation, and subsequent Windows cache hits. Desktop tests cover offline requests without provider access, drive-letter-independent keys, metadata changes, cancellation, stale results during clearing, limits, and actual maintenance progress.

Use a registered disposable partition for GUI verification: select an image, an unknown format, a folder, and an application; let images load, detach the test disk, and check the historical preview. Reopen the UI and remount the disk to verify reuse. Modify the test image, wait for index updates, and check the refreshed preview. Finally change the limit, cancel and confirm clearing, and verify persisted settings, usage, and unchanged source files. Automated checks do not replace verification across Windows versions, installed extensions, and devices.
