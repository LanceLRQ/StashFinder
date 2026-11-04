# StashFinder

[简体中文](README.md) | English

StashFinder is a personal disk catalog and file search tool for Windows 10/11. It aims to search files across multiple disks and use a local catalog to locate their partitions and paths even when those disks are disconnected.

## Status

The project is in M2 development, providing partition registration, manual NTFS indexing and basic offline search. Search supports paths, multiple keywords, filename-only matching and partition filters; results are local index snapshots and the UI is read-only. Device state requires manual refresh and file changes require manual rebuilding. Automatic synchronization, indexing other filesystems and advanced search are not available, and there is no everyday installer yet. The development view retains the M0 file bridge and synthetic benchmarks.

See the [development guide](docs/development.en.md) for build and validation steps and [third-party notices](docs/third-party.en.md) for dependency sources.

## Development goals

- Register selected partitions and manage them by physical disk. Give partitions custom names without creating duplicate registrations when drive letters change.
- Store file and directory names, paths, sizes, and timestamps, and search online and offline records together. File contents are not indexed.
- Use fast enumeration and USN updates on NTFS; use scanning and change monitoring on exFAT/FAT32, with full verification when needed.
- Search paths by default, with multiple keywords, file type and extension filters, and size, date, and regular expression conditions.
- Find duplicate candidates by name, size, or combined attributes across the entire filtered result set. Matching metadata does not establish identical contents.
- Show partition capacity, free space, connection state, and update times. Clearly label offline information as a historical snapshot.
- Copy files from online results or drag them into Explorer and other applications with copy semantics, and back up and restore the catalog.

## Boundaries

File scanning is limited to partitions registered by the user. Indexing does not modify ordinary user files. If a USN journal is missing, creating one requires separate consent; it is never enlarged automatically.

Offline results describe the last saved state and do not imply that files can currently be opened. Formatting a partition preserves its old catalog; the new filesystem is registered separately after confirmation.

## Technical direction

The selected stack is a Rust background service, a Tauri 2 + React + TypeScript UI, and local SQLite storage. Installing the service requires administrator permission; the everyday UI runs without elevation. The initial target is Windows 10/11 x64.

## License

MIT. See [LICENSE](LICENSE).
