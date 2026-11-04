# Development and validation

[简体中文](development.md) | English

This project currently provides an M0 validation window and synthetic benchmarks. It does not register, enumerate, or index existing disks. The service reports a fixed registration count of zero. Do not use this version as a personal file catalog.

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
cargo run -p stashfinder-service -- --foreground
```

In a second terminal:

```powershell
npm --prefix ui run tauri -- dev
```

Use the connection check button. It should report a foreground validation process. This reuses the service communication loop but does not install a Windows service. Closing the GUI does not stop this process; use Ctrl+C in its terminal. The local pipe allows the configured account, SYSTEM and administrators and rejects remote clients. No scanning, arbitrary file operations or command execution are exposed over IPC.

To build and launch without a development server:

```powershell
npm --prefix ui run tauri -- build --debug --no-bundle
& .\target\debug\stashfinder-desktop.exe
```

## File bridge acceptance

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

The script copies the built service to protected `%ProgramFiles%/StashFinder-M0-Verification/`, manually starts temporary service `StashFinderM0` as LocalSystem, probes it, waits 120 seconds for ordinary-user GUI checks, then stops/removes the service and generated protected binary. Existing services or directories with those names cause rejection rather than replacement. This is not a production installer and performs no disk scanning.

During the wait, the ordinary-user window should report a Windows background service. Close the GUI and run `target/debug/stashfinder-service.exe --probe` as that user; the service should still respond. Results are saved to ignored `.local/service-verification.json`. Actual second-account rejection and Windows 10 checks require corresponding test environments.

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

`crates/core` contains the protocol, `crates/windows` Windows adapters, `crates/service` the background entry point, `crates/store` the synthetic baseline, and `ui` the validation window. Production asset registration and independent partition database schemas begin in M1.
