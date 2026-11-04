# Development and validation

[简体中文](development.md) | English

This project provides M1 partition registration and persistence. The service queries device metadata without modifying it and registers only selected partitions. No files are scanned on any partition; registered partitions show “not indexed.” The development view retains M0 file bridge tests and synthetic benchmarks.

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

The home screen manages disks. An ordinary foreground process may lack access to physical disk layouts; the UI must report this explicitly. Use the LocalSystem verification service below for complete device validation. The development view includes a connection check. Closing the GUI does not stop the backend; use Ctrl+C in its terminal. The local pipe allows the configured account, SYSTEM and administrators and rejects remote clients. IPC accepts neither database paths nor arbitrary commands.

## Partition registration acceptance

- Nothing is selected on first launch. Select and name the partitions to manage; unselected partitions are neither registered nor scanned.
- Registered partitions are grouped by physical disk, with current mounts, filesystem, capacity and collection time. Naming changes software records only, not volume labels.
- Refresh devices after connection, disconnection or drive-letter changes. Offline capacity is historical; unavailable values remain unknown. M1 has no automatic hotplug synchronization.
- Conflicting or insufficient identity evidence, filesystem changes and database errors have explicit states. Uncertain devices are not automatically associated and old records are not overwritten. Post-format history management comes later.
- Registrations, IDs, names and historical capacity survive UI/service restart. A missing or corrupt ready database is not replaced by an empty one, and other partitions remain usable.

Foreground development requires an explicit absolute data directory. SCM mode defaults to `%ProgramData%\StashFinder`, which must be provisioned beforehand as a direct child of ProgramData. Disable root ACL inheritance, grant only SYSTEM and Administrators inheritable full control, and disallow reparse points. The GUI reads through the service, not directly from databases. The verification script below provisions isolated protected storage. Production installation and migration are not implemented.

`catalog.sqlite` stores registrations; `volumes/<filesystem-id>.sqlite` stores independent filesystem data. SQLite uses WAL/FULL: copying only a live main database is not a backup. The recovery API creates a separate candidate catalog only, not a complete backup/restore tool. Protocol version 2 has a 16 KiB frame limit and paginated asset snapshots; use UI and service binaries from the same build.

To build and launch without a development server:

```powershell
npm --prefix ui run tauri -- build --debug --no-bundle
& .\target\debug\stashfinder-desktop.exe
```

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

`crates/core` defines identity models and the protocol; `crates/windows` provides Windows adapters; `crates/service` coordinates device discovery and registration; `crates/store` provides the asset catalog, independent partition databases and synthetic baseline; `ui` provides asset management and the development validation view.
