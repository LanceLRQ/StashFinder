# StashFinder Portable

[简体中文](portable.md)

For Windows 10/11 x64. This is a development portable build; the full clean-machine release matrix has not been completed.

1. Extract the entire ZIP into a writable folder, such as `D:\Tools\StashFinder`. Do not run it inside the archive.
2. Double-click `StashFinder.exe` as an ordinary user. Choose **Start indexing backend** and approve UAC if Windows prompts you. You can still open settings if you choose **Start later**. Exit and reopen StashFinder to choose again.
3. Register the partitions you want to manage from the overview. Unregistered partitions are not scanned. Development or installed-version data is not imported automatically.
4. Closing the window keeps the application in the tray and indexing continues. Click the tray icon or select **Show StashFinder** to restore the window.
5. To stop or move the application, select **Exit** from the tray menu or **Settings → About and diagnostics → Exit portable version**, and wait for the backend to finish before moving the entire folder. Active file operations prevent exit.

## Data location

All persistent application data resides in `data` next to the executable:

| Subdirectory | Purpose |
|---|---|
| `index` | Registrations, tasks, and independent partition indexes |
| `config` | UI preferences and image-cache limit |
| `local` | Image cache, local application data, and `EBWebView` user data |
| `runtime` | Instance lock and active session |

Data directories are created on first use. The original ZIP contains no indexes, disk registrations, or caches. Only one instance can run from a folder at a time. Exit before copying or backing up the directory; copying a live SQLite main file alone is not a backup.

The current instance's `data` directory is automatically excluded from the file index so database, cache, and WebView writes do not repeatedly trigger synchronization. Existing indexed paths in this directory are removed without deleting actual files; other files alongside the application and ordinary database files elsewhere remain indexed. Excluded-only events do not update the index or persisted cursor. Restart can safely replay this journal tail; journal gaps still use the existing validation or rebuild policy.

Delete-pending files held in NTFS internal storage are not exposed as user files. Already-deleted temporary files that were never indexed do not trigger empty synchronization after deletion evidence is verified; read failures still produce explicit errors.

Offline records remain searchable after moving to another computer. Connected disks need identity checks and index updates. The image-cache limit defaults to 2 GB and can be changed or cleared in settings without deleting indexes or source files. Carry the backend, `portable.json`, and the entire `data` folder along with the GUI.

## Dependencies and limitations

The GUI requires an installed [Microsoft Edge WebView2 Runtime](https://developer.microsoft.com/microsoft-edge/webview2/). This lightweight package does not carry a fixed runtime or install one over the network. Install the runtime first if it is missing.

The backend is elevated separately on each run. No Windows service or login startup entry is installed. Its endpoint is tied to this folder and isolated from the installed version. It stops on normal exit or when the GUI process ends. Canceling UAC has the same effect as choosing **Start later**. If the same account launches the desktop application again in the same Windows session, the new process exits. Unwritable directories produce an error and do not fall back to another data directory. The application does not grant administrator access on another computer.

The elevated backend enables the account's existing backup read privilege to enumerate protected directories. It does not grant new account rights, change source file ACLs or enable restore/write privileges. The backend reports an error if system policy has not granted this read privilege.

`manifest.json` records the base source commit, whether the working tree was modified, file sizes, and SHA256 hashes. `built_from_source=false` means prebuilt binaries were packaged. The adjacent `.sha256` file checks the complete ZIP. This development package is unsigned. Dependency sources are documented inside; complete release licensing and system verification remain part of the release phase.

Run `pnpm package:portable` from the repository root to build from source. Output defaults to `.local/packages/<timestamp>/`. Each run needs a new output directory and never replaces an existing package. To package prebuilt binaries, run `pnpm package:portable -SkipBuild -BinaryDirectory <directory>` with a directory containing `stashfinder-desktop.exe` and `stashfinder-service.exe`; the script does not execute these inputs. Verify with `scripts/verify-portable-package.ps1 -Archive <zip-path>`.
