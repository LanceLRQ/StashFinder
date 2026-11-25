# StashFinder Portable

[简体中文](portable.md)

For Windows 10/11 x64. This is a development portable build; the full clean-machine release matrix has not been completed.

1. Extract the ZIP into a writable folder, such as `D:\Tools\StashFinder`. You can also copy just `StashFinder.exe` to an empty directory. On first use it creates an adjacent `data` folder.
2. On first use, double-click `StashFinder.exe` as an ordinary user. Choose **Start indexing backend**, approve UAC if prompted, then register the partitions you want to manage from the overview. Only registered partitions are scanned. Data from development builds and installed versions is not imported automatically.
3. Once you have an index, run `StashFinder.exe --quick-search` or double-click the included `QuickSearch.cmd`. Type a file name or path to search, including records of offline partitions. The app reuses a running backend. If one needs to start, choosing **Start later** or canceling UAC exits Quick Search. Choosing **Start later** in the main application still lets you open settings. Exit and reopen the application to choose again.
4. Closing the main window leaves it in the tray. Click the tray icon to restore it. To exit the main application, choose **Exit** in the tray menu or **Settings → About and diagnostics → Exit portable version**. Active file operations prevent the main application from exiting.
5. Open Quick Search windows continue working after you exit the main application. The backend stops after its last client exits. Before moving the folder, exit the main application and close every Quick Search window, then move the whole folder.

To browse an indexed directory directly, including offline partitions, run `StashFinder.exe --browse` or choose **浏览 → 已登记分区** (Browse → Registered partitions) in Quick Search. Double-click a partition or folder to enter it, press Backspace to go up, and use the previous and next buttons to page through a directory. Ctrl+F returns to search.

With an empty catalog, click **登记分区…** (Register partition) in the native window. Identify one partition by its label, mount point and disk, select it, and click **登记所选分区** (Register selected partition). Automatic initial indexing is checked by default. The window selects the newly registered partition and shows its index progress. If you clear the checkbox, you can start indexing later from the registered partitions list. For NTFS without a USN journal, click **仅扫描建索引** to build a scan-only snapshot.

Choose **浏览 → 索引任务** (Browse → Index tasks) or run `StashFinder.exe --tasks` directly. The native list refreshes manual tasks and automatic tasks requiring attention. Select a task to read its error, cancel active work or retry once its partition is online and ready. Cancelling preserves the committed index and pauses automatic updates for that partition. Press Ctrl+F to return to search.

## Data location

All persistent application data resides in `data` next to the executable:

| Subdirectory | Purpose |
|---|---|
| `index` | Registrations, tasks, and independent partition indexes |
| `config` | UI preferences and image-cache limit |
| `local` | Image cache, local application data, and `EBWebView` user data |
| `runtime` | Instance lock and active session |

Data directories are created on first use. The original ZIP contains no indexes, disk registrations, or caches. Each portable folder has its own index and backend, and multiple Quick Search windows can use the same folder. For a backup, exit all programs using that folder and copy the entire directory.

The current instance's `data` directory is automatically excluded from the file index so database, cache, and WebView writes do not repeatedly trigger synchronization. Existing indexed paths in this directory are removed without deleting actual files; other files alongside the application and ordinary database files elsewhere remain indexed. Excluded-only events do not update the index or persisted cursor. Restart can safely replay this journal tail; journal gaps still use the existing validation or rebuild policy.

Delete-pending files held in NTFS internal storage are not exposed as user files. Already-deleted temporary files that were never indexed do not trigger empty synchronization after deletion evidence is verified; read failures still produce explicit errors.

Offline records remain searchable after moving to another computer. Connected disks need identity checks and index updates. The image-cache limit defaults to 2 GB and can be changed or cleared in settings without deleting indexes or source files. To keep registrations and indexes, move `StashFinder.exe` and the entire `data` folder together. To upgrade an older portable build, exit all its programs, extract the new package into a fresh directory, and copy the original `data` folder across.

## Dependencies and limitations

The main workspace and Quick Search use native Windows controls. Double-click the app to browse, search, and manage tasks in tabs. To use the web workspace, exit the current main window and run `StashFinder.exe --web-ui`. The web entry uses the system WebView2 Runtime. [Microsoft Edge WebView2 Runtime](https://developer.microsoft.com/microsoft-edge/webview2/)

When the backend needs to start, Windows elevates a second process running the same `StashFinder.exe`. The windows keep ordinary user privileges. The portable build installs no Windows service or login startup entry. Its endpoint is tied to this folder and isolated from the installed version. A client process that exits unexpectedly also releases its claim on the backend. The same account can run one main application per Windows session. Unwritable directories produce an error. The application does not grant administrator access on another computer.

The elevated backend enables the account's existing backup read privilege to enumerate protected directories. It does not grant new account rights, change source file ACLs or enable restore/write privileges. The backend reports an error if system policy has not granted this read privilege.

`manifest.json` records the base source commit, whether the working tree was modified, file sizes, and SHA256 hashes. `built_from_source=false` means prebuilt binaries were packaged. The adjacent `.sha256` file checks the complete ZIP. This development package is unsigned. Dependency sources are documented inside; complete release licensing and system verification remain part of the release phase.

Run `pnpm package:portable` from the repository root to build from source. Output defaults to `.local/packages/<timestamp>/`. Each build uses a new directory. To package prebuilt binaries, run `pnpm package:portable -SkipBuild -BinaryDirectory <directory>` with a directory containing the release build of `stashfinder-desktop.exe`. The script packages it as `StashFinder.exe`. Verify with `scripts/verify-portable-package.ps1 -Archive <zip-path>`.
