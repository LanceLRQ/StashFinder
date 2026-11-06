# File operations

[简体中文](file-operations.md) | English

Directory and search lists share the same operations. Indexing remains read-only. The ordinary-user desktop app delegates file operations to Windows; the background service exposes no arbitrary-path write command.

## Operations and shortcuts

| Operation | Access |
|---|---|
| Open a file / browse a folder | Double-click or Enter; offline folders browse the catalog only |
| Copy files | Ctrl+C, then paste in Explorer or another receiving app |
| Drag files out | Drag a file row; always copy semantics |
| Cut | Ctrl+X publishes a cut list; source files have not moved yet |
| Paste | Ctrl+V in the current browse directory, or its blank-area menu; search has no paste destination |
| Rename | F2 for one item, Enter to submit, Escape to cancel; existing-name collisions are rejected |
| Delete | Delete, then confirm a Windows recycle request |
| Permanently delete | Shift+Delete shows items and count; Cancel is the default |
| Properties | Windows properties for one online item; catalog metadata for other selections |
| Copy text | Copy names or paths from the menu, including offline and historical records |
| Windows menu | “更多...” at the bottom, for one item or multiple items with the same parent |

Use Ctrl/Shift to select. Ctrl+A selects all immediate directory children or all search results, including unloaded rows. Preparation is limited to 10,000 selected records per operation, checked before collapsing descendants; larger selections are rejected rather than truncated. After preparation, actual file operations collapse selected descendants under selected folders. Distinct hard-link paths remain distinct. File shortcuts do not intercept text inputs or IME composition.

## Availability and results

Preflight checks the registered partition, filesystem instance, catalog version and actual native file identity. Drive letters are not identity. Offline, historical, unknown or replaced objects and reparse points block real file operations. Update older scan catalogs lacking native identity first. Text copying does not access the source disk; offline paths use the partition nickname and are not currently openable paths.

Open search results update their connection markers, mount paths and menu availability from current device state without another search. A connection-state change alone preserves selection and scroll position. Results from a different filesystem instance remain historical records.

Paste requires an online registered browse directory. Explicit pastes may use external physical files from Explorer or another app. Plain text, virtual attachments and ambiguous move flags are rejected. Permissions match the ordinary user; there is no automatic elevation. A successful copy/cut command only publishes the clipboard. A drop accepted by the receiver does not prove copying has completed.

Other apps receive standard Windows file paths on the clipboard and own the subsequent operation. If source files, drive letters or mount locations change after publication, copy or cut again before pasting.

App-executed paste, rename and delete retain per-item completed, failed, skipped and not-run outcomes. Cancellation stops subsequent work without rolling back completed items. Closing during execution offers waiting or cancellation. Windows may present conflict, permission or non-recyclable-item prompts. A recycle failure is never silently retried as permanent deletion. Recycle support depends on the filesystem, device and Windows settings.

Cancelling a Windows conflict or similar dialog marks unfinished items as cancelled/skipped rather than failed copies. Earlier completed items and genuine failures retain their outcomes.

Changes appear through the normal background indexing process. Disabled automatic updates or paused partitions remain pending; the app does not fake synchronization by changing React rows. Browsing location stays in place, and an index version change clears the old selection to avoid targeting another object.

Basic scanning binds its watcher to the current mount session. A quick detach and reattach invalidates the old session even if device polling misses the offline interval. The background worker then recreates the watcher and reconciles changes while keeping the existing catalog searchable. Monitor or read failures are not evidence of deletion; periodic validation remains a supplement.

## Windows and long paths

NTFS supports case-only rename. On the tested exFAT/FAT32 volumes, Windows Shell can return success without changing the stored spelling. The app checks the actual name and reports failure in that case. Rename to a different name first, then to the desired spelling; the app does not automatically perform a two-step rename.

“更多...” hosts the traditional Shell menu using installed extensions. It does not promise the Windows 11 modern menu or Explorer-only features. Cross-directory selections do not expose this entry. Third-party commands are owned by Windows/extensions; the app does not fabricate their per-item completion results.

Long-path support also depends on the receiver, destination and associated app. Explorer on the development machine displayed “Destination Path Too Long” for a deeply nested directory copy. That copy is incomplete even if clipboard publication or drop acceptance succeeded. Individual files can be selected and copied to a shorter destination. The app does not alter system long-path settings, bypass permissions or implement a separate copying engine.

The preview pane currently shows metadata only. Content previews, tray support, startup integration and layout persistence across sessions are later work.
