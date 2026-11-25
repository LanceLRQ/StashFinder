# StashFinder

[简体中文](README.md) · English

StashFinder catalogs drives on Windows. Say you remember saving a video but can't remember which external drive holds it. If you've already indexed that drive, you can look up the partition and path now and plug it in when you need to open the file.

## Getting started

You need Windows 10 or 11 (x64). Double-click the app to open the native Windows workspace with Overview, Browse, Search, and Tasks tabs in one window. Set up the build tools and run these commands from the repository root. [Development guide](docs/development.en.md)

```powershell
pnpm install --frozen-lockfile
pnpm package
```

- The **portable ZIP** is in `.local/packages/`. Place `StashFinder.exe` in a writable folder and open it to register and manage partitions. Run `StashFinder.exe --quick-search` or double-click the included `QuickSearch.cmd` to search with the main window closed. Both entry points use the adjacent `data/` folder. [Using the portable build](docs/portable.en.md)
- The **Windows installer** is in `target/release/bundle/nsis/`. Run the installer. By default, the backend starts and stops with the app. Choose Windows service mode during installation to keep indexing after you close the window.

When you first use the desktop-managed backend, choose to start it and approve the Windows administrator prompt. In the overview, open the partition registration dialog, select the partitions you want, and leave automatic initial indexing enabled. Once indexing finishes, double-click a partition to browse it, or enter a query in the overview and press Enter to search all registered partitions.

Ctrl+T opens a Browse tab, Ctrl+Shift+F opens a Search tab, and Ctrl+Tab switches tabs. Choose **Tools → Tasks** to see index progress and history. Ctrl+P opens Settings. To use the web workspace, run `StashFinder.exe --web-ui`. The web entry uses the system WebView2 Runtime.

## Searching and browsing

Run `StashFinder.exe --browse` or choose **浏览 → 已登记分区** (Browse → Registered partitions) in Quick Search to browse the indexed folders on a partition, including offline ones. Press Backspace to go up or Ctrl+F to return to search.

You can also start here with an empty catalog. Click **登记分区…** (Register partition), select a volume and click **登记所选分区** (Register selected partition). Initial indexing is enabled by default. The status bar shows progress for the selected partition. If you turn automatic indexing off, select the registered partition later to start it manually. For NTFS without a USN journal, click **仅扫描建索引** to build a scan-only index.

Open **浏览 → 索引任务** (Browse → Index tasks) or run `StashFinder.exe --tasks` to see execution phases and errors in the native window. Cancel a queued, running or waiting task to pause automatic updates for its partition. Once the partition is online and ready, you can retry a failed, interrupted or cancelled task.

In either installation mode, open **StashFinder 快速搜索** (Quick Search) directly from Start. Type a query to search registered partitions with the main desktop closed. In the results list, press Enter or double-click to open an online item, or press Ctrl+Shift+C to copy the path of an online or offline record.

Search includes both online and offline registered partitions by default. Browse folders and open search results in separate tabs, then return to where you left off. While a drive is offline, you can see the last saved names, paths, sizes, and timestamps, along with any cached thumbnails. Reconnect the drive to open or work with the files. You can include historical records from before a reformat in the search scope.

- Search by name or path, or filter by type, size, and date. For example, `*.mkv size:>=1GiB` finds MKV files at least 1 GiB in size. Save filters as presets or bookmark the whole search. [Search syntax and examples](docs/search.en.md)
- Find duplicate candidates by name and size across the full set of filtered results. Groups are based on metadata. Check the files separately to confirm whether their contents match.
- Copy, cut and paste, rename, or delete files on connected drives. You can also drag files into File Explorer to copy them. [File operations and shortcuts](docs/file-operations.en.md)

## Indexing and thumbnails

StashFinder scans registered partitions in read-only mode to collect file and folder metadata. Searches read the index directly. On NTFS, StashFinder uses the USN journal for incremental updates when it is available. You can choose a directory scan when the journal is missing, or give separate consent to create a journal. For exFAT and FAT32, StashFinder scans directories, monitors changes, and checks existing records when the drive reconnects.

Choose file analysis for a partition or folder to generate thumbnails ahead of time. The Tasks view in the Tools menu shows progress and the total number of files. On an online NTFS partition with a continuous USN journal index, you can also identify video files with ambiguous extensions such as `.ts` or `.dat`. This requires `ffprobe.exe` with `fd:` support on your `PATH`. The Video filter can use the saved results offline. [File analysis and thumbnail cache](docs/system-visuals.en.md)

## Development

The backend is written in Rust, the native UI uses Rust and Win32, and indexes are stored in local SQLite databases. The web workspace uses Tauri 2, React, and TypeScript. Run `pnpm dev` to develop the web workspace and `pnpm check` for automated checks. [Development and verification](docs/development.en.md) · [Query performance baseline](docs/query-performance.en.md)

## License

[MIT](LICENSE) · [Third-party notices](docs/third-party.en.md)
