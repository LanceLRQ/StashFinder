# Development and validation

[简体中文](development.md) | English

This project provides the desktop shell, partition registration, automatic NTFS indexing and incremental updates, task management and offline file search. Automatic updates are enabled for registered partitions; unregistered partitions are not scanned. A missing USN journal requires a scanning choice or separate creation consent. The partition indexing action still supports manual validation. The M0 file bridge view is shown only in frontend development builds, separate from real search results.

NTFS volume identity queries require a read-access handle, not a zero-access device-attribute handle. VHD acceptance exercises this query; automated checks cannot replace device-level validation.

## Environment and build

- Windows x64, targeting Windows 10/11. A successful developer-machine run does not validate both systems.
- Rust 1.94.0, pinned with rustfmt and Clippy in `rust-toolchain.toml`.
- Node.js and npm. The development environment uses Node 26.13.0 and npm 11.20.0.
- MSVC C++ build tools, Windows SDK, and WebView2: see the [official Tauri prerequisites](https://v2.tauri.app/start/prerequisites/).
- Preserve `Cargo.lock` and `ui/package-lock.json`. Initial dependency downloads require network access.

From the repository root:

```powershell
npm ci --prefix ui
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/check.ps1
```

The check runs Rust formatting, Clippy with warnings denied, tests and builds, plus Prettier, TypeScript, component tests and Vite builds. Close the validation window and foreground service process first so Windows does not lock build outputs. Outputs in `target/` and `ui/dist/` are ignored.

When available memory is limited, set `$env:CARGO_BUILD_JOBS = '1'` in the current terminal before running the check. The first full Windows/Tauri dependency build can require substantial memory; a single build job reduces concurrent memory use.

## Run the validation window

In an ordinary-user terminal:

```powershell
cargo run -p stashfinder-service -- --foreground --data-dir "$PWD\.local\dev-assets"
```

In a second terminal:

```powershell
npm --prefix ui run tauri -- dev
```

The default home screen is Overview, with a collapsible navigation rail. Files retains the current session's search input and results. Tasks separates current work, items needing attention and history. Cancellation targets a task ID and pauses its partition until updates are explicitly resumed. An ordinary foreground process may lack access to physical disk layouts; the UI must report this explicitly. Use the LocalSystem verification service below for complete device validation. Settings → About and diagnostics reports the actual app version, backend mode and connection state. Closing the GUI does not stop the backend; use Ctrl+C in its terminal. The local pipe allows the configured account, SYSTEM and administrators and rejects remote clients. IPC accepts neither database paths nor arbitrary commands.

## Desktop and preferences acceptance

- Search from Overview, switch pages or collapse the rail, then return: the query and results remain intact. Complete results are loaded in bounded viewport pages.
- Register partitions opens a dialog. Closing it preserves the overview connection filter. A service disconnect keeps the displayed asset snapshot with an error, rather than marking every device offline.
- Settings supports the startup page, collapsed rail and default path matching. Path matching applies when a search page next opens, without overriding an existing query's selection. Restart to verify persistence.
- Preferences live in `preferences.json` under Tauri's `app_config_dir` (usually `%APPDATA%\io.github.lancelrq.stashfinder\preferences.json` on Windows), separately from service indexes. Saving replaces the file through a temporary file in the same directory; failures preserve the old configuration and show an error. System startup entries are not changed.
- Corrupt or unsupported configuration versions produce an explicit error. Back up and move the configuration file, then restart to restore defaults. Do not change the index databases to reset the UI.
- Settings remains accessible without the service. UI preferences still work; About and diagnostics can reconnect and never reports a failed read as connected.
- Check the initial 1280×800 and minimum 1000×650 window sizes, plus Windows scaling at 100%, 125%, 150% and 200%. Component tests do not replace native-window and real-backend validation.

## Directory browsing and tabs acceptance

- Click a partition name in Overview or select a registered partition in Files. The tree reads the local index on demand and works offline. Double-click a folder or press Enter to enter it; use breadcrumbs or Alt+Up to go up.
- Use + or Ctrl+T for a browse tab, Ctrl+Tab to switch, and Ctrl+W to close. Each tab retains its location, scroll and multi-selection. Ctrl/Shift-click and arrow keys select entries; Ctrl+A selects all immediate directory children. Renaming a partition or changing its drive letter does not change its directory identity.
- Typing a query in a browse tab or searching from Overview opens the single search tab. Closing it returns to the previous browse tab. Ctrl+F focuses search. A result's context action, “在新标签页浏览所在目录” (browse parent in a new tab), uses filesystem and directory IDs and works offline.
- View toggles the tree, metadata preview and breadcrumbs. Resize tree and preview panes from their bottom corners. Preview shows indexed metadata only, explicitly labels unknown values and never reads file contents. Tab and pane state lasts for the current window session only.
- Directory pages contain at most 100 entries, also bounded by the 16 KiB frame budget. Load more fetches subsequent pages; a search target beyond the loaded page requires loading more to view it. F5 refreshes the current directory. Index changes reject old page cursors and prompt a refresh; read failures preserve visible rows instead of displaying an empty directory. A confirmed deleted directory falls back to its nearest available ancestor and clears selection.
- With a matching ordinary-user GUI and backend build, verify online and offline partitions: nested folders, two tabs with independent selection and scrolling, search and return, parent-directory location from results, and pane toggles. Automated tests and controlled browser fixtures do not replace real IPC acceptance.

## Production file operation acceptance

See [file operations](file-operations.en.md) for shortcuts, availability and limits. The desktop runs ordinary-user operations; service selection endpoints are read-only. Protocol 7 adds chunked selection. Partition schema 4 independently stores native scan identity. Older scan catalogs remain searchable but require an identity update before real operations.

After building, obtain the ordinary GUI user's SID, then run this in administrator PowerShell with a new disposable test directory:

```powershell
target/debug/stashfinder-service.exe --sid
.\scripts\verify-service.ps1 -OwnerSid 'S-1-5-21-REPLACE-WITH-USER-SID' -FileOperationsVhdDirectory 'D:\tmp\StashFinder-M6-new-directory' -HoldSeconds 1800
```

The temporary LocalSystem service uses an isolated catalog. A new 1 GiB VHD supplies NTFS/exFAT/FAT32 partitions and ordinary, Chinese, folder, empty-folder, long-path and collision fixtures. No USN journal is created and only test partitions are registered. Start ordinary-user GUI checks after `verification.json` reports `ready: true`. Create `stop-gui` inside the test directory to finish early. The script detaches its exact VHD; the wrapper stops/removes the temporary service, retaining the image, isolated data and evidence. Inspect reports on failure; never substitute personal disks. `-FileOperationsPhase Resume` only resumes the previously generated, label-matched fixture partitions with a new empty test catalog.

Check complete multi-selection and same-parent native menus; Explorer copy/drop contents and unchanged sources; bidirectional app/Explorer cut/paste, cross-volume moves and external physical sources; conflicts, cancellation and partial results; F2 and case-only rename; recycling and restoring only the test item; default cancellation and explicit permanent-delete confirmation; offline/replaced identity rejection, letter changes and backend restart; text/IME focus and close-during-operation prompts. Never empty the whole Recycle Bin or substitute drop acceptance for destination-file checks. Test a long source file copied to a short destination separately from copying a deeply nested folder hierarchy, which can encounter Windows limits.

A real IPC/native check is ignored by default. Run it only against the authorized, ready VHD above. It creates its own fixtures and checks identity, batch cross-volume copy, moves, truthful case-only rename results and permanent deletion on all three filesystems. It does not replace GUI acceptance:

```powershell
$env:SF_M6_MANIFEST = 'D:\tmp\StashFinder-M6-new-directory\verification.json'
cargo test -p stashfinder-service --test file_operations_real registered_three --locked -- --ignored --nocapture
Remove-Item Env:SF_M6_MANIFEST
```

## Partition registration acceptance

- Open Register partitions from Overview; nothing is initially selected. Select and name the partitions to manage; unselected partitions are neither registered nor scanned.
- Registered partitions are grouped by physical disk, with current mounts, filesystem, capacity and collection time. Naming changes software records only, not volume labels.
- The backend checks devices approximately every 5 seconds and online NTFS changes approximately every 2 seconds; device latency and running tasks affect response time. Offline capacity is historical and unavailable values remain unknown.
- Conflicting or insufficient identity evidence and database errors have explicit states. Uncertain devices are not automatically associated. Formatting preserves old records as historical; the new filesystem needs explicit registration. Manual removal of historical records is not available yet.
- Registrations, IDs, names and historical capacity survive UI/service restart. A missing or corrupt ready database is not replaced by an empty one, and other partitions remain usable.

Foreground development requires an explicit absolute data directory. SCM mode defaults to `%ProgramData%\StashFinder`, which must be provisioned beforehand as a direct child of ProgramData. Disable root ACL inheritance, grant only SYSTEM and Administrators inheritable full control, and disallow reparse points. The GUI reads through the service, not directly from databases. The verification script below provisions isolated protected storage. Production installation and migration are not implemented.

`catalog.sqlite` stores registrations; `volumes/<filesystem-id>.sqlite` stores independent filesystem data; `tasks.sqlite` stores tasks, automatic-update policy and paused partitions. SQLite uses WAL/FULL: copying only a live main database is not a backup. The recovery API creates a separate candidate catalog only, not a complete backup/restore tool. Protocol version 7 has a 16 KiB frame limit and paginated assets, tasks, directory entries and search results; use UI and service binaries from the same build. Existing partition databases migrate transactionally only after ownership validation; mismatched databases are not migrated.

## Automatic-update acceptance

- Settings → Indexing and synchronization enables automatic updates by default. Disabling requires confirmation and lets running tasks finish; manual indexing remains available. Failed saves retain the previous policy.
- Reconnection validates identity and journal bounds before replay. Gaps trigger rebuilding while the old active generation remains searchable. A failed change-page read does not advance its cursor or prove deletion.
- One worker serializes indexing. Empty journal checks do not create tasks or reload caches. Failures back off with at most three automatic attempts; reconnection or a manual retry can resume work. Keep the latest 200 completed, cancelled or superseded tasks, plus each partition's latest attention state.
- Pausing stops later batches and preserves committed data. Incremental work resumes from its cursor; full builds restart. Service shutdown or disconnection may wait for the current system call to return.
- Before running this command in an administrator terminal, explicitly authorize creating, changing and formatting the new test VHD and resetting its journal. Existing directories are rejected. The script unmounts its disk and removes the temporary service, retaining reports and test data.

```powershell
.\scripts\verify-service.ps1 -OwnerSid 'S-1-5-21-your-normal-user-SID' -SyncVhdDirectory 'D:\tmp\StashFinder-M3-new-directory' -HoldSeconds 600
```

The script covers automatic builds, directory renames and hard links, incremental deletion, policy and pause controls, SCM restart, remounting with a new letter, journal replacement and post-format history. Results are in `.local/sync-verification.json`. A VHD does not replace physical USB surprise-removal or sleep/resume tests.

To build and launch without a development server:

```powershell
npm --prefix ui run tauri -- build --debug --no-bundle
& .\target\debug\stashfinder-desktop.exe
```

## NTFS indexing and offline search

1. Register an accessible NTFS partition and choose its index action. The default uses an existing USN journal; an absent journal produces an explicit failure without creating one.
2. Scan-only mode creates no journal and starts monitoring before enumeration. Changes during a scan trigger validation; results are marked as scan snapshots. Creating a journal requires a separate checkbox: a new journal has a 32 MiB maximum and an 8 MiB allocation delta; existing settings remain unchanged.
3. Progress shows NTFS enumeration, metadata scanning and replay of changes during scanning. Cancellation is available. Failed/cancelled builds preserve the previous active index; readable data from a failed first build is marked incomplete. Read errors are not evidence of deletion.
4. Open file search. Matching is case-insensitive and includes paths by default: `aaa` matches `aaa\bbb.exe`; `aaa bbb` requires both words in either order; `"aaa bbb"` matches consecutive text. Filename-only mode and partition filters are available. Drive-letter terms match current online mounts only.
5. Detach the test disk, refresh devices, then search or restart the service. Partition names, relative paths, index timestamps and offline status must persist. Online describes partition state, not a guarantee that each snapshot entry still exists unchanged.

Search provides complete counts, global sorting, bounded batch pages, type/regex filters and metadata duplicate candidates; see search help. See file operations help for actions on real results. Reparse points are recorded without following targets; hard links retain multiple paths. File contents are not read. Cache and no-match performance still need optimization.

Build generations currently remain in partition databases, so repeated rebuilding increases database size; automatic reclamation is not implemented. Healthy local snapshots remain searchable when device access or identity is uncertain, with the connection issue shown instead of online status.

For complete device validation, run this command in elevated PowerShell. It explicitly authorizes journal creation and sample creation, renaming and deletion **only inside a newly created disposable VHD**:

```powershell
.\scripts\verify-service.ps1 -OwnerSid 'S-1-5-21-your-normal-user-SID' -IndexVhdDirectory 'D:\tmp\StashFinder-M2-new-directory' -HoldSeconds 600
```

The script requires a nonexistent directory, creates a 512 MiB VHDX with an NTFS test partition, and compares Unicode names, zero sizes, hard links, long paths, reparse points and metadata against independent directory enumeration. It then tests changes during scanning, cancellation, detachment and search after SCM restart. `.local/index-verification.json` records sample size, build time, no-match query time and service working set. These small debug-build measurements do not represent million/ten-million-file performance. The successful hold period permits ordinary-user GUI acceptance while the test disk is already offline. Completion or failure detaches the disk and cleans up the temporary service, retaining the image and protected data.

## Scan mode and capacity acceptance

exFAT/FAT32 partitions scan automatically after registration. For NTFS without a journal, choose scan-only mode in the indexing dialog. The choice persists across restarts without creating a journal. NTFS retains file references and hard links; exFAT/FAT32 use relative paths within each filesystem instance, not stable object IDs across renames.

Online monitoring uses [FindFirstChangeNotificationW](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-findfirstchangenotificationw) to coalesce changes before enumerating metadata again. It is not a persistent change journal. Idle volumes do not repeatedly rebuild, with fallback validation every 15 minutes. Reconnection, resuming automatic updates and service restart require validation. Read or monitoring failures do not publish a new generation or confirm deletion: the old index remains searchable, while incomplete first builds are explicitly partial. Continued changes retry a bounded number of times, with failures shown in tasks. This mode rereads the registered partition; large-disk latency and retained-generation space still need optimization.

Partitions display total, used and free space and the free percentage. Failed capacity reads retain the last successful values and timestamp with a separate error. They do not mark an online partition offline or block healthy indexing. Physical capacity remains separate; unallocated space is not partition free space.

This administrator check operates only on the specified new VHD. It creates a 1 GiB image with three 256 MiB NTFS/exFAT/FAT32 partitions, tests Unicode, empty files, long paths, online changes, SCM restart, detach/remount and offline capacity, and never creates a USN journal.

```powershell
.\scripts\verify-service.ps1 -OwnerSid 'S-1-5-21-your-normal-user-SID' -ScanVhdDirectory 'D:\tmp\StashFinder-M4-new-directory' -HoldSeconds 600
```

Results are in `.local/scans-verification.json`. During the hold period, validate the partitions using the ordinary-user GUI. Creating `show-offline` in the test directory detaches the image for offline GUI checks; `stop-gui` ends the hold early. Cleanup detaches the image and removes the temporary service, retaining test files and catalogs. This does not replace physical USB surprise-removal, sleep/resume or large-scale performance tests.

## File bridge acceptance

Open the development view first. These operations are limited to disposable samples.

1. Use the prepare-samples button: by default it generates two files, a directory containing a file, and a file with a path longer than 260 characters. Their displayed locations are under the local application data directory `io.github.lancelrq.stashfinder/m0-samples/`. They remain after the GUI closes.
2. Click the first row and Shift-click the third (Ctrl toggles individual entries; Ctrl+A selects all). Press Ctrl+C or right-click and choose copy, then paste into an empty directory in Explorer. Verify contents, nested files, and that sources remain. Arrow keys select entries and Esc clears selection.
3. Use a second empty directory. Hold a selected row and drag directly into Explorer. The selected group should remain intact and only copying should be offered; Esc during dragging should cancel. Test `second.txt` alone first, then multiple files and the directory.
4. Test the long-path sample separately. Record source length, Windows version, acceptance and errors. Shell or target-application limitations must be reported rather than silently skipped.
5. Remove one generated sample and retry its operation; an unavailable error must appear. Only generated samples may be removed for this test, never personal files.

The file bridge uses Unicode absolute paths in `CF_HDROP` and `Preferred DropEffect=COPY`. Automated tests inspect paths, use Windows `IFileOperation` to copy all four fixture types into a temporary directory, and verify destination and source contents. They do not replace Explorer end-to-end acceptance. The GUI accepts generated sample IDs only, not arbitrary paths. Production partition-identity revalidation requires the future registration model.

API references: [Shell clipboard formats](https://learn.microsoft.com/en-us/windows/win32/shell/clipboard) and [IFileOperation::CopyItems](https://learn.microsoft.com/en-us/windows/win32/api/shobjidl_core/nf-shobjidl_core-ifileoperation-copyitems). Global results can span directories, so the flat-item restriction of [BHID_DataObject](https://learn.microsoft.com/en-us/windows/win32/api/shobjidl_core/nf-shobjidl_core-ishellitemarray-bindtohandler) must be respected.

Launching a development process from a packaged app can redirect AppData into the launcher's `Packages/.../LocalCache`, leaving external Explorer unable to access the logical paths. This has been observed in validation; see [Windows packaged-app filesystem behavior](https://learn.microsoft.com/en-us/windows/msix/desktop/desktop-to-uwp-behind-the-scenes). Launch from an ordinary terminal, or set `STASHFINDER_M0_SAMPLE_ROOT` to an already prepared external M0 fixture directory, for example under `D:\tmp`. The window reads the fixed fixture set without overwriting it and rejects missing or incorrectly typed entries. This is not general folder registration or scanning.

```powershell
$env:STASHFINDER_M0_SAMPLE_ROOT = 'D:\tmp\prepared-M0-fixtures'
& .\target\debug\stashfinder-desktop.exe
```

Run the cross-process clipboard test explicitly with `cargo test -p stashfinder-windows --test clipboard -- --ignored`. It replaces the system clipboard and must not run alongside manual acceptance. Tests within the same execution environment do not replace external Explorer acceptance.

### Extended operations (disposable fixtures only)

The context menu also offers cut, rename, recycle, permanent deletion and refresh. These validate the Windows bridge; they do not expand the agreed production feature scope.

- `Ctrl+X` publishes `Preferred DropEffect=MOVE` without moving the source yet. Paste into an empty Explorer directory, verify content and source removal, then press `F5` in the window. A clipboard-success message does not prove a move succeeded.
- `F2` renames one item using a complete name including the extension. Paths, invalid names and existing targets are rejected. Verify the old path is gone, contents are unchanged and the list reflects the new name.
- `Delete` requests recycling after confirmation. Verify the Recycle Bin item and restore it, then compare contents. Cancel if Windows warns that recycling is unavailable and offers permanent deletion.
- `Shift+Delete` permanently deletes after a separate confirmation, bypassing the Recycle Bin. First cancel and verify the source remains; then confirm for the designated fixture and check the source and Recycle Bin. Never use personal files.
- `F5` rechecks the fixed fixture paths. Missing entries remain marked unavailable for verification; this is not production index synchronization. Restore and refresh to make them available again. Restarting still requires the complete initial fixture set; prepare a new set for subsequent runs.

Rename and deletion use [IFileOperation](https://learn.microsoft.com/en-us/windows/win32/api/shobjidl_core/nf-shobjidl_core-ifileoperation-setoperationflags). Cancellation or partial failure reports an error and refreshes the list rather than claiming full success. Drag-out remains copy-only.

## Windows service validation

Stop the foreground process. Run `target/debug/stashfinder-service.exe --sid` as the ordinary GUI user. In an administrator PowerShell, supply that SID:

```powershell
.\scripts\verify-service.ps1 -OwnerSid 'S-1-5-21-REPLACE-WITH-USER-SID' -HoldSeconds 120
```

The script copies the built service to protected `%ProgramFiles%/StashFinder-M0-Verification/`, provisions `%ProgramData%/StashFinder-Verification-<random-id>/`, manually starts temporary service `StashFinderM0` as LocalSystem, probes it, waits 120 seconds for ordinary-user GUI checks, then stops/removes the service and generated binary. Existing services or binary directories cause rejection. Data is retained for inspection and never reuses the development catalog. This is not a production installer; it queries device metadata without scanning files.

During the wait, the ordinary-user window should report a Windows background service. Close the GUI and run `target/debug/stashfinder-service.exe --probe` as that user; the service should still respond. Results are saved to ignored `.local/service-verification.json`. Actual second-account rejection and Windows 10 checks require corresponding test environments.

For drive-letter and detach/remount checks, explicitly add `-VhdDirectory 'D:\tmp\StashFinder-M1-new-directory'`. The script accepts only a new directory under D:\tmp, creates a 512 MiB dynamic VHDX and formats NTFS/exFAT/FAT32 partitions on that new disk. It verifies selection, naming, drive-letter changes, offline capacity, remounting and SCM restart. The hold period permits ordinary-user GUI checks; afterward the VHD is detached. The image and protected data directory are retained, not automatically deleted. Results are in `.local/assets-verification.json`. Never run these mounting/formatting tests against personal data disks.

After automated checks pass, the script creates an additional 64 MiB NTFS partition for GUI registration and naming; its drive letter is recorded as `guiMount`. Operate only on this test partition, then close and reopen the UI to check its name and the registration count. Failures also retain the requested diagnostic hold before disk and service cleanup; do not terminate the elevated script process directly.

## Synthetic baseline

```powershell
New-Item -ItemType Directory -Force .local | Out-Null
cargo build -p stashfinder-store --bin baseline --release --locked
.\target\release\baseline.exe .local\baseline-1m.sqlite 1000000 > .local\baseline-1m.json
.\target\release\baseline.exe .local\baseline-10m.sqlite 10000000 > .local\baseline-10m.json
```

The database path must not already exist. Each dataset contains 12 synthetic partitions, 1200 directories, Chinese/English names, duplicate names/sizes and nullable metadata. SQLite uses WAL, FULL synchronization and batched transactions, followed by an integrity check. The relationship cache loads from a reopened read-only database.

Reports include import, reopen and cache-load times, disk bytes, private commit, working set and peak working set. Five query classes run 20 repetitions each, reporting P95 for the first 200 results and comparing SQL/cache results. Queries use literal whitespace-separated AND substrings, not the full search grammar. Terms spanning a directory/name boundary conservatively match the full path.

The OS file cache is still warm after import; this is not a cold-start test. It does not measure real disk indexing, the complete production schema, global sorting, duplicates, regex, cancellation, GUI budgets or service idle budgets. Avoid simultaneous builds or other heavy tasks. `.local/` holds ignored validation artifacts.

## Layout

`crates/core` defines identity/query models and the protocol; `crates/windows` provides device and NTFS adapters; `crates/service` coordinates registration, indexing and search jobs; `crates/store` provides the asset catalog, independent partition databases, search caches and synthetic baseline; `ui` provides asset management, search, file operations and development validation.

## Full search acceptance

See [search help](search.en.md). Verify category persistence across restart, cross-partition joint duplicate keys, zero and unknown sizes, regex errors, global column sorting, distant scrolling and cross-page selection, and cancellation. Protocol version 7 keeps batch pages within 16 KiB. See the [query baseline](query-performance.en.md) for million/ten-million-row reproduction and limits.
